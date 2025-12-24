use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Animal {
    pub name: String,
    pub kind: String,
    pub position: (u16, u16),
    pub frame: usize,
    pub state: AnimalState,
    pub animation_timer: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AnimalState {
    Idle,
    Sleeping,
    Playing,
    Walking,
}

impl Animal {
    pub fn new(name: String, kind: String, position: (u16, u16)) -> Self {
        Self {
            name,
            kind,
            position,
            frame: 0,
            state: AnimalState::Idle,
            animation_timer: 0,
        }
    }

    pub fn update(&mut self) {
        self.animation_timer = self.animation_timer.wrapping_add(1);

        // Change animation frame based on timer
        if self.animation_timer % 10 == 0 {
            let frames = self.get_sprite_frames();
            self.frame = (self.frame + 1) % frames.len();
        }

        // Randomly change state occasionally
        if self.animation_timer % 100 == 0 {
            self.state = match rand::random::<u8>() % 4 {
                0 => AnimalState::Idle,
                1 => AnimalState::Sleeping,
                2 => AnimalState::Playing,
                _ => AnimalState::Walking,
            };
        }
    }

    pub fn get_sprite(&self) -> Vec<&'static str> {
        let frames = self.get_sprite_frames();
        frames[self.frame].to_vec()
    }

    fn get_sprite_frames(&self) -> &'static Vec<Vec<&'static str>> {
        match (self.kind.as_str(), &self.state) {
            ("cat", AnimalState::Idle) => &CAT_IDLE,
            ("cat", AnimalState::Sleeping) => &CAT_SLEEPING,
            ("cat", AnimalState::Playing) => &CAT_PLAYING,
            ("cat", AnimalState::Walking) => &CAT_WALKING,
            ("dog", AnimalState::Idle) => &DOG_IDLE,
            ("dog", AnimalState::Sleeping) => &DOG_SLEEPING,
            ("dog", AnimalState::Playing) => &DOG_PLAYING,
            ("dog", AnimalState::Walking) => &DOG_WALKING,
            ("bird", AnimalState::Idle) => &BIRD_IDLE,
            ("bird", AnimalState::Sleeping) => &BIRD_SLEEPING,
            ("bird", AnimalState::Playing) => &BIRD_PLAYING,
            ("bird", AnimalState::Walking) => &BIRD_WALKING,
            _ => &UNKNOWN_FRAMES,
        }
    }

    pub fn get_color(&self) -> (u8, u8, u8) {
        match self.kind.as_str() {
            "cat" => (255, 200, 100),  // Orange/yellow
            "dog" => (100, 150, 255),  // Blue
            "bird" => (255, 255, 100), // Yellow
            _ => (200, 200, 200),      // Gray
        }
    }
}

