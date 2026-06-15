use ratatui::style::{Color, Style};
use std::time::Instant;

#[derive(Debug, Clone, Copy)]
pub enum ToastKind { Info, Success, Warn, Error }

#[derive(Debug)]
pub struct Toast {
    pub msg: String,
    pub kind: ToastKind,
    pub created: Instant,
    pub ttl_ms: u128,
}

impl Toast {
    pub fn info(msg: impl Into<String>) -> Self { Self::new(msg, ToastKind::Info) }
    pub fn success(msg: impl Into<String>) -> Self { Self::new(msg, ToastKind::Success) }
    pub fn warn(msg: impl Into<String>) -> Self { Self::new(msg, ToastKind::Warn) }
    pub fn error(msg: impl Into<String>) -> Self { Self::new(msg, ToastKind::Error) }

    fn new(msg: impl Into<String>, kind: ToastKind) -> Self {
        Self { msg: msg.into(), kind, created: Instant::now(), ttl_ms: 3000 }
    }

    pub fn expired(&self) -> bool {
        Instant::now().duration_since(self.created).as_millis() >= self.ttl_ms
    }

    pub fn style(&self) -> Style {
        match self.kind {
            ToastKind::Info => Style::default().fg(Color::Cyan),
            ToastKind::Success => Style::default().fg(Color::Green),
            ToastKind::Warn => Style::default().fg(Color::Yellow),
            ToastKind::Error => Style::default().fg(Color::Red),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn toast_expires() {
        let mut t = Toast::info("hi");
        t.ttl_ms = 1;
        std::thread::sleep(Duration::from_millis(5));
        assert!(t.expired());
    }

    #[test]
    fn toast_kinds_have_distinct_colors() {
        use ratatui::style::Color;
        let info = Toast::info("a").style();
        let success = Toast::success("a").style();
        let warn = Toast::warn("a").style();
        let error = Toast::error("a").style();
        assert_eq!(info.fg, Some(Color::Cyan));
        assert_eq!(success.fg, Some(Color::Green));
        assert_eq!(warn.fg, Some(Color::Yellow));
        assert_eq!(error.fg, Some(Color::Red));
    }
}
