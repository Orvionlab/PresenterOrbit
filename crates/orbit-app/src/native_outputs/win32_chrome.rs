use windows::Win32::{
    Foundation::{HWND, POINT, RECT},
    Graphics::Gdi::ClientToScreen,
    UI::{
        HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow},
        WindowsAndMessaging::*,
    },
};

#[cfg(not(test))]
pub(super) const TIMER_ID: usize = 0x4f52;

pub(super) fn refresh(hwnd: HWND) {
    if let Err(error) = unsafe { update(hwnd) } {
        eprintln!("[output-window-chrome] {error}");
    }
}

unsafe fn update(hwnd: HWND) -> windows::core::Result<()> {
    let active = GetForegroundWindow() == hwnd && !IsIconic(hwnd).as_bool();
    let mut hovered = false;
    if active {
        let mut cursor = POINT::default();
        let mut bounds = RECT::default();
        GetCursorPos(&mut cursor)?;
        GetWindowRect(hwnd, &mut bounds)?;
        hovered = cursor.x >= bounds.left
            && cursor.x < bounds.right
            && cursor.y >= bounds.top
            && cursor.y < bounds.bottom;
    }
    let visible = super::window_controls_visible(active, hovered);
    set_caption(hwnd, visible)
}

unsafe fn set_caption(hwnd: HWND, visible: bool) -> windows::core::Result<()> {
    let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
    if (style & WS_CAPTION.0 == WS_CAPTION.0) == visible {
        return Ok(());
    }
    let new_style = WINDOW_STYLE(if visible {
        style | WS_CAPTION.0
    } else {
        style & !WS_CAPTION.0
    });
    // Keep the rendered client area fixed when the native caption appears or disappears.
    let mut client = RECT::default();
    GetClientRect(hwnd, &mut client)?;
    let mut origin = POINT::default();
    if !ClientToScreen(hwnd, &mut origin).as_bool() {
        return Err(windows::core::Error::from_win32());
    }
    let mut outer = RECT {
        left: origin.x,
        top: origin.y,
        right: origin.x + client.right,
        bottom: origin.y + client.bottom,
    };
    AdjustWindowRectExForDpi(
        &mut outer,
        new_style,
        false,
        WINDOW_EX_STYLE(GetWindowLongW(hwnd, GWL_EXSTYLE) as u32),
        GetDpiForWindow(hwnd),
    )?;
    if SetWindowLongW(hwnd, GWL_STYLE, new_style.0 as i32) == 0 {
        return Err(windows::core::Error::from_win32());
    }
    let result = if IsZoomed(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
        )
    } else {
        SetWindowPos(
            hwnd,
            None,
            outer.left,
            outer.top,
            outer.right - outer.left,
            outer.bottom - outer.top,
            SWP_FRAMECHANGED | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
    if result.is_err() {
        SetWindowLongW(hwnd, GWL_STYLE, style as i32);
    }
    result
}
