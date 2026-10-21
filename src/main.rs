#![cfg_attr(not(test), windows_subsystem = "windows")]

mod edit;
mod syntax;
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
        System::{LibraryLoader::GetModuleHandleW, Threading::*},
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
const MAX_CLOSED: usize = 50;
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
";

#[derive(Clone, Copy)]
enum Enc { Utf8, Utf8Bom, Utf16, Ansi }

struct Tab {
    edit: HWND,
    path: Option<PathBuf>,
    dirty: bool, // as last shown in the tab label
    enc: Enc,
    lf: bool, // file used bare LF line endings
    stamp: Option<SystemTime>, // modification time of the file as last loaded or saved
}

/// A closed tab that Ctrl+Shift+T can bring back.
struct Closed {
    path: Option<PathBuf>,
    text: Vec<u16>, // kept only for unsaved tabs; clean files are reloaded from disk
    enc: Enc,
    lf: bool,
    dirty: bool,
    caret: usize,
}

struct State {
    main: HWND,
    menu: HMENU, // popup opened by the ≡ button and Alt
    efont: HFONT,
    ufont: HFONT,
    ifont: HFONT,  // caption button glyphs
    ifont_small: HFONT,
    hover: Hit,   // title bar element under the mouse
    pressed: Hit, // caption button being clicked
    active: bool, // window is active
    checking: bool, // a file-changed prompt is open
    wrap_rect: RECT, // "Word Wrap" in the status bar, clickable
    tabs: Vec<Tab>,
    closed: Vec<Closed>, // most recently closed last
    cur: usize,
    dpi: u32,
    face: String,
    size: i32,  // font size from the settings file
    pt: i32,    // current font size, changed by Ctrl+wheel
    wheel: i32, // accumulated Ctrl+wheel delta, for high-resolution wheels and touchpads
    opts: edit::Options,
    place: Option<(RECT, bool)>, // restored window rect and maximized state
    fdlg: HWND,                  // open Find/Replace dialog
    fdlg_title: String,          // its original (localized) title, before the match count
    fr: *mut FINDREPLACEW,       // lives for the whole process, as the modeless dialog requires
}

thread_local! {
    static S: RefCell<State> = const { RefCell::new(State {
        main: null_mut(), menu: null_mut(), efont: null_mut(), ufont: null_mut(), ifont: null_mut(), ifont_small: null_mut(),
        hover: Hit::None, pressed: Hit::None, active: true, checking: false, wrap_rect: RECT { left: 0, top: 0, right: 0, bottom: 0 }, tabs: Vec::new(), closed: Vec::new(), cur: 0,
        dpi: 96, face: String::new(), size: 11, pt: 11, wheel: 0, opts: edit::Options { tab_width: 4, use_spaces: false, line_numbers: true, highlight_line: true, margin: 8, word_wrap: false }, place: None,
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
        (s.face, s.size) = ("Consolas".into(), 11);
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
        (old, s.tabs.iter().map(|t| t.edit).collect::<Vec<_>>())
    });
    for e in edits {
        PostMessageW(e, edit::UPDATE, 0, 0);
    }
    for f in old.into_iter().filter(|f| !f.is_null()) {
        DeleteObject(f);
    }
    layout();
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
    }
}

