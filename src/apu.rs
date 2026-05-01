use crate::Gba;

// Duty cycle waveforms (8 steps each)
const DUTY_WAVE: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1],  // 12.5%
    [1, 0, 0, 0, 0, 0, 0, 1],  // 25%
    [1, 0, 0, 0, 0, 1, 1, 1],  // 50%
    [0, 1, 1, 1, 1, 1, 1, 0],  // 75%
];

impl Gba {
    // ===== APU Sample Generation =====
    pub(crate) fn apu_mix_sample(&mut self) {
        // APU frame sequencer runs at 512 Hz (every 32768 cycles)
        // 32768 Hz sample rate, 512 Hz frame seq → every 64 samples
        self.apu_frame_cycles += 1;
        if self.apu_frame_cycles >= 64 {
            self.apu_frame_cycles = 0;
            self.apu_tick_frame_seq();
        }

        // Tick channel frequency timers
        // At 32768 Hz output rate, each sample = 512 GBA cycles
        // Channel frequencies: timer period in GBA cycles
        // But we're called once per 512 cycles, so tick each channel
        self.tick_ch1_freq();
        self.tick_ch2_freq();
        self.tick_ch3_freq();
        self.tick_ch4_freq();

        // Get channel samples
        let master_enable = (self.soundcnt_x >> 7) & 1 != 0;
        if !master_enable {
            self.audio_buffer.push(0);
            self.audio_buffer.push(0);
            self.audio_samples += 1;
            return;
        }

        let ch1 = self.sample_ch1();
        let ch2 = self.sample_ch2();
        let ch3 = self.sample_ch3();
        let ch4 = self.sample_ch4();

        // DMA audio channels
        let fifo_a_vol = if (self.soundcnt_h >> 2) & 1 != 0 { 2 } else { 1 };
        let fifo_b_vol = if (self.soundcnt_h >> 3) & 1 != 0 { 2 } else { 1 };
        let fifo_a_l = (self.soundcnt_h >> 8) & 1 != 0;
        let fifo_a_r = (self.soundcnt_h >> 9) & 1 != 0;
        let fifo_b_l = (self.soundcnt_h >> 12) & 1 != 0;
        let fifo_b_r = (self.soundcnt_h >> 13) & 1 != 0;

        let dma_a = self.fifo_a_sample as i32 * fifo_a_vol;
        let dma_b = self.fifo_b_sample as i32 * fifo_b_vol;

        // PSG channels mixing
        let psg_vol = match self.soundcnt_h & 0x3 {
            0 => 1,  // 25%
            1 => 2,  // 50%
            2 => 4,  // 100%
            _ => 4,
        };

        // Left/right PSG enable (from SOUNDCNT_L)
        let vol_r = (self.soundcnt_l & 0x7) as i32;
        let vol_l = ((self.soundcnt_l >> 4) & 0x7) as i32;
        let psg_en_r = (self.soundcnt_l >> 8) & 0xF;
        let psg_en_l = (self.soundcnt_l >> 12) & 0xF;

        let mut left: i32 = 0;
        let mut right: i32 = 0;

        // PSG channels (range -8..7 scaled to -128..112)
        macro_rules! add_psg {
            ($ch:expr, $bit:expr) => {
                if (psg_en_l >> $bit) & 1 != 0 { left += $ch as i32 * psg_vol; }
                if (psg_en_r >> $bit) & 1 != 0 { right += $ch as i32 * psg_vol; }
            }
        }
        add_psg!(ch1, 0);
        add_psg!(ch2, 1);
        add_psg!(ch3, 2);
        add_psg!(ch4, 3);

        // Scale PSG by master volume
        left = left * (vol_l + 1);
        right = right * (vol_r + 1);

        // Add DMA audio
        if fifo_a_l { left += dma_a * 8; }
        if fifo_a_r { right += dma_a * 8; }
        if fifo_b_l { left += dma_b * 8; }
        if fifo_b_r { right += dma_b * 8; }

        // Clamp to i16
        let left = left.max(-32768).min(32767) as i16;
        let right = right.max(-32768).min(32767) as i16;

        self.audio_buffer.push(left);
        self.audio_buffer.push(right);
        self.audio_samples += 1;
    }

