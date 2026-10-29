//! Game Boy Color Pokémon palette (Gold/Silver/Crystal menus and battle HUD).
//!
//! Colours are named by role, not hue, so another palette (e.g. the hidden
//! GBC boot palettes for original Game Boy games) can later replace this set
//! without touching the widgets.

use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, BorderType, Borders};

// Text boxes: black ink on white, like every GBC Pokémon dialog/menu box.
pub(super) const PAPER: Color = Color::Rgb(248, 248, 248); // GBC white (31,31,31)
pub(super) const INK: Color = Color::Rgb(24, 24, 32); // menu text and frame
pub(super) const INK_MUTED: Color = Color::Rgb(112, 112, 128); // hints, secondary text
pub(super) const FRAME: BorderType = BorderType::Double; // dialog-box double rule

// Accents.
pub(super) const TITLE_RED: Color = Color::Rgb(200, 32, 32); // Pokémon logo red
pub(super) const CURSOR: Color = Color::Rgb(200, 32, 32); // selected nameplate frame

// Battle HUD bars.
pub(super) const HP_GREEN: Color = Color::Rgb(0, 168, 0);
pub(super) const HP_YELLOW: Color = Color::Rgb(200, 136, 0); // darkened to read on PAPER
pub(super) const HP_RED: Color = Color::Rgb(208, 0, 0);
pub(super) const EXP_BLUE: Color = Color::Rgb(32, 96, 216);
pub(super) const PSYCHIC_PINK: Color = Color::Rgb(176, 48, 152); // evolve/swap messages

// Overworld.
pub(super) const FENCE_BROWN: Color = Color::Rgb(160, 96, 40); // route fence wood
pub(super) const GRASS_GREEN: Color = Color::Rgb(64, 160, 48); // short grass blades
pub(super) const FLOWER_YELLOW: Color = Color::Rgb(248, 208, 64);

/// A GBC text box: double frame in ink, white fill, ink as the default text colour.
pub(super) fn text_box() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(FRAME)
        .border_style(Style::default().fg(INK))
        .style(Style::default().fg(INK).bg(PAPER))
}
