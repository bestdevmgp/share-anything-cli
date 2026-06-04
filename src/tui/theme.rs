use ratatui::style::{Color, Style};

#[allow(dead_code)]
pub fn accent() -> Style { Style::default().fg(Color::Cyan) }
#[allow(dead_code)]
pub fn muted() -> Style { Style::default().fg(Color::DarkGray) }
#[allow(dead_code)]
pub fn error() -> Style { Style::default().fg(Color::Red) }
#[allow(dead_code)]
pub fn success() -> Style { Style::default().fg(Color::Green) }
#[allow(dead_code)]
pub fn warning() -> Style { Style::default().fg(Color::Yellow) }
