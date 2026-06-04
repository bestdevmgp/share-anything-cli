use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmChoice {
    Yes,
    No,
}

impl ConfirmChoice {
    pub fn toggle(self) -> Self {
        match self {
            ConfirmChoice::Yes => ConfirmChoice::No,
            ConfirmChoice::No => ConfirmChoice::Yes,
        }
    }
}

pub fn render(
    f: &mut Frame,
    area: Rect,
    title: &str,
    message: &str,
    selected: ConfirmChoice,
) {
    let width = std::cmp::min(area.width, 60);
    let content_w = width.saturating_sub(4).max(1) as usize;
    let msg_lines = wrapped_line_count(message, content_w).max(1);
    // 2 borders + 1 blank + msg_lines + 1 blank + 1 buttons.
    let needed: u16 = 5 + msg_lines as u16;
    let height = std::cmp::min(area.height, needed);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    let box_area = Rect { x, y, width, height };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            format!(" {} ", title),
            Style::default().add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);

    // Anchor buttons to the bottom so a long wrapped message can't push them off-screen.
    let chunks = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(inner);

    let body = Paragraph::new(vec![
        Line::from(""),
        Line::from(format!(" {}", message)),
    ])
    .wrap(Wrap { trim: false });
    f.render_widget(body, chunks[0]);

    let selected_style = Style::default()
        .fg(Color::Black)
        .bg(Color::White)
        .add_modifier(Modifier::BOLD);
    let plain_style = Style::default().fg(Color::White);
    let buttons = Line::from(vec![
        Span::styled(
            " [y] yes ",
            if selected == ConfirmChoice::Yes { selected_style } else { plain_style },
        ),
        Span::raw("    "),
        Span::styled(
            " [n] no ",
            if selected == ConfirmChoice::No { selected_style } else { plain_style },
        ),
    ]);
    f.render_widget(
        Paragraph::new(buttons).alignment(Alignment::Center),
        chunks[1],
    );
}

/// Byte-length wrap estimate — good enough for the ASCII messages this widget hosts.
fn wrapped_line_count(text: &str, width: usize) -> usize {
    if width == 0 { return 1; }
    text.split('\n')
        .map(|l| ((l.len() + width - 1) / width).max(1))
        .sum()
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, layout::Position, Terminal};

    #[test]
    fn confirm_centers_in_small_area() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                super::render(f, f.area(), "Test", "Message?", super::ConfirmChoice::Yes);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .filter_map(|(x, y)| buf.cell(Position { x, y }))
            .flat_map(|c| c.symbol().chars())
            .collect();
        assert!(text.contains("Test"), "buffer should contain title 'Test'");
        assert!(text.contains("Message?"), "buffer should contain message");
        assert!(text.contains("Yes"), "buffer should contain 'Yes'");
        assert!(text.contains("No"), "buffer should contain 'No'");
    }
}
