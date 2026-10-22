//! The sidebar's search view (Ctrl+Shift+F / Ctrl+Shift+H): find and replace in all open tabs and the
//! files of the open folder, with match case, whole word, regular expressions and a glob file filter.
//! The matches are listed by file below the fields; the folder is searched in the background.

use crate::{theme, tree};
use regex_lite::{Regex, RegexBuilder};
use std::{
    cell::RefCell,
    collections::HashSet,
    mem::zeroed,
    ops::Range,
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    sync::{atomic::{AtomicBool, Ordering::Relaxed}, Arc},
};
use windows_sys::{
    w,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::{Dialogs::*, *},
            Input::KeyboardAndMouse::*,
            WindowsAndMessaging::*,
        },
    },
};

/// Where `what` occurs in `text` (positions), as the Find dialog matches: FR_MATCHCASE, FR_WHOLEWORD.
pub unsafe fn hits(text: &[u16], what: &[u16], flags: u32) -> Vec<usize> {
    let fold = |v: &[u16]| {
        let mut v = v.to_vec();
        if flags & FR_MATCHCASE == 0 && !v.is_empty() {
            CharLowerBuffW(v.as_mut_ptr(), v.len() as u32);
        }
        v
    };
    let (t, w) = (fold(text), fold(what));
    if w.is_empty() {
        return Vec::new();
    }
    let word = |c: Option<&u16>| c.is_some_and(|&c| IsCharAlphaNumericW(c) != 0 || c == b'_' as u16);
    (0..t.len())
        .filter(|&i| t[i..].starts_with(&w))
        .filter(|&i| flags & FR_WHOLEWORD == 0 || !(word(i.checked_sub(1).and_then(|j| t.get(j))) || word(t.get(i + w.len()))))
        .collect()
}

/// Which files to search: comma-separated globs ("*.rs, src/**"); a leading "!" excludes ("!*.min.js").
/// `*` matches within a name, `**` across folders, `?` one character. A pattern without "/" is matched
/// against every part of the path, so "*.md" finds Markdown files anywhere and "target" leaves out a folder.
#[derive(Default)]
pub struct Filter {
    include: Vec<Vec<char>>,
    exclude: Vec<Vec<char>>,
}

impl Filter {
    pub fn new(spec: &str) -> Filter {
        let mut f = Filter::default();
        for p in spec.split(',').map(|p| p.trim().to_lowercase().replace('\\', "/")).filter(|p| !p.is_empty()) {
            let (list, p) = match p.strip_prefix('!') {
                Some(rest) => (&mut f.exclude, rest.to_string()),
                None => (&mut f.include, p),
            };
            let p = p.trim_start_matches("./").trim_start_matches('/').trim_end_matches('/');
            if !p.is_empty() {
                list.push(p.chars().collect());
            }
        }
        f
    }

    /// `rel`: the path relative to the folder (or a name for files outside it).
    pub fn matches(&self, rel: &str) -> bool {
        let rel: Vec<char> = rel.to_lowercase().replace('\\', "/").chars().collect();
        let hit = |p: &Vec<char>| {
            if p.contains(&'/') {
                // A path pattern: the whole path, or a folder it names.
                glob(p, &rel) || glob(&[p.as_slice(), &['/', '*', '*']].concat(), &rel)
            } else {
                rel.split(|&c| c == '/').any(|part| glob(p, part))
            }
        };
        (self.include.is_empty() || self.include.iter().any(hit)) && !self.exclude.iter().any(hit)
    }
}

fn glob(p: &[char], s: &[char]) -> bool {
    match p {
        [] => s.is_empty(),
        ['*', '*', rest @ ..] => {
            let rest = rest.strip_prefix(&['/']).unwrap_or(rest);
            (0..=s.len()).any(|i| glob(rest, &s[i..]))
        }
        ['*', rest @ ..] => (0..=s.len()).take_while(|&i| i == 0 || s[i - 1] != '/').any(|i| glob(rest, &s[i..])),
        ['?', rest @ ..] => s.first().is_some_and(|&c| c != '/') && glob(rest, &s[1..]),
        [c, rest @ ..] => s.first() == Some(c) && glob(rest, &s[1..]),
    }
}

/// What to look for.
pub struct Query {
    what: Vec<u16>,
    flags: u32,
    re: Option<Regex>,
    pub filter: Filter,
}

