use super::pen::render_pen;
use super::theme::{
    EXP_BLUE, HP_GREEN, HP_RED, HP_YELLOW, INK, INK_MUTED, PSYCHIC_PINK, TITLE_RED, text_box,
};
use crate::animation::AnimationState;
use crate::app::{App, PromptMode};
use crate::notification::NotifLevel;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};
use ratatui_image::picker::Picker;

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
        Span::styled(
            "PoCLImon",
            Style::default().fg(TITLE_RED).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" — "),
        Span::styled(
            format!("{} creatures", app.slots.len()),
            Style::default().fg(INK),
        ),
        Span::styled(
            format!(" [selected: {selected_name}]"),
            Style::default().fg(INK_MUTED),
        ),
    ]))
    .block(
        text_box()
            .title(format!("◆ PoCLImon {version} ◆"))
            .title_style(Style::default().fg(INK).add_modifier(Modifier::BOLD)),
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
            AnimationState::Idle => HP_GREEN,
            AnimationState::Eating => HP_YELLOW,
            AnimationState::Sleeping => EXP_BLUE,
            // Pink distinguishes Playing from other states at a glance.
            AnimationState::Playing => PSYCHIC_PINK,
        })
        .unwrap_or(INK);

    // Shared pen — all creatures on one open canvas.
    let placements = render_pen(f, chunks[1], app, picker);

    // Status + notification panel.
    // Line 0: current creature state.
    // Lines 1–2: most recent notifications, newest first.
    let mut first_line = vec![
        Span::styled(
            format!("{selected_name}: "),
            Style::default().fg(INK).add_modifier(Modifier::BOLD),
        ),
        Span::styled(state_label, Style::default().fg(status_color)),
    ];
    if app.evolution.is_none()
        && app
            .slots
            .get(app.selected)
            .is_some_and(|s| s.ready_to_evolve())
    {
        first_line.push(Span::styled(
            "  ✨ Ready to evolve. Press [V]",
            Style::default().fg(HP_YELLOW),
        ));
    }
    let mut status_lines = vec![Line::from(first_line)];
    if let Some(evo) = app.evolution.as_ref() {
        status_lines.push(Line::from(vec![
            Span::styled("[Evolve] ", Style::default().fg(PSYCHIC_PINK)),
            Span::styled(
                format!("What? {} is evolving!", evo.from_name),
                Style::default().fg(PSYCHIC_PINK),
            ),
        ]));
    } else if let Some(transition) = app.swap_transition.as_ref() {
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
            Span::styled("[Swap]  ", Style::default().fg(PSYCHIC_PINK)),
            Span::styled(
                format!("{action} {display_name}..."),
                Style::default().fg(PSYCHIC_PINK),
            ),
        ]));
    } else if let Some(transition) = app.add_transition.as_ref() {
        status_lines.push(Line::from(vec![
            Span::styled("[Add]   ", Style::default().fg(PSYCHIC_PINK)),
            Span::styled(
                format!("Loading {}...", transition.target_name),
                Style::default().fg(PSYCHIC_PINK),
            ),
        ]));
    }

    let notif_rows = if app.has_background_load() { 1 } else { 2 };
    for notif in app.notifications.iter().rev().take(notif_rows) {
        let (prefix, color) = match notif.level {
            NotifLevel::Error => ("[Error] ", HP_RED),
            NotifLevel::Warn => ("[Warn]  ", HP_YELLOW),
            NotifLevel::Info => ("[Info]  ", INK_MUTED),
        };
        status_lines.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(color)),
            Span::styled(notif.message.clone(), Style::default().fg(color)),
        ]));
    }

    let status = Paragraph::new(status_lines).block(text_box());
    f.render_widget(status, chunks[2]);

    // Help bar
    let help_pairs: &[(&str, &str)] = &[
        ("[Q]", "uit"),
        ("[E]", "at"),
        ("[S]", "leep"),
        ("[I]", "dle"),
        ("[P]", "lay"),
        ("[A]", "dd"),
        ("[R]", "emove"),
        ("[V]", "olve"),
        ("[Tab]", "swap"),
        ("[←→]", "select"),
    ];
    let mut help_spans: Vec<Span> = Vec::new();
    for (i, (key, action)) in help_pairs.iter().enumerate() {
        if i > 0 {
            help_spans.push(Span::raw("  "));
        }
        help_spans.push(Span::styled(
            *key,
            Style::default().fg(INK).add_modifier(Modifier::BOLD),
        ));
        help_spans.push(Span::styled(*action, Style::default().fg(INK_MUTED)));
    }
    let help = Paragraph::new(Line::from(help_spans)).block(text_box());
    f.render_widget(help, chunks[3]);

    // Branch popup: one numbered line per species the creature can become.
    if app.prompt_mode == PromptMode::EvolveChoice {
        let n = app.evolve_choices.len() as u16;
        let popup_w = 32u16;
        let popup_h = n + 3; // borders + hint line
        let popup_x = chunks[1].x + (chunks[1].width.saturating_sub(popup_w)) / 2;
        let popup_y = chunks[1].y + (chunks[1].height.saturating_sub(popup_h)) / 2;
        let popup_area = Rect::new(popup_x, popup_y, popup_w, popup_h).intersection(chunks[1]);
        f.render_widget(Clear, popup_area);
        let block = text_box().title(" Evolve into ");
        let inner = block.inner(popup_area);
        f.render_widget(block, popup_area);
        let mut lines: Vec<Line> = app
            .evolve_choices
            .iter()
            .enumerate()
            .map(|(i, &id)| {
                let name = crate::creatures::lookup_name(id).unwrap_or("???");
                Line::styled(format!("[{}] {name}", i + 1), Style::default().fg(INK))
            })
            .collect();
        lines.push(Line::styled("[Esc] cancel", Style::default().fg(INK_MUTED)));
        f.render_widget(Paragraph::new(lines), inner);
    }

    // Prompt overlay — appears centered over the pen area when Add or Swap is active.
    if matches!(app.prompt_mode, PromptMode::Add | PromptMode::Swap) {
        let title = match app.prompt_mode {
            PromptMode::Add => " Add Pokémon ",
            _ => " Swap to Pokémon ",
        };
        // Small centered popup: 32 wide, 4 tall.
        let popup_w = 32u16;
        let popup_h = 4u16;
        let popup_x = chunks[1].x + (chunks[1].width.saturating_sub(popup_w)) / 2;
        let popup_y = chunks[1].y + (chunks[1].height.saturating_sub(popup_h)) / 2;
        let popup_area = Rect::new(popup_x, popup_y, popup_w, popup_h);

        // Clear background (overwrite pen content in this area).
        f.render_widget(Clear, popup_area);

        let block = text_box().title(title);
        let inner = block.inner(popup_area);
        f.render_widget(block, popup_area);

        // Line 1: input field.
        let input_text = format!("Pokédex #: {}█", app.prompt_buffer);
        let row1 = Rect::new(inner.x, inner.y, inner.width, 1);
        f.render_widget(
            Paragraph::new(input_text).style(Style::default().fg(INK)),
            row1,
        );

        // Line 2: hint.
        let row2 = Rect::new(inner.x, inner.y + 1, inner.width, 1);
        f.render_widget(
            Paragraph::new("[Enter] confirm  [Esc] cancel").style(Style::default().fg(INK_MUTED)),
            row2,
        );
    }

    // Last: anything drawn over sprites (nameplates, toy, popup) may have
    // split Kitty image rows; restore explicit positions on the split cells.
    crate::kitty_upload::fix_orphan_placeholders(f.buffer_mut(), &placements);
}
