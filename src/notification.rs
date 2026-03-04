use std::time::Instant;

/// Maximum number of notifications to keep in the deque at once.
pub const MAX_NOTIFICATIONS: usize = 5;

/// How long (seconds) before a notification expires from the display.
pub const NOTIF_TTL_SECS: u64 = 8;

/// Severity level for an in-TUI notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotifLevel {
    // Info is currently used in tests and reserved for future informational
    // messages (e.g., successful sprite downloads).
    #[allow(dead_code)]
    Info,
    Warn,
    Error,
}

/// A single in-TUI notification message.
pub struct Notification {
    pub message: String,
    pub level: NotifLevel,
    pub created_at: Instant,
}
