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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Field {
    Password,
    Submit,
}

#[derive(Debug, Clone)]
pub struct FileState {
    pub name: String,
    pub size: u64,
    /// Bytes sent so far (only meaningful when status is Sending).
    pub sent: u64,
    pub status: FileStatus,
    /// Timestamp when this file's transfer started (used for speed / ETA).
    pub started_at: Option<std::time::Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    Pending,
    Sending,
    Done,
}

#[allow(clippy::large_enum_variant)]
pub enum Phase {
    Form {
        password: TextArea<'static>,
        focus: Field,
        authenticated: bool,
    },
    Running {
        share_code: Option<String>,
        file_states: Vec<FileState>,
        receiver_info: Option<String>,
        connected_info: Option<String>,
        /// Index into file_states of the file currently being sent (None = waiting).
        active_idx: Option<usize>,
        total: u64,
        started_at: std::time::Instant,
        log: Vec<String>,
        /// ICE picked a TURN relay candidate — flagged so the UI can warn about slower throughput.
        relay_in_use: bool,
        /// Flipped to `true` after the user presses [c] so the inline "Press [c] to copy"
        /// hint can morph into a green confirmation in place.
        copied: bool,
    },
    Done {
        share_code: String,
        log: Vec<String>,
        copied: bool,
    },
    Failed(String),
}

pub struct State {
    pub paths: Vec<PathBuf>,
    pub phase: Phase,
}

impl State {
    pub fn new(paths: Vec<PathBuf>, authenticated: bool) -> Self {
        let mut password = TextArea::default();
        password.set_placeholder_text(if authenticated { "(none)" } else { "Sign in to set a password" });
        password.set_block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(Span::styled(
                    " Password (optional) ",
                    Style::default().fg(Color::DarkGray),
                )),
        );
        password.set_mask_char('\u{2022}');
        Self {
            paths,
            phase: Phase::Form {
                password,
                focus: Field::Submit,
                authenticated,
            },
        }
    }
}

pub fn update(s: &mut State, ev: &Event, ctx: &mut AppCtx) -> ScreenAction {
    match &mut s.phase {
        Phase::Form {
            password,
            focus,
            authenticated,
        } => {
            let Event::Key(k) = ev else { return ScreenAction::Stay; };
            if matches!(k.code, KeyCode::Esc) {
                return ScreenAction::Pop;
            }
            match *focus {
                Field::Submit => match k.code {
                    KeyCode::Enter => {
                        let pw_text = password.lines().join("");
                        let pw = if pw_text.is_empty() { None } else { Some(pw_text) };
                        start_send(s, pw, ctx)
                    }
                    // Lowercase 'b' (no modifiers) and Left arrow drop back to the file picker
                    // so a stray Enter on the picker can be undone. Capital 'B' (Shift+b) is
                    // deliberately excluded — it still flows into the password field along
                    // with every other typed character.
                    KeyCode::Char('b') if k.modifiers.is_empty() => ScreenAction::Pop,
                    KeyCode::Left => ScreenAction::Pop,
                    KeyCode::Char(_) if *authenticated => {
                        // Typing a character while focus is on Submit jumps focus to the
                        // password field and forwards the keystroke.
                        *focus = Field::Password;
                        password.input(event::ev_to_input(k));
                        ScreenAction::Stay
                    }
                    _ => ScreenAction::Stay,
                },
                Field::Password => match k.code {
                    KeyCode::Enter => {
                        let pw_text = password.lines().join("");
                        let pw = if pw_text.is_empty() { None } else { Some(pw_text) };
                        start_send(s, pw, ctx)
                    }
                    _ => {
                        if *authenticated {
                            password.input(event::ev_to_input(k));
                        }
                        ScreenAction::Stay
                    }
                },
            }
        }
        Phase::Running { share_code, copied, .. } => {
            let Event::Key(k) = ev else { return ScreenAction::Stay; };
            match k.code {
                KeyCode::Char('c') => {
                    if let Some(code) = share_code.as_deref() {
                        if crate::tui::copy_to_clipboard(code) {
                            *copied = true;
                        } else {
                            *ctx.toast = Some(crate::tui::widgets::toast::Toast::warn(
                                "Clipboard unavailable - code is shown above.",
                            ));
                        }
                    }
                    ScreenAction::Stay
                }
                KeyCode::Left | KeyCode::Char('b') | KeyCode::Esc => ScreenAction::Pop,
                _ => ScreenAction::Stay,
            }
        }
        Phase::Done { share_code, copied, .. } => {
            let Event::Key(k) = ev else { return ScreenAction::Stay; };
            match k.code {
                KeyCode::Char('c') => {
                    if crate::tui::copy_to_clipboard(share_code) {
                        *copied = true;
                    } else {
                        *ctx.toast = Some(crate::tui::widgets::toast::Toast::warn(
                            "Clipboard unavailable - code is shown above.",
                        ));
                    }
                    ScreenAction::Stay
                }
                // ← / b drop back into the picker so the same selection can be re-sent with
                // tweaked options. Other confirm keys go home.
                KeyCode::Left | KeyCode::Char('b') => ScreenAction::Pop,
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => ScreenAction::PopToRoot,
                _ => ScreenAction::Stay,
            }
        }
        Phase::Failed(_) => {
            let Event::Key(k) = ev else { return ScreenAction::Stay; };
            if matches!(
                k.code,
                KeyCode::Enter
                    | KeyCode::Esc
                    | KeyCode::Char('q')
                    | KeyCode::Left
                    | KeyCode::Char('b')
            ) {
                ScreenAction::Pop
            } else {
                ScreenAction::Stay
            }
        }
    }
}

