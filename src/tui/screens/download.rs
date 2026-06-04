use crate::core::shares::FileInfo;
use crate::tui::app::{AppCtx, ScreenAction};
use crate::tui::event::{self, Event};
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph},
    Frame,
};
use std::path::{Path, PathBuf};
use tui_textarea::TextArea;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecureFileStatus {
    Pending,
    Receiving,
    Done,
}

#[derive(Debug, Clone)]
pub struct SecureFileState {
    pub name: String,
    pub size: u64,
    pub received: u64,
    pub status: SecureFileStatus,
    pub started_at: Option<std::time::Instant>,
}

#[allow(clippy::large_enum_variant)]
pub enum Phase {
    InputCode { code: TextArea<'static> },
    FetchingInfo { code: String },
    NeedsPassword { info: FileInfo, password: TextArea<'static> },
    ChoosePath {
        info: FileInfo,
        password: Option<String>,
        picker: PathPicker,
    },
    Running {
        info: FileInfo,
        received: u64,
        total: u64,
        target_display: String,
        started_at: std::time::Instant,
    },
    SecureRunning {
        share_code: String,
        connected_info: Option<String>,
        file_states: Vec<SecureFileState>,
        /// Index into `file_states` of the file currently being received.
        active_idx: Option<usize>,
        started_at: std::time::Instant,
        log: Vec<String>,
        saved_files: Vec<PathBuf>,
    },
    Done { saved: PathBuf },
    SecureDone { saved_files: Vec<PathBuf>, log: Vec<String> },
    Failed(String),
}

pub struct State {
    pub phase: Phase,
}

impl State {
    pub fn new() -> Self {
        let mut code = TextArea::default();
        code.set_placeholder_text("123456");
        code.set_block(Block::default().borders(Borders::ALL).title(" Share code "));
        Self { phase: Phase::InputCode { code } }
    }

    pub fn new_with_pending(code: String) -> Self {
        Self { phase: Phase::FetchingInfo { code } }
    }
}

/// Directory picker for the save-path step. Built fresh from the user's current working
/// directory in `app.rs`'s `DownloadInfo` handler.
pub struct PathPicker {
    pub cwd: PathBuf,
    pub entries: Vec<PathBuf>,
    pub cursor: usize,
}

impl PathPicker {
    pub fn new() -> Self {
        let cwd = std::env::current_dir()
            .and_then(|p| p.canonicalize())
            .unwrap_or_else(|_| PathBuf::from("."));
        let entries = read_subdirs(&cwd);
        Self { cwd, entries, cursor: 0 }
    }

    /// Row layout: 0 = "Save here", 1 = ".." (if cwd has a parent), 2.. = subdirs.
    fn has_parent(&self) -> bool {
        self.cwd.parent().is_some()
    }

    fn rows_len(&self) -> usize {
        1 + (if self.has_parent() { 1 } else { 0 }) + self.entries.len()
    }

    fn move_cursor(&mut self, delta: i32) {
        let len = self.rows_len() as i32;
        if len == 0 { return; }
        let mut next = self.cursor as i32 + delta;
        if next < 0 { next = 0; }
        if next >= len { next = len - 1; }
        self.cursor = next as usize;
    }

    fn navigate(&mut self, target: PathBuf) {
        if let Ok(new_cwd) = target.canonicalize() {
            let entries = read_subdirs(&new_cwd);
            self.cwd = new_cwd;
            self.entries = entries;
            self.cursor = 0;
        }
    }

    fn parent(&mut self) {
        if let Some(p) = self.cwd.parent().map(|p| p.to_path_buf()) {
            self.navigate(p);
        }
    }
}

fn read_subdirs(p: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(p) else { return Vec::new(); };
    let mut dirs: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .collect();
    dirs.sort();
    dirs
}

pub fn update(s: &mut State, ev: &Event, ctx: &mut AppCtx) -> ScreenAction {
    match &mut s.phase {
        Phase::InputCode { code } => {
            let Event::Key(k) = ev else { return ScreenAction::Stay; };
            match k.code {
                KeyCode::Esc => ScreenAction::Pop,
                KeyCode::Enter => {
                    let code_text = code.lines().join("").trim().to_string();
                    if code_text.is_empty() {
                        *ctx.toast = Some(crate::tui::widgets::toast::Toast::warn(
                            "Enter a share code first.",
                        ));
                        return ScreenAction::Stay;
                    }
                    let Some(tx) = ctx.tx.cloned() else {
                        s.phase = Phase::Failed("Internal error: event channel not ready.".into());
                        return ScreenAction::Stay;
                    };
                    let client = ctx.client.clone();
                    let code_for_task = code_text.clone();
                    let handle = tokio::spawn(async move {
                        let r = crate::core::shares::get_share_info(&client, &code_for_task).await;
                        let _ = tx.send(Event::DownloadInfo(r));
                    });
                    ctx.tasks.push(handle.abort_handle());
                    s.phase = Phase::FetchingInfo { code: code_text };
                    ScreenAction::Stay
                }
                _ => {
                    if !event::accept_share_code_input(code, k) {
                        return ScreenAction::Stay;
                    }
                    code.input(event::ev_to_input(k));
                    ScreenAction::Stay
                }
            }
        }
        Phase::FetchingInfo { .. } => {
            ScreenAction::Stay
        }
        Phase::NeedsPassword { password, .. } => {
            let Event::Key(k) = ev else { return ScreenAction::Stay; };
            match k.code {
                KeyCode::Esc => ScreenAction::Pop,
                KeyCode::Enter => {
                    let pw = password.lines().join("");
                    if pw.is_empty() {
                        *ctx.toast = Some(crate::tui::widgets::toast::Toast::warn(
                            "Password required.",
                        ));
                        return ScreenAction::Stay;
                    }
                    if let Phase::NeedsPassword { info, password: _ } =
                        std::mem::replace(&mut s.phase, Phase::Failed("Internal state error.".into()))
                    {
                        s.phase = Phase::ChoosePath {
                            info,
                            password: Some(pw),
                            picker: PathPicker::new(),
                        };
                    }
                    ScreenAction::Stay
                }
                _ => {
                    password.input(event::ev_to_input(k));
                    ScreenAction::Stay
                }
            }
        }
        Phase::ChoosePath { picker, .. } => {
            let Event::Key(k) = ev else { return ScreenAction::Stay; };
            match k.code {
                KeyCode::Esc | KeyCode::Char('q') => ScreenAction::Pop,
                KeyCode::Up | KeyCode::Char('k') => { picker.move_cursor(-1); ScreenAction::Stay }
                KeyCode::Down | KeyCode::Char('j') => { picker.move_cursor(1); ScreenAction::Stay }
                KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                    picker.parent();
                    ScreenAction::Stay
                }
                KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                    let parent_row = if picker.has_parent() { 1 } else { usize::MAX };
                    if picker.cursor == 0 {
                        let dir = picker.cwd.clone();
                        start_download(s, dir, ctx)
                    } else if picker.cursor == parent_row {
                        picker.parent();
                        ScreenAction::Stay
                    } else {
                        let entry_idx = picker.cursor.saturating_sub(if picker.has_parent() { 2 } else { 1 });
                        if let Some(target) = picker.entries.get(entry_idx).cloned() {
                            picker.navigate(target);
                        }
                        ScreenAction::Stay
                    }
                }
                _ => ScreenAction::Stay,
            }
        }
        Phase::Running { .. } | Phase::SecureRunning { .. } => ScreenAction::Stay,
        Phase::Done { .. } => {
            let Event::Key(k) = ev else { return ScreenAction::Stay; };
            if matches!(k.code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') | KeyCode::Left | KeyCode::Char('b')) {
                if let Phase::Done { saved } = &s.phase {
                    let display = if saved.is_relative() && !saved.starts_with(".") {
                        format!("./{}", saved.display())
                    } else {
                        saved.display().to_string()
                    };
                    ctx.stdout_lines.push("Download complete!".into());
                    ctx.stdout_lines.push(format!("  Saved to: {}", display));
                }
                ScreenAction::PopToRoot
            } else {
                ScreenAction::Stay
            }
        }
        Phase::Failed(_) => {
            let Event::Key(k) = ev else { return ScreenAction::Stay; };
            if matches!(k.code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') | KeyCode::Left | KeyCode::Char('b')) {
                // Reset to the code-input step so a typo can be corrected in place.
                let mut code = TextArea::default();
                code.set_placeholder_text("123456");
                code.set_block(Block::default().borders(Borders::ALL).title(" Share code "));
                s.phase = Phase::InputCode { code };
            }
            ScreenAction::Stay
        }
        Phase::SecureDone { .. } => {
            let Event::Key(k) = ev else { return ScreenAction::Stay; };
            if matches!(k.code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') | KeyCode::Left | KeyCode::Char('b')) {
                return ScreenAction::PopToRoot;
            }
            ScreenAction::Stay
        }
    }
}

