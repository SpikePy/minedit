//! The file tree on the left: a folder's subfolders and files, each folder listed when it is first expanded.

use crate::theme;
use std::{collections::HashSet, mem::zeroed, path::{Path, PathBuf}, ptr::{null, null_mut}};
use windows_sys::{
    w,
    Win32::{
        Foundation::*,
        Graphics::Gdi::{InvalidateRect, ScreenToClient, HFONT},
        UI::Input::KeyboardAndMouse::SetFocus,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Controls::*, Shell::*, WindowsAndMessaging::*},
    },
};

const DIR: LPARAM = 1; // item data of folders; files have 0
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;

pub unsafe fn create(parent: HWND) -> HWND {
    let style = WS_CHILD | WS_TABSTOP | TVS_HASBUTTONS | TVS_SHOWSELALWAYS | TVS_FULLROWSELECT | TVS_TRACKSELECT | TVS_NOHSCROLL | TVS_EDITLABELS;
    let h = CreateWindowExW(0, WC_TREEVIEWW, null(), style, 0, 0, 0, 0, parent, null_mut(), GetModuleHandleW(null()), null());
    SendMessageW(h, TVM_SETEXTENDEDSTYLE, TVS_EX_DOUBLEBUFFER as WPARAM, TVS_EX_DOUBLEBUFFER as LPARAM);
    // The system's small icons, looked up by name only (no disk access).
    let mut sfi: SHFILEINFOW = zeroed();
    let icons = SHGetFileInfoW(w!("x"), FILE_ATTRIBUTE_NORMAL, &mut sfi, size_of::<SHFILEINFOW>() as u32,
        SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES);
    SendMessageW(h, TVM_SETIMAGELIST, TVSIL_NORMAL as WPARAM, icons as LPARAM);
    recolor(h);
    h
}

pub unsafe fn set_font(h: HWND, font: HFONT, dpi: u32) {
    SendMessageW(h, WM_SETFONT, font as WPARAM, 1);
    SendMessageW(h, TVM_SETITEMHEIGHT, (24 * dpi / 96) as WPARAM, 0);
}

pub unsafe fn recolor(h: HWND) {
    let c = theme::palette();
    SendMessageW(h, TVM_SETBKCOLOR, 0, c.strip as LPARAM);
    SendMessageW(h, TVM_SETTEXTCOLOR, 0, c.fg as LPARAM);
    SetWindowTheme(h, if theme::is_dark() { w!("DarkMode_Explorer") } else { w!("Explorer") }, null());
}

/// Subfolders first, then files, each sorted by name; hidden and system entries are left out.
fn list(dir: &Path) -> Vec<(String, bool)> {
    use std::os::windows::fs::MetadataExt;
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<(String, bool)> = entries.flatten().filter_map(|e| {
        let m = e.metadata().ok()?;
        (m.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) == 0)
            .then(|| (e.file_name().to_string_lossy().into_owned(), m.is_dir()))
    }).collect();
    v.sort_by_cached_key(|(name, dir)| (!dir, name.to_lowercase()));
    v
}

/// Names to skip from the folder's .gitignore: (anywhere, only at the top). Only plain names are
/// understood ("target", "/target", "target/"); patterns with wildcards or inner slashes are ignored.
fn gitignore(root: &Path) -> (HashSet<String>, HashSet<String>) {
    let (mut anywhere, mut top) = (HashSet::from([".git".to_string()]), HashSet::new());
    for l in std::fs::read_to_string(root.join(".gitignore")).unwrap_or_default().lines() {
        let l = l.trim();
        if l.is_empty() || l.starts_with(['#', '!']) || l.contains(['*', '?', '[', '\\']) {
            continue;
        }
        let name = l.trim_end_matches('/');
        match name.strip_prefix('/') {
            Some(n) if !n.contains('/') => _ = top.insert(n.to_lowercase()),
            None if !name.contains('/') => _ = anywhere.insert(name.to_lowercase()),
            _ => {}
        }
    }
    (anywhere, top)
}

