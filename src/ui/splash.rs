mod title_art {
    include!(concat!(env!("OUT_DIR"), "/title_art.rs"));
}

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

pub fn render_splash(f: &mut Frame<'_>) {
    let area = f.area();
    let img_rows = title_art::TITLE_ROWS as u16;
    let img_cols = title_art::TITLE_COLS as u16;
    let data = &title_art::TITLE_ART;

    // ▀/▄ chosen per cell so Color::Reset is never used as fg (which would
    // render the terminal's text color instead of the background).
    let art_lines: Vec<Line> = (0..title_art::TITLE_ROWS)
        .map(|row| {
            let spans: Vec<Span> = (0..title_art::TITLE_COLS)
                .map(|col| {
                    let (tr, tg, tb, ta, br, bg_r, bb, ba) =
                        data[row * title_art::TITLE_COLS + col];
                    let top_on = ta >= 128;
                    let bot_on = ba >= 128;
                    match (top_on, bot_on) {
                        (true, true) => Span::styled(
                            "▀",
                            Style::default()
                                .fg(Color::Rgb(tr, tg, tb))
                                .bg(Color::Rgb(br, bg_r, bb)),
                        ),
                        (true, false) => Span::styled(
                            "▀",
                            Style::default().fg(Color::Rgb(tr, tg, tb)).bg(Color::Reset),
                        ),
                        (false, true) => Span::styled(
                            "▄",
                            Style::default()
                                .fg(Color::Rgb(br, bg_r, bb))
                                .bg(Color::Reset),
                        ),
                        (false, false) => Span::raw(" "),
                    }
                })
                .collect();
            Line::from(spans)
        })
        .collect();

    let top_pad = area.height.saturating_sub(img_rows + 9) / 2;
    let chunks = Layout::vertical([
        Constraint::Length(top_pad),  // top padding
        Constraint::Length(img_rows), // logo
        Constraint::Length(1),        // blank
        Constraint::Length(1),        // version
        Constraint::Length(2),        // blank
        Constraint::Length(1),        // github handle
        Constraint::Length(1),        // blank
        Constraint::Length(1),        // trademark 1
        Constraint::Length(1),        // trademark 2
        Constraint::Length(1),        // trademark 3
        Constraint::Min(0),           // bottom padding
    ])
    .split(area);

    let img_x = area.x + area.width.saturating_sub(img_cols) / 2;
    f.render_widget(
        Paragraph::new(art_lines),
        Rect::new(img_x, chunks[1].y, img_cols.min(area.width), img_rows),
    );

    f.render_widget(
        Paragraph::new(format!("v{}", env!("CARGO_PKG_VERSION")))
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center),
        chunks[3],
    );

    f.render_widget(
        Paragraph::new("@JohnSBarden")
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(Alignment::Center),
        chunks[5],
    );

    for (i, text) in [
        "Pokemon and all related names are trademarks of",
        "Nintendo / Creatures Inc. / GAME FREAK inc.",
        "PoCLImon is an unofficial fan project.",
    ]
    .iter()
    .enumerate()
    {
        f.render_widget(
            Paragraph::new(*text)
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center),
            chunks[7 + i],
        );
    }
}
