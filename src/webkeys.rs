//! Arrow, Home, End and Page keys for the message editor, a web view.
//!
//! GPUI takes those keys as key equivalents before the web view sees them, and hands them
//! to its own text input context, as it does for any key it does not count as a function
//! key. The editor's cursor then never moves, and with the app in front the input system
//! can type the key's private character (U+F700 to U+F8FF) into the message as a box. So
//! while the editor has the keyboard, these keys go straight to it. (An email shown in its
//! original formatting keeps them for the app: there they move between threads.)

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // emitted by the objc crate's macros
mod platform {
    use block::ConcreteBlock;
    use objc::runtime::{BOOL, NO, Object};
    use objc::{class, msg_send, sel, sel_impl};

    /// NSEventMaskKeyDown.
    const KEY_DOWN_MASK: u64 = 1 << 10;
    /// NSEventModifierFlagControl: ⌃ with an arrow belongs to the system (Spaces).
    const CONTROL: u64 = 1 << 18;
    /// What the editor's web view is called (its NSView identifier).
    const EDITOR: &str = "nimbra-editor";

    /// Marks `view` (an NSView) as the message editor.
    ///
    /// # Safety
    /// Main thread, with a live view.
    pub unsafe fn mark_editor(view: *mut Object) {
        unsafe {
            let name = std::ffi::CString::new(EDITOR).unwrap();
            let name: *mut Object =
                msg_send![class!(NSString), stringWithUTF8String: name.as_ptr()];
            let _: () = msg_send![view, setIdentifier: name];
        }
    }

    /// The keys AppKit gives private characters: arrows, Home, End, Page Up and Down,
    /// forward Delete, function keys.
    pub fn is_function_key(first: u16) -> bool {
        (0xF700..=0xF8FF).contains(&first)
    }

    pub fn install() {
        let handler = ConcreteBlock::new(|event: *mut Object| -> *mut Object {
            // SAFETY: AppKit calls this on the main thread with a live key-down event.
            unsafe { route(event) }
        })
        .copy();
        // SAFETY: main thread; the monitor and its block live as long as the app.
        unsafe {
            let monitor: *mut Object = msg_send![
                class!(NSEvent),
                addLocalMonitorForEventsMatchingMask: KEY_DOWN_MASK
                handler: &*handler
            ];
            let _: *mut Object = msg_send![monitor, retain];
        }
        std::mem::forget(handler);
    }

    /// The event, to go on as usual, or nothing once a web view has had it.
    unsafe fn route(event: *mut Object) -> *mut Object {
        unsafe {
            let flags: u64 = msg_send![event, modifierFlags];
            if flags & CONTROL != 0 {
                return event;
            }
            let chars: *mut Object = msg_send![event, charactersIgnoringModifiers];
            let length: usize = msg_send![chars, length];
            if length == 0 {
                return event;
            }
            let first: u16 = msg_send![chars, characterAtIndex: 0usize];
            if !is_function_key(first) {
                return event;
            }
            let window: *mut Object = msg_send![event, window];
            if window.is_null() {
                return event;
            }
            let responder: *mut Object = msg_send![window, firstResponder];
            if responder.is_null() || !inside_editor(responder) {
                return event;
            }
            let _: () = msg_send![responder, keyDown: event];
            std::ptr::null_mut()
        }
    }

    /// The responder is the editor's web view, or a view inside it.
    unsafe fn inside_editor(responder: *mut Object) -> bool {
        unsafe {
            let is_view: BOOL = msg_send![responder, isKindOfClass: class!(NSView)];
            if is_view == NO {
                return false;
            }
            let mut view = responder;
            while !view.is_null() {
                let name: *mut Object = msg_send![view, identifier];
                if !name.is_null() {
                    let name: *const std::os::raw::c_char = msg_send![name, UTF8String];
                    if !name.is_null()
                        && std::ffi::CStr::from_ptr(name).to_bytes() == EDITOR.as_bytes()
                    {
                        return true;
                    }
                }
                view = msg_send![view, superview];
            }
            false
        }
    }
}

#[cfg(target_os = "macos")]
pub use platform::{install, mark_editor};

#[cfg(not(target_os = "macos"))]
pub fn install() {}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn only_appkit_function_keys_are_taken() {
        use super::platform::is_function_key;
        for arrow in [0xF700, 0xF701, 0xF702, 0xF703] {
            assert!(is_function_key(arrow));
        }
        assert!(is_function_key(0xF729)); // Home
        assert!(is_function_key(0xF72B)); // End
        assert!(!is_function_key('a' as u16));
        assert!(!is_function_key(' ' as u16));
        assert!(!is_function_key(0x1B)); // Escape: the page reports it
        assert!(!is_function_key(0x0D));
    }
}
