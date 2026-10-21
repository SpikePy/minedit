#![cfg_attr(not(test), windows_subsystem = "windows")]

mod edit;
mod help;
mod quick;
mod syntax;
mod theme;
mod titlebar;

use edit::ed;
use titlebar::Hit;
use std::{cell::RefCell, mem::zeroed, path::PathBuf, ptr::{null, null_mut}, sync::atomic::{AtomicU32, Ordering::Relaxed}, time::SystemTime};
use windows_sys::{
    w,
    Win32::{
        Foundation::*,
        Globalization::*,
        Graphics::Gdi::*,
        Graphics::Dwm::DwmSetWindowAttribute,
        System::{
            DataExchange::COPYDATASTRUCT,
            LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW},
            Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD},
            Threading::*,
        },
        UI::{
            Controls::{Dialogs::*, *},
            HiDpi::*,
            Input::KeyboardAndMouse::*,
            Shell::*,
            WindowsAndMessaging::*,
        },
    },
};

const APP: &str = "minedit";
const ID_NEW: u16 = 1;
const ID_OPEN: u16 = 2;
const ID_SAVE: u16 = 3;
const ID_SAVEAS: u16 = 4;
const ID_CLOSE: u16 = 5;
const ID_NEXT: u16 = 6;
const ID_PREV: u16 = 7;
const ID_SELALL: u16 = 8;
const ID_EXIT: u16 = 9;
const ID_FIND: u16 = 10;
const ID_FINDNEXT: u16 = 11;
const ID_REPLACE: u16 = 12;
const ID_SETTINGS: u16 = 13;
const ID_WRAP: u16 = 14;
const ID_REOPEN: u16 = 15;
const ID_GOTO: u16 = 16;
const ID_SELNEXT: u16 = 17;
const ID_DUPLINE: u16 = 18;
const ID_DELLINE: u16 = 19;
const ID_MOVEUP: u16 = 20;
const ID_MOVEDOWN: u16 = 21;
const ID_ZOOMRESET: u16 = 22;
const ID_HELP: u16 = 23;
const ID_QUICKOPEN: u16 = 24;
const MAX_CLOSED: usize = 50;
const TIMER_FILES: usize = 1; // look for files changed by other programs
const TIMER_SESSION: usize = 2; // save the session shortly after a change
/// WM_COPYDATA tag for "open these files" sent by a second launch.
const COPYDATA_OPEN: usize = 0x6F70656E;
/// Posted to itself to open the files in `incoming`.
const WM_OPEN_INCOMING: u32 = WM_APP + 2;
/// Posted to a window whose last tab was dragged away: it closes.
const WM_CLOSE_EMPTY: u32 = WM_APP + 3;
const ID_TAB1: u16 = 100; // Alt+1..9 map to ID_TAB1..ID_TAB1+8

const DEFAULT_SETTINGS: &str = "\
# minedit settings - save (Ctrl+S) to apply
font = Consolas
size = 11
tab_width = 4
use_spaces = false
line_numbers = true
highlight_line = true
margin = 8
word_wrap = false
auto_close = true
# system, light or dark
theme = system
# number of recently opened files offered by Ctrl+P
recent_files = 20
";

#[derive(Clone, Copy, PartialEq)]
enum Enc { Utf8, Utf8Bom, Utf16, Ansi }

const ENCODINGS: [(Enc, &str); 4] = [(Enc::Utf8, "UTF-8"), (Enc::Utf8Bom, "UTF-8 with BOM"), (Enc::Utf16, "UTF-16 LE"), (Enc::Ansi, "ANSI")];

/// Clickable status bar items.
#[derive(Clone, Copy, PartialEq)]
enum Status { Pos, Lang, Eol, Enc, Wrap }

struct Tab {
    edit: HWND,
    path: Option<PathBuf>,
    dirty: bool, // as last shown in the tab label
    enc: Enc,
    lf: bool, // file used bare LF line endings
    stamp: Option<SystemTime>, // modification time of the file as last loaded or saved
    lang: Option<syntax::Lang>, // language chosen in the status bar; None = detect from the extension
}

/// A closed tab that Ctrl+Shift+T can bring back.
struct Closed {
    path: Option<PathBuf>,
    text: Vec<u16>, // kept only for unsaved tabs; clean files are reloaded from disk
    enc: Enc,
    lf: bool,
    dirty: bool,
    caret: usize,
    lang: Option<syntax::Lang>,
}

/// State of one window. The window handling a message is swapped into `State::win` (see `enter`),
/// so most code simply works on "the current window".
struct Win {
    main: HWND,
    tabs: Vec<Tab>,
    cur: usize,
    hover: Hit,   // title bar element under the mouse
    pressed: Hit, // caption button being clicked
    active: bool, // window is active
    status_items: Vec<(RECT, Status)>, // clickable parts of the status bar, in status bar coordinates
    place: Option<(RECT, bool)>, // restored window rect and maximized state
    drag: Option<Drag>, // a tab being dragged
    scroll: i32,        // tab bar scroll position in pixels
}

impl Win {
    const fn new(main: HWND) -> Win {
        Win { main, tabs: Vec::new(), cur: 0, hover: Hit::None, pressed: Hit::None, active: false, status_items: Vec::new(), place: None, drag: None, scroll: 0 }
    }
}

#[derive(Clone, Copy)]
struct Drag {
    tab: usize,
    from: POINT, // where the button went down, client coordinates
    moved: bool,
}

struct State {
    win: Win,         // the current window
    parked: Vec<Win>, // all other windows
    menu: HMENU, // popup opened by the ≡ button and Alt
    efont: HFONT,
    ufont: HFONT,
    ifont: HFONT,  // caption button glyphs
    ifont_small: HFONT,
    checking: bool, // a file-changed prompt is open
    goto: Option<(usize, usize)>, // line and column entered in the Go to Line dialog
    closed: Vec<Closed>, // most recently closed last
    incoming: Vec<PathBuf>, // files handed over by another launch, opened asynchronously
    later: Vec<(HWND, Option<(RECT, bool)>)>, // restored extra windows, shown after the main one
    recent: Vec<PathBuf>, // recently opened files, newest first
    max_recent: usize,
    quick: Vec<Pick>,     // what each Ctrl+P list entry opens
    dpi: u32,
    face: String,
    theme: String, // "system", "light" or "dark"
    size: i32,  // font size from the settings file
    pt: i32,    // current font size, changed by Ctrl+wheel
    wheel: i32, // accumulated Ctrl+wheel delta, for high-resolution wheels and touchpads
    opts: edit::Options,
    fdlg: HWND,                  // open Find/Replace dialog
    fdlg_title: String,          // its original (localized) title, before the match count
    fr: *mut FINDREPLACEW,       // lives for the whole process, as the modeless dialog requires
}

thread_local! {
    static S: RefCell<State> = const { RefCell::new(State {
        win: Win::new(null_mut()), parked: Vec::new(), menu: null_mut(), efont: null_mut(), ufont: null_mut(), ifont: null_mut(), ifont_small: null_mut(),
        checking: false, goto: None, closed: Vec::new(), incoming: Vec::new(), later: Vec::new(), recent: Vec::new(), max_recent: 20, quick: Vec::new(),
        dpi: 96, face: String::new(), theme: String::new(), size: 11, pt: 11, wheel: 0, opts: edit::Options { tab_width: 4, use_spaces: false, line_numbers: true, highlight_line: true, margin: 8, word_wrap: false, auto_close: true },
        fdlg: null_mut(), fdlg_title: String::new(), fr: null_mut(),
    }) };
}

/// Registered message the Find/Replace dialog sends to the main window.
static FINDMSG: AtomicU32 = AtomicU32::new(0);

// Never call anything that can re-enter the window procedure inside `with`.
fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    S.with(|s| f(&mut s.borrow_mut()))
}

fn try_with<R>(f: impl FnOnce(&mut State) -> Option<R>) -> Option<R> {
    S.with(|s| s.try_borrow_mut().ok().and_then(|mut s| f(&mut s)))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn name(t: &Tab) -> String {
    t.path.as_ref().and_then(|p| p.file_name()).map_or("Untitled".into(), |n| n.to_string_lossy().into())
}

fn decode(b: &[u8]) -> (Vec<u16>, Enc) {
    if let Some(r) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        (String::from_utf8_lossy(r).encode_utf16().collect(), Enc::Utf8Bom)
    } else if let Some(r) = b.strip_prefix(&[0xFF, 0xFE]) {
        (r.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect(), Enc::Utf16)
    } else if let Ok(s) = std::str::from_utf8(b) {
        (s.encode_utf16().collect(), Enc::Utf8)
    } else {
        unsafe {
            let n = MultiByteToWideChar(CP_ACP, 0, b.as_ptr(), b.len() as i32, null_mut(), 0);
            let mut out = vec![0u16; n as usize];
            MultiByteToWideChar(CP_ACP, 0, b.as_ptr(), b.len() as i32, out.as_mut_ptr(), n);
            (out, Enc::Ansi)
        }
    }
}

fn encode(t: &[u16], enc: Enc) -> Vec<u8> {
    match enc {
        Enc::Utf8 => String::from_utf16_lossy(t).into_bytes(),
        Enc::Utf8Bom => [&[0xEF, 0xBB, 0xBF][..], String::from_utf16_lossy(t).as_bytes()].concat(),
        Enc::Utf16 => [0xFF, 0xFE].into_iter().chain(t.iter().flat_map(|c| c.to_le_bytes())).collect(),
        Enc::Ansi => unsafe {
            let n = WideCharToMultiByte(CP_ACP, 0, t.as_ptr(), t.len() as i32, null_mut(), 0, null(), null_mut());
            let mut out = vec![0u8; n as usize];
            WideCharToMultiByte(CP_ACP, 0, t.as_ptr(), t.len() as i32, out.as_mut_ptr(), n, null(), null_mut());
            out
        },
    }
}

/// Reads a file into LF text, its encoding and whether it used bare LF line endings.
fn load(p: &PathBuf) -> std::io::Result<(Vec<u16>, Enc, bool)> {
    let (text, enc) = decode(&std::fs::read(p)?);
    let lf = text.iter().position(|&c| c == b'\n' as u16).is_some_and(|i| i == 0 || text[i - 1] != b'\r' as u16);
    Ok((edit::to_lf(&text), enc, lf))
}

fn app_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join(APP))
}

fn settings_path() -> Option<PathBuf> {
    Some(app_dir()?.join("settings.ini"))
}

fn yes(v: &str) -> bool {
    matches!(v, "true" | "yes" | "on" | "1")
}

/// Reads settings.ini ("key = value" lines); missing or invalid values keep their defaults.
fn load_settings() {
    let text = settings_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    with(|s| {
        (s.face, s.size, s.theme) = ("Consolas".into(), 11, "system".into());
        for line in DEFAULT_SETTINGS.lines().chain(text.lines()) {
            let Some((k, v)) = line.split_once('=').filter(|_| !line.trim_start().starts_with('#')) else { continue };
            match (k.trim(), v.trim()) {
                ("font", v) if !v.is_empty() => s.face = v.into(),
                ("size", v) => s.size = v.parse().map_or(s.size, |n: i32| n.clamp(6, 72)),
                ("tab_width", v) => s.opts.tab_width = v.parse().map_or(s.opts.tab_width, |n: usize| n.clamp(1, 16)),
                ("use_spaces", v) => s.opts.use_spaces = yes(v),
                ("line_numbers", v) => s.opts.line_numbers = yes(v),
                ("highlight_line", v) => s.opts.highlight_line = yes(v),
                ("word_wrap", v) => s.opts.word_wrap = yes(v),
                ("auto_close", v) => s.opts.auto_close = yes(v),
                ("theme", v) => s.theme = v.to_ascii_lowercase(),
                ("recent_files", v) => s.max_recent = v.parse().map_or(s.max_recent, |n: usize| n.min(500)),
                ("margin", v) => s.opts.margin = v.parse().map_or(s.opts.margin, |n: i32| n.clamp(0, 100)),
                _ => {}
            }
        }
        s.pt = s.size;
    });
}

