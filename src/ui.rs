use crate::animation::AnimationState;
use crate::app::{App, PromptMode};
use crate::creature::{LABEL_H, LABEL_OVERLAP, SPRITE_H, SPRITE_H_HALFBLOCKS, SPRITE_W};
use crate::notification::NotifLevel;
use crate::sprite_loading::encode_all_frames;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use ratatui_image::{
    Image,
    picker::{Picker, ProtocolType},
    protocol::Protocol,
};

pub fn ui(f: &mut Frame<'_>, app: &mut App, picker: &mut Picker, version: &str) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // Title bar
        Constraint::Min(10),   // Pen (shared creature canvas)
        Constraint::Length(5), // Status + notifications (3 inner rows)
        Constraint::Length(3), // Help bar
    ])
    .split(f.area());

    // Collect data before mutable borrows.
    let selected_name: String = app
        .slots
        .get(app.selected)
        .map(|s| s.creature_name.clone())
        .unwrap_or_else(|| "???".to_string());

    let title = Paragraph::new(Line::from(vec![
        Span::styled("PoCLImon", Style::default().fg(Color::Yellow)),
        Span::raw(" — "),
        Span::styled(
            format!("{} creatures", app.slots.len()),
            Style::default().fg(Color::LightYellow),
        ),
        Span::styled(
            format!(" [selected: {selected_name}]"),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("⚡ PoCLImon {version}")),
    );
    f.render_widget(title, chunks[0]);

    // Gather status info before mutable borrow.
    let state_label = app
        .slots
        .get(app.selected)
        .map(|s| match s.animator.state() {
            AnimationState::Idle => "Idle",
            AnimationState::Eating => "Nomming 🍖",
            AnimationState::Sleeping => "Sleeping 💤",
            AnimationState::Playing => "Playing 🧸",
        })
        .unwrap_or("—");
    let status_color = app
        .slots
        .get(app.selected)
        .map(|s| match s.animator.state() {
            AnimationState::Idle => Color::Green,
            AnimationState::Eating => Color::Yellow,
            AnimationState::Sleeping => Color::Blue,
            // Magenta distinguishes Playing from other states at a glance.
            AnimationState::Playing => Color::Magenta,
        })
        .unwrap_or(Color::White);

    // Shared pen — all creatures on one open canvas.
    render_pen(f, chunks[1], app, picker);

    // Status + notification panel.
    // Line 0: current creature state.
    // Lines 1–2: most recent notifications, newest first.
    let mut status_lines = vec![Line::from(vec![
        Span::raw(format!("{selected_name}: ")),
        Span::styled(state_label, Style::default().fg(status_color)),
    ])];
    if let Some(transition) = app.swap_transition.as_ref() {
        let action = if transition.recall_ticks > 0 {
            "Recalling"
        } else {
            "Loading"
        };
        let display_name = if transition.recall_ticks > 0 {
            // Show the outgoing Pokémon during recall
            app.slots
                .get(transition.slot_index)
                .map(|s| s.creature_name.clone())
                .unwrap_or_else(|| "???".to_string())
        } else {
            // Show the incoming Pokémon during load
            transition.target_name.clone()
        };
        status_lines.push(Line::from(vec![
            Span::styled("[Swap]  ", Style::default().fg(Color::LightMagenta)),
            Span::styled(
                format!("{action} {display_name}..."),
                Style::default().fg(Color::LightMagenta),
            ),
        ]));
    } else if let Some(transition) = app.add_transition.as_ref() {
        status_lines.push(Line::from(vec![
            Span::styled("[Add]   ", Style::default().fg(Color::LightMagenta)),
            Span::styled(
                format!("Loading {}...", transition.target_name),
                Style::default().fg(Color::LightMagenta),
            ),
        ]));
    }

    let notif_rows = if app.swap_transition.is_some() || app.add_transition.is_some() {
        1
    } else {
        2
    };
    for notif in app.notifications.iter().rev().take(notif_rows) {
        let (prefix, color) = match notif.level {
            NotifLevel::Error => ("[Error] ", Color::Red),
            NotifLevel::Warn => ("[Warn]  ", Color::Yellow),
            NotifLevel::Info => ("[Info]  ", Color::DarkGray),
        };
        status_lines.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(color)),
            Span::styled(notif.message.clone(), Style::default().fg(color)),
        ]));
    }

    let status = Paragraph::new(status_lines).block(Block::default().borders(Borders::ALL));
    f.render_widget(status, chunks[2]);

    // Help bar
    let help =
        Paragraph::new("[E]at [S]leep [I]dle [P]lay [←/→] [1-6] [A]dd # [Tab]Swap # [R]emove [Q]uit")
            .style(Style::default().fg(Color::DarkGray))
            .block(Block::default().borders(Borders::ALL));
    f.render_widget(help, chunks[3]);

    // Prompt overlay — appears centered over the pen area when Add or Swap is active.
    if app.prompt_mode != PromptMode::None {
        let title = match app.prompt_mode {
            PromptMode::Add => " Add Pokémon ",
            PromptMode::Swap => " Swap to Pokémon ",
            PromptMode::None => "",
        };
        // Small centered popup: 32 wide, 4 tall.
        let popup_w = 32u16;
        let popup_h = 4u16;
        let popup_x = chunks[1].x + (chunks[1].width.saturating_sub(popup_w)) / 2;
        let popup_y = chunks[1].y + (chunks[1].height.saturating_sub(popup_h)) / 2;
        let popup_area = Rect::new(popup_x, popup_y, popup_w, popup_h);

        // Clear background (overwrite pen content in this area).
        f.render_widget(Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Yellow));
        let inner = block.inner(popup_area);
        f.render_widget(block, popup_area);

        // Line 1: input field.
        let input_text = format!("Pokédex #: {}█", app.prompt_buffer);
        let row1 = Rect::new(inner.x, inner.y, inner.width, 1);
        f.render_widget(
            Paragraph::new(input_text).style(Style::default().fg(Color::White)),
            row1,
        );

        // Line 2: hint.
        let row2 = Rect::new(inner.x, inner.y + 1, inner.width, 1);
        f.render_widget(
            Paragraph::new("[Enter] confirm  [Esc] cancel")
                .style(Style::default().fg(Color::DarkGray)),
            row2,
        );
    }
}

