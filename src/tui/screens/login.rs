use crate::core::auth::DeviceSession;
use crate::tui::app::{AppCtx, ScreenAction};
use crate::tui::event::Event;
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use std::time::{Duration, Instant};

pub enum Phase {
    Starting,
    /// Session created; polling status every 2s.
    WaitingDevice {
        session: DeviceSession,
        qr_lines: Vec<String>,
        last_poll: Instant,
        poll_inflight: bool,
        started_at: Instant,
    },
    Failed(String),
}

pub struct State {
    pub phase: Phase,
}

impl State {
    pub fn new() -> Self {
        Self { phase: Phase::Starting }
    }
}

const POLL_INTERVAL: Duration = Duration::from_secs(2);

pub fn on_push(s: &mut State, ctx: &mut AppCtx) {
    spawn_start_session(s, ctx);
}

fn spawn_start_session(s: &mut State, ctx: &mut AppCtx) {
    let Some(tx) = ctx.tx.cloned() else {
        s.phase = Phase::Failed("Internal error: event channel not ready.".into());
        return;
    };
    let cfg = ctx.cfg.clone();
    s.phase = Phase::Starting;
    let handle = tokio::spawn(async move {
        let r = crate::core::auth::start_device_session(&cfg).await;
        let _ = tx.send(Event::LoginSessionReady(r));
    });
    ctx.tasks.push(handle.abort_handle());
}

pub fn update(s: &mut State, ev: &Event, ctx: &mut AppCtx) -> ScreenAction {
    match &mut s.phase {
        Phase::Starting => {
            if let Event::Key(k) = ev {
                if matches!(k.code, KeyCode::Esc | KeyCode::Left | KeyCode::Char('b')) {
                    return ScreenAction::Pop;
                }
            }
            ScreenAction::Stay
        }
        Phase::WaitingDevice { last_poll, poll_inflight, session, started_at, .. } => {
            if matches!(ev, Event::Tick) {
                let now = Instant::now();
                if !*poll_inflight && now.duration_since(*last_poll) >= POLL_INTERVAL {
                    let expired = now.duration_since(*started_at)
                        >= Duration::from_secs(session.expires_in_seconds);
                    if expired {
                        s.phase = Phase::Failed("Session expired. Try again.".into());
                        return ScreenAction::Stay;
                    }
                    let Some(tx) = ctx.tx.cloned() else { return ScreenAction::Stay; };
                    let cfg = ctx.cfg.clone();
                    let session_id = session.session_id.clone();
                    *last_poll = now;
                    *poll_inflight = true;
                    let handle = tokio::spawn(async move {
                        let r = crate::core::auth::poll_device_status(&cfg, &session_id).await;
                        let _ = tx.send(Event::LoginPolled(r));
                    });
                    ctx.tasks.push(handle.abort_handle());
                }
                return ScreenAction::Stay;
            }
            if let Event::Key(k) = ev {
                if matches!(k.code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Left | KeyCode::Char('b')) {
                    return ScreenAction::Pop;
                }
            }
            ScreenAction::Stay
        }
        Phase::Failed(_) => {
            let Event::Key(k) = ev else { return ScreenAction::Stay; };
            if matches!(k.code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') | KeyCode::Left | KeyCode::Char('b')) {
                ScreenAction::Pop
            } else {
                ScreenAction::Stay
            }
        }
    }
}

pub fn render(s: &State, f: &mut Frame) {
    let area = f.area();
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Span::styled(
            " Sign in ",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        chunks[0],
    );

    match &s.phase {
        Phase::Starting => f.render_widget(
            Paragraph::new(" Creating sign-in session…"),
            chunks[1],
        ),
        Phase::WaitingDevice { session, qr_lines, .. } => {
            render_waiting(f, chunks[1], session, qr_lines);
        }
        Phase::Failed(msg) => f.render_widget(
            Paragraph::new(format!(" \u{2717} {}", msg))
                .style(Style::default().fg(Color::Red)),
            chunks[1],
        ),
    }

    let hint_text = match &s.phase {
        Phase::Starting => " [Esc/b/\u{2190}] cancel ",
        Phase::WaitingDevice { .. } => " [Esc/q/b/\u{2190}] cancel ",
        Phase::Failed(_) => " [Enter/b/\u{2190}] back ",
    };
    f.render_widget(
        Paragraph::new(hint_text).style(Style::default().fg(Color::DarkGray)),
        chunks[2],
    );
}

fn render_waiting(f: &mut Frame, area: Rect, session: &DeviceSession, qr_lines: &[String]) {
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(vec![
            Line::from(" Open this URL in a browser:"),
            Line::from(Span::styled(
                format!(" {}", session.login_url),
                Style::default().fg(Color::Cyan),
            )),
        ]),
        chunks[0],
    );

    let qr_para: Vec<Line> = qr_lines.iter().map(|s| Line::from(s.clone())).collect();
    f.render_widget(Paragraph::new(qr_para), chunks[1]);

    f.render_widget(
        Paragraph::new(" Waiting for sign-in…")
            .style(Style::default().fg(Color::DarkGray)),
        chunks[2],
    );
}

/// Render a QR for `url` as Vec<String>, one line per terminal row.
/// Uses half-block chars to fit two QR rows per terminal row, matching the CLI version.
pub fn build_qr_lines(url: &str) -> Vec<String> {
    use qrcode::{EcLevel, QrCode};
    let code = match QrCode::with_error_correction_level(url.as_bytes(), EcLevel::L) {
        Ok(c) => c,
        Err(_) => return vec!["(Failed to generate QR code)".into()],
    };
    let w = code.width();
    let data = code.to_colors();
    let is_dark = |x: usize, y: usize| -> bool {
        x >= 1 && y >= 1 && x <= w && y <= w
            && data[(y - 1) * w + (x - 1)] == qrcode::Color::Dark
    };
    let total = w + 2;
    let mut lines: Vec<String> = Vec::with_capacity(total / 2 + 1);
    for y in (0..total).step_by(2) {
        let mut line = String::with_capacity(total);
        for x in 0..total {
            line.push(match (is_dark(x, y), is_dark(x, y + 1)) {
                (true, true) => '\u{2588}',
                (true, false) => '\u{2580}',
                (false, true) => '\u{2584}',
                (false, false) => ' ',
            });
        }
        lines.push(line);
    }
    lines
}
