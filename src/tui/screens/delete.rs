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

pub struct State {
    pub code: String,
    pub file_name: String,
    pub phase: Phase,
    pub choice: ConfirmChoice,
}

impl State {
    pub fn new(code: String, file_name: String) -> Self {
        Self {
            code,
            file_name,
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
    let code = s.code.clone();
    s.phase = Phase::Deleting;
    let handle = tokio::spawn(async move {
        let r = crate::core::shares::delete_share(&client, &code).await;
        // Send the code back so the app can locate and remove the row.
        let _ = tx.send(Event::DeleteFinished(r.map(|()| code)));
    });
    ctx.tasks.push(handle.abort_handle());
    ScreenAction::Stay
}

pub fn render(s: &State, f: &mut Frame) {
    let area = f.area();
    match &s.phase {
        Phase::Awaiting => {
            let msg = format!("Delete share \"{}\" ({})?", s.file_name, s.code);
            crate::tui::widgets::confirm::render(f, area, "Confirm delete", &msg, s.choice);
        }
        Phase::Deleting => {
            crate::tui::widgets::confirm::render(
                f,
                area,
                "Deleting\u{2026}",
                &format!("Deleting {}…", s.code),
                s.choice,
            );
        }
        Phase::Failed(err) => {
            crate::tui::widgets::confirm::render(
                f,
                area,
                "Delete failed",
                &format!("{}\n\n[Enter] back", err),
                s.choice,
            );
        }
    }
}