// ── Shared pen rendering ───────────────────────────────────────────────────────

/// Render all creatures in a single shared pen with no internal borders.
///
/// A single outer border wraps the pen area. Creatures wander freely using
/// `pos_x`/`pos_y` + `vel_x`/`vel_y`, bouncing off walls, overlapping freely
/// (later slots render on top). Name labels follow each sprite.
///
/// Sprite protocols are encoded at a fixed size `(sprite_w, sprite_h)` at
/// position `(0,0)` — position-independent. They are only re-encoded when
/// pen size changes (terminal resize). At render time the widget is
/// placed at the creature's current position.
pub fn render_pen(f: &mut Frame<'_>, area: Rect, app: &mut App, picker: &mut Picker) {
    let count = app.slots.len();
    if count == 0 {
        return;
    }

    // Single outer border — no inner dividers.
    let block = Block::default().borders(Borders::ALL).title("🌿 Pen");
    let pen_inner = block.inner(area);
    f.render_widget(block, area);

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

    // Size rect used for protocol encoding (position 0,0 — decoupled from render pos).
    let size_rect = Rect::new(0, 0, sprite_w, sprite_h);

    // Phase 1: initialize positions, update movement, and set direction for all slots.
    for i in 0..count {
        let slot = &mut app.slots[i];

        // First time this slot enters the pen: randomize position and velocity.
        if slot.encoded_rect.is_none() {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            let max_px = (pen_inner.width.saturating_sub(sprite_w)) as f32;
            let max_py = (pen_inner
                .height
                .saturating_sub(crate::creature::sprite_stack_h(sprite_h)))
                as f32;
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

        // Update position for this tick (frozen during eating/sleeping).
        // Direction is locked inside update_position when a new heading is picked.
        let is_moving = matches!(slot.animator.state(), AnimationState::Idle)
            && transition_slot_index != Some(i);
        slot.update_position(
            pen_inner.width,
            pen_inner.height,
            sprite_w,
            sprite_h,
            is_moving,
        );
        if slot.current_dir > 3 {
            crate::creature::debug_log(format!(
                "bad_dir id={} dir={} vx={:.3} vy={:.3}",
                slot.creature_id, slot.current_dir, slot.vel_x, slot.vel_y
            ));
            slot.current_dir =
                crate::creature::stable_velocity_to_dir(slot.vel_x, slot.vel_y, slot.current_dir);
        }

        // Lazily encode (or re-encode on resize) — compare size only, not position.
        if slot.encoded_rect != Some(size_rect) {
            encode_all_frames(slot, picker, size_rect);
        }
    }

    // Phase 2: resolve creature-to-creature collisions (elastic bounce).
    crate::creature::resolve_collisions(
        &mut app.slots,
        SPRITE_W,
        crate::creature::sprite_stack_h(sprite_h),
        pen_inner.width,
        pen_inner.height,
    );

    for slot in &mut app.slots {
        crate::creature::maybe_update_facing_from_velocity(slot);
    }

    // ── Phase 3a: render all sprites ──────────────────────────────────────────────
    for i in 0..count {
        let slot = &mut app.slots[i];

        let state = slot.animator.state();
        let mut frame_idx = slot.animator.current_frame_index().unwrap_or(0);
        // Map AnimationState → encoded_frames index.
        // Must stay in sync with the order in encode_all_frames:
        //   0=Idle, 1=Eat, 2=Sleep, 3=Recall, 4=Playing.
        let mut state_idx = match state {
            AnimationState::Idle => 0,
            AnimationState::Eating => 1,
            AnimationState::Sleeping => 2,
            AnimationState::Playing => 4,
        };
        let dir_idx = slot.current_dir;

        let render_x = (pen_inner.x + slot.pos_x.round() as u16)
            .min(pen_inner.x + pen_inner.width.saturating_sub(sprite_w));
        let render_y = (pen_inner.y + slot.pos_y.round() as u16).min(
            pen_inner.y
                + pen_inner
                    .height
                    .saturating_sub(crate::creature::sprite_stack_h(sprite_h)),
        );
        let mut img_area = Rect::new(render_x, render_y, sprite_w, sprite_h);

        let is_transition_slot = transition_slot_index == Some(i);
        let mut render_waiting_ball = false;
        let mut white_flash = false;
        if is_transition_slot && let Some((_, recall_ticks, worker_done)) = transition_state {
            if recall_ticks > 0 {
                state_idx = 3; // Recall (Spin/Rotate fallback)
                let elapsed = crate::creature::RECALL_TICKS.saturating_sub(recall_ticks);
                frame_idx = elapsed as usize;
                if elapsed >= crate::creature::RECALL_FLASH_SHRINK_DELAY_TICKS {
                    let shrink_phase = elapsed - crate::creature::RECALL_FLASH_SHRINK_DELAY_TICKS;
                    let shrink_total = (crate::creature::RECALL_TICKS
                        - crate::creature::RECALL_FLASH_SHRINK_DELAY_TICKS)
                        .max(1);
                    let scale = 1.0 - (shrink_phase as f32 / shrink_total as f32);
                    let w = ((sprite_w as f32 * scale).round() as u16).clamp(2, sprite_w);
                    let h = ((sprite_h as f32 * scale).round() as u16).clamp(1, sprite_h);
                    let x = render_x + sprite_w.saturating_sub(w) / 2;
                    let y = render_y + sprite_h.saturating_sub(h) / 2;
                    img_area = Rect::new(x, y, w, h);
                    white_flash = shrink_phase % 2 == 0;
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

        match pick_protocol_index(&slot.encoded_frames, state_idx, dir_idx, frame_idx) {
            Some((picked_state, picked_dir, picked_frame)) => {
                if let Some(protocol) =
                    slot.encoded_frames[picked_state][picked_dir][picked_frame].as_mut()
                {
                    f.render_widget(Image::new(protocol), img_area);
                } else {
                    crate::creature::debug_log(format!(
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
                crate::creature::debug_log(format!(
                    "protocol_miss id={} state={} dir={} frame={} lens=[{}/{}/{}/{}]",
                    slot.creature_id,
                    state_idx,
                    dir_idx,
                    frame_idx,
                    slot.encoded_frames[state_idx][0].len(),
                    slot.encoded_frames[state_idx][1].len(),
                    slot.encoded_frames[state_idx][2].len(),
                    slot.encoded_frames[state_idx][3].len()
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

        let render_x = (pen_inner.x + slot.pos_x.round() as u16)
            .min(pen_inner.x + pen_inner.width.saturating_sub(sprite_w));
        let render_y = (pen_inner.y + slot.pos_y.round() as u16).min(
            pen_inner.y
                + pen_inner
                    .height
                    .saturating_sub(crate::creature::sprite_stack_h(sprite_h)),
        );

        let is_selected = selected == i;

        // Build name and level display strings.
        let name_display = format!(
            "{} {}",
            if is_selected { "◉" } else { " " },
            slot.creature_name.to_uppercase()
        );
        // Row 2: show level + XP progress when in an active (XP-earning or
        // sleeping) state; just the level when Idle (no XP accrues while Idle).
        let level_display = {
            let threshold = 50 * slot.level;
            match slot.animator.state() {
                // Idle: just show level — no XP bar since XP doesn't accrue.
                AnimationState::Idle => format!("Lv.{}", slot.level),
                // Active states: show level, XP progress, and emoji icon.
                AnimationState::Eating => {
                    format!("Lv.{}  {}/{}xp  🍖", slot.level, slot.xp, threshold)
                }
                AnimationState::Sleeping => {
                    format!("Lv.{}  {}/{}xp  💤", slot.level, slot.xp, threshold)
                }
                AnimationState::Playing => {
                    format!("Lv.{}  {}/{}xp  🧸", slot.level, slot.xp, threshold)
                }
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
        let actual_sprite_w = pick_protocol_index(&slot.encoded_frames, 0, 0, 0)
            .and_then(|(si, di, fi)| slot.encoded_frames[si][di][fi].as_ref())
            .map(|p| p.area().width)
            .unwrap_or(sprite_w);
        let label_x = render_x + (actual_sprite_w.saturating_sub(label_w) / 2);
        let label_y = render_y + sprite_h.saturating_sub(LABEL_OVERLAP);

        if label_y + LABEL_H <= pen_inner.y + pen_inner.height {
            let label_area = Rect::new(label_x, label_y, label_w, LABEL_H);
            let name_color = if is_selected {
                Color::Yellow
            } else {
                Color::White
            };
            let block = Block::default().borders(Borders::ALL);
            let inner = block.inner(label_area);
            f.render_widget(block, label_area);

            let row1 = Rect::new(inner.x, inner.y, inner.width, 1);
            let row2 = Rect::new(inner.x, inner.y + 1, inner.width, 1);
            f.render_widget(
                Paragraph::new(name_display).style(Style::default().fg(name_color)),
                row1,
            );
            f.render_widget(
                Paragraph::new(level_display).style(Style::default().fg(Color::DarkGray)),
                row2,
            );
        }
    }

    // ── Phase 3c: poke-doll sprite for Playing creatures ─────────────────────────
    //
    // The poke-doll image (bundled PNG, decoded at startup) is rendered as a
    // real Protocol image — same pipeline as creature sprites. It is placed
    // OUTSIDE the creature sprite, adjacent to the face-edge in the direction
    // the creature is currently facing.
    //
    // The Protocol is encoded once per terminal size/type (lazy, same pattern as
    // encode_all_frames) and stored in app.toy_proto.  Position-independence:
    // we encode at (0,0) and render at the actual toy_area position, so the
    // same protocol works for every Playing creature regardless of where it is.
    //
    // Toy width: half the sprite width.  Height: same as sprite height (fills
    // the same vertical band as the creature for clean alignment).
    //
    // Direction layout (current_dir): 0=Down  1=Left  2=Up  3=Right
    // NOTE: for Left-facing sprites the face is on the RIGHT side of the image,
    // so the toy goes RIGHT; Right-facing sprites have the face on the LEFT.

    const TOY_W: u16 = SPRITE_W / 2; // 16 cols
    let toy_h = sprite_h;
    let toy_size_rect = Rect::new(0, 0, TOY_W, toy_h);

    // Lazily encode (or re-encode on terminal resize / protocol-type change).
    if app.toy_proto_rect != Some(toy_size_rect) {
        let img = app.toy_image.clone();
        app.toy_proto = crate::sprite_loading::encode_toy_image(&img, picker, toy_size_rect);
        app.toy_proto_rect = Some(toy_size_rect);
    }

    for i in 0..count {
        let slot = &app.slots[i];
        if slot.animator.state() != AnimationState::Playing {
            continue;
        }

        // Recompute render position (same formula as Phase 3a/3b).
        let render_x = (pen_inner.x + slot.pos_x.round() as u16)
            .min(pen_inner.x + pen_inner.width.saturating_sub(sprite_w));
        let render_y = (pen_inner.y + slot.pos_y.round() as u16).min(
            pen_inner.y
                + pen_inner
                    .height
                    .saturating_sub(crate::creature::sprite_stack_h(sprite_h)),
        );

        // Use the true rendered sprite width (same as nameplate logic).
        let actual_sprite_w = pick_protocol_index(&slot.encoded_frames, 0, 0, 0)
            .and_then(|(si, di, fi)| slot.encoded_frames[si][di][fi].as_ref())
            .map(|p| p.area().width)
            .unwrap_or(sprite_w);

        let rx = render_x as i32;
        let ry = render_y as i32;
        let sw = actual_sprite_w as i32;
        let sh = sprite_h as i32;
        let tw = TOY_W as i32;
        let th = toy_h as i32;

        let (toy_x, toy_y) = match slot.current_dir {
            // Down  — just below, horizontally centred on the sprite
            0 => (rx + sw / 2 - tw / 2, ry + sh),
            // Left  — face on right side of the Left sprite → toy to the right
            1 => (rx + sw, ry + sh / 2 - th / 2),
            // Up    — just above, horizontally centred on the sprite
            2 => (rx + sw / 2 - tw / 2, ry - th),
            // Right — face on left side of the Right sprite → toy to the left
            _ => (rx - tw, ry + sh / 2 - th / 2),
        };

        let px = pen_inner.x as i32;
        let py = pen_inner.y as i32;
        let fits = toy_x >= px
            && toy_y >= py
            && toy_x + tw <= px + pen_inner.width as i32
            && toy_y + th <= py + pen_inner.height as i32;

        if fits {
            if let Some(proto) = app.toy_proto.as_mut() {
                f.render_widget(
                    Image::new(proto),
                    Rect::new(toy_x as u16, toy_y as u16, TOY_W, toy_h),
                );
            }
        }
    }
}

/// Pick a renderable protocol frame with fallbacks:
/// 1) requested state+dir with wrapped frame index
/// 2) any frame in requested state+dir
/// 3) any direction in requested state
/// 4) any direction in Idle state
///
/// The array has 5 states: 0=Idle, 1=Eat, 2=Sleep, 3=Recall, 4=Playing.
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
        for d in 0..4 {
            if d == dir_idx {
                continue;
            }
            if let Some(fi) = pick_from_dir_index(&encoded[s][d], frame_idx) {
                return Some((s, d, fi));
            }
        }
    }
    None
}
