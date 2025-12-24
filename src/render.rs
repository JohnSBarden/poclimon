// src/render.rs
use crossterm::{
    cursor, queue,
    style::{Color, Print, SetForegroundColor},
    terminal,
};
use std::io::{self, Write};

use crate::animal::Animal;

pub struct Renderer {
    stdout: io::Stdout,
    height: u16,
}

impl Renderer {
    pub fn new() -> io::Result<Self> {
        let mut stdout = io::stdout();
        let (_, height) = terminal::size()?;

        // Enter raw mode and hide cursor
        terminal::enable_raw_mode()?;
        queue!(stdout, terminal::EnterAlternateScreen, cursor::Hide,)?;
        stdout.flush()?;

        Ok(Self { stdout, height })
    }

    pub fn clear(&mut self) -> io::Result<()> {
        queue!(
            self.stdout,
            terminal::Clear(terminal::ClearType::All),
            cursor::MoveTo(0, 0)
        )?;
        Ok(())
    }

    pub fn draw_animal(&mut self, animal: &Animal) -> io::Result<()> {
        let (x, y) = animal.position;
        let sprite = animal.get_sprite();
        let (r, g, b) = animal.get_color();

        // Draw the sprite
        for (dy, line) in sprite.iter().enumerate() {
            if y + dy as u16 >= self.height {
                break;
            }
            queue!(
                self.stdout,
                cursor::MoveTo(x, y + dy as u16),
                SetForegroundColor(Color::Rgb { r, g, b }),
                Print(line),
                SetForegroundColor(Color::Reset),
            )?;
        }

        // Draw animal name and state above the sprite
        if y > 1 {
            let state_indicator = match animal.state {
                crate::animal::AnimalState::Idle => "🧍",
                crate::animal::AnimalState::Sleeping => "😴",
                crate::animal::AnimalState::Playing => "🎾",
                crate::animal::AnimalState::Walking => "🚶",
            };

            queue!(
                self.stdout,
                cursor::MoveTo(x, y.saturating_sub(1)),
                SetForegroundColor(Color::Rgb { r, g, b }),
                Print(&animal.name),
                Print(" "),
                Print(state_indicator),
                SetForegroundColor(Color::Reset),
            )?;
        }

        Ok(())
    }

    pub fn draw_help(&mut self) -> io::Result<()> {
        let help_text = "Q: Quit  P: Pause  R: Reset";
        queue!(
            self.stdout,
            cursor::MoveTo(0, self.height.saturating_sub(1)),
            Print(help_text)
        )?;
        Ok(())
    }

    pub fn present(&mut self) -> io::Result<()> {
        self.stdout.flush()
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = queue!(self.stdout, terminal::LeaveAlternateScreen, cursor::Show);
        let _ = self.stdout.flush();
    }
}
