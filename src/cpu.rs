use crate::Gba;

// CPSR bits
pub const CPSR_N: u32 = 1 << 31;
pub const CPSR_Z: u32 = 1 << 30;
pub const CPSR_C: u32 = 1 << 29;
pub const CPSR_V: u32 = 1 << 28;
pub const CPSR_I: u32 = 1 << 7;
pub const CPSR_F: u32 = 1 << 6;
pub const CPSR_T: u32 = 1 << 5;

// CPU modes
pub const MODE_USR: u32 = 0x10;
pub const MODE_FIQ: u32 = 0x11;
pub const MODE_IRQ: u32 = 0x12;
pub const MODE_SVC: u32 = 0x13;
pub const MODE_ABT: u32 = 0x17;
pub const MODE_UND: u32 = 0x1B;
pub const MODE_SYS: u32 = 0x1F;

impl Gba {
    // ===== CPU Entry Point =====
    pub(crate) fn cpu_step(&mut self) {
        if (self.cpsr & CPSR_T) != 0 {
            self.thumb_step();
        } else {
            self.arm_step();
        }
    }

    // ===== ARM mode =====
    fn arm_step(&mut self) {
        // PC is 8 ahead: fetch at PC-8
        let pc = self.regs[15].wrapping_sub(8);
        let instr = self.mem_read32(pc);
        self.regs[15] = self.regs[15].wrapping_add(4);
        self.arm_execute(instr);
    }

    fn arm_execute(&mut self, instr: u32) {
        let cond = (instr >> 28) as u8;
        if !self.check_cond(cond) { return; }

        let op = (instr >> 25) & 0x7;
        let hi = (instr >> 20) & 0xFF;
        let lo = (instr >> 4) & 0xF;

        match op {
            0b000 => {
                // Special cases first
                // BX: 0001 0010 1111 1111 1111 0001 xxxx
                if (instr & 0x0FFFFFF0) == 0x012FFF10 {
                    let rm = self.reg(instr & 0xF);
                    self.arm_bx(rm);
                    return;
                }
                // BLX register (ARMv5, not in GBA - treat as UND)
                if (instr & 0x0FFFFFF0) == 0x012FFF30 {
                    return; // ignore
                }

                if lo == 0b1001 {
                    let bits_27_23 = hi >> 3;
                    if bits_27_23 == 0 {
                        self.arm_mul(instr); return;
                    } else if bits_27_23 == 1 {
                        self.arm_mull(instr); return;
                    } else if bits_27_23 & 0x1E == 0x02 {
                        self.arm_swp(instr); return;
                    }
                    // Otherwise fall through to data processing
                }

                // Halfword/signed byte transfer
                // bit[7]=1 AND bit[4]=1 AND bits[6:5] != 00
                if (lo & 0b1001) == 0b1001 && (lo & 0b0110) != 0 {
                    self.arm_halfword(instr); return;
                }

                // PSR transfer: bits[24:23]=10, bit[20]=0
                if (hi & 0xFB) == 0x10 && (lo & 1) == 0 {
                    self.arm_psr(instr); return;
                }

                // Data processing register
                self.arm_dp(instr);
            }
            0b001 => {
                // MSR immediate: bits[24:23]=10, bit[20]=0
                if (hi & 0xFB) == 0x32 {
                    self.arm_psr(instr); return;
                }
                // Data processing immediate
                self.arm_dp(instr);
            }
            0b010 | 0b011 => {
                if op == 0b011 && (instr & (1 << 4)) != 0 {
                    // Undefined
                    self.arm_undefined(instr);
                } else {
                    self.arm_ldr_str(instr);
                }
            }
            0b100 => {
                self.arm_ldm_stm(instr);
            }
            0b101 => {
                self.arm_branch(instr);
            }
            0b110 => {
                // Coprocessor LS - NOP for GBA
            }
            0b111 => {
                if (instr >> 24) & 0xF == 0xF {
                    self.arm_swi(instr);
                }
                // else coprocessor - NOP
            }
            _ => unreachable!()
        }
    }

    fn check_cond(&self, cond: u8) -> bool {
        let n = (self.cpsr & CPSR_N) != 0;
        let z = (self.cpsr & CPSR_Z) != 0;
        let c = (self.cpsr & CPSR_C) != 0;
        let v = (self.cpsr & CPSR_V) != 0;
        match cond {
            0x0 => z,
            0x1 => !z,
            0x2 => c,
            0x3 => !c,
            0x4 => n,
            0x5 => !n,
            0x6 => v,
            0x7 => !v,
            0x8 => c && !z,
            0x9 => !c || z,
            0xA => n == v,
            0xB => n != v,
            0xC => !z && (n == v),
            0xD => z || (n != v),
            0xE => true,
            0xF => false,  // NV condition (undefined in ARMv4T, skip)
            _ => false,
        }
    }

    // Read register (R15 reads as PC, which is already offset)
    #[inline]
    pub(crate) fn reg(&self, n: u32) -> u32 {
        self.regs[n as usize]
    }

    #[inline]
    fn set_reg(&mut self, n: u32, val: u32) {
        if n == 15 {
            // Branch: set PC with pipeline
            if (self.cpsr & CPSR_T) != 0 {
                self.regs[15] = (val & !1).wrapping_add(4);
            } else {
                self.regs[15] = (val & !3).wrapping_add(8);
            }
        } else {
            self.regs[n as usize] = val;
        }
    }

    fn arm_bx(&mut self, addr: u32) {
        if addr & 1 != 0 {
            // Switch to Thumb
            self.cpsr |= CPSR_T;
            self.regs[15] = (addr & !1).wrapping_add(4);
        } else {
            self.cpsr &= !CPSR_T;
            self.regs[15] = (addr & !3).wrapping_add(8);
        }
    }

    // ===== Barrel Shifter =====
    fn shift_register(&self, rm: u32, shift_type: u32, shift_amount: u32, update_carry: bool) -> (u32, bool) {
        let val = self.reg(rm);
        let carry = (self.cpsr & CPSR_C) != 0;
        if shift_amount == 0 {
            match shift_type {
                0 => (val, carry),  // LSL #0: no shift, no carry change
                1 => (0, (val >> 31) != 0),  // LSR #32 equivalent
                2 => {
                    let sign = ((val as i32) >> 31) as u32;
                    (sign, (val >> 31) != 0)
                }  // ASR #32 equiv
                3 => {
                    // RRX (rotate right 1 with carry)
                    let new_carry = (val & 1) != 0;
                    let result = (val >> 1) | ((carry as u32) << 31);
                    (result, new_carry)
                }
                _ => unreachable!()
            }
        } else {
            self.barrel_shift(val, shift_type, shift_amount, carry)
        }
    }

