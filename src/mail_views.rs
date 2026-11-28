//! Emails in their own formatting inside a conversation: a web view for each open message
//! that has HTML, as tall as the email, placed where that message's body is in the
//! reader's column and clipped to it (`HtmlView::place`), so the conversation scrolls as
//! one, with every message's header and actions native around the emails.
//!
//! Heights come from the page (`HtmlView::measure`) once it has loaded, and again when its
//! width or zoom changes; they arrive as `Event`s the window's owner hands to `handle`.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

use crate::webview::HtmlView;

/// How the emails are shown: a change loads them again.
#[derive(Clone, Copy, PartialEq)]
pub struct Look {
    /// Remote images allowed.
    pub images: bool,
    pub dark: bool,
    /// Light emails given a dark page (`dark_mail.js`).
    pub darkened: bool,
}

/// A message to show: its key (`message_key`) and its HTML. A message's HTML never
/// changes under its key, so it is only read when the view is made or its look changes.
pub struct Wanted<'a> {
    pub key: String,
    pub html: &'a str,
    /// Shown, as its message is open. A folded one is loaded and laid out hidden, at the
    /// column's width, so its message opens straight to its height.
    pub open: bool,
    /// The thread's latest message: there at once at its height, with no opening.
    pub latest: bool,
    /// The account it is in, for the `mailto:` links in it (`None`: the window's).
    pub account: Option<&'a str>,
}

pub enum Event {
    /// The page of that view has loaded.
    Loaded(u64),
    /// How tall that view's email is, in points.
    Height(u64, f32),
    /// A picture of that view's email (TIFF), at the width and height it was taken at.
    Snapshot(u64, Option<Vec<u8>>, f32, f32),
}

/// A picture of an email as drawn, and the size it was drawn at (`Slot::snapshot`).
struct Snapshot {
    image: std::sync::Arc<gpui::Image>,
    width: f32,
    height: f32,
}

/// One message's web view, as the column places it while it is drawn.
pub struct Slot {
    id: u64,
    pub view: Rc<HtmlView>,
    /// The email's height in points; 0 until it has been measured.
    height: Rc<Cell<f32>>,
    /// When it first got its height, and from what: it opens to it (`shown_height`).
    opening: Cell<Option<(std::time::Instant, f32)>>,
    width: Cell<f32>,
    /// Whether it has been placed in a column (`HtmlView::place`) at least once.
    placed: Cell<bool>,
    /// The latest message's: it does not open to its height, it is there.
    latest: Cell<bool>,
    /// When it was made, for how long it may keep the reader waiting (`waiting`).
    made: std::time::Instant,
    /// When its height was last asked for while it has none (`measure_waiting`).
    asked: Cell<Option<std::time::Instant>>,
    /// The zoom its height was measured at.
    zoom: Cell<f32>,
    /// Shown as its message is open (`Wanted::open`).
    open: Cell<bool>,
    /// Its page has loaded, images and all.
    loaded: Cell<bool>,
    /// When its height last changed: a picture is taken once it holds still.
    steady_since: Cell<std::time::Instant>,
    /// A picture of it is on its way.
    snapping: Cell<bool>,
    /// The picture the app draws in its place while things move (`take_snapshots`).
    snapshot: std::cell::RefCell<Option<Snapshot>>,
    /// The column's width as last placed, shared by the window's views.
    column: Rc<Cell<f32>>,
    page: Cell<Option<Look>>,
    tell: UnboundedSender<Event>,
}

/// How long an email takes to open to its height.
const OPENING: std::time::Duration = std::time::Duration::from_millis(200);

impl Slot {
    /// The email's height in points; 0 until it has been measured.
    pub fn height(&self) -> f32 {
        self.height.get()
    }

    /// The picture of the email as it is drawn now (its size unchanged since), if any.
    pub fn snapshot(&self) -> Option<std::sync::Arc<gpui::Image>> {
        let snapshot = self.snapshot.borrow();
        let snapshot = snapshot.as_ref()?;
        ((snapshot.width - self.width.get()).abs() <= 0.5
            && (snapshot.height - self.height.get()).abs() <= 0.5)
            .then(|| snapshot.image.clone())
    }

    /// Its page is still loading or being measured, for a while at most: one that never
    /// answers does not keep the reader waiting.
    pub fn waiting(&self) -> bool {
        const LONGEST: std::time::Duration = std::time::Duration::from_secs(10);
        self.height.get() <= 0. && self.made.elapsed() < LONGEST
    }

