//! The small pop-up windows (menus, the Undo and notice pills, the update's card): how they
//! come and go, and keeping the app's own notices with its window.
//!
//! A pop-up is a window of its own, and AppKit animates a window's alpha smoothly: one
//! fades in, and fades out before it is closed (`remove_popup`). A notice also rises as it
//! comes and sinks as it goes, by moving its picture in Core Animation, never the window:
//! moving a window frame by frame does not keep pace with what is drawn in it, and
//! jitters. The notices at the bottom of the main window are its child
//! windows at its level, so they move with it and stay behind other apps' windows when
//! Nimbra is not in front, rather than floating over everything as pop-ups do.

/// How long a pop-up takes to come or go (seconds).
pub const FADE: f64 = 0.2;

/// How far a notice at the bottom of the window rises as it comes in, and sinks as it goes
/// (points). Within the margin its window keeps around it, so it is never cut off.
const RISE: f64 = 10.;

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // emitted by the objc crate's macros
mod platform {
    use std::cell::Cell;

    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    thread_local! {
        /// The main window's NSWindow, as last drawn (`note_main`).
        static MAIN: Cell<usize> = const { Cell::new(0) };
    }

    fn ns_window(window: &gpui::Window) -> Option<*mut Object> {
        let handle = HasWindowHandle::window_handle(window).ok()?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return None;
        };
        let view = handle.ns_view.as_ptr() as *mut Object;
        // SAFETY: the live content view of this window, on the main thread.
        let ns_window: *mut Object = unsafe { msg_send![view, window] };
        (!ns_window.is_null()).then_some(ns_window)
    }

    pub fn note_main(window: &gpui::Window) {
        if let Some(ns_window) = ns_window(window) {
            MAIN.with(|main| main.set(ns_window as usize));
        }
    }

    /// Runs `animate` (on the window's animator proxy) over `FADE`, easing.
    ///
    /// # Safety
    /// Main thread only, with a live NSWindow.
    unsafe fn animated(
        ns_window: *mut Object,
        ease: &std::ffi::CStr,
        animate: impl FnOnce(*mut Object),
    ) {
        unsafe {
            let _: () = msg_send![class!(NSAnimationContext), beginGrouping];
            let context: *mut Object = msg_send![class!(NSAnimationContext), currentContext];
            let _: () = msg_send![context, setDuration: super::FADE];
            let name: *mut Object =
                msg_send![class!(NSString), stringWithUTF8String: ease.as_ptr()];
            let timing: *mut Object =
                msg_send![class!(CAMediaTimingFunction), functionWithName: name];
            if !timing.is_null() {
                let _: () = msg_send![context, setTimingFunction: timing];
            }
            let animator: *mut Object = msg_send![ns_window, animator];
            animate(animator);
            let _: () = msg_send![class!(NSAnimationContext), endGrouping];
        }
    }

    /// Moves what `ns_window` draws from `from` points above where it is to `to` (negative:
    /// below), easing, in Core Animation: only the picture moves, not the window or its
    /// layout, so it keeps pace. With `hold`, it stays at `to` (the window is on its way out).
    ///
    /// # Safety
    /// Main thread only, with a live NSWindow.
    unsafe fn slide(ns_window: *mut Object, from: f64, to: f64, ease: &std::ffi::CStr, hold: bool) {
        unsafe {
            let view: *mut Object = msg_send![ns_window, contentView];
            if view.is_null() {
                return;
            }
            let layer: *mut Object = msg_send![view, layer];
            if layer.is_null() {
                return;
            }
            let path: *mut Object = msg_send![class!(NSString),
                stringWithUTF8String: c"transform.translation.y".as_ptr()];
            let animation: *mut Object =
                msg_send![class!(CABasicAnimation), animationWithKeyPath: path];
            let from: *mut Object = msg_send![class!(NSNumber), numberWithDouble: from];
            let to: *mut Object = msg_send![class!(NSNumber), numberWithDouble: to];
            let _: () = msg_send![animation, setFromValue: from];
            let _: () = msg_send![animation, setToValue: to];
            let _: () = msg_send![animation, setDuration: super::FADE];
            let name: *mut Object =
                msg_send![class!(NSString), stringWithUTF8String: ease.as_ptr()];
            let timing: *mut Object =
                msg_send![class!(CAMediaTimingFunction), functionWithName: name];
            if !timing.is_null() {
                let _: () = msg_send![animation, setTimingFunction: timing];
            }
            if hold {
                let forwards: *mut Object = msg_send![class!(NSString),
                    stringWithUTF8String: c"forwards".as_ptr()];
                let _: () = msg_send![animation, setFillMode: forwards];
                let _: () = msg_send![animation, setRemovedOnCompletion: false];
            }
            let key: *mut Object =
                msg_send![class!(NSString), stringWithUTF8String: c"nimbra.slide".as_ptr()];
            let _: () = msg_send![layer, addAnimation: animation forKey: key];
        }
    }

    pub fn enter(window: &gpui::Window, attach: bool) {
        let Some(ns_window) = ns_window(window) else {
            return;
        };
        // SAFETY: main thread, a live NSWindow (and the main window's, while it is noted).
        unsafe {
            if attach {
                let main = MAIN.with(Cell::get) as *mut Object;
                if !main.is_null() && main != ns_window {
                    let level: isize = msg_send![main, level];
                    let _: () = msg_send![ns_window, setLevel: level];
                    // NSWindowAbove = 1.
                    let _: () = msg_send![main, addChildWindow: ns_window ordered: 1isize];
                }
            }
            let _: () = msg_send![ns_window, setAlphaValue: 0f64];
        }
        // Its layer is made as it is first drawn: both start then, together.
        window.on_next_frame(move |window, _| {
            let Some(ns_window) = self::ns_window(window) else {
                return;
            };
            // SAFETY: main thread, a live NSWindow.
            unsafe {
                animated(ns_window, c"easeOut", |animator| {
                    let _: () = msg_send![animator, setAlphaValue: 1f64];
                });
                // A notice rises into place from just below it.
                if attach {
                    slide(ns_window, -super::RISE, 0., c"easeOut", false);
                }
            }
        });
    }

    pub fn leave(window: &gpui::Window) {
        let Some(ns_window) = ns_window(window) else {
            return;
        };
        // SAFETY: main thread, a live NSWindow.
        unsafe {
            animated(ns_window, c"easeIn", |animator| {
                let _: () = msg_send![animator, setAlphaValue: 0f64];
            });
            // A notice (a child of the main window, `enter`) sinks back the way it came.
            let parent: *mut Object = msg_send![ns_window, parentWindow];
            if !parent.is_null() {
                slide(ns_window, 0., -super::RISE, c"easeIn", true);
            }
        }
    }

    /// The display number (CGDirectDisplayID) of the screen holding `at` (AppKit's
    /// global coordinates).
    pub fn screen_number_at(at: (f64, f64)) -> Option<u32> {
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Rect {
            x: f64,
            y: f64,
            width: f64,
            height: f64,
        }
        unsafe impl objc::Encode for Rect {
            fn encode() -> objc::Encoding {
                // SAFETY: the encoding of CGRect / NSRect on 64-bit macOS.
                unsafe { objc::Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
            }
        }
        // SAFETY: class methods and getters on live AppKit objects, on the main thread.
        unsafe {
            let screens: *mut Object = msg_send![class!(NSScreen), screens];
            let count: usize = msg_send![screens, count];
            for i in 0..count {
                let screen: *mut Object = msg_send![screens, objectAtIndex: i];
                let frame: Rect = msg_send![screen, frame];
                let inside = at.0 >= frame.x
                    && at.0 < frame.x + frame.width
                    && at.1 >= frame.y
                    && at.1 < frame.y + frame.height;
                if !inside {
                    continue;
                }
                let description: *mut Object = msg_send![screen, deviceDescription];
                let key: *mut Object = msg_send![class!(NSString),
                    stringWithUTF8String: c"NSScreenNumber".as_ptr()];
                let number: *mut Object = msg_send![description, objectForKey: key];
                if number.is_null() {
                    return None;
                }
                let number: u32 = msg_send![number, unsignedIntValue];
                return Some(number);
            }
            None
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    pub fn note_main(_window: &gpui::Window) {}
    pub fn enter(_window: &gpui::Window, _attach: bool) {}
    pub fn leave(_window: &gpui::Window) {}
    pub fn screen_number_at(_at: (f64, f64)) -> Option<u32> {
        None
    }
}

/// Notes the main window, which the notices are attached to (`enter`). Called as it draws.
pub fn note_main(window: &gpui::Window) {
    platform::note_main(window);
}

/// A pop-up window coming in: it fades in. With `attach`, it is one of the main window's
/// notices: it becomes its child window, at its level, and rises into place as it fades.
pub fn enter(window: &gpui::Window, attach: bool) {
    platform::enter(window, attach);
}

/// A pop-up window going: it fades out, and a notice sinks as it does (closed `FADE` later,
/// by `menus::remove_popup`).
pub fn leave(window: &gpui::Window) {
    platform::leave(window);
}

/// The display `window` is on. A pop-up placed by a window's bounds opens on that display:
/// GPUI places a new window relative to the display it is given (the primary one if none),
/// as it gives a window's bounds relative to its own.
pub fn display_of(window: &gpui::Window, cx: &gpui::App) -> Option<gpui::DisplayId> {
    window.display(cx).map(|display| display.id())
}

/// The display a point on the screen is on, given as AppKit gives it (from the bottom of
/// the main screen). A pop-up moved to a point (`blur::move_to`) opens on that display
/// first, so it is drawn at that display's scale. Asked of AppKit: GPUI's displays all
/// say they start at 0, 0, so their bounds cannot tell.
pub fn display_at(at: (f64, f64), cx: &gpui::App) -> Option<gpui::DisplayId> {
    let number = platform::screen_number_at(at)?;
    cx.displays()
        .into_iter()
        .map(|display| display.id())
        .find(|id| u32::from(*id) == number)
}
