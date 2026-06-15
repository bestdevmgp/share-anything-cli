use crate::client::ApiClient;
use crate::core::shares::{DownloadItem, UploadItem};
use crate::tui::app::{AppCtx, Screen, ScreenAction};
use crate::tui::event::Event;
use crate::tui::widgets::toast::Toast;
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelFocus {
    Uploads,
    Downloads,
    Actions,
}

pub struct State {
    pub loading: bool,
    pub items: Vec<UploadItem>,
    pub selected: usize,
    pub load_error: Option<String>,
    pub table_state: TableState,
    pub focus: PanelFocus,
    pub actions_cursor: usize,

    pub downloads_loading: bool,
    pub downloads: Vec<DownloadItem>,
    pub downloads_selected: usize,
    pub downloads_load_error: Option<String>,
    pub downloads_table_state: TableState,

    pub update_available: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            loading: false,
            items: Vec::new(),
            selected: 0,
            load_error: None,
            table_state: TableState::default(),
            focus: PanelFocus::Actions,
            actions_cursor: 0,
            downloads_loading: false,
            downloads: Vec::new(),
            downloads_selected: 0,
            downloads_load_error: None,
            downloads_table_state: TableState::default(),
            update_available: None,
        }
    }
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Upload,
    Download,
    SecureTransfer,
    Info,
    Delete,
    DeleteAll,
    Logout,
    SignIn,
    Quit,
}

fn visible_actions(authenticated: bool) -> &'static [Action] {
    if authenticated {
        &[
            Action::Upload,
            Action::Download,
            Action::SecureTransfer,
            Action::Info,
            Action::Delete,
            Action::DeleteAll,
            Action::Logout,
            Action::Quit,
        ]
    } else {
        &[
            Action::Upload,
            Action::Download,
            Action::SecureTransfer,
            Action::Info,
            Action::SignIn,
            Action::Quit,
        ]
    }
}

fn action_key(a: Action) -> &'static str {
    match a {
        Action::Upload => "u",
        Action::Download => "d",
        Action::SecureTransfer => "s",
        Action::Info => "i",
        Action::Delete => "x",
        Action::DeleteAll => "X",
        Action::Logout => "L",
        Action::SignIn => "L",
        Action::Quit => "Q",
    }
}

fn action_label(a: Action) -> &'static str {
    match a {
        Action::Upload => "Upload",
        Action::Download => "Download",
        Action::SecureTransfer => "Secure Transfer",
        Action::Info => "Info",
        Action::Delete => "Delete",
        Action::DeleteAll => "Delete all",
        Action::Logout => "Sign out",
        Action::SignIn => "Sign in",
        Action::Quit => "Quit",
    }
}

fn execute_action(a: Action, s: &mut State, ctx: &mut AppCtx) -> ScreenAction {
    match a {
        Action::Upload => {
            match crate::tui::screens::upload::Screen::picker_start() {
                Ok(picker) => ScreenAction::Push(Screen::Upload(picker)),
                Err(e) => {
                    *ctx.toast = Some(crate::tui::widgets::toast::Toast::error(
                        format!("Cannot open file picker: {}", e),
                    ));
                    ScreenAction::Stay
                }
            }
        }
        Action::Download => {
            let state = crate::tui::screens::download::State::new();
            ScreenAction::Push(Screen::Download(state))
        }
        Action::SecureTransfer => {
            match crate::tui::screens::upload::picker::State::new_secure() {
                Ok(state) => ScreenAction::Push(Screen::Upload(
                    crate::tui::screens::upload::Screen::Picker(state),
                )),
                Err(e) => {
                    *ctx.toast = Some(Toast::error(format!("Cannot open file picker: {}", e)));
                    ScreenAction::Stay
                }
            }
        }
        Action::Info => {
            let state = crate::tui::screens::info::State::new();
            ScreenAction::Push(Screen::Info(state))
        }
        Action::Delete => {
            if s.items.is_empty() {
                *ctx.toast = Some(crate::tui::widgets::toast::Toast::warn("No shares to delete."));
                ScreenAction::Stay
            } else if s.focus != PanelFocus::Uploads {
                s.focus = PanelFocus::Uploads;
                s.table_state.select(Some(s.selected));
                *ctx.toast = Some(crate::tui::widgets::toast::Toast::info(
                    "Pick the upload to delete, then press [x].",
                ));
                ScreenAction::Stay
            } else if let Some(item) = s.items.get(s.selected) {
                let state = crate::tui::screens::delete::State::new(
                    item.share_code.clone(),
                    item.file_name.clone(),
                );
                ScreenAction::Push(crate::tui::app::Screen::Delete(state))
            } else {
                ScreenAction::Stay
            }
        }
        Action::DeleteAll => start_delete_all(s, ctx),
        Action::Logout => ScreenAction::Push(crate::tui::app::Screen::Logout(
            crate::tui::screens::logout::State::new(),
        )),
        Action::SignIn => ScreenAction::PushLogin,
        Action::Quit => ScreenAction::Quit,
    }
}

