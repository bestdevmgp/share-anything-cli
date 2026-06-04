use crate::core::upload::ShareResult;
use crate::tui::app::{AppCtx, ScreenAction};
use crate::tui::event::{self, Event};
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};
use std::path::PathBuf;
use tui_textarea::TextArea;

/// Tuples of (label shown to user, value sent to API).
pub const EXPIRES_OPTIONS: &[(&str, &str)] = &[
    ("5m", "5m"),
    ("30m", "30m"),
    ("1h", "1h"),
    ("3h", "3h"),
    ("6h", "6h"),
    ("12h", "12h"),
    ("24h", "24h"),
];

const DEFAULT_EXPIRES_IDX: usize = 1; // "30m"

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Field {
    Password,
    Expires,
    OneTime,
    SubmitButton,
}

#[allow(clippy::large_enum_variant)]
pub enum Phase {
    Form {
        password: TextArea<'static>,
        expires_idx: usize,
        one_time: bool,
        focus: Field,
        authenticated: bool,
    },
    Running {
        sent: u64,
        total: u64,
        display: String,
        started_at: std::time::Instant,
    },
    Done { result: ShareResult, copied: bool },
    Failed(String),
}

pub struct State {
    pub paths: Vec<PathBuf>,
    pub phase: Phase,
}

impl State {
    pub fn new(paths: Vec<PathBuf>, authenticated: bool) -> Self {
        let mut password = TextArea::default();
        password.set_placeholder_text("(none)");
        password.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Password "),
        );
        password.set_mask_char('\u{2022}');
        let phase = Phase::Form {
            password,
            expires_idx: DEFAULT_EXPIRES_IDX,
            one_time: false,
            focus: Field::SubmitButton,
            authenticated,
        };
        Self { paths, phase }
    }
}

