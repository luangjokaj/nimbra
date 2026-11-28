//! The desktop picture behind the main window's glass (the sidebar, its open row).
//!
//! The window is transparent where the app draws glass, and what shows there is whatever
//! is under the app's drawing. macOS's own blur (`blur.rs`) blurs what is behind the
//! window: other apps' windows as much as the desktop. Here a native layer with the
//! desktop picture, blurred, goes under the app's drawing inside the window, placed where
//! the picture is on screen: the glass shows the desktop, as if the window were not there,
//! and never another window. macOS moves it with the window itself, so it never lags.

#[cfg(target_os = "macos")]
pub use mac::install;

#[cfg(not(target_os = "macos"))]
pub fn install(_window: &gpui::Window) {}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // emitted by the objc crate's macros
mod mac {
    use std::cell::RefCell;
    use std::ffi::{CStr, CString, c_void};

    use objc::runtime::{Object, YES};
    use objc::{Encode, Encoding, class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Rect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    }
    unsafe impl Encode for Rect {
        fn encode() -> Encoding {
            // SAFETY: the encoding of CGRect / NSRect on 64-bit macOS.
            unsafe { Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
        }
    }

    /// How much the picture is blurred, in points of the picture as shown (it is made
    /// smaller first, so the blur costs little and is as soft).
    const BLUR: f64 = 40.;
    /// The picture is blurred at this share of its size.
    const SCALE: f64 = 0.25;

    unsafe extern "C" {
        static _dispatch_main_q: c_void;
        fn dispatch_async_f(
            queue: *const c_void,
            context: *mut c_void,
            work: extern "C" fn(*mut c_void),
        );
        static NSWindowDidMoveNotification: *mut Object;
        static NSWindowWillMoveNotification: *mut Object;
        static NSWindowDidResizeNotification: *mut Object;
        static NSWindowDidChangeScreenNotification: *mut Object;
        static NSWindowDidBecomeKeyNotification: *mut Object;
        static NSWorkspaceActiveSpaceDidChangeNotification: *mut Object;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGImageRelease(image: *mut c_void);
        fn CGWindowListCopyWindowInfo(option: u32, window: u32) -> *const c_void;
        fn CGRectMakeWithDictionaryRepresentation(dict: *const c_void, rect: *mut Rect) -> bool;
        static kCGWindowBounds: *const c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFArrayGetCount(array: *const c_void) -> isize;
        fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
        fn CFDictionaryGetValue(dict: *const c_void, key: *const c_void) -> *const c_void;
        fn CFRelease(value: *const c_void);
    }

    /// How often the picture follows a window being moved: about every frame.
    const FOLLOW_EVERY: f64 = 1. / 120.;
    /// A window still for this many of those has stopped moving.
    const STILL_FOR: u32 = 30;

    // Core Image (CIImage, CIFilter, CIContext) and Core Animation (CALayer): linked so
    // that their classes are there to look up.
    #[link(name = "CoreImage", kind = "framework")]
    unsafe extern "C" {}
    #[link(name = "QuartzCore", kind = "framework")]
    unsafe extern "C" {}

    struct Wall {
        window: *mut Object,
        /// The layer with the picture, in a view under the app's own.
        picture: *mut Object,
        /// The picture shown (its file), so that a picture is only made once.
        shown: Option<String>,
        /// While the window moves, the timer that keeps the picture where it is on screen
        /// (`follow`), with where the window was at its last tick and for how many it has
        /// not moved.
        following: Option<*mut Object>,
        last: Option<Rect>,
        still: u32,
    }

    thread_local! {
        static WALL: RefCell<Option<Wall>> = const { RefCell::new(None) };
    }