fn refresh_history(s: &mut State, ctx: &mut AppCtx) -> ScreenAction {
    if !ctx.client.is_authenticated() {
        *ctx.toast = Some(crate::tui::widgets::toast::Toast::warn(
            "Sign in to see your history.",
        ));
        return ScreenAction::Stay;
    }
    let Some(tx) = ctx.tx.cloned() else {
        return ScreenAction::Stay;
    };
    s.loading = true;
    s.downloads_loading = true;
    s.load_error = None;
    s.downloads_load_error = None;

    let client_u = ctx.client.clone();
    let tx_u = tx.clone();
    let h_u = tokio::spawn(async move {
        let r = crate::core::shares::list_my_uploads(&client_u).await;
        let _ = tx_u.send(Event::UploadsLoaded(r));
    });
    let client_d = ctx.client.clone();
    let h_d = tokio::spawn(async move {
        let r = crate::core::shares::list_my_downloads(&client_d).await;
        let _ = tx.send(Event::DownloadsLoaded(r));
    });
    ctx.tasks.push(h_u.abort_handle());
    ctx.tasks.push(h_d.abort_handle());
    ScreenAction::Stay
}

fn start_delete_all(s: &State, ctx: &mut AppCtx) -> ScreenAction {
    if !ctx.client.is_authenticated() {
        *ctx.toast = Some(crate::tui::widgets::toast::Toast::warn(
            "Sign in to manage shares.",
        ));
        ScreenAction::Stay
    } else if s.items.is_empty() {
        *ctx.toast = Some(crate::tui::widgets::toast::Toast::warn(
            "No shares to delete.",
        ));
        ScreenAction::Stay
    } else {
        let state = crate::tui::screens::delete::State::new_all(s.items.len());
        ScreenAction::Push(crate::tui::app::Screen::Delete(state))
    }
}

fn cycle_focus_next(s: &mut State, authenticated: bool) {
    let order = [PanelFocus::Uploads, PanelFocus::Downloads, PanelFocus::Actions];
    let start = order.iter().position(|p| *p == s.focus).unwrap_or(0);
    for i in 1..=order.len() {
        let cand = order[(start + i) % order.len()];
        if focus_is_reachable(s, cand, authenticated) {
            s.focus = cand;
            return;
        }
    }
}

fn cycle_focus_prev(s: &mut State, authenticated: bool) {
    let order = [PanelFocus::Uploads, PanelFocus::Downloads, PanelFocus::Actions];
    let start = order.iter().position(|p| *p == s.focus).unwrap_or(0);
    for i in 1..=order.len() {
        let cand = order[(start + order.len() - i) % order.len()];
        if focus_is_reachable(s, cand, authenticated) {
            s.focus = cand;
            return;
        }
    }
}

fn focus_is_reachable(s: &State, p: PanelFocus, authenticated: bool) -> bool {
    match p {
        PanelFocus::Actions => true,
        PanelFocus::Uploads => authenticated && !s.items.is_empty(),
        PanelFocus::Downloads => authenticated && !s.downloads.is_empty(),
    }
}

