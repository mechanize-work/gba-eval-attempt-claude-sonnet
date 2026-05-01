use crate::Gba;

// GBA color: 5-bit per channel RGB, stored as BGR555 in palette
// Output format: 0xAABBGGRR (alpha=0xFF)

fn color555_to_rgba(c: u16) -> u32 {
    let r = ((c & 0x1F) as u32) << 3;
    let g = (((c >> 5) & 0x1F) as u32) << 3;
    let b = (((c >> 10) & 0x1F) as u32) << 3;
    // Expand 5-bit to 8-bit
    let r = r | (r >> 5);
    let g = g | (g >> 5);
    let b = b | (b >> 5);
    0xFF000000 | (b << 16) | (g << 8) | r
}

// Layer IDs for priority/blend
const LAYER_BG0: usize = 0;
const LAYER_BG1: usize = 1;
const LAYER_BG2: usize = 2;
const LAYER_BG3: usize = 3;
const LAYER_OBJ: usize = 4;
const LAYER_BD:  usize = 5;

struct Pixel {
    color: u16,   // BGR555 color
    priority: u8,
    layer: u8,
    transparent: bool,
    semi_transparent: bool,
}

impl Default for Pixel {
    fn default() -> Self {
        Pixel { color: 0, priority: 4, layer: LAYER_BD as u8, transparent: true, semi_transparent: false }
    }
}

impl Gba {
    pub(crate) fn ppu_render_scanline(&mut self, line: u32) {
        let y = line as usize;
        let mode = self.dispcnt & 0x7;
        let forced_blank = (self.dispcnt >> 7) & 1 != 0;

        if forced_blank {
            // White screen
            for x in 0..240 {
                self.framebuffer[y * 240 + x] = 0xFFFFFFFF;
            }
            return;
        }

        // Build scanline layers
        let mut bg_lines: [[Option<u16>; 240]; 4] = [[None; 240]; 4];
        let mut obj_line: [(u16, u8, bool); 240] = [(0, 4, false); 240];
        // obj_line: (color, priority, semi_transparent)

        // Render backgrounds based on mode
        match mode {
            0 => {
                // Mode 0: 4 regular BG layers
                for bg in 0..4 {
                    if (self.dispcnt >> (8 + bg)) & 1 != 0 {
                        self.render_bg_regular(bg, line, &mut bg_lines[bg]);
                    }
                }
            }
            1 => {
                // Mode 1: BG0, BG1 regular; BG2 affine
                if (self.dispcnt >> 8) & 1 != 0 {
                    self.render_bg_regular(0, line, &mut bg_lines[0]);
                }
                if (self.dispcnt >> 9) & 1 != 0 {
                    self.render_bg_regular(1, line, &mut bg_lines[1]);
                }
                if (self.dispcnt >> 10) & 1 != 0 {
                    self.render_bg_affine(0, line, &mut bg_lines[2]);
                }
            }
            2 => {
                // Mode 2: BG2 and BG3 affine
                if (self.dispcnt >> 10) & 1 != 0 {
                    self.render_bg_affine(0, line, &mut bg_lines[2]);
                }
                if (self.dispcnt >> 11) & 1 != 0 {
                    self.render_bg_affine(1, line, &mut bg_lines[3]);
                }
            }
            3 => {
                // Mode 3: 240x160 15-bit direct color bitmap
                if (self.dispcnt >> 10) & 1 != 0 {
                    for x in 0..240 {
                        let off = (y * 240 + x) * 2;
                        let c = self.vram_read16(off);
                        bg_lines[2][x] = Some(c);
                    }
                }
            }
            4 => {
                // Mode 4: 240x160 8-bit paletted bitmap
                if (self.dispcnt >> 10) & 1 != 0 {
                    let frame = if (self.dispcnt >> 4) & 1 != 0 { 0xA000 } else { 0 };
                    for x in 0..240 {
                        let idx = self.vram[(frame + y * 240 + x) as usize] as usize;
                        if idx != 0 {
                            let c = self.palette_read16(idx * 2);
                            bg_lines[2][x] = Some(c);
                        }
                    }
                }
            }
            5 => {
                // Mode 5: 160x128 15-bit direct color (2 frames)
                if (self.dispcnt >> 10) & 1 != 0 && y < 128 {
                    let frame = if (self.dispcnt >> 4) & 1 != 0 { 0xA000 } else { 0 };
                    for x in 0..160usize {
                        let off = frame + (y * 160 + x) * 2;
                        let c = self.vram_read16(off);
                        bg_lines[2][x] = Some(c);
                    }
                }
            }
            _ => {}
        }

        // Render sprites if enabled
        if (self.dispcnt >> 12) & 1 != 0 {
            self.render_sprites(line, &mut obj_line);
        }

        // Composite the scanline
        self.composite_scanline(y, &bg_lines, &obj_line);

        // Update affine BG reference points at end of each visible scanline
        if mode == 1 || mode == 2 {
            self.bgx_latch[0] = self.bgx_latch[0].wrapping_add(self.bgpb[0] as i32);
            self.bgy_latch[0] = self.bgy_latch[0].wrapping_add(self.bgpd[0] as i32);
            if mode == 2 {
                self.bgx_latch[1] = self.bgx_latch[1].wrapping_add(self.bgpb[1] as i32);
                self.bgy_latch[1] = self.bgy_latch[1].wrapping_add(self.bgpd[1] as i32);
            }
        } else if mode == 0 {
            // No affine BGs in mode 0, but still update both latches if used elsewhere
        } else if mode >= 3 {
            // Bitmap modes use BG2 affine, update latch
            self.bgx_latch[0] = self.bgx_latch[0].wrapping_add(self.bgpb[0] as i32);
            self.bgy_latch[0] = self.bgy_latch[0].wrapping_add(self.bgpd[0] as i32);
        }
    }

