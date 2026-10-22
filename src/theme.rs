//! Color schemes. Each variant is 12 base colors (as 0xRRGGBB, like in the original themes);
//! all UI colors are derived from them. Palette colors are 0x00BBGGRR, as GDI wants them.

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

/// bg, fg, current line, selection, comment, string, number, keyword, function, key/property, tag, attribute.
pub type Base = [u32; 12];

pub struct Scheme {
    pub name: &'static str,
    pub dark: Option<Base>,
    pub light: Option<Base>,
}

const fn s(name: &'static str, dark: Option<Base>, light: Option<Base>) -> Scheme {
    Scheme { name, dark, light }
}

// Light variants shared by several dark ones.
const GITHUB_LIGHT: Base = [0xFFFFFF, 0x24292F, 0xF6F8FA, 0xCCE0F5, 0x6E7781, 0x0A3069, 0x0550AE, 0xCF222E, 0x8250DF, 0x0550AE, 0x116329, 0x0550AE];
const MATERIAL_LIGHTER: Base = [0xFAFAFA, 0x546E7A, 0xEFEFEF, 0xCCD7DA, 0x90A4AE, 0x91B859, 0xF76D47, 0x7C4DFF, 0x6182B8, 0xE53935, 0xE53935, 0xF6A434];
const CATPPUCCIN_LATTE: Base = [0xEFF1F5, 0x4C4F69, 0xE6E9EF, 0xACB0BE, 0x8C8FA1, 0x40A02B, 0xFE640B, 0x8839EF, 0x1E66F5, 0x7287FD, 0x1E66F5, 0xDF8E1D];
const TOKYO_DAY: Base = [0xE1E2E7, 0x3760BF, 0xD5D6DB, 0xB7C1E3, 0x848CB5, 0x587539, 0xB15C00, 0x9854F1, 0x2E7DE9, 0x007197, 0xF52A65, 0x8C6C3E];
const AYU_LIGHT: Base = [0xFCFCFC, 0x5C6166, 0xF0F0F0, 0xD1E4F4, 0x8A9199, 0x86B300, 0xA37ACC, 0xFA8D3E, 0xF2AE49, 0x399EE6, 0x55B4D4, 0xF2AE49];
const ROSE_PINE_DAWN: Base = [0xFAF4ED, 0x575279, 0xF4EDE8, 0xDFDAD9, 0x9893A5, 0xEA9D34, 0xD7827E, 0x286983, 0xD7827E, 0x56949F, 0x56949F, 0x907AA9];
const KANAGAWA_LOTUS: Base = [0xF2ECBC, 0x545464, 0xE7DBA0, 0xC9CBD1, 0x8A8980, 0x6F894E, 0xB35B79, 0x624C83, 0x4D699B, 0x77713F, 0x4D699B, 0xCC6D00];