pub fn update(s: &mut State, ev: &Event, ctx: &mut AppCtx) -> ScreenAction {
    match &mut s.phase {
        Phase::Form { password, expires_idx, one_time, focus, .. } => {
            let Event::Key(k) = ev else { return ScreenAction::Stay; };
            if matches!(k.code, KeyCode::Esc) {
                return ScreenAction::Pop;
            }
            // Global 'u' shortcut — same behavior as Enter on the Submit button. Suppressed
            // while focus is on the Password field so the letter remains typeable there.
            // Capital 'U' / Ctrl-U etc. fall through to existing handlers.
            if matches!(k.code, KeyCode::Char('u'))
                && k.modifiers.is_empty()
                && *focus != Field::Password
            {
                let password_text = password.lines().join("");
                let password_val = if password_text.is_empty() {
                    None
                } else {
                    Some(password_text)
                };
                let expiration = if ctx.client.is_authenticated() {
                    Some(EXPIRES_OPTIONS[*expires_idx].1.to_string())
                } else {
                    None
                };
                let one_time_val = ctx.client.is_authenticated() && *one_time;
                return start_upload(s, password_val, expiration, one_time_val, ctx);
            }
            if matches!(k.code, KeyCode::Tab) {
                *focus = next_field(*focus, ctx.client.is_authenticated());
                return ScreenAction::Stay;
            }
            if matches!(k.code, KeyCode::BackTab) {
                *focus = prev_field(*focus, ctx.client.is_authenticated());
                return ScreenAction::Stay;
            }
            if matches!(k.code, KeyCode::Up) {
                *focus = prev_field(*focus, ctx.client.is_authenticated());
                return ScreenAction::Stay;
            }
            if matches!(k.code, KeyCode::Down) {
                *focus = next_field(*focus, ctx.client.is_authenticated());
                return ScreenAction::Stay;
            }
            match *focus {
                Field::Password => {
                    if !ctx.client.is_authenticated() {
                        return ScreenAction::Stay;
                    }
                    // Enter must NOT be forwarded to tui-textarea: it would insert a newline,
                    // and `lines().join("")` would silently concatenate to a corrupted password
                    // (e.g. typed "1234", scrolled off, retyped "1234", got "12341234"). Treat
                    // Enter as "advance to next field" instead.
                    if matches!(k.code, KeyCode::Enter) {
                        *focus = next_field(*focus, ctx.client.is_authenticated());
                        return ScreenAction::Stay;
                    }
                    password.input(event::ev_to_input(k));
                    ScreenAction::Stay
                }
                Field::Expires => {
                    if !ctx.client.is_authenticated() {
                        return ScreenAction::Stay;
                    }
                    match k.code {
                        // Left/Right cycle expiration options here — they don't go back.
                        KeyCode::Left | KeyCode::Char('h') => {
                            if *expires_idx > 0 { *expires_idx -= 1; }
                        }
                        KeyCode::Right | KeyCode::Char('l') => {
                            if *expires_idx + 1 < EXPIRES_OPTIONS.len() { *expires_idx += 1; }
                        }
                        KeyCode::Char('b') if k.modifiers.is_empty() => return ScreenAction::Pop,
                        _ => {}
                    }
                    ScreenAction::Stay
                }
                Field::OneTime => {
                    if !ctx.client.is_authenticated() {
                        return ScreenAction::Stay;
                    }
                    match k.code {
                        KeyCode::Char(' ') | KeyCode::Enter => {
                            *one_time = !*one_time;
                        }
                        KeyCode::Char('b') if k.modifiers.is_empty() => return ScreenAction::Pop,
                        KeyCode::Left => return ScreenAction::Pop,
                        _ => {}
                    }
                    ScreenAction::Stay
                }
                Field::SubmitButton => {
                    if matches!(k.code, KeyCode::Enter) {
                        let password_text = password.lines().join("");
                        let password_val = if password_text.is_empty() {
                            None
                        } else {
                            Some(password_text)
                        };
                        let expiration = if ctx.client.is_authenticated() {
                            Some(EXPIRES_OPTIONS[*expires_idx].1.to_string())
                        } else {
                            None
                        };
                        let one_time_val = ctx.client.is_authenticated() && *one_time;
                        start_upload(s, password_val, expiration, one_time_val, ctx)
                    } else if matches!(k.code, KeyCode::Char('b')) && k.modifiers.is_empty() {
                        // Lowercase 'b' on the submit button drops back to the file picker, so
                        // a stray Enter on the picker can be undone. Capital 'B' (Shift+b)
                        // deliberately falls through to the password-forwarding branch below
                        // so it can still be typed into a password.
                        ScreenAction::Pop
                    } else if matches!(k.code, KeyCode::Left) {
                        ScreenAction::Pop
                    } else if ctx.client.is_authenticated() && matches!(k.code, KeyCode::Char(_)) {
                        // Typing on the Submit button jumps to Password and forwards the key,
                        // so users can start typing without moving focus first.
                        *focus = Field::Password;
                        password.input(event::ev_to_input(k));
                        ScreenAction::Stay
                    } else {
                        ScreenAction::Stay
                    }
                }
            }
        }
        Phase::Running { .. } => ScreenAction::Stay,
        Phase::Done { .. } => {
            let Event::Key(k) = ev else { return ScreenAction::Stay; };
            match k.code {
                KeyCode::Char('c') => {
                    if let Phase::Done { result, copied } = &mut s.phase {
                        let ok = crate::tui::copy_to_clipboard(&result.share_code);
                        if ok {
                            *copied = true;
                            *ctx.toast = Some(crate::tui::widgets::toast::Toast::success(
                                "Share code copied to clipboard.",
                            ));
                        } else {
                            *ctx.toast = Some(crate::tui::widgets::toast::Toast::warn(
                                "Clipboard unavailable - code is shown above.",
                            ));
                        }
                    }
                    ScreenAction::Stay
                }
                KeyCode::Enter
                | KeyCode::Esc
                | KeyCode::Char('q')
                | KeyCode::Left
                | KeyCode::Char('b') => {
                    if let Phase::Done { result: r, .. } = &s.phase {
                        ctx.stdout_lines.push("Upload complete!".into());
                        ctx.stdout_lines.push(format!("  Share code : {}", r.share_code));
                        ctx.stdout_lines.push(format!("  Download   : share download {}", r.share_code));
                        ctx.stdout_lines.push(format!("  Expires    : {}", crate::time::utc_to_local(&r.expires_at)));
                    }
                    // ← / b drops back to the picker so the same selection can be re-uploaded
                    // with adjusted options. Other confirm keys go home.
                    if matches!(k.code, KeyCode::Left | KeyCode::Char('b')) {
                        ScreenAction::Pop
                    } else {
                        ScreenAction::PopToRoot
                    }
                }
                _ => ScreenAction::Stay,
            }
        }
        Phase::Failed(_) => {
            let Event::Key(k) = ev else { return ScreenAction::Stay; };
            match k.code {
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') | KeyCode::Left | KeyCode::Char('b') => ScreenAction::Pop,
                _ => ScreenAction::Stay,
            }
        }
    }
}

fn next_field(f: Field, authenticated: bool) -> Field {
    if !authenticated {
        return Field::SubmitButton;
    }
    match f {
        Field::SubmitButton => Field::Password,
        Field::Password => Field::Expires,
        Field::Expires => Field::OneTime,
        Field::OneTime => Field::SubmitButton,
    }
}

