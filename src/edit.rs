//! Multi-cursor text control with unlimited undo/redo, drawn with GDI.
//! Text is UTF-16 with '\n' line breaks; positions are offsets into it.

use crate::syntax::{self, Lang};
use std::{cell::Cell, mem::zeroed, ops::Range, ptr::{null, null_mut}};
use windows_sys::{
    w,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{DataExchange::*, LibraryLoader::GetModuleHandleW, Memory::*},
        UI::{Controls::SetScrollInfo, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
};

const NL: u16 = b'\n' as u16;
const CR: u16 = b'\r' as u16;
const TAB: u16 = b'\t' as u16;
const SP: u16 = b' ' as u16;
const PAD: i32 = 4; // inside the line-number gutter
const CF_UNICODETEXT: u32 = 13;
/// Posted to an editor to resync its scrollbars after a size or font change.
pub const UPDATE: u32 = WM_APP;
/// Posted to the parent when the main caret moved; lParam is the editor.
pub const CARET: u32 = WM_APP + 1;
const SEL_COLOR: u32 = 0xFFD6AD; // light blue (0x00BBGGRR)
const SEL_COLOR_INACTIVE: u32 = 0xE5E5E5;
const LINE_COLOR: u32 = 0xF2F2F2; // current line
const GUTTER_COLOR: u32 = 0xF8F8F8;

/// Font and tab settings shared by all editors.
#[derive(Clone, Copy)]
struct View {
    font: HFONT,
    dc: HDC, // memory DC with the font selected, for measuring
    lh: i32, // line height
    space: i32,
    digit: i32,
    tab_width: usize,
    use_spaces: bool,
    line_numbers: bool,
    highlight_line: bool,
    margin: i32,
    wrap: bool,
}

thread_local! {
    static VIEW: Cell<View> = const { Cell::new(View {
        font: null_mut(), dc: null_mut(), lh: 16, space: 8, digit: 8, tab_width: 4, use_spaces: false,
        line_numbers: true, highlight_line: true, margin: 8, wrap: false,
    }) };
}

fn view() -> View {
    VIEW.with(|v| v.get())
}

/// Editor options from the settings file.
#[derive(Clone, Copy)]
pub struct Options {
    pub tab_width: usize,
    pub use_spaces: bool,
    pub line_numbers: bool,
    pub highlight_line: bool,
    pub margin: i32, // pixels around the text
    pub word_wrap: bool,
}

pub unsafe fn set_view(font: HFONT, o: &Options) {
    let mut v = view();
    if v.dc.is_null() {
        v.dc = CreateCompatibleDC(null_mut());
    }
    SelectObject(v.dc, font);
    let mut tm: TEXTMETRICW = zeroed();
    GetTextMetricsW(v.dc, &mut tm);
    let mut sz: SIZE = zeroed();
    GetTextExtentPoint32W(v.dc, w!(" "), 1, &mut sz);
    (v.font, v.lh, v.space) = (font, tm.tmHeight.max(1), sz.cx.max(1));
    GetTextExtentPoint32W(v.dc, w!("0"), 1, &mut sz);
    v.digit = sz.cx.max(1);
    (v.tab_width, v.use_spaces) = (o.tab_width.max(1), o.use_spaces);
    (v.line_numbers, v.highlight_line, v.margin, v.wrap) = (o.line_numbers, o.highlight_line, o.margin.max(0), o.word_wrap);
    VIEW.with(|c| c.set(v));
}

/// X offset of every position in a line (len + 1 entries), expanding tabs.
unsafe fn xs(v: View, line: &[u16]) -> Vec<i32> {
    let tab = v.space * v.tab_width as i32;
    let mut out = Vec::with_capacity(line.len() + 1);
    out.push(0);
    let mut x = 0;
    for seg in line.split_inclusive(|&c| c == TAB) {
        let n = seg.len() - (seg.last() == Some(&TAB)) as usize;
        if n > 0 {
            let mut dx = vec![0i32; n];
            let mut sz: SIZE = zeroed();
            GetTextExtentExPointW(v.dc, seg.as_ptr(), n as i32, 0, null_mut(), dx.as_mut_ptr(), &mut sz);
            out.extend(dx.iter().map(|d| x + d));
            x += dx[n - 1];
        }
        if n < seg.len() {
            x = (x / tab + 1) * tab;
            out.push(x);
        }
    }
    out
}

/// Index of the entry in `xs` closest to `x`.
fn hit(xs: &[i32], x: i32) -> usize {
    let i = xs.partition_point(|&v| v < x);
    if i == 0 {
        0
    } else if i == xs.len() || x - xs[i - 1] < xs[i] - x {
        i - 1
    } else {
        i
    }
}

/// Character class for word movement: space, word, punctuation, newline.
fn class(c: u16) -> u8 {
    match c {
        NL => 3,
        SP | TAB => 0,
        _ if c == b'_' as u16 || unsafe { IsCharAlphaNumericW(c) } != 0 => 1,
        _ => 2,
    }
}

fn key(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(vk as i32) < 0 }
}

/// Converts CRLF to LF.
pub fn to_lf(t: &[u16]) -> Vec<u16> {
    let mut out = Vec::with_capacity(t.len());
    for (i, &c) in t.iter().enumerate() {
        if !(c == CR && t.get(i + 1) == Some(&NL)) {
            out.push(c);
        }
    }
    out
}

pub fn to_crlf(t: &[u16]) -> Vec<u16> {
    let mut out = Vec::with_capacity(t.len() + t.len() / 32);
    for &c in t {
        if c == NL {
            out.push(CR);
        }
        out.push(c);
    }
    out
}

/// A selection; a caret when anchor == head. `x` remembers the column for up/down moves.
#[derive(Clone, Copy, PartialEq)]
struct Sel {
    a: usize,
    h: usize,
    x: i32,
}

impl Sel {
    fn at(p: usize) -> Sel {
        Sel { a: p, h: p, x: -1 }
    }
    fn r(&self) -> Range<usize> {
        self.a.min(self.h)..self.a.max(self.h)
    }
}

/// One replacement, at an offset valid when it was applied.
struct Op {
    at: usize,
    del: Vec<u16>,
    ins: Vec<u16>,
}

struct Step {
    ops: Vec<Op>,
    before: Vec<Sel>,
    after: Vec<Sel>,
    typing: bool,
}