fn start_download(s: &mut State, output_dir: PathBuf, ctx: &mut AppCtx) -> ScreenAction {
    let replaced = std::mem::replace(&mut s.phase, Phase::Failed("Internal state error.".into()));
    let Phase::ChoosePath { info, password, picker: _ } = replaced else {
        s.phase = Phase::Failed("Internal state error.".into());
        return ScreenAction::Stay;
    };

    if info.transfer_type.as_deref() == Some("p2p") {
        let share_code = info.share_code.clone();
        let total: u64 = info.files.iter().map(|f| f.file_size.max(0) as u64).sum();

        let Some(tx) = ctx.tx.cloned() else {
            s.phase = Phase::Failed("Internal error: event channel not ready.".into());
            return ScreenAction::Stay;
        };
        let client = ctx.client.clone();
        let tx_events = tx.clone();
        let on_event: crate::core::p2p::receiver::ReceiverEventFn = std::sync::Arc::new(move |ev| {
            let _ = tx_events.send(Event::P2PReceive(ev));
        });
        let file_names: Vec<String> = info.files.iter().map(|f| f.file_name.clone()).collect();
        let file_states: Vec<SecureFileState> = info
            .files
            .iter()
            .map(|f| SecureFileState {
                name: f.file_name.clone(),
                size: f.file_size.max(0) as u64,
                received: 0,
                status: SecureFileStatus::Pending,
                started_at: None,
            })
            .collect();
        let _ = total;
        let receiver_opts = crate::core::p2p::receiver::ReceiverOptions {
            share_code: share_code.clone(),
            password,
            output_dir,
            files: file_names,
        };
        s.phase = Phase::SecureRunning {
            share_code,
            connected_info: None,
            file_states,
            active_idx: None,
            started_at: std::time::Instant::now(),
            log: vec!["Connecting to sender\u{2026}".into()],
            saved_files: vec![],
        };
        let handle = tokio::spawn(async move {
            let r = crate::core::p2p::receiver::run(&client, receiver_opts, on_event).await;
            if let Err(e) = r {
                let _ = tx.send(Event::P2PReceive(
                    crate::core::p2p::receiver::ReceiverEvent::Failed(format!("Transfer failed: {}", e))
                ));
            }
        });
        ctx.tasks.push(handle.abort_handle());
        return ScreenAction::Stay;
    }

    let code = info.share_code.clone();
    let target_total = if !info.files.is_empty() {
        info.files[0].file_size.max(0) as u64
    } else {
        0
    };
    let target_display = if !info.files.is_empty() {
        info.files[0].file_name.clone()
    } else {
        format!("download_{}", code)
    };

    let Some(tx) = ctx.tx.cloned() else {
        s.phase = Phase::Failed("Internal error: event channel not ready.".into());
        return ScreenAction::Stay;
    };

    let client = ctx.client.clone();
    let info_for_task = info.clone();
    let opts = crate::core::download::DownloadOptions { password, file_id: None };
    let tx_for_progress = tx.clone();
    let on_progress: crate::core::ProgressFn = std::sync::Arc::new(move |n: u64| {
        let _ = tx_for_progress.send(Event::DownloadProgress { delta: n });
    });

    let handle = tokio::spawn(async move {
        let r = crate::core::download::download_share(
            &client,
            &code,
            &info_for_task,
            opts,
            &output_dir,
            on_progress,
        )
        .await;
        let _ = tx.send(Event::DownloadFinished(r));
    });
    ctx.tasks.push(handle.abort_handle());

    s.phase = Phase::Running {
        info,
        received: 0,
        total: target_total,
        target_display,
        started_at: std::time::Instant::now(),
    };
    ScreenAction::Stay
}