fn prev_field(f: Field, authenticated: bool) -> Field {
    if !authenticated {
        return Field::SubmitButton;
    }
    match f {
        Field::SubmitButton => Field::OneTime,
        Field::OneTime => Field::Expires,
        Field::Expires => Field::Password,
        Field::Password => Field::SubmitButton,
    }
}

fn start_upload(
    s: &mut State,
    password: Option<String>,
    expiration: Option<String>,
    one_time: bool,
    ctx: &mut AppCtx,
) -> ScreenAction {
    let paths = s.paths.clone();
    let entries = match crate::core::upload::read_files(&paths) {
        Ok(e) => e,
        Err(e) => {
            s.phase = Phase::Failed(e.to_string());
            return ScreenAction::Stay;
        }
    };
    let total: u64 = entries.iter().map(|e| e.size).sum();
    let display = if entries.len() == 1 {
        entries[0].name.clone()
    } else {
        format!("{} files", entries.len())
    };

    s.phase = Phase::Running { sent: 0, total, display: display.clone(), started_at: std::time::Instant::now() };

    let Some(tx) = ctx.tx.cloned() else {
        s.phase = Phase::Failed("Internal error: event channel not ready.".to_string());
        return ScreenAction::Stay;
    };
    let client = ctx.client.clone();
    let opts = crate::core::upload::UploadOptions { password, expiration, one_time };

    let tx_for_progress = tx.clone();
    let on_progress: crate::core::ProgressFn =
        std::sync::Arc::new(move |n: u64| {
            let _ = tx_for_progress.send(Event::UploadProgress { delta: n });
        });
    let handle = tokio::spawn(async move {
        let r = crate::core::upload::upload_files(&client, entries, opts, on_progress, None).await;
        let _ = tx.send(Event::UploadFinished(r));
    });
    ctx.tasks.push(handle.abort_handle());

    ScreenAction::Stay
}

pub fn render(s: &State, f: &mut Frame) {
    let area = f.area();
    match &s.phase {
        Phase::Form { password, expires_idx, one_time, focus, authenticated } => {
            render_form(f, area, &s.paths, password, *expires_idx, *one_time, *focus, *authenticated);
        }
        Phase::Running { sent, total, display, started_at } => {
            render_running(f, area, *sent, *total, display, *started_at);
        }
        Phase::Done { result, copied } => render_done(f, area, result, *copied),
        Phase::Failed(msg) => render_failed(f, area, msg),
    }
}

fn card(f: &mut Frame, area: Rect, title: &str, accent: Color) -> Rect {
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(accent))
        .title(Span::styled(
            format!(" {} ", title),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ));
    let inner = outer.inner(area);
    f.render_widget(outer, area);
    Rect {
        x: inner.x.saturating_add(1),
        y: inner.y,
        width: inner.width.saturating_sub(2),
        height: inner.height,
    }
}

