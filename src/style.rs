use anstyle::{AnsiColor, Effects, Style};

pub(crate) const STATUS: Style = AnsiColor::BrightGreen.on_default().effects(Effects::BOLD);
pub(crate) const WARNING: Style = AnsiColor::Yellow.on_default().effects(Effects::BOLD);
pub(crate) const ERROR: Style = AnsiColor::BrightRed.on_default().effects(Effects::BOLD);