impl Query {
    /// None for an empty search; Err for an invalid regular expression.
    pub fn new(what: &[u16], case: bool, word: bool, regex: bool, files: &str) -> Option<Result<Query, String>> {
        if what.is_empty() {
            return None;
        }
        let flags = if case { FR_MATCHCASE } else { 0 } | if word { FR_WHOLEWORD } else { 0 };
        let re = if regex {
            let p = String::from_utf16_lossy(what);
            let p = if word { format!(r"\b(?:{p})\b") } else { p };
            match RegexBuilder::new(&p).case_insensitive(!case).multi_line(true).build() {
                Ok(r) => Some(r),
                Err(e) => return Some(Err(e.to_string().lines().last().unwrap_or("invalid").trim().to_string())),
            }
        } else {
            None
        };
        Some(Ok(Query { what: what.to_vec(), flags, re, filter: Filter::new(files) }))
    }

    /// The matches in `text` as UTF-16 ranges; with `repl`, each with its replacement ($1 … for groups).
    pub unsafe fn find(&self, text: &[u16], repl: Option<&[u16]>) -> Vec<(Range<usize>, Vec<u16>)> {
        let Some(re) = &self.re else {
            let r = repl.unwrap_or_default().to_vec();
            return hits(text, &self.what, self.flags).into_iter().map(|i| (i..i + self.what.len(), r.clone())).collect();
        };
        let s = String::from_utf16_lossy(text);
        let repl = repl.map(String::from_utf16_lossy);
        // Byte offsets into `s` → UTF-16 offsets into `text`, counted forward as the matches come.
        let (mut byte, mut unit) = (0, 0);
        let mut to_unit = |b: usize| {
            unit += s[byte..b].encode_utf16().count();
            byte = b;
            unit
        };
        let mut out = Vec::new();
        for caps in re.captures_iter(&s) {
            let m = caps.get(0).unwrap();
            if m.is_empty() {
                continue;
            }
            let (start, end) = (to_unit(m.start()), to_unit(m.end()));
            let mut r = String::new();
            if let Some(repl) = &repl {
                caps.expand(repl, &mut r);
            }
            out.push((start..end, r.encode_utf16().collect()));
        }
        out
    }
}

/// An open tab to search: its editor's text right now.
pub struct Tab {
    pub path: Option<PathBuf>,
    pub label: String,
    pub edit: isize, // HWND of the editor (window handles cannot cross threads as pointers)
    pub text: Vec<u16>,
}

/// One match.
#[derive(Clone)]
pub struct Hit {
    pub path: Option<PathBuf>,
    pub edit: isize, // the open tab's editor, or 0 for a file on disk
    pub label: String,
    pub line: usize, // 0-based
    pub start: usize,
    pub len: usize,
    pub preview: String,
}

const MAX_HITS: usize = 10_000;
const MAX_FILE: u64 = 20 << 20;
const WM_HITS: u32 = WM_APP + 1; // wParam: search number, lParam: Box<Vec<Hit>> (one file's matches)
const WM_DONE: u32 = WM_APP + 2; // wParam: search number
const TIMER_SEARCH: usize = 1; // searching starts shortly after typing stops
const ID_FIND: usize = 10;
const ID_REPL: usize = 11;
const ID_FILES: usize = 12;
const ID_CASE: usize = 13;
const ID_WORD: usize = 14;
const ID_REGEX: usize = 15;
const ID_REPLACE_ALL: usize = 16;
const ID_STATUS: usize = 17;
const ID_RESULTS: usize = 18;

struct Panel {
    hwnd: HWND,
    find: HWND,
    repl: HWND,
    files: HWND,
    toggles: [HWND; 3], // match case, whole word, regex
    on: [bool; 3],
    replace_all: HWND,
    status: HWND,
    results: HWND,
    font: HFONT,
    bg: HBRUSH,   // panel background
    edit: HBRUSH, // input fields
    hits: Vec<Hit>,
    files_found: usize,
    search: u32,
    cancel: Arc<AtomicBool>,
    done: bool,
    error: Option<String>,
}

thread_local! {
    static PANELS: RefCell<Vec<Panel>> = const { RefCell::new(Vec::new()) };
}

fn with<R>(h: HWND, f: impl FnOnce(&mut Panel) -> R) -> Option<R> {
    PANELS.with(|p| p.borrow_mut().iter_mut().find(|p| p.hwnd == h).map(f))
}

fn px(v: i32, dpi: u32) -> i32 {
    v * dpi as i32 / 96
}