    fn apu_tick_frame_seq(&mut self) {
        let step = self.apu_frame_seq;
        self.apu_frame_seq = (self.apu_frame_seq + 1) & 7;

        // Length counters (step 0, 2, 4, 6)
        if step & 1 == 0 {
            self.tick_length_ch1();
            self.tick_length_ch2();
            self.tick_length_ch3();
            self.tick_length_ch4();
        }

        // Sweep (step 2, 6)
        if step == 2 || step == 6 {
            self.tick_sweep_ch1();
        }

        // Envelopes (step 7)
        if step == 7 {
            self.tick_envelope_ch1();
            self.tick_envelope_ch2();
            self.tick_envelope_ch4();
        }
    }

    // ===== CH1: Square with Sweep =====
    fn tick_ch1_freq(&mut self) {
        if !self.sound_ch1.enabled { return; }
        let period = (2048 - (self.sound_ch1.freq & 0x7FF) as i32) * 4;
        // At 32768 Hz output, we decrement by 512 each sample
        // Period is in GBA cycles, so effective_period = period / 512 samples
        // We use fractional approach: decrement by 512, trigger when <= 0
        self.sound_ch1.freq_timer -= 512;
        while self.sound_ch1.freq_timer <= 0 {
            self.sound_ch1.freq_timer += period;
            self.sound_ch1.duty_pos = (self.sound_ch1.duty_pos + 1) & 7;
        }
    }

    fn sample_ch1(&self) -> i8 {
        if !self.sound_ch1.enabled { return 0; }
        let duty_idx = ((self.sound_ch1.duty >> 6) & 3) as usize;
        let wave = DUTY_WAVE[duty_idx][self.sound_ch1.duty_pos as usize];
        if wave != 0 { self.sound_ch1.env_volume as i8 } else { -(self.sound_ch1.env_volume as i8) }
    }

    fn tick_length_ch1(&mut self) {
        if (self.sound_ch1.freq >> 14) & 1 == 0 { return; }  // length disabled
        if self.sound_ch1.length_ctr > 0 {
            self.sound_ch1.length_ctr -= 1;
            if self.sound_ch1.length_ctr == 0 {
                self.sound_ch1.enabled = false;
            }
        }
    }

    fn tick_sweep_ch1(&mut self) {
        let sweep = self.sound_ch1.sweep;
        let period = ((sweep >> 4) & 7) as u8;
        let direction = (sweep >> 3) & 1 != 0;  // 0=up, 1=down
        let shift = (sweep & 7) as u8;

        if period == 0 { return; }

        if self.sound_ch1.sweep_timer > 0 {
            self.sound_ch1.sweep_timer -= 1;
        }
        if self.sound_ch1.sweep_timer == 0 {
            self.sound_ch1.sweep_timer = period;
            if shift > 0 {
                let freq = self.sound_ch1.sweep_freq;
                let delta = freq >> shift;
                let new_freq = if direction {
                    freq.wrapping_sub(delta as u32)
                } else {
                    freq.wrapping_add(delta as u32)
                };
                if new_freq > 2047 {
                    self.sound_ch1.enabled = false;
                } else {
                    self.sound_ch1.sweep_freq = new_freq;
                    self.sound_ch1.freq = (self.sound_ch1.freq & !0x7FF) | (new_freq as u16 & 0x7FF);
                }
            }
        }
    }

    fn tick_envelope_ch1(&mut self) {
        let env = self.sound_ch1.envelope;
        let period = (env & 7) as u8;
        if period == 0 { return; }
        if self.sound_ch1.env_timer > 0 { self.sound_ch1.env_timer -= 1; }
        if self.sound_ch1.env_timer == 0 {
            self.sound_ch1.env_timer = period;
            if (env >> 3) & 1 != 0 {
                if self.sound_ch1.env_volume < 15 { self.sound_ch1.env_volume += 1; }
            } else {
                if self.sound_ch1.env_volume > 0 { self.sound_ch1.env_volume -= 1; }
            }
        }
    }

    // ===== CH2: Square =====
    fn tick_ch2_freq(&mut self) {
        if !self.sound_ch2.enabled { return; }
        let period = (2048 - (self.sound_ch2.freq & 0x7FF) as i32) * 4;
        self.sound_ch2.freq_timer -= 512;
        while self.sound_ch2.freq_timer <= 0 {
            self.sound_ch2.freq_timer += period;
            self.sound_ch2.duty_pos = (self.sound_ch2.duty_pos + 1) & 7;
        }
    }

