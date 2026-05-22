use unicode_width::UnicodeWidthStr;

/// Format a byte count as a compact CLI string (no space between number and unit).
/// Examples: "5B", "5.2KB", "1.5MB", "3.7GB"
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

/// u64 variant for in-memory file buffers (P2P).
pub fn format_size_u64(bytes: u64) -> String {
    format_size(bytes as i64)
}

/// Pad a string to the given terminal display width.
/// Korean/CJK chars take 2 cells, ASCII takes 1.
pub fn pad_display(s: &str, width: usize) -> String {
    let w = UnicodeWidthStr::width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - w))
    }
}

/// Truncate a string to at most `max_width` display cells, adding "..." if needed.
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