    fn barrel_shift(&self, val: u32, shift_type: u32, amount: u32, carry: bool) -> (u32, bool) {
        match shift_type {
            0 => { // LSL
                if amount >= 32 {
                    let c = if amount == 32 { (val & 1) != 0 } else { false };
                    (0, c)
                } else {
                    let c = if amount > 0 { ((val >> (32 - amount)) & 1) != 0 } else { carry };
                    (val << amount, c)
                }
            }
            1 => { // LSR
                if amount >= 32 {
                    let c = if amount == 32 { (val >> 31) != 0 } else { false };
                    (0, c)
                } else {
                    let c = ((val >> (amount - 1)) & 1) != 0;
                    (val >> amount, c)
                }
            }
            2 => { // ASR
                if amount >= 32 {
                    let c = (val >> 31) != 0;
                    (((val as i32) >> 31) as u32, c)
                } else {
                    let c = ((val >> (amount - 1)) & 1) != 0;
                    (((val as i32) >> amount) as u32, c)
                }
            }
            3 => { // ROR
                let amount = amount & 31;
                if amount == 0 {
                    let c = (val >> 31) != 0;
                    (val, c)  // ROR #0 is ROR #32 = keep val, carry from bit31
                } else {
                    let c = ((val >> (amount - 1)) & 1) != 0;
                    (val.rotate_right(amount), c)
                }
            }
            _ => unreachable!()
        }
    }

    // Get operand2 for data processing (register or immediate)
    fn arm_operand2(&self, instr: u32) -> (u32, bool) {
        let carry = (self.cpsr & CPSR_C) != 0;
        if (instr >> 25) & 1 != 0 {
            // Immediate: 8-bit rotated right by 2*rotate
            let imm = instr & 0xFF;
            let rot = ((instr >> 8) & 0xF) * 2;
            if rot == 0 {
                (imm, carry)
            } else {
                let result = imm.rotate_right(rot);
                let c = (result >> 31) != 0;
                (result, c)
            }
        } else {
            // Register
            let rm = instr & 0xF;
            let shift_type = (instr >> 5) & 0x3;
            if (instr >> 4) & 1 != 0 {
                // Shift by register
                let rs = (instr >> 8) & 0xF;
                let shift_amount = self.reg(rs) & 0xFF;
                if shift_amount == 0 {
                    return (self.reg(rm), carry);
                }
                self.barrel_shift(self.reg(rm), shift_type, shift_amount, carry)
            } else {
                // Shift by immediate
                let shift_amount = (instr >> 7) & 0x1F;
                self.shift_register(rm, shift_type, shift_amount, true)
            }
        }
    }

    // ===== Data Processing =====
    fn arm_dp(&mut self, instr: u32) {
        let rn = (instr >> 16) & 0xF;
        let rd = (instr >> 12) & 0xF;
        let s = (instr >> 20) & 1 != 0;
        let opcode = (instr >> 21) & 0xF;

        let (op2, new_carry) = self.arm_operand2(instr);
        let rn_val = self.reg(rn);

        let result: u32;
        let mut carry_out = new_carry;
        let mut overflow = (self.cpsr & CPSR_V) != 0;
        let mut write_rd = true;

        match opcode {
            0x0 => { result = rn_val & op2; }  // AND
            0x1 => { result = rn_val ^ op2; }  // EOR
            0x2 => { // SUB
                let (r, c, v) = sub_with_flags(rn_val, op2, 0);
                result = r; carry_out = c; overflow = v;
            }
            0x3 => { // RSB
                let (r, c, v) = sub_with_flags(op2, rn_val, 0);
                result = r; carry_out = c; overflow = v;
            }
            0x4 => { // ADD
                let (r, c, v) = add_with_flags(rn_val, op2, 0);
                result = r; carry_out = c; overflow = v;
            }
            0x5 => { // ADC
                let cin = if (self.cpsr & CPSR_C) != 0 { 1 } else { 0 };
                let (r, c, v) = add_with_flags(rn_val, op2, cin);
                result = r; carry_out = c; overflow = v;
            }
            0x6 => { // SBC
                let cin = if (self.cpsr & CPSR_C) != 0 { 0 } else { 1 };
                let (r, c, v) = sub_with_flags(rn_val, op2, cin);
                result = r; carry_out = c; overflow = v;
            }
            0x7 => { // RSC
                let cin = if (self.cpsr & CPSR_C) != 0 { 0 } else { 1 };
                let (r, c, v) = sub_with_flags(op2, rn_val, cin);
                result = r; carry_out = c; overflow = v;
            }
            0x8 => { result = rn_val & op2; write_rd = false; }  // TST
            0x9 => { result = rn_val ^ op2; write_rd = false; }  // TEQ
            0xA => { // CMP
                let (r, c, v) = sub_with_flags(rn_val, op2, 0);
                result = r; carry_out = c; overflow = v; write_rd = false;
            }
            0xB => { // CMN
                let (r, c, v) = add_with_flags(rn_val, op2, 0);
                result = r; carry_out = c; overflow = v; write_rd = false;
            }
            0xC => { result = rn_val | op2; }  // ORR
            0xD => { result = op2; }            // MOV
            0xE => { result = rn_val & !op2; }  // BIC
            0xF => { result = !op2; }           // MVN
            _ => unreachable!()
        }

        if s {
            if rd == 15 {
                // SPSR -> CPSR when Rd=R15 and S bit set
                let spsr = self.get_spsr();
                self.set_cpsr(spsr);
            } else {
                self.update_flags_nz(result);
                if carry_out { self.cpsr |= CPSR_C; } else { self.cpsr &= !CPSR_C; }
                if overflow { self.cpsr |= CPSR_V; } else { self.cpsr &= !CPSR_V; }
            }
        }

        if write_rd {
            self.set_reg(rd, result);
        }
    }

    fn update_flags_nz(&mut self, result: u32) {
        if result == 0 { self.cpsr |= CPSR_Z; } else { self.cpsr &= !CPSR_Z; }
        if result >> 31 != 0 { self.cpsr |= CPSR_N; } else { self.cpsr &= !CPSR_N; }
    }

    // ===== Multiply =====
    fn arm_mul(&mut self, instr: u32) {
        let rd = (instr >> 16) & 0xF;
        let rn = (instr >> 12) & 0xF;  // accumulate
        let rs = (instr >> 8) & 0xF;
        let rm = instr & 0xF;
        let a = (instr >> 21) & 1 != 0;  // accumulate
        let s = (instr >> 20) & 1 != 0;  // set flags

        let result = self.reg(rm).wrapping_mul(self.reg(rs));
        let result = if a { result.wrapping_add(self.reg(rn)) } else { result };

        self.regs[rd as usize] = result;
        if s {
            self.update_flags_nz(result);
            // C flag is unpredictable, V is unchanged
        }
    }

    fn arm_mull(&mut self, instr: u32) {
        let rdhi = (instr >> 16) & 0xF;
        let rdlo = (instr >> 12) & 0xF;
        let rs = (instr >> 8) & 0xF;
        let rm = instr & 0xF;
        let u = (instr >> 22) & 1 != 0;  // 0=unsigned, 1=signed (confusingly named U in docs)
        let a = (instr >> 21) & 1 != 0;  // accumulate
        let s = (instr >> 20) & 1 != 0;

        let result: u64;
        if !u {
            // UMULL/UMLAL
            let r = (self.reg(rm) as u64).wrapping_mul(self.reg(rs) as u64);
            result = if a {
                r.wrapping_add(((self.reg(rdhi) as u64) << 32) | self.reg(rdlo) as u64)
            } else { r };
        } else {
            // SMULL/SMLAL
            let r = ((self.reg(rm) as i32) as i64).wrapping_mul((self.reg(rs) as i32) as i64) as u64;
            result = if a {
                r.wrapping_add(((self.reg(rdhi) as u64) << 32) | self.reg(rdlo) as u64)
            } else { r };
        }

        self.regs[rdlo as usize] = result as u32;
        self.regs[rdhi as usize] = (result >> 32) as u32;

        if s {
            if result == 0 { self.cpsr |= CPSR_Z; } else { self.cpsr &= !CPSR_Z; }
            if (result >> 63) != 0 { self.cpsr |= CPSR_N; } else { self.cpsr &= !CPSR_N; }
        }
    }

