//! Ctrl+F / Ctrl+H: a small find (and replace) bar over the top right corner of the editor, with match
//! case, regular expressions, previous/next and the number of matches. One per window.

use crate::{edit::ed, search::Query, theme};
use std::{cell::RefCell, mem::zeroed, ops::Range, ptr::{null, null_mut}};
use windows_sys::{
    w,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Controls::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
};

const ID_FIND: usize = 10;
const ID_REPL: usize = 11;
const ID_CASE: usize = 12;
const ID_REGEX: usize = 13;
const ID_PREV: usize = 14;
const ID_NEXT: usize = 15;
const ID_CLOSE: usize = 16;
const ID_REPLACE: usize = 17;
const ID_REPLACE_ALL: usize = 18;
const ID_COUNT: usize = 19;
const TIMER_COUNT: usize = 1; // counts again shortly after the text changed

/// Owner-drawn buttons: id, text, drawn with the icon font.
const BUTTONS: [(usize, &str, bool); 7] = [
    (ID_CASE, "Aa", false),
    (ID_REGEX, ".*", false),
    (ID_PREV, "\u{E74A}", true), // arrow up
    (ID_NEXT, "\u{E74B}", true), // arrow down
    (ID_CLOSE, "\u{E8BB}", true),
    (ID_REPLACE, "Replace", false),
    (ID_REPLACE_ALL, "All", false),
];

struct Bar {
    hwnd: HWND,
    owner: HWND, // the main window
    find: HWND,
    repl: HWND,
    count: HWND,
    buttons: Vec<HWND>, // as BUTTONS
    font: HFONT,
    icons: HFONT,
    bg: HBRUSH,
    edit_bg: HBRUSH,
    case: bool,
    regex: bool,
    replace: bool,               // the replace row is shown
    matches: Vec<Range<usize>>,  // in the current editor
    error: bool,                 // invalid regular expression
}

thread_local! {
    static BARS: RefCell<Vec<Bar>> = const { RefCell::new(Vec::new()) };
}

fn with<R>(f: impl Fn(&Bar) -> bool, g: impl FnOnce(&mut Bar) -> R) -> Option<R> {
    BARS.with(|b| b.borrow_mut().iter_mut().find(|b| f(b)).map(g))
}

fn of(owner: HWND) -> impl Fn(&Bar) -> bool {
    move |b| b.owner == owner
}

fn hwnd_is(h: HWND) -> impl Fn(&Bar) -> bool {
    move |b| b.hwnd == h
}

fn px(v: i32, dpi: u32) -> i32 {
    v * dpi as i32 / 96
}

unsafe fn text_of(h: HWND) -> Vec<u16> {
    let mut buf = vec![0u16; GetWindowTextLengthW(h) as usize + 1];
    let n = GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32);
    buf.truncate(n.max(0) as usize);
    buf
}

/// The query in the bar of window `owner`: None if empty, Err for an invalid regex.
unsafe fn query(owner: HWND) -> Option<Result<Query, String>> {
    let (find, case, regex) = with(of(owner), |b| (b.find, b.case, b.regex))?;
    Query::new(&text_of(find), case, false, regex, "")
}

/// The window's bar if it is open.
pub fn open_bar(owner: HWND) -> HWND {
    with(of(owner), |b| b.hwnd).filter(|&h| unsafe { IsWindowVisible(h) } != 0).unwrap_or(null_mut())
}

/// Finds all matches in the editor again and shows the count.
pub unsafe fn recount(owner: HWND) {
    let Some(edit) = crate::editor_of(owner) else { return };
    let q = query(owner);
    let (matches, error) = match &q {
        Some(Ok(q)) => (q.find(&ed(edit).text, None).into_iter().map(|m| m.0).collect(), false),
        Some(Err(_)) => (Vec::new(), true),
        None => (Vec::new(), false),
    };
    with(of(owner), |b| (b.matches, b.error) = (matches, error));
    show_count(owner);
}

