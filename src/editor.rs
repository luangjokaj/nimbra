//! The message editor: formatted text (bold, italic, lists, links, quotes) with real line
//! wrapping, in a system web view running our own page (`editor.html`).
//!
//! The page is the only script that runs: the Content-Security-Policy allows scripts with a
//! fresh random nonce only, so inline handlers in loaded or pasted markup never run.
//! Everything loaded or pasted is filtered down to plain formatting by the page itself.
//! Nothing remote loads, the view can never navigate, and messages back to the app are
//! checked here before they are used.

use std::path::PathBuf;

use futures::channel::mpsc::UnboundedSender;
use serde::Deserialize;

pub use nimbra_core::rich::text_to_html;

/// What the editor page tells the app.
#[derive(Debug)]
pub enum EditorEvent {
    /// The page has loaded and can take content.
    Ready,
    /// The content of composer `session`, as HTML and as plain text. `edited` is false for
    /// the copy sent right after loading.
    Content {
        session: u64,
        html: String,
        text: String,
        edited: bool,
    },
    /// The formatting at the cursor: bold, italic, underline, bullets, numbers.
    Format {
        session: u64,
        state: [bool; 5],
    },
    /// The loaded content has been drawn: the view can be shown without a blank frame.
    Painted,
    /// The editor got keyboard focus.
    Focused,
    /// Arrow up at the very start (or Shift-Tab): go to the Subject field.
    Up,
    /// Escape in the editor.
    Escape,
    /// ⌘↵, ⌘S or ⌘⇧F in the editor.
    Send,
    Save,
    ToggleFull,
    /// Files dropped on the editor, to attach.
    Dropped(Vec<PathBuf>),
    /// The handle above the signature was clicked: `at` is the handle's bottom left in the
    /// view, where the menu of signatures goes.
    SignatureMenu {
        session: u64,
        at: (f32, f32),
    },
    /// The HTML source being edited (`EditorView::source`), as typed.
    Source {
        session: u64,
        text: String,
    },
}

/// The largest message the page may send back (HTML of what the user wrote).
const MAX_MESSAGE: usize = 4 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum Raw {
    Ready,
    Change {
        s: u64,
        html: String,
        text: String,
    },
    Loaded {
        s: u64,
        html: String,
        text: String,
    },
    State {
        s: u64,
        b: bool,
        i: bool,
        u: bool,
        ul: bool,
        ol: bool,
    },
    Painted,
    Focus,
    Key {
        k: String,
    },
    Sig {
        s: u64,
        x: f64,
        y: f64,
    },
    Source {
        s: u64,
        text: String,
    },
}

/// Turns a message from the page into an event; anything unexpected is dropped.
fn parse(body: &str) -> Option<EditorEvent> {
    if body.len() > MAX_MESSAGE {
        return None;
    }
    Some(match serde_json::from_str::<Raw>(body).ok()? {
        Raw::Ready => EditorEvent::Ready,
        Raw::Change { s, html, text } => EditorEvent::Content {
            session: s,
            html,
            text,
            edited: true,
        },
        Raw::Loaded { s, html, text } => EditorEvent::Content {
            session: s,
            html,
            text,
            edited: false,
        },
        Raw::State { s, b, i, u, ul, ol } => EditorEvent::Format {
            session: s,
            state: [b, i, u, ul, ol],
        },
        Raw::Painted => EditorEvent::Painted,
        Raw::Focus => EditorEvent::Focused,
        Raw::Key { k } => match k.as_str() {
            "up" => EditorEvent::Up,
            "esc" => EditorEvent::Escape,
            "send" => EditorEvent::Send,
            "save" => EditorEvent::Save,
            "full" => EditorEvent::ToggleFull,
            _ => return None,
        },
        Raw::Sig { s, x, y } => {
            // A place in the view, which is never far from its origin.
            let place =
                |v: f64| (v.is_finite() && (0. ..=100_000.).contains(&v)).then_some(v as f32);
            EditorEvent::SignatureMenu {
                session: s,
                at: (place(x)?, place(y)?),
            }
        }
        Raw::Source { s, text } => EditorEvent::Source { session: s, text },
    })
}

