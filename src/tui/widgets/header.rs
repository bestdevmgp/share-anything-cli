use crate::client::ApiClient;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

pub fn render(f: &mut Frame, area: Rect, client: &ApiClient) {
    let right: String = if client.is_authenticated() {
        client.user_name.clone().unwrap_or_default()
    } else {
        "guest".to_string()
    };
    let line = Line::from(vec![
        Span::styled(
            " ShareAnything ",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(" - "),
        Span::raw(right),
    ]);
    f.render_widget(Paragraph::new(line), area);
}