    /// Puts the desktop picture under the app's drawing in `window`, and keeps it where
    /// the picture is on screen and up to date. Once per window (the main one).
    pub fn install(window: &gpui::Window) {
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return;
        };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return;
        };
        // SAFETY: main thread; `ns_view` is this window's live content view. The view
        // made here is owned by the window's frame view, the layer by that view.
        unsafe {
            let content = handle.ns_view.as_ptr() as *mut Object;
            let window: *mut Object = msg_send![content, window];
            let frame_view: *mut Object = msg_send![content, superview];
            if window.is_null() || frame_view.is_null() {
                return;
            }
            let bounds: Rect = msg_send![frame_view, bounds];
            let view: *mut Object = msg_send![class!(NSView), alloc];
            let view: *mut Object = msg_send![view, initWithFrame: bounds];
            if view.is_null() {
                return;
            }
            let _: () = msg_send![view, setWantsLayer: YES];
            // NSViewWidthSizable | NSViewHeightSizable: it stays the window's size.
            let _: () = msg_send![view, setAutoresizingMask: 18usize];
            let layer: *mut Object = msg_send![view, layer];
            let _: () = msg_send![layer, setMasksToBounds: YES];
            let picture: *mut Object = msg_send![class!(CALayer), layer];
            let gravity = ns_string("resizeAspectFill");
            let _: () = msg_send![picture, setContentsGravity: gravity];
            let _: () = msg_send![layer, addSublayer: picture];
            // NSWindowBelow = -1: under the app's view, which draws over it.
            let _: () =
                msg_send![frame_view, addSubview: view positioned: -1isize relativeTo: content];
            let _: () = msg_send![view, release];
            WALL.with(|wall| {
                *wall.borrow_mut() = Some(Wall {
                    window,
                    picture,
                    shown: None,
                    following: None,
                    last: None,
                    still: 0,
                })
            });
            for name in [
                NSWindowDidMoveNotification,
                NSWindowDidResizeNotification,
                NSWindowDidChangeScreenNotification,
                NSWindowDidBecomeKeyNotification,
            ] {
                observe(name, window, false);
            }
            observe(NSWorkspaceActiveSpaceDidChangeNotification, window, true);
            // macOS says where a window is after it has moved, often only once the move is
            // over: while it moves, the picture follows it frame by frame (`follow`).
            for name in [NSWindowWillMoveNotification, NSWindowDidMoveNotification] {
                on(name, window, follow);
            }
        }
        refresh();
    }

    /// Calls `act` on `name` of `window`.
    ///
    /// # Safety
    /// Main thread, with a live window.
    unsafe fn on(name: *mut Object, window: *mut Object, act: fn()) {
        if name.is_null() {
            return;
        }
        let block = block::ConcreteBlock::new(move |_note: *mut Object| act()).copy();
        // SAFETY: as `observe`.
        unsafe {
            let center: *mut Object = msg_send![class!(NSNotificationCenter), defaultCenter];
            let main: *mut Object = msg_send![class!(NSOperationQueue), mainQueue];
            let observer: *mut Object = msg_send![center,
                addObserverForName: name object: window queue: main usingBlock: &*block];
            if !observer.is_null() {
                let _: *mut Object = msg_send![observer, retain];
            }
        }
    }

    /// Starts following the window where it is on screen, about every frame, until it has
    /// been still a moment. Main thread.
    fn follow() {
        WALL.with(|wall| {
            let mut wall = wall.borrow_mut();
            let Some(wall) = wall.as_mut() else {
                return;
            };
            wall.still = 0;
            if wall.following.is_some() {
                return;
            }
            let block = block::ConcreteBlock::new(|_timer: *mut Object| tick()).copy();
            // SAFETY: main thread; the timer copies the block, the run loop keeps the timer
            // until it is invalidated (`tick`), and it is retained here until then.
            unsafe {
                let timer: *mut Object = msg_send![class!(NSTimer),
                    timerWithTimeInterval: FOLLOW_EVERY repeats: YES block: &*block];
                if timer.is_null() {
                    return;
                }
                // Also while the pointer drags the window (an event-tracking run loop mode).
                let run_loop: *mut Object = msg_send![class!(NSRunLoop), mainRunLoop];
                let _: () = msg_send![run_loop,
                    addTimer: timer forMode: ns_string("kCFRunLoopCommonModes")];
                let _: *mut Object = msg_send![timer, retain];
                wall.following = Some(timer);
            }
        });
    }

    /// One step of `follow`: the picture where the window is now, as the window server has
    /// it (AppKit's frame lags while the window is dragged).
    fn tick() {
        WALL.with(|wall| {
            let mut wall = wall.borrow_mut();
            let Some(wall) = wall.as_mut() else {
                return;
            };
            // SAFETY: main thread; the window, the layer and the timer are alive.
            unsafe {
                let frame =
                    live_frame(wall.window).unwrap_or_else(|| msg_send![wall.window, frame]);
                if wall.last == Some(frame) {
                    wall.still += 1;
                } else {
                    wall.still = 0;
                    wall.last = Some(frame);
                    place(wall, frame);
                }
                if wall.still >= STILL_FOR
                    && let Some(timer) = wall.following.take()
                {
                    let _: () = msg_send![timer, invalidate];
                    let _: () = msg_send![timer, release];
                }
            }
        });
    }

    /// Where the window is on screen now, as the window server has it, in AppKit's
    /// coordinates (from the bottom left of the main screen).
    ///
    /// # Safety
    /// Main thread, with a live window.
    unsafe fn live_frame(window: *mut Object) -> Option<Rect> {
        // kCGWindowListOptionIncludingWindow: only this window.
        const ONLY: u32 = 1 << 3;
        unsafe {
            let number: isize = msg_send![window, windowNumber];
            let list = CGWindowListCopyWindowInfo(ONLY, u32::try_from(number).ok()?);
            if list.is_null() {
                return None;
            }
            let mut bounds = Rect {
                x: 0.,
                y: 0.,
                width: 0.,
                height: 0.,
            };
            let found = CFArrayGetCount(list) > 0 && {
                let info = CFArrayGetValueAtIndex(list, 0);
                let dict = CFDictionaryGetValue(info, kCGWindowBounds);
                !dict.is_null() && CGRectMakeWithDictionaryRepresentation(dict, &mut bounds)
            };
            CFRelease(list);
            if !found {
                return None;
            }
            // The window server measures from the main screen's top left.
            let screens: *mut Object = msg_send![class!(NSScreen), screens];
            let main: *mut Object = msg_send![screens, firstObject];
            if main.is_null() {
                return None;
            }
            let main_frame: Rect = msg_send![main, frame];
            Some(Rect {
                y: main_frame.height - bounds.y - bounds.height,
                ..bounds
            })
        }
    }

    /// The picture where the screen's is, seen from a window at `window_frame`.
    ///
    /// # Safety
    /// Main thread.
    unsafe fn place(wall: &Wall, window_frame: Rect) {
        unsafe {
            let screen: *mut Object = msg_send![wall.window, screen];
            if screen.is_null() {
                return;
            }
            let screen_frame: Rect = msg_send![screen, frame];
            let at = Rect {
                x: screen_frame.x - window_frame.x,
                y: screen_frame.y - window_frame.y,
                ..screen_frame
            };
            let _: () = msg_send![class!(CATransaction), begin];
            let _: () = msg_send![class!(CATransaction), setDisableActions: YES];
            let _: () = msg_send![wall.picture, setFrame: at];
            let _: () = msg_send![class!(CATransaction), commit];
        }
    }

    /// Calls `refresh` on `name` (of `window`, or of the workspace).
    ///
    /// # Safety
    /// Main thread, with a live window.
    unsafe fn observe(name: *mut Object, window: *mut Object, workspace: bool) {
        if name.is_null() {
            return;
        }
        let block = block::ConcreteBlock::new(|_note: *mut Object| refresh()).copy();
        // SAFETY: the block runs on the main queue; the center copies it, and the
        // observer it returns is kept for the life of the app, as the window is.
        unsafe {
            let center: *mut Object = if workspace {
                let shared: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
                msg_send![shared, notificationCenter]
            } else {
                msg_send![class!(NSNotificationCenter), defaultCenter]
            };
            let main: *mut Object = msg_send![class!(NSOperationQueue), mainQueue];
            if center.is_null() || main.is_null() {
                return;
            }
            let object: *mut Object = if workspace {
                std::ptr::null_mut()
            } else {
                window
            };
            let observer: *mut Object = msg_send![center,
                addObserverForName: name object: object queue: main usingBlock: &*block];
            if !observer.is_null() {
                let _: *mut Object = msg_send![observer, retain];
            }
        }
    }

    /// Places the picture where it is on the window's screen, and makes it anew when the
    /// screen's desktop picture is another one. Main thread.
    fn refresh() {
        WALL.with(|wall| {
            let mut wall = wall.borrow_mut();
            let Some(wall) = wall.as_mut() else {
                return;
            };
            // SAFETY: main thread; the window and the layer live as long as the app.
            unsafe {
                let screen: *mut Object = msg_send![wall.window, screen];
                if screen.is_null() {
                    return;
                }
                let window_frame =
                    live_frame(wall.window).unwrap_or_else(|| msg_send![wall.window, frame]);
                place(wall, window_frame);

                let workspace: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
                let url: *mut Object = msg_send![workspace, desktopImageURLForScreen: screen];
                let Some(path) = string_of(msg_send![url, path]) else {
                    return;
                };
                if wall.shown.as_deref() == Some(path.as_str()) {
                    return;
                }
                wall.shown = Some(path.clone());
                std::thread::spawn(move || blurred(&path));
            }
        });
    }

    /// The picture in `path`, smaller and blurred, handed to the main thread to show.
    /// Off the main thread: Core Image is safe to use on any thread.
    fn blurred(path: &str) {
        // SAFETY: Core Image and Foundation objects made, used and released on this
        // thread, inside its own autorelease pool; the image handed over is retained for
        // the main thread, which releases it.
        unsafe {
            let pool: *mut Object = msg_send![class!(NSAutoreleasePool), new];
            let image = make(path);
            let _: () = msg_send![pool, drain];
            if let Some(image) = image {
                dispatch_async_f(
                    &raw const _dispatch_main_q,
                    image,
                    show as extern "C" fn(*mut c_void),
                );
            }
        }
    }

    /// The blurred picture as a CGImage the caller owns, or `None`.
    ///
    /// # Safety
    /// Inside an autorelease pool.
    unsafe fn make(path: &str) -> Option<*mut c_void> {
        unsafe {
            let url: *mut Object = msg_send![class!(NSURL), fileURLWithPath: ns_string(path)];
            let image: *mut Object = msg_send![class!(CIImage), imageWithContentsOfURL: url];
            if image.is_null() {
                return None;
            }
            let number = |value: f64| -> *mut Object {
                msg_send![class!(NSNumber), numberWithDouble: value]
            };
            // Smaller first: the blur hides the detail, and costs a sixteenth.
            let scale = filter("CILanczosScaleTransform");
            let _: () = msg_send![scale, setValue: image forKey: ns_string("inputImage")];
            let _: () = msg_send![scale, setValue: number(SCALE) forKey: ns_string("inputScale")];
            let small: *mut Object = msg_send![scale, outputImage];
            if small.is_null() {
                return None;
            }
            let extent: Rect = msg_send![small, extent];
            // Clamped, so the edges are not blurred into transparency, then cut back.
            let clamped: *mut Object = msg_send![small, imageByClampingToExtent];
            let blur = filter("CIGaussianBlur");
            let _: () = msg_send![blur, setValue: clamped forKey: ns_string("inputImage")];
            let _: () =
                msg_send![blur, setValue: number(BLUR * SCALE) forKey: ns_string("inputRadius")];
            let soft: *mut Object = msg_send![blur, outputImage];
            if soft.is_null() {
                return None;
            }
            let context: *mut Object = msg_send![class!(CIContext), context];
            let cg: *mut c_void = msg_send![context, createCGImage: soft fromRect: extent];
            (!cg.is_null()).then_some(cg)
        }
    }

    /// Shows the picture `make` made. On the main queue.
    extern "C" fn show(image: *mut c_void) {
        WALL.with(|wall| {
            if let Some(wall) = wall.borrow().as_ref() {
                // SAFETY: main thread; the layer retains the image it is given.
                unsafe {
                    let _: () = msg_send![class!(CATransaction), begin];
                    let _: () = msg_send![class!(CATransaction), setDisableActions: YES];
                    let _: () = msg_send![wall.picture, setContents: image as *mut Object];
                    let _: () = msg_send![class!(CATransaction), commit];
                }
            }
        });
        // SAFETY: the image `make` created, owned here.
        unsafe { CGImageRelease(image) };
    }

    /// # Safety
    /// Inside an autorelease pool.
    unsafe fn filter(name: &str) -> *mut Object {
        unsafe { msg_send![class!(CIFilter), filterWithName: ns_string(name)] }
    }

    /// # Safety
    /// Inside an autorelease pool (the string is autoreleased).
    unsafe fn ns_string(text: &str) -> *mut Object {
        let text = CString::new(text).unwrap_or_default();
        unsafe { msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()] }
    }

    /// # Safety
    /// An NSString or nil.
    unsafe fn string_of(string: *mut Object) -> Option<String> {
        if string.is_null() {
            return None;
        }
        unsafe {
            let utf8: *const std::ffi::c_char = msg_send![string, UTF8String];
            (!utf8.is_null()).then(|| CStr::from_ptr(utf8).to_string_lossy().into_owned())
        }
    }
}
