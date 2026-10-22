//! The title bar: tabs and caption buttons drawn in the main window's client area.

use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*};

// Glyphs from Segoe Fluent Icons / Segoe MDL2 Assets.
const GLYPH_CLOSE: &str = "\u{E8BB}";
const GLYPH_ADD: &str = "\u{E710}";
const GLYPH_MENU: &str = "\u{E700}";
const GLYPH_TREE: &str = "\u{E90C}"; // DockLeft: the sidebar
const GLYPH_MIN: &str = "\u{E921}";
const GLYPH_MAX: &str = "\u{E922}";
const GLYPH_RESTORE: &str = "\u{E923}";

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    None,
    Tab(usize),
    TabClose(usize),
    NewTab,
    Tree,
    Menu,
    Min,
    Max,
    Close,
    Caption,
}

pub struct Rects {
    pub height: i32,
    pub clip: RECT,       // visible tab area; tabs scrolled out of it are hidden
    pub max_scroll: i32,  // how far the tabs can scroll when they do not fit
    pub tabs: Vec<RECT>,
    pub closes: Vec<RECT>,
    pub new_tab: RECT,
    pub tree: RECT,
    pub menu: RECT,
    pub min: RECT,
    pub max: RECT,
    pub close: RECT,
}

/// What the bar looks like right now.
pub struct Look {
    pub font: HFONT,
    pub icons: HFONT,
    pub small_icons: HFONT,
    pub dpi: u32,
    pub active: bool,
    pub maximized: bool,
    pub hover: Hit,
    pub tree_open: bool,
}

fn px(v: i32, dpi: u32) -> i32 {
    v * dpi as i32 / 96
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
    RECT { left, top, right, bottom }
}

