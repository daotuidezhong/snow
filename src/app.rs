//! Windows, tray icon, menu, hotkey and the frame loop.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, ERROR_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{
    GetDpiForSystem, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::{self, Config};
use crate::snow::{FlakeSize, Intensity, Snow, Weather, Wind};
use crate::{autostart, render::Renderer};

const WM_TRAY: u32 = WM_APP + 1;
const HOTKEY_PAUSE: i32 = 1;
const TIMER_MENU_FRAME: usize = 1;

const ID_PAUSE: u32 = 100;
const ID_INTENSITY: u32 = 110; // + index
const ID_WIND: u32 = 120; // + index
const ID_WIND_LEFT: u32 = 125;
const ID_WIND_RIGHT: u32 = 126;
const ID_SIZE: u32 = 130; // + index
const ID_AUTOSTART: u32 = 140;
const ID_ABOUT: u32 = 150;
const ID_EXIT: u32 = 160;

struct App {
    overlay: HWND,
    renderer: Option<Renderer>,
    snow: Snow,
    cfg: Config,
    last: Instant,
    last_topmost: Instant,
    last_device_try: Instant,
    taskbar_created: u32,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<T>(f: impl FnOnce(&mut App) -> T) -> Option<T> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut a| a.as_mut().map(f)))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The rectangle covering every monitor, in physical pixels.
fn virtual_screen() -> (i32, i32, i32, i32) {
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1),
            GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1),
        )
    }
}

fn dpi_scale() -> f32 {
    (unsafe { GetDpiForSystem() } as f32 / 96.0).max(1.0)
}

fn seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1)
}

fn message_box(text: &str) {
    let t = wide(text);
    unsafe {
        MessageBoxW(HWND::default(), PCWSTR(t.as_ptr()), w!("Snow"), MB_OK | MB_ICONINFORMATION);
    }
}

pub fn run() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        // One copy at a time; a second launch would just stack another layer of snow.
        let _mutex = CreateMutexW(None, true, w!("Local\\SnowDesktop.SingleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            message_box("Snow 已经在运行了。\n\n请在屏幕右下角的托盘区（或 ^ 隐藏图标里）找到雪花图标。");
            return;
        }

        let hinst: HINSTANCE = GetModuleHandleW(None).map(|m| m.into()).unwrap_or_default();
        let icon = LoadIconW(hinst, PCWSTR(1 as _)).unwrap_or_default();

        let overlay_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(overlay_proc),
            hInstance: hinst,
            lpszClassName: w!("SnowOverlay"),
            hIcon: icon,
            ..Default::default()
        };
        let tray_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(tray_proc),
            hInstance: hinst,
            lpszClassName: w!("SnowTray"),
            hIcon: icon,
            ..Default::default()
        };
        RegisterClassExW(&overlay_class);
        RegisterClassExW(&tray_class);

        let (x, y, w, h) = virtual_screen();
        let overlay = match CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP
                | WS_EX_LAYERED
                | WS_EX_TRANSPARENT
                | WS_EX_TOPMOST
                | WS_EX_TOOLWINDOW
                | WS_EX_NOACTIVATE,
            w!("SnowOverlay"),
            w!("Snow"),
            WS_POPUP,
            x,
            y,
            w,
            h,
            HWND::default(),
            HMENU::default(),
            hinst,
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                message_box(&format!("Snow 无法创建窗口：{e}"));
                return;
            }
        };
        // Layered + transparent makes the window click-through; alpha 255 keeps
        // the DirectComposition content fully visible.
        let _ = SetLayeredWindowAttributes(overlay, windows::Win32::Foundation::COLORREF(0), 255, LWA_ALPHA);

        let tray = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("SnowTray"),
            w!("Snow"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            HWND::default(),
            HMENU::default(),
            hinst,
            None,
        )
        .unwrap_or_default();

        let cfg = config::load();
        autostart::refresh();
        let renderer = Renderer::new(overlay, w as u32, h as u32).ok();
        let snow = Snow::new(w as f32, h as f32, dpi_scale(), cfg.weather, seed());
        let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));

        APP.with(|a| {
            *a.borrow_mut() = Some(App {
                overlay,
                renderer,
                snow,
                cfg,
                last: Instant::now(),
                last_topmost: Instant::now(),
                last_device_try: Instant::now(),
                taskbar_created,
            })
        });

        add_tray_icon(tray, hinst);
        if RegisterHotKey(tray, HOTKEY_PAUSE, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, 'S' as u32).is_err() {
            // Another program owns the shortcut; the tray menu still works.
        }
        if !cfg.paused {
            let _ = ShowWindow(overlay, SW_SHOWNOACTIVATE);
        }

        main_loop();

        with_app(|a| config::save(&a.cfg));
        remove_tray_icon(tray);
    }
}