pub fn update(s: &mut State, ev: &Event, ctx: &mut AppCtx) -> ScreenAction {
    let authenticated = ctx.client.is_authenticated();

    if !authenticated && matches!(s.focus, PanelFocus::Uploads | PanelFocus::Downloads) {
        s.focus = PanelFocus::Actions;
    }

    let actions = visible_actions(authenticated);
    if s.actions_cursor >= actions.len() {
        s.actions_cursor = actions.len().saturating_sub(1);
    }

    let Event::Key(k) = ev else {
        return ScreenAction::Stay;
    };

    match k.code {
        KeyCode::Char('Q') | KeyCode::Esc => ScreenAction::Quit,

        KeyCode::Tab => {
            cycle_focus_next(s, authenticated);
            ScreenAction::Stay
        }
        KeyCode::Right | KeyCode::Char('l') => {
            match s.focus {
                PanelFocus::Uploads | PanelFocus::Downloads => {
                    if focus_is_reachable(s, PanelFocus::Actions, authenticated) {
                        s.focus = PanelFocus::Actions;
                    }
                }
                PanelFocus::Actions => {
                    if focus_is_reachable(s, PanelFocus::Uploads, authenticated) {
                        s.focus = PanelFocus::Uploads;
                    } else if focus_is_reachable(s, PanelFocus::Downloads, authenticated) {
                        s.focus = PanelFocus::Downloads;
                    }
                }
            }
            ScreenAction::Stay
        }
        KeyCode::Left | KeyCode::Char('h') => {
            match s.focus {
                PanelFocus::Actions => {
                    if focus_is_reachable(s, PanelFocus::Uploads, authenticated) {
                        s.focus = PanelFocus::Uploads;
                    } else if focus_is_reachable(s, PanelFocus::Downloads, authenticated) {
                        s.focus = PanelFocus::Downloads;
                    }
                }
                PanelFocus::Uploads | PanelFocus::Downloads => {
                    if focus_is_reachable(s, PanelFocus::Actions, authenticated) {
                        s.focus = PanelFocus::Actions;
                    }
                }
            }
            ScreenAction::Stay
        }
        KeyCode::BackTab => {
            cycle_focus_prev(s, authenticated);
            ScreenAction::Stay
        }

        KeyCode::Up | KeyCode::Char('k') => {
            match s.focus {
                PanelFocus::Uploads => {
                    if s.selected > 0 {
                        s.selected -= 1;
                    }
                    s.table_state.select(Some(s.selected));
                }
                PanelFocus::Downloads => {
                    if s.downloads_selected == 0
                        && focus_is_reachable(s, PanelFocus::Uploads, authenticated)
                    {
                        s.focus = PanelFocus::Uploads;
                        s.selected = s.items.len().saturating_sub(1);
                        s.table_state.select(Some(s.selected));
                    } else if s.downloads_selected > 0 {
                        s.downloads_selected -= 1;
                        s.downloads_table_state.select(Some(s.downloads_selected));
                    }
                }
                PanelFocus::Actions => {
                    if s.actions_cursor > 0 {
                        s.actions_cursor -= 1;
                    }
                }
            }
            ScreenAction::Stay
        }

        KeyCode::Down | KeyCode::Char('j') => {
            match s.focus {
                PanelFocus::Uploads => {
                    let at_end = s.selected + 1 >= s.items.len();
                    if at_end && focus_is_reachable(s, PanelFocus::Downloads, authenticated) {
                        s.focus = PanelFocus::Downloads;
                        s.downloads_selected = 0;
                        s.downloads_table_state.select(Some(0));
                    } else if s.selected + 1 < s.items.len() {
                        s.selected += 1;
                        s.table_state.select(Some(s.selected));
                    }
                }
                PanelFocus::Downloads => {
                    if s.downloads_selected + 1 < s.downloads.len() {
                        s.downloads_selected += 1;
                    }
                    s.downloads_table_state.select(Some(s.downloads_selected));
                }
                PanelFocus::Actions => {
                    if s.actions_cursor + 1 < actions.len() {
                        s.actions_cursor += 1;
                    }
                }
            }
            ScreenAction::Stay
        }

        KeyCode::Enter => {
            match s.focus {
                PanelFocus::Actions => {
                    if let Some(&a) = actions.get(s.actions_cursor) {
                        execute_action(a, s, ctx)
                    } else {
                        ScreenAction::Stay
                    }
                }
                PanelFocus::Uploads => {
                    if let Some(item) = s.items.get(s.selected) {
                        ScreenAction::PushInfoForCode(item.share_code.clone())
                    } else {
                        ScreenAction::Stay
                    }
                }
                PanelFocus::Downloads => {
                    if let Some(item) = s.downloads.get(s.downloads_selected) {
                        ScreenAction::PushInfoForCode(item.share_code.clone())
                    } else {
                        ScreenAction::Stay
                    }
                }
            }
        }

        KeyCode::Char('u') => execute_action(Action::Upload, s, ctx),
        KeyCode::Char('d') => execute_action(Action::Download, s, ctx),
        KeyCode::Char('s') => execute_action(Action::SecureTransfer, s, ctx),
        KeyCode::Char('i') => execute_action(Action::Info, s, ctx),
        KeyCode::Char('x') => execute_action(Action::Delete, s, ctx),
        KeyCode::Char('X') => start_delete_all(s, ctx),
        KeyCode::Char('r') | KeyCode::Char('R') => refresh_history(s, ctx),
        KeyCode::Char('L') => {
            if authenticated {
                execute_action(Action::Logout, s, ctx)
            } else {
                execute_action(Action::SignIn, s, ctx)
            }
        }

        _ => ScreenAction::Stay,
    }
}