    // ===== SWP =====
    fn arm_swp(&mut self, instr: u32) {
        let rn = (instr >> 16) & 0xF;
        let rd = (instr >> 12) & 0xF;
        let rm = instr & 0xF;
        let byte = (instr >> 22) & 1 != 0;
        let addr = self.reg(rn);

        if byte {
            let mem = self.mem_read8(addr);
            self.mem_write8(addr, self.reg(rm) as u8);
            self.regs[rd as usize] = mem as u32;
        } else {
            let mem = self.mem_read32_rotate(addr);
            self.mem_write32(addr & !3, self.reg(rm));
            self.regs[rd as usize] = mem;
        }
    }

    // ===== Halfword/Signed Byte Transfer =====
    fn arm_halfword(&mut self, instr: u32) {
        let p = (instr >> 24) & 1 != 0;
        let u = (instr >> 23) & 1 != 0;
        let imm = (instr >> 22) & 1 != 0;
        let w = (instr >> 21) & 1 != 0;
        let l = (instr >> 20) & 1 != 0;
        let rn = (instr >> 16) & 0xF;
        let rd = (instr >> 12) & 0xF;
        let sh = (instr >> 5) & 0x3;

        let offset = if imm {
            ((instr >> 4) & 0xF0) | (instr & 0xF)
        } else {
            self.reg(instr & 0xF)
        };

        let base = self.reg(rn);
        let addr = if p {
            if u { base.wrapping_add(offset) } else { base.wrapping_sub(offset) }
        } else {
            base
        };

        if l {
            let val = match sh {
                1 => self.mem_read16(addr & !1) as u32,  // LDRH
                2 => self.mem_read8(addr) as i8 as i32 as u32,  // LDRSB
                3 => {  // LDRSH
                    if addr & 1 != 0 {
                        self.mem_read8(addr) as i8 as i32 as u32
                    } else {
                        self.mem_read16(addr) as i16 as i32 as u32
                    }
                }
                _ => 0  // sh=0 shouldn't happen for halfword
            };
            self.regs[rd as usize] = val;
        } else {
            // STRH (sh=1)
            self.mem_write16(addr & !1, self.reg(rd) as u16);
        }

        // Writeback
        if !p || w {
            let wb = if u { base.wrapping_add(offset) } else { base.wrapping_sub(offset) };
            if rn != rd || !l {
                self.regs[rn as usize] = wb;
            }
        }
    }

    // ===== Load/Store =====
    fn arm_ldr_str(&mut self, instr: u32) {
        let p = (instr >> 24) & 1 != 0;
        let u = (instr >> 23) & 1 != 0;
        let b = (instr >> 22) & 1 != 0;  // byte
        let w = (instr >> 21) & 1 != 0;  // writeback
        let l = (instr >> 20) & 1 != 0;  // load
        let rn = (instr >> 16) & 0xF;
        let rd = (instr >> 12) & 0xF;

        let offset = if (instr >> 25) & 1 == 0 {
            // Immediate offset
            instr & 0xFFF
        } else {
            // Register offset with shift
            let rm = instr & 0xF;
            let shift_type = (instr >> 5) & 0x3;
            let shift_amount = (instr >> 7) & 0x1F;
            let (shifted, _) = self.shift_register(rm, shift_type, shift_amount, true);
            shifted
        };

        let base = self.reg(rn);
        let eff_addr = if u { base.wrapping_add(offset) } else { base.wrapping_sub(offset) };
        let addr = if p { eff_addr } else { base };

        if l {
            let val = if b {
                self.mem_read8(addr) as u32
            } else {
                self.mem_read32_rotate(addr)
            };
            self.regs[rd as usize] = val;
            if rd == 15 {
                // If loading into PC, flush pipeline
                self.regs[15] = (val & !3).wrapping_add(8);
            }
        } else {
            let src = self.reg(rd);  // R15 = current PC + 12 in STR
            if b {
                self.mem_write8(addr, src as u8);
            } else {
                self.mem_write32(addr & !3, src);
            }
        }

        // Writeback
        if !p || w {
            if rn != rd || !l {
                self.regs[rn as usize] = eff_addr;
            }
        }
    }

    // ===== LDM/STM =====
    fn arm_ldm_stm(&mut self, instr: u32) {
        let p = (instr >> 24) & 1 != 0;
        let u = (instr >> 23) & 1 != 0;
        let s = (instr >> 22) & 1 != 0;  // load PSR or force user bank
        let w = (instr >> 21) & 1 != 0;  // writeback
        let l = (instr >> 20) & 1 != 0;  // load
        let rn = (instr >> 16) & 0xF;
        let rlist = instr & 0xFFFF;

        let base = self.reg(rn);
        let count = rlist.count_ones();

        let start_addr = if u {
            if p { base.wrapping_add(4) } else { base }
        } else {
            if p { base.wrapping_sub(count * 4) } else { base.wrapping_sub(count * 4).wrapping_add(4) }
        };

        let end_addr = if u {
            start_addr.wrapping_add(count * 4).wrapping_sub(4)
        } else {
            start_addr.wrapping_add(count * 4).wrapping_sub(4)
        };

        let mut addr = start_addr;

        if l {
            // LDM
            let pc_in_list = (rlist >> 15) & 1 != 0;
            for i in 0..16u32 {
                if (rlist >> i) & 1 != 0 {
                    let val = self.mem_read32(addr & !3);
                    if s && !pc_in_list {
                        // Load user mode registers
                        self.set_user_reg(i, val);
                    } else {
                        self.regs[i as usize] = val;
                        if i == 15 {
                            if s {
                                // Load PC and CPSR from SPSR
                                let spsr = self.get_spsr();
                                self.set_cpsr(spsr);
                                self.regs[15] = if (self.cpsr & CPSR_T) != 0 {
                                    (val & !1).wrapping_add(4)
                                } else {
                                    (val & !3).wrapping_add(8)
                                };
                            } else {
                                self.regs[15] = (val & !3).wrapping_add(8);
                            }
                        }
                    }
                    addr = addr.wrapping_add(4);
                }
            }
        } else {
            // STM
            for i in 0..16u32 {
                if (rlist >> i) & 1 != 0 {
                    let val = if s {
                        self.get_user_reg(i)
                    } else {
                        self.reg(i)
                    };
                    self.mem_write32(addr & !3, val);
                    addr = addr.wrapping_add(4);
                }
            }
        }

        // Writeback
        if w && (!l || (rlist >> rn) & 1 == 0) {
            let new_base = if u {
                base.wrapping_add(count * 4)
            } else {
                base.wrapping_sub(count * 4)
            };
            self.regs[rn as usize] = new_base;
        }
    }

