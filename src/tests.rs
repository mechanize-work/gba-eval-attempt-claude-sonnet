#[cfg(test)]
mod tests {
    use crate::Gba;
    use std::fs;

    fn make_gba(rom_path: &str) -> Gba {
        let bios = fs::read("/task/spec/gba_bios_stub.bin").expect("bios");
        let rom_data = fs::read(rom_path).expect("rom");

        let mut gba = Gba::new();

        let blen = bios.len().min(gba.bios.len());
        gba.bios[..blen].copy_from_slice(&bios[..blen]);

        let rlen = rom_data.len().min(32 * 1024 * 1024);
        gba.rom.resize(rlen, 0);
        gba.rom[..rlen].copy_from_slice(&rom_data[..rlen]);

        gba.reset();
        gba
    }

    #[test]
    fn test_anguna_frames() {
        let mut gba = make_gba("/task/dev-roms/anguna.gba");

        for i in 0..10 {
            gba.run_frame();
            let fb = &gba.framebuffer;
            let unique: std::collections::HashSet<u32> = fb.iter().map(|&p| p & 0xFFFFFF).collect();
            let mut colors: Vec<u32> = unique.iter().copied().collect();
            colors.sort();
            if colors.len() <= 5 {
                println!("Frame {}: unique_colors={} colors={:?}", i, colors.len(),
                    colors.iter().map(|c| format!("#{:06X}", c)).collect::<Vec<_>>());
            } else {
                println!("Frame {}: unique_colors={}", i, colors.len());
            }
        }
    }

    #[test]
    fn test_cpu_trace() {
        let mut gba = make_gba("/task/dev-roms/anguna.gba");

        let mut trace = Vec::new();

        for _ in 0..2000 {
            let pc = gba.regs[15];
            let is_thumb = (gba.cpsr & 0x20) != 0;

            if is_thumb {
                let instr_addr = pc.wrapping_sub(4);
                let instr = u32::from(gba.mem_read16(instr_addr));
                trace.push(format!("T 0x{:08X} {:04X} sp={:08X}", instr_addr, instr, gba.regs[13]));
            } else {
                let instr_addr = pc.wrapping_sub(8);
                let instr = gba.mem_read32(instr_addr);
                trace.push(format!("A 0x{:08X} {:08X} sp={:08X}", instr_addr, instr, gba.regs[13]));
            }

            gba.cpu_step();
        }

        println!("=== First 30 instructions ===");
        for s in trace.iter().take(30) { println!("{}", s); }
        println!("=== Last 10 instructions ===");
        for s in trace.iter().rev().take(10).collect::<Vec<_>>().iter().rev() { println!("{}", s); }
        println!("PC after trace: 0x{:08X}", gba.regs[15].wrapping_sub(if (gba.cpsr & 0x20) != 0 { 4 } else { 8 }));
        println!("CPSR: 0x{:08X}", gba.cpsr);
        println!("DISPCNT: 0x{:04X}", gba.dispcnt);
    }
}
