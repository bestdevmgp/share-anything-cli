use crate::client::ApiClient;
use crate::config::CliConfig;
use crate::tui::event::{Event, Tx};
use crate::tui::screens;
use crate::tui::widgets::toast::Toast;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use tokio::task::AbortHandle;

#[allow(clippy::large_enum_variant)]
pub enum Screen {
    Dashboard(screens::dashboard::State),
    Upload(screens::upload::Screen),
    Download(screens::download::State),
    Info(screens::info::State),
    Delete(screens::delete::State),
    Login(screens::login::State),
    Logout(screens::logout::State),
    Secure(screens::secure::Screen),
}

pub enum ScreenAction {
    Stay,
    Quit,
    Pop,
    /// Pop every screen down to (but not including) the dashboard at the bottom of the stack.
    PopToRoot,
    Push(Screen),
    #[allow(dead_code)]
    Replace(Screen),
    LogOut,
    PushLogin,
    PushDownloadForCode(String),
    PushInfoForCode(String),
}

pub struct App {
    pub cfg: CliConfig,
    pub client: ApiClient,
    pub stack: Vec<Screen>,
    /// Per-screen task handles, kept in lockstep with `stack`. Popping a screen aborts its tasks.
    pub screen_tasks: Vec<Vec<AbortHandle>>,
    pub toast: Option<Toast>,
    pub tx: Option<Tx>,
    pub stdout_lines: Vec<String>,
    last_root_ctrl_c: Option<std::time::Instant>,
    quit: bool,
}

const ROOT_QUIT_DOUBLE_TAP: std::time::Duration = std::time::Duration::from_secs(2);

impl App {
    pub fn new(cfg: CliConfig) -> anyhow::Result<Self> {
        let client = ApiClient::new(&cfg)?;
        Ok(Self {
            cfg,
            client,
            stack: vec![Screen::Dashboard(screens::dashboard::State::new())],
            screen_tasks: vec![Vec::new()],
            toast: None,
            tx: None,
            stdout_lines: Vec::new(),
            last_root_ctrl_c: None,
            quit: false,
        })
    }

    pub fn set_tx(&mut self, tx: Tx) { self.tx = Some(tx); }
    pub fn should_quit(&self) -> bool { self.quit }
    pub fn drain_stdout(&mut self) -> Vec<String> { std::mem::take(&mut self.stdout_lines) }

    pub fn on_enter(&mut self) {
        if self.client.is_authenticated() {
            if let Some(tx) = self.tx.clone() {
                if let Some(Screen::Dashboard(s)) = self.stack.last_mut() {
                    s.loading = true;
                    s.downloads_loading = true;
                }
                self.spawn_dashboard_fetches(tx);
            }
        }
    }

    fn spawn_dashboard_fetches(&mut self, tx: Tx) {
        let client_u = self.client.clone();
        let tx_u = tx.clone();
        let h_u = tokio::spawn(async move {
            let r = crate::core::shares::list_my_uploads(&client_u).await;
            let _ = tx_u.send(Event::UploadsLoaded(r));
        });
        let client_d = self.client.clone();
        let h_d = tokio::spawn(async move {
            let r = crate::core::shares::list_my_downloads(&client_d).await;
            let _ = tx.send(Event::DownloadsLoaded(r));
        });
        if let Some(tasks) = self.screen_tasks.last_mut() {
            tasks.push(h_u.abort_handle());
            tasks.push(h_d.abort_handle());
        }
    }

    pub fn update(&mut self, ev: Event) {
        if matches!(&ev, Event::Tick) {
            if let Some(t) = &self.toast {
                if t.expired() { self.toast = None; }
            }
        }

        // At the root, require a second Ctrl+C within ROOT_QUIT_DOUBLE_TAP to actually quit —
        // a stray Ctrl+C shouldn't kill the session.
        if let Event::Key(k) = &ev {
            if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
                if self.stack.len() <= 1 {
                    let now = std::time::Instant::now();
                    let confirmed = self
                        .last_root_ctrl_c
                        .map(|t| now.duration_since(t) <= ROOT_QUIT_DOUBLE_TAP)
                        .unwrap_or(false);
                    if confirmed {
                        self.quit = true;
                    } else {
                        self.last_root_ctrl_c = Some(now);
                        self.toast = Some(Toast::warn(
                            "Press Ctrl+C again to quit (or [Q] to quit immediately).",
                        ));
                    }
                } else {
                    self.last_root_ctrl_c = None;
                    self.apply_action(ScreenAction::PopToRoot);
                }
                return;
            }
        }