/// The matches in one text, labelled for the list.
unsafe fn search_text(q: &Query, text: &[u16], path: &Option<PathBuf>, edit: isize, label: &str) -> Vec<Hit> {
    let found = q.find(text, None);
    if found.is_empty() {
        return Vec::new();
    }
    let starts: Vec<usize> = std::iter::once(0).chain(text.iter().enumerate().filter(|(_, &c)| c == b'\n' as u16).map(|(i, _)| i + 1)).collect();
    found.into_iter().map(|(r, _)| {
        let line = starts.partition_point(|&s| s <= r.start) - 1;
        let end = starts.get(line + 1).map_or(text.len(), |&e| e - 1);
        let preview: String = String::from_utf16_lossy(&text[starts[line]..end]).trim().chars().take(300).collect();
        Hit { path: path.clone(), edit, label: label.into(), line, start: r.start, len: r.len(), preview }
    }).collect()
}

/// A file's text for searching, or None for big or binary files.
fn read_text(p: &Path) -> Option<Vec<u16>> {
    if std::fs::metadata(p).ok()?.len() > MAX_FILE {
        return None;
    }
    let bytes = std::fs::read(p).ok()?;
    // NUL bytes mean binary, except in UTF-16 text (which starts with its byte order mark).
    if !bytes.starts_with(&[0xFF, 0xFE]) && bytes.iter().take(8000).any(|&b| b == 0) {
        return None;
    }
    Some(crate::edit::to_lf(&crate::decode(&bytes).0))
}

/// Searches the tabs, then the folder's files that are not open; posts each file's matches to `panel`.
fn run(search: u32, panel: isize, q: Query, tabs: Vec<Tab>, root: Option<PathBuf>, cancel: Arc<AtomicBool>) {
    let post = |hits: Vec<Hit>| unsafe {
        let hits = Box::into_raw(Box::new(hits));
        if PostMessageW(panel as HWND, WM_HITS, search as WPARAM, hits as LPARAM) == 0 {
            drop(Box::from_raw(hits));
        }
    };
    let total = std::cell::Cell::new(0);
    let mut open = HashSet::new();
    for t in &tabs {
        if let Some(p) = &t.path {
            open.insert(p.to_string_lossy().to_lowercase());
        }
        if !q.filter.matches(&t.label) {
            continue;
        }
        let h = unsafe { search_text(&q, &t.text, &t.path, t.edit, &t.label) };
        total.set(total.get() + h.len());
        if !h.is_empty() {
            post(h);
        }
    }
    if let Some(root) = root {
        tree::walk(&root, || !cancel.load(Relaxed) && total.get() < MAX_HITS, |files| {
            for f in files {
                if cancel.load(Relaxed) || open.contains(&f.to_string_lossy().to_lowercase()) {
                    continue;
                }
                let label = f.strip_prefix(&root).map_or_else(|_| f.display().to_string(), |r| r.display().to_string());
                if !q.filter.matches(&label) {
                    continue;
                }
                let Some(text) = read_text(&f) else { continue };
                let h = unsafe { search_text(&q, &text, &Some(f.clone()), 0, &label) };
                total.set(total.get() + h.len());
                if !h.is_empty() {
                    post(h);
                }
            }
        });
    }
    unsafe { PostMessageW(panel as HWND, WM_DONE, search as WPARAM, 0) };
}

unsafe fn text_of(h: HWND) -> Vec<u16> {
    let mut buf = vec![0u16; GetWindowTextLengthW(h) as usize + 1];
    let n = GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32);
    buf.truncate(n.max(0) as usize);
    buf
}

/// The query in the fields: None while the search field is empty.
unsafe fn query(panel: HWND) -> Option<Result<Query, String>> {
    let (find, files, on) = with(panel, |p| (p.find, p.files, p.on))?;
    Query::new(&text_of(find), on[0], on[1], on[2], &String::from_utf16_lossy(&text_of(files)))
}

/// Searches again with the current fields.
unsafe fn start(panel: HWND) {
    KillTimer(panel, TIMER_SEARCH);
    let q = query(panel);
    let Some((search, cancel, results)) = with(panel, |p| {
        p.cancel.store(true, Relaxed);
        p.cancel = Arc::new(AtomicBool::new(false));
        p.search += 1;
        p.hits.clear();
        p.files_found = 0;
        p.error = q.as_ref().and_then(|q| q.as_ref().err().cloned());
        p.done = !matches!(q, Some(Ok(_)));
        (p.search, p.cancel.clone(), p.results)
    }) else { return };
    SendMessageW(results, TVM_DELETEITEM, 0, TVI_ROOT);
    update_status(panel);
    let Some(Ok(q)) = q else { return };
    let (tabs, root) = crate::search_sources();
    let panel = panel as isize;
    std::thread::spawn(move || run(search, panel, q, tabs, root, cancel));
}

