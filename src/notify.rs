//! System notifications for new mail.
//!
//! macOS only for now, through the UserNotifications framework. That framework needs a
//! real app bundle (it identifies the app by its bundle id), so nothing happens when the
//! bare binary is run with `cargo run`. Banners show whether or not Nimbra is the front app
//! (see `show_while_active`).

/// Identifiers of notifications the user clicked, oldest first, waiting for the app.
static CLICKED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Takes the clicked notifications' identifiers (thread ids, or "more" for the summary).
pub fn take_clicked() -> Vec<String> {
    CLICKED
        .lock()
        .map(|mut c| std::mem::take(&mut *c))
        .unwrap_or_default()
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

/// Logs (to stderr) whether macOS currently allows this app to notify. Diagnostic only:
/// prints a status number, never any mail content.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
pub fn log_permission() {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    if !available() {
        eprintln!("nimbra: notifications unavailable (not running from Nimbra.app)");
        return;
    }
    let handler = block::ConcreteBlock::new(|settings: *mut Object| {
        // 0 not decided, 1 denied, 2 allowed, 3 provisional, 4 ephemeral.
        // SAFETY: the framework passes a live UNNotificationSettings.
        let status: isize = unsafe { msg_send![settings, authorizationStatus] };
        let alert: isize = unsafe { msg_send![settings, alertSetting] };
        eprintln!("nimbra: notification permission status={status} alerts={alert}");
    })
    .copy();
    // SAFETY: main thread, inside an app bundle; the block is heap-copied.
    unsafe {
        let center: *mut Object =
            msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
        let _: () = msg_send![center, getNotificationSettingsWithCompletionHandler: &*handler];
    }
}

/// Lets banners appear while Nimbra is the front app too. By default macOS drops a
/// notification silently when its app is active, which made new-mail alerts look broken.
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
        // SAFETY: the framework passes a live completion block taking the options.
        let handler = handler as *mut block::Block<(usize,), ()>;
        unsafe { (*handler).call((PRESENT,)) };
    }

    // A notification was clicked: remember which thread it was about, for the app to open.
    extern "C" fn did_receive(
        _: &Object,
        _: Sel,
        _center: *mut Object,
        response: *mut Object,
        handler: *mut Object,
    ) {
        // SAFETY: the framework passes a live UNNotificationResponse and completion block;
        // `identifier` is an NSString whose UTF-8 bytes are copied before returning.
        unsafe {
            let notification: *mut Object = msg_send![response, notification];
            let request: *mut Object = msg_send![notification, request];
            let identifier: *mut Object = msg_send![request, identifier];
            let utf8: *const std::os::raw::c_char = msg_send![identifier, UTF8String];
            if !utf8.is_null() {
                let id = std::ffi::CStr::from_ptr(utf8)
                    .to_string_lossy()
                    .into_owned();
                if let Ok(mut clicked) = CLICKED.lock() {
                    clicked.push(id);
                }
            }
            let handler = handler as *mut block::Block<(), ()>;
            (*handler).call(());
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
        // SAFETY: the signature matches the delegate method
        // userNotificationCenter:willPresentNotification:withCompletionHandler:.
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
        // because the notification center only keeps a weak reference to it.
        unsafe {
            let delegate: *mut Object = msg_send![class, new];
            let center: *mut Object =
                msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
            let _: () = msg_send![center, setDelegate: delegate];
        }
    });
}

/// Asks the user, once, for permission to show notifications. macOS remembers the answer.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
pub fn request_permission() {
    use objc::runtime::{BOOL, Object};
    use objc::{class, msg_send, sel, sel_impl};
    if !available() {
        return;
    }
    // Alert (banner) and sound.
    const OPTIONS: usize = (1 << 2) | (1 << 1);
    let handler = block::ConcreteBlock::new(|_granted: BOOL, _error: *mut Object| {}).copy();
    // SAFETY: main thread, inside an app bundle (checked above); the block is heap-copied
    // and retained by the framework for as long as it needs it.
    unsafe {
        let center: *mut Object =
            msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
        let _: () = msg_send![center, requestAuthorizationWithOptions: OPTIONS
                                                     completionHandler: &*handler];
    }
}

/// Shows a notification. `id` identifies it, so the same thread never stacks up twice;
/// `sound` plays the default notification sound with it.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
pub fn show(id: &str, title: &str, body: &str, sound: bool) {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    if !available() {
        return;
    }
    // SAFETY: main thread, inside an app bundle; strings are passed with their length and
    // every object created here is autoreleased or released before returning.
    unsafe {
        let string = |text: &str| -> *mut Object {
            let object: *mut Object = msg_send![class!(NSString), alloc];
            let object: *mut Object =
                msg_send![object, initWithBytes: text.as_ptr() length: text.len() encoding: 4usize];
            msg_send![object, autorelease]
        };
        let content: *mut Object = msg_send![class!(UNMutableNotificationContent), new];
        let _: () = msg_send![content, setTitle: string(title)];
        let _: () = msg_send![content, setBody: string(body)];
        if sound {
            let sound: *mut Object = msg_send![class!(UNNotificationSound), defaultSound];
            let _: () = msg_send![content, setSound: sound];
        }
        let nil: *mut Object = std::ptr::null_mut();
        let request: *mut Object = msg_send![class!(UNNotificationRequest),
            requestWithIdentifier: string(id) content: content trigger: nil];
        let center: *mut Object =
            msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
        let _: () = msg_send![center, addNotificationRequest: request withCompletionHandler: nil];
        let _: () = msg_send![content, release];
    }
}

#[cfg(not(target_os = "macos"))]
pub fn request_permission() {}

#[cfg(not(target_os = "macos"))]
pub fn show_while_active() {}

#[cfg(not(target_os = "macos"))]
pub fn log_permission() {}

#[cfg(not(target_os = "macos"))]
pub fn show(_id: &str, _title: &str, _body: &str, _sound: bool) {}
