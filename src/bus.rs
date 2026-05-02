use crate::Gba;

impl Gba {
    // ===== Wait State Cycle Calculations =====

    // EWRAM 32-bit N/S cycles based on memcnt bits 24-27 (wait states: 15-bits, min 0x0E=1ws→2cyc 8/16bit, 4cyc 32bit)
    fn ewram_cycles(&self, width: u8) -> u32 {
        let ws = (self.memcnt >> 24) & 0xF;  // 0-14 = 15..1 wait states, 15=lockup
        let cyc = if ws >= 14 { 2 } else { 15 - ws };  // cycles for 8/16-bit
        if width == 4 { cyc * 2 } else { cyc }
    }

    // Non-sequential (N) access cycles for given address and width (1=8bit, 2=16bit, 4=32bit)
    // GBATek: table values are total cycle counts (1 base + wait states).
    pub(crate) fn mem_cycles_n(&self, addr: u32, width: u8) -> u32 {
        let wc = self.waitcnt;
        const WS_N: [u32; 4] = [4, 3, 2, 8];
        match addr >> 24 {
            0x00 => 1,                                          // BIOS
            0x02 => self.ewram_cycles(width),                   // WRAM 256K (16-bit bus)
            0x03 | 0x04 | 0x07 => 1,                           // IRAM, I/O, OAM
            0x05 | 0x06 => if width == 4 { 2 } else { 1 },     // Palette, VRAM (16-bit bus)
            0x08 | 0x09 => {                                    // ROM WS0
                let n = 1 + WS_N[((wc >> 2) & 3) as usize];
                let s = [2u32, 1][((wc >> 4) & 1) as usize];
                if width == 4 { n + s } else { n }
            }
            0x0A | 0x0B => {                                    // ROM WS1
                let n = 1 + WS_N[((wc >> 5) & 3) as usize];
                let s = [4u32, 1][((wc >> 7) & 1) as usize];
                if width == 4 { n + s } else { n }
            }
            0x0C | 0x0D => {                                    // ROM WS2
                let n = 1 + WS_N[((wc >> 8) & 3) as usize];
                let s = [8u32, 1][((wc >> 10) & 1) as usize];
                if width == 4 { n + s } else { n }
            }
            0x0E | 0x0F => 1 + WS_N[(wc & 3) as usize],       // SRAM
            _ => 1,
        }
    }

    // Sequential (S) access cycles for DATA reads (no prefetch buffer benefit)
    pub(crate) fn mem_cycles_s(&self, addr: u32, width: u8) -> u32 {
        let wc = self.waitcnt;
        match addr >> 24 {
            0x00 => 1,
            0x02 => self.ewram_cycles(width),
            0x03 | 0x04 | 0x07 => 1,
            0x05 | 0x06 => if width == 4 { 2 } else { 1 },
            0x08 | 0x09 => {
                // Data reads from ROM use wait-state S timing (NOT prefetch)
                let s = [2u32, 1][((wc >> 4) & 1) as usize];
                if width == 4 { s + s } else { s }
            }
            0x0A | 0x0B => {
                let s = [4u32, 1][((wc >> 7) & 1) as usize];
                if width == 4 { s + s } else { s }
            }
            0x0C | 0x0D => {
                let s = [8u32, 1][((wc >> 10) & 1) as usize];
                if width == 4 { s + s } else { s }
            }
            0x0E | 0x0F => {
                const WS_N: [u32; 4] = [4, 3, 2, 8];
                WS_N[(wc & 3) as usize]
            }
            _ => 1,
        }
    }

    // Sequential (S) instruction fetch cycles (prefetch buffer applies to ROM)
    pub(crate) fn insn_cycles_s(&self, addr: u32, width: u8) -> u32 {
        let wc = self.waitcnt;
        let prefetch = (wc >> 14) & 1 != 0;
        match addr >> 24 {
            0x08 | 0x09 => {
                let s = if prefetch { 1 } else { [2u32, 1][((wc >> 4) & 1) as usize] };
                if width == 4 { s + s } else { s }
            }
            0x0A | 0x0B => {
                let s = if prefetch { 1 } else { [4u32, 1][((wc >> 7) & 1) as usize] };
                if width == 4 { s + s } else { s }
            }
            0x0C | 0x0D => {
                let s = if prefetch { 1 } else { [8u32, 1][((wc >> 10) & 1) as usize] };
                if width == 4 { s + s } else { s }
            }
            _ => self.mem_cycles_s(addr, width),
        }
    }