    /// How much of the email shows now, and whether that is still growing: a message opens
    /// to its email's height (ease-out), rather than the email appearing all at once and
    /// pushing what is under it down. Later changes of height (a wider window) are at once.
    pub fn shown_height(&self) -> (f32, bool) {
        let height = self.height.get();
        let Some((start, from)) = self.opening.get() else {
            return (height, false);
        };
        let t = start.elapsed().as_secs_f32() / OPENING.as_secs_f32();
        if t >= 1. {
            self.opening.set(None);
            return (height, false);
        }
        let eased = 1. - (1. - t).powi(3);
        (from + (height - from) * eased, true)
    }

    /// Puts the email at `full` (window points), shown inside `clip`. A new width asks for
    /// its height again.
    pub fn place(&self, full: (f32, f32, f32, f32), clip: (f32, f32, f32, f32)) {
        self.view.place(full, clip);
        self.placed.set(true);
        self.column.set(full.2);
        if (self.width.replace(full.2) - full.2).abs() > 0.5 {
            self.measure();
        }
    }

    /// Its height asked for early, before its page has finished loading (its images): the
    /// text is laid out long before, so it shows then, and grows when the images come.
    /// Too small an answer is the page not there yet, and is not taken.
    fn measure_early(&self) {
        const SOME_TEXT: f32 = 40.;
        let (id, tell) = (self.id, self.tell.clone());
        self.view.measure(move |height| {
            if height >= SOME_TEXT {
                let _ = tell.unbounded_send(Event::Height(id, height));
            }
        });
    }

    fn measure(&self) {
        let (id, tell) = (self.id, self.tell.clone());
        self.view.measure(move |height| {
            let _ = tell.unbounded_send(Event::Height(id, height));
        });
    }
}

/// How many emails no longer wanted stay loaded, hidden, so that going back to them is
/// instant: the most recently left first.
const KEEP_LEFT: usize = 10;

/// The web views of the conversation a window shows.
pub struct MailViews {
    slots: HashMap<String, Rc<Slot>>,
    /// The slots not wanted now, kept for a return (`KEEP_LEFT`), most recently left first.
    left: std::collections::VecDeque<String>,
    next: u64,
    tell: UnboundedSender<Event>,
    zoom: f32,
    shown: bool,
    /// The column's width as last placed (`Slot::column`).
    column: Rc<Cell<f32>>,
}

impl MailViews {
    /// The views, and where their events arrive (for the owner to `handle`).
    pub fn new() -> (Self, UnboundedReceiver<Event>) {
        let (tell, events) = unbounded();
        (
            MailViews {
                slots: HashMap::new(),
                left: std::collections::VecDeque::new(),
                next: 0,
                tell,
                zoom: 1.,
                shown: false,
                column: Rc::new(Cell::new(0.)),
            },
            events,
        )
    }

    /// Makes the views match `wanted`: new ones are made and loaded; the others are
    /// hidden, the last `KEEP_LEFT` of them kept loaded for a return. While `shown` is
    /// false they are all hidden but kept, so they need not load again. False when there
    /// is no web view on this system.
    #[allow(clippy::too_many_arguments)]
    pub fn sync(
        &mut self,
        wanted: Vec<Wanted<'_>>,
        look: Look,
        account: &str,
        zoom: f32,
        shown: bool,
        hole: Option<(f32, f32, f32, f32)>,
        window: &gpui::Window,
    ) -> bool {
        // Left now: hidden, and kept among the last few left; older ones go.
        let mut gone: Vec<String> = self
            .slots
            .keys()
            .filter(|key| !wanted.iter().any(|w| w.key == **key) && !self.left.contains(key))
            .cloned()
            .collect();
        gone.sort();
        for key in gone {
            if let Some(slot) = self.slots.get(&key) {
                slot.view.set_visible(false);
                // Pictures are for the conversation on screen only (they are large).
                slot.snapshot.borrow_mut().take();
            }
            self.left.push_front(key);
        }
        self.left
            .retain(|key| !wanted.iter().any(|w| w.key == *key));
        while self.left.len() > KEEP_LEFT {
            if let Some(oldest) = self.left.pop_back() {
                self.slots.remove(&oldest);
            }
        }
        self.zoom = zoom;
        self.shown = shown;
        for Wanted {
            key,
            html,
            open,
            latest,
            account: own,
        } in wanted
        {
            let slot = match self.slots.get(&key) {
                Some(slot) => slot.clone(),
                None => {
                    let id = self.next;
                    self.next += 1;
                    let tell = self.tell.clone();
                    let loaded = tell.clone();
                    let Some(view) = HtmlView::with_loaded(window, move || {
                        let _ = loaded.unbounded_send(Event::Loaded(id));
                    }) else {
                        return false;
                    };
                    let slot = Rc::new(Slot {
                        id,
                        view: Rc::new(view),
                        height: Rc::new(Cell::new(0.)),
                        opening: Cell::new(None),
                        width: Cell::new(0.),
                        placed: Cell::new(false),
                        latest: Cell::new(latest),
                        made: std::time::Instant::now(),
                        asked: Cell::new(None),
                        zoom: Cell::new(zoom),
                        open: Cell::new(open),
                        loaded: Cell::new(false),
                        steady_since: Cell::new(std::time::Instant::now()),
                        snapping: Cell::new(false),
                        snapshot: std::cell::RefCell::new(None),
                        column: self.column.clone(),
                        page: Cell::new(None),
                        tell,
                    });
                    self.slots.insert(key, slot.clone());
                    slot
                }
            };
            slot.open.set(open);
            if slot.page.get() != Some(look) {
                // Another page: its picture is of the old one.
                slot.loaded.set(false);
                slot.snapshot.borrow_mut().take();
                slot.view.set_dark(look.dark, look.darkened);
                slot.view
                    .show(html, look.images, Some(own.unwrap_or(account)));
                slot.page.set(Some(look));
            }
            slot.view.set_zoom(zoom);
            // Measured at another zoom (or kept while the zoom changed): measured again.
            if (slot.zoom.replace(zoom) - zoom).abs() > f32::EPSILON {
                slot.measure();
            }
            if open {
                slot.view.set_hole(hole);
                slot.view.set_visible(shown);
                continue;
            }
            slot.view.set_visible(false);
            // Laid out where it will show, so its height is known before it opens. (Once
            // placed, it keeps the frame its column gave it.)
            let column = self.column.get();
            if !slot.placed.get() && column > 0. && (slot.width.get() - column).abs() > 0.5 {
                slot.view.set_bounds(0., 0., column, 1.);
                slot.width.set(column);
                slot.measure();
            }
        }
        true
    }