pub struct Editor {
    h: HWND,
    pub text: Vec<u16>,
    lines: Vec<usize>, // start offset of each line
    sels: Vec<Sel>,    // sorted, non-overlapping
    main: usize,       // the selection the view follows
    undo: Vec<Step>,
    redo: Vec<Step>,
    saved: Option<usize>, // undo depth that matches the file on disk
    merge: bool,          // next typing may join the last undo step
    top: usize,           // first visible line
    top_sub: usize,       // first visible screen row within it (word wrap)
    sx: i32,              // horizontal scroll in pixels
    wmax: i32,            // widest line seen, for the horizontal scrollbar
    wheel: i32,
    drag: bool,
    alt_click: bool, // swallow the Alt release so Alt+click does not open the menu
    focus: bool,
    lang: Lang,
    blink: bool,
}

pub unsafe fn ed<'a>(h: HWND) -> &'a mut Editor {
    &mut *(GetWindowLongPtrW(h, GWLP_USERDATA) as *mut Editor)
}

pub unsafe fn register() {
    let wc = WNDCLASSW {
        style: CS_DBLCLKS,
        lpfnWndProc: Some(proc),
        hInstance: GetModuleHandleW(null()),
        hCursor: LoadCursorW(null_mut(), IDC_IBEAM),
        lpszClassName: w!("minedit_text"),
        ..zeroed()
    };
    RegisterClassW(&wc);
}

pub unsafe fn create(parent: HWND) -> HWND {
    let h = CreateWindowExW(0, w!("minedit_text"), null(), WS_CHILD | WS_VSCROLL | WS_HSCROLL,
        0, 0, 0, 0, parent, null_mut(), GetModuleHandleW(null()), null());
    let e = Box::new(Editor {
        h, text: Vec::new(), lines: vec![0], sels: vec![Sel::at(0)], main: 0, undo: Vec::new(), redo: Vec::new(),
        saved: Some(0), merge: false, top: 0, top_sub: 0, sx: 0, wmax: 0, wheel: 0, drag: false, alt_click: false, focus: false, lang: Lang::Plain, blink: true,
    });
    SetWindowLongPtrW(h, GWLP_USERDATA, Box::into_raw(e) as isize);
    h
}

impl Editor {
    pub fn dirty(&self) -> bool {
        self.saved != Some(self.undo.len())
    }

    pub fn mark_saved(&mut self) {
        (self.saved, self.merge) = (Some(self.undo.len()), false);
    }

    pub fn mark_dirty(&mut self) {
        self.saved = None;
    }

    pub unsafe fn set_lang(&mut self, lang: Lang) {
        self.lang = lang;
        InvalidateRect(self.h, null(), 0);
    }

    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// 1-based line and column of the main caret, counting a tab up to the next tab stop.
    pub fn caret(&self) -> (usize, usize) {
        let h = self.sels[self.main].h;
        (self.line_of(h) + 1, self.vcol(h) + 1)
    }

    pub unsafe fn set_text(&mut self, t: Vec<u16>) {
        self.text = t;
        self.reindex();
        (self.sels, self.main, self.top, self.top_sub, self.sx, self.wmax) = (vec![Sel::at(0)], 0, 0, 0, 0, 0);
        self.undo.clear();
        self.redo.clear();
        self.mark_saved();
        self.update();
    }

    /// The main selection.
    pub fn sel(&self) -> Range<usize> {
        self.sels[self.main].r()
    }

    pub unsafe fn select(&mut self, a: usize, h: usize) {
        (self.sels, self.main, self.merge) = (vec![Sel { a, h, x: -1 }], 0, false);
        self.show();
    }

    /// Replaces a range as one undo step, leaving the caret after the new text.
    pub unsafe fn replace(&mut self, r: Range<usize>, t: Vec<u16>) {
        (self.sels, self.main) = (vec![Sel { a: r.start, h: r.end, x: -1 }], 0);
        self.change(vec![(r, t)], true, false);
    }

    fn reindex(&mut self) {
        self.lines.clear();
        self.lines.push(0);
        self.lines.extend(self.text.iter().enumerate().filter(|(_, &c)| c == NL).map(|(i, _)| i + 1));
    }

    fn line_of(&self, p: usize) -> usize {
        self.lines.partition_point(|&s| s <= p) - 1
    }

    /// Line content without its newline.
    fn line(&self, l: usize) -> Range<usize> {
        self.lines[l]..self.lines.get(l + 1).map_or(self.text.len(), |&n| n - 1)
    }

    // A screen row is (line, row within the line); without word wrap every line is one row.

    unsafe fn wrap_width(&self) -> i32 {
        (self.client().0 - self.left_edge() - view().margin).max(view().space * 8)
    }

    /// Start offsets (within the line) of the screen rows of line `l`, given its `xs`.
    unsafe fn breaks(&self, l: usize, xs: &[i32]) -> Vec<usize> {
        let mut out = vec![0];
        if !view().wrap {
            return out;
        }
        let line = &self.text[self.line(l)];
        let width = self.wrap_width();
        let mut start = 0;
        loop {
            let mut end = xs.partition_point(|&x| x <= xs[start] + width) - 1;
            if end >= line.len() {
                return out;
            }
            // Break after the last space that fits, else mid-word.
            if let Some(i) = (start + 1..=end).rev().find(|&i| matches!(line[i - 1], SP | TAB)) {
                end = i;
            }
            end = end.max(start + 1);
            out.push(end);
            start = end;
        }
    }

    /// Positions and row starts of line `l`.
    unsafe fn line_rows(&self, l: usize) -> (Vec<i32>, Vec<usize>) {
        let xs = xs(view(), &self.text[self.line(l)]);
        let b = self.breaks(l, &xs);
        (xs, b)
    }

    unsafe fn row_count(&self, l: usize) -> usize {
        if view().wrap { self.line_rows(l).1.len() } else { 1 }
    }

    unsafe fn row_of(&self, p: usize) -> (usize, usize) {
        let l = self.line_of(p);
        let b = self.line_rows(l).1;
        (l, b.partition_point(|&s| s <= p - self.lines[l]) - 1)
    }

