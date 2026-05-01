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
    fn test_meteorain_loop_trace() {
        let mut gba = make_gba("/task/dev-roms/meteorain.gba");

        // Sample PC values during startup (first 10 frames)
        let mut pc_counts: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();

        for cycle in 0..(280896u32 * 10) {
            let is_thumb = (gba.cpsr & 0x20) != 0;
            let pc = gba.regs[15].wrapping_sub(if is_thumb { 4 } else { 8 });

            if cycle % 100 == 0 {
                *pc_counts.entry(pc).or_insert(0) += 1;
            }

            gba.tick_one_cycle();
        }

        let mut entries: Vec<(u32, u64)> = pc_counts.into_iter().collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        println!("Top PCs during meteorain startup:");
        for (pc, count) in entries.iter().take(15) {
            println!("  PC=0x{:08X}: samples={}", pc, count);
        }
    }
}
