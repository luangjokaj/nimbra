//! The list of threads: its header and rows, pulling it down to reload, the pane for several
//! picked threads, and what an empty or cleared mailbox shows.

use std::hash::{Hash, Hasher};

use super::*;
use crate::ui::motion;

/// Pull-to-refresh distances, in points: where a release triggers a refresh, where the
/// indicator rests while refreshing, and the distance the resistance is scaled to.
pub(super) const PULL_TRIGGER: f32 = 64.;

pub(super) const PULL_REST: f32 = 46.;

pub(super) const PULL_LIMIT: f32 = 420.;

/// Rubber-band resistance: the further the pull, the less the list follows.
pub(super) fn rubber_band(pull: f32) -> f32 {
    (1. - 1. / (pull * 0.55 / PULL_LIMIT + 1.)) * PULL_LIMIT
}

/// Where the indicator is, `start.elapsed()` after being let go.
///
/// Settling at the "checking" height uses a critically damped spring (no overshoot).
/// Going back up (to 0) is a short, plain ease-out: quick, with no spring tail.
pub(super) fn settle(from: f32, to: f32, start: std::time::Instant) -> f32 {
    let t = start.elapsed().as_secs_f32();
    if to == 0. {
        let p = (t / CLOSE_SECONDS).min(1.);
        return from * (1. - p).powi(3);
    }
    if t >= SETTLE_SECONDS {
        return to;
    }
    let k = 13. * t;
    to + (from - to) * (-k).exp() * (1. + k)
}

/// How long the pull-to-refresh strip takes to close.
pub(super) const CLOSE_SECONDS: f32 = 0.22;

pub(super) const SETTLE_SECONDS: f32 = 0.7;

/// A row's height: its padding (11 above and below) and two lines of text (22 + 3 + 18).
pub(super) const ROW_H: f32 = 65.;
/// The space under each row.
pub(super) const ROW_GAP: f32 = 2.;
/// A group head (TODAY, EARLIER): 16 above its 12pt line, 6 below.
pub(super) const GROUP_H: f32 = 34.;
/// The list's own margin left and right of the rows, and under the last.
const LIST_SIDE: f32 = 10.;
const LIST_BOTTOM: f32 = 10.;
/// How far past the visible rows the virtual list lays out, so a fast scroll finds them.
const OVERDRAW: f32 = 480.;
/// The least room a sender keeps beside a label pill; with less, the pill gives way.
const SENDER_MIN: f32 = 96.;

