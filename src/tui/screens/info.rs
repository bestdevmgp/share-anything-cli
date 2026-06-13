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
                KeyCode::Char('c') => {
                    let ok = crate::tui::copy_to_clipboard(&info.share_code);
                    *ctx.toast = Some(if ok {
                        crate::tui::widgets::toast::Toast::success("Share code copied to clipboard.")
                    } else {
                        crate::tui::widgets::toast::Toast::warn("Clipboard unavailable - code is shown above.")
                    });
                    ScreenAction::Stay
                }
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
        Phase::InputCode { code } => render_input_code(f, chunks[1], code),
        Phase::Loading { code } => render_loading(f, chunks[1], code),
        Phase::Loaded(info) => render_loaded(f, chunks[1], info),
        Phase::Failed(msg) => render_failed(f, chunks[1], msg),
    }

    let hint_text = match &s.phase {
        Phase::Loaded(info) => {
            if info.has_password {
                " [d] download with password    [c] copy code    [Enter/Esc/q/b/\u{2190}] back "
            } else {
                " [d] download    [c] copy code    [Enter/Esc/q/b/\u{2190}] back "
            }
        }
        Phase::InputCode { .. } => " [Enter] look up    [Esc] back ",
        _ => " [Esc] back ",
    };
    f.render_widget(
        Paragraph::new(hint_text).style(Style::default().fg(Color::DarkGray)),
        chunks[2],
    );
}

fn render_input_code(f: &mut Frame, area: Rect, code: &TextArea) {
    let input_width = std::cmp::min(area.width, 28);
    let input_height: u16 = 3;
    let x = area.x + area.width.saturating_sub(input_width) / 2;
    let y = area.y + 2;
    let input_area = Rect { x, y, width: input_width, height: input_height };
    let above = Rect { x: area.x, y: area.y, width: area.width, height: 2 };
    f.render_widget(
        Paragraph::new(" Enter a share code:")
            .style(Style::default().fg(Color::DarkGray)),
        above,
    );
    f.render_widget(code, input_area);
}

fn render_loading(f: &mut Frame, area: Rect, code: &str) {
    let inner = super::download::card(f, area, "Share info", Color::Cyan);
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Looking up ", Style::default().fg(Color::DarkGray)),
            Span::styled(code.to_string(), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("\u{2026}", Style::default().fg(Color::DarkGray)),
        ])),
        chunks[1],
    );
}

fn render_failed(f: &mut Frame, area: Rect, msg: &str) {
    let inner = super::download::card(f, area, "Share info", Color::Red);
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " \u{2717}  ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled(msg.to_string(), Style::default().fg(Color::Red)),
        ])),
        chunks[1],
    );
}

fn render_loaded(f: &mut Frame, area: Rect, info: &FileInfo) {
    let inner = super::download::card(f, area, "Share info", Color::Cyan);

    let total_bytes: i64 = info.files.iter().map(|f| f.file_size).sum();
    let total_str = crate::format::format_size(total_bytes);

    let body_lines_for_protected: u16 = if info.has_password { 5 } else { 0 };
    let files_count = info.files.len() as u16;
    let fixed_h: u16 = 1 + 1 + 1 + body_lines_for_protected;
    let want_files_h = if info.has_password { 0 } else { 1 + files_count + 1 };
    let max_files_h = inner.height.saturating_sub(fixed_h);
    let files_h = want_files_h.min(max_files_h);

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(files_h),
        Constraint::Length(body_lines_for_protected),
        Constraint::Min(0),
    ])
    .split(inner);

    super::download::render_info_bar(f, chunks[1], info);

    if !info.has_password {
        let header = Line::from(vec![
            Span::styled(
                format!(" Files ({})", info.files.len()),
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" \u{00b7} {} total", total_str),
                Style::default().fg(Color::DarkGray),
            ),
        ]);

        let area = chunks[3];
        let cap = area.height as usize;
        if cap > 0 {
            let name_col = area.width.saturating_sub(20).max(8);
            let visible_rows = cap.saturating_sub(1);
            let total = info.files.len();
            let need_overflow = total > visible_rows;
            let shown = if need_overflow { visible_rows.saturating_sub(1) } else { total };

            let mut lines: Vec<Line> = Vec::with_capacity(cap);
            lines.push(header);
            for fd in info.files.iter().take(shown) {
                let size = crate::format::format_size(fd.file_size);
                let name = truncate_middle(&fd.file_name, name_col as usize);
                let pad = (name_col as usize).saturating_sub(visible_width(&name));
                lines.push(Line::from(vec![
                    Span::styled(" \u{2022} ", Style::default().fg(Color::Cyan)),
                    Span::raw(name),
                    Span::raw(" ".repeat(pad + 2)),
                    Span::styled(size, Style::default().fg(Color::DarkGray)),
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
    } else {
        let notice = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    " \u{1f512} ",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "Password-protected",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(Span::styled(
                "    File details are hidden until the correct password is entered.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "    Press [d] to start the download and enter the password.",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        f.render_widget(Paragraph::new(notice), chunks[4]);
    }
}

fn visible_width(s: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    UnicodeWidthStr::width(s)
}

fn truncate_middle(s: &str, max: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    let total = visible_width(s);
    if total <= max || max < 3 {
        return s.to_string();
    }
    let keep = max - 1;
    let head_budget = keep / 2;
    let tail_budget = keep - head_budget;

    let chars: Vec<char> = s.chars().collect();
    let mut head = String::new();
    let mut head_w = 0;
    for ch in &chars {
        let w = UnicodeWidthChar::width(*ch).unwrap_or(0);
        if head_w + w > head_budget { break; }
        head.push(*ch);
        head_w += w;
    }
    let mut tail_buf: Vec<char> = Vec::new();
    let mut tail_w = 0;
    for ch in chars.iter().rev() {
        let w = UnicodeWidthChar::width(*ch).unwrap_or(0);
        if tail_w + w > tail_budget { break; }
        tail_buf.push(*ch);
        tail_w += w;
    }
    let tail: String = tail_buf.into_iter().rev().collect();

    format!("{}\u{2026}{}", head, tail)
}