fn start_send(s: &mut State, password: Option<String>, ctx: &mut AppCtx) -> ScreenAction {
    let opts = crate::core::p2p::sender::SenderOptions {
        files: s.paths.clone(),
        stdin_data: None,
        stdin_name: None,
        password,
    };
    s.phase = Phase::Running {
        share_code: None,
        file_states: vec![],
        receiver_info: None,
        connected_info: None,
        active_idx: None,
        total: 0,
        started_at: std::time::Instant::now(),
        log: vec!["Connecting to signaling server\u{2026}".into()],
        relay_in_use: false,
        copied: false,
    };
    let Some(tx) = ctx.tx.cloned() else {
        s.phase = Phase::Failed("Internal error: event channel not ready.".into());
        return ScreenAction::Stay;
    };
    let client = ctx.client.clone();
    let tx_for_events = tx.clone();
    let on_event: crate::core::p2p::sender::SenderEventFn =
        std::sync::Arc::new(move |ev| {
            let _ = tx_for_events.send(Event::P2PSend(ev));
        });
    let handle = tokio::spawn(async move {
        let r = crate::core::p2p::sender::run(&client, opts, on_event).await;
        if let Err(e) = r {
            let _ = tx.send(Event::P2PSend(
                crate::core::p2p::sender::SenderEvent::Failed(format!("Transfer failed: {}", e)),
            ));
        }
    });
    ctx.tasks.push(handle.abort_handle());
    ScreenAction::Stay
}

pub fn render(s: &State, f: &mut Frame) {
    let area = f.area();
    match &s.phase {
        Phase::Form {
            password,
            focus,
            authenticated,
        } => render_form(f, area, &s.paths, password, *focus, *authenticated),
        Phase::Running {
            share_code,
            file_states,
            receiver_info,
            connected_info,
            active_idx,
            total,
            started_at,
            log,
            relay_in_use,
            copied,
            ..
        } => {
            render_running(
                f,
                area,
                share_code.as_deref(),
                file_states,
                *active_idx,
                *total,
                receiver_info.as_deref(),
                connected_info.as_deref(),
                *started_at,
                log,
                *relay_in_use,
                *copied,
            );
        }
        Phase::Done { share_code, log, copied } => render_done(f, area, share_code, log, *copied),
        Phase::Failed(msg) => render_failed(f, area, msg),
    }
}

fn render_form(
    f: &mut Frame,
    area: Rect,
    paths: &[PathBuf],
    password: &TextArea,
    focus: Field,
    authenticated: bool,
) {
    let inner = card(f, area, "Secure Transfer (P2P)", Color::Cyan);

    let options_h: u16 = if authenticated { 4 } else { 2 };
    // Reserve space for options + submit button + spacers + hints; the file list shrinks
    // first so they can't be pushed off-screen. Truncated entries collapse into a "…" row.
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
    render_options_section(f, chunks[3], password, focus, authenticated);
    // Always-on Submit — Enter triggers start_send from either field, so the button represents
    // an always-available action rather than a focus target.
    render_submit_button(f, chunks[5], true);

    hints_bar(f, chunks[7], "[Enter] start    [b/\u{2190}] picker    [Esc] cancel");
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

fn section_header(text: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(
        text.into(),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    ))
}