/// What is in the list, one per item of GPUI's virtual list.
#[derive(Clone, Copy, Debug)]
pub(super) enum Item {
    Group(Group),
    /// A thread: its row in `mails`, and who it is (its key and account, hashed), which
    /// stays the same when a refresh moves it.
    Row {
        index: usize,
        id: u64,
    },
    /// One of the placeholder rows while more is on its way.
    More(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Group {
    Today,
    Earlier,
}

impl Item {
    /// The same thing in the list, wherever it is in `mails` now.
    fn same(&self, other: &Item) -> bool {
        match (self, other) {
            (Item::Group(a), Item::Group(b)) => a == b,
            (Item::Row { id: a, .. }, Item::Row { id: b, .. }) => a == b,
            (Item::More(a), Item::More(b)) => a == b,
            _ => false,
        }
    }

    /// Every item is drawn at exactly this height, so where anything is can be worked out
    /// without laying it out.
    fn height(&self) -> f32 {
        match self {
            Item::Group(_) => GROUP_H,
            Item::Row { .. } | Item::More(_) => ROW_H + ROW_GAP,
        }
    }
}

/// The thread list's scroll state: GPUI's virtual list, which builds only the rows in view,
/// and what each of its items is.
pub(super) struct ListScroll {
    state: gpui::ListState,
    items: Vec<Item>,
}

impl ListScroll {
    pub(super) fn new() -> Self {
        Self {
            state: gpui::ListState::new(0, gpui::ListAlignment::Top, px(OVERDRAW)),
            items: Vec::new(),
        }
    }

    fn viewport(&self) -> f32 {
        f32::from(self.state.viewport_bounds().size.height)
    }

    fn content(&self) -> f32 {
        self.items.iter().map(Item::height).sum::<f32>() + LIST_BOTTOM
    }

    /// How far down the list is scrolled, in points.
    fn top(&self) -> f32 {
        let top = self.state.logical_scroll_top();
        let before: f32 = self.items.iter().take(top.item_ix).map(Item::height).sum();
        before + f32::from(top.offset_in_item)
    }

    /// As a scroll handle has it: up from the top, so never above 0.
    pub(super) fn offset(&self) -> gpui::Point<gpui::Pixels> {
        gpui::point(px(0.), px(-self.top()))
    }

    pub(super) fn max_offset(&self) -> gpui::Size<gpui::Pixels> {
        gpui::size(px(0.), px((self.content() - self.viewport()).max(0.)))
    }

    pub(super) fn at_top(&self) -> bool {
        self.top() <= 0.5
    }

    pub(super) fn to_top(&self) {
        self.scroll_to(0.);
    }

    fn scroll_to(&self, y: f32) {
        let y = y.clamp(0., (self.content() - self.viewport()).max(0.));
        let mut above = 0.;
        for (item_ix, item) in self.items.iter().enumerate() {
            if above + item.height() > y {
                self.state.scroll_to(gpui::ListOffset {
                    item_ix,
                    offset_in_item: px(y - above),
                });
                return;
            }
            above += item.height();
        }
        self.state.scroll_to(gpui::ListOffset {
            item_ix: self.items.len(),
            offset_in_item: px(0.),
        });
    }

    /// Scrolls just enough for item `ix` to be in view.
    fn reveal(&self, ix: usize) {
        let Some(item) = self.items.get(ix) else {
            return;
        };
        let start: f32 = self.items[..ix].iter().map(Item::height).sum();
        let end = start + item.height();
        let (top, viewport) = (self.top(), self.viewport());
        if start < top {
            self.scroll_to(start);
        } else if viewport > 0. && end > top + viewport {
            self.scroll_to(end - viewport);
        }
    }

    /// The list is now `items`. Only what changed is laid out again, and the list stays on
    /// the thread it was scrolled to when a refresh moves it.
    pub(super) fn update(&mut self, items: Vec<Item>) {
        let (old, new) = (self.items.len(), items.len());
        let prefix = self
            .items
            .iter()
            .zip(&items)
            .take_while(|(a, b)| a.same(b))
            .count();
        let suffix = self.items[prefix..]
            .iter()
            .rev()
            .zip(items[prefix..].iter().rev())
            .take_while(|(a, b)| a.same(b))
            .count();
        if prefix == old && prefix == new {
            self.items = items;
            return;
        }
        let top = self.state.logical_scroll_top();
        let changed = prefix..old - suffix;
        let anchor = changed
            .contains(&top.item_ix)
            .then(|| self.items.get(top.item_ix).copied())
            .flatten()
            .and_then(|was| items.iter().position(|item| item.same(&was)));
        self.state.splice(changed, new - prefix - suffix);
        if let Some(item_ix) = anchor {
            self.state.scroll_to(gpui::ListOffset {
                item_ix,
                offset_in_item: top.offset_in_item,
            });
        }
        self.items = items;
    }
}

/// A thread's identity in the list: its key and its account.
fn row_id(key: &str, account: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    account.hash(&mut hasher);
    hasher.finish()
}

/// What a row says, cut to the room it has.
struct RowText {
    sender: SharedString,
    tag: Option<(SharedString, Accent)>,
    subject: SharedString,
    snippet: SharedString,
}

impl Nimbra {
    /// Whether nothing on screen means Inbox Zero, rather than nothing in this mailbox.
    pub(super) fn is_zero(&self) -> bool {
        match self.mailbox {
            View::Inbox if self.is_demo() => true,
            View::Inbox => self.all_cleared() && !self.next_pages.contains_key(&View::Inbox),
            // Not while mail is filtered away (Unread), unchecked or past an account's last page.
            View::AllInboxes => self.all_inboxes_whole && self.all_cleared(),
            _ => false,
        }
    }

    /// Every thread that is listed was cleared, rather than some being filtered away.
    fn all_cleared(&self) -> bool {
        self.done.iter().all(|done| *done)
    }

    /// Everything listed was cleared and more is on its way: that is loading, not empty.
    pub(super) fn next_page_coming(&self) -> bool {
        !self.is_demo() && self.all_cleared() && self.more().is_some()
    }

    /// Still loading what was empty when it was last saved.
    pub(super) fn opening_to_zero(&self) -> bool {
        self.status == Status::Loading
            && match self.mailbox {
                View::Inbox => self.store.is_none() && self.inbox_saved_empty,
                View::AllInboxes => self.all_inboxes_saved_empty,
                _ => false,
            }
    }

    /// Whether mail is known to be in some account's Inbox, whatever All inboxes last had.
    pub(super) fn inbox_has_mail(&self) -> bool {
        let here = self
            .cache
            .get(&View::Inbox)
            .is_some_and(|mails| !mails.is_empty());
        let elsewhere = self
            .account_unread
            .iter()
            .any(|(id, unread)| *id != self.account_id && *unread > 0);
        here || elsewhere
    }

    /// Brings the virtual list up to date with `visible` (the rows as `visible()` has them),
    /// and scrolls to the open thread when the keyboard asked for it.
    pub(super) fn sync_list(&mut self, visible: &[usize]) {
        let mut items = Vec::with_capacity(visible.len() + 4);
        for (group, earlier) in [(Group::Today, false), (Group::Earlier, true)] {
            let mut rows = visible
                .iter()
                .copied()
                .filter(|i| self.mails[*i].yesterday == earlier)
                .map(|index| {
                    let (key, account) = self.thread_key(&self.mails[index]);
                    Item::Row {
                        index,
                        id: row_id(&key, &account),
                    }
                })
                .peekable();
            if rows.peek().is_some() {
                items.push(Item::Group(group));
                items.extend(rows);
            }
        }
        if self.more().is_some() && !visible.is_empty() {
            items.extend((0..2).map(Item::More));
        }
        self.list_scroll.update(items);
        if self.reveal_row.replace(false) {
            let target = self.pick_end.unwrap_or(self.selected);
            if let Some(ix) = self
                .list_scroll
                .items
                .iter()
                .position(|item| matches!(item, Item::Row { index, .. } if *index == target))
            {
                self.list_scroll.reveal(ix);
            }
        }
    }

    /// Several threads picked: what can be done to all of them.
    pub(super) fn bulk_pane(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.targets().len();
        let action = |id: &'static str, name: &'static str, label: &'static str| {
            labelled(t, name, label).id(id).press(t)
        };
        let inbox_like = inbox_like(self.mailbox);
        let back_to_inbox = matches!(
            self.mailbox,
            View::Done | View::Spam | View::Trash | View::Snoozed | View::Reminders
        );
        panel(t, Depth::Front)
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(space::XL))
            .child(title_l(format!("{count} Selected"), t.text))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap(px(space::S))
                    .max_w(px(620.))
                    .when(inbox_like, |d| {
                        d.child(crate::ui::glow(
                            button(t, ButtonKind::Primary, "Mark Done")
                                .id("bulk-done")
                                .on_click(cx.listener(|this, _, _, cx| this.mark_done(cx))),
                            "bulk-done",
                        ))
                    })
                    .when(back_to_inbox, |d| {
                        d.child(
                            action("bulk-done", "inbox", "Move to Inbox")
                                .on_click(cx.listener(|this, _, _, cx| this.mark_done(cx))),
                        )
                    })
                    .child(
                        action("bulk-read", "mail", "Read/Unread")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_unread(cx))),
                    )
                    .child(
                        action("bulk-star", "star", "Star")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_star(cx))),
                    )
                    .child(action("bulk-label", "tag", "Label").on_click(cx.listener(
                        |this, _, window, cx| this.toggle_menu(Menu::Labels, window, cx),
                    )))
                    .child(
                        action("bulk-move", "folder", "Move To").on_click(cx.listener(
                            |this, _, window, cx| this.toggle_menu(Menu::MoveTo, window, cx),
                        )),
                    )
                    .when(inbox_like && !self.is_demo(), |d| {
                        d.child(
                            action("bulk-snooze", "clock", "Snooze").on_click(cx.listener(
                                |this, event: &gpui::ClickEvent, window, cx| {
                                    this.open_snooze(Some(event.position()), window, cx)
                                },
                            )),
                        )
                    })
                    .when(self.mailbox != View::Spam && !self.is_demo(), |d| {
                        d.child(action("bulk-spam", "ban", "Spam").on_click(cx.listener(
                            |this, _, _, cx| this.remove_selected(mail::Change::Spam, cx),
                        )))
                    })
                    .child(
                        action("bulk-trash", "trash", "Trash")
                            .on_click(cx.listener(|this, _, _, cx| this.trash(cx))),
                    ),
            )
            .child(
                button(t, ButtonKind::Ghost, "Deselect All")
                    .id("bulk-clear")
                    .press(t)
                    .text_color(t.text2)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.picked.clear();
                        cx.notify();
                    })),
            )
            .child(rim(t, radius::SHEET))
    }

    /// Pull to refresh, in the style of the system's: the list stretches with rubber-band
    /// resistance, letting go past the threshold starts the refresh, and the indicator
    /// springs back when it is done. Only a trackpad gesture that begins with the list
    /// already at the top counts, so the momentum of a normal scroll up never triggers it.
    fn pull_list(&mut self, event: &gpui::ScrollWheelEvent, cx: &mut Context<Self>) {
        let at_top = self.list_scroll.at_top();
        match event.touch_phase {
            gpui::TouchPhase::Started => {
                self.pull_armed = at_top && !self.pull_hold;
                return;
            }
            gpui::TouchPhase::Ended => {
                self.pull_armed = false;
                if self.pull <= 0. {
                    return;
                }
                let shown = rubber_band(self.pull);
                self.pull = 0.;
                self.pull_gen += 1;
                if shown >= PULL_TRIGGER && !self.fetching() {
                    // Settle at the resting height and stay there while mail is fetched.
                    self.pull_anim = Some((shown, PULL_REST, std::time::Instant::now()));
                    self.pull_hold = true;
                    self.reload(cx);
                } else {
                    self.pull_anim = Some((shown, 0., std::time::Instant::now()));
                }
                cx.notify();
                return;
            }
            gpui::TouchPhase::Moved => {}
        }
        if !self.pull_armed {
            return;
        }
        let dy = f32::from(event.delta.pixel_delta(px(18.)).y);
        if (dy > 0. && at_top) || (dy < 0. && self.pull > 0.) {
            self.pull = (self.pull + dy).max(0.);
            self.pull_anim = None;
            // While the list is being pulled it must not scroll as well, or the two
            // movements fight each other.
            self.list_scroll.to_top();
            cx.notify();
        }
    }

    /// The strip at the top of the list while pulling, refreshing or settling.
    fn pull_indicator(&self, t: &Theme) -> Option<AnyElement> {
        let strip = |height: f32, label: &'static str, spin: bool, turn: f32| {
            let arrow = icon("reload", 15., t.violet);
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .gap(px(space::S))
                .h(px(height.max(0.)))
                .overflow_hidden()
                .child(if spin && !motion::reduced() {
                    spinning(arrow, "pull-spin")
                } else {
                    arrow
                        .with_transformation(Transformation::rotate(percentage(turn)))
                        .into_any_element()
                })
                .child(meta(label, t.text3))
        };
        if self.pull > 0. {
            // Following the finger: no animation, just the stretched height.
            let shown = rubber_band(self.pull);
            let ready = shown >= PULL_TRIGGER;
            return Some(
                strip(
                    shown,
                    if ready {
                        "Release to refresh"
                    } else {
                        "Pull to refresh"
                    },
                    false,
                    (shown / PULL_TRIGGER).min(1.) * 0.75,
                )
                .opacity(0.45 + 0.55 * (shown / PULL_TRIGGER).min(1.))
                .into_any_element(),
            );
        }
        let (from, to, start) = self.pull_anim?;
        let label = if self.pull_hold {
            "Checking for mail…"
        } else {
            ""
        };
        Some(
            strip(settle(from, to, start), label, self.pull_hold, 0.75)
                .with_animation(
                    ("pull-settle", self.pull_gen),
                    // The animation only keeps frames coming; the height follows the spring.
                    Animation::new(Duration::from_secs_f32(SETTLE_SECONDS + 0.1)),
                    move |strip, _| strip.h(px(settle(from, to, start))),
                )
                .into_any_element(),
        )
    }

    /// The message list in a wrapper whose width slides open or shut after a toggle, the
    /// same way the sidebar does. Once it has finished closing it is not drawn at all.
    pub(super) fn list_slot(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let moving = self
            .list_toggled
            .is_some_and(|at| at.elapsed() < motion::SLIDE);
        if !self.list_open && !moving {
            return None;
        }
        // The panel and the gutter on its right.
        let width = self.list_width + GUTTER;
        // The list keeps its width while the slot around it grows or shrinks, so its rows
        // slide instead of re-wrapping on every frame.
        let slot = div()
            .flex()
            .justify_end()
            .flex_none()
            .h_full()
            .overflow_hidden()
            .child(
                div()
                    .flex_none()
                    .w(px(width))
                    .h_full()
                    .pr(px(GUTTER))
                    .child(self.list(t, cx)),
            );
        if !moving {
            return Some(slot.w(px(width)).into_any_element());
        }
        let (from, to) = if self.list_open { (0., 1.) } else { (1., 0.) };
        Some(
            slot.with_animation(
                ("list-width", self.list_gen),
                Animation::new(motion::SLIDE).with_easing(motion::drawer),
                move |slot, delta| slot.w(px(width * (from + (to - from) * delta))),
            )
            .into_any_element(),
        )
    }

    /// Hides or shows the message list, leaving the reading pane the full width.
    pub(super) fn toggle_list(&mut self, cx: &mut Context<Self>) {
        self.list_open = !self.list_open;
        if self.list_open {
            self.focus_restore = None;
        }
        self.list_grab = None;
        self.list_toggled = (!motion::reduced()).then(std::time::Instant::now);
        self.list_gen += 1;
        cx.notify();
    }

    /// A sidebar row with the keyboard's cursor on it: lit, ringed, and scrolled to.
    pub(super) fn cursor_on<E: Styled + ParentElement>(
        &self,
        el: E,
        t: &Theme,
        cursor: &Option<SidebarRow>,
        row: SidebarRow,
        open: bool,
        radius: f32,
    ) -> E {
        if cursor.as_ref() != Some(&row) {
            return el;
        }
        // Settings sits below the scrolled rows, always in view.
        let scrolled = row != SidebarRow::Settings;
        let el = if open { el } else { el.bg(t.hover) };
        let el = el.relative().child(focus_ring(t, radius));
        if !scrolled {
            return el;
        }
        el.child(reveal(
            self.sidebar_scroll.clone(),
            self.reveal_cursor.clone(),
            SIDEBAR_CLEAR,
        ))
    }

    /// The width the row's text has: the list, less its margins, the row's padding, the
    /// avatar and the gap after it.
    fn row_room(&self) -> f32 {
        self.list_width - 2. * LIST_SIDE - 3. * ROW_PAD - AVATAR
    }

    /// Sender, label, subject and snippet, each cut where its real width runs out.
    fn row_text(&self, m: &Mail, right: f32, cx: &App) -> RowText {
        let room = self.row_room();
        let accents = [
            Accent::Violet,
            Accent::Rose,
            Accent::Amber,
            Accent::Sky,
            Accent::Mint,
        ];
        let tag = m.tag.clone().or_else(|| {
            self.thread_labels(m)
                .into_iter()
                .next()
                .map(|(index, label)| (label.name.clone().into(), accents[index % accents.len()]))
        });
        let sans = |weight| gpui::Font {
            weight,
            ..gpui::font(crate::theme::SANS)
        };
        let gap = 8.;
        let count = m
            .thread
            .as_ref()
            .map_or(0., |n| gap + text_width(n, 11., crate::theme::MONO, cx));
        let star = if m.starred { gap + 13. } else { 0. };
        let sender_room = room - count - star - right - gap;
        // On a narrow list the label gives way, so the sender's name stays readable.
        let tag = tag.and_then(|(label, accent)| {
            let label = fit(&label, 120., &sans(FontWeight::NORMAL), 12., cx).0;
            let pill = gap + PILL_CHROME + text_width(&label, 12., crate::theme::SANS, cx);
            (sender_room - pill >= SENDER_MIN).then_some(((label, accent), pill))
        });
        let sender_room = sender_room - tag.as_ref().map_or(0., |(_, pill)| *pill);
        let (sender_weight, subject_weight) = if m.unread {
            (FontWeight::SEMIBOLD, FontWeight::MEDIUM)
        } else {
            (FontWeight::MEDIUM, FontWeight::NORMAL)
        };
        let sender = fit(&m.sender, sender_room, &sans(sender_weight), 13., cx).0;
        let subject = m.subject.split(',').next().unwrap_or(&m.subject);
        let (subject, used) = fit(subject, room, &sans(subject_weight), 13., cx);
        let snippet_room = room - used - 6.;
        let snippet = if snippet_room >= 40. {
            fit(&m.snippet, snippet_room, &sans(FontWeight::NORMAL), 13., cx).0
        } else {
            SharedString::default()
        };
        RowText {
            sender,
            tag: tag.map(|(tag, _)| tag),
            subject,
            snippet,
        }
    }

    fn row(&self, t: &Theme, i: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let m = &self.mails[i];
        // Snoozed here: a clock (a bell for a reminder) and the day it comes back.
        let snoozed = super::threads::snooze_of(
            self.snoozed.iter().chain(&self.snoozed_elsewhere),
            m.id.as_deref(),
        );
        let picked = self.picked.contains(&i);
        let selected = i == self.selected && self.picked.len() < 2;
        let lit = selected || picked;
        // The right end of the first line: when it comes back, or when it came.
        let right = match &snoozed {
            Some((_, when)) => 27. + text_width(when, 11., crate::theme::MONO, cx),
            None => text_width(&m.time, 11., crate::theme::MONO, cx),
        };
        let text = self.row_text(m, right, cx);
        let ground = if lit { t.card } else { over(t.hover, t.panel) };
        let (primary, secondary) = if m.unread {
            (t.text, t.text)
        } else {
            (t.text2, t.text2)
        };

        let (key, account) = self.thread_key(m);
        let held = over(t.pressed, t.card);
        div()
            .id(SharedString::from(format!("row:{account}:{key}")))
            .group("thread-row")
            .map(|d| {
                if lit {
                    d.active(move |s| s.bg(held))
                } else {
                    d.press(t)
                }
            })
            .relative()
            .flex()
            .flex_none()
            .h(px(ROW_H))
            .gap(px(ROW_PAD))
            .px(px(ROW_PAD))
            .py(px(11.))
            .rounded(px(radius::ROW))
            .cursor_pointer()
            // The open row is a card raised off the panel, lit from inside.
            .when(lit, |d| d.bg(t.card).shadow(card_glow(t)))
            // Its edge is drawn over the row rather than as its border, which would take up
            // space and make the row (and those below it) jump.
            .when(lit, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(px(radius::ROW))
                        .border_1()
                        .border_color(if picked {
                            t.violet.opacity(0.5)
                        } else {
                            t.card_edge
                        }),
                )
            })
            .when(!lit, |d| {
                let hover = t.hover;
                d.hover(move |s| s.bg(hover))
            })
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    this.open_row_menu(i, event.position, window, cx)
                }),
            )
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                this.click_row(i, event.modifiers(), event.click_count(), cx)
            }))
            // In the padding, so the avatar has the same space either side, and level with
            // the first line, which is the sender it is about.
            .when(m.unread, |d| {
                d.child(
                    div()
                        .absolute()
                        .left(px((ROW_PAD - UNREAD_DOT) / 2.))
                        .top(px(11. + (22. - UNREAD_DOT) / 2.))
                        .size(px(UNREAD_DOT))
                        .rounded_full()
                        .bg(t.violet),
                )
            })
            .child(div().flex().flex_none().items_center().child(avatar(
                m.tone,
                &m.initials,
                AVATAR,
            )))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.))
                    .gap(px(3.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::S))
                            .h(px(22.))
                            .child(
                                div()
                                    .flex_none()
                                    .whitespace_nowrap()
                                    .font_weight(if m.unread {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::MEDIUM
                                    })
                                    .text_color(primary)
                                    .child(text.sender),
                            )
                            .children(m.thread.clone().map(|n| caption(n, t.text3)))
                            .when(m.starred, |d| d.child(icon("star", 13., t.amber)))
                            .children(text.tag.map(|(label, a)| pill(t, &label, t.accent(a))))
                            .child(div().flex_1())
                            .children(snoozed.as_ref().map(|(name, when)| {
                                super::threads::snooze_chip(t, name, when.clone())
                                    .when(!selected, |d| {
                                        d.group_hover("thread-row", |s| s.invisible())
                                    })
                                    .when(selected, |d| d.invisible())
                            }))
                            .when(snoozed.is_none(), |d| {
                                d.child(
                                    caption(m.time.clone(), t.text3)
                                        .when(!selected, |d| {
                                            d.group_hover("thread-row", |s| s.invisible())
                                        })
                                        .when(selected, |d| d.invisible()),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .h(px(18.))
                            .child(
                                div()
                                    .flex_none()
                                    .whitespace_nowrap()
                                    .font_weight(if m.unread {
                                        FontWeight::MEDIUM
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(secondary)
                                    .child(text.subject),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .text_color(t.text3)
                                    .child(text.snippet),
                            ),
                    ),
            )
            .child(self.quick_actions(t, i, selected, ground, cx))
    }

    /// Done and Snooze over the end of the first line: on the open row, and on any row
    /// under the pointer. Their keys show only on the open row, which is what the keys act on.
    fn quick_actions(
        &self,
        t: &Theme,
        i: usize,
        selected: bool,
        ground: Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let quick = |id: &'static str, name: &'static str| {
            let (hover, bright) = (t.hover, t.text);
            div()
                .id((id, i))
                .group("quick")
                .flex()
                .items_center()
                .justify_center()
                .size(px(24.))
                .rounded(px(radius::CHIP))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .press(t)
                .child(icon(name, 15., t.text2).group_hover("quick", move |s| s.text_color(bright)))
        };
        let key = |key: &'static str| if selected { key } else { "" };
        let done = self.done_change().map(|_| {
            let label = if inbox_like(self.mailbox) {
                "Mark Done"
            } else {
                "Move to Inbox"
            };
            quick("quick-done", "done")
                .tooltip(tip(t.dark, label, key("E")))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    if i == this.selected && this.picked.len() < 2 {
                        this.mark_done(cx);
                    } else {
                        this.done_thread(i, cx);
                    }
                }))
        });
        let snooze = inbox_like(self.mailbox).then(|| {
            quick("quick-snooze", "clock")
                .tooltip(tip(t.dark, "Snooze", key("H")))
                .on_click(
                    cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.picked.clear();
                        this.open(i);
                        this.open_selected(cx);
                        this.open_snooze(Some(event.position()), window, cx);
                    }),
                )
        });
        div()
            .absolute()
            .top(px(10.))
            .right(px(ROW_PAD - 4.))
            .h(px(24.))
            .flex()
            .items_center()
            .when(!selected, |d| {
                d.invisible().group_hover("thread-row", |s| s.visible())
            })
            // The row's own colour, fading in from the left over what it covers.
            .child(
                div()
                    .w(px(20.))
                    .h_full()
                    .bg(gradient(90., ground.opacity(0.), ground)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .h_full()
                    .bg(ground)
                    .rounded_r(px(radius::CHIP))
                    .children(done)
                    .children(snooze),
            )
    }

    /// One item of the virtual list, built only while it is in view (or about to be).
    fn list_item(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme();
        let Some(item) = self.list_scroll.items.get(ix).copied() else {
            return div().into_any_element();
        };
        let item_box = || div().px(px(LIST_SIDE)).h(px(item.height()));
        match item {
            Item::Group(group) => item_box()
                .pl(px(LIST_SIDE + ROW_PAD))
                .pt(px(16.))
                .child(overline(
                    &t,
                    match group {
                        Group::Today => "TODAY",
                        Group::Earlier => "EARLIER",
                    },
                ))
                .into_any_element(),
            Item::Row { index, .. } if index < self.mails.len() => {
                item_box().child(self.row(&t, index, cx)).into_any_element()
            }
            Item::Row { .. } => item_box().into_any_element(),
            Item::More(n) => item_box().child(self.more_row(&t, n)).into_any_element(),
        }
    }

    /// A row of the next page, before it is here: the shape of a row, breathing while it
    /// loads and still otherwise. The virtual list only builds it once it is near the view.
    fn more_row(&self, t: &Theme, n: usize) -> AnyElement {
        let bar = |w: f32, h: f32| div().h(px(h)).w(px(w)).rounded_full().bg(t.hover);
        let row = div()
            .flex()
            .flex_none()
            .h(px(ROW_H))
            .gap(px(ROW_PAD))
            .px(px(ROW_PAD))
            .py(px(11.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(div().size(px(AVATAR)).rounded_full().bg(t.hover)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .h(px(22.))
                            .child(bar(if n == 0 { 120. } else { 96. }, 10.)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .h(px(18.))
                            .child(bar(if n == 0 { 220. } else { 180. }, 8.)),
                    ),
            );
        if !self.loading_more || motion::reduced() {
            return row.opacity(0.7).into_any_element();
        }
        row.with_animation(
            ("more-loading", n),
            Animation::new(Duration::from_millis(1400)).repeat(),
            move |row, delta| {
                let wave = (std::f32::consts::TAU * (delta - n as f32 * 0.15)).cos();
                row.opacity(0.55 + 0.35 * wave)
            },
        )
        .into_any_element()
    }

    fn list(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let unread = self
            .list_scroll
            .items
            .iter()
            .filter(|item| matches!(item, Item::Row { index, .. } if self.mails[*index].unread))
            .count();

        // Each option has a fixed width, so the highlight behind them can slide from one to
        // the other (and nothing shifts when it does).
        let unread_text = unread.to_string();
        let widths = [44., 70. + 7. * unread_text.len() as f32];
        // The switch: its options, the 2pt between them, its padding and its border.
        let switch_w = widths[0] + widths[1] + 2. + 8.;
        // Dot, gap, padding and border around the pill's words.
        let offline_w = self
            .offline
            .then(|| 30. + text_width(&self.offline_label(), 12., crate::theme::SANS, cx));
        let fit = HeaderFit::new(
            self.list_width - 1. - LIST_LEFT - LIST_RIGHT,
            switch_w,
            offline_w,
        );
        let segment = |label: &str, count: Option<&str>, active: bool, width: f32| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .w(px(width))
                .h(px(SEGMENT_H))
                .rounded(px(radius::CONTROL))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if active { t.text } else { t.text3 })
                .child(label.to_string())
                .children(count.map(|n| caption(n.to_string(), t.text3)))
        };
        // The highlight: where it is, and where it was before the last switch.
        let spot = |unread: bool| {
            if unread {
                (widths[0] + 2., widths[1])
            } else {
                (0., widths[0])
            }
        };
        let (to_x, to_w) = spot(self.unread_only);
        let (from_x, from_w) = spot(!self.unread_only);
        let pill = div()
            .absolute()
            .top(px(3.))
            .h(px(SEGMENT_H))
            .rounded(px(radius::CONTROL))
            .bg(t.raised)
            .border_1()
            .border_color(t.edge)
            .shadow(vec![shadow(t.shadow.opacity(0.5), 1., 3., 0.)]);
        // Only right after a switch (not again each time the list is drawn anew).
        let switching = self
            .filter_switched
            .is_some_and(|at| at.elapsed() < motion::PILL);
        let pill = if self.filter_gen == 0 || !switching || motion::reduced() {
            pill.left(px(3. + to_x)).w(px(to_w)).into_any_element()
        } else {
            pill.with_animation(
                ("filter-pill", self.filter_gen),
                Animation::new(motion::PILL).with_easing(motion::ease_out),
                move |d, delta| {
                    d.left(px(3. + from_x + (to_x - from_x) * delta))
                        .w(px(from_w + (to_w - from_w) * delta))
                },
            )
            .into_any_element()
        };

        let rows = gpui::list(
            self.list_scroll.state.clone(),
            cx.processor(|this, ix: usize, _: &mut Window, cx| this.list_item(ix, cx)),
        )
        .flex_1()
        .w_full()
        .pb(px(LIST_BOTTOM));

        panel(t, Depth::Back)
            .flex()
            .flex_col()
            .flex_none()
            .w(px(self.list_width))
            .h_full()
            // Drag the gutter on the right to resize the list; double-click it to hide the list.
            .child({
                let hover = t.aura;
                div()
                    .id("list-resize")
                    .absolute()
                    .top(px(radius::SHEET))
                    .bottom(px(radius::SHEET))
                    .right(px(-GUTTER))
                    .w(px(GUTTER))
                    .rounded_full()
                    .cursor(gpui::CursorStyle::ResizeLeftRight)
                    .hover(move |s| s.bg(hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            if event.click_count == 2 {
                                this.toggle_list(cx);
                            } else {
                                this.list_grab =
                                    Some((f32::from(event.position.x), this.list_width));
                            }
                        }),
                    )
            })
            .child(
                // One row on the top line (y = 25): the filters, then Offline and Compose.
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(HEADER_GAP))
                    .h(px(TOP_BAR - 1.))
                    .pl(px(LIST_LEFT))
                    .pr(px(LIST_RIGHT))
                    .on_mouse_down(MouseButton::Left, drag_or_zoom)
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_none()
                            .gap(px(2.))
                            .p(px(3.))
                            .rounded(px(radius::ROW))
                            .bg(t.hover)
                            .border_1()
                            .border_color(t.border)
                            .child(pill)
                            .child(
                                segment("All", None, !self.unread_only, widths[0])
                                    .id("filter-all")
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_unread_only(false, cx)
                                    })),
                            )
                            .child(
                                segment("Unread", Some(&unread_text), self.unread_only, widths[1])
                                    .id("filter-unread")
                                    .cursor_pointer()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.set_unread_only(true, cx)
                                        }),
                                    ),
                            ),
                    )
                    .child(div().flex_1().min_w(px(0.)).mx(px(-HEADER_GAP / 2.)))
                    .when(self.offline, |d| d.child(self.offline_pill(t, fit.words)))
                    .child(
                        compose_button(t, fit.compose)
                            .id("compose")
                            .press(t)
                            .tooltip(tip(t.dark, "New Message", "C"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_compose(ComposeKind::New, cx)
                            })),
                    ),
            )
            // While mail is checked, a thin bar glides along the top of the list. Its 2px
            // strip is always there, so nothing below it moves.
            .child(
                div()
                    .relative()
                    .flex_none()
                    .h(px(2.))
                    .mx(px(18.))
                    .rounded_full()
                    .overflow_hidden()
                    .when(self.bar_shown || self.bar_fade.is_some(), |d| {
                        d.child(self.checking_bar(t))
                    }),
            )
            .child(
                div()
                    .id("rows")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.))
                    .on_scroll_wheel(cx.listener(|this, event, _, cx| this.pull_list(event, cx)))
                    .children(self.pull_indicator(t))
                    .child(rows),
            )
            .child(rim(t, radius::SHEET))
    }

    /// The light gliding along the top of the list while mail is checked, fading out once
    /// the check is done. With Reduce Motion it holds still.
    fn checking_bar(&self, t: &Theme) -> AnyElement {
        let light = gradient(90., c(GRADIENT_GLOW.from, 0.), c(GRADIENT_GLOW.to, 1.));
        let glide = div().absolute().top_0().h_full().rounded_full().bg(light);
        let glide = if motion::reduced() {
            glide
                .left(relative(0.))
                .w_full()
                .opacity(0.6)
                .into_any_element()
        } else {
            glide
                .w(relative(0.45))
                .with_animation(
                    "checking-mail",
                    Animation::new(Duration::from_millis(1150))
                        .repeat()
                        .with_easing(gpui::ease_in_out),
                    |bar, delta| bar.left(relative(-0.45 + 1.45 * delta)),
                )
                .into_any_element()
        };
        let strip = div().absolute().inset_0().bg(t.border).child(glide);
        match self.bar_fade {
            Some((_, n)) => strip
                .with_animation(
                    ("checking-mail-fade", n),
                    Animation::new(motion::FADE).with_easing(motion::ease_out),
                    |strip, delta| strip.opacity(1. - delta),
                )
                .into_any_element(),
            None => strip.into_any_element(),
        }
    }

    /// An empty mailbox other than the inbox (which gets the Inbox Zero screen): what it is,
    /// and at most one thing to do about it.
    pub(super) fn empty_mailbox(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        // Not empty at all: filtered, or the rest could not be had.
        let filtered = self.unread_only && !self.all_cleared();
        let unlisted = match self.mailbox {
            View::AllInboxes => !self.all_inboxes_whole,
            view => self.next_pages.contains_key(&view),
        };
        let title: SharedString = if filtered {
            "Nothing Unread".into()
        } else if self.mailbox == View::Search {
            "No Results".into()
        } else if unlisted {
            "Couldn’t Load Everything".into()
        } else {
            format!("Nothing in {}", self.inbox_name()).into()
        };
        let action = if filtered {
            Some(
                button(t, ButtonKind::Secondary, "Show All")
                    .id("empty-action")
                    .press(t)
                    .on_click(cx.listener(|this, _, _, cx| this.set_unread_only(false, cx))),
            )
        } else if unlisted {
            Some(
                button(t, ButtonKind::Secondary, "Try Again")
                    .id("empty-action")
                    .press(t)
                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
            )
        } else {
            None
        };
        panel(t, Depth::Front)
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(space::XL))
            .child(title_l(title, t.text2))
            .children(action)
            .child(rim(t, radius::SHEET))
    }

    /// The night over Inbox Zero is still. Once it has come up, one star falls, drawn at the
    /// display's rate while it does; before that a single wake is all that is waiting. Not
    /// with Reduce Motion, and not while the window is in the background.
    pub(super) fn move_sky(
        &mut self,
        zero_shown: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !zero_shown {
            self.sky = None;
            self.sky_wake = None;
            return;
        }
        if self.arrival.is_some() || motion::reduced() {
            return;
        }
        let now = std::time::Instant::now();
        let sky = self.sky.get_or_insert_with(|| crate::sky::Sky::new(now));
        if !window.is_window_active() {
            sky.let_go(now);
            self.sky_wake = None;
            return;
        }
        if sky.falling(now) {
            window.request_animation_frame();
        } else if let Some(wait) = sky.until_fall(now)
            && self.sky_wake.is_none()
        {
            self.sky_wake = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(wait).await;
                this.update(cx, |this, cx| {
                    this.sky_wake = None;
                    cx.notify();
                })
                .ok();
            }));
        }
    }

    pub(super) fn zero(&self, t: &Theme) -> impl IntoElement {
        let corner = radius::SHEET;
        let ink = c(0xf5f3ef, 1.);
        let soft = c(0xffffff, 0.7);
        let glass = |d: Div| {
            d.bg(c(0xffffff, 0.07))
                .border_1()
                .border_color(c(0xffffff, 0.14))
                .rounded(px(radius::CARD))
        };
        let stat = |name: &str, color: Hsla, label: &str, value: &str| {
            glass(div())
                .flex()
                .flex_col()
                .gap(px(6.))
                .pl(px(20.))
                .pr(px(28.))
                .py(px(16.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(icon(name, 16., color))
                        .child(meta(label.to_string(), soft)),
                )
                .child(display(value.to_string(), 40., ink))
        };
        let arrival = self.arrival.unwrap_or(Arrival::SETTLED);
        let settle = |d: Div, shown: f32| d.relative().top(px(drop_for(shown))).opacity(shown);
        // Deterministic star field.
        let mut seed = 7u32;
        let mut rnd = move || {
            seed = (seed.wrapping_mul(9301).wrapping_add(49297)) % 233280;
            seed as f32 / 233280.
        };
        let stars: Vec<Div> = (0..48)
            .map(|i| {
                let (x, y, a) = (rnd(), rnd() * 0.42, 0.2 + rnd() * 0.6);
                let size = if a > 0.7 { 2.5 } else { 1.5 };
                div()
                    .absolute()
                    .left(relative(x))
                    .top(relative(y))
                    .size(px(size))
                    .rounded_full()
                    .bg(c(0xffffff, a * arrival.star(i)))
            })
            .collect();
        let now = std::time::Instant::now();
        let falling = self
            .sky
            .as_ref()
            .and_then(|sky| sky.fall(now))
            .map(|(fall, dots)| {
                div()
                    .absolute()
                    .left(relative(fall.x))
                    .top(relative(fall.y))
                    .children(dots.into_iter().map(|dot| {
                        div()
                            .absolute()
                            .left(px(dot.x - dot.size / 2.))
                            .top(px(dot.y - dot.size / 2.))
                            .size(px(dot.size))
                            .rounded_full()
                            .bg(c(0xfffaf0, dot.alpha))
                    }))
            });
        // The sample inbox keeps the day its mail is from.
        let today = if self.is_demo() {
            "THURSDAY, NOVEMBER 20".to_string()
        } else {
            chrono::Local::now()
                .format("%A, %B %-d")
                .to_string()
                .to_uppercase()
        };
        let layer = |index: usize, angle: f32, color: u32, alpha: f32, end: f32| {
            let shown = arrival.glow(index);
            // Each reaches further from its edge as it brightens, so the light rises.
            let reach = end * (0.6 + 0.4 * shown);
            div().absolute().inset_0().rounded(px(corner)).bg(fade(
                angle,
                color,
                alpha * shown,
                reach,
            ))
        };

        let below = div()
            .w_full()
            .px(px(space::XXL))
            .text_center()
            .font_weight(FontWeight::LIGHT)
            .text_size(px(22.))
            .line_height(px(30.))
            .text_color(soft)
            .child(crate::inspiration::line_for(
                chrono::Local::now().date_naive(),
            ));

        div().flex_1().min_w(px(0.)).h_full().child(
            div()
                .relative()
                .flex()
                .flex_col()
                .items_center()
                .size_full()
                .overflow_hidden()
                .rounded(px(corner))
                .shadow(aura(t, Depth::Front))
                .bg(c(0x0a0912, 1.))
                // Aurora: GPUI has no blur filter, so the glow is layered two-stop gradients.
                .child(layer(0, 45., 0x6b4dff, 0.9, 0.62))
                .child(layer(2, 315., 0xffa94d, 0.6, 0.5))
                .child(layer(1, 0., 0xff4fa0, 0.55, 0.42))
                .child(layer(3, 300., 0x3fa9ff, 0.22, 0.4))
                .children(stars)
                .children(falling)
                // Two halves with the horizon between: each holds its content in its middle.
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .w_full()
                        .h(relative(HORIZON))
                        .child(settle(
                            overline(t, today).text_color(c(0xffffff, 0.55)),
                            arrival.date(),
                        ))
                        .child(div().h(px(18.)).flex_none())
                        .child(settle(hero("Inbox zero.", ink), arrival.headline()))
                        // Sample content: real mail shows the headline only.
                        .when(self.is_demo(), |d| {
                            d.child(
                                settle(div(), arrival.line())
                                    .flex()
                                    .mt(px(40.))
                                    .gap(px(space::M))
                                    .child(stat(
                                        "done",
                                        t.mint,
                                        "threads handled",
                                        &(32 + self.history.len()).to_string(),
                                    ))
                                    .child(stat("clock", t.sky, "time in inbox", "38m"))
                                    .child(stat("zap", t.amber, "zero streak", "8 days")),
                            )
                        }),
                )
                // Drawn out from the middle.
                .child(
                    div()
                        .flex_none()
                        .w(relative(arrival.horizon()))
                        .h(px(1.))
                        .bg(c(0xffffff, 0.22)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .w_full()
                        .gap(px(18.))
                        .when(self.is_demo(), |d| {
                            d.child(
                                glass(div())
                                    .bg(c(0x14131c, 0.72))
                                    .flex()
                                    .items_center()
                                    .gap(px(14.))
                                    .pl(px(14.))
                                    .pr(px(10.))
                                    .py(px(10.))
                                    .text_color(ink)
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .size(px(34.))
                                            .rounded(px(radius::CONTROL))
                                            .bg(Tone::Sky.gradient())
                                            .child(icon("calendar", 16., gpui::white())),
                                    )
                                    .child(
                                        div()
                                            .mr(px(40.))
                                            .child(
                                                div()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .child("Design Crit"),
                                            )
                                            .child(meta("2:00 PM · Studio B", c(0xffffff, 0.55))),
                                    )
                                    .child(button(&Theme::dark(), ButtonKind::Secondary, "Join")),
                            )
                        })
                        .child(settle(below, arrival.below())),
                )
                .child(rim(&Theme::dark(), corner)),
        )
    }
}