/// All files below `root` (as the tree shows them, minus what .gitignore names), shallow ones first,
/// handed over in chunks. Stops when `go_on` says so, or after a large number of files.
pub fn walk(root: &Path, go_on: impl Fn() -> bool, mut chunk: impl FnMut(Vec<PathBuf>)) {
    const MAX: usize = 200_000;
    let (anywhere, top) = gitignore(root);
    let (mut dirs, mut files, mut total) = (std::collections::VecDeque::from([root.to_path_buf()]), Vec::new(), 0);
    while let Some(dir) = dirs.pop_front() {
        if !go_on() || total >= MAX {
            return;
        }
        for (name, is_dir) in list(&dir) {
            let lower = name.to_lowercase();
            if anywhere.contains(&lower) || (dir == root && top.contains(&lower)) {
                continue;
            }
            if is_dir {
                dirs.push_back(dir.join(name));
            } else {
                files.push(dir.join(name));
                total += 1;
            }
        }
        if files.len() >= 2000 {
            chunk(std::mem::take(&mut files));
        }
    }
    if !files.is_empty() {
        chunk(files);
    }
}

unsafe fn icon(name: &str, dir: bool, open: bool) -> i32 {
    let mut sfi: SHFILEINFOW = zeroed();
    let attr = if dir { FILE_ATTRIBUTE_DIRECTORY } else { FILE_ATTRIBUTE_NORMAL };
    let flags = SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES | if open { SHGFI_OPENICON } else { 0 };
    SHGetFileInfoW(crate::wide(name).as_ptr(), attr, &mut sfi, size_of::<SHFILEINFOW>() as u32, flags);
    sfi.iIcon
}

unsafe fn insert(h: HWND, parent: HTREEITEM, name: &str, dir: bool) -> HTREEITEM {
    let mut text = crate::wide(name);
    let mut ins: TVINSERTSTRUCTW = zeroed();
    ins.hParent = parent;
    ins.hInsertAfter = TVI_LAST;
    ins.Anonymous.item = TVITEMW {
        mask: TVIF_TEXT | TVIF_IMAGE | TVIF_SELECTEDIMAGE | TVIF_CHILDREN | TVIF_PARAM,
        pszText: text.as_mut_ptr(),
        iImage: icon(name, dir, false),
        iSelectedImage: icon(name, dir, false),
        cChildren: dir as _, // folders get a button; their content is listed on expanding
        lParam: if dir { DIR } else { 0 },
        ..zeroed()
    };
    SendMessageW(h, TVM_INSERTITEMW, 0, &ins as *const _ as LPARAM)
}

fn next(h: HWND, item: HTREEITEM, how: u32) -> HTREEITEM {
    unsafe { SendMessageW(h, TVM_GETNEXTITEM, how as WPARAM, item) }
}

unsafe fn item(h: HWND, item: HTREEITEM) -> (String, bool, bool) {
    let mut buf = [0u16; 1024];
    let mut it = TVITEMW {
        mask: TVIF_TEXT | TVIF_PARAM | TVIF_STATE, hItem: item, stateMask: TVIS_EXPANDED,
        pszText: buf.as_mut_ptr(), cchTextMax: buf.len() as i32, ..zeroed()
    };
    SendMessageW(h, TVM_GETITEMW, 0, &mut it as *mut _ as LPARAM);
    let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
    (String::from_utf16_lossy(&buf[..n]), it.lParam == DIR, it.state & TVIS_EXPANDED != 0)
}

/// The file or folder an item stands for: its names up to the top item, which is `root`.
unsafe fn path_of(h: HWND, mut it: HTREEITEM, root: &Path) -> PathBuf {
    let mut names = Vec::new();
    loop {
        let parent = next(h, it, TVGN_PARENT);
        if parent == 0 {
            break;
        }
        names.push(item(h, it).0);
        it = parent;
    }
    names.iter().rev().fold(root.to_path_buf(), |p, n| p.join(n))
}