fn cur_edit() -> HWND {
    with(|s| s.tabs[s.cur].edit)
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
    let owner = with(|s| if s.fdlg.is_null() { s.main } else { s.fdlg });
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
    let (old, fr, edit) = with(|s| (std::mem::replace(&mut s.fdlg, null_mut()), s.fr, s.tabs[s.cur].edit));
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
    let (dlg, title, edit) = with(|s| (s.fdlg, s.fdlg_title.clone(), s.tabs.get(s.cur).map(|t| t.edit)));
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

fn lang_of(p: Option<&PathBuf>) -> syntax::Lang {
    p.and_then(|p| p.extension()).map_or(syntax::Lang::Plain, |e| syntax::Lang::from_ext(&e.to_string_lossy()))
}

fn stamp(p: &PathBuf) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Reloads tabs whose file was changed by another program; asks first when a tab has unsaved changes.
unsafe fn check_files() {
    if with(|s| std::mem::replace(&mut s.checking, true)) {
        return;
    }
    for i in 0..with(|s| s.tabs.len()) {
        let Some((path, old, edit, main)) = with(|s| s.tabs.get(i).and_then(|t| Some((t.path.clone()?, t.stamp, t.edit, s.main)))) else { continue };
        let now = stamp(&path);
        if now.is_none() || now == old {
            continue; // unchanged, or deleted: keep the text so it can be saved again
        }
        with(|s| s.tabs[i].stamp = now); // ask only once per change
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
        let t = &mut s.tabs[i];
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
    let (main, dpi, n) = with(|s| (s.main, s.dpi, s.tabs.len()));
    let mut rc: RECT = unsafe { zeroed() };
    unsafe { GetClientRect(main, &mut rc) };
    titlebar::layout(rc.right, dpi, n)
}

/// Editors fill the window below the title bar.
unsafe fn layout() {
    let Some((main, edits)) = try_with(|s| Some((s.main, s.tabs.iter().map(|t| t.edit).collect::<Vec<_>>()))) else { return };
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
    InvalidateRect(with(|s| s.main), null(), 0);
}

fn status_height() -> i32 {
    24 * with(|s| s.dpi) as i32 / 96
}

/// Status bar: caret position on the left; word wrap and language on the right.
unsafe fn paint_status(dc: HDC, w: i32, sh: i32) {
    let (edit, font, dpi, wrap) = with(|s| (s.tabs.get(s.cur).map(|t| t.edit), s.ufont, s.dpi, s.opts.word_wrap));
    let p = |v: i32| v * dpi as i32 / 96;
    let fill = |r: RECT, c: u32| {
        let b = CreateSolidBrush(c);
        FillRect(dc, &r, b);
        DeleteObject(b);
    };
    fill(RECT { left: 0, top: 0, right: w, bottom: sh }, 0xF3F3F3);
    fill(RECT { left: 0, top: 0, right: w, bottom: 1 }, 0xE5E5E5);
    let Some(edit) = edit else { return };
    let e = ed(edit);
    let (line, col) = e.caret();
    SelectObject(dc, font);
    SetBkMode(dc, TRANSPARENT as _);
    SetTextColor(dc, 0x5A5A5A);
    let draw = |text: &str, x: i32| {
        let t: Vec<u16> = text.encode_utf16().collect();
        let mut sz: SIZE = zeroed();
        GetTextExtentPoint32W(dc, t.as_ptr(), t.len() as i32, &mut sz);
        TextOutW(dc, x, (sh - sz.cy) / 2, t.as_ptr(), t.len() as i32);
        sz.cx
    };
    draw(&format!("Ln {line}, Col {col}"), p(12));
    let mut x = w - p(12);
    for (i, item) in [e.lang().name(), if wrap { "Word Wrap: On" } else { "Word Wrap: Off" }].into_iter().enumerate() {
        let t: Vec<u16> = item.encode_utf16().collect();
        let mut sz: SIZE = zeroed();
        GetTextExtentPoint32W(dc, t.as_ptr(), t.len() as i32, &mut sz);
        x -= sz.cx;
        draw(item, x);
        if i == 1 {
            with(|s| s.wrap_rect = RECT { left: x - p(6), top: 0, right: x + sz.cx + p(6), bottom: sh });
        }
        x -= p(24);
    }
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
        let tabs: Vec<(String, bool)> = s.tabs.iter().map(|t| (name(t), t.dirty)).collect();
        let look = titlebar::Look {
            font: s.ufont, icons: s.ifont, small_icons: s.ifont_small, dpi: s.dpi,
            active: s.active, maximized: IsZoomed(h) != 0, hover: s.hover,
        };
        (tabs, s.cur, look)
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
    if with(|s| std::mem::replace(&mut s.hover, hit)) != hit {
        repaint_bar();
    }
}

/// Opens the menu under the ≡ button. `key`: Some(0) for plain Alt (highlight the first item),
/// Some(letter) for Alt+letter (open that submenu), None for a mouse click.
unsafe fn show_menu(key: Option<u16>) {
    let (main, menu) = with(|s| (s.main, s.menu));
    let r = bar_rects();
    let mut pt = POINT { x: r.menu.right, y: r.height };
    ClientToScreen(main, &mut pt);
    // The menu loop reads these from the queue as if they were typed.
    match key {
        Some(0) => _ = PostMessageW(main, WM_KEYDOWN, VK_DOWN as usize, 0),
        Some(c) => _ = PostMessageW(main, WM_CHAR, c as usize, 0),
        None => {}
    }
    with(|s| s.hover = Hit::Menu);
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
        let t = s.tabs.get_mut(i)?;
        t.dirty = ed(t.edit).dirty();
        let n = name(t);
        let mark = if t.dirty { "*" } else { "" };
        let full = t.path.as_ref().map_or(n.clone(), |p| p.display().to_string());
        Some((s.main, format!("{mark}{full} - {APP}"), s.cur == i))
    }) else { return };
    repaint_bar();
    if is_cur {
        SetWindowTextW(main, wide(&title).as_ptr());
    }
}