pub fn render(s: &State, f: &mut Frame, client: &ApiClient, toast: Option<&Toast>) {
    let area = f.area();
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    crate::tui::widgets::header::render(f, chunks[0], client);

    let body_cols = Layout::horizontal([
        Constraint::Percentage(70),
        Constraint::Percentage(30),
    ])
    .split(chunks[1]);

    if client.is_authenticated() {
        let left_rows = Layout::vertical([
            Constraint::Percentage(50),
            Constraint::Percentage(50),
        ])
        .split(body_cols[0]);
        render_uploads_panel(s, f, left_rows[0]);
        render_downloads_panel(s, f, left_rows[1]);
        render_actions_panel(s, f, body_cols[1], true);
    } else {
        render_welcome_panel(f, body_cols[0]);
        render_actions_panel(s, f, body_cols[1], false);
    }

    if let Some(t) = toast {
        f.render_widget(
            Paragraph::new(t.msg.clone()).style(t.style()),
            chunks[2],
        );
    } else if let Some(latest) = &s.update_available {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(
                    " \u{26a0} A new version (v{}) is available. Run: npm update -g share-anything-cli",
                    latest
                ),
                Style::default()
                    .fg(Color::Yellow),
            ))),
            chunks[2],
        );
    }
}

fn panel_title(label: &str, focused: bool) -> Span<'static> {
    if focused {
        Span::styled(
            format!(" ▸ {} ", label),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            format!("   {} ", label),
            Style::default().fg(Color::DarkGray),
        )
    }
}

fn render_uploads_panel(s: &State, f: &mut Frame, area: Rect) {
    let focused = s.focus == PanelFocus::Uploads;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(panel_title("My uploads", focused));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if s.loading {
        f.render_widget(Paragraph::new(" Loading…"), inner);
        return;
    }
    if let Some(err) = &s.load_error {
        f.render_widget(
            Paragraph::new(format!(" Failed to load: {}", err))
                .style(Style::default().fg(Color::Red)),
            inner,
        );
        return;
    }

    let total: i64 = s.items.iter().map(|u| u.file_size).sum();
    let stats = format!(
        " {} upload{} · {}",
        s.items.len(),
        if s.items.len() == 1 { "" } else { "s" },
        crate::format::format_size(total),
    );

    let inner_rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
    ])
    .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            stats,
            Style::default().fg(Color::DarkGray),
        ))),
        inner_rows[0],
    );

    if s.items.is_empty() {
        f.render_widget(
            Paragraph::new(" No uploads yet. Press [u] to upload.").alignment(Alignment::Left),
            inner_rows[1],
        );
        return;
    }

    let rows: Vec<Row> = s
        .items
        .iter()
        .map(|it| {
            Row::new(vec![
                Cell::from(format!("  {}", it.share_code)),
                Cell::from(it.file_name.clone()),
                Cell::from(crate::format::format_size(it.file_size)),
                Cell::from(crate::time::utc_to_local(&it.expires_at)),
                Cell::from(""),
            ])
        })
        .collect();

    let header_row = Row::new(vec!["  CODE", "FILE", "SIZE", "EXPIRES", ""])
        .style(Style::default().add_modifier(Modifier::BOLD));

    let widths = [
        Constraint::Length(10),
        Constraint::Min(20),
        Constraint::Length(10),
        Constraint::Length(22),
        Constraint::Length(1),
    ];
    let table = Table::new(rows, widths)
        .header(header_row)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = TableState::default();
    if focused {
        state.select(Some(s.selected));
    }
    f.render_stateful_widget(table, inner_rows[1], &mut state);
}

