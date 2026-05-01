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
                println!("Frame {:2}: unique={} dispcnt=0x{:04X} pc=0x{:08X} bg2cnt={:04X} vofs={} hofs={}",
                    i, colors.len(), gba.dispcnt,
                    gba.regs[15].wrapping_sub(if (gba.cpsr & 0x20) != 0 { 4 } else { 8 }),
                    gba.bgcnt[2], gba.bgvofs[2], gba.bghofs[2]);
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

    fn dump_frames(rom: &str, dir: &str, num_frames: usize) {
        let mut gba = make_gba(rom);
        fs::create_dir_all(dir).unwrap();
        for i in 0..num_frames {
            gba.run_frame();
            let fb = &gba.framebuffer;
            let mut data = format!("P6\n240 160\n255\n").into_bytes();
            for &px in fb.iter() {
                let r = (px & 0xFF) as u8;
                let g = ((px >> 8) & 0xFF) as u8;
                let b = ((px >> 16) & 0xFF) as u8;
                data.push(r); data.push(g); data.push(b);
            }
            fs::write(format!("{}/frame_{:05}.ppm", dir, i), &data).unwrap();
        }
    }

    fn compare_frames(oracle_dir: &str, my_dir: &str, num_frames: usize) {
        let mut total_diffs = 0usize;
        let mut total_pixels = 0usize;
        let mut perfect = 0usize;
        for i in 0..num_frames {
            let oracle_path = format!("{}/frame_{:05}.ppm", oracle_dir, i);
            let my_path = format!("{}/frame_{:05}.ppm", my_dir, i);
            if !fs::metadata(&oracle_path).is_ok() || !fs::metadata(&my_path).is_ok() {
                continue;
            }
            let mut read_ppm = |path: &str| {
                let data = fs::read(path).unwrap();
                // Skip header (3 lines)
                let mut pos = 0;
                for _ in 0..3 { while pos < data.len() && data[pos] != b'\n' { pos += 1; } pos += 1; }
                data[pos..].to_vec()
            };
            let oracle = read_ppm(&oracle_path);
            let mine = read_ppm(&my_path);
            let pixels = oracle.len() / 3;
            let diffs: usize = (0..oracle.len().min(mine.len())).step_by(3)
                .filter(|&j| oracle[j..j+3] != mine[j..j+3]).count();
            total_diffs += diffs;
            total_pixels += pixels;
            if diffs == 0 { perfect += 1; } else {
                println!("  Frame {}: {} diffs ({:.1}%)", i, diffs, 100.0 * diffs as f32 / pixels as f32);
            }
        }
        println!("Perfect: {}/{}, accuracy: {:.2}%", perfect, num_frames,
            100.0 * (total_pixels - total_diffs) as f32 / total_pixels.max(1) as f32);
    }

    #[test]
    fn dump_frames_ppm() {
        dump_frames("/task/dev-roms/anguna.gba", "/tmp/my_frames", 30);
        println!("Wrote 30 frames to /tmp/my_frames/");
    }

    #[test]
    fn test_anguna_init_trace() {
        let mut gba = make_gba("/task/dev-roms/anguna.gba");
        // Trace the memory fill loop at 0x08000190-0x08000196
        let mut in_loop = false;
        let mut loop_count = 0u32;
        let mut last_r0 = 0u32;
        let mut last_r1 = 0u32;

        for cycle in 0..(280896u32 * 5) {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });

            // Detect entry into the fill loop (STMIA at 0x08000190)
            if is_thumb && (pc == 0x08000190 || pc == 0x08000192 || pc == 0x08000194) {
                if !in_loop {
                    in_loop = true;
                    println!("Cycle {}: Entering fill loop R0=0x{:08X} R1=0x{:08X}",
                        cycle, gba.regs[0], gba.regs[1]);
                    last_r0 = gba.regs[0];
                    last_r1 = gba.regs[1];
                }
                loop_count += 1;
            } else if in_loop {
                println!("Cycle {}: Leaving fill loop after {} iterations, R0=0x{:08X}",
                    cycle, loop_count / 3, gba.regs[0]);
                in_loop = false;
                loop_count = 0;
            }

            let old_dc = gba.dispcnt;
            gba.tick_one_cycle();
            if gba.dispcnt != old_dc {
                println!("Cycle {}: DISPCNT 0x{:04X}->0x{:04X}", cycle, old_dc, gba.dispcnt);
            }
        }
    }

    #[test]
    fn test_halt_trace() {
        // Count how many times each ROM halts (VBlankIntrWait) before clearing forced blank
        for (name, rom) in [
            ("anguna", "/task/dev-roms/anguna.gba"),
            ("meteorain", "/task/dev-roms/meteorain.gba"),
        ] {
            let mut gba = make_gba(rom);
            let mut halt_count = 0u32;
            let mut done = false;
            for cycle in 0..(280896u32 * 15) {
                let was_halted = gba.halted;
                let old_dc = gba.dispcnt;
                gba.tick_one_cycle();
                if !was_halted && gba.halted {
                    halt_count += 1;
                }
                if gba.dispcnt != old_dc && (old_dc & 0x80) != 0 {
                    println!("{}: forced blank cleared at cycle {} (frame {}), halts before: {}",
                        name, cycle, cycle/280896, halt_count);
                    done = true;
                    break;
                }
            }
            if !done {
                println!("{}: forced blank NOT cleared in 15 frames, halts: {}", name, halt_count);
            }
        }
    }

    #[test]
    fn test_waitcnt_trace() {
        for (name, rom) in [
            ("anguna", "/task/dev-roms/anguna.gba"),
            ("meteorain", "/task/dev-roms/meteorain.gba"),
            ("trogdor", "/task/dev-roms/trogdor.gba"),
            ("xniq", "/task/dev-roms/xniq.gba"),
        ] {
            let mut gba = make_gba(rom);
            print!("{}: WAITCNT=0x{:04X}", name, gba.waitcnt);
            let mut last_wc = gba.waitcnt;
            for cycle in 0..(280896u32 * 5) {
                let old = gba.waitcnt;
                gba.tick_one_cycle();
                if gba.waitcnt != old {
                    let frame = cycle / 280896;
                    print!(", [f{frame} c{cycle}] 0x{:04X}->0x{:04X}", old, gba.waitcnt);
                    last_wc = gba.waitcnt;
                }
            }
            println!(" (final: 0x{:04X})", last_wc);
        }
    }

    #[test]
    fn test_meteorain_dispcnt() {
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");
        let mut last_dispcnt = gba.dispcnt;
        println!("Initial DISPCNT: 0x{:04X}", gba.dispcnt);
        for cycle in 0..((280896u32 * 15)) {
            let old = gba.dispcnt;
            gba.tick_one_cycle();
            if gba.dispcnt != old {
                let frame = cycle / 280896;
                println!("Cycle {cycle} (frame {frame}): DISPCNT 0x{:04X} -> 0x{:04X}", old, gba.dispcnt);
                last_dispcnt = gba.dispcnt;
            }
        }
        println!("Final DISPCNT: 0x{:04X}", last_dispcnt);
    }

    #[test]
    fn test_compare_all_roms() {
        let roms = [
            ("anguna", "/task/dev-roms/anguna.gba", "/tmp/oracle_frames2", "/tmp/my_anguna"),
            ("another-world", "/task/dev-roms/another-world.gba", "/tmp/oracle_another-world", "/tmp/my_another-world"),
            ("meteorain", "/task/dev-roms/meteorain.gba", "/tmp/oracle_meteorain", "/tmp/my_meteorain"),
            ("trogdor", "/task/dev-roms/trogdor.gba", "/tmp/oracle_trogdor", "/tmp/my_trogdor"),
            ("xniq", "/task/dev-roms/xniq.gba", "/tmp/oracle_xniq", "/tmp/my_xniq"),
        ];
        for (name, rom, oracle_dir, my_dir) in &roms {
            println!("\n=== {} ===", name);
            dump_frames(rom, my_dir, 30);
            compare_frames(oracle_dir, my_dir, 30);
        }
    }

    #[test]
    fn test_bios_exec() {
        // Trace what happens in the BIOS after the first SWI 5 call
        let mut gba = make_gba("/task/dev-roms/anguna.gba");
        let mut in_swi5 = false;
        let mut bios_entry_cycle = 0u32;
        let mut step_count = 0u32;

        for cycle in 0..1000000u32 {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });

            // Detect SWI 5 call
            if !in_swi5 && is_thumb {
                let instr = gba.mem_read16(pc);
                if instr == 0xDF05 {
                    println!("Cycle {:6}: SWI 5 called at {:08X}, R0={:08X} R1={:08X}",
                        cycle, pc, gba.regs[0], gba.regs[1]);
                    in_swi5 = true;
                    bios_entry_cycle = cycle;
                    step_count = 0;
                }
            }

            if in_swi5 {
                step_count += 1;
                // Trace execution in BIOS/SWI handler
                if pc < 0x4000 && step_count <= 100 {
                    let instr = gba.mem_read32(pc);
                    println!("  BIOS {:08X}: {:08X} mode={} R2={:08X} R4={:08X} R12={:08X} cpsr={:08X}",
                        pc, instr, if is_thumb { "T" } else { "A" },
                        gba.regs[2], gba.regs[4], gba.regs[12], gba.cpsr);
                }
                // Check if we returned from BIOS
                if pc >= 0x08000000 && step_count > 5 {
                    println!("  Returned from BIOS at cycle {:6} (after {} steps), PC={:08X}",
                        cycle, step_count, pc);
                    in_swi5 = false;
                    break;  // Stop after first SWI 5 completes
                }
            }

            gba.cpu_step();
        }
    }

    #[test]
    fn test_vblank_intr() {
        // Test VBlankIntrWait behavior: trace halts, IRQ flags, and 0x03FFFFF8
        let mut gba = make_gba("/task/dev-roms/anguna.gba");
        let mut halt_count = 0u32;
        let mut irq_count = 0u32;
        let mut last_if = 0u16;
        let mut last_halted = false;
        let mut last_dispstat = 0u16;
        let mut last_ie = 0u16;

        for cycle in 0..280896u32 {
            // Track halted state changes
            if gba.halted != last_halted {
                if gba.halted {
                    halt_count += 1;
                    if halt_count <= 10 {
                        println!("Cycle {:6}: CPU HALTED #{}, IE={:04X} IF={:04X} DISPSTAT={:04X} IME={:08X}",
                            cycle, halt_count, gba.ie, gba.if_, gba.dispstat, gba.ime);
                    }
                } else if last_halted {
                    if halt_count <= 10 {
                        println!("Cycle {:6}: CPU WOKE, IE={:04X} IF={:04X}", cycle, gba.ie, gba.if_);
                    }
                }
                last_halted = gba.halted;
            }

            // Track IF changes
            if gba.if_ != last_if {
                if irq_count < 10 {
                    println!("Cycle {:6}: IF changed {:04X} -> {:04X}", cycle, last_if, gba.if_);
                }
                if gba.if_ != 0 { irq_count += 1; }
                last_if = gba.if_;
            }

            // Track DISPSTAT bit 3 (VBlank IRQ enable) and other bits
            if gba.dispstat != last_dispstat {
                println!("Cycle {:6}: DISPSTAT {:04X} -> {:04X}", cycle, last_dispstat, gba.dispstat);
                last_dispstat = gba.dispstat;
            }

            // Track IE changes
            if gba.ie != last_ie {
                println!("Cycle {:6}: IE {:04X} -> {:04X}", cycle, last_ie, gba.ie);
                last_ie = gba.ie;
            }

            gba.tick_one_cycle();
        }
        println!("Total halts: {}, IRQ events: {}", halt_count, irq_count);
        println!("Final: IE={:04X} IF={:04X} IME={:08X} halted={} DISPSTAT={:04X}",
            gba.ie, gba.if_, gba.ime, gba.halted, gba.dispstat);
        println!("0x03007FFC = {:08X}", gba.mem_read32(0x03007FFC));
        println!("0x03FFFFF8 = {:08X}", gba.mem_read32(0x03FFFFF8));
        println!("palette[0..8]: {:?}", &gba.palette[0..8]);
    }

    #[test]
    fn test_row0_debug() {
        let mut gba = make_gba("/task/dev-roms/anguna.gba");
        for _ in 0..3 { gba.run_frame(); }

        // Check VRAM state BEFORE frame 4
        let bgcnt2 = gba.bgcnt[2];
        let map_base = (((bgcnt2 >> 8) & 0x1F) as usize) * 0x800;
        println!("BEFORE frame 4: BGCNT2={:04X} map_base=0x{:X}", bgcnt2, map_base);
        println!("Row 0 map before frame 4: {:?}", &gba.vram[map_base..map_base+20]);
        // Print first non-zero map entry to see the structure
        let first_nonzero = (0..1024).find(|&i| gba.vram[map_base + i*2] != 0 || gba.vram[map_base + i*2 + 1] != 0);
        println!("First non-zero map entry at index: {:?}", first_nonzero);

        gba.run_frame();

        // Check what's in VRAM at the map and tile addresses for BG2 row 0
        let bgcnt2 = gba.bgcnt[2];
        let char_base = (((bgcnt2 >> 2) & 3) as usize) * 0x4000;
        let map_base = (((bgcnt2 >> 8) & 0x1F) as usize) * 0x800;
        println!("BGCNT2={:04X} char_base=0x{:X} map_base=0x{:X}", bgcnt2, char_base, map_base);
        println!("bghofs[2]={} bgvofs[2]={}", gba.bghofs[2], gba.bgvofs[2]);

        // Check map entries for row 0
        println!("Map entries for y=0 row (first 10 tiles):");
        for tx in 0..10usize {
            let off = map_base + (0 * 32 + tx) * 2;
            let entry = gba.vram[off] as u16 | ((gba.vram[off+1] as u16) << 8);
            let tile_num = entry & 0x3FF;
            println!("  tx={} off=0x{:X} entry={:04X} tile={}", tx, off, entry, tile_num);
        }

        // Check VRAM bytes at map_base
        println!("First 16 bytes at map_base (0x{:X}): {:?}", map_base, &gba.vram[map_base..map_base+16]);

        // Check tile 0 data
        println!("Tile 0 at char_base (0x{:X}): first 16 bytes = {:?}", char_base, &gba.vram[char_base..char_base+16]);

        // Check frame 3 pixel at (0,0)
        let fb = &gba.framebuffer;
        let px = fb[0];
        println!("Framebuffer pixel (0,0) = 0x{:08X}", px);
        println!("Framebuffer pixels 0-15 at row 0:");
        for x in 0..16 {
            let p = fb[x];
            let r = (p & 0xFF) as u8;
            let g = ((p >> 8) & 0xFF) as u8;
            let b = ((p >> 16) & 0xFF) as u8;
            print!("  x={}: #{:02X}{:02X}{:02X}", x, r, g, b);
        }
        println!();

        // Check if BGHOFS/BGVOFS is being written to by game
        println!("Scroll: hofs={} vofs={}", gba.bghofs[2], gba.bgvofs[2]);

        // Search for the mysterious 0x00E4 color in palette
        println!("Searching for GBA color 0x00E4 in all palette RAM:");
        for i in 0..512usize {
            let lo = gba.palette[i*2];
            let hi = gba.palette[i*2 + 1];
            let c = lo as u16 | ((hi as u16) << 8);
            if c == 0x00E4 {
                if i < 256 {
                    println!("  Found at BG palette[{}]", i);
                } else {
                    println!("  Found at OBJ palette[{}]", i - 256);
                }
            }
        }

        // Also check if any tile in VRAM has non-zero data that could cause row 0 pixels
        // We know map row 0 is all tile 0, and tile 0 is all zeros. BUT let's check
        // if maybe the SCROLL is non-zero somehow during rendering.
        // Print ALL BG2 registers including actual register reads
        println!("BG2 BGCNT: 0x{:04X}", gba.bgcnt[2]);
        println!("BG2 HOFS: {} VOFS: {}", gba.bghofs[2], gba.bgvofs[2]);

        // Check OBJ VRAM tile 0 (at 0x10000)
        println!("OBJ VRAM tile 0 (4bpp, at 0x10000): first 32 bytes = {:?}", &gba.vram[0x10000..0x10020]);

        // Check OAM entries 20-25
        println!("OAM entries 20-25:");
        for i in 20..26usize {
            let attr0 = gba.oam[i*8] as u16 | ((gba.oam[i*8+1] as u16) << 8);
            let attr1 = gba.oam[i*8+2] as u16 | ((gba.oam[i*8+3] as u16) << 8);
            let attr2 = gba.oam[i*8+4] as u16 | ((gba.oam[i*8+5] as u16) << 8);
            println!("  OAM[{}]: attr0={:04X} attr1={:04X} attr2={:04X}", i, attr0, attr1, attr2);
        }

        // Check the OBJ palette entry 0 for palette 0
        println!("OBJ palette entry 0 (for palette 0): {:04X}", gba.palette[0x200] as u16 | ((gba.palette[0x201] as u16) << 8));
        // Check a few OBJ palette entries
        for i in 0..16usize {
            let lo = gba.palette[0x200 + i*2];
            let hi = gba.palette[0x200 + i*2 + 1];
            let c = lo as u16 | ((hi as u16) << 8);
            if c != 0 {
                println!("  OBJ pal[{}] = {:04X}", i, c);
            }
        }
    }

    #[test]
    fn test_vram_analysis() {
        let mut gba = make_gba("/task/dev-roms/anguna.gba");
        for _ in 0..3 { gba.run_frame(); }

        // BG palette entries (all 256)
        println!("=== BG Palette after frame 3 ===");
        for i in 0..256usize {
            let lo = gba.palette[i*2];
            let hi = gba.palette[i*2 + 1];
            let c = lo as u16 | ((hi as u16) << 8);
            if c != 0 {
                let r8 = ((c & 0x1F) * 8 | (c & 0x1F) >> 2) as u8;
                let g8 = (((c >> 5) & 0x1F) * 8 | ((c >> 5) & 0x1F) >> 2) as u8;
                let b8 = (((c >> 10) & 0x1F) * 8 | ((c >> 10) & 0x1F) >> 2) as u8;
                println!("  pal[{:3}] = {:04X} = #{:02X}{:02X}{:02X}", i, c, r8, g8, b8);
            }
        }

        // Check map entries at screen base block 28 (0x0600E000)
        let map_base = 0xE000usize;
        println!("\n=== BG2 Map (screen base 28, first 30 rows) ===");
        for ty in 0..20usize {
            print!("  row {:2}: ", ty);
            for tx in 0..30usize {
                let off = map_base + (ty * 32 + tx) * 2;
                let entry = gba.vram[off] as u16 | ((gba.vram[off+1] as u16) << 8);
                let tile_num = entry & 0x3FF;
                print!("{:4} ", tile_num);
            }
            println!();
        }

        // Check first few tile pixels to see what palette indices are used
        println!("\n=== First 10 tiles' pixel palette indices ===");
        let tile_base = 0usize;
        for t in 0..10usize {
            let off = tile_base + t * 64;
            let bytes: Vec<u8> = (0..64).map(|i| gba.vram[off + i]).collect();
            let non_zero: Vec<(usize, u8)> = bytes.iter().enumerate().filter(|(_, &b)| b != 0).map(|(i, &b)| (i, b)).collect();
            if !non_zero.is_empty() {
                println!("  Tile {:3}: {:?}", t, &non_zero[..non_zero.len().min(8)]);
            }
        }
    }

    #[test]
    fn test_sprite_debug() {
        let mut gba = make_gba("/task/dev-roms/anguna.gba");
        for _ in 0..30 { gba.run_frame(); }

        // Print BG palette
        println!("=== BG Palette (non-black entries) ===");
        for i in 0..256usize {
            let lo = gba.palette[i*2];
            let hi = gba.palette[i*2 + 1];
            let c = lo as u16 | ((hi as u16) << 8);
            if c != 0 {
                let r = ((c & 0x1F) as u32 * 255 / 31) as u8;
                let g = (((c >> 5) & 0x1F) as u32 * 255 / 31) as u8;
                let b = (((c >> 10) & 0x1F) as u32 * 255 / 31) as u8;
                println!("  BG pal[{}] = {:04X} = RGB({},{},{})", i, c, r, g, b);
            }
        }

        // Print OBJ palette
        println!("=== OBJ Palette (non-black entries) ===");
        for i in 0..256usize {
            let lo = gba.palette[0x200 + i*2];
            let hi = gba.palette[0x200 + i*2 + 1];
            let c = lo as u16 | ((hi as u16) << 8);
            if c != 0 {
                let r = ((c & 0x1F) as u32 * 255 / 31) as u8;
                let g = (((c >> 5) & 0x1F) as u32 * 255 / 31) as u8;
                let b = (((c >> 10) & 0x1F) as u32 * 255 / 31) as u8;
                println!("  OBJ pal[{}] = {:04X} = RGB({},{},{})", i, c, r, g, b);
            }
        }

        // Print first 10 active sprites
        println!("=== Active OBJ Sprites (first 10) ===");
        let mut shown = 0;
        for i in 0..128usize {
            let attr0 = gba.oam[i*8] as u16 | ((gba.oam[i*8+1] as u16) << 8);
            let attr1 = gba.oam[i*8+2] as u16 | ((gba.oam[i*8+3] as u16) << 8);
            let attr2 = gba.oam[i*8+4] as u16 | ((gba.oam[i*8+5] as u16) << 8);
            let rot_scale = (attr0 >> 8) & 1 != 0;
            let disable = !rot_scale && (attr0 >> 9) & 1 != 0;
            if !disable && shown < 10 {
                let y = attr0 & 0xFF;
                let x = attr1 & 0x1FF;
                let tile = attr2 & 0x3FF;
                let pal = (attr2 >> 12) & 0xF;
                let shape = (attr0 >> 14) & 3;
                let size = (attr1 >> 14) & 3;
                let col256 = (attr0 >> 13) & 1;
                println!("  OBJ[{}]: y={} x={} tile={} pal={} shape={} size={} col256={} attr0={:04X}",
                    i, y, x, tile, pal, shape, size, col256, attr0);
                shown += 1;
            }
        }
    }

    #[test]
    fn test_multiframe_irq() {
        let mut gba = make_gba("/task/dev-roms/anguna.gba");
        let mut last_if = 0u16;
        let mut last_halted = false;
        let mut last_fffff8 = 0u16;
        let mut last_pc = 0u32;
        let mut last_ime = 0u32;

        // Run 3 frames worth of cycles
        for cycle in 0..840000u32 {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });
            let fffff8 = gba.mem_read16(0x03FFFFF8);

            // Track PC jumps to BIOS IRQ vector (0x18)
            if pc == 0x18 && last_pc != 0x18 {
                println!("Cycle {:7}: IRQ TAKEN -> BIOS 0x18, IF={:04X} IE={:04X} IME={:08X} CPSR={:08X}",
                    cycle, gba.if_, gba.ie, gba.ime, gba.cpsr);
            }

            // Track IF changes
            if gba.if_ != last_if {
                println!("Cycle {:7}: IF {:04X} -> {:04X}, halted={}, DISPSTAT={:04X}",
                    cycle, last_if, gba.if_, gba.halted, gba.dispstat);
                last_if = gba.if_;
            }

            // Track halt/wake
            if gba.halted != last_halted {
                println!("Cycle {:7}: CPU {} PC=0x{:08X} CPSR={:08X} IF={:04X} IE={:04X} 0x3FFFFF8={:04X}",
                    cycle,
                    if gba.halted { "HALTED" } else { "WOKE" },
                    pc, gba.cpsr, gba.if_, gba.ie, fffff8);
                last_halted = gba.halted;
            }

            // Track 0x03FFFFF8 changes
            if fffff8 != last_fffff8 {
                println!("Cycle {:7}: 0x3FFFFF8 {:04X} -> {:04X}", cycle, last_fffff8, fffff8);
                last_fffff8 = fffff8;
            }

            // Track IME changes
            if gba.ime != last_ime {
                println!("Cycle {:7}: IME {:08X} -> {:08X} PC=0x{:08X}",
                    cycle, last_ime, gba.ime, pc);
                last_ime = gba.ime;
            }

            last_pc = pc;
            gba.tick_one_cycle();
        }
        let fffff8 = gba.mem_read16(0x03FFFFF8);
        println!("After 3 frames: halted={} IF={:04X} IE={:04X} DISPSTAT={:04X} 0x3FFFFF8={:04X} IME={:08X}",
            gba.halted, gba.if_, gba.ie, gba.dispstat, fffff8, gba.ime);
    }

    #[test]
    fn test_dma_trace() {
        let mut gba = make_gba("/task/dev-roms/anguna.gba");
        // Trace all DMA transfers in the first frame
        let mut last_dma_ctrls = [0u16; 4];
        for cycle in 0..280896u32 {
            // Check for DMA enables
            for ch in 0..4 {
                let ctrl = gba.dma[ch].ctrl;
                let was_enabled = last_dma_ctrls[ch] >> 15 != 0;
                let now_enabled = ctrl >> 15 != 0;
                if now_enabled && !was_enabled {
                    let src = gba.dma[ch].src_raw;
                    let dst = gba.dma[ch].dst_raw;
                    let cnt = gba.dma[ch].cnt_raw;
                    let timing = (ctrl >> 12) & 3;
                    let width = if (ctrl >> 10) & 1 != 0 { 32 } else { 16 };
                    println!("Cycle {:6}: DMA{} ENABLED src=0x{:08X} dst=0x{:08X} cnt={} timing={} width={}",
                        cycle, ch, src, dst, cnt, timing, width);
                    // Show first few source words
                    let words = cnt.min(8) as u32;
                    for i in 0..words {
                        let addr = src.wrapping_add(i * (width/8));
                        let val = if width == 32 { gba.mem_read32(addr) } else { gba.mem_read16(addr) as u32 };
                        print!("  [+{}]=0x{:08X}", i, val);
                    }
                    println!();
                }
                last_dma_ctrls[ch] = ctrl;
            }
            gba.tick_one_cycle();
        }
        println!("Final palette[0..16]: {:?}", &gba.palette[0..16]);
        println!("DISPCNT={:04X} BGCNT[2]={:04X}", gba.dispcnt, gba.bgcnt[2]);
    }


    #[test]
    fn test_meteorain_io_reads() {
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");
        // Trace all I/O reads (4MHz range) before forced blank clears
        // We instrument by checking if PC is doing an LDR from I/O space

        let mut io_read_counts: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();

        for cycle in 0..(280896u32 * 15) {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });

            if is_thumb {
                let instr = gba.mem_read16(pc);
                // Format 9: LDR Rd,[Rb,#offset] (bits 15:11 = 01101)
                if (instr >> 11) == 0b01101 {
                    let rb = ((instr >> 3) & 7) as usize;
                    let offset = (((instr >> 6) & 0x1F) * 4) as u32;
                    let addr = gba.regs[rb].wrapping_add(offset);
                    if (addr >> 24) == 0x04 {
                        *io_read_counts.entry(addr).or_insert(0) += 1;
                    }
                }
                // Format 7: LDR Rd,[Rb,Ro]
                if (instr >> 9) == 0b0101100 {
                    let ro = ((instr >> 6) & 7) as usize;
                    let rb = ((instr >> 3) & 7) as usize;
                    let addr = gba.regs[rb].wrapping_add(gba.regs[ro]);
                    if (addr >> 24) == 0x04 {
                        *io_read_counts.entry(addr).or_insert(0) += 1;
                    }
                }
                // LDRH Rd,[Rb,#offset] (format 10: bits 15:11 = 10001)
                if (instr >> 11) == 0b10001 {
                    let rb = ((instr >> 3) & 7) as usize;
                    let offset = (((instr >> 6) & 0x1F) * 2) as u32;
                    let addr = gba.regs[rb].wrapping_add(offset);
                    if (addr >> 24) == 0x04 {
                        *io_read_counts.entry(addr).or_insert(0) += 1;
                    }
                }
                // LDRB Rd,[Rb,#offset] (format 9b: bits 15:11 = 01111)
                if (instr >> 11) == 0b01111 {
                    let rb = ((instr >> 3) & 7) as usize;
                    let offset = (((instr >> 6) & 0x1F)) as u32;
                    let addr = gba.regs[rb].wrapping_add(offset);
                    if (addr >> 24) == 0x04 {
                        *io_read_counts.entry(addr).or_insert(0) += 1;
                    }
                }
            }

            let old_dc = gba.dispcnt;
            gba.tick_one_cycle();
            if gba.dispcnt != old_dc && (old_dc & 0x80) != 0 {
                println!("Forced blank CLEARED at cycle {} (frame {})", cycle, cycle/280896);
                break;
            }
        }

        let mut sorted: Vec<(u32, u32)> = io_read_counts.into_iter().collect();
        sorted.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
        println!("I/O reads before forced blank clear (top 20):");
        for (addr, count) in sorted.iter().take(20) {
            let name = match addr {
                0x04000000 => "DISPCNT",
                0x04000004 => "DISPSTAT",
                0x04000006 => "VCOUNT",
                0x04000130 => "KEYINPUT",
                0x04000200 => "IE",
                0x04000202 => "IF",
                0x04000204 => "WAITCNT",
                0x04000208 => "IME",
                _ => "?",
            };
            println!("  0x{addr:08X} ({name}): {count} reads");
        }
    }

    #[test]
    fn test_meteorain_swi_trace() {
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");
        let mut swi_counts: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        for cycle in 0..(280896u32 * 15) {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });
            if is_thumb {
                let instr = gba.mem_read16(pc);
                if instr >> 8 == 0xDF {
                    let num = (instr & 0xFF) as u32;
                    let cnt = swi_counts.entry(num).or_insert(0);
                    if *cnt < 3 {
                        println!("Cycle {cycle} (frame {}): SWI #{num} at PC={pc:08X} R0={:08X} R1={:08X}",
                            cycle/280896, gba.regs[0], gba.regs[1]);
                    }
                    *cnt += 1;
                }
            } else {
                let instr = gba.mem_read32(pc);
                if (instr >> 24) & 0x0F == 0x0F {
                    let num = instr & 0xFFFFFF;
                    let cnt = swi_counts.entry(num).or_insert(0);
                    if *cnt < 3 {
                        println!("Cycle {cycle} (frame {}): ARM SWI #{num:X} at PC={pc:08X} R0={:08X}",
                            cycle/280896, gba.regs[0]);
                    }
                    *cnt += 1;
                }
            }
            let old_dc = gba.dispcnt;
            gba.tick_one_cycle();
            if gba.dispcnt != old_dc && (old_dc & 0x80) != 0 {
                println!("Forced blank CLEARED at cycle {cycle} (frame {})", cycle/280896);
                break;
            }
        }
        println!("SWI call counts:");
        let mut sorted: Vec<_> = swi_counts.iter().collect();
        sorted.sort_by_key(|&(k, _)| k);
        for (num, cnt) in sorted {
            println!("  SWI #{num}: {cnt} times");
        }
    }

    #[test]
    fn test_meteorain_dma_trace() {
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");
        let mut last_dma_ctrls = [0u16; 4];
        for cycle in 0..(280896u32 * 15) {
            for ch in 0..4 {
                let ctrl = gba.dma[ch].ctrl;
                let was_enabled = last_dma_ctrls[ch] >> 15 != 0;
                let now_enabled = ctrl >> 15 != 0;
                if now_enabled && !was_enabled {
                    let src = gba.dma[ch].src_raw;
                    let dst = gba.dma[ch].dst_raw;
                    let cnt = gba.dma[ch].cnt_raw;
                    let timing = (ctrl >> 12) & 3;
                    let width = if (ctrl >> 10) & 1 != 0 { 32 } else { 16 };
                    println!("Cycle {:7} (frame {}): DMA{} ENABLED src=0x{:08X} dst=0x{:08X} cnt={} timing={} width={}",
                        cycle, cycle/280896, ch, src, dst, cnt, timing, width);
                }
                last_dma_ctrls[ch] = ctrl;
            }
            let old_dc = gba.dispcnt;
            gba.tick_one_cycle();
            if gba.dispcnt != old_dc && (old_dc & 0x80) != 0 {
                println!("Forced blank CLEARED at cycle {} (frame {})", cycle, cycle/280896);
                break;
            }
        }
    }

    #[test]
    fn test_anguna_dispcnt_pc() {
        let mut gba = make_gba("/task/dev-roms/anguna.gba");
        let mut last_pc = 0u32;
        for cycle in 0..(280896u32 * 5) {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });
            if pc == 0x080236B4 && last_pc != 0x080236B4 {
                println!("Cycle {cycle}: anguna hits 0x080236B4 (meteorain's VRAM fill), DISPCNT={:04X}", gba.dispcnt);
            }
            let old_dc = gba.dispcnt;
            gba.tick_one_cycle();
            if gba.dispcnt != old_dc {
                let is_forced_blank = (old_dc & 0x80) != 0 && (gba.dispcnt & 0x80) == 0;
                println!("Cycle {cycle}: DISPCNT {:04X} -> {:04X} (PC was {:08X}) forced_blank_cleared={}",
                    old_dc, gba.dispcnt, pc, is_forced_blank);
                if is_forced_blank { break; }
            }
            last_pc = pc;
        }
    }

    #[test]
    fn test_anguna_init_timing() {
        let mut gba = make_gba("/task/dev-roms/anguna.gba");
        let mut fill_loop_iters = 0u64;
        let mut copy_loop_iters = 0u64;
        let mut last_pc = 0u32;
        let mut forced_blank_cleared = false;

        for cycle in 0..(280896u32 * 5) {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });

            // Fill loop at 0x08000190
            if pc == 0x08000190 && last_pc != 0x08000190 {
                if fill_loop_iters == 0 {
                    println!("Fill loop start at cycle {}: R0={:08X} R1={:08X} R2={:08X} (DISPCNT={:04X})",
                        cycle, gba.regs[0], gba.regs[1], gba.regs[2], gba.dispcnt);
                }
                fill_loop_iters += 1;
            }

            // Copy loop at 0x080001A4
            if pc == 0x080001A4 && last_pc != 0x080001A4 {
                if copy_loop_iters == 0 {
                    println!("Copy loop start at cycle {}: R1={:08X} R2={:08X} R3={:08X} (DISPCNT={:04X})",
                        cycle, gba.regs[1], gba.regs[2], gba.regs[3], gba.dispcnt);
                }
                copy_loop_iters += 1;
            }

            let old_dc = gba.dispcnt;
            gba.tick_one_cycle();
            if gba.dispcnt != old_dc {
                println!("Cycle {}: DISPCNT {:04X} -> {:04X} (frame {}, fill_iters={}, copy_iters={})",
                    cycle, old_dc, gba.dispcnt, cycle/280896, fill_loop_iters, copy_loop_iters);
                if (old_dc & 0x80) != 0 {
                    forced_blank_cleared = true;
                    break;
                }
            }
            last_pc = pc;
        }
        println!("Fill loop total: {fill_loop_iters}, copy loop total: {copy_loop_iters}");
    }

    #[test]
    fn test_meteorain_comprehensive_trace() {
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");
        let mut total_instructions = 0u64;
        let mut loop_pc_counts: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
        let mut dispcnt_writes = 0u32;

        for cycle in 0..(280896u32 * 15) {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });

            // Count instruction visits (approximation: count by cycle change would miss stalls)
            // Instead, let's use a different approach: count BNE back-branches as loop iterations
            if is_thumb {
                let instr = gba.mem_read16(pc);
                // BNE: 0b11010001 prefix
                if (instr >> 8) == 0b11010001 {
                    let off = (instr & 0xFF) as i8 as i32;
                    if off < 0 { // backward branch
                        *loop_pc_counts.entry(pc).or_insert(0) += 1;
                    }
                }
                // B backward: 0b11100 prefix (format 18)
                if (instr >> 11) == 0b11100 {
                    let off = (instr & 0x7FF) as i32;
                    let off = if off & 0x400 != 0 { off - 0x800 } else { off };
                    if off < 0 {
                        *loop_pc_counts.entry(pc | 0x80000000).or_insert(0) += 1;
                    }
                }
            }

            let old_dc = gba.dispcnt;
            gba.tick_one_cycle();
            if gba.dispcnt != old_dc {
                println!("Cycle {cycle} (frame {}): DISPCNT {:04X} -> {:04X}",
                    cycle/280896, old_dc, gba.dispcnt);
                if (old_dc & 0x80) != 0 {
                    println!("Forced blank CLEARED at cycle {cycle}!");
                    break;
                }
            }
        }

        println!("Top backward branches (loop heads):");
        let mut sorted: Vec<(u32, u64)> = loop_pc_counts.into_iter().collect();
        sorted.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
        for (pc, count) in sorted.iter().take(20) {
            let real_pc = *pc & !0x80000000;
            let btype = if *pc & 0x80000000 != 0 { "B" } else { "BNE" };
            println!("  PC=0x{real_pc:08X} ({btype}): {count} iterations");
        }
    }

    #[test]
    fn test_meteorain_pre_clear_trace() {
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");
        let mut last_dispcnt = gba.dispcnt;

        // Run to just before cycle 1,200,000 quickly
        for _ in 0..1200000u32 {
            gba.tick_one_cycle();
        }

        println!("At cycle 1200000: PC={:08X} DISPCNT={:04X} halted={}",
            gba.regs[15].wrapping_sub(if (gba.cpsr & 0x20) != 0 { 4 } else { 8 }),
            gba.dispcnt, gba.halted);

        // Now trace closely until forced blank clears
        for cycle in 1200000u32..1300000 {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });

            // Print occasional PC to see what code is running
            if cycle % 10000 == 0 {
                println!("Cycle {cycle}: PC={pc:08X} halted={} DISPCNT={:04X} VCOUNT={}",
                    gba.halted, gba.dispcnt, gba.vcount);
            }

            let old_dc = gba.dispcnt;
            gba.tick_one_cycle();
            if gba.dispcnt != old_dc {
                println!("Cycle {cycle}: DISPCNT 0x{old_dc:04X} -> 0x{:04X} (frame {}), PC at time={pc:08X}",
                    gba.dispcnt, cycle/280896);
                if (old_dc & 0x80) != 0 {
                    println!("Forced blank CLEARED at cycle {cycle}!");
                    break;
                }
            }
        }
    }

    #[test]
    fn test_meteorain_vcount_trace() {
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");
        let mut last_vcount_read_pc = 0u32;
        let mut vcount_reads = 0u64;
        let mut last_dispcnt = gba.dispcnt;

        // Instrument io_read16 equivalent: we need to trace VCOUNT (0x04000006) reads
        // Strategy: run cycle by cycle and check VCOUNT reads by checking PC and instructions
        for cycle in 0..(280896u32 * 15) {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });

            // Check if this instruction is an LDR/LDRH/LDRB from VCOUNT address 0x04000006
            // In Thumb: LDR Rd, [Rb, Ro] (format 7) or LDR Rd, [Rb, #offset]
            // In ARM: LDR Rd, [Rb, #offset]
            // The game typically loads base into register then LDR from [Rb, #6]
            // Check if PC is in the "poll VBLANK" region (after copy loop)
            // We'll watch VCOUNT by checking if a read from 0x04000006 is happening
            // by detecting if the PC is in a busy-wait pattern

            if is_thumb {
                let instr = gba.mem_read16(pc);
                // Look for: LDR Rd, [Rb, #offset] (format 9: 0110 1 xxx xxxx xxxx)
                // Offset field is bits [10:6], Rb=[5:3], Rd=[2:0]
                // For LDR (bit12=1,bit11=0): 0110_1
                // Actually trace ANY IO read by watching if the instruction loads from 0x04000000 range
                // Much simpler: just watch PC patterns for tight loops
                // Format: 0110_1 off Rb Rd = LDR Rd,[Rb,#off*4]
                if (instr >> 11) == 0b01101 {
                    let rb = ((instr >> 3) & 7) as usize;
                    let offset = (((instr >> 6) & 0x1F) * 4) as u32;
                    let base = gba.regs[rb];
                    let addr = base.wrapping_add(offset);
                    if addr == 0x04000006 {
                        if vcount_reads < 20 || (vcount_reads % 1000 == 0) {
                            println!("Cycle {cycle} (frame {}): VCOUNT read from PC={:08X}, VCOUNT={}, reads_so_far={}",
                                cycle/280896, pc, gba.vcount, vcount_reads);
                        }
                        vcount_reads += 1;
                        last_vcount_read_pc = pc;
                    }
                }
                // LDR Rd,[Rb,Ro] format: 0101 100 Ro Rb Rd
                if (instr >> 9) == 0b0101100 {
                    let ro = ((instr >> 6) & 7) as usize;
                    let rb = ((instr >> 3) & 7) as usize;
                    let addr = gba.regs[rb].wrapping_add(gba.regs[ro]);
                    if addr == 0x04000006 {
                        if vcount_reads < 20 || (vcount_reads % 1000 == 0) {
                            println!("Cycle {cycle} (frame {}): VCOUNT read(reg) from PC={:08X}, VCOUNT={}, reads_so_far={}",
                                cycle/280896, pc, gba.vcount, vcount_reads);
                        }
                        vcount_reads += 1;
                        last_vcount_read_pc = pc;
                    }
                }
            } else {
                let instr = gba.mem_read32(pc);
                // ARM LDR: cond 01 I P U B W L Rn Rd offset12
                // bit[20]=1 (load), bit[26]=0, bit[27]=0 -> LDR
                if (instr & 0x0C100000) == 0x04100000 {
                    let rn = ((instr >> 16) & 0xF) as usize;
                    let base = gba.regs[rn];
                    // Simple immediate offset case
                    if (instr >> 25) & 1 == 0 {
                        let offset = instr & 0xFFF;
                        let up = (instr >> 23) & 1;
                        let addr = if up != 0 { base.wrapping_add(offset) } else { base.wrapping_sub(offset) };
                        if addr == 0x04000006 || (addr & 0xFFFFFFFE) == 0x04000006 {
                            if vcount_reads < 20 {
                                println!("Cycle {cycle} (frame {}): ARM VCOUNT read from PC={:08X}, VCOUNT={}",
                                    cycle/280896, pc, gba.vcount);
                            }
                            vcount_reads += 1;
                        }
                    }
                }
            }

            let old_dc = gba.dispcnt;
            gba.tick_one_cycle();
            if gba.dispcnt != old_dc && (old_dc & 0x80) != 0 {
                println!("Cycle {cycle} (frame {}): Forced blank CLEARED! DISPCNT 0x{:04X} -> 0x{:04X}",
                    cycle/280896, old_dc, gba.dispcnt);
                println!("Total VCOUNT reads before clear: {vcount_reads}, last VCOUNT read PC: {:08X}", last_vcount_read_pc);
                break;
            }
        }
        if vcount_reads == 0 {
            println!("No VCOUNT reads detected");
        }
    }

    #[test]
    fn test_meteorain_loop_trace() {
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");

        let mut fill_loop_iters = 0u64;
        let mut copy_loop_iters = 0u64;
        let mut vram_fill_iters = 0u64;
        let mut last_pc = 0u32;
        let mut saw_vram_loop = false;

        for cycle in 0..(280896u32 * 10) {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });

            // Fill loop at 0x08000190 (STMIA to EWRAM)
            if pc == 0x08000190 && last_pc != 0x08000190 {
                if fill_loop_iters == 0 {
                    println!("Fill loop start at cycle {}: R0={:08X} R1={:08X} R2={:08X}",
                        cycle, gba.regs[0], gba.regs[1], gba.regs[2]);
                }
                fill_loop_iters += 1;
            }

            // Copy loop at 0x080001A4 (LDMIA from ROM → STMIA to IRAM)
            if pc == 0x080001A4 && last_pc != 0x080001A4 {
                if copy_loop_iters == 0 {
                    println!("Copy loop start at cycle {}: R1={:08X} R2={:08X} R3={:08X}",
                        cycle, gba.regs[1], gba.regs[2], gba.regs[3]);
                }
                copy_loop_iters += 1;
            }

            // VRAM fill loop at 0x080236B4
            if pc == 0x080236B4 && last_pc != 0x080236B4 {
                if !saw_vram_loop {
                    println!("VRAM fill loop start at cycle {}: R2={:08X} R5={:08X} R6={:08X}",
                        cycle, gba.regs[2], gba.regs[5], gba.regs[6]);
                    saw_vram_loop = true;
                }
                vram_fill_iters += 1;
            }

            last_pc = pc;
            gba.tick_one_cycle();
        }

        println!("Fill loop total entry: {}", fill_loop_iters);
        println!("Copy loop total entry: {}", copy_loop_iters);
        println!("VRAM fill loop total entry: {}", vram_fill_iters);
    }


    #[test]
    fn dump_trogdor_frames() {
        dump_frames("/task/dev-roms/trogdor.gba", "/tmp/my_trog", 60);
        println!("Done");
    }

    #[test]
    fn dump_xniq_frames() {
        dump_frames("/task/dev-roms/xniq.gba", "/tmp/my_xniq", 60);
        println!("Done");
    }

    #[test]
    fn dump_meteorain_frames() {
        dump_frames("/task/dev-roms/meteorain.gba", "/tmp/my_met", 60);
        println!("Done");
    }

    #[test]
    fn test_xniq_init() {
        let mut gba = make_gba("/task/dev-roms/xniq.gba");
        let mut done = false;
        for cycle in 0..(280896u64 * 200) as u32 {
            let old_dc = gba.dispcnt;
            gba.tick_one_cycle();
            if gba.dispcnt != old_dc {
                let is_thumb = (gba.cpsr & 0x20) != 0;
                let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });
                println!("Cycle {} (frame {}): DISPCNT 0x{:04X}->0x{:04X} PC=0x{:08X}",
                    cycle, cycle/280896, old_dc, gba.dispcnt, pc);
                if (old_dc & 0x80) != 0 && (gba.dispcnt & 0x80) == 0 {
                    println!("Forced blank CLEARED at cycle {} (frame {})", cycle, cycle/280896);
                    done = true;
                    break;
                }
            }
        }
        if !done {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });
            println!("Forced blank NEVER cleared! Final DISPCNT=0x{:04X} PC=0x{:08X}", gba.dispcnt, pc);
            // Check if halted
            println!("Halted: {}", gba.halted);
            println!("IME={} IE=0x{:04X} IF=0x{:04X}", gba.ime, gba.ie, gba.if_);
        }
    }

    #[test]
    fn test_xniq_loop() {
        let mut gba = make_gba("/task/dev-roms/xniq.gba");
        // Run 1M cycles, track PC distribution
        let mut pc_counts = std::collections::HashMap::new();
        let mut prev_dispcnt = gba.dispcnt;
        for cycle in 0..2_000_000u32 {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });
            *pc_counts.entry(pc).or_insert(0u32) += 1;
            let old_dc = gba.dispcnt;
            gba.tick_one_cycle();
            if gba.dispcnt != old_dc {
                println!("Cycle {}: DISPCNT 0x{:04X}->0x{:04X} PC=0x{:08X}",
                    cycle, old_dc, gba.dispcnt, pc);
            }
            if gba.halted && !gba.halted {
                println!("CPU halted at cycle {}, PC=0x{:08X}", cycle, pc);
            }
        }
        // Print top 10 hottest PCs
        let mut sorted: Vec<_> = pc_counts.iter().collect();
        sorted.sort_by_key(|&(_, &c)| std::cmp::Reverse(c));
        println!("Top PCs by cycle count:");
        for (pc, count) in sorted.iter().take(15) {
            let is_thumb = gba.mem_read16(**pc) & 0x8000 != 0;
            println!("  0x{:08X}: {} cycles", pc, count);
        }
        let is_thumb = (gba.cpsr & 0x20) != 0;
        let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });
        println!("Final PC=0x{:08X} DISPCNT=0x{:04X} halted={}", pc, gba.dispcnt, gba.halted);
        println!("IME={} IE=0x{:04X} IF=0x{:04X}", gba.ime, gba.ie, gba.if_);
    }

    #[test]
    fn test_xniq_loop_detail() {
        let mut gba = make_gba("/task/dev-roms/xniq.gba");
        let loop1_addr = 0x08003E66u32;
        let loop2_addr = 0x08000150u32;
        let mut in_loop1 = false;
        let mut in_loop2 = false;
        let mut loops_entered = 0u32;
        
        for cycle in 0..3_000_000u32 {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });
            
            if is_thumb && pc == loop1_addr && !in_loop1 {
                in_loop1 = true;
                loops_entered += 1;
                println!("Cycle {}: Entering loop1(0x08003E66) R0=0x{:08X} R1={} R2=0x{:08X}",
                    cycle, gba.regs[0], gba.regs[1], gba.regs[2]);
            } else if in_loop1 && !(is_thumb && (pc == loop1_addr || pc == 0x08003E68 || pc == 0x08003E6A)) {
                in_loop1 = false;
                println!("Cycle {}: Leaving loop1, final R0=0x{:08X}", cycle, gba.regs[0]);
            }
            
            if is_thumb && pc == loop2_addr && !in_loop2 {
                in_loop2 = true;
                loops_entered += 1;
                println!("Cycle {}: Entering loop2(0x08000150) R0=0x{:08X} R1={} R2=0x{:08X}",
                    cycle, gba.regs[0], gba.regs[1], gba.regs[2]);
            } else if in_loop2 && !(is_thumb && (pc == loop2_addr || pc == 0x08000152 || pc == 0x08000154)) {
                in_loop2 = false;
                println!("Cycle {}: Leaving loop2, final R0=0x{:08X}", cycle, gba.regs[0]);
            }
            
            let old_dc = gba.dispcnt;
            gba.tick_one_cycle();
            if gba.dispcnt != old_dc {
                println!("Cycle {}: DISPCNT 0x{:04X}->0x{:04X}", cycle, old_dc, gba.dispcnt);
            }
            
            if loops_entered > 10 { break; }
        }
        let is_thumb = (gba.cpsr & 0x20) != 0;
        let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });
        println!("Final PC=0x{:08X} loops_entered={}", pc, loops_entered);
    }

    #[test]
    fn test_xniq_before_loop1() {
        let mut gba = make_gba("/task/dev-roms/xniq.gba");
        let loop1_addr = 0x08003E66u32;
        let mut trace_start = false;
        
        for cycle in 0..1_000_000u32 {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });
            
            // Start tracing 500 cycles before loop1 entry
            if !trace_start && is_thumb && pc == loop1_addr {
                trace_start = true;
                println!("Loop1 entered at cycle {}: R0={:08X} R1={} R2={:08X}",
                    cycle, gba.regs[0], gba.regs[1], gba.regs[2]);
                break;
            }
            
            gba.tick_one_cycle();
        }
        
        // Now run again from scratch and trace the 100 instructions before loop1 entry
        let mut gba2 = make_gba("/task/dev-roms/xniq.gba");
        let mut insn_count = 0u32;
        let mut insns_before_loop: Vec<(u32, u32, [u32; 4])> = Vec::new();
        let mut found_loop = false;
        
        for _cycle in 0..1_000_000u32 {
            let is_thumb = (gba2.cpsr & 0x20) != 0;
            let pc = gba2.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });
            let instr = if is_thumb { gba2.mem_read16(pc) as u32 } else { gba2.mem_read32(pc) };
            
            // Track last 100 instructions
            if is_thumb && (pc == loop1_addr || pc == 0x08003E68 || pc == 0x08003E6A) {
                if !found_loop {
                    found_loop = true;
                    println!("100 instructions before loop1:");
                    for (addr, i, regs) in insns_before_loop.iter().rev().take(100).rev() {
                        println!("  {:08X}: {:04X}  r0={:08X} r1={:08X} r2={:08X} r3={:08X}",
                            addr, i, regs[0], regs[1], regs[2], regs[3]);
                    }
                    println!("Loop1 R1={}", gba2.regs[1]);
                }
            } else if is_thumb {
                insns_before_loop.push((pc, instr, [gba2.regs[0], gba2.regs[1], gba2.regs[2], gba2.regs[3]]));
                if insns_before_loop.len() > 200 {
                    insns_before_loop.remove(0);
                }
            }
            
            if found_loop { break; }
            gba2.tick_one_cycle();
        }
    }

    #[test]
    fn test_xniq_trace_all() {
        let mut gba = make_gba("/task/dev-roms/xniq.gba");
        let loop1_addr = 0x08003E66u32;
        let mut insns: Vec<(u32, u32, bool, [u32; 5])> = Vec::new(); // (pc, instr, is_thumb, [r0,r1,r2,r3,r14])
        let mut found = false;
        
        for _cycle in 0..1_000_000u32 {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });
            let instr = if is_thumb { gba.mem_read16(pc) as u32 } else { gba.mem_read32(pc) };
            
            if is_thumb && pc == loop1_addr && !found {
                found = true;
                println!("Loop1 at cycle _: R1={}", gba.regs[1]);
                println!("Last 100 instructions:");
                let start = if insns.len() > 100 { insns.len() - 100 } else { 0 };
                for (addr, i, thumb, regs) in &insns[start..] {
                    println!("  {:08X}: {:08X} {} r0={:08X} r1={:08X} r2={:08X} r3={:08X} lr={:08X}",
                        addr, i, if *thumb {"T"} else {"A"}, regs[0], regs[1], regs[2], regs[3], regs[4]);
                }
                break;
            }
            
            insns.push((pc, instr, is_thumb, [gba.regs[0], gba.regs[1], gba.regs[2], gba.regs[3], gba.regs[14]]));
            if insns.len() > 300 { insns.remove(0); }
            gba.tick_one_cycle();
        }
    }

    #[test]
    fn test_anguna_trace_init() {
        // Trace anguna's initialization to find 0x04000800 writes and understand timing
        let mut gba = make_gba("/task/dev-roms/anguna.gba");
        let mut insn_count = 0u32;
        let mut last_pc = 0xFFFFFFFFu32;

        for _ in 0..(280896u32 * 2) {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });

            if pc != last_pc {
                // New instruction
                let mode = if is_thumb { "T" } else { "A" };

                // Check for writes to IO region (we'll detect via the PC trace)
                // Print first 200 distinct instructions
                if insn_count < 200 {
                    let r = &gba.regs;
                    println!("{:08X}: {} r0={:08X} r1={:08X} r2={:08X} r3={:08X} r4={:08X} r13={:08X} r14={:08X}",
                        pc, mode, r[0], r[1], r[2], r[3], r[4], r[13], r[14]);
                }
                insn_count += 1;
                last_pc = pc;
            }

            gba.tick_one_cycle();
        }
    }

    #[test]
    fn test_anguna_800_writes() {
        // Check if anguna writes to 0x04000800 (EWRAM wait state control)
        let mut gba = make_gba("/task/dev-roms/anguna.gba");

        // Instrument by monitoring writes. We'll check bus_write via a wrapper.
        // Instead, just trace all writes in the first 100K cycles
        struct WriteMonitor {
            cycles: u32,
        }

        // Simple approach: scan for STR/STRH/STRB patterns that write to 0x04000800
        // We'll just run and watch mem_write calls. Since we can't easily hook,
        // let's trace and look for the sequence that sets up 0x04000800.

        // Actually let's just check: does the game change EWRAM timing?
        // Run 100K cycles and check for writes to addresses around 0x04000800
        for _ in 0..(280896u32 * 5) {
            let old_dispcnt = gba.dispcnt;
            gba.tick_one_cycle();
            if old_dispcnt != gba.dispcnt {
                let is_thumb = (gba.cpsr & 0x20) != 0;
                let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });
                println!("DISPCNT changed at cycle {} PC={:08X}: {:04X} -> {:04X}",
                    gba.cycles, pc, old_dispcnt, gba.dispcnt);
                if gba.dispcnt & 0x80 == 0 { break; }
            }
        }
    }

    #[test]
    fn test_meteorain_vblankintr() {
        // Trace SWI calls for meteorain to see if VBlankIntrWait (SWI 5) works correctly
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");
        let mut last_scanline = 0u16;
        let mut swi_count = [0u32; 256];
        let mut vblank_count = 0u32;
        let mut last_pc = 0u32;

        // Run for 50 frames
        for _ in 0..(280896u32 * 50) {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });

            // Detect SWI instructions (Thumb: 0xDFxx)
            if is_thumb && pc != last_pc {
                // Check if current instruction is SWI
                let rom = if pc >= 0x08000000 { pc - 0x08000000 } else { 0 };
                if rom < gba.rom.len() as u32 {
                    let instr = (gba.rom[rom as usize] as u16) | ((gba.rom[rom as usize + 1] as u16) << 8);
                    if (instr >> 8) == 0xDF {
                        let swi_num = (instr & 0xFF) as usize;
                        swi_count[swi_num] += 1;
                        if swi_count[swi_num] <= 3 {
                            println!("SWI {} at PC={:08X} cycle={} frame={}",
                                swi_num, pc, gba.cycles, gba.cycles / 280896);
                        }
                    }
                }
                last_pc = pc;
            }

            // Count VBlanks
            if gba.vcount == 160 && last_scanline == 159 {
                vblank_count += 1;
            }
            last_scanline = gba.vcount;

            gba.tick_one_cycle();
        }

        println!("Total SWI counts:");
        for (i, &cnt) in swi_count.iter().enumerate() {
            if cnt > 0 { println!("  SWI {}: {} times", i, cnt); }
        }
        println!("Total VBlanks: {}", vblank_count);
    }

    #[test]
    fn test_meteorain_game_speed() {
        // Check how many instruction executions happen per frame for meteorain
        // Compare with oracle to understand speed difference
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");
        let mut last_frame = 0u64;
        let mut instr_count = 0u64;
        let mut halt_cycles = 0u64;
        let mut in_halt = false;

        for _ in 0..(280896u64 * 50) as u32 {
            let frame = gba.cycles / 280896;

            if frame != last_frame {
                if last_frame >= 10 && last_frame <= 15 {
                    println!("Frame {}: {} instrs, {} halt_cycles",
                        last_frame, instr_count, halt_cycles);
                }
                instr_count = 0;
                halt_cycles = 0;
                last_frame = frame;
            }

            if gba.halted {
                halt_cycles += 1;
                in_halt = true;
            } else {
                if in_halt { in_halt = false; }
                if gba.cpu_cycles_remaining == 0 {
                    instr_count += 1;
                }
            }

            gba.tick_one_cycle();
        }
    }

    #[test]
    fn test_meteorain_wait_mechanism() {
        // Understand what busy-wait loop meteorain uses for frame pacing
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");

        let target_cycle = 3u64 * 280896;
        while (gba.cycles as u64) < target_cycle {
            gba.tick_one_cycle();
        }
        println!("After 3 frames (cycle {}): VCOUNT={}, DISPSTAT={:04X}",
            gba.cycles, gba.vcount, gba.dispstat);
        for i in 0..4 {
            println!("TM{}CNT_L={:04X} TM{}CNT_H={:04X}",
                i, gba.timers[i].counter, i, gba.timers[i].ctrl);
        }
    }

    #[test]
    fn test_meteorain_vcount_trace2() {
        // Identify callers of the tight loop at 0x08000190 and what R0/R1 they pass
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");

        // Run past init to frame 1
        let frame_start = 280896u64;
        while (gba.cycles as u64) < frame_start {
            gba.tick_one_cycle();
        }

        let frame_end = frame_start + 280896 * 3;
        let mut call_count = 0u32;
        let mut last_caller = 0u32;

        while (gba.cycles as u64) < frame_end {
            if !gba.halted && gba.cpu_cycles_remaining == 0 {
                let is_thumb = (gba.cpsr & 0x20) != 0;
                if is_thumb {
                    let pc = gba.regs[15].wrapping_sub(4);
                    // Detect entry to loop at 0x08000190
                    if pc == 0x08000190 {
                        let caller = gba.regs[14]; // LR = return address
                        if caller != last_caller || call_count < 5 {
                            println!("Loop called: cycle={} R0={:08X} R1={:08X} LR={:08X} VCOUNT={}",
                                gba.cycles, gba.regs[0], gba.regs[1], caller, gba.vcount);
                            last_caller = caller;
                        }
                        call_count += 1;
                    }
                    // Detect return from loop (BX LR at 0x08000196)
                    if pc == 0x08000196 && call_count > 0 {
                        println!("Loop return: cycle={} iterations_done={} R0={:08X} VCOUNT={}",
                            gba.cycles, gba.regs[1]/4, gba.regs[0], gba.vcount);
                    }
                }
            }
            gba.tick_one_cycle();
        }
        println!("Total loop calls in 3 frames: {}", call_count);
    }

    #[test]
    fn test_meteorain_frame34() {
        // Investigate why meteorain goes all-black starting at frame 34
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");

        // Run to near frame 34
        let target_cycle = 34u64 * 280896;
        let mut last_dispcnt = gba.dispcnt;
        let mut last_pal0: u16 = 0;

        while (gba.cycles as u64) < target_cycle + 280896 * 2 {
            // Check palette[0] (backdrop color)
            let pal0 = (gba.palette[0] as u16) | ((gba.palette[1] as u16) << 8);
            if pal0 != last_pal0 {
                let is_thumb = (gba.cpsr & 0x20) != 0;
                let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });
                println!("Cycle {}: Palette[0] changed {:04X} -> {:04X} PC={:08X} Frame={}",
                    gba.cycles, last_pal0, pal0, pc, gba.cycles / 280896);
                last_pal0 = pal0;
            }

            // Check DISPCNT changes
            if gba.dispcnt != last_dispcnt {
                let is_thumb = (gba.cpsr & 0x20) != 0;
                let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });
                println!("Cycle {}: DISPCNT {:04X} -> {:04X} PC={:08X} Frame={}",
                    gba.cycles, last_dispcnt, gba.dispcnt, pc, gba.cycles / 280896);
                last_dispcnt = gba.dispcnt;
            }

            gba.tick_one_cycle();
        }
    }

    #[test]
    fn test_meteorain_waitcnt_writes() {
        // Check all WAITCNT writes to understand initial timing
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");
        let mut last_wc = gba.waitcnt;
        println!("Initial WAITCNT: {:04X}", last_wc);
        for _ in 0..(280896u64 * 15) as u32 {
            if gba.waitcnt != last_wc {
                let is_thumb = (gba.cpsr & 0x20) != 0;
                let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });
                println!("WAITCNT {:04X} -> {:04X} at cycle={} PC={:08X}",
                    last_wc, gba.waitcnt, gba.cycles, pc);
                last_wc = gba.waitcnt;
            }
            gba.tick_one_cycle();
        }
        println!("Final WAITCNT: {:04X}", gba.waitcnt);
    }

    #[test]
    fn test_meteorain_waitcnt_timing() {
        // Check WAITCNT value when the ROM search loop runs
        // The loop reads from ROM, so timing depends on WAITCNT
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");

        // Run to just before dark blue starts
        let target = 4_000_000u64;
        while (gba.cycles as u64) < target {
            gba.tick_one_cycle();
        }

        // Report WAITCNT and sample cycles per loop iteration
        println!("At cycle {}: WAITCNT={:04X}", gba.cycles, gba.waitcnt);

        // Count cycles for 1000 iterations of the outer loop at 0x08016FD2
        let mut loop_iters = 0u32;
        let mut loop_start_cycle = 0u64;
        let mut last_loop_cycle = 0u64;

        let end_cycle = target + 1_000_000;
        while (gba.cycles as u64) < end_cycle {
            if !gba.halted && gba.cpu_cycles_remaining == 0 {
                let is_thumb = (gba.cpsr & 0x20) != 0;
                if is_thumb {
                    let pc = gba.regs[15].wrapping_sub(4);
                    if pc == 0x08016FD2 {
                        if loop_iters == 0 {
                            loop_start_cycle = gba.cycles;
                        }
                        loop_iters += 1;
                        if loop_iters <= 5 || loop_iters % 1000 == 0 {
                            let delta = if loop_iters > 1 { gba.cycles - last_loop_cycle } else { 0 };
                            println!("iter {}: cycle={} delta={} WAITCNT={:04X}",
                                loop_iters, gba.cycles, delta, gba.waitcnt);
                        }
                        last_loop_cycle = gba.cycles;
                        if loop_iters >= 5000 { break; }
                    }
                }
            }
            gba.tick_one_cycle();
        }
        if loop_iters >= 2 {
            let total = last_loop_cycle - loop_start_cycle;
            println!("Total {} iterations in {} cycles = {:.1} cycles/iter",
                loop_iters, total, total as f64 / loop_iters as f64);
        }
    }

    #[test]
    fn test_meteorain_dark_blue_trace() {
        // The game shows dark blue (pal0=0x0800) from cycle 2448486 to 9647099
        // Oracle shows it for ~2.5x longer. Find the loop that controls this duration.
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");

        // Run to start of dark blue
        let dark_start = 2_448_000u64;
        while (gba.cycles as u64) < dark_start {
            gba.tick_one_cycle();
        }

        // Sample PC every 50K instructions during dark blue period
        let dark_end = 9_700_000u64;
        let mut instr_count = 0u64;
        let mut last_sample = 0u64;
        let mut pc_histogram: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();

        while (gba.cycles as u64) < dark_end {
            if !gba.halted && gba.cpu_cycles_remaining == 0 {
                instr_count += 1;
                let is_thumb = (gba.cpsr & 0x20) != 0;
                let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });
                *pc_histogram.entry(pc).or_insert(0) += 1;

                if instr_count - last_sample >= 200_000 {
                    last_sample = instr_count;
                    // Show top 5 PCs
                    let mut top: Vec<_> = pc_histogram.iter().collect();
                    top.sort_by(|a, b| b.1.cmp(a.1));
                    println!("At instr {}: cycle={}", instr_count, gba.cycles);
                    for (pc, cnt) in top.iter().take(5) {
                        println!("  PC={:08X}: {} times ({:.1}%)", pc, *cnt,
                            **cnt as f64 / instr_count as f64 * 100.0);
                    }
                }
            }
            gba.tick_one_cycle();
        }
        let mut top: Vec<_> = pc_histogram.into_iter().collect();
        top.sort_by(|a, b| b.1.cmp(&a.1));
        println!("Final histogram (top 10):");
        for (pc, cnt) in top.iter().take(10) {
            println!("  PC={:08X}: {} times ({:.1}%)", pc, cnt,
                *cnt as f64 / instr_count as f64 * 100.0);
        }
    }

    #[test]
    fn test_meteorain_palette_loop() {
        // Trace what loop controls the dark-blue-screen duration
        // Oracle: dark blue frames 13-76 (64 frames), we: ~2-34 (32 frames)
        // The loop at 0x080184D4 (STRH R0,[R4,#0]) writes to palette
        // and the outer loop at 0x080184B4 appears to control frame count
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");

        // Run to cycle ~2M to get past init
        let start_cycle = 2_000_000u64;
        while (gba.cycles as u64) < start_cycle {
            gba.tick_one_cycle();
        }

        let end_cycle = 25_000_000u64;
        let mut vblank_count = 0u32;
        let mut last_vcount = gba.vcount;
        let mut last_pal0: u16 = 0;
        let mut loop_frame_counter: Option<u32> = None;

        while (gba.cycles as u64) < end_cycle {
            let pal0 = (gba.palette[0] as u16) | ((gba.palette[1] as u16) << 8);
            if pal0 != last_pal0 {
                let is_thumb = (gba.cpsr & 0x20) != 0;
                let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });
                println!("pal0 {:04X}->{:04X} cycle={} frame={} vblank_count={}",
                    last_pal0, pal0, gba.cycles, gba.cycles/280896, vblank_count);
                last_pal0 = pal0;
            }

            if gba.vcount == 160 && last_vcount != 160 {
                vblank_count += 1;
            }
            last_vcount = gba.vcount;

            // Detect the outer loop counter - watch for reads from a "frame counter" address
            // Track when PC is at 0x080184B4 and capture R1 (likely loop counter)
            if !gba.halted && gba.cpu_cycles_remaining == 0 {
                let is_thumb = (gba.cpsr & 0x20) != 0;
                if is_thumb {
                    let pc = gba.regs[15].wrapping_sub(4);
                    if pc == 0x080184B4 {
                        if loop_frame_counter.map_or(true, |v| v != gba.regs[1]) {
                            println!("  loop@{:08X}: R0={:08X} R1={:08X} R2={:08X} R3={:08X} R4={:08X} VCOUNT={}",
                                pc, gba.regs[0], gba.regs[1], gba.regs[2], gba.regs[3], gba.regs[4], gba.vcount);
                            loop_frame_counter = Some(gba.regs[1]);
                        }
                    }
                }
            }

            gba.tick_one_cycle();
        }
    }

    #[test]
    fn test_meteorain_search_iters() {
        // Count loop iterations during dark blue phase to verify iteration count
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");

        let mut last_pal0: u16 = 0xFFFF;
        let mut dark_blue_active = false;
        let mut iter_count = 0u64;
        let mut loop_entry_cycle = 0u64;
        let mut loop_exit_cycle = 0u64;
        let mut first_r5 = 0u32;

        let mut halt_cycles = 0u64;
        for _ in 0..(30_000_000u64 * 3) {
            // Track palette[0]
            let pal0 = (gba.palette[0] as u16) | ((gba.palette[1] as u16) << 8);
            if pal0 != last_pal0 {
                if pal0 == 0x0800 && !dark_blue_active {
                    dark_blue_active = true;
                    println!("Dark blue START at cycle={} frame={}",
                        gba.cycles, gba.cycles/280896);
                } else if pal0 != 0x0800 && dark_blue_active {
                    dark_blue_active = false;
                    loop_exit_cycle = gba.cycles;
                    println!("Dark blue END at cycle={} frame={}", gba.cycles, gba.cycles/280896);
                    println!("Loop iterations: {} in {} cycles = {:.1} cycles/iter",
                        iter_count, loop_exit_cycle - loop_entry_cycle,
                        (loop_exit_cycle - loop_entry_cycle) as f64 / iter_count as f64);
                    println!("Halt cycles in dark blue: {} ({:.1}/iter)", halt_cycles,
                        halt_cycles as f64 / iter_count as f64);
                    println!("Final R5={:08X}", gba.regs[5]);
                    break;
                }
                last_pal0 = pal0;
            }

            if dark_blue_active && gba.halted {
                halt_cycles += 1;
            }

            // Count loop iterations
            if dark_blue_active && gba.cpu_cycles_remaining == 0 && !gba.halted {
                let is_thumb = (gba.cpsr & 0x20) != 0;
                if is_thumb && gba.regs[15].wrapping_sub(4) == 0x08016FD2 {
                    if iter_count == 0 {
                        loop_entry_cycle = gba.cycles;
                        first_r5 = gba.regs[5];
                        println!("First R5={:08X} R7={:08X} WAITCNT=0x{:04X} MEMCNT=0x{:08X}",
                            gba.regs[5], gba.regs[7], gba.waitcnt, gba.memcnt);
                        println!("IME=0x{:X} IE=0x{:04X} IF=0x{:04X}",
                            gba.ime, gba.ie, gba.if_);
                        for ch in 0..4 {
                            let d = &gba.dma[ch];
                            println!("DMA{}: ctrl=0x{:04X} src=0x{:08X} dst=0x{:08X} cnt={}",
                                ch, d.ctrl, d.src_raw, d.dst_raw, d.cnt_raw);
                        }
                        for t in 0..4 {
                            let tm = &gba.timers[t];
                            println!("Timer{}: counter={} reload={} ctrl=0x{:04X} enabled={} irq={} prescaler={}",
                                t, tm.counter, tm.reload, tm.ctrl, tm.enabled, tm.irq, tm.prescaler);
                        }
                    }
                    iter_count += 1;
                }
            }

            gba.tick_one_cycle();
        }
    }

    #[test]
    fn test_meteorain_ldr_addr() {
        // Trace what address LDR at 08016FE4 is loading from
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");
        let target = 4_000_000u64;
        while (gba.cycles as u64) < target {
            gba.tick_one_cycle();
        }

        let mut count = 0;
        for _ in 0..200_000 {
            if gba.cpu_cycles_remaining == 0 && !gba.halted {
                let is_thumb = (gba.cpsr & 0x20) != 0;
                if is_thumb {
                    let pc = gba.regs[15].wrapping_sub(4);
                    if pc == 0x08016FD2 || pc == 0x08016FE4 {
                        if count < 5 {
                            let r2 = gba.regs[2];
                            let r5 = gba.regs[5];
                            let r7 = gba.regs[7];
                            println!("PC={:08X}: R2={:08X} R5={:08X} R7={:08X} cycle={}",
                                pc, r2, r5, r7, gba.cycles);
                        }
                        count += 1;
                        if count >= 10 { break; }
                    }
                }
            }
            gba.tick_one_cycle();
        }
    }

    #[test]
    fn test_meteorain_loop_disasm() {
        // Read and print ROM bytes at loop addresses for manual disassembly
        let gba = make_gba("/task/dev-roms/meteorain.gba");
        // Loop is in Thumb mode; print 16-bit halfwords at key addresses
        let addrs = [
            0x08016FD2u32, 0x08016FD4, 0x08016FD6, 0x08016FD8, 0x08016FDA,
            0x08016FDC, 0x08016FDE, 0x08016FE0, 0x08016FE2, 0x08016FE4,
            0x08016FE6, 0x08016FE8, 0x08016FEA,
            0x08017002, 0x08017004,
            0x08018E10, 0x08018E12, 0x08018E14, 0x08018E16,
            0x08019868, 0x0801986A, 0x0801986C, 0x0801986E, 0x08019870,
            0x08019872, 0x08019874, 0x08019876, 0x08019878, 0x0801987A,
            0x0801987C, 0x0801987E, 0x08019880, 0x08019882, 0x08019884,
            0x08019886, 0x08019888, 0x0801988A, 0x0801988C, 0x0801988E,
            0x08019890, 0x08019892,
            0x080198CA, 0x080198CC, 0x080198CE, 0x080198D0, 0x080198D2, 0x080198D4,
        ];
        println!("ROM loop instruction bytes:");
        for &addr in &addrs {
            let offset = (addr - 0x08000000) as usize;
            let hw = (gba.rom[offset] as u16) | ((gba.rom[offset + 1] as u16) << 8);
            println!("  {:08X}: {:04X}  (decode: {})", addr, hw, decode_thumb(hw));
        }
    }

    fn decode_thumb(hw: u16) -> String {
        // Minimal Thumb disassembler for key instruction types
        let bits15_11 = hw >> 11;
        let bits15_10 = hw >> 10;
        let bits15_13 = hw >> 13;
        if bits15_11 == 0b11110 { return format!("BL/BLX hi (upper half)"); }
        if bits15_11 == 0b11111 { return format!("BL lo (lower half, branch)"); }
        if bits15_11 == 0b11110 { return format!("BL lo (ARM)"); }
        if bits15_10 == 0b010000 { return format!("ALU ops"); }
        if hw >> 12 == 0b1011 {
            let op = (hw >> 9) & 3;
            let rlist = hw & 0xFF;
            let r = (hw >> 8) & 1;
            if op == 2 { return format!("PUSH rlist={:02X} R={}", rlist, r); }
            if op == 3 { return format!("POP rlist={:02X} R={}", rlist, r); }
        }
        if bits15_11 == 0b01111 { return format!("LDR Rd,[PC,#{}] (PC-relative load from ROM)", (hw & 0xFF) << 2); }
        if bits15_11 == 0b01101 { return format!("LDR Rd,[Rn,#imm]"); }
        if bits15_11 == 0b11001 { return format!("LDR Rd,[PC,#{}] word", (hw & 0xFF) << 2); }  // same as 01111
        let fmt = match bits15_13 {
            0b000 => "shift/add/sub",
            0b001 => "mov/cmp/add/sub imm",
            0b010 => "ALU/data",
            0b011 => "load/store",
            0b100 => "load/store/branch",
            0b101 => "load/store/branch",
            0b110 => "cond branch/swi",
            0b111 => "unconditional branch",
            _ => "?",
        };
        format!("bits15-13={:03b} fmt={}", bits15_13, fmt)
    }

    #[test]
    fn test_meteorain_loop_insn_cycles() {
        // Trace per-instruction cycle costs in the ROM search loop at 0x08016FA6
        // to understand the 25% gap between our timing and oracle's
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");

        // Run to the loop
        let target = 4_000_000u64;
        while (gba.cycles as u64) < target {
            gba.tick_one_cycle();
        }

        // Wait until we hit the loop entry point
        loop {
            if gba.cpu_cycles_remaining == 0 && !gba.halted {
                let is_thumb = (gba.cpsr & 0x20) != 0;
                if is_thumb {
                    let pc = gba.regs[15].wrapping_sub(4);
                    if pc == 0x08016FD2 { break; }
                }
            }
            gba.tick_one_cycle();
        }

        // Trace 100 iterations: record (pc, stall_cycles) for all instructions
        let mut pc_cycles: std::collections::HashMap<u32, (u64, u64)> = std::collections::HashMap::new();
        let iter_start_cycle = gba.cycles;
        let mut iter_count = 0u32;
        let mut total_insns = 0u64;
        let mut arm_cycles = 0u64;
        let mut arm_insns = 0u64;

        for _ in 0..500_000 {
            if gba.cpu_cycles_remaining == 0 && !gba.halted {
                let is_thumb = (gba.cpsr & 0x20) != 0;
                if is_thumb {
                    let pc = gba.regs[15].wrapping_sub(4);
                    gba.tick_one_cycle();
                    let sc = gba.stall_cycles as u64;
                    let e = pc_cycles.entry(pc).or_insert((0, 0));
                    e.0 += 1;
                    e.1 += sc;
                    total_insns += 1;
                    if pc == 0x08016FD2 {
                        iter_count += 1;
                        if iter_count >= 100 { break; }
                    }
                    continue;
                } else {
                    // ARM mode instruction (e.g., IRQ handler)
                    gba.tick_one_cycle();
                    arm_cycles += gba.stall_cycles as u64;
                    arm_insns += 1;
                    continue;
                }
            }
            gba.tick_one_cycle();
        }

        let total_cycles = gba.cycles as i64 - iter_start_cycle as i64;
        println!("100 iterations: {} total cycles, {:.1} avg cycles/iter, {} insns total",
            total_cycles, total_cycles as f64 / iter_count as f64, total_insns);
        println!("ARM mode: {} instructions, {} cycles ({:.2}/iter)",
            arm_insns, arm_cycles, arm_cycles as f64 / iter_count as f64);

        // Print in PC order
        let mut sorted: Vec<_> = pc_cycles.into_iter().collect();
        sorted.sort_by_key(|e| e.0);
        println!("Per-PC breakdown (total over 100 iterations):");
        let mut grand_total_cycles = 0u64;
        for (pc, (count, cycles)) in &sorted {
            println!("  PC={:08X}: {} executions, {} total cycles ({:.2} avg)",
                pc, count, cycles, *cycles as f64 / *count as f64);
            grand_total_cycles += cycles;
        }
        println!("Grand total from PC breakdown: {} cycles ({:.1} avg/iter)",
            grand_total_cycles, grand_total_cycles as f64 / iter_count as f64);
    }
}