fn hints_bar(f: &mut Frame, area: Rect, text: &str) {
    f.render_widget(
        Paragraph::new(text).style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_form(
    f: &mut Frame,
    area: Rect,
    paths: &[PathBuf],
    password: &TextArea,
    expires_idx: usize,
    one_time: bool,
    focus: Field,
    authenticated: bool,
) {
    let inner = card(f, area, "Upload", Color::Cyan);

    let options_h: u16 = if authenticated { 11 } else { 2 };
    // Reserve space for the options block + submit button + spacers + hints so a share with
    // many files can't push them off-screen. The file list shrinks first; the renderer drops
    // a final " ..." row to signal that some entries were hidden.
    let fixed_h: u16 = 1 + 1 + options_h + 1 + 3 + 1; // chunks 0,2,3,4,5,7
    let want_files_h = paths.len() as u16 + 1;
    let max_files_h = inner.height.saturating_sub(fixed_h);
    let files_h = want_files_h.min(max_files_h).max(2);

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(files_h),
        Constraint::Length(1),
        Constraint::Length(options_h),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(inner);

    render_files_section(f, chunks[1], paths);

    if authenticated {
        render_options_section(f, chunks[3], password, expires_idx, one_time, focus);
    } else {
        f.render_widget(
            Paragraph::new(vec![
                section_header(format!("Options")),
                Line::from(Span::styled(
                    " Sign in to enable password / expiration / one-time.",
                    Style::default().fg(Color::DarkGray),
                )),
            ]),
            chunks[3],
        );
    }

    render_submit_button(f, chunks[5], focus == Field::SubmitButton);

    hints_bar(
        f,
        chunks[7],
        "[Enter/u] upload    [Tab/\u{2191}\u{2193}] options    [b/\u{2190}] picker    [Esc] cancel",
    );
}

fn section_header(text: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(
        text.into(),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    ))
}

fn render_files_section(f: &mut Frame, area: Rect, paths: &[PathBuf]) {
    let cap = area.height as usize;
    if cap == 0 { return; }

    let mut lines: Vec<Line> = Vec::with_capacity(cap);
    lines.push(section_header(format!("Files ({})", paths.len())));

    // One row for the header, then one row per file. If we don't have enough rows for every
    // file, sacrifice the last visible row to a "…" hint so the user knows some files were
    // dropped from view (instead of silently truncating).
    let visible_rows = cap.saturating_sub(1);
    let total = paths.len();
    let need_overflow = total > visible_rows;
    let shown = if need_overflow { visible_rows.saturating_sub(1) } else { total };

    let visible: Vec<(String, String)> = paths
        .iter()
        .take(shown)
        .map(|p| {
            let name = p
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            (name, crate::format::format_size_u64(size))
        })
        .collect();

    let max_size_w = visible.iter().map(|(_, s)| s.len()).max().unwrap_or(0);

    const PREFIX_W: usize = 3;
    const GAP_W: usize = 2;
    let avail = (area.width as usize).saturating_sub(PREFIX_W + GAP_W + max_size_w);
    let name_w = avail.max(5);

    for (name, size_str) in visible.iter() {
        let size_left_pad = max_size_w.saturating_sub(size_str.len());
        let name_truncated = crate::format::truncate_display(name, name_w);
        let name_padded = crate::format::pad_display(&name_truncated, name_w);

        lines.push(Line::from(vec![
            Span::styled(" \u{2022} ", Style::default().fg(Color::Cyan)),
            Span::raw(name_padded),
            Span::raw(" ".repeat(GAP_W + size_left_pad)),
            Span::styled(size_str.clone(), Style::default().fg(Color::DarkGray)),
        ]));
    }
    if need_overflow {
        lines.push(Line::from(Span::styled(
            "   \u{2026}",
            Style::default().fg(Color::DarkGray),
        )));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn render_options_section(
    f: &mut Frame,
    area: Rect,
    password: &TextArea,
    expires_idx: usize,
    one_time: bool,
    focus: Field,
) {
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Min(0),
    ])
    .split(area);

    f.render_widget(Paragraph::new(section_header("Options")), rows[0]);

    // Clone-and-reskin the TextArea so its border reflects focus without mutating the source
    // (render takes `&TextArea`).
    let pw_focused = focus == Field::Password;
    let (border_color, title_style) = if pw_focused {
        (
            Color::Cyan,
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )
    } else {
        (Color::DarkGray, Style::default().fg(Color::DarkGray))
    };
    let mut pw = password.clone();
    pw.set_block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(Span::styled(" Password ", title_style)),
    );
    f.render_widget(&pw, rows[1]);

    let expires_label = EXPIRES_OPTIONS[expires_idx].0;
    let focus_marker = |on: bool| -> Span<'static> {
        if on {
            Span::styled(" \u{25b8} ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        } else {
            Span::raw("   ")
        }
    };
    // Pin-on-rail slider: ▼ head on line 1, ┃ body breaking the rail on line 2 at the
    // same column.
    let active_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let rail_style = Style::default().fg(Color::DarkGray);

    let stop_spacing: usize = 6;
    let bar_len = (EXPIRES_OPTIONS.len() - 1) * stop_spacing + 1;
    let pin_pos = expires_idx * stop_spacing;
    // focus_marker (3) + "Expires   " (10) = 13 chars before the rail.
    let prefix_len: usize = 13;
    let head_line = Line::from(vec![
        Span::raw(" ".repeat(prefix_len + pin_pos)),
        Span::styled("\u{25BC}", active_style),
    ]);

    let mut rail_spans: Vec<Span<'static>> = vec![
        focus_marker(focus == Field::Expires),
        Span::styled("Expires   ", Style::default().fg(Color::DarkGray)),
    ];
    for i in 0..bar_len {
        if i == pin_pos {
            rail_spans.push(Span::styled("\u{2503}", active_style));
        } else {
            rail_spans.push(Span::styled("\u{2501}", rail_style));
        }
    }
    rail_spans.push(Span::raw("   "));
    rail_spans.push(Span::styled(
        expires_label,
        if focus == Field::Expires {
            active_style
        } else {
            Style::default().add_modifier(Modifier::BOLD)
        },
    ));

    f.render_widget(
        Paragraph::new(vec![head_line, Line::from(rail_spans)]),
        rows[2],
    );

    let check = if one_time { "[\u{2713}]" } else { "[ ]" };
    let check_style = if one_time {
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let one_time_line = Line::from(vec![
        focus_marker(focus == Field::OneTime),
        Span::styled("One-time  ", Style::default().fg(Color::DarkGray)),
        Span::styled(check.to_string(), check_style),
        Span::styled(
            "   (space to toggle)",
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    f.render_widget(Paragraph::new(one_time_line), rows[4]);
}

fn render_submit_button(f: &mut Frame, area: Rect, focused: bool) {
    let inner_width = area.width.saturating_sub(2);
    let btn_text = " Start Upload ";
    let btn_width = (btn_text.chars().count() as u16 + 4).min(inner_width);
    let x = area.x + (area.width.saturating_sub(btn_width)) / 2;
    let btn_area = Rect {
        x,
        y: area.y,
        width: btn_width,
        height: area.height.min(3),
    };

    let (style, border_style) = if focused {
        (
            Style::default()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(Color::Green),
        )
    } else {
        (
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            Style::default().fg(Color::DarkGray),
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style);
    let inner = block.inner(btn_area);
    f.render_widget(block, btn_area);
    f.render_widget(
        Paragraph::new(Span::styled(btn_text, style))
            .alignment(ratatui::layout::Alignment::Center),
        inner,
    );
}

fn fmt_progress(done: u64, total: u64, started_at: std::time::Instant) -> String {
    let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
    let rate = (done as f64) / elapsed;
    let pct = if total == 0 { 0.0 } else { (done as f64 / total as f64) * 100.0 };
    let eta = if rate > 0.0 && total > done {
        let secs = ((total - done) as f64 / rate).round() as u64;
        format_duration(secs)
    } else if total > 0 && done >= total {
        "done".to_string()
    } else {
        "-".to_string()
    };
    let speed = if rate > 0.0 {
        format!("{}/s", crate::format::format_size_u64(rate as u64))
    } else {
        "- /s".to_string()
    };
    format!(
        "{:.0}% \u{2022} {} / {} \u{2022} {} \u{2022} ETA {}",
        pct,
        crate::format::format_size_u64(done),
        crate::format::format_size_u64(total),
        speed,
        eta,
    )
}

fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

fn render_running(f: &mut Frame, area: Rect, sent: u64, total: u64, display: &str, started_at: std::time::Instant) {
    let inner = card(f, area, "Uploading", Color::Cyan);
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            display.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ))),
        chunks[1],
    );

    let ratio = if total == 0 { 0.0 } else { (sent as f64 / total as f64).min(1.0) };
    let label = fmt_progress(sent, total, started_at);
    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .gauge_style(Style::default().fg(Color::Cyan))
        .ratio(ratio)
        .label(label);
    f.render_widget(gauge, chunks[3]);

    hints_bar(f, chunks[5], "[Ctrl+C] cancel");
}

