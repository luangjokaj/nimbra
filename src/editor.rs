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
    Focus,
    Key {
        k: String,
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
        Raw::Focus => EditorEvent::Focused,
        Raw::Key { k } => match k.as_str() {
            "up" => EditorEvent::Up,
            "esc" => EditorEvent::Escape,
            "send" => EditorEvent::Send,
            "save" => EditorEvent::Save,
            "full" => EditorEvent::ToggleFull,
            _ => return None,
        },
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

/// Text for the editor: escaped, one line per `<div>` (as the page itself writes them).
pub fn text_to_html(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.is_empty() {
                "<div><br></div>".to_string()
            } else {
                format!("<div>{}</div>", escape(line))
            }
        })
        .collect()
}

/// The HTML part of an outgoing message: what was written, then the quoted earlier
/// message (kept as text) in Gmail's usual quote block.
pub fn message_html(written: &str, quoted: Option<&str>) -> String {
    let mut html = format!("<div dir=\"ltr\">{written}</div>");
    if let Some(quoted) = quoted.filter(|q| !q.trim().is_empty()) {
        let mut lines = quoted.lines().peekable();
        html.push_str("<br><div class=\"nimbra-quote\">");
        // A reply: "On …, … wrote:" then the quoted lines (without their "> ").
        if let Some(intro) = lines.next_if(|l| l.trim_end().ends_with("wrote:")) {
            html.push_str(&format!("<div>{}</div>", escape(intro)));
            html.push_str(
                "<blockquote style=\"margin:0 0 0 .8ex;border-left:1px solid #ccc;padding-left:1ex\">",
            );
            for line in lines {
                let line = line
                    .strip_prefix("> ")
                    .or(line.strip_prefix('>'))
                    .unwrap_or(line);
                html.push_str(&escape(line));
                html.push_str("<br>");
            }
            html.push_str("</blockquote>");
        } else {
            for line in lines {
                html.push_str(&escape(line));
                html.push_str("<br>");
            }
        }
        html.push_str("</div>");
    }
    html
}

/// The part of a saved draft's HTML the user wrote: everything before the quote block.
pub fn own_html(html: &str) -> Option<String> {
    let marker = [
        "class=\"gmail_quote",
        "class=\"nimbra-quote",
        "class=gmail_quote",
    ]
    .iter()
    .filter_map(|m| html.find(m))
    .min()?;
    let start = html[..marker].rfind('<')?;
    Some(html[..start].to_string())
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

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
            getrandom::fill(&mut nonce).ok()?;
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
        pub fn exec(&self, _: &str) {}
        pub fn theme(&self, _: serde_json::Value) {}
        pub fn focus(&self) {}
        pub fn blur(&self) {}
        pub fn set_bounds(&self, _: f32, _: f32, _: f32, _: f32) {}
        pub fn set_visible(&self, _: bool) {}
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
        assert!(parse(r#"{"t":"navigate","to":"https://x"}"#).is_none());
        assert!(parse("not json").is_none());
    }

    #[test]
    fn outgoing_html_escapes_and_quotes() {
        assert_eq!(
            text_to_html("a<b>\n\nc"),
            "<div>a&lt;b&gt;</div><div><br></div><div>c</div>"
        );
        let html = message_html(
            "<div>Hi</div>",
            Some("On Tue, Ana <a@x.co> wrote:\n> old <tag>\n> more"),
        );
        assert!(html.starts_with("<div dir=\"ltr\"><div>Hi</div></div>"));
        assert!(html.contains("On Tue, Ana &lt;a@x.co&gt; wrote:"));
        assert!(html.contains("<blockquote") && html.contains("old &lt;tag&gt;<br>more<br>"));
        assert_eq!(message_html("x", None), "<div dir=\"ltr\">x</div>");
    }

    #[test]
    fn a_drafts_own_part_stops_at_the_quote() {
        let html = "<div dir=\"ltr\"><b>Hi</b></div><br><div class=\"gmail_quote\">old</div>";
        assert_eq!(
            own_html(html).as_deref(),
            Some("<div dir=\"ltr\"><b>Hi</b></div><br>")
        );
        assert_eq!(own_html("<div>no quote</div>"), None);
    }
}
