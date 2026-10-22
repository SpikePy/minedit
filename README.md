# minedit

[![build](https://github.com/SpikePy/minedit/actions/workflows/build.yml/badge.svg)](https://github.com/SpikePy/minedit/actions/workflows/build.yml)

A minimal text editor for Windows 11 that starts instantly. Written in Rust on
the plain Win32 API: one ~750 KB `.exe`, no runtime, no DLLs to install.

**Download** `minedit.exe` from the
[latest release](https://github.com/SpikePy/minedit/releases/latest) and run it.

## Features

- **Tabs in the title bar**, like Windows 11 Notepad or a browser: drag to
  reorder, drag out into a new window or onto another window, right-click for
  more; the tab bar scrolls when it is full
- **Restores everything** on start: all windows, open tabs, unsaved text,
  window sizes and positions; closing never asks to save
- **Multi-cursor editing**: Alt+click adds a cursor, Alt+drag selects a
  column, Ctrl+D adds the next match
- Line commands (duplicate, delete, move), auto-closing brackets and quotes,
  matching-bracket highlight, go to line
- **Unlimited undo/redo**
- **Syntax highlighting** for YAML, JSON, CSV, Markdown, INI, TOML, XML,
  Bash, PowerShell, Python and JavaScript/TypeScript
- **Word wrap**, line numbers, current-line highlight, configurable margin
- **Light and dark theme**, following Windows by default, with 29 color
  schemes from the most popular families (One Dark, Dracula, GitHub, Monokai,
  Solarized, Material, Catppuccin, Tokyo Night, Gruvbox, Nord, Ayu, Darcula,
  Rosé Pine, Kanagawa, …) — pick with live preview via Ctrl+K Ctrl+T
- **Find and replace** bar over the editor: match case, regex, up/down,
  match counter
- **Detects files changed by other programs** and reloads them
- Keeps each file's encoding (UTF-8, UTF-8 BOM, UTF-16, ANSI)
  and line endings (CRLF/LF)
- Status bar with line/column, word wrap, encoding, line endings and
  language — click any of them to change it
- **Sidebar** (button at the left of the title bar) with two views:
  - **Files**: "Select Folder" in the Open dialog (or Ctrl+K Ctrl+O) shows a
    folder without opening its files; rename (F2), move (drag) and delete
    (Del) files; the active tab's file is highlighted, and Ctrl+P also finds
    the folder's files
  - **Search** (Ctrl+Shift+F / Ctrl+Shift+H): find and replace in open tabs
    and the folder, with match case, whole word, regular expressions and
    file filters like `*.rs, !target`; matches listed by file and line
- **Recently Opened** in the File menu: recent files and folders
- **Multiple windows**: Ctrl+Shift+N or middle-click on the taskbar button
  opens a new window on the current virtual desktop; files opened from
  Explorer become tabs in the frontmost window on that desktop. "Open with"
  and "Edit with minedit" entries are added by the installer
- Settings in a plain `settings.ini`, edited in a tab

## Keyboard shortcuts

Press **Ctrl+/** in minedit for an overview of all shortcuts.

| Keys | Action |
|---|---|
| Ctrl+T / Ctrl+N | New tab |
| Ctrl+O | Open files, or a folder with "Select Folder" |
| Ctrl+P | Quick open: fuzzy search open, recent and folder files |
| Ctrl+K Ctrl+O | Open folder in the file tree |
| Ctrl+Shift+E | Sidebar: files (again: hide) |
| Ctrl+S / Ctrl+Shift+S | Save / Save As |
| Ctrl+W, middle-click a tab | Close tab |
| Ctrl+Shift+T | Reopen closed tab |
| Ctrl+Shift+N | New window |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab |
| Alt+1 … Alt+9 | Go to tab 1–9 |
| Alt | Open the menu |
| Alt+click / Alt+drag | Add or remove a cursor / column selection |
| Alt+Shift+↑ / Alt+Shift+↓ | Add a cursor above / below |
| Ctrl+D | Select word, then add the next match |
| Ctrl+Shift+D / Ctrl+Shift+K | Duplicate / delete line |
| Alt+↑ / Alt+↓ | Move line up / down |
| Ctrl+G | Go to line |
| Ctrl+B / Ctrl+I | Bold / italic (Markdown) |
| Esc | Back to a single cursor |
| Ctrl+Z / Ctrl+Y | Undo / redo |
| Tab / Shift+Tab | Indent / unindent selected lines |
| Ctrl+F / Ctrl+H | Find / replace bar |
| F3 / Shift+F3 | Next / previous match |
| Ctrl+Shift+F / Ctrl+Shift+H | Sidebar: search / replace in tabs and folder |
| Alt+Z | Toggle word wrap |
| Ctrl+K Ctrl+T | Color scheme (live preview) |
| Ctrl+, | Settings |
| Ctrl+mouse wheel / Ctrl+0 | Font size / reset font size |

## Settings

Press **Ctrl+,** (or **≡ → Settings**) for the settings window: dropdowns for
font, size, theme, color scheme, tab width, margin and the number of recent
files, and switches for spaces instead of tabs, word wrap, line numbers,
current-line highlight and auto-closing brackets. Changes apply immediately.

They are stored in `%LOCALAPPDATA%\minedit\settings.ini`, which **Open settings
file** shows in a tab for editing by hand.

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
| `src/tree.rs` | File tree |
| `src/dialog.rs` | Open and folder dialogs |
| `src/search.rs` | Sidebar search: regex, file filter, results |
| `src/main.rs` | Window, tabs, files, session, settings, find/replace |
| `src/edit.rs` | The text control: multi-cursor editing, undo, drawing |
| `src/syntax.rs` | Line-by-line syntax highlighting |
| `src/titlebar.rs` | Title bar with tabs and caption buttons |
| `SPEC.md` | Feature specification |
