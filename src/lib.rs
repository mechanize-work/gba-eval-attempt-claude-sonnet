mod cpu;
mod bus;
mod ppu;
mod apu;
mod dma;
mod timer;
#[cfg(test)]
mod tests;

// ============================================================
// Main GBA state struct
// ============================================================
pub(crate) struct Gba {
    // === CPU ===
    pub regs: [u32; 16],
    pub cpsr: u32,
    pub spsr: u32,

    // Banked registers
    pub bank_user: [u32; 7],  // R8-R14 user/sys
    pub bank_fiq:  [u32; 7],  // R8-R14 FIQ
    pub bank_irq:  [u32; 2],  // R13-R14 IRQ
    pub bank_svc:  [u32; 2],  // R13-R14 SVC
    pub bank_abt:  [u32; 2],  // R13-R14 ABT
    pub bank_und:  [u32; 2],  // R13-R14 UND

    pub spsr_fiq: u32,
    pub spsr_irq: u32,
    pub spsr_svc: u32,
    pub spsr_abt: u32,
    pub spsr_und: u32,

    pub halted: bool,
    pub stopped: bool,

    // === MEMORY ===
    pub bios:    Vec<u8>,  // 16 KiB
    pub wram:    Vec<u8>,  // 256 KiB
    pub iram:    Vec<u8>,  // 32 KiB
    pub rom:     Vec<u8>,  // up to 32 MiB
    pub sram:    Vec<u8>,  // 64 KiB

    // === PPU ===
    pub vram:    Vec<u8>,  // 96 KiB
    pub palette: Vec<u8>,  // 1 KiB
    pub oam:     Vec<u8>,  // 1 KiB

    // LCD control registers
    pub dispcnt:   u16,
    pub dispstat:  u16,
    pub vcount:    u16,
    pub bgcnt:     [u16; 4],
    pub bghofs:    [u16; 4],
    pub bgvofs:    [u16; 4],
    pub bgpa:      [i16; 2],
    pub bgpb:      [i16; 2],
    pub bgpc:      [i16; 2],
    pub bgpd:      [i16; 2],
    pub bgx_raw:   [i32; 2],
    pub bgy_raw:   [i32; 2],
    pub bgx_latch: [i32; 2],
    pub bgy_latch: [i32; 2],
    pub winh:      [u16; 2],
    pub winv:      [u16; 2],
    pub winin:     u16,
    pub winout:    u16,
    pub mosaic:    u16,
    pub bldcnt:    u16,
    pub bldalpha:  u16,
    pub bldy:      u16,

    // PPU timing
    pub scanline:  u32,
    pub dot:       u32,

    // Framebuffer output
    pub framebuffer: Vec<u32>,

    // === APU ===
    pub sound_ch1: SoundCh1,
    pub sound_ch2: SoundCh2,
    pub sound_ch3: SoundCh3,
    pub sound_ch4: SoundCh4,
    pub fifo_a:    [i8; 32],
    pub fifo_a_rd: usize,
    pub fifo_a_wr: usize,
    pub fifo_a_len: usize,
    pub fifo_b:    [i8; 32],
    pub fifo_b_rd: usize,
    pub fifo_b_wr: usize,
    pub fifo_b_len: usize,
    pub fifo_a_sample: i8,
    pub fifo_b_sample: i8,
    pub soundcnt_l: u16,
    pub soundcnt_h: u16,
    pub soundcnt_x: u16,
    pub soundbias:  u16,
    pub wave_ram:   [u8; 32],
    pub wave_bank:  u8,

    // Audio output
    pub audio_buffer:  Vec<i16>,
    pub audio_samples: usize,
    pub audio_cycles:  u32,  // cycles until next sample

    // Frame sequencer for APU envelopes/sweeps
    pub apu_frame_seq: u8,
    pub apu_frame_cycles: u32,

    // === DMA ===
    pub dma: [DmaChannel; 4],

    // === TIMERS ===
    pub timers: [TimerState; 4],

    // === INTERRUPTS ===
    pub ie:  u16,
    pub if_: u16,
    pub ime: u32,

    // === INPUT ===
    pub keyinput: u16,  // active-low (KEYINPUT register)
    pub keycnt:   u16,  // Key interrupt control

    // === MISC ===
    pub waitcnt:  u16,
    pub postflg:  u8,
    pub haltcnt:  u8,
    pub cycles:   u64,  // total cycles
    pub frame_cycles: u32,  // cycles within current frame

    // DMA active state
    pub dma_pending: u8,  // bitmask of DMA channels pending

    // Branch taken flag: set by any instruction that modifies PC
    pub branch_taken: bool,

