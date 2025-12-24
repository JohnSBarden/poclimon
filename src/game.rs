use crate::{animal::Animal, config::GameConfig, render::Renderer};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent};
use std::time::{Duration, Instant};

pub struct Game {
    config: GameConfig,
    animals: Vec<Animal>,
    renderer: Renderer,
    running: bool,
    paused: bool,
    last_update: Instant,
    frame_duration: Duration,
}

impl Game {
    pub fn new(config: GameConfig) -> Result<Self> {
        let animals = config
            .animals
            .iter()
            .map(|ac| Animal::new(ac.name.clone(), ac.kind.clone(), ac.position))
            .collect();

        Ok(Self {
            config,
            animals,
            renderer: Renderer::new()?,
            running: true,
            paused: false,
            last_update: Instant::now(),
            frame_duration: Duration::from_millis(100), // 10 FPS
        })
    }

    pub fn run(&mut self) -> Result<()> {
        while self.running {
            self.handle_events()?;

            if !self.paused {
                self.update();
            }

            self.render()?;

            // Cap the frame rate
            let now = Instant::now();
            let elapsed = now - self.last_update;
            if elapsed < self.frame_duration {
                std::thread::sleep(self.frame_duration - elapsed);
            }
            self.last_update = now;
        }
        Ok(())
    }

    fn handle_events(&mut self) -> Result<()> {
        while event::poll(Duration::from_millis(0))? {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => self.running = false,
                    KeyCode::Char('p') | KeyCode::Char('P') => self.paused = !self.paused,
                    KeyCode::Char('r') | KeyCode::Char('R') => self.reset(),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn update(&mut self) {
        for animal in &mut self.animals {
            animal.update();
        }
    }

    fn render(&mut self) -> Result<()> {
        self.renderer.clear()?;

        for animal in &self.animals {
            self.renderer.draw_animal(animal)?;
        }

        self.renderer.draw_help()?;
        self.renderer.present()?;
        Ok(())
    }

    fn reset(&mut self) {
        self.animals.clear();
        self.animals = self
            .config
            .animals
            .iter()
            .map(|ac| Animal::new(ac.name.clone(), ac.kind.clone(), ac.position))
            .collect();
    }
}