    // Non-sequential (N) instruction fetch cycles
    // With prefetch enabled, N-fetch served from prefetch buffer = 1 cycle (same as S)
    pub(crate) fn insn_cycles_n(&self, addr: u32, width: u8) -> u32 {
        let prefetch = (self.waitcnt >> 14) & 1 != 0;
        if prefetch {
            self.insn_cycles_s(addr, width)
        } else {
            self.mem_cycles_n(addr, width)
        }
    }

    // Write cycles: EWRAM has write buffer — CPU does not stall for EWRAM writes
    pub(crate) fn write_cycles_n(&self, addr: u32, width: u8) -> u32 {
        if (addr >> 24) == 0x02 { 0 } else { self.mem_cycles_n(addr, width) }
    }

    // Write cycles sequential: EWRAM has write buffer — CPU does not stall for EWRAM writes
    pub(crate) fn write_cycles_s(&self, addr: u32, width: u8) -> u32 {
        if (addr >> 24) == 0x02 { 0 } else { self.mem_cycles_s(addr, width) }
    }

    // Add cycles for a data access (N cycles, non-sequential)
    #[inline(always)]
    pub(crate) fn add_data_cycles(&mut self, addr: u32, width: u8) {
        self.stall_cycles += self.mem_cycles_n(addr, width);
    }

    // ===== Memory Read =====

    pub(crate) fn mem_read8(&mut self, addr: u32) -> u8 {
        let addr = addr & 0x0FFFFFFF;
        match addr >> 24 {
            0x00 => {
                // BIOS - only accessible when PC is in BIOS range
                self.bios[(addr & 0x3FFF) as usize]
            }
            0x02 => self.wram[(addr & 0x3FFFF) as usize],
            0x03 => self.iram[(addr & 0x7FFF) as usize],
            0x04 => self.io_read8(addr & 0x3FF),
            0x05 => self.palette[(addr & 0x3FF) as usize],
            0x06 => {
                let off = (addr & 0x1FFFF) as usize;
                let off = if off >= 0x18000 { off - 0x8000 } else { off };
                self.vram[off]
            }
            0x07 => self.oam[(addr & 0x3FF) as usize],
            0x08..=0x0D => {
                let off = (addr & 0x1FFFFFF) as usize;
                if off < self.rom.len() { self.rom[off] } else { 0xFF }
            }
            0x0E | 0x0F => self.sram[(addr & 0xFFFF) as usize],
            _ => 0xFF
        }
    }

    pub(crate) fn mem_read16(&mut self, addr: u32) -> u16 {
        let addr = addr & !1;  // align
        let lo = self.mem_read8(addr) as u16;
        let hi = self.mem_read8(addr + 1) as u16;
        lo | (hi << 8)
    }

    pub(crate) fn mem_read32(&mut self, addr: u32) -> u32 {
        let addr = addr & !3;  // align
        // Special case for 0x04000800 (EWRAM wait control, mirrored every 64K)
        if (addr >> 24) == 0x04 && (addr & 0xFFFF) == 0x0800 {
            return self.memcnt;
        }
        let b0 = self.mem_read8(addr) as u32;
        let b1 = self.mem_read8(addr + 1) as u32;
        let b2 = self.mem_read8(addr + 2) as u32;
        let b3 = self.mem_read8(addr + 3) as u32;
        b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
    }

    // Read 32-bit with rotation for misaligned LDR
    pub(crate) fn mem_read32_rotate(&mut self, addr: u32) -> u32 {
        let align = addr & 3;
        let val = self.mem_read32(addr & !3);
        val.rotate_right(align * 8)
    }

    // ===== Memory Write =====