    // CPU timing / wait states
    pub stall_cycles: u32,          // cycles consumed by current instruction
    pub cpu_cycles_remaining: i32,  // cycles CPU is stalled (waiting for memory)
    pub fetch_sequential: bool,     // true if last code fetch was sequential (no branch)
}

#[derive(Clone)]
pub(crate) struct SoundCh1 {
    pub sweep:      u16,
    pub duty:       u16,
    pub envelope:   u16,
    pub freq:       u16,
    pub enabled:    bool,
    pub duty_pos:   u8,
    pub length_ctr: u16,
    pub sweep_timer: u8,
    pub sweep_freq:  u32,
    pub env_volume:  u8,
    pub env_timer:   u8,
    pub freq_timer:  i32,
}

#[derive(Clone)]
pub(crate) struct SoundCh2 {
    pub duty:       u16,
    pub envelope:   u16,
    pub freq:       u16,
    pub enabled:    bool,
    pub duty_pos:   u8,
    pub length_ctr: u16,
    pub env_volume: u8,
    pub env_timer:  u8,
    pub freq_timer: i32,
}

#[derive(Clone)]
pub(crate) struct SoundCh3 {
    pub select:     u16,
    pub length:     u16,
    pub freq:       u16,
    pub enabled:    bool,
    pub length_ctr: u16,
    pub pos:        u8,
    pub freq_timer: i32,
}

#[derive(Clone)]
pub(crate) struct SoundCh4 {
    pub length:     u16,
    pub envelope:   u16,
    pub freq:       u16,
    pub enabled:    bool,
    pub length_ctr: u16,
    pub env_volume: u8,
    pub env_timer:  u8,
    pub lfsr:       u16,
    pub freq_timer: i32,
}

#[derive(Clone)]
pub(crate) struct DmaChannel {
    pub src_raw:  u32,
    pub dst_raw:  u32,
    pub cnt_raw:  u16,
    pub ctrl:     u16,
    pub src_int:  u32,
    pub dst_int:  u32,
    pub cnt_int:  u32,
    pub enabled:  bool,
}

#[derive(Clone)]
pub(crate) struct TimerState {
    pub counter:  u16,
    pub reload:   u16,
    pub ctrl:     u16,
    pub enabled:  bool,
    pub cascade:  bool,
    pub irq:      bool,
    pub prescaler: u32,  // 1, 64, 256, 1024
    pub cycles:   u32,
}

impl Default for SoundCh1 {
    fn default() -> Self { Self {
        sweep:0, duty:0, envelope:0, freq:0, enabled:false,
        duty_pos:0, length_ctr:0, sweep_timer:0, sweep_freq:0,
        env_volume:0, env_timer:0, freq_timer:0,
    }}
}
impl Default for SoundCh2 {
    fn default() -> Self { Self {
        duty:0, envelope:0, freq:0, enabled:false,
        duty_pos:0, length_ctr:0, env_volume:0, env_timer:0, freq_timer:0,
    }}
}
impl Default for SoundCh3 {
    fn default() -> Self { Self {
        select:0, length:0, freq:0, enabled:false,
        length_ctr:0, pos:0, freq_timer:0,
    }}
}
impl Default for SoundCh4 {
    fn default() -> Self { Self {
        length:0, envelope:0, freq:0, enabled:false,
        length_ctr:0, env_volume:0, env_timer:0, lfsr:0x7FFF, freq_timer:0,
    }}
}
impl Default for DmaChannel {
    fn default() -> Self { Self {
        src_raw:0, dst_raw:0, cnt_raw:0, ctrl:0,
        src_int:0, dst_int:0, cnt_int:0, enabled:false,
    }}
}
impl Default for TimerState {
    fn default() -> Self { Self {
        counter:0, reload:0, ctrl:0, enabled:false,
        cascade:false, irq:false, prescaler:1, cycles:0,
    }}
}

