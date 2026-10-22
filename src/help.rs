//! Ctrl+/: a popup listing all keyboard shortcuts in groups.

use crate::theme;
use std::{cell::Cell, mem::zeroed, ptr::{null, null_mut}};
use windows_sys::{
    w,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
};

pub const GROUPS: [(&str, &[(&str, &str)]); 6] = [
    ("Files", &[
        ("Ctrl+O", "Open"),
        ("Ctrl+P", "Quick open: open and recent files"),
        ("Ctrl+S", "Save"),
        ("Ctrl+Shift+S", "Save as"),
        ("Drag files onto the window", "Open them"),
    ]),
    ("Tabs and windows", &[
        ("Ctrl+T, Ctrl+N", "New tab"),
        ("Ctrl+Shift+N", "New window"),
        ("Middle-click taskbar button", "New window"),
        ("Ctrl+W, middle-click", "Close tab"),
        ("Ctrl+Shift+T", "Reopen closed tab"),
        ("Ctrl+Tab, Ctrl+PgDn", "Next tab"),
        ("Ctrl+Shift+Tab, Ctrl+PgUp", "Previous tab"),
        ("Alt+1 … Alt+9", "Go to tab 1–9"),
        ("Drag a tab", "Reorder, new window, other window"),
        ("Right-click a tab", "More tab commands"),
    ]),
    ("Editing", &[
        ("Ctrl+Z / Ctrl+Y, Ctrl+Shift+Z", "Undo / redo"),
        ("Ctrl+X / C / V", "Cut / copy / paste"),
        ("Ctrl+A", "Select all"),
        ("Shift+arrows, Shift+click", "Extend the selection"),
        ("Double-click", "Select word"),
        ("Tab / Shift+Tab", "Indent / unindent"),
        ("Ctrl+Shift+D", "Duplicate line"),
        ("Ctrl+Shift+K", "Delete line"),
        ("Alt+Up / Alt+Down", "Move line"),
        ("Ctrl+Backspace / Del", "Delete word"),
        ("Ctrl+B / Ctrl+I", "Bold / italic (Markdown)"),
    ]),
    ("Multiple cursors", &[
        ("Alt+click", "Add or remove a cursor"),
        ("Alt+drag", "Column selection"),
        ("Alt+Shift+Up / Down", "Add cursor above / below"),
        ("Ctrl+D", "Select word, then next match"),
        ("Esc", "Back to one cursor"),
    ]),
    ("Navigation and search", &[
        ("Ctrl+G", "Go to line"),
        ("Ctrl+F / F3", "Find / find next"),
        ("Ctrl+H", "Replace"),
        ("Ctrl+Left / Right", "Previous / next word"),
        ("Home / End", "Line start / end"),
        ("Ctrl+Home / End", "Start / end of file"),
        ("PgUp / PgDn", "Page up / down"),
        ("Ctrl+Up / Down", "Scroll"),
    ]),
    ("View", &[
        ("Alt", "Menu"),
        ("Alt+Z", "Word wrap"),
        ("Ctrl+K Ctrl+T", "Color scheme"),
        ("Ctrl+,", "Settings"),
        ("Ctrl+wheel", "Font size"),
        ("Ctrl+0, Ctrl+Num0", "Reset font size"),
        ("Ctrl+/", "This overview"),
    ]),
];

thread_local! {
    static HELP: Cell<HWND> = const { Cell::new(null_mut()) };
    static FONTS: Cell<(HFONT, HFONT)> = const { Cell::new((null_mut(), null_mut())) };
}

unsafe fn font(px: i32, weight: i32) -> HFONT {
    CreateFontW(-px, 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET as _, OUT_DEFAULT_PRECIS as _, CLIP_DEFAULT_PRECIS as _,
        CLEARTYPE_QUALITY as _, 0, w!("Segoe UI"))
}

unsafe fn text_width(dc: HDC, s: &str) -> i32 {
    let t: Vec<u16> = s.encode_utf16().collect();
    let mut sz: SIZE = zeroed();
    GetTextExtentPoint32W(dc, t.as_ptr(), t.len() as i32, &mut sz);
    sz.cx
}

/// Lays out (and with `draw`, paints) the overview; returns its size.
unsafe fn render(dc: HDC, dpi: u32, draw: bool) -> (i32, i32) {
    let p = |v: i32| v * dpi as i32 / 96;
    let (normal, bold) = FONTS.with(|f| f.get());
    let c = theme::palette();
    let (pad, line, gap) = (p(20), p(22), p(18));
    let mut x = pad;
    let mut height = 0;
    SetBkMode(dc, TRANSPARENT as _);
    // Three columns of two groups each.
    for column in GROUPS.chunks(2) {
        SelectObject(dc, normal);
        let keys_w = column.iter().flat_map(|g| g.1.iter()).map(|r| text_width(dc, r.0)).max().unwrap_or(0);
        let action_w = column.iter().flat_map(|g| g.1.iter()).map(|r| text_width(dc, r.1)).max().unwrap_or(0);
        let mut y = pad;
        for (title, rows) in column {
            if draw {
                SelectObject(dc, bold);
                SetTextColor(dc, c.fg);
                let t: Vec<u16> = title.encode_utf16().collect();
                TextOutW(dc, x, y, t.as_ptr(), t.len() as i32);
            }
            y += line + p(4);
            for (keys, action) in rows.iter() {
                if draw {
                    SelectObject(dc, normal);
                    for (s, cx, color) in [(*keys, x, c.key), (*action, x + keys_w + p(16), c.status_fg)] {
                        let t: Vec<u16> = s.encode_utf16().collect();
                        SetTextColor(dc, color);
                        TextOutW(dc, cx, y, t.as_ptr(), t.len() as i32);
                    }
                }
                y += line;
            }
            y += gap;
        }
        height = height.max(y);
        x += keys_w + p(16) + action_w + p(40);
    }
    (x - p(40) + pad, height - gap + pad)
}