unsafe fn update_status(panel: HWND) {
    let Some((status, find, n, files, done, error)) = with(panel, |p| (p.status, p.find, p.hits.len(), p.files_found, p.done, p.error.clone())) else { return };
    let empty = text_of(find).is_empty();
    let s = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let text = match (error, n, done) {
        (Some(e), _, _) => format!("Invalid regular expression: {e}"),
        (None, 0, true) if empty => String::new(),
        (None, 0, true) => "No results".into(),
        (None, 0, false) => "Searching…".into(),
        (None, n, done) => format!("{} in {}{}{}", s(n, "result", "results"), s(files, "file", "files"),
            if n >= MAX_HITS { " (stopped)" } else { "" }, if done { "" } else { " – searching…" }),
    };
    SetWindowTextW(status, crate::wide(&text).as_ptr());
}

/// Adds one file's matches to the result tree: the file, then its lines.
unsafe fn add_hits(panel: HWND, found: Vec<Hit>) {
    let Some((results, items)) = with(panel, |p| {
        let room = MAX_HITS.saturating_sub(p.hits.len());
        let first = p.hits.len();
        p.hits.extend(found.into_iter().take(room));
        if p.hits.len() > first {
            p.files_found += 1;
        }
        let items: Vec<(usize, String, String)> = p.hits[first..].iter().enumerate()
            .map(|(k, h)| (first + k, h.label.clone(), format!("{}: {}", h.line + 1, h.preview))).collect();
        (p.results, items)
    }) else { return };
    let Some((_, label, _)) = items.first() else { return };
    let insert = |parent: HTREEITEM, text: &str, data: isize, bold: bool| {
        let mut t = crate::wide(text);
        let mut ins: TVINSERTSTRUCTW = zeroed();
        ins.hParent = parent;
        ins.hInsertAfter = TVI_LAST;
        ins.Anonymous.item = TVITEMW {
            mask: TVIF_TEXT | TVIF_PARAM | TVIF_STATE, pszText: t.as_mut_ptr(), lParam: data,
            state: if bold { TVIS_BOLD } else { 0 }, stateMask: TVIS_BOLD, ..zeroed()
        };
        SendMessageW(results, TVM_INSERTITEMW, 0, &ins as *const _ as LPARAM)
    };
    // File items carry -1, lines the index of their match.
    let file = insert(TVI_ROOT, &format!("{label} ({})", items.len()), -1, true);
    for (i, _, text) in &items {
        insert(file, text, *i as isize, false);
    }
    SendMessageW(results, TVM_EXPAND, TVE_EXPAND as WPARAM, file);
}

/// Replace All: in open tabs as one undoable step each, in other files on disk (after asking).
unsafe fn replace_all(panel: HWND) {
    let Some(Ok(q)) = query(panel) else { return };
    let Some((repl, targets, n)) = with(panel, |p| {
        let mut seen = HashSet::new();
        let targets: Vec<(Option<PathBuf>, isize)> = p.hits.iter().filter(|h| seen.insert((h.path.clone(), h.edit))).map(|h| (h.path.clone(), h.edit)).collect();
        (p.repl, targets, p.hits.len())
    }) else { return };
    let repl = text_of(repl);
    if targets.is_empty() {
        return;
    }
    let disk = targets.iter().filter(|t| t.1 == 0).count();
    let msg = format!("Replace {n} match{} in {} file{} with \"{}\"?{}", if n == 1 { "" } else { "es" }, targets.len(),
        if targets.len() == 1 { "" } else { "s" }, String::from_utf16_lossy(&repl),
        if disk > 0 { format!("\n\n{disk} of them are not open and are changed on disk (no undo).") } else { String::new() });
    let owner = GetAncestor(panel, GA_ROOT);
    if MessageBoxW(owner, crate::wide(&msg).as_ptr(), w!("Replace All"), MB_OKCANCEL | MB_ICONQUESTION) != IDOK {
        return;
    }
    for (path, edit) in targets {
        if edit != 0 && IsWindow(edit as HWND) != 0 {
            let e = crate::edit::ed(edit as HWND);
            let changes = q.find(&e.text, Some(&repl));
            e.replace_each(changes);
        } else if let Some(p) = path {
            let Ok((text, enc, lf)) = crate::load(&p) else { continue };
            let changes = q.find(&text, Some(&repl));
            if changes.is_empty() {
                continue;
            }
            let mut out = Vec::with_capacity(text.len());
            let mut at = 0;
            for (r, t) in changes {
                out.extend_from_slice(&text[at..r.start]);
                out.extend_from_slice(&t);
                at = r.end;
            }
            out.extend_from_slice(&text[at..]);
            let out = if lf { out } else { crate::edit::to_crlf(&out) };
            if let Err(e) = std::fs::write(&p, crate::encode(&out, enc)) {
                let msg = crate::wide(&format!("{}\n\n{e}", p.display()));
                MessageBoxW(owner, msg.as_ptr(), w!("Replace All"), MB_ICONERROR);
            }
        }
    }
    start(panel);
}