/// The schemes offered in the settings, in the order shown. The first is the default.
pub const SCHEMES: [Scheme; 29] = [
    s("Visual Studio",
        Some([0x1E1E1E, 0xD4D4D4, 0x2A2D2E, 0x264F78, 0x6A9955, 0xCE9178, 0xB5CEA8, 0x569CD6, 0xDCDCAA, 0x9CDCFE, 0x569CD6, 0x9CDCFE]),
        Some([0xFFFFFF, 0x000000, 0xF2F2F2, 0xADD6FF, 0x008000, 0xA31515, 0x098658, 0x0000FF, 0x795E26, 0x0451A5, 0x800000, 0xE50000])),
    s("One Dark Pro",
        Some([0x282C34, 0xABB2BF, 0x2C313C, 0x3E4451, 0x7F848E, 0x98C379, 0xD19A66, 0xC678DD, 0x61AFEF, 0xE06C75, 0xE06C75, 0xD19A66]),
        Some([0xFAFAFA, 0x383A42, 0xF0F0F1, 0xE5E5E6, 0x8A8B92, 0x50A14F, 0x986801, 0xA626A4, 0x4078F2, 0xE45649, 0xE45649, 0x986801])),
    s("Dracula",
        Some([0x282A36, 0xF8F8F2, 0x343746, 0x44475A, 0x6272A4, 0xF1FA8C, 0xBD93F9, 0xFF79C6, 0x50FA7B, 0x8BE9FD, 0xFF79C6, 0x50FA7B]), None),
    s("GitHub",
        Some([0x0D1117, 0xC9D1D9, 0x161B22, 0x1F3B5C, 0x8B949E, 0xA5D6FF, 0x79C0FF, 0xFF7B72, 0xD2A8FF, 0x79C0FF, 0x7EE787, 0x79C0FF]),
        Some(GITHUB_LIGHT)),
    s("GitHub Dimmed",
        Some([0x22272E, 0xADBAC7, 0x2D333B, 0x2E4A6B, 0x768390, 0x96D0FF, 0x6CB6FF, 0xF47067, 0xDCBDFB, 0x6CB6FF, 0x8DDB8C, 0x6CB6FF]),
        Some(GITHUB_LIGHT)),
    s("Monokai",
        Some([0x272822, 0xF8F8F2, 0x3E3D32, 0x49483E, 0x88846F, 0xE6DB74, 0xAE81FF, 0xF92672, 0xA6E22E, 0x66D9EF, 0xF92672, 0xA6E22E]), None),
    s("Solarized",
        Some([0x002B36, 0x93A1A1, 0x073642, 0x274642, 0x6C7C7F, 0x2AA198, 0xD33682, 0x859900, 0x268BD2, 0xB58900, 0x268BD2, 0x93A1A1]),
        Some([0xFDF6E3, 0x586E75, 0xEEE8D5, 0xE4DCC4, 0x839496, 0x2AA198, 0xD33682, 0x859900, 0x268BD2, 0xB58900, 0x268BD2, 0x586E75])),
    s("Material Darker",
        Some([0x212121, 0xEEFFFF, 0x2A2A2A, 0x404040, 0x7A7A7A, 0xC3E88D, 0xF78C6C, 0xC792EA, 0x82AAFF, 0xF07178, 0xF07178, 0xFFCB6B]),
        Some(MATERIAL_LIGHTER)),
    s("Material Ocean",
        Some([0x0F111A, 0xA6ACCD, 0x1A1C25, 0x1F2233, 0x5F667E, 0xC3E88D, 0xF78C6C, 0xC792EA, 0x82AAFF, 0xF07178, 0xF07178, 0xFFCB6B]),
        Some(MATERIAL_LIGHTER)),
    s("Material Palenight",
        Some([0x292D3E, 0xA6ACCD, 0x2C3043, 0x3C435E, 0x7A80A6, 0xC3E88D, 0xF78C6C, 0xC792EA, 0x82AAFF, 0xF07178, 0xF07178, 0xFFCB6B]),
        Some(MATERIAL_LIGHTER)),
    s("Catppuccin Mocha",
        Some([0x1E1E2E, 0xCDD6F4, 0x2A2B3C, 0x45475A, 0x7F849C, 0xA6E3A1, 0xFAB387, 0xCBA6F7, 0x89B4FA, 0xB4BEFE, 0x89B4FA, 0xF9E2AF]),
        Some(CATPPUCCIN_LATTE)),
    s("Catppuccin Macchiato",
        Some([0x24273A, 0xCAD3F5, 0x2E3248, 0x494D64, 0x8087A2, 0xA6DA95, 0xF5A97F, 0xC6A0F6, 0x8AADF4, 0xB7BDF8, 0x8AADF4, 0xEED49F]),
        Some(CATPPUCCIN_LATTE)),
    s("Catppuccin Frappé",
        Some([0x303446, 0xC6D0F5, 0x3A3F55, 0x51576D, 0x838BA7, 0xA6D189, 0xEF9F76, 0xCA9EE6, 0x8CAAEE, 0xBABBF1, 0x8CAAEE, 0xE5C890]),
        Some(CATPPUCCIN_LATTE)),
    s("Tokyo Night",
        Some([0x1A1B26, 0xA9B1D6, 0x1E202E, 0x283457, 0x6B7394, 0x9ECE6A, 0xFF9E64, 0xBB9AF7, 0x7AA2F7, 0x73DACA, 0xF7768E, 0xE0AF68]),
        Some(TOKYO_DAY)),
    s("Tokyo Night Storm",
        Some([0x24283B, 0xA9B1D6, 0x292E42, 0x2E3C64, 0x6B7394, 0x9ECE6A, 0xFF9E64, 0xBB9AF7, 0x7AA2F7, 0x73DACA, 0xF7768E, 0xE0AF68]),
        Some(TOKYO_DAY)),
    s("Night Owl",
        Some([0x011627, 0xD6DEEB, 0x0B2942, 0x1D3B53, 0x7A8C8C, 0xECC48D, 0xF78C6C, 0xC792EA, 0x82AAFF, 0x7FDBCA, 0xCAECE6, 0xADDB67]),
        Some([0xFBFBFB, 0x403F53, 0xF0F0F0, 0xE0E0E0, 0x8A90A3, 0x4876D6, 0xAA0982, 0x994CC3, 0x4876D6, 0x0C969B, 0x994CC3, 0x4876D6])),
    s("Gruvbox",
        Some([0x282828, 0xEBDBB2, 0x32302F, 0x504945, 0x928374, 0xB8BB26, 0xD3869B, 0xFB4934, 0xFABD2F, 0x83A598, 0x8EC07C, 0xFE8019]),
        Some([0xFBF1C7, 0x3C3836, 0xF2E5BC, 0xD5C4A1, 0x7C6F64, 0x79740E, 0x8F3F71, 0x9D0006, 0xB57614, 0x076678, 0x427B58, 0xAF3A03])),
    s("Nord",
        Some([0x2E3440, 0xD8DEE9, 0x3B4252, 0x434C5E, 0x7B88A1, 0xA3BE8C, 0xB48EAD, 0x81A1C1, 0x88C0D0, 0x8FBCBB, 0x81A1C1, 0x8FBCBB]), None),
    s("Ayu Dark",
        Some([0x0B0E14, 0xBFBDB6, 0x131721, 0x1B3A5B, 0x6C7380, 0xAAD94C, 0xD2A6FF, 0xFF8F40, 0xFFB454, 0x59C2FF, 0x39BAE6, 0xFFB454]),
        Some(AYU_LIGHT)),
    s("Ayu Mirage",
        Some([0x1F2430, 0xCCCAC2, 0x242936, 0x33415E, 0x707A8C, 0xD5FF80, 0xDFBFFF, 0xFFAD66, 0xFFD173, 0x73D0FF, 0x5CCFE6, 0xFFD173]),
        Some(AYU_LIGHT)),
    s("Darcula",
        Some([0x2B2B2B, 0xA9B7C6, 0x323232, 0x214283, 0x808080, 0x6A8759, 0x6897BB, 0xCC7832, 0xFFC66D, 0x9876AA, 0xE8BF6A, 0xBABABA]), None),
    s("Rosé Pine",
        Some([0x191724, 0xE0DEF4, 0x1F1D2E, 0x403D52, 0x908CAA, 0xF6C177, 0xEBBCBA, 0x31748F, 0xEBBCBA, 0x9CCFD8, 0x9CCFD8, 0xC4A7E7]),
        Some(ROSE_PINE_DAWN)),
    s("Rosé Pine Moon",
        Some([0x232136, 0xE0DEF4, 0x2A273F, 0x44415A, 0x908CAA, 0xF6C177, 0xEA9A97, 0x3E8FB0, 0xEA9A97, 0x9CCFD8, 0x9CCFD8, 0xC4A7E7]),
        Some(ROSE_PINE_DAWN)),
    s("Everforest",
        Some([0x2D353B, 0xD3C6AA, 0x343F44, 0x475258, 0x859289, 0xDBBC7F, 0xD699B6, 0xE67E80, 0xA7C080, 0x7FBBB3, 0xE69875, 0x83C092]),
        Some([0xFDF6E3, 0x5C6A72, 0xF4F0D9, 0xE6E2CC, 0x829181, 0xDFA000, 0xDF69BA, 0xF85552, 0x8DA101, 0x3A94C5, 0xF57D26, 0x35A77C])),
    s("Kanagawa Wave",
        Some([0x1F1F28, 0xDCD7BA, 0x2A2A37, 0x2D4F67, 0x727169, 0x98BB6C, 0xD27E99, 0x957FB8, 0x7E9CD8, 0xE6C384, 0x7E9CD8, 0xFFA066]),
        Some(KANAGAWA_LOTUS)),
    s("Kanagawa Dragon",
        Some([0x181616, 0xC5C9C5, 0x282727, 0x2D4F67, 0x737C73, 0x8A9A7B, 0xA292A3, 0x8992A7, 0x8BA4B0, 0xC4B28A, 0x8BA4B0, 0xB6927B]),
        Some(KANAGAWA_LOTUS)),
    s("Cobalt2",
        Some([0x193549, 0xFFFFFF, 0x1F4662, 0x0050A4, 0x0088FF, 0x3AD900, 0xFF628C, 0xFF9D00, 0xFFC600, 0x9EFFFF, 0x9EFFFF, 0xFFC600]), None),
    s("Shades of Purple",
        Some([0x2D2B55, 0xFFFFFF, 0x1F1F41, 0x5A3D9A, 0xB362FF, 0xA5FF90, 0xFF628C, 0xFF9D00, 0xFAD000, 0x9EFFFF, 0x9EFFFF, 0xFAD000]), None),
    s("Tomorrow",
        Some([0x1D1F21, 0xC5C8C6, 0x282A2E, 0x373B41, 0x969896, 0xB5BD68, 0xDE935F, 0xB294BB, 0x81A2BE, 0xCC6666, 0xCC6666, 0xF0C674]),
        Some([0xFFFFFF, 0x4D4D4C, 0xEFEFEF, 0xD6D6D6, 0x8E908C, 0x718C00, 0xF5871F, 0x8959A8, 0x4271AE, 0xC82829, 0xC82829, 0xEAB700])),
];