        match ev {
            Event::UploadsLoaded(result) => {
                if let Some(Screen::Dashboard(s)) = self.stack.last_mut() {
                    s.loading = false;
                    match result {
                        Ok(items) => {
                            s.items = items;
                            s.load_error = None;
                            if s.selected >= s.items.len() {
                                s.selected = s.items.len().saturating_sub(1);
                            }
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            s.load_error = Some(msg.clone());
                            self.toast = Some(Toast::error(format!("Failed to load uploads: {}", msg)));
                        }
                    }
                }
            }
            Event::DownloadsLoaded(result) => {
                if let Some(Screen::Dashboard(s)) = self.stack.last_mut() {
                    s.downloads_loading = false;
                    match result {
                        Ok(items) => {
                            s.downloads = items;
                            s.downloads_load_error = None;
                            if s.downloads_selected >= s.downloads.len() {
                                s.downloads_selected = s.downloads.len().saturating_sub(1);
                            }
                        }
                        Err(e) => {
                            // No toast — UploadsLoaded already raises one for the shared
                            // auth/network failure modes; stacking a second toast adds noise.
                            s.downloads_load_error = Some(e.to_string());
                        }
                    }
                }
            }
            Event::UploadProgress { delta } => {
                if let Some(Screen::Upload(crate::tui::screens::upload::Screen::Options(s))) = self.stack.last_mut() {
                    if let crate::tui::screens::upload::options::Phase::Running { sent, total, .. } = &mut s.phase {
                        *sent = sent.saturating_add(delta).min(*total);
                    }
                }
            }
            Event::UploadFinished(result) => {
                if let Some(Screen::Upload(crate::tui::screens::upload::Screen::Options(s))) = self.stack.last_mut() {
                    s.phase = match result {
                        Ok(r) => crate::tui::screens::upload::options::Phase::Done { result: r, copied: false },
                        Err(e) => crate::tui::screens::upload::options::Phase::Failed(e.to_string()),
                    };
                }
            }
            Event::DownloadInfo(result) => {
                if let Some(Screen::Download(s)) = self.stack.last_mut() {
                    match result {
                        Ok(info) => {
                            if info.has_password {
                                let mut password = tui_textarea::TextArea::default();
                                password.set_placeholder_text("Password");
                                password.set_mask_char('•');
                                password.set_block(
                                    ratatui::widgets::Block::default()
                                        .borders(ratatui::widgets::Borders::ALL)
                                        .title(" Password "),
                                );
                                s.phase = crate::tui::screens::download::Phase::NeedsPassword { info, password };
                            } else {
                                s.phase = crate::tui::screens::download::Phase::ChoosePath {
                                    info,
                                    password: None,
                                    picker: crate::tui::screens::download::PathPicker::new(),
                                };
                            }
                        }
                        Err(e) => {
                            s.phase = crate::tui::screens::download::Phase::Failed(e.to_string());
                        }
                    }
                }
            }
            Event::DownloadProgress { delta } => {
                if let Some(Screen::Download(s)) = self.stack.last_mut() {
                    if let crate::tui::screens::download::Phase::Running { received, total, .. } = &mut s.phase {
                        *received = received.saturating_add(delta).min(*total);
                    }
                }
            }
            Event::DownloadFinished(result) => {
                if let Some(Screen::Download(s)) = self.stack.last_mut() {
                    s.phase = match result {
                        Ok(saved) => crate::tui::screens::download::Phase::Done { saved },
                        Err(e) => crate::tui::screens::download::Phase::Failed(e.to_string()),
                    };
                }
            }
            Event::InfoLoaded(result) => {
                if let Some(Screen::Info(s)) = self.stack.last_mut() {
                    s.phase = match result {
                        Ok(info) => crate::tui::screens::info::Phase::Loaded(info),
                        Err(e) => crate::tui::screens::info::Phase::Failed(e.to_string()),
                    };
                }
            }
            Event::DeleteFinished(result) => {
                match result {
                    Ok(code) => {
                        for screen in self.stack.iter_mut() {
                            if let Screen::Dashboard(d) = screen {
                                d.items.retain(|u| u.share_code != code);
                                if d.selected >= d.items.len() {
                                    d.selected = d.items.len().saturating_sub(1);
                                }
                                break;
                            }
                        }
                        self.toast = Some(crate::tui::widgets::toast::Toast::success(
                            format!("Deleted share {}", code),
                        ));
                        self.abort_top_screen_tasks();
                        self.stack.pop();
                        if self.stack.is_empty() { self.quit = true; }
                    }
                    Err(e) => {
                        if let Some(Screen::Delete(s)) = self.stack.last_mut() {
                            s.phase = crate::tui::screens::delete::Phase::Failed(e.to_string());
                        } else {
                            self.toast = Some(Toast::error(format!("Delete failed: {}", e)));
                        }
                    }
                }
            }
            Event::LoginSessionReady(result) => {
                if let Some(Screen::Login(s)) = self.stack.last_mut() {
                    match result {
                        Ok(session) => {
                            let qr_lines = crate::tui::screens::login::build_qr_lines(&session.login_url);
                            match open::that(&session.login_url) {
                                Ok(_) => {
                                    self.toast = Some(Toast::success("Browser opened. Complete the sign-in there."));
                                }
                                Err(_) => {
                                    self.toast = Some(Toast::warn("Could not open the browser. Use the URL or QR below."));
                                }
                            }
                            s.phase = crate::tui::screens::login::Phase::WaitingDevice {
                                qr_lines,
                                last_poll: std::time::Instant::now(),
                                poll_inflight: false,
                                started_at: std::time::Instant::now(),
                                session,
                            };
                        }
                        Err(e) => {
                            s.phase = crate::tui::screens::login::Phase::Failed(e.to_string());
                        }
                    }
                }
            }
            Event::LoginPolled(result) => {
                if let Some(Screen::Login(s)) = self.stack.last_mut() {
                    if let crate::tui::screens::login::Phase::WaitingDevice { poll_inflight, .. } = &mut s.phase {
                        *poll_inflight = false;
                    }
                    match result {
                        Ok(status) => match status.status.as_str() {
                            "completed" => {
                                if let Some(token) = status.personal_token {
                                    self.cfg.token = Some(token);
                                    self.cfg.user_name = status.user_name.clone();
                                    if let Err(e) = self.cfg.save() {
                                        if let Some(Screen::Login(s)) = self.stack.last_mut() {
                                            s.phase = crate::tui::screens::login::Phase::Failed(
                                                format!("Failed to save config: {}", e),
                                            );
                                        }
                                        return;
                                    }
                                    match crate::client::ApiClient::new(&self.cfg) {
                                        Ok(c) => self.client = c,
                                        Err(e) => {
                                            if let Some(Screen::Login(s)) = self.stack.last_mut() {
                                                s.phase = crate::tui::screens::login::Phase::Failed(
                                                    format!("Internal: {}", e),
                                                );
                                            }
                                            return;
                                        }
                                    }
                                    let name = status.user_name.unwrap_or_else(|| "User".to_string());
                                    self.abort_top_screen_tasks();
                                    self.stack.pop();
                                    self.toast = Some(Toast::success(format!("Welcome, {}!", name)));
                                    self.refresh_dashboard_if_top();
                                } else if let Some(Screen::Login(s)) = self.stack.last_mut() {
                                    s.phase = crate::tui::screens::login::Phase::Failed(
                                        "Server did not return a token.".into(),
                                    );
                                }
                            }
                            "expired" => {
                                if let Some(Screen::Login(s)) = self.stack.last_mut() {
                                    s.phase = crate::tui::screens::login::Phase::Failed(
                                        "Session expired. Try again.".into(),
                                    );
                                }
                            }
                            _ => {}
                        },
                        Err(_) => {
                            // Network blip: silently keep polling.
                        }
                    }
                }
            }
            Event::P2PSend(ev) => {
                use crate::core::p2p::sender::SenderEvent as E;
                use crate::tui::screens::secure::options::{FileState, FileStatus};
                if let Some(Screen::Secure(crate::tui::screens::secure::Screen::Options(s))) = self.stack.last_mut() {
                    if let crate::tui::screens::secure::options::Phase::Running {
                        share_code, file_states, receiver_info, connected_info, active_idx, total, log, relay_in_use, ..
                    } = &mut s.phase {
                        match ev {
                            E::Created { share_code: code, files: f } => {
                                *share_code = Some(code.clone());
                                *total = f.iter().map(|x| x.size).sum();
                                *file_states = f.iter().map(|x| FileState {
                                    name: x.name.clone(),
                                    size: x.size,
                                    sent: 0,
                                    status: FileStatus::Pending,
                                    started_at: None,
                                }).collect();
                                log.push(format!("Share code: {}", code));
                                log.push("Waiting for receiver\u{2026}".into());
                            }
                            E::ReceiverArrived { device_info } => {
                                *receiver_info = device_info.clone();
                                log.push(format!("Receiver arrived: {}", device_info.as_deref().unwrap_or("Unknown device")));
                            }
                            E::PeerMatched { device_info } => {
                                *connected_info = device_info.clone();
                                log.push(format!("Connected to {}", device_info.as_deref().unwrap_or("Unknown device")));
                            }
                            E::FileStart { name, size: _ } => {
                                let idx = file_states.iter().position(|f| f.name == name);
                                *active_idx = idx;
                                if let Some(i) = idx {
                                    file_states[i].status = FileStatus::Sending;
                                    file_states[i].sent = 0;
                                    file_states[i].started_at = Some(std::time::Instant::now());
                                }
                                log.push(format!("Sending {}", name));
                            }
                            E::Progress { delta } => {
                                if let Some(i) = *active_idx {
                                    if let Some(fs) = file_states.get_mut(i) {
                                        fs.sent = fs.sent.saturating_add(delta).min(fs.size);
                                    }
                                }
                            }
                            E::FileEnd => {
                                if let Some(i) = *active_idx {
                                    if let Some(fs) = file_states.get_mut(i) {
                                        fs.status = FileStatus::Done;
                                        fs.sent = fs.size;
                                    }
                                }
                                *active_idx = None;
                            }
                            E::WaitingForNext => {
                                *active_idx = None;
                                log.push("Waiting for next request\u{2026}".into());
                            }
                            E::TransferComplete => {
                                let code = share_code.clone().unwrap_or_default();
                                let log_taken = std::mem::take(log);
                                s.phase = crate::tui::screens::secure::options::Phase::Done {
                                    share_code: code,
                                    log: log_taken,
                                };
                            }
                            E::ReceiverDisconnected => {
                                // Receiver offline === finished session: from the sender's
                                // perspective "Done clicked" and "tab closed" are indistinguishable.
                                log.push("Receiver disconnected.".into());
                                let code = share_code.clone().unwrap_or_default();
                                let log_taken = std::mem::take(log);
                                s.phase = crate::tui::screens::secure::options::Phase::Done {
                                    share_code: code,
                                    log: log_taken,
                                };
                            }
                            E::Warning(msg) => {
                                log.push(format!("\u{26a0} {}", msg));
                            }
                            E::RelayDetected => {
                                *relay_in_use = true;
                                log.push("TURN relay in use".into());
                            }
                            E::Failed(msg) => {
                                s.phase = crate::tui::screens::secure::options::Phase::Failed(msg);
                            }
                        }
                    }
                }
            }
            Event::P2PReceive(ev) => {
                use crate::core::p2p::receiver::ReceiverEvent as E;
                use crate::tui::screens::download::{SecureFileStatus, SecureFileState};
                if let Some(Screen::Download(s)) = self.stack.last_mut() {
                    if let crate::tui::screens::download::Phase::SecureRunning {
                        connected_info, file_states, active_idx, log, saved_files, ..
                    } = &mut s.phase {
                        match ev {
                            E::Connecting => log.push("Connecting\u{2026}".into()),
                            E::PeerMatched { device_info } => {
                                *connected_info = device_info.clone();
                                log.push(format!("Connected to {}", device_info.as_deref().unwrap_or("Unknown device")));
                            }
                            E::FileStart { name, size } => {
                                let idx = if let Some(i) = file_states.iter().position(|f| f.name == name) {
                                    let st = &mut file_states[i];
                                    if st.size == 0 { st.size = size; }
                                    st.received = 0;
                                    st.status = SecureFileStatus::Receiving;
                                    st.started_at = Some(std::time::Instant::now());
                                    i
                                } else {
                                    file_states.push(SecureFileState {
                                        name: name.clone(),
                                        size,
                                        received: 0,
                                        status: SecureFileStatus::Receiving,
                                        started_at: Some(std::time::Instant::now()),
                                    });
                                    file_states.len() - 1
                                };
                                *active_idx = Some(idx);
                                log.push(format!("Receiving {}", name));
                            }
                            E::Progress { delta } => {
                                if let Some(idx) = *active_idx {
                                    if let Some(st) = file_states.get_mut(idx) {
                                        st.received = st.received.saturating_add(delta);
                                    }
                                }
                            }
                            E::FileEnd { name, saved_to } => {
                                let finished_idx = file_states.iter().position(|f| f.name == name);
                                if let Some(i) = finished_idx {
                                    let st = &mut file_states[i];
                                    st.status = SecureFileStatus::Done;
                                    if st.size > 0 { st.received = st.size; }
                                }
                                if *active_idx == finished_idx {
                                    *active_idx = None;
                                }
                                saved_files.push(saved_to.clone());
                                log.push(format!("Saved {} to {}", name, saved_to.display()));
                            }
                            E::TransferComplete => {
                                let log_taken = std::mem::take(log);
                                let files_taken = std::mem::take(saved_files);
                                s.phase = crate::tui::screens::download::Phase::SecureDone {
                                    saved_files: files_taken,
                                    log: log_taken,
                                };
                            }
                            E::SenderGone(msg) => {
                                s.phase = crate::tui::screens::download::Phase::Failed(msg);
                            }
                            E::Failed(msg) => {
                                s.phase = crate::tui::screens::download::Phase::Failed(msg);
                            }
                        }
                    }
                }
            }
            ev => {
                let (Some(top), Some(tasks)) =
                    (self.stack.last_mut(), self.screen_tasks.last_mut())
                else {
                    self.quit = true;
                    return;
                };
                let mut ctx = AppCtx {
                    cfg: &mut self.cfg,
                    client: &self.client,
                    tx: self.tx.as_ref(),
                    toast: &mut self.toast,
                    stdout_lines: &mut self.stdout_lines,
                    tasks,
                };
                let action = match top {
                    Screen::Dashboard(s) => crate::tui::screens::dashboard::update(s, &ev, &mut ctx),
                    Screen::Upload(us) => crate::tui::screens::upload::update(us, &ev, &mut ctx),
                    Screen::Download(s) => crate::tui::screens::download::update(s, &ev, &mut ctx),
                    Screen::Info(s) => crate::tui::screens::info::update(s, &ev, &mut ctx),
                    Screen::Delete(s) => crate::tui::screens::delete::update(s, &ev, &mut ctx),
                    Screen::Login(s) => crate::tui::screens::login::update(s, &ev, &mut ctx),
                    Screen::Logout(s) => crate::tui::screens::logout::update(s, &ev, &mut ctx),
                    Screen::Secure(s) => crate::tui::screens::secure::update(s, &ev, &mut ctx),
                };
                self.apply_action(action);
            }
        }
    }

    fn refresh_dashboard_if_top(&mut self) {
        if !self.client.is_authenticated() { return; }
        if !matches!(self.stack.last(), Some(Screen::Dashboard(_))) { return; }
        let Some(tx) = self.tx.clone() else { return; };
        if let Some(Screen::Dashboard(d)) = self.stack.last_mut() {
            d.loading = true;
            d.load_error = None;
            d.downloads_loading = true;
            d.downloads_load_error = None;
        }
        self.spawn_dashboard_fetches(tx);
    }

    fn abort_top_screen_tasks(&mut self) {
        if let Some(handles) = self.screen_tasks.pop() {
            for h in handles {
                h.abort();
            }
        }
    }

    fn apply_action(&mut self, action: ScreenAction) {
        debug_assert!(!self.stack.is_empty(), "stack must be non-empty before apply_action");
        debug_assert_eq!(
            self.stack.len(),
            self.screen_tasks.len(),
            "stack and screen_tasks must stay in lockstep"
        );
        match action {
            ScreenAction::Stay => {}
            ScreenAction::Quit => self.quit = true,
            ScreenAction::Pop => {
                self.abort_top_screen_tasks();
                self.stack.pop();
                if self.stack.is_empty() { self.quit = true; }
                self.refresh_dashboard_if_top();
            }
            ScreenAction::PopToRoot => {
                while self.stack.len() > 1 {
                    self.abort_top_screen_tasks();
                    self.stack.pop();
                }
                self.refresh_dashboard_if_top();
            }
            ScreenAction::Push(s) => {
                self.stack.push(s);
                self.screen_tasks.push(Vec::new());
            }
            ScreenAction::Replace(s) => {
                self.abort_top_screen_tasks();
                self.stack.pop();
                self.stack.push(s);
                self.screen_tasks.push(Vec::new());
                self.refresh_dashboard_if_top();
            }
            ScreenAction::LogOut => {
                self.abort_top_screen_tasks();
                self.stack.pop();
                self.cfg.token = None;
                self.cfg.user_name = None;
                if let Err(e) = self.cfg.save() {
                    self.toast = Some(Toast::error(format!("Failed to save config: {}", e)));
                    return;
                }
                match crate::client::ApiClient::new(&self.cfg) {
                    Ok(c) => { self.client = c; }
                    Err(e) => {
                        self.toast = Some(Toast::error(format!("Internal: {}", e)));
                        return;
                    }
                }
                for screen in self.stack.iter_mut() {
                    if let Screen::Dashboard(d) = screen {
                        d.items.clear();
                        d.selected = 0;
                        d.load_error = None;
                        d.loading = false;
                        break;
                    }
                }
                self.toast = Some(Toast::success("Signed out."));
            }
            ScreenAction::PushLogin => {
                self.stack
                    .push(Screen::Login(crate::tui::screens::login::State::new()));
                self.screen_tasks.push(Vec::new());
                let (Some(Screen::Login(state)), Some(tasks)) =
                    (self.stack.last_mut(), self.screen_tasks.last_mut())
                else {
                    return;
                };
                let mut ctx = AppCtx {
                    cfg: &mut self.cfg,
                    client: &self.client,
                    tx: self.tx.as_ref(),
                    toast: &mut self.toast,
                    stdout_lines: &mut self.stdout_lines,
                    tasks,
                };
                crate::tui::screens::login::on_push(state, &mut ctx);
            }
            ScreenAction::PushDownloadForCode(code) => {
                let state = crate::tui::screens::download::State::new_with_pending(code.clone());
                self.stack.push(Screen::Download(state));
                self.screen_tasks.push(Vec::new());
                if let Some(tx) = self.tx.clone() {
                    let client = self.client.clone();
                    let code_for_task = code.clone();
                    let handle = tokio::spawn(async move {
                        let r = crate::core::shares::get_share_info(&client, &code_for_task).await;
                        let _ = tx.send(crate::tui::event::Event::DownloadInfo(r));
                    });
                    if let Some(tasks) = self.screen_tasks.last_mut() {
                        tasks.push(handle.abort_handle());
                    }
                }
            }
            ScreenAction::PushInfoForCode(code) => {
                let state = crate::tui::screens::info::State::new_with_pending(code.clone());
                self.stack.push(Screen::Info(state));
                self.screen_tasks.push(Vec::new());
                if let Some(tx) = self.tx.clone() {
                    let client = self.client.clone();
                    let code_for_task = code.clone();
                    let handle = tokio::spawn(async move {
                        let r = crate::core::shares::get_share_info(&client, &code_for_task).await;
                        let _ = tx.send(crate::tui::event::Event::InfoLoaded(r));
                    });
                    if let Some(tasks) = self.screen_tasks.last_mut() {
                        tasks.push(handle.abort_handle());
                    }
                }
            }
        }
    }

    pub fn render(&self, f: &mut Frame) {
        let area = f.area();
        if area.width < 60 || area.height < 16 {
            let msg = format!(
                " Resize terminal to at least 60x16 (current: {}x{}). ",
                area.width, area.height
            );
            f.render_widget(
                ratatui::widgets::Paragraph::new(msg)
                    .style(ratatui::style::Style::default().fg(ratatui::style::Color::Yellow)),
                area,
            );
            return;
        }
        let Some(top) = self.stack.last() else { return; };
        match top {
            Screen::Dashboard(s) => screens::dashboard::render(s, f, &self.client, self.toast.as_ref()),
            Screen::Upload(s) => screens::upload::render(s, f),
            Screen::Download(s) => screens::download::render(s, f),
            Screen::Info(s) => screens::info::render(s, f),
            Screen::Delete(s) => screens::delete::render(s, f),
            Screen::Login(s) => screens::login::render(s, f),
            Screen::Logout(s) => screens::logout::render(s, f),
            Screen::Secure(s) => screens::secure::render(s, f),
        }
    }
}

pub struct AppCtx<'a> {
    pub cfg: &'a mut CliConfig,
    pub client: &'a ApiClient,
    pub tx: Option<&'a Tx>,
    pub toast: &'a mut Option<Toast>,
    pub stdout_lines: &'a mut Vec<String>,
    /// Screens that call `tokio::spawn` push the resulting `abort_handle()` here so the work
    /// gets cancelled when the screen pops.
    pub tasks: &'a mut Vec<AbortHandle>,
}
