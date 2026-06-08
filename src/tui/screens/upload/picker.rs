use crate::tui::app::{AppCtx, ScreenAction};
use crate::tui::event::Event;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerMode {
    Upload,
    Secure,
}

pub struct State {
    pub cwd: PathBuf,
    pub entries: Vec<Entry>,
    pub cursor: usize,
    pub selected: BTreeMap<PathBuf, u64>,
    pub list_state: ListState,
    pub anchor: Option<usize>,
    pub mode: PickerMode,
    pub last_entries_cursor: usize,
}

#[derive(Debug)]
enum Row {
    Entry { idx: usize },
    Separator,
    Selected { path: PathBuf },
}

fn build_rows(s: &State) -> Vec<Row> {
    let mut rows: Vec<Row> = s.entries.iter().enumerate().map(|(i, _)| Row::Entry { idx: i }).collect();
    if !s.selected.is_empty() {
        rows.push(Row::Separator);
        for path in s.selected.keys() {
            rows.push(Row::Selected { path: path.clone() });
        }
    }
    rows
}

#[cfg(test)]
fn is_interactive(rows: &[Row], idx: usize) -> bool {
    !matches!(rows.get(idx), Some(Row::Separator) | None)
}

impl State {
    pub fn new() -> std::io::Result<Self> {
        let cwd = std::env::current_dir()?.canonicalize()?;
        let entries = read_dir(&cwd)?;
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Ok(Self {
            cwd,
            entries,
            cursor: 0,
            selected: BTreeMap::new(),
            list_state,
            anchor: None,
            mode: PickerMode::Upload,
            last_entries_cursor: 0,
        })
    }

    pub fn new_secure() -> std::io::Result<Self> {
        let cwd = std::env::current_dir()?.canonicalize()?;
        let entries = read_dir(&cwd)?;
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Ok(Self {
            cwd,
            entries,
            cursor: 0,
            selected: BTreeMap::new(),
            list_state,
            anchor: None,
            mode: PickerMode::Secure,
            last_entries_cursor: 0,
        })
    }

    fn navigate(&mut self, p: &Path) -> std::io::Result<()> {
        let new_cwd = p.canonicalize()?;
        let entries = read_dir(&new_cwd)?;
        self.cwd = new_cwd;
        self.entries = entries;
        self.cursor = 0;
        self.list_state.select(Some(0));
        self.anchor = None;
        Ok(())
    }

    fn toggle_entry(&mut self, entry_idx: usize) {
        if let Some(e) = self.entries.get(entry_idx) {
            if e.is_dir { return; }
            if self.selected.remove(&e.path).is_none() {
                self.selected.insert(e.path.clone(), e.size);
            }
            self.anchor = Some(self.cursor);
        }
    }

    fn deselect_path(&mut self, path: &Path) {
        self.selected.remove(path);
    }

    fn range_select(&mut self) {
        let Some(anchor) = self.anchor else {
            let rows = build_rows(self);
            if let Some(Row::Entry { idx }) = rows.get(self.cursor).map(|r| {
                match r {
                    Row::Entry { idx } => Row::Entry { idx: *idx },
                    _ => Row::Separator,
                }
            }) {
                self.toggle_entry(idx);
            }
            return;
        };
        let entries_len = self.entries.len();
        let effective_anchor = if anchor < entries_len { anchor } else {
            let rows = build_rows(self);
            if let Some(Row::Entry { idx }) = rows.get(self.cursor).map(|r| match r {
                Row::Entry { idx } => Row::Entry { idx: *idx },
                _ => Row::Separator,
            }) {
                self.toggle_entry(idx);
            }
            return;
        };
        let rows = build_rows(self);
        let cursor_entry_idx = match rows.get(self.cursor) {
            Some(Row::Entry { idx }) => *idx,
            _ => {
                if let Some(Row::Entry { idx }) = rows.get(self.cursor).map(|r| match r {
                    Row::Entry { idx } => Row::Entry { idx: *idx },
                    _ => Row::Separator,
                }) {
                    self.toggle_entry(idx);
                }
                return;
            }
        };
        let (lo, hi) = if effective_anchor <= cursor_entry_idx {
            (effective_anchor, cursor_entry_idx)
        } else {
            (cursor_entry_idx, effective_anchor)
        };
        for i in lo..=hi {
            if let Some(e) = self.entries.get(i) {
                if !e.is_dir {
                    self.selected.insert(e.path.clone(), e.size);
                }
            }
        }
        self.anchor = Some(self.cursor);
    }