    /// Moves `d` screen rows from `r`; returns the row reached and how many rows were moved.
    unsafe fn step(&self, (mut l, mut s): (usize, usize), d: isize) -> ((usize, usize), usize) {
        if !view().wrap {
            let t = (l as isize).saturating_add(d).clamp(0, self.lines.len() as isize - 1) as usize;
            return ((t, 0), t.abs_diff(l));
        }
        let mut n = 0;
        for _ in 0..d.unsigned_abs() {
            if d > 0 {
                if s + 1 < self.row_count(l) {
                    s += 1;
                } else if l + 1 < self.lines.len() {
                    (l, s) = (l + 1, 0);
                } else {
                    break;
                }
            } else if s > 0 {
                s -= 1;
            } else if l > 0 {
                l -= 1;
                s = self.row_count(l) - 1;
            } else {
                break;
            }
            n += 1;
        }
        ((l, s), n)
    }

    /// X of `p` relative to the start of its screen row.
    unsafe fn x_of(&self, p: usize) -> i32 {
        let l = self.line_of(p);
        let (xs, b) = self.line_rows(l);
        let c = p - self.lines[l];
        xs[c] - xs[b[b.partition_point(|&s| s <= c) - 1]]
    }

    /// Position closest to `x` in screen row `row`.
    unsafe fn pos_in_row(&self, (l, sub): (usize, usize), x: i32) -> usize {
        let (xs, b) = self.line_rows(l);
        let sub = sub.min(b.len() - 1);
        // A wrapped row ends just before the next row's start, so the caret stays on it.
        let (a, e) = (b[sub], b.get(sub + 1).map_or(xs.len() - 1, |&n| n - 1));
        self.lines[l] + a + hit(&xs[a..=e], x + xs[a])
    }

    unsafe fn pos_from(&self, x: i32, y: i32) -> usize {
        let v = view();
        let row = self.step((self.top, self.top_sub), (y - v.margin).div_euclid(v.lh) as isize).0;
        self.pos_in_row(row, x - self.left_edge() + self.sx)
    }

    /// Width of the line-number gutter.
    fn gutter(&self) -> i32 {
        let v = view();
        if v.line_numbers { self.lines.len().to_string().len().max(2) as i32 * v.digit + 3 * PAD } else { 0 }
    }

    /// Window x where unscrolled text starts.
    fn left_edge(&self) -> i32 {
        self.gutter() + view().margin
    }

    /// Visual column of `p`, with tabs advancing to the next tab stop.
    fn vcol(&self, p: usize) -> usize {
        let tw = view().tab_width;
        self.text[self.lines[self.line_of(p)]..p].iter().fold(0, |c, &ch| if ch == TAB { (c / tw + 1) * tw } else { c + 1 })
    }

    fn left(&self, p: usize) -> usize {
        let low = |c: u16| (0xDC00..0xE000).contains(&c);
        if p >= 2 && low(self.text[p - 1]) && !low(self.text[p - 2]) && self.text[p - 2] >= 0xD800 { p - 2 } else { p.saturating_sub(1) }
    }

    fn right(&self, p: usize) -> usize {
        let high = |c: u16| (0xD800..0xDC00).contains(&c);
        if p + 1 < self.text.len() && high(self.text[p]) { p + 2 } else { (p + 1).min(self.text.len()) }
    }

    fn word_left(&self, mut p: usize) -> usize {
        while p > 0 && class(self.text[p - 1]) == 0 {
            p -= 1;
        }
        if p > 0 {
            let k = class(self.text[p - 1]);
            if k == 3 {
                return p - 1;
            }
            while p > 0 && class(self.text[p - 1]) == k {
                p -= 1;
            }
        }
        p
    }

    fn word_right(&self, mut p: usize) -> usize {
        let n = self.text.len();
        if p < n {
            let k = class(self.text[p]);
            if k == 3 {
                return p + 1;
            }
            while p < n && class(self.text[p]) == k {
                p += 1;
            }
        }
        while p < n && class(self.text[p]) == 0 {
            p += 1;
        }
        p
    }

    /// Home toggles between the first non-blank character and the line start.
    fn home(&self, p: usize) -> usize {
        let r = self.line(self.line_of(p));
        let first = r.start + self.text[r.clone()].iter().take_while(|&&c| c == SP || c == TAB).count();
        if p == first { r.start } else { first }
    }

    /// Sorts selections and merges overlapping ones, keeping track of the main one.
    fn normalize(&mut self) {
        let mh = self.sels[self.main].h;
        self.sels.sort_by_key(|s| (s.r().start, s.r().end));
        let mut out: Vec<Sel> = Vec::with_capacity(self.sels.len());
        for s in self.sels.drain(..) {
            match out.last_mut() {
                Some(l) if s.r().start < l.r().end || s.r() == l.r() => {
                    let (a, b) = (l.r().start, s.r().end.max(l.r().end));
                    *l = if s.h >= s.a { Sel { a, h: b, x: s.x } } else { Sel { a: b, h: a, x: s.x } };
                }
                _ => out.push(s),
            }
        }
        self.main = out.iter().position(|s| s.r().start <= mh && mh <= s.r().end).unwrap_or(out.len() - 1);
        self.sels = out;
    }

    /// Applies replacements given in current coordinates as one undo step and maps all selections.
    unsafe fn change(&mut self, mut edits: Vec<(Range<usize>, Vec<u16>)>, collapse: bool, typing: bool) {
        edits.sort_by_key(|e| e.0.start);
        let mut end = 0; // two cursors may want to delete the same character
        for (r, _) in edits.iter_mut() {
            r.start = r.start.max(end);
            r.end = r.end.max(r.start);
            end = r.end;
        }
        edits.retain(|(r, t)| !r.is_empty() || !t.is_empty());
        if edits.is_empty() {
            return;
        }
        let map = |p: usize| {
            let mut d = 0isize;
            for (r, t) in &edits {
                if r.end <= p {
                    d += t.len() as isize - r.len() as isize;
                } else {
                    if r.start < p {
                        return (r.start as isize + d) as usize + t.len();
                    }
                    break;
                }
            }
            (p as isize + d) as usize
        };
        let before = self.sels.clone();
        let sels = self.sels.iter().map(|s| { let h = map(s.h); Sel { a: if collapse { h } else { map(s.a) }, h, x: -1 } }).collect();
        let mut ops = Vec::with_capacity(edits.len());
        let mut d = 0isize;
        for (r, ins) in edits {
            let at = (r.start as isize + d) as usize;
            let del: Vec<u16> = self.text.splice(at..at + r.len(), ins.iter().copied()).collect();
            d += ins.len() as isize - del.len() as isize;
            ops.push(Op { at, del, ins });
        }
        self.sels = sels;
        self.normalize();
        self.reindex();
        self.redo.clear();
        if self.saved.is_some_and(|s| s > self.undo.len()) {
            self.saved = None; // the saved state was only reachable by redo
        }
        let after = self.sels.clone();
        let mergeable = typing && self.merge && self.saved != Some(self.undo.len());
        match self.undo.last_mut() {
            Some(last) if mergeable && last.typing => {
                last.ops.extend(ops);
                last.after = after;
            }
            _ => self.undo.push(Step { ops, before, after, typing }),
        }
        self.merge = typing;
        self.changed();
    }