    fn render_bg_regular(&self, bg: usize, line: u32, out: &mut [Option<u16>; 240]) {
        let cnt = self.bgcnt[bg];
        let priority = cnt & 3;
        let tile_base = (((cnt >> 2) & 3) as usize) * 0x4000;
        let _mosaic = (cnt >> 4) & 1 != 0;
        let color256 = (cnt >> 5) & 1 != 0;
        let map_base = (((cnt >> 8) & 0x1F) as usize) * 0x800;
        let screen_size = (cnt >> 14) & 3;

        let scroll_x = self.bghofs[bg] as u32;
        let scroll_y = self.bgvofs[bg] as u32;

        // Map dimensions
        let (map_w, map_h) = match screen_size {
            0 => (256u32, 256u32),
            1 => (512u32, 256u32),
            2 => (256u32, 512u32),
            3 => (512u32, 512u32),
            _ => unreachable!()
        };

        let y_eff = (line.wrapping_add(scroll_y)) & (map_h - 1);

        for x in 0..240u32 {
            let x_eff = (x.wrapping_add(scroll_x)) & (map_w - 1);

            // Map block index
            let tile_x = x_eff >> 3;
            let tile_y = y_eff >> 3;
            let px = (x_eff & 7) as usize;
            let py = (y_eff & 7) as usize;

            // Screen block selection
            let sb = match screen_size {
                0 => 0,
                1 => tile_x / 32,
                2 => tile_y / 32,
                3 => (tile_x / 32) + (tile_y / 32) * 2,
                _ => unreachable!()
            };
            let local_tx = tile_x & 31;
            let local_ty = tile_y & 31;

            let map_off = map_base + (sb as usize) * 0x800 + ((local_ty * 32 + local_tx) as usize) * 2;
            let tile_entry = self.vram_read16(map_off);

            let tile_num = (tile_entry & 0x3FF) as usize;
            let hflip = (tile_entry >> 10) & 1 != 0;
            let vflip = (tile_entry >> 11) & 1 != 0;
            let palette_num = ((tile_entry >> 12) & 0xF) as usize;

            let px = if hflip { 7 - px } else { px };
            let py = if vflip { 7 - py } else { py };

            let color_idx = if color256 {
                // 256 color: 1 byte per pixel
                let tile_off = tile_base + tile_num * 64 + py * 8 + px;
                self.vram_read8(tile_off) as usize
            } else {
                // 16 color: 4 bits per pixel
                let tile_off = tile_base + tile_num * 32 + py * 4 + px / 2;
                let byte = self.vram_read8(tile_off);
                let nibble = if px & 1 != 0 { (byte >> 4) & 0xF } else { byte & 0xF } as usize;
                nibble
            };

            if color_idx == 0 { continue; }  // transparent

            let pal_off = if color256 {
                color_idx * 2
            } else {
                palette_num * 32 + color_idx * 2
            };
            let color = self.palette_read16(pal_off);
            out[x as usize] = Some(color);
        }
    }