    fn clamp_cursor_after_selection_change(&mut self) {
        let rows = build_rows(self);
        let max = rows.len().saturating_sub(1);
        if self.cursor > max { self.cursor = max; }
        if matches!(rows.get(self.cursor), Some(Row::Separator)) && self.cursor > 0 {
            self.cursor -= 1;
        }
        self.list_state.select(Some(self.cursor));
    }
}

fn read_dir(p: &Path) -> std::io::Result<Vec<Entry>> {
    let mut out: Vec<Entry> = Vec::new();
    for it in std::fs::read_dir(p)? {
        let it = it?;
        let Ok(md) = it.metadata() else { continue };
        let name = it.file_name().to_string_lossy().to_string();
        if name.starts_with('.') { continue; }
        out.push(Entry {
            name,
            path: it.path(),
            is_dir: md.is_dir(),
            size: md.len(),
        });
    }
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    Ok(out)
}

fn cursor_absolute_path(s: &State, rows: &[Row]) -> String {
    match rows.get(s.cursor) {
        Some(Row::Entry { idx }) => s.entries[*idx].path.display().to_string(),
        Some(Row::Selected { path }) => path.display().to_string(),
        _ => String::new(),
    }
}

fn trigger_range_select(s: &mut State, ctx: &mut AppCtx, rows: &[Row]) {
    match rows.get(s.cursor) {
        Some(Row::Entry { idx }) => {
            let entry = &s.entries[*idx];
            if entry.is_dir {
                *ctx.toast = Some(crate::tui::widgets::toast::Toast::info(
                    "[r] selects a range of files. Move to a file first.",
                ));
            } else {
                s.range_select();
                s.clamp_cursor_after_selection_change();
            }
        }
        Some(Row::Selected { .. }) => {
            *ctx.toast = Some(crate::tui::widgets::toast::Toast::info(
                "[r] range selection only works on the file list, not Selected.",
            ));
        }
        _ => {}
    }
}

