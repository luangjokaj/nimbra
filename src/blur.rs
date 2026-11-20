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
