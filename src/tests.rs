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

        for i in 0..30 {
            gba.run_frame();
            let fb = &gba.framebuffer;
            let unique: std::collections::HashSet<u32> = fb.iter().map(|&p| p & 0xFFFFFF).collect();
            let mut colors: Vec<u32> = unique.iter().copied().collect();
            colors.sort();
            if colors.len() <= 5 {
                println!("Frame {:2}: unique={} dispcnt=0x{:04X} pc=0x{:08X} colors={:?}", 
                    i, colors.len(), gba.dispcnt, 
                    gba.regs[15].wrapping_sub(if (gba.cpsr & 0x20) != 0 { 4 } else { 8 }),
                    colors.iter().map(|c| format!("#{:06X}", c)).collect::<Vec<_>>());
            } else {
                println!("Frame {:2}: unique={} dispcnt=0x{:04X} pc=0x{:08X}",
                    i, colors.len(), gba.dispcnt,
                    gba.regs[15].wrapping_sub(if (gba.cpsr & 0x20) != 0 { 4 } else { 8 }));
            }
        }
    }
}
