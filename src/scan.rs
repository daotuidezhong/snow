//! Find the surfaces snow can land on: the visible parts of window top edges
//! and the top of each taskbar. Only positions and sizes are read, never contents.

use windows::core::w;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, GetClassNameW, GetWindowLongW, GetWindowRect, IsIconic, IsWindowVisible,
    IsZoomed, GWL_EXSTYLE, GWL_STYLE, WS_CAPTION, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_THICKFRAME,
};

use crate::pile::{visible_parts, Ledge};

/// Taskbar ledges use ids that can't collide with window handles.
const TASKBAR_ID: u64 = u64::MAX - 1000;

struct Win {
    hwnd: HWND,
    rect: RECT,
}

/// What a scan found: surfaces snow can land on now, plus windows that still
/// exist but are out of sight for the moment (minimized, maximized, on another
/// virtual desktop). Snow on those is kept, not dropped, until they come back.
pub struct Scan {
    pub ledges: Vec<Ledge>,
    pub parked: Vec<u64>,
}

fn id_of(hwnd: HWND) -> u64 {
    hwnd.0 as usize as u64
}

unsafe fn frame_rect(hwnd: HWND) -> Option<RECT> {
    let mut r = RECT::default();
    // The visible frame, without the invisible resize borders Windows 10/11 add.
    let ok = DwmGetWindowAttribute(
        hwnd,
        DWMWA_EXTENDED_FRAME_BOUNDS,
        &mut r as *mut _ as *mut _,
        std::mem::size_of::<RECT>() as u32,
    );
    if ok.is_err() {
        GetWindowRect(hwnd, &mut r).ok()?;
    }
    Some(r)
}

unsafe fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = GetClassNameW(hwnd, &mut buf);
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Is this an ordinary application window (one that could carry snow)?
/// Hidden ones count too: an app "closed" to the tray only hides its window.
unsafe fn is_app_window(hwnd: HWND, own: &[HWND]) -> bool {
    if own.contains(&hwnd) {
        return false;
    }
    let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    if ex & (WS_EX_TOOLWINDOW.0 | WS_EX_TRANSPARENT.0) != 0 {
        return false;
    }
    let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
    if style & (WS_CAPTION.0 | WS_THICKFRAME.0) == 0 {
        return false;
    }
    !matches!(class_name(hwnd).as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
}

unsafe extern "system" fn collect(hwnd: HWND, lp: LPARAM) -> BOOL {
    let out = &mut *(lp.0 as *mut Vec<HWND>);
    out.push(hwnd);
    true.into()
}

/// Scan the desktop. `origin` is the overlay's top-left in screen coordinates.
/// `window_max` is the deepest snow allowed on a window's top edge, in physical
/// pixels; taskbars allow a little more.
pub fn scan(own: &[HWND], origin: (i32, i32), window_max: f32) -> Scan {
    let (ox, oy) = (origin.0 as f32, origin.1 as f32);
    let mut out = Vec::new();
    let mut parked = Vec::new();
    unsafe {
        let mut all: Vec<HWND> = Vec::new();
        let _ = EnumWindows(Some(collect), LPARAM(&mut all as *mut _ as isize));

        // EnumWindows lists top-level windows front to back.
        let mut wins: Vec<Win> = Vec::new();
        for hwnd in all {
            if !is_app_window(hwnd, own) {
                continue;
            }
            if !IsWindowVisible(hwnd).as_bool() {
                // Cheap check first: there are usually hundreds of hidden windows.
                parked.push(id_of(hwnd));
                continue;
            }
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4);
            // Hidden to the tray, minimized, on another virtual desktop, or maximized (its
            // top is the top of the screen): out of sight for now, but its snow is kept
            // for when it returns.
            if IsIconic(hwnd).as_bool() || cloaked != 0 || IsZoomed(hwnd).as_bool() {
                parked.push(id_of(hwnd));
                continue;
            }
            let Some(rect) = frame_rect(hwnd) else { continue };
            if rect.right - rect.left < 120 || rect.bottom - rect.top < 60 {
                continue;
            }
            wins.push(Win { hwnd, rect });
            if wins.len() >= 256 {
                break;
            }
        }

        for (i, win) in wins.iter().enumerate() {
            let r = win.rect;
            // A top edge at the very top of the screen has nowhere for snow to sit.
            if r.top <= origin.1 + 2 {
                parked.push(id_of(win.hwnd));
                continue;
            }
            let y = r.top as f32;
            let covers = wins[..i]
                .iter()
                .filter(|o| (o.rect.top as f32) <= y + 1.0 && (o.rect.bottom as f32) > y - 2.0)
                .map(|o| (o.rect.left as f32, o.rect.right as f32))
                .collect();
            // Fully covered windows are still reported (with nothing visible): their
            // snow stays put behind the window in front and is there again later.
            let visible = visible_parts(r.left as f32, r.right as f32, covers);
            out.push(Ledge {
                id: id_of(win.hwnd),
                x0: r.left as f32 - ox,
                x1: r.right as f32 - ox,
                y: y - oy,
                visible: visible.into_iter().map(|(a, b)| (a - ox, b - ox)).collect(),
                max_h: window_max,
            });
        }

        // Taskbars (primary and one per extra monitor). They sit above normal windows.
        let mut k = 0u64;
        for class in [w!("Shell_TrayWnd"), w!("Shell_SecondaryTrayWnd")] {
            let mut prev = HWND::default();
            while let Ok(tb) = FindWindowExW(HWND::default(), prev, class, None) {
                if tb.is_invalid() {
                    break;
                }
                prev = tb;
                let mut r = RECT::default();
                if !IsWindowVisible(tb).as_bool() || GetWindowRect(tb, &mut r).is_err() {
                    continue;
                }
                let (w, h) = (r.right - r.left, r.bottom - r.top);
                // Only horizontal taskbars along the bottom, and not while auto-hidden.
                if w <= h || h < 16 || r.top <= origin.1 + 2 {
                    continue;
                }
                let (x0, x1) = (r.left as f32 - ox, r.right as f32 - ox);
                out.push(Ledge {
                    id: TASKBAR_ID + k,
                    x0,
                    x1,
                    y: r.top as f32 - oy,
                    visible: vec![(x0, x1)],
                    max_h: window_max * 1.4,
                });
                k += 1;
            }
        }
    }
    Scan { ledges: out, parked }
}