/// "3 of 12" when the selection is a match, else "12 results".
pub unsafe fn show_count(owner: HWND) {
    let Some(edit) = crate::editor_of(owner) else { return };
    let sel = ed(edit).sel();
    let Some((count, text)) = with(of(owner), |b| {
        let text = if b.error {
            "Invalid regex".to_string()
        } else if b.matches.is_empty() {
            if text_of(b.find).is_empty() { String::new() } else { "No results".into() }
        } else if let Some(i) = b.matches.iter().position(|m| *m == sel) {
            format!("{} of {}", i + 1, b.matches.len())
        } else {
            format!("{} result{}", b.matches.len(), if b.matches.len() == 1 { "" } else { "s" })
        };
        (b.count, text)
    }) else { return };
    SetWindowTextW(count, crate::wide(&text).as_ptr());
}

/// Selects the next match after the selection (or the one before it), wrapping around.
pub unsafe fn go(owner: HWND, back: bool) {
    let Some(edit) = crate::editor_of(owner) else { return };
    let e = ed(edit);
    let sel = e.sel();
    let found = with(of(owner), |b| {
        if back {
            b.matches.iter().rev().find(|m| m.start < sel.start).or(b.matches.last()).cloned()
        } else {
            b.matches.iter().find(|m| m.start > sel.start || (m.start == sel.start && sel.is_empty())).or(b.matches.first()).cloned()
        }
    }).flatten();
    if let Some(m) = found {
        e.select(m.start, m.end);
    }
    show_count(owner);
}

/// While typing in the search field: the first match from the selection's start on.
unsafe fn incremental(owner: HWND) {
    recount(owner);
    let Some(edit) = crate::editor_of(owner) else { return };
    let e = ed(edit);
    let start = e.sel().start;
    let found = with(of(owner), |b| b.matches.iter().find(|m| m.start >= start).or(b.matches.first()).cloned()).flatten();
    if let Some(m) = found {
        e.select(m.start, m.end);
    }
    show_count(owner);
}

/// Replaces the selected match (then goes to the next one), or all matches as one undo step.
unsafe fn replace(owner: HWND, all: bool) {
    let (Some(Ok(q)), Some(edit), Some(repl)) = (query(owner), crate::editor_of(owner), with(of(owner), |b| b.repl)) else { return };
    let repl = text_of(repl);
    let e = ed(edit);
    let changes = q.find(&e.text, Some(&repl));
    if all {
        e.replace_each(changes);
    } else if let Some((r, t)) = changes.into_iter().find(|c| c.0 == e.sel()) {
        e.replace(r, t);
    }
    recount(owner);
    if !all {
        go(owner, false);
    }
}

/// Opens the bar of window `owner` (creating it), with `initial` searched for if given; `replace`
/// shows the replace row.
pub unsafe fn show(owner: HWND, font: HFONT, icons: HFONT, initial: &[u16], replace: bool) {
    if with(of(owner), |_| ()).is_none() {
        create(owner, font, icons);
    }
    // A selection that is already a match keeps the current search (e.g. a regular expression).
    recount(owner);
    let sel = crate::editor_of(owner).map(|e| ed(e).sel());
    let initial = if with(of(owner), |b| sel.is_some_and(|s| b.matches.contains(&s))) == Some(true) { &[][..] } else { initial };
    let Some((hwnd, find, repl)) = with(of(owner), |b| {
        b.replace = replace;
        (b.hwnd, b.find, b.repl)
    }) else { return };
    if !initial.is_empty() {
        let mut t = initial.to_vec();
        t.push(0);
        SetWindowTextW(find, t.as_ptr());
    }
    ShowWindow(repl, if replace { SW_SHOW } else { SW_HIDE });
    for &(id, _, _) in &BUTTONS[5..] {
        ShowWindow(GetDlgItem(hwnd, id as i32), if replace { SW_SHOW } else { SW_HIDE });
    }
    ShowWindow(hwnd, SW_SHOW);
    crate::layout(); // places the bar
    // Ctrl+H with something to search for goes on to the replacement.
    let field = if replace && GetWindowTextLengthW(find) > 0 { repl } else { find };
    SetFocus(field);
    SendMessageW(field, EM_SETSEL, 0, -1);
    recount(owner);
}

