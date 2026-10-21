//! Ctrl+P: quick open. A popup with a search box and a fuzzy-filtered list of open files,
//! then (below a line) recently opened files.

use crate::theme;
use std::{cell::RefCell, mem::zeroed, ptr::{null, null_mut}};
use windows_sys::{
    w,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
    },
};

pub struct Item {
    pub name: String,
    pub dir: String,
    pub open: bool, // an open tab (listed first) rather than a recent file
}

struct Quick {
    popup: HWND,
    input: HWND,
    items: Vec<Item>,
    shown: Vec<usize>, // matching items, best first
    sel: usize,
    top: usize, // first visible row
    font: HFONT,
    brush: HBRUSH, // input background
    pick: unsafe fn(usize),
}

thread_local! {
    static QUICK: RefCell<Option<Quick>> = const { RefCell::new(None) };
}

const ROWS: usize = 12;
const INPUT_ID: usize = 1;

fn px(v: i32, dpi: u32) -> i32 {
    v * dpi as i32 / 96
}

/// Score of `q` matching `t` as a subsequence (both lowercase), or None. Higher is better:
/// consecutive letters and word starts count extra, gaps and long texts cost a little.
pub fn fuzzy(q: &[char], t: &[char]) -> Option<i32> {
    let (mut score, mut from, mut prev) = (0, 0, usize::MAX);
    for &c in q {
        let i = (from..t.len()).find(|&i| t[i] == c)?;
        score += 10;
        if i > 0 && prev == i - 1 {
            score += 15;
        }
        if i == 0 || matches!(t[i - 1], '\\' | '/' | '_' | '-' | '.' | ' ') {
            score += 10;
        }
        score -= (i - from) as i32;
        (prev, from) = (i, i + 1);
    }
    Some(score - t.len() as i32 / 4)
}

/// Matches on the file name rank above matches that need the folder path.
pub fn rank(query: &str, item: &Item) -> Option<i32> {
    let q: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    let name: Vec<char> = item.name.to_lowercase().chars().collect();
    let full: Vec<char> = format!("{}\\{}", item.dir, item.name).to_lowercase().chars().collect();
    fuzzy(&q, &name).map(|s| s + 1000).or_else(|| fuzzy(&q, &full))
}

/// Indices of the items matching `query`: open files first, then best match first.
pub fn filter(items: &[Item], query: &str) -> Vec<usize> {
    let mut hits: Vec<(bool, i32, usize)> = items.iter().enumerate()
        .filter_map(|(i, it)| if query.trim().is_empty() { Some((it.open, 0, i)) } else { rank(query, it).map(|s| (it.open, s, i)) })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
    hits.into_iter().map(|h| h.2).collect()
}

/// Opens the popup over `owner`; `pick` is called with the chosen item's index.
pub unsafe fn show(owner: HWND, items: Vec<Item>, font: HFONT, dpi: u32, top: i32, pick: unsafe fn(usize)) {
    close();
    let p = |v| px(v, dpi);
    let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: GetModuleHandleW(null()), lpszClassName: w!("minedit_quick"),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW), ..zeroed() };
    RegisterClassW(&wc); // fails harmlessly when already registered
    let mut rc: RECT = zeroed();
    GetWindowRect(owner, &mut rc);
    let width = (rc.right - rc.left - p(40)).min(p(700)).max(p(300));
    let popup = CreateWindowExW(WS_EX_TOOLWINDOW, w!("minedit_quick"), w!("Open"), WS_POPUP | WS_BORDER,
        rc.left + (rc.right - rc.left - width) / 2, rc.top + top, width, p(100), owner, null_mut(), GetModuleHandleW(null()), null());
    let input = CreateWindowExW(0, w!("EDIT"), null(), WS_CHILD | WS_VISIBLE | ES_AUTOHSCROLL as u32,
        p(10), p(9), width - p(22), p(22), popup, INPUT_ID as HMENU, GetModuleHandleW(null()), null());
    SendMessageW(input, WM_SETFONT, font as WPARAM, 0);
    SetWindowSubclass(input, Some(input_proc), 1, 0);
    let shown = filter(&items, "");
    let brush = CreateSolidBrush(theme::palette().strip);
    QUICK.with(|q| *q.borrow_mut() = Some(Quick { popup, input, items, shown, sel: 0, top: 0, font, brush, pick }));
    resize();
    ShowWindow(popup, SW_SHOW);
    SetFocus(input);
}

