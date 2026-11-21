//! Backdrop blur for the translucent sidebar.
//!
//! GPUI's `WindowBackgroundAppearance::Blurred` makes the window transparent, but on
//! current macOS its effect view no longer blurs what is behind the window. This asks the
//! window server for the blur directly, the same call GPUI itself uses on older macOS.
//! It is a private (but long-stable) API, so treat it as a prototype-only measure.

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // emitted by the objc crate's macros
pub fn apply(window: &gpui::Window, radius: i64) {
    use objc::runtime::Object;
    use objc::{msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGSMainConnectionID() -> *mut Object;
        fn CGSSetWindowBackgroundBlurRadius(
            connection: *mut Object,
            window_number: isize,
            radius: i64,
        ) -> i32;
    }

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    // SAFETY: `ns_view` is the live content view of this window, and we are on the main
    // thread (GPUI only hands out `&Window` there).
    unsafe {
        let view = handle.ns_view.as_ptr() as *mut Object;
        let ns_window: *mut Object = msg_send![view, window];
        if ns_window.is_null() {
            return;
        }
        let number: isize = msg_send![ns_window, windowNumber];
        CGSSetWindowBackgroundBlurRadius(CGSMainConnectionID(), number, radius);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn apply(_window: &gpui::Window, _radius: i64) {}

/// Removes the outline and shadow macOS draws around a window. Used for the menu pop-up,
/// which is a transparent window drawing its own panel and shadow: the system's would show
/// as a second, larger rounded frame around it.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // emitted by the objc crate's macros
pub fn no_window_shadow(window: &gpui::Window) {
    use objc::runtime::Object;
    use objc::{msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    // SAFETY: `ns_view` is the live content view of this window, on the main thread.
    unsafe {
        let view = handle.ns_view.as_ptr() as *mut Object;
        let ns_window: *mut Object = msg_send![view, window];
        if !ns_window.is_null() {
            let _: () = msg_send![ns_window, setHasShadow: false];
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn no_window_shadow(_window: &gpui::Window) {}

/// True while a mouse button is held down somewhere outside this window's content, leaving
/// out a `margin` of transparent edge. The menu pop-up polls this to close on any outside
/// click, including clicks that land on the email's native web view or on another app,
/// which never reach the app's own event handling.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // emitted by the objc crate's macros
pub fn mouse_pressed_outside(window: &gpui::Window, margin: f64) -> bool {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Rect {
        origin: Point,
        size: Point,
    }

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return false;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return false;
    };
    // SAFETY: main thread; `ns_view` is this window's live content view; both calls on
    // NSEvent are class methods that only read the current mouse state.
    unsafe {
        let buttons: usize = msg_send![class!(NSEvent), pressedMouseButtons];
        if buttons == 0 {
            return false;
        }
        let view = handle.ns_view.as_ptr() as *mut Object;
        let ns_window: *mut Object = msg_send![view, window];
        if ns_window.is_null() {
            return false;
        }
        let frame: Rect = msg_send![ns_window, frame];
        let mouse: Point = msg_send![class!(NSEvent), mouseLocation];
        let inside = mouse.x >= frame.origin.x + margin
            && mouse.x <= frame.origin.x + frame.size.x - margin
            && mouse.y >= frame.origin.y + margin
            && mouse.y <= frame.origin.y + frame.size.y - margin;
        !inside
    }
}

#[cfg(not(target_os = "macos"))]
pub fn mouse_pressed_outside(_window: &gpui::Window, _margin: f64) -> bool {
    false
}
