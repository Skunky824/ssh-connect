use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine;
use russh::client;
use russh::ChannelMsg;
use russh_sftp::client::SftpSession;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

/// Entry returned from SFTP directory listing.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// Wraps an SSH session, PTY channel, and SFTP session.
pub struct SshSession {
    handle: Mutex<client::Handle<Handler>>,
    pub _pty_channel_id: russh::ChannelId,
    pty_channel: russh::Channel<client::Msg>,
    sftp: SftpSession,
    sudo_sftp: Mutex<Option<SftpSession>>,
    auth_password: Option<String>,
    strict_privacy: bool,
}

struct Handler;

#[async_trait]
impl client::Handler for Handler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        // TODO: check known_hosts for production use
        Ok(true)
    }
}

impl SshSession {
    async fn open_sudo_sftp_channel(
        &self,
        command: &str,
        stdin_data: Option<&[u8]>,
    ) -> Result<SftpSession> {
        let mut handle = self.handle.lock().await;
        let sudo_channel = handle
            .channel_open_session()
            .await
            .context("Failed to open sudo SFTP channel")?;
        drop(handle);

        sudo_channel
            .exec(true, command)
            .await
            .context("Failed to exec sudo sftp-server command")?;

        if let Some(data) = stdin_data {
            sudo_channel
                .data(data)
                .await
                .context("Failed to send sudo password to channel")?;
        }

        let sudo_sftp = SftpSession::new(sudo_channel.into_stream())
            .await
            .context("Failed to initialize sudo SFTP session")?;
        Ok(sudo_sftp)
    }

    async fn ensure_sudo_sftp(&self) -> Result<()> {
        {
            let guard = self.sudo_sftp.lock().await;
            if guard.is_some() {
                return Ok(());
            }
        }

        // First try non-interactive sudo (works if sudo timestamp is reusable).
        let sudo_sftp = match self
            .open_sudo_sftp_channel(
                "sudo -n sh -c 'exec /usr/libexec/openssh/sftp-server || exec /usr/lib/openssh/sftp-server || exec sftp-server'",
                None,
            )
            .await
        {
            Ok(s) => s,
            Err(non_interactive_err) => {
                // If tty tickets prevent timestamp reuse, retry by sending password.
                if let Some(password) = &self.auth_password {
                    let mut pass = password.clone();
                    pass.push('\n');
                    self.open_sudo_sftp_channel(
                        "sudo -S -p '' sh -c 'exec /usr/libexec/openssh/sftp-server || exec /usr/lib/openssh/sftp-server || exec sftp-server'",
                        Some(pass.as_bytes()),
                    )
                    .await
                    .with_context(|| {
                        format!(
                            "Failed to start sudo SFTP with both sudo -n and sudo -S. First error: {:#}",
                            non_interactive_err
                        )
                    })?
                } else {
                    return Err(anyhow::anyhow!(
                        "Failed to start sudo SFTP with sudo -n and no password is available for sudo -S fallback: {:#}",
                        non_interactive_err
                    ));
                }
            }
        };

        let mut guard = self.sudo_sftp.lock().await;
        *guard = Some(sudo_sftp);
        Ok(())
    }

