use super::theme::{FENCE_BROWN, GB_DARK, GB_DARKEST, GB_LIGHT, GB_LIGHTEST, GRASS_GREEN};
use crate::animation::AnimationState;
use crate::app::App;
use crate::creature::{
    CreatureSlot, Direction, LABEL_H, LABEL_OVERLAP, RECALL_FLASH_SHRINK_DELAY_TICKS, RECALL_TICKS,
    SPRITE_H, SPRITE_H_HALFBLOCKS, SPRITE_W, debug_log, sprite_stack_h,
};
use crate::sprite_loading::{encode_all_frames, encode_toy_image};
use rand::Rng;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};
use ratatui_image::{
    Image,
    picker::{Picker, ProtocolType},
    protocol::Protocol,
};

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
        .map(|p| p.area().width)
        .unwrap_or(fallback)
}

/// Render all creatures in a single shared pen.
pub(super) fn render_pen(f: &mut Frame<'_>, area: Rect, app: &mut App, picker: &mut Picker) {
    let count = app.slots.len();
    if count == 0 {
        return;
    }

    // Single outer border — no inner dividers.
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(FENCE_BROWN))
        .style(Style::default().bg(GB_DARKEST))
        .title(" 🌿 Pen ")
        .title_style(
            Style::default()
                .fg(GB_LIGHTEST)
                .add_modifier(Modifier::BOLD),
        );
    let pen_inner = block.inner(area);
    f.render_widget(block, area);

    // ── Phase 0: Background + grass decorations ───────────────────────────────
    // 0a. Solid background fill
    f.render_widget(
        Block::default().style(Style::default().bg(GB_DARKEST)),
        pen_inner,
    );

    // 0b. Short grass tiles — deterministic per cell position
    for row in 0..pen_inner.height {
        let y = pen_inner.y + row;
        let spans: Vec<Span> = (0..pen_inner.width)
            .map(|col| {
                let x = pen_inner.x + col;
                let hash =
                    (x as u32).wrapping_mul(2_654_435_761) ^ (y as u32).wrapping_mul(2_246_822_519);
                match hash % 9 {
                    0 => Span::styled("\"", Style::default().fg(GRASS_GREEN).bg(GB_DARKEST)),
                    1 => Span::styled("'", Style::default().fg(GRASS_GREEN).bg(GB_DARKEST)),
                    8 => Span::styled(
                        "*",
                        Style::default().fg(Color::Rgb(200, 200, 50)).bg(GB_DARKEST),
                    ),
                    _ => Span::styled(" ", Style::default().bg(GB_DARKEST)),
                }
            })
            .collect();
        f.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(pen_inner.x, y, pen_inner.width, 1),
        );
    }

    // 0c. Fence posts along top and bottom inner edge rows, every 4 columns
    for col in (0..pen_inner.width).step_by(4) {
        f.render_widget(
            Paragraph::new(Span::styled(
                "┃",
                Style::default().fg(FENCE_BROWN).bg(GB_DARKEST),
            )),
            Rect::new(pen_inner.x + col, pen_inner.y, 1, 1),
        );
    }
    if pen_inner.height > 0 {
        let bottom_y = pen_inner.y + pen_inner.height - 1;
        for col in (0..pen_inner.width).step_by(4) {
            f.render_widget(
                Paragraph::new(Span::styled(
                    "┃",
                    Style::default().fg(FENCE_BROWN).bg(GB_DARKEST),
                )),
                Rect::new(pen_inner.x + col, bottom_y, 1, 1),
            );
        }
    }

    let selected = app.selected;
    let transition_slot_index = app.transition_slot_index();
    let transition_state = app
        .swap_transition
        .as_ref()
        .map(|t| (t.slot_index, t.recall_ticks, t.worker_result.is_some()));

    // Sprite size: fixed width; height scales up for halfblock terminals so
    // creatures render at 32×32 "pixels" instead of 20×20.
    let sprite_w = SPRITE_W;
    let sprite_h = if picker.protocol_type() == ProtocolType::Halfblocks {
        SPRITE_H_HALFBLOCKS
    } else {
        SPRITE_H
    };

    // Publish pen dimensions so update_physics can use them next tick.
    app.pen_dims = Some((pen_inner.width, pen_inner.height, sprite_h));

    // Size rect used for protocol encoding (position 0,0 — decoupled from render pos).
    let size_rect = Rect::new(0, 0, sprite_w, sprite_h);

    for i in 0..count {
        let slot = &mut app.slots[i];

        // First time this slot enters the pen: randomize position and velocity.
        if slot.sprites.encoded_rect.is_none() {
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
        }

        // Lazily encode (or re-encode on resize) — compare size only, not position.
        // Physics updates have been moved to App::update_physics (called each tick).
        if slot.sprites.encoded_rect != Some(size_rect) {
            encode_all_frames(slot, picker, size_rect);
        }
    }

    // ── Phase 3a: render all sprites ──────────────────────────────────────────────
    for i in 0..count {
        let slot = &mut app.slots[i];

        let state = slot.animator.state();
        let mut frame_idx = slot.animator.current_frame_index().unwrap_or(0);
        let mut state_idx = state.encoded_index();
        // Playing: snap to Left/Right only — hop sprites don't have meaningful
        // Up/Down frames and it looks odd when the creature briefly faces away.
        // Use the current horizontal velocity to pick the side, so it stays
        // consistent with where the creature will hop next.
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
            f.render_widget(flash, img_area);
            continue;
        }

        match pick_protocol_index(&slot.sprites.encoded, state_idx, dir_idx, frame_idx) {
            Some((picked_state, picked_dir, picked_frame)) => {
                if let Some(protocol) =
                    slot.sprites.encoded[picked_state][picked_dir][picked_frame].as_mut()
                {
                    f.render_widget(Image::new(protocol), img_area);
                } else {
                    debug_log(format!(
                        "protocol_race_miss id={} state={} dir={} frame={}",
                        slot.creature_id, picked_state, picked_dir, picked_frame
                    ));
                    f.render_widget(
                        Paragraph::new("Loading…").style(Style::default().fg(Color::DarkGray)),
                        img_area,
                    );
                }
            }
            None => {
                debug_log(format!(
                    "protocol_miss id={} state={} dir={} frame={} lens=[{}/{}/{}/{}]",
                    slot.creature_id,
                    state_idx,
                    dir_idx,
                    frame_idx,
                    slot.sprites.encoded[state_idx][0].len(),
                    slot.sprites.encoded[state_idx][1].len(),
                    slot.sprites.encoded[state_idx][2].len(),
                    slot.sprites.encoded[state_idx][3].len()
                ));
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
        // just the level when Idle (no XP accrues while Idle).
        let level_display = {
            let threshold = 50 * slot.level;
            let filled = if threshold > 0 {
                (slot.xp * 8 / threshold).min(8) as usize
            } else {
                0
            };
            let bar: String = "▓".repeat(filled) + &"░".repeat(8 - filled);
            match slot.animator.state() {
                AnimationState::Idle => format!("Lv.{}", slot.level),
                AnimationState::Eating => format!("Lv.{} [{}] 🍖", slot.level, bar),
                AnimationState::Sleeping => format!("Lv.{} [{}] 💤", slot.level, bar),
                AnimationState::Playing => format!("Lv.{} [{}] 🧸", slot.level, bar),
            }
        };

        // Auto-size width to content, clamped to [8, SPRITE_W].
        let content_w = name_display
            .chars()
            .count()
            .max(level_display.chars().count()) as u16;
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
            let name_color = if is_selected { GB_LIGHTEST } else { GB_LIGHT };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Thick)
                .border_style(Style::default().fg(if is_selected { GB_LIGHTEST } else { GB_DARK }))
                .style(Style::default().bg(GB_DARKEST));
            let inner = block.inner(label_area);
            f.render_widget(block, label_area);

            let row1 = Rect::new(inner.x, inner.y, inner.width, 1);
            let row2 = Rect::new(inner.x, inner.y + 1, inner.width, 1);
            f.render_widget(
                Paragraph::new(name_display).style(Style::default().fg(name_color).add_modifier(
                    if is_selected {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    },
                )),
                row1,
            );
            f.render_widget(
                Paragraph::new(level_display).style(Style::default().fg(GB_LIGHT)),
                row2,
            );
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
    }

    for i in 0..count {
        let slot = &app.slots[i];
        if slot.animator.state() != AnimationState::Playing {
            continue;
        }

        // Recompute render position (same formula as Phase 3a/3b).
        let (render_x, render_y) = sprite_render_pos(slot, pen_inner, sprite_w, sprite_h);

        // Use the true rendered sprite width (same as nameplate logic).
        let actual_sprite_w = slot_rendered_width(slot, sprite_w);

        let rx = render_x as i32;
        let ry = render_y as i32;
        let sw = actual_sprite_w as i32;
        let sh = sprite_h as i32;
        let tw = TOY_W as i32;
        let th = toy_h as i32;

        // Mirror the dir_idx snap above: Playing is always Left or Right.
        // Pull the toy 4 cells into the sprite allocation to close any gap
        // from transparent padding around the PMDCollab sprite art.
        const INSET: i32 = 4;
        let toy_mid_y = ry + sh / 2 - th / 2;
        let (toy_x, toy_y) = if slot.vel_x >= 0.0 {
            // Facing right → toy in front, to the right
            (rx + sw - INSET, toy_mid_y)
        } else {
            // Facing left → toy in front, to the left
            (rx - tw + INSET, toy_mid_y)
        };

        let px = pen_inner.x as i32;
        let py = pen_inner.y as i32;
        let fits = toy_x >= px
            && toy_y >= py
            && toy_x + tw <= px + pen_inner.width as i32
            && toy_y + th <= py + pen_inner.height as i32;

        if fits && let Some(proto) = app.toy_proto.as_mut() {
            f.render_widget(
                Image::new(proto),
                Rect::new(toy_x as u16, toy_y as u16, TOY_W, toy_h),
            );
        }
    }
}

/// Pick a renderable protocol frame with fallbacks:
/// 1) requested state+dir with wrapped frame index
/// 2) any frame in requested state+dir
/// 3) any direction in requested state
/// 4) any direction in Idle state
///
/// The array has 5 states: 0=Idle, 1=Eat, 2=Sleep, 3=Recall, 4=Playing (Hop).
fn pick_protocol_index(
    encoded: &[[Vec<Option<Protocol>>; 4]; 5],
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