    fn sample_ch2(&self) -> i8 {
        if !self.sound_ch2.enabled { return 0; }
        let duty_idx = ((self.sound_ch2.duty >> 6) & 3) as usize;
        let wave = DUTY_WAVE[duty_idx][self.sound_ch2.duty_pos as usize];
        if wave != 0 { self.sound_ch2.env_volume as i8 } else { -(self.sound_ch2.env_volume as i8) }
    }

    fn tick_length_ch2(&mut self) {
        if (self.sound_ch2.freq >> 14) & 1 == 0 { return; }
        if self.sound_ch2.length_ctr > 0 {
            self.sound_ch2.length_ctr -= 1;
            if self.sound_ch2.length_ctr == 0 { self.sound_ch2.enabled = false; }
        }
    }

    fn tick_envelope_ch2(&mut self) {
        let env = self.sound_ch2.envelope;
        let period = (env & 7) as u8;
        if period == 0 { return; }
        if self.sound_ch2.env_timer > 0 { self.sound_ch2.env_timer -= 1; }
        if self.sound_ch2.env_timer == 0 {
            self.sound_ch2.env_timer = period;
            if (env >> 3) & 1 != 0 {
                if self.sound_ch2.env_volume < 15 { self.sound_ch2.env_volume += 1; }
            } else {
                if self.sound_ch2.env_volume > 0 { self.sound_ch2.env_volume -= 1; }
            }
        }
    }

    // ===== CH3: Wave =====
    fn tick_ch3_freq(&mut self) {
        if !self.sound_ch3.enabled { return; }
        let period = (2048 - (self.sound_ch3.freq & 0x7FF) as i32) * 2;
        self.sound_ch3.freq_timer -= 512;
        while self.sound_ch3.freq_timer <= 0 {
            self.sound_ch3.freq_timer += period;
            self.sound_ch3.pos = (self.sound_ch3.pos + 1) & 63;
        }
    }

    fn sample_ch3(&self) -> i8 {
        if !self.sound_ch3.enabled { return 0; }
        let bank = self.wave_bank as usize;
        let pos = self.sound_ch3.pos as usize;
        let bank_off = if (self.sound_ch3.select >> 6) & 1 != 0 {
            (bank ^ 1) * 16  // playing from opposite bank
        } else {
            bank * 16
        };
        let byte = self.wave_ram[bank_off + pos / 2];
        let nibble = if pos & 1 != 0 { (byte & 0xF) as i8 } else { ((byte >> 4) & 0xF) as i8 };
        // Volume
        let vol = (self.sound_ch3.length >> 13) & 3;
        let sample = nibble - 8;  // convert 0..15 to -8..7
        match vol {
            0 => 0,
            1 => sample,
            2 => sample / 2,
            3 => sample / 4,
            _ => 0
        }
    }

    fn tick_length_ch3(&mut self) {
        if (self.sound_ch3.freq >> 14) & 1 == 0 { return; }
        if self.sound_ch3.length_ctr > 0 {
            self.sound_ch3.length_ctr -= 1;
            if self.sound_ch3.length_ctr == 0 { self.sound_ch3.enabled = false; }
        }
    }

    // ===== CH4: Noise =====
    fn tick_ch4_freq(&mut self) {
        if !self.sound_ch4.enabled { return; }
        // LFSR period
        let r = (self.sound_ch4.freq & 0x7) as i32;
        let s = ((self.sound_ch4.freq >> 4) & 0xF) as i32;
        let period = (if r == 0 { 1 } else { r * 2 }) << (s + 1);
        self.sound_ch4.freq_timer -= 512;
        while self.sound_ch4.freq_timer <= 0 {
            self.sound_ch4.freq_timer += period;
            // Tick LFSR
            let bit = (self.sound_ch4.lfsr ^ (self.sound_ch4.lfsr >> 1)) & 1;
            self.sound_ch4.lfsr >>= 1;
            self.sound_ch4.lfsr |= bit << 14;
            if (self.sound_ch4.freq >> 3) & 1 != 0 {
                // 7-bit mode
                self.sound_ch4.lfsr &= !0x40;
                self.sound_ch4.lfsr |= bit << 6;
            }
        }
    }

    fn sample_ch4(&self) -> i8 {
        if !self.sound_ch4.enabled { return 0; }
        let out = !(self.sound_ch4.lfsr & 1) as i8;  // output is bit 0 inverted
        if out != 0 { self.sound_ch4.env_volume as i8 } else { -(self.sound_ch4.env_volume as i8) }
    }