fn main_loop() {
    let mut msg = MSG::default();
    loop {
        let paused = with_app(|a| a.cfg.paused).unwrap_or(true);
        unsafe {
            if paused {
                // Nothing to animate: sleep until a message arrives.
                if !GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
                    return;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
                continue;
            }
            while PeekMessageW(&mut msg, HWND::default(), 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let t0 = Instant::now();
        frame();
        // Present(1) normally waits for vsync. If it returned immediately (some
        // drivers do for composition swap chains), wait for the compositor so we
        // never spin the CPU.
        if t0.elapsed() < Duration::from_millis(4) {
            unsafe {
                if DwmFlush().is_err() {
                    std::thread::sleep(Duration::from_millis(15));
                }
            }
        }
    }
}

fn frame() {
    with_app(|a| {
        let now = Instant::now();
        let dt = (now - a.last).as_secs_f32();
        a.last = now;
        a.snow.step(dt);

        if a.renderer.is_none() && now - a.last_device_try > Duration::from_secs(2) {
            a.last_device_try = now;
            let (_, _, w, h) = virtual_screen();
            a.renderer = Renderer::new(a.overlay, w as u32, h as u32).ok();
        }
        if let Some(r) = a.renderer.as_mut() {
            if r.draw(&a.snow.flakes).is_err() {
                // GPU reset or driver update: rebuild on a later frame.
                a.renderer = None;
                a.last_device_try = now;
            }
        }

        // Other always-on-top windows (the taskbar) can rise above us; reclaim the top now and then.
        if now - a.last_topmost > Duration::from_secs(3) {
            a.last_topmost = now;
            unsafe {
                let _ = SetWindowPos(
                    a.overlay,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                );
            }
        }
    });
}

fn tray_tip() -> [u16; 128] {
    let mut tip = [0u16; 128];
    for (i, c) in "Snow 桌面下雪（右键打开菜单）".encode_utf16().take(127).enumerate() {
        tip[i] = c;
    }
    tip
}

fn add_tray_icon(tray: HWND, hinst: HINSTANCE) {
    unsafe {
        let cx = GetSystemMetrics(SM_CXSMICON);
        let cy = GetSystemMetrics(SM_CYSMICON);
        let icon = LoadImageW(hinst, PCWSTR(1 as _), IMAGE_ICON, cx, cy, LR_DEFAULTCOLOR)
            .map(|h| HICON(h.0))
            .or_else(|_| LoadIconW(HINSTANCE::default(), IDI_APPLICATION))
            .unwrap_or_default();
        let nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: tray,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: icon,
            szTip: tray_tip(),
            ..Default::default()
        };
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

fn remove_tray_icon(tray: HWND) {
    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: tray,
        uID: 1,
        ..Default::default()
    };
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

fn set_paused(paused: bool) {
    with_app(|a| {
        if a.cfg.paused == paused {
            return;
        }
        a.cfg.paused = paused;
        unsafe {
            if paused {
                if let Some(r) = a.renderer.as_mut() {
                    let _ = r.clear();
                }
                let _ = ShowWindow(a.overlay, SW_HIDE);
            } else {
                a.last = Instant::now();
                let _ = ShowWindow(a.overlay, SW_SHOWNOACTIVATE);
            }
        }
        config::save(&a.cfg);
    });
}

fn set_weather(f: impl FnOnce(&mut Weather)) {
    with_app(|a| {
        let mut w = a.cfg.weather;
        f(&mut w);
        a.cfg.weather = w;
        a.snow.set_weather(w);
        config::save(&a.cfg);
    });
}

fn on_display_change() {
    with_app(|a| {
        let (x, y, w, h) = virtual_screen();
        unsafe {
            let _ = SetWindowPos(a.overlay, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE);
        }
        a.snow.resize(w as f32, h as f32, dpi_scale());
        if let Some(r) = a.renderer.as_mut() {
            if r.resize(w as u32, h as u32).is_err() {
                a.renderer = None;
            }
        }
    });
}

unsafe fn append(menu: HMENU, id: u32, text: &str, checked: bool) {
    let t = wide(text);
    let flags = MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED };
    let _ = AppendMenuW(menu, flags, id as usize, PCWSTR(t.as_ptr()));
}

unsafe fn append_sub(menu: HMENU, sub: HMENU, text: &str) {
    let t = wide(text);
    let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, PCWSTR(t.as_ptr()));
}

unsafe fn separator(menu: HMENU) {
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
}

fn show_menu(tray: HWND) {
    let Some((cfg, w)) = with_app(|a| (a.cfg, a.cfg.weather)) else { return };
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };

        append(menu, ID_PAUSE, "暂停下雪\tCtrl+Alt+S", cfg.paused);
        separator(menu);

        if let Ok(sub) = CreatePopupMenu() {
            let names = ["小雪", "中雪", "大雪", "暴风雪"];
            for (i, v) in Intensity::ALL.iter().enumerate() {
                append(sub, ID_INTENSITY + i as u32, names[i], w.intensity == *v);
            }
            append_sub(menu, sub, "降雪量");
        }
        if let Ok(sub) = CreatePopupMenu() {
            let names = ["无风", "微风", "大风"];
            for (i, v) in Wind::ALL.iter().enumerate() {
                append(sub, ID_WIND + i as u32, names[i], w.wind == *v);
            }
            separator(sub);
            append(sub, ID_WIND_LEFT, "向左吹", w.wind_dir < 0.0);
            append(sub, ID_WIND_RIGHT, "向右吹", w.wind_dir >= 0.0);
            append_sub(menu, sub, "风");
        }
        if let Ok(sub) = CreatePopupMenu() {
            let names = ["小", "中", "大"];
            for (i, v) in FlakeSize::ALL.iter().enumerate() {
                append(sub, ID_SIZE + i as u32, names[i], w.size == *v);
            }
            append_sub(menu, sub, "雪花大小");
        }
        separator(menu);
        append(menu, ID_AUTOSTART, "开机自动启动", autostart::is_enabled());
        append(menu, ID_ABOUT, &format!("关于 Snow {}", env!("CARGO_PKG_VERSION")), false);
        separator(menu);
        append(menu, ID_EXIT, "退出", false);

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // Required so the menu closes when the user clicks elsewhere.
        let _ = SetForegroundWindow(tray);
        // Keep the snow moving while the menu's modal loop runs.
        SetTimer(tray, TIMER_MENU_FRAME, 16, None);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_RIGHTALIGN,
            pt.x,
            pt.y,
            0,
            tray,
            None,
        );
        let _ = KillTimer(tray, TIMER_MENU_FRAME);
        let _ = PostMessageW(tray, WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        run_command(cmd.0 as u32);
    }
}

