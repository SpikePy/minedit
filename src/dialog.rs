//! The Windows file dialogs for opening: files and folders (Ctrl+O) and a folder only (Open Folder).
//! COM by hand, through vtable indices: windows-sys has no COM interface wrappers.

use std::{cell::RefCell, ffi::c_void, path::{Path, PathBuf}, ptr::null_mut};
use windows_sys::{
    core::{GUID, HRESULT, PCWSTR, PWSTR},
    w,
    Win32::{Foundation::*, System::Com::*, UI::Shell::*},
};

type Obj = *mut c_void;

const CLSID_FILE_OPEN_DIALOG: GUID = GUID::from_u128(0xdc1c5a9c_e88a_4dde_a5a1_60f82a20aef7);
const IID_IFILE_OPEN_DIALOG: GUID = GUID::from_u128(0xd57c7288_d4ad_4768_be02_9d969532d960);
const IID_ISHELL_ITEM: GUID = GUID::from_u128(0x43826d1e_e718_42ee_bc55_a1e261c37bfe);
const IID_IFILE_DIALOG_CUSTOMIZE: GUID = GUID::from_u128(0xe6fdd21a_163f_4975_9c8c_a69f1ba37034);
const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
const IID_IFILE_DIALOG_EVENTS: GUID = GUID::from_u128(0x973510db_7d7f_452b_8975_74a85828d354);
const IID_IFILE_DIALOG_CONTROL_EVENTS: GUID = GUID::from_u128(0x36116642_d713_4b97_9b83_7484a9d00433);
const FOS_PICKFOLDERS: u32 = 0x20;
const FOS_FORCEFILESYSTEM: u32 = 0x40;
const FOS_ALLOWMULTISELECT: u32 = 0x200;
const FOS_FILEMUSTEXIST: u32 = 0x1000;
const E_NOINTERFACE: HRESULT = 0x80004002u32 as i32;
const E_NOTIMPL: HRESULT = 0x80004001u32 as i32;
const SELECT_FOLDER: u32 = 1; // id of our button

// Vtable indices.
const SHOW: usize = 3; // IModalWindow
const SET_FILE_TYPES: usize = 4; // IFileDialog …
const ADVISE: usize = 7;
const UNADVISE: usize = 8;
const SET_OPTIONS: usize = 9;
const GET_OPTIONS: usize = 10;
const SET_FOLDER: usize = 12;
const GET_FOLDER: usize = 13;
const GET_CURRENT_SELECTION: usize = 14;
const GET_RESULT: usize = 20;
const CLOSE: usize = 23;
const GET_RESULTS: usize = 27; // IFileOpenDialog
const ADD_PUSH_BUTTON: usize = 5; // IFileDialogCustomize
const GET_DISPLAY_NAME: usize = 5; // IShellItem
const GET_COUNT: usize = 7; // IShellItemArray
const GET_ITEM_AT: usize = 8;

unsafe fn method<F: Copy>(obj: Obj, i: usize) -> F {
    let vtable = *(obj as *const *const usize);
    std::mem::transmute_copy(&*vtable.add(i))
}

unsafe fn release(obj: Obj) {
    method::<unsafe extern "system" fn(Obj) -> u32>(obj, 2)(obj);
}

/// The file system path of an IShellItem (released here).
unsafe fn take_path(item: Obj) -> Option<PathBuf> {
    let mut name: PWSTR = null_mut();
    let ok = method::<unsafe extern "system" fn(Obj, i32, *mut PWSTR) -> HRESULT>(item, GET_DISPLAY_NAME)(item, SIGDN_FILESYSPATH, &mut name) == 0;
    release(item);
    if !ok {
        return None;
    }
    let n = (0..).take_while(|&i| *name.add(i) != 0).count();
    let p = PathBuf::from(String::from_utf16_lossy(std::slice::from_raw_parts(name, n)));
    CoTaskMemFree(name as *const c_void);
    Some(p)
}