    fn render_bg_affine(&self, aff_idx: usize, line: u32, out: &mut [Option<u16>; 240]) {
        // aff_idx: 0 = BG2, 1 = BG3
        let bg = aff_idx + 2;
        let cnt = self.bgcnt[bg];
        let tile_base = (((cnt >> 2) & 3) as usize) * 0x4000;
        let map_base = (((cnt >> 8) & 0x1F) as usize) * 0x800;
        let wraparound = (cnt >> 13) & 1 != 0;
        let screen_size = (cnt >> 14) & 3;

        let map_size = 16u32 << screen_size;  // in tiles: 16, 32, 64, 128

        let pa = self.bgpa[aff_idx] as i32;
        let pb = self.bgpb[aff_idx] as i32;
        let pc = self.bgpc[aff_idx] as i32;
        let pd = self.bgpd[aff_idx] as i32;

        let refx = self.bgx_latch[aff_idx];
        let refy = self.bgy_latch[aff_idx];

        for x in 0..240i32 {
            // Transform screen coords to texture coords (fixed point 8.8 -> 8)
            let tx = (refx + pa * x) >> 8;
            let ty = (refy + pc * x) >> 8;

            let map_px = map_size as i32 * 8;
            let (tx, ty) = if wraparound {
                (tx.rem_euclid(map_px), ty.rem_euclid(map_px))
            } else {
                if tx < 0 || ty < 0 || tx >= map_px || ty >= map_px {
                    continue;
                }
                (tx, ty)
            };

            let tile_x = (tx >> 3) as usize;
            let tile_y = (ty >> 3) as usize;
            let px = (tx & 7) as usize;
            let py = (ty & 7) as usize;

            let map_off = map_base + (tile_y * map_size as usize + tile_x);
            let tile_num = self.vram_read8(map_off) as usize;

            // Affine BG always uses 256-color tiles
            let tile_off = tile_base + tile_num * 64 + py * 8 + px;
            let color_idx = self.vram_read8(tile_off) as usize;

            if color_idx == 0 { continue; }

            let color = self.palette_read16(color_idx * 2);
            out[x as usize] = Some(color);
        }
    }

