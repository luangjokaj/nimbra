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
}

pub enum Event {
    /// The page of that view has loaded.
    Loaded(u64),
    /// How tall that view's email is, in points.
    Height(u64, f32),
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

    fn measure(&self) {
        let (id, tell) = (self.id, self.tell.clone());
        self.view.measure(move |height| {
            let _ = tell.unbounded_send(Event::Height(id, height));
        });
    }
}

/// The web views of the conversation a window shows.
pub struct MailViews {
    slots: HashMap<String, Rc<Slot>>,
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
                next: 0,
                tell,
                zoom: 1.,
                shown: false,
                column: Rc::new(Cell::new(0.)),
            },
            events,
        )
    }

    /// Makes the views match `wanted`: new ones are made and loaded, the others go. While
    /// `shown` is false they are all hidden but kept, so they need not load again. False
    /// when there is no web view on this system.
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
        self.slots
            .retain(|key, _| wanted.iter().any(|w| w.key == *key));
        let rezoom = (self.zoom - zoom).abs() > f32::EPSILON;
        self.zoom = zoom;
        self.shown = shown;
        for Wanted { key, html, open } in wanted {
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
                        column: self.column.clone(),
                        page: Cell::new(None),
                        tell,
                    });
                    self.slots.insert(key, slot.clone());
                    slot
                }
            };
            if slot.page.get() != Some(look) {
                slot.view.set_dark(look.dark, look.darkened);
                slot.view.show(html, look.images, Some(account));
                slot.page.set(Some(look));
            }
            slot.view.set_zoom(zoom);
            if rezoom {
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
                    slot.measure();
                }
                false
            }
            Event::Height(id, height) => {
                let Some(slot) = self.slots.values().find(|s| s.id == id) else {
                    return false;
                };
                let old = slot.height.replace(height);
                // Its first height: the message opens to it (unless motion is reduced).
                if old <= 0. && !crate::integration::display_options().reduce_motion {
                    slot.opening.set(Some((std::time::Instant::now(), 0.)));
                }
                (old - height).abs() > 0.5
            }
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
/// latest folded ones, hidden (`all`: every message open, as a message window shows them).
pub fn wanted<'a>(mail: &'a crate::data::Mail, thread: &str, all: bool) -> Vec<Wanted<'a>> {
    let latest = mail
        .html
        .as_deref()
        .map(|html| (0, mail.header.message_id.as_str(), html, true));
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
            wanted(m, "t", all).into_iter().map(|w| w.key).collect()
        };
        let open = |m: &crate::data::Mail, all: bool| -> Vec<String> {
            wanted(m, "t", all)
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
        assert!(wanted(&m, "t", false)[1].html.contains("first pass"));
    }
}