/// Shows a match (`item` in the result tree); a file item opens or closes instead.
unsafe fn activate(panel: HWND, item: HTREEITEM) {
    let Some(results) = with(panel, |p| p.results) else { return };
    let mut it = TVITEMW { mask: TVIF_PARAM, hItem: item, ..zeroed() };
    SendMessageW(results, TVM_GETITEMW, 0, &mut it as *mut _ as LPARAM);
    if it.lParam < 0 {
        SendMessageW(results, TVM_EXPAND, TVE_TOGGLE as WPARAM, item);
    } else if let Some(hit) = with(panel, |p| p.hits.get(it.lParam as usize).cloned()).flatten() {
        crate::open_hit(&hit);
    }
}

/// The search view inside window `parent` (hidden until laid out).
pub unsafe fn create(parent: HWND, font: HFONT, dpi: u32) -> HWND {
    let hinst = GetModuleHandleW(null());
    let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: hinst, lpszClassName: w!("minedit_search"),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW), ..zeroed() };
    RegisterClassW(&wc); // fails harmlessly when already registered
    let hwnd = CreateWindowExW(WS_EX_CONTROLPARENT, w!("minedit_search"), null(), WS_CHILD | WS_CLIPCHILDREN, 0, 0, 0, 0, parent, null_mut(), hinst, null());
    let child = |ex: u32, class: *const u16, text: &str, style: u32, id: usize| {
        CreateWindowExW(ex, class, crate::wide(text).as_ptr(), WS_CHILD | WS_VISIBLE | style, 0, 0, 0, 0, hwnd, id as HMENU, hinst, null())
    };
    let edit = |id, cue: &str| {
        let e = child(WS_EX_CLIENTEDGE, w!("EDIT"), "", ES_AUTOHSCROLL as u32 | WS_TABSTOP, id);
        SendMessageW(e, EM_SETCUEBANNER, 1, crate::wide(cue).as_ptr() as LPARAM);
        e
    };
    let find = edit(ID_FIND, "Search");
    let toggles = [ID_CASE, ID_WORD, ID_REGEX].map(|id| child(0, w!("BUTTON"), "", BS_OWNERDRAW as u32 | WS_TABSTOP, id));
    let repl = edit(ID_REPL, "Replace");
    let replace_all = child(0, w!("BUTTON"), "Replace All", BS_PUSHBUTTON as u32 | WS_TABSTOP, ID_REPLACE_ALL);
    let files = edit(ID_FILES, "Files, e.g. *.rs, src/**, !*.min.js");
    let status = child(0, w!("STATIC"), "", 0x4000 /* SS_ENDELLIPSIS */, ID_STATUS);
    let style = TVS_HASBUTTONS | TVS_SHOWSELALWAYS | TVS_FULLROWSELECT | TVS_NOHSCROLL | WS_TABSTOP;
    let results = child(0, WC_TREEVIEWW, "", style, ID_RESULTS);
    SendMessageW(results, TVM_SETEXTENDEDSTYLE, TVS_EX_DOUBLEBUFFER as WPARAM, TVS_EX_DOUBLEBUFFER as LPARAM);
    PANELS.with(|p| p.borrow_mut().push(Panel {
        hwnd, find, repl, files, toggles, on: [false; 3], replace_all, status, results, font: null_mut(), bg: null_mut(), edit: null_mut(),
        hits: Vec::new(), files_found: 0, search: 0, cancel: Arc::new(AtomicBool::new(false)), done: true, error: None,
    }));
    set_font(hwnd, font, dpi);
    recolor(hwnd);
    hwnd
}

/// Uses a copy of `font` (the editor replaces its own when settings change).
pub unsafe fn set_font(panel: HWND, font: HFONT, dpi: u32) {
    let mut lf: LOGFONTW = zeroed();
    GetObjectW(font, size_of::<LOGFONTW>() as i32, &mut lf as *mut _ as _);
    let copy = CreateFontIndirectW(&lf);
    let Some((old, controls, results)) = with(panel, |p| {
        let controls = [p.find, p.repl, p.files, p.replace_all, p.status, p.results, p.toggles[0], p.toggles[1], p.toggles[2]];
        (std::mem::replace(&mut p.font, copy), controls, p.results)
    }) else { return };
    for c in controls {
        SendMessageW(c, WM_SETFONT, copy as WPARAM, 1);
    }
    SendMessageW(results, TVM_SETITEMHEIGHT, px(22, dpi) as WPARAM, 0);
    if !old.is_null() {
        DeleteObject(old);
    }
    layout(panel);
}