/// Closes the bar; the keyboard goes back to the editor.
pub unsafe fn close(owner: HWND) {
    if let Some(h) = with(of(owner), |b| b.hwnd) {
        ShowWindow(h, SW_HIDE);
        SetFocus(owner);
    }
}

/// Places the bar (if open) at the top right of `editor` (client coordinates of the owner).
pub unsafe fn place(owner: HWND, editor: RECT) {
    let Some((hwnd, replace)) = with(of(owner), |b| (b.hwnd, b.replace)) else { return };
    if IsWindowVisible(hwnd) == 0 {
        return;
    }
    let dpi = windows_sys::Win32::UI::HiDpi::GetDpiForWindow(owner);
    let p = |v| px(v, dpi);
    let (w, h) = (p(440).min(editor.right - editor.left - p(24)), if replace { p(70) } else { p(38) });
    SetWindowPos(hwnd, HWND_TOP, editor.right - w - p(20), editor.top, w, h, 0);
    layout(hwnd);
}

/// Whether window `h` is (part of) a find bar; returns that bar.
pub unsafe fn owner_bar(h: HWND) -> HWND {
    BARS.with(|b| b.borrow().iter().find(|b| h == b.hwnd || IsChild(b.hwnd, h) != 0).map_or(null_mut(), |b| b.hwnd))
}

unsafe fn create(owner: HWND, font: HFONT, icons: HFONT) {
    let hinst = GetModuleHandleW(null());
    let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: hinst, lpszClassName: w!("minedit_find"),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW), ..zeroed() };
    RegisterClassW(&wc); // fails harmlessly when already registered
    let hwnd = CreateWindowExW(WS_EX_CONTROLPARENT, w!("minedit_find"), null(), WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
        0, 0, 0, 0, owner, null_mut(), hinst, null());
    let child = |class: *const u16, style: u32, id: usize| {
        CreateWindowExW(0, class, null(), WS_CHILD | WS_VISIBLE | style, 0, 0, 0, 0, hwnd, id as HMENU, hinst, null())
    };
    let edit = |id, cue: &str| {
        let e = child(w!("EDIT"), ES_AUTOHSCROLL as u32 | WS_TABSTOP | WS_BORDER, id);
        SendMessageW(e, EM_SETCUEBANNER, 1, crate::wide(cue).as_ptr() as LPARAM);
        e
    };
    let find = edit(ID_FIND, "Find");
    let repl = edit(ID_REPL, "Replace");
    let count = child(w!("STATIC"), 0x200 /* SS_CENTERIMAGE */, ID_COUNT);
    let buttons = BUTTONS.iter().map(|&(id, _, _)| child(w!("BUTTON"), BS_OWNERDRAW as u32 | WS_TABSTOP, id)).collect();
    BARS.with(|b| b.borrow_mut().push(Bar {
        hwnd, owner, find, repl, count, buttons, font: null_mut(), icons: null_mut(), bg: null_mut(), edit_bg: null_mut(),
        case: false, regex: false, replace: false, matches: Vec::new(), error: false,
    }));
    set_fonts(hwnd, font, icons);
    recolor(hwnd);
}

/// Uses copies of the fonts (the editor replaces its own when settings change).
pub unsafe fn set_fonts(bar: HWND, font: HFONT, icons: HFONT) {
    let copy = |f: HFONT| {
        let mut lf: LOGFONTW = zeroed();
        GetObjectW(f, size_of::<LOGFONTW>() as i32, &mut lf as *mut _ as _);
        CreateFontIndirectW(&lf)
    };
    let (font, icons) = (copy(font), copy(icons));
    let Some((old, controls)) = with(hwnd_is(bar), |b| {
        let old = [std::mem::replace(&mut b.font, font), std::mem::replace(&mut b.icons, icons)];
        (old, [b.find, b.repl, b.count])
    }) else { return };
    for c in controls {
        SendMessageW(c, WM_SETFONT, font as WPARAM, 1);
    }
    for o in old.into_iter().filter(|o| !o.is_null()) {
        DeleteObject(o);
    }
}

/// Font changes for every window's bar.
pub unsafe fn set_all_fonts(font: HFONT, icons: HFONT) {
    for h in BARS.with(|b| b.borrow().iter().map(|b| b.hwnd).collect::<Vec<_>>()) {
        set_fonts(h, font, icons);
    }
}