/// The light the open row's card gives off into the panel.
fn card_glow(t: &Theme) -> Vec<gpui::BoxShadow> {
    vec![
        shadow(
            t.shadow.opacity(if t.dark { 0.5 } else { 0.25 }),
            4.,
            12.,
            -6.,
        ),
        shadow(t.aura.opacity((t.aura.a * 1.4).min(1.)), 0., 20., -4.),
    ]
}

/// A turning reload arrow, for as long as it is drawn.
pub(super) fn spinning(arrow: gpui::Svg, id: &'static str) -> AnyElement {
    arrow
        .with_animation(
            id,
            Animation::new(Duration::from_millis(900)).repeat(),
            |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
        )
        .into_any_element()
}

/// A secondary button with its icon before its label.
pub(super) fn labelled(t: &Theme, name: &str, label: &str) -> Div {
    let (selected, strong) = (t.selected, t.border_strong);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .h(px(34.))
        .pl(px(10.))
        .pr(px(14.))
        .rounded(px(radius::CONTROL))
        .bg(t.hover)
        .border_1()
        .border_color(t.border_strong)
        .cursor_pointer()
        .hover(move |s| s.bg(selected).border_color(strong))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.text)
        .child(icon(name, 14., t.text2))
        .child(label.to_string())
}

/// Compose, as wide as the header lets it be.
pub(super) fn compose_button(t: &Theme, look: ComposeLook) -> Div {
    match look {
        ComposeLook::Labelled => labelled(t, "draft", "Compose"),
        ComposeLook::Icon => icon_button(t, "draft")
            .size(px(34.))
            .bg(t.hover)
            .border_1()
            .border_color(t.border_strong),
    }
}