/// Shows the search with `initial` in the search field (if given); `replace` focuses the replace field.
pub unsafe fn focus(panel: HWND, initial: &[u16], replace: bool) {
    let Some((find, repl)) = with(panel, |p| (p.find, p.repl)) else { return };
    if !initial.is_empty() {
        let mut t = initial.to_vec();
        t.push(0);
        SetWindowTextW(find, t.as_ptr());
        start(panel);
    }
    let field = if replace { repl } else { find };
    SetFocus(field);
    SendMessageW(field, EM_SETSEL, 0, -1);
}

/// Whether window `h` is part of search view `panel` (its fields or results).
pub unsafe fn owns(panel: HWND, h: HWND) -> bool {
    !panel.is_null() && (h == panel || IsChild(panel, h) != 0)
}

unsafe fn layout(panel: HWND) {
    let Some((find, repl, files, toggles, replace_all, status, results, font)) =
        with(panel, |p| (p.find, p.repl, p.files, p.toggles, p.replace_all, p.status, p.results, p.font)) else { return };
    let dpi = windows_sys::Win32::UI::HiDpi::GetDpiForWindow(panel);
    let p = |v| px(v, dpi);
    let mut rc: RECT = zeroed();
    GetClientRect(panel, &mut rc);
    // The fields are as high as the font needs.
    let dc = GetDC(panel);
    let old = SelectObject(dc, font);
    let mut tm: TEXTMETRICW = zeroed();
    GetTextMetricsW(dc, &mut tm);
    SelectObject(dc, old);
    ReleaseDC(panel, dc);
    let (pad, rh, gap) = (p(8), tm.tmHeight + p(10), p(6));
    let w = rc.right - pad * 2;
    let tw = rh; // square toggles
    let mut y = pad;
    MoveWindow(find, pad, y, (w - 3 * (tw + p(2))).max(p(40)), rh, 1);
    for (i, t) in toggles.iter().enumerate() {
        MoveWindow(*t, rc.right - pad - (3 - i as i32) * (tw + p(2)) + p(2), y, tw, rh, 1);
    }
    y += rh + gap;
    let bw = p(84);
    MoveWindow(repl, pad, y, (w - bw - p(4)).max(p(40)), rh, 1);
    MoveWindow(replace_all, rc.right - pad - bw, y, bw, rh, 1);
    y += rh + gap;
    MoveWindow(files, pad, y, w, rh, 1);
    y += rh + p(4);
    MoveWindow(status, pad, y, w, rh, 1);
    y += rh;
    MoveWindow(results, 0, y, rc.right, (rc.bottom - y).max(0), 1);
}