/// Shows the overview centered on `owner`, or closes it if it is open.
pub unsafe fn toggle(owner: HWND, dpi: u32) {
    let open = HELP.with(|h| h.get());
    if !open.is_null() {
        DestroyWindow(open);
        return;
    }
    let p = |v: i32| v * dpi as i32 / 96;
    let (old_normal, old_bold) = FONTS.with(|f| f.get());
    for f in [old_normal, old_bold].into_iter().filter(|f| !f.is_null()) {
        DeleteObject(f);
    }
    FONTS.with(|f| f.set((font(p(15), FW_NORMAL as i32), font(p(15), FW_SEMIBOLD as i32))));
    let dc = CreateCompatibleDC(null_mut());
    let (w, h) = render(dc, dpi, false);
    DeleteDC(dc);
    let mut rc: RECT = zeroed();
    GetWindowRect(owner, &mut rc);
    let (x, y) = ((rc.left + rc.right - w) / 2, (rc.top + rc.bottom - h) / 2);
    let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: GetModuleHandleW(null()), lpszClassName: w!("minedit_help"),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW), ..zeroed() };
    RegisterClassW(&wc); // fails harmlessly when already registered
    let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW, w!("minedit_help"), w!("Keyboard shortcuts"), WS_POPUP | WS_BORDER,
        x, y, w, h, owner, null_mut(), GetModuleHandleW(null()), null());
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, dpi as isize);
    HELP.with(|c| c.set(hwnd));
    ShowWindow(hwnd, SW_SHOW);
}

/// True for key presses that type '/' (on any keyboard layout, ignoring Ctrl).
pub unsafe fn is_slash(msg: &MSG) -> bool {
    if msg.message != WM_KEYDOWN {
        return false;
    }
    if msg.wParam == VK_DIVIDE as usize {
        return true;
    }
    let mut keys = [0u8; 256];
    GetKeyboardState(keys.as_mut_ptr());
    for k in [VK_CONTROL, VK_LCONTROL, VK_RCONTROL] {
        keys[k as usize] = 0;
    }
    let mut buf = [0u16; 4];
    let scan = ((msg.lParam >> 16) & 0xFF) as u32;
    // Flag 4: do not change the keyboard state (dead keys).
    ToUnicode(msg.wParam as u32, scan, keys.as_ptr(), buf.as_mut_ptr(), 4, 4) == 1 && buf[0] == b'/' as u16
}

unsafe extern "system" fn proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(h, &mut ps);
            let mut rc: RECT = zeroed();
            GetClientRect(h, &mut rc);
            let b = CreateSolidBrush(theme::palette().bg);
            FillRect(dc, &rc, b);
            DeleteObject(b);
            render(dc, GetWindowLongPtrW(h, GWLP_USERDATA) as u32, true);
            EndPaint(h, &ps);
            0
        }
        WM_KEYDOWN | WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
            DestroyWindow(h);
            0
        }
        WM_ACTIVATE if (wp & 0xFFFF) as u32 == WA_INACTIVE => {
            PostMessageW(h, WM_CLOSE, 0, 0);
            0
        }
        WM_DESTROY => {
            HELP.with(|c| c.set(null_mut()));
            0
        }
        _ => DefWindowProcW(h, m, wp, lp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders the overview in light and dark (MINEDIT_RENDER_HELP=<path prefix>) for a visual check.
    #[test]
    fn render_preview() {
        let Ok(prefix) = std::env::var("MINEDIT_RENDER_HELP") else { return };
        unsafe {
            FONTS.with(|f| f.set((font(15, FW_NORMAL as i32), font(15, FW_SEMIBOLD as i32))));
            for dark in [false, true] {
                theme::set_dark(dark);
                let dc = CreateCompatibleDC(null_mut());
                let (w, h) = render(dc, 96, false);
                let mut bits = null_mut();
                let mut bi: BITMAPINFO = zeroed();
                bi.bmiHeader = BITMAPINFOHEADER { biSize: 40, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, ..zeroed() };
                let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
                SelectObject(dc, bmp);
                let b = CreateSolidBrush(theme::palette().bg);
                FillRect(dc, &RECT { left: 0, top: 0, right: w, bottom: h }, b);
                render(dc, 96, true);
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
                std::fs::write(format!("{prefix}_{}.bmp", if dark { "dark" } else { "light" }), f).unwrap();
            }
        }
    }
}
