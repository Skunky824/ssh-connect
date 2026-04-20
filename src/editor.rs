use crate::ssh::SshSession;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::time::SystemTime;

/// Download a remote file, open it in an external editor, and upload if changed.
///
/// This suspends the TUI, runs the editor, then the caller should resume the TUI.
/// Returns `true` if the file was modified and re-uploaded.
pub async fn edit_remote_file(
    ssh: &SshSession,
    remote_path: &str,
) -> Result<bool> {
    // Download file
    let data = ssh.read_file(remote_path).await?;

    // Write to a temp file preserving the filename
    let file_name = remote_path
        .rsplit('/')
        .next()
        .unwrap_or("file");
    let temp_dir = tempfile::tempdir().context("Failed to create temp dir")?;
    let local_path = temp_dir.path().join(file_name);

    std::fs::write(&local_path, &data).context("Failed to write temp file")?;
    let mtime_before = file_mtime(&local_path);

    // Open in editor (local)
    let (editor_cmd, editor_args) = find_editor();
    let mut cmd = std::process::Command::new(&editor_cmd);
    cmd.arg(&local_path);
    for arg in editor_args {
        cmd.arg(arg);
    }
    
    let status = cmd.status()
        .context(format!("Failed to launch editor '{}'", editor_cmd))?;

    if !status.success() {
        anyhow::bail!("Editor exited with error: {}", status);
    }

    // Check if file was modified
    let mtime_after = file_mtime(&local_path);
    let contents_after = std::fs::read(&local_path)?;

    let changed = mtime_before != mtime_after || contents_after != data;

    if changed {
        if let Err(e) = ssh.write_file(remote_path, &contents_after).await {
            ssh.write_file_via_shell(remote_path, &contents_after)
                .await
                .context(format!(
                    "Save failed for '{}' via SFTP (likely identity mismatch after sudo -i): {:#}",
                    remote_path,
                    e
                ))?;
        }
    }

    Ok(changed)
}

/// Determine which editor to use. Returns (command, args).
fn find_editor() -> (String, Vec<String>) {
    // Check $VISUAL, then $EDITOR
    if let Ok(editor) = std::env::var("VISUAL") {
        let parts: Vec<&str> = editor.split_whitespace().collect();
        if !parts.is_empty() {
            let cmd = parts[0].to_string();
            let args = parts[1..].iter().map(|s| s.to_string()).collect();
            return (cmd, args);
        }
    }
    if let Ok(editor) = std::env::var("EDITOR") {
        let parts: Vec<&str> = editor.split_whitespace().collect();
        if !parts.is_empty() {
            let cmd = parts[0].to_string();
            let args = parts[1..].iter().map(|s| s.to_string()).collect();
            return (cmd, args);
        }
    }

    // Platform-specific defaults
    #[cfg(windows)]
    {
        // Try code command (if in PATH)
        if command_works("code", &["--version"]) {
            return ("code".to_string(), vec!["--wait".to_string()]);
        }
        
        // Try common VS Code installation paths on Windows
        let vscode_paths = vec![
            r"C:\Program Files\Microsoft VS Code\bin\code.cmd",
            r"C:\Program Files (x86)\Microsoft VS Code\bin\code.cmd",
            r"C:\Users\claudio.salvai\AppData\Local\Programs\Microsoft VS Code\bin\code.cmd",
        ];
        for path in vscode_paths {
            if std::path::Path::new(path).exists() {
                return (path.to_string(), vec!["--wait".to_string()]);
            }
        }
        
        // Fallback to Notepad (always available on Windows)
        return ("notepad".to_string(), vec![]);
    }

    #[cfg(not(windows))]
    {
        // Try nano, vim, vi
        for name in &["nano", "vim", "vi"] {
            if which_exists(name) {
                return (name.to_string(), vec![]);
            }
        }
        ("vi".to_string(), vec![])
    }
}

fn command_works(cmd: &str, args: &[&str]) -> bool {
    std::process::Command::new(cmd)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn which_exists(name: &str) -> bool {
    #[cfg(windows)]
    {
        std::process::Command::new("where")
            .arg(name)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[cfg(not(windows))]
    {
        std::process::Command::new("which")
            .arg(name)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

fn file_mtime(path: &PathBuf) -> Option<SystemTime> {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
}