    unsafe fn changed(&mut self) {
        self.show();
        PostMessageW(GetParent(self.h), WM_COMMAND, (EN_CHANGE as usize) << 16, self.h as LPARAM);
    }

    unsafe fn undo(&mut self) {
        let Some(s) = self.undo.pop() else { return };
        for op in s.ops.iter().rev() {
            self.text.splice(op.at..op.at + op.ins.len(), op.del.iter().copied());
        }
        self.restore(s.before.clone());
        self.redo.push(s);
    }

    unsafe fn redo(&mut self) {
        let Some(s) = self.redo.pop() else { return };
        for op in &s.ops {
            self.text.splice(op.at..op.at + op.del.len(), op.ins.iter().copied());
        }
        self.restore(s.after.clone());
        self.undo.push(s);
    }

    unsafe fn restore(&mut self, sels: Vec<Sel>) {
        (self.main, self.sels, self.merge) = (sels.len() - 1, sels, false);
        self.reindex();
        self.changed();
    }

    /// Replaces every selection with text computed per selection.
    unsafe fn type_each(&mut self, typing: bool, f: impl Fn(&Self, Sel, usize) -> Vec<u16>) {
        let edits = self.sels.iter().enumerate().map(|(i, &s)| (s.r(), f(self, s, i))).collect();
        self.change(edits, true, typing);
    }

    unsafe fn delete(&mut self, word: bool, fwd: bool) {
        let edits = self.sels.iter().map(|s| {
            let r = if s.a != s.h {
                s.r()
            } else if fwd {
                s.h..if word { self.word_right(s.h) } else { self.right(s.h) }
            } else {
                (if word { self.word_left(s.h) } else { self.left(s.h) })..s.h
            };
            (r, Vec::new())
        }).collect();
        self.change(edits, true, !word);
    }

    /// Tab inserts a tab or spaces; with a multi-line selection (or Shift) it indents/unindents lines.
    unsafe fn tab(&mut self, out: bool) {
        let v = view();
        if !out && !self.sels.iter().any(|s| self.text[s.r()].contains(&NL)) {
            return self.type_each(true, |e, s, _| {
                if v.use_spaces { vec![SP; v.tab_width - e.vcol(s.r().start) % v.tab_width] } else { vec![TAB] }
            });
        }
        let mut lines = Vec::new();
        for s in &self.sels {
            let r = s.r();
            let (a, mut b) = (self.line_of(r.start), self.line_of(r.end));
            if b > a && self.lines[b] == r.end {
                b -= 1; // selection ends at the start of a line: leave that line alone
            }
            lines.extend(a..=b);
        }
        lines.dedup();
        let unit = if v.use_spaces { vec![SP; v.tab_width] } else { vec![TAB] };
        let edits = lines.into_iter().filter_map(|l| {
            let r = self.line(l);
            let t = &self.text[r.clone()];
            if out {
                let n = if t.first() == Some(&TAB) { 1 } else { t.iter().take(v.tab_width).take_while(|&&c| c == SP).count() };
                (n > 0).then(|| (r.start..r.start + n, Vec::new()))
            } else {
                (!t.is_empty()).then(|| (r.start..r.start, unit.clone()))
            }
        }).collect();
        self.change(edits, false, false);
    }

    unsafe fn copy(&self) -> bool {
        let parts: Vec<&[u16]> = self.sels.iter().filter(|s| s.a != s.h).map(|s| &self.text[s.r()]).collect();
        if parts.is_empty() || OpenClipboard(self.h) == 0 {
            return false;
        }
        let t = to_crlf(&parts.join(&NL));
        EmptyClipboard();
        let g = GlobalAlloc(GMEM_MOVEABLE, (t.len() + 1) * 2);
        let p = GlobalLock(g) as *mut u16;
        if !p.is_null() {
            p.copy_from_nonoverlapping(t.as_ptr(), t.len());
            *p.add(t.len()) = 0;
            GlobalUnlock(g);
            SetClipboardData(CF_UNICODETEXT, g);
        }
        CloseClipboard();
        true
    }

    unsafe fn paste(&mut self) {
        if OpenClipboard(self.h) == 0 {
            return;
        }
        let g = GetClipboardData(CF_UNICODETEXT);
        let p = if g.is_null() { null_mut() } else { GlobalLock(g) as *const u16 };
        let t = if p.is_null() { Vec::new() } else { (0..).map(|i| *p.add(i)).take_while(|&c| c != 0).collect() };
        if !p.is_null() {
            GlobalUnlock(g);
        }
        CloseClipboard();
        let t = to_lf(&t);
        // With one clipboard line per cursor, each cursor gets its own line.
        let parts: Vec<&[u16]> = t.split(|&c| c == NL).collect();
        let each = self.sels.len() > 1 && parts.len() == self.sels.len();
        self.type_each(false, |_, _, i| if each { parts[i].to_vec() } else { t.clone() });
    }

    unsafe fn move_to(&mut self, extend: bool, f: impl Fn(&Self, Sel) -> usize) {
        self.sels = self.sels.iter().map(|&s| { let p = f(self, s); Sel { a: if extend { s.a } else { p }, h: p, x: -1 } }).collect();
        self.merge = false;
        self.normalize();
        self.show();
    }

    /// Moves every cursor `dl` screen rows up or down, keeping its remembered column.
    unsafe fn vmove(&mut self, extend: bool, dl: isize) {
        self.sels = self.sels.iter().map(|&s| {
            let x = if s.x >= 0 { s.x } else { self.x_of(s.h) };
            let (row, moved) = self.step(self.row_of(s.h), dl);
            let p = if moved < dl.unsigned_abs() { if dl < 0 { 0 } else { self.text.len() } } else { self.pos_in_row(row, x) };
            Sel { a: if extend { s.a } else { p }, h: p, x }
        }).collect();
        self.merge = false;
        self.normalize();
        self.show();
    }

