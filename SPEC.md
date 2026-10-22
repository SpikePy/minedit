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
- the Open dialog (Ctrl+O) opens one or more files as tabs, or with its
  "Select Folder" button the selected (or shown) folder in the file tree;
  folders in a multi-selection open too, further ones in new windows
- Ctrl+P: fuzzy search over open files (file name first, then path) and,
  below a line, recently opened files (count set in the settings, default 20);
  with a folder open in the file tree, a third section below another line
  lists its matching files (name and path relative to the folder; only for a
  search, only if something matches; files and folders named in the folder's
  .gitignore are skipped, plain names only); the folder is searched in the
  background and results appear as they are found
- file tree on the left: "Select Folder" in the Open dialog (or
  Ctrl+K Ctrl+O) shows a folder in it without opening its files; folders
  list their content when expanded
  (subfolders first, hidden and system files left out); a click opens a file
  or opens/closes a folder, Enter does the same, Esc goes back to the editor
- in the tree, files and folders can be renamed (F2 or right-click, typed in
  place), moved (dragged onto a folder, or right-click > Move To...) and
  deleted (Del or right-click; to the Recycle Bin), all through the Windows
  shell with its usual prompts; open tabs follow renamed or moved files; the
  right-click menu also has Open, Copy Path and Show in Explorer
- the active tab's file is revealed in the tree (its folders opened, the
  file highlighted light gray) whenever the active file changes; files
  outside the folder leave nothing highlighted
- File > Recently Opened lists the newest recently opened files and folders
  (as many as the "Number of recent files" setting, newest first; files
  open as tabs, folders in the file tree) and can be cleared
- the sidebar on the left is shown or hidden with the button at the left of
  the title bar; icons at its top switch between the file tree (Files,
  Ctrl+Shift+E, which also hides it again) and Search; without a folder the
  Files view offers "Open Folder..."; the sidebar is resizable by dragging
  its edge, the tree is refreshed when the window is activated, and the
  sidebar (shown, width, view, folder) is restored per window
- one process, many windows: files opened while minedit runs become tabs in
  its frontmost window on the current virtual desktop (a new window if there
  is none there); starting minedit without files while it runs (Start menu,
  middle-click on the taskbar button) opens a new window on the current
  desktop; Ctrl+Shift+N opens a new window
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
  (closing one of several windows asks about its unsaved tabs); File > Exit
  closes all windows without prompting, everything kept in the session
- the session is also saved 2 s after each change, so a crash loses at most
  the last 2 seconds
- restore window size, position and maximized state
- light and dark theme, following the Windows app setting by default
- 29 color schemes (the 20 most popular families, e.g. One Dark, Dracula,
  Monokai, Solarized, Catppuccin); Ctrl+K Ctrl+T picker with live preview
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
- find (Ctrl+F) and replace (Ctrl+H) in a small bar over the top right
  corner of the editor (one per window), starting with the selection; it
  searches as you type and shows "3 of 12" / "12 results" / "No results"
- switches for match case (Aa) and regular expressions (.*; replacements
  can use $1 for groups); up/down buttons, Enter / Shift+Enter and
  F3 / Shift+F3 go to the next / previous match (F3 also with the bar
  closed); Replace replaces the selected match and goes on, All replaces
  every match as one undo step; Esc or x closes the bar
- the sidebar's Search view (Ctrl+Shift+F, Ctrl+Shift+H for the replace
  field; starts with the selection) searches all open tabs (their current
  text) and the open folder's files as you type, in the background
  (.gitignore names, binary files and files over 20 MB skipped)
- switches for match case (Aa), whole word (ab) and regular expressions
  (.*; replacements can use $1 for groups)
- a files field filters with globs, comma-separated: "*.rs, src/**" to
  include, "!*.min.js" to exclude; a pattern without "/" matches any part of
  the path ("target" leaves out that folder)
- below the fields the matches are listed by file, each line under its file;
  a click or Enter shows the match selected in its tab (opening the file)
- Replace All asks first, changes open tabs as one undoable step each and
  other files on disk (keeping their encoding and line endings)
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

- Ctrl+, opens a settings window with dropdowns (font, size, theme, color
  scheme, tab width, margin, recent files) and toggle switches (spaces for
  Tab, word wrap, line numbers, current-line highlight, auto-close brackets);
  changes apply immediately
- values are stored in `settings.ini`, which can still be opened as a tab
