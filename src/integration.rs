//! What macOS tells Nimbra outside its windows: links to open, answers to notifications,
//! quitting, waking from sleep, the network coming back, display and appearance changes.
//! Each arrives as an `Event` on one channel, in order, from whichever thread macOS used,
//! and the app reads them on the main thread. What arrives before the app is ready (a link
//! that launched it) waits in the channel.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use futures::channel::{mpsc, oneshot};

// Most are only sent by the macOS side.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub enum Event {
    /// Links from other apps (`mailto:`), as macOS hands them over.
    OpenUrls(Vec<String>),
    /// A `mailto:` link clicked in a message, from the account it is in when known, and the
    /// window it was clicked in.
    Mailto {
        link: String,
        account: Option<String>,
        window: Option<gpui::AnyWindowHandle>,
    },
    /// An answer to a notification. No account: the one shown (older notifications).
    Notification {
        account: Option<String>,
        thread: String,
        action: NoteAction,
    },
    /// Quitting was put off: it would lose something (see `quitting`).
    QuitPutOff,
    Woke,
    Online,
    /// Reduce Motion, Reduce Transparency or Increase Contrast changed.
    DisplayOptions,
    /// The Mac switched between light and dark.
    Appearance,
    /// Badges were allowed or disallowed in System Settings › Notifications.
    BadgeSetting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub enum NoteAction {
    Open,
    Archive,
    MarkRead,
}

struct Channel {
    sender: mpsc::UnboundedSender<Event>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<Event>>>,
}

fn channel() -> &'static Channel {
    static CHANNEL: OnceLock<Channel> = OnceLock::new();
    CHANNEL.get_or_init(|| {
        let (sender, receiver) = mpsc::unbounded();
        Channel {
            sender,
            receiver: Mutex::new(Some(receiver)),
        }
    })
}

/// From any thread.
pub fn post(event: Event) {
    channel().sender.unbounded_send(event).ok();
}

/// The events, for their one reader.
pub fn take_events() -> Option<Listener> {
    let receiver = channel()
        .receiver
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()?;
    Some(Listener(receiver))
}

/// A link clicked in a message: a `mailto:` one opens Nimbra's composer (from `account`,
/// the one the message is in, when known) instead of another mail app. Whether it was one.
pub fn mailto_from_mail(
    link: &str,
    account: Option<String>,
    window: Option<gpui::AnyWindowHandle>,
) -> bool {
    if !crate::mailto::is_mailto(link) {
        return false;
    }
    post(Event::Mailto {
        link: link.to_string(),
        account,
        window,
    });
    true
}

pub struct Listener(mpsc::UnboundedReceiver<Event>);

impl Listener {
    pub async fn next(&mut self) -> Option<Event> {
        futures::StreamExt::next(&mut self.0).await
    }
}

thread_local! {
    static PAUSES: RefCell<Vec<oneshot::Sender<()>>> = const { RefCell::new(Vec::new()) };
}

/// Waits for `timer`, or less if `end_pauses` is called first (the Mac woke, the network
/// came back): GPUI's timers do not count time asleep. Whether it was cut short. Main
/// thread only.
pub async fn pause(timer: impl std::future::Future<Output = ()>) -> bool {
    let (end, ended) = oneshot::channel();
    PAUSES.with(|pauses| {
        let mut pauses = pauses.borrow_mut();
        // Those whose timer ended first.
        pauses.retain(|pause| !pause.is_canceled());
        pauses.push(end);
    });
    match futures::future::select(std::pin::pin!(timer), ended).await {
        futures::future::Either::Left(_) => false,
        futures::future::Either::Right((result, _)) => result.is_ok(),
    }
}

pub fn end_pauses() {
    let pauses = PAUSES.with(|pauses| std::mem::take(&mut *pauses.borrow_mut()));
    for end in pauses {
        end.send(()).ok();
    }
}

/// Reduce Motion, Reduce Transparency and Increase Contrast, as last read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayOptions {
    pub reduce_motion: bool,
    pub reduce_transparency: bool,
    pub increase_contrast: bool,
}

static REDUCE_MOTION: AtomicBool = AtomicBool::new(false);
static REDUCE_TRANSPARENCY: AtomicBool = AtomicBool::new(false);
static INCREASE_CONTRAST: AtomicBool = AtomicBool::new(false);
static OPTIONS_READ: AtomicBool = AtomicBool::new(false);

