# minedit

[![build](https://github.com/SpikePy/minedit/actions/workflows/build.yml/badge.svg)](https://github.com/SpikePy/minedit/actions/workflows/build.yml)

A minimal text editor for Windows 11 that starts instantly. Written in Rust on
the plain Win32 API: one ~400 KB `.exe`, no runtime, no DLLs to install.

**Download** `minedit.exe` from the
[latest release](https://github.com/SpikePy/minedit/releases/latest) and run it.

## Features

- **Tabs in the title bar**, like Windows 11 Notepad or a browser
- **Restores everything** on start: open tabs, unsaved text, window size
  and position; closing never asks to save
- **Multi-cursor editing**: Alt+click adds a cursor
- **Unlimited undo/redo**
- **Syntax highlighting** for YAML, JSON, CSV, Markdown, INI and XML
- **Word wrap**, line numbers, current-line highlight, configurable margin
- **Find and replace** with a match counter
- **Detects files changed by other programs** and reloads them
- Keeps each file's encoding (UTF-8, UTF-8 BOM, UTF-16, ANSI)
  and line endings (CRLF/LF)
- Status bar with line/column, word wrap state and language
- Settings in a plain `settings.ini`, edited in a tab

## Keyboard shortcuts

| Keys | Action |
|---|---|
| Ctrl+T / Ctrl+N | New tab |
| Ctrl+O | Open |
| Ctrl+S / Ctrl+Shift+S | Save / Save As |
| Ctrl+W, middle-click a tab | Close tab |
| Ctrl+Shift+T | Reopen closed tab |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab |
| Alt+1 … Alt+9 | Go to tab 1–9 |
| Alt | Open the menu |
| Alt+click | Add or remove a cursor |
| Esc | Back to a single cursor |
| Ctrl+Z / Ctrl+Y | Undo / redo |
| Tab / Shift+Tab | Indent / unindent selected lines |
| Ctrl+F / F3 / Ctrl+H | Find / find next / replace |
| Alt+Z | Toggle word wrap |
| Ctrl+mouse wheel | Font size |

## Settings

Open **≡ → Settings** to edit `%LOCALAPPDATA%\minedit\settings.ini`;
saving it (Ctrl+S) applies the changes immediately.

```ini
font = Consolas
size = 11
tab_width = 4
use_spaces = false
line_numbers = true
highlight_line = true
margin = 8
word_wrap = false
```

## Building

Requires Rust. On Windows:

```
cargo build --release
```

From WSL or Linux, cross-compile with MinGW
(`sudo apt install gcc-mingw-w64-x86-64`):

```
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

`./install.sh` (from WSL) builds, installs the exe to
`%LOCALAPPDATA%\Programs\minedit`, adds a Start menu shortcut and restarts a
running copy.

Tests run on Windows (from WSL the test exe runs through interop):

```
cargo test --release --target x86_64-pc-windows-gnu
```

## Releases

GitHub Actions tests and builds every push on Windows. Pushing a tag like
`v0.2.0` publishes a GitHub release with `minedit.exe` attached.

## Code

| File | Purpose |
|---|---|
| `src/main.rs` | Window, tabs, files, session, settings, find/replace |
| `src/edit.rs` | The text control: multi-cursor editing, undo, drawing |
| `src/syntax.rs` | Line-by-line syntax highlighting |
| `src/titlebar.rs` | Title bar with tabs and caption buttons |
| `SPEC.md` | Feature specification |