    fn get_user_reg(&self, n: u32) -> u32 {
        let mode = self.cpsr & 0x1F;
        match n {
            0..=7 => self.regs[n as usize],
            8..=12 => {
                if mode == MODE_FIQ { self.bank_user[(n - 8) as usize] } else { self.regs[n as usize] }
            }
            13 | 14 => self.bank_user[(n - 8) as usize],
            15 => self.regs[15],
            _ => 0
        }
    }

    fn set_user_reg(&mut self, n: u32, val: u32) {
        let mode = self.cpsr & 0x1F;
        match n {
            0..=7 => self.regs[n as usize] = val,
            8..=12 => {
                if mode == MODE_FIQ { self.bank_user[(n - 8) as usize] = val; }
                else { self.regs[n as usize] = val; }
            }
            13 | 14 => self.bank_user[(n - 8) as usize] = val,
            _ => {}
        }
    }

    // ===== Branch =====
    fn arm_branch(&mut self, instr: u32) {
        let l = (instr >> 24) & 1 != 0;
        // Sign-extend 24-bit offset, shift left 2
        let offset = ((instr & 0xFFFFFF) << 2) as i32;
        let offset = (offset << 6) >> 6;  // sign extend from bit 25

        if l {
            // BL: save return address in LR
            self.regs[14] = self.regs[15].wrapping_sub(4);
        }

        let target = self.regs[15].wrapping_sub(8)  // current PC
            .wrapping_add(8)  // pipeline
            .wrapping_add(offset as u32);  // branch offset (already accounts for pipeline since encoded as relative to PC+8)

        // Actually: target = PC + 8 + offset (where PC is already ahead by 8)
        // But we have regs[15] = current_instr + 8
        // So target = regs[15] + offset... wait let me reconsider

        // The branch offset is: target - (current_instr + 8)
        // And we want to set PC such that next fetch is from target
        // regs[15] should be target + 8
        // target = regs[15] (before this instruction advanced) - 4 (the advance we did) + offset...

        // Correction: at this point, regs[15] = current_instr + 8 + 4 (we advanced by 4 at start)
        // So current_instr = regs[15] - 12
        // target = current_instr + 8 + offset = regs[15] - 12 + 8 + offset = regs[15] - 4 + offset

        let pc = self.regs[15].wrapping_sub(4);  // regs[15] after the advance we did in arm_step
        let target = pc.wrapping_add(offset as u32);
        self.regs[15] = (target & !3).wrapping_add(8);
    }

    // ===== Software Interrupt =====
    fn arm_swi(&mut self, _instr: u32) {
        // Save return address and CPSR
        self.bank_svc[1] = self.regs[15].wrapping_sub(4);  // LR_svc = next instr
        self.spsr_svc = self.cpsr;
        // Switch to SVC mode
        self.switch_mode(MODE_SVC);
        self.cpsr &= !CPSR_T;
        self.cpsr |= CPSR_I;  // Disable IRQ
        // Jump to SWI vector
        self.regs[15] = 0x08;  // SWI vector
        // Pipeline effect
        self.regs[15] = self.regs[15].wrapping_add(8);
    }

    fn arm_undefined(&mut self, _instr: u32) {
        self.bank_und[1] = self.regs[15].wrapping_sub(4);
        self.spsr_und = self.cpsr;
        self.switch_mode(MODE_UND);
        self.cpsr &= !CPSR_T;
        self.cpsr |= CPSR_I;
        self.regs[15] = 0x04 + 8;
    }

    // ===== PSR Transfer =====
    fn arm_psr(&mut self, instr: u32) {
        let r = (instr >> 22) & 1 != 0;  // 0=CPSR, 1=SPSR

        if (instr >> 21) & 1 != 0 {
            // MSR
            let imm = (instr >> 25) & 1 != 0;
            let op2 = if imm {
                let imm = instr & 0xFF;
                let rot = ((instr >> 8) & 0xF) * 2;
                imm.rotate_right(rot)
            } else {
                self.reg(instr & 0xF)
            };

            let mask = {
                let mut m = 0u32;
                if (instr >> 16) & 1 != 0 { m |= 0x000000FF; }  // c
                if (instr >> 17) & 1 != 0 { m |= 0x0000FF00; }  // x
                if (instr >> 18) & 1 != 0 { m |= 0x00FF0000; }  // s
                if (instr >> 19) & 1 != 0 { m |= 0xFF000000; }  // f
                m
            };

            if r {
                let spsr = self.get_spsr();
                let new_spsr = (spsr & !mask) | (op2 & mask);
                self.set_spsr(new_spsr);
            } else {
                let new_cpsr = (self.cpsr & !mask) | (op2 & mask);
                self.set_cpsr(new_cpsr);
            }
        } else {
            // MRS
            let rd = (instr >> 12) & 0xF;
            let val = if r { self.get_spsr() } else { self.cpsr };
            self.regs[rd as usize] = val;
        }
    }

    // ===== Mode switching =====
    pub(crate) fn switch_mode(&mut self, new_mode: u32) {
        let old_mode = self.cpsr & 0x1F;
        if old_mode == new_mode { return; }

        // Save current registers to old bank
        self.save_banked(old_mode);

        // Update mode bits
        self.cpsr = (self.cpsr & !0x1F) | new_mode;

        // Restore new bank
        self.restore_banked(new_mode);
    }

    fn save_banked(&mut self, mode: u32) {
        match mode {
            MODE_FIQ => {
                for i in 0..7 { self.bank_fiq[i] = self.regs[8 + i]; }
            }
            MODE_IRQ => {
                self.bank_irq[0] = self.regs[13];
                self.bank_irq[1] = self.regs[14];
            }
            MODE_SVC => {
                self.bank_svc[0] = self.regs[13];
                self.bank_svc[1] = self.regs[14];
            }
            MODE_ABT => {
                self.bank_abt[0] = self.regs[13];
                self.bank_abt[1] = self.regs[14];
            }
            MODE_UND => {
                self.bank_und[0] = self.regs[13];
                self.bank_und[1] = self.regs[14];
            }
            MODE_USR | MODE_SYS => {
                for i in 0..7 { self.bank_user[i] = self.regs[8 + i]; }
            }
            _ => {}
        }
    }

    fn restore_banked(&mut self, mode: u32) {
        match mode {
            MODE_FIQ => {
                // Save user R8-R14 and load FIQ R8-R14
                for i in 0..7 { self.bank_user[i] = self.regs[8 + i]; }
                for i in 0..7 { self.regs[8 + i] = self.bank_fiq[i]; }
            }
            MODE_IRQ => {
                // Restore user R8-R12 if coming from FIQ
                self.ensure_user_r8_r12();
                self.regs[13] = self.bank_irq[0];
                self.regs[14] = self.bank_irq[1];
            }
            MODE_SVC => {
                self.ensure_user_r8_r12();
                self.regs[13] = self.bank_svc[0];
                self.regs[14] = self.bank_svc[1];
            }
            MODE_ABT => {
                self.ensure_user_r8_r12();
                self.regs[13] = self.bank_abt[0];
                self.regs[14] = self.bank_abt[1];
            }
            MODE_UND => {
                self.ensure_user_r8_r12();
                self.regs[13] = self.bank_und[0];
                self.regs[14] = self.bank_und[1];
            }
            MODE_USR | MODE_SYS => {
                // Restore user banks
                for i in 0..7 { self.regs[8 + i] = self.bank_user[i]; }
            }
            _ => {}
        }
    }