/// The editor page with a fresh nonce and the bundled fonts.
fn page(nonce: &str) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    static REGULAR: &[u8] = include_bytes!("../assets/fonts/Geist-Regular.ttf");
    static BOLD: &[u8] = include_bytes!("../assets/fonts/Geist-SemiBold.ttf");
    include_str!("editor.html")
        .replace("__NONCE__", nonce)
        .replace("__REGULAR__", &STANDARD.encode(REGULAR))
        .replace("__BOLD__", &STANDARD.encode(BOLD))
}

// The HTML part sent and a draft's signature: the engine's (both apps send the same).
pub use nimbra_core::composing::{message_html, own_html, split_signature};

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod platform {
    use super::{EditorEvent, UnboundedSender, page, parse};
    use wry::dpi::{LogicalPosition, LogicalSize};
    use wry::{DragDropEvent, NewWindowResponse, Rect, WebView, WebViewBuilder};

    pub struct EditorView {
        view: WebView,
    }

    impl EditorView {
        /// Creates the (hidden) editor as a child of the app window. Events from the page
        /// arrive on `events`.
        pub fn new(window: &gpui::Window, events: UnboundedSender<EditorEvent>) -> Option<Self> {
            let mut nonce = [0u8; 18];
            getrandom::getrandom(&mut nonce).ok()?;
            let nonce: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
            let drops = events.clone();
            let view = WebViewBuilder::new()
                .with_html(page(&nonce))
                .with_incognito(true)
                .with_devtools(false)
                .with_visible(false)
                .with_ipc_handler(move |request| {
                    if let Some(event) = parse(request.body()) {
                        let _ = events.unbounded_send(event);
                    }
                })
                .with_drag_drop_handler(move |event| match event {
                    // Files become attachments instead of being opened by the page.
                    DragDropEvent::Drop { paths, .. } if !paths.is_empty() => {
                        let _ = drops.unbounded_send(EditorEvent::Dropped(paths));
                        true
                    }
                    DragDropEvent::Enter { paths, .. } => !paths.is_empty(),
                    _ => false,
                })
                // Our page arrives as about:blank; nothing may replace it. Links in a
                // message being written are not followed.
                .with_navigation_handler(|link| link == "about:blank")
                .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
                .build_as_child(window)
                .ok()?;
            #[cfg(target_os = "macos")]
            {
                use wry::WebViewExtMacOS as _;
                let webview = view.webview();
                // SAFETY: main thread, the WKWebView wry has just made.
                unsafe {
                    crate::webkeys::mark_editor(&*webview as *const _ as *mut objc::runtime::Object)
                };
            }
            Some(Self { view })
        }

        fn call(&self, function: &str, args: &[serde_json::Value]) {
            let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            let _ = self.view.evaluate_script(&format!(
                "window.nimbra && window.nimbra.{function}({})",
                args.join(",")
            ));
        }

        /// Shows composer `session`'s content and the quoted message below it.
        pub fn load(&self, session: u64, html: &str, quoted: &str, focus: bool) {
            self.call(
                "load",
                &[session.into(), html.into(), quoted.into(), focus.into()],
            );
            if focus {
                let _ = self.view.focus();
            }
        }

        /// Shows `html` (from `signature::for_display`) as the signature, under a handle
        /// that opens the menu of signatures. Empty: no signature, the handle only.
        pub fn signature(&self, html: &str) {
            self.call("signature", &[html.into()]);
        }

        /// Shows `text` as editable HTML source for `session`, in place of the formatted
        /// text; each change comes back as `EditorEvent::Source`. `load` goes back.
        pub fn source(&self, session: u64, text: &str) {
            self.call("source", &[session.into(), text.into()]);
        }

        /// What the empty editor says.
        pub fn placeholder(&self, text: &str) {
            self.call("placeholder", &[text.into()]);
        }

        /// One of the page's formatting commands ("bold", "link", …).
        pub fn exec(&self, command: &str) {
            let _ = self.view.focus();
            self.call("exec", &[command.into()]);
        }

        /// Colours and layout, as CSS custom properties.
        pub fn theme(&self, vars: serde_json::Value) {
            self.call("theme", &[vars]);
        }

        pub fn focus(&self) {
            let _ = self.view.focus();
            self.call("focus", &[]);
        }

        /// Gives the keyboard back to the app's own view.
        pub fn blur(&self) {
            let _ = self.view.focus_parent();
        }

        pub fn set_bounds(&self, x: f32, y: f32, width: f32, height: f32) {
            let _ = self.view.set_bounds(Rect {
                position: LogicalPosition::new(x as f64, y as f64).into(),
                size: LogicalSize::new(width.max(1.) as f64, height.max(1.) as f64).into(),
            });
        }

        pub fn set_visible(&self, visible: bool) {
            let _ = self.view.set_visible(visible);
        }

        /// See-through (`false`) or not: the view stays on screen and keeps running (a
        /// hidden one stops drawing), but shows nothing until its page has drawn.
        #[allow(unexpected_cfgs)]
        pub fn set_opaque(&self, opaque: bool) {
            #[cfg(target_os = "macos")]
            {
                use objc::{msg_send, sel, sel_impl};
                use wry::WebViewExtMacOS as _;
                let webview = self.view.webview();
                let alpha: f64 = if opaque { 1. } else { 0. };
                // SAFETY: main thread, the live WKWebView wry made (an NSView).
                unsafe {
                    let _: () = msg_send![&*webview as *const _ as *mut objc::runtime::Object,
                        setAlphaValue: alpha];
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = opaque;
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::{EditorEvent, UnboundedSender};

    /// No web view here; the composer keeps its plain-text editor.
    pub struct EditorView;

    impl EditorView {
        pub fn new(_: &gpui::Window, _: UnboundedSender<EditorEvent>) -> Option<Self> {
            None
        }
        pub fn load(&self, _: u64, _: &str, _: &str, _: bool) {}
        pub fn signature(&self, _: &str) {}
        pub fn source(&self, _: u64, _: &str) {}
        pub fn placeholder(&self, _: &str) {}
        pub fn exec(&self, _: &str) {}
        pub fn theme(&self, _: serde_json::Value) {}
        pub fn focus(&self) {}
        pub fn blur(&self) {}
        pub fn set_bounds(&self, _: f32, _: f32, _: f32, _: f32) {}
        pub fn set_visible(&self, _: bool) {}
        pub fn set_opaque(&self, _: bool) {}
    }
}

pub use platform::EditorView;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_only_runs_its_own_script() {
        let page = page("abc123");
        assert!(page.contains("script-src 'nonce-abc123'"));
        assert!(page.contains("<script nonce=\"abc123\">"));
        assert!(page.contains("default-src 'none'") && page.contains("img-src data:;"));
        assert_eq!(page.matches("<script").count(), 1);
        assert!(!page.contains("__"));
    }

    #[test]
    fn page_messages_are_checked() {
        assert!(matches!(
            parse(r#"{"t":"ready"}"#),
            Some(EditorEvent::Ready)
        ));
        assert!(matches!(
            parse(r#"{"t":"change","s":3,"html":"<b>x</b>","text":"x"}"#),
            Some(EditorEvent::Content {
                session: 3,
                edited: true,
                ..
            })
        ));
        assert!(matches!(
            parse(r#"{"t":"key","k":"up"}"#),
            Some(EditorEvent::Up)
        ));
        assert!(parse(r#"{"t":"key","k":"rm -rf"}"#).is_none());
        assert!(matches!(
            parse(r#"{"t":"sig","s":2,"x":20.5,"y":180}"#),
            Some(EditorEvent::SignatureMenu {
                session: 2,
                at: (20.5, 180.)
            })
        ));
        assert!(parse(r#"{"t":"sig","s":2,"x":-5,"y":180}"#).is_none());
        assert!(parse(r#"{"t":"sig","s":2,"x":"20","y":180}"#).is_none());
        assert!(parse(r#"{"t":"sig","s":2,"x":1e300,"y":1}"#).is_none());
        assert!(parse(r#"{"t":"sig","s":2}"#).is_none());
        assert!(matches!(
            parse(r#"{"t":"source","s":4,"text":"<b>x</b>"}"#),
            Some(EditorEvent::Source { session: 4, .. })
        ));
        assert!(parse(r#"{"t":"source","s":4}"#).is_none());
        assert!(parse(r#"{"t":"navigate","to":"https://x"}"#).is_none());
        assert!(parse("not json").is_none());
    }
}