/// Lists folder `it` (once).
unsafe fn populate(h: HWND, it: HTREEITEM, root: &Path) {
    if next(h, it, TVGN_CHILD) != 0 {
        return;
    }
    let entries = list(&path_of(h, it, root));
    for (name, dir) in &entries {
        insert(h, it, name, *dir);
    }
    if entries.is_empty() {
        let none = TVITEMW { mask: TVIF_CHILDREN, hItem: it, cChildren: 0, ..zeroed() };
        SendMessageW(h, TVM_SETITEMW, 0, &none as *const _ as LPARAM);
    }
}

/// Shows folder `root`, expanded one level.
pub unsafe fn fill(h: HWND, root: &Path) {
    SendMessageW(h, TVM_DELETEITEM, 0, TVI_ROOT);
    let name = root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned());
    let top = insert(h, TVI_ROOT, &name, true);
    populate(h, top, root);
    SendMessageW(h, TVM_EXPAND, TVE_EXPAND as WPARAM, top);
}

/// Lists the folder again, keeping expanded folders, the selection and the scroll position.
pub unsafe fn refresh(h: HWND, root: &Path) {
    let top = next(h, 0, TVGN_ROOT);
    if top == 0 {
        return fill(h, root);
    }
    let (sel, first) = (next(h, 0, TVGN_CARET), next(h, 0, TVGN_FIRSTVISIBLE));
    let path = |it: HTREEITEM| (it != 0).then(|| path_of(h, it, root));
    let (sel, first) = (path(sel), path(first));
    let mut open = HashSet::new();
    let mut stack = vec![top];
    while let Some(it) = stack.pop() {
        if item(h, it).2 {
            open.insert(path_of(h, it, root));
            let mut c = next(h, it, TVGN_CHILD);
            while c != 0 {
                stack.push(c);
                c = next(h, c, TVGN_NEXT);
            }
        }
    }
    SendMessageW(h, WM_SETREDRAW, 0, 0);
    fill(h, root);
    // Expand again from the top down, finding the selected and first visible items on the way.
    let mut stack = vec![(next(h, 0, TVGN_ROOT), root.to_path_buf())];
    let (mut new_sel, mut new_first) = (0, 0);
    while let Some((it, p)) = stack.pop() {
        if sel.as_ref() == Some(&p) {
            new_sel = it;
        }
        if first.as_ref() == Some(&p) {
            new_first = it;
        }
        if !open.contains(&p) {
            continue;
        }
        populate(h, it, root);
        SendMessageW(h, TVM_EXPAND, TVE_EXPAND as WPARAM, it);
        let mut c = next(h, it, TVGN_CHILD);
        while c != 0 {
            stack.push((c, p.join(item(h, c).0)));
            c = next(h, c, TVGN_NEXT);
        }
    }
    if new_sel != 0 {
        SendMessageW(h, TVM_SELECTITEM, TVGN_CARET as WPARAM, new_sel);
    }
    if new_first != 0 {
        SendMessageW(h, TVM_SELECTITEM, TVGN_FIRSTVISIBLE as WPARAM, new_first);
    }
    SendMessageW(h, WM_SETREDRAW, 1, 0);
    InvalidateRect(h, null(), 1);
}

/// The names leading from `root` to `path`, if `path` is inside it (letter case ignored, as Windows does).
fn relative(root: &Path, path: &Path) -> Option<Vec<String>> {
    let lower = |p: &Path| p.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect::<Vec<_>>();
    let (r, p) = (lower(root), lower(path));
    (p.len() > r.len() && p.starts_with(&r)).then(|| {
        path.components().skip(r.len()).map(|c| c.as_os_str().to_string_lossy().into_owned()).collect()
    })
}

