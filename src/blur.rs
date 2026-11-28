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

/// True while any mouse button is held down.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // emitted by the objc crate's macros
pub fn mouse_down() -> bool {
    use objc::{class, msg_send, sel, sel_impl};
    // SAFETY: a class method that only reads the current mouse state.
    let buttons: usize = unsafe { msg_send![class!(NSEvent), pressedMouseButtons] };
    buttons != 0
}

#[cfg(not(target_os = "macos"))]
pub fn mouse_down() -> bool {
    false
}

/// Hides or shows a window without closing it (macOS `orderOut:` / `makeKeyAndOrderFront:`).
/// Closing the main window with ⌘W or the red button hides it, so Nimbra keeps checking for
/// mail in the background, as Mail and other macOS mail apps do.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // emitted by the objc crate's macros
pub fn set_visible(window: &gpui::Window, visible: bool) {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    // SAFETY: main thread; `ns_view` is this window's live content view.
    unsafe {
        let view = handle.ns_view.as_ptr() as *mut Object;
        let ns_window: *mut Object = msg_send![view, window];
        if ns_window.is_null() {
            return;
        }
        let nil: *mut Object = std::ptr::null_mut();
        if visible {
            let _: () = msg_send![ns_window, makeKeyAndOrderFront: nil];
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![app, activateIgnoringOtherApps: true];
        } else {
            let _: () = msg_send![ns_window, orderOut: nil];
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set_visible(_window: &gpui::Window, _visible: bool) {}

/// Sets a window's opacity (0 to 1) as a whole. A window at 0 is still drawn and laid out,
/// as one on screen is, but not seen: a message window waits so until its emails are drawn
/// (`message_window`), and then shows whole.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // emitted by the objc crate's macros
pub fn set_alpha(window: &gpui::Window, alpha: f64) {
    use objc::runtime::Object;
    use objc::{msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    // SAFETY: main thread; `ns_view` is this window's live content view.
    unsafe {
        let view = handle.ns_view.as_ptr() as *mut Object;
        let ns_window: *mut Object = msg_send![view, window];
        if !ns_window.is_null() {
            let _: () = msg_send![ns_window, setAlphaValue: alpha];
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set_alpha(_window: &gpui::Window, _alpha: f64) {}

/// Screen geometry for the tooltips' windows, in AppKit's own coordinates (from the bottom
/// left, across all displays).
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
mod screen {
    use objc::runtime::Object;
    use objc::{Encode, Encoding, class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: f64,
        y: f64,
    }
    unsafe impl Encode for Point {
        fn encode() -> Encoding {
            // SAFETY: the encoding of NSPoint on 64-bit macOS.
            unsafe { Encoding::from_str("{CGPoint=dd}") }
        }
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Frame {
        origin: Point,
        size: Point,
    }

    fn ns_window(window: &gpui::Window) -> Option<*mut Object> {
        let handle = HasWindowHandle::window_handle(window).ok()?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return None;
        };
        // SAFETY: `ns_view` is the live content view of this window, on the main thread.
        let found: *mut Object =
            unsafe { msg_send![handle.ns_view.as_ptr() as *mut Object, window] };
        (!found.is_null()).then_some(found)
    }

    /// A point of the window (from its top left) on the screen.
    /// A point in `window` (design points, see `ui::zoom`) on the screen.
    pub fn screen_point(window: &gpui::Window, x: f32, y: f32) -> Option<(f64, f64)> {
        let ns_window = ns_window(window)?;
        let zoom = crate::ui::zoom();
        // SAFETY: a getter on a live NSWindow, main thread.
        let frame: Frame = unsafe { msg_send![ns_window, frame] };
        Some((
            frame.origin.x + f64::from(x * zoom),
            frame.origin.y + frame.size.y - f64::from(y * zoom),
        ))
    }

    /// Puts the window's top left corner there.
    pub fn move_to(window: &gpui::Window, (x, y): (f64, f64)) {
        if let Some(ns_window) = ns_window(window) {
            // SAFETY: a live NSWindow, main thread.
            unsafe {
                let _: () = msg_send![ns_window, setFrameTopLeftPoint: Point { x, y }];
            }
        }
    }

    /// The window takes no mouse at all: clicks and hovers reach what is under it.
    pub fn click_through(window: &gpui::Window) {
        if let Some(ns_window) = ns_window(window) {
            // SAFETY: a live NSWindow, main thread.
            unsafe {
                let _: () = msg_send![ns_window, setIgnoresMouseEvents: true];
            }
        }
    }

    pub fn screen_pointer() -> (f64, f64) {
        // SAFETY: a class method that only reads the pointer's position.
        let at: Point = unsafe { msg_send![class!(NSEvent), mouseLocation] };
        (at.x, at.y)
    }
}

#[cfg(not(target_os = "macos"))]
mod screen {
    pub fn screen_point(_window: &gpui::Window, _x: f32, _y: f32) -> Option<(f64, f64)> {
        None
    }
    pub fn move_to(_window: &gpui::Window, _to: (f64, f64)) {}
    pub fn click_through(_window: &gpui::Window) {}
    pub fn screen_pointer() -> (f64, f64) {
        (0., 0.)
    }
}

pub use screen::{click_through, move_to, screen_point, screen_pointer};
