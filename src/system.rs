//! Small macOS integrations: the unread count on the Dock icon, and opening at login.

/// Shows `count` on the Dock icon, or nothing for zero.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // emitted by the objc crate's macros
pub fn set_badge(count: u32) {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};

    let label = if count == 0 {
        String::new()
    } else if count > 9999 {
        "9999+".to_string()
    } else {
        count.to_string()
    };
    let Ok(label) = std::ffi::CString::new(label) else {
        return;
    };
    // SAFETY: called on the main thread; the dock tile and the string are live AppKit
    // objects for the duration of the call.
    unsafe {
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        let tile: *mut Object = msg_send![app, dockTile];
        if tile.is_null() {
            return;
        }
        let text: *mut Object = msg_send![class!(NSString), stringWithUTF8String: label.as_ptr()];
        let _: () = msg_send![tile, setBadgeLabel: text];
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set_badge(_count: u32) {}

#[cfg(target_os = "macos")]
#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

/// Whether Nimbra is set to open at login.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
pub fn opens_at_login() -> bool {
    use objc::runtime::{Class, Object};
    use objc::{msg_send, sel, sel_impl};
    let Some(class) = Class::get("SMAppService") else {
        return false;
    };
    // SAFETY: main thread; SMAppService exists (macOS 13+), checked above.
    unsafe {
        let service: *mut Object = msg_send![class, mainAppService];
        if service.is_null() {
            return false;
        }
        let status: isize = msg_send![service, status];
        status == 1 // SMAppServiceStatusEnabled
    }
}

/// Turns opening at login on or off. Needs macOS 13 and the app bundle (Nimbra.app).
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
pub fn set_open_at_login(on: bool) -> Result<(), String> {
    use objc::runtime::{BOOL, Class, NO, Object};
    use objc::{msg_send, sel, sel_impl};
    if std::env::current_exe()
        .map(|exe| !exe.to_string_lossy().contains(".app/Contents/MacOS/"))
        .unwrap_or(true)
    {
        return Err("Open at login works from Nimbra.app.".into());
    }
    let Some(class) = Class::get("SMAppService") else {
        return Err("Open at login needs macOS 13 or later.".into());
    };
    // SAFETY: main thread; SMAppService exists, checked above; the error out-pointer is
    // a local that AppKit may fill with an autoreleased NSError.
    unsafe {
        let service: *mut Object = msg_send![class, mainAppService];
        if service.is_null() {
            return Err("Open at login is not available.".into());
        }
        let mut error: *mut Object = std::ptr::null_mut();
        let ok: BOOL = if on {
            msg_send![service, registerAndReturnError: &mut error]
        } else {
            msg_send![service, unregisterAndReturnError: &mut error]
        };
        if ok == NO {
            return Err(if on {
                "macOS did not allow Nimbra to open at login. Check System Settings › General › Login Items.".into()
            } else {
                "Could not turn off opening at login.".into()
            });
        }
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn opens_at_login() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn set_open_at_login(_on: bool) -> Result<(), String> {
    Err("Open at login is only available on macOS for now.".into())
}