pub fn update(s: &mut State, ev: &Event, ctx: &mut AppCtx) -> ScreenAction {
    let Event::Key(k) = ev else { return ScreenAction::Stay; };

    let rows = build_rows(s);

    if matches!(k.code, KeyCode::Enter) && k.modifiers.contains(KeyModifiers::SHIFT) {
        trigger_range_select(s, ctx, &rows);
        return ScreenAction::Stay;
    }

    match k.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('b') => ScreenAction::Pop,

        KeyCode::Tab => {
            let entries_len = s.entries.len();
            let in_entries = s.cursor < entries_len;
            let in_selected = s.cursor > entries_len && !s.selected.is_empty();
            if in_entries && !s.selected.is_empty() {
                s.last_entries_cursor = s.cursor;
                s.cursor = entries_len + 1;
                s.list_state.select(Some(s.cursor));
            } else if in_selected && entries_len > 0 {
                s.cursor = s.last_entries_cursor.min(entries_len - 1);
                s.list_state.select(Some(s.cursor));
            }
            ScreenAction::Stay
        }

        KeyCode::Char('r') => {
            trigger_range_select(s, ctx, &rows);
            ScreenAction::Stay
        }

        KeyCode::Up | KeyCode::Char('k') => {
            if s.cursor > 0 {
                s.cursor -= 1;
                if matches!(build_rows(s).get(s.cursor), Some(Row::Separator)) && s.cursor > 0 {
                    s.cursor -= 1;
                }
            }
            s.list_state.select(Some(s.cursor));
            ScreenAction::Stay
        }

        KeyCode::Down | KeyCode::Char('j') => {
            let max = rows.len().saturating_sub(1);
            if s.cursor < max {
                s.cursor += 1;
                if matches!(build_rows(s).get(s.cursor), Some(Row::Separator)) {
                    let new_max = build_rows(s).len().saturating_sub(1);
                    if s.cursor < new_max { s.cursor += 1; }
                }
            }
            s.list_state.select(Some(s.cursor));
            ScreenAction::Stay
        }

        KeyCode::Left | KeyCode::Char('h') => {
            if let Some(parent) = s.cwd.parent().map(|p| p.to_path_buf()) {
                if let Err(err) = s.navigate(&parent) {
                    *ctx.toast = Some(crate::tui::widgets::toast::Toast::error(
                        format!("Cannot navigate to parent: {}", err),
                    ));
                }
            }
            ScreenAction::Stay
        }

        KeyCode::Right | KeyCode::Char('l') => {
            if let Some(Row::Entry { idx }) = rows.get(s.cursor) {
                let e = &s.entries[*idx];
                if e.is_dir {
                    let path = e.path.clone();
                    if let Err(err) = s.navigate(&path) {
                        *ctx.toast = Some(crate::tui::widgets::toast::Toast::error(
                            format!("Cannot open directory: {}", err),
                        ));
                    }
                } else {
                    *ctx.toast = Some(crate::tui::widgets::toast::Toast::info(
                        "Right arrow opens a folder. Use Space/Enter to toggle the file.",
                    ));
                }
            }
            ScreenAction::Stay
        }

        KeyCode::Char(' ') => {
            match rows.get(s.cursor) {
                Some(Row::Entry { idx }) => {
                    let idx = *idx;
                    s.toggle_entry(idx);
                    s.clamp_cursor_after_selection_change();
                }
                Some(Row::Selected { path, .. }) => {
                    let path = path.clone();
                    s.deselect_path(&path);
                    s.clamp_cursor_after_selection_change();
                }
                _ => {}
            }
            ScreenAction::Stay
        }

        KeyCode::Enter => {
            match rows.get(s.cursor) {
                Some(Row::Entry { idx }) => {
                    let idx = *idx;
                    let e = &s.entries[idx];
                    if e.is_dir {
                        let path = e.path.clone();
                        if let Err(err) = s.navigate(&path) {
                            *ctx.toast = Some(crate::tui::widgets::toast::Toast::error(
                                format!("Cannot open directory: {}", err),
                            ));
                        }
                    } else {
                        s.toggle_entry(idx);
                        s.clamp_cursor_after_selection_change();
                    }
                }
                Some(Row::Selected { path, .. }) => {
                    let path = path.clone();
                    s.deselect_path(&path);
                    s.clamp_cursor_after_selection_change();
                }
                _ => {}
            }
            ScreenAction::Stay
        }

        KeyCode::Char('u') => {
            let paths: Vec<PathBuf> = s.selected.keys().cloned().collect();
            if paths.is_empty() {
                *ctx.toast = Some(crate::tui::widgets::toast::Toast::warn(
                    "Select at least one file with space first.",
                ));
                return ScreenAction::Stay;
            }
            let authenticated = ctx.client.is_authenticated();
            match s.mode {
                PickerMode::Upload => {
                    let st = crate::tui::screens::upload::options::State::new(paths, authenticated);
                    ScreenAction::Push(crate::tui::app::Screen::Upload(
                        crate::tui::screens::upload::Screen::Options(st),
                    ))
                }
                PickerMode::Secure => {
                    let st = crate::tui::screens::secure::options::State::new(paths, authenticated);
                    ScreenAction::Push(crate::tui::app::Screen::Secure(
                        crate::tui::screens::secure::Screen::Options(st),
                    ))
                }
            }
        }

        _ => ScreenAction::Stay,
    }
}

fn render_entry_line(e: &Entry, selected: bool, cursor: bool) -> Line<'static> {
    let size_str = if e.is_dir {
        String::new()
    } else {
        format!("  {}", crate::format::format_size_u64(e.size))
    };
    let prefix = if cursor { Span::raw("▸ ") } else { Span::raw("  ") };
    let marker = if e.is_dir {
        Span::raw("📁 ".to_string())
    } else if selected {
        Span::raw("[✓] ".to_string())
    } else {
        Span::raw("[ ] ".to_string())
    };
    Line::from(vec![
        prefix,
        marker,
        Span::raw(e.name.clone()),
        Span::styled(size_str, Style::default().fg(Color::DarkGray)),
    ])
}

fn render_selected_line(path: &Path, size: u64, cursor: bool) -> Line<'static> {
    let display_path = path.to_string_lossy().to_string();
    let prefix = if cursor { Span::raw("▸ ") } else { Span::raw("  ") };
    Line::from(vec![
        prefix,
        Span::raw("[✓] ".to_string()),
        Span::raw(display_path),
        Span::styled(
            format!("  {}", crate::format::format_size_u64(size)),
            Style::default().fg(Color::DarkGray),
        ),
    ])
}

