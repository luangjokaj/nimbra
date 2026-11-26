//! A toast belongs to the window it is about. On the Mac it is an AppKit child window of that
//! window: it moves and orders with it, stays on its Space and under other apps' windows, and
//! is hidden and shown again rather than opened each time (a new window compiles its shaders).
//! Elsewhere a toast is a window of its own, opened and closed each time.

/// Whether a toast's window is kept, hidden, for the next toast.
pub const REUSED: bool = cfg!(target_os = "macos");

/// Where a window of `size` goes over the bottom centre of `parent` (x, y, width, height,
/// from the bottom left as AppKit counts), its bottom edge `bottom` points above the parent's.
#[cfg(any(target_os = "macos", test))]
fn centred(parent: (f64, f64, f64, f64), size: (f64, f64), bottom: f64) -> (f64, f64) {
    let (x, y, width, _) = parent;
    ((x + (width - size.0) / 2.).round(), y + bottom)
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // emitted by the objc crate's macros
mod imp {
    use objc::runtime::{BOOL, NO, Object, YES};
    use objc::{Encode, Encoding, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Rect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    }
    unsafe impl Encode for Rect {
        fn encode() -> Encoding {
            // SAFETY: the encoding of NSRect on 64-bit macOS.
            unsafe { Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
        }
    }

    /// `NSWindowOrderingMode.above`.
    const ABOVE: isize = 1;
    /// `NSWindowCollectionBehavior`: one Space, its full-screen one included, and out of ⌘`.
    const MANAGED: usize = 1 << 2;
    const IGNORES_CYCLE: usize = 1 << 6;
    const FULL_SCREEN_AUXILIARY: usize = 1 << 8;

    /// A window's NSWindow, taken while GPUI lends the window out and used before anything
    /// else runs on the main thread.
    #[derive(Clone, Copy)]
    pub struct Native(*mut Object);

    pub fn native(window: &gpui::Window) -> Option<Native> {
        let handle = HasWindowHandle::window_handle(window).ok()?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return None;
        };
        // SAFETY: `ns_view` is the live content view of this window, on the main thread.
        let found: *mut Object =
            unsafe { msg_send![handle.ns_view.as_ptr() as *mut Object, window] };
        (!found.is_null()).then_some(Native(found))
    }

    /// On screen, and not in the Dock.
    pub fn showing(window: &gpui::Window) -> bool {
        native(window).is_some_and(|window| {
            // SAFETY: getters on a live NSWindow, main thread.
            unsafe {
                let visible: BOOL = msg_send![window.0, isVisible];
                let minimized: BOOL = msg_send![window.0, isMiniaturized];
                visible == YES && minimized == NO
            }
        })
    }

    /// Turns GPUI's pop-up panel (above every app, on every Space) into one that lives in a
    /// window and never takes the keyboard from it.
    pub fn prepare(window: &gpui::Window) {
        let Some(window) = native(window) else {
            return;
        };
        // SAFETY: setters on a live NSPanel (GPUI's pop-up class), main thread.
        unsafe {
            let _: () = msg_send![window.0, setLevel: 0isize];
            let _: () = msg_send![
                window.0,
                setCollectionBehavior: MANAGED | IGNORES_CYCLE | FULL_SCREEN_AUXILIARY
            ];
            let _: () = msg_send![window.0, setHidesOnDeactivate: NO];
            let _: () = msg_send![window.0, setBecomesKeyOnlyIfNeeded: YES];
        }
    }

    /// Sets `child`'s frame over `parent` (see `centred`) and, with `order`, makes it a child
    /// of `parent`, at its level, just above it.
    ///
    /// # Safety
    /// Both are live windows, and nothing has run on the main thread since they were taken.
    /// Not inside a GPUI update: the frame change calls back into GPUI at once.
    pub unsafe fn put(child: Native, parent: Native, size: (f64, f64), bottom: f64, order: bool) {
        unsafe {
            let frame: Rect = msg_send![parent.0, frame];
            let (x, y) =
                super::centred((frame.x, frame.y, frame.width, frame.height), size, bottom);
            let to = Rect {
                x,
                y,
                width: size.0,
                height: size.1,
            };
            let _: () = msg_send![child.0, setFrame: to display: YES];
            if !order {
                return;
            }
            let attached: *mut Object = msg_send![child.0, parentWindow];
            if attached != parent.0 {
                if !attached.is_null() {
                    let _: () = msg_send![attached, removeChildWindow: child.0];
                }
                let level: isize = msg_send![parent.0, level];
                let _: () = msg_send![child.0, setLevel: level];
                let _: () = msg_send![parent.0, addChildWindow: child.0 ordered: ABOVE];
            }
            let number: isize = msg_send![parent.0, windowNumber];
            let _: () = msg_send![child.0, orderWindow: ABOVE relativeTo: number];
        }
    }

    /// Off the screen, and no longer its parent's child.
    pub fn hide(window: &gpui::Window) {
        if let Some(window) = native(window) {
            let nil: *mut Object = std::ptr::null_mut();
            // SAFETY: a live NSWindow, main thread. `orderOut:` removes a child window from
            // its parent first.
            unsafe {
                let _: () = msg_send![window.0, orderOut: nil];
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    #[derive(Clone, Copy)]
    pub struct Native;

    pub fn native(_window: &gpui::Window) -> Option<Native> {
        None
    }

    pub fn showing(_window: &gpui::Window) -> bool {
        true
    }

    pub fn prepare(_window: &gpui::Window) {}

    /// # Safety
    /// Nothing to uphold here; the Mac's does.
    pub unsafe fn put(
        _child: Native,
        _parent: Native,
        _size: (f64, f64),
        _bottom: f64,
        _order: bool,
    ) {
    }

    pub fn hide(_window: &gpui::Window) {}
}

pub use imp::{hide, native, prepare, put, showing};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_toast_sits_centred_over_its_parents_bottom_edge() {
        let parent = (100., 50., 1200., 800.);
        assert_eq!(centred(parent, (400., 100.), -4.), (500., 46.));
    }
}
