//! Windows 11 snap layouts on a custom maximize button.
//!
//! The window has no native frame, so the web page draws its own caption buttons. The snap-layout flyout only
//! appears when Windows hit-tests the pointer as *the maximize button*, so the page reports where that button
//! is and this subclass answers `HTMAXBUTTON` for that spot. Windows then sends non-client mouse messages instead
//! of the webview getting them, so hover is forwarded as an event and the click is carried out here.

use std::sync::{Arc, Mutex};

/// The maximize button's rectangle in physical client pixels: left, top, right, bottom. `None` until the page reports it.
pub type ButtonRect = Arc<Mutex<Option<[i32; 4]>>>;

#[cfg(windows)]
pub fn install(window: &tauri::WebviewWindow, rect: ButtonRect, app: tauri::AppHandle) {
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;

    let Ok(hwnd) = window.hwnd() else { return };
    let ctx = Box::into_raw(Box::new(Ctx { rect, app, hovering: false }));
    // SAFETY: the context is leaked on purpose and freed in WM_NCDESTROY, the last message the window gets.
    unsafe {
        SetWindowSubclass(hwnd.0, Some(proc), 1, ctx as usize);
    }
}

#[cfg(not(windows))]
pub fn install(_window: &tauri::WebviewWindow, _rect: ButtonRect, _app: tauri::AppHandle) {}

#[cfg(windows)]
struct Ctx {
    rect: ButtonRect,
    app: tauri::AppHandle,
    hovering: bool,
}

#[cfg(windows)]
unsafe extern "system" fn proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
    _id: usize,
    data: usize,
) -> windows_sys::Win32::Foundation::LRESULT {
    use tauri::Emitter;
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT};
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IsZoomed, PostMessageW, HTCLIENT, HTMAXBUTTON, SC_MAXIMIZE, SC_RESTORE, WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE,
        WM_NCMOUSEMOVE, WM_SYSCOMMAND,
    };

    // SAFETY: `data` is the pointer installed above; it stays valid until WM_NCDESTROY.
    let ctx = unsafe { &mut *(data as *mut Ctx) };
    let over_button = wparam as u32 == HTMAXBUTTON;

    match msg {
        WM_NCHITTEST => {
            let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            // Resize borders keep priority; only the page area can become the maximize button.
            if result == HTCLIENT as isize {
                if let Some([l, t, r, b]) = *ctx.rect.lock().unwrap() {
                    let mut p = POINT { x: (lparam & 0xFFFF) as i16 as i32, y: ((lparam >> 16) & 0xFFFF) as i16 as i32 };
                    unsafe { ScreenToClient(hwnd, &mut p) };
                    if p.x >= l && p.x < r && p.y >= t && p.y < b {
                        return HTMAXBUTTON as isize;
                    }
                }
            }
            result
        }
        WM_NCMOUSEMOVE if over_button => {
            if !ctx.hovering {
                ctx.hovering = true;
                let _ = ctx.app.emit("caption-hover", true);
                let mut track = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE | TME_NONCLIENT, hwndTrack: hwnd, dwHoverTime: 0 };
                unsafe { TrackMouseEvent(&mut track) };
            }
            0
        }
        WM_NCMOUSELEAVE => {
            if ctx.hovering {
                ctx.hovering = false;
                let _ = ctx.app.emit("caption-hover", false);
            }
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }
        WM_NCLBUTTONDOWN if over_button => 0,
        WM_NCLBUTTONUP if over_button => {
            let command = if unsafe { IsZoomed(hwnd) } != 0 { SC_RESTORE } else { SC_MAXIMIZE };
            unsafe { PostMessageW(hwnd, WM_SYSCOMMAND, command as usize, 0) };
            0
        }
        WM_NCDESTROY => {
            unsafe {
                RemoveWindowSubclass(hwnd, Some(proc), 1);
                drop(Box::from_raw(data as *mut Ctx));
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
        }
        _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    }
}

