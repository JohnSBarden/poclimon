use super::pen::render_pen;
use super::theme::{GB_DARK, GB_DARKEST, GB_LIGHT, GB_LIGHTEST};
use crate::animation::AnimationState;
use crate::app::{App, PromptMode};
use crate::notification::NotifLevel;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
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
            Style::default()
                .fg(GB_LIGHTEST)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" — "),
        Span::styled(
            format!("{} creatures", app.slots.len()),
            Style::default().fg(GB_LIGHT),
        ),
        Span::styled(
            format!(" [selected: {selected_name}]"),
            Style::default().fg(GB_DARK),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(GB_LIGHT))
            .style(Style::default().bg(GB_DARKEST))
            .title(format!("◆ PoCLImon {version} ◆"))
            .title_style(
                Style::default()
                    .fg(GB_LIGHTEST)
                    .add_modifier(Modifier::BOLD),
            ),
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
        Span::styled(
            format!("{selected_name}: "),
            Style::default().fg(GB_LIGHTEST),
        ),
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
            NotifLevel::Info => ("[Info]  ", GB_DARK),
        };
        status_lines.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(color)),
            Span::styled(notif.message.clone(), Style::default().fg(color)),
        ]));
    }

    let status = Paragraph::new(status_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(GB_LIGHT))
            .style(Style::default().bg(GB_DARKEST)),
    );
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
            Style::default()
                .fg(GB_LIGHTEST)
                .add_modifier(Modifier::BOLD),
        ));
        help_spans.push(Span::styled(*action, Style::default().fg(GB_DARK)));
    }
    let help = Paragraph::new(Line::from(help_spans)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(GB_LIGHT))
            .style(Style::default().bg(GB_DARKEST)),
    );
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
            .border_style(Style::default().fg(Color::Yellow))
            .style(Style::default().bg(GB_DARKEST));
        let inner = block.inner(popup_area);
        f.render_widget(block, popup_area);

        // Line 1: input field.
        let input_text = format!("Pokédex #: {}█", app.prompt_buffer);
        let row1 = Rect::new(inner.x, inner.y, inner.width, 1);
        f.render_widget(
            Paragraph::new(input_text).style(Style::default().fg(GB_LIGHTEST)),
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