/// A file open dialog with `options` added, starting in `initial`.
unsafe fn create(options: u32, initial: Option<&Path>) -> Option<Obj> {
    let mut dlg: Obj = null_mut();
    if CoCreateInstance(&CLSID_FILE_OPEN_DIALOG, null_mut(), CLSCTX_INPROC_SERVER, &IID_IFILE_OPEN_DIALOG, &mut dlg) != 0 {
        return None;
    }
    let mut opts = 0u32;
    method::<unsafe extern "system" fn(Obj, *mut u32) -> HRESULT>(dlg, GET_OPTIONS)(dlg, &mut opts);
    method::<unsafe extern "system" fn(Obj, u32) -> HRESULT>(dlg, SET_OPTIONS)(dlg, opts | options | FOS_FORCEFILESYSTEM);
    if let Some(dir) = initial {
        let mut item: Obj = null_mut();
        if SHCreateItemFromParsingName(crate::wide(&dir.display().to_string()).as_ptr(), null_mut(), &IID_ISHELL_ITEM, &mut item) == 0 {
            method::<unsafe extern "system" fn(Obj, Obj) -> HRESULT>(dlg, SET_FOLDER)(dlg, item);
            release(item);
        }
    }
    Some(dlg)
}

/// Open Folder: asks for one folder.
pub unsafe fn pick_folder(owner: HWND, initial: Option<&Path>) -> Option<PathBuf> {
    let dlg = create(FOS_PICKFOLDERS, initial)?;
    let mut result = None;
    if method::<unsafe extern "system" fn(Obj, HWND) -> HRESULT>(dlg, SHOW)(dlg, owner) == 0 {
        let mut item: Obj = null_mut();
        if method::<unsafe extern "system" fn(Obj, *mut Obj) -> HRESULT>(dlg, GET_RESULT)(dlg, &mut item) == 0 {
            result = take_path(item);
        }
    }
    release(dlg);
    result
}

/// Receives the dialog's events; only the "Select Folder" button matters. It lives on the stack
/// for the time of the dialog, so reference counting is not needed.
#[repr(C)]
struct Events {
    events: *const [*const (); 10],  // IFileDialogEvents
    controls: *const [*const (); 7], // IFileDialogControlEvents
    dialog: Obj,
    folder: RefCell<Option<PathBuf>>,
}

unsafe extern "system" fn query_interface(this: Obj, iid: *const GUID, out: *mut Obj) -> HRESULT {
    let me = this as *mut Events; // both interfaces are answered for the whole object
    let iid = &*iid;
    let same = |g: &GUID| g.data1 == iid.data1 && g.data2 == iid.data2 && g.data3 == iid.data3 && g.data4 == iid.data4;
    *out = if same(&IID_IUNKNOWN) || same(&IID_IFILE_DIALOG_EVENTS) {
        me as Obj
    } else if same(&IID_IFILE_DIALOG_CONTROL_EVENTS) {
        std::ptr::addr_of_mut!((*me).controls) as Obj
    } else {
        null_mut()
    };
    if (*out).is_null() { E_NOINTERFACE } else { 0 }
}

unsafe extern "system" fn control_query_interface(this: Obj, iid: *const GUID, out: *mut Obj) -> HRESULT {
    query_interface((this as *mut Obj).sub(1) as Obj, iid, out)
}

unsafe extern "system" fn add_ref(_: Obj) -> u32 {
    1
}

unsafe extern "system" fn ok1(_: Obj, _: Obj) -> HRESULT {
    0
}

unsafe extern "system" fn ok2(_: Obj, _: Obj, _: Obj) -> HRESULT {
    0
}

unsafe extern "system" fn not_impl3(_: Obj, _: Obj, _: Obj, _: Obj) -> HRESULT {
    E_NOTIMPL
}

unsafe extern "system" fn ok_id_item(_: Obj, _: Obj, _: u32, _: u32) -> HRESULT {
    0
}

unsafe extern "system" fn ok_id_bool(_: Obj, _: Obj, _: u32, _: BOOL) -> HRESULT {
    0
}

unsafe extern "system" fn ok_id(_: Obj, _: Obj, _: u32) -> HRESULT {
    0
}