unsafe fn select(i: usize) {
    let (old, new) = with(|s| {
        let old = s.tabs.get(s.cur).map(|t| t.edit);
        s.cur = i;
        (old, s.tabs[i].edit)
    });
    layout();
    ShowWindow(new, SW_SHOW);
    if let Some(old) = old.filter(|&o| o != new) {
        ShowWindow(old, SW_HIDE);
    }
    SetFocus(new);
    refresh(i);
}

unsafe fn add_tab(path: Option<PathBuf>, text: Vec<u16>, enc: Enc, lf: bool, dirty: bool) {
    let edit = edit::create(with(|s| s.main));
    ed(edit).set_text(text);
    ed(edit).set_lang(lang_of(path.as_ref()));
    if dirty {
        ed(edit).mark_dirty();
    }
    let i = with(|s| {
        let stamp = path.as_ref().and_then(stamp);
        s.tabs.push(Tab { edit, path, dirty, enc, lf, stamp });
        s.tabs.len() - 1
    });
    select(i);
}

unsafe fn new_tab() {
    add_tab(None, Vec::new(), Enc::Utf8, false, false);
}

fn session_path() -> Option<PathBuf> {
    Some(app_dir()?.join("session"))
}

/// Session format: header line, "current-tab font-pt maximized left top right bottom", then per tab
/// "dirty enc lf len\tpath\n" followed by `len` bytes of UTF-8 text (only for dirty tabs).
unsafe fn save_session() {
    let Some(p) = session_path() else { return };
    let (tabs, cur, pt, main) = with(|s| (s.tabs.iter().map(|t| (t.edit, t.path.clone(), t.enc, t.lf)).collect::<Vec<_>>(), s.cur, s.pt, s.main));
    let mut wp: WINDOWPLACEMENT = zeroed();
    wp.length = size_of::<WINDOWPLACEMENT>() as u32;
    GetWindowPlacement(main, &mut wp);
    let max = wp.showCmd == SW_SHOWMAXIMIZED as _
        || (wp.showCmd == SW_SHOWMINIMIZED as _ && wp.flags & WPF_RESTORETOMAXIMIZED != 0);
    let RECT { left, top, right, bottom } = wp.rcNormalPosition;
    let mut out = format!("{APP}-session-1\n{cur} {pt} {} {left} {top} {right} {bottom}\n", max as u8).into_bytes();
    for (edit, path, enc, lf) in tabs {
        let e = ed(edit);
        let body = if e.dirty() { String::from_utf16_lossy(&e.text) } else { String::new() };
        let path = path.map_or(String::new(), |p| p.to_string_lossy().into());
        out.extend(format!("{} {} {} {}\t{path}\n", e.dirty() as u8, enc as u8, lf as u8, body.len()).bytes());
        out.extend(body.bytes());
    }
    let tmp = p.with_extension("tmp");
    if std::fs::create_dir_all(p.parent().unwrap()).is_ok() && std::fs::write(&tmp, out).is_ok() {
        _ = std::fs::rename(tmp, p);
    }
}