    fn tick_length_ch4(&mut self) {
        if (self.sound_ch4.freq >> 14) & 1 == 0 { return; }
        if self.sound_ch4.length_ctr > 0 {
            self.sound_ch4.length_ctr -= 1;
            if self.sound_ch4.length_ctr == 0 { self.sound_ch4.enabled = false; }
        }
    }

    fn tick_envelope_ch4(&mut self) {
        let env = self.sound_ch4.envelope;
        let period = (env & 7) as u8;
        if period == 0 { return; }
        if self.sound_ch4.env_timer > 0 { self.sound_ch4.env_timer -= 1; }
        if self.sound_ch4.env_timer == 0 {
            self.sound_ch4.env_timer = period;
            if (env >> 3) & 1 != 0 {
                if self.sound_ch4.env_volume < 15 { self.sound_ch4.env_volume += 1; }
            } else {
                if self.sound_ch4.env_volume > 0 { self.sound_ch4.env_volume -= 1; }
            }
        }
    }

    // ===== Register read/write =====
    pub(crate) fn apu_read_ch1_sweep(&self) -> u16 { self.sound_ch1.sweep }
    pub(crate) fn apu_read_ch1_duty(&self) -> u16 { self.sound_ch1.duty & 0xC0 }  // only duty readable
    pub(crate) fn apu_read_ch2_duty(&self) -> u16 { self.sound_ch2.duty & 0xC0 }
    pub(crate) fn apu_read_ch3_select(&self) -> u16 { self.sound_ch3.select }
    pub(crate) fn apu_read_ch4_length(&self) -> u16 { self.sound_ch4.envelope }

    pub(crate) fn apu_write_ch1_sweep(&mut self, val: u16) {
        self.sound_ch1.sweep = val;
    }

    pub(crate) fn apu_write_ch1_duty(&mut self, val: u16) {
        self.sound_ch1.duty = val;
        self.sound_ch1.length_ctr = 64 - (val & 0x3F);
    }

    pub(crate) fn apu_write_ch1_freq(&mut self, val: u16) {
        self.sound_ch1.freq = val;
        if (val >> 15) & 1 != 0 {
            // Trigger
            self.sound_ch1.enabled = true;
            if self.sound_ch1.length_ctr == 0 { self.sound_ch1.length_ctr = 64; }
            let period = (2048 - (val & 0x7FF) as i32) * 4;
            self.sound_ch1.freq_timer = period;
            self.sound_ch1.env_volume = (self.sound_ch1.envelope >> 12) as u8;
            self.sound_ch1.env_timer = (self.sound_ch1.envelope & 7) as u8;
            self.sound_ch1.sweep_freq = (val & 0x7FF) as u32;
            self.sound_ch1.sweep_timer = ((self.sound_ch1.sweep >> 4) & 7) as u8;
            // Check DAC enable
            if (self.sound_ch1.envelope >> 11) == 0 { self.sound_ch1.enabled = false; }
        }
    }

    pub(crate) fn apu_write_ch2_duty(&mut self, val: u16) {
        self.sound_ch2.duty = val;
        self.sound_ch2.length_ctr = 64 - (val & 0x3F);
    }

    pub(crate) fn apu_write_ch2_freq(&mut self, val: u16) {
        self.sound_ch2.freq = val;
        if (val >> 15) & 1 != 0 {
            self.sound_ch2.enabled = true;
            if self.sound_ch2.length_ctr == 0 { self.sound_ch2.length_ctr = 64; }
            let period = (2048 - (val & 0x7FF) as i32) * 4;
            self.sound_ch2.freq_timer = period;
            self.sound_ch2.env_volume = (self.sound_ch2.envelope >> 12) as u8;
            self.sound_ch2.env_timer = (self.sound_ch2.envelope & 7) as u8;
            if (self.sound_ch2.envelope >> 11) == 0 { self.sound_ch2.enabled = false; }
        }
    }

    pub(crate) fn apu_write_ch3_select(&mut self, val: u16) {
        self.sound_ch3.select = val;
        if (val >> 7) & 1 == 0 {
            self.sound_ch3.enabled = false;
        }
        self.wave_bank = if (val >> 6) & 1 != 0 { 1 } else { 0 };
    }

    pub(crate) fn apu_write_ch3_length(&mut self, val: u16) {
        self.sound_ch3.length = val;
        self.sound_ch3.length_ctr = 256 - (val & 0xFF);
    }

