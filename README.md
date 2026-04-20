# ssh-connect

A fast SSH terminal with a built-in remote file browser and local editor workflow.

## Why This Program Is Useful

- Fast remote workflow in one TUI: terminal + file browser side-by-side.
- CWD-aware sidebar: follows shell directory changes, including `sudo -i` scenarios.
- Root-friendly operations: handles protected paths with privileged fallbacks when available.
- External local editing: open remote files in your local editor and sync changes back.
- Keyboard-driven UX: tmux-like pane switching (`Ctrl+B`) and vim-like browser search (`/` or `Ctrl+F`).
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
  - Search mode: type to search, `Backspace` delete char, `Enter` accept, `Esc` cancel

## Build

```powershell
cargo build --release
```

Binary path:

- `target\release\ssh-connect.exe`

## Installer (.exe)

This repository includes an Inno Setup script:

- `installer\ssh-connect.iss`

Build installer with:

```powershell
"C:\Program Files (x86)\Inno Setup 6\ISCC.exe" installer\ssh-connect.iss
```

Output installer:

- `dist\ssh-connect-setup.exe`

## License

GPL-3.0. See `LICENSE`.
