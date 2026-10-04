//! ANSI escape codes for terminal styling.

// Core
pub const RESET: &str = "\x1b[0m";
pub const DIM: &str = "\x1b[2m";
pub const BOLD: &str = "\x1b[1m";
pub const REVERSE: &str = "\x1b[7m";

// Foreground
pub const FG_GREEN: &str = "\x1b[32m";
pub const FG_CYAN: &str = "\x1b[36m";
pub const FG_WHITE: &str = "\x1b[37m";
pub const FG_BLACK: &str = "\x1b[30m";
pub const FG_GRAY: &str = "\x1b[38;5;242m";
pub const FG_YELLOW: &str = "\x1b[33m";
pub const FG_RED: &str = "\x1b[31m";
pub const FG_BLUE: &str = "\x1b[38;5;75m";

// TUI-specific foreground
pub const FG_ORANGE: &str = "\x1b[38;5;208m";
pub const FG_GOLD: &str = "\x1b[38;5;220m";
pub const FG_LIGHTGRAY: &str = "\x1b[38;5;250m";
pub const FG_DELIVER: &str = "\x1b[38;5;156m";
pub const FG_STALE: &str = "\x1b[38;5;137m";

// Background
pub const BG_BLUE: &str = "\x1b[48;5;69m";
pub const BG_GREEN: &str = "\x1b[42m";
pub const BG_CYAN: &str = "\x1b[46m";
pub const BG_YELLOW: &str = "\x1b[43m";
pub const BG_RED: &str = "\x1b[41m";
pub const BG_GRAY: &str = "\x1b[100m";
pub const BG_STALE: &str = "\x1b[48;5;137m";
pub const BG_ORANGE: &str = "\x1b[48;5;208m";
pub const BG_CHARCOAL: &str = "\x1b[48;5;236m";
pub const BG_GOLD: &str = "\x1b[48;5;220m";

// Terminal control
pub const CLEAR_SCREEN: &str = "\x1b[2J";
pub const CURSOR_HOME: &str = "\x1b[H";
pub const HIDE_CURSOR: &str = "\x1b[?25l";
pub const SHOW_CURSOR: &str = "\x1b[?25h";

// Box drawing
pub const BOX_H: &str = "\u{2500}"; // ─