    unsafe fn key(&mut self, vk: VIRTUAL_KEY) -> bool {
        let (ctrl, shift) = (key(VK_CONTROL), key(VK_SHIFT));
        let page = self.rows() as isize;
        match vk {
            VK_LEFT => self.move_to(shift, |e, s| if !shift && s.a != s.h { s.r().start } else if ctrl { e.word_left(s.h) } else { e.left(s.h) }),
            VK_RIGHT => self.move_to(shift, |e, s| if !shift && s.a != s.h { s.r().end } else if ctrl { e.word_right(s.h) } else { e.right(s.h) }),
            VK_UP if ctrl => self.scroll(-1),
            VK_DOWN if ctrl => self.scroll(1),
            VK_UP => self.vmove(shift, -1),
            VK_DOWN => self.vmove(shift, 1),
            VK_PRIOR => { self.scroll(-page); self.vmove(shift, -page) }
            VK_NEXT => { self.scroll(page); self.vmove(shift, page) }
            VK_HOME => self.move_to(shift, |e, s| if ctrl { 0 } else { e.home(s.h) }),
            VK_END => self.move_to(shift, |e, s| if ctrl { e.text.len() } else { e.line(e.line_of(s.h)).end }),
            VK_BACK => self.delete(ctrl, false),
            VK_DELETE => self.delete(ctrl, true),
            VK_ESCAPE => self.select(self.sels[self.main].h, self.sels[self.main].h),
            _ if !ctrl => return false,
            VK_Z if shift => self.redo(),
            VK_Z => self.undo(),
            VK_Y => self.redo(),
            VK_C => _ = self.copy(),
            VK_X => {
                if self.copy() {
                    self.type_each(false, |_, _, _| Vec::new());
                }
            }
            VK_V => self.paste(),
            _ => return false,
        }
        true
    }

    unsafe fn char(&mut self, c: u16) {
        match c {
            CR => self.type_each(true, |e, s, _| {
                // Keep the indentation of the current line.
                let start = s.r().start;
                let indent = e.text[e.lines[e.line_of(start)]..start].iter().take_while(|&&c| c == SP || c == TAB);
                [NL].into_iter().chain(indent.copied()).collect()
            }),
            TAB => self.tab(key(VK_SHIFT)),
            _ if c < 32 || c == 127 => {}
            _ => self.type_each(true, |_, _, _| vec![c]),
        }
    }

    /// Click places the caret, Shift+click extends, Alt+click adds (or removes) a cursor.
    unsafe fn click(&mut self, x: i32, y: i32) {
        let p = self.pos_from(x, y);
        self.drag = true;
        if key(VK_MENU) {
            self.alt_click = true;
            match self.sels.iter().position(|s| s.a == p && s.h == p) {
                Some(i) if self.sels.len() > 1 => {
                    self.sels.remove(i);
                    (self.main, self.drag) = (self.sels.len() - 1, false);
                }
                _ => {
                    self.sels.push(Sel::at(p));
                    self.main = self.sels.len() - 1;
                }
            }
        } else if key(VK_SHIFT) {
            self.sels[self.main].h = p;
        } else {
            (self.sels, self.main) = (vec![Sel::at(p)], 0);
        }
        self.merge = false;
        self.normalize();
        self.show();
    }

    unsafe fn drag_to(&mut self, x: i32, y: i32) {
        self.sels[self.main].h = self.pos_from(x, y);
        self.normalize();
        self.show();
    }

    unsafe fn select_word(&mut self, x: i32, y: i32) {
        let p = self.pos_from(x, y);
        let n = self.text.len();
        let k = if p < n && self.text[p] != NL { class(self.text[p]) } else if p > 0 { class(self.text[p - 1]) } else { 3 };
        let (mut a, mut b) = (p, p);
        while k != 3 && a > 0 && class(self.text[a - 1]) == k {
            a -= 1;
        }
        while k != 3 && b < n && class(self.text[b]) == k {
            b += 1;
        }
        self.sels[self.main] = Sel { a, h: b, x: -1 };
        self.normalize();
        self.show();
    }

    unsafe fn client(&self) -> (i32, i32) {
        let mut rc: RECT = zeroed();
        GetClientRect(self.h, &mut rc);
        (rc.right, rc.bottom)
    }

    fn rows(&self) -> usize {
        let v = view();
        ((unsafe { self.client() }.1 - v.margin) / v.lh).max(1) as usize
    }

    unsafe fn scroll(&mut self, d: isize) {
        (self.top, self.top_sub) = self.step((self.top, self.top_sub), d).0;
        self.update();
    }

    unsafe fn scroll_to_line(&mut self, l: usize) {
        (self.top, self.top_sub) = (l, 0);
        self.update();
    }

    /// Topmost row that still fills the window.
    unsafe fn max_top(&self) -> (usize, usize) {
        let last = self.lines.len() - 1;
        self.step((last, self.row_count(last) - 1), 1 - self.rows() as isize).0
    }

    unsafe fn hscroll(&mut self, d: i32) {
        self.sx += d;
        self.update();
    }

    unsafe fn bar(&self, bar: SCROLLBAR_CONSTANTS) -> i32 {
        let mut si: SCROLLINFO = zeroed();
        si.cbSize = size_of::<SCROLLINFO>() as u32;
        si.fMask = SIF_TRACKPOS;
        GetScrollInfo(self.h, bar, &mut si);
        si.nTrackPos
    }

    /// Scrolls so the main caret is visible and restarts the caret blink.
    unsafe fn show(&mut self) {
        let s = self.sels[self.main];
        let (row, rows) = (self.row_of(s.h), self.rows() as isize);
        if row < (self.top, self.top_sub) {
            (self.top, self.top_sub) = row;
        } else if row > self.step((self.top, self.top_sub), rows - 1).0 {
            (self.top, self.top_sub) = self.step(row, 1 - rows).0;
        }
        let x = self.x_of(s.h);
        let w = (self.client().0 - self.left_edge() - view().margin).max(1);
        if x < self.sx {
            self.sx = (x - w / 4).max(0);
        } else if x > self.sx + w {
            self.sx = x - w * 3 / 4;
        }
        self.wmax = self.wmax.max(x);
        self.wake();
        self.update();
        PostMessageW(GetParent(self.h), CARET, 0, self.h as LPARAM);
    }

