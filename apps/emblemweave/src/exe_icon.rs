//! Gives the game executable EmblemWeave's icon.
//!
//! Windows draws a program's taskbar button from the icon stored in its .exe,
//! and the engine builds its own artwork into `fefw.exe`. The game is built on
//! the player's PC, so after each build (and before each start) the launcher
//! replaces that icon resource with EmblemWeave's: the old icon entries are
//! removed and ours are written under the same group name and language.

#![cfg(windows)]

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows_sys::Win32::Foundation::{FreeLibrary, BOOL, HMODULE};
use windows_sys::Win32::System::LibraryLoader::*;

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const OURS: &[u8] = include_bytes!("../assets/icon.ico");

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain([0]).collect()
}

/// A resource name: an integer id or a string.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Name {
    Id(u16),
    Text(Vec<u16>),
}

impl Name {
    fn from_ptr(p: *const u16) -> Name {
        let v = p as usize;
        if v >> 16 == 0 {
            Name::Id(v as u16)
        } else {
            let mut n = 0;
            unsafe {
                while *p.add(n) != 0 {
                    n += 1;
                }
                Name::Text(std::slice::from_raw_parts(p, n).iter().copied().chain([0]).collect())
            }
        }
    }

    fn ptr(&self) -> *const u16 {
        match self {
            Name::Id(i) => *i as usize as *const u16,
            Name::Text(t) => t.as_ptr(),
        }
    }
}

unsafe extern "system" fn on_name(_m: HMODULE, _t: *const u16, name: *const u16, out: isize) -> BOOL {
    let v = unsafe { &mut *(out as *mut Vec<Name>) };
    v.push(Name::from_ptr(name));
    1
}

unsafe extern "system" fn on_lang(_m: HMODULE, _t: *const u16, _n: *const u16, lang: u16, out: isize) -> BOOL {
    let v = unsafe { &mut *(out as *mut Vec<u16>) };
    v.push(lang);
    1
}

/// Every (name, language) of a resource type in the module.
fn resources(m: HMODULE, kind: u16) -> Vec<(Name, u16)> {
    let mut names: Vec<Name> = Vec::new();
    unsafe { EnumResourceNamesW(m, kind as usize as *const u16, Some(on_name), &mut names as *mut _ as isize) };
    let mut out = Vec::new();
    for n in names {
        let mut langs: Vec<u16> = Vec::new();
        unsafe { EnumResourceLanguagesW(m, kind as usize as *const u16, n.ptr(), Some(on_lang), &mut langs as *mut _ as isize) };
        for l in langs {
            out.push((n.clone(), l));
        }
    }
    out
}

/// The group icon's bytes as stored in the module, to see whether it is ours.
fn group_bytes(m: HMODULE, name: &Name, lang: u16) -> Option<Vec<u8>> {
    unsafe {
        let r = FindResourceExW(m, RT_GROUP_ICON as usize as *const u16, name.ptr(), lang);
        if r.is_null() {
            return None;
        }
        let size = SizeofResource(m, r) as usize;
        let h = LoadResource(m, r);
        let p = LockResource(h) as *const u8;
        (!p.is_null()).then(|| std::slice::from_raw_parts(p, size).to_vec())
    }
}

/// Our .ico as the images and the group directory that points at them by id.
fn ours(first_id: u16) -> Option<(Vec<(u16, Vec<u8>)>, Vec<u8>)> {
    let d = OURS;
    let count = u16::from_le_bytes([*d.get(4)?, *d.get(5)?]) as usize;
    let mut images = Vec::new();
    let mut group = vec![0, 0, 1, 0];
    group.extend((count as u16).to_le_bytes());
    for i in 0..count {
        let e = d.get(6 + 16 * i..22 + 16 * i)?;
        let size = u32::from_le_bytes(e[8..12].try_into().ok()?) as usize;
        let off = u32::from_le_bytes(e[12..16].try_into().ok()?) as usize;
        let id = first_id + i as u16;
        images.push((id, d.get(off..off + size)?.to_vec()));
        group.extend(&e[0..12]); // width, height, colours, reserved, planes, bit count, bytes
        group.extend(id.to_le_bytes());
    }
    Some((images, group))
}

/// Give `exe` EmblemWeave's icon. Returns whether it changed anything.
pub fn brand(exe: &Path) -> Result<bool, String> {
    let w = wide(exe.as_os_str());
    // Read what is there now.
    let m = unsafe { LoadLibraryExW(w.as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE) };
    if m.is_null() {
        return Err(format!("could not open {}: {}", exe.display(), std::io::Error::last_os_error()));
    }
    let groups = resources(m, RT_GROUP_ICON);
    let icons = resources(m, RT_ICON);
    let (name, lang) = groups.first().cloned().unwrap_or((Name::Id(1), 1033));
    let first_id = icons.iter().filter_map(|(n, _)| if let Name::Id(i) = n { Some(*i) } else { None }).min().unwrap_or(1);
    let current = group_bytes(m, &name, lang);
    unsafe { FreeLibrary(m) };
    let (images, group) = ours(first_id).ok_or("EmblemWeave's icon is not a valid .ico")?;
    if current.as_deref() == Some(&group[..]) && groups.len() == 1 {
        return Ok(false);
    }
    unsafe {
        let h = BeginUpdateResourceW(w.as_ptr(), 0);
        if h.is_null() {
            return Err(format!("could not update {}: {}", exe.display(), std::io::Error::last_os_error()));
        }
        // Remove every old icon and group, then write ours.
        for (n, l) in &icons {
            UpdateResourceW(h, RT_ICON as usize as *const u16, n.ptr(), *l, std::ptr::null(), 0);
        }
        for (n, l) in &groups {
            UpdateResourceW(h, RT_GROUP_ICON as usize as *const u16, n.ptr(), *l, std::ptr::null(), 0);
        }
        for (id, data) in &images {
            UpdateResourceW(h, RT_ICON as usize as *const u16, *id as usize as *const u16, lang, data.as_ptr() as _, data.len() as u32);
        }
        UpdateResourceW(h, RT_GROUP_ICON as usize as *const u16, name.ptr(), lang, group.as_ptr() as _, group.len() as u32);
        if EndUpdateResourceW(h, 0) == 0 {
            return Err(format!("could not write {}: {}", exe.display(), std::io::Error::last_os_error()));
        }
        // Let Explorer and the taskbar drop their cached copy of the old icon.
        windows_sys::Win32::UI::Shell::SHChangeNotify(
            windows_sys::Win32::UI::Shell::SHCNE_ASSOCCHANGED as i32,
            windows_sys::Win32::UI::Shell::SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    #[test]
    fn our_icon_parses() {
        let (images, group) = super::ours(1).unwrap();
        assert_eq!(images.len(), 7);
        assert_eq!(group.len(), 6 + 14 * 7);
        for (_, d) in &images {
            // each image is a BITMAPINFOHEADER or a PNG
            assert!(d[..4] == [40, 0, 0, 0] || d[..4] == [0x89, b'P', b'N', b'G']);
        }
    }

    /// Brands a copy of the built game (EW_EXE) and leaves it for checking.
    #[test]
    #[ignore]
    fn brands_a_copy() {
        let src = std::path::PathBuf::from(std::env::var("EW_EXE").expect("EW_EXE"));
        let copy = std::env::temp_dir().join("ew-icon-test.exe");
        std::fs::copy(&src, &copy).unwrap();
        assert!(super::brand(&copy).unwrap(), "first run changes the icon");
        assert!(!super::brand(&copy).unwrap(), "second run finds it already done");
        println!("{}", copy.display());
    }
}