pub fn render(s: &State, f: &mut Frame) {
    let area = f.area();
    match &s.phase {
        Phase::InputCode { code } => render_input_code(f, area, code),
        Phase::FetchingInfo { code } => render_fetching(f, area, code),
        Phase::NeedsPassword { info, password } => render_password(f, area, info, password),
        Phase::ChoosePath { info, picker, .. } => render_choose_path(f, area, info, picker),
        Phase::Running { info, received, total, target_display, started_at } => {
            render_running(f, area, info, *received, *total, target_display, *started_at)
        }
        Phase::SecureRunning { share_code, connected_info, file_states, active_idx, started_at, log, .. } => {
            render_secure_running_dl(f, area, share_code, connected_info.as_deref(), file_states, *active_idx, *started_at, log);
        }
        Phase::Done { saved } => render_done(f, area, saved),
        Phase::SecureDone { saved_files, log } => render_secure_done_dl(f, area, saved_files, log),
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

fn hints(f: &mut Frame, area: Rect, text: &str) {
    f.render_widget(
        Paragraph::new(text).style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

fn render_info_bar(f: &mut Frame, area: Rect, info: &FileInfo) {
    let is_p2p = info.transfer_type.as_deref() == Some("p2p");
    let mut spans: Vec<Span> = vec![];
    let chip = |label: &str, val: &str, c: Color| -> Vec<Span<'static>> {
        vec![
            Span::styled(format!(" {} ", label), Style::default().fg(Color::DarkGray)),
            Span::styled(val.to_string(), Style::default().fg(c).add_modifier(Modifier::BOLD)),
            Span::raw("   "),
        ]
    };
    // P2P doesn't support One-time; swap that chip for a "Secure transfer" mode badge.
    if is_p2p {
        spans.extend(chip("Mode", "Secure transfer", Color::Cyan));
    }
    spans.extend(chip("Code", &info.share_code, Color::Cyan));
    spans.extend(chip(
        "Password",
        if info.has_password { "Yes" } else { "No" },
        if info.has_password { Color::Yellow } else { Color::Gray },
    ));
    if !is_p2p {
        spans.extend(chip(
            "One-time",
            if info.is_one_time { "Yes" } else { "No" },
            if info.is_one_time { Color::Yellow } else { Color::Gray },
        ));
    }
    spans.extend(chip("Expires", &crate::time::utc_to_local(&info.expires_at), Color::Gray));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn section_header(text: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(
        text.into(),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    ))
}

fn render_files_section(f: &mut Frame, area: Rect, info: &FileInfo) {
    let mut lines: Vec<Line> = Vec::with_capacity(info.files.len() + 1);
    lines.push(section_header(format!("Files ({})", info.files.len())));
    for fd in &info.files {
        lines.push(Line::from(vec![
            Span::styled(" \u{2022} ", Style::default().fg(Color::Cyan)),
            Span::raw(fd.file_name.clone()),
            Span::styled(
                format!("    {}", crate::format::format_size(fd.file_size)),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn render_input_code(f: &mut Frame, area: Rect, code: &TextArea) {
    let inner = card(f, area, "Download", Color::Cyan);

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(inner);

    f.render_widget(
        Paragraph::new("Enter a share code to download."),
        chunks[1],
    );
    f.render_widget(code, chunks[2]);
    hints(f, chunks[4], "[Enter] fetch info    [Esc] back");
}

fn render_fetching(f: &mut Frame, area: Rect, code: &str) {
    let inner = card(f, area, "Download", Color::Cyan);
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Min(0),
    ])
    .split(inner);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                spinner_frame(),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::raw(format!("Fetching info for {}", code)),
        ])),
        chunks[1],
    );
}

fn render_password(f: &mut Frame, area: Rect, info: &FileInfo, password: &TextArea) {
    let inner = card(f, area, "Download", Color::Cyan);
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(inner);
    render_info_bar(f, chunks[1], info);
    f.render_widget(password, chunks[3]);
    hints(f, chunks[5], "[Enter] continue    [Esc] back");
}

fn render_choose_path(f: &mut Frame, area: Rect, info: &FileInfo, picker: &PathPicker) {
    let inner = card(f, area, "Download", Color::Cyan);

    let files_h = info.files.len() as u16 + 1;
    let pw_h: u16 = if info.has_password { 1 } else { 0 };
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(files_h),
        Constraint::Length(1),
        Constraint::Length(pw_h),
        Constraint::Length(1),
        Constraint::Min(6),
        Constraint::Length(1),
    ])
    .split(inner);

    render_info_bar(f, chunks[1], info);
    render_files_section(f, chunks[3], info);

    if info.has_password {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " \u{2713}",
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    "Password verified",
                    Style::default().fg(Color::Green),
                ),
            ])),
            chunks[5],
        );
    }

    f.render_widget(Paragraph::new(section_header("Save path")), chunks[6]);
    render_picker(f, chunks[7], picker);
    hints(f, chunks[8], "[\u{2191}\u{2193}] move    [Enter] open/save    [\u{232b}/h] up    [Esc] back");
}