    fn render_sprites(&self, line: u32, obj_line: &mut [(u16, u8, bool); 240]) {
        // OAM has 128 sprites, each 8 bytes (4 * 16-bit attributes)
        let y = line as i32;

        // Process sprites in reverse priority order (sprite 0 has highest priority)
        for sprite_idx in (0..128usize).rev() {
            let attr0 = self.oam_read16(sprite_idx * 8) as u32;
            let attr1 = self.oam_read16(sprite_idx * 8 + 2) as u32;
            let attr2 = self.oam_read16(sprite_idx * 8 + 4) as u32;

            let rot_scale = (attr0 >> 8) & 1 != 0;
            let disable_or_dbl = (attr0 >> 9) & 1 != 0;

            if !rot_scale && disable_or_dbl {
                continue;  // disabled
            }

            let obj_y = (attr0 & 0xFF) as i32;
            let obj_mode = (attr0 >> 10) & 3;  // 0=normal, 1=semi-transparent, 2=OBJ window, 3=invalid
            let obj_mosaic = (attr0 >> 12) & 1 != 0;
            let color256 = (attr0 >> 13) & 1 != 0;
            let shape = (attr0 >> 14) & 3;

            let obj_x = ((attr1 & 0x1FF) as i32).wrapping_sub(if attr1 & 0x100 != 0 { 512 } else { 0 });
            let hflip = !rot_scale && (attr1 >> 12) & 1 != 0;
            let vflip = !rot_scale && (attr1 >> 13) & 1 != 0;
            let size = (attr1 >> 14) & 3;

            let tile_num = (attr2 & 0x3FF) as usize;
            let priority = ((attr2 >> 10) & 3) as u8;
            let palette_num = ((attr2 >> 12) & 0xF) as usize;

            // Get sprite dimensions
            let (w, h) = sprite_dimensions(shape, size);
            let (disp_w, disp_h) = if rot_scale && disable_or_dbl {
                (w * 2, h * 2)  // double-size affine
            } else {
                (w, h)
            };

            // Check if sprite is on this scanline
            let sy = if obj_y + disp_h as i32 > 256 {
                obj_y as i32 - 256
            } else {
                obj_y as i32
            };

            if y < sy || y >= sy + disp_h as i32 { continue; }

            let ty_in_sprite = y - sy;

            // Pixel mapping
            for dx in 0..disp_w as i32 {
                let px_screen = obj_x + dx;
                if px_screen < 0 || px_screen >= 240 { continue; }

                let (tex_x, tex_y);
                if rot_scale {
                    // Affine rotation
                    let pa_idx = ((attr1 >> 9) & 0x1F) as usize;
                    let rpa = self.oam_read16(pa_idx * 32 + 6) as i16 as i32;
                    let rpb = self.oam_read16(pa_idx * 32 + 14) as i16 as i32;
                    let rpc = self.oam_read16(pa_idx * 32 + 22) as i16 as i32;
                    let rpd = self.oam_read16(pa_idx * 32 + 30) as i16 as i32;

                    let cx = (disp_w as i32) / 2;
                    let cy = (disp_h as i32) / 2;
                    let tx_fp = rpa * (dx - cx) + rpb * (ty_in_sprite - cy);
                    let ty_fp = rpc * (dx - cx) + rpd * (ty_in_sprite - cy);
                    let atx = (tx_fp >> 8) + (w as i32) / 2;
                    let aty = (ty_fp >> 8) + (h as i32) / 2;
                    if atx < 0 || aty < 0 || atx >= w as i32 || aty >= h as i32 { continue; }
                    tex_x = atx as usize;
                    tex_y = aty as usize;
                } else {
                    tex_x = if hflip { (w as i32 - 1 - dx) as usize } else { dx as usize };
                    tex_y = if vflip { h - 1 - ty_in_sprite as usize } else { ty_in_sprite as usize };
                };

                // Look up pixel color
                let color_idx = if color256 {
                    // 256-color: 1D or 2D tile mapping
                    let tile_off = if (self.dispcnt >> 6) & 1 != 0 {
                        // 1D mapping
                        tile_num * 64 + tex_y * 8 + tex_x
                    } else {
                        // 2D mapping
                        (tile_num + (tex_y / 8) * 32) * 64 + (tex_y & 7) * 8 + tex_x
                    };
                    self.vram_read8(0x10000 + tile_off) as usize
                } else {
                    // 16-color
                    let tile_off = if (self.dispcnt >> 6) & 1 != 0 {
                        // 1D mapping
                        let tile_row = tex_y / 8;
                        let tile_col_base = tile_num + tile_row * (w / 8);
                        let tile_in_row = tex_x / 8;
                        (tile_col_base + tile_in_row) * 32 + (tex_y & 7) * 4 + (tex_x & 7) / 2
                    } else {
                        // 2D mapping
                        (tile_num + (tex_y / 8) * 32 + tex_x / 8) * 32 + (tex_y & 7) * 4 + (tex_x & 7) / 2
                    };
                    let byte = self.vram_read8(0x10000 + tile_off);
                    let nibble = if tex_x & 1 != 0 { (byte >> 4) & 0xF } else { byte & 0xF } as usize;
                    nibble
                };

                if color_idx == 0 { continue; }  // transparent

                let pal_off = if color256 {
                    0x200 + color_idx * 2
                } else {
                    0x200 + palette_num * 32 + color_idx * 2
                };
                let color = self.palette_read16(pal_off);

                if obj_mode != 2 {  // Not OBJ window
                    let px = px_screen as usize;
                    let semi_transparent = obj_mode == 1;
                    // Only draw if higher priority than current
                    if priority <= obj_line[px].1 {
                        obj_line[px] = (color, priority, semi_transparent);
                    }
                }
            }
        }
    }

