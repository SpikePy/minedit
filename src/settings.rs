//! Ctrl+,: the settings window — dropdowns and toggle switches that apply immediately.
//! Values are still stored in settings.ini (written by the caller's `on_change`).

use crate::theme;
use std::{cell::RefCell, mem::zeroed, ptr::{null, null_mut}};
use windows_sys::{
    w,
    Win32::{
        Foundation::*,
        Graphics::{Dwm::DwmSetWindowAttribute, Gdi::*},
        System::LibraryLoader::GetModuleHandleW,
        UI::{Controls::*, HiDpi::AdjustWindowRectExForDpi, WindowsAndMessaging::*},
    },
};

/// Current values shown in the window.
pub struct Values {
    pub face: String,
    pub size: i32,
    pub theme: String,
    pub scheme: String,
    pub tab_width: usize,
    pub use_spaces: bool,
    pub word_wrap: bool,
    pub line_numbers: bool,
    pub highlight_line: bool,
    pub auto_close: bool,
    pub margin: i32,
    pub recent: usize,
}

enum Kind {
    /// (label shown, value stored) choices.
    Choice(Vec<(String, String)>),
    Toggle,
}

struct Row {
    label: &'static str,
    key: &'static str,
    kind: Kind,
    current: String, // value stored in settings.ini
    ctl: HWND,
    label_ctl: HWND,
}

struct Window {
    hwnd: HWND,
    rows: Vec<Row>,
    buttons: Vec<HWND>,
    brush: HBRUSH,
    font: HFONT,
    on_change: unsafe fn(&str, &str),
}

thread_local! {
    static WIN: RefCell<Option<Window>> = const { RefCell::new(None) };
}

const ID_ROW: usize = 100;
const ID_OPEN_FILE: usize = 1;
const ACCENT: u32 = 0xD47800; // Windows blue #0078D4 as 0x00BBGGRR

fn px(v: i32, dpi: u32) -> i32 {
    v * dpi as i32 / 96
}

fn choices<T: ToString>(values: impl IntoIterator<Item = T>) -> Kind {
    Kind::Choice(values.into_iter().map(|v| (v.to_string(), v.to_string())).collect())
}

/// Fixed-pitch fonts installed on the system, sorted.
unsafe fn monospace_fonts() -> Vec<String> {
    unsafe extern "system" fn add(lf: *const LOGFONTW, _: *const TEXTMETRICW, _: u32, list: LPARAM) -> i32 {
        let lf = &*lf;
        let n = lf.lfFaceName.iter().position(|&c| c == 0).unwrap_or(32);
        let name = String::from_utf16_lossy(&lf.lfFaceName[..n]);
        if lf.lfPitchAndFamily & 3 == FIXED_PITCH && !name.starts_with('@') {
            (*(list as *mut Vec<String>)).push(name);
        }
        1
    }
    let mut list: Vec<String> = Vec::new();
    let dc = GetDC(null_mut());
    let mut lf: LOGFONTW = zeroed();
    lf.lfCharSet = DEFAULT_CHARSET;
    EnumFontFamiliesExW(dc, &lf, Some(add), &mut list as *mut _ as LPARAM, 0);
    ReleaseDC(null_mut(), dc);
    list.sort_by_key(|f| f.to_lowercase());
    list.dedup();
    list
}

fn rows(v: &Values, fonts: Vec<String>) -> Vec<Row> {
    let mut fonts = fonts;
    if !fonts.iter().any(|f| f.eq_ignore_ascii_case(&v.face)) {
        fonts.insert(0, v.face.clone());
    }
    let mut sizes: Vec<i32> = (8..=24).chain([26, 28, 32, 36, 48]).collect();
    if !sizes.contains(&v.size) {
        sizes.push(v.size);
        sizes.sort();
    }
    let mut margins = vec![0, 4, 8, 12, 16, 24, 32, 48];
    if !margins.contains(&v.margin) {
        margins.push(v.margin);
        margins.sort();
    }
    let mut recents = vec![0, 5, 10, 20, 30, 50, 100];
    if !recents.contains(&v.recent) {
        recents.push(v.recent);
        recents.sort();
    }
    let row = |label, key, kind, current: String| Row { label, key, kind, current, ctl: null_mut(), label_ctl: null_mut() };
    let on = |b: bool| (if b { "true" } else { "false" }).to_string();
    vec![
        row("Font", "font", choices(fonts), v.face.clone()),
        row("Font size", "size", choices(sizes), v.size.to_string()),
        row("Theme", "theme", Kind::Choice(vec![
            ("Follow Windows".into(), "system".into()), ("Light".into(), "light".into()), ("Dark".into(), "dark".into()),
        ]), v.theme.clone()),
        row("Color scheme", "color_scheme", choices(theme::SCHEMES.iter().map(|s| s.name)), v.scheme.clone()),
        row("Tab width", "tab_width", choices(1..=8), v.tab_width.to_string()),
        row("Insert spaces for Tab", "use_spaces", Kind::Toggle, on(v.use_spaces)),
        row("Word wrap", "word_wrap", Kind::Toggle, on(v.word_wrap)),
        row("Line numbers", "line_numbers", Kind::Toggle, on(v.line_numbers)),
        row("Highlight the current line", "highlight_line", Kind::Toggle, on(v.highlight_line)),
        row("Auto-close brackets and quotes", "auto_close", Kind::Toggle, on(v.auto_close)),
        row("Margin around the text", "margin", choices(margins), v.margin.to_string()),
        row("Recent files in Ctrl+P", "recent_files", choices(recents), v.recent.to_string()),
    ]
}

