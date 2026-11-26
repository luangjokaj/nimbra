//! System notifications for new mail.
//!
//! macOS only for now, through the UserNotifications framework. That framework needs a
//! real app bundle (it identifies the app by its bundle id), so nothing happens when the
//! bare binary is run with `cargo run`. Banners show whether or not Nimbra is the front app
//! (see `show_while_active`).
//!
//! A notification's id is `acct:<account>:<thread>` (older ones: a bare thread id, of the
//! account shown). Each account's are grouped together; a thread's has Archive and Mark as
//! Read; answers reach the app as `integration::Event::Notification`.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use crate::integration::NoteAction;

/// What a notification is about. `thread` is "more" for the summary of several.
#[derive(Debug, PartialEq, Eq)]
struct About {
    account: Option<String>,
    thread: String,
}

fn about(id: &str) -> Option<About> {
    if id == "test" {
        return None;
    }
    Some(
        match id
            .strip_prefix("acct:")
            .and_then(|rest| rest.split_once(':'))
        {
            Some((account, thread)) => About {
                account: Some(account.to_string()),
                thread: thread.to_string(),
            },
            None => About {
                account: None,
                thread: id.to_string(),
            },
        },
    )
}

/// The ids a thread's notification may have been shown under.
fn ids_of(account: &str, thread: &str) -> [String; 2] {
    [format!("acct:{account}:{thread}"), thread.to_string()]
}

const CATEGORY: &str = "thread";
const ARCHIVE: &str = "archive";
const MARK_READ: &str = "read";

fn action_of(identifier: &str) -> NoteAction {
    match identifier {
        ARCHIVE => NoteAction::Archive,
        MARK_READ => NoteAction::MarkRead,
        _ => NoteAction::Open,
    }
}

/// Whether notifications can work in this process.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // emitted by the objc crate's macros
pub fn available() -> bool {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    // SAFETY: plain message sends to NSBundle on the main thread.
    unsafe {
        let bundle: *mut Object = msg_send![class!(NSBundle), mainBundle];
        let id: *mut Object = msg_send![bundle, bundleIdentifier];
        !id.is_null()
    }
}

#[cfg(target_os = "macos")]
#[link(name = "UserNotifications", kind = "framework")]
unsafe extern "C" {}

