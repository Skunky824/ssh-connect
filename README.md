# ssh-connect

A fast SSH terminal with a built-in remote file browser and local editor workflow.

## Privacy Mode

- Strict privacy is ON by default.
- In strict mode, ssh-connect avoids sending helper shell commands to the remote host.
- This reduces shell-history traces created by the tool itself.
- If you explicitly want helper shell fallbacks, run with `--no-strict-privacy`.

## Why This Program Is Useful

- Fast remote workflow in one TUI: terminal + file browser side-by-side.
- CWD-aware sidebar: follows shell directory changes, including `sudo -i` scenarios.
- Root-friendly operations: handles protected paths with privileged fallbacks when available.
- External local editing: open remote files in your local editor and sync changes back.
- Keyboard-driven UX: tmux-like pane switching (`Ctrl+B`) and vim-like browser search (`/` or `Ctrl+F`).
- Terminal zoom support with `Ctrl+Mouse Wheel`.
- Cross-platform runtime: works on Windows, Linux, and macOS (built with Rust).

## Core Features

- SSH interactive terminal with PTY support.
- Left sidebar remote browser (directories/files).
- Open file from sidebar into local editor.
- Upload on save/close.
- Current-directory tracking from shell output.
- Sidebar search in current directory.

## Keybindings

- `Ctrl+Q`: Quit
- `F2`: Toggle sidebar
- `F3`: Re-inject cwd hook
- `Ctrl+B`: Switch focus between terminal and sidebar
- Sidebar mode:
  - `Up/Down` or `k/j`: Move selection
  - `Enter`: Enter directory or open file
  - `Backspace` or `Left`: Go parent directory
  - `r`: Refresh listing
  - `/` or `Ctrl+F`: Start incremental search
  - `Home` / `End`: Jump to first/last item
  - `PageUp` / `PageDown`: Move by one page
  - `Ctrl+Mouse Wheel`: Zoom in/out terminal pane
  - Search mode: type to search, `Backspace` delete char, `Enter` accept, `Esc` cancel

## Build

```powershell
cargo build --release
```

Binary path:

- `target\release\ssh-connect.exe`

## Usage

```powershell
ssh-connect user@host
```

Disable strict privacy (optional):

```powershell
ssh-connect user@host --no-strict-privacy
```

## Installer (.exe)

This repository includes an Inno Setup script:

- `installer\ssh-connect.iss`

Build installer with:

```powershell
"C:\Program Files (x86)\Inno Setup 6\ISCC.exe" installer\ssh-connect.iss
```

Output installer:

- `dist\ssh-connect-setup.exe`

Installer behavior:

- Adds `ssh-connect` install directory to the current user `PATH` (default enabled in installer tasks).

## License

GPL-3.0. See `LICENSE`.
