# SPEC

## Platform and build
- written in rust
- for windows 11
- minimal code
- super fast startup time
- artifact with one binary that includes all
  (icon and manifest embedded, static C runtime)
- installed per user to `%LOCALAPPDATA%\Programs\minedit`
  with a Start menu shortcut (`./install.sh`); a running copy is closed
  (session saved) and restarted with the new version
- app icon resembling a minimalistic editor
- GitHub Actions tests and builds every push; a `v*` tag creates a release

## Files
- open files (dialog, multi-select, drag and drop, command line)
- Ctrl+P: fuzzy search over open files (file name first, then path) and,
  below a line, recently opened files (count set in the settings, default 20)
- one window: files opened while minedit runs become tabs in that window
- Explorer integration: "Open with" for every file type and an
  "Edit with minedit" entry in the right-click menu
- save files (Save / Save As), keeping the file's encoding and line endings
- recognize when an open file is changed by another program:
  reload it (undoable), asking first if the tab has unsaved changes

## Tabs
- tabs to open multiple files at once
- tabs integrated in the title bar (always shown, also with a single tab)
- Ctrl+T / Ctrl+N new tab; Ctrl+W, middle-click or the tab's × closes a tab
- Ctrl+Shift+T reopens the last closed tab (repeatable, also unsaved text)
- drag tabs to reorder them; drag a tab out of the window to open it in a new
  window, or onto another window to move it there; a window whose last tab
  leaves closes
- the tab bar scrolls (mouse wheel) when there are too many tabs
- right-click a tab: close, close others, close tabs to the right, move to new
  window, copy path, show in Explorer
- Ctrl+Tab / Ctrl+Shift+Tab cycle tabs; Alt+1 … Alt+9 jump to tab 1–9

## Window
- menu hidden; a ≡ button at the right edge of the title bar
  (left of minimize/maximize/close) or Alt opens it as a dropdown
  that closes after an action
- on start, restore all windows with their tabs and content from the last
  exit, including unsaved changes; closing the last window never prompts
  (closing one of several windows asks about its unsaved tabs)
- the session is also saved 2 s after each change, so a crash loses at most
  the last 2 seconds
- restore window size, position and maximized state
- light and dark theme, following the Windows app setting by default
  (title bar, text, status bar, scrollbars, menus)
- status bar: line and column (click: go to line), word wrap (click to
  toggle), encoding and line endings (click to change for saving),
  recognized language (click to choose another one or auto detect)

## Editing
- own text control written in Rust (no external editor component)
- multi-cursor: Alt+click adds (or removes) a cursor; Alt+drag selects a
  column (one cursor per line); Ctrl+D selects the word, then each next match
- Alt+Shift+Up/Down adds a cursor above the topmost / below the bottommost
  cursor (same column)
- line commands: duplicate (Ctrl+Shift+D), delete (Ctrl+Shift+K),
  move up/down (Alt+Up/Down)
- brackets and quotes close automatically (setting); the bracket matching the
  one at the cursor is highlighted
- go to line (Ctrl+G or click "Ln, Col" in the status bar)
- unlimited undo/redo (Ctrl+Z, Ctrl+Y / Ctrl+Shift+Z)
- Tab / Shift+Tab indent and unindent all selected lines
- find (Ctrl+F, F3) and replace (Ctrl+H); the dialog shows the number of
  matches and which one is selected
- Ctrl+mouse wheel changes the font size, Ctrl+0 resets it
- word wrap, toggled with Alt+Z
- Ctrl+/ shows an overview of all keyboard shortcuts, in groups
- line numbers
- highlight of the line(s) with a cursor
- scrollbars only when needed
- editing stays fast in large files (no full rescan per keystroke)
- Markdown: Ctrl+B / Ctrl+I put ** / * around the selection (again: remove);
  bold, italic and headings are shown in bold / italic
- automatic syntax highlighting by file type:
  YAML, JSON, CSV, Markdown, INI, TOML, XML, Bash, PowerShell, Python,
  JavaScript/TypeScript

## Settings
- Settings opens `settings.ini` in a tab; saving it (Ctrl+S) applies it
- options: font, size, tab width, insert spaces or tabs,
  line numbers on/off, current-line highlight on/off,
  inner margin, word wrap, auto-close brackets, theme (system/light/dark),
  number of recent files