fn run_command(id: u32) {
    match id {
        0 => {}
        ID_PAUSE => {
            let p = with_app(|a| a.cfg.paused).unwrap_or(false);
            set_paused(!p);
        }
        ID_WIND_LEFT => set_weather(|w| w.wind_dir = -1.0),
        ID_WIND_RIGHT => set_weather(|w| w.wind_dir = 1.0),
        ID_AUTOSTART => autostart::set(!autostart::is_enabled()),
        ID_ABOUT => message_box(&format!(
            "Snow {}\n\n让雪花落在你的桌面上。\n\n快捷键：Ctrl+Alt+S 暂停 / 继续\n设置保存在：%APPDATA%\\Snow\\config.txt",
            env!("CARGO_PKG_VERSION")
        )),
        ID_EXIT => unsafe { PostQuitMessage(0) },
        _ if (ID_INTENSITY..ID_INTENSITY + 4).contains(&id) => {
            set_weather(|w| w.intensity = Intensity::ALL[(id - ID_INTENSITY) as usize])
        }
        _ if (ID_WIND..ID_WIND + 3).contains(&id) => set_weather(|w| w.wind = Wind::ALL[(id - ID_WIND) as usize]),
        _ if (ID_SIZE..ID_SIZE + 3).contains(&id) => set_weather(|w| w.size = FlakeSize::ALL[(id - ID_SIZE) as usize]),
        _ => {}
    }
}

extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            // Belt and braces: never take mouse input.
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

extern "system" fn tray_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TRAY => {
                let event = (lp.0 & 0xffff) as u32;
                if event == WM_RBUTTONUP || event == WM_LBUTTONUP {
                    show_menu(hwnd);
                }
                LRESULT(0)
            }
            WM_HOTKEY if wp.0 as i32 == HOTKEY_PAUSE => {
                run_command(ID_PAUSE);
                LRESULT(0)
            }
            WM_TIMER if wp.0 == TIMER_MENU_FRAME => {
                if with_app(|a| !a.cfg.paused).unwrap_or(false) {
                    frame();
                }
                LRESULT(0)
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED => {
                on_display_change();
                LRESULT(0)
            }
            WM_SETTINGCHANGE => {
                on_display_change();
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            _ => {
                // Explorer restarted: the tray is new and empty, so add the icon again.
                let tb = with_app(|a| a.taskbar_created).unwrap_or(0);
                if tb != 0 && msg == tb {
                    let hinst: HINSTANCE = GetModuleHandleW(None).map(|m| m.into()).unwrap_or_default();
                    add_tray_icon(hwnd, hinst);
                    return LRESULT(0);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }
    }
}