    /// The view of a message, while it is wanted.
    pub fn slot(&self, key: &str) -> Option<Rc<Slot>> {
        self.slots.get(key).cloned()
    }

    /// Takes an event in; true when a height changed, so the column is laid out again.
    pub fn handle(&mut self, event: Event) -> bool {
        match event {
            Event::Loaded(id) => {
                if let Some(slot) = self.slots.values().find(|s| s.id == id) {
                    slot.loaded.set(true);
                    slot.measure();
                }
                false
            }
            Event::Snapshot(id, tiff, width, height) => {
                let Some(slot) = self.slots.values().find(|s| s.id == id) else {
                    return false;
                };
                slot.snapping.set(false);
                let Some(tiff) = tiff else {
                    return false;
                };
                *slot.snapshot.borrow_mut() = Some(Snapshot {
                    image: std::sync::Arc::new(gpui::Image::from_bytes(
                        gpui::ImageFormat::Tiff,
                        tiff,
                    )),
                    width,
                    height,
                });
                true
            }
            Event::Height(id, height) => {
                let Some(slot) = self.slots.values().find(|s| s.id == id) else {
                    return false;
                };
                let old = slot.height.replace(height);
                if (old - height).abs() > 0.5 {
                    slot.steady_since.set(std::time::Instant::now());
                }
                // Its first height: a message opened opens to it (unless motion is reduced);
                // the latest, which the conversation opens on, is there at once.
                if old <= 0.
                    && !slot.latest.get()
                    && !crate::integration::display_options().reduce_motion
                {
                    slot.opening.set(Some((std::time::Instant::now(), 0.)));
                }
                (old - height).abs() > 0.5
            }
        }
    }

    /// Asks the shown emails that have no height yet for one, every so often (each frame
    /// asks; this keeps it to `EVERY`), so they show as soon as their text is laid out.
    pub fn measure_waiting(&self) {
        const EVERY: std::time::Duration = std::time::Duration::from_millis(50);
        if !self.shown {
            return;
        }
        for slot in self.slots.values() {
            if slot.height.get() > 0.
                || !slot.placed.get()
                || slot.asked.get().is_some_and(|at| at.elapsed() < EVERY)
            {
                continue;
            }
            slot.asked.set(Some(std::time::Instant::now()));
            slot.measure_early();
        }
    }

    /// Takes a picture of each open email that has loaded and holds still, unless it has
    /// one of it as it is: the app draws it in the email's place while things move
    /// (`reading::email_frame`). Not while they move, nor of very tall emails (memory).
    pub fn take_snapshots(&self, moving: bool) {
        const STEADY: std::time::Duration = std::time::Duration::from_millis(250);
        const TALLEST: f32 = 4000.;
        if moving || !self.shown {
            return;
        }
        for slot in self.slots.values() {
            let height = slot.height.get();
            if !slot.open.get()
                || !slot.placed.get()
                || !slot.loaded.get()
                || slot.snapping.get()
                || height <= 0.
                || height > TALLEST
                || slot.steady_since.get().elapsed() < STEADY
                || slot.snapshot().is_some()
            {
                continue;
            }
            slot.snapping.set(true);
            let (id, tell, width) = (slot.id, slot.tell.clone(), slot.width.get());
            slot.view.snapshot(move |tiff| {
                let _ = tell.unbounded_send(Event::Snapshot(id, tiff, width, height));
            });
        }
    }

