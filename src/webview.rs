//! "Show original": the email's own HTML in a system web view placed inside the window.
//!
//! Email HTML is untrusted, so the view is locked down:
//! - JavaScript is disabled.
//! - A Content-Security-Policy blocks everything remote: no scripts, frames, fonts, media,
//!   form posts or `<base>` tricks. Remote images load only after the user asks.
//! - It can never navigate. Clicked links are checked and handed to the system browser;
//!   pop-ups are refused.
//! - It runs in a non-persistent (incognito) store, so mail cannot set lasting cookies.

/// Puts the policy above in front of the email's own document.
///
/// The email is left as the sender wrote it: no wrapper element, and only zero-specificity
/// defaults (see [`DEFAULTS`]), because anything stronger changes how table-based email
/// layouts size their columns. The
/// policy `<meta>` comes first, so it lands in `<head>` and applies before anything in the
/// message is parsed; the message's own `<head>` (including its `color-scheme` and its
/// dark-mode styles) then merges into the same head.
/// What a message looks like when it brings no styling of its own: a readable sans-serif
/// with some margin, as mail clients show it, not the browser's bare serif page.
///
/// `:where()` gives these rules zero specificity, so anything the email sets wins, and they
/// only touch inherited text properties and the page margin, never table or image sizing.
const DEFAULTS: &str = ":where(html){font:14px/1.5 -apple-system,BlinkMacSystemFont,'Helvetica Neue',Arial,sans-serif}\
:where(body){margin:20px 28px}:where(pre,code,tt){font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:0.92em}";

pub fn document(body: &str, images: bool) -> String {
    let img = if images { "data: https:" } else { "data:" };
    format!(
        "<!doctype html><meta charset=\"utf-8\">\
         <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; \
         img-src {img}; style-src 'unsafe-inline'; form-action 'none'; base-uri 'none'\">\
         <style>{DEFAULTS}</style>{body}"
    )
}

/// Only web and mail links may leave the app.
fn is_external_link(link: &str) -> bool {
    url::Url::parse(link).is_ok_and(|url| matches!(url.scheme(), "http" | "https" | "mailto"))
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod platform {
    use super::{document, is_external_link};
    use wry::dpi::{LogicalPosition, LogicalSize};
    use wry::{NewWindowResponse, Rect, WebView, WebViewBuilder};

    pub struct HtmlView {
        view: WebView,
    }

    impl HtmlView {
        /// Creates the (hidden) web view as a child of the app window.
        pub fn new(window: &gpui::Window) -> Option<Self> {
            let view = WebViewBuilder::new()
                .with_html(document("", false))
                .with_javascript_disabled()
                .with_incognito(true)
                .with_devtools(false)
                .with_visible(false)
                .with_navigation_handler(|link| {
                    // The page we load ourselves arrives as about:blank; nothing else may
                    // replace it. Real links open in the browser instead.
                    if link == "about:blank" {
                        return true;
                    }
                    if is_external_link(&link) {
                        open_in_browser(&link);
                    }
                    false
                })
                .with_new_window_req_handler(|link, _| {
                    if is_external_link(&link) {
                        open_in_browser(&link);
                    }
                    NewWindowResponse::Deny
                })
                .build_as_child(window)
                .ok()?;
            Some(Self { view })
        }

        pub fn show(&self, body: &str, images: bool) {
            let _ = self.view.load_html(&document(body, images));
        }

        /// Position and size in window points.
        pub fn set_bounds(&self, x: f32, y: f32, width: f32, height: f32) {
            let _ = self.view.set_bounds(Rect {
                position: LogicalPosition::new(x as f64, y as f64).into(),
                size: LogicalSize::new(width.max(1.) as f64, height.max(1.) as f64).into(),
            });
        }

        pub fn set_visible(&self, visible: bool) {
            let _ = self.view.set_visible(visible);
        }

        /// Makes the page see a light or dark colour scheme, following the app's theme
        /// rather than the system's. Emails that ship dark-mode styles then use them;
        /// emails that do not simply stay as designed.
        #[cfg(target_os = "macos")]
        #[allow(unexpected_cfgs)] // emitted by the objc crate's macros
        pub fn set_dark(&self, dark: bool) {
            use objc::runtime::Object;
            use objc::{class, msg_send, sel, sel_impl};
            use wry::WebViewExtMacOS as _;

            let name = if dark {
                c"NSAppearanceNameDarkAqua"
            } else {
                c"NSAppearanceNameAqua"
            };
            let webview = self.view.webview();
            // SAFETY: called on the main thread with a live WKWebView (an NSView), and the
            // appearance name is one of AppKit's two built-in constants.
            unsafe {
                let view = &*webview as *const _ as *mut Object;
                let name: *mut Object =
                    msg_send![class!(NSString), stringWithUTF8String: name.as_ptr()];
                let appearance: *mut Object =
                    msg_send![class!(NSAppearance), appearanceNamed: name];
                if !appearance.is_null() {
                    let _: () = msg_send![view, setAppearance: appearance];
                }
            }
        }

        #[cfg(target_os = "windows")]
        pub fn set_dark(&self, dark: bool) {
            use wry::WebViewExtWindows as _;
            let theme = if dark {
                wry::Theme::Dark
            } else {
                wry::Theme::Light
            };
            let _ = self.view.set_theme(theme);
        }
    }

    /// Hands a checked link to the system, as a plain argument (never through a shell).
    fn open_in_browser(link: &str) {
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("/usr/bin/open")
            .arg(link)
            .spawn();
        #[cfg(target_os = "windows")]
        let _ = std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", link])
            .spawn();
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    /// No web view on this platform yet; the app keeps the native reader.
    pub struct HtmlView;

    impl HtmlView {
        pub fn new(_window: &gpui::Window) -> Option<Self> {
            None
        }
        pub fn show(&self, _body: &str, _images: bool) {}
        pub fn set_bounds(&self, _x: f32, _y: f32, _width: f32, _height: f32) {}
        pub fn set_visible(&self, _visible: bool) {}
        pub fn set_dark(&self, _dark: bool) {}
    }
}

pub use platform::HtmlView;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_blocks_remote_content_until_images_are_allowed() {
        let blocked = document("<p>Hi</p>", false);
        assert!(blocked.contains("default-src 'none'"));
        assert!(blocked.contains("img-src data:;"));
        assert!(blocked.contains("form-action 'none'") && blocked.contains("base-uri 'none'"));
        assert!(!blocked.contains("script-src"));
        assert!(document("<p>Hi</p>", true).contains("img-src data: https:;"));
        // The policy precedes the message, and the message is not wrapped or restyled.
        let policy = blocked.find("Content-Security-Policy").unwrap();
        assert!(policy < blocked.find("<p>Hi</p>").unwrap());
        assert!(!blocked.contains("<body>"));
        // Our defaults can never outrank the email's own styles or touch layout.
        assert!(
            DEFAULTS
                .split('}')
                .filter(|rule| !rule.is_empty())
                .all(|rule| rule.starts_with(":where("))
        );
        assert!(!DEFAULTS.contains("max-width") && !DEFAULTS.contains("overflow-wrap"));
    }

    #[test]
    fn only_web_and_mail_links_leave_the_app() {
        assert!(is_external_link("https://example.com/a"));
        assert!(is_external_link("mailto:a@b.co"));
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "about:blank",
            "x-apple:foo",
            "not a url",
        ] {
            assert!(!is_external_link(bad), "{bad}");
        }
    }
}
