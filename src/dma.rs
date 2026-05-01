use crate::Gba;

impl Gba {
    pub(crate) fn dma_write_ctrl(&mut self, ch: usize, val: u16) {
        let old_ctrl = self.dma[ch].ctrl;
        self.dma[ch].ctrl = val;

        let was_enabled = (old_ctrl >> 15) & 1 != 0;
        let now_enabled = (val >> 15) & 1 != 0;

        if now_enabled && !was_enabled {
            // DMA channel just enabled - latch registers
            self.dma[ch].src_int = self.dma[ch].src_raw;
            self.dma[ch].dst_int = self.dma[ch].dst_raw;
            let cnt = self.dma[ch].cnt_raw as u32;
            self.dma[ch].cnt_int = if cnt == 0 {
                match ch {
                    3 => 0x10000,
                    _ => 0x4000,
                }
            } else { cnt };

            // Check start timing
            let timing = (val >> 12) & 3;
            if timing == 0 {
                // Immediate
                self.dma_pending |= 1 << ch;
            }
            // Other timings are triggered by VBlank/HBlank/Sound events
        } else if !now_enabled {
            self.dma[ch].enabled = false;
            self.dma_pending &= !(1 << ch);
        }
    }

    pub(crate) fn trigger_dma(&mut self, timing: u32) {
        // timing: 1=VBlank, 2=HBlank, 3=Sound
        for ch in 0..4usize {
            if (self.dma[ch].ctrl >> 15) & 1 == 0 { continue; }
            let ch_timing = (self.dma[ch].ctrl >> 12) & 3;
            if ch_timing as u32 == timing {
                // Re-latch for repeat DMA
                let cnt = self.dma[ch].cnt_raw as u32;
                self.dma[ch].cnt_int = if cnt == 0 {
                    match ch { 3 => 0x10000, _ => 0x4000 }
                } else { cnt };
                let dst_ctrl = (self.dma[ch].ctrl >> 5) & 3;
                if dst_ctrl != 3 {
                    // Reload dst if not increment (for non-FIFO)
                }
                self.dma_pending |= 1 << ch;
            }
        }
    }

    pub(crate) fn run_pending_dma(&mut self) {
        // Process DMA channels in priority order (0 highest)
        for ch in 0..4usize {
            if (self.dma_pending >> ch) & 1 == 0 { continue; }
            self.dma_pending &= !(1 << ch);
            self.run_dma(ch);
            // Only one DMA at a time, restart check from 0
            return;
        }
    }

    fn run_dma(&mut self, ch: usize) {
        let ctrl = self.dma[ch].ctrl;
        let enabled = (ctrl >> 15) & 1 != 0;
        if !enabled { return; }

        let width_32 = (ctrl >> 10) & 1 != 0;  // 0=16-bit, 1=32-bit
        let repeat = (ctrl >> 9) & 1 != 0;
        let src_ctrl = (ctrl >> 7) & 3;   // 0=inc, 1=dec, 2=fixed
        let dst_ctrl = (ctrl >> 5) & 3;   // 0=inc, 1=dec, 2=fixed, 3=reload
        let timing = (ctrl >> 12) & 3;
        let irq = (ctrl >> 14) & 1 != 0;

        let is_sound = timing == 3;
        let count = if is_sound {
            4  // Sound DMA always transfers 4 words (16 bytes)
        } else {
            self.dma[ch].cnt_int
        };

        let src_step: i32 = match src_ctrl {
            0 => if width_32 { 4 } else { 2 },
            1 => if width_32 { -4 } else { -2 },
            2 | 3 => 0,  // fixed
            _ => 0
        };
        let dst_step: i32 = if is_sound {
            0  // FIFO destination is fixed
        } else {
            match dst_ctrl {
                0 | 3 => if width_32 { 4 } else { 2 },
                1 => if width_32 { -4 } else { -2 },
                2 => 0,
                _ => 0
            }
        };

        let mut src = self.dma[ch].src_int;
        let mut dst = self.dma[ch].dst_int;

        for _ in 0..count {
            if width_32 || is_sound {
                let val = self.mem_read32(src & !3);
                self.mem_write32(dst & !3, val);
            } else {
                let val = self.mem_read16(src & !1);
                self.mem_write16(dst & !1, val);
            }
            src = src.wrapping_add_signed(src_step);
            dst = dst.wrapping_add_signed(dst_step);
        }

        self.dma[ch].src_int = src;
        self.dma[ch].dst_int = dst;

        if irq {
            self.if_ |= 1 << (8 + ch);
        }

        if repeat && timing != 0 {
            // Reload count for next trigger
            let cnt = self.dma[ch].cnt_raw as u32;
            self.dma[ch].cnt_int = if cnt == 0 {
                match ch { 3 => 0x10000, _ => 0x4000 }
            } else { cnt };
            // Reload dst if dst_ctrl=3
            if dst_ctrl == 3 {
                self.dma[ch].dst_int = self.dma[ch].dst_raw;
            }
        } else {
            // Disable DMA if not repeat
            self.dma[ch].ctrl &= !0x8000;
        }
    }
}
