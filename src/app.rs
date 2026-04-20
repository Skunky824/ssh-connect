use crate::browser::{EntryAction, FileBrowser};
use crate::editor;
use crate::ssh::SshSession;
use crate::ui::{self, Focus};
use crate::vterm::VTerm;
use anyhow::Result;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode,
    KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::terminal::{
    self, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use russh::ChannelMsg;
use std::io;

/// Run the main application.
pub async fn run(
    user: String,
    host: String,
    port: u16,
    identity: Option<String>,
    strict_privacy: bool,
) -> Result<()> {
    // Get initial terminal size for the PTY
    let (cols, rows) = terminal::size()?;
    let mut zoom: u16 = 1;

    // Reserve space: 2 rows for borders, 1 for status bar, 30 cols for sidebar + border.
    let (pty_cols, pty_rows) = compute_pty_size(cols, rows, true, zoom);

    eprintln!("Connecting to {}@{}:{}...", user, host, port);

    let mut ssh = SshSession::connect(
        &user,
        &host,
        port,
        identity.as_deref(),
        pty_cols as u32,
        pty_rows as u32,
        strict_privacy,
    )
    .await?;

    // Enter TUI mode
    let mut stdout = io::stdout();
    terminal::enable_raw_mode()?;
    stdout.execute(EnterAlternateScreen)?;
    stdout.execute(EnableMouseCapture)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // State
    let mut vterm = VTerm::new(pty_rows as usize, pty_cols as usize);
    let mut browser = FileBrowser::new();
    let mut focus = Focus::Terminal;
    let mut show_sidebar = true;
    let mut status_msg = format!(
        "{}@{}:{}  privacy:{}",
        user,
        host,
        port,
        if strict_privacy { "strict" } else { "relaxed" }
    );
    let mut should_quit = false;
    let mut event_stream = EventStream::new();

    // Main event loop
    loop {
        // Keep current selection visible in sidebar.
        if show_sidebar {
            let (_, rows) = terminal::size()?;
            let visible_height = rows.saturating_sub(3) as usize;
            browser.adjust_scroll(visible_height);
        }

        // Refresh browser if needed
        if browser.needs_refresh {
            browser.refresh(&ssh).await;
        }

        // Render
        {
            let bref = &browser;
            let vref = &vterm;
            let smsg = &status_msg;
            terminal.draw(|f| {
                ui::render(f, bref, vref, focus, show_sidebar, smsg);
            })?;
        }

        if should_quit {
            break;
        }

        // Wait for events
        tokio::select! {
            biased;

            // SSH channel data (prioritized for responsiveness)
            msg = ssh.wait() => {
                match msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        vterm.process(data);

                        // Check for CWD change
                        if let Some(cwd) = vterm.take_cwd() {
                            status_msg = format!("{}@{}  {}", user, host, cwd);
                            browser.set_path(cwd);
                        } else if let Some(cwd) = vterm.detect_cwd_from_prompt() {
                            if browser.current_path != cwd {
                                status_msg = format!("{}@{}  {}", user, host, cwd);
                                browser.set_path(cwd);
                            }
                        }
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        status_msg = format!("Remote exited with status {}", exit_status);
                        should_quit = true;
                    }
                    Some(ChannelMsg::Eof) | None => {
                        status_msg = "Connection closed".to_string();
                        should_quit = true;
                    }
                    _ => {}
                }
            }

            // Local input events
            event = event_stream.next() => {
                if let Some(Ok(ev)) = event {
                    match ev {
                        Event::Key(key) => {
                            handle_key(
                                key,
                                &mut focus,
                                &mut show_sidebar,
                                &mut should_quit,
                                &mut browser,
                                &mut vterm,
                                &ssh,
                                &mut terminal,
                                &mut status_msg,
                                &user,
                                &host,
                                zoom,
                            )
                            .await?;
                        }
                        Event::Resize(new_cols, new_rows) => {
                            let (pty_cols, pty_rows) = compute_pty_size(new_cols, new_rows, show_sidebar, zoom);
                            vterm.resize(pty_rows as usize, pty_cols as usize);
                            ssh.resize(pty_cols as u32, pty_rows as u32).await?;
                        }
                        Event::Mouse(mouse) => {
                            handle_mouse(
                                mouse,
                                &mut focus,
                                show_sidebar,
                                &mut browser,
                                &mut vterm,
                                &ssh,
                                &mut zoom,
                            )
                            .await?;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Cleanup
    terminal::disable_raw_mode()?;
    io::stdout().execute(DisableMouseCapture)?;
    io::stdout().execute(LeaveAlternateScreen)?;
    println!("Disconnected.");
    Ok(())
}

fn compute_pty_size(
    total_cols: u16,
    total_rows: u16,
    show_sidebar: bool,
    zoom: u16,
) -> (u16, u16) {
    let pane_cols = if show_sidebar {
        total_cols.saturating_sub(33)
    } else {
        total_cols.saturating_sub(3)
    };
    let pane_rows = total_rows.saturating_sub(3);
    let z = zoom.max(1);
    ((pane_cols / z).max(1), (pane_rows / z).max(1))
}

async fn handle_mouse(
    mouse: MouseEvent,
    focus: &mut Focus,
    show_sidebar: bool,
    browser: &mut FileBrowser,
    vterm: &mut VTerm,
    ssh: &SshSession,
    zoom: &mut u16,
) -> Result<()> {
    let (cols, rows) = terminal::size()?;
    let status_row = rows.saturating_sub(1);
    if mouse.row >= status_row {
        return Ok(());
    }

    if mouse.modifiers.contains(KeyModifiers::CONTROL) {
        let old_zoom = *zoom;
        match mouse.kind {
            MouseEventKind::ScrollUp => *zoom = zoom.saturating_add(1).min(4),
            MouseEventKind::ScrollDown => *zoom = zoom.saturating_sub(1).max(1),
            _ => {}
        }

        if *zoom != old_zoom {
            let (pty_cols, pty_rows) = compute_pty_size(cols, rows, show_sidebar, *zoom);
            vterm.resize(pty_rows as usize, pty_cols as usize);
            ssh.resize(pty_cols as u32, pty_rows as u32).await?;
        }
        return Ok(());
    }

    if !show_sidebar {
        return Ok(());
    }

    // Sidebar occupies the first 30 columns when visible.
    let in_sidebar = mouse.column < 30;

    match mouse.kind {
        MouseEventKind::ScrollUp if in_sidebar => {
            *focus = Focus::Sidebar;
            browser.select_up();
        }
        MouseEventKind::ScrollDown if in_sidebar => {
            *focus = Focus::Sidebar;
            browser.select_down();
        }
        MouseEventKind::Down(MouseButton::Left) if in_sidebar => {
            *focus = Focus::Sidebar;
        }
        MouseEventKind::Down(MouseButton::Left) if mouse.column < cols => {
            *focus = Focus::Terminal;
        }
        _ => {}
    }

    Ok(())
}

async fn handle_key(
    key: KeyEvent,
    focus: &mut Focus,
    show_sidebar: &mut bool,
    should_quit: &mut bool,
    browser: &mut FileBrowser,
    vterm: &mut VTerm,
    ssh: &SshSession,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    status_msg: &mut String,
    user: &str,
    host: &str,
    zoom: u16,
) -> Result<()> {
    // Filter out release events on Windows (events fire twice: press + release)
    if key.kind != crossterm::event::KeyEventKind::Press {
        return Ok(());
    }
    
    // Global keybindings
    match (key.modifiers, key.code) {
        // Ctrl+Q: Quit
        (KeyModifiers::CONTROL, KeyCode::Char('q')) => {
            *should_quit = true;
            return Ok(());
        }
        // Ctrl+B: Toggle focus (tmux-like)
        (KeyModifiers::CONTROL, KeyCode::Char('b')) if *show_sidebar => {
            *focus = match focus {
                Focus::Sidebar => Focus::Terminal,
                Focus::Terminal => Focus::Sidebar,
            };
            return Ok(());
        }
        // Ctrl+F: Start search in sidebar
        (KeyModifiers::CONTROL, KeyCode::Char('f')) if *show_sidebar => {
            *focus = Focus::Sidebar;
            browser.start_search();
            return Ok(());
        }
        // F2: Toggle sidebar
        (_, KeyCode::F(2)) => {
            *show_sidebar = !*show_sidebar;
            // Resize PTY when sidebar toggles
            let (cols, rows) = terminal::size()?;
            let (pty_cols, pty_rows) = compute_pty_size(cols, rows, *show_sidebar, zoom);
            vterm.resize(pty_rows as usize, pty_cols as usize);
            ssh.resize(pty_cols as u32, pty_rows as u32).await?;
            return Ok(());
        }
        // F3: Re-inject CWD hook
        (_, KeyCode::F(3)) => {
            if ssh.strict_privacy() {
                *status_msg = format!(
                    "{}@{} — privacy:strict (helper shell commands are disabled)",
                    user, host
                );
            } else {
                *status_msg = format!(
                    "{}@{} — privacy:relaxed (helper shell fallbacks enabled)",
                    user, host
                );
            }
            return Ok(());
        }
        _ => {}
    }

    // Focus-specific keybindings
    match focus {
        Focus::Sidebar => handle_sidebar_key(key, browser, ssh, terminal, status_msg).await,
        Focus::Terminal => handle_terminal_key(key, ssh).await,
    }
}

async fn handle_sidebar_key(
    key: KeyEvent,
    browser: &mut FileBrowser,
    ssh: &SshSession,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    status_msg: &mut String,
) -> Result<()> {
    if browser.search_active {
        match key.code {
            KeyCode::Esc => browser.cancel_search(),
            KeyCode::Enter => browser.finish_search(),
            KeyCode::Backspace => browser.pop_search_char(),
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                browser.push_search_char(c);
            }
            _ => {}
        }
        return Ok(());
    }

    match key.code {
        KeyCode::Char('/') => {
            browser.start_search();
        }
        KeyCode::Home => {
            browser.select_home();
        }
        KeyCode::End => {
            browser.select_end();
        }
        KeyCode::PageUp => {
            let (_, rows) = terminal::size()?;
            let page = rows.saturating_sub(3) as usize;
            browser.page_up(page.max(1));
        }
        KeyCode::PageDown => {
            let (_, rows) = terminal::size()?;
            let page = rows.saturating_sub(3) as usize;
            browser.page_down(page.max(1));
        }
        KeyCode::Up | KeyCode::Char('k') => {
            browser.select_up();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            browser.select_down();
        }
        KeyCode::Enter => {
            if let Some(action) = browser.enter() {
                match action {
                    EntryAction::EnterDir => {
                        // Will refresh on next loop iteration
                    }
                    EntryAction::OpenFile(path) => {
                        // Suspend TUI, open in editor
                        terminal::disable_raw_mode()?;
                        io::stdout().execute(LeaveAlternateScreen)?;

                        *status_msg = format!("Editing {}...", path);

                        match editor::edit_remote_file(ssh, &path).await {
                            Ok(true) => {
                                *status_msg = format!("Saved {}", path);
                            }
                            Ok(false) => {
                                *status_msg = format!("No changes to {}", path);
                            }
                            Err(e) => {
                                *status_msg = format!("Edit error: {}", e);
                            }
                        }

                        // Resume TUI
                        terminal::enable_raw_mode()?;
                        io::stdout().execute(EnterAlternateScreen)?;
                        terminal.clear()?;
                    }
                }
            }
        }
        KeyCode::Backspace | KeyCode::Left => {
            browser.go_parent();
        }
        KeyCode::Char('r') => {
            browser.needs_refresh = true;
        }
        _ => {}
    }
    Ok(())
}

async fn handle_terminal_key(key: KeyEvent, ssh: &SshSession) -> Result<()> {
    // Translate crossterm key events to bytes for the remote PTY
    let bytes = key_to_bytes(key);
    if !bytes.is_empty() {
        ssh.send_bytes(&bytes).await?;
    }
    Ok(())
}

/// Convert a crossterm KeyEvent into the byte sequence expected by a remote PTY.
fn key_to_bytes(key: KeyEvent) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                // Ctrl+A = 0x01, Ctrl+Z = 0x1A, etc.
                let byte = (c.to_ascii_lowercase() as u8).wrapping_sub(b'a').wrapping_add(1);
                if alt {
                    vec![0x1b, byte]
                } else {
                    vec![byte]
                }
            } else if alt {
                let mut bytes = vec![0x1b];
                let mut buf = [0u8; 4];
                bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                bytes
            } else {
                let mut buf = [0u8; 4];
                c.encode_utf8(&mut buf).as_bytes().to_vec()
            }
        }
        KeyCode::Enter => vec![0x0d],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![0x09],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::F(n) => match n {
            1 => b"\x1bOP".to_vec(),
            2 => b"\x1bOQ".to_vec(),
            3 => b"\x1bOR".to_vec(),
            4 => b"\x1bOS".to_vec(),
            5 => b"\x1b[15~".to_vec(),
            6 => b"\x1b[17~".to_vec(),
            7 => b"\x1b[18~".to_vec(),
            8 => b"\x1b[19~".to_vec(),
            9 => b"\x1b[20~".to_vec(),
            10 => b"\x1b[21~".to_vec(),
            11 => b"\x1b[23~".to_vec(),
            12 => b"\x1b[24~".to_vec(),
            _ => vec![],
        },
        _ => vec![],
    }
}