/// Opens settings.ini in a tab, first adding any options it does not list yet.
unsafe fn open_settings() {
    let Some(p) = settings_path() else { return };
    let mut text = std::fs::read_to_string(&p).unwrap_or_default();
    let key = |l: &str| l.split_once('=').map(|(k, _)| k.trim().to_string()).filter(|_| !l.trim_start().starts_with('#'));
    let have: Vec<String> = text.lines().filter_map(key).collect();
    let missing: Vec<&str> = DEFAULT_SETTINGS.lines().filter(|l| key(l).is_some_and(|k| !have.contains(&k))).collect();
    let new = text.is_empty();
    if new {
        text = DEFAULT_SETTINGS.into();
    } else if !missing.is_empty() {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        missing.iter().for_each(|l| text.extend([l, "\n"]));
    }
    if new || !missing.is_empty() {
        _ = std::fs::create_dir_all(p.parent().unwrap());
        _ = std::fs::write(&p, &text);
    }
    open_path(p);
}

unsafe fn font(face: &str, pt: i32, dpi: u32) -> HFONT {
    CreateFontW(-(pt * dpi as i32 / 72), 0, 0, 0, FW_NORMAL as _, 0, 0, 0, DEFAULT_CHARSET as _,
        OUT_DEFAULT_PRECIS as _, CLIP_DEFAULT_PRECIS as _, CLEARTYPE_QUALITY as _, 0, wide(face).as_ptr())
}

/// Font for the caption glyphs, `px` pixels high at 96 DPI.
unsafe fn icon_font(px: i32, dpi: u32) -> HFONT {
    for face in ["Segoe Fluent Icons", "Segoe MDL2 Assets"] {
        let f = CreateFontW(-(px * dpi as i32 / 96), 0, 0, 0, FW_NORMAL as _, 0, 0, 0, DEFAULT_CHARSET as _,
            OUT_DEFAULT_PRECIS as _, CLIP_DEFAULT_PRECIS as _, ANTIALIASED_QUALITY as _, 0, wide(face).as_ptr());
        // Windows substitutes a missing font silently; check what was actually picked.
        let dc = CreateCompatibleDC(null_mut());
        SelectObject(dc, f);
        let mut got = [0u16; 32];
        let n = GetTextFaceW(dc, 32, got.as_mut_ptr());
        DeleteDC(dc);
        if String::from_utf16_lossy(&got[..(n.max(1) - 1) as usize]) == face {
            return f;
        }
        DeleteObject(f);
    }
    font("Segoe UI Symbol", 8, dpi)
}

unsafe fn set_fonts() {
    let (old, edits) = with(|s| {
        let old = [s.efont, s.ufont, s.ifont, s.ifont_small];
        s.efont = font(&s.face, s.pt, s.dpi);
        s.ufont = font("Segoe UI", 9, s.dpi);
        (s.ifont, s.ifont_small) = (icon_font(10, s.dpi), icon_font(8, s.dpi));
        edit::set_view(s.efont, &edit::Options { margin: s.opts.margin * s.dpi as i32 / 96, ..s.opts });
        (old, all_edits(s))
    });
    for e in edits {
        PostMessageW(e, edit::UPDATE, 0, 0);
    }
    for f in old.into_iter().filter(|f| !f.is_null()) {
        DeleteObject(f);
    }
    apply_theme();
    layout();
}

/// The setting name sent with WM_SETTINGCHANGE, read only up to its terminator.
unsafe fn setting_name(lp: LPARAM) -> String {
    let p = lp as *const u16;
    String::from_utf16_lossy(&(0..64).map(|i| *p.add(i)).take_while(|&c| c != 0).collect::<Vec<_>>())
}

fn theme_is_dark(setting: &str, system_dark: bool) -> bool {
    match setting {
        "dark" => true,
        "light" => false,
        _ => system_dark,
    }
}

/// Windows' "app mode" setting (Settings > Personalization > Colors).
unsafe fn system_dark() -> bool {
    let (mut light, mut size) = (1u32, 4u32);
    let key = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    RegGetValueW(HKEY_CURRENT_USER, key, w!("AppsUseLightTheme"), RRF_RT_REG_DWORD, null_mut(), &mut light as *mut _ as _, &mut size);
    light == 0
}

/// Applies the light or dark palette to everything, including the parts Windows draws.
unsafe fn apply_theme() {
    let (windows, setting, edits) = with(|s| (all_windows(s), s.theme.clone(), all_edits(s)));
    let dark = theme_is_dark(&setting, system_dark());
    theme::set_dark(dark);
    let flag = dark as i32;
    for &w in &windows {
        DwmSetWindowAttribute(w, 20 /* DWMWA_USE_IMMERSIVE_DARK_MODE */, &flag as *const _ as _, 4);
        InvalidateRect(w, null(), 0);
    }
    dark_menus(dark);
    for e in edits {
        theme_editor(e);
        InvalidateRect(e, null(), 0);
    }
}

fn all_windows(s: &State) -> Vec<HWND> {
    std::iter::once(&s.win).chain(&s.parked).map(|w| w.main).filter(|m| !m.is_null()).collect()
}

fn all_edits(s: &State) -> Vec<HWND> {
    std::iter::once(&s.win).chain(&s.parked).flat_map(|w| w.tabs.iter().map(|t| t.edit)).collect()
}

/// Makes window `h` the current one (see `Win`); returns the previous current window.
fn enter(h: HWND) -> HWND {
    with(|s| {
        let prev = s.win.main;
        if prev != h {
            let w = match s.parked.iter().position(|w| w.main == h) {
                Some(i) => s.parked.swap_remove(i),
                None => Win::new(h), // a window being created
            };
            let old = std::mem::replace(&mut s.win, w);
            if !old.main.is_null() {
                s.parked.push(old);
            }
        }
        prev
    })
}

/// Makes `prev` (from `enter`) current again, if it still exists.
fn leave(prev: HWND) {
    with(|s| {
        if s.win.main != prev {
            if let Some(i) = s.parked.iter().position(|w| w.main == prev) {
                let w = s.parked.swap_remove(i);
                let old = std::mem::replace(&mut s.win, w);
                s.parked.push(old);
            }
        }
    })
}

unsafe fn create_window() -> HWND {
    CreateWindowExW(0, wide(&instance_name()).as_ptr(), wide(APP).as_ptr(), WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
        CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, null_mut(), null_mut(), GetModuleHandleW(null()), null())
}

/// Shows a window at its saved position.
unsafe fn show_placed(h: HWND, place: Option<(RECT, bool)>, show: SHOW_WINDOW_CMD) {
    match place {
        Some((rect, max)) => {
            let minimized = matches!(show, SW_MINIMIZE | SW_SHOWMINIMIZED | SW_SHOWMINNOACTIVE);
            let mut wp: WINDOWPLACEMENT = zeroed();
            wp.length = size_of::<WINDOWPLACEMENT>() as u32;
            wp.rcNormalPosition = rect;
            wp.showCmd = (if max && !minimized { SW_SHOWMAXIMIZED } else { show }) as _;
            wp.flags = if max { WPF_RESTORETOMAXIMIZED } else { 0 };
            SetWindowPlacement(h, &wp);
        }
        None => _ = ShowWindow(h, show),
    }
}

/// Moves tab `i` of the current window to window `target`, at the tab under screen point `at`
/// (or at the end). The source window closes when it has no tabs left.
unsafe fn move_tab(i: usize, target: HWND, at: Option<POINT>) {
    let (src, tab, next, empty) = with(|s| {
        let w = &mut s.win;
        let t = w.tabs.remove(i);
        w.cur = if i < w.cur { w.cur - 1 } else if i == w.cur { i.min(w.tabs.len().saturating_sub(1)) } else { w.cur };
        (w.main, t, w.cur, w.tabs.is_empty())
    });
    if !empty {
        select(next);
    }
    SetParent(tab.edit, target);
    let prev = enter(target);
    let at = at.map(|mut pt| {
        ScreenToClient(target, &mut pt);
        pt
    });
    let idx = match at.map(|pt| titlebar::hit(&bar_rects(), pt.x, pt.y)) {
        Some(Hit::Tab(j) | Hit::TabClose(j)) => j,
        _ => with(|s| s.win.tabs.len()),
    };
    with(|s| s.win.tabs.insert(idx, tab));
    select(idx);
    SetForegroundWindow(target);
    leave(prev);
    if empty {
        PostMessageW(src, WM_CLOSE_EMPTY, 0, 0);
    } else {
        repaint_bar();
    }
    schedule_save();
}

/// Tab `i` dropped at screen point `pt`: onto another window, or outside all windows into a new one.
unsafe fn drop_tab(i: usize, pt: POINT) {
    let target = GetAncestor(WindowFromPoint(pt), GA_ROOT);
    let (me, ours, single) = with(|s| (s.win.main, s.parked.iter().any(|w| w.main == target), s.win.tabs.len() == 1));
    if ours {
        move_tab(i, target, Some(pt));
    } else if target != me && !single {
        tear_off(i, pt);
    }
}

/// Moves tab `i` into a new window placed under screen point `pt`.
unsafe fn tear_off(i: usize, pt: POINT) {
    let mut rc: RECT = zeroed();
    GetWindowRect(with(|s| s.win.main), &mut rc);
    let new = create_window();
    SetWindowPos(new, null_mut(), pt.x - 120, pt.y - 16, rc.right - rc.left, rc.bottom - rc.top, SWP_NOZORDER | SWP_NOACTIVATE);
    move_tab(i, new, None);
    ShowWindow(new, SW_SHOW);
}

/// Scrolls the tab bar so tab `i` is fully visible.
unsafe fn scroll_to_tab(i: usize) {
    let (main, dpi, n, scroll) = with(|s| (s.win.main, s.dpi, s.win.tabs.len(), s.win.scroll));
    let mut rc: RECT = zeroed();
    GetClientRect(main, &mut rc);
    let r = titlebar::layout(rc.right, dpi, n, 0);
    let Some(t) = r.tabs.get(i) else { return };
    let width = r.clip.right - r.clip.left;
    let scroll = scroll.clamp(t.right - r.clip.left - width, t.left - r.clip.left).clamp(0, r.max_scroll);
    with(|s| s.win.scroll = scroll);
}

/// Right-click menu of a tab.
unsafe fn tab_menu(i: usize, pt: POINT) {
    let (main, path, n) = with(|s| (s.win.main, s.win.tabs[i].path.clone(), s.win.tabs.len()));
    let menu = CreatePopupMenu();
    let gray = |on: bool| if on { MF_ENABLED } else { MF_GRAYED };
    AppendMenuW(menu, MF_STRING, 1, w!("&Close\tCtrl+W"));
    AppendMenuW(menu, MF_STRING | gray(n > 1), 2, w!("Close &Others"));
    AppendMenuW(menu, MF_STRING | gray(i + 1 < n), 3, w!("Close Tabs to the &Right"));
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    AppendMenuW(menu, MF_STRING | gray(n > 1), 4, w!("Move to &New Window"));
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    AppendMenuW(menu, MF_STRING | gray(path.is_some()), 5, w!("Copy &Path"));
    AppendMenuW(menu, MF_STRING | gray(path.is_some()), 6, w!("Show in &Explorer"));
    let cmd = TrackPopupMenuEx(menu, TPM_RETURNCMD, pt.x, pt.y, main, null());
    DestroyMenu(menu);
    let keep = with(|s| s.win.tabs[i].edit);
    // Closes from the end; `close_tab` may be cancelled for unsaved tabs.
    let close_where = |f: &dyn Fn(usize, usize) -> bool| {
        let pos = |e| with(|s| s.win.tabs.iter().position(|t| t.edit == e));
        for j in (0..with(|s| s.win.tabs.len())).rev() {
            if let Some(k) = pos(keep) {
                if f(j, k) {
                    close_tab(j);
                }
            }
        }
    };
    match cmd {
        1 => close_tab(i),
        2 => close_where(&|j, k| j != k),
        3 => close_where(&|j, k| j > k),
        4 => tear_off(i, pt),
        5 => _ = edit::set_clipboard(main, &path.unwrap().to_string_lossy().encode_utf16().collect::<Vec<_>>()),
        6 => {
            let args = wide(&format!("/select,\"{}\"", path.unwrap().display()));
            ShellExecuteW(null_mut(), w!("open"), w!("explorer.exe"), args.as_ptr(), null(), SW_SHOWNORMAL);
        }
        _ => {}
    }
}

/// Closes all tabs of the current window (asking about unsaved ones); false if cancelled.
unsafe fn close_window_tabs() -> bool {
    let n = with(|s| s.win.tabs.len());
    if !(0..n).all(|i| confirm(i)) {
        return false;
    }
    with(|s| {
        for t in std::mem::take(&mut s.win.tabs) {
            remember_closed(s, t);
        }
    });
    true
}