/// Applies the current theme to every window's bar.
pub unsafe fn recolor_all() {
    for h in BARS.with(|b| b.borrow().iter().map(|b| b.hwnd).collect::<Vec<_>>()) {
        recolor(h);
    }
}

unsafe fn recolor(bar: HWND) {
    let c = theme::palette();
    let Some((edits, old)) = with(hwnd_is(bar), |b| {
        let old = [std::mem::replace(&mut b.bg, CreateSolidBrush(c.strip)), std::mem::replace(&mut b.edit_bg, CreateSolidBrush(c.bg))];
        ([b.find, b.repl], old)
    }) else { return };
    for e in edits {
        SetWindowTheme(e, if theme::is_dark() { w!("DarkMode_CFD") } else { w!("CFD") }, null());
    }
    RedrawWindow(bar, null(), null_mut(), RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
    for b in old.into_iter().filter(|b| !b.is_null()) {
        DeleteObject(b);
    }
}

unsafe fn layout(bar: HWND) {
    let Some((find, repl, count, buttons, font, replace)) = with(hwnd_is(bar), |b| (b.find, b.repl, b.count, b.buttons.clone(), b.font, b.replace)) else { return };
    let dpi = windows_sys::Win32::UI::HiDpi::GetDpiForWindow(bar);
    let p = |v| px(v, dpi);
    let mut rc: RECT = zeroed();
    GetClientRect(bar, &mut rc);
    let dc = GetDC(bar);
    let old = SelectObject(dc, font);
    let mut tm: TEXTMETRICW = zeroed();
    GetTextMetricsW(dc, &mut tm);
    SelectObject(dc, old);
    ReleaseDC(bar, dc);
    let (pad, rh, b) = (p(5), p(28), p(26));
    let eh = (tm.tmHeight + p(8)).min(rh);
    // Row 1: field, Aa, .*, count, up, down, close (right-aligned).
    let mut x = rc.right - pad;
    for (i, wdt) in [(4, b), (3, b), (2, b)] {
        x -= wdt;
        MoveWindow(buttons[i], x, pad, wdt, rh, 1);
    }
    x -= p(76);
    MoveWindow(count, x, pad, p(74), rh, 1);
    for i in [1, 0] {
        x -= b + p(2);
        MoveWindow(buttons[i], x, pad, b, rh, 1);
    }
    let fw = (x - pad * 2).max(p(60));
    MoveWindow(find, pad, pad + (rh - eh) / 2, fw, eh, 1);
    if replace {
        let y = pad + rh + p(4);
        MoveWindow(repl, pad, y + (rh - eh) / 2, fw, eh, 1);
        MoveWindow(buttons[5], pad * 2 + fw, y, p(62), rh, 1);
        MoveWindow(buttons[6], pad * 2 + fw + p(64), y, p(36), rh, 1);
    }
}

unsafe fn draw_button(bar: HWND, d: &DRAWITEMSTRUCT) {
    let c = theme::palette();
    let Some((label, icon, on, font, icons)) = with(hwnd_is(bar), |b| {
        let id = d.CtlID as usize;
        let &(_, label, icon) = BUTTONS.iter().find(|x| x.0 == id)?;
        let on = (id == ID_CASE && b.case) || (id == ID_REGEX && b.regex);
        Some((label, icon, on, b.font, b.icons))
    }).flatten() else { return };
    let fill = |r: &RECT, color: u32| {
        let br = CreateSolidBrush(color);
        FillRect(d.hDC, r, br);
        DeleteObject(br);
    };
    fill(&d.rcItem, c.strip);
    let mut r = d.rcItem;
    InflateRect(&mut r, -1, -2);
    let pressed = d.itemState & ODS_SELECTED != 0;
    if on || pressed {
        fill(&r, if on { c.sel } else { c.tab_hover });
    }
    if on {
        let pen = CreatePen(PS_SOLID, 1, 0xD47800); // Windows blue
        let old = (SelectObject(d.hDC, pen), SelectObject(d.hDC, GetStockObject(NULL_BRUSH)));
        Rectangle(d.hDC, r.left, r.top, r.right, r.bottom);
        SelectObject(d.hDC, old.0);
        SelectObject(d.hDC, old.1);
        DeleteObject(pen);
    }
    SetBkMode(d.hDC, TRANSPARENT as _);
    SetTextColor(d.hDC, if on || icon { c.fg } else { c.status_fg });
    SelectObject(d.hDC, if icon { icons } else { font });
    let mut t = crate::wide(label);
    DrawTextW(d.hDC, t.as_mut_ptr(), -1, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    if d.itemState & ODS_FOCUS != 0 {
        DrawFocusRect(d.hDC, &r);
    }
}

unsafe extern "system" fn proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let owner = with(hwnd_is(h), |b| b.owner).unwrap_or(null_mut());
    match m {
        WM_ERASEBKGND => {
            let c = theme::palette();
            let mut rc: RECT = zeroed();
            GetClientRect(h, &mut rc);
            for (r, color) in [(rc, c.separator), (RECT { left: 1, top: 0, right: rc.right - 1, bottom: rc.bottom - 1 }, c.strip)] {
                let b = CreateSolidBrush(color);
                FillRect(wp as HDC, &r, b);
                DeleteObject(b);
            }
            1
        }
        WM_CTLCOLOREDIT => {
            let c = theme::palette();
            SetTextColor(wp as HDC, c.fg);
            SetBkColor(wp as HDC, c.bg);
            with(hwnd_is(h), |b| b.edit_bg as LRESULT).unwrap_or(0)
        }
        WM_CTLCOLORSTATIC => {
            let c = theme::palette();
            SetTextColor(wp as HDC, c.status_fg);
            SetBkColor(wp as HDC, c.strip);
            with(hwnd_is(h), |b| b.bg as LRESULT).unwrap_or(0)
        }
        WM_DRAWITEM => {
            draw_button(h, &*(lp as *const DRAWITEMSTRUCT));
            1
        }
        WM_SIZE => {
            layout(h);
            0
        }
        WM_TIMER if wp == TIMER_COUNT => {
            KillTimer(h, TIMER_COUNT);
            recount(owner);
            0
        }
        WM_COMMAND => {
            let (id, code) = (wp & 0xFFFF, ((wp >> 16) & 0xFFFF) as u32);
            match id {
                // Enter: next match (Shift+Enter: previous); in the replace field: replace.
                x if x == IDOK as usize => {
                    let repl = with(hwnd_is(h), |b| b.repl).unwrap_or(null_mut());
                    if GetFocus() == repl {
                        replace(owner, false);
                    } else {
                        go(owner, GetKeyState(VK_SHIFT as i32) < 0);
                    }
                }
                x if x == IDCANCEL as usize || x == ID_CLOSE => close(owner),
                ID_FIND if code == EN_CHANGE => incremental(owner),
                ID_PREV | ID_NEXT => go(owner, id == ID_PREV),
                ID_REPLACE | ID_REPLACE_ALL => replace(owner, id == ID_REPLACE_ALL),
                ID_CASE | ID_REGEX => {
                    with(hwnd_is(h), |b| {
                        if id == ID_CASE { b.case = !b.case } else { b.regex = !b.regex }
                    });
                    InvalidateRect(lp as HWND, null(), 0);
                    incremental(owner);
                }
                _ => {}
            }
            0
        }
        WM_DESTROY => {
            let gone = BARS.with(|b| {
                let mut b = b.borrow_mut();
                let i = b.iter().position(|b| b.hwnd == h)?;
                Some(b.remove(i))
            });
            if let Some(b) = gone {
                for o in [b.font as HGDIOBJ, b.icons, b.bg, b.edit_bg].into_iter().filter(|o| !o.is_null()) {
                    DeleteObject(o);
                }
            }
            0
        }
        _ => DefWindowProcW(h, m, wp, lp),
    }
}

/// The editor's text changed: count again a moment later (not on every key).
pub unsafe fn text_changed(owner: HWND) {
    let bar = open_bar(owner);
    if !bar.is_null() {
        SetTimer(bar, TIMER_COUNT, 200, None);
    }
}