unsafe fn restore_session() -> Option<()> {
    fn line(rest: &mut &[u8]) -> Option<String> {
        let i = rest.iter().position(|&b| b == b'\n')?;
        let l = String::from_utf8_lossy(&rest[..i]).into_owned();
        *rest = &rest[i + 1..];
        Some(l)
    }
    let data = std::fs::read(session_path()?).ok()?;
    let mut rest = data.as_slice();
    (line(&mut rest)? == format!("{APP}-session-1")).then_some(())?;
    let l = line(&mut rest)?;
    let nums = l.split('\t').next()?; // older sessions also stored the font face here
    let v: Vec<i32> = nums.split(' ').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    let [cur, pt, ref place @ ..] = v[..] else { return None };
    let cur = cur.max(0) as usize;
    with(|s| s.pt = pt.clamp(6, 72));
    set_fonts();
    if let [max, left, top, right, bottom] = *place {
        with(|s| s.place = Some((RECT { left, top, right, bottom }, max == 1)));
    }
    while let Some(l) = line(&mut rest) {
        let (meta, path) = l.split_once('\t')?;
        let n: Vec<usize> = meta.split(' ').map(|x| x.parse().ok()).collect::<Option<_>>()?;
        let [dirty, enc, lf, len] = n[..] else { return None };
        let body = rest.get(..len)?;
        rest = &rest[len..];
        let path = (!path.is_empty()).then(|| PathBuf::from(path));
        let enc = [Enc::Utf8, Enc::Utf8Bom, Enc::Utf16, Enc::Ansi].get(enc).copied()?;
        if dirty == 1 {
            let text: Vec<u16> = String::from_utf8_lossy(body).encode_utf16().collect();
            add_tab(path, edit::to_lf(&text), enc, lf == 1, true);
        } else if let Some(p) = path {
            // Unmodified files are reloaded from disk; ones that no longer exist are dropped.
            if let Ok((text, enc, lf)) = load(&p) {
                add_tab(Some(p), text, enc, lf, false);
            }
        } else {
            new_tab();
        }
    }
    let n = with(|s| s.tabs.len());
    (n > 0).then(|| select(cur.min(n - 1)))
}

unsafe fn error(msg: String) {
    let main = with(|s| s.main);
    MessageBoxW(main, wide(&msg).as_ptr(), wide(APP).as_ptr(), MB_ICONERROR);
}

unsafe fn open_path(p: PathBuf) {
    let p = std::path::absolute(&p).unwrap_or(p);
    if let Some(i) = with(|s| s.tabs.iter().position(|t| t.path.as_ref() == Some(&p))) {
        return select(i);
    }
    let (text, enc, lf) = match load(&p) {
        Ok(r) => r,
        Err(e) => return error(format!("{}\n\n{e}", p.display())),
    };
    // Replace the current tab if it is an untouched, empty "Untitled".
    let reuse = with(|s| {
        let t = s.tabs.get(s.cur)?;
        let e = ed(t.edit);
        (t.path.is_none() && !e.dirty() && e.text.is_empty()).then_some(s.cur)
    });
    match reuse {
        Some(i) => {
            let edit = with(|s| {
                let t = &mut s.tabs[i];
                (t.stamp, t.enc, t.lf) = (stamp(&p), enc, lf);
                t.path = Some(p);
                t.edit
            });
            ed(edit).set_text(text);
            ed(edit).set_lang(lang_of(with(|s| s.tabs[i].path.clone()).as_ref()));
            refresh(i);
        }
        None => add_tab(Some(p), text, enc, lf, false),
    }
}

/// Shows the common file dialog. Open supports multi-select.
unsafe fn file_dialog(save: bool, initial: Option<&PathBuf>) -> Vec<PathBuf> {
    let main = with(|s| s.main);
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
        let t = &s.tabs[i];
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
    ed(edit).set_lang(lang_of(Some(&path)));
    with(|s| s.tabs[i].stamp = stamp(&path));
    let is_settings = Some(&path) == settings_path().as_ref();
    with(|s| s.tabs[i].path = Some(path));
    refresh(i);
    if is_settings {
        load_settings();
        set_fonts();
    }
    true
}