impl Gba {
    pub fn new() -> Self {
        let mut g = Gba {
            regs: [0u32; 16],
            cpsr: 0xD3,  // SVC mode, IRQ+FIQ disabled, ARM
            spsr: 0,
            bank_user: [0u32; 7],
            bank_fiq:  [0u32; 7],
            bank_irq:  [0u32; 2],
            bank_svc:  [0u32; 2],
            bank_abt:  [0u32; 2],
            bank_und:  [0u32; 2],
            spsr_fiq: 0, spsr_irq: 0, spsr_svc: 0, spsr_abt: 0, spsr_und: 0,
            halted: false, stopped: false,

            bios:    vec![0u8; 0x4000],
            wram:    vec![0u8; 0x40000],
            iram:    vec![0u8; 0x8000],
            rom:     vec![0u8; 0x2000000],
            sram:    vec![0u8; 0x10000],

            vram:    vec![0u8; 0x18000],
            palette: vec![0u8; 0x400],
            oam:     vec![0u8; 0x400],

            dispcnt: 0, dispstat: 0, vcount: 0,
            bgcnt: [0u16; 4], bghofs: [0u16; 4], bgvofs: [0u16; 4],
            bgpa: [0x100i16; 2], bgpb: [0i16; 2], bgpc: [0i16; 2], bgpd: [0x100i16; 2],
            bgx_raw: [0i32; 2], bgy_raw: [0i32; 2],
            bgx_latch: [0i32; 2], bgy_latch: [0i32; 2],
            winh: [0u16; 2], winv: [0u16; 2],
            winin: 0, winout: 0, mosaic: 0,
            bldcnt: 0, bldalpha: 0, bldy: 0,
            scanline: 0, dot: 0,
            framebuffer: vec![0u32; 240 * 160],

            sound_ch1: SoundCh1::default(),
            sound_ch2: SoundCh2::default(),
            sound_ch3: SoundCh3::default(),
            sound_ch4: SoundCh4::default(),
            fifo_a: [0i8; 32], fifo_a_rd: 0, fifo_a_wr: 0, fifo_a_len: 0,
            fifo_b: [0i8; 32], fifo_b_rd: 0, fifo_b_wr: 0, fifo_b_len: 0,
            fifo_a_sample: 0, fifo_b_sample: 0,
            soundcnt_l: 0, soundcnt_h: 0, soundcnt_x: 0, soundbias: 0x200,
            wave_ram: [0u8; 32], wave_bank: 0,
            audio_buffer: Vec::new(),
            audio_samples: 0,
            audio_cycles: 512,
            apu_frame_seq: 0, apu_frame_cycles: 0,

            dma: [DmaChannel::default(), DmaChannel::default(),
                  DmaChannel::default(), DmaChannel::default()],
            timers: [TimerState::default(), TimerState::default(),
                     TimerState::default(), TimerState::default()],

            ie: 0, if_: 0, ime: 0,
            keyinput: 0x03FF,  // all buttons released (active-low)
            keycnt: 0,

            waitcnt: 0, postflg: 0, haltcnt: 0,
            cycles: 0, frame_cycles: 0,
            dma_pending: 0,
            branch_taken: false,

            stall_cycles: 0,
            cpu_cycles_remaining: 0,
            fetch_sequential: false,
        };

        // Load BIOS stub
        let bios_bytes = include_bytes!("../spec/gba_bios_stub.bin");
        g.bios[..bios_bytes.len()].copy_from_slice(bios_bytes);

        // Boot: PC starts at 0x08000000 (ROM entry), skip BIOS
        g.regs[15] = 0x08000008;  // PC = ROM start + 8 (pipeline)
        // Set up stack pointers for each mode (standard GBA values)
        g.bank_svc[0] = 0x03007FE0;  // SVC SP
        g.bank_irq[0] = 0x03007FA0;  // IRQ SP
        g.regs[13]    = 0x03007F00;  // User SP
        g.cpsr = 0x5F;  // System mode, IRQ/FIQ enabled, ARM

        g
    }