    pub(crate) fn mem_write8(&mut self, addr: u32, val: u8) {
        let addr = addr & 0x0FFFFFFF;
        match addr >> 24 {
            0x02 => self.wram[(addr & 0x3FFFF) as usize] = val,
            0x03 => self.iram[(addr & 0x7FFF) as usize] = val,
            0x04 => self.io_write8(addr & 0x3FF, val),
            0x05 => {
                // Palette: 8-bit writes write the byte to both halves of the 16-bit entry
                let off = (addr & 0x3FE) as usize;
                self.palette[off] = val;
                self.palette[off + 1] = val;
            }
            0x06 => {
                let off = (addr & 0x1FFFF) as usize;
                let off = if off >= 0x18000 { off - 0x8000 } else { off };
                // 8-bit writes to VRAM: replicate byte to both bytes of halfword
                // in BG tile areas; ignored in bitmap BG areas and OAM
                let mode = self.dispcnt & 7;
                let bg_end = if mode >= 3 { 0x14000 } else { 0x10000 };
                if off < bg_end {
                    // BG area: replicate byte
                    let aligned = off & !1;
                    if aligned + 1 < self.vram.len() {
                        self.vram[aligned] = val;
                        self.vram[aligned + 1] = val;
                    }
                } else if off >= 0x10000 {
                    // OBJ tile area: replicate byte
                    let aligned = off & !1;
                    if aligned + 1 < self.vram.len() {
                        self.vram[aligned] = val;
                        self.vram[aligned + 1] = val;
                    }
                }
                // else: bitmap mode BG area - 8-bit writes ignored
            }
            0x07 => {
                // OAM: 8-bit writes are ignored
            }
            0x0E | 0x0F => self.sram[(addr & 0xFFFF) as usize] = val,
            _ => {}
        }
    }

    pub(crate) fn mem_write16(&mut self, addr: u32, val: u16) {
        let addr = addr & !1;
        let lo = val as u8;
        let hi = (val >> 8) as u8;
        match (addr & 0x0FFFFFFF) >> 24 {
            0x02 => {
                let off = (addr & 0x3FFFE) as usize;
                self.wram[off] = lo; self.wram[off + 1] = hi;
            }
            0x03 => {
                let off = (addr & 0x7FFE) as usize;
                self.iram[off] = lo; self.iram[off + 1] = hi;
            }
            0x04 => self.io_write16(addr & 0x3FF, val),
            0x05 => {
                let off = (addr & 0x3FE) as usize;
                self.palette[off] = lo; self.palette[off + 1] = hi;
            }
            0x06 => {
                let off = (addr & 0x1FFFE) as usize;
                let off = if off >= 0x18000 { off - 0x8000 } else { off };
                if off + 1 < self.vram.len() {
                    self.vram[off] = lo; self.vram[off + 1] = hi;
                }
            }
            0x07 => {
                let off = (addr & 0x3FE) as usize;
                self.oam[off] = lo; self.oam[off + 1] = hi;
            }
            0x0E | 0x0F => {
                let off = (addr & 0xFFFE) as usize;
                self.sram[off] = lo; self.sram[off + 1] = hi;
            }
            _ => {}
        }
    }

    pub(crate) fn mem_write32(&mut self, addr: u32, val: u32) {
        let addr = addr & !3;
        match (addr & 0x0FFFFFFF) >> 24 {
            0x04 => {
                // 0x04000800 is mirrored every 64K (lower 16 bits = 0x0800)
                if (addr & 0xFFFF) == 0x0800 {
                    self.memcnt = val & 0xF000_00FF;
                } else {
                    self.io_write32(addr & 0x3FF, val);
                }
            }
            _ => {
                self.mem_write16(addr, val as u16);
                self.mem_write16(addr + 2, (val >> 16) as u16);
            }
        }
    }

    // ===== I/O Register Read =====
    pub(crate) fn io_read8(&mut self, offset: u32) -> u8 {
        let val = self.io_read16(offset & !1);
        if offset & 1 != 0 { (val >> 8) as u8 } else { val as u8 }
    }

