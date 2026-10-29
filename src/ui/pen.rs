use super::theme::{
    CURSOR, EXP_BLUE, FENCE_BROWN, FLOWER_YELLOW, GRASS_GREEN, INK, PAPER, text_box,
};
use crate::animation::AnimationState;
use crate::app::App;
use crate::creature::{
    CreatureSlot, Direction, LABEL_H, LABEL_OVERLAP, RECALL_FLASH_SHRINK_DELAY_TICKS, RECALL_TICKS,
    SPRITE_W, debug_log, sprite_stack_h,
};
use crate::kitty_upload::{Placement, placement_at, take_upload};
use crate::sprite_loading::{encode_all_frames, encode_toy_image, sprite_encode_rect};
use rand::Rng;
use ratatui::{
    Frame,
    buffer::{Buffer, Cell},
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use ratatui_image::{Image, picker::Picker, protocol::Protocol};

/// Draw an encoded image. Halfblock images are drawn so their transparent
/// pixels (marked `HALFBLOCK_CLEAR` at encode time) show what was beneath.
fn draw_image(f: &mut Frame<'_>, protocol: &mut Protocol, area: Rect) {
    if !matches!(protocol, Protocol::Halfblocks(_)) {
        f.render_widget(Image::new(protocol), area);
        return;
    }
    let area = area.intersection(f.area());
    let beneath: Vec<Cell> = area
        .positions()
        .map(|p| f.buffer_mut()[p].clone())
        .collect();
    f.render_widget(Image::new(protocol), area);
    restore_halfblock_transparency(f.buffer_mut(), area, &beneath);
}

/// After a halfblock image is drawn over `area`, rewrite cells that are
/// wholly or half transparent: wholly → the cell that was beneath; half → just
/// the opaque half, drawn over the background that was beneath.
fn restore_halfblock_transparency(buf: &mut Buffer, area: Rect, beneath: &[Cell]) {
    const CLEAR: Color = crate::sprite_loading::HALFBLOCK_CLEAR;
    for (pos, under) in area.positions().zip(beneath) {
        let cell = &mut buf[pos];
        let (upper, lower) = match cell.symbol() {
            "▀" => (cell.fg, cell.bg),
            "▄" => (cell.bg, cell.fg),
            " " => (cell.bg, cell.bg),
            _ => continue,
        };
        match (upper == CLEAR, lower == CLEAR) {
            (true, true) => *cell = under.clone(),
            (true, false) => {
                cell.set_symbol("▄").set_fg(lower).set_bg(under.bg);
            }
            (false, true) => {
                cell.set_symbol("▀").set_fg(upper).set_bg(under.bg);
            }
            (false, false) => {}
        }
    }
}

fn sprite_render_pos(
    slot: &CreatureSlot,
    pen_inner: Rect,
    sprite_w: u16,
    sprite_h: u16,
) -> (u16, u16) {
    let render_x = (pen_inner.x + slot.pos_x.round() as u16)
        .min(pen_inner.x + pen_inner.width.saturating_sub(sprite_w));
    let render_y = (pen_inner.y + slot.pos_y.round() as u16)
        .min(pen_inner.y + pen_inner.height.saturating_sub(sprite_stack_h(sprite_h)));
    (render_x, render_y)
}

fn slot_rendered_width(slot: &CreatureSlot, fallback: u16) -> u16 {
    pick_protocol_index(&slot.sprites.encoded, 0, 0, 0)
        .and_then(|(si, di, fi)| slot.sprites.encoded[si][di][fi].as_ref())
        .map(|p| p.size().width)
        .unwrap_or(fallback)
}

/// Render all creatures in a single shared pen.
/// Returns the Kitty image placements drawn this frame; the caller runs
/// `fix_orphan_placeholders` once everything (including popups) is drawn.
pub(super) fn render_pen(
    f: &mut Frame<'_>,
    area: Rect,
    app: &mut App,
    picker: &mut Picker,
) -> Vec<Placement> {
    let count = app.slots.len();
    if count == 0 {
        return Vec::new();
    }

    // Single outer border — no inner dividers.
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(FENCE_BROWN))
        .title(" 🌿 Pen ")
        .title_style(
            Style::default()
                .fg(INK)
                .bg(PAPER)
                .add_modifier(Modifier::BOLD),
        );
    let pen_inner = block.inner(area);
    f.render_widget(block, area);

    // ── Phase 0: Sparse grass + fence posts ───────────────────────────────────
    // Iterate every cell but only call render_widget on the ~7% that are tiles.
    // This keeps render calls to O(tiles) rather than O(width × height) and lets
    // the terminal's own background show through everywhere else.
    for y in pen_inner.y..pen_inner.y + pen_inner.height {
        for x in pen_inner.x..pen_inner.x + pen_inner.width {
            let hash =
                (x as u32).wrapping_mul(2_654_435_761) ^ (y as u32).wrapping_mul(2_246_822_519);
            let (tile, color) = match hash % 45 {
                0 => ("\"", GRASS_GREEN),
                1 => ("'", GRASS_GREEN),
                44 => ("*", FLOWER_YELLOW),
                _ => continue,
            };
            f.render_widget(
                Paragraph::new(Span::styled(tile, Style::default().fg(color))),
                Rect::new(x, y, 1, 1),
            );
        }
    }

    // Fence posts every 4 cols on the top and bottom inner edge rows.
    let fence_style = Style::default().fg(FENCE_BROWN);
    for col in (0..pen_inner.width).step_by(4) {
        f.render_widget(
            Paragraph::new(Span::styled("┃", fence_style)),
            Rect::new(pen_inner.x + col, pen_inner.y, 1, 1),
        );
    }
    if pen_inner.height > 0 {
        let bottom_y = pen_inner.y + pen_inner.height - 1;
        for col in (0..pen_inner.width).step_by(4) {
            f.render_widget(
                Paragraph::new(Span::styled("┃", fence_style)),
                Rect::new(pen_inner.x + col, bottom_y, 1, 1),
            );
        }
    }

    let selected = app.selected;
    let transition_slot_index = app.transition_slot_index();
    let evolving_slot_index = app.evolving_slot_index();
    let transition_state = app
        .swap_transition
        .as_ref()
        .map(|t| (t.slot_index, t.recall_ticks, t.worker_result.is_some()));

    // Sprite size: fixed width; height scales up for halfblock terminals so
    // creatures render at 32×32 "pixels" instead of 20×20.
    // Same size the loader threads encode at, so their frames are used as-is.
    let size_rect = sprite_encode_rect(picker);
    let sprite_w = size_rect.width;
    let sprite_h = size_rect.height;

    // Publish pen dimensions so update_physics can use them next tick.
    app.pen_dims = Some((pen_inner.width, pen_inner.height, sprite_h));

    let mut retired: Vec<u32> = Vec::new();
    for i in 0..count {
        let slot = &mut app.slots[i];

        // First time this slot enters the pen: randomize position and velocity.
        // Gated on `position_initialized` (not `encoded_rect`) so that when
        // background sprite loads complete and replace the slot, the creature
        // doesn't jump to a new random position.
        if !slot.position_initialized {
            let mut rng = rand::thread_rng();
            let max_px = (pen_inner.width.saturating_sub(sprite_w)) as f32;
            let max_py = (pen_inner.height.saturating_sub(sprite_stack_h(sprite_h))) as f32;
            slot.pos_x = rng.gen_range(0.0..=max_px.max(0.0));
            // Staggered vertical start: divide pen height into count slots.
            // Each creature gets its own slot so they start spread out vertically.
            let y_step = if count > 1 {
                max_py / (count - 1) as f32
            } else {
                0.0
            };
            let base_y = i as f32 * y_step;
            // Small random jitter (±20% of step) so they don't look rigidly spaced.
            let jitter = rng.gen_range(-y_step * 0.2..=y_step * 0.2_f32);
            slot.pos_y = (base_y + jitter).clamp(0.0, max_py.max(0.0));
            slot.vel_x = rng.gen_range(-0.4..=0.4_f32);
            slot.vel_y = rng.gen_range(-0.4..=0.4_f32);
            if slot.vel_x.abs() < 0.12 {
                slot.vel_x = if slot.vel_x >= 0.0 { 0.18 } else { -0.18 };
            }
            if slot.vel_y.abs() < 0.12 {
                slot.vel_y = if slot.vel_y >= 0.0 { 0.18 } else { -0.18 };
            }
            slot.dir_hold_ticks = rng.gen_range(40_u32..160);
            slot.position_initialized = true;
        }

        // Lazily encode (or re-encode on resize) — compare size only, not position.
        // Physics updates have been moved to App::update_physics (called each tick).
        // Normally already encoded by the loader thread; this is the fallback
        // for placeholder slots and protocol changes.
        if slot.sprites.encoded_rect != Some(size_rect) {
            retired.append(&mut slot.sprites.kitty_ids);
            encode_all_frames(slot, picker, size_rect);
        }
    }
    app.retire_kitty_images(&retired);

    // Kitty uploads due this frame; written to the terminal before the diff.
    let mut uploads: Vec<String> = Vec::new();
    // Kitty image placements this frame, for the orphaned-placeholder fixup.
    let mut placements: Vec<Placement> = Vec::new();

    // ── Phase 3a: render all sprites ──────────────────────────────────────────────
    for i in 0..count {
        let slot = &mut app.slots[i];

        let state = slot.animator.state();
        let mut frame_idx = slot.animator.current_frame_index().unwrap_or(0);
        let mut state_idx = state.encoded_index();
        // Playing: snap to Left/Right only — hop sprites don't have meaningful
        // Up/Down frames and it looks odd when the creature briefly faces away.
        let dir_idx = if state == AnimationState::Playing {
            if slot.vel_x >= 0.0 {
                Direction::Right.as_index()
            } else {
                Direction::Left.as_index()
            }
        } else {
            slot.current_dir.as_index()
        };

        let (render_x, render_y) = sprite_render_pos(slot, pen_inner, sprite_w, sprite_h);
        let mut img_area = Rect::new(render_x, render_y, sprite_w, sprite_h);

        let is_transition_slot = transition_slot_index == Some(i);
        let mut render_waiting_ball = false;
        let mut white_flash = false;
        if is_transition_slot && let Some((_, recall_ticks, worker_done)) = transition_state {
            if recall_ticks > 0 {
                state_idx = 3; // Recall (Spin/Rotate fallback)
                let elapsed = RECALL_TICKS.saturating_sub(recall_ticks);
                frame_idx = elapsed as usize;
                if elapsed >= RECALL_FLASH_SHRINK_DELAY_TICKS {
                    let shrink_phase = elapsed - RECALL_FLASH_SHRINK_DELAY_TICKS;
                    let shrink_total = (RECALL_TICKS - RECALL_FLASH_SHRINK_DELAY_TICKS).max(1);
                    let scale = 1.0 - (shrink_phase as f32 / shrink_total as f32);
                    let w = ((sprite_w as f32 * scale).round() as u16).clamp(2, sprite_w);
                    let h = ((sprite_h as f32 * scale).round() as u16).clamp(1, sprite_h);
                    let x = render_x + sprite_w.saturating_sub(w) / 2;
                    let y = render_y + sprite_h.saturating_sub(h) / 2;
                    img_area = Rect::new(x, y, w, h);
                    white_flash = shrink_phase.is_multiple_of(2);
                }
            } else if !worker_done {
                render_waiting_ball = true;
            }
        }

        if render_waiting_ball {
            f.render_widget(Clear, img_area);
            f.render_widget(
                Paragraph::new("⚪").style(Style::default().fg(Color::LightRed)),
                Rect::new(
                    render_x + sprite_w.saturating_sub(3) / 2,
                    render_y + sprite_h.saturating_sub(1) / 2,
                    3,
                    1,
                ),
            );
            continue;
        }

        if white_flash {
            let flash = Block::default().style(Style::default().bg(Color::White));
            f.render_widget(Clear, img_area);
            f.render_widget(flash, img_area);
            continue;
        }

        // Evolution flicker: draw a white silhouette, of the old form or (once
        // loaded) the new one. The new form's frames live in the transition
        // until it's applied; uploads go through the same Kitty path.
        let creature_id = slot.creature_id;
        let mut sprites = &mut slot.sprites;
        if evolving_slot_index == Some(i) {
            state_idx = crate::creature::SILHOUETTE_STATE;
            frame_idx = 0;
            if let Some(evo) = app.evolution.as_mut()
                && evo.shows_new_form()
                && let Some(new) = evo.loaded_slot_mut()
            {
                sprites = &mut new.sprites;
            }
            // Silhouettes have transparent gaps; clear what was drawn before.
            f.render_widget(Clear, img_area);
        }

        match pick_protocol_index(&sprites.encoded, state_idx, dir_idx, frame_idx) {
            Some((picked_state, picked_dir, picked_frame)) => {
                if let Some(up) = sprites.kitty_uploads[picked_state][picked_dir]
                    .get_mut(picked_frame)
                    .and_then(Option::take)
                {
                    sprites.kitty_ids.push(up.id);
                    uploads.push(up.seq);
                }
                if let Some(protocol) =
                    sprites.encoded[picked_state][picked_dir][picked_frame].as_mut()
                {
                    draw_image(f, protocol, img_area);
                    placements.extend(placement_at(f.buffer_mut(), img_area, protocol));
                } else {
                    debug_log(format!(
                        "protocol_race_miss id={} state={} dir={} frame={}",
                        slot.creature_id, picked_state, picked_dir, picked_frame
                    ));
                    f.render_widget(Clear, img_area);
                    f.render_widget(
                        Paragraph::new("Loading…").style(Style::default().fg(Color::DarkGray)),
                        img_area,
                    );
                }
            }
            None => {
                debug_log(format!(
                    "protocol_miss id={} state={} dir={} frame={} lens=[{}/{}/{}/{}]",
                    creature_id,
                    state_idx,
                    dir_idx,
                    frame_idx,
                    sprites.encoded[state_idx][0].len(),
                    sprites.encoded[state_idx][1].len(),
                    sprites.encoded[state_idx][2].len(),
                    sprites.encoded[state_idx][3].len()
                ));
                f.render_widget(Clear, img_area);
                f.render_widget(
                    Paragraph::new("Loading…").style(Style::default().fg(Color::DarkGray)),
                    img_area,
                );
            }
        }
    }

    // ── Phase 3b: render compact bordered nameplates ──────────────────────────────
    for i in 0..count {
        let slot = &app.slots[i];

        let (render_x, render_y) = sprite_render_pos(slot, pen_inner, sprite_w, sprite_h);

        let is_selected = selected == i;

        // Build name and level display strings.
        let name_display = format!(
            "{} {}",
            if is_selected { "◉" } else { " " },
            slot.creature_name.to_uppercase()
        );
        // Row 2: show level + XP bar when in an active (XP-earning) state;
        // just the level when Idle, to keep the wandering nameplate compact.
        let level_display = {
            let threshold = crate::creature::xp_to_next_level(slot.level);
            let filled = (slot.xp * 8)
                .checked_div(threshold)
                .map_or(0, |f| f.min(8) as usize);
            let bar: String = "▓".repeat(filled) + &"░".repeat(8 - filled);
            let lv = if slot.ready_to_evolve() {
                format!("Lv.{} ✨", slot.level)
            } else {
                format!("Lv.{}", slot.level)
            };
            let icon = match slot.animator.state() {
                AnimationState::Idle => None,
                AnimationState::Eating => Some("🍖"),
                AnimationState::Sleeping => Some("💤"),
                AnimationState::Playing => Some("🧸"),
            };
            // EXP bar in the Gen 2 battle HUD's blue.
            match icon {
                None => ratatui::text::Line::from(lv),
                Some(icon) => ratatui::text::Line::from(vec![
                    Span::raw(format!("{lv} [")),
                    Span::styled(bar, Style::default().fg(EXP_BLUE)),
                    Span::raw(format!("] {icon}")),
                ]),
            }
        };

        // Auto-size width to content, clamped to [8, SPRITE_W]. Display width,
        // not char count: the emoji are two cells wide.
        let display_w = |s: &str| ratatui::text::Line::from(s).width();
        let content_w = display_w(&name_display).max(level_display.width()) as u16;
        let label_w = (content_w + 2).clamp(8, SPRITE_W); // +2 for left/right borders

        // Center under the actual rendered sprite width, not the full cell area.
        // Kitty/Sixel/iTerm2 encode sprites narrower than sprite_w due to aspect
        // ratio (e.g. a square sprite in a 32×10 area only fills ~18 columns).
        // Halfblock always fills the full sprite_w. Use Protocol::area().width to
        // get the true rendered width rather than guessing from font metrics.
        let actual_sprite_w = slot_rendered_width(slot, sprite_w);
        let label_x = render_x + (actual_sprite_w.saturating_sub(label_w) / 2);
        let label_y = render_y + sprite_h.saturating_sub(LABEL_OVERLAP);

        if label_y + LABEL_H <= pen_inner.y + pen_inner.height {
            let label_area = Rect::new(label_x, label_y, label_w, LABEL_H);
            let block = text_box().border_style(Style::default().fg(if is_selected {
                CURSOR
            } else {
                INK
            }));
            let inner = block.inner(label_area);
            // Clear first: a styled Block only restyles cells, so sprite cells
            // underneath (Kitty placeholders, halfblocks) would survive with the
            // nameplate's colours and show as image garbage inside the label.
            f.render_widget(Clear, label_area);
            f.render_widget(block, label_area);

            let row1 = Rect::new(inner.x, inner.y, inner.width, 1);
            let row2 = Rect::new(inner.x, inner.y + 1, inner.width, 1);
            f.render_widget(
                Paragraph::new(name_display).style(Style::default().add_modifier(if is_selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                })),
                row1,
            );
            f.render_widget(Paragraph::new(level_display), row2);
        }
    }

    // ── Phase 3c: poke-doll sprite for Playing creatures ─────────────────────────
    // Encoded once per terminal size; placed beside the creature in its facing direction.

    const TOY_W: u16 = SPRITE_W / 2; // 16 cols
    let toy_h = sprite_h / 2;
    let toy_size_rect = Rect::new(0, 0, TOY_W, toy_h);

    // Lazily encode (or re-encode on terminal resize / protocol-type change).
    if app.toy_proto_rect != Some(toy_size_rect) {
        let img = app.toy_image.clone();
        app.toy_proto = encode_toy_image(&img, picker, toy_size_rect);
        app.toy_proto_rect = Some(toy_size_rect);
        if let Some(old) = app.toy_kitty_id.take() {
            app.retire_kitty_images(&[old]);
        }
        app.toy_upload = app.toy_proto.as_ref().and_then(take_upload);
    }

    for i in 0..count {
        let slot = &app.slots[i];
        if slot.animator.state() != AnimationState::Playing {
            continue;
        }

        // Recompute render position (same formula as Phase 3a/3b).
        let (render_x, render_y) = sprite_render_pos(slot, pen_inner, sprite_w, sprite_h);

        let rx = render_x as i32;
        let ry = render_y as i32;
        let sw = sprite_w as i32;
        let sh = sprite_h as i32;
        let tw = TOY_W as i32;
        let th = toy_h as i32;

        // East-facing (vel_x >= 0): face/toy right
        // Clamp to terminal bounds for a valid u16 Rect; ratatui clips at the edge.
        //
        // INSET pulls the toy inside the sprite's bounding box to close the gap
        // caused by transparent padding around the PMDCollab sprite art. Without
        // it the toy renders at the bounding-box edge, far from the visible art.
        const INSET: i32 = 4;
        // Keep the toy inside the pen so it never draws over the fence/border.
        let pen_x0 = pen_inner.x as i32;
        let pen_y0 = pen_inner.y as i32;
        let pen_x1 = pen_x0 + pen_inner.width as i32;
        let pen_y1 = pen_y0 + pen_inner.height as i32;
        let toy_mid_y = ry + sh / 2 - th / 2;
        let toy_x_ideal = if slot.vel_x >= 0.0 {
            rx + sw - INSET
        } else {
            rx - tw + INSET
        };
        let toy_x = toy_x_ideal.clamp(pen_x0, (pen_x1 - tw).max(pen_x0));
        let toy_y = toy_mid_y.clamp(pen_y0, (pen_y1 - th).max(pen_y0));

        if let Some(up) = app.toy_upload.take() {
            app.toy_kitty_id = Some(up.id);
            uploads.push(up.seq);
        }
        if let Some(proto) = app.toy_proto.as_mut() {
            let toy_area = Rect::new(toy_x as u16, toy_y as u16, TOY_W, toy_h);
            draw_image(f, proto, toy_area);
            placements.extend(placement_at(f.buffer_mut(), toy_area, proto));
        }
    }
    app.term_writes.extend(uploads);
    placements
}

