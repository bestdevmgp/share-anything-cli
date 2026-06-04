pub mod picker;
pub mod options;

use crate::tui::app::{AppCtx, ScreenAction};
use crate::tui::event::Event;

#[allow(clippy::large_enum_variant)]
pub enum Screen {
    Picker(picker::State),
    Options(options::State),
}

impl Screen {
    pub fn picker_start() -> std::io::Result<Self> {
        Ok(Self::Picker(picker::State::new()?))
    }
}

pub fn update(s: &mut Screen, ev: &Event, ctx: &mut AppCtx) -> ScreenAction {
    match s {
        Screen::Picker(state) => picker::update(state, ev, ctx),
        Screen::Options(state) => options::update(state, ev, ctx),
    }
}

pub fn render(s: &Screen, f: &mut ratatui::Frame) {
    match s {
        Screen::Picker(state) => picker::render(state, f),
        Screen::Options(state) => options::render(state, f),
    }
}