pub fn window() -> HWND {
    WIN.with(|w| w.borrow().as_ref().map_or(null_mut(), |w| w.hwnd))
}

/// Opens the settings window over `owner` (or brings it to the front).
pub unsafe fn show(owner: HWND, values: Values, font: HFONT, dpi: u32, on_change: unsafe fn(&str, &str)) {
    let open = window();
    if !open.is_null() {
        SetForegroundWindow(open);
        return;
    }
    let hwnd = create(owner, values, font, dpi, on_change);
    ShowWindow(hwnd, SW_SHOW);
}

/// Creates the (hidden) window with all its controls.
unsafe fn create(owner: HWND, values: Values, font: HFONT, dpi: u32, on_change: unsafe fn(&str, &str)) -> HWND {
    let p = |v| px(v, dpi);
    // An own copy: the editor deletes its font when a setting changes, which would leave the controls
    // with the bitmap system font.
    let mut lf: LOGFONTW = zeroed();
    GetObjectW(font, size_of::<LOGFONTW>() as i32, &mut lf as *mut _ as _);
    let font = CreateFontIndirectW(&lf);
    let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: GetModuleHandleW(null()), lpszClassName: w!("minedit_settings"),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW), ..zeroed() };
    RegisterClassW(&wc); // fails harmlessly when already registered
    let mut rows = rows(&values, monospace_fonts());
    let (row_h, label_w, ctl_w, pad) = (p(38), p(230), p(240), p(20));
    let width = pad * 2 + label_w + ctl_w;
    let height = pad * 2 + rows.len() as i32 * row_h + p(50);
    // Outer size for the wanted client size.
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
    let mut rc = RECT { left: 0, top: 0, right: width, bottom: height };
    AdjustWindowRectExForDpi(&mut rc, style, 0, WS_EX_DLGMODALFRAME, dpi);
    let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
    let mut owner_rc: RECT = zeroed();
    GetWindowRect(owner, &mut owner_rc);
    let hwnd = CreateWindowExW(WS_EX_DLGMODALFRAME, w!("minedit_settings"), w!("Settings"), style,
        (owner_rc.left + owner_rc.right - w) / 2, (owner_rc.top + owner_rc.bottom - h) / 2, w, h,
        owner, null_mut(), GetModuleHandleW(null()), null());
    let hinst = GetModuleHandleW(null());
    let child = |class: *const u16, text: &str, style: u32, x, y, cw, ch, id: usize| {
        let c = CreateWindowExW(0, class, crate::wide(text).as_ptr(), WS_CHILD | WS_VISIBLE | style, x, y, cw, ch, hwnd, id as HMENU, hinst, null());
        SendMessageW(c, WM_SETFONT, font as WPARAM, 0);
        c
    };
    for (i, r) in rows.iter_mut().enumerate() {
        let y = pad + i as i32 * row_h;
        match &r.kind {
            Kind::Choice(options) => {
                r.label_ctl = child(w!("STATIC"), r.label, 0x200 /* SS_CENTERIMAGE: vertically centered */, pad, y, label_w, p(28), 0xFFFF);
                r.ctl = child(w!("COMBOBOX"), "", CBS_DROPDOWNLIST as u32 | WS_VSCROLL | WS_TABSTOP, pad + label_w, y, ctl_w, p(300), ID_ROW + i);
                for (label, _) in options {
                    SendMessageW(r.ctl, CB_ADDSTRING, 0, crate::wide(label).as_ptr() as LPARAM);
                }
                let sel = options.iter().position(|(_, v)| v.eq_ignore_ascii_case(&r.current)).unwrap_or(0);
                SendMessageW(r.ctl, CB_SETCURSEL, sel, 0);
            }
            Kind::Toggle => {
                // The whole row is one owner-drawn button: label on the left, switch on the right.
                r.ctl = child(w!("BUTTON"), r.label, BS_OWNERDRAW as u32 | WS_TABSTOP, pad, y, label_w + ctl_w, p(28), ID_ROW + i);
            }
        }
    }
    let by = height - pad - p(30);
    let buttons = vec![
        child(w!("BUTTON"), "Open settings file", BS_PUSHBUTTON as u32 | WS_TABSTOP, pad, by, p(160), p(30), ID_OPEN_FILE),
        child(w!("BUTTON"), "Close", BS_DEFPUSHBUTTON as u32 | WS_TABSTOP, width - pad - p(100), by, p(100), p(30), IDCANCEL as usize),
    ];
    WIN.with(|wn| *wn.borrow_mut() = Some(Window { hwnd, rows, buttons, brush: null_mut(), font, on_change }));
    recolor();
    hwnd
}