    fn ensure_user_r8_r12(&mut self) {
        // If coming from FIQ mode, restore user R8-R12
        // This is already handled by save_banked for FIQ
        // For non-FIQ modes, bank_user[0..5] already has R8-R12
        // We just need to make sure regs[8..13] has user values
        // Actually save_banked for non-FIQ modes saves R8-R14 to bank_user,
        // but for non-FIQ modes R8-R12 are shared with user mode.
        // Let me just restore from bank_user for R8-R12
        for i in 0..5 { self.regs[8 + i] = self.bank_user[i]; }
    }

    pub(crate) fn get_spsr(&self) -> u32 {
        match self.cpsr & 0x1F {
            MODE_FIQ => self.spsr_fiq,
            MODE_IRQ => self.spsr_irq,
            MODE_SVC => self.spsr_svc,
            MODE_ABT => self.spsr_abt,
            MODE_UND => self.spsr_und,
            _ => self.cpsr,  // User/System has no SPSR
        }
    }

    fn set_spsr(&mut self, val: u32) {
        match self.cpsr & 0x1F {
            MODE_FIQ => self.spsr_fiq = val,
            MODE_IRQ => self.spsr_irq = val,
            MODE_SVC => self.spsr_svc = val,
            MODE_ABT => self.spsr_abt = val,
            MODE_UND => self.spsr_und = val,
            _ => {} // User/System: NOP
        }
    }

    pub(crate) fn set_cpsr(&mut self, val: u32) {
        let new_mode = val & 0x1F;
        let old_mode = self.cpsr & 0x1F;
        if new_mode != old_mode {
            self.save_banked(old_mode);
            self.cpsr = val;
            self.restore_banked(new_mode);
        } else {
            self.cpsr = val;
        }
    }

    // ===== IRQ handling =====
    pub(crate) fn cpu_do_irq(&mut self) {
        if (self.cpsr & CPSR_I) != 0 { return; }
        // Save return address (current PC - 4... actually for ARM:)
        // IRQ return address: LR_irq = instruction after the one that was interrupted + 4
        // Since we're at the "next instruction" which is regs[15] - 4 (after arm_step advance),
        // the interrupt return address is regs[15] - 4 + 4 = regs[15]...
        // Wait: for ARM mode, IRQ return: LR = PC - 4 (where PC = instruction + 8 at time of IRQ)
        // So LR = regs[15] - 4 + 4... Actually standard ARM IRQ return:
        // SUBS PC, LR, #4 (which means execution continues from the interrupted instruction)
        // So LR = PC (the +8 ahead value) - 4 = current executing instruction + 4...
        // Simpler: LR_irq = regs[15] - 4 (since regs[15] is already the next instruction + 4 after step)
        // Hmm. Let me just use the standard: LR_irq = PC of next instruction after interrupt

        let pc = self.regs[15];  // This is instruction_addr + 8 + 4 (after advance)
        // For IRQ: LR_irq = address of instruction that would have executed + 4
        // = (pc - 4) is address of next instruction that was about to execute
        // IRQ standard: LR = PC - 4 (so SUBS PC, LR, #4 returns to interrupted instruction)
        // = (pc - 4) - 4 = pc - 8? No...

        // OK standard definition: when IRQ is taken:
        // In ARM mode: LR_irq = address of next instruction to execute (the one that was interrupted)
        // Return is: SUBS PC, LR_irq, #4 to re-execute the instruction
        // But actually GBA games use: SUBS PC, LR, #4
        // So LR_irq should be = address_of_instruction_that_would_execute + 4

        // Let me use: LR_irq = PC (i.e., regs[15] - 4)
        // When interrupt happens, CPU was about to execute instruction at regs[15] - 8 (after arm_step advance)
        // Wait, after arm_step: regs[15] = old_pc + 4 (where old_pc was instruction + 8)
        // So current instruction was at: regs[15] - 4 - 8 = regs[15] - 12

        // For GBA IRQ handling (from GBATEK):
        // LR = PC (pointing 2 instructions ahead in ARM mode = current + 8)
        // Well actually: The ARM spec says for IRQ:
        // LR_irq = address of next instruction + 4
        // = (instruction_being_executed) + 4 + 4 = instruction + 8
        // But we want return to be MOVS PC, LR (or SUBS PC, LR, #4)
        // GBA BIOS uses: STMFD SP!, {R0-R3, R12, LR}; LDR R0, =IE_addr; ... SUBS PC, LR, #4

        // Let's use: LR_irq = regs[15] - 4 (which at this point = instruction + 8)
        // This is wrong but let me check: after arm_step ran, regs[15] was incremented by 4
        // So regs[15] = instruction_addr + 8 + 4 = instruction_addr + 12
        // LR = regs[15] - 4 = instruction_addr + 8... hmm

        // For Thumb mode: LR_irq = PC (2 instructions ahead) = current_thumb_instr + 4
        // After thumb_step: regs[15] = thumb_instr + 4 + 2 = thumb_instr + 6
        // LR = regs[15] - 2 = thumb_instr + 4

        // The standard GBA approach:
        // For ARM: LR_irq = PC + 4 (where PC = instruction + 8) = instruction + 12
        // ... No, for ARM: interrupted instruction is at addr A. Next to execute is A+4.
        // LR_irq = A + 4 + 4 = A + 8. Return: SUBS PC, LR, #4 → executes from A+4. OK.
        // But: regs[15] after arm_step = A + 8 + 4 = A + 12
        // So LR_irq = regs[15] - 4 = A + 8. This seems right!

        self.bank_irq[1] = if (self.cpsr & CPSR_T) != 0 {
            self.regs[15] - 2 + 4  // Thumb: LR = next_thumb_instr + 4
        } else {
            self.regs[15] - 4  // ARM: LR = A + 8
        };
        self.spsr_irq = self.cpsr;
        self.save_banked(self.cpsr & 0x1F);
        self.cpsr = (self.cpsr & !0x3F) | MODE_IRQ | CPSR_I;
        self.regs[13] = self.bank_irq[0];
        self.regs[14] = self.bank_irq[1];
        // Jump to IRQ vector
        self.regs[15] = 0x18 + 8;  // IRQ vector + pipeline
        self.halted = false;
    }

    // ===== THUMB MODE =====
    fn thumb_step(&mut self) {
        let pc = self.regs[15].wrapping_sub(4);
        let instr = self.mem_read16(pc) as u32;
        self.regs[15] = self.regs[15].wrapping_add(2);
        self.thumb_execute(instr);
    }