fn inside(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// Positions of everything in the bar for a window `width` pixels wide with `n` tabs,
/// scrolled `scroll` pixels to the left when they do not fit.
pub fn layout(width: i32, dpi: u32, n: usize, scroll: i32) -> Rects {
    let p = |v| px(v, dpi);
    let (h, bw) = (p(32), p(46));
    let close = rect(width - bw, 0, width, h);
    let max = rect(close.left - bw, 0, close.left, h);
    let min = rect(max.left - bw, 0, max.left, h);
    let menu = rect(min.left - bw, 0, min.left, h);
    let (top, plus) = (p(4), p(32));
    // The file tree button comes first, then the tabs.
    let tree = rect(p(6), top + p(2), p(6) + plus, h - p(2));
    let x0 = tree.right + p(4);
    // Tabs share the space, leaving some empty title bar to drag the window by.
    let avail = (menu.left - x0 - plus - p(40)).max(p(60));
    let tw = (avail / n.max(1) as i32).clamp(p(60), p(200));
    let max_scroll = (n as i32 * tw - avail).max(0);
    let sx = scroll.clamp(0, max_scroll);
    let tabs: Vec<RECT> = (0..n as i32).map(|i| rect(x0 + i * tw - sx, top, x0 + (i + 1) * tw - sx, h)).collect();
    let cs = p(20);
    let closes = tabs.iter().map(|t| {
        let y = (t.top + t.bottom - cs) / 2;
        rect(t.right - p(6) - cs, y, t.right - p(6), y + cs)
    }).collect();
    let nx = (x0 + n as i32 * tw - sx).min(x0 + avail) + p(4);
    let clip = rect(x0, 0, x0 + avail.min(n as i32 * tw), h);
    Rects { height: h, clip, max_scroll, tabs, closes, new_tab: rect(nx, top + p(2), nx + plus, h - p(2)), tree, menu, min, max, close }
}

pub fn hit(r: &Rects, x: i32, y: i32) -> Hit {
    if y < 0 || y >= r.height {
        return Hit::None;
    }
    for (rc, h) in [(&r.close, Hit::Close), (&r.max, Hit::Max), (&r.min, Hit::Min), (&r.menu, Hit::Menu)] {
        if inside(rc, x, y) {
            return h;
        }
    }
    if inside(&r.clip, x, y) {
        if let Some(i) = r.closes.iter().position(|c| inside(c, x, y)) {
            return Hit::TabClose(i);
        }
        if let Some(i) = r.tabs.iter().position(|t| inside(t, x, y)) {
            return Hit::Tab(i);
        }
    }
    if inside(&r.new_tab, x, y) {
        return Hit::NewTab;
    }
    if inside(&r.tree, x, y) {
        return Hit::Tree;
    }
    Hit::Caption
}

unsafe fn fill(dc: HDC, r: &RECT, color: u32) {
    let b = CreateSolidBrush(color);
    FillRect(dc, r, b);
    DeleteObject(b);
}

/// A rectangle with rounded top corners, like a browser tab.
unsafe fn tab_shape(dc: HDC, r: &RECT, color: u32, radius: i32) {
    let b = CreateSolidBrush(color);
    let old = (SelectObject(dc, b), SelectObject(dc, GetStockObject(NULL_PEN)));
    RoundRect(dc, r.left, r.top, r.right + 1, r.bottom + radius, radius * 2, radius * 2);
    SelectObject(dc, old.0);
    SelectObject(dc, old.1);
    DeleteObject(b);
}

unsafe fn text(dc: HDC, font: HFONT, s: &str, r: &RECT, color: u32, flags: DRAW_TEXT_FORMAT) {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    let mut rc = *r;
    SelectObject(dc, font);
    SetTextColor(dc, color);
    DrawTextW(dc, w.as_mut_ptr(), w.len() as i32, &mut rc, flags | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
}

/// Draws the bar into `dc`. `tabs` holds each tab's label and modified state.
pub unsafe fn paint(dc: HDC, width: i32, r: &Rects, tabs: &[(String, bool)], cur: usize, l: &Look) {
    let p = |v| px(v, l.dpi);
    let c = crate::theme::palette();
    let fg = if l.active { c.fg } else { c.inactive_text };
    fill(dc, &rect(0, 0, width, r.height), c.strip);
    SetBkMode(dc, TRANSPARENT as _);
    // Tabs scrolled out of view are clipped.
    SaveDC(dc);
    IntersectClipRect(dc, r.clip.left, 0, r.clip.right, r.height);
    for (i, ((label, dirty), t)) in tabs.iter().zip(&r.tabs).enumerate() {
        let hovered = matches!(l.hover, Hit::Tab(j) | Hit::TabClose(j) if j == i);
        if i == cur {
            tab_shape(dc, t, c.bg, p(6));
        } else if hovered {
            tab_shape(dc, &rect(t.left + p(2), t.top + p(2), t.right - p(2), t.bottom - p(3)), c.tab_hover, p(4));
        } else if i + 1 != cur && i + 1 < tabs.len() {
            fill(dc, &rect(t.right - 1, t.top + p(8), t.right, t.bottom - p(6)), c.separator);
        }
        let x = r.closes[i];
        let show_close = i == cur || hovered;
        let label_rect = rect(t.left + p(12), t.top, if show_close { x.left - p(2) } else { t.right - p(12) }, t.bottom);
        let label = if *dirty { format!("*{label}") } else { label.clone() };
        text(dc, l.font, &label, &label_rect, fg, DT_END_ELLIPSIS);
        if show_close {
            if l.hover == Hit::TabClose(i) {
                tab_shape(dc, &rect(x.left, x.top, x.right, x.bottom - p(4)), c.hover, p(4));
            }
            text(dc, l.small_icons, GLYPH_CLOSE, &x, fg, DT_CENTER);
        }
    }
    RestoreDC(dc, -1);
    if l.hover == Hit::NewTab {
        tab_shape(dc, &rect(r.new_tab.left, r.new_tab.top, r.new_tab.right, r.new_tab.bottom - p(4)), c.hover, p(4));
    }
    text(dc, l.icons, GLYPH_ADD, &r.new_tab, fg, DT_CENTER);
    // The file tree button stays highlighted while the tree is shown.
    let tr = rect(r.tree.left, r.tree.top, r.tree.right, r.tree.bottom - p(4));
    if l.hover == Hit::Tree {
        tab_shape(dc, &tr, c.hover, p(4));
    } else if l.tree_open {
        tab_shape(dc, &tr, c.tab_hover, p(4));
    }
    text(dc, l.icons, GLYPH_TREE, &tr, fg, DT_CENTER);
    let max_glyph = if l.maximized { GLYPH_RESTORE } else { GLYPH_MAX };
    for (rc, h, glyph) in [(&r.menu, Hit::Menu, GLYPH_MENU), (&r.min, Hit::Min, GLYPH_MIN), (&r.max, Hit::Max, max_glyph), (&r.close, Hit::Close, GLYPH_CLOSE)] {
        let hot = l.hover == h;
        if hot {
            fill(dc, rc, if h == Hit::Close { c.close_red } else { c.hover });
        }
        text(dc, l.icons, glyph, rc, if hot && h == Hit::Close { 0xFFFFFF } else { fg }, DT_CENTER);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::null_mut;

    #[test]
    fn buttons_are_right_aligned_and_scale() {
        for dpi in [96, 144, 192] {
            let r = layout(1000, dpi, 1, 0);
            let bw = px(46, dpi);
            assert_eq!((r.close.right, r.close.left), (1000, 1000 - bw));
            assert_eq!((r.max.right, r.min.right, r.menu.right), (r.close.left, r.max.left, r.min.left));
            assert_eq!(r.height, px(32, dpi));
        }
    }

    #[test]
    fn tab_widths_clamp_and_shrink() {
        let w = |n| { let r = layout(1200, 96, n, 0); r.tabs[0].right - r.tabs[0].left };
        assert_eq!(w(1), 200);
        assert!(w(5) < 200 && w(5) > 60);
        assert_eq!(w(50), 60);
        let r = layout(1200, 96, 3, 0);
        assert!(r.tabs.windows(2).all(|t| t[0].right == t[1].left));
        assert!(r.new_tab.left >= r.tabs[2].right);
    }

    #[test]
    fn tabs_scroll_when_they_do_not_fit() {
        let r = layout(1000, 96, 30, 0);
        assert!(r.max_scroll > 0);
        assert_eq!(r.tabs[0].left, r.clip.left);
        let s = layout(1000, 96, 30, 100_000); // clamped to the end
        assert_eq!(s.tabs[29].right, s.clip.right);
        assert!(s.new_tab.left >= s.clip.right);
        assert_eq!(hit(&s, s.clip.left + 2, 30), Hit::Tab(29 - (s.clip.right - s.clip.left) as usize / 60));
        assert_eq!(layout(1000, 96, 3, 50).max_scroll, 0); // fits: no scrolling
    }

    #[test]
    fn hit_testing() {
        let r = layout(1000, 96, 2, 0);
        let mid = |rc: &RECT| ((rc.left + rc.right) / 2, (rc.top + rc.bottom) / 2);
        let at = |rc: &RECT| { let (x, y) = mid(rc); hit(&r, x, y) };
        assert_eq!(at(&r.close), Hit::Close);
        assert_eq!(at(&r.max), Hit::Max);
        assert_eq!(at(&r.min), Hit::Min);
        assert_eq!(at(&r.menu), Hit::Menu);
        assert_eq!(at(&r.closes[1]), Hit::TabClose(1));
        assert_eq!(hit(&r, r.tabs[0].left + 5, 20), Hit::Tab(0));
        assert_eq!(at(&r.new_tab), Hit::NewTab);
        assert_eq!(at(&r.tree), Hit::Tree);
        assert!(r.tree.right < r.tabs[0].left);
        assert_eq!(hit(&r, r.menu.left - 5, 10), Hit::Caption);
        assert_eq!(hit(&r, 100, 1), Hit::Caption); // above the tabs: drag area
        assert_eq!(hit(&r, 100, 40), Hit::None);
    }

    /// Renders a few states into a bitmap file (MINEDIT_RENDER=<path>) for a visual check.
    #[test]
    fn render_preview() {
        let Ok(path) = std::env::var("MINEDIT_RENDER") else { return };
        unsafe {
            let font = |face: &str, px: i32| {
                let face: Vec<u16> = face.encode_utf16().chain(Some(0)).collect();
                CreateFontW(-px, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, CLEARTYPE_QUALITY as _, 0, face.as_ptr())
            };
            let (w, dpi) = (900, 120);
            let tabs = [("main.rs".to_string(), false), ("notes with a very long name.md".to_string(), true), ("settings.ini".to_string(), false)];
            let states = [
                (1, true, false, Hit::Tab(2)),
                (0, true, true, Hit::Close),
                (1, false, false, Hit::TabClose(1)),
                (0, true, false, Hit::Menu),
                (1, true, false, Hit::Tab(2)), // the same again in dark
                (0, true, true, Hit::Close),
                (1, false, false, Hit::TabClose(1)),
                (0, true, false, Hit::Menu),
            ];
            let bh = layout(w, dpi, 3, 0).height;
            let h = bh * states.len() as i32 + 8 * (states.len() as i32 - 1);
            let dc = CreateCompatibleDC(null_mut());
            let mut bits = null_mut();
            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader = BITMAPINFOHEADER { biSize: 40, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, ..std::mem::zeroed() };
            let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            SelectObject(dc, bmp);
            fill(dc, &rect(0, 0, w, h), 0x808080);
            for (k, &(cur, active, maximized, hover)) in states.iter().enumerate() {
                crate::theme::set_dark(k >= 4);
                let n = if k % 4 == 3 { 1 } else { 3 };
                let r = layout(w, dpi, n, 0);
                let look = Look {
                    font: font("Segoe UI", 15), icons: font("Segoe Fluent Icons", 12), small_icons: font("Segoe Fluent Icons", 10),
                    dpi, active, maximized, hover, tree_open: k % 2 == 0,
                };
                SetViewportOrgEx(dc, 0, k as i32 * (bh + 8), null_mut());
                paint(dc, w, &r, &tabs[..n], cur, &look);
            }
            GdiFlush();
            let px = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
            let mut f = Vec::new();
            let size = 54 + px.len() as u32;
            f.extend(b"BM");
            f.extend(size.to_le_bytes());
            f.extend([0u8; 4]);
            f.extend(54u32.to_le_bytes());
            f.extend(40u32.to_le_bytes());
            f.extend(w.to_le_bytes());
            f.extend((-h).to_le_bytes());
            f.extend(1u16.to_le_bytes());
            f.extend(32u16.to_le_bytes());
            f.extend([0u8; 24]);
            f.extend(px);
            std::fs::write(path, f).unwrap();
        }
    }
}