    pub(crate) fn apu_write_ch3_freq(&mut self, val: u16) {
        self.sound_ch3.freq = val;
        if (val >> 15) & 1 != 0 {
            self.sound_ch3.enabled = true;
            if self.sound_ch3.length_ctr == 0 { self.sound_ch3.length_ctr = 256; }
            let period = (2048 - (val & 0x7FF) as i32) * 2;
            self.sound_ch3.freq_timer = period;
            if (self.sound_ch3.select >> 7) & 1 == 0 { self.sound_ch3.enabled = false; }
        }
    }

    pub(crate) fn apu_write_ch4_length(&mut self, val: u16) {
        self.sound_ch4.length = val;
        self.sound_ch4.envelope = val;  // wait, they're separate registers
        self.sound_ch4.length_ctr = 64 - (val & 0x3F);
    }

    pub(crate) fn apu_write_ch4_freq(&mut self, val: u16) {
        self.sound_ch4.freq = val;
        if (val >> 15) & 1 != 0 {
            self.sound_ch4.enabled = true;
            if self.sound_ch4.length_ctr == 0 { self.sound_ch4.length_ctr = 64; }
            let r = (val & 0x7) as i32;
            let s = ((val >> 4) & 0xF) as i32;
            let period = (if r == 0 { 1 } else { r * 2 }) << (s + 1);
            self.sound_ch4.freq_timer = period;
            self.sound_ch4.env_volume = (self.sound_ch4.envelope >> 12) as u8;
            self.sound_ch4.env_timer = (self.sound_ch4.envelope & 7) as u8;
            self.sound_ch4.lfsr = 0x7FFF;
            if (self.sound_ch4.envelope >> 11) == 0 { self.sound_ch4.enabled = false; }
        }
    }

    pub(crate) fn apu_write_soundcnt_h(&mut self, val: u16) {
        let old = self.soundcnt_h;
        self.soundcnt_h = val;
        // Reset FIFOs if requested
        if (val >> 11) & 1 != 0 {
            self.fifo_a_rd = 0; self.fifo_a_wr = 0; self.fifo_a_len = 0;
        }
        if (val >> 15) & 1 != 0 {
            self.fifo_b_rd = 0; self.fifo_b_wr = 0; self.fifo_b_len = 0;
        }
    }

    pub(crate) fn apu_write_soundcnt_x(&mut self, val: u16) {
        if (val >> 7) & 1 == 0 {
            // Power off sound
            self.sound_ch1.enabled = false;
            self.sound_ch2.enabled = false;
            self.sound_ch3.enabled = false;
            self.sound_ch4.enabled = false;
        }
        self.soundcnt_x = val & 0xFF80;
    }

    // Called by timer overflow to clock DMA audio FIFOs
    pub(crate) fn apu_timer_overflow(&mut self, timer: usize) {
        let fifo_a_timer = ((self.soundcnt_h >> 10) & 1) as usize;
        let fifo_b_timer = ((self.soundcnt_h >> 14) & 1) as usize;

        if timer == fifo_a_timer && (self.soundcnt_x >> 7) & 1 != 0 {
            self.fifo_a_sample = self.fifo_pop_a();
            // Request DMA refill if FIFO has <= 16 bytes
            if self.fifo_a_len <= 16 {
                // Trigger DMA 1 or 2 for sound A
                self.trigger_sound_dma(false);
            }
        }

        if timer == fifo_b_timer && (self.soundcnt_x >> 7) & 1 != 0 {
            self.fifo_b_sample = self.fifo_pop_b();
            if self.fifo_b_len <= 16 {
                self.trigger_sound_dma(true);
            }
        }
    }

    fn trigger_sound_dma(&mut self, fifo_b: bool) {
        // Find DMA channels 1 and 2 that are set to sound FIFO mode
        for ch in 1..=2usize {
            let ctrl = self.dma[ch].ctrl;
            let timing = (ctrl >> 12) & 3;
            if timing == 3 {
                // Sound FIFO timing
                let dest = self.dma[ch].dst_raw & 0xFFFFFFFC;
                let is_fifo_b = dest == 0x040000A4;
                let is_fifo_a = dest == 0x040000A0;
                if (!fifo_b && is_fifo_a) || (fifo_b && is_fifo_b) {
                    self.dma_pending |= 1 << ch;
                }
            }
        }
    }
}
