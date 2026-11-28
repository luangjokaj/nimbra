//! "Show original": the email's own HTML in a system web view placed inside the window.
//!
//! Email HTML is untrusted, so the view is locked down:
//! - The mail's JavaScript is disabled. The one script that can run is Nimbra's own
//!   `dark_mail.js`, in a WebKit world of its own, which only reads and sets colours.
//! - A Content-Security-Policy blocks everything remote: no scripts, frames, fonts, media,
//!   form posts or `<base>` tricks. Remote images load only after the user asks.
//! - It can never navigate. Clicked links are checked and handed to the system browser;
//!   navigations the mail starts by itself (a meta refresh needs no script) go nowhere, and
//!   pop-ups are refused. wry reports only the URL, so telling a click apart asks WebKit or
//!   WebView2 itself.
//! - `<link>` tags are made inert: the policy cannot stop a preconnect, which tells the
//!   sender the mail was opened even with images blocked.
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
/// only touch inherited text properties, never table or image sizing. The body also holds
/// its children's margins (`flow-root`, where they would otherwise run out past it), so its
/// box is as tall as the email, which is how tall the email's view is made (`MEASURE`).
///
/// The one rule that does win is the space around the email: 20pt on every side, as the
/// subject and sender above it have (in line with the toolbar's first icon), whatever margins the email sets (many set none, and
/// their content would touch the edges). It is the page's padding, so the email's own
/// background still fills the view (a body's background spreads to the whole page).
const DEFAULTS: &str = ":where(html){font:14px/1.5 -apple-system,BlinkMacSystemFont,'Helvetica Neue',Arial,sans-serif}\
html{padding:20px!important}body{margin:0!important}:where(body){display:flow-root}\
:where(pre,code,tt){font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:0.92em}";

/// How much an email in its own formatting is zoomed at the mail text size: as much larger
/// or smaller than Default as the reader's text is (1 at Default).
pub fn zoom_for(size: crate::settings::TextSize) -> f32 {
    size.metrics().0 / crate::settings::TextSize::Default.metrics().0
}

/// How tall the email is, in CSS pixels: its body (the page's padding included), however
/// tall the view showing it is; the body holds its children's margins (`DEFAULTS`). A page
/// that still runs past the view (something placed outside the body) is as tall as it
/// scrolls. Run by the app, not the mail; fixed text.
const MEASURE: &str = "(function(){var d=document.documentElement,b=document.body;\
if(!b)return 0;var s=getComputedStyle(d),r=b.getBoundingClientRect(),y=window.scrollY;\
var h=Math.max(r.bottom+y,r.top+y+b.scrollHeight)+parseFloat(s.paddingBottom||0);\
if(d.scrollHeight>window.innerHeight+1)h=Math.max(h,d.scrollHeight);\
return Math.ceil(h);})()";

/// The height `MEASURE` answered, in points at `zoom`.
fn measured(answer: &str, zoom: f32) -> Option<f32> {
    let css: f32 = answer.trim().parse().ok()?;
    (css.is_finite() && css > 0.).then(|| (css * zoom).ceil())
}

pub fn document(body: &str, images: bool) -> String {
    let img = if images { "data: https:" } else { "data:" };
    format!(
        "<!doctype html><meta charset=\"utf-8\">\
         <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; \
         img-src {img}; style-src 'unsafe-inline'; form-action 'none'; base-uri 'none'\">\
         <meta http-equiv=\"x-dns-prefetch-control\" content=\"off\">\
         <style>{DEFAULTS}</style>{}",
        without_links(body)
    )
}

/// The mail with each `<link` start tag renamed to `<meta`, which is also empty, also allowed
/// in the head, and ignores `rel` and `href`. Nothing is lost, as the policy blocks all a
/// link could load. No parser is needed: a tag's name can only be written out literally
/// (entities and NULs cannot spell one), in any letter case.
fn without_links(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut done = 0;
    for (at, _) in html.match_indices('<') {
        let tag = &html.as_bytes()[at + 1..];
        if tag.len() >= 4
            && tag[..4].eq_ignore_ascii_case(b"link")
            && matches!(
                tag.get(4),
                None | Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'/' | b'>')
            )
        {
            out.push_str(&html[done..at]);
            out.push_str("<meta");
            done = at + 5;
        }
    }
    out.push_str(&html[done..]);
    out
}

/// Only web and mail links may leave the app.
fn is_external_link(link: &str) -> bool {
    url::Url::parse(link).is_ok_and(|url| matches!(url.scheme(), "http" | "https" | "mailto"))
}

/// Nimbra's script that gives light mail a dark page; see the file.
#[cfg(any(target_os = "macos", test))]
const DARK_MAIL: &str = include_str!("dark_mail.js");

/// A navigation is handed to the system only for a followed link to the web or mail.
fn leaves_app(link: &str, followed: bool) -> bool {
    followed && is_external_link(link)
}

/// What WebView2 may do with a navigation.
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, PartialEq)]
enum Navigation {
    /// The document Nimbra handed it.
    Load,
    /// A clicked link, for the system to open. The view stays where it is.
    Open,
    Stop,
}