    /// Connect, authenticate, open a PTY channel and an SFTP session.
    pub async fn connect(
        user: &str,
        host: &str,
        port: u16,
        identity: Option<&str>,
        cols: u32,
        rows: u32,
        strict_privacy: bool,
    ) -> Result<Self> {
        let config = client::Config {
            ..Default::default()
        };
        let mut handle =
            client::connect(Arc::new(config), (host, port), Handler)
                .await
                .context("SSH connect failed")?;

        // --- authenticate ---
        let mut auth_password: Option<String> = None;

        let authenticated = if let Some(key_path) = identity {
            let key = russh_keys::load_secret_key(key_path, None)
                .context("Failed to load identity key")?;
            handle
                .authenticate_publickey(user, Arc::new(key))
                .await
                .context("Public-key auth failed")?
        } else {
            // Try default keys
            let mut authed = false;
            for name in &["id_ed25519", "id_rsa", "id_ecdsa"] {
                if let Some(home) = dirs::home_dir() {
                    let path = home.join(".ssh").join(name);
                    if path.exists() {
                        if let Ok(key) = russh_keys::load_secret_key(path.to_str().unwrap(), None)
                        {
                            if let Ok(true) = handle
                                .authenticate_publickey(user, Arc::new(key))
                                .await
                            {
                                authed = true;
                                break;
                            }
                        }
                    }
                }
            }
            if !authed {
                // Prompt for password
                let password = rpassword_prompt("Password: ")?;
                let res = handle
                    .authenticate_password(user, &password)
                    .await;
                match res {
                    Ok(ok) => {
                        if ok {
                            auth_password = Some(password);
                        }
                        ok
                    }
                    Err(e) => {
                        eprintln!("Password auth error: {}", e);
                        false
                    }
                }
            } else {
                true
            }
        };

        if !authenticated {
            anyhow::bail!("Authentication failed");
        }

        // --- open PTY channel ---
        let pty_channel = handle
            .channel_open_session()
            .await
            .context("Failed to open PTY channel")?;

        pty_channel
            .request_pty(true, "xterm-256color", cols, rows, 0, 0, &[])
            .await
            .context("Failed to request PTY")?;

        pty_channel
            .request_shell(true)
            .await
            .context("Failed to request shell")?;

        let pty_channel_id = pty_channel.id();

        // --- open SFTP session ---
        let sftp_channel = handle
            .channel_open_session()
            .await
            .context("Failed to open SFTP channel")?;
        sftp_channel
            .request_subsystem(true, "sftp")
            .await
            .context("Failed to request SFTP subsystem")?;

        let sftp = SftpSession::new(sftp_channel.into_stream())
            .await
            .context("Failed to initialise SFTP session")?;

        Ok(SshSession {
            handle: Mutex::new(handle),
            _pty_channel_id: pty_channel_id,
            pty_channel,
            sftp,
            sudo_sftp: Mutex::new(None),
            auth_password,
            strict_privacy,
        })
    }

    pub fn strict_privacy(&self) -> bool {
        self.strict_privacy
    }

    /// Send bytes to the remote PTY.
    pub async fn send_bytes(&self, data: &[u8]) -> Result<()> {
        self.pty_channel
            .data(data)
            .await
            .context("Failed to send data to PTY")?;
        Ok(())
    }

    /// Wait for the next message from the PTY channel.
    pub async fn wait(&mut self) -> Option<ChannelMsg> {
        self.pty_channel.wait().await
    }

    /// Resize the remote PTY.
    pub async fn resize(&self, cols: u32, rows: u32) -> Result<()> {
        self.pty_channel
            .window_change(cols, rows, 0, 0)
            .await
            .context("Failed to resize PTY")?;
        Ok(())
    }

    /// Inject the CWD-tracking shell hook into the current remote shell.
    pub async fn inject_cwd_hook(&self) -> Result<()> {
        // History-safe mode: do not inject shell commands on the remote host.
        // CWD tracking relies on prompt parsing plus OSC 7 when already present.
        Ok(())
    }

    // ---- SFTP operations ----

    /// List directory contents.
    pub async fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>> {
        if path.is_empty() {
            anyhow::bail!("Cannot list empty path");
        }

        let entries = match self.sftp.read_dir(path).await {
            Ok(entries) => entries,
            Err(primary_err) => {
                self.ensure_sudo_sftp().await.with_context(|| {
                    format!(
                        "Cannot read dir [{}] with user SFTP ({:?}) and failed to enable sudo SFTP",
                        path, primary_err
                    )
                })?;

                let mut guard = self.sudo_sftp.lock().await;
                let sudo = guard
                    .as_mut()
                    .context("sudo SFTP session missing after initialization")?;
                sudo.read_dir(path)
                    .await
                    .map_err(|sudo_err| {
                        anyhow::anyhow!(
                            "Cannot read dir [{}]: user SFTP error: {:?}; sudo SFTP error: {:?}",
                            path,
                            primary_err,
                            sudo_err
                        )
                    })?
            }
        };

        let mut result: Vec<DirEntry> = entries
            .into_iter()
            .filter(|e| {
                let name = e.file_name();
                name != "." && name != ".."
            })
            .map(|e| {
                let name = e.file_name();
                let is_dir = e.file_type().is_dir();
                let size = e.metadata().len();
                DirEntry { name, is_dir, size }
            })
            .collect();

        // Sort: directories first, then alphabetical
        result.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        Ok(result)
    }

