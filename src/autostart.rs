//! "Start with Windows": a value under HKCU\...\Run pointing at this exe.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE: PCWSTR = w!("Snow");

fn with_key<T>(f: impl FnOnce(HKEY) -> T) -> Option<T> {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, 0, KEY_QUERY_VALUE | KEY_SET_VALUE, &mut key) != ERROR_SUCCESS {
            return None;
        }
        let r = f(key);
        let _ = RegCloseKey(key);
        Some(r)
    }
}

fn command_line() -> Option<Vec<u8>> {
    let exe = std::env::current_exe().ok()?;
    let s = format!("\"{}\"", exe.display());
    let wide: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    Some(wide.iter().flat_map(|c| c.to_le_bytes()).collect())
}

pub fn is_enabled() -> bool {
    with_key(|k| unsafe { RegQueryValueExW(k, VALUE, None, None, None, None) == ERROR_SUCCESS }).unwrap_or(false)
}

pub fn set(enabled: bool) {
    with_key(|k| unsafe {
        if enabled {
            if let Some(data) = command_line() {
                let _ = RegSetValueExW(k, VALUE, 0, REG_SZ, Some(&data));
            }
        } else {
            let _ = RegDeleteValueW(k, VALUE);
        }
    });
}

/// If autostart is on but the exe was moved, point the entry at the new location.
pub fn refresh() {
    if is_enabled() {
        set(true);
    }
}