/// Applies the current theme (also after the color scheme was changed in this window).
pub unsafe fn recolor() {
    // Only swap the brush while borrowed: the calls below send messages that read WIN again.
    let Some((hwnd, combos, buttons, old)) = WIN.with(|wn| {
        let mut wn = wn.borrow_mut();
        let w = wn.as_mut()?;
        let old = std::mem::replace(&mut w.brush, CreateSolidBrush(theme::palette().bg));
        let combos: Vec<HWND> = w.rows.iter().filter(|r| matches!(r.kind, Kind::Choice(_))).map(|r| r.ctl).collect();
        Some((w.hwnd, combos, w.buttons.clone(), old))
    }) else { return };
    let dark = theme::is_dark();
    let flag = dark as i32;
    DwmSetWindowAttribute(hwnd, 20 /* DWMWA_USE_IMMERSIVE_DARK_MODE */, &flag as *const _ as _, 4);
    for c in combos {
        SetWindowTheme(c, if dark { w!("DarkMode_CFD") } else { w!("CFD") }, null());
    }
    for b in buttons {
        SetWindowTheme(b, if dark { w!("DarkMode_Explorer") } else { w!("Explorer") }, null());
    }
    RedrawWindow(hwnd, null(), null_mut(), RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_FRAME);
    if !old.is_null() {
        DeleteObject(old);
    }
}