pub unsafe fn close() {
    let popup = QUICK.with(|q| q.borrow().as_ref().map(|q| q.popup));
    if let Some(p) = popup {
        DestroyWindow(p);
    }
}

fn dpi_of(h: HWND) -> u32 {
    unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(h) }
}

/// Height fits the number of matches (up to ROWS).
unsafe fn resize() {
    let Some((popup, n)) = QUICK.with(|q| q.borrow().as_ref().map(|q| (q.popup, q.shown.len()))) else { return };
    let p = |v| px(v, dpi_of(popup));
    let mut rc: RECT = zeroed();
    GetWindowRect(popup, &mut rc);
    let rows = n.clamp(1, ROWS) as i32;
    SetWindowPos(popup, null_mut(), 0, 0, rc.right - rc.left, p(40) + rows * p(26) + p(8), SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    InvalidateRect(popup, null(), 1);
}

unsafe fn refilter() {
    let Some(input) = QUICK.with(|q| q.borrow().as_ref().map(|q| q.input)) else { return };
    let mut buf = [0u16; 512];
    let n = GetWindowTextW(input, buf.as_mut_ptr(), buf.len() as i32) as usize;
    let query = String::from_utf16_lossy(&buf[..n]);
    QUICK.with(|q| {
        if let Some(q) = q.borrow_mut().as_mut() {
            (q.shown, q.sel, q.top) = (filter(&q.items, &query), 0, 0);
        }
    });
    resize();
}

/// Moves the selection by `d` rows, keeping it visible.
unsafe fn move_sel(d: isize) {
    QUICK.with(|q| {
        if let Some(q) = q.borrow_mut().as_mut() {
            if q.shown.is_empty() {
                return;
            }
            q.sel = (q.sel as isize + d).clamp(0, q.shown.len() as isize - 1) as usize;
            q.top = q.top.min(q.sel).max((q.sel + 1).saturating_sub(ROWS));
            InvalidateRect(q.popup, null(), 0);
        }
    });
}

unsafe fn choose(row: usize) {
    let Some((item, pick)) = QUICK.with(|q| q.borrow().as_ref().and_then(|q| q.shown.get(row).map(|&i| (i, q.pick)))) else { return };
    close();
    pick(item);
}

unsafe fn paint(h: HWND) {
    let mut ps: PAINTSTRUCT = zeroed();
    let hdc = BeginPaint(h, &mut ps);
    let p = |v| px(v, dpi_of(h));
    let c = theme::palette();
    let mut rc: RECT = zeroed();
    GetClientRect(h, &mut rc);
    let fill = |r: RECT, color: u32| {
        let b = CreateSolidBrush(color);
        FillRect(hdc, &r, b);
        DeleteObject(b);
    };
    fill(rc, c.bg);
    fill(RECT { left: 0, top: 0, right: rc.right, bottom: p(40) }, c.strip);
    QUICK.with(|q| {
        let q = q.borrow();
        let Some(q) = q.as_ref() else { return };
        SelectObject(hdc, q.font);
        SetBkMode(hdc, TRANSPARENT as _);
        let text = |s: &str, x: i32, y: i32, color: u32| {
            let t: Vec<u16> = s.encode_utf16().collect();
            SetTextColor(hdc, color);
            TextOutW(hdc, x, y, t.as_ptr(), t.len() as i32);
            let mut sz: SIZE = zeroed();
            GetTextExtentPoint32W(hdc, t.as_ptr(), t.len() as i32, &mut sz);
            sz.cx
        };
        if q.shown.is_empty() {
            text("No matching files", p(14), p(44), c.inactive_text);
        }
        for (row, &i) in q.shown.iter().enumerate().skip(q.top).take(ROWS) {
            let y = p(44) + (row - q.top) as i32 * p(26);
            let it = &q.items[i];
            if row == q.sel {
                fill(RECT { left: p(4), top: y - p(2), right: rc.right - p(4), bottom: y + p(24) }, c.sel);
            }
            // A line between the open files and the recent ones.
            if !it.open && row > 0 && q.items[q.shown[row - 1]].open {
                fill(RECT { left: p(8), top: y - p(3), right: rc.right - p(8), bottom: y - p(2) }, c.separator);
            }
            let w = text(&it.name, p(14), y + p(2), c.fg);
            text(&it.dir, p(14) + w + p(12), y + p(2), c.inactive_text);
        }
    });
    EndPaint(h, &ps);
}

unsafe extern "system" fn proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_PAINT => {
            paint(h);
            0
        }
        WM_COMMAND if (wp >> 16) as u32 == EN_CHANGE => {
            refilter();
            0
        }
        // The search box in the theme's colors.
        WM_CTLCOLOREDIT => {
            let c = theme::palette();
            SetTextColor(wp as HDC, c.fg);
            SetBkColor(wp as HDC, c.strip);
            QUICK.with(|q| q.borrow().as_ref().map_or(0, |q| q.brush as LRESULT))
        }
        WM_LBUTTONDOWN => {
            let y = ((lp >> 16) & 0xFFFF) as i16 as i32;
            let p = |v| px(v, dpi_of(h));
            if y >= p(42) {
                let row = QUICK.with(|q| q.borrow().as_ref().map_or(0, |q| q.top)) + ((y - p(42)) / p(26)) as usize;
                choose(row);
            }
            0
        }
        WM_MOUSEWHEEL => {
            move_sel(-((wp >> 16) as i16 as isize) / 40);
            0
        }
        WM_ACTIVATE if (wp & 0xFFFF) as u32 == WA_INACTIVE => {
            PostMessageW(h, WM_CLOSE, 0, 0);
            0
        }
        WM_DESTROY => {
            QUICK.with(|q| {
                if let Some(q) = q.borrow_mut().take() {
                    DeleteObject(q.brush);
                }
            });
            0
        }
        _ => DefWindowProcW(h, m, wp, lp),
    }
}

