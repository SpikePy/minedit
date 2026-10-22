//! Ctrl+P: quick open. A popup with a search box and a fuzzy-filtered list of open files,
//! then (each below a line) recently opened files and matching files of the open folder.

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
    pub group: Group,
}

/// The sections of the list, in this order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Group {
    Open,
    Recent,
    Folder, // files of the open folder: only shown for a search
}

/// How the list behaves.
pub struct Opts {
    pub font: HFONT,
    pub dpi: u32,
    pub top: i32,                       // distance from the owner's top edge
    pub pick: unsafe fn(usize),         // Enter or click on an item
    pub preview: Option<unsafe fn(usize)>, // the highlighted item changed
    pub cancel: Option<unsafe fn()>,    // closed without picking
    pub selected: usize,                // item highlighted at the start
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
    preview: Option<unsafe fn(usize)>,
    cancel: Option<unsafe fn()>,
    picked: bool,
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
/// Every place the first letter occurs is tried ("md" scores best at the end of "readme.md").
pub fn fuzzy(q: &[char], t: &[char]) -> Option<i32> {
    let first = *q.first()?;
    (0..t.len()).filter(|&i| t[i] == first).filter_map(|i| fuzzy_from(q, t, i)).max()
}

fn fuzzy_from(q: &[char], t: &[char], start: usize) -> Option<i32> {
    let (mut score, mut from, mut prev) = (0, start, usize::MAX);
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

/// Indices of the items matching `query`: by group, then best match first.
pub fn filter(items: &[Item], query: &str) -> Vec<usize> {
    let empty = query.trim().is_empty();
    let mut hits: Vec<(Group, i32, usize)> = items.iter().enumerate()
        .filter_map(|(i, it)| match (empty, it.group) {
            (true, Group::Folder) => None,
            (true, _) => Some((it.group, 0, i)),
            _ => rank(query, it).map(|s| (it.group, s, i)),
        })
        .collect();
    hits.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
    hits.into_iter().map(|h| h.2).collect()
}

/// Opens the popup over `owner`; `pick` is called with the chosen item's index.
pub unsafe fn show(owner: HWND, items: Vec<Item>, o: Opts) {
    close();
    let Opts { font, dpi, top, pick, preview, cancel, selected } = o;
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
    let sel = shown.iter().position(|&i| i == selected).unwrap_or(0);
    let brush = CreateSolidBrush(theme::palette().strip);
    QUICK.with(|q| *q.borrow_mut() = Some(Quick { popup, input, items, shown, sel, top: sel.saturating_sub(ROWS - 1), font, brush, pick, preview, cancel, picked: false }));
    resize();
    ShowWindow(popup, SW_SHOW);
    SetFocus(input);
}

/// Repaints the popup in the current theme (after a live preview changed it).
pub unsafe fn recolor() {
    QUICK.with(|q| {
        if let Some(q) = q.borrow_mut().as_mut() {
            DeleteObject(q.brush);
            q.brush = CreateSolidBrush(theme::palette().strip);
            InvalidateRect(q.popup, null(), 1);
            InvalidateRect(q.input, null(), 1);
        }
    });
}

/// Tells the caller which item is highlighted now.
unsafe fn previewed() {
    let call = QUICK.with(|q| q.borrow().as_ref().and_then(|q| Some((q.preview?, *q.shown.get(q.sel)?))));
    if let Some((f, i)) = call {
        f(i);
    }
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
    update(Vec::new(), false);
}

/// Whether the popup is open.
pub fn is_open() -> bool {
    QUICK.with(|q| q.borrow().is_some())
}

/// Adds items found later (the open folder's files), keeping the highlighted item.
pub unsafe fn add(items: Vec<Item>) {
    update(items, true);
}

unsafe fn update(new: Vec<Item>, keep: bool) {
    let Some(input) = QUICK.with(|q| q.borrow().as_ref().map(|q| q.input)) else { return };
    let mut buf = [0u16; 512];
    let n = GetWindowTextW(input, buf.as_mut_ptr(), buf.len() as i32) as usize;
    let query = String::from_utf16_lossy(&buf[..n]);
    let moved = QUICK.with(|q| {
        let mut q = q.borrow_mut();
        let q = q.as_mut()?;
        let old = q.shown.get(q.sel).copied();
        q.items.extend(new);
        q.shown = filter(&q.items, &query);
        let sel = old.filter(|_| keep).and_then(|o| q.shown.iter().position(|&i| i == o)).unwrap_or(0);
        q.top = if keep { q.top.min(sel) } else { 0 };
        let moved = q.shown.get(sel).copied() != old;
        q.sel = sel;
        InvalidateRect(q.popup, null(), 0);
        Some(moved)
    });
    resize();
    if moved == Some(true) || !keep {
        previewed();
    }
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
    previewed();
}

unsafe fn choose(row: usize) {
    let Some((item, pick)) = QUICK.with(|q| q.borrow().as_ref().and_then(|q| q.shown.get(row).map(|&i| (i, q.pick)))) else { return };
    QUICK.with(|q| q.borrow_mut().as_mut().map(|q| q.picked = true));
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
            // A line between the sections.
            if row > 0 && it.group != q.items[q.shown[row - 1]].group {
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
            let cancel = QUICK.with(|q| {
                let q = q.borrow_mut().take()?;
                DeleteObject(q.brush);
                if q.picked { None } else { q.cancel }
            });
            if let Some(f) = cancel {
                f();
            }
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
        Item { name: name.into(), dir: dir.into(), group: if open { Group::Open } else { Group::Recent } }
    }

    #[test]
    fn fuzzy_matching() {
        let s = |q: &str, t: &str| fuzzy(&q.chars().collect::<Vec<_>>(), &t.chars().collect::<Vec<_>>());
        assert!(s("mnrs", "main.rs").is_some());
        assert!(s("rsm", "main.rs").is_none()); // order matters
        assert!(s("main", "main.rs") > s("main", "my_awesome_main.rs")); // prefix and shorter win
        assert!(s("er", "edit.rs") < s("ed", "edit.rs"));
        assert!(s("md", "readme.md") > s("md", "minedit.ico")); // the best place, not the first one
        assert!(s("", "x").is_none());
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

    #[test]
    fn folder_files_only_for_a_search_and_last() {
        let mut items = vec![item(r"C:\a\recent.md", false), item(r"C:\a\open.rs", true)];
        items.push(Item { name: "readme.md".into(), dir: r"proj\docs".into(), group: Group::Folder });
        items.push(Item { name: "lib.rs".into(), dir: r"proj\src".into(), group: Group::Folder });
        assert_eq!(filter(&items, ""), [1, 0]);
        assert_eq!(filter(&items, "md"), [0, 2]); // recent before the folder's files
        assert_eq!(filter(&items, "srclib"), [3]); // matched with its path
        assert_eq!(filter(&items, "lib"), [3]);
    }
}