/// `loads` is how many documents Nimbra handed the view that have not started yet. WebView2
/// starts each as a navigation to a data URL (or about:blank), and counts it as the user's
/// own, as it does everything an app asks of it. So a load is told by its URL and by one
/// being due, never by the click flag; at any other time those URLs go nowhere.
#[cfg(any(target_os = "windows", test))]
fn navigation(link: &str, clicked: bool, loads: u32) -> Navigation {
    if loads > 0 && (link == "about:blank" || link.starts_with("data:text/html")) {
        Navigation::Load
    } else if leaves_app(link, clicked) {
        Navigation::Open
    } else {
        Navigation::Stop
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod platform {
    use super::document;
    #[cfg(target_os = "macos")]
    use super::leaves_app;
    #[cfg(target_os = "macos")]
    use wry::NewWindowResponse;
    use wry::dpi::{LogicalPosition, LogicalSize};
    use wry::{Rect, WebView, WebViewBuilder};

    /// x, y, width and height, in window points.
    type Bounds = (f32, f32, f32, f32);

    pub struct HtmlView {
        view: WebView,
        /// Documents handed over that have not started loading (see `navigation`).
        #[cfg(target_os = "windows")]
        loads: std::rc::Rc<std::cell::Cell<u32>>,
        /// Where the view is (window points), and the hole cut in it, as last applied.
        frame: std::cell::Cell<Bounds>,
        hole: std::cell::Cell<Option<Bounds>>,
        applied: std::cell::Cell<Option<(Bounds, Option<Bounds>)>>,
        /// Whether the view is on screen (`set_visible`), so ⌘C goes to it only then.
        shown: std::cell::Cell<bool>,
        /// Whether its column has put it in place (`place`). Until then it sits where wry
        /// made it, at the window's top-left corner, so it is not put on screen.
        placed: std::cell::Cell<bool>,
        /// The zoom last applied (`set_zoom`).
        zoom: std::cell::Cell<f32>,
        /// The account of the mail shown, for the `mailto:` links in it.
        #[cfg(target_os = "macos")]
        account: std::rc::Rc<std::cell::RefCell<Option<String>>>,
        /// The view that clips it to a scrolling column (`place`), made the first time.
        #[cfg(target_os = "macos")]
        clip: std::cell::Cell<Option<usize>>,
    }

    impl HtmlView {
        /// Creates the (hidden) web view as a child of the app window.
        pub fn new(window: &gpui::Window) -> Option<Self> {
            Self::build(window, None)
        }

        /// As `new`, with `loaded` called each time a page has loaded (its images too).
        pub fn with_loaded(window: &gpui::Window, loaded: impl Fn() + 'static) -> Option<Self> {
            Self::build(window, Some(Box::new(loaded)))
        }

        fn build(window: &gpui::Window, loaded: Option<Box<dyn Fn()>>) -> Option<Self> {
            let builder = WebViewBuilder::new()
                .with_javascript_disabled()
                .with_incognito(true)
                .with_devtools(false)
                .with_visible(false);
            // Windows takes neither of wry's handlers, as they are not told whether it was a
            // click: `click::watch` has its own. (wry still refuses every pop-up.) Nor does it
            // load before those are in place, or download what a mail points at.
            #[cfg(target_os = "windows")]
            let builder = builder.with_download_started_handler(|_, _| false);
            #[cfg(target_os = "macos")]
            let account = std::rc::Rc::new(std::cell::RefCell::new(None::<String>));
            #[cfg(target_os = "macos")]
            let follow = {
                let (account, window) = (account.clone(), window.window_handle());
                move |link: &str| {
                    let from = account.borrow().clone();
                    if leaves_app(link, click::followed())
                        && !crate::integration::mailto_from_mail(link, from, Some(window))
                    {
                        open_in_browser(link);
                    }
                }
            };
            #[cfg(target_os = "macos")]
            let builder = builder
                .with_html(document("", false))
                .with_navigation_handler({
                    let follow = follow.clone();
                    move |link| {
                        // The page we load ourselves arrives as about:blank; nothing else may
                        // replace it. Real links open in the browser instead.
                        if link == "about:blank" {
                            return true;
                        }
                        follow(&link);
                        false
                    }
                })
                .with_new_window_req_handler(move |link, _| {
                    follow(&link);
                    NewWindowResponse::Deny
                });
            let builder = match loaded {
                Some(loaded) => builder.with_on_page_load_handler(move |event, _| {
                    if matches!(event, wry::PageLoadEvent::Finished) {
                        loaded();
                    }
                }),
                None => builder,
            };
            let view = builder.build_as_child(window).ok()?;
            // Without the check there is no view at all (the reader shows the mail instead).
            #[cfg(target_os = "windows")]
            let loads = click::watch(&view)?;
            #[cfg(target_os = "macos")]
            {
                use wry::WebViewExtMacOS as _;
                let webview = view.webview();
                // SAFETY: main thread, a live WKWebView that wry has just given its delegate.
                unsafe { click::watch(&*webview as *const _ as *mut objc::runtime::Object) };
            }
            Some(Self {
                view,
                #[cfg(target_os = "windows")]
                loads,
                frame: std::cell::Cell::new((0., 0., 0., 0.)),
                shown: std::cell::Cell::new(false),
                placed: std::cell::Cell::new(false),
                zoom: std::cell::Cell::new(1.),
                hole: std::cell::Cell::new(None),
                applied: std::cell::Cell::new(None),
                #[cfg(target_os = "macos")]
                account,
                #[cfg(target_os = "macos")]
                clip: std::cell::Cell::new(None),
            })
        }

        /// Leaves a rounded hole in the view (window points) where something of the app's
        /// own sits on top of the email, such as a floating reply: the app shows through it
        /// and clicks there reach the app. `None` closes it.
        pub fn set_hole(&self, hole: Option<(f32, f32, f32, f32)>) {
            self.hole.set(hole);
            self.apply_hole();
        }

        #[cfg(target_os = "macos")]
        fn apply_hole(&self) {
            use wry::WebViewExtMacOS as _;
            let state = (self.frame.get(), self.hole.get());
            if self.applied.get() == Some(state) {
                return;
            }
            self.applied.set(Some(state));
            let (frame, hole) = state;
            // In the view's own coordinates, from its top-left corner.
            let local = hole.and_then(|(x, y, w, h)| {
                let (x, y) = (x - frame.0, y - frame.1);
                (w > 0. && h > 0. && x < frame.2 && y < frame.3 && x + w > 0. && y + h > 0.)
                    .then_some((x as f64, y as f64, w as f64, h as f64))
            });
            let webview = self.view.webview();
            // SAFETY: main thread, live WKWebView; see `hole::set`.
            unsafe { hole::set(&*webview as *const _ as *mut objc::runtime::Object, local) };
        }

        #[cfg(not(target_os = "macos"))]
        fn apply_hole(&self) {}

        /// Loads a mail of `account` (its id; `None` for a page that is not one).
        pub fn show(&self, body: &str, images: bool, account: Option<&str>) {
            #[cfg(target_os = "macos")]
            self.account.replace(account.map(str::to_string));
            #[cfg(not(target_os = "macos"))]
            let _ = account;
            #[cfg(target_os = "windows")]
            self.loads.set(self.loads.get() + 1);
            let shown = self.view.load_html(&document(body, images));
            #[cfg(target_os = "windows")]
            if shown.is_err() {
                self.loads.set(self.loads.get().saturating_sub(1));
            }
            let _ = shown;
        }

        /// Position and size in window points.
        pub fn set_bounds(&self, x: f32, y: f32, width: f32, height: f32) {
            self.frame.set((x, y, width, height));
            if self.hole.get().is_some() || self.applied.get().is_some_and(|(_, h)| h.is_some()) {
                self.apply_hole();
            }
            let _ = self.view.set_bounds(Rect {
                position: LogicalPosition::new(x as f64, y as f64).into(),
                size: LogicalSize::new(width.max(1.) as f64, height.max(1.) as f64).into(),
            });
        }

        /// Places the whole email at `full` (window points) and shows only the part of it
        /// inside `clip`, such as the scrolling column it is in: nothing outside `clip` is
        /// drawn or takes a click. Hidden when nothing of it is inside.
        #[cfg(target_os = "macos")]
        pub fn place(&self, full: (f32, f32, f32, f32), clip: (f32, f32, f32, f32)) {
            use objc::runtime::Object;
            use wry::WebViewExtMacOS as _;
            let (x, y, w, h) = full;
            let left = x.max(clip.0);
            let top = y.max(clip.1);
            let right = (x + w).min(clip.0 + clip.2);
            let bottom = (y + h).min(clip.1 + clip.3);
            let seen = (left, top, (right - left).max(0.), (bottom - top).max(0.));
            self.frame.set(full);
            let webview = self.view.webview();
            let webview = &*webview as *const _ as *mut Object;
            // SAFETY: main thread, a live WKWebView inside the app's view; see `clip`.
            unsafe {
                let container = match self.clip.get() {
                    Some(container) => container as *mut Object,
                    None => {
                        let Some(container) = clip::wrap(webview) else {
                            return;
                        };
                        self.clip.set(Some(container as usize));
                        container
                    }
                };
                clip::place(container, webview, full, seen);
                clip::show(container, self.shown.get() && seen.2 > 0. && seen.3 > 0.);
            }
            self.now_placed();
            if self.hole.get().is_some() || self.applied.get().is_some_and(|(_, h)| h.is_some()) {
                self.apply_hole();
            }
        }

        /// As above: WebView2 cannot be clipped, so it covers what of the email is inside.
        #[cfg(target_os = "windows")]
        pub fn place(&self, full: (f32, f32, f32, f32), clip: (f32, f32, f32, f32)) {
            let (x, y, w, h) = full;
            let top = y.max(clip.1);
            let bottom = (y + h).min(clip.1 + clip.3);
            self.set_bounds(x, top, w, (bottom - top).max(0.));
            self.now_placed();
        }

        /// The first time it is in place: shown now if it was asked to be meanwhile.
        fn now_placed(&self) {
            if !self.placed.replace(true) && self.shown.get() {
                let _ = self.view.set_visible(true);
            }
        }

        /// A picture of the whole email as it is drawn now (TIFF), for `answer`; `None` when
        /// WebKit made none. The app draws it in the email's place while things move
        /// (`reading::email_frame`), as the app's drawing and the web view's reach the
        /// screen at different times.
        #[cfg(target_os = "macos")]
        #[allow(unexpected_cfgs)]
        pub fn snapshot(&self, answer: impl Fn(Option<Vec<u8>>) + 'static) {
            use objc::runtime::Object;
            use objc::{msg_send, sel, sel_impl};
            use wry::WebViewExtMacOS as _;
            let webview = self.view.webview();
            let webview = &*webview as *const _ as *mut Object;
            let handler = block::ConcreteBlock::new(move |image: *mut Object, _: *mut Object| {
                // SAFETY: WebKit calls this on the main thread with an NSImage or nil.
                answer(unsafe { super::tiff_of(image) });
            })
            .copy();
            let none: *mut Object = std::ptr::null_mut();
            // SAFETY: main thread, a live WKWebView; the block is copied to the heap and
            // WebKit keeps it until it has called it.
            unsafe {
                let _: () = msg_send![webview, takeSnapshotWithConfiguration: none completionHandler: &*handler];
            }
        }

        #[cfg(target_os = "windows")]
        pub fn snapshot(&self, answer: impl Fn(Option<Vec<u8>>) + 'static) {
            answer(None);
        }

        /// Asks how tall the email is (points, at the current zoom); `answer` gets it once
        /// the page has worked it out.
        pub fn measure(&self, answer: impl Fn(f32) + Send + 'static) {
            let zoom = self.zoom.get();
            let _ = self
                .view
                .evaluate_script_with_callback(super::MEASURE, move |result| {
                    if let Some(height) = super::measured(&result, zoom) {
                        answer(height);
                    }
                });
        }

        /// Whether the keyboard is in the email (it was clicked last), for ⌘C.
        #[cfg(target_os = "macos")]
        #[allow(unexpected_cfgs)]
        pub fn focused(&self) -> bool {
            use objc::runtime::Object;
            use objc::{msg_send, sel, sel_impl};
            use wry::WebViewExtMacOS as _;
            if !self.shown.get() {
                return false;
            }
            let webview = self.view.webview();
            let webview = &*webview as *const _ as *mut Object;
            // SAFETY: main thread, a live WKWebView; the first responder is checked to be a
            // view before it is asked about its place.
            unsafe {
                let window: *mut Object = msg_send![webview, window];
                if window.is_null() {
                    return false;
                }
                let first: *mut Object = msg_send![window, firstResponder];
                if first.is_null() {
                    return false;
                }
                let is_view: bool = msg_send![first, isKindOfClass: objc::class!(NSView)];
                is_view && msg_send![first, isDescendantOf: webview]
            }
        }

        #[cfg(target_os = "windows")]
        pub fn focused(&self) -> bool {
            self.shown.get()
        }

        /// Sizes the email up or down with the mail text size (1 is as the sender made it).
        pub fn set_zoom(&self, zoom: f32) {
            if self.zoom.replace(zoom) != zoom {
                // Unsupported before macOS 11; the email then shows at its own size.
                let _ = self.view.zoom(f64::from(zoom));
            }
        }

        pub fn set_visible(&self, visible: bool) {
            self.shown.set(visible);
            let _ = self.view.set_visible(visible && self.placed.get());
            #[cfg(target_os = "macos")]
            if let Some(container) = self.clip.get() {
                // SAFETY: main thread, the live container `place` made.
                unsafe { clip::show(container as *mut objc::runtime::Object, visible) };
            }
            // A hidden view keeps no hole (its click-through view would block the app).
            if !visible && self.applied.get().is_some_and(|(_, hole)| hole.is_some()) {
                self.hole.set(None);
                self.apply_hole();
            }
        }

        /// ⌘C: copies what is selected in the email, as WebKit does itself (text with its
        /// formatting). The app takes ⌘C as its own key before the view sees it, so it is
        /// handed on here. False when the view isn't on screen.
        pub fn copy(&self) -> bool {
            if !self.shown.get() {
                return false;
            }
            #[cfg(target_os = "macos")]
            #[allow(unexpected_cfgs)] // emitted by the objc crate's macros
            {
                use objc::runtime::Object;
                use objc::{msg_send, sel, sel_impl};
                use wry::WebViewExtMacOS as _;
                let webview = self.view.webview();
                let webview = &*webview as *const _ as *mut Object;
                let nothing: *mut Object = std::ptr::null_mut();
                // SAFETY: main thread, a live WKWebView; copy: is its NSResponder action.
                unsafe {
                    let _: () = msg_send![webview, copy: nothing];
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = self.view.evaluate_script("document.execCommand('copy')");
            }
            true
        }

        /// Opens the system print panel for what the view shows.
        pub fn print(&self) {
            let _ = self.view.print();
        }

        /// Makes the page see a light or dark colour scheme, following the app's theme
        /// rather than the system's. Emails that ship dark-mode styles then use them;
        /// with `darken`, light emails get a dark page too (macOS only). Takes effect at
        /// the next `show`.
        #[cfg(target_os = "macos")]
        #[allow(unexpected_cfgs)] // emitted by the objc crate's macros
        pub fn set_dark(&self, dark: bool, darken: bool) {
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
                let manager = self.view.manager();
                let manager = &*manager as *const _ as *mut Object;
                let darkened = restyle::install(manager, dark && darken);
                // See-through while darkening: before the page's first paint WebKit would
                // show white, and the app's dark surface is what lies under the view.
                let draws = sel!(_setDrawsBackground:);
                let can: bool = msg_send![view, respondsToSelector: draws];
                if can {
                    let _: () = msg_send![view, _setDrawsBackground: !darkened];
                }
            }
        }

        #[cfg(target_os = "windows")]
        pub fn set_dark(&self, dark: bool, _darken: bool) {
            use wry::WebViewExtWindows as _;
            let theme = if dark {
                wry::Theme::Dark
            } else {
                wry::Theme::Light
            };
            let _ = self.view.set_theme(theme);
        }
    }

    /// Whether the navigation being decided is a link someone followed. wry's handler only
    /// gets the URL, so the view's navigation delegate is given a subclass that notes WebKit's
    /// navigation type and then lets wry decide as before. Without it no link leaves the app.
    #[cfg(target_os = "macos")]
    #[allow(unexpected_cfgs)]
    mod click {
        use std::cell::Cell;

        use objc::declare::ClassDecl;
        use objc::runtime::{Class, Object, Sel};
        use objc::{msg_send, sel, sel_impl};

        /// WKNavigationTypeLinkActivated. A refresh, a redirect or a load is `Other` (-1).
        const LINK_ACTIVATED: isize = 0;
        const GATE: &str = "NimbraClickGate";

        #[link(name = "objc")]
        unsafe extern "C" {
            fn object_setClass(object: *mut Object, class: *const Class) -> *const Class;
        }

        thread_local! {
            static FOLLOWED: Cell<bool> = const { Cell::new(false) };
        }

        pub(super) fn followed() -> bool {
            FOLLOWED.with(Cell::get)
        }

        extern "C" fn decide(
            this: &Object,
            _: Sel,
            view: *mut Object,
            action: *mut Object,
            reply: *mut Object,
        ) {
            let Some(wrys) = Class::get(GATE).and_then(Class::superclass) else {
                return;
            };
            // SAFETY: WebKit calls this on the main thread with live objects; they are passed
            // on untouched to the method this overrides, which answers before it returns.
            unsafe {
                let kind: isize = msg_send![action, navigationType];
                FOLLOWED.with(|f| f.set(kind == LINK_ACTIVATED));
                let _: () = msg_send![super(this, wrys),
                    webView: view
                    decidePolicyForNavigationAction: action
                    decisionHandler: reply];
                FOLLOWED.with(|f| f.set(false));
            }
        }

        /// # Safety
        /// Main thread only, with a live WKWebView built by wry.
        pub(super) unsafe fn watch(view: *mut Object) {
            unsafe {
                let delegate: *mut Object = msg_send![view, navigationDelegate];
                if delegate.is_null() {
                    return;
                }
                let gate = match Class::get(GATE) {
                    Some(gate) => gate,
                    None => {
                        let Some(mut decl) = ClassDecl::new(GATE, (*delegate).class()) else {
                            return;
                        };
                        decl.add_method(
                            sel!(webView:decidePolicyForNavigationAction:decisionHandler:),
                            decide
                                as extern "C" fn(
                                    &Object,
                                    Sel,
                                    *mut Object,
                                    *mut Object,
                                    *mut Object,
                                ),
                        );
                        decl.register()
                    }
                };
                // Only a delegate of the class the gate extends: it adds no fields to it.
                if gate.superclass() == Some((*delegate).class()) {
                    object_setClass(delegate, gate);
                }
            }
        }
    }

    /// WebView2 says whether a navigation was the user's; wry does not pass that on. So this
    /// is the view's only navigation handler. Each one is cancelled until shown to be a load
    /// of ours, and whatever cannot be read stays cancelled.
    #[cfg(target_os = "windows")]
    mod click {
        use std::cell::Cell;
        use std::rc::Rc;

        use webview2_com::{NavigationStartingEventHandler, NewWindowRequestedEventHandler};
        use windows::core::{BOOL, PWSTR};
        use wry::{WebView, WebViewExtWindows as _};

        use super::super::{Navigation, navigation};
        use super::open_in_browser;

        pub(super) fn watch(view: &WebView) -> Option<Rc<Cell<u32>>> {
            let loads = Rc::new(Cell::new(0));
            let webview = view.webview();
            let mut token = 0i64;
            let pending = loads.clone();
            let starting = NavigationStartingEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                // SAFETY: WebView2 calls this on the UI thread with live arguments. The URI
                // is its allocation, which `take_pwstr` frees.
                unsafe {
                    args.SetCancel(true)?;
                    let mut uri = PWSTR::null();
                    args.Uri(&mut uri)?;
                    let uri = webview2_com::take_pwstr(uri);
                    let mut clicked = BOOL::default();
                    args.IsUserInitiated(&mut clicked)?;
                    match navigation(&uri, clicked.as_bool(), pending.get()) {
                        Navigation::Load => {
                            args.SetCancel(false)?;
                            pending.set(pending.get().saturating_sub(1));
                        }
                        Navigation::Open => open_in_browser(&uri),
                        Navigation::Stop => {}
                    }
                }
                Ok(())
            }));
            let pop_up = NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                // SAFETY: as above.
                unsafe {
                    args.SetHandled(true)?;
                    let mut uri = PWSTR::null();
                    args.Uri(&mut uri)?;
                    let uri = webview2_com::take_pwstr(uri);
                    let mut clicked = BOOL::default();
                    args.IsUserInitiated(&mut clicked)?;
                    if navigation(&uri, clicked.as_bool(), 0) == Navigation::Open {
                        open_in_browser(&uri);
                    }
                }
                Ok(())
            }));
            // SAFETY: on the UI thread, with the live webview wry has just built.
            unsafe {
                webview.add_NavigationStarting(&starting, &mut token).ok()?;
                webview.add_NewWindowRequested(&pop_up, &mut token).ok()?;
            }
            Some(loads)
        }
    }

    /// Puts `dark_mail.js` in, or takes it out. wry's own script calls always run in the
    /// page's world, so this goes to WebKit directly for a world the mail cannot reach.
    #[cfg(target_os = "macos")]
    #[allow(unexpected_cfgs)]
    mod restyle {
        use objc::runtime::{Class, Object};
        use objc::{msg_send, sel, sel_impl};

        /// Whether the script is in.
        ///
        /// # Safety
        /// Main thread only, with the live WKUserContentController of a view that shows mail.
        pub(super) unsafe fn install(manager: *mut Object, on: bool) -> bool {
            unsafe {
                // This also drops wry's message stub, which the mail view has no use for.
                let _: () = msg_send![manager, removeAllUserScripts];
                if !on {
                    return false;
                }
                let (Some(strings), Some(worlds), Some(scripts), Ok(source)) = (
                    Class::get("NSString"),
                    Class::get("WKContentWorld"),
                    Class::get("WKUserScript"),
                    std::ffi::CString::new(super::super::DARK_MAIL),
                ) else {
                    return false;
                };
                let name: *mut Object =
                    msg_send![strings, stringWithUTF8String: c"Nimbra".as_ptr()];
                let world: *mut Object = msg_send![worlds, worldWithName: name];
                let source: *mut Object = msg_send![strings, stringWithUTF8String: source.as_ptr()];
                if world.is_null() || source.is_null() {
                    return false;
                }
                let script: *mut Object = msg_send![scripts, alloc];
                // WKUserScriptInjectionTimeAtDocumentStart = 0.
                let script: *mut Object = msg_send![script,
                    initWithSource: source
                    injectionTime: 0isize
                    forMainFrameOnly: true
                    inContentWorld: world];
                if script.is_null() {
                    return false;
                }
                let _: () = msg_send![manager, addUserScript: script];
                let _: () = msg_send![script, release];
                true
            }
        }
    }

    /// A web view inside a scrolling column: it sits in a view of its own that clips it,
    /// so the part of the email scrolled out of the column is neither drawn nor clicked.
    #[cfg(target_os = "macos")]
    #[allow(unexpected_cfgs)]
    mod clip {
        use objc::declare::ClassDecl;
        use objc::runtime::{BOOL, Class, Object, Sel, YES};
        use objc::{Encode, Encoding, class, msg_send, sel, sel_impl};

        #[repr(C)]
        #[derive(Clone, Copy)]
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

        /// A plain view, flipped so that its subview's frame reads from the top.
        fn class() -> &'static Class {
            if let Some(class) = Class::get("NimbraClipView") {
                return class;
            }
            extern "C" fn flipped(_: &Object, _: Sel) -> BOOL {
                YES
            }
            let mut decl =
                ClassDecl::new("NimbraClipView", class!(NSView)).expect("class name free");
            // SAFETY: the signature matches -[NSView isFlipped].
            unsafe {
                decl.add_method(
                    sel!(isFlipped),
                    flipped as extern "C" fn(&Object, Sel) -> BOOL,
                );
            }
            decl.register()
        }

        #[link(name = "objc")]
        unsafe extern "C" {
            fn class_addMethod(
                class: *const Class,
                name: Sel,
                imp: extern "C" fn(&Object, Sel, *mut Object),
                types: *const std::ffi::c_char,
            ) -> BOOL;
        }

        /// Up and down scrolling over an email in a column scrolls the column: the email is as
        /// tall as its page, so it has nothing to scroll itself. A sideways scroll stays with
        /// the email (one wider than the column), as does any scroll over another web view.
        extern "C" fn scroll_wheel(this: &Object, _: Sel, event: *mut Object) {
            // SAFETY: AppKit calls this on the main thread with a live view and event.
            unsafe {
                let container: *mut Object = msg_send![this, superview];
                let clipped = !container.is_null() && {
                    let is: bool = msg_send![container, isKindOfClass: class()];
                    is
                };
                if clipped {
                    let dx: f64 = msg_send![event, scrollingDeltaX];
                    let dy: f64 = msg_send![event, scrollingDeltaY];
                    let app: *mut Object = msg_send![container, superview];
                    if dy.abs() >= dx.abs() && !app.is_null() {
                        let _: () = msg_send![app, scrollWheel: event];
                        return;
                    }
                }
                let _: () = msg_send![super(this, class!(WKWebView)), scrollWheel: event];
            }
        }

        /// Gives wry's web view class the `scroll_wheel` above, once. The method is added to
        /// the class, never by changing a view's class: AppKit's key-value observing already
        /// replaces that, and taking its place makes removing the view throw.
        ///
        /// # Safety
        /// Main thread only, with a live WKWebView built by wry.
        unsafe fn take_scrolls(webview: *mut Object) {
            unsafe {
                // What `class` answers is the class itself, also under key-value observing.
                let own: *const Class = msg_send![webview, class];
                if own.is_null() || std::ptr::eq(own, class!(WKWebView)) {
                    return;
                }
                // Refused (and harmless) when the class has the method already.
                let _ = class_addMethod(own, sel!(scrollWheel:), scroll_wheel, c"v@:@".as_ptr());
            }
        }

        /// Puts the web view in a clipping view where it was, and makes it hand its up and
        /// down scrolling to the app (`take_scrolls`). The container is owned by the web
        /// view's superview.
        ///
        /// # Safety
        /// Main thread only, with a live WKWebView inside the app's view.
        pub(super) unsafe fn wrap(webview: *mut Object) -> Option<*mut Object> {
            unsafe {
                let parent: *mut Object = msg_send![webview, superview];
                if parent.is_null() {
                    return None;
                }
                let container: *mut Object = msg_send![class(), alloc];
                let frame: Rect = msg_send![webview, frame];
                let container: *mut Object = msg_send![container, initWithFrame: frame];
                if container.is_null() {
                    return None;
                }
                // Clipping is on by default before macOS 14, and a switch since.
                let clips = sel!(setClipsToBounds:);
                let can: bool = msg_send![container, respondsToSelector: clips];
                if can {
                    let _: () = msg_send![container, setClipsToBounds: true];
                }
                // NSWindowBelow = -1, relative to none: under every other native view, such
                // as the composer's editor. An email is part of the page under them, as the
                // app's own drawing is.
                let none: *mut Object = std::ptr::null_mut();
                let _: () =
                    msg_send![parent, addSubview: container positioned: -1isize relativeTo: none];
                let _: () = msg_send![container, release];
                // wry keeps its own reference to the web view, so it lives through the move.
                let _: () = msg_send![webview, removeFromSuperview];
                let _: () = msg_send![container, addSubview: webview];
                // Its frame is set in full each time (`place`), never resized along.
                let _: () = msg_send![webview, setAutoresizingMask: 0usize];
                take_scrolls(webview);
                Some(container)
            }
        }

        /// The container at `seen` and the web view at `full` in it, both in window points
        /// from the top left.
        ///
        /// # Safety
        /// Main thread only, with the container `wrap` made and its web view.
        pub(super) unsafe fn place(
            container: *mut Object,
            webview: *mut Object,
            full: (f32, f32, f32, f32),
            seen: (f32, f32, f32, f32),
        ) {
            unsafe {
                let parent: *mut Object = msg_send![container, superview];
                if parent.is_null() {
                    return;
                }
                let (x, y, w, h) = (seen.0 as f64, seen.1 as f64, seen.2 as f64, seen.3 as f64);
                let parent_flipped: bool = msg_send![parent, isFlipped];
                let y_in_parent = if parent_flipped {
                    y
                } else {
                    let bounds: Rect = msg_send![parent, bounds];
                    bounds.height - y - h
                };
                let outer = Rect {
                    x,
                    y: y_in_parent,
                    width: w,
                    height: h,
                };
                let inner = Rect {
                    x: full.0 as f64 - x,
                    y: full.1 as f64 - y,
                    width: (full.2 as f64).max(1.),
                    height: (full.3 as f64).max(1.),
                };
                let old: Rect = msg_send![container, frame];
                if (old.x, old.y, old.width, old.height) != (x, y_in_parent, w, h) {
                    let _: () = msg_send![container, setFrame: outer];
                }
                let old: Rect = msg_send![webview, frame];
                if (old.x, old.y, old.width, old.height)
                    != (inner.x, inner.y, inner.width, inner.height)
                {
                    let _: () = msg_send![webview, setFrame: inner];
                }
            }
        }

        /// # Safety
        /// Main thread only, with the container `wrap` made.
        pub(super) unsafe fn show(container: *mut Object, shown: bool) {
            unsafe {
                let _: () = msg_send![container, setHidden: !shown];
            }
        }

        /// Takes the container away with the web view.
        ///
        /// # Safety
        /// Main thread only, with the container `wrap` made.
        pub(super) unsafe fn remove(container: *mut Object) {
            unsafe {
                let _: () = msg_send![container, removeFromSuperview];
            }
        }
    }

    /// The hole: a mask on the web view's layer (even-odd, so the rounded rectangle is left
    /// out), and an empty view laid over it. That view handles no events itself, so AppKit
    /// passes clicks and scrolls there up to its superview, the app's own view.
    #[cfg(target_os = "macos")]
    #[allow(unexpected_cfgs)]
    mod hole {
        use std::cell::RefCell;
        use std::collections::HashMap;
        use std::ffi::c_void;

        use objc::declare::ClassDecl;
        use objc::runtime::{Class, Object, Sel};
        use objc::{Encode, Encoding, class, msg_send, sel, sel_impl};

        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Point {
            x: f64,
            y: f64,
        }
        unsafe impl Encode for Point {
            fn encode() -> Encoding {
                // SAFETY: the encoding of CGPoint / NSPoint on 64-bit macOS.
                unsafe { Encoding::from_str("{CGPoint=dd}") }
            }
        }
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Size {
            width: f64,
            height: f64,
        }
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct CgRect {
            origin: Point,
            size: Size,
        }
        unsafe impl Encode for CgRect {
            fn encode() -> Encoding {
                // SAFETY: the encoding of CGRect / NSRect on 64-bit macOS.
                unsafe { Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
            }
        }

        #[link(name = "CoreGraphics", kind = "framework")]
        unsafe extern "C" {
            fn CGPathCreateMutable() -> *mut c_void;
            fn CGPathAddRect(path: *mut c_void, transform: *const c_void, rect: CgRect);
            fn CGPathAddRoundedRect(
                path: *mut c_void,
                transform: *const c_void,
                rect: CgRect,
                corner_width: f64,
                corner_height: f64,
            );
            fn CGPathRelease(path: *mut c_void);
        }

        thread_local! {
            /// The empty view over each web view's hole (retained by its superview).
            static SHIELDS: RefCell<HashMap<usize, usize>> = RefCell::new(HashMap::new());
        }

        /// An empty view whose hit test answers with its superview (the app's own view), so
        /// every mouse event over it goes straight to the app, as if the web view under it
        /// were not there.
        fn pass_through_class() -> &'static Class {
            if let Some(class) = Class::get("NimbraPassThroughView") {
                return class;
            }
            extern "C" fn hit_test(this: &Object, _: Sel, point: Point) -> *mut Object {
                // SAFETY: AppKit calls this on the main thread with a live view; `point` is in
                // the superview's coordinates, like the view's frame.
                unsafe {
                    let hidden: bool = msg_send![this, isHidden];
                    let frame: CgRect = msg_send![this, frame];
                    let inside = point.x >= frame.origin.x
                        && point.x < frame.origin.x + frame.size.width
                        && point.y >= frame.origin.y
                        && point.y < frame.origin.y + frame.size.height;
                    if hidden || !inside {
                        return std::ptr::null_mut();
                    }
                    msg_send![this, superview]
                }
            }
            let mut decl =
                ClassDecl::new("NimbraPassThroughView", class!(NSView)).expect("class name free");
            // SAFETY: the signature matches -[NSView hitTest:] (NSPoint in, NSView out).
            unsafe {
                decl.add_method(
                    sel!(hitTest:),
                    hit_test as extern "C" fn(&Object, Sel, Point) -> *mut Object,
                );
            }
            decl.register()
        }

        fn rect(x: f64, y: f64, width: f64, height: f64) -> CgRect {
            CgRect {
                origin: Point { x, y },
                size: Size { width, height },
            }
        }

        /// Takes the view's click-through view away (when the web view goes).
        ///
        /// # Safety
        /// Main thread only.
        pub(super) unsafe fn remove(view: *mut Object) {
            if let Some(shield) = SHIELDS.with(|s| s.borrow_mut().remove(&(view as usize))) {
                unsafe {
                    let _: () = msg_send![shield as *mut Object, removeFromSuperview];
                }
            }
        }

        /// Opens (`Some`, top-left-origin view coordinates) or closes the hole.
        ///
        /// # Safety
        /// Main thread only, with a live WKWebView inside the app's view.
        pub(super) unsafe fn set(view: *mut Object, hole: Option<(f64, f64, f64, f64)>) {
            unsafe {
                let key = view as usize;
                let _: () = msg_send![view, setWantsLayer: true];
                let layer: *mut Object = msg_send![view, layer];
                let superview: *mut Object = msg_send![view, superview];
                let shield = SHIELDS
                    .with(|s| s.borrow().get(&key).copied())
                    .map(|p| p as *mut Object);
                let Some((x, y, w, h)) = hole else {
                    if !layer.is_null() {
                        let nil: *mut Object = std::ptr::null_mut();
                        let _: () = msg_send![layer, setMask: nil];
                    }
                    if let Some(shield) = shield {
                        let _: () = msg_send![shield, setHidden: true];
                    }
                    return;
                };
                // The mask, in the layer's coordinates (from the bottom unless flipped).
                let bounds: CgRect = msg_send![view, bounds];
                if !layer.is_null() {
                    let flipped: bool = msg_send![layer, isGeometryFlipped];
                    let my = if flipped {
                        y
                    } else {
                        bounds.size.height - y - h
                    };
                    let path = CGPathCreateMutable();
                    CGPathAddRect(path, std::ptr::null(), bounds);
                    CGPathAddRoundedRect(path, std::ptr::null(), rect(x, my, w, h), 18., 18.);
                    let mask: *mut Object = msg_send![class!(CAShapeLayer), layer];
                    let _: () = msg_send![mask, setFrame: bounds];
                    let _: () = msg_send![mask, setPath: path];
                    let rule: *mut Object =
                        msg_send![class!(NSString), stringWithUTF8String: c"even-odd".as_ptr()];
                    let _: () = msg_send![mask, setFillRule: rule];
                    let _: () = msg_send![layer, setMask: mask];
                    CGPathRelease(path);
                }
                // The empty view over the hole, in the superview's coordinates.
                if superview.is_null() {
                    return;
                }
                let frame: CgRect = msg_send![view, frame];
                let parent_flipped: bool = msg_send![superview, isFlipped];
                let view_flipped: bool = msg_send![view, isFlipped];
                // Top-left of the hole within the superview.
                let top = if view_flipped || !parent_flipped {
                    // Convert from the view's top-left space to the superview's.
                    if parent_flipped {
                        frame.origin.y + y
                    } else {
                        frame.origin.y + frame.size.height - y - h
                    }
                } else {
                    frame.origin.y + y
                };
                // A little wider than the hole, so whatever sits on top can be grabbed at its
                // very edge (the email still shows there; only the mouse goes to the app).
                const REACH: f64 = 8.;
                let target = rect(
                    frame.origin.x + x - REACH,
                    top - REACH,
                    w + 2. * REACH,
                    h + 2. * REACH,
                );
                let shield = match shield {
                    Some(shield) => shield,
                    None => {
                        let shield: *mut Object = msg_send![pass_through_class(), alloc];
                        let shield: *mut Object = msg_send![shield, initWithFrame: target];
                        // NSWindowAbove = 1: just above the web view.
                        let _: () = msg_send![superview, addSubview: shield positioned: 1isize relativeTo: view];
                        let _: () = msg_send![shield, release];
                        SHIELDS.with(|s| s.borrow_mut().insert(key, shield as usize));
                        shield
                    }
                };
                let _: () = msg_send![shield, setFrame: target];
                let _: () = msg_send![shield, setHidden: false];
            }
        }
    }

    #[cfg(target_os = "macos")]
    impl Drop for HtmlView {
        fn drop(&mut self) {
            use wry::WebViewExtMacOS as _;
            let webview = self.view.webview();
            // SAFETY: main thread, live WKWebView; see `hole::remove`.
            unsafe { hole::remove(&*webview as *const _ as *mut objc::runtime::Object) };
            if let Some(container) = self.clip.get() {
                // SAFETY: main thread, the container `place` made, which wry's web view
                // leaves with (its own drop takes it out of the container).
                unsafe { clip::remove(container as *mut objc::runtime::Object) };
            }
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
        pub fn show(&self, _body: &str, _images: bool, _account: Option<&str>) {}
        pub fn set_bounds(&self, _x: f32, _y: f32, _width: f32, _height: f32) {}
        pub fn set_visible(&self, _visible: bool) {}
        pub fn set_zoom(&self, _zoom: f32) {}
        pub fn set_hole(&self, _hole: Option<(f32, f32, f32, f32)>) {}
        pub fn set_dark(&self, _dark: bool, _darken: bool) {}
        pub fn print(&self) {}
        pub fn copy(&self) -> bool {
            false
        }
        pub fn with_loaded(_window: &gpui::Window, _loaded: impl Fn() + 'static) -> Option<Self> {
            None
        }
        pub fn place(&self, _full: (f32, f32, f32, f32), _clip: (f32, f32, f32, f32)) {}
        pub fn measure(&self, _answer: impl Fn(f32) + Send + 'static) {}
        pub fn snapshot(&self, answer: impl Fn(Option<Vec<u8>>) + 'static) {
            answer(None);
        }
        pub fn focused(&self) -> bool {
            false
        }
    }
}

pub use platform::HtmlView;

/// An NSImage's pixels as TIFF (uncompressed, so quick to make and to read back).
///
/// # Safety
/// Main thread, with an NSImage or nil.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
unsafe fn tiff_of(image: *mut objc::runtime::Object) -> Option<Vec<u8>> {
    use objc::runtime::Object;
    use objc::{msg_send, sel, sel_impl};
    if image.is_null() {
        return None;
    }
    unsafe {
        let data: *mut Object = msg_send![image, TIFFRepresentation];
        if data.is_null() {
            return None;
        }
        let length: usize = msg_send![data, length];
        let bytes: *const u8 = msg_send![data, bytes];
        (!bytes.is_null() && length > 0).then(|| std::slice::from_raw_parts(bytes, length).to_vec())
    }
}

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
        // Our defaults never outrank the email's own styles or touch its layout, except for
        // the space around it (the page's padding and the body's margin).
        let space = ["html{padding:20px!important", "body{margin:0!important"];
        assert!(
            DEFAULTS
                .split('}')
                .filter(|rule| !rule.is_empty())
                .all(|rule| rule.starts_with(":where(") || space.contains(&rule))
        );
        assert!(!DEFAULTS.contains("max-width") && !DEFAULTS.contains("overflow-wrap"));
    }

    #[test]
    fn an_emails_height_is_read_in_points() {
        assert_eq!(measured("412", 1.), Some(412.));
        assert_eq!(measured("100.2", 1.25), Some(126.));
        for bad in ["0", "-3", "null", "\"x\"", "NaN", "Infinity", ""] {
            assert_eq!(measured(bad, 1.), None, "{bad}");
        }
    }

    #[test]
    fn dns_prefetch_is_off_before_the_message_starts() {
        let page = document("<a href=\"https://x.example\">x</a>", true);
        let off = page.find("<meta http-equiv=\"x-dns-prefetch-control\" content=\"off\">");
        assert!(off.is_some_and(|at| at < page.find("<a href").unwrap()));
    }

    #[test]
    fn the_dark_mail_script_only_touches_colours() {
        let code: String = DARK_MAIL
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect();
        for sink in [
            "innerHTML",
            "outerHTML",
            "insertAdjacent",
            "document.write",
            "eval(",
            "Function(",
            "fetch(",
            "XMLHttpRequest",
            "WebSocket",
            "sendBeacon",
            "postMessage",
            "messageHandlers",
            "location",
            "createElement",
            "setAttribute",
            "http:",
            "https:",
            "url(",
        ] {
            assert!(!code.contains(sink), "{sink}");
        }
        assert!(!DARK_MAIL.contains('\0'));
        // The page it paints is the surface around it.
        assert!(code.contains("const SURFACE = [14, 14, 20]"));
    }

    #[test]
    fn link_tags_are_inert() {
        for tag in [
            "<link rel=preconnect href=x>",
            "<LiNk\trel=preconnect>",
            "<link/rel=preconnect>",
            "<link\nrel=preconnect>",
            "<link\r\nrel=preconnect>",
            "<link\x0crel=preconnect>",
            "<link>",
            "<<link rel=preconnect>",
            "é<link rel=preconnect>ü",
            "<link",
        ] {
            let page = document(tag, false).to_ascii_lowercase();
            assert!(!page.contains("<link"), "{tag:?}");
        }
        for same in [
            "<linkbox>",
            "a link",
            "&lt;link&gt;",
            "</p>",
            "<p>héllo</p>",
        ] {
            assert_eq!(without_links(same), same);
        }
    }

    #[test]
    fn on_windows_only_our_loads_and_clicked_links_go_anywhere() {
        use Navigation::{Load, Open, Stop};
        let ours = "data:text/html;charset=utf-8;base64,PHA-";
        for (link, clicked, loads, wanted) in [
            (ours, false, 1, Load),
            // WebView2 says a load the app asked for is the user's.
            (ours, true, 1, Load),
            ("about:blank", true, 1, Load),
            // None of ours is due.
            (ours, false, 0, Stop),
            (ours, true, 0, Stop),
            ("about:blank", true, 0, Stop),
            // A refresh or a redirect, even while a load of ours is waiting.
            ("https://example.com/a", false, 1, Stop),
            ("mailto:a@b.co", false, 1, Stop),
            ("ms-msdt:/id", false, 1, Stop),
            ("https://example.com/a", true, 0, Open),
            ("mailto:a@b.co", true, 0, Open),
            ("file:///c:/windows/system32/calc.exe", true, 0, Stop),
            ("ms-msdt:/id", true, 0, Stop),
        ] {
            assert_eq!(
                navigation(link, clicked, loads),
                wanted,
                "{link} {clicked} {loads}"
            );
        }
    }

    #[test]
    fn mail_cannot_open_the_browser_by_itself() {
        assert!(leaves_app("https://example.com/a", true));
        assert!(!leaves_app("https://example.com/a", false));
        assert!(!leaves_app("mailto:a@b.co", false));
        assert!(!leaves_app("file:///etc/passwd", true));
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
