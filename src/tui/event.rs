use crate::core::auth::{DeviceSession, DeviceStatus};
use crate::core::error::CoreError;
use crate::core::shares::{DownloadItem, FileInfo, UploadItem};
use crate::core::upload::ShareResult;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use tui_textarea::{Input, Key};

#[allow(dead_code)]
pub enum Event {
    Key(KeyEvent),
    Tick,
    Resize(u16, u16),

    UploadsLoaded(Result<Vec<UploadItem>, CoreError>),
    DownloadsLoaded(Result<Vec<DownloadItem>, CoreError>),
    UploadProgress { delta: u64 },
    UploadFinished(Result<ShareResult, CoreError>),

    DownloadInfo(Result<FileInfo, CoreError>),
    DownloadProgress { delta: u64 },
    DownloadFinished(Result<PathBuf, CoreError>),

    InfoLoaded(Result<FileInfo, CoreError>),
    DeleteFinished(Result<String, CoreError>),

    LoginSessionReady(Result<DeviceSession, CoreError>),
    LoginPolled(Result<DeviceStatus, CoreError>),

    P2PSend(crate::core::p2p::sender::SenderEvent),
    P2PReceive(crate::core::p2p::receiver::ReceiverEvent),
}

pub type Tx = mpsc::UnboundedSender<Event>;

/// Returns true if `k` should be forwarded to a share-code TextArea.
/// Share codes are 6-digit numbers; anything else (letters, symbols, overflow) is rejected.
/// Editing keys (Backspace, arrows, Home/End, Delete) always pass through.
pub fn accept_share_code_input(code: &tui_textarea::TextArea, k: &KeyEvent) -> bool {
    match k.code {
        KeyCode::Char(c) => {
            c.is_ascii_digit() && code.lines().join("").chars().count() < 6
        }
        _ => true,
    }
}

/// Convert a crossterm KeyEvent into a tui_textarea Input.
pub fn ev_to_input(k: &KeyEvent) -> Input {
    let key = match k.code {
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Enter => Key::Enter,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::Delete => Key::Delete,
        _ => Key::Null,
    };
    Input {
        key,
        ctrl: k.modifiers.contains(KeyModifiers::CONTROL),
        alt: k.modifiers.contains(KeyModifiers::ALT),
        shift: k.modifiers.contains(KeyModifiers::SHIFT),
    }
}

pub async fn run_loop<B: ratatui::backend::Backend>(
    terminal: &mut ratatui::Terminal<B>,
    app: &mut crate::tui::app::App,
) -> anyhow::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();
    spawn_input_task(tx.clone());
    spawn_tick_task(tx.clone(), Duration::from_millis(100));
    app.set_tx(tx);
    app.on_enter();

    loop {
        terminal.draw(|f| app.render(f))?;
        let Some(ev) = rx.recv().await else { break; };
        app.update(ev);
        if app.should_quit() { break; }
    }
    Ok(())
}

fn spawn_input_task(tx: Tx) {
    tokio::task::spawn_blocking(move || {
        use crossterm::event::{self, Event as CtEvent};
        loop {
            match event::read() {
                Ok(CtEvent::Key(k)) if k.kind == KeyEventKind::Press => {
                    if tx.send(Event::Key(k)).is_err() {
                        break;
                    }
                }
                Ok(CtEvent::Resize(w, h)) => {
                    if tx.send(Event::Resize(w, h)).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
}

fn spawn_tick_task(tx: Tx, period: Duration) {
    tokio::spawn(async move {
        let mut t = tokio::time::interval(period);
        loop {
            t.tick().await;
            if tx.send(Event::Tick).is_err() {
                break;
            }
        }
    });
}
