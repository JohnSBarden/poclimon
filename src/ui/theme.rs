//! Colour themes, chosen with `[display] theme` in the settings file.
//!
//! Colours are named by role, not hue, so widgets never care which theme is
//! active.
//!
//! - `gbc`: a Game Boy Color Pokémon screen (Gold/Silver/Crystal menus and
//!   battle HUD). Everything is painted, pen included, so the terminal's own
//!   background never shows between the white text boxes.
//! - `terminal`: no fills. Text uses the terminal's own foreground, so it
//!   reads on light and dark terminals alike.

use crate::config::ThemeName;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, BorderType, Borders};

pub(super) struct Theme {
    // Text boxes.
    /// Text box fill. `Color::Reset` leaves the terminal background.
    pub paper: Color,
    pub ink: Color,       // menu text and frame
    pub ink_muted: Color, // hints, secondary text
    pub frame: BorderType,

    // Accents.
    pub title: Color,  // logo
    pub cursor: Color, // selected nameplate frame
    pub hp_green: Color,
    pub hp_yellow: Color,
    pub hp_red: Color,
    pub exp_blue: Color,
    pub psychic_pink: Color, // evolve/swap messages

    // Overworld.
    /// Pen fill behind the creatures. `Color::Reset` leaves the terminal background.
    pub ground: Color,
    pub fence: Color,
    pub grass: Color,
    pub flower: Color,
}

const GBC_WHITE: Color = Color::Rgb(248, 248, 248); // GBC white (31,31,31)

pub(super) const GBC: Theme = Theme {
    paper: GBC_WHITE,
    ink: Color::Rgb(24, 24, 32),
    ink_muted: Color::Rgb(112, 112, 128),
    frame: BorderType::Double,      // dialog-box double rule
    title: Color::Rgb(200, 32, 32), // Pokémon logo red
    cursor: Color::Rgb(200, 32, 32),
    hp_green: Color::Rgb(0, 168, 0),
    hp_yellow: Color::Rgb(200, 136, 0), // darkened to read on white
    hp_red: Color::Rgb(208, 0, 0),
    exp_blue: Color::Rgb(32, 96, 216),
    psychic_pink: Color::Rgb(176, 48, 152),
    ground: GBC_WHITE,
    fence: Color::Rgb(160, 96, 40),  // route fence wood
    grass: Color::Rgb(64, 160, 48),  // short grass blades
    flower: Color::Rgb(224, 64, 64), // route flowers; yellow vanishes on white
};

pub(super) const TERMINAL: Theme = Theme {
    paper: Color::Reset,
    ink: Color::Reset,
    ink_muted: Color::DarkGray,
    frame: BorderType::Double,
    // Mid-tone accents that read on both light and dark backgrounds.
    title: Color::Rgb(220, 48, 48),
    cursor: Color::Rgb(220, 48, 48),
    hp_green: Color::Rgb(32, 176, 32),
    hp_yellow: Color::Rgb(208, 152, 0),
    hp_red: Color::Rgb(220, 32, 32),
    exp_blue: Color::Rgb(64, 128, 232),
    psychic_pink: Color::Rgb(192, 72, 168),
    ground: Color::Reset,
    fence: Color::Rgb(160, 96, 40),
    grass: Color::Rgb(64, 160, 48),
    flower: Color::Rgb(232, 192, 48),
};

pub(super) fn theme(name: ThemeName) -> &'static Theme {
    match name {
        ThemeName::Gbc => &GBC,
        ThemeName::Terminal => &TERMINAL,
    }
}

impl Theme {
    /// A text box: double frame in ink, paper fill, ink as the default text colour.
    pub(super) fn text_box(&self) -> Block<'static> {
        Block::default()
            .borders(Borders::ALL)
            .border_type(self.frame)
            .border_style(Style::default().fg(self.ink))
            .style(Style::default().fg(self.ink).bg(self.paper))
    }
}
