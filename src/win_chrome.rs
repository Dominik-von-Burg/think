/// Win32 hit-testing for the borderless custom frame.
///
/// Returns HT* borders for native resize, HTMAXBUTTON over the maximize
/// button, and HTCLIENT for the rest of the title strip. Native handling
/// provides the drop shadow, Aero snap, Win+arrows, the Alt+Space system
/// menu and, through HTMAXBUTTON, the Windows 11 Snap Layouts flyout.
///
/// Minimize and close stay HTCLIENT so egui handles them. Maximize reports
/// HTMAXBUTTON, so its NC mouse messages are handled here and NOT forwarded
/// to DefWindowProc: forwarding them makes Windows paint classic-style
/// buttons over ours. Hover/pressed state is published through atomics and
/// egui paints the button.
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::OnceLock;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromRect, ScreenToClient, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::UI::Controls::MARGINS;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TrackMouseEvent, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

static ORIG_PROC: AtomicIsize = AtomicIsize::new(0);
static CAPTION_H_PX: AtomicU32 = AtomicU32::new(36);
static BTN_W_PX: AtomicU32 = AtomicU32::new(46);
static MAX_HOVERED: AtomicBool = AtomicBool::new(false);
static MAX_PRESSED: AtomicBool = AtomicBool::new(false);
static EGUI_CTX: OnceLock<egui::Context> = OnceLock::new();

pub fn set_hit_metrics(caption_h_px: u32, btn_w_px: u32) {
    CAPTION_H_PX.store(caption_h_px.max(1), Ordering::Relaxed);
    BTN_W_PX.store(btn_w_px.max(1), Ordering::Relaxed);
}

/// (hovered, pressed) for the natively hit-tested maximize button.
pub fn max_button_state() -> (bool, bool) {
    (
        MAX_HOVERED.load(Ordering::Relaxed),
        MAX_PRESSED.load(Ordering::Relaxed),
    )
}

pub fn install_from(cc: &eframe::CreationContext<'_>) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let _ = EGUI_CTX.set(cc.egui_ctx.clone());
    let Ok(handle) = cc.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(window) = handle.as_raw() else {
        return;
    };
    install(window.hwnd.get() as _);
}

fn install(hwnd: HWND) {
    unsafe {
        // Keep the window borderless but re-add the sizing frame + caption
        // + system menu flags so DWM provides a full shadow (including the
        // top edge), rounded corners, snap, and Alt+Space. The caption
        // itself is suppressed via WM_NCCALCSIZE below, so no native title
        // is drawn.
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let style =
            style | WS_THICKFRAME | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
        SetWindowLongPtrW(hwnd, GWL_STYLE, style as isize);

        let margins = MARGINS {
            cxLeftWidth: 1,
            cxRightWidth: 1,
            cyTopHeight: 1,
            cyBottomHeight: 1,
        };
        let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);

        // Subclass once; chain to the previous (winit) proc.
        if ORIG_PROC.load(Ordering::Relaxed) == 0 {
            let prev = SetWindowLongPtrW(hwnd, GWL_WNDPROC, chrome_wnd_proc as *const () as isize);
            if prev != 0 {
                ORIG_PROC.store(prev, Ordering::Relaxed);
            }
        }

        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
        );
    }
}

fn set_max_state(hovered: bool, pressed: bool) {
    let hover_changed = MAX_HOVERED.swap(hovered, Ordering::Relaxed) != hovered;
    let press_changed = MAX_PRESSED.swap(pressed, Ordering::Relaxed) != pressed;
    if hover_changed || press_changed {
        if let Some(ctx) = EGUI_CTX.get() {
            ctx.request_repaint();
        }
    }
}