/// Draws a toggle row: label left, a Windows 11 style switch right.
unsafe fn draw_toggle(d: &DRAWITEMSTRUCT, label: &str, on: bool) {
    let dc = d.hDC;
    let c = theme::palette();
    let rc = d.rcItem;
    let dpi = windows_sys::Win32::UI::HiDpi::GetDpiForWindow(d.hwndItem);
    let p = |v| px(v, dpi);
    let fill = |r: RECT, color: u32| {
        let b = CreateSolidBrush(color);
        FillRect(dc, &r, b);
        DeleteObject(b);
    };
    fill(rc, c.bg);
    SetBkMode(dc, TRANSPARENT as _);
    SetTextColor(dc, c.fg);
    SelectObject(dc, SendMessageW(d.hwndItem, WM_GETFONT, 0, 0) as HFONT);
    let mut text = crate::wide(label);
    let mut lr = rc;
    DrawTextW(dc, text.as_mut_ptr(), -1, &mut lr, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
    // Switch: a pill with a round knob, accent-colored when on.
    let (sw, sh) = (p(40), p(20));
    let x = rc.right - sw - p(4);
    let y = (rc.top + rc.bottom - sh) / 2;
    let track = if on { ACCENT } else { c.bg };
    let border = if on { ACCENT } else { c.inactive_text };
    let brush = CreateSolidBrush(track);
    let pen = CreatePen(PS_SOLID, p(1).max(1), border);
    let (ob, op) = (SelectObject(dc, brush), SelectObject(dc, pen));
    RoundRect(dc, x, y, x + sw, y + sh, sh, sh);
    let knob = if on { 0xFFFFFF } else { c.inactive_text };
    let kb = CreateSolidBrush(knob);
    let kp = CreatePen(PS_SOLID, 1, knob);
    SelectObject(dc, kb);
    SelectObject(dc, kp);
    let k = sh - p(8);
    let kx = if on { x + sw - p(4) - k } else { x + p(4) };
    Ellipse(dc, kx, y + p(4), kx + k, y + p(4) + k);
    SelectObject(dc, ob);
    SelectObject(dc, op);
    for o in [brush as HGDIOBJ, pen, kb, kp] {
        DeleteObject(o);
    }
    if d.itemState & ODS_FOCUS != 0 {
        let f = RECT { left: rc.left, top: rc.top + p(2), right: rc.right, bottom: rc.bottom - p(2) };
        DrawFocusRect(dc, &f);
    }
}

unsafe extern "system" fn proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_ERASEBKGND => {
            let mut rc: RECT = zeroed();
            GetClientRect(h, &mut rc);
            let b = CreateSolidBrush(theme::palette().bg);
            FillRect(wp as HDC, &rc, b);
            DeleteObject(b);
            1
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN | WM_CTLCOLORLISTBOX | WM_CTLCOLOREDIT => {
            let c = theme::palette();
            SetTextColor(wp as HDC, c.fg);
            SetBkColor(wp as HDC, c.bg);
            WIN.with(|wn| wn.borrow().as_ref().map_or(0, |w| w.brush as LRESULT))
        }
        WM_DRAWITEM => {
            let d = &*(lp as *const DRAWITEMSTRUCT);
            let info = WIN.with(|wn| wn.borrow().as_ref().and_then(|w| {
                w.rows.iter().find(|r| r.ctl == d.hwndItem).map(|r| (r.label, r.current == "true"))
            }));
            if let Some((label, on)) = info {
                draw_toggle(d, label, on);
            }
            1
        }
        WM_COMMAND => {
            let (id, code) = (wp & 0xFFFF, (wp >> 16) & 0xFFFF);
            match id {
                ID_OPEN_FILE => {
                    DestroyWindow(h);
                    crate::open_settings();
                }
                x if x == IDCANCEL as usize => _ = DestroyWindow(h),
                x if x >= ID_ROW => {
                    // A dropdown choice or a toggle click: store the new value and tell the editor.
                    let change = WIN.with(|wn| {
                        let mut wn = wn.borrow_mut();
                        let w = wn.as_mut()?;
                        let r = w.rows.get_mut(x - ID_ROW)?;
                        let value = match &r.kind {
                            Kind::Choice(options) if code as u32 == CBN_SELCHANGE => {
                                let i = SendMessageW(r.ctl, CB_GETCURSEL, 0, 0);
                                options.get(usize::try_from(i).ok()?)?.1.clone()
                            }
                            Kind::Toggle if code as u32 == BN_CLICKED => (if r.current == "true" { "false" } else { "true" }).into(),
                            _ => return None,
                        };
                        r.current = value.clone();
                        InvalidateRect(r.ctl, null(), 0);
                        Some((r.key, value, w.on_change))
                    });
                    if let Some((key, value, on_change)) = change {
                        on_change(key, &value);
                        recolor();
                    }
                }
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(h);
            0
        }
        WM_DESTROY => {
            WIN.with(|wn| {
                if let Some(w) = wn.borrow_mut().take() {
                    DeleteObject(w.brush);
                    DeleteObject(w.font);
                }
            });
            0
        }
        _ => DefWindowProcW(h, m, wp, lp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws the window, never shown, in light and dark (MINEDIT_RENDER_SETTINGS=<path prefix>).
    #[test]
    fn render_preview() {
        let Ok(prefix) = std::env::var("MINEDIT_RENDER_SETTINGS") else { return };
        unsafe {
            unsafe fn ignore(_: &str, _: &str) {}
            let owner = CreateWindowExW(0, w!("STATIC"), null(), 0, 0, 0, 1200, 900, null_mut(), null_mut(), null_mut(), null());
            let font = CreateFontW(-15, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, CLEARTYPE_QUALITY as _, 0, w!("Segoe UI"));
            for dark in [false, true] {
                theme::apply("Visual Studio", dark);
                let v = Values {
                    face: "Consolas".into(), size: 11, theme: "system".into(), scheme: "Visual Studio".into(), tab_width: 4,
                    use_spaces: false, word_wrap: true, line_numbers: true, highlight_line: true, auto_close: false, margin: 8, recent: 20,
                };
                let h = create(owner, v, font, 96, ignore);
                // Each dropdown shows the current value.
                let shown: Vec<String> = WIN.with(|wn| wn.borrow().as_ref().unwrap().rows.iter().filter(|r| matches!(r.kind, Kind::Choice(_))).map(|r| {
                    let i = SendMessageW(r.ctl, CB_GETCURSEL, 0, 0);
                    let mut buf = [0u16; 64];
                    let n = SendMessageW(r.ctl, CB_GETLBTEXT, i as WPARAM, buf.as_mut_ptr() as LPARAM);
                    String::from_utf16_lossy(&buf[..n.max(0) as usize])
                }).collect());
                assert_eq!(shown, ["Consolas", "11", "Follow Windows", "Visual Studio", "4", "8", "20"]);
                let mut rc: RECT = zeroed();
                GetClientRect(h, &mut rc);
                let (w, ht) = (rc.right, rc.bottom);
                let dc = CreateCompatibleDC(null_mut());
                let mut bits = null_mut();
                let mut bi: BITMAPINFO = zeroed();
                bi.bmiHeader = BITMAPINFOHEADER { biSize: 40, biWidth: w, biHeight: -ht, biPlanes: 1, biBitCount: 32, ..zeroed() };
                let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
                SelectObject(dc, bmp);
                SendMessageW(h, WM_PRINT, dc as WPARAM, (PRF_CLIENT | PRF_CHILDREN | PRF_ERASEBKGND) as LPARAM);
                GdiFlush();
                let px = std::slice::from_raw_parts(bits as *const u8, (w * ht * 4) as usize);
                let mut f = b"BM".to_vec();
                f.extend((54 + px.len() as u32).to_le_bytes());
                f.extend([0u8; 4]);
                f.extend([54u32, 40].iter().flat_map(|v| v.to_le_bytes()));
                f.extend([w, -ht].iter().flat_map(|v| v.to_le_bytes()));
                f.extend([1u16, 32].iter().flat_map(|v| v.to_le_bytes()));
                f.extend([0u8; 24]);
                f.extend(px);
                std::fs::write(format!("{prefix}_{}.bmp", if dark { "dark" } else { "light" }), f).unwrap();
                DestroyWindow(h);
            }
        }
    }

    /// Re-theming while the window is open. (The crash this guards against needed an open
    /// dropdown list, whose WM_CTLCOLORLISTBOX re-entered `recolor`; a test cannot open one invisibly.)
    #[test]
    fn recolor_while_open() {
        unsafe {
            unsafe fn ignore(_: &str, _: &str) {}
            let owner = CreateWindowExW(0, w!("STATIC"), null(), 0, 0, 0, 1200, 900, null_mut(), null_mut(), null_mut(), null());
            let font = CreateFontW(-15, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, CLEARTYPE_QUALITY as _, 0, w!("Segoe UI"));
            let v = Values {
                face: "Consolas".into(), size: 11, theme: "system".into(), scheme: "Visual Studio".into(), tab_width: 4,
                use_spaces: false, word_wrap: true, line_numbers: true, highlight_line: true, auto_close: false, margin: 8, recent: 20,
            };
            let h = create(owner, v, font, 96, ignore);
            // The editor deletes its font when a setting changes; the controls must not use it.
            DeleteObject(font);
            let ctls: Vec<HWND> = WIN.with(|wn| wn.borrow().as_ref().unwrap().rows.iter().map(|r| r.ctl).collect());
            for c in ctls {
                let f = SendMessageW(c, WM_GETFONT, 0, 0) as HFONT;
                assert!(f != font && GetObjectW(f, size_of::<LOGFONTW>() as i32, null_mut()) > 0);
            }
            for dark in [true, false, true] {
                theme::apply("Dracula", dark);
                recolor();
                theme::apply("GitHub", dark);
                recolor();
            }
            DestroyWindow(h);
            DestroyWindow(owner);
        }
    }

    #[test]
    fn rows_preselect_current_values() {
        let v = Values {
            face: "My Font".into(), size: 13, theme: "dark".into(), scheme: "Nord".into(), tab_width: 4, use_spaces: true,
            word_wrap: false, line_numbers: true, highlight_line: true, auto_close: false, margin: 10, recent: 20,
        };
        let rows = rows(&v, vec!["Consolas".into()]);
        let find = |key: &str| rows.iter().find(|r| r.key == key).unwrap();
        // Unlisted current values are added so they can be shown.
        let Kind::Choice(fonts) = &find("font").kind else { panic!() };
        assert_eq!(fonts[0].1, "My Font");
        let Kind::Choice(sizes) = &find("size").kind else { panic!() };
        assert!(sizes.iter().any(|s| s.1 == "13"));
        let Kind::Choice(margins) = &find("margin").kind else { panic!() };
        assert!(margins.iter().any(|s| s.1 == "10"));
        assert_eq!(find("use_spaces").current, "true");
        assert_eq!(find("auto_close").current, "false");
        assert_eq!(find("theme").current, "dark");
        // Every key is one the settings file understands.
        for r in &rows {
            assert!(crate::DEFAULT_SETTINGS.contains(&format!("{} =", r.key)), "{} not in DEFAULT_SETTINGS", r.key);
        }
    }
}
