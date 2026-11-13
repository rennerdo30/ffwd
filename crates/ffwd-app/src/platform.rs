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

/// Clipboard text (Windows: the system clipboard API, no process).
#[cfg(windows)]
pub fn clipboard() -> Option<String> {
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    const CF_UNICODETEXT: u32 = 13;
    // SAFETY: the clipboard is opened and closed on this thread; the
    // locked memory is read within its reported size and unlocked again.
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return None;
        }
        let mut out = None;
        let h = GetClipboardData(CF_UNICODETEXT);
        if !h.is_null() {
            let p = GlobalLock(h) as *const u16;
            if !p.is_null() {
                let max = GlobalSize(h) / 2;
                let len = (0..max).take_while(|&i| *p.add(i) != 0).count();
                out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, len.min(64 * 1024))));
                GlobalUnlock(h);
            }
        }
        CloseClipboard();
        out
    }
}

#[cfg(windows)]
pub fn set_clipboard(text: &str) {
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    const CF_UNICODETEXT: u32 = 13;
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    // SAFETY: the allocation is sized for `wide`, written while locked, and
    // owned by the clipboard after a successful SetClipboardData.
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return;
        }
        EmptyClipboard();
        let h = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        if !h.is_null() {
            let p = GlobalLock(h) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
                GlobalUnlock(h);
                SetClipboardData(CF_UNICODETEXT, h);
            }
        }
        CloseClipboard();
    }
}

/// macOS and Linux: the desktop's clipboard tools (pbpaste, wl-paste, xclip).
#[cfg(not(windows))]
pub fn clipboard() -> Option<String> {
    let tries: &[&[&str]] = if cfg!(target_os = "macos") {
        &[&["pbpaste"]]
    } else {
        &[&["wl-paste", "--no-newline"], &["xclip", "-selection", "clipboard", "-o"]]
    };
    tries.iter().find_map(|c| {
        let out = std::process::Command::new(c[0]).args(&c[1..]).stdin(std::process::Stdio::null()).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    })
}

#[cfg(not(windows))]
pub fn set_clipboard(text: &str) {
    use std::io::Write;
    let tries: &[&[&str]] =
        if cfg!(target_os = "macos") { &[&["pbcopy"]] } else { &[&["wl-copy"], &["xclip", "-selection", "clipboard"]] };
    for c in tries {
        if let Ok(mut child) =
            std::process::Command::new(c[0]).args(&c[1..]).stdin(std::process::Stdio::piped()).spawn()
        {
            if let Some(mut si) = child.stdin.take() {
                let _ = si.write_all(text.as_bytes());
            }
            let _ = child.wait();
            return;
        }
    }
}

/// The installed WSL distributions (`wsl.exe -l -q`), at most five
/// seconds; empty when WSL is missing or slow. Windows only.
#[cfg(windows)]
pub fn wsl_distros() -> Vec<String> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(mut child) = Command::new("wsl.exe")
        .args(["-l", "-q"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
    else {
        return Vec::new();
    };
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < until => std::thread::sleep(std::time::Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Vec::new();
            }
        }
    }
    let mut out = Vec::new();
    if let Some(mut s) = child.stdout.take() {
        use std::io::Read;
        let _ = s.by_ref().take(64 << 10).read_to_end(&mut out);
    }
    parse_wsl_list(&out)
}

#[cfg(not(windows))]
pub fn wsl_distros() -> Vec<String> {
    Vec::new()
}

/// `wsl -l -q` writes UTF-16LE, one distribution per line.
pub fn parse_wsl_list(out: &[u8]) -> Vec<String> {
    // A UTF-16LE byte-order mark, or zero high bytes: UTF-16; else UTF-8.
    let (body, utf16) = match out.strip_prefix(&[0xFF, 0xFE]) {
        Some(rest) => (rest, true),
        None => (out, out.len() >= 2 && out.iter().skip(1).step_by(2).take(16).all(|&b| b == 0)),
    };
    let text = if utf16 {
        let units: Vec<u16> = body.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(body).into_owned()
    };
    text.lines()
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}' || c == '\0').to_string())
        .filter(|l| !l.is_empty())
        .take(64)
        .collect()
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

/// Opens `dir` in the file manager. Runs on its own thread; the path is
/// passed as one argument, never through a shell.
pub fn open_folder(dir: std::path::PathBuf) {
    std::thread::spawn(move || {
        #[cfg(windows)]
        let _ = std::process::Command::new("explorer.exe").arg(&dir).spawn();
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(&dir).spawn();
        #[cfg(not(any(windows, target_os = "macos")))]
        let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
    });
}

/// Opens a terminal in `dir`: Windows Terminal, else a console; Terminal
/// on macOS; the desktop's terminal on Linux.
pub fn open_terminal(dir: std::path::PathBuf) {
    std::thread::spawn(move || {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            if std::process::Command::new("wt.exe").arg("-d").arg(&dir).spawn().is_err() {
                // A new console in the folder (the path is the working
                // directory, not part of a command line).
                const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
                let _ = std::process::Command::new("cmd.exe")
                    .arg("/K")
                    .current_dir(&dir)
                    .creation_flags(CREATE_NEW_CONSOLE)
                    .spawn();
            }
        }
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").args(["-a", "Terminal"]).arg(&dir).spawn();
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let tried = ["x-terminal-emulator", "gnome-terminal", "konsole", "xfce4-terminal", "xterm"];
            for t in tried {
                if std::process::Command::new(t).current_dir(&dir).spawn().is_ok() {
                    break;
                }
            }
        }
    });
}

/// Opens an https URL in the default browser (as one argument, no shell).
pub fn open_url(url: String) {
    if !url.starts_with("https://") {
        return;
    }
    std::thread::spawn(move || {
        #[cfg(windows)]
        let _ = std::process::Command::new("explorer.exe").arg(&url).spawn();
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(&url).spawn();
        #[cfg(not(any(windows, target_os = "macos")))]
        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    });
}

/// Opens a file with its default app (as one argument, no shell parsing
/// of the path).
pub fn open_path(p: std::path::PathBuf) {
    std::thread::spawn(move || {
        #[cfg(windows)]
        {
            // explorer.exe opens a file with its associated app.
            let _ = std::process::Command::new("explorer.exe").arg(&p).spawn();
        }
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(&p).spawn();
        #[cfg(not(any(windows, target_os = "macos")))]
        let _ = std::process::Command::new("xdg-open").arg(&p).spawn();
    });
}

/// Shows a file selected in the file manager (its folder elsewhere).
pub fn show_file(p: std::path::PathBuf) {
    std::thread::spawn(move || {
        #[cfg(windows)]
        {
            let mut arg = std::ffi::OsString::from("/select,");
            arg.push(p.as_os_str());
            let _ = std::process::Command::new("explorer.exe").arg(arg).spawn();
        }
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg("-R").arg(&p).spawn();
        #[cfg(not(any(windows, target_os = "macos")))]
        if let Some(dir) = p.parent() {
            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
        }
    });
}

#[cfg(test)]
mod wsl_tests {
    #[test]
    fn wsl_list_parses_utf16_and_utf8() {
        let utf16: Vec<u8> =
            "\u{feff}Ubuntu-24.04\r\nDebian\r\n".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(super::parse_wsl_list(&utf16), ["Ubuntu-24.04", "Debian"]);
        assert_eq!(super::parse_wsl_list(b"Ubuntu\nkali-linux\n"), ["Ubuntu", "kali-linux"]);
        assert!(super::parse_wsl_list(b"").is_empty());
    }
}