/// Selects `path` (the active tab's file) in the tree, opening the folders on the way; a file outside
/// the folder clears the selection. Shown light gray while the tree has no focus.
pub unsafe fn reveal(h: HWND, root: &Path, path: Option<&Path>) {
    let mut it = next(h, 0, TVGN_ROOT);
    for name in path.and_then(|p| relative(root, p)).unwrap_or_default() {
        if it == 0 {
            break;
        }
        populate(h, it, root);
        SendMessageW(h, TVM_EXPAND, TVE_EXPAND as WPARAM, it);
        let mut c = next(h, it, TVGN_CHILD);
        while c != 0 && !item(h, c).0.eq_ignore_ascii_case(&name) {
            c = next(h, c, TVGN_NEXT);
        }
        it = c;
    }
    let found = it != 0 && it != next(h, 0, TVGN_ROOT);
    SendMessageW(h, TVM_SELECTITEM, TVGN_CARET as WPARAM, if found { it } else { 0 });
}

/// Click or Enter on an item: folders open or close, files are returned to be opened.
unsafe fn activate(h: HWND, it: HTREEITEM, root: &Path) -> Option<PathBuf> {
    let (_, dir, expanded) = item(h, it);
    if dir {
        if !expanded {
            populate(h, it, root);
        }
        SendMessageW(h, TVM_EXPAND, TVE_TOGGLE as WPARAM, it);
        None
    } else {
        Some(path_of(h, it, root))
    }
}

/// Enter: the selected item.
pub unsafe fn enter(h: HWND, root: &Path) -> Option<PathBuf> {
    let it = next(h, 0, TVGN_CARET);
    if it == 0 { None } else { activate(h, it, root) }
}

/// What the user did in the tree, for the window to carry out.
pub enum Action {
    None,
    Open(PathBuf),
    Rename(PathBuf, String),   // the item and its new name (typed in place)
    Menu(PathBuf, bool, POINT), // right-click on an item: path, is the top folder, screen point
    Drag(PathBuf),             // an item is being dragged
}

/// The item under screen point `pt`, if any.
unsafe fn item_at(h: HWND, pt: POINT) -> HTREEITEM {
    let mut ht = TVHITTESTINFO { pt, ..zeroed() };
    ScreenToClient(h, &mut ht.pt);
    let it = SendMessageW(h, TVM_HITTEST, 0, &mut ht as *mut _ as LPARAM);
    if ht.flags & (TVHT_ONITEM | TVHT_ONITEMRIGHT) == 0 { 0 } else { it }
}

fn message_pos() -> POINT {
    let pos = unsafe { GetMessagePos() };
    POINT { x: (pos & 0xFFFF) as i16 as i32, y: (pos >> 16) as i16 as i32 }
}

/// The selected item's path and whether it is the top folder.
pub unsafe fn selected(h: HWND, root: &Path) -> Option<(PathBuf, bool)> {
    let it = next(h, 0, TVGN_CARET);
    (it != 0).then(|| (path_of(h, it, root), next(h, it, TVGN_PARENT) == 0))
}

/// F2: edits the selected item's name in place (not the top folder's).
pub unsafe fn edit_name(h: HWND) {
    let it = next(h, 0, TVGN_CARET);
    if it != 0 && next(h, it, TVGN_PARENT) != 0 {
        SetFocus(h);
        SendMessageW(h, TVM_EDITLABELW, 0, it);
    }
}

/// While dragging: the folder under screen point `pt` (a file stands for its folder), highlighted
/// as the drop target; None clears the highlight.
pub unsafe fn drop_target(h: HWND, root: &Path, pt: Option<POINT>) -> Option<PathBuf> {
    let mut it = pt.map_or(0, |pt| item_at(h, pt));
    if it != 0 && !item(h, it).1 {
        it = next(h, it, TVGN_PARENT);
    }
    SendMessageW(h, TVM_SELECTITEM, TVGN_DROPHILITE as WPARAM, it);
    (it != 0).then(|| path_of(h, it, root))
}

