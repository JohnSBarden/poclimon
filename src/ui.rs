//! UI rendering for PoCLImon.

use crate::app::App;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};
use ratatui_image::StatefulImage;

/// Render the full UI.
pub fn draw(f: &mut Frame<'_>, app: &mut App) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // Title / slot indicator
        Constraint::Min(10),  // Sprite area
        Constraint::Length(5), // Stats
        Constraint::Length(3), // Status / help bar
    ])
    .split(f.area());

    draw_title(f, app, chunks[0]);
    draw_sprite(f, app, chunks[1]);
    draw_stats(f, app, chunks[2]);
    draw_help(f, app, chunks[3]);
}

fn draw_title(f: &mut Frame<'_>, app: &App, area: Rect) {
    let pet = app.current_pet();
    let slot_text = format!(
        "< {}/{} {} >",
        app.current_slot + 1,
        app.slot_count(),
        pet.entry.display_name()
    );

    let title = Paragraph::new(Line::from(vec![
        Span::styled("PoCLImon", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(slot_text, Style::default().fg(Color::LightCyan)),
    ]))
    .block(Block::default().borders(Borders::ALL).title("⚡ PoCLImon"));
    f.render_widget(title, area);
}

fn draw_sprite(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let sprite_block = Block::default()
        .borders(Borders::ALL)
        .title(format!("🎮 {}", app.current_pet().entry.name));

    let inner = sprite_block.inner(area);
    f.render_widget(sprite_block, area);

    let slot = app.current_slot;
    if let Some(ref mut protocol) = app.pets[slot].image {
        let img_area = centered_rect(inner, 40, 20);
        let image_widget = StatefulImage::default();
        f.render_stateful_widget(image_widget, img_area, protocol);
    } else {
        let msg = app
            .status_message
            .as_deref()
            .unwrap_or("Loading sprite...");
        let fallback = Paragraph::new(msg).style(Style::default().fg(Color::Red));
        f.render_widget(fallback, inner);
    }
}

fn draw_stats(f: &mut Frame<'_>, app: &App, area: Rect) {
    let pet = app.current_pet();

    let cols = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(Block::default().borders(Borders::ALL).inner(area));

    let stats_block = Block::default().borders(Borders::ALL).title("📊 Stats");
    f.render_widget(stats_block, area);

    let happiness_gauge = Gauge::default()
        .block(Block::default().title("❤️  Happiness"))
        .gauge_style(Style::default().fg(Color::Magenta))
        .percent(pet.happiness as u16)
        .label(format!("{}/100", pet.happiness));
    f.render_widget(happiness_gauge, cols[0]);

    let hunger_gauge = Gauge::default()
        .block(Block::default().title("🍔 Hunger"))
        .gauge_style(Style::default().fg(Color::Green))
        .percent(pet.hunger as u16)
        .label(format!("{}/100", pet.hunger));
    f.render_widget(hunger_gauge, cols[1]);
}

fn draw_help(f: &mut Frame<'_>, app: &App, area: Rect) {
    let status = app
        .status_message
        .as_deref()
        .map(|s| format!(" | {s}"))
        .unwrap_or_default();

    let help_text = format!(
        "←/→ or 1-6: Switch Pokémon  |  F: Feed  |  P: Pet  |  Q/Esc: Quit{status}"
    );
    let help = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::ALL).title("Controls"));
    f.render_widget(help, area);
}

/// Center a rectangle of at most `max_width` × `max_height` within `area`.
fn centered_rect(area: Rect, max_width: u16, max_height: u16) -> Rect {
    let width = max_width.min(area.width);
    let height = max_height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}