/// Adds a closed tab to the Ctrl+Shift+T list.
fn remember_closed(s: &mut State, t: Tab) {
    if let Some(p) = &t.path {
        add_recent(s, p);
    }
    let e = unsafe { ed(t.edit) };
    if t.path.is_some() || !e.text.is_empty() {
        let text = if e.dirty() { e.text.clone() } else { Vec::new() };
        s.closed.push(Closed { path: t.path, text, enc: t.enc, lf: t.lf, dirty: e.dirty(), caret: e.sel().start, lang: t.lang });
        if s.closed.len() > MAX_CLOSED {
            s.closed.remove(0);
        }
    }
}

/// Scrollbars follow the theme through the Explorer visual styles.
unsafe fn theme_editor(e: HWND) {
    SetWindowTheme(e, if theme::is_dark() { w!("DarkMode_Explorer") } else { w!("Explorer") }, null());
}

/// Popup menus: uxtheme's SetPreferredAppMode (ordinal 135) and FlushMenuThemes (136).
/// Undocumented, but what Windows' own apps use; skipped if missing.
unsafe fn dark_menus(dark: bool) {
    let ux = LoadLibraryW(w!("uxtheme.dll"));
    if ux.is_null() {
        return;
    }
    if let Some(f) = GetProcAddress(ux, 135usize as *const u8) {
        let set_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(f);
        set_mode(if dark { 2 } else { 3 }); // ForceDark / ForceLight
    }
    if let Some(f) = GetProcAddress(ux, 136usize as *const u8) {
        let flush: unsafe extern "system" fn() = std::mem::transmute(f);
        flush();
    }
}

/// Ctrl+wheel: one font point per wheel notch.
unsafe fn zoom(delta: i32) {
    let changed = with(|s| {
        s.wheel += delta;
        let steps = s.wheel / WHEEL_DELTA as i32;
        s.wheel %= WHEEL_DELTA as i32;
        let pt = (s.pt + steps).clamp(6, 72);
        std::mem::replace(&mut s.pt, pt) != pt
    });
    if changed {
        set_fonts();
        schedule_save();
    }
}

fn cur_edit() -> HWND {
    with(|s| s.win.tabs[s.win.cur].edit)
}

/// Start positions of all matches of `what` in `text`, honoring match-case and whole-word flags.
unsafe fn hits(text: &[u16], what: &[u16], flags: u32) -> Vec<usize> {
    let fold = |v: &[u16]| {
        let mut v = v.to_vec();
        if flags & FR_MATCHCASE == 0 && !v.is_empty() {
            CharLowerBuffW(v.as_mut_ptr(), v.len() as u32);
        }
        v
    };
    let (t, w) = (fold(text), fold(what));
    let word = |c: Option<&u16>| c.is_some_and(|&c| IsCharAlphaNumericW(c) != 0 || c == b'_' as u16);
    (0..t.len())
        .filter(|&i| t[i..].starts_with(&w))
        .filter(|&i| flags & FR_WHOLEWORD == 0 || !(word(i.checked_sub(1).and_then(|j| t.get(j))) || word(t.get(i + w.len()))))
        .collect()
}

unsafe fn not_found(what: &[u16]) {
    let owner = with(|s| if s.fdlg.is_null() { s.win.main } else { s.fdlg });
    let msg = wide(&format!("Cannot find \"{}\"", String::from_utf16_lossy(what)));
    MessageBoxW(owner, msg.as_ptr(), wide(APP).as_ptr(), MB_ICONINFORMATION);
}

/// Selects the next match after the selection (or before it, searching up), wrapping around.
unsafe fn find_next(edit: HWND, what: &[u16], flags: u32) {
    let e = ed(edit);
    let hits = hits(&e.text, what, flags);
    let sel = e.sel();
    let found = if flags & FR_DOWN != 0 {
        hits.iter().find(|&&i| i >= sel.end).or(hits.first())
    } else {
        hits.iter().rev().find(|&&i| i < sel.start).or(hits.last())
    };
    match found {
        Some(&i) => e.select(i, i + what.len()),
        None => not_found(what),
    }
}

unsafe fn find_dialog(replace: bool) {
    let (old, fr, edit) = with(|s| (std::mem::replace(&mut s.fdlg, null_mut()), s.fr, s.win.tabs[s.win.cur].edit));
    if !old.is_null() {
        DestroyWindow(old);
    }
    // Prefill with the selection when it is a single line.
    let e = ed(edit);
    let selected = &e.text[e.sel()];
    if !selected.is_empty() && selected.len() < 255 && !selected.contains(&(b'\n' as u16)) {
        let buf = std::slice::from_raw_parts_mut((*fr).lpstrFindWhat, 256);
        buf[..selected.len()].copy_from_slice(selected);
        buf[selected.len()] = 0;
    }
    (*fr).Flags &= !(FR_DIALOGTERM | FR_FINDNEXT | FR_REPLACE | FR_REPLACEALL);
    (*fr).hwndOwner = with(|s| s.win.main);
    let dlg = if replace { ReplaceTextW(fr) } else { FindTextW(fr) };
    with(|s| s.fdlg = dlg);
}

unsafe fn on_find(fr: *const FINDREPLACEW) {
    let f = (*fr).Flags;
    if f & FR_DIALOGTERM != 0 {
        return with(|s| s.fdlg = null_mut());
    }
    let cstr = |p: *const u16| (0..).take_while(|&i| *p.add(i) != 0).map(|i| *p.add(i)).collect::<Vec<u16>>();
    let (what, repl) = (cstr((*fr).lpstrFindWhat), cstr((*fr).lpstrReplaceWith));
    let edit = cur_edit();
    if what.is_empty() {
        return;
    }
    let e = ed(edit);
    if f & FR_REPLACEALL != 0 {
        let mut out = Vec::with_capacity(e.text.len());
        let mut pos = 0;
        for i in hits(&e.text, &what, f) {
            if i >= pos {
                out.extend_from_slice(&e.text[pos..i]);
                out.extend_from_slice(&repl);
                pos = i + what.len();
            }
        }
        if pos == 0 {
            return not_found(&what);
        }
        out.extend_from_slice(&e.text[pos..]);
        e.replace(0..e.text.len(), out);
        return e.select(0, 0);
    }
    if f & FR_REPLACE != 0 {
        let sel = e.sel();
        if sel.len() == what.len() && hits(&e.text, &what, f).contains(&sel.start) {
            e.replace(sel, repl);
        }
    }
    // The Replace dialog has no direction buttons; it always searches down.
    find_next(edit, &what, if f & FR_REPLACE != 0 { f | FR_DOWN } else { f });
    update_count();
}

// Controls of the common Find/Replace dialog.
const FIND_TEXT: i32 = 0x480;
const WHOLE_WORD: i32 = 0x410;
const MATCH_CASE: i32 = 0x411;

/// Hook for the Find/Replace dialog: keeps the match count in its title up to date.
unsafe extern "system" fn find_hook(dlg: HWND, m: u32, wp: WPARAM, _: LPARAM) -> usize {
    match m {
        WM_INITDIALOG => {
            let mut buf = [0u16; 128];
            let n = GetWindowTextW(dlg, buf.as_mut_ptr(), buf.len() as i32);
            with(|s| (s.fdlg, s.fdlg_title) = (dlg, String::from_utf16_lossy(&buf[..n as usize])));
            update_count();
            1 // let the dialog set the default focus
        }
        WM_COMMAND => {
            let (id, code) = ((wp & 0xFFFF) as i32, ((wp >> 16) & 0xFFFF) as u32);
            if (id == FIND_TEXT && code == EN_CHANGE) || ((id == WHOLE_WORD || id == MATCH_CASE) && code == BN_CLICKED) {
                update_count();
            }
            0
        }
        _ => 0,
    }
}

/// Shows "k of n matches" (or "n matches" / "No matches") in the Find/Replace dialog's title.
unsafe fn update_count() {
    let (dlg, title, edit) = with(|s| (s.fdlg, s.fdlg_title.clone(), s.win.tabs.get(s.win.cur).map(|t| t.edit)));
    let Some(edit) = edit.filter(|_| !dlg.is_null()) else { return };
    let mut buf = [0u16; 256];
    let n = GetDlgItemTextW(dlg, FIND_TEXT, buf.as_mut_ptr(), buf.len() as i32) as usize;
    let what = &buf[..n];
    let checked = |id| IsDlgButtonChecked(dlg, id) == BST_CHECKED;
    let flags = if checked(WHOLE_WORD) { FR_WHOLEWORD } else { 0 } | if checked(MATCH_CASE) { FR_MATCHCASE } else { 0 };
    let count = if what.is_empty() {
        String::new()
    } else {
        let e = ed(edit);
        let hits = hits(&e.text, what, flags);
        let sel = e.sel();
        match (hits.len(), hits.iter().position(|&i| i == sel.start && sel.len() == what.len())) {
            (0, _) => " – No matches".into(),
            (1, Some(_)) => " – 1 of 1 match".into(),
            (n, Some(k)) => format!(" – {} of {n} matches", k + 1),
            (1, None) => " – 1 match".into(),
            (n, None) => format!(" – {n} matches"),
        }
    };
    SetWindowTextW(dlg, wide(&format!("{title}{count}")).as_ptr());
}

/// F3: repeat the last search, or open the Find dialog if there is none yet.
unsafe fn find_again() {
    let fr = with(|s| s.fr);
    if *(*fr).lpstrFindWhat == 0 {
        return find_dialog(false);
    }
    let what: Vec<u16> = (0..).map(|i| *(*fr).lpstrFindWhat.add(i)).take_while(|&c| c != 0).collect();
    find_next(cur_edit(), &what, (*fr).Flags);
}

/// Name shared by all launches using the same data folder (sandboxed test copies get their own).
fn instance_name() -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    app_dir().hash(&mut h);
    format!("minedit.{:x}", h.finish())
}

/// Paths as UTF-16 separated by NUL, for WM_COPYDATA.
fn encode_paths(paths: &[PathBuf]) -> Vec<u16> {
    paths.iter().flat_map(|p| p.to_string_lossy().encode_utf16().chain(Some(0)).collect::<Vec<_>>()).collect()
}

fn decode_paths(data: &[u16]) -> Vec<PathBuf> {
    data.split(|&c| c == 0).filter(|p| !p.is_empty()).map(|p| PathBuf::from(String::from_utf16_lossy(p))).collect()
}

/// If minedit is already running, gives it the files to open, brings it to the front and returns true.
unsafe fn hand_over(class: &[u16], paths: &[PathBuf]) -> bool {
    for _ in 0..40 {
        let h = FindWindowW(class.as_ptr(), null());
        if !h.is_null() {
            let data = encode_paths(paths);
            let cds = COPYDATASTRUCT { dwData: COPYDATA_OPEN, cbData: (data.len() * 2) as u32, lpData: data.as_ptr() as _ };
            SendMessageW(h, WM_COPYDATA, 0, &cds as *const _ as LPARAM);
            if IsIconic(h) != 0 {
                ShowWindow(h, SW_RESTORE);
            }
            SetForegroundWindow(h);
            return true;
        }
        Sleep(50); // the other launch may still be creating its window
    }
    false
}

fn lang_of(p: Option<&PathBuf>) -> syntax::Lang {
    let Some(p) = p else { return syntax::Lang::Plain };
    let name = p.file_name().map_or(String::new(), |n| n.to_string_lossy().to_ascii_lowercase());
    if [".bashrc", ".bash_profile", ".bash_aliases", ".profile", ".zshrc", ".zprofile"].contains(&name.as_str()) {
        return syntax::Lang::Bash;
    }
    match p.extension() {
        Some(e) => syntax::Lang::from_ext(&e.to_string_lossy()),
        // Scripts without an extension: look at the #! line.
        None => match std::fs::read(p).ok().and_then(|b| b.split(|&c| c == b'\n').next().map(|l| l.to_vec())) {
            Some(first) if first.starts_with(b"#!") && first.windows(2).any(|w| w == b"sh") => syntax::Lang::Bash,
            _ => syntax::Lang::Plain,
        },
    }
}

/// Sets or clears (None = detect again) a tab's language override.
unsafe fn set_tab_lang(i: usize, lang: Option<syntax::Lang>) {
    let (edit, path) = with(|s| {
        s.win.tabs[i].lang = lang;
        (s.win.tabs[i].edit, s.win.tabs[i].path.clone())
    });
    ed(edit).set_lang(lang.unwrap_or(lang_of(path.as_ref())));
    repaint_bar();
    schedule_save();
}

