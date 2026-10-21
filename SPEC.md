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
- save files (Save / Save As), keeping the file's encoding and line endings
- recognize when an open file is changed by another program:
  reload it (undoable), asking first if the tab has unsaved changes

## Tabs
- tabs to open multiple files at once
- tabs integrated in the title bar (always shown, also with a single tab)
- Ctrl+T / Ctrl+N new tab; Ctrl+W, middle-click or the tab's × closes a tab
- Ctrl+Shift+T reopens the last closed tab (repeatable, also unsaved text)
- Ctrl+Tab / Ctrl+Shift+Tab cycle tabs; Alt+1 … Alt+9 jump to tab 1–9

## Window
- menu hidden; a ≡ button at the right edge of the title bar
  (left of minimize/maximize/close) or Alt opens it as a dropdown
  that closes after an action
- on start, restore the tabs and their content from the last exit,
  including unsaved changes; closing never prompts
- restore window size, position and maximized state
- status bar: line and column, word wrap (click to toggle),
  recognized language

## Editing
- own text control written in Rust (no external editor component)
- multi-cursor: Alt+click adds (or removes) a cursor
- unlimited undo/redo (Ctrl+Z, Ctrl+Y / Ctrl+Shift+Z)
- Tab / Shift+Tab indent and unindent all selected lines
- find (Ctrl+F, F3) and replace (Ctrl+H); the dialog shows the number of
  matches and which one is selected
- Ctrl+mouse wheel changes the font size
- word wrap, toggled with Alt+Z
- line numbers
- highlight of the line(s) with a cursor
- scrollbars only when needed
- automatic syntax highlighting by file type:
  YAML, JSON, CSV, Markdown, INI, XML

## Settings
- Settings opens `settings.ini` in a tab; saving it (Ctrl+S) applies it
- options: font, size, tab width, insert spaces or tabs,
  line numbers on/off, current-line highlight on/off,
  inner margin, word wrap