/// Pick a renderable protocol frame with fallbacks:
/// 1) requested state+dir with wrapped frame index
/// 2) any frame in requested state+dir
/// 3) any direction in requested state
/// 4) any direction in Idle state
///
/// States: 0=Idle, 1=Eat, 2=Sleep, 3=Recall, 4=Playing (Hop), 5=Silhouette.
fn pick_protocol_index(
    encoded: &[[Vec<Option<Protocol>>; 4]; crate::creature::ENCODED_STATES],
    state_idx: usize,
    dir_idx: usize,
    frame_idx: usize,
) -> Option<(usize, usize, usize)> {
    fn pick_from_dir_index(dir_frames: &[Option<Protocol>], frame_idx: usize) -> Option<usize> {
        if dir_frames.is_empty() {
            return None;
        }
        let wrapped = frame_idx % dir_frames.len();
        if dir_frames[wrapped].is_some() {
            return Some(wrapped);
        }
        dir_frames.iter().position(|o| o.is_some())
    }

    for s in [state_idx, 0] {
        if let Some(fi) = pick_from_dir_index(&encoded[s][dir_idx], frame_idx) {
            return Some((s, dir_idx, fi));
        }
        for (d, enc_d) in encoded[s].iter().enumerate().take(4) {
            if d == dir_idx {
                continue;
            }
            if let Some(fi) = pick_from_dir_index(enc_d, frame_idx) {
                return Some((s, d, fi));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn halfblock_transparency_shows_what_is_beneath() {
        // 2×4 px = 2×2 cells. Column 0: opaque red on top, transparent below.
        // Column 1: transparent top pixel over a blue one, then transparent.
        let red = image::Rgba([200, 0, 0, 255]);
        let blue = image::Rgba([0, 0, 200, 255]);
        let mut img = image::RgbaImage::new(2, 4);
        img.put_pixel(0, 0, red);
        img.put_pixel(0, 1, red);
        img.put_pixel(1, 1, blue);
        let area = Rect::new(0, 0, 2, 2);
        let mut proto = crate::sprite_loading::halfblocks_keeping_transparency(img, area).unwrap();

        let mut term = Terminal::new(TestBackend::new(2, 2)).unwrap();
        term.draw(|f| {
            let grass = Style::default().fg(Color::Green).bg(Color::Rgb(9, 9, 9));
            f.render_widget(Paragraph::new("xx\nxx").style(grass), area);
            draw_image(f, &mut proto, area);
        })
        .unwrap();
        let buf = term.backend().buffer();

        // Fully opaque cell: unchanged.
        assert_eq!(buf[(0, 0)].bg, Color::Rgb(200, 0, 0));
        // Top half transparent: only the lower half is drawn, over the grass bg.
        assert_eq!(buf[(1, 0)].symbol(), "▄");
        assert_eq!(buf[(1, 0)].fg, Color::Rgb(0, 0, 200));
        assert_eq!(buf[(1, 0)].bg, Color::Rgb(9, 9, 9));
        // Fully transparent row: the grass is back.
        for x in 0..2 {
            assert_eq!(buf[(x, 1)].symbol(), "x");
            assert_eq!(buf[(x, 1)].fg, Color::Green);
        }
    }
}