/// The space either side of a row's avatar, and at the row's right. With the list's own
/// 10pt margin it puts the avatars on `LIST_LEFT`.
pub(super) const ROW_PAD: f32 = 14.;

const AVATAR: f32 = 28.;

const UNREAD_DOT: f32 = 7.;

/// What a label pill adds around its words: padding, dot, gap and border.
const PILL_CHROME: f32 = 7. + 6. + 6. + 9. + 2.;

/// The list header's margins: the list's text starts and ends on them.
pub(super) const LIST_LEFT: f32 = 24.;

pub(super) const LIST_RIGHT: f32 = 18.;

pub(super) const HEADER_GAP: f32 = 8.;

/// How Compose is drawn, as the room for it shrinks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ComposeLook {
    Labelled,
    Icon,
}

impl ComposeLook {
    pub(super) fn width(self) -> f32 {
        match self {
            ComposeLook::Labelled => 112.,
            ComposeLook::Icon => 34.,
        }
    }
}

/// What fits in the list's one-row header. The list can be 300pt narrow, so first the
/// Offline pill loses its words, then Compose its label.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct HeaderFit {
    pub(super) words: bool,
    pub(super) compose: ComposeLook,
}

impl HeaderFit {
    pub(super) fn new(room: f32, switch: f32, offline: Option<f32>) -> Self {
        use ComposeLook::{Icon, Labelled};
        for words in [true, false] {
            let pill = offline.map_or(0., |wide| HEADER_GAP + if words { wide } else { 22. });
            if switch + pill + HEADER_GAP + Labelled.width() <= room {
                return HeaderFit {
                    words,
                    compose: Labelled,
                };
            }
        }
        HeaderFit {
            words: false,
            compose: Icon,
        }
    }
}

