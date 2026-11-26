//! Nimbra › About Nimbra: the system About panel, with Nimbra's icon (light or dark, as macOS
//! shows the app), its version, a line on its license, and links to the source.

/// Where the links in the panel go.
const SOURCE: &str = "https://github.com/nimbra-mail/desktop";
const LICENSE: &str = "https://github.com/nimbra-mail/desktop/blob/main/LICENSE";
const CHANGELOG: &str = "https://github.com/nimbra-mail/desktop/blob/main/CHANGELOG.md";

/// The paragraph under the version, and the links after it (label, address).
fn credits() -> (String, Vec<(&'static str, &'static str)>) {
    let text = "© 2026 Luan Gjokaj. Nimbra is free software under the MIT License, with \
                ABSOLUTELY NO WARRANTY. Its source is at the link below. It's built on GPUI and \
                other open-source libraries, each under its own license."
        .to_string();
    (
        text,
        vec![
            ("Source code", SOURCE),
            ("License", LICENSE),
            ("Changelog", CHANGELOG),
        ],
    )
}

/// The credits as one string, and where each link sits in it, in UTF-16 units (as AppKit
/// counts them).
fn credits_text() -> (String, Vec<(usize, usize, &'static str)>) {
    let (text, links) = credits();
    let mut full = format!("{text}\n\n");
    let mut ranges = Vec::new();
    for (i, (label, url)) in links.iter().enumerate() {
        if i > 0 {
            full.push_str("  ·  ");
        }
        let start = full.encode_utf16().count();
        full.push_str(label);
        ranges.push((start, label.encode_utf16().count(), *url));
    }
    (full, ranges)
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // emitted by the objc crate's macros
pub fn show() {
    use objc::runtime::Object;
    use objc::{Encode, Encoding, class, msg_send, sel, sel_impl};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NSRange {
        location: usize,
        length: usize,
    }
    unsafe impl Encode for NSRange {
        fn encode() -> Encoding {
            // SAFETY: the encoding of NSRange on 64-bit macOS.
            unsafe { Encoding::from_str("{_NSRange=QQ}") }
        }
    }

    // SAFETY: main thread (a menu action); plain Foundation and AppKit calls on objects
    // made here, all of them autoreleased or owned by the panel.
    unsafe {
        let string = |s: &str| -> *mut Object {
            let c = std::ffi::CString::new(s).unwrap_or_default();
            msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()]
        };
        let (text, links) = credits_text();
        let total = text.encode_utf16().count();

        let credits: *mut Object = msg_send![class!(NSMutableAttributedString), alloc];
        let credits: *mut Object = msg_send![credits, initWithString: string(&text)];
        let credits: *mut Object = msg_send![credits, autorelease];

        // Centred, small, and in the secondary label colour, which follows light and dark.
        let style: *mut Object = msg_send![class!(NSMutableParagraphStyle), new];
        let style: *mut Object = msg_send![style, autorelease];
        let _: () = msg_send![style, setAlignment: 1isize]; // NSTextAlignmentCenter
        let _: () = msg_send![style, setLineSpacing: 2.0f64];
        let font: *mut Object = msg_send![class!(NSFont), systemFontOfSize: 11.0f64];
        let colour: *mut Object = msg_send![class!(NSColor), secondaryLabelColor];
        let all = NSRange {
            location: 0,
            length: total,
        };
        let _: () = msg_send![credits, addAttribute: string("NSFont") value: font range: all];
        let _: () = msg_send![credits, addAttribute: string("NSColor") value: colour range: all];
        let _: () =
            msg_send![credits, addAttribute: string("NSParagraphStyle") value: style range: all];
        for (start, length, url) in links {
            let address: *mut Object = msg_send![class!(NSURL), URLWithString: string(url)];
            if address.is_null() {
                continue;
            }
            let range = NSRange {
                location: start,
                length,
            };
            let _: () =
                msg_send![credits, addAttribute: string("NSLink") value: address range: range];
        }

        // The panel's own keys. "Version" is the build number in brackets: left out, as
        // Nimbra's is the same as its version.
        let version = env!("CARGO_PKG_VERSION");
        let keys = [
            string("ApplicationName"),
            string("ApplicationVersion"),
            string("Version"),
            string("Copyright"),
            string("Credits"),
        ];
        let values = [
            string("Nimbra"),
            string(version),
            string(""),
            string(""),
            credits,
        ];
        let options: *mut Object = msg_send![class!(NSDictionary),
            dictionaryWithObjects: values.as_ptr()
            forKeys: keys.as_ptr()
            count: keys.len()];

        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, activateIgnoringOtherApps: true];
        let _: () = msg_send![app, orderFrontStandardAboutPanelWithOptions: options];
    }
}

#[cfg(not(target_os = "macos"))]
pub fn show() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_link_covers_its_label() {
        let (text, links) = credits_text();
        let units: Vec<u16> = text.encode_utf16().collect();
        let labels: Vec<String> = links
            .iter()
            .map(|(start, length, _)| String::from_utf16(&units[*start..start + length]).unwrap())
            .collect();
        assert_eq!(labels, ["Source code", "License", "Changelog"]);
        assert!(text.starts_with("© 2026 Luan Gjokaj."));
    }
}