unsafe fn hit_test(hwnd: HWND, lparam: LPARAM) -> Option<u32> {
    let mut pt = POINT {
        x: (lparam & 0xFFFF) as i16 as i32,
        y: ((lparam >> 16) & 0xFFFF) as i16 as i32,
    };
    let mut rc = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // Client coordinates, so the maths matches what egui draws even when
    // maximized (the window rect then extends past the client rect).
    if ScreenToClient(hwnd, &mut pt) == 0 || GetClientRect(hwnd, &mut rc) == 0 {
        return None;
    }
    let (w, h) = (rc.right, rc.bottom);
    let caption_h = CAPTION_H_PX.load(Ordering::Relaxed) as i32;
    let btn_w = BTN_W_PX.load(Ordering::Relaxed) as i32;

    if IsZoomed(hwnd) == 0 {
        // Native resize borders (physical px).
        let border = GetSystemMetrics(SM_CXSIZEFRAME).max(3);
        let left = pt.x < border;
        let right = pt.x >= w - border;
        let top = pt.y < border;
        let bottom = pt.y >= h - border;
        let hit = match (top, bottom, left, right) {
            (true, _, true, _) => HTTOPLEFT,
            (true, _, _, true) => HTTOPRIGHT,
            (_, true, true, _) => HTBOTTOMLEFT,
            (_, true, _, true) => HTBOTTOMRIGHT,
            (true, _, _, _) => HTTOP,
            (_, true, _, _) => HTBOTTOM,
            (_, _, true, _) => HTLEFT,
            (_, _, _, true) => HTRIGHT,
            _ => HTCLIENT,
        };
        if hit != HTCLIENT {
            return Some(hit);
        }
    }

    let in_strip = pt.y >= 0 && pt.y < caption_h && pt.x >= 0 && pt.x < w;
    if in_strip && pt.x >= w - 2 * btn_w && pt.x < w - btn_w {
        // Native maximize button: enables the Snap Layouts flyout.
        return Some(HTMAXBUTTON);
    }
    if in_strip {
        // Everything else up here is egui-drawn and needs client mouse
        // messages. Drag falls back to StartDrag (an OS drag loop, so
        // snapping still works).
        return Some(HTCLIENT);
    }
    None
}

unsafe extern "system" fn chrome_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // Suppress the native caption (we paint our own) while keeping
        // the DWM shadow and rounded corners. A maximized window hangs
        // past the monitor edge by the frame thickness; clamp the client
        // area to the work area so nothing (caption buttons) is clipped.
        WM_NCCALCSIZE => {
            if wparam != 0 && IsZoomed(hwnd) != 0 {
                let params = &mut *(lparam as *mut NCCALCSIZE_PARAMS);
                let monitor = MonitorFromRect(&params.rgrc[0], MONITOR_DEFAULTTONEAREST);
                let mut info: MONITORINFO = std::mem::zeroed();
                info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
                if GetMonitorInfoW(monitor, &mut info) != 0 {
                    params.rgrc[0] = info.rcWork;
                }
            }
            return 0;
        }
        WM_NCHITTEST => {
            if let Some(hit) = hit_test(hwnd, lparam) {
                return hit as LRESULT;
            }
        }
        WM_NCMOUSEMOVE => {
            if wparam as u32 == HTMAXBUTTON {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE | TME_NONCLIENT,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut tme);
                set_max_state(true, MAX_PRESSED.load(Ordering::Relaxed));
                return 0;
            }
            set_max_state(false, false);
        }
        WM_NCMOUSELEAVE => set_max_state(false, false),
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if wparam as u32 == HTMAXBUTTON => {
            set_max_state(true, true);
            return 0;
        }
        WM_NCLBUTTONUP if wparam as u32 == HTMAXBUTTON => {
            if MAX_PRESSED.load(Ordering::Relaxed) {
                set_max_state(true, false);
                let cmd = if IsZoomed(hwnd) != 0 {
                    SC_RESTORE
                } else {
                    SC_MAXIMIZE
                };
                PostMessageW(hwnd, WM_SYSCOMMAND, cmd as WPARAM, 0);
            }
            return 0;
        }
        _ => {}
    }

    let prev = ORIG_PROC.load(Ordering::Relaxed);
    if prev != 0 {
        // SAFETY: stored from SetWindowLongPtrW; winit's proc outlives us.
        let prev: WNDPROC = unsafe { std::mem::transmute::<isize, WNDPROC>(prev) };
        CallWindowProcW(prev, hwnd, msg, wparam, lparam)
    } else {
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}
