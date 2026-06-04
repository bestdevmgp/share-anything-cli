pub mod options;

use crate::tui::app::{AppCtx, ScreenAction};
use crate::tui::event::Event;

pub enum Screen {
    Options(options::State),
}

pub fn update(s: &mut Screen, ev: &Event, ctx: &mut AppCtx) -> ScreenAction {
    match s {
        Screen::Options(state) => options::update(state, ev, ctx),
    }
}

pub fn render(s: &Screen, f: &mut ratatui::Frame) {
    match s {
        Screen::Options(state) => options::render(state, f),
    }
}