/// Keys in the search box: arrows move through the list, Enter opens, Esc closes.
unsafe extern "system" fn input_proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
    if m == WM_KEYDOWN {
        match wp as VIRTUAL_KEY {
            VK_UP => return { move_sel(-1); 0 },
            VK_DOWN => return { move_sel(1); 0 },
            VK_PRIOR => return { move_sel(-(ROWS as isize)); 0 },
            VK_NEXT => return { move_sel(ROWS as isize); 0 },
            VK_RETURN => return { choose(QUICK.with(|q| q.borrow().as_ref().map_or(0, |q| q.sel))); 0 },
            VK_ESCAPE => return { close(); 0 },
            _ => {}
        }
    }
    // No beep for Enter/Esc characters.
    if m == WM_CHAR && (wp == 13 || wp == 27) {
        return 0;
    }
    DefSubclassProc(h, m, wp, lp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(path: &str, open: bool) -> Item {
        let (dir, name) = path.rsplit_once('\\').unwrap();
        Item { name: name.into(), dir: dir.into(), open }
    }

    #[test]
    fn fuzzy_matching() {
        let s = |q: &str, t: &str| fuzzy(&q.chars().collect::<Vec<_>>(), &t.chars().collect::<Vec<_>>());
        assert!(s("mnrs", "main.rs").is_some());
        assert!(s("rsm", "main.rs").is_none()); // order matters
        assert!(s("main", "main.rs") > s("main", "my_awesome_main.rs")); // prefix and shorter win
        assert!(s("er", "edit.rs") < s("ed", "edit.rs"));
    }

    #[test]
    fn name_matches_rank_above_path_matches() {
        let items = [
            item(r"C:\src\editor\notes.txt", false),
            item(r"C:\work\editor.rs", false),
            item(r"C:\src\main.rs", true),
        ];
        // "editor" is the name of item 1 but only in the path of item 0.
        assert_eq!(filter(&items, "editor"), [1, 0]);
        // Open files come first, whatever the score; a match only in the path ("src … notes") comes last.
        assert_eq!(filter(&items, "rs"), [2, 1, 0]);
        // Empty query: open files, then the rest in their order.
        assert_eq!(filter(&items, ""), [2, 0, 1]);
        assert_eq!(filter(&items, "xyz"), Vec::<usize>::new());
    }
}