    pub(crate) fn io_read16(&mut self, offset: u32) -> u16 {
        match offset {
            // LCD
            0x000 => self.dispcnt,
            0x002 => 0,  // GREENSWAP
            0x004 => self.dispstat,
            0x006 => self.vcount,
            0x008 => self.bgcnt[0],
            0x00A => self.bgcnt[1],
            0x00C => self.bgcnt[2],
            0x00E => self.bgcnt[3],
            0x010 => self.bghofs[0],
            0x012 => self.bgvofs[0],
            0x014 => self.bghofs[1],
            0x016 => self.bgvofs[1],
            0x018 => self.bghofs[2],
            0x01A => self.bgvofs[2],
            0x01C => self.bghofs[3],
            0x01E => self.bgvofs[3],
            0x040 => self.winh[0],
            0x042 => self.winh[1],
            0x044 => self.winv[0],
            0x046 => self.winv[1],
            0x048 => self.winin,
            0x04A => self.winout,
            0x04C => self.mosaic,
            0x050 => self.bldcnt,
            0x052 => self.bldalpha,

            // Sound
            0x060 => self.apu_read_ch1_sweep(),
            0x062 => self.apu_read_ch1_duty(),
            0x064 => self.sound_ch1.freq & 0x4000,  // only bit 14 readable
            0x068 => self.apu_read_ch2_duty(),
            0x06C => self.sound_ch2.freq & 0x4000,
            0x070 => self.apu_read_ch3_select(),
            0x072 => self.sound_ch3.length & 0xE000,
            0x074 => self.sound_ch3.freq & 0x4000,
            0x078 => self.apu_read_ch4_length(),
            0x07C => self.sound_ch4.freq & 0x4000,
            0x080 => self.soundcnt_l,
            0x082 => self.soundcnt_h,
            0x084 => self.read_soundcnt_x(),
            0x088 => self.soundbias,
            0x090..=0x09E => {
                let idx = (offset - 0x090) as usize;
                (self.wave_ram[idx] as u16) | ((self.wave_ram[idx + 1] as u16) << 8)
            }

            // DMA
            0x0B8 => 0,  // DMA0 count (write only)
            0x0BA => self.dma[0].ctrl,
            0x0C4 => 0,
            0x0C6 => self.dma[1].ctrl,
            0x0D0 => 0,
            0x0D2 => self.dma[2].ctrl,
            0x0DC => 0,
            0x0DE => self.dma[3].ctrl,

            // Timers
            0x100 => self.timers[0].counter,
            0x102 => self.timers[0].ctrl,
            0x104 => self.timers[1].counter,
            0x106 => self.timers[1].ctrl,
            0x108 => self.timers[2].counter,
            0x10A => self.timers[2].ctrl,
            0x10C => self.timers[3].counter,
            0x10E => self.timers[3].ctrl,

            // Serial (stub)
            0x120 => 0,
            0x128 => 0,
            0x12A => 0,
            0x134 => 0,

            // Keypad
            0x130 => self.keyinput,
            0x132 => self.keycnt,

            // Interrupts
            0x200 => self.ie,
            0x202 => self.if_,
            0x204 => self.waitcnt,
            0x208 => self.ime as u16,
            0x20A => (self.ime >> 16) as u16,

            // Misc
            0x300 => self.postflg as u16,

            _ => 0
        }
    }

    fn read_soundcnt_x(&self) -> u16 {
        let mut val = self.soundcnt_x & 0xFF00;
        if self.sound_ch1.enabled { val |= 1; }
        if self.sound_ch2.enabled { val |= 2; }
        if self.sound_ch3.enabled { val |= 4; }
        if self.sound_ch4.enabled { val |= 8; }
        val
    }

    pub(crate) fn io_read32(&mut self, offset: u32) -> u32 {
        let lo = self.io_read16(offset) as u32;
        let hi = self.io_read16(offset + 2) as u32;
        lo | (hi << 16)
    }

    // ===== I/O Register Write =====
    pub(crate) fn io_write8(&mut self, offset: u32, val: u8) {
        // Special-case byte-only registers
        match offset {
            0x301 => {
                // HALTCNT
                if val & 0x80 == 0 {
                    self.halted = true;
                } else {
                    self.stopped = true;
                }
                return;
            }
            _ => {}
        }
        // Read-modify-write for 8-bit I/O writes
        let current = self.io_read16(offset & !1);
        let new = if offset & 1 != 0 {
            (current & 0x00FF) | ((val as u16) << 8)
        } else {
            (current & 0xFF00) | val as u16
        };
        self.io_write16(offset & !1, new);
    }