/// Applies the current theme.
pub unsafe fn recolor(panel: HWND) {
    let c = theme::palette();
    let Some((edits, buttons, results, old)) = with(panel, |p| {
        let old = [std::mem::replace(&mut p.bg, CreateSolidBrush(c.strip)), std::mem::replace(&mut p.edit, CreateSolidBrush(c.bg))];
        ([p.find, p.repl, p.files], [p.replace_all], p.results, old)
    }) else { return };
    let dark = theme::is_dark();
    for e in edits {
        SetWindowTheme(e, if dark { w!("DarkMode_CFD") } else { w!("CFD") }, null());
    }
    for b in buttons {
        SetWindowTheme(b, if dark { w!("DarkMode_Explorer") } else { w!("Explorer") }, null());
    }
    tree::recolor(results);
    RedrawWindow(panel, null(), null_mut(), RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
    for b in old.into_iter().filter(|b| !b.is_null()) {
        DeleteObject(b);
    }
}

/// A toggle button: "Aa", "ab" or ".*", filled while on.
unsafe fn draw_toggle(d: &DRAWITEMSTRUCT, label: &str, on: bool) {
    let c = theme::palette();
    let fill = |r: &RECT, color: u32| {
        let b = CreateSolidBrush(color);
        FillRect(d.hDC, r, b);
        DeleteObject(b);
    };
    fill(&d.rcItem, c.strip);
    let mut r = d.rcItem;
    InflateRect(&mut r, -1, -1);
    if on {
        fill(&r, c.sel);
        let pen = CreatePen(PS_SOLID, 1, 0xD47800 /* Windows blue */);
        let old = (SelectObject(d.hDC, pen), SelectObject(d.hDC, GetStockObject(NULL_BRUSH)));
        Rectangle(d.hDC, r.left, r.top, r.right, r.bottom);
        SelectObject(d.hDC, old.0);
        SelectObject(d.hDC, old.1);
        DeleteObject(pen);
    }
    SetBkMode(d.hDC, TRANSPARENT as _);
    SetTextColor(d.hDC, if on { c.fg } else { c.status_fg });
    SelectObject(d.hDC, SendMessageW(d.hwndItem, WM_GETFONT, 0, 0) as HFONT);
    let mut t = crate::wide(label);
    DrawTextW(d.hDC, t.as_mut_ptr(), -1, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    if d.itemState & ODS_FOCUS != 0 {
        DrawFocusRect(d.hDC, &r);
    }
}

const TOGGLES: [&str; 3] = ["Aa", "ab", ".*"]; // match case, whole word, regular expression

unsafe extern "system" fn proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_ERASEBKGND => {
            let mut rc: RECT = zeroed();
            GetClientRect(h, &mut rc);
            let b = CreateSolidBrush(theme::palette().strip);
            FillRect(wp as HDC, &rc, b);
            DeleteObject(b);
            1
        }
        WM_CTLCOLOREDIT => {
            let c = theme::palette();
            SetTextColor(wp as HDC, c.fg);
            SetBkColor(wp as HDC, c.bg);
            with(h, |p| p.edit as LRESULT).unwrap_or(0)
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            let c = theme::palette();
            let status = with(h, |p| p.status).unwrap_or(null_mut());
            SetTextColor(wp as HDC, if lp as HWND == status { c.status_fg } else { c.fg });
            SetBkColor(wp as HDC, c.strip);
            with(h, |p| p.bg as LRESULT).unwrap_or(0)
        }
        WM_DRAWITEM => {
            let d = &*(lp as *const DRAWITEMSTRUCT);
            if let Some((i, on)) = with(h, |p| p.toggles.iter().position(|&t| t == d.hwndItem).map(|i| (i, p.on[i]))).flatten() {
                draw_toggle(d, TOGGLES[i], on);
            }
            1
        }
        WM_SIZE => {
            layout(h);
            0
        }
        WM_TIMER if wp == TIMER_SEARCH => {
            start(h);
            0
        }
        WM_COMMAND => {
            let (id, code) = (wp & 0xFFFF, ((wp >> 16) & 0xFFFF) as u32);
            match id {
                // Enter: in the results it shows the match, elsewhere it searches now.
                x if x == IDOK as usize => {
                    let results = with(h, |p| p.results).unwrap_or(null_mut());
                    if GetFocus() == results {
                        activate(h, SendMessageW(results, TVM_GETNEXTITEM, TVGN_CARET as WPARAM, 0));
                    } else {
                        start(h);
                    }
                }
                // Esc: back to the editor.
                x if x == IDCANCEL as usize => _ = SetFocus(GetParent(h)),
                ID_FIND | ID_FILES if code == EN_CHANGE => _ = SetTimer(h, TIMER_SEARCH, 300, None),
                ID_REPLACE_ALL => replace_all(h),
                ID_CASE | ID_WORD | ID_REGEX if code == BN_CLICKED => {
                    let i = id - ID_CASE;
                    with(h, |p| {
                        p.on[i] = !p.on[i];
                        InvalidateRect(p.toggles[i], null(), 0);
                    });
                    start(h);
                }
                _ => {}
            }
            0
        }
        WM_NOTIFY => {
            let hdr = &*(lp as *const NMHDR);
            match hdr.code {
                NM_CLICK => {
                    let pos = GetMessagePos();
                    let mut ht = TVHITTESTINFO { pt: POINT { x: (pos & 0xFFFF) as i16 as i32, y: (pos >> 16) as i16 as i32 }, ..zeroed() };
                    ScreenToClient(hdr.hwndFrom, &mut ht.pt);
                    let it = SendMessageW(hdr.hwndFrom, TVM_HITTEST, 0, &mut ht as *mut _ as LPARAM);
                    if it != 0 && ht.flags & (TVHT_ONITEM | TVHT_ONITEMRIGHT) != 0 {
                        SendMessageW(hdr.hwndFrom, TVM_SELECTITEM, TVGN_CARET as WPARAM, it);
                        activate(h, it);
                    }
                    0
                }
                NM_DBLCLK => 1, // a click already did it
                _ => 0,
            }
        }
        WM_HITS => {
            let found = *Box::from_raw(lp as *mut Vec<Hit>);
            if with(h, |p| p.search == wp as u32 && p.hits.len() < MAX_HITS) == Some(true) {
                add_hits(h, found);
                update_status(h);
            }
            0
        }
        WM_DONE => {
            let current = with(h, |p| {
                let current = p.search == wp as u32;
                if current {
                    p.done = true;
                }
                current
            });
            if current == Some(true) {
                update_status(h);
            }
            0
        }
        WM_DESTROY => {
            let gone = PANELS.with(|p| {
                let mut p = p.borrow_mut();
                let i = p.iter().position(|p| p.hwnd == h)?;
                Some(p.remove(i))
            });
            if let Some(p) = gone {
                p.cancel.store(true, Relaxed);
                for o in [p.font as HGDIOBJ, p.bg, p.edit].into_iter().filter(|o| !o.is_null()) {
                    DeleteObject(o);
                }
            }
            0
        }
        _ => DefWindowProcW(h, m, wp, lp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn q(what: &str, case: bool, word: bool, regex: bool) -> Query {
        Query::new(&u(what), case, word, regex, "").unwrap().unwrap()
    }

    #[test]
    fn matches_with_lines() {
        unsafe {
            let text = u("fn main() {\n    let Main = 1;\n}\nmain();\n");
            let h = search_text(&q("main", false, false, false), &text, &None, 7, "x.rs");
            assert_eq!(h.iter().map(|h| (h.line, h.start)).collect::<Vec<_>>(), [(0, 3), (1, 20), (3, 32)]);
            assert_eq!(h[1].preview, "let Main = 1;");
            assert_eq!((h[1].len, h[1].edit), (4, 7));
            assert_eq!(search_text(&q("main", true, false, false), &text, &None, 0, "").len(), 2);
            assert_eq!(search_text(&q("main", false, true, false), &u("mainly main"), &None, 0, "").len(), 1);
            assert!(Query::new(&[], false, false, false, "").is_none());
        }
    }

    #[test]
    fn regular_expressions() {
        unsafe {
            // UTF-16 positions after a character outside the BMP (two units, four UTF-8 bytes).
            let text = u("😀 let x1 = 5;\nlet Y22 = 6;\n");
            let found = q(r"let (\w)(\d+)", false, false, true).find(&text, Some(&u("var $2$1")));
            assert_eq!(found.iter().map(|f| f.0.clone()).collect::<Vec<_>>(), [3..9, 15..22]);
            assert_eq!(found[1].1, u("var 22Y"));
            assert_eq!(q("^let", true, false, true).find(&text, None).len(), 1); // ^ at line starts
            assert_eq!(q("x", false, true, true).find(&u("x xy x"), None).len(), 2); // whole word
            assert!(q("a*", false, false, true).find(&u("bbb"), None).is_empty()); // no empty matches
            assert!(matches!(Query::new(&u("("), false, false, true, ""), Some(Err(_))));
        }
    }

    #[test]
    fn file_filter() {
        let f = Filter::new("*.rs, src/**, !*_test.rs");
        assert!(f.matches(r"src\main.rs"));
        assert!(f.matches("lib/deep/x.rs"));
        assert!(f.matches("src/a.txt"));
        assert!(!f.matches("README.md"));
        assert!(!f.matches("lib/a_test.rs"));
        let f = Filter::new("!target, !.github/");
        assert!(f.matches("src/main.rs"));
        assert!(!f.matches("target/release/x.d"));
        assert!(!f.matches(".github/workflows/build.yml"));
        assert!(Filter::new("").matches("anything"));
        assert!(Filter::new("docs/*.md").matches("docs/a.md"));
        assert!(!Filter::new("docs/*.md").matches("docs/x/a.md"));
        assert!(Filter::new("SPEC.?D").matches("spec.md"));
    }

    #[test]
    fn binary_and_utf16_files() {
        let d = std::env::temp_dir().join(format!("minedit-search-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("bin"), [1u8, 0, 2, 3]).unwrap();
        std::fs::write(d.join("u16"), [0xFF, 0xFE, b'h', 0, b'i', 0]).unwrap();
        std::fs::write(d.join("crlf"), "a\r\nb").unwrap();
        assert!(read_text(&d.join("bin")).is_none());
        assert_eq!(read_text(&d.join("u16")), Some(u("hi")));
        assert_eq!(read_text(&d.join("crlf")), Some(u("a\nb")));
        _ = std::fs::remove_dir_all(d);
    }
}