fn render_downloads_panel(s: &State, f: &mut Frame, area: Rect) {
    let focused = s.focus == PanelFocus::Downloads;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(panel_title("My downloads", focused));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if s.downloads_loading {
        f.render_widget(Paragraph::new(" Loading…"), inner);
        return;
    }
    if let Some(err) = &s.downloads_load_error {
        f.render_widget(
            Paragraph::new(format!(" Failed to load: {}", err))
                .style(Style::default().fg(Color::Red)),
            inner,
        );
        return;
    }

    let total: i64 = s.downloads.iter().map(|d| d.file_size).sum();
    let stats = format!(
        " {} download{} · {}",
        s.downloads.len(),
        if s.downloads.len() == 1 { "" } else { "s" },
        crate::format::format_size(total),
    );

    let inner_rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
    ])
    .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            stats,
            Style::default().fg(Color::DarkGray),
        ))),
        inner_rows[0],
    );

    if s.downloads.is_empty() {
        f.render_widget(
            Paragraph::new(" No downloads yet.").alignment(Alignment::Left),
            inner_rows[1],
        );
        return;
    }

    let rows: Vec<Row> = s
        .downloads
        .iter()
        .map(|it| {
            Row::new(vec![
                Cell::from(format!("  {}", it.share_code)),
                Cell::from(it.file_name.clone()),
                Cell::from(crate::format::format_size(it.file_size)),
                Cell::from(crate::time::utc_to_local(&it.downloaded_at)),
                Cell::from(""),
            ])
        })
        .collect();

    let header_row = Row::new(vec!["  CODE", "FILE", "SIZE", "DOWNLOADED", ""])
        .style(Style::default().add_modifier(Modifier::BOLD));

    let widths = [
        Constraint::Length(10),
        Constraint::Min(20),
        Constraint::Length(10),
        Constraint::Length(22),
        Constraint::Length(1),
    ];
    let table = Table::new(rows, widths)
        .header(header_row)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = TableState::default();
    if focused {
        state.select(Some(s.downloads_selected));
    }
    f.render_stateful_widget(table, inner_rows[1], &mut state);
}

fn render_actions_panel(s: &State, f: &mut Frame, area: Rect, authenticated: bool) {
    let focused = s.focus == PanelFocus::Actions;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(panel_title("Actions", focused));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let actions = visible_actions(authenticated);
    let lines: Vec<Line> = actions
        .iter()
        .enumerate()
        .map(|(i, &a)| {
            let mut line = action_line(action_key(a), action_label(a));

            if matches!(a, Action::Delete | Action::DeleteAll) && s.items.is_empty() {
                line = Line::from(
                    line.spans
                        .into_iter()
                        .map(|sp| {
                            Span::styled(sp.content.into_owned(), Style::default().fg(Color::DarkGray))
                        })
                        .collect::<Vec<_>>(),
                );
            }

            if focused && i == s.actions_cursor {
                line = Line::from(
                    line.spans
                        .into_iter()
                        .map(|sp| {
                            let style = sp.style.add_modifier(Modifier::REVERSED);
                            Span::styled(sp.content.into_owned(), style)
                        })
                        .collect::<Vec<_>>(),
                );
            }

            line
        })
        .collect();

    f.render_widget(Paragraph::new(lines), inner);
}

fn action_line(key: &str, label: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw(" ["),
        Span::styled(
            key.to_string(),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::raw("] "),
        Span::raw(label.to_string()),
    ])
}

fn render_welcome_panel(f: &mut Frame, area: Rect) {
    let card_width = area.width.saturating_sub(6).min(50);
    let card_height = 12u16;
    let x = area.x + (area.width.saturating_sub(card_width)) / 2;
    let y = area.y + (area.height.saturating_sub(card_height)) / 2;
    let card_rect = Rect::new(x, y, card_width, card_height.min(area.height));

    let block = Block::default().borders(Borders::ALL).title(" Welcome ");
    let inner = block.inner(card_rect);
    f.render_widget(block, card_rect);

    let lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  ShareAnything CLI",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  Fast file sharing from"),
        Line::from("  the terminal."),
        Line::from(""),
        Line::from(vec![
            Span::raw("  ["),
            Span::styled("L", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("] Sign in to manage"),
        ]),
        Line::from("      your shares"),
        Line::from(""),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}