/// Handles the tree's WM_NOTIFY; returns the result and what to do.
pub unsafe fn notify(h: HWND, root: &Path, lp: LPARAM) -> (LRESULT, Action) {
    let hdr = &*(lp as *const NMHDR);
    match hdr.code {
        // The top folder keeps its name.
        TVN_BEGINLABELEDITW => {
            let it = (*(lp as *const NMTVDISPINFOW)).item.hItem;
            ((next(h, it, TVGN_PARENT) == 0) as LRESULT, Action::None)
        }
        TVN_ENDLABELEDITW => {
            let item = &(*(lp as *const NMTVDISPINFOW)).item;
            if item.pszText.is_null() {
                return (0, Action::None); // cancelled
            }
            let n = (0..).take_while(|&i| *item.pszText.add(i) != 0).count();
            let name = String::from_utf16_lossy(std::slice::from_raw_parts(item.pszText, n)).trim().to_string();
            // Not taken as the label: the tree is listed again once the file is renamed.
            (0, if name.is_empty() { Action::None } else { Action::Rename(path_of(h, item.hItem, root), name) })
        }
        TVN_BEGINDRAGW => {
            let it = (*(lp as *const NMTREEVIEWW)).itemNew.hItem;
            (0, if next(h, it, TVGN_PARENT) == 0 { Action::None } else { Action::Drag(path_of(h, it, root)) })
        }
        NM_RCLICK => {
            let pt = message_pos();
            let it = item_at(h, pt);
            if it == 0 {
                return (1, Action::None);
            }
            SendMessageW(h, TVM_SELECTITEM, TVGN_CARET as WPARAM, it);
            (1, Action::Menu(path_of(h, it, root), next(h, it, TVGN_PARENT) == 0, pt))
        }
        TVN_ITEMEXPANDINGW => {
            let nm = &*(lp as *const NMTREEVIEWW);
            if nm.action == TVE_EXPAND {
                populate(h, nm.itemNew.hItem, root);
            }
            // The open folder icon while expanded.
            let (name, dir, _) = item(h, nm.itemNew.hItem);
            let img = icon(&name, dir, nm.action == TVE_EXPAND);
            let it = TVITEMW { mask: TVIF_IMAGE | TVIF_SELECTEDIMAGE, hItem: nm.itemNew.hItem, iImage: img, iSelectedImage: img, ..zeroed() };
            SendMessageW(h, TVM_SETITEMW, 0, &it as *const _ as LPARAM);
            (0, Action::None)
        }
        NM_CLICK => {
            // The expand button toggles by itself.
            let it = item_at(h, message_pos());
            if it == 0 {
                return (0, Action::None);
            }
            SendMessageW(h, TVM_SELECTITEM, TVGN_CARET as WPARAM, it);
            (0, activate(h, it, root).map_or(Action::None, Action::Open))
        }
        NM_DBLCLK => (1, Action::None), // a click already opened or toggled it
        _ => (0, Action::None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_tree(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("minedit-tree-{name}-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&d);
        for f in ["b.txt", "A.md", "sub/inner.rs", "Zdir/x", "sub/deeper/y"] {
            let p = d.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        d
    }

    #[test]
    fn folders_first_then_files_by_name() {
        let d = temp_tree("list");
        let names: Vec<_> = list(&d).into_iter().map(|e| e.0).collect();
        assert_eq!(names, ["sub", "Zdir", "A.md", "b.txt"]);
        let mut all = Vec::new();
        walk(&d, || true, |c| all.extend(c));
        let rel: Vec<_> = all.iter().map(|p| p.strip_prefix(&d).unwrap().to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(rel, ["A.md", "b.txt", "sub/inner.rs", "Zdir/x", "sub/deeper/y"]);
        // .gitignore: "/zdir" only at the top, "inner.rs" anywhere; wildcards are not understood.
        std::fs::write(d.join(".gitignore"), "# build\n/zdir\ninner.rs\n*.md\n").unwrap();
        let mut all = Vec::new();
        walk(&d, || true, |c| all.extend(c));
        let rel: Vec<_> = all.iter().map(|p| p.strip_prefix(&d).unwrap().to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(rel, [".gitignore", "A.md", "b.txt", "sub/deeper/y"]);
        _ = std::fs::remove_dir_all(d);
    }

    /// Fill, expand, refresh: paths resolve and expanded folders stay expanded.
    #[test]
    fn tree_lists_lazily_and_refresh_keeps_state() {
        let d = temp_tree("view");
        unsafe {
            let owner = CreateWindowExW(0, w!("STATIC"), null(), 0, 0, 0, 400, 400, null_mut(), null_mut(), null_mut(), null());
            let h = create(owner);
            fill(h, &d);
            let top = next(h, 0, TVGN_ROOT);
            let children = |it| {
                let mut v = Vec::new();
                let mut c = next(h, it, TVGN_CHILD);
                while c != 0 {
                    v.push(c);
                    c = next(h, c, TVGN_NEXT);
                }
                v
            };
            let kids = children(top);
            assert_eq!(kids.len(), 4);
            assert_eq!(path_of(h, kids[3], &d), d.join("b.txt"));
            assert!(children(kids[0]).is_empty()); // "sub" is listed only when expanded
            assert_eq!(activate(h, kids[3], &d), Some(d.join("b.txt")));
            populate(h, kids[0], &d);
            SendMessageW(h, TVM_EXPAND, TVE_EXPAND as WPARAM, kids[0]);
            let sub = children(kids[0]);
            assert_eq!(path_of(h, sub[1], &d), d.join("sub").join("inner.rs"));
            std::fs::write(d.join("sub").join("new.txt"), "").unwrap();
            refresh(h, &d);
            let kids = children(next(h, 0, TVGN_ROOT));
            assert!(item(h, kids[0]).2, "sub stays expanded");
            let names: Vec<_> = children(kids[0]).into_iter().map(|c| item(h, c).0).collect();
            assert_eq!(names, ["deeper", "inner.rs", "new.txt"]);
            // MINEDIT_RENDER_TREE=<path prefix>: the tree in light and dark, for a visual check.
            if let Ok(prefix) = std::env::var("MINEDIT_RENDER_TREE") {
                use windows_sys::Win32::Graphics::Gdi::*;
                let font = CreateFontW(-15, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, CLEARTYPE_QUALITY as _, 0, w!("Segoe UI"));
                set_font(h, font, 96);
                SetWindowPos(h, null_mut(), 0, 0, 260, 220, 0);
                SendMessageW(h, TVM_SELECTITEM, TVGN_CARET as WPARAM, children(kids[0])[1]);
                for dark in [false, true] {
                    theme::apply("Visual Studio", dark);
                    recolor(h);
                    let (w, ht) = (260, 220);
                    let dc = CreateCompatibleDC(null_mut());
                    let mut bits = null_mut();
                    let mut bi: BITMAPINFO = zeroed();
                    bi.bmiHeader = BITMAPINFOHEADER { biSize: 40, biWidth: w, biHeight: -ht, biPlanes: 1, biBitCount: 32, ..zeroed() };
                    let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
                    SelectObject(dc, bmp);
                    SendMessageW(h, WM_PRINT, dc as WPARAM, (PRF_CLIENT | PRF_ERASEBKGND) as LPARAM);
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
                }
            }
            // The active tab's file is selected, with its folders opened; others clear the selection.
            refresh(h, &d);
            reveal(h, &d, Some(&d.join("SUB").join("deeper").join("y")));
            let sel = next(h, 0, TVGN_CARET);
            assert_eq!(path_of(h, sel, &d), d.join("sub").join("deeper").join("y"));
            reveal(h, &d, Some(Path::new(r"C:\elsewhere\x.txt")));
            assert_eq!(next(h, 0, TVGN_CARET), 0);
            assert_eq!(relative(Path::new(r"C:\a"), Path::new(r"c:\A\b\c.txt")), Some(vec!["b".into(), "c.txt".into()]));
            assert_eq!(relative(Path::new(r"C:\a"), Path::new(r"C:\ab\c.txt")), None);
            DestroyWindow(owner);
        }
        _ = std::fs::remove_dir_all(d);
    }
}