pub fn display_options() -> DisplayOptions {
    if !OPTIONS_READ.load(Ordering::Relaxed) {
        read_display_options();
    }
    DisplayOptions {
        reduce_motion: REDUCE_MOTION.load(Ordering::Relaxed),
        reduce_transparency: REDUCE_TRANSPARENCY.load(Ordering::Relaxed),
        increase_contrast: INCREASE_CONTRAST.load(Ordering::Relaxed),
    }
}

/// Main thread.
pub fn read_display_options() {
    let options = platform::display_options();
    REDUCE_MOTION.store(options.reduce_motion, Ordering::Relaxed);
    REDUCE_TRANSPARENCY.store(options.reduce_transparency, Ordering::Relaxed);
    INCREASE_CONTRAST.store(options.increase_contrast, Ordering::Relaxed);
    OPTIONS_READ.store(true, Ordering::Relaxed);
}

/// Starts listening to the Mac. Main thread, once the app has launched.
pub fn install() {
    platform::install();
    read_display_options();
}

/// Which app opens `mailto:` links. Blocking: call it off the main thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailApp {
    pub ours: bool,
    pub name: String,
}

pub use platform::{default_mail_app, mac_is_dark, set_app_appearance, set_menu_check};

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // emitted by the objc crate's macros
mod platform {
    use std::ffi::{CStr, c_char, c_void};
    use std::sync::Once;

    use objc::declare::ClassDecl;
    use objc::runtime::{BOOL, Class, NO, Object, Sel};
    use objc::{class, msg_send, sel, sel_impl};

    use super::{DisplayOptions, Event, MailApp, post};

    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {
        static NSWorkspaceDidWakeNotification: *mut Object;
        static NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification: *mut Object;
        static NSAppearanceNameAqua: *mut Object;
        static NSAppearanceNameDarkAqua: *mut Object;
    }

    #[link(name = "Network", kind = "framework")]
    unsafe extern "C" {
        fn nw_path_monitor_create() -> *mut c_void;
        fn nw_path_monitor_set_queue(monitor: *mut c_void, queue: *mut c_void);
        /// `handler` is a block taking the path (`nw_path_monitor_update_handler_t`).
        fn nw_path_monitor_set_update_handler(monitor: *mut c_void, handler: *const c_void);
        fn nw_path_monitor_start(monitor: *mut c_void);
        fn nw_path_get_status(path: *mut c_void) -> i32;
    }

    unsafe extern "C" {
        fn dispatch_queue_create(label: *const c_char, attr: *mut c_void) -> *mut c_void;
    }

    const NW_PATH_STATUS_SATISFIED: i32 = 1;
    const UTF8: usize = 4;

    /// An autoreleased NSString. Main thread or inside an autorelease pool.
    unsafe fn ns_string(text: &str) -> *mut Object {
        // SAFETY: the bytes are valid for their length; the string copies them.
        unsafe {
            let string: *mut Object = msg_send![class!(NSString), alloc];
            let string: *mut Object =
                msg_send![string, initWithBytes: text.as_ptr() length: text.len() encoding: UTF8];
            msg_send![string, autorelease]
        }
    }

    unsafe fn rust_string(string: *mut Object) -> Option<String> {
        if string.is_null() {
            return None;
        }
        // SAFETY: a live NSString; its UTF-8 buffer is copied before the string can go.
        unsafe {
            let utf8: *const c_char = msg_send![string, UTF8String];
            (!utf8.is_null()).then(|| CStr::from_ptr(utf8).to_string_lossy().into_owned())
        }
    }

    pub fn install() {
        static INSTALLED: Once = Once::new();
        INSTALLED.call_once(|| {
            observe_workspace();
            observe_appearance();
            watch_network();
        });
    }

    fn observe_workspace() {
        // SAFETY: AppKit's notification name constants, read once they are linked.
        let (wake, options) = unsafe {
            (
                NSWorkspaceDidWakeNotification,
                NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification,
            )
        };
        observe(wake, false);
        observe(options, true);
    }

    fn observe(name: *mut Object, options: bool) {
        let block = block::ConcreteBlock::new(move |_note: *mut Object| {
            if options {
                super::read_display_options();
                post(Event::DisplayOptions);
            } else {
                post(Event::Woke);
            }
        })
        .copy();
        // SAFETY: main thread, and the block runs on the main queue (it reads AppKit). The
        // center copies the block; the observer it returns is retained here and kept for
        // the life of the app, as the observation is.
        unsafe {
            let workspace: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
            if workspace.is_null() || name.is_null() {
                return;
            }
            let center: *mut Object = msg_send![workspace, notificationCenter];
            let main: *mut Object = msg_send![class!(NSOperationQueue), mainQueue];
            if center.is_null() || main.is_null() {
                return;
            }
            let nil: *mut Object = std::ptr::null_mut();
            let observer: *mut Object = msg_send![center,
                addObserverForName: name object: nil queue: main usingBlock: &*block];
            if !observer.is_null() {
                let _: *mut Object = msg_send![observer, retain];
            }
        }
    }