/// An autoreleased NSString.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
unsafe fn ns_string(text: &str) -> *mut objc::runtime::Object {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    // SAFETY: the bytes are valid for their length and copied by the string.
    unsafe {
        let object: *mut Object = msg_send![class!(NSString), alloc];
        let object: *mut Object =
            msg_send![object, initWithBytes: text.as_ptr() length: text.len() encoding: 4usize];
        msg_send![object, autorelease]
    }
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
unsafe fn rust_string(string: *mut objc::runtime::Object) -> Option<String> {
    use objc::{msg_send, sel, sel_impl};
    if string.is_null() {
        return None;
    }
    // SAFETY: a live NSString; its UTF-8 bytes are copied before returning.
    unsafe {
        let utf8: *const std::os::raw::c_char = msg_send![string, UTF8String];
        (!utf8.is_null()).then(|| {
            std::ffi::CStr::from_ptr(utf8)
                .to_string_lossy()
                .into_owned()
        })
    }
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
unsafe fn center() -> *mut objc::runtime::Object {
    use objc::{class, msg_send, sel, sel_impl};
    // SAFETY: only called inside an app bundle (`available`).
    unsafe { msg_send![class!(UNUserNotificationCenter), currentNotificationCenter] }
}

/// Whether macOS lets Nimbra badge its icon (Notifications settings), as last read. Until
/// read, and outside an app bundle, it does.
static BADGES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn badges_allowed() -> bool {
    BADGES.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether macOS lets Nimbra show notifications, as last read; like `BADGES`.
static ALERTS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn alerts_allowed() -> bool {
    ALERTS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Logs (to stderr) whether macOS currently allows this app to notify. Diagnostic only:
/// prints a status number, never any mail content.
pub fn log_permission() {
    read_settings(true);
}

/// Reads again whether badges are allowed (they are changed in System Settings).
pub fn refresh_settings() {
    read_settings(false);
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
fn read_settings(log: bool) {
    use objc::runtime::Object;
    use objc::{msg_send, sel, sel_impl};
    if !available() {
        if log {
            eprintln!("nimbra: notifications unavailable (not running from Nimbra.app)");
        }
        return;
    }
    let handler = block::ConcreteBlock::new(move |settings: *mut Object| {
        if settings.is_null() {
            return;
        }
        // Status: 0 not decided, 1 denied, 2 allowed, 3 provisional, 4 ephemeral.
        // A setting: 0 not supported, 1 disabled, 2 enabled.
        // SAFETY: the framework passes a live UNNotificationSettings.
        let (status, alert, badge): (isize, isize, isize) = unsafe {
            (
                msg_send![settings, authorizationStatus],
                msg_send![settings, alertSetting],
                msg_send![settings, badgeSetting],
            )
        };
        if log {
            eprintln!("nimbra: notification permission status={status} alerts={alert}");
        }
        let allowed = badge != 1;
        let alerts = status != 1 && alert != 1;
        let badges_changed = BADGES.swap(allowed, std::sync::atomic::Ordering::Relaxed) != allowed;
        let alerts_changed = ALERTS.swap(alerts, std::sync::atomic::Ordering::Relaxed) != alerts;
        if badges_changed || alerts_changed {
            crate::integration::post(crate::integration::Event::BadgeSetting);
        }
    })
    .copy();
    // SAFETY: main thread, inside an app bundle; the block is heap-copied and called on a
    // background queue, where it only reads numbers and posts an event.
    unsafe {
        let _: () = msg_send![center(), getNotificationSettingsWithCompletionHandler: &*handler];
    }
}

#[cfg(not(target_os = "macos"))]
fn read_settings(_log: bool) {}

/// Lets banners appear while Nimbra is the front app too (by default macOS drops a
/// notification silently when its app is active), takes the answers to notifications, and
/// registers the actions on a thread's notification.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
pub fn show_while_active() {
    use objc::declare::ClassDecl;
    use objc::runtime::{Class, Object, Protocol, Sel};
    use objc::{class, msg_send, sel, sel_impl};
    use std::sync::Once;

    // Banner, list (Notification Center) and sound.
    const PRESENT: usize = (1 << 4) | (1 << 3) | (1 << 1);

    extern "C" fn will_present(
        _: &Object,
        _: Sel,
        _center: *mut Object,
        _notification: *mut Object,
        handler: *mut Object,
    ) {
        if handler.is_null() {
            return;
        }
        // SAFETY: the framework passes a live completion block taking the options.
        let handler = handler as *mut block::Block<(usize,), ()>;
        unsafe { (*handler).call((PRESENT,)) };
    }

    // May be called on any thread: it only reads the response and posts an event.
    extern "C" fn did_receive(
        _: &Object,
        _: Sel,
        _center: *mut Object,
        response: *mut Object,
        handler: *mut Object,
    ) {
        // SAFETY: the framework passes a live UNNotificationResponse and completion block;
        // strings are copied out before returning, inside a pool of our own.
        unsafe {
            objc::rc::autoreleasepool(|| {
                if response.is_null() {
                    return;
                }
                let action = rust_string(msg_send![response, actionIdentifier]);
                let notification: *mut Object = msg_send![response, notification];
                if notification.is_null() {
                    return;
                }
                let request: *mut Object = msg_send![notification, request];
                if request.is_null() {
                    return;
                }
                let id = rust_string(msg_send![request, identifier]);
                if let Some(about) = id.as_deref().and_then(about) {
                    crate::integration::post(crate::integration::Event::Notification {
                        account: about.account,
                        thread: about.thread,
                        action: action_of(action.as_deref().unwrap_or_default()),
                    });
                }
            });
            if !handler.is_null() {
                let handler = handler as *mut block::Block<(), ()>;
                (*handler).call(());
            }
        }
    }

    static SET: Once = Once::new();
    if !available() {
        return;
    }
    SET.call_once(|| {
        let Some(mut decl) = ClassDecl::new("NimbraNotificationDelegate", class!(NSObject)) else {
            return;
        };
        if let Some(protocol) = Protocol::get("UNUserNotificationCenterDelegate") {
            decl.add_protocol(protocol);
        }
        // SAFETY: the signatures match the delegate methods
        // userNotificationCenter:willPresentNotification:withCompletionHandler: and
        // userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:.
        unsafe {
            decl.add_method(
                sel!(userNotificationCenter:willPresentNotification:withCompletionHandler:),
                will_present as extern "C" fn(&Object, Sel, *mut Object, *mut Object, *mut Object),
            );
            decl.add_method(
                sel!(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:),
                did_receive as extern "C" fn(&Object, Sel, *mut Object, *mut Object, *mut Object),
            );
        }
        decl.register();
        let Some(class) = Class::get("NimbraNotificationDelegate") else {
            return;
        };
        // SAFETY: main thread; the delegate is created once and deliberately never freed,
        // because the notification center only keeps a weak reference to it. The actions,
        // category and set are autoreleased and retained by the center.
        unsafe {
            let delegate: *mut Object = msg_send![class, new];
            let center = center();
            let _: () = msg_send![center, setDelegate: delegate];
            // No options: the action runs without bringing Nimbra forward.
            let action = |id: &str, title: &str| -> *mut Object {
                msg_send![class!(UNNotificationAction),
                    actionWithIdentifier: ns_string(id) title: ns_string(title) options: 0usize]
            };
            let actions = [
                action(ARCHIVE, "Archive"),
                action(MARK_READ, "Mark as Read"),
            ];
            if actions.iter().any(|a| a.is_null()) {
                return;
            }
            let actions: *mut Object = msg_send![class!(NSArray),
                arrayWithObjects: actions.as_ptr() count: actions.len()];
            let intents: *mut Object = msg_send![class!(NSArray), array];
            let category: *mut Object = msg_send![class!(UNNotificationCategory),
                categoryWithIdentifier: ns_string(CATEGORY)
                               actions: actions
                     intentIdentifiers: intents
                               options: 0usize];
            if category.is_null() {
                return;
            }
            let categories: *mut Object = msg_send![class!(NSSet), setWithObject: category];
            let _: () = msg_send![center, setNotificationCategories: categories];
        }
    });
}

/// Asks the user, once, for permission to show notifications. macOS remembers the answer.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
pub fn request_permission() {
    use objc::runtime::{BOOL, Object};
    use objc::{msg_send, sel, sel_impl};
    if !available() {
        return;
    }
    // Alert (banner), sound and badge.
    const OPTIONS: usize = (1 << 2) | (1 << 1) | 1;
    // The answer may have turned badges on or off.
    let handler =
        block::ConcreteBlock::new(|_granted: BOOL, _error: *mut Object| refresh_settings()).copy();
    // SAFETY: main thread, inside an app bundle (checked above); the block is heap-copied
    // and retained by the framework for as long as it needs it.
    unsafe {
        let _: () = msg_send![center(), requestAuthorizationWithOptions: OPTIONS
                                                    completionHandler: &*handler];
    }
}

thread_local! {
    /// The sound being previewed: kept until it is done, or it would stop at once.
    static PLAYING: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Plays the mail sound now (Settings' preview), as a notification would. Nothing when the
/// app has no sound of its own (a bare debug binary).
#[cfg(target_os = "macos")]
pub fn preview_sound() {
    play(MAIL_SOUND);
}

/// The hello that greets a new user, with the first screen's animation.
#[cfg(target_os = "macos")]
pub fn welcome_sound() {
    play("welcome.caf");
}

/// Plays one of the app's own sounds (in its Resources) now. Nothing when it has none (a
/// bare debug binary).
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
fn play(name: &str) {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("Resources").join(name)));
    // A debug build run from the repo plays them from assets/sounds.
    #[cfg(debug_assertions)]
    let bundled = bundled.filter(|path| path.exists()).or_else(|| {
        Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets/sounds")
                .join(name),
        )
    });
    let Some(path) = bundled.filter(|path| path.exists()) else {
        return;
    };
    // SAFETY: main thread; an NSSound made here and kept (retained) in PLAYING until the
    // next one replaces it, so it plays to the end.
    unsafe {
        if let Some(old) = PLAYING.with(|p| p.take()) {
            let old = old as *mut Object;
            let _: () = msg_send![old, stop];
            let _: () = msg_send![old, release];
        }
        let sound: *mut Object = msg_send![class!(NSSound), alloc];
        let sound: *mut Object = msg_send![sound, initWithContentsOfFile: ns_string(&path.to_string_lossy())
                                                   byReference: true];
        if sound.is_null() {
            return;
        }
        let _: bool = msg_send![sound, play];
        PLAYING.with(|p| p.set(Some(sound as usize)));
    }
}

#[cfg(not(target_os = "macos"))]
pub fn preview_sound() {}

#[cfg(not(target_os = "macos"))]
pub fn welcome_sound() {}

/// The sound every mail notification plays: a file in the app's Resources (macOS takes
/// AIFF, WAV or CAF of up to 30 seconds).
#[cfg(target_os = "macos")]
const MAIL_SOUND: &str = "mail.caf";

/// The name the chime is installed under in `~/Library/Sounds`.
#[cfg(target_os = "macos")]
const INSTALLED_SOUND: &str = "Nimbra.caf";

/// Puts the chime where notifications can find it, once a run, and gives its name. macOS
/// looks for a notification's sound in `~/Library/Sounds` (an app outside the App Store
/// has no container of its own), not in the app, so it is copied there, and again when a
/// new version brings a different one. `None` when the app has no chime (a bare debug
/// binary) or it cannot be copied: the system's sound plays then.
#[cfg(target_os = "macos")]
fn mail_sound() -> Option<&'static str> {
    static INSTALLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let installed = *INSTALLED.get_or_init(|| {
        let install = || -> Option<()> {
            let exe = std::env::current_exe().ok()?;
            let bundled =
                std::fs::read(exe.parent()?.parent()?.join("Resources").join(MAIL_SOUND)).ok()?;
            let sounds = std::path::Path::new(&std::env::var_os("HOME")?).join("Library/Sounds");
            let target = sounds.join(INSTALLED_SOUND);
            if std::fs::read(&target).ok().as_deref() != Some(bundled.as_slice()) {
                std::fs::create_dir_all(&sounds).ok()?;
                // Written whole, then renamed into place.
                let partial = sounds.join(".Nimbra.caf.partial");
                std::fs::write(&partial, &bundled).ok()?;
                std::fs::rename(&partial, &target).ok()?;
            }
            Some(())
        };
        install().is_some()
    });
    installed.then_some(INSTALLED_SOUND)
}

/// Shows a notification. `id` identifies it (see the module), so the same thread never
/// stacks up twice; `sound` plays Nimbra's mail sound with it.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
pub fn show(id: &str, title: &str, body: &str, sound: bool) {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    if !available() {
        return;
    }
    let about = about(id);
    // SAFETY: main thread, inside an app bundle; every object created here is autoreleased
    // or released before returning.
    unsafe {
        let content: *mut Object = msg_send![class!(UNMutableNotificationContent), new];
        if content.is_null() {
            return;
        }
        let _: () = msg_send![content, setTitle: ns_string(title)];
        let _: () = msg_send![content, setBody: ns_string(body)];
        if sound {
            // Nimbra's own chime (from assets/sounds, installed by `mail_sound`), or the
            // system's when the app has none.
            let sound: *mut Object = match mail_sound() {
                Some(name) => {
                    msg_send![class!(UNNotificationSound), soundNamed: ns_string(name)]
                }
                None => msg_send![class!(UNNotificationSound), defaultSound],
            };
            let _: () = msg_send![content, setSound: sound];
        }
        if let Some(about) = &about {
            let group = format!("acct:{}", about.account.as_deref().unwrap_or_default());
            let _: () = msg_send![content, setThreadIdentifier: ns_string(&group)];
            if about.thread != "more" {
                let _: () = msg_send![content, setCategoryIdentifier: ns_string(CATEGORY)];
            }
        }
        let nil: *mut Object = std::ptr::null_mut();
        let request: *mut Object = msg_send![class!(UNNotificationRequest),
            requestWithIdentifier: ns_string(id) content: content trigger: nil];
        if !request.is_null() {
            let _: () =
                msg_send![center(), addNotificationRequest: request withCompletionHandler: nil];
        }
        let _: () = msg_send![content, release];
    }
}

/// Takes a thread's notification out of Notification Center (read, archived or deleted
/// here). Asynchronous; nothing happens when there is none.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
pub fn withdraw(account: &str, thread: &str) {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    if !available() {
        return;
    }
    // SAFETY: main thread, inside an app bundle; the strings and array are autoreleased.
    unsafe {
        let ids = ids_of(account, thread).map(|id| ns_string(&id));
        let ids: *mut Object =
            msg_send![class!(NSArray), arrayWithObjects: ids.as_ptr() count: ids.len()];
        if !ids.is_null() {
            let _: () = msg_send![center(), removeDeliveredNotificationsWithIdentifiers: ids];
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn request_permission() {}

#[cfg(not(target_os = "macos"))]
pub fn show_while_active() {}

#[cfg(not(target_os = "macos"))]
pub fn show(_id: &str, _title: &str, _body: &str, _sound: bool) {}

#[cfg(not(target_os = "macos"))]
pub fn withdraw(_account: &str, _thread: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_says_whose_thread_it_is() {
        assert_eq!(
            about("acct:a1:t9"),
            Some(About {
                account: Some("a1".into()),
                thread: "t9".into()
            })
        );
        // The first account's id is "".
        assert_eq!(
            about("acct::t9"),
            Some(About {
                account: Some(String::new()),
                thread: "t9".into()
            })
        );
        assert_eq!(
            about("t9"),
            Some(About {
                account: None,
                thread: "t9".into()
            })
        );
        assert_eq!(about("test"), None);
    }

    #[test]
    fn a_thread_is_withdrawn_under_both_ids() {
        assert_eq!(ids_of("a1", "t9"), ["acct:a1:t9", "t9"]);
    }

    #[test]
    fn only_our_actions_act() {
        assert_eq!(action_of("archive"), NoteAction::Archive);
        assert_eq!(action_of("read"), NoteAction::MarkRead);
        assert_eq!(
            action_of("com.apple.UNNotificationDefaultActionIdentifier"),
            NoteAction::Open
        );
        assert_eq!(
            action_of("com.apple.UNNotificationDismissActionIdentifier"),
            NoteAction::Open
        );
    }
}
