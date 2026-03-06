use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/poclimon-title.png");

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let dest = PathBuf::from(&out_dir).join("title_art.rs");

    let mut img = image::open("assets/poclimon-title.png")
        .expect("Failed to open assets/poclimon-title.png")
        .into_rgba8();

    let (w, h) = img.dimensions();

    // Zero out near-white pixels *before* scaling so Lanczos blends against
    // transparency rather than white, eliminating anti-aliased white fringe.
    for y in 0..h {
        for x in 0..w {
            let p = img.get_pixel(x, y);
            if p[3] < 128 || (p[0] > 230 && p[1] > 230 && p[2] > 230) {
                img.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
            }
        }
    }

    // Find tight bounding box of remaining opaque pixels.
    let mut min_x = w;
    let mut max_x = 0u32;
    let mut min_y = h;
    let mut max_y = 0u32;

    for y in 0..h {
        for x in 0..w {
            let p = img.get_pixel(x, y);
            if p[3] >= 128 {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }

    const OUT_W: u32 = 80;
    const OUT_H: u32 = 32; // 32 pixel rows → 16 terminal rows (half-blocks)

    // Canvas: transparent by default.
    let mut canvas = vec![[0u8; 4]; (OUT_W * OUT_H) as usize];

    if max_x >= min_x && max_y >= min_y {
        let crop_w = max_x - min_x + 1;
        let crop_h = max_y - min_y + 1;

        // Scale to fit, preserving aspect ratio.
        let scale_x = OUT_W as f64 / crop_w as f64;
        let scale_y = OUT_H as f64 / crop_h as f64;
        let scale = scale_x.min(scale_y);

        let scaled_w = ((crop_w as f64 * scale).round() as u32).clamp(1, OUT_W);
        let scaled_h = ((crop_h as f64 * scale).round() as u32).clamp(1, OUT_H);

        // Center in canvas.
        let off_x = (OUT_W - scaled_w) / 2;
        let off_y = (OUT_H - scaled_h) / 2;

        // Crop the source image and scale with Lanczos3.
        let cropped = image::imageops::crop_imm(&img, min_x, min_y, crop_w, crop_h).to_image();
        let scaled = image::imageops::resize(
            &cropped,
            scaled_w,
            scaled_h,
            image::imageops::FilterType::Lanczos3,
        );

        for dy in 0..scaled_h {
            for dx in 0..scaled_w {
                let p = scaled.get_pixel(dx, dy);
                let cx = off_x + dx;
                let cy = off_y + dy;
                canvas[(cy * OUT_W + cx) as usize] = [p[0], p[1], p[2], p[3]];
            }
        }
    }

    // Pair rows into half-block cells.
    // Each terminal row = 2 pixel rows (top = fg of ▀, bottom = bg of ▀).
    type PixelPair = (u8, u8, u8, u8, u8, u8, u8, u8);
    let term_rows = OUT_H.div_ceil(2); // 16
    let mut pairs: Vec<PixelPair> = Vec::with_capacity((OUT_W * term_rows) as usize);

    for row in 0..term_rows {
        for col in 0..OUT_W {
            let top = canvas[((row * 2) * OUT_W + col) as usize];
            let bot = if row * 2 + 1 < OUT_H {
                canvas[((row * 2 + 1) * OUT_W + col) as usize]
            } else {
                [0, 0, 0, 0]
            };
            pairs.push((top[0], top[1], top[2], top[3], bot[0], bot[1], bot[2], bot[3]));
        }
    }

    // Emit Rust source.
    let mut code = format!(
        "pub const TITLE_ROWS: usize = {term_rows};\n\
         pub const TITLE_COLS: usize = {OUT_W};\n\
         /// Half-block pixel pairs: (top_r, top_g, top_b, top_a, bot_r, bot_g, bot_b, bot_a)\n\
         #[allow(clippy::unreadable_literal, clippy::type_complexity)]\n\
         pub const TITLE_ART: [(u8,u8,u8,u8,u8,u8,u8,u8); {term_rows} * {OUT_W}] = [\n",
    );

    for (i, &(tr, tg, tb, ta, br, bg, bb, ba)) in pairs.iter().enumerate() {
        code.push_str(&format!("    ({tr},{tg},{tb},{ta},{br},{bg},{bb},{ba}),"));
        if (i + 1) % OUT_W as usize == 0 {
            code.push('\n');
        }
    }
    code.push_str("];\n");

    std::fs::write(&dest, code).unwrap();
}