    fn thumb_execute(&mut self, instr: u32) {
        match instr >> 13 {
            0b000 => {
                // Format 1/2: Move shifted register / Add-subtract
                let op = (instr >> 11) & 0x3;
                if op == 3 {
                    // Format 2: Add/subtract
                    self.thumb_add_sub(instr);
                } else {
                    // Format 1: Move shifted register
                    self.thumb_shift(instr, op);
                }
            }
            0b001 => {
                // Format 3: Move/compare/add/subtract immediate
                self.thumb_dp_imm(instr);
            }
            0b010 => {
                let op5 = (instr >> 10) & 0x7;
                if op5 == 0b000 {
                    // Format 4: ALU operations
                    self.thumb_alu(instr);
                } else if op5 == 0b001 {
                    // Format 5: Hi reg operations / BX
                    self.thumb_hi_reg(instr);
                } else {
                    // Format 6: PC-relative load / Format 7/8: Load/store
                    let bit12 = (instr >> 12) & 1;
                    if bit12 == 0 {
                        // Format 6: PC-relative load
                        self.thumb_pc_rel_load(instr);
                    } else {
                        // Load/store with register/immediate offset
                        self.thumb_load_store(instr);
                    }
                }
            }
            0b011 => {
                // Format 9: Load/store with immediate offset (word/byte)
                self.thumb_load_store_imm(instr);
            }
            0b100 => {
                let bit12 = (instr >> 12) & 1;
                if bit12 == 0 {
                    // Format 10: Load/store halfword
                    self.thumb_load_store_half(instr);
                } else {
                    // Format 11: SP-relative load/store
                    self.thumb_sp_rel_load_store(instr);
                }
            }
            0b101 => {
                let bit12 = (instr >> 12) & 1;
                if bit12 == 0 {
                    // Format 12: Load address
                    self.thumb_load_address(instr);
                } else {
                    // Format 13/14: Add offset to SP / Push/Pop
                    if (instr >> 10) & 1 == 0 {
                        self.thumb_add_sp(instr);
                    } else {
                        self.thumb_push_pop(instr);
                    }
                }
            }
            0b110 => {
                let bit12 = (instr >> 12) & 1;
                if bit12 == 0 {
                    // Format 15: Multiple load/store
                    self.thumb_ldm_stm(instr);
                } else {
                    // Format 16/17: Conditional branch / SWI
                    let cond = (instr >> 8) & 0xF;
                    if cond == 0xF {
                        self.thumb_swi(instr);
                    } else if cond == 0xE {
                        // Undefined
                    } else {
                        self.thumb_cond_branch(instr);
                    }
                }
            }
            0b111 => {
                let op11 = (instr >> 11) & 0x3;
                match op11 {
                    0b00 => self.thumb_branch(instr),  // Format 18: Unconditional branch
                    0b10 => {
                        // Format 19 part 1: BL/BLX prefix (set LR)
                        let offset = (instr & 0x7FF) as i32;
                        let offset = (offset << 21) >> 21;  // sign extend 11 bits
                        self.regs[14] = self.regs[15].wrapping_add((offset << 12) as u32);
                    }
                    0b11 => {
                        // Format 19 part 2: BL suffix
                        let offset = (instr & 0x7FF) << 1;
                        let target = self.regs[14].wrapping_add(offset);
                        self.regs[14] = (self.regs[15].wrapping_sub(2)) | 1;  // LR = return addr | 1
                        self.regs[15] = (target & !1).wrapping_add(4);
                    }
                    0b01 => {
                        // BLX offset (ARMv5, not in GBA)
                    }
                    _ => {}
                }
            }
            _ => unreachable!()
        }
    }

    fn thumb_shift(&mut self, instr: u32, op: u32) {
        let rs = (instr >> 3) & 0x7;
        let rd = instr & 0x7;
        let offset = (instr >> 6) & 0x1F;
        let val = self.regs[rs as usize];
        let carry = (self.cpsr & CPSR_C) != 0;

        let (result, new_carry) = if offset == 0 {
            match op {
                0 => (val, carry),  // LSL #0
                1 => (0, (val >> 31) != 0),  // LSR #32
                2 => (((val as i32) >> 31) as u32, (val >> 31) != 0),  // ASR #32
                _ => (val, carry)
            }
        } else {
            self.barrel_shift(val, op, offset, carry)
        };

        self.regs[rd as usize] = result;
        self.update_flags_nz(result);
        if new_carry { self.cpsr |= CPSR_C; } else { self.cpsr &= !CPSR_C; }
    }

    fn thumb_add_sub(&mut self, instr: u32) {
        let i = (instr >> 10) & 1 != 0;  // immediate
        let op = (instr >> 9) & 1 != 0;  // 0=add, 1=sub
        let rn = (instr >> 6) & 0x7;
        let rs = (instr >> 3) & 0x7;
        let rd = instr & 0x7;

        let operand = if i { rn } else { self.regs[rn as usize] };
        let base = self.regs[rs as usize];

        let (result, carry, overflow) = if op {
            sub_with_flags(base, operand, 0)
        } else {
            add_with_flags(base, operand, 0)
        };

        self.regs[rd as usize] = result;
        self.update_flags_nz(result);
        if carry { self.cpsr |= CPSR_C; } else { self.cpsr &= !CPSR_C; }
        if overflow { self.cpsr |= CPSR_V; } else { self.cpsr &= !CPSR_V; }
    }

    fn thumb_dp_imm(&mut self, instr: u32) {
        let op = (instr >> 11) & 0x3;
        let rd = (instr >> 8) & 0x7;
        let imm = instr & 0xFF;

        let rval = self.regs[rd as usize];

        match op {
            0 => {  // MOV
                self.regs[rd as usize] = imm;
                self.update_flags_nz(imm);
            }
            1 => {  // CMP
                let (result, carry, overflow) = sub_with_flags(rval, imm, 0);
                self.update_flags_nz(result);
                if carry { self.cpsr |= CPSR_C; } else { self.cpsr &= !CPSR_C; }
                if overflow { self.cpsr |= CPSR_V; } else { self.cpsr &= !CPSR_V; }
            }
            2 => {  // ADD
                let (result, carry, overflow) = add_with_flags(rval, imm, 0);
                self.regs[rd as usize] = result;
                self.update_flags_nz(result);
                if carry { self.cpsr |= CPSR_C; } else { self.cpsr &= !CPSR_C; }
                if overflow { self.cpsr |= CPSR_V; } else { self.cpsr &= !CPSR_V; }
            }
            3 => {  // SUB
                let (result, carry, overflow) = sub_with_flags(rval, imm, 0);
                self.regs[rd as usize] = result;
                self.update_flags_nz(result);
                if carry { self.cpsr |= CPSR_C; } else { self.cpsr &= !CPSR_C; }
                if overflow { self.cpsr |= CPSR_V; } else { self.cpsr &= !CPSR_V; }
            }
            _ => unreachable!()
        }
    }

