use crate::tui::app::{AppCtx, ScreenAction};
use crate::tui::event::Event;
use crate::tui::widgets::confirm::ConfirmChoice;
use crossterm::event::KeyCode;
use ratatui::Frame;

pub struct State {
    pub choice: ConfirmChoice,
}

impl State {
    pub fn new() -> Self {
        Self { choice: ConfirmChoice::Yes }
    }
}

pub fn update(s: &mut State, ev: &Event, _ctx: &mut AppCtx) -> ScreenAction {
    let Event::Key(k) = ev else { return ScreenAction::Stay; };
    match k.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => ScreenAction::LogOut,
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
            ConfirmChoice::Yes => ScreenAction::LogOut,
            ConfirmChoice::No => ScreenAction::Pop,
        },
        _ => ScreenAction::Stay,
    }
}

pub fn render(s: &State, f: &mut Frame) {
    let area = f.area();
    crate::tui::widgets::confirm::render(
        f,
        area,
        "Sign out",
        "Sign out of ShareAnything? You will need to sign in again to manage your shares.",
        s.choice,
    );
}