    /// Read a remote file into bytes.
    pub async fn read_file(&self, path: &str) -> Result<Vec<u8>> {
        match self.sftp.read(path).await {
            Ok(data) => Ok(data),
            Err(primary_err) => {
                self.ensure_sudo_sftp().await.with_context(|| {
                    format!(
                        "SFTP read failed for [{}] ({:?}) and sudo SFTP is unavailable",
                        path, primary_err
                    )
                })?;

                let mut guard = self.sudo_sftp.lock().await;
                let sudo = guard
                    .as_mut()
                    .context("sudo SFTP session missing after initialization")?;
                sudo.read(path).await.map_err(|sudo_err| {
                    anyhow::anyhow!(
                        "Read failed for [{}]: user SFTP error: {:?}; sudo SFTP error: {:?}",
                        path,
                        primary_err,
                        sudo_err
                    )
                })
            }
        }
    }

    /// Write bytes to a remote file.
    pub async fn write_file(&self, path: &str, data: &[u8]) -> Result<()> {
        match self.sftp.create(path).await {
            Ok(mut file) => {
                file.write_all(data).await.context("SFTP write failed")?;
                file.shutdown().await.ok();
                Ok(())
            }
            Err(primary_err) => {
                self.ensure_sudo_sftp().await.with_context(|| {
                    format!(
                        "SFTP create failed for [{}] ({:?}) and sudo SFTP is unavailable",
                        path, primary_err
                    )
                })?;

                let mut guard = self.sudo_sftp.lock().await;
                let sudo = guard
                    .as_mut()
                    .context("sudo SFTP session missing after initialization")?;
                let mut file = sudo.create(path).await.map_err(|sudo_err| {
                    anyhow::anyhow!(
                        "Create failed for [{}]: user SFTP error: {:?}; sudo SFTP error: {:?}",
                        path,
                        primary_err,
                        sudo_err
                    )
                })?;
                file.write_all(data)
                    .await
                    .context("sudo SFTP write failed")?;
                file.shutdown().await.ok();
                Ok(())
            }
        }
    }

    /// Write bytes through the interactive shell.
    ///
    /// This is a fallback for cases where SFTP identity cannot write a path,
    /// but the current shell has elevated privileges (e.g. after `sudo -i`).
    pub async fn write_file_via_shell(&self, path: &str, data: &[u8]) -> Result<()> {
        if self.strict_privacy {
            anyhow::bail!(
                "Shell write fallback blocked by strict privacy mode. Re-run with --no-strict-privacy to allow helper shell commands."
            );
        }
        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        let quoted_path = shell_quote(path);
        let cmd = format!(
            "base64 -d > {path} <<'__SSH_CONNECT_B64__'\n{data}\n__SSH_CONNECT_B64__\n",
            path = quoted_path,
            data = encoded
        );
        self.send_bytes(cmd.as_bytes())
            .await
            .context("Failed to send shell write command")?;
        Ok(())
    }
}

fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\"'\"'");
    format!("'{}'", escaped)
}

/// Simple password prompt (read from stdin, no echo—handled by SSH connection).
fn rpassword_prompt(prompt: &str) -> Result<String> {
    use std::io::Write;

    eprint!("{}", prompt);
    std::io::stderr().flush()?;

    // Simply read a line—SSH connection will handle TTY mode
    let mut password = String::new();
    std::io::stdin().read_line(&mut password)?;
    Ok(password.trim_end().to_string())
}