pub fn render(s: &State, f: &mut Frame) {
    let area = f.area();
    let outer = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .split(area);

    let title = match s.mode {
        PickerMode::Upload => "Upload: choose files",
        PickerMode::Secure => "Secure transfer: choose files",
    };
    let header_block = Block::default().borders(Borders::BOTTOM).title(format!(" {} ", title));
    f.render_widget(Paragraph::new("").block(header_block), outer[0]);

    let rows = build_rows(s);
    let cursor_path = cursor_absolute_path(s, &rows);

    let selected_count = s.selected.len() as u16;
    let body_area = outer[1];
    let selected_box_h: u16 = if selected_count > 0 {
        (selected_count + 2).min(body_area.height / 2).max(3)
    } else {
        0
    };
    let body_chunks = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(selected_box_h),
    ])
    .split(body_area);

    let files_block = Block::default()
        .borders(Borders::ALL)
        .title(" Files ");
    let files_inner = files_block.inner(body_chunks[0]);
    f.render_widget(files_block, body_chunks[0]);

    let cursor_bar_h: u16 = if cursor_path.is_empty() { 0 } else { 2 };
    let files_inner_chunks = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(cursor_bar_h),
    ])
    .split(files_inner);

    if s.entries.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  (empty folder)",
                Style::default().fg(Color::DarkGray),
            ))),
            files_inner_chunks[0],
        );
    } else {
        let entry_items: Vec<ListItem> = s.entries.iter().map(|e| {
            ListItem::new(render_entry_line(e, s.selected.contains_key(&e.path), false))
        }).collect();
        let files_list = List::new(entry_items)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        let mut entry_state = ListState::default();
        if s.cursor < s.entries.len() {
            entry_state.select(Some(s.cursor));
        }
        f.render_stateful_widget(files_list, files_inner_chunks[0], &mut entry_state);
    }

    if cursor_bar_h > 0 {
        let sep_w = files_inner.width as usize;
        let cursor_bar = Paragraph::new(vec![
            Line::from(Span::styled(
                "\u{2500}".repeat(sep_w),
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(vec![
                Span::styled(
                    " \u{25b8} ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    cursor_path,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
        ]);
        f.render_widget(cursor_bar, files_inner_chunks[1]);
    }

    if selected_count > 0 {
        let sel_items: Vec<ListItem> = s.selected.iter().map(|(path, &size)| {
            ListItem::new(render_selected_line(path, size, false))
        }).collect();
        let sel_title = format!(" Selected files ({}) ", selected_count);
        let sel_block = Block::default()
            .borders(Borders::ALL)
            .title(sel_title);
        let sel_list = List::new(sel_items)
            .block(sel_block)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        let mut sel_state = ListState::default();
        if s.cursor > s.entries.len() {
            let sel_idx = s.cursor - s.entries.len() - 1;
            sel_state.select(Some(sel_idx));
        }
        f.render_stateful_widget(sel_list, body_chunks[1], &mut sel_state);
    }

    let total: u64 = s.selected.values().sum();
    let summary = format!(
        " Selected: {} files, {} ",
        s.selected.len(),
        crate::format::format_size_u64(total)
    );
    let primary_action = match s.mode {
        PickerMode::Upload => "upload",
        PickerMode::Secure => "confirm",
    };
    let secondary_hints = "[↑↓]move  [tab]switch  [space/enter]toggle  [enter]open  [r]range  [←/→]dir  [q/b]cancel ";
    let white_bold = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    let cyan_bold = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let hint_line = Line::from(vec![
        Span::raw(" "),
        Span::styled("[", white_bold),
        Span::styled("u", cyan_bold),
        Span::styled("]", white_bold),
        Span::styled(primary_action, white_bold),
        Span::raw("   "),
        Span::styled(secondary_hints, Style::default().fg(Color::DarkGray)),
    ]);
    let body = Paragraph::new(vec![
        Line::from(Span::styled(summary, Style::default().add_modifier(Modifier::BOLD))),
        hint_line,
    ]);
    f.render_widget(body, outer[2]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("picker_test_{}", std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()));
            fs::create_dir_all(&path).unwrap();
            TmpDir(path)
        }
        fn path(&self) -> &Path { &self.0 }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    fn make_state_with_files() -> (State, TmpDir) {
        let dir = TmpDir::new();
        let a = dir.path().join("alpha.txt");
        let b = dir.path().join("beta.txt");
        fs::write(&a, "hello").unwrap();
        fs::write(&b, "world!").unwrap();
        let mut s = State::new().unwrap();
        let entries = read_dir(dir.path()).unwrap();
        s.cwd = dir.path().to_path_buf();
        s.entries = entries;
        s.cursor = 0;
        s.selected = BTreeMap::new();
        s.list_state.select(Some(0));
        (s, dir)
    }

    #[test]
    fn selected_section_appended_after_entries() {
        let (mut s, _dir) = make_state_with_files();
        let file_idx = s.entries.iter().position(|e| !e.is_dir).unwrap();
        s.toggle_entry(file_idx);
        let rows = build_rows(&s);
        let has_separator = rows.iter().any(|r| matches!(r, Row::Separator));
        let has_selected = rows.iter().any(|r| matches!(r, Row::Selected { .. }));
        assert!(has_separator, "expected Separator row");
        assert!(has_selected, "expected Selected row");
        let sep_pos = rows.iter().position(|r| matches!(r, Row::Separator)).unwrap();
        for i in 0..sep_pos {
            assert!(matches!(rows[i], Row::Entry { .. }), "row {} before separator must be Entry", i);
        }
    }

    #[test]
    fn no_separator_when_nothing_selected() {
        let (s, _dir) = make_state_with_files();
        let rows = build_rows(&s);
        assert!(rows.iter().all(|r| !matches!(r, Row::Separator)));
    }

    #[test]
    fn deselect_from_selected_row_removes_entry() {
        let (mut s, _dir) = make_state_with_files();
        let file_idx = s.entries.iter().position(|e| !e.is_dir).unwrap();
        s.toggle_entry(file_idx);
        assert_eq!(s.selected.len(), 1);
        let path = s.entries[file_idx].path.clone();
        s.deselect_path(&path);
        assert!(s.selected.is_empty());
    }

    #[test]
    fn cursor_skips_separator_going_down() {
        let (mut s, _dir) = make_state_with_files();
        let file_idx = s.entries.iter().position(|e| !e.is_dir).unwrap();
        s.toggle_entry(file_idx);
        let rows = build_rows(&s);
        let sep_pos = rows.iter().position(|r| matches!(r, Row::Separator)).unwrap();
        s.cursor = sep_pos - 1;
        let max = rows.len().saturating_sub(1);
        if s.cursor < max {
            s.cursor += 1;
            if matches!(build_rows(&s).get(s.cursor), Some(Row::Separator)) {
                let new_max = build_rows(&s).len().saturating_sub(1);
                if s.cursor < new_max { s.cursor += 1; }
            }
        }
        let final_rows = build_rows(&s);
        assert!(!matches!(final_rows.get(s.cursor), Some(Row::Separator)));
    }

    #[test]
    fn cursor_skips_separator_going_up() {
        let (mut s, _dir) = make_state_with_files();
        let file_idx = s.entries.iter().position(|e| !e.is_dir).unwrap();
        s.toggle_entry(file_idx);
        let rows = build_rows(&s);
        let sep_pos = rows.iter().position(|r| matches!(r, Row::Separator)).unwrap();
        s.cursor = sep_pos + 1;
        if s.cursor > 0 {
            s.cursor -= 1;
            if matches!(build_rows(&s).get(s.cursor), Some(Row::Separator)) {
                if s.cursor > 0 { s.cursor -= 1; }
            }
        }
        let final_rows = build_rows(&s);
        assert!(!matches!(final_rows.get(s.cursor), Some(Row::Separator)));
    }

    #[test]
    fn upload_paths_in_btreemap_order() {
        let (mut s, _dir) = make_state_with_files();
        for e in &s.entries {
            if !e.is_dir {
                s.selected.insert(e.path.clone(), e.size);
            }
        }
        let paths: Vec<PathBuf> = s.selected.keys().cloned().collect();
        let mut expected = paths.clone();
        expected.sort();
        assert_eq!(paths, expected);
    }

    #[test]
    fn is_interactive_returns_false_for_separator() {
        let rows = vec![
            Row::Entry { idx: 0 },
            Row::Separator,
            Row::Selected { path: PathBuf::from("/foo") },
        ];
        assert!(is_interactive(&rows, 0));
        assert!(!is_interactive(&rows, 1));
        assert!(is_interactive(&rows, 2));
        assert!(!is_interactive(&rows, 99));
    }
}