    fn thumb_alu(&mut self, instr: u32) {
        let op = (instr >> 6) & 0xF;
        let rs = (instr >> 3) & 0x7;
        let rd = instr & 0x7;
        let rval = self.regs[rd as usize];
        let rsval = self.regs[rs as usize];
        let carry = (self.cpsr & CPSR_C) != 0;

        let mut write = true;
        let result: u32;
        let mut new_carry = carry;
        let mut new_overflow = (self.cpsr & CPSR_V) != 0;

        match op {
            0x0 => { result = rval & rsval; }  // AND
            0x1 => { result = rval ^ rsval; }  // EOR
            0x2 => {  // LSL
                let amt = rsval & 0xFF;
                if amt == 0 { result = rval; }
                else if amt < 32 { new_carry = ((rval >> (32 - amt)) & 1) != 0; result = rval << amt; }
                else if amt == 32 { new_carry = (rval & 1) != 0; result = 0; }
                else { new_carry = false; result = 0; }
            }
            0x3 => {  // LSR
                let amt = rsval & 0xFF;
                if amt == 0 { result = rval; }
                else if amt < 32 { new_carry = ((rval >> (amt - 1)) & 1) != 0; result = rval >> amt; }
                else if amt == 32 { new_carry = (rval >> 31) != 0; result = 0; }
                else { new_carry = false; result = 0; }
            }
            0x4 => {  // ASR
                let amt = (rsval & 0xFF).min(32);
                if amt == 0 { result = rval; }
                else { new_carry = ((rval >> (amt - 1)) & 1) != 0; result = ((rval as i32) >> amt) as u32; }
            }
            0x5 => {  // ADC
                let cin = if carry { 1 } else { 0 };
                let (r, c, v) = add_with_flags(rval, rsval, cin);
                result = r; new_carry = c; new_overflow = v;
            }
            0x6 => {  // SBC
                let cin = if carry { 0 } else { 1 };
                let (r, c, v) = sub_with_flags(rval, rsval, cin);
                result = r; new_carry = c; new_overflow = v;
            }
            0x7 => {  // ROR
                let amt = rsval & 0xFF;
                if amt == 0 { result = rval; }
                else { let a = amt & 31; new_carry = ((rval >> (a - 1)) & 1) != 0; result = rval.rotate_right(a); }
            }
            0x8 => { result = rval & rsval; write = false; }  // TST
            0x9 => {  // NEG (0 - Rs)
                let (r, c, v) = sub_with_flags(0, rsval, 0);
                result = r; new_carry = c; new_overflow = v;
            }
            0xA => {  // CMP
                let (r, c, v) = sub_with_flags(rval, rsval, 0);
                result = r; new_carry = c; new_overflow = v; write = false;
            }
            0xB => {  // CMN
                let (r, c, v) = add_with_flags(rval, rsval, 0);
                result = r; new_carry = c; new_overflow = v; write = false;
            }
            0xC => { result = rval | rsval; }  // ORR
            0xD => { result = rval.wrapping_mul(rsval); }  // MUL
            0xE => { result = rval & !rsval; }  // BIC
            0xF => { result = !rsval; }  // MVN
            _ => unreachable!()
        }

        self.update_flags_nz(result);
        if new_carry { self.cpsr |= CPSR_C; } else { self.cpsr &= !CPSR_C; }
        if new_overflow { self.cpsr |= CPSR_V; } else { self.cpsr &= !CPSR_V; }
        if write { self.regs[rd as usize] = result; }
    }

    fn thumb_hi_reg(&mut self, instr: u32) {
        let op = (instr >> 8) & 0x3;
        let h1 = (instr >> 7) & 1;
        let h2 = (instr >> 6) & 1;
        let rs = ((instr >> 3) & 0x7) | (h2 << 3);
        let rd = (instr & 0x7) | (h1 << 3);

        match op {
            0 => {  // ADD (no flags)
                let result = self.reg(rd).wrapping_add(self.reg(rs));
                if rd == 15 {
                    self.regs[15] = (result & !1).wrapping_add(4);
                } else {
                    self.regs[rd as usize] = result;
                }
            }
            1 => {  // CMP
                let (result, carry, overflow) = sub_with_flags(self.reg(rd), self.reg(rs), 0);
                self.update_flags_nz(result);
                if carry { self.cpsr |= CPSR_C; } else { self.cpsr &= !CPSR_C; }
                if overflow { self.cpsr |= CPSR_V; } else { self.cpsr &= !CPSR_V; }
            }
            2 => {  // MOV (no flags)
                let val = self.reg(rs);
                if rd == 15 {
                    self.regs[15] = (val & !1).wrapping_add(4);
                } else {
                    self.regs[rd as usize] = val;
                }
            }
            3 => {  // BX / BLX
                let addr = self.reg(rs);
                if h1 != 0 {
                    // BLX: save return address
                    self.regs[14] = self.regs[15].wrapping_sub(2) | 1;
                }
                self.arm_bx(addr);
            }
            _ => unreachable!()
        }
    }

    fn thumb_pc_rel_load(&mut self, instr: u32) {
        let rd = (instr >> 8) & 0x7;
        let offset = (instr & 0xFF) << 2;
        // PC is word-aligned, + 4 (thumb pipeline)
        let pc = self.regs[15] & !2;  // word aligned
        let addr = pc.wrapping_add(offset);
        self.regs[rd as usize] = self.mem_read32(addr & !3);
    }

    fn thumb_load_store(&mut self, instr: u32) {
        // Format 7 or 8
        let bit10 = (instr >> 10) & 1;
        let bit11 = (instr >> 11) & 1;

        if bit10 == 0 {
            // Format 7: load/store with register offset
            let l = bit11 != 0;
            let b = (instr >> 10) & 1 != 0;
            let ro = (instr >> 6) & 0x7;
            let rb = (instr >> 3) & 0x7;
            let rd = instr & 0x7;
            let addr = self.regs[rb as usize].wrapping_add(self.regs[ro as usize]);
            if l {
                let val = if b { self.mem_read8(addr) as u32 } else { self.mem_read32_rotate(addr) };
                self.regs[rd as usize] = val;
            } else {
                if b { self.mem_write8(addr, self.regs[rd as usize] as u8); }
                else { self.mem_write32(addr & !3, self.regs[rd as usize]); }
            }
        } else {
            // Format 8: load/store sign-extended
            let h = bit11 != 0;
            let s = (instr >> 10) & 1 != 0;
            let ro = (instr >> 6) & 0x7;
            let rb = (instr >> 3) & 0x7;
            let rd = instr & 0x7;
            let addr = self.regs[rb as usize].wrapping_add(self.regs[ro as usize]);
            if !s && !h {
                // STRH
                self.mem_write16(addr & !1, self.regs[rd as usize] as u16);
            } else if s && !h {
                // LDRSB
                self.regs[rd as usize] = self.mem_read8(addr) as i8 as i32 as u32;
            } else if !s && h {
                // LDRH
                self.regs[rd as usize] = self.mem_read16(addr & !1) as u32;
            } else {
                // LDRSH
                if addr & 1 != 0 {
                    self.regs[rd as usize] = self.mem_read8(addr) as i8 as i32 as u32;
                } else {
                    self.regs[rd as usize] = self.mem_read16(addr & !1) as i16 as i32 as u32;
                }
            }
        }
    }

