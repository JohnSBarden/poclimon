mod animal;
mod config;
mod game;
mod render;

use anyhow::Result;
use clap::Parser;
use config::GameConfig;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "PoCLImon - A terminal-based virtual pet game")]
struct Cli {
    /// Path to config file
    #[arg(short, long)]
    config: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Cli::parse();
    let config = match args.config {
        Some(path) => GameConfig::load(path)?,
        None => {
            let default_config = GameConfig::default();
            default_config.save("poclimon_config.json")?;
            println!("Created default config at poclimon_config.json");
            default_config
        }
    };

    let mut game = game::Game::new(config)?;
    game.run()
}