fn render_done(f: &mut Frame, area: Rect, r: &ShareResult, copied: bool) {
    let inner = card(f, area, "Upload complete", Color::Green);
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(5),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "\u{2713}",
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                "Files uploaded successfully",
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
        ])),
        chunks[1],
    );

    f.render_widget(
        Paragraph::new(vec![
            section_header("Share details"),
            Line::from(vec![
                Span::styled(" Code     ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    r.share_code.clone(),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Download ", Style::default().fg(Color::DarkGray)),
                Span::raw(format!("share download {}", r.share_code)),
            ]),
            Line::from(vec![
                Span::styled(" Expires  ", Style::default().fg(Color::DarkGray)),
                Span::raw(crate::time::utc_to_local(&r.expires_at)),
            ]),
        ]),
        chunks[2],
    );

    if copied {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    "\u{2713}",
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled("Copied to clipboard", Style::default().fg(Color::Green)),
            ])),
            chunks[4],
        );
    } else {
        f.render_widget(
            Paragraph::new(Span::styled(
                "Press [c] to copy the share code",
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            )),
            chunks[4],
        );
    }

    hints_bar(f, chunks[6], "[c] copy code    [Enter/Esc/q] home    [b/\u{2190}] new upload");
}

fn render_failed(f: &mut Frame, area: Rect, msg: &str) {
    let inner = card(f, area, "Upload failed", Color::Red);
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "\u{2717}",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                "Something went wrong",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
        ])),
        chunks[1],
    );

    f.render_widget(
        Paragraph::new(format!(" {}", msg))
            .style(Style::default().fg(Color::Red))
            .wrap(ratatui::widgets::Wrap { trim: false }),
        chunks[2],
    );

    hints_bar(f, chunks[3], "[Enter/Esc/q/b/\u{2190}] retry");
}