fn hints_bar(f: &mut Frame, area: Rect, text: &str) {
    f.render_widget(
        Paragraph::new(text).style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

fn render_files_section(f: &mut Frame, area: Rect, paths: &[PathBuf]) {
    let cap = area.height as usize;
    if cap == 0 { return; }

    let mut lines: Vec<Line> = Vec::with_capacity(cap);
    lines.push(section_header(format!("Files ({})", paths.len())));

    // Drop the last visible row to a "…" hint when entries would otherwise be silently cut
    // off, so the user knows the list is longer than what's shown.
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
    focus: Field,
    authenticated: bool,
) {
    if !authenticated {
        f.render_widget(
            Paragraph::new(vec![
                section_header("Options"),
                Line::from(Span::styled(
                    " Sign in to set a password.",
                    Style::default().fg(Color::DarkGray),
                )),
            ]),
            area,
        );
        return;
    }

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
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
            .title(Span::styled(" Password (optional) ", title_style)),
    );
    f.render_widget(&pw, rows[1]);
}

fn render_submit_button(f: &mut Frame, area: Rect, focused: bool) {
    let btn_text = " [Enter] Start Secure Transfer ";
    let btn_width = (btn_text.chars().count() as u16 + 4).min(area.width);
    let x = area.x + (area.width.saturating_sub(btn_width)) / 2;
    let btn_area = Rect {
        x,
        y: area.y,
        width: btn_width,
        height: area.height.min(3),
    };

    let (text_style, border_style) = if focused {
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
        Paragraph::new(Span::styled(btn_text, text_style))
            .alignment(ratatui::layout::Alignment::Center),
        inner,
    );
}

enum StepState {
    Done,
    Active,
    /// Like Active but the work is intentionally idle (e.g. waiting for the receiver to pick
    /// the next file). Rendered with a pause glyph instead of the spinner.
    Paused,
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

fn session_step_line(state: StepState, share_code: Option<&str>, copied: bool) -> Line<'static> {
    let (marker, marker_style) = step_marker(&state);
    let label_style = match state {
        StepState::Pending => Style::default().fg(Color::DarkGray),
        _ => Style::default(),
    };
    let mut spans = vec![
        Span::raw("  "),
        Span::raw("["),
        Span::styled(marker, marker_style),
        Span::raw("] "),
        Span::styled(format!("{:<22}", "Session created"), label_style),
    ];
    if let Some(code) = share_code {
        spans.push(Span::styled("  - ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            code.to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        if copied {
            spans.push(Span::styled(
                "    \u{2713} Copied to clipboard",
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                "    Press [c] to copy the share code",
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            ));
        }
    }
    Line::from(spans)
}

fn step_marker(state: &StepState) -> (String, Style) {
    match state {
        StepState::Done => (
            "\u{2713}".to_string(),
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ),
        StepState::Active => (
            spinner_frame().to_string(),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        StepState::Paused => (
            "\u{23F8}".to_string(),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        StepState::Pending => (
            " ".to_string(),
            Style::default().fg(Color::DarkGray),
        ),
    }
}

fn step_line(state: StepState, label: &str, detail: Option<&str>) -> Line<'static> {
    let (marker, marker_style) = step_marker(&state);
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
fn render_running(
    f: &mut Frame,
    area: Rect,
    share_code: Option<&str>,
    file_states: &[FileState],
    active_idx: Option<usize>,
    total: u64,
    receiver_info: Option<&str>,
    connected_info: Option<&str>,
    started_at: std::time::Instant,
    log: &[String],
    relay_in_use: bool,
    copied: bool,
) {
    let active_file = active_idx.and_then(|i| file_states.get(i));
    // Big gauge is redundant when there are several files (each row already has its own inline
    // bar). Only draw it for single-file shares.
    let show_gauge = file_states.len() <= 1
        && active_file.map(|f| f.sent > 0).unwrap_or(false);
    let any_done = file_states.iter().any(|f| f.status == FileStatus::Done);
    let show_waiting_banner = active_idx.is_none() && connected_info.is_some() && any_done;
    let gauge_h: u16 = if show_gauge { 3 } else if show_waiting_banner { 1 } else { 0 };

    let file_list_h = (file_states.len().max(1) + 2) as u16;

    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(5),
        Constraint::Length(gauge_h),
        Constraint::Length(file_list_h),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Span::styled(
            " Secure Transfer (P2P) ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        chunks[0],
    );

    let any_file_done = file_states.iter().any(|f| f.status == FileStatus::Done);
    let all_files_done = !file_states.is_empty()
        && file_states.iter().all(|f| f.status == FileStatus::Done);

    let session_state = if share_code.is_some() {
        StepState::Done
    } else {
        StepState::Active
    };

    // Receiver step flips on DownloaderArrived (receiver_info), not on WebRTC PeerMatched.
    // The WebRTC negotiation happens after the user clicks Download, which conceptually
    // belongs to the Transferring phase.
    let receiver_state = if receiver_info.is_some() {
        StepState::Done
    } else if share_code.is_some() {
        StepState::Active
    } else {
        StepState::Pending
    };
    let receiver_label = if receiver_info.is_some() {
        "Receiver connected"
    } else {
        "Receiver connecting"
    };
    let receiver_detail = receiver_info;

    // Awaiting covers only the *first* request — subsequent idle gaps between files are
    // handled by the Transferring step's Paused mode below.
    let awaiting_state = if receiver_info.is_none() {
        StepState::Pending
    } else if active_idx.is_some() || any_file_done {
        StepState::Done
    } else {
        StepState::Active
    };

    // Switches to Paused (⏸) during the gap between two files of a multi-file share so the
    // user can tell we're waiting on the receiver, not stalled.
    let idle_between =
        active_idx.is_none() && any_file_done && !all_files_done;
    let transferring_state = if all_files_done {
        StepState::Done
    } else if idle_between {
        StepState::Paused
    } else if active_idx.is_some() || any_file_done {
        StepState::Active
    } else {
        StepState::Pending
    };
    let active_name: Option<String> = active_file.map(|f| f.name.clone());
    let transferring_detail_owned: Option<String> = if let Some(name) = active_name {
        if relay_in_use {
            Some(format!("{}  (TURN relay)", name))
        } else {
            Some(name)
        }
    } else if matches!(transferring_state, StepState::Paused) {
        Some("Waiting for receiver to pick next file".to_string())
    } else if relay_in_use && matches!(transferring_state, StepState::Active | StepState::Done) {
        Some("(TURN relay)".to_string())
    } else {
        None
    };
    let transferring_detail = transferring_detail_owned.as_deref();

    // Complete is only marked Done when we reach Phase::Done (handled elsewhere).
    let complete_state = StepState::Pending;

    let steps = vec![
        session_step_line(session_state, share_code, copied),
        step_line(receiver_state, receiver_label, receiver_detail),
        step_line(awaiting_state, "Awaiting request", None),
        step_line(transferring_state, "Transferring", transferring_detail),
        step_line(complete_state, "Complete", None),
    ];
    f.render_widget(Paragraph::new(steps), chunks[1]);

    if show_gauge {
        if let Some(af) = active_file {
            let ratio = if af.size > 0 { (af.sent as f64 / af.size as f64).min(1.0) } else { 0.0 };
            let label = if let Some(t) = af.started_at {
                fmt_progress(af.sent, af.size, t)
            } else {
                fmt_progress(af.sent, af.size, started_at)
            };
            let title = format!(" {} ", af.name);
            let gauge = Gauge::default()
                .block(Block::default().borders(Borders::ALL).title(title))
                .gauge_style(Style::default().fg(Color::Cyan))
                .ratio(ratio)
                .label(label);
            f.render_widget(gauge, chunks[2]);
        }
    } else if show_waiting_banner {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    spinner_frame(),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    "Waiting for next request\u{2026}",
                    Style::default().fg(Color::Yellow),
                ),
            ])),
            chunks[2],
        );
    }

    if !file_states.is_empty() {
        let _ = total;
        let file_lines: Vec<Line> = file_states.iter().enumerate().map(|(i, fs)| {
            let (marker, marker_style, detail_style) = match fs.status {
                FileStatus::Done => (
                    "\u{2713}".to_string(),
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                    Style::default().fg(Color::DarkGray),
                ),
                FileStatus::Sending => (
                    "\u{25b6}".to_string(),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    Style::default().fg(Color::White),
                ),
                FileStatus::Pending => (
                    " ".to_string(),
                    Style::default().fg(Color::DarkGray),
                    Style::default().fg(Color::DarkGray),
                ),
            };

            let detail = match fs.status {
                FileStatus::Done => {
                    format!("  ({})", crate::format::format_size_u64(fs.size))
                }
                FileStatus::Sending => {
                    let pct = if fs.size > 0 { (fs.sent as f64 / fs.size as f64 * 100.0) as u64 } else { 0 };
                    if let Some(t) = fs.started_at {
                        let elapsed = t.elapsed().as_secs_f64().max(0.001);
                        let rate = fs.sent as f64 / elapsed;
                        let speed = format!("{}/s", crate::format::format_size_u64(rate as u64));
                        let eta = if rate > 0.0 && fs.size > fs.sent {
                            let secs = ((fs.size - fs.sent) as f64 / rate).round() as u64;
                            format_duration(secs)
                        } else {
                            "-".to_string()
                        };
                        format!("  {}% \u{00b7} {} \u{00b7} ETA {}", pct, speed, eta)
                    } else {
                        format!("  {}%", pct)
                    }
                }
                FileStatus::Pending => String::new(),
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
            if matches!(fs.status, FileStatus::Sending) && fs.size > 0 {
                let bar_width: usize = 28;
                let ratio = (fs.sent as f64 / fs.size as f64).min(1.0);
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
            chunks[3],
        );
    }

    let max_lines = chunks[4].height.saturating_sub(2) as usize;
    let start = log.len().saturating_sub(max_lines.max(1));
    let log_lines: Vec<Line> = log[start..].iter().map(|s| Line::from(s.as_str())).collect();
    f.render_widget(
        Paragraph::new(log_lines)
            .block(Block::default().borders(Borders::ALL).title(" Log ")),
        chunks[4],
    );

    let hint_text = if share_code.is_some() {
        " [c] copy code    [b/\u{2190}/Esc] back    [Ctrl+C] cancel "
    } else {
        " [b/\u{2190}/Esc] back    [Ctrl+C] cancel "
    };
    f.render_widget(
        Paragraph::new(hint_text).style(Style::default().fg(Color::DarkGray)),
        chunks[5],
    );
}

fn render_done(f: &mut Frame, area: Rect, share_code: &str, log: &[String], copied: bool) {
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .split(area);
    f.render_widget(
        Paragraph::new(Span::styled(
            " \u{2713} Secure transfer complete! ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
        chunks[0],
    );
    let copy_hint = if copied {
        Line::from(Span::styled(
            " \u{2713} Copied to clipboard",
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ))
    } else {
        Line::from(Span::styled(
            " Press [c] to copy the share code",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
        ))
    };
    f.render_widget(
        Paragraph::new(vec![
            Line::from(format!(" Share code was: {}", share_code)),
            copy_hint,
        ]),
        chunks[1],
    );
    let max_lines = chunks[2].height.saturating_sub(2) as usize;
    let start = log.len().saturating_sub(max_lines.max(1));
    let log_lines: Vec<Line> = log[start..].iter().map(|s| Line::from(s.as_str())).collect();
    f.render_widget(
        Paragraph::new(log_lines)
            .block(Block::default().borders(Borders::ALL).title(" Log ")),
        chunks[2],
    );
    f.render_widget(
        Paragraph::new(" [c] copy code    [Enter/Esc/q] home    [b/\u{2190}] new transfer ")
            .style(Style::default().fg(Color::DarkGray)),
        chunks[3],
    );
}

fn render_failed(f: &mut Frame, area: Rect, msg: &str) {
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(5),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);
    f.render_widget(
        Paragraph::new(Span::styled(
            " \u{2717} Secure transfer failed ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new(format!(" {}", msg)).style(Style::default().fg(Color::Red)),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new(" [Enter/Esc/q/b/\u{2190}] retry ")
            .style(Style::default().fg(Color::DarkGray)),
        chunks[3],
    );
}

fn fmt_progress(done: u64, total: u64, started_at: std::time::Instant) -> String {
    let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
    let rate = (done as f64) / elapsed;
    let pct = if total == 0 {
        0.0
    } else {
        (done as f64 / total as f64) * 100.0
    };
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
        eta
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