    pub fn hide(&mut self) {
        self.shown = false;
        for slot in self.slots.values() {
            slot.view.set_visible(false);
        }
    }

    pub fn clear(&mut self) {
        self.slots.clear();
        self.left.clear();
    }

    /// ⌘C in the email the keyboard is in. False when it is in none.
    pub fn copy(&self) -> bool {
        self.slots
            .values()
            .find(|slot| slot.view.focused())
            .is_some_and(|slot| slot.view.copy())
    }
}

/// How many of a thread's folded messages (the latest of them) have their emails loaded
/// ahead, hidden, so they open at once to their height.
const AHEAD: usize = 12;

/// The messages of `mail` with HTML to have as emails: the open ones, shown, and the
/// latest folded ones, hidden (`all`: every message open, as a message window shows them;
/// `latest_open`: the latest message is not folded).
pub fn wanted<'a>(
    mail: &'a crate::data::Mail,
    thread: &str,
    all: bool,
    latest_open: bool,
) -> Vec<Wanted<'a>> {
    let latest = mail
        .html
        .as_deref()
        .map(|html| (0, mail.header.message_id.as_str(), html, all || latest_open));
    // Newest first, so the ones loaded ahead are the latest.
    let earlier = mail.earlier.iter().enumerate().filter_map(|(k, e)| {
        let open = all || e.expanded;
        let html = e.html.as_deref().filter(|_| open || k < AHEAD)?;
        Some((k + 1, e.header.message_id.as_str(), html, open))
    });
    latest
        .into_iter()
        .chain(earlier)
        .map(|(index, id, html, open)| Wanted {
            key: message_key(thread, id, index),
            html,
            open,
            // In a window of its own every message is open at once: none grows into place.
            latest: index == 0 || all,
            account: None,
        })
        .collect()
}

/// A message's key among the views: its thread's, and its own Message-ID, or else its
/// place in the thread (sample mail has no ids).
pub fn message_key(thread: &str, message_id: &str, index: usize) -> String {
    if message_id.is_empty() {
        format!("{thread}#{index}")
    } else {
        format!("{thread}<{message_id}>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_known_by_its_id_else_its_place() {
        assert_eq!(message_key("a:t1", "<m@x>", 2), "a:t1<<m@x>>");
        assert_eq!(message_key("a:t1", "", 2), "a:t1#2");
        assert_ne!(message_key("a:t1", "", 1), message_key("a:t2", "", 1));
    }

    #[test]
    fn the_open_messages_with_html_are_shown_as_emails() {
        let mut m = crate::data::inbox().remove(0);
        let keys = |m: &crate::data::Mail, all: bool| -> Vec<String> {
            wanted(m, "t", all, true)
                .into_iter()
                .map(|w| w.key)
                .collect()
        };
        let open = |m: &crate::data::Mail, all: bool| -> Vec<String> {
            wanted(m, "t", all, true)
                .into_iter()
                .filter(|w| w.open)
                .map(|w| w.key)
                .collect()
        };
        // The latest is always open; the earlier ones once opened, and loaded ahead till then.
        assert_eq!(keys(&m, false), ["t#0", "t#1", "t#2"]);
        assert_eq!(open(&m, false), ["t#0"]);
        m.earlier[1].expanded = true;
        assert_eq!(open(&m, false), ["t#0", "t#2"]);
        // A message window shows every message.
        assert_eq!(keys(&m, true), ["t#0", "t#1", "t#2"]);
        // One without HTML is drawn by the reader instead.
        m.earlier[0].html = None;
        assert_eq!(keys(&m, true), ["t#0", "t#2"]);
        assert!(wanted(&m, "t", false, true)[1].html.contains("first pass"));
    }

    #[test]
    fn a_folded_latest_message_keeps_its_email_loaded_but_hidden() {
        let m = crate::data::inbox().remove(0);
        let latest = |all: bool, open: bool| {
            wanted(&m, "t", all, open)
                .into_iter()
                .find(|w| w.key == "t#0")
                .map(|w| w.open)
        };
        assert_eq!(latest(false, false), Some(false));
        assert_eq!(latest(false, true), Some(true));
        // Its own window shows it whatever the reader does.
        assert_eq!(latest(true, false), Some(true));
    }
}