const GOTO_EDIT: i32 = 100;

/// Go to Line dialog as an in-memory template: a label, an edit box and OK/Cancel.
fn goto_template() -> Vec<u32> {
    fn dword(v: &mut Vec<u16>, x: u32) {
        v.extend([x as u16, (x >> 16) as u16]);
    }
    fn text(v: &mut Vec<u16>, t: &str) {
        v.extend(t.encode_utf16().chain(Some(0)));
    }
    let items: [(u32, [i16; 4], u16, u16, &str); 4] = [
        (0 /* SS_LEFT */, [7, 7, 170, 9], 0xFFFF, 0x82, "Line or line:column"),
        (WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32, [7, 18, 170, 13], GOTO_EDIT as u16, 0x81, ""),
        (WS_TABSTOP | BS_DEFPUSHBUTTON as u32, [66, 38, 52, 14], IDOK as u16, 0x80, "OK"),
        (WS_TABSTOP | BS_PUSHBUTTON as u32, [124, 38, 52, 14], IDCANCEL as u16, 0x80, "Cancel"),
    ];
    let mut v = Vec::new();
    dword(&mut v, (DS_MODALFRAME | DS_CENTER | DS_SETFONT) as u32 | WS_POPUP | WS_CAPTION | WS_SYSMENU);
    dword(&mut v, 0);
    v.push(items.len() as u16);
    v.extend([0, 0, 184, 59, 0, 0]); // x, y, cx, cy in dialog units; no menu; default class
    text(&mut v, "Go to Line");
    v.push(9);
    text(&mut v, "Segoe UI");
    for (style, rect, id, class, label) in items {
        if v.len() % 2 == 1 {
            v.push(0); // items start on a DWORD boundary
        }
        dword(&mut v, style | WS_CHILD | WS_VISIBLE);
        dword(&mut v, 0);
        v.extend(rect.map(|n| n as u16));
        v.extend([id, 0xFFFF, class]); // class given as a predefined atom
        text(&mut v, label);
        v.push(0); // no creation data
    }
    v.resize(v.len().next_multiple_of(2), 0);
    v.chunks(2).map(|c| c[0] as u32 | (c[1] as u32) << 16).collect()
}