// True pixel art sprites using Unicode block characters
lazy_static! {
    // Cat animations - small, artistic pixel art
    static ref CAT_IDLE: Vec<Vec<&'static str>> = vec![
        vec![
            " ▄▄█▄▄ ",
            "▐█⚬⚬█▌",
            " ▀▀▀▀▀ ",
        ],
        vec![
            " ▄▄█▄▄ ",
            "▐█⚬ᴥ⚬█▌",
            " ▀▀▀▀▀ ",
        ],
    ];

    static ref CAT_SLEEPING: Vec<Vec<&'static str>> = vec![
        vec![
            "  zzz   ",
            " ▄▄█▄▄ ",
            "▐█- -█▌",
            " ▀▀▀▀▀ ",
        ],
        vec![
            "   zz   ",
            " ▄▄█▄▄ ",
            "▐█- -█▌",
            " ▀▀▀▀▀ ",
        ],
    ];

    static ref CAT_PLAYING: Vec<Vec<&'static str>> = vec![
        vec![
            " ▄▄█▄▄ ",
            "▐█⚬⚬█▌",
            "▐█▄▄▄█▌",
            " ▀▀▀▀▀ ",
        ],
        vec![
            " ▄▄█▄▄ ",
            "▐█ᴥᴥ█▌",
            "▐█▄▄▄█▌",
            " ▀▀▀▀▀ ",
        ],
    ];

    static ref CAT_WALKING: Vec<Vec<&'static str>> = vec![
        vec![
            " ▄▄█▄▄ ",
            "▐█⚬⚬█▌",
            " ▀▀█▀▀ ",
            "  ▀▀   ",
        ],
        vec![
            " ▄▄█▄▄ ",
            "▐█⚬⚬█▌",
            " ▀▀▀▀▀ ",
            "  ▀▀   ",
        ],
    ];

    // Dog animations - small, artistic pixel art
    static ref DOG_IDLE: Vec<Vec<&'static str>> = vec![
        vec![
            " ▄▄▄█▄▄",
            "▐█⚬⚬⚬█▌",
            "▐█████▌",
            " ▀▀▀▀▀ ",
        ],
        vec![
            " ▄▄▄█▄▄",
            "▐█⚬ᴥ⚬█▌",
            "▐█████▌",
            " ▀▀▀▀▀ ",
        ],
    ];

    static ref DOG_SLEEPING: Vec<Vec<&'static str>> = vec![
        vec![
            "  zzz   ",
            " ▄▄▄█▄▄",
            "▐█- -█▌",
            "▐█████▌",
            " ▀▀▀▀▀ ",
        ],
        vec![
            "   zz   ",
            " ▄▄▄█▄▄",
            "▐█- -█▌",
            "▐█████▌",
            " ▀▀▀▀▀ ",
        ],
    ];

    static ref DOG_PLAYING: Vec<Vec<&'static str>> = vec![
        vec![
            " ▄▄▄█▄▄",
            "▐█⚬⚬⚬█▌",
            "▐█▄▄▄█▌",
            " ▀▀▀▀▀ ",
        ],
        vec![
            " ▄▄▄█▄▄",
            "▐█ᴥᴥᴥ█▌",
            "▐█▄▄▄█▌",
            " ▀▀▀▀▀ ",
        ],
    ];

    static ref DOG_WALKING: Vec<Vec<&'static str>> = vec![
        vec![
            " ▄▄▄█▄▄",
            "▐█⚬⚬⚬█▌",
            " ▀▀█▀▀ ",
            "  ▀▀   ",
        ],
        vec![
            " ▄▄▄█▄▄",
            "▐█⚬⚬⚬█▌",
            " ▀▀▀▀▀ ",
            "  ▀▀   ",
        ],
    ];

    // Bird animations - small, artistic pixel art
    static ref BIRD_IDLE: Vec<Vec<&'static str>> = vec![
        vec![
            "  ▄▄▄  ",
            " ▐█⚬█▌",
            "▐█⚬⚬█▌",
            " ▀▀▀▀▀ ",
        ],
        vec![
            "  ▄▄▄  ",
            " ▐█⚬█▌",
            "▐█⚬⚬█▌",
            " ▀▀▀▀▀ ",
        ],
    ];

    static ref BIRD_SLEEPING: Vec<Vec<&'static str>> = vec![
        vec![
            "   zz  ",
            "  ▄▄▄  ",
            " ▐█-█▌",
            "▐█- -█▌",
            " ▀▀▀▀▀ ",
        ],
        vec![
            "    z  ",
            "  ▄▄▄  ",
            " ▐█-█▌",
            "▐█- -█▌",
            " ▀▀▀▀▀ ",
        ],
    ];

    static ref BIRD_PLAYING: Vec<Vec<&'static str>> = vec![
        vec![
            "  ▄▄▄  ",
            " ▐█⚬█▌",
            "▐█⚬⚬█▌",
            "▐█▄▄▄█▌",
            " ▀▀▀▀▀ ",
        ],
        vec![
            "  ▄▄▄  ",
            " ▐█ᴥ█▌",
            "▐█ᴥᴥ█▌",
            "▐█▄▄▄█▌",
            " ▀▀▀▀▀ ",
        ],
    ];

    static ref BIRD_WALKING: Vec<Vec<&'static str>> = vec![
        vec![
            "  ▄▄▄  ",
            " ▐█⚬█▌",
            "▐█⚬⚬█▌",
            " ▀▀█▀▀ ",
            "  ▀▀   ",
        ],
        vec![
            "  ▄▄▄  ",
            " ▐█⚬█▌",
            "▐█⚬⚬█▌",
            " ▀▀▀▀▀ ",
            "  ▀▀   ",
        ],
    ];

    static ref UNKNOWN_FRAMES: Vec<Vec<&'static str>> = vec![
        vec![
            " ▄▄▄▄▄ ",
            "▐█???█▌",
            "▐█???█▌",
            " ▀▀▀▀▀ ",
        ],
    ];
}