    fn composite_scanline(
        &mut self,
        y: usize,
        bg_lines: &[[Option<u16>; 240]; 4],
        obj_line: &[(u16, u8, bool); 240],
    ) {
        let mode = self.dispcnt & 0x7;
        let bg_enable = (self.dispcnt >> 8) & 0xF;
        let obj_enable = (self.dispcnt >> 12) & 1 != 0;

        // Window control
        let win0_en = (self.dispcnt >> 13) & 1 != 0;
        let win1_en = (self.dispcnt >> 14) & 1 != 0;
        let winobj_en = (self.dispcnt >> 15) & 1 != 0;
        let any_win = win0_en || win1_en || winobj_en;

        let bld_mode = (self.bldcnt >> 6) & 3;
        let bld_a = self.bldcnt & 0x3F;  // which layers are "1st target"
        let bld_b = (self.bldcnt >> 8) & 0x3F;  // which layers are "2nd target"

        let eva = (self.bldalpha & 0x1F).min(16) as u32;
        let evb = ((self.bldalpha >> 8) & 0x1F).min(16) as u32;
        let evy = (self.bldy & 0x1F).min(16) as u32;

        // Get background priorities
        let mut bg_prio = [0u8; 4];
        for bg in 0..4 {
            bg_prio[bg] = (self.bgcnt[bg] & 3) as u8;
        }

        // Backdrop color
        let backdrop = self.palette_read16(0);

        for x in 0..240usize {
            // Determine window for this pixel
            let win_mask = if any_win {
                self.get_window_mask(x, y, win0_en, win1_en, winobj_en, obj_line[x].0 != 0)
            } else {
                0xFF  // all enabled
            };

            // Collect visible layers in priority order
            // Layer: [priority, layer_id, color, semi_transparent]
            let mut first: Option<(u16, u8, bool)> = None;  // (color, layer, semi_trans)
            let mut second: Option<(u16, u8)> = None;

            // Check each BG and OBJ
            let mut candidates: [(u16, u8, u8, bool); 5] = [(0, 4, 0xFF, false); 5];
            let mut n_candidates = 0;

            // BGS
            for bg in 0..4usize {
                if (bg_enable >> bg) & 1 == 0 { continue; }
                if (win_mask >> bg) & 1 == 0 { continue; }
                if let Some(color) = bg_lines[bg][x] {
                    candidates[n_candidates] = (color, bg_prio[bg], bg as u8, false);
                    n_candidates += 1;
                }
            }

            // OBJ
            if obj_enable && (win_mask >> 4) & 1 != 0 {
                let (obj_col, obj_prio, obj_semi) = obj_line[x];
                if obj_col != 0 || obj_line[x].0 != 0 {
                    // Actually check if there's a valid sprite pixel
                    let has_sprite = obj_line[x].1 < 4;  // priority 4 means no sprite
                    if has_sprite {
                        candidates[n_candidates] = (obj_col, obj_prio, LAYER_OBJ as u8, obj_semi);
                        n_candidates += 1;
                    }
                }
            }

            // Sort by priority (lower number = higher priority), then by layer
            // Simple insertion sort for small arrays
            for i in 1..n_candidates {
                let mut j = i;
                while j > 0 && (candidates[j].1 < candidates[j-1].1 ||
                    (candidates[j].1 == candidates[j-1].1 && (candidates[j].2 as usize) < (candidates[j-1].2 as usize))) {
                    candidates.swap(j, j-1);
                    j -= 1;
                }
            }

            let top_color;
            let top_layer;
            let semi_trans;

            if n_candidates > 0 {
                top_color = candidates[0].0;
                top_layer = candidates[0].2;
                semi_trans = candidates[0].3;
            } else {
                top_color = backdrop;
                top_layer = LAYER_BD as u8;
                semi_trans = false;
            }

            // Apply blending
            let final_color = if (win_mask >> 5) & 1 != 0 {
                // Blend enabled for this pixel
                let is_first_target = (bld_a >> top_layer) & 1 != 0;
                let needs_second = bld_mode != 0 && (is_first_target || semi_trans);

                if needs_second {
                    let second_color = if n_candidates > 1 {
                        let c = candidates[1];
                        if (bld_b >> c.2) & 1 != 0 { Some(c.0) }
                        else { Some(backdrop) }
                    } else { Some(backdrop) };

                    if let Some(sc) = second_color {
                        if semi_trans || (bld_mode == 1 && is_first_target) {
                            alpha_blend(top_color, sc, eva, evb)
                        } else if bld_mode == 2 && is_first_target {
                            brightness_up(top_color, evy)
                        } else if bld_mode == 3 && is_first_target {
                            brightness_down(top_color, evy)
                        } else {
                            top_color
                        }
                    } else { top_color }
                } else if bld_mode == 2 && is_first_target {
                    brightness_up(top_color, evy)
                } else if bld_mode == 3 && is_first_target {
                    brightness_down(top_color, evy)
                } else {
                    top_color
                }
            } else {
                top_color
            };

            self.framebuffer[y * 240 + x] = color555_to_rgba(final_color);
        }
    }