unsafe extern "system" fn goto_proc(dlg: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> isize {
    match m {
        WM_INITDIALOG => {
            SetDlgItemTextW(dlg, GOTO_EDIT, wide(&lp.to_string()).as_ptr());
            SendDlgItemMessageW(dlg, GOTO_EDIT, EM_SETSEL, 0, -1);
            1
        }
        WM_COMMAND => {
            match (wp & 0xFFFF) as i32 {
                IDOK => {
                    let mut buf = [0u16; 32];
                    let n = GetDlgItemTextW(dlg, GOTO_EDIT, buf.as_mut_ptr(), buf.len() as i32) as usize;
                    // Invalid input keeps the dialog open.
                    if let Some(target) = parse_goto(&String::from_utf16_lossy(&buf[..n])) {
                        with(|s| s.goto = Some(target));
                        EndDialog(dlg, 1);
                    }
                }
                IDCANCEL => _ = EndDialog(dlg, 0),
                _ => {}
            }
            1
        }
        _ => 0,
    }
}

/// "12" or "12:5" (also "12,5") as line and column.
fn parse_goto(t: &str) -> Option<(usize, usize)> {
    let mut parts = t.split([':', ',']).map(|x| x.trim().parse::<usize>().ok());
    let line = parts.next()??;
    let col = parts.next().unwrap_or(Some(1))?;
    parts.next().is_none().then_some((line, col))
}

/// Ctrl+G or a click on "Ln, Col" in the status bar.
unsafe fn go_to_line() {
    let (main, edit) = with(|s| (s.win.main, s.win.tabs[s.win.cur].edit));
    let template = goto_template();
    let line = ed(edit).caret().0 as LPARAM;
    if DialogBoxIndirectParamW(GetModuleHandleW(null()), template.as_ptr() as *const DLGTEMPLATE, main, Some(goto_proc), line) == 1 {
        if let Some((l, c)) = with(|s| s.goto.take()) {
            ed(edit).go_to(l, c);
        }
    }
}

/// Screen point just above the status bar item `at`, where its menu opens.
unsafe fn menu_point(at: RECT) -> POINT {
    let main = with(|s| s.win.main);
    let mut rc: RECT = zeroed();
    GetClientRect(main, &mut rc);
    let mut pt = POINT { x: at.left, y: rc.bottom - status_height() };
    ClientToScreen(main, &mut pt);
    pt
}

/// A menu of `items` above a status bar item, with `checked` ticked. Returns the chosen index.
unsafe fn pick(at: RECT, items: &[&str], checked: usize) -> Option<usize> {
    let menu = CreatePopupMenu();
    for (k, item) in items.iter().enumerate() {
        AppendMenuW(menu, MF_STRING | if k == checked { MF_CHECKED } else { MF_UNCHECKED }, k + 1, wide(item).as_ptr());
    }
    let pt = menu_point(at);
    let cmd = TrackPopupMenuEx(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_BOTTOMALIGN, pt.x, pt.y, with(|s| s.win.main), null());
    DestroyMenu(menu);
    (cmd > 0).then(|| cmd as usize - 1)
}

/// Line endings used when saving; changing them marks the tab as modified.
unsafe fn choose_eol(at: RECT) {
    let (i, lf, edit) = with(|s| (s.win.cur, s.win.tabs[s.win.cur].lf, s.win.tabs[s.win.cur].edit));
    if let Some(k) = pick(at, &["CRLF (Windows)", "LF (Unix)"], lf as usize).filter(|&k| (k == 1) != lf) {
        with(|s| s.win.tabs[i].lf = k == 1);
        ed(edit).mark_dirty();
        refresh(i);
        schedule_save();
    }
}

/// Encoding used when saving; changing it marks the tab as modified.
unsafe fn choose_enc(at: RECT) {
    let (i, enc, edit) = with(|s| (s.win.cur, s.win.tabs[s.win.cur].enc, s.win.tabs[s.win.cur].edit));
    let names: Vec<&str> = ENCODINGS.iter().map(|e| e.1).collect();
    let cur = ENCODINGS.iter().position(|e| e.0 == enc).unwrap_or(0);
    if let Some(k) = pick(at, &names, cur).filter(|&k| k != cur) {
        with(|s| s.win.tabs[i].enc = ENCODINGS[k].0);
        ed(edit).mark_dirty();
        refresh(i);
        schedule_save();
    }
}

/// Language menu opened from the status bar.
unsafe fn choose_lang(at: RECT) {
    let (main, i, over, path) = with(|s| (s.win.main, s.win.cur, s.win.tabs[s.win.cur].lang, s.win.tabs[s.win.cur].path.clone()));
    let menu = CreatePopupMenu();
    let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
    let auto = wide(&format!("Auto Detect ({})", lang_of(path.as_ref()).name()));
    AppendMenuW(menu, MF_STRING | check(over.is_none()), 1, auto.as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    // Plain Text first, then by name.
    let mut langs = syntax::Lang::ALL.to_vec();
    langs[1..].sort_by_key(|l| l.name());
    for (k, l) in langs.iter().enumerate() {
        AppendMenuW(menu, MF_STRING | check(over == Some(*l)), 2 + k, wide(l.name()).as_ptr());
    }
    let pt = menu_point(at);
    let cmd = TrackPopupMenuEx(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_BOTTOMALIGN, pt.x, pt.y, main, null()) as usize;
    DestroyMenu(menu);
    match cmd {
        0 => {}
        1 => set_tab_lang(i, None),
        k => set_tab_lang(i, Some(langs[k - 2])),
    }
}

fn stamp(p: &PathBuf) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Reloads tabs whose file was changed by another program; asks first when a tab has unsaved changes.
unsafe fn check_files() {
    if with(|s| std::mem::replace(&mut s.checking, true)) {
        return;
    }
    for i in 0..with(|s| s.win.tabs.len()) {
        let Some((path, old, edit, main)) = with(|s| s.win.tabs.get(i).and_then(|t| Some((t.path.clone()?, t.stamp, t.edit, s.win.main)))) else { continue };
        let now = stamp(&path);
        if now.is_none() || now == old {
            continue; // unchanged, or deleted: keep the text so it can be saved again
        }
        with(|s| s.win.tabs[i].stamp = now); // ask only once per change
        if ed(edit).dirty() {
            select(i);
            let msg = wide(&format!("{}\n\nThis file was changed by another program. Reload it and lose your unsaved changes?", path.display()));
            if MessageBoxW(main, msg.as_ptr(), wide(APP).as_ptr(), MB_YESNO | MB_ICONWARNING) != IDYES {
                continue;
            }
        }
        reload(i, &path);
    }
    with(|s| s.checking = false);
}

/// Replaces a tab's text with the file on disk as one undoable step, keeping the caret near where it was.
unsafe fn reload(i: usize, path: &PathBuf) {
    let Ok((text, enc, lf)) = load(path) else { return };
    let edit = with(|s| {
        let t = &mut s.win.tabs[i];
        (t.enc, t.lf) = (enc, lf);
        t.edit
    });
    let e = ed(edit);
    if e.text != text {
        let p = e.sel().start.min(text.len());
        e.replace(0..e.text.len(), text);
        e.select(p, p);
    }
    e.mark_saved();
    refresh(i);
    if Some(path) == settings_path().as_ref() {
        load_settings();
        set_fonts();
    }
}

fn bar_rects() -> titlebar::Rects {
    let (main, dpi, n, scroll) = with(|s| (s.win.main, s.dpi, s.win.tabs.len(), s.win.scroll));
    let mut rc: RECT = unsafe { zeroed() };
    unsafe { GetClientRect(main, &mut rc) };
    titlebar::layout(rc.right, dpi, n, scroll)
}

/// Editors fill the window below the title bar.
unsafe fn layout() {
    let Some((main, edits)) = try_with(|s| Some((s.win.main, s.win.tabs.iter().map(|t| t.edit).collect::<Vec<_>>()))) else { return };
    let mut rc: RECT = zeroed();
    GetClientRect(main, &mut rc);
    let (h, sh) = (bar_rects().height, status_height());
    for e in edits {
        MoveWindow(e, 0, h, rc.right, rc.bottom - h - sh, 1);
    }
    repaint_bar();
}

unsafe fn repaint_bar() {
    // WS_CLIPCHILDREN: only the title bar (not covered by an editor) is repainted.
    InvalidateRect(with(|s| s.win.main), null(), 0);
}

fn status_height() -> i32 {
    24 * with(|s| s.dpi) as i32 / 96
}

/// Status bar: caret position on the left; word wrap and language on the right.
unsafe fn paint_status(dc: HDC, w: i32, sh: i32) {
    let (tab, font, dpi, wrap) = with(|s| (s.win.tabs.get(s.win.cur).map(|t| (t.edit, t.enc, t.lf)), s.ufont, s.dpi, s.opts.word_wrap));
    let p = |v: i32| v * dpi as i32 / 96;
    let fill = |r: RECT, c: u32| {
        let b = CreateSolidBrush(c);
        FillRect(dc, &r, b);
        DeleteObject(b);
    };
    let c = theme::palette();
    fill(RECT { left: 0, top: 0, right: w, bottom: sh }, c.status_bg);
    fill(RECT { left: 0, top: 0, right: w, bottom: 1 }, c.status_border);
    let Some((edit, enc, lf)) = tab else { return };
    let e = ed(edit);
    let (line, col) = e.caret();
    SelectObject(dc, font);
    SetBkMode(dc, TRANSPARENT as _);
    SetTextColor(dc, c.status_fg);
    let draw = |text: &str, x: i32| {
        let t: Vec<u16> = text.encode_utf16().collect();
        let mut sz: SIZE = zeroed();
        GetTextExtentPoint32W(dc, t.as_ptr(), t.len() as i32, &mut sz);
        TextOutW(dc, x, (sh - sz.cy) / 2, t.as_ptr(), t.len() as i32);
        sz.cx
    };
    let pw = draw(&format!("Ln {line}, Col {col}"), p(12));
    let mut items = vec![(RECT { left: p(6), top: 0, right: p(18) + pw, bottom: sh }, Status::Pos)];
    let enc_name = ENCODINGS.iter().find(|e| e.0 == enc).map_or("", |e| e.1);
    // Drawn from the right edge leftwards: Word Wrap · encoding · line endings · language.
    let mut x = w - p(12);
    for (item, kind) in [
        (e.lang().name(), Status::Lang),
        (if lf { "LF" } else { "CRLF" }, Status::Eol),
        (enc_name, Status::Enc),
        (if wrap { "Word Wrap: On" } else { "Word Wrap: Off" }, Status::Wrap),
    ] {
        let t: Vec<u16> = item.encode_utf16().collect();
        let mut sz: SIZE = zeroed();
        GetTextExtentPoint32W(dc, t.as_ptr(), t.len() as i32, &mut sz);
        x -= sz.cx;
        draw(item, x);
        items.push((RECT { left: x - p(6), top: 0, right: x + sz.cx + p(6), bottom: sh }, kind));
        x -= p(24);
    }
    with(|s| s.win.status_items = items);
}

unsafe fn paint_bar(h: HWND) {
    let mut ps: PAINTSTRUCT = zeroed();
    let hdc = BeginPaint(h, &mut ps);
    let r = bar_rects();
    let mut rc: RECT = zeroed();
    GetClientRect(h, &mut rc);
    let (w, bh) = (rc.right.max(1), r.height);
    let dc = CreateCompatibleDC(hdc);
    let bmp = CreateCompatibleBitmap(hdc, w, bh);
    let old = SelectObject(dc, bmp);
    let (tabs, cur, look) = with(|s| {
        let tabs: Vec<(String, bool)> = s.win.tabs.iter().map(|t| (name(t), t.dirty)).collect();
        let look = titlebar::Look {
            font: s.ufont, icons: s.ifont, small_icons: s.ifont_small, dpi: s.dpi,
            active: s.win.active, maximized: IsZoomed(h) != 0, hover: s.win.hover,
        };
        (tabs, s.win.cur, look)
    });
    titlebar::paint(dc, w, &r, &tabs, cur, &look);
    BitBlt(hdc, 0, 0, w, bh, dc, 0, 0, SRCCOPY);
    SelectObject(dc, old);
    DeleteObject(bmp);
    let sh = status_height();
    let bmp = CreateCompatibleBitmap(hdc, w, sh);
    let old = SelectObject(dc, bmp);
    paint_status(dc, w, sh);
    BitBlt(hdc, 0, rc.bottom - sh, w, sh, dc, 0, 0, SRCCOPY);
    SelectObject(dc, old);
    DeleteObject(bmp);
    DeleteDC(dc);
    EndPaint(h, &ps);
}

unsafe fn set_hover(hit: Hit) {
    if with(|s| std::mem::replace(&mut s.win.hover, hit)) != hit {
        repaint_bar();
    }
}

/// Opens the menu under the ≡ button. `key`: Some(0) for plain Alt (highlight the first item),
/// Some(letter) for Alt+letter (open that submenu), None for a mouse click.
unsafe fn show_menu(key: Option<u16>) {
    let (main, menu) = with(|s| (s.win.main, s.menu));
    let r = bar_rects();
    let mut pt = POINT { x: r.menu.right, y: r.height };
    ClientToScreen(main, &mut pt);
    // The menu loop reads these from the queue as if they were typed.
    match key {
        Some(0) => _ = PostMessageW(main, WM_KEYDOWN, VK_DOWN as usize, 0),
        Some(c) => _ = PostMessageW(main, WM_CHAR, c as usize, 0),
        None => {}
    }
    with(|s| s.win.hover = Hit::Menu);
    repaint_bar();
    TrackPopupMenuEx(menu, TPM_RIGHTALIGN | TPM_TOPALIGN, pt.x, pt.y, main, null());
    set_hover(Hit::None);
}

/// Thickness of the resize border hidden above the title bar.
unsafe fn frame(h: HWND) -> i32 {
    let dpi = GetDpiForWindow(h);
    GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
}

unsafe fn refresh(i: usize) {
    let Some((main, title, is_cur)) = try_with(|s| {
        let t = s.win.tabs.get_mut(i)?;
        t.dirty = ed(t.edit).dirty();
        let n = name(t);
        let mark = if t.dirty { "*" } else { "" };
        let full = t.path.as_ref().map_or(n.clone(), |p| p.display().to_string());
        Some((s.win.main, format!("{mark}{full} - {APP}"), s.win.cur == i))
    }) else { return };
    repaint_bar();
    if is_cur {
        SetWindowTextW(main, wide(&title).as_ptr());
    }
}

/// Saves the session two seconds after the last change, so a crash loses at most that much.
unsafe fn schedule_save() {
    if let Some(main) = try_with(|s| (!s.win.main.is_null()).then_some(s.win.main)) {
        SetTimer(main, TIMER_SESSION, 2000, None); // restarts the countdown if already running
    }
}

unsafe fn select(i: usize) {
    scroll_to_tab(i);
    let (old, new) = with(|s| {
        let old = s.win.tabs.get(s.win.cur).map(|t| t.edit);
        s.win.cur = i;
        (old, s.win.tabs[i].edit)
    });
    layout();
    ShowWindow(new, SW_SHOW);
    if let Some(old) = old.filter(|&o| o != new) {
        ShowWindow(old, SW_HIDE);
    }
    // Only while we are in front: SetFocus would otherwise activate the window (e.g. during startup
    // restore, before it is shown) and take the keyboard from the user's current app.
    // WM_SETFOCUS hands focus to the current tab once the window is activated.
    if GetForegroundWindow() == with(|s| s.win.main) {
        SetFocus(new);
    }
    schedule_save();
    refresh(i);
}

unsafe fn add_tab(path: Option<PathBuf>, text: Vec<u16>, enc: Enc, lf: bool, dirty: bool) {
    let edit = edit::create(with(|s| s.win.main));
    theme_editor(edit);
    ed(edit).set_text(text);
    ed(edit).set_lang(lang_of(path.as_ref()));
    if dirty {
        ed(edit).mark_dirty();
    }
    let i = with(|s| {
        let stamp = path.as_ref().and_then(stamp);
        s.win.tabs.push(Tab { edit, path, dirty, enc, lf, stamp, lang: None });
        s.win.tabs.len() - 1
    });
    select(i);
}

unsafe fn new_tab() {
    add_tab(None, Vec::new(), Enc::Utf8, false, false);
}

/// A Ctrl+P list entry: an open tab (window, editor) or a recent file.
enum Pick {
    Tab(HWND, HWND),
    File(PathBuf),
}

fn recent_path() -> Option<PathBuf> {
    Some(app_dir()?.join("recent"))
}

fn add_recent(s: &mut State, p: &PathBuf) {
    s.recent.retain(|r| r != p);
    s.recent.insert(0, p.clone());
    s.recent.truncate(s.max_recent);
}

fn load_recent() {
    let list = recent_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    with(|s| {
        s.recent = list.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect();
        s.recent.truncate(s.max_recent);
    });
}

fn save_recent() {
    let text: String = with(|s| s.recent.iter().map(|p| format!("{}\n", p.display())).collect());
    if let Some(p) = recent_path() {
        _ = std::fs::create_dir_all(p.parent().unwrap());
        _ = std::fs::write(p, text);
    }
}

/// Ctrl+P: open tabs of all windows, then recent files that are not open.
unsafe fn quick_open() {
    let split = |p: &PathBuf| (p.file_name().map_or(String::new(), |n| n.to_string_lossy().into()),
        p.parent().map_or(String::new(), |d| d.display().to_string()));
    let (items, picks, main, font, dpi) = with(|s| {
        let mut items = Vec::new();
        let mut picks = Vec::new();
        for w in std::iter::once(&s.win).chain(&s.parked) {
            for t in &w.tabs {
                let (name, dir) = t.path.as_ref().map_or(("Untitled".into(), String::new()), split);
                items.push(quick::Item { name, dir, open: true });
                picks.push(Pick::Tab(w.main, t.edit));
            }
        }
        let open: Vec<PathBuf> = std::iter::once(&s.win).chain(&s.parked).flat_map(|w| w.tabs.iter().filter_map(|t| t.path.clone())).collect();
        for p in s.recent.iter().filter(|p| !open.contains(p)) {
            let (name, dir) = split(p);
            items.push(quick::Item { name, dir, open: false });
            picks.push(Pick::File(p.clone()));
        }
        (items, picks, s.win.main, s.ufont, s.dpi)
    });
    with(|s| s.quick = picks);
    quick::show(main, items, font, dpi, bar_rects().height, quick_pick);
}

unsafe fn quick_pick(i: usize) {
    match with(|s| s.quick.get(i).map(|p| match p { Pick::Tab(w, e) => Pick::Tab(*w, *e), Pick::File(f) => Pick::File(f.clone()) })) {
        Some(Pick::Tab(w, e)) => show_tab(w, e),
        Some(Pick::File(f)) => open_path(f),
        None => {}
    }
}

/// Brings window `w` to the front with the tab of editor `e`.
unsafe fn show_tab(w: HWND, e: HWND) {
    let prev = enter(w);
    if let Some(i) = with(|s| s.win.tabs.iter().position(|t| t.edit == e)) {
        select(i);
    }
    if IsIconic(w) != 0 {
        ShowWindow(w, SW_RESTORE);
    }
    SetForegroundWindow(w);
    leave(prev);
}

fn session_path() -> Option<PathBuf> {
    Some(app_dir()?.join("session"))
}

/// Session format (version 2): header line, "font-pt", then per window a line
/// "W current-tab maximized left top right bottom" followed by its tabs, each a line
/// "dirty enc lf len lang\tpath\n" and `len` bytes of UTF-8 text (only for dirty tabs).
/// Version 1 (one window) is still read.
unsafe fn save_session() {
    save_recent();
    let Some(p) = session_path() else { return };
    type TabInfo = (HWND, Option<PathBuf>, Enc, bool, Option<syntax::Lang>);
    let (windows, pt): (Vec<(HWND, usize, Vec<TabInfo>)>, i32) = with(|s| {
        let windows = std::iter::once(&s.win).chain(&s.parked).filter(|w| !w.tabs.is_empty())
            .map(|w| (w.main, w.cur, w.tabs.iter().map(|t| (t.edit, t.path.clone(), t.enc, t.lf, t.lang)).collect()))
            .collect();
        (windows, s.pt)
    });
    let mut out = format!("{APP}-session-2\n{pt}\n").into_bytes();
    for (main, cur, tabs) in windows {
        let mut wp: WINDOWPLACEMENT = zeroed();
        wp.length = size_of::<WINDOWPLACEMENT>() as u32;
        GetWindowPlacement(main, &mut wp);
        let max = wp.showCmd == SW_SHOWMAXIMIZED as _
            || (wp.showCmd == SW_SHOWMINIMIZED as _ && wp.flags & WPF_RESTORETOMAXIMIZED != 0);
        let RECT { left, top, right, bottom } = wp.rcNormalPosition;
        out.extend(format!("W {cur} {} {left} {top} {right} {bottom}\n", max as u8).bytes());
        for (edit, path, enc, lf, lang) in tabs {
            let e = ed(edit);
            let body = if e.dirty() { String::from_utf16_lossy(&e.text) } else { String::new() };
            let path = path.map_or(String::new(), |p| p.to_string_lossy().into());
            // Language override: 0 = detect, otherwise 1 + index in Lang::ALL.
            let lang = lang.map_or(0, |l| 1 + syntax::Lang::ALL.iter().position(|&a| a == l).unwrap());
            out.extend(format!("{} {} {} {} {lang}\t{path}\n", e.dirty() as u8, enc as u8, lf as u8, body.len()).bytes());
            out.extend(body.bytes());
        }
    }
    let tmp = p.with_extension("tmp");
    if std::fs::create_dir_all(p.parent().unwrap()).is_ok() && std::fs::write(&tmp, out).is_ok() {
        _ = std::fs::rename(tmp, p);
    }
}

/// A tab as stored in the session.
struct SavedTab {
    path: Option<PathBuf>,
    dirty: bool,
    enc: Enc,
    lf: bool,
    lang: usize,
    body: Vec<u8>,
}

/// A window as stored in the session.
struct SavedWindow {
    cur: usize,
    place: Option<(RECT, bool)>,
    tabs: Vec<SavedTab>,
}

fn parse_session(data: &[u8]) -> Option<(i32, Vec<SavedWindow>)> {
    fn line(rest: &mut &[u8]) -> Option<String> {
        let i = rest.iter().position(|&b| b == b'\n')?;
        let l = String::from_utf8_lossy(&rest[..i]).into_owned();
        *rest = &rest[i + 1..];
        Some(l)
    }
    fn nums<T: std::str::FromStr>(l: &str) -> Option<Vec<T>> {
        l.split(' ').map(|x| x.parse().ok()).collect()
    }
    let window = |v: &[i32]| match *v {
        [cur, max, left, top, right, bottom] => Some(SavedWindow { cur: cur.max(0) as usize, place: Some((RECT { left, top, right, bottom }, max == 1)), tabs: Vec::new() }),
        [cur] => Some(SavedWindow { cur: cur.max(0) as usize, place: None, tabs: Vec::new() }),
        _ => None,
    };
    let mut rest = data;
    let version = line(&mut rest)?;
    let header = line(&mut rest)?;
    let header: Vec<i32> = nums(header.split('\t').next()?)?; // version 1 also stored the font face after a tab
    let (pt, mut windows) = if version == format!("{APP}-session-2") {
        (*header.first()?, Vec::new())
    } else if version == format!("{APP}-session-1") {
        // Version 1: "cur pt [max left top right bottom]" for its single window.
        let (&cur, &pt) = (header.first()?, header.get(1)?);
        let w = window(&[[cur].as_slice(), &header[2..]].concat())?;
        (pt, vec![w])
    } else {
        return None;
    };
    while let Some(l) = line(&mut rest) {
        if let Some(w) = l.strip_prefix("W ") {
            windows.push(window(&nums(w)?)?);
            continue;
        }
        let (meta, path) = l.split_once('\t')?;
        let n: Vec<usize> = nums(meta)?;
        let (dirty, enc, lf, len, lang) = match n[..] {
            [d, e, l, n] => (d, e, l, n, 0), // sessions from before language overrides
            [d, e, l, n, g] => (d, e, l, n, g),
            _ => return None,
        };
        let body = rest.get(..len)?.to_vec();
        rest = &rest[len..];
        windows.last_mut()?.tabs.push(SavedTab {
            path: (!path.is_empty()).then(|| PathBuf::from(path)),
            dirty: dirty == 1,
            enc: *[Enc::Utf8, Enc::Utf8Bom, Enc::Utf16, Enc::Ansi].get(enc)?,
            lf: lf == 1,
            lang,
            body,
        });
    }
    Some((pt, windows))
}

/// Restores the saved windows: the first into the current window, the others into new windows.
unsafe fn restore_session() -> Option<()> {
    let (pt, windows) = parse_session(&std::fs::read(session_path()?).ok()?)?;
    with(|s| s.pt = pt.clamp(6, 72));
    set_fonts();
    for (k, w) in windows.into_iter().enumerate() {
        let (h, prev) = if k == 0 {
            (with(|s| s.win.main), None)
        } else {
            let h = create_window();
            (h, Some(enter(h)))
        };
        with(|s| s.win.place = w.place);
        for t in w.tabs {
            let before = with(|s| s.win.tabs.len());
            if t.dirty {
                let text: Vec<u16> = String::from_utf8_lossy(&t.body).encode_utf16().collect();
                add_tab(t.path, edit::to_lf(&text), t.enc, t.lf, true);
            } else if let Some(p) = t.path {
                // Unmodified files are reloaded from disk; ones that no longer exist are dropped.
                if let Ok((text, enc, lf)) = load(&p) {
                    add_tab(Some(p), text, enc, lf, false);
                }
            } else {
                new_tab();
            }
            if let Some(&l) = t.lang.checked_sub(1).and_then(|k| syntax::Lang::ALL.get(k)) {
                if with(|s| s.win.tabs.len()) > before {
                    set_tab_lang(before, Some(l));
                }
            }
        }
        let n = with(|s| s.win.tabs.len());
        if n > 0 {
            select(w.cur.min(n - 1));
        }
        if let Some(prev) = prev {
            if n == 0 {
                DestroyWindow(h); // all its files are gone
            } else {
                // Shown by main() after the main window: the first window a new process shows is special.
                with(|s| s.later.push((h, w.place)));
            }
            leave(prev);
        }
    }
    Some(())
}

unsafe fn error(msg: String) {
    let main = with(|s| s.win.main);
    MessageBoxW(main, wide(&msg).as_ptr(), wide(APP).as_ptr(), MB_ICONERROR);
}

unsafe fn open_path(p: PathBuf) {
    let p = std::path::absolute(&p).unwrap_or(p);
    with(|s| add_recent(s, &p));
    if let Some(i) = with(|s| s.win.tabs.iter().position(|t| t.path.as_ref() == Some(&p))) {
        return select(i);
    }
    // Already open in another window: go there.
    let elsewhere = with(|s| s.parked.iter().find_map(|w| w.tabs.iter().find(|t| t.path.as_ref() == Some(&p)).map(|t| (w.main, t.edit))));
    if let Some((w, e)) = elsewhere {
        return show_tab(w, e);
    }
    let (text, enc, lf) = match load(&p) {
        Ok(r) => r,
        Err(e) => return error(format!("{}\n\n{e}", p.display())),
    };
    // Replace the current tab if it is an untouched, empty "Untitled".
    let reuse = with(|s| {
        let t = s.win.tabs.get(s.win.cur)?;
        let e = ed(t.edit);
        (t.path.is_none() && !e.dirty() && e.text.is_empty()).then_some(s.win.cur)
    });
    match reuse {
        Some(i) => {
            let edit = with(|s| {
                let t = &mut s.win.tabs[i];
                (t.stamp, t.enc, t.lf) = (stamp(&p), enc, lf);
                t.path = Some(p);
                t.edit
            });
            ed(edit).set_text(text);
            with(|s| s.win.tabs[i].lang = None);
            ed(edit).set_lang(lang_of(with(|s| s.win.tabs[i].path.clone()).as_ref()));
            refresh(i);
        }
        None => add_tab(Some(p), text, enc, lf, false),
    }
}

/// Shows the common file dialog. Open supports multi-select.
unsafe fn file_dialog(save: bool, initial: Option<&PathBuf>) -> Vec<PathBuf> {
    let main = with(|s| s.win.main);
    let mut buf = vec![0u16; 65536];
    if let Some(p) = initial {
        let w = wide(&p.display().to_string());
        buf[..w.len()].copy_from_slice(&w);
    }
    let filter = wide("All Files (*.*)\0*.*\0Text Files (*.txt)\0*.txt\0");
    let mut ofn: OPENFILENAMEW = zeroed();
    ofn.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = main;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    ofn.Flags = OFN_EXPLORER | OFN_NOCHANGEDIR
        | if save { OFN_OVERWRITEPROMPT } else { OFN_FILEMUSTEXIST | OFN_ALLOWMULTISELECT };
    if (if save { GetSaveFileNameW(&mut ofn) } else { GetOpenFileNameW(&mut ofn) }) == 0 {
        return Vec::new();
    }
    // Multi-select yields "dir\0file1\0file2\0\0"; single yields "path\0\0".
    let mut parts = buf.split(|&c| c == 0).take_while(|s| !s.is_empty()).map(String::from_utf16_lossy);
    let first = PathBuf::from(parts.next().unwrap_or_default());
    let rest: Vec<PathBuf> = parts.map(|f| first.join(f)).collect();
    if rest.is_empty() { vec![first] } else { rest }
}

unsafe fn save(i: usize, save_as: bool) -> bool {
    let (edit, path, enc, lf) = with(|s| {
        let t = &s.win.tabs[i];
        (t.edit, t.path.clone(), t.enc, t.lf)
    });
    let path = match path.clone().filter(|_| !save_as) {
        Some(p) => p,
        None => match file_dialog(true, path.as_ref()).pop() {
            Some(p) => p,
            None => return false,
        },
    };
    let text = &ed(edit).text;
    let text = if lf { encode(text, enc) } else { encode(&edit::to_crlf(text), enc) };
    if let Err(e) = std::fs::write(&path, text) {
        error(format!("{}\n\n{e}", path.display()));
        return false;
    }
    ed(edit).mark_saved();
    ed(edit).set_lang(with(|s| s.win.tabs[i].lang).unwrap_or(lang_of(Some(&path))));
    with(|s| s.win.tabs[i].stamp = stamp(&path));
    let is_settings = Some(&path) == settings_path().as_ref();
    with(|s| {
        add_recent(s, &path);
        s.win.tabs[i].path = Some(path);
    });
    refresh(i);
    if is_settings {
        load_settings();
        set_fonts();
    }
    true
}

/// Asks to save a dirty tab. Returns false if the user cancelled.
unsafe fn confirm(i: usize) -> bool {
    let (main, edit, n) = with(|s| (s.win.main, s.win.tabs[i].edit, name(&s.win.tabs[i])));
    if !ed(edit).dirty() {
        return true;
    }
    select(i);
    let msg = wide(&format!("Save changes to {n}?"));
    match MessageBoxW(main, msg.as_ptr(), wide(APP).as_ptr(), MB_YESNOCANCEL | MB_ICONWARNING) {
        IDYES => save(i, false),
        IDNO => true,
        _ => false,
    }
}

unsafe fn close_tab(i: usize) {
    if i >= with(|s| s.win.tabs.len()) || !confirm(i) {
        return;
    }
    let (edit, next, empty) = with(|s| {
        let t = s.win.tabs.remove(i);
        let edit = t.edit;
        remember_closed(s, t);
        let next = if i < s.win.cur { s.win.cur - 1 } else if i == s.win.cur { i.min(s.win.tabs.len().saturating_sub(1)) } else { s.win.cur };
        s.win.cur = next;
        (edit, next, s.win.tabs.is_empty())
    });
    DestroyWindow(edit);
    if empty { new_tab() } else { select(next) }
}

/// Ctrl+Shift+T: reopens the most recently closed tab.
unsafe fn reopen_tab() {
    let Some(c) = with(|s| s.closed.pop()) else { return };
    // An untouched empty "Untitled" tab is replaced, like when opening a file.
    let pristine = with(|s| {
        let t = s.win.tabs.get(s.win.cur)?;
        let e = ed(t.edit);
        (t.path.is_none() && !e.dirty() && e.text.is_empty()).then_some(s.win.cur)
    });
    match c.path {
        Some(p) if !c.dirty && p.exists() => open_path(p),
        path => {
            add_tab(path, c.text, c.enc, c.lf, c.dirty);
            if let Some(i) = pristine {
                close_tab(i);
            }
        }
    }
    if c.lang.is_some() {
        set_tab_lang(with(|s| s.win.cur), c.lang);
    }
    let e = ed(cur_edit());
    let p = c.caret.min(e.text.len());
    e.select(p, p);
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let prev = enter(h);
    let r = window_proc(h, m, wp, lp);
    // An activated window stays current, so the message loop and other launches use it.
    if !(m == WM_ACTIVATE && (wp & 0xFFFF) as u32 != WA_INACTIVE) {
        leave(prev);
    }
    r
}

unsafe fn window_proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_CREATE => {
            with(|s| (s.win.main, s.dpi) = (h, GetDpiForWindow(h)));
            set_fonts();
            DragAcceptFiles(h, 1);
            SetTimer(h, TIMER_FILES, 2000, None);
            // Re-run WM_NCCALCSIZE so the system title bar is replaced by ours.
            SetWindowPos(h, null_mut(), 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            0
        }
        // Keep the normal frame except the caption: the client area extends to the window top.
        WM_NCCALCSIZE if wp != 0 => {
            let p = &mut *(lp as *mut NCCALCSIZE_PARAMS);
            let top = p.rgrc[0].top;
            let r = DefWindowProcW(h, m, wp, lp);
            p.rgrc[0].top = top + if IsZoomed(h) != 0 { frame(h) } else { 0 };
            r
        }
        // Stop the system from drawing a classic caption over ours.
        WM_NCACTIVATE => DefWindowProcW(h, m, wp, -1),
        WM_ACTIVATE => {
            let active = (wp & 0xFFFF) as u32 != WA_INACTIVE;
            with(|s| s.win.active = active);
            repaint_bar();
            if active {
                PostMessageW(h, WM_TIMER, 1, 0); // check files once activation is done
            }
            DefWindowProcW(h, m, wp, lp)
        }
        // The Windows light/dark setting changed.
        WM_SETTINGCHANGE if lp != 0 && setting_name(lp) == "ImmersiveColorSet" => {
            apply_theme();
            0
        }
        WM_COPYDATA => {
            let cds = &*(lp as *const COPYDATASTRUCT);
            if cds.dwData != COPYDATA_OPEN {
                return 0;
            }
            let data = std::slice::from_raw_parts(cds.lpData as *const u16, cds.cbData as usize / 2);
            with(|s| s.incoming.extend(decode_paths(data)));
            // Open after returning, so an error message cannot block the sending launch.
            PostMessageW(h, WM_OPEN_INCOMING, 0, 0);
            1
        }
        WM_OPEN_INCOMING => {
            for p in with(|s| std::mem::take(&mut s.incoming)) {
                open_path(p);
            }
            0
        }
        edit::CARET => {
            repaint_bar();
            update_count();
            0
        }
        WM_TIMER if wp == TIMER_SESSION => {
            KillTimer(h, TIMER_SESSION);
            save_session();
            0
        }
        WM_TIMER => {
            if with(|s| s.win.active) {
                check_files();
            }
            0
        }
        WM_EXITSIZEMOVE => {
            schedule_save();
            0
        }
        WM_NCHITTEST => {
            let r = DefWindowProcW(h, m, wp, lp);
            if r != HTCLIENT as isize {
                return r; // side and bottom resize borders
            }
            let mut pt = POINT { x: (lp & 0xFFFF) as i16 as i32, y: ((lp >> 16) & 0xFFFF) as i16 as i32 };
            ScreenToClient(h, &mut pt);
            let bar = bar_rects();
            if pt.y >= bar.height {
                return HTCLIENT as isize;
            }
            let mut rc: RECT = zeroed();
            GetClientRect(h, &mut rc);
            let f = frame(h);
            if IsZoomed(h) == 0 && pt.y < f {
                return (if pt.x < f * 2 { HTTOPLEFT } else if pt.x >= rc.right - f * 2 { HTTOPRIGHT } else { HTTOP }) as isize;
            }
            (match titlebar::hit(&bar, pt.x, pt.y) {
                Hit::Min => HTMINBUTTON,
                Hit::Max => HTMAXBUTTON, // lets Windows 11 show the snap layouts flyout
                Hit::Close => HTCLOSE,
                Hit::Caption => HTCAPTION,
                _ => HTCLIENT,
            }) as isize
        }
        WM_PAINT => {
            paint_bar(h);
            0
        }
        WM_ERASEBKGND => 1,
        // Caption buttons report non-client hits, so their mouse input arrives as WM_NC* messages.
        // They are handled here instead of DefWindowProcW, which would draw classic buttons.
        WM_NCMOUSEMOVE | WM_NCLBUTTONDOWN | WM_NCLBUTTONUP if matches!(wp as u32, HTMINBUTTON | HTMAXBUTTON | HTCLOSE) => {
            let hit = match wp as u32 { HTMINBUTTON => Hit::Min, HTMAXBUTTON => Hit::Max, _ => Hit::Close };
            set_hover(hit);
            let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE | TME_NONCLIENT, hwndTrack: h, dwHoverTime: 0 };
            TrackMouseEvent(&mut tme);
            match m {
                WM_NCLBUTTONDOWN => with(|s| s.win.pressed = hit),
                WM_NCLBUTTONUP if with(|s| std::mem::replace(&mut s.win.pressed, Hit::None)) == hit => {
                    set_hover(Hit::None);
                    match hit {
                        Hit::Min => _ = ShowWindow(h, SW_MINIMIZE),
                        Hit::Max => _ = ShowWindow(h, if IsZoomed(h) != 0 { SW_RESTORE } else { SW_MAXIMIZE }),
                        _ => _ = PostMessageW(h, WM_CLOSE, 0, 0),
                    }
                }
                _ => {}
            }
            0
        }
        WM_NCMOUSEMOVE => {
            set_hover(Hit::None);
            DefWindowProcW(h, m, wp, lp)
        }
        WM_NCMOUSELEAVE | WM_MOUSELEAVE => {
            set_hover(Hit::None);
            0
        }
        WM_MOUSEMOVE if with(|s| s.win.drag.is_some()) => {
            let (x, y) = ((lp & 0xFFFF) as i16 as i32, ((lp >> 16) & 0xFFFF) as i16 as i32);
            let d = with(|s| {
                let d = s.win.drag.as_mut().unwrap();
                d.moved |= (x - d.from.x).abs() > 4 || (y - d.from.y).abs() > 4;
                *d
            });
            // Within the tab bar the tab follows the mouse.
            if let (true, Hit::Tab(j) | Hit::TabClose(j)) = (d.moved, titlebar::hit(&bar_rects(), x, y)) {
                if j != d.tab {
                    with(|s| {
                        let t = s.win.tabs.remove(d.tab);
                        s.win.tabs.insert(j, t);
                        s.win.cur = j;
                        s.win.drag.as_mut().unwrap().tab = j;
                    });
                    repaint_bar();
                    schedule_save();
                }
            }
            0
        }
        WM_LBUTTONUP if with(|s| s.win.drag.is_some()) => {
            let d = with(|s| s.win.drag.take()).unwrap();
            ReleaseCapture();
            let mut pt = POINT { x: (lp & 0xFFFF) as i16 as i32, y: ((lp >> 16) & 0xFFFF) as i16 as i32 };
            ClientToScreen(h, &mut pt);
            if d.moved {
                drop_tab(d.tab, pt);
            }
            0
        }
        WM_CAPTURECHANGED => {
            with(|s| s.win.drag = None);
            0
        }
        WM_RBUTTONUP => {
            let (x, y) = ((lp & 0xFFFF) as i16 as i32, ((lp >> 16) & 0xFFFF) as i16 as i32);
            if let Hit::Tab(i) | Hit::TabClose(i) = titlebar::hit(&bar_rects(), x, y) {
                let mut pt = POINT { x, y };
                ClientToScreen(h, &mut pt);
                tab_menu(i, pt);
            }
            0
        }
        WM_MOUSEWHEEL => {
            // Over the tab bar the wheel scrolls the tabs.
            let mut pt = POINT { x: (lp & 0xFFFF) as i16 as i32, y: ((lp >> 16) & 0xFFFF) as i16 as i32 };
            ScreenToClient(h, &mut pt);
            if pt.y >= 0 && pt.y < bar_rects().height {
                let step = 60 * with(|s| s.dpi) as i32 / 96;
                let max = bar_rects().max_scroll;
                with(|s| s.win.scroll = (s.win.scroll - (wp >> 16) as i16 as i32 / 120 * step).clamp(0, max));
                repaint_bar();
            }
            0
        }
        WM_MOUSEMOVE => {
            let hit = titlebar::hit(&bar_rects(), (lp & 0xFFFF) as i16 as i32, ((lp >> 16) & 0xFFFF) as i16 as i32);
            set_hover(hit);
            let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
            TrackMouseEvent(&mut tme);
            0
        }
        WM_LBUTTONDOWN | WM_MBUTTONUP => {
            let (x, y) = ((lp & 0xFFFF) as i16 as i32, ((lp >> 16) & 0xFFFF) as i16 as i32);
            let mut rc: RECT = zeroed();
            GetClientRect(h, &mut rc);
            let y_status = y - (rc.bottom - status_height());
            let item = with(|s| s.win.status_items.iter().find(|(r, _)| x >= r.left && x < r.right && y_status >= 0 && y_status < r.bottom).copied());
            if let Some((r, kind)) = item.filter(|_| m == WM_LBUTTONDOWN) {
                match kind {
                    Status::Pos => go_to_line(),
                    Status::Lang => choose_lang(r),
                    Status::Eol => choose_eol(r),
                    Status::Enc => choose_enc(r),
                    Status::Wrap => _ = SendMessageW(h, WM_COMMAND, ID_WRAP as usize, 0),
                }
                return 0;
            }
            match (m, titlebar::hit(&bar_rects(), x, y)) {
                (WM_LBUTTONDOWN, Hit::Tab(i)) => {
                    select(i);
                    with(|s| s.win.drag = Some(Drag { tab: i, from: POINT { x, y }, moved: false }));
                    SetCapture(h);
                }
                (WM_LBUTTONDOWN, Hit::NewTab) => new_tab(),
                (WM_LBUTTONDOWN, Hit::Menu) => show_menu(None),
                (_, Hit::TabClose(i)) | (WM_MBUTTONUP, Hit::Tab(i)) => close_tab(i),
                _ => {}
            }
            0
        }
        WM_SIZE => {
            layout();
            if wp == SIZE_MAXIMIZED as usize || wp == SIZE_RESTORED as usize {
                schedule_save();
            }
            0
        }
        WM_SETFOCUS => {
            if let Some(e) = try_with(|s| s.win.tabs.get(s.win.cur).map(|t| t.edit)) {
                SetFocus(e);
            }
            0
        }
        WM_COMMAND => {
            let (id, code) = ((wp & 0xFFFF) as u16, ((wp >> 16) & 0xFFFF) as u32);
            if lp != 0 {
                // An editor changed: update its tab label when the modified state flipped.
                if code == EN_CHANGE {
                    schedule_save();
                    let e = lp as HWND;
                    if let Some(i) = try_with(|s| s.win.tabs.iter().position(|t| t.edit == e && t.dirty != ed(e).dirty())) {
                        refresh(i);
                    }
                }
                return 0;
            }
            let (cur, n) = with(|s| (s.win.cur, s.win.tabs.len()));
            match id {
                ID_NEW => new_tab(),
                ID_OPEN => file_dialog(false, None).into_iter().for_each(|p| open_path(p)),
                ID_SAVE => _ = save(cur, false),
                ID_SAVEAS => _ = save(cur, true),
                ID_CLOSE => close_tab(cur),
                ID_NEXT => select((cur + 1) % n),
                ID_PREV => select((cur + n - 1) % n),
                ID_SELALL => {
                    let e = ed(cur_edit());
                    e.select(0, e.text.len());
                }
                ID_EXIT => _ = SendMessageW(h, WM_CLOSE, 0, 0),
                ID_FIND => find_dialog(false),
                ID_REPLACE => find_dialog(true),
                ID_FINDNEXT => find_again(),
                ID_SETTINGS => open_settings(),
                ID_REOPEN => reopen_tab(),
                ID_GOTO => go_to_line(),
                ID_HELP => help::toggle(h, with(|s| s.dpi)),
                ID_QUICKOPEN => quick_open(),
                ID_ZOOMRESET => {
                    with(|s| s.pt = s.size);
                    set_fonts();
                    schedule_save();
                }
                ID_SELNEXT => ed(cur_edit()).command(edit::Command::SelectNext),
                ID_DUPLINE => ed(cur_edit()).command(edit::Command::DuplicateLines),
                ID_DELLINE => ed(cur_edit()).command(edit::Command::DeleteLines),
                ID_MOVEUP => ed(cur_edit()).command(edit::Command::MoveUp),
                ID_MOVEDOWN => ed(cur_edit()).command(edit::Command::MoveDown),
                ID_WRAP => {
                    // Until the settings file is saved again or the editor restarts.
                    with(|s| s.opts.word_wrap = !s.opts.word_wrap);
                    set_fonts();
                }
                ID_TAB1..=108 if ((id - ID_TAB1) as usize) < n => select((id - ID_TAB1) as usize),
                _ => {}
            }
            0
        }
        WM_DROPFILES => {
            let drop = wp as HDROP;
            let mut files = Vec::new();
            for k in 0..DragQueryFileW(drop, u32::MAX, null_mut(), 0) {
                let mut buf = vec![0u16; DragQueryFileW(drop, k, null_mut(), 0) as usize + 1];
                let n = DragQueryFileW(drop, k, buf.as_mut_ptr(), buf.len() as u32);
                files.push(PathBuf::from(String::from_utf16_lossy(&buf[..n as usize])));
            }
            DragFinish(drop);
            files.into_iter().for_each(|p| open_path(p));
            0
        }
        WM_DPICHANGED => {
            with(|s| s.dpi = (wp & 0xFFFF) as u32);
            set_fonts();
            let r = &*(lp as *const RECT);
            SetWindowPos(h, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            0
        }
        WM_CLOSE => {
            if with(|s| s.parked.is_empty()) {
                // The last window: unsaved work is kept in the session, so closing never prompts.
                save_session();
                DestroyWindow(h);
            } else if close_window_tabs() {
                DestroyWindow(h);
            }
            0
        }
        WM_CLOSE_EMPTY => {
            if with(|s| s.win.tabs.is_empty()) {
                DestroyWindow(h);
            }
            0
        }
        WM_NCDESTROY => {
            with(|s| s.win = s.parked.pop().unwrap_or(Win::new(null_mut())));
            schedule_save();
            0
        }
        WM_QUERYENDSESSION => {
            save_session();
            1
        }
        // Alt or Alt+letter opens our menu; Alt+Space still opens the system menu.
        WM_SYSCOMMAND if (wp & 0xFFF0) as u32 == SC_KEYMENU && lp != b' ' as isize => {
            show_menu(Some(lp as u16));
            0
        }
        WM_DESTROY => {
            if with(|s| s.parked.is_empty()) {
                PostQuitMessage(0);
            }
            0
        }
        _ if m == FINDMSG.load(Relaxed) => {
            on_find(lp as *const FINDREPLACEW);
            0
        }
        _ => DefWindowProcW(h, m, wp, lp),
    }
}

/// Window-level shortcuts. Each must also be listed in the Ctrl+/ overview (see the test).
fn accelerators() -> Vec<ACCEL> {
    let (c, cs) = (FCONTROL | FVIRTKEY, FCONTROL | FSHIFT | FVIRTKEY);
    let mut accels = vec![
        ACCEL { fVirt: c, key: b'T' as u16, cmd: ID_NEW },
        ACCEL { fVirt: c, key: b'N' as u16, cmd: ID_NEW },
        ACCEL { fVirt: c, key: b'O' as u16, cmd: ID_OPEN },
        ACCEL { fVirt: c, key: b'S' as u16, cmd: ID_SAVE },
        ACCEL { fVirt: cs, key: b'S' as u16, cmd: ID_SAVEAS },
        ACCEL { fVirt: c, key: b'W' as u16, cmd: ID_CLOSE },
        ACCEL { fVirt: cs, key: b'T' as u16, cmd: ID_REOPEN },
        ACCEL { fVirt: c, key: b'A' as u16, cmd: ID_SELALL },
        ACCEL { fVirt: c, key: b'F' as u16, cmd: ID_FIND },
        ACCEL { fVirt: c, key: b'H' as u16, cmd: ID_REPLACE },
        ACCEL { fVirt: FVIRTKEY, key: VK_F3, cmd: ID_FINDNEXT },
        ACCEL { fVirt: FALT | FVIRTKEY, key: b'Z' as u16, cmd: ID_WRAP },
        ACCEL { fVirt: c, key: b'G' as u16, cmd: ID_GOTO },
        ACCEL { fVirt: c, key: b'P' as u16, cmd: ID_QUICKOPEN },
        ACCEL { fVirt: c, key: b'0' as u16, cmd: ID_ZOOMRESET },
        ACCEL { fVirt: c, key: VK_NUMPAD0, cmd: ID_ZOOMRESET },
        ACCEL { fVirt: c, key: VK_TAB, cmd: ID_NEXT },
        ACCEL { fVirt: cs, key: VK_TAB, cmd: ID_PREV },
        ACCEL { fVirt: c, key: VK_NEXT, cmd: ID_NEXT },
        ACCEL { fVirt: c, key: VK_PRIOR, cmd: ID_PREV },
    ];
    accels.extend((0..9).map(|k| ACCEL { fVirt: FALT | FVIRTKEY, key: b'1' as u16 + k, cmd: ID_TAB1 + k }));
    accels
}

/// How a shortcut is written in the overview, e.g. "Ctrl+Shift+S".
#[cfg(test)]
fn accel_name(a: &ACCEL) -> String {
    let mut s = String::new();
    for (flag, name) in [(FCONTROL, "Ctrl+"), (FSHIFT, "Shift+"), (FALT, "Alt+")] {
        if a.fVirt & flag != 0 {
            s += name;
        }
    }
    s + &match a.key {
        VK_TAB => "Tab".into(),
        VK_NEXT => "PgDn".into(),
        VK_PRIOR => "PgUp".into(),
        VK_F3 => "F3".into(),
        VK_NUMPAD0 => "Num0".into(),
        k => char::from(k as u8).to_string(),
    }
}

fn main() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let hinst = GetModuleHandleW(null());
        let class = wide(&instance_name());
        let args: Vec<PathBuf> = std::env::args_os().skip(1).map(|p| {
            let p = PathBuf::from(p);
            std::path::absolute(&p).unwrap_or(p)
        }).collect();
        // One window per user: a second launch hands its files to the first one.
        CreateMutexW(null(), 0, wide(&format!("Local\\{}", instance_name())).as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS && hand_over(&class, &args) {
            return;
        }
        let class = class.as_ptr();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            lpszClassName: class,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hIcon: LoadIconW(hinst, 1 as _),
            hbrBackground: (COLOR_WINDOW + 1) as isize as HBRUSH,
            ..zeroed()
        };
        RegisterClassW(&wc);
        edit::register();
        load_settings();
        load_recent();

        let bar = CreatePopupMenu();
        for (title, items) in [
            ("&File", &[
                (ID_NEW, "&New Tab\tCtrl+T"),
                (ID_OPEN, "&Open...\tCtrl+O"),
                (ID_QUICKOPEN, "&Quick Open...\tCtrl+P"),
                (ID_SAVE, "&Save\tCtrl+S"),
                (ID_SAVEAS, "Save &As...\tCtrl+Shift+S"),
                (ID_CLOSE, "&Close Tab\tCtrl+W"),
                (ID_REOPEN, "&Reopen Closed Tab\tCtrl+Shift+T"),
                (0, ""),
                (ID_EXIT, "E&xit"),
            ][..]),
            ("&Edit", &[
                (ID_FIND, "&Find...\tCtrl+F"),
                (ID_FINDNEXT, "Find &Next\tF3"),
                (ID_REPLACE, "&Replace...\tCtrl+H"),
                (0, ""),
                (ID_SELALL, "Select &All\tCtrl+A"),
                (ID_WRAP, "&Word Wrap\tAlt+Z"),
                (0, ""),
                (ID_SELNEXT, "Select Ne&xt Match\tCtrl+D"),
                (ID_DUPLINE, "&Duplicate Line\tCtrl+Shift+D"),
                (ID_DELLINE, "De&lete Line\tCtrl+Shift+K"),
                (ID_MOVEUP, "Move Line &Up\tAlt+Up"),
                (ID_MOVEDOWN, "Move Line D&own\tAlt+Down"),
                (ID_GOTO, "&Go to Line...\tCtrl+G"),
            ]),
            ("&Help", &[(ID_HELP, "&Keyboard Shortcuts\tCtrl+/")][..]),
        ] {
            let popup = CreatePopupMenu();
            for &(id, text) in items {
                if id == 0 {
                    AppendMenuW(popup, MF_SEPARATOR, 0, null());
                } else {
                    AppendMenuW(popup, MF_STRING, id as usize, wide(text).as_ptr());
                }
            }
            AppendMenuW(bar, MF_POPUP, popup as usize, wide(title).as_ptr());
        }
        AppendMenuW(bar, MF_STRING, ID_SETTINGS as usize, w!("&Settings"));
        with(|s| s.menu = bar);
        FINDMSG.store(RegisterWindowMessageW(FINDMSGSTRINGW), Relaxed);

        let hwnd = create_window();

        let mut fr: FINDREPLACEW = zeroed();
        fr.lStructSize = size_of::<FINDREPLACEW>() as u32;
        fr.hwndOwner = hwnd;
        fr.Flags = FR_DOWN | FR_ENABLEHOOK;
        fr.lpfnHook = Some(find_hook);
        fr.lpstrFindWhat = Box::leak(Box::new([0u16; 256])).as_mut_ptr();
        fr.lpstrReplaceWith = Box::leak(Box::new([0u16; 256])).as_mut_ptr();
        (fr.wFindWhatLen, fr.wReplaceWithLen) = (256, 256);
        with(|s| s.fr = Box::into_raw(Box::new(fr)));

        let accels = accelerators();
        let haccel = CreateAcceleratorTableW(accels.as_ptr(), accels.len() as i32);

        restore_session();
        args.into_iter().for_each(|p| open_path(p));
        if with(|s| s.win.tabs.is_empty()) {
            new_tab();
        }
        // Position and show in one call so the launcher's show command (e.g. "Run: Minimized") still applies.
        let mut si: STARTUPINFOW = zeroed();
        GetStartupInfoW(&mut si);
        let show = if si.dwFlags & STARTF_USESHOWWINDOW != 0 { si.wShowWindow as SHOW_WINDOW_CMD } else { SW_SHOWNORMAL };
        enter(hwnd);
        show_placed(hwnd, with(|s| s.win.place), if with(|s| s.win.place.is_some()) { show } else { SW_SHOWDEFAULT });
        let minimized = matches!(show, SW_MINIMIZE | SW_SHOWMINIMIZED | SW_SHOWMINNOACTIVE);
        for (h, place) in with(|s| std::mem::take(&mut s.later)) {
            show_placed(h, place, if minimized { SW_SHOWMINNOACTIVE } else { SW_SHOWNOACTIVATE });
        }

        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            let dlg = with(|s| s.fdlg);
            if !dlg.is_null() && IsDialogMessageW(dlg, &msg) != 0 {
                continue;
            }
            // Ctrl+/ (whatever key types '/') toggles the shortcut overview.
            if GetKeyState(VK_CONTROL as i32) < 0 && help::is_slash(&msg) {
                let (main, dpi) = with(|s| (s.win.main, s.dpi));
                help::toggle(main, dpi);
                continue;
            }
            // Ctrl+wheel zooms instead of scrolling.
            if msg.message == WM_MOUSEWHEEL && GetKeyState(VK_CONTROL as i32) < 0 {
                zoom((msg.wParam >> 16) as i16 as i32);
            } else if TranslateAcceleratorW(hwnd, haccel, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_accelerator_is_in_the_overview() {
        let overview: String = help::GROUPS.iter().flat_map(|g| g.1.iter().map(|r| r.0)).collect::<Vec<_>>().join(" | ");
        for a in accelerators() {
            let name = accel_name(&a);
            // "Alt+1 … Alt+9" stands for all tab numbers.
            let covered = overview.contains(&name) || (name.starts_with("Alt+") && overview.contains("Alt+1 … Alt+9") && name.len() == 5);
            assert!(covered, "{name} is missing from the Ctrl+/ overview (src/help.rs)");
        }
    }

    #[test]
    fn theme_setting() {
        assert!(theme_is_dark("dark", false));
        assert!(!theme_is_dark("light", true));
        assert!(theme_is_dark("system", true));
        assert!(!theme_is_dark("system", false));
    }

    #[test]
    fn session_formats() {
        // Version 1: one window, header "cur pt max left top right bottom".
        let v1 = b"minedit-session-1\n1 13 0 1 2 3 4\n1 0 0 2\t\nhi0 0 1 0 5\tC:\\a.md\n";
        let (pt, w) = parse_session(v1).unwrap();
        assert_eq!((pt, w.len(), w[0].cur, w[0].tabs.len()), (13, 1, 1, 2));
        assert_eq!((w[0].tabs[0].body.as_slice(), w[0].tabs[1].lang, w[0].tabs[1].lf), (b"hi".as_slice(), 5, true));
        // Version 2: several windows.
        let v2 = b"minedit-session-2\n11\nW 0 1 1 2 3 4\n1 0 0 1 0\t\naW 1 0 5 6 7 8\n0 0 0 0 0\tC:\\x\n0 0 0 0 0\tC:\\y\n";
        let (pt, w) = parse_session(v2).unwrap();
        assert_eq!((pt, w.len()), (11, 2));
        assert_eq!((w[0].tabs.len(), w[1].tabs.len(), w[1].cur), (1, 2, 1));
        assert!(w[0].place.unwrap().1);
        assert_eq!(w[1].place.unwrap().0.left, 5);
        assert!(parse_session(b"something else\n").is_none());
    }

    #[test]
    fn go_to_input() {
        assert_eq!(parse_goto("12"), Some((12, 1)));
        assert_eq!(parse_goto(" 12 : 5 "), Some((12, 5)));
        assert_eq!(parse_goto("3,4"), Some((3, 4)));
        assert_eq!(parse_goto("x"), None);
        assert_eq!(parse_goto("1:2:3"), None);
    }

    #[test]
    fn paths_survive_copydata_encoding() {
        let paths = [PathBuf::from(r"C:\a b\ü.txt"), PathBuf::from(r"D:\x.json")];
        assert_eq!(decode_paths(&encode_paths(&paths)), paths);
        assert!(decode_paths(&encode_paths(&[])).is_empty());
    }
}