    unsafe fn wake(&mut self) {
        self.blink = true;
        SetTimer(self.h, 1, GetCaretBlinkTime(), None);
        InvalidateRect(self.h, null(), 0);
    }

    /// Syncs the scrollbars and repaints. Showing or hiding a scrollbar resizes the window;
    /// that WM_SIZE only posts UPDATE, so this never re-enters.
    unsafe fn update(&mut self) {
        let w = self.client().0 - self.gutter();
        let v = view();
        self.top = self.top.min(self.lines.len() - 1);
        self.top_sub = self.top_sub.min(self.row_count(self.top) - 1);
        (self.top, self.top_sub) = (self.top, self.top_sub).min(self.max_top());
        let m = v.margin;
        self.sx = if v.wrap { 0 } else { self.sx.min(self.wmax + 2 * m - w).max(0) };
        let mut si = SCROLLINFO {
            cbSize: size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
            nMin: 0,
            nMax: self.lines.len() as i32 - 1,
            nPage: self.rows() as u32,
            nPos: self.top as i32,
            nTrackPos: 0,
        };
        SetScrollInfo(self.h, SB_VERT, &si, 1);
        // With word wrap nothing is wider than the window: an empty range hides the scrollbar.
        (si.nMax, si.nPage, si.nPos) = (if v.wrap { 0 } else { self.wmax + 2 * m - 1 }, w.max(1) as u32, self.sx);
        SetScrollInfo(self.h, SB_HORZ, &si, 1);
        InvalidateRect(self.h, null(), 0);
    }

    unsafe fn paint(&mut self) {
        let mut ps: PAINTSTRUCT = zeroed();
        let hdc = BeginPaint(self.h, &mut ps);
        let (w, hgt) = self.client();
        let dc = CreateCompatibleDC(hdc);
        let bmp = CreateCompatibleBitmap(hdc, w.max(1), hgt.max(1));
        let old = SelectObject(dc, bmp);
        self.draw(dc, w, hgt);
        BitBlt(hdc, 0, 0, w, hgt, dc, 0, 0, SRCCOPY);
        SelectObject(dc, old);
        DeleteObject(bmp);
        DeleteDC(dc);
        EndPaint(self.h, &ps);
    }

    /// Draws the whole view into `dc`.
    unsafe fn draw(&mut self, dc: HDC, w: i32, hgt: i32) {
        let v = view();
        let fill = |l: i32, t: i32, r: i32, b: i32, color: u32| {
            let br = CreateSolidBrush(color);
            FillRect(dc, &RECT { left: l, top: t, right: r, bottom: b }, br);
            DeleteObject(br);
        };
        fill(0, 0, w, hgt, GetSysColor(COLOR_WINDOW));
        SelectObject(dc, v.font);
        SetBkMode(dc, TRANSPARENT as _);
        let fg = GetSysColor(COLOR_WINDOWTEXT);
        SetTextColor(dc, fg);
        let (gutter, x0) = (self.gutter(), self.left_edge() - self.sx);
        let caret_lines: Vec<usize> = self.sels.iter().map(|s| self.line_of(s.h)).collect();
        let cw = (v.lh / 12).max(2);
        let wmax = self.wmax;
        let sel_color = if self.focus { SEL_COLOR } else { SEL_COLOR_INACTIVE };
        let mut numbers = Vec::new(); // (y, line) of each line's first row, for the gutter
        let (mut l, mut first_sub, mut y) = (self.top, self.top_sub, v.margin);
        let mut ctx = syntax::context(self.lang, (0..self.lines.len()).map(|l| &self.text[self.line(l)]), l);
        while y < hgt && l < self.lines.len() {
            let r = self.line(l);
            let line = &self.text[r.clone()];
            let xs = xs(v, line);
            let b = self.breaks(l, &xs);
            self.wmax = self.wmax.max(xs[line.len()]);
            let mut colors = vec![fg; line.len()];
            for (r, style) in syntax::line_spans(self.lang, line, &mut ctx) {
                colors[r.start.min(line.len())..r.end.min(line.len())].fill(syntax::color(style));
            }
            for sub in first_sub..b.len() {
                if y >= hgt {
                    break;
                }
                // This row shows line[a..e]; `base` is where line offset 0 would be drawn.
                let (a, e, last) = (b[sub], b.get(sub + 1).copied().unwrap_or(line.len()), sub + 1 == b.len());
                let (ra, re, base) = (r.start + a, r.start + e, x0 - xs[a]);
                if sub == 0 {
                    numbers.push((y, l));
                }
                if v.highlight_line && caret_lines.contains(&l) {
                    fill(0, y, w, y + v.lh, LINE_COLOR);
                }
                for s in &self.sels {
                    let sr = s.r();
                    if !sr.is_empty() && sr.end > ra && (sr.start < re || (last && sr.start == re)) {
                        let sa = xs[sr.start.max(ra) - r.start];
                        let sb = xs[sr.end.min(re) - r.start] + if last && sr.end > re { v.space } else { 0 };
                        fill(base + sa, y, base + sb, y + v.lh, sel_color);
                    }
                }
                // Draw runs of one color, skipping tabs.
                let mut c = a;
                while c < e {
                    if line[c] == TAB {
                        c += 1;
                        continue;
                    }
                    let n = (c..e).take_while(|&k| line[k] != TAB && colors[k] == colors[c]).count();
                    SetTextColor(dc, colors[c]);
                    TextOutW(dc, base + xs[c], y, line[c..].as_ptr(), n as i32);
                    c += n;
                }
                for (i, s) in self.sels.iter().enumerate() {
                    if (ra..re).contains(&s.h) || (last && s.h == re) {
                        let x = base + xs[s.h - r.start];
                        if self.focus && self.blink {
                            fill(x, y, x + cw, y + v.lh, fg);
                        }
                        if i == self.main {
                            SetCaretPos(x, y); // invisible system caret, so IME windows appear here
                        }
                    }
                }
                y += v.lh;
            }
            (l, first_sub) = (l + 1, 0);
        }
        // The gutter is drawn last so horizontally scrolled text never shows through it.
        if gutter > 0 {
            fill(0, 0, gutter, hgt, GUTTER_COLOR);
            SetTextAlign(dc, TA_RIGHT);
            for (y, l) in numbers {
                SetTextColor(dc, if caret_lines.contains(&l) { fg } else { GetSysColor(COLOR_GRAYTEXT) });
                let n: Vec<u16> = (l + 1).to_string().encode_utf16().collect();
                TextOutW(dc, gutter - 2 * PAD, y, n.as_ptr(), n.len() as i32);
            }
            SetTextAlign(dc, TA_LEFT);
        }
        if self.wmax > wmax {
            PostMessageW(self.h, UPDATE, 0, 0); // a wider line came into view: grow the scrollbar
        }
    }
}