    pub fn reset(&mut self) {
        // Reset CPU state (keep ROM loaded)
        self.regs = [0u32; 16];
        self.cpsr = 0x5F;
        self.spsr = 0;
        self.bank_user = [0u32; 7];
        self.bank_fiq  = [0u32; 7];
        self.bank_irq  = [0u32; 2];
        self.bank_svc  = [0u32; 2];
        self.bank_abt  = [0u32; 2];
        self.bank_und  = [0u32; 2];
        self.spsr_fiq = 0; self.spsr_irq = 0; self.spsr_svc = 0;
        self.spsr_abt = 0; self.spsr_und = 0;
        self.halted = false; self.stopped = false;

        // Reset memory
        self.wram.iter_mut().for_each(|b| *b = 0);
        self.iram.iter_mut().for_each(|b| *b = 0);
        self.vram.iter_mut().for_each(|b| *b = 0);
        self.palette.iter_mut().for_each(|b| *b = 0);
        self.oam.iter_mut().for_each(|b| *b = 0);

        // Reset PPU
        self.dispcnt = 0x0080; self.dispstat = 0; self.vcount = 0;  // GBA resets with forced blank
        self.bgcnt = [0u16; 4]; self.bghofs = [0u16; 4]; self.bgvofs = [0u16; 4];
        self.bgpa = [0x100i16; 2]; self.bgpb = [0i16; 2];
        self.bgpc = [0i16; 2]; self.bgpd = [0x100i16; 2];
        self.bgx_raw = [0i32; 2]; self.bgy_raw = [0i32; 2];
        self.bgx_latch = [0i32; 2]; self.bgy_latch = [0i32; 2];
        self.winh = [0u16; 2]; self.winv = [0u16; 2];
        self.winin = 0; self.winout = 0; self.mosaic = 0;
        self.bldcnt = 0; self.bldalpha = 0; self.bldy = 0;
        self.scanline = 0; self.dot = 0;

        // Reset APU
        self.sound_ch1 = SoundCh1::default();
        self.sound_ch2 = SoundCh2::default();
        self.sound_ch3 = SoundCh3::default();
        self.sound_ch4 = SoundCh4::default();
        self.fifo_a = [0i8; 32]; self.fifo_a_rd = 0; self.fifo_a_wr = 0; self.fifo_a_len = 0;
        self.fifo_b = [0i8; 32]; self.fifo_b_rd = 0; self.fifo_b_wr = 0; self.fifo_b_len = 0;
        self.fifo_a_sample = 0; self.fifo_b_sample = 0;
        self.soundcnt_l = 0; self.soundcnt_h = 0; self.soundcnt_x = 0;
        self.soundbias = 0x200;
        self.wave_ram = [0u8; 32]; self.wave_bank = 0;
        self.audio_buffer.clear(); self.audio_samples = 0; self.audio_cycles = 512;
        self.apu_frame_seq = 0; self.apu_frame_cycles = 0;

        // Reset DMA/timers
        self.dma = [DmaChannel::default(), DmaChannel::default(),
                    DmaChannel::default(), DmaChannel::default()];
        self.timers = [TimerState::default(), TimerState::default(),
                       TimerState::default(), TimerState::default()];

        // Reset interrupts
        self.ie = 0; self.if_ = 0; self.ime = 0;
        self.keyinput = 0x03FF;
        self.keycnt = 0;
        self.waitcnt = 0; self.postflg = 0; self.haltcnt = 0;
        self.cycles = 0; self.frame_cycles = 0;
        self.dma_pending = 0;
        self.branch_taken = false;
        self.stall_cycles = 0;
        self.cpu_cycles_remaining = 0;
        self.fetch_sequential = false;

        // Boot from ROM
        self.regs[15] = 0x08000008;
        self.bank_svc[0] = 0x03007FE0;
        self.bank_irq[0] = 0x03007FA0;
        self.regs[13]    = 0x03007F00;
        self.bank_user[5] = 0x03007F00;  // user/sys R13
        self.bank_user[6] = 0;            // user/sys R14
        self.cpsr = 0x5F;  // System mode
    }

    pub fn run_frame(&mut self) {
        // A frame is 280896 cycles
        // 228 scanlines * 1232 cycles each
        // Scanlines 0-159: visible, 160-227: VBlank
        // Each scanline: 960 cycles visible (dots 0-239), 272 cycles HBlank (dots 240-307 "equivalent")

        for _ in 0..280896 {
            self.tick_one_cycle();
        }

        // Latch affine BG reference points at end of VBlank
        // (they're latched at VBlank start; here they update continuously during frame)
    }

