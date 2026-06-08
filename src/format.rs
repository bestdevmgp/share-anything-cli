use unicode_width::UnicodeWidthStr;

pub fn format_size(bytes: i64) -> String {
    const KB: i64 = 1024;
    const MB: i64 = 1024 * KB;
    const GB: i64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1}GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}KB", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}

pub fn format_size_u64(bytes: u64) -> String {
    format_size(bytes as i64)
}

pub fn format_human_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

#[derive(Debug, Default, Clone)]
pub struct ThrottledMetrics {
    pub speed: String,
    pub eta: String,
    pub pct: u64,
    last_refresh: Option<std::time::Instant>,
}

impl ThrottledMetrics {
    pub fn last_refresh_was_done(&self) -> bool {
        self.last_refresh.is_some()
    }

    pub fn maybe_refresh(
        &mut self,
        current: u64,
        total: u64,
        started_at: std::time::Instant,
    ) {
        let now = std::time::Instant::now();
        let should = self
            .last_refresh
            .map_or(true, |t| now.duration_since(t) >= std::time::Duration::from_secs(1));
        if !should {
            return;
        }
        self.last_refresh = Some(now);
        let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
        let rate = current as f64 / elapsed;
        self.pct = if total > 0 {
            (current as f64 / total as f64 * 100.0) as u64
        } else {
            0
        };
        self.speed = format!("{}/s", format_size_u64(rate as u64));
        self.eta = if rate > 0.0 && total > current {
            let secs = ((total - current) as f64 / rate).round() as u64;
            format_human_duration(secs)
        } else {
            "-".into()
        };
    }
}

pub fn pad_display(s: &str, width: usize) -> String {
    let w = UnicodeWidthStr::width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - w))
    }
}

pub fn truncate_display(s: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(s) <= max_width {
        return s.to_string();
    }
    let cap = max_width.saturating_sub(3);
    let mut acc = String::new();
    let mut acc_w = 0;
    for ch in s.chars() {
        let cw = UnicodeWidthStr::width(ch.to_string().as_str());
        if acc_w + cw > cap {
            break;
        }
        acc.push(ch);
        acc_w += cw;
    }
    format!("{}...", acc)
}