    pub(crate) fn io_write16(&mut self, offset: u32, val: u16) {
        match offset {
            // LCD
            0x000 => self.dispcnt = val,
            0x002 => {}  // GREENSWAP
            0x004 => {
                // DISPSTAT - bits 0-2 are read-only (set by hardware)
                self.dispstat = (self.dispstat & 0x07) | (val & !0x07);
            }
            0x006 => {}  // VCOUNT read-only
            0x008 => self.bgcnt[0] = val,
            0x00A => self.bgcnt[1] = val,
            0x00C => self.bgcnt[2] = val,
            0x00E => self.bgcnt[3] = val,
            0x010 => self.bghofs[0] = val & 0x1FF,
            0x012 => self.bgvofs[0] = val & 0x1FF,
            0x014 => self.bghofs[1] = val & 0x1FF,
            0x016 => self.bgvofs[1] = val & 0x1FF,
            0x018 => self.bghofs[2] = val & 0x1FF,
            0x01A => self.bgvofs[2] = val & 0x1FF,
            0x01C => self.bghofs[3] = val & 0x1FF,
            0x01E => self.bgvofs[3] = val & 0x1FF,
            0x020 => self.bgpa[0] = val as i16,
            0x022 => self.bgpb[0] = val as i16,
            0x024 => self.bgpc[0] = val as i16,
            0x026 => self.bgpd[0] = val as i16,
            0x028 => {
                self.bgx_raw[0] = (self.bgx_raw[0] & !0xFFFF) | val as i32;
                self.bgx_latch[0] = sign_extend28(self.bgx_raw[0] as u32);
            }
            0x02A => {
                self.bgx_raw[0] = ((val as i32 & 0xFFF) << 16) | (self.bgx_raw[0] & 0xFFFF);
                self.bgx_latch[0] = sign_extend28(self.bgx_raw[0] as u32);
            }
            0x02C => {
                self.bgy_raw[0] = (self.bgy_raw[0] & !0xFFFF) | val as i32;
                self.bgy_latch[0] = sign_extend28(self.bgy_raw[0] as u32);
            }
            0x02E => {
                self.bgy_raw[0] = ((val as i32 & 0xFFF) << 16) | (self.bgy_raw[0] & 0xFFFF);
                self.bgy_latch[0] = sign_extend28(self.bgy_raw[0] as u32);
            }
            0x030 => self.bgpa[1] = val as i16,
            0x032 => self.bgpb[1] = val as i16,
            0x034 => self.bgpc[1] = val as i16,
            0x036 => self.bgpd[1] = val as i16,
            0x038 => {
                self.bgx_raw[1] = (self.bgx_raw[1] & !0xFFFF) | val as i32;
                self.bgx_latch[1] = sign_extend28(self.bgx_raw[1] as u32);
            }
            0x03A => {
                self.bgx_raw[1] = ((val as i32 & 0xFFF) << 16) | (self.bgx_raw[1] & 0xFFFF);
                self.bgx_latch[1] = sign_extend28(self.bgx_raw[1] as u32);
            }
            0x03C => {
                self.bgy_raw[1] = (self.bgy_raw[1] & !0xFFFF) | val as i32;
                self.bgy_latch[1] = sign_extend28(self.bgy_raw[1] as u32);
            }
            0x03E => {
                self.bgy_raw[1] = ((val as i32 & 0xFFF) << 16) | (self.bgy_raw[1] & 0xFFFF);
                self.bgy_latch[1] = sign_extend28(self.bgy_raw[1] as u32);
            }
            0x040 => self.winh[0] = val,
            0x042 => self.winh[1] = val,
            0x044 => self.winv[0] = val,
            0x046 => self.winv[1] = val,
            0x048 => self.winin = val,
            0x04A => self.winout = val,
            0x04C => self.mosaic = val,
            0x050 => self.bldcnt = val,
            0x052 => self.bldalpha = val,
            0x054 => self.bldy = val,

            // Sound
            0x060 => self.apu_write_ch1_sweep(val),
            0x062 => self.apu_write_ch1_duty(val),
            0x064 => self.apu_write_ch1_freq(val),
            0x068 => self.apu_write_ch2_duty(val),
            0x06C => self.apu_write_ch2_freq(val),
            0x070 => self.apu_write_ch3_select(val),
            0x072 => self.apu_write_ch3_length(val),
            0x074 => self.apu_write_ch3_freq(val),
            0x078 => self.apu_write_ch4_length(val),
            0x07C => self.apu_write_ch4_freq(val),
            0x080 => self.soundcnt_l = val,
            0x082 => self.apu_write_soundcnt_h(val),
            0x084 => self.apu_write_soundcnt_x(val),
            0x088 => self.soundbias = val,
            0x090..=0x09E => {
                let idx = (offset - 0x090) as usize;
                self.wave_ram[idx] = val as u8;
                self.wave_ram[idx + 1] = (val >> 8) as u8;
            }
            0x0A0 => {  // FIFO_A low
                self.fifo_push_a(val as i8);
                self.fifo_push_a((val >> 8) as i8);
            }
            0x0A2 => {  // FIFO_A high
                self.fifo_push_a(val as i8);
                self.fifo_push_a((val >> 8) as i8);
            }
            0x0A4 => {  // FIFO_B low
                self.fifo_push_b(val as i8);
                self.fifo_push_b((val >> 8) as i8);
            }
            0x0A6 => {  // FIFO_B high
                self.fifo_push_b(val as i8);
                self.fifo_push_b((val >> 8) as i8);
            }

            // DMA
            0x0B0 => self.dma[0].src_raw = (self.dma[0].src_raw & 0xFFFF0000) | val as u32,
            0x0B2 => self.dma[0].src_raw = (self.dma[0].src_raw & 0x0000FFFF) | ((val as u32) << 16),
            0x0B4 => self.dma[0].dst_raw = (self.dma[0].dst_raw & 0xFFFF0000) | val as u32,
            0x0B6 => self.dma[0].dst_raw = (self.dma[0].dst_raw & 0x0000FFFF) | ((val as u32) << 16),
            0x0B8 => self.dma[0].cnt_raw = val,
            0x0BA => self.dma_write_ctrl(0, val),

            0x0BC => self.dma[1].src_raw = (self.dma[1].src_raw & 0xFFFF0000) | val as u32,
            0x0BE => self.dma[1].src_raw = (self.dma[1].src_raw & 0x0000FFFF) | ((val as u32) << 16),
            0x0C0 => self.dma[1].dst_raw = (self.dma[1].dst_raw & 0xFFFF0000) | val as u32,
            0x0C2 => self.dma[1].dst_raw = (self.dma[1].dst_raw & 0x0000FFFF) | ((val as u32) << 16),
            0x0C4 => self.dma[1].cnt_raw = val,
            0x0C6 => self.dma_write_ctrl(1, val),

            0x0C8 => self.dma[2].src_raw = (self.dma[2].src_raw & 0xFFFF0000) | val as u32,
            0x0CA => self.dma[2].src_raw = (self.dma[2].src_raw & 0x0000FFFF) | ((val as u32) << 16),
            0x0CC => self.dma[2].dst_raw = (self.dma[2].dst_raw & 0xFFFF0000) | val as u32,
            0x0CE => self.dma[2].dst_raw = (self.dma[2].dst_raw & 0x0000FFFF) | ((val as u32) << 16),
            0x0D0 => self.dma[2].cnt_raw = val,
            0x0D2 => self.dma_write_ctrl(2, val),

            0x0D4 => self.dma[3].src_raw = (self.dma[3].src_raw & 0xFFFF0000) | val as u32,
            0x0D6 => self.dma[3].src_raw = (self.dma[3].src_raw & 0x0000FFFF) | ((val as u32) << 16),
            0x0D8 => self.dma[3].dst_raw = (self.dma[3].dst_raw & 0xFFFF0000) | val as u32,
            0x0DA => self.dma[3].dst_raw = (self.dma[3].dst_raw & 0x0000FFFF) | ((val as u32) << 16),
            0x0DC => self.dma[3].cnt_raw = val,
            0x0DE => self.dma_write_ctrl(3, val),

            // Timers
            0x100 => self.timers[0].reload = val,
            0x102 => self.timer_write_ctrl(0, val),
            0x104 => self.timers[1].reload = val,
            0x106 => self.timer_write_ctrl(1, val),
            0x108 => self.timers[2].reload = val,
            0x10A => self.timer_write_ctrl(2, val),
            0x10C => self.timers[3].reload = val,
            0x10E => self.timer_write_ctrl(3, val),

            // Serial (stub)
            0x120..=0x12E => {}
            0x134 => {}
            0x140 => {}
            0x150..=0x15A => {}

            // Keypad
            0x130 => {}  // KEYINPUT read-only
            0x132 => self.keycnt = val,

            // Interrupts
            0x200 => self.ie = val,
            0x202 => self.if_ &= !val,  // writing 1 clears interrupt flags
            0x204 => self.waitcnt = val,
            0x208 => self.ime = (self.ime & 0xFFFF0000) | val as u32,
            0x20A => self.ime = (self.ime & 0x0000FFFF) | ((val as u32) << 16),
            0x300 => {
                self.postflg = val as u8;
                if (val >> 15) & 1 != 0 {
                    self.stopped = true;
                }
            }
            0x301 => {
                // HALTCNT
                if val & 0x80 == 0 {
                    self.halted = true;
                } else {
                    self.stopped = true;
                }
            }

            _ => {}
        }
    }

