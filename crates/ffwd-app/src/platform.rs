//! Platform specifics: remote-session detection, screen-to-screen copies
//! for scrolling, and cloaked windows for agent smoke runs.

use ffwd_render::Rect;

#[cfg(windows)]
pub fn is_remote_session() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_REMOTESESSION};
    unsafe { GetSystemMetrics(SM_REMOTESESSION) != 0 }
}

#[cfg(not(windows))]
pub fn is_remote_session() -> bool {
    std::env::var_os("FFWD_REMOTE").is_some()
}

/// Moves window pixels on screen, so the compositor and RDP see a copy
/// instead of new pixels. The caller has already moved its own buffer.
#[cfg(windows)]
pub fn screen_copy(hwnd: isize, area: Rect, dy: i32) -> bool {
    use windows_sys::Win32::Graphics::Gdi::{BitBlt, GetDC, ReleaseDC, SRCCOPY};
    if hwnd == 0 || dy == 0 {
        return false;
    }
    let (src_y, dst_y, h) =
        if dy < 0 { (area.y - dy, area.y, area.h + dy) } else { (area.y, area.y + dy, area.h - dy) };
    if h <= 0 {
        return false;
    }
    unsafe {
        let hwnd = hwnd as windows_sys::Win32::Foundation::HWND;
        let dc = GetDC(hwnd);
        if dc.is_null() {
            return false;
        }
        let ok = BitBlt(dc, area.x, dst_y, area.w, h, dc, area.x, src_y, SRCCOPY) != 0;
        ReleaseDC(hwnd, dc);
        ok
    }
}

#[cfg(not(windows))]
pub fn screen_copy(_hwnd: isize, _area: Rect, _dy: i32) -> bool {
    false
}

/// Shows a window that DWM composes but never puts on screen (agent runs).
#[cfg(windows)]
pub fn cloak_and_show(hwnd: isize) -> bool {
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_SHOWNOACTIVATE, ShowWindow};
    let on: i32 = 1;
    unsafe {
        let hwnd = hwnd as windows_sys::Win32::Foundation::HWND;
        let hr = DwmSetWindowAttribute(hwnd, DWMWA_CLOAK as u32, (&on as *const i32).cast(), 4);
        if hr < 0 {
            return false;
        }
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    true
}

#[cfg(not(windows))]
pub fn cloak_and_show(_hwnd: isize) -> bool {
    false
}

/// Dark title bar on Windows 10/11 so the native caption matches the theme.
#[cfg(windows)]
pub fn dark_title_bar(hwnd: isize) {
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
    let on: i32 = 1;
    unsafe {
        let hwnd = hwnd as windows_sys::Win32::Foundation::HWND;
        DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, (&on as *const i32).cast(), 4);
    }
}

#[cfg(not(windows))]
pub fn dark_title_bar(_hwnd: isize) {}