    /// `NSApp.effectiveAppearance` is observable: it changes with the Mac's appearance,
    /// whichever windows are shown.
    fn observe_appearance() {
        extern "C" fn changed(
            _: &Object,
            _: Sel,
            _key: *mut Object,
            _object: *mut Object,
            _change: *mut Object,
            _context: *mut c_void,
        ) {
            post(Event::Appearance);
        }
        let Some(mut decl) = ClassDecl::new("NimbraAppearanceObserver", class!(NSObject)) else {
            return;
        };
        // SAFETY: the signature matches observeValueForKeyPath:ofObject:change:context:.
        unsafe {
            decl.add_method(
                sel!(observeValueForKeyPath:ofObject:change:context:),
                changed
                    as extern "C" fn(
                        &Object,
                        Sel,
                        *mut Object,
                        *mut Object,
                        *mut Object,
                        *mut c_void,
                    ),
            );
        }
        decl.register();
        let Some(class) = Class::get("NimbraAppearanceObserver") else {
            return;
        };
        // SAFETY: main thread. KVO does not retain the observer: it is made once and kept
        // for the life of the app, which is as long as NSApp is observed.
        unsafe {
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            if app.is_null() {
                return;
            }
            let observer: *mut Object = msg_send![class, new];
            let nil: *mut c_void = std::ptr::null_mut();
            let _: () = msg_send![app, addObserver: observer
                                        forKeyPath: ns_string("effectiveAppearance")
                                           options: 0usize
                                           context: nil];
        }
    }

    /// Network.framework calls on its own queue with the current path at once, then on
    /// every change. Only a path becoming usable is an event.
    fn watch_network() {
        let was_up = std::sync::atomic::AtomicBool::new(true);
        let handler = block::ConcreteBlock::new(move |path: *mut c_void| {
            // SAFETY: Network passes a live path for the duration of the call.
            let up =
                !path.is_null() && unsafe { nw_path_get_status(path) } == NW_PATH_STATUS_SATISFIED;
            if up && !was_up.swap(true, std::sync::atomic::Ordering::Relaxed) {
                post(Event::Online);
            } else if !up {
                was_up.store(false, std::sync::atomic::Ordering::Relaxed);
            }
        })
        .copy();
        // SAFETY: the monitor and its queue live as long as the app and are never released;
        // Network copies the handler block. The label is a static C string.
        unsafe {
            let queue =
                dispatch_queue_create(c"com.riangle.nimbra.network".as_ptr(), std::ptr::null_mut());
            let monitor = nw_path_monitor_create();
            if queue.is_null() || monitor.is_null() {
                return;
            }
            nw_path_monitor_set_queue(monitor, queue);
            nw_path_monitor_set_update_handler(monitor, &*handler as *const _ as *const c_void);
            nw_path_monitor_start(monitor);
        }
    }

    pub fn display_options() -> DisplayOptions {
        // SAFETY: main thread; the shared workspace lives as long as the app.
        unsafe {
            let workspace: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
            if workspace.is_null() {
                return DisplayOptions::default();
            }
            let motion: BOOL = msg_send![workspace, accessibilityDisplayShouldReduceMotion];
            let transparency: BOOL =
                msg_send![workspace, accessibilityDisplayShouldReduceTransparency];
            let contrast: BOOL = msg_send![workspace, accessibilityDisplayShouldIncreaseContrast];
            DisplayOptions {
                reduce_motion: motion != NO,
                reduce_transparency: transparency != NO,
                increase_contrast: contrast != NO,
            }
        }
    }

    /// `Some(dark)` makes the app's own panels, menus and window buttons light or dark;
    /// `None` lets them follow the Mac again. Main thread.
    pub fn set_app_appearance(dark: Option<bool>) {
        // SAFETY: main thread; NSApp and the named appearances live as long as the app.
        unsafe {
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            if app.is_null() {
                return;
            }
            let appearance: *mut Object = match dark {
                None => std::ptr::null_mut(),
                Some(dark) => {
                    let name = if dark {
                        NSAppearanceNameDarkAqua
                    } else {
                        NSAppearanceNameAqua
                    };
                    msg_send![class!(NSAppearance), appearanceNamed: name]
                }
            };
            let _: () = msg_send![app, setAppearance: appearance];
        }
    }