fn render_picker(f: &mut Frame, area: Rect, picker: &PathPicker) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                picker.cwd.display().to_string(),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut items: Vec<ListItem> = Vec::new();
    let cwd_str = picker.cwd.display().to_string();
    items.push(ListItem::new(Line::from(vec![
        Span::styled("[\u{2713} Save here]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(cwd_str, Style::default().fg(Color::DarkGray)),
    ])));
    if picker.has_parent() {
        items.push(ListItem::new(Line::from(vec![
            Span::styled("..", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled("parent directory", Style::default().fg(Color::DarkGray)),
        ])));
    }
    for entry in &picker.entries {
        let name = entry.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        items.push(ListItem::new(Line::from(vec![
            Span::styled("\u{1f4c1} ", Style::default().fg(Color::Blue)),
            Span::styled(name, Style::default().fg(Color::White)),
            Span::styled("/", Style::default().fg(Color::DarkGray)),
        ])));
    }

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(Color::Cyan)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        );
    let mut state = ListState::default();
    state.select(Some(picker.cursor));
    f.render_stateful_widget(list, inner, &mut state);
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

fn render_running(
    f: &mut Frame,
    area: Rect,
    info: &FileInfo,
    received: u64,
    total: u64,
    target_display: &str,
    started_at: std::time::Instant,
) {
    let inner = card(f, area, "Downloading", Color::Cyan);
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
        Paragraph::new(Line::from(vec![
            Span::styled(
                target_display.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("    code {}", info.share_code),
                Style::default().fg(Color::DarkGray),
            ),
        ])),
        chunks[1],
    );
    let ratio = if total == 0 { 0.0 } else { (received as f64 / total as f64).min(1.0) };
    let label = fmt_progress(received, total, started_at);
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
    hints(f, chunks[5], "[Ctrl+C] cancel");
}