/// Asks to save a dirty tab. Returns false if the user cancelled.
unsafe fn confirm(i: usize) -> bool {
    let (main, edit, n) = with(|s| (s.main, s.tabs[i].edit, name(&s.tabs[i])));
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
    if i >= with(|s| s.tabs.len()) || !confirm(i) {
        return;
    }
    let (edit, next, empty) = with(|s| {
        let t = s.tabs.remove(i);
        let e = ed(t.edit);
        if t.path.is_some() || !e.text.is_empty() {
            let text = if e.dirty() { e.text.clone() } else { Vec::new() };
            s.closed.push(Closed { path: t.path, text, enc: t.enc, lf: t.lf, dirty: e.dirty(), caret: e.sel().start });
            if s.closed.len() > MAX_CLOSED {
                s.closed.remove(0);
            }
        }
        let next = if i < s.cur { s.cur - 1 } else if i == s.cur { i.min(s.tabs.len().saturating_sub(1)) } else { s.cur };
        s.cur = next;
        (t.edit, next, s.tabs.is_empty())
    });
    DestroyWindow(edit);
    if empty { new_tab() } else { select(next) }
}

/// Ctrl+Shift+T: reopens the most recently closed tab.
unsafe fn reopen_tab() {
    let Some(c) = with(|s| s.closed.pop()) else { return };
    // An untouched empty "Untitled" tab is replaced, like when opening a file.
    let pristine = with(|s| {
        let t = s.tabs.get(s.cur)?;
        let e = ed(t.edit);
        (t.path.is_none() && !e.dirty() && e.text.is_empty()).then_some(s.cur)
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
    let e = ed(cur_edit());
    let p = c.caret.min(e.text.len());
    e.select(p, p);
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_CREATE => {
            with(|s| (s.main, s.dpi) = (h, GetDpiForWindow(h)));
            set_fonts();
            DragAcceptFiles(h, 1);
            SetTimer(h, 1, 2000, None); // look for files changed by other programs
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
            with(|s| s.active = active);
            repaint_bar();
            if active {
                PostMessageW(h, WM_TIMER, 1, 0); // check files once activation is done
            }
            DefWindowProcW(h, m, wp, lp)
        }
        edit::CARET => {
            repaint_bar();
            update_count();
            0
        }
        WM_TIMER => {
            if with(|s| s.active) {
                check_files();
            }
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
                WM_NCLBUTTONDOWN => with(|s| s.pressed = hit),
                WM_NCLBUTTONUP if with(|s| std::mem::replace(&mut s.pressed, Hit::None)) == hit => {
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
            let wrap = with(|s| s.wrap_rect);
            let y_status = y - (rc.bottom - status_height());
            if m == WM_LBUTTONDOWN && x >= wrap.left && x < wrap.right && y_status >= 0 && y_status < wrap.bottom {
                SendMessageW(h, WM_COMMAND, ID_WRAP as usize, 0);
                return 0;
            }
            match (m, titlebar::hit(&bar_rects(), x, y)) {
                (WM_LBUTTONDOWN, Hit::Tab(i)) => select(i),
                (WM_LBUTTONDOWN, Hit::NewTab) => new_tab(),
                (WM_LBUTTONDOWN, Hit::Menu) => show_menu(None),
                (_, Hit::TabClose(i)) | (WM_MBUTTONUP, Hit::Tab(i)) => close_tab(i),
                _ => {}
            }
            0
        }
        WM_SIZE => {
            layout();
            0
        }
        WM_SETFOCUS => {
            if let Some(e) = try_with(|s| s.tabs.get(s.cur).map(|t| t.edit)) {
                SetFocus(e);
            }
            0
        }
        WM_COMMAND => {
            let (id, code) = ((wp & 0xFFFF) as u16, ((wp >> 16) & 0xFFFF) as u32);
            if lp != 0 {
                // An editor changed: update its tab label when the modified state flipped.
                if code == EN_CHANGE {
                    let e = lp as HWND;
                    if let Some(i) = try_with(|s| s.tabs.iter().position(|t| t.edit == e && t.dirty != ed(e).dirty())) {
                        refresh(i);
                    }
                }
                return 0;
            }
            let (cur, n) = with(|s| (s.cur, s.tabs.len()));
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
            // Unsaved work is kept in the session, so closing never prompts.
            save_session();
            DestroyWindow(h);
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
            PostQuitMessage(0);
            0
        }
        _ if m == FINDMSG.load(Relaxed) => {
            on_find(lp as *const FINDREPLACEW);
            0
        }
        _ => DefWindowProcW(h, m, wp, lp),
    }
}

fn main() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let hinst = GetModuleHandleW(null());
        let class = w!("minedit");
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

        let bar = CreatePopupMenu();
        for (title, items) in [
            ("&File", &[
                (ID_NEW, "&New Tab\tCtrl+T"),
                (ID_OPEN, "&Open...\tCtrl+O"),
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
            ]),
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

        let hwnd = CreateWindowExW(0, class, wide(APP).as_ptr(), WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, null_mut(), null_mut(), hinst, null());

        let mut fr: FINDREPLACEW = zeroed();
        fr.lStructSize = size_of::<FINDREPLACEW>() as u32;
        fr.hwndOwner = hwnd;
        fr.Flags = FR_DOWN | FR_ENABLEHOOK;
        fr.lpfnHook = Some(find_hook);
        fr.lpstrFindWhat = Box::leak(Box::new([0u16; 256])).as_mut_ptr();
        fr.lpstrReplaceWith = Box::leak(Box::new([0u16; 256])).as_mut_ptr();
        (fr.wFindWhatLen, fr.wReplaceWithLen) = (256, 256);
        with(|s| s.fr = Box::into_raw(Box::new(fr)));

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
            ACCEL { fVirt: c, key: VK_TAB, cmd: ID_NEXT },
            ACCEL { fVirt: cs, key: VK_TAB, cmd: ID_PREV },
            ACCEL { fVirt: c, key: VK_NEXT, cmd: ID_NEXT },
            ACCEL { fVirt: c, key: VK_PRIOR, cmd: ID_PREV },
        ];
        accels.extend((0..9).map(|k| ACCEL { fVirt: FALT | FVIRTKEY, key: b'1' as u16 + k, cmd: ID_TAB1 + k }));
        let haccel = CreateAcceleratorTableW(accels.as_ptr(), accels.len() as i32);

        restore_session();
        std::env::args_os().skip(1).for_each(|p| open_path(p.into()));
        if with(|s| s.tabs.is_empty()) {
            new_tab();
        }
        match with(|s| s.place) {
            // Position and show in one call so the launcher's show command (e.g. "Run: Minimized") still applies.
            Some((rect, max)) => {
                let mut si: STARTUPINFOW = zeroed();
                GetStartupInfoW(&mut si);
                let show = if si.dwFlags & STARTF_USESHOWWINDOW != 0 { si.wShowWindow as _ } else { SW_SHOWNORMAL };
                let minimized = matches!(show, SW_MINIMIZE | SW_SHOWMINIMIZED | SW_SHOWMINNOACTIVE);
                let mut wp: WINDOWPLACEMENT = zeroed();
                wp.length = size_of::<WINDOWPLACEMENT>() as u32;
                wp.rcNormalPosition = rect;
                wp.showCmd = (if max && !minimized { SW_SHOWMAXIMIZED } else { show }) as _;
                wp.flags = if max { WPF_RESTORETOMAXIMIZED } else { 0 };
                SetWindowPlacement(hwnd, &wp);
            }
            None => _ = ShowWindow(hwnd, SW_SHOWDEFAULT),
        }

        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            let dlg = with(|s| s.fdlg);
            if !dlg.is_null() && IsDialogMessageW(dlg, &msg) != 0 {
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
