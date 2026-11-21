//! System notifications for new mail.
//!
//! macOS only for now, through the UserNotifications framework. That framework needs a
//! real app bundle (it identifies the app by its bundle id), so nothing happens when the
//! bare binary is run with `cargo run`. macOS shows these banners while the app is in the
//! background; with the app in front, the new mail simply appears in the list.

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

/// Shows a notification. `id` identifies it, so the same thread never stacks up twice.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
pub fn show(id: &str, title: &str, body: &str) {
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
        let sound: *mut Object = msg_send![class!(UNNotificationSound), defaultSound];
        let _: () = msg_send![content, setSound: sound];
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
pub fn show(_id: &str, _title: &str, _body: &str) {}