    pub(crate) fn io_write32(&mut self, offset: u32, val: u32) {
        // Special case for FIFO writes (must write all 4 bytes)
        match offset {
            0x0A0 => {
                self.fifo_push_a(val as i8);
                self.fifo_push_a((val >> 8) as i8);
                self.fifo_push_a((val >> 16) as i8);
                self.fifo_push_a((val >> 24) as i8);
            }
            0x0A4 => {
                self.fifo_push_b(val as i8);
                self.fifo_push_b((val >> 8) as i8);
                self.fifo_push_b((val >> 16) as i8);
                self.fifo_push_b((val >> 24) as i8);
            }
            _ => {
                self.io_write16(offset, val as u16);
                self.io_write16(offset + 2, (val >> 16) as u16);
            }
        }
    }

    // Direct VRAM/palette/OAM access (faster path, no banking)
    pub(crate) fn vram_read8(&self, addr: usize) -> u8 {
        let off = addr & 0x1FFFF;
        let off = if off >= 0x18000 { off - 0x8000 } else { off };
        self.vram[off]
    }

    pub(crate) fn vram_read16(&self, addr: usize) -> u16 {
        let addr = addr & !1;
        let lo = self.vram_read8(addr) as u16;
        let hi = self.vram_read8(addr + 1) as u16;
        lo | (hi << 8)
    }