enum StepState {
    Done,
    Active,
    Pending,
}

fn spinner_frame() -> &'static str {
    const FRAMES: &[&str] = &[
        "\u{280B}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283C}",
        "\u{2834}", "\u{2826}", "\u{2827}", "\u{2807}", "\u{280F}",
    ];
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    FRAMES[(ms / 80) as usize % FRAMES.len()]
}

fn step_line(state: StepState, label: &str, detail: Option<&str>) -> Line<'static> {
    let (marker, marker_style) = match state {
        StepState::Done => (
            "\u{2713}".to_string(),
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ),
        StepState::Active => (
            spinner_frame().to_string(),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        StepState::Pending => (
            " ".to_string(),
            Style::default().fg(Color::DarkGray),
        ),
    };
    let label_style = match state {
        StepState::Pending => Style::default().fg(Color::DarkGray),
        _ => Style::default(),
    };
    let detail_text = detail.map(|d| format!("  - {}", d)).unwrap_or_default();
    Line::from(vec![
        Span::raw("  "),
        Span::raw("["),
        Span::styled(marker, marker_style),
        Span::raw("] "),
        Span::styled(format!("{:<22}", label), label_style),
        Span::styled(detail_text, Style::default().fg(Color::DarkGray)),
    ])
}

#[allow(clippy::too_many_arguments)]
fn render_secure_running_dl(
    f: &mut Frame,
    area: Rect,
    _share_code: &str,
    connected_info: Option<&str>,
    file_states: &[SecureFileState],
    active_idx: Option<usize>,
    _started_at: std::time::Instant,
    log: &[String],
) {
    let done_count = file_states.iter().filter(|f| f.status == SecureFileStatus::Done).count();
    let total_files = file_states.len();
    let all_done = total_files > 0 && done_count == total_files;

    let files_block_h: u16 = if file_states.is_empty() {
        0
    } else {
        (2 + file_states.len() as u16).min(area.height.saturating_sub(11))
    };

    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Length(files_block_h),
        Constraint::Length(5),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Span::styled(
            " Secure download (P2P) ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        chunks[0],
    );

    let step1_state = if connected_info.is_some() { StepState::Done } else { StepState::Active };
    let step2_state = if all_done {
        StepState::Done
    } else if connected_info.is_some() {
        StepState::Active
    } else {
        StepState::Pending
    };
    let step2_detail_owned = if total_files > 1 {
        Some(format!("file {} of {}", done_count.min(total_files).max(1), total_files))
    } else {
        active_idx.and_then(|i| file_states.get(i)).map(|f| f.name.clone())
    };
    let step3_state = if all_done { StepState::Active } else { StepState::Pending };

    let steps = vec![
        step_line(step1_state, "Connected to sender", connected_info),
        step_line(step2_state, "Receiving", step2_detail_owned.as_deref()),
        step_line(step3_state, "Complete", None),
    ];
    f.render_widget(Paragraph::new(steps), chunks[1]);

    if !file_states.is_empty() {
        let file_lines: Vec<Line> = file_states.iter().enumerate().map(|(i, fs)| {
            let (marker, marker_style, detail_style) = match fs.status {
                SecureFileStatus::Done => (
                    "\u{2713}".to_string(),
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                    Style::default().fg(Color::DarkGray),
                ),
                SecureFileStatus::Receiving => (
                    "\u{25b6}".to_string(),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    Style::default().fg(Color::White),
                ),
                SecureFileStatus::Pending => (
                    " ".to_string(),
                    Style::default().fg(Color::DarkGray),
                    Style::default().fg(Color::DarkGray),
                ),
            };

            let detail = match fs.status {
                SecureFileStatus::Done => {
                    format!("  ({})", crate::format::format_size_u64(fs.size))
                }
                SecureFileStatus::Receiving => {
                    let pct = if fs.size > 0 { (fs.received as f64 / fs.size as f64 * 100.0) as u64 } else { 0 };
                    if let Some(t) = fs.started_at {
                        let elapsed = t.elapsed().as_secs_f64().max(0.001);
                        let rate = fs.received as f64 / elapsed;
                        let speed = format!("{}/s", crate::format::format_size_u64(rate as u64));
                        let eta = if rate > 0.0 && fs.size > fs.received {
                            let secs = ((fs.size - fs.received) as f64 / rate).round() as u64;
                            format_duration(secs)
                        } else {
                            "-".to_string()
                        };
                        format!("  {}% \u{00b7} {} \u{00b7} ETA {}", pct, speed, eta)
                    } else {
                        format!("  {}%", pct)
                    }
                }
                SecureFileStatus::Pending => String::new(),
            };

            let is_active = active_idx == Some(i);
            let name_style = if is_active {
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let mut spans: Vec<Span> = vec![
                Span::raw("  "),
                Span::styled(marker, marker_style),
                Span::raw(" "),
                Span::styled(fs.name.clone(), name_style),
            ];
            if matches!(fs.status, SecureFileStatus::Receiving) && fs.size > 0 {
                let bar_width: usize = 28;
                let ratio = (fs.received as f64 / fs.size as f64).min(1.0);
                let filled = (ratio * bar_width as f64).round() as usize;
                let filled_bar: String = "\u{2501}".repeat(filled);
                let empty_bar: String = "\u{2500}".repeat(bar_width - filled);
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    filled_bar,
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    empty_bar,
                    Style::default().fg(Color::DarkGray),
                ));
            }
            spans.push(Span::styled(detail, detail_style));
            Line::from(spans)
        }).collect();

        f.render_widget(
            Paragraph::new(file_lines)
                .block(Block::default().borders(Borders::ALL).title(" Files ")),
            chunks[2],
        );
    }

    let max_lines = chunks[3].height.saturating_sub(2) as usize;
    let start = log.len().saturating_sub(max_lines.max(1));
    let log_lines: Vec<Line> = log[start..].iter().map(|s| Line::from(s.as_str())).collect();
    f.render_widget(
        Paragraph::new(log_lines)
            .block(Block::default().borders(Borders::ALL).title(" Log ")),
        chunks[3],
    );

    hints(f, chunks[5], " [Ctrl+C] cancel ");
}

