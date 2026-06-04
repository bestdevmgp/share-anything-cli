use crate::core::shares::FileInfo;
use crate::tui::app::{AppCtx, ScreenAction};
use crate::tui::event::{self, Event};
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use tui_textarea::TextArea;

#[allow(clippy::large_enum_variant)]
pub enum Phase {
    InputCode { code: TextArea<'static> },
    Loading { code: String },
    Loaded(FileInfo),
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
        Self { phase: Phase::Loading { code } }
    }
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
                        let _ = tx.send(Event::InfoLoaded(r));
                    });
                    ctx.tasks.push(handle.abort_handle());
                    s.phase = Phase::Loading { code: code_text };
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
        Phase::Loading { .. } => ScreenAction::Stay,
        Phase::Loaded(info) => {
            let Event::Key(k) = ev else { return ScreenAction::Stay; };
            match k.code {
                KeyCode::Char('d') => ScreenAction::PushDownloadForCode(info.share_code.clone()),
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q')
                    | KeyCode::Left | KeyCode::Char('b') => ScreenAction::Pop,
                _ => ScreenAction::Stay,
            }
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
            " Info ",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        chunks[0],
    );

    match &s.phase {
        Phase::InputCode { code } => {
            let body = chunks[1];
            let input_width = std::cmp::min(body.width, 28);
            let input_height: u16 = 3;
            let x = body.x + body.width.saturating_sub(input_width) / 2;
            let y = body.y + 2;
            let input_area = ratatui::layout::Rect { x, y, width: input_width, height: input_height };
            let above = ratatui::layout::Rect { x: body.x, y: body.y, width: body.width, height: 2 };
            f.render_widget(
                Paragraph::new(" Enter a share code:")
                    .style(Style::default().fg(Color::DarkGray)),
                above,
            );
            f.render_widget(code, input_area);
        }
        Phase::Loading { code } => {
            f.render_widget(Paragraph::new(format!(" Loading {}…", code)), chunks[1]);
        }
        Phase::Loaded(info) => render_info(f, chunks[1], info),
        Phase::Failed(msg) => f.render_widget(
            Paragraph::new(format!(" {}", msg)).style(Style::default().fg(Color::Red)),
            chunks[1],
        ),
    }

    let hint_text = match &s.phase {
        Phase::Loaded(_) => " [d] download  [Enter/b/\u{2190}] back  [Esc] back ",
        _ => " [Enter/b/\u{2190}] back  [Esc] back ",
    };
    f.render_widget(
        Paragraph::new(hint_text)
            .style(Style::default().fg(Color::DarkGray)),
        chunks[2],
    );
}

fn render_info(f: &mut Frame, area: Rect, info: &FileInfo) {
    let mut lines: Vec<Line> = vec![
        Line::from(format!(" Share code : {}", info.share_code)),
    ];
    if info.transfer_type.as_deref() == Some("p2p") {
        lines.push(Line::from(" Transfer   : Secure (P2P)"));
    }
    lines.push(Line::from(format!(
        " Password   : {}",
        if info.has_password { "Yes" } else { "No" }
    )));
    lines.push(Line::from(format!(
        " One-time   : {}",
        if info.is_one_time { "Yes" } else { "No" }
    )));
    lines.push(Line::from(format!(
        " Expires at : {}",
        crate::time::utc_to_local(&info.expires_at)
    )));

    if info.has_password {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " \u{1f512} This share is password-protected.",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "    File details are hidden. Press [d] to download with the password.",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(format!(" Files ({}):", info.files.len())));
        for fd in &info.files {
            lines.push(Line::from(format!(
                "   - {} ({})",
                fd.file_name,
                crate::format::format_size(fd.file_size)
            )));
        }
    }
    f.render_widget(Paragraph::new(lines), area);
}