    pub(crate) fn palette_read16(&self, addr: usize) -> u16 {
        let addr = addr & !1;
        let lo = self.palette[addr & 0x3FF] as u16;
        let hi = self.palette[(addr + 1) & 0x3FF] as u16;
        lo | (hi << 8)
    }

    pub(crate) fn oam_read16(&self, addr: usize) -> u16 {
        let addr = addr & !1;
        let lo = self.oam[addr & 0x3FF] as u16;
        let hi = self.oam[(addr + 1) & 0x3FF] as u16;
        lo | (hi << 8)
    }

    pub(crate) fn fifo_push_a(&mut self, sample: i8) {
        if self.fifo_a_len < 32 {
            self.fifo_a[self.fifo_a_wr] = sample;
            self.fifo_a_wr = (self.fifo_a_wr + 1) & 31;
            self.fifo_a_len += 1;
        }
    }

    pub(crate) fn fifo_push_b(&mut self, sample: i8) {
        if self.fifo_b_len < 32 {
            self.fifo_b[self.fifo_b_wr] = sample;
            self.fifo_b_wr = (self.fifo_b_wr + 1) & 31;
            self.fifo_b_len += 1;
        }
    }

    pub(crate) fn fifo_pop_a(&mut self) -> i8 {
        if self.fifo_a_len > 0 {
            let s = self.fifo_a[self.fifo_a_rd];
            self.fifo_a_rd = (self.fifo_a_rd + 1) & 31;
            self.fifo_a_len -= 1;
            s
        } else {
            self.fifo_a_sample
        }
    }

    pub(crate) fn fifo_pop_b(&mut self) -> i8 {
        if self.fifo_b_len > 0 {
            let s = self.fifo_b[self.fifo_b_rd];
            self.fifo_b_rd = (self.fifo_b_rd + 1) & 31;
            self.fifo_b_len -= 1;
            s
        } else {
            self.fifo_b_sample
        }
    }
}

fn sign_extend28(val: u32) -> i32 {
    let val = val & 0x0FFFFFFF;
    if val & (1 << 27) != 0 {
        (val | 0xF0000000) as i32
    } else {
        val as i32
    }
}