fn render_secure_done_dl(f: &mut Frame, area: Rect, saved_files: &[PathBuf], log: &[String]) {
    let files_block_height = (2 + saved_files.len()).max(3) as u16;
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(files_block_height),
        Constraint::Min(0),
        Constraint::Length(5),
        Constraint::Length(1),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Span::styled(
            " \u{2713} Secure download complete! ",
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        )),
        chunks[0],
    );

    let header = if saved_files.is_empty() {
        " No files received.".to_string()
    } else if saved_files.len() == 1 {
        " 1 file saved:".to_string()
    } else {
        format!(" {} files saved:", saved_files.len())
    };
    let mut file_lines: Vec<Line> = vec![Line::from(Span::styled(
        header,
        Style::default().fg(if saved_files.is_empty() { Color::Red } else { Color::Green }).add_modifier(Modifier::BOLD),
    ))];
    for p in saved_files {
        let size_str = match std::fs::metadata(p).map(|m| m.len()).ok() {
            Some(sz) => format!("   {}  ({})", p.display(), crate::format::format_size_u64(sz)),
            None => format!("   {}", p.display()),
        };
        file_lines.push(Line::from(size_str));
    }
    f.render_widget(Paragraph::new(file_lines), chunks[1]);

    let max_lines = chunks[3].height.saturating_sub(2) as usize;
    let start = log.len().saturating_sub(max_lines.max(1));
    let log_lines: Vec<Line> = log[start..].iter().map(|s| Line::from(s.as_str())).collect();
    f.render_widget(
        Paragraph::new(log_lines)
            .block(Block::default().borders(Borders::ALL).title(" Log ")),
        chunks[3],
    );

    hints(f, chunks[4], " [Enter/b/\u{2190}] back ");
}

fn render_done(f: &mut Frame, area: Rect, saved: &Path) {
    let inner = card(f, area, "Download complete", Color::Green);
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(2),
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
                "File saved successfully",
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
        ])),
        chunks[1],
    );

    let display = if saved.is_relative() && !saved.starts_with(".") {
        format!("./{}", saved.display())
    } else {
        saved.display().to_string()
    };
    f.render_widget(
        Paragraph::new(vec![
            section_header("Saved to"),
            Line::from(format!(" {}", display)),
        ]),
        chunks[2],
    );

    hints(f, chunks[4], "[Enter/b/\u{2190}] back");
}

fn render_failed(f: &mut Frame, area: Rect, msg: &str) {
    let inner = card(f, area, "Download failed", Color::Red);
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

    hints(f, chunks[3], "[Enter/b/\u{2190}] back");
}
