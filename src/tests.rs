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

    #[test]
    fn test_trace_instructions() {
        let mut gba = make_gba("/task/dev-roms/anguna.gba");

        // Trace first 2000 instructions with full register dump
        for i in 0..2000usize {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });
            let instr = if is_thumb {
                gba.mem_read16(pc) as u32
            } else {
                gba.mem_read32(pc)
            };

            let mode_str = if is_thumb { "T" } else { "A" };
            if i < 100 || (i % 50 == 0) {
                println!("{:4} {} pc={:08X} instr={:08X} r0={:08X} r1={:08X} r2={:08X} r3={:08X} r13={:08X} cpsr={:08X}",
                    i, mode_str, pc, instr,
                    gba.regs[0], gba.regs[1], gba.regs[2], gba.regs[3],
                    gba.regs[13], gba.cpsr);
            }

            gba.cpu_step();
        }
    }

    #[test]
    fn test_trace_wram() {
        let mut gba = make_gba("/task/dev-roms/anguna.gba");
        
        // Run just enough cycles to get into WRAM code
        // First frame: expect copy loop then WRAM execution
        let mut dispcnt_found = false;
        
        // Run frame by frame, tracing WRAM execution
        for frame in 0..10 {
            // Run 1000 cpu steps at a time to find interesting states
            for _ in 0..2800 {
                let pc_addr = gba.regs[15].wrapping_sub(if (gba.cpsr & 0x20) != 0 { 4 } else { 8 });
                
                // Check if this is a SWI
                let is_thumb = (gba.cpsr & 0x20) != 0;
                if is_thumb {
                    let instr = gba.mem_read16(pc_addr);
                    if instr >> 8 == 0xDF {
                        println!("Frame {}: SWI #{} at 0x{:08X}", frame, instr & 0xFF, pc_addr);
                    }
                } else {
                    let instr = gba.mem_read32(pc_addr);
                    if (instr >> 24) & 0x0F == 0x0F {
                        println!("Frame {}: SWI #{:X} at 0x{:08X}", frame, instr & 0xFFFFFF, pc_addr);
                    }
                }
                
                // Track DISPCNT changes
                let old_dispcnt = gba.dispcnt;
                gba.tick_one_cycle();
                if gba.dispcnt != old_dispcnt {
                    println!("Frame {}: DISPCNT changed to 0x{:04X} at cycle {}", frame, gba.dispcnt, gba.cycles);
                    dispcnt_found = true;
                }
            }
            println!("  End of iter: frame={} pc=0x{:08X} dispcnt=0x{:04X}",
                frame, 
                gba.regs[15].wrapping_sub(if (gba.cpsr & 0x20) != 0 { 4 } else { 8 }),
                gba.dispcnt);
        }
    }

    #[test]
    fn test_swi_trace() {
        let mut gba = make_gba("/task/dev-roms/anguna.gba");
        let mut swi_count = 0;
        // Run first full frame cycle by cycle, tracing SWIs and DISPCNT
        for _ in 0..280896u32 {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });
            // Check for SWI
            let swi_num = if is_thumb {
                let instr = gba.mem_read16(pc);
                if instr >> 8 == 0xDF { Some((instr & 0xFF) as u32) } else { None }
            } else {
                let instr = gba.mem_read32(pc);
                if instr >> 24 == 0xEF { Some(instr & 0xFFFFFF) } else { None }
            };
            if let Some(n) = swi_num {
                let regs: Vec<String> = (0..4).map(|i| format!("r{}={:08X}", i, gba.regs[i])).collect();
                println!("SWI {:3} at {:08X}: {}", n, pc, regs.join(" "));
                swi_count += 1;
            }
            gba.tick_one_cycle();
        }
        println!("Total SWIs in frame 0: {}", swi_count);
        println!("After frame 0: DISPCNT={:04X} BGCNT[0]={:04X}", gba.dispcnt, gba.bgcnt[0]);
        println!("Palette[0..8]: {:?}", &gba.palette[0..16]);
    }

    #[test]
    fn dump_frames_ppm() {
        let mut gba = make_gba("/task/dev-roms/anguna.gba");
        fs::create_dir_all("/tmp/my_frames").unwrap();
        for i in 0..30 {
            gba.run_frame();
            let fb = &gba.framebuffer;
            let mut data = format!("P6\n240 160\n255\n").into_bytes();
            for &px in fb.iter() {
                let r = (px & 0xFF) as u8;
                let g = ((px >> 8) & 0xFF) as u8;
                let b = ((px >> 16) & 0xFF) as u8;
                data.push(r); data.push(g); data.push(b);
            }
            fs::write(format!("/tmp/my_frames/frame_{:05}.ppm", i), &data).unwrap();
        }
        println!("Wrote 30 frames to /tmp/my_frames/");
    }
}