/// How far down Inbox Zero's horizon is.
pub(super) const HORIZON: f32 = 0.72;

/// The height of an option in the All / Unread switch.
pub(super) const SEGMENT_H: f32 = 26.;

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u64) -> Item {
        Item::Row { index: 0, id }
    }

    fn scroll(items: Vec<Item>) -> ListScroll {
        let mut list = ListScroll::new();
        list.update(items);
        list
    }

    #[test]
    fn a_thread_keeps_its_item_when_it_moves_in_mails() {
        assert!(row(7).same(&Item::Row { index: 40, id: 7 }));
        assert!(!row(7).same(&row(8)));
        assert!(!Item::Group(Group::Today).same(&Item::Group(Group::Earlier)));
    }

    #[test]
    fn new_mail_at_the_top_keeps_the_thread_in_view() {
        let mut list = scroll((0..50).map(row).collect());
        list.state.scroll_to(gpui::ListOffset {
            item_ix: 10,
            offset_in_item: px(5.),
        });
        let mut items = vec![row(99)];
        items.extend((0..50).map(row));
        list.update(items);
        let top = list.state.logical_scroll_top();
        assert_eq!((top.item_ix, top.offset_in_item), (11, px(5.)));
    }

    #[test]
    fn a_refresh_that_reorders_the_top_follows_the_thread() {
        let mut list = scroll((0..20).map(row).collect());
        list.state.scroll_to(gpui::ListOffset {
            item_ix: 3,
            offset_in_item: px(2.),
        });
        // Threads 0 to 5 come back in another order; thread 3 is now fifth.
        let mut items: Vec<Item> = [5, 1, 0, 4, 3, 2].into_iter().map(row).collect();
        items.extend((6..20).map(row));
        list.update(items);
        assert_eq!(list.state.logical_scroll_top().item_ix, 4);
    }

    #[test]
    fn positions_come_from_the_known_heights() {
        let mut items = vec![Item::Group(Group::Today)];
        items.extend((0..10).map(row));
        let list = scroll(items);
        list.state.scroll_to(gpui::ListOffset {
            item_ix: 3,
            offset_in_item: px(4.),
        });
        assert_eq!(list.top(), GROUP_H + 2. * (ROW_H + ROW_GAP) + 4.);
        assert!(!list.at_top());
        list.to_top();
        assert!(list.at_top());
        assert_eq!(
            list.content(),
            GROUP_H + 10. * (ROW_H + ROW_GAP) + LIST_BOTTOM
        );
    }

    #[test]
    fn an_unchanged_list_is_not_laid_out_again() {
        let mut list = scroll((0..5).map(row).collect());
        list.state.scroll_to(gpui::ListOffset {
            item_ix: 2,
            offset_in_item: px(1.),
        });
        list.update((0..5).map(row).collect());
        assert_eq!(list.state.logical_scroll_top().item_ix, 2);
        assert_eq!(list.state.item_count(), 5);
    }

    #[test]
    fn compose_loses_its_label_only_when_it_must() {
        let wide = HeaderFit::new(452. - 1. - LIST_LEFT - LIST_RIGHT, 131., Some(126.));
        assert_eq!((wide.words, wide.compose), (true, ComposeLook::Labelled));
        let narrow = HeaderFit::new(300. - 1. - LIST_LEFT - LIST_RIGHT, 131., Some(126.));
        assert_eq!(narrow.compose, ComposeLook::Icon);
    }
}