/// "Select Folder": the folder selected in the list, or else the folder being shown.
unsafe extern "system" fn button_clicked(this: Obj, _: Obj, id: u32) -> HRESULT {
    let me = &*((this as *mut Obj).sub(1) as *const Events);
    if id != SELECT_FOLDER {
        return 0;
    }
    let get = |index: usize| {
        let mut item: Obj = null_mut();
        (method::<unsafe extern "system" fn(Obj, *mut Obj) -> HRESULT>(me.dialog, index)(me.dialog, &mut item) == 0 && !item.is_null())
            .then(|| take_path(item)).flatten()
    };
    let folder = get(GET_CURRENT_SELECTION).filter(|p| p.is_dir()).or_else(|| get(GET_FOLDER));
    if folder.is_some() {
        *me.folder.borrow_mut() = folder;
        method::<unsafe extern "system" fn(Obj, HRESULT) -> HRESULT>(me.dialog, CLOSE)(me.dialog, 1);
    }
    0
}

#[repr(C)]
struct FilterSpec {
    name: PCWSTR,
    spec: PCWSTR,
}

/// Ctrl+O: one or more files, or (with the "Select Folder" button) a folder.
/// Returns files and folders; folders can also come from a multi-selection.
pub unsafe fn open(owner: HWND, initial: Option<&Path>) -> Vec<PathBuf> {
    let Some(dlg) = create(FOS_ALLOWMULTISELECT | FOS_FILEMUSTEXIST, initial) else { return Vec::new() };
    let types = [FilterSpec { name: w!("All Files (*.*)"), spec: w!("*.*") }, FilterSpec { name: w!("Text Files (*.txt)"), spec: w!("*.txt") }];
    method::<unsafe extern "system" fn(Obj, u32, *const FilterSpec) -> HRESULT>(dlg, SET_FILE_TYPES)(dlg, types.len() as u32, types.as_ptr());
    let mut custom: Obj = null_mut();
    if method::<unsafe extern "system" fn(Obj, *const GUID, *mut Obj) -> HRESULT>(dlg, 0)(dlg, &IID_IFILE_DIALOG_CUSTOMIZE, &mut custom) == 0 {
        method::<unsafe extern "system" fn(Obj, u32, PCWSTR) -> HRESULT>(custom, ADD_PUSH_BUTTON)(custom, SELECT_FOLDER, w!("Select Folder"));
        release(custom);
    }
    let events_vtable: [*const (); 10] = [
        query_interface as *const (), add_ref as *const (), add_ref as *const (),
        ok1 as *const (), // OnFileOk
        ok2 as *const (), // OnFolderChanging
        ok1 as *const (), // OnFolderChange
        ok1 as *const (), // OnSelectionChange
        not_impl3 as *const (), // OnShareViolation
        ok1 as *const (), // OnTypeChange
        not_impl3 as *const (), // OnOverwrite
    ];

    let controls_vtable: [*const (); 7] = [
        control_query_interface as *const (), add_ref as *const (), add_ref as *const (),
        ok_id_item as *const (), // OnItemSelected
        button_clicked as *const (), // OnButtonClicked
        ok_id_bool as *const (), // OnCheckButtonToggled
        ok_id as *const (), // OnControlActivating
    ];
    let events = Events { events: &events_vtable, controls: &controls_vtable, dialog: dlg, folder: RefCell::new(None) };
    let mut cookie = 0u32;
    let advised = method::<unsafe extern "system" fn(Obj, Obj, *mut u32) -> HRESULT>(dlg, ADVISE)(dlg, &events as *const _ as Obj, &mut cookie) == 0;
    let shown = method::<unsafe extern "system" fn(Obj, HWND) -> HRESULT>(dlg, SHOW)(dlg, owner);
    if advised {
        method::<unsafe extern "system" fn(Obj, u32) -> HRESULT>(dlg, UNADVISE)(dlg, cookie);
    }
    let mut paths = Vec::new();
    if let Some(folder) = events.folder.borrow_mut().take() {
        paths.push(folder);
    } else if shown == 0 {
        let mut items: Obj = null_mut();
        if method::<unsafe extern "system" fn(Obj, *mut Obj) -> HRESULT>(dlg, GET_RESULTS)(dlg, &mut items) == 0 {
            let mut n = 0u32;
            method::<unsafe extern "system" fn(Obj, *mut u32) -> HRESULT>(items, GET_COUNT)(items, &mut n);
            for i in 0..n {
                let mut item: Obj = null_mut();
                if method::<unsafe extern "system" fn(Obj, u32, *mut Obj) -> HRESULT>(items, GET_ITEM_AT)(items, i, &mut item) == 0 {
                    paths.extend(take_path(item));
                }
            }
            release(items);
        }
    }
    release(dlg);
    paths
}
