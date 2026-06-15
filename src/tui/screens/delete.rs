use crate::tui::app::{AppCtx, ScreenAction};
use crate::tui::event::Event;
use crate::tui::widgets::confirm::ConfirmChoice;
use crossterm::event::KeyCode;
use ratatui::Frame;

pub enum Phase {
    Awaiting,
    Deleting,
    Failed(String),
}

pub enum Target {
    Single { code: String, file_name: String },
    All { count: usize },
}

pub struct State {
    pub target: Target,
    pub phase: Phase,
    pub choice: ConfirmChoice,
}

impl State {
    pub fn new(code: String, file_name: String) -> Self {
        Self {
            target: Target::Single { code, file_name },
            phase: Phase::Awaiting,
            choice: ConfirmChoice::Yes,
        }
    }

    pub fn new_all(count: usize) -> Self {
        Self {
            target: Target::All { count },
            phase: Phase::Awaiting,
            choice: ConfirmChoice::Yes,
        }
    }
}

pub fn update(s: &mut State, ev: &Event, ctx: &mut AppCtx) -> ScreenAction {
    let Event::Key(k) = ev else { return ScreenAction::Stay; };
    match &s.phase {
        Phase::Awaiting => match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => start_delete(s, ctx),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('b') => {
                ScreenAction::Pop
            }
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Char('h')
            | KeyCode::Char('l')
            | KeyCode::Tab
            | KeyCode::BackTab => {
                s.choice = s.choice.toggle();
                ScreenAction::Stay
            }
            KeyCode::Enter => match s.choice {
                ConfirmChoice::Yes => start_delete(s, ctx),
                ConfirmChoice::No => ScreenAction::Pop,
            },
            _ => ScreenAction::Stay,
        },
        Phase::Deleting => ScreenAction::Stay,
        Phase::Failed(_) => match k.code {
            KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('n') | KeyCode::Left | KeyCode::Char('b') => {
                ScreenAction::Pop
            }
            _ => ScreenAction::Stay,
        },
    }
}

fn start_delete(s: &mut State, ctx: &mut AppCtx) -> ScreenAction {
    let Some(tx) = ctx.tx.cloned() else {
        s.phase = Phase::Failed("Internal error: event channel not ready.".into());
        return ScreenAction::Stay;
    };
    let client = ctx.client.clone();
    s.phase = Phase::Deleting;
    match &s.target {
        Target::Single { code, .. } => {
            let code = code.clone();
            let handle = tokio::spawn(async move {
                let r = crate::core::shares::delete_share(&client, &code).await;
                let _ = tx.send(Event::DeleteFinished(r.map(|()| code)));
            });
            ctx.tasks.push(handle.abort_handle());
        }
        Target::All { .. } => {
            let handle = tokio::spawn(async move {
                let r = crate::core::shares::delete_all_shares(&client).await;
                let _ = tx.send(Event::DeleteAllFinished(r));
            });
            ctx.tasks.push(handle.abort_handle());
        }
    }
    ScreenAction::Stay
}

pub fn render(s: &State, f: &mut Frame) {
    let area = f.area();
    match &s.phase {
        Phase::Awaiting => match &s.target {
            Target::Single { code, file_name } => {
                let msg = format!("Delete share \"{}\" ({})?", file_name, code);
                crate::tui::widgets::confirm::render(f, area, "Confirm delete", &msg, s.choice);
            }
            Target::All { count } => {
                let msg = format!(
                    "Delete ALL {} shares?\nDownloaders will no longer be able to fetch them.",
                    count
                );
                crate::tui::widgets::confirm::render(f, area, "Delete all shares", &msg, s.choice);
            }
        },
        Phase::Deleting => {
            let msg = match &s.target {
                Target::Single { code, .. } => format!("Deleting {}…", code),
                Target::All { count } => format!("Deleting {} shares…", count),
            };
            render_status_box(f, area, "Deleting\u{2026}", &msg, None);
        }
        Phase::Failed(err) => {
            render_status_box(
                f,
                area,
                "Delete failed",
                &format!("{}\n\n[Enter] back", err),
                None,
            );
        }
    }
}

fn render_status_box(
    f: &mut Frame,
    area: ratatui::layout::Rect,
    title: &str,
    message: &str,
    _placeholder: Option<()>,
) {
    use ratatui::{
        layout::Rect,
        style::{Modifier, Style},
        text::{Line, Span},
        widgets::{Block, Borders, Paragraph, Wrap},
    };
    let width = std::cmp::min(area.width, 60);
    let lines: Vec<Line> = std::iter::once(Line::from(""))
        .chain(message.split('\n').map(|p| Line::from(format!(" {}", p))))
        .collect();
    let needed: u16 = (lines.len() as u16 + 2).min(area.height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(needed) / 2;
    let box_area = Rect { x, y, width, height: needed };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            format!(" {} ", title),
            Style::default().add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