    fn get_window_mask(&self, x: usize, y: usize, win0: bool, win1: bool, winobj: bool, has_obj: bool) -> u8 {
        let x = x as u16;
        let y = y as u16;

        // Window 0 (highest priority)
        if win0 {
            let wx1 = (self.winh[0] >> 8) as u16;
            let wx2 = (self.winh[0] & 0xFF) as u16;
            let wy1 = (self.winv[0] >> 8) as u16;
            let wy2 = (self.winv[0] & 0xFF) as u16;

            let in_x = if wx1 <= wx2 { x >= wx1 && x < wx2 } else { x >= wx1 || x < wx2 };
            let in_y = if wy1 <= wy2 { y >= wy1 && y < wy2 } else { y >= wy1 || y < wy2 };

            if in_x && in_y {
                return (self.winin & 0x3F) as u8;
            }
        }

        // Window 1
        if win1 {
            let wx1 = (self.winh[1] >> 8) as u16;
            let wx2 = (self.winh[1] & 0xFF) as u16;
            let wy1 = (self.winv[1] >> 8) as u16;
            let wy2 = (self.winv[1] & 0xFF) as u16;

            let in_x = if wx1 <= wx2 { x >= wx1 && x < wx2 } else { x >= wx1 || x < wx2 };
            let in_y = if wy1 <= wy2 { y >= wy1 && y < wy2 } else { y >= wy1 || y < wy2 };

            if in_x && in_y {
                return ((self.winin >> 8) & 0x3F) as u8;
            }
        }

        // OBJ window
        if winobj && has_obj {
            return ((self.winout >> 8) & 0x3F) as u8;
        }

        // Outside all windows
        (self.winout & 0x3F) as u8
    }
}

fn sprite_dimensions(shape: u32, size: u32) -> (usize, usize) {
    match (shape, size) {
        (0, 0) => (8, 8),
        (0, 1) => (16, 16),
        (0, 2) => (32, 32),
        (0, 3) => (64, 64),
        (1, 0) => (16, 8),
        (1, 1) => (32, 8),
        (1, 2) => (32, 16),
        (1, 3) => (64, 32),
        (2, 0) => (8, 16),
        (2, 1) => (8, 32),
        (2, 2) => (16, 32),
        (2, 3) => (32, 64),
        _ => (8, 8)
    }
}

fn alpha_blend(c1: u16, c2: u16, eva: u32, evb: u32) -> u16 {
    let r1 = (c1 & 0x1F) as u32;
    let g1 = ((c1 >> 5) & 0x1F) as u32;
    let b1 = ((c1 >> 10) & 0x1F) as u32;
    let r2 = (c2 & 0x1F) as u32;
    let g2 = ((c2 >> 5) & 0x1F) as u32;
    let b2 = ((c2 >> 10) & 0x1F) as u32;

    let r = ((r1 * eva + r2 * evb) / 16).min(31);
    let g = ((g1 * eva + g2 * evb) / 16).min(31);
    let b = ((b1 * eva + b2 * evb) / 16).min(31);
    (r | (g << 5) | (b << 10)) as u16
}

fn brightness_up(c: u16, evy: u32) -> u16 {
    let r = (c & 0x1F) as u32;
    let g = ((c >> 5) & 0x1F) as u32;
    let b = ((c >> 10) & 0x1F) as u32;
    let r = r + (31 - r) * evy / 16;
    let g = g + (31 - g) * evy / 16;
    let b = b + (31 - b) * evy / 16;
    (r.min(31) | (g.min(31) << 5) | (b.min(31) << 10)) as u16
}

fn brightness_down(c: u16, evy: u32) -> u16 {
    let r = (c & 0x1F) as u32;
    let g = ((c >> 5) & 0x1F) as u32;
    let b = ((c >> 10) & 0x1F) as u32;
    let r = r - r * evy / 16;
    let g = g - g * evy / 16;
    let b = b - b * evy / 16;
    (r | (g << 5) | (b << 10)) as u16
}
