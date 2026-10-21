//! Light and dark color palettes. Colors are 0x00BBGGRR.

use std::cell::Cell;

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: u32,
    pub fg: u32,
    pub gutter_bg: u32,
    pub gutter_fg: u32,
    pub line: u32, // current line
    pub sel: u32,
    pub sel_inactive: u32,
    pub bracket: u32, // behind a bracket and its partner
    pub strip: u32,   // title bar
    pub hover: u32,
    pub tab_hover: u32,
    pub separator: u32,
    pub inactive_text: u32,
    pub close_red: u32,
    pub status_bg: u32,
    pub status_fg: u32,
    pub status_border: u32,
    // Syntax highlighting.
    pub key: u32,
    pub string: u32,
    pub number: u32,
    pub keyword: u32,
    pub comment: u32,
    pub punct: u32,
    pub heading: u32,
    pub code: u32,
    pub link: u32,
    pub tag: u32,
    pub attr: u32,
    pub columns: [u32; 8],
}

pub const LIGHT: Palette = Palette {
    bg: 0xFFFFFF,
    fg: 0x000000,
    gutter_bg: 0xF8F8F8,
    gutter_fg: 0x999999,
    line: 0xF2F2F2,
    sel: 0xFFD6AD,
    sel_inactive: 0xE5E5E5,
    bracket: 0xD0D0D0,
    strip: 0xF3F3F3,
    hover: 0xE5E5E5,
    tab_hover: 0xEAEAEA,
    separator: 0xD5D5D5,
    inactive_text: 0x8A8A8A,
    close_red: 0x1C2BC4,
    status_bg: 0xF3F3F3,
    status_fg: 0x5A5A5A,
    status_border: 0xE5E5E5,
    key: 0xA55104,
    string: 0x1515A3,
    number: 0x588609,
    keyword: 0xFF0000,
    comment: 0x008000,
    punct: 0x808080,
    heading: 0x000080,
    code: 0x265E79,
    link: 0xA55104,
    tag: 0x000080,
    attr: 0x0000E5,
    columns: [0xA55104, 0x1515A3, 0x588609, 0x265E79, 0xDB00AF, 0x801000, 0x3F1F81, 0x997F26],
};

/// Modeled on VS Code's Dark+ theme.
pub const DARK: Palette = Palette {
    bg: 0x1E1E1E,
    fg: 0xD4D4D4,
    gutter_bg: 0x1E1E1E,
    gutter_fg: 0x858585,
    line: 0x2E2D2A,
    sel: 0x784F26,
    sel_inactive: 0x413D3A,
    bracket: 0x515151,
    strip: 0x262525,
    hover: 0x3A3A3A,
    tab_hover: 0x2F2F2F,
    separator: 0x3C3C3C,
    inactive_text: 0x8A8A8A,
    close_red: 0x1C2BC4,
    status_bg: 0x262525,
    status_fg: 0xCCCCCC,
    status_border: 0x333333,
    key: 0xFEDC9C,
    string: 0x7891CE,
    number: 0xA8CEB5,
    keyword: 0xD69C56,
    comment: 0x55996A,
    punct: 0x808080,
    heading: 0xD69C56,
    code: 0x7DBAD7,
    link: 0xFFC14F,
    tag: 0xD69C56,
    attr: 0xFEDC9C,
    columns: [0xFEDC9C, 0x7891CE, 0xA8CEB5, 0xAADCDC, 0xC086C5, 0xFFC14F, 0x7DBAD7, 0xB0C94E],
};

thread_local! {
    static DARK_MODE: Cell<bool> = const { Cell::new(false) };
}

pub fn set_dark(dark: bool) {
    DARK_MODE.with(|d| d.set(dark));
}

pub fn is_dark() -> bool {
    DARK_MODE.with(|d| d.get())
}

pub fn palette() -> &'static Palette {
    if is_dark() { &DARK } else { &LIGHT }
}