    /// The Mac's own appearance, also while the app forces one of its own. Main thread.
    pub fn mac_is_dark() -> bool {
        // SAFETY: main thread. The global defaults hold `AppleInterfaceStyle` = "Dark" while
        // the Mac is dark (also under Auto); it is absent in light.
        unsafe {
            let defaults: *mut Object = msg_send![class!(NSUserDefaults), standardUserDefaults];
            if defaults.is_null() {
                return false;
            }
            let style: *mut Object =
                msg_send![defaults, stringForKey: ns_string("AppleInterfaceStyle")];
            rust_string(style).is_some_and(|style| style.eq_ignore_ascii_case("dark"))
        }
    }

    /// Puts a check mark on the item titled `checked` in the menu bar submenu at `path`
    /// (titles), and takes it off the others. Main thread.
    pub fn set_menu_check(path: &[&str], checked: &str) {
        // SAFETY: main thread; every menu and item is checked for null before use, and
        // titles are compared as NSStrings made here.
        unsafe {
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            if app.is_null() {
                return;
            }
            let mut menu: *mut Object = msg_send![app, mainMenu];
            for title in path {
                if menu.is_null() {
                    return;
                }
                let item: *mut Object = msg_send![menu, itemWithTitle: ns_string(title)];
                if item.is_null() {
                    return;
                }
                menu = msg_send![item, submenu];
            }
            if menu.is_null() {
                return;
            }
            let wanted = ns_string(checked);
            let count: isize = msg_send![menu, numberOfItems];
            for index in 0..count {
                let item: *mut Object = msg_send![menu, itemAtIndex: index];
                if item.is_null() {
                    continue;
                }
                let title: *mut Object = msg_send![item, title];
                let same: BOOL = msg_send![title, isEqualToString: wanted];
                let state: isize = if same != NO { 1 } else { 0 };
                let _: () = msg_send![item, setState: state];
            }
        }
    }

    /// Thread-safe: `URLForApplicationToOpenURL:` may be called from any thread.
    pub fn default_mail_app() -> Option<MailApp> {
        objc::rc::autoreleasepool(|| {
            // SAFETY: inside an autorelease pool; each object is checked for null.
            unsafe {
                let url: *mut Object =
                    msg_send![class!(NSURL), URLWithString: ns_string("mailto:")];
                let workspace: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
                if url.is_null() || workspace.is_null() {
                    return None;
                }
                let app: *mut Object = msg_send![workspace, URLForApplicationToOpenURL: url];
                if app.is_null() {
                    return Some(MailApp {
                        ours: false,
                        name: String::new(),
                    });
                }
                let bundle: *mut Object = msg_send![class!(NSBundle), bundleWithURL: app];
                let main: *mut Object = msg_send![class!(NSBundle), mainBundle];
                let theirs = if bundle.is_null() {
                    None
                } else {
                    rust_string(msg_send![bundle, bundleIdentifier])
                };
                let ours = rust_string(msg_send![main, bundleIdentifier]);
                let path: *mut Object = msg_send![app, path];
                let files: *mut Object = msg_send![class!(NSFileManager), defaultManager];
                let name = if path.is_null() || files.is_null() {
                    None
                } else {
                    rust_string(msg_send![files, displayNameAtPath: path])
                };
                let name = name.unwrap_or_default();
                Some(MailApp {
                    ours: ours.is_some() && theirs == ours,
                    name: name.strip_suffix(".app").unwrap_or(&name).to_string(),
                })
            }
        })
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::{DisplayOptions, MailApp};

    pub fn install() {}

    pub fn display_options() -> DisplayOptions {
        DisplayOptions::default()
    }

    pub fn set_app_appearance(_dark: Option<bool>) {}

    pub fn mac_is_dark() -> bool {
        false
    }

    pub fn set_menu_check(_path: &[&str], _checked: &str) {}

    pub fn default_mail_app() -> Option<MailApp> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pause_ends_early_when_asked() {
        let ended = futures::executor::block_on(async {
            let pause = pause(futures::future::pending());
            let mut pause = std::pin::pin!(pause);
            assert!(futures::poll!(pause.as_mut()).is_pending());
            end_pauses();
            futures::poll!(pause.as_mut())
        });
        assert_eq!(ended, std::task::Poll::Ready(true));
    }

    #[test]
    fn a_pause_ends_with_its_timer() {
        assert!(!futures::executor::block_on(pause(futures::future::ready(
            ()
        ))));
    }

    #[test]
    fn pauses_that_ended_with_their_timer_are_not_kept() {
        for _ in 0..100 {
            futures::executor::block_on(pause(futures::future::ready(())));
        }
        assert!(PAUSES.with(|pauses| pauses.borrow().len()) <= 1);
    }
}