unsafe extern "system" fn proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let p = GetWindowLongPtrW(h, GWLP_USERDATA) as *mut Editor;
    if p.is_null() {
        return DefWindowProcW(h, m, wp, lp);
    }
    // Each arm creates its own short-lived &mut; calls that re-enter (SetFocus) happen outside of them.
    let (x, y) = ((lp & 0xFFFF) as i16 as i32, ((lp >> 16) & 0xFFFF) as i16 as i32);
    let delta = (wp >> 16) as i16 as i32;
    match m {
        WM_NCDESTROY => {
            SetWindowLongPtrW(h, GWLP_USERDATA, 0);
            drop(Box::from_raw(p));
            return DefWindowProcW(h, m, wp, lp);
        }
        WM_PAINT => (*p).paint(),
        WM_ERASEBKGND => return 1,
        WM_SIZE => _ = PostMessageW(h, UPDATE, 0, 0),
        UPDATE => (*p).update(),
        WM_SETFOCUS => {
            (*p).focus = true;
            CreateCaret(h, null_mut(), 1, view().lh);
            (*p).wake();
        }
        WM_KILLFOCUS => {
            (*p).focus = false;
            KillTimer(h, 1);
            DestroyCaret();
            InvalidateRect(h, null(), 0);
        }
        WM_TIMER => {
            (*p).blink = !(*p).blink;
            InvalidateRect(h, null(), 0);
        }
        WM_LBUTTONDOWN => {
            SetFocus(h);
            SetCapture(h);
            (*p).click(x, y);
        }
        WM_LBUTTONDBLCLK => (*p).select_word(x, y),
        WM_MOUSEMOVE if (*p).drag => (*p).drag_to(x, y),
        WM_LBUTTONUP => {
            (*p).drag = false;
            ReleaseCapture();
        }
        WM_MOUSEWHEEL => {
            let mut lines = 3u32;
            SystemParametersInfoW(SPI_GETWHEELSCROLLLINES, 0, &mut lines as *mut _ as _, 0);
            let e = &mut *p;
            e.wheel += delta * lines.max(1) as i32;
            let n = e.wheel / WHEEL_DELTA as i32;
            e.wheel %= WHEEL_DELTA as i32;
            e.scroll(-n as isize);
        }
        WM_MOUSEHWHEEL => (*p).hscroll(delta / 2),
        WM_VSCROLL => {
            let e = &mut *p;
            let page = e.rows() as isize;
            match (wp & 0xFFFF) as i32 {
                SB_LINEUP => e.scroll(-1),
                SB_LINEDOWN => e.scroll(1),
                SB_PAGEUP => e.scroll(-page),
                SB_PAGEDOWN => e.scroll(page),
                SB_THUMBTRACK | SB_THUMBPOSITION => e.scroll_to_line(e.bar(SB_VERT).max(0) as usize),
                SB_TOP => e.scroll_to_line(0),
                SB_BOTTOM => e.scroll_to_line(usize::MAX),
                _ => {}
            }
        }
        WM_HSCROLL => {
            let e = &mut *p;
            let (step, page) = (view().space * 4, e.client().0 / 2);
            let d = match (wp & 0xFFFF) as i32 {
                SB_LINELEFT => -step,
                SB_LINERIGHT => step,
                SB_PAGELEFT => -page,
                SB_PAGERIGHT => page,
                SB_THUMBTRACK | SB_THUMBPOSITION => e.bar(SB_HORZ) - e.sx,
                _ => 0,
            };
            e.hscroll(d);
        }
        WM_KEYDOWN => {
            if !(*p).key(wp as VIRTUAL_KEY) {
                return DefWindowProcW(h, m, wp, lp);
            }
        }
        WM_CHAR => (*p).char(wp as u16),
        // A fresh Alt press clears the Alt+click flag; the release after an Alt+click is swallowed.
        WM_SYSKEYDOWN if wp == VK_MENU as usize && lp & (1 << 30) == 0 => {
            (*p).alt_click = false;
            return DefWindowProcW(h, m, wp, lp);
        }
        WM_SYSKEYUP if wp == VK_MENU as usize && (*p).alt_click => (*p).alt_click = false,
        _ => return DefWindowProcW(h, m, wp, lp),
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(use_spaces: bool) -> Options {
        Options { tab_width: 4, use_spaces, line_numbers: true, highlight_line: true, margin: 8, word_wrap: false }
    }

    /// An editor inside a parent window that is never shown.
    unsafe fn editor(text: &str) -> &'static mut Editor {
        register();
        set_view(GetStockObject(ANSI_FIXED_FONT) as HFONT, &opts(false));
        let parent = CreateWindowExW(0, w!("STATIC"), null(), 0, 0, 0, 400, 300, null_mut(), null_mut(), null_mut(), null());
        let e = ed(create(parent));
        e.set_text(text.encode_utf16().collect());
        e
    }

    fn text(e: &Editor) -> String {
        String::from_utf16_lossy(&e.text)
    }

    fn carets(e: &mut Editor, ps: &[usize]) {
        e.sels = ps.iter().map(|&p| Sel::at(p)).collect();
        e.main = 0;
    }

    #[test]
    fn multi_cursor_typing_undo_redo() {
        unsafe {
            let e = editor("ab\ncd");
            carets(e, &[1, 4]);
            for c in "XY".encode_utf16() {
                e.char(c);
            }
            assert_eq!(text(e), "aXYb\ncXYd");
            assert_eq!(e.sels.iter().map(|s| s.h).collect::<Vec<_>>(), [3, 8]);
            assert!(e.dirty());
            e.undo(); // typing merged into one step
            assert_eq!(text(e), "ab\ncd");
            assert!(!e.dirty());
            e.redo();
            assert_eq!(text(e), "aXYb\ncXYd");
        }
    }

    #[test]
    fn backspace_and_overlapping_deletes() {
        unsafe {
            let e = editor("abcdef");
            carets(e, &[2, 3]);
            e.delete(false, false);
            assert_eq!(text(e), "adef");
            assert_eq!(e.sels.len(), 1); // both cursors met at 1 and merged
            e.delete(false, false);
            e.delete(false, false);
            assert_eq!(text(e), "def");
            e.undo();
            assert_eq!(text(e), "abcdef");
        }
    }

    #[test]
    fn typing_replaces_selections() {
        unsafe {
            let e = editor("one two one");
            e.sels = vec![Sel { a: 0, h: 3, x: -1 }, Sel { a: 8, h: 11, x: -1 }];
            e.char('1' as u16);
            assert_eq!(text(e), "1 two 1");
        }
    }

    #[test]
    fn newline_keeps_indent() {
        unsafe {
            let e = editor("\tfoo");
            carets(e, &[4]);
            e.char(CR);
            assert_eq!(text(e), "\tfoo\n\t");
        }
    }

    #[test]
    fn block_indent_and_unindent() {
        unsafe {
            let e = editor("a\nb\nc");
            e.sels = vec![Sel { a: 0, h: 3, x: -1 }]; // lines a and b
            e.tab(false);
            assert_eq!(text(e), "\ta\n\tb\nc");
            e.tab(true);
            assert_eq!(text(e), "a\nb\nc");
            e.undo();
            e.undo();
            assert_eq!(text(e), "a\nb\nc");
            assert!(!e.dirty());
        }
    }

    #[test]
    fn tab_inserts_spaces_to_next_stop() {
        unsafe {
            let e = editor("ab");
            set_view(GetStockObject(ANSI_FIXED_FONT) as HFONT, &opts(true));
            carets(e, &[2]);
            e.tab(false);
            assert_eq!(text(e), "ab  ");
        }
    }

    #[test]
    fn saved_state_survives_undo_redo_and_new_edits() {
        unsafe {
            let e = editor("");
            e.char('a' as u16);
            e.mark_saved();
            e.char('b' as u16); // must not merge into the saved step
            e.undo();
            assert_eq!(text(e), "a");
            assert!(!e.dirty());
            e.undo();
            assert!(e.dirty());
            e.char('z' as u16); // saved state now unreachable
            e.undo();
            assert_eq!(text(e), "");
            assert!(e.dirty());
        }
    }

    #[test]
    fn word_movement() {
        unsafe {
            let e = editor("foo  bar.baz\nx");
            assert_eq!(e.word_right(0), 5);
            assert_eq!(e.word_right(5), 8);
            assert_eq!(e.word_left(8), 5);
            assert_eq!(e.word_left(13), 12);
        }
    }

    #[test]
    fn word_wrap_breaks_after_spaces_and_moves_by_row() {
        unsafe {
            let e = editor(&"word ".repeat(60));
            set_view(GetStockObject(ANSI_FIXED_FONT) as HFONT, &Options { word_wrap: true, ..opts(false) });
            MoveWindow(e.h, 0, 0, 400, 300, 0);
            let (xs, b) = e.line_rows(0);
            assert!(b.len() > 2);
            for w in b.windows(2) {
                assert_eq!(e.text[w[1] - 1], SP); // every break follows a space
                assert!(xs[w[1]] - xs[w[0]] <= e.wrap_width());
            }
            carets(e, &[2]);
            e.vmove(false, 1);
            assert_eq!(e.row_of(e.sels[0].h), (0, 1));
            assert_eq!(e.sels[0].h, b[1] + 2); // same column on the next row
            e.vmove(false, -1);
            assert_eq!(e.sels[0].h, 2);
        }
    }

    /// Renders sample files into bitmaps (MINEDIT_RENDER_EDIT=<path prefix>) for a visual check.
    #[test]
    fn render_preview() {
        let Ok(prefix) = std::env::var("MINEDIT_RENDER_EDIT") else { return };
        let samples = [
            ("json", Lang::Json, "{\n  \"name\": \"minedit\",\n  \"version\": 1.5,\n  \"tags\": [\"rust\", \"win32\"],\n  \"fast\": true,\n  \"deps\": null\n}"),
            ("yaml", Lang::Yaml, "# service config\n---\nserver:\n  host: \"0.0.0.0\" # all interfaces\n  port: 8080\n  debug: false\nitems:\n  - name: web\n    base: &base\n  - *base\n"),
            ("md", Lang::Markdown, "# Title\n\nSome *emphasis*, **bold**, `code` and a [link](http://x).\n\n- list item\n1. numbered\n> quote\n\n```rust\n# not a heading\nfn main() {}\n```\n---"),
            ("xml", Lang::Xml, "<?xml version=\"1.0\"?>\n<!-- a comment\n     over two lines -->\n<root id=\"1\">\n  <item\n    name='a' enabled=\"true\">x &amp; y</item>\n  <empty/>\n</root>"),
        ];
        for (ext, lang, text) in samples {
            unsafe {
                let e = editor(text);
                e.set_lang(lang);
                let face: Vec<u16> = "Consolas".encode_utf16().chain(Some(0)).collect();
                let font = CreateFontW(-18, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, CLEARTYPE_QUALITY as _, 0, face.as_ptr());
                set_view(font, &Options { margin: 10, highlight_line: false, ..opts(false) });
                let (w, h) = (560, 300);
                MoveWindow(e.h, 0, 0, w, h, 0);
                let dc = CreateCompatibleDC(null_mut());
                let mut bits = null_mut();
                let mut bi: BITMAPINFO = zeroed();
                bi.bmiHeader = BITMAPINFOHEADER { biSize: 40, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, ..zeroed() };
                let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
                SelectObject(dc, bmp);
                e.draw(dc, w, h);
                GdiFlush();
                let px = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
                let mut f = b"BM".to_vec();
                f.extend((54 + px.len() as u32).to_le_bytes());
                f.extend([0u8; 4]);
                f.extend([54u32, 40].iter().flat_map(|v| v.to_le_bytes()));
                f.extend([w, -h].iter().flat_map(|v| v.to_le_bytes()));
                f.extend([1u16, 32].iter().flat_map(|v| v.to_le_bytes()));
                f.extend([0u8; 24]);
                f.extend(px);
                std::fs::write(format!("{prefix}_{ext}.bmp"), f).unwrap();
            }
        }
    }

    #[test]
    fn crlf_round_trip() {
        let t: Vec<u16> = "a\r\nb\nc".encode_utf16().collect();
        assert_eq!(String::from_utf16_lossy(&to_crlf(&to_lf(&t))), "a\r\nb\r\nc");
    }
}