    fn thumb_load_store_imm(&mut self, instr: u32) {
        // Format 9: word/byte with immediate offset
        let b = (instr >> 12) & 1 != 0;
        let l = (instr >> 11) & 1 != 0;
        let offset5 = (instr >> 6) & 0x1F;
        let rb = (instr >> 3) & 0x7;
        let rd = instr & 0x7;

        let base = self.regs[rb as usize];
        let offset = if b { offset5 } else { offset5 << 2 };
        let addr = base.wrapping_add(offset);

        if l {
            let val = if b { self.mem_read8(addr) as u32 } else { self.mem_read32_rotate(addr) };
            self.regs[rd as usize] = val;
        } else {
            if b { self.mem_write8(addr, self.regs[rd as usize] as u8); }
            else { self.mem_write32(addr & !3, self.regs[rd as usize]); }
        }
    }

    fn thumb_load_store_half(&mut self, instr: u32) {
        // Format 10: halfword with immediate offset
        let l = (instr >> 11) & 1 != 0;
        let offset = ((instr >> 6) & 0x1F) << 1;
        let rb = (instr >> 3) & 0x7;
        let rd = instr & 0x7;
        let addr = self.regs[rb as usize].wrapping_add(offset);

        if l {
            self.regs[rd as usize] = self.mem_read16(addr & !1) as u32;
        } else {
            self.mem_write16(addr & !1, self.regs[rd as usize] as u16);
        }
    }

    fn thumb_sp_rel_load_store(&mut self, instr: u32) {
        // Format 11
        let l = (instr >> 11) & 1 != 0;
        let rd = (instr >> 8) & 0x7;
        let offset = (instr & 0xFF) << 2;
        let addr = self.regs[13].wrapping_add(offset);

        if l {
            self.regs[rd as usize] = self.mem_read32_rotate(addr);
        } else {
            self.mem_write32(addr & !3, self.regs[rd as usize]);
        }
    }

    fn thumb_load_address(&mut self, instr: u32) {
        // Format 12
        let sp = (instr >> 11) & 1 != 0;
        let rd = (instr >> 8) & 0x7;
        let offset = (instr & 0xFF) << 2;
        let base = if sp { self.regs[13] } else { self.regs[15] & !2 };
        self.regs[rd as usize] = base.wrapping_add(offset);
    }

    fn thumb_add_sp(&mut self, instr: u32) {
        // Format 13
        let neg = (instr >> 7) & 1 != 0;
        let offset = (instr & 0x7F) << 2;
        if neg { self.regs[13] = self.regs[13].wrapping_sub(offset); }
        else { self.regs[13] = self.regs[13].wrapping_add(offset); }
    }

    fn thumb_push_pop(&mut self, instr: u32) {
        // Format 14
        let l = (instr >> 11) & 1 != 0;
        let r = (instr >> 8) & 1 != 0;  // PC/LR
        let rlist = instr & 0xFF;

        if l {
            // POP
            let mut sp = self.regs[13];
            for i in 0..8u32 {
                if (rlist >> i) & 1 != 0 {
                    self.regs[i as usize] = self.mem_read32(sp & !3);
                    sp = sp.wrapping_add(4);
                }
            }
            if r {
                let pc = self.mem_read32(sp & !3);
                sp = sp.wrapping_add(4);
                self.arm_bx(pc);
            }
            self.regs[13] = sp;
        } else {
            // PUSH
            let mut sp = self.regs[13];
            if r { sp = sp.wrapping_sub(4); }
            for i in (0..8u32).rev() {
                if (rlist >> i) & 1 != 0 { sp = sp.wrapping_sub(4); }
            }
            self.regs[13] = sp;
            let mut addr = sp;
            for i in 0..8u32 {
                if (rlist >> i) & 1 != 0 {
                    self.mem_write32(addr & !3, self.regs[i as usize]);
                    addr = addr.wrapping_add(4);
                }
            }
            if r {
                self.mem_write32(addr & !3, self.regs[14]);
            }
        }
    }

    fn thumb_ldm_stm(&mut self, instr: u32) {
        // Format 15
        let l = (instr >> 11) & 1 != 0;
        let rb = (instr >> 8) & 0x7;
        let rlist = instr & 0xFF;
        let mut addr = self.regs[rb as usize];

        if l {
            for i in 0..8u32 {
                if (rlist >> i) & 1 != 0 {
                    self.regs[i as usize] = self.mem_read32(addr & !3);
                    addr = addr.wrapping_add(4);
                }
            }
        } else {
            for i in 0..8u32 {
                if (rlist >> i) & 1 != 0 {
                    self.mem_write32(addr & !3, self.regs[i as usize]);
                    addr = addr.wrapping_add(4);
                }
            }
        }
        // Writeback (always, unless base in list for LDM)
        if !l || (rlist >> rb) & 1 == 0 {
            self.regs[rb as usize] = addr;
        }
    }

    fn thumb_cond_branch(&mut self, instr: u32) {
        let cond = ((instr >> 8) & 0xF) as u8;
        if !self.check_cond(cond) { return; }
        let offset = ((instr & 0xFF) as i8 as i32) * 2;
        // regs[15] after thumb_step = thumb_instr + 6
        // target = thumb_instr + 4 + offset
        // = (regs[15] - 2) + offset
        let target = self.regs[15].wrapping_add(offset as u32);
        self.regs[15] = (target & !1).wrapping_add(4);
    }

    fn thumb_swi(&mut self, _instr: u32) {
        self.bank_svc[1] = self.regs[15].wrapping_sub(2);  // LR_svc = next thumb instr
        self.spsr_svc = self.cpsr;
        self.save_banked(self.cpsr & 0x1F);
        self.cpsr = (self.cpsr & !0x3F) | MODE_SVC | CPSR_I;
        self.cpsr &= !CPSR_T;  // Switch to ARM
        self.regs[13] = self.bank_svc[0];
        self.regs[14] = self.bank_svc[1];
        self.regs[15] = 0x08 + 8;
    }

    fn thumb_branch(&mut self, instr: u32) {
        // Unconditional branch, 11-bit offset
        let offset = (instr & 0x7FF) as i32;
        let offset = (offset << 21) >> 21;  // sign extend
        let offset = offset * 2;
        let target = self.regs[15].wrapping_add(offset as u32);
        self.regs[15] = (target & !1).wrapping_add(4);
    }
}

// Helper arithmetic functions
#[inline]
pub(crate) fn add_with_flags(a: u32, b: u32, carry: u32) -> (u32, bool, bool) {
    let result64 = a as u64 + b as u64 + carry as u64;
    let result = result64 as u32;
    let c = result64 > 0xFFFFFFFF;
    let v = (!(a ^ b) & (a ^ result)) >> 31 != 0;
    (result, c, v)
}

#[inline]
pub(crate) fn sub_with_flags(a: u32, b: u32, borrow: u32) -> (u32, bool, bool) {
    let result64 = a as u64 + !b as u64 + (1 - borrow) as u64;
    let result = result64 as u32;
    let c = result64 > 0xFFFFFFFF;  // borrow-out (inverted)
    let v = ((a ^ b) & (a ^ result)) >> 31 != 0;
    (result, c, v)
}
