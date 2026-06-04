use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::Paragraph,
    Frame,
};

#[allow(dead_code)]
pub fn render(f: &mut Frame, area: Rect, hints: &[&str]) {
    let text = hints.join("  ");
    f.render_widget(
        Paragraph::new(Line::from(text)).style(Style::default().fg(Color::DarkGray)),
        area,
    );
}