    fn tick_one_cycle(&mut self) {
        let dot = self.dot;
        let scanline = self.scanline;

        // PPU state machine
        if dot == 0 {
            // Start of scanline
            if scanline < 160 {
                // Render this scanline at the start (simplified)
                self.ppu_render_scanline(scanline);
            }
            if scanline == 0 {
                // VBlank end / new frame
                // Update affine BG latches at start of frame
                self.bgx_latch[0] = self.bgx_raw[0];
                self.bgy_latch[0] = self.bgy_raw[0];
                self.bgx_latch[1] = self.bgx_raw[1];
                self.bgy_latch[1] = self.bgy_raw[1];
                // Clear VBlank flag
                self.dispstat &= !0x01;
            }
        }

        // HBlank management
        if dot == 960 {
            // HBlank start
            self.dispstat |= 0x02;
            // HBlank interrupt
            if (self.dispstat & 0x10) != 0 {
                self.if_ |= 0x0002;
            }
            // HBlank DMA trigger (type 2)
            if scanline < 160 {
                self.trigger_dma(2);
            }
        }

        if dot == 1232 - 1 {
            // End of scanline
            self.dispstat &= !0x02;  // clear HBlank flag
            self.dot = 0;
            self.scanline += 1;

            if self.scanline == 228 {
                self.scanline = 0;
            }

            // VCount match
            let lyc = (self.dispstat >> 8) as u16;
            if self.scanline as u16 == lyc {
                self.dispstat |= 0x04;
                if (self.dispstat & 0x20) != 0 {
                    self.if_ |= 0x0004;
                }
            } else {
                self.dispstat &= !0x04;
            }

            // Update vcount register
            self.vcount = self.scanline as u16;

            if self.scanline == 160 {
                // VBlank start
                self.dispstat |= 0x01;
                if (self.dispstat & 0x08) != 0 {
                    self.if_ |= 0x0001;
                }
                // VBlank DMA trigger (type 1)
                self.trigger_dma(1);
                // Latch affine BG reference points at VBlank
                self.bgx_latch[0] = self.bgx_raw[0];
                self.bgy_latch[0] = self.bgy_raw[0];
                self.bgx_latch[1] = self.bgx_raw[1];
                self.bgy_latch[1] = self.bgy_raw[1];
            }
        } else {
            self.dot += 1;
        }

        // Audio sample generation
        if self.audio_cycles == 0 {
            self.apu_mix_sample();
            self.audio_cycles = 512;
        } else {
            self.audio_cycles -= 1;
        }

        // Timer update
        self.tick_timers(1);

        // If CPU is stalled for memory wait states, just advance hardware
        if self.cpu_cycles_remaining > 0 {
            self.cpu_cycles_remaining -= 1;
            self.cycles += 1;
            return;
        }

        // Run DMA if pending (DMA runs at "start" of cycle before CPU)
        if self.dma_pending != 0 {
            self.run_pending_dma();
        }

        // Check interrupts and run CPU
        self.stall_cycles = 0;
        if !self.halted {
            self.cpu_step();
        } else {
            // Check if interrupt is pending to wake up
            if (self.if_ & self.ie) != 0 {
                self.halted = false;
            }
        }

        // Service interrupts after CPU step (only bit 0 of IME matters)
        if (self.ime & 1) != 0 && (self.if_ & self.ie) != 0 && !self.halted {
            self.cpu_do_irq();
        }

        // stall_cycles accumulated by memory accesses; CPU stalls for remaining cycles
        if self.stall_cycles > 1 {
            self.cpu_cycles_remaining = (self.stall_cycles - 1) as i32;
        }

        self.cycles += 1;
    }
}

// ============================================================
// Global emulator instance (static)
// ============================================================
static mut GBA: Option<Box<Gba>> = None;
static mut ROM_BUF: Vec<u8> = Vec::new();

fn get_gba() -> &'static mut Gba {
    unsafe {
        GBA.as_mut().unwrap()
    }
}

// ============================================================
// ABI exports
// ============================================================

#[no_mangle]
pub extern "C" fn emu_init() -> i32 {
    unsafe {
        ROM_BUF = Vec::with_capacity(0x2000000);  // 32 MiB
        ROM_BUF.resize(0x2000000, 0);
        GBA = Some(Box::new(Gba::new()));
    }
    1
}

#[no_mangle]
pub extern "C" fn emu_rom_buffer() -> *mut u8 {
    unsafe { ROM_BUF.as_mut_ptr() }
}

#[no_mangle]
pub extern "C" fn emu_load_rom(len: i32) -> i32 {
    let gba = get_gba();
    let len = (len as usize).min(0x2000000);
    unsafe {
        gba.rom[..len].copy_from_slice(&ROM_BUF[..len]);
        if len < 0x2000000 {
            gba.rom[len..].iter_mut().for_each(|b| *b = 0xFF);
        }
    }
    gba.reset();
    1
}

#[no_mangle]
pub extern "C" fn emu_reset() -> i32 {
    get_gba().reset();
    1
}

#[no_mangle]
pub extern "C" fn emu_set_keys(k: u32) {
    let gba = get_gba();
    // k is active-high (bit set = button pressed)
    // KEYINPUT is active-low (bit clear = pressed)
    gba.keyinput = (!k as u16) & 0x03FF;
}

#[no_mangle]
pub extern "C" fn emu_run_frame() {
    get_gba().run_frame();
}

#[no_mangle]
pub extern "C" fn emu_framebuffer() -> *mut u32 {
    get_gba().framebuffer.as_mut_ptr()
}

#[no_mangle]
pub extern "C" fn emu_audio_buffer() -> *mut i16 {
    get_gba().audio_buffer.as_mut_ptr()
}

#[no_mangle]
pub extern "C" fn emu_audio_samples() -> i32 {
    let gba = get_gba();
    let n = gba.audio_samples as i32;
    gba.audio_samples = 0;
    gba.audio_buffer.clear();
    n
}

#[no_mangle]
pub extern "C" fn emu_audio_rate() -> i32 {
    32768
}