fn rgb(c: u32) -> [f32; 3] {
    [((c >> 16) & 0xFF) as f32, ((c >> 8) & 0xFF) as f32, (c & 0xFF) as f32]
}

fn pack([r, g, b]: [f32; 3]) -> u32 {
    (r.round() as u32) << 16 | (g.round() as u32) << 8 | b.round() as u32
}

/// `a` moved towards `b` by `t` (0..1), in RGB.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let (a, b) = (rgb(a), rgb(b));
    pack([0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t))
}

/// Relative luminance (WCAG), 0 = black, 1 = white.
pub fn luminance(c: u32) -> f32 {
    let lin = |v: f32| {
        let v = v / 255.0;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let [r, g, b] = rgb(c);
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

/// WCAG contrast ratio of two colors (1..21).
#[cfg(test)]
pub fn contrast(a: u32, b: u32) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// 0xRRGGBB <-> 0x00BBGGRR (the swap is its own inverse).
fn bgr(c: u32) -> u32 {
    (c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16) & 0xFF
}

/// The full palette for 12 base colors.
pub fn derive(base: &Base) -> Palette {
    let [bg, fg, line, sel, comment, string, number, keyword, func, key, tag, attr] = *base;
    let dark = luminance(bg) < 0.25;
    // UI surfaces: slightly lighter than the text background on dark themes, darker on light ones.
    let shift = |t: f32| mix(bg, if dark { 0xFFFFFF } else { 0x000000 }, t);
    Palette {
        bg: bgr(bg),
        fg: bgr(fg),
        gutter_bg: bgr(bg),
        gutter_fg: bgr(mix(fg, bg, 0.55)),
        line: bgr(line),
        sel: bgr(sel),
        sel_inactive: bgr(mix(sel, bg, 0.5)),
        bracket: bgr(mix(fg, bg, 0.72)),
        strip: bgr(shift(0.045)),
        hover: bgr(shift(0.13)),
        tab_hover: bgr(shift(0.08)),
        separator: bgr(mix(fg, bg, 0.8)),
        inactive_text: bgr(mix(fg, bg, 0.45)),
        close_red: bgr(0xC42B1C),
        status_bg: bgr(shift(0.045)),
        status_fg: bgr(mix(fg, bg, 0.2)),
        status_border: bgr(shift(0.1)),
        key: bgr(key),
        string: bgr(string),
        number: bgr(number),
        keyword: bgr(keyword),
        comment: bgr(comment),
        punct: bgr(mix(fg, bg, 0.35)),
        heading: bgr(keyword),
        code: bgr(string),
        link: bgr(func),
        tag: bgr(tag),
        attr: bgr(attr),
        columns: [key, string, number, keyword, func, tag, attr, comment].map(bgr),
    }
}

pub fn find(name: &str) -> &'static Scheme {
    SCHEMES.iter().find(|s| s.name.to_lowercase() == name.trim().to_lowercase()).unwrap_or(&SCHEMES[0])
}

/// The variant for the wanted mode, or the scheme's only one.
pub fn variant(s: &Scheme, dark: bool) -> &Base {
    let (want, other) = if dark { (&s.dark, &s.light) } else { (&s.light, &s.dark) };
    want.as_ref().or(other.as_ref()).expect("a scheme has at least one variant")
}

thread_local! {
    static CURRENT: Cell<Palette> = Cell::new(derive(variant(&SCHEMES[0], false)));
}

/// Uses scheme `name` in its dark or light variant (when it has that one).
pub fn apply(name: &str, dark: bool) {
    CURRENT.with(|c| c.set(derive(variant(find(name), dark))));
}

#[cfg(test)]
pub fn set_dark(dark: bool) {
    apply(SCHEMES[0].name, dark);
}

pub fn palette() -> Palette {
    CURRENT.with(|c| c.get())
}

/// Whether the current palette has a dark background (drives scrollbars, menus and the window border).
pub fn is_dark() -> bool {
    luminance(bgr(palette().bg)) < 0.25
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variants() -> impl Iterator<Item = (&'static str, &'static Base)> {
        SCHEMES.iter().flat_map(|s| [s.dark.as_ref(), s.light.as_ref()].into_iter().flatten().map(move |b| (s.name, b)))
    }

    #[test]
    fn names_are_unique_and_lookup_ignores_case() {
        for (i, a) in SCHEMES.iter().enumerate() {
            assert!(SCHEMES[i + 1..].iter().all(|b| b.name != a.name), "{} twice", a.name);
            assert!(a.dark.is_some() || a.light.is_some());
        }
        assert_eq!(find("dracula").name, "Dracula");
        assert_eq!(find("  catppuccin FRAPPÉ ").name, "Catppuccin Frappé");
        assert_eq!(find("no such scheme").name, "Visual Studio");
    }

    #[test]
    fn text_is_readable_in_every_variant() {
        let mut bad = Vec::new();
        for (name, b) in variants() {
            let [bg, fg, line, sel, comment, ..] = *b;
            let light = if luminance(bg) > 0.25 { "light" } else { "dark" };
            let mut check = |what: &str, ratio: f32, min: f32| {
                if ratio < min {
                    bad.push(format!("{name} ({light}): {what} {ratio:.2} < {min}"));
                }
            };
            check("text", contrast(fg, bg), 4.5);
            check("comment", contrast(comment, bg), 2.4);
            check("text on current line", contrast(fg, line), 4.0);
            check("selected text", contrast(fg, sel), 3.0);
            // Light themes traditionally use faint yellows and oranges (Material, Ayu, Tomorrow):
            // they are kept as designed, so only clear mistakes are caught there.
            let syntax_min = if light == "light" { 1.8 } else { 2.5 };
            for c in &b[5..] {
                check(&format!("syntax {c:06X}"), contrast(*c, bg), syntax_min);
            }
        }
        assert!(bad.is_empty(), "\n{}", bad.join("\n"));
    }

    #[test]
    fn variants_follow_the_mode_when_possible() {
        assert!(luminance(variant(find("Gruvbox"), true)[0]) < 0.25);
        assert!(luminance(variant(find("Gruvbox"), false)[0]) > 0.25);
        assert!(luminance(variant(find("Dracula"), false)[0]) < 0.25); // dark only
        apply("Dracula", false);
        assert!(is_dark());
        apply("Solarized", false);
        assert!(!is_dark());
        let p = palette();
        assert_ne!(p.strip, p.bg);
        assert_ne!(p.hover, p.strip);
    }
}
