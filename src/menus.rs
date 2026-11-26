//! Menus and the other small pop-ups (the Undo pill, notices). Each is a window of its own,
//! so it can lie over the email, which is a native view above everything GPUI draws.

use gpui::Task;

use super::*;
use crate::theme::{radius, space};
use crate::ui::motion;

/// What a row's right-click menu can do.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum RowAction {
    MarkRead,
    MarkUnread,
    Star,
    Unstar,
    Done,
    MoveToInbox,
    Trash,
    DeleteForever,
    ReportSpam,
    OpenInGmail,
}

impl RowAction {
    /// Icon and label for the menu.
    pub(super) fn label(self) -> (&'static str, &'static str) {
        match self {
            RowAction::MarkRead => ("mail", "Mark as Read"),
            RowAction::MarkUnread => ("mail", "Mark as Unread"),
            RowAction::Star => ("star", "Star"),
            RowAction::Unstar => ("star", "Unstar"),
            RowAction::Done => ("done", "Mark Done"),
            RowAction::MoveToInbox => ("inbox", "Move to Inbox"),
            RowAction::Trash => ("trash", "Move to Trash"),
            RowAction::DeleteForever => ("trash", "Delete Forever"),
            RowAction::ReportSpam => ("ban", "Report Spam"),
            RowAction::OpenInGmail => ("link", "Open in Gmail"),
        }
    }

    /// The key that does the same to the open thread, if there is one.
    pub(super) fn key(self) -> &'static str {
        match self {
            RowAction::MarkRead | RowAction::MarkUnread => "U",
            RowAction::Star | RowAction::Unstar => "S",
            RowAction::Done | RowAction::MoveToInbox => "E",
            RowAction::Trash | RowAction::DeleteForever => "⌫",
            RowAction::ReportSpam | RowAction::OpenInGmail => "",
        }
    }
}

/// What the reader's More menu can do to the open thread.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum MoreAction {
    MarkRead,
    MarkUnread,
    ShowOriginal,
    ReaderView,
    OpenInGmail,
    ReportSpam,
    Block,
    Unblock,
}

impl MoreAction {
    /// Icon, label and the key that does the same.
    pub(super) fn label(self) -> (&'static str, &'static str, &'static str) {
        match self {
            MoreAction::MarkRead => ("mail", "Mark as Read", "U"),
            MoreAction::MarkUnread => ("mail", "Mark as Unread", "U"),
            MoreAction::ShowOriginal => ("eye", "Show Original", "V"),
            MoreAction::ReaderView => ("eye", "Reader View", "V"),
            MoreAction::OpenInGmail => ("link", "Open in Gmail", ""),
            MoreAction::ReportSpam => ("ban", "Report Spam", ""),
            MoreAction::Block => ("ban", "Block Sender", ""),
            MoreAction::Unblock => ("ban", "Unblock Sender", ""),
        }
    }
}

/// Which toolbar button a menu drops from, for `Nimbra::menu_anchors`.
#[derive(Clone, Copy)]
pub(super) enum Anchor {
    Labels,
    MoveTo,
    More,
}

/// A small menu dropped from the reading toolbar.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Menu {
    Labels,
    More,
    /// Which account a new message is from (the composer's From).
    From,
    /// Times to send the open composer's message at.
    SendLater,
    /// Inbox, a label or Spam, for the open thread.
    MoveTo,
    /// Rename or delete a label (right-click in the sidebar), by index.
    Label(usize),
    /// The right-click menu of a row in the list (by index into the loaded threads).
    Row(usize),
}

pub(super) const MENU_WIDTH: f32 = 220.;

/// Closes one of the small pop-up windows (a menu, the Undo or notice pill). While that
/// window is busy (its own button was clicked) it cannot be closed from here, so it is
/// closed right after instead of being left on screen.
pub(super) fn remove_popup<V: 'static>(handle: gpui::WindowHandle<V>, cx: &mut App) {
    if handle
        .update(cx, |_, window, _| window.remove_window())
        .is_err()
    {
        cx.defer(move |cx| {
            handle
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        });
    }
}

/// What a menu row has beside its label: the card's padding (7) and the row's padding (10)
/// on each side, and the icon (16) with its gap (10).
pub(super) const MENU_ROW_CHROME: f32 = 2. * (CARD_PAD + 10.) + 16. + 10.;

/// From a pop-up card's edge to its rows, the rim's hairline included.
pub(super) const CARD_PAD: f32 = 7.;

/// Transparent space around the menu inside its window, so its shadow is not cut off.
pub(super) const MENU_MARGIN: f32 = 28.;

/// A pop-up's lift (offset down, blur, spread).
#[cfg(test)]
pub(super) const POPUP_SHADOW: (f32, f32, f32) = {
    let (down, blur, spread, _) = crate::tooltip::LIFT[0];
    (down, blur, spread)
};

/// The glass every pop-up window draws (menus, the Undo and notice pills): raised, with a
/// hairline edge and a faint neutral shadow. The edge is laid over it and takes no room, so
/// its content sits `CARD_PAD` in.
pub(super) fn popup_card(t: &Theme) -> Div {
    div()
        .relative()
        .rounded(px(radius::CARD))
        .bg(t.raised)
        .shadow(crate::tooltip::lift(t))
        .child(edge(t, radius::CARD))
}

pub(super) use crate::tooltip::edge;

/// Glass laid over the main window (the finder and the sheets): one faint, neutral shadow.
pub(super) fn sheet_shadow(t: &Theme) -> Vec<gpui::BoxShadow> {
    vec![shadow(
        t.shadow_sheet.opacity(t.shadow_sheet.a * 0.6),
        16.,
        40.,
        -12.,
    )]
}

/// The lit row of a menu or the finder: a violet tint under it and, laid over it, a violet
/// hairline. Give the row `.relative()`.
pub(super) fn lit_edge(t: &Theme, corner: f32) -> Div {
    div()
        .absolute()
        .inset_0()
        .rounded(px(corner))
        .border_1()
        .border_color(t.violet.opacity(0.32))
}

/// Solid violet for what is chosen or is the one main action, under a white label:
/// (at rest, hovered, held).
pub(super) fn solid_violet(t: &Theme) -> (Hsla, Hsla, Hsla) {
    (t.primary, t.primary_hover, t.primary_pressed)
}

/// A sheet's one main action.
pub(super) fn primary_button(t: &Theme, id: &'static str, label: &str) -> Stateful<Div> {
    crate::ui::primary(t, id, label)
}

/// How a pop-up's card comes and goes: fading in while it travels `from` points (up when
/// negative) back to its place, and fading out where it is. Opened from the keyboard, it is
/// simply there.
pub(super) struct Reveal {
    shown: motion::Tween,
    from: f32,
    leaving: Option<Task<()>>,
}

impl Reveal {
    pub(super) fn new(animate: bool, from: f32, length: Duration) -> Self {
        let mut shown =
            motion::Tween::fade(if animate { 0. } else { 1. }, length, motion::ease_out);
        shown.retarget(1.);
        Self {
            shown,
            from,
            leaving: None,
        }
    }

    /// The card as it is at this frame.
    pub(super) fn apply(&self, card: Div, window: &mut Window) -> Div {
        let shown = self.shown.frame(window);
        let travel = if motion::reduced() {
            0.
        } else {
            self.from * (1. - shown)
        };
        card.relative().top(px(travel)).opacity(shown)
    }

    pub(super) fn leaving(&self) -> bool {
        self.leaving.is_some()
    }

    /// Fades the card out, then takes its window away.
    pub(super) fn leave<V: 'static>(
        &mut self,
        length: Duration,
        window: &mut Window,
        cx: &mut Context<V>,
    ) {
        if self.leaving() {
            return;
        }
        let mut shown = motion::Tween::fade(self.shown.value(), length, motion::ease_out);
        shown.retarget(0.);
        self.shown = shown;
        self.leaving = Some(cx.spawn_in(window, async move |_, cx| {
            cx.background_executor().timer(length).await;
            cx.update(|window, _| window.remove_window()).ok();
        }));
        cx.notify();
    }
}

/// A pop-up that fades out before its window goes.
pub(super) trait Leave: Render + Sized {
    fn leave(&mut self, window: &mut Window, cx: &mut Context<Self>);
}

/// Lets a pop-up window go the way it goes by itself. While that window is busy (its own
/// button was clicked) it cannot be reached from here, so that happens right after.
pub(super) fn let_go<V: Leave>(handle: gpui::WindowHandle<V>, cx: &mut App) {
    if handle
        .update(cx, |popup, window, cx| popup.leave(window, cx))
        .is_err()
    {
        cx.defer(move |cx| {
            handle
                .update(cx, |popup, window, cx| popup.leave(window, cx))
                .ok();
        });
    }
}

/// How far a toast rises into place from below.
const TOAST_RISE: f32 = 8.;

/// Focus moving away right as a pop-up opens is part of the click that opened it.
pub(super) const SETTLING: Duration = Duration::from_millis(300);

/// One row of a menu: its icon and tint, its label, and its key while that key would do the
/// same thing.
pub(super) struct MenuItem {
    pub(super) icon: &'static str,
    pub(super) label: SharedString,
    pub(super) tint: Hsla,
    pub(super) key: &'static str,
}

fn item(icon: &'static str, label: impl Into<SharedString>, tint: Hsla) -> MenuItem {
    MenuItem {
        icon,
        label: label.into(),
        tint,
        key: "",
    }
}

/// A toolbar menu, shown in a small window of its own (see `Nimbra::toggle_menu`).
pub(super) struct MenuPopup {
    owner: gpui::WeakEntity<Nimbra>,
    menu: Menu,
    items: Vec<MenuItem>,
    /// False for a menu that only carries a message (no labels yet).
    enabled: bool,
    dark: bool,
    /// The row Return would choose: under the pointer, or moved to with the arrows.
    lit: Option<usize>,
    focus: gpui::FocusHandle,
    reveal: Reveal,
    was_active: bool,
    opened: std::time::Instant,
    _activation: gpui::Subscription,
    _settling: Option<Task<()>>,
}

/// A short message at the bottom of the window (see `Nimbra::sync_notice_window`).
pub(super) struct NoticeToast {
    owner: gpui::WeakEntity<Nimbra>,
    toast: Toast,
    dark: bool,
    reveal: Reveal,
    _timer: Task<()>,
}

impl Leave for NoticeToast {
    fn leave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reveal.leave(motion::TOAST_OUT, window, cx);
    }
}

impl Render for NoticeToast {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        let owner = self.owner.clone();
        let card = popup_card(&t)
            .flex()
            .items_center()
            .justify_between()
            .gap(px(toast::GAP))
            .size_full()
            .pl(px(toast::LEFT))
            .pr(px(toast::RIGHT))
            // As wide as it was measured, so that it wraps as it was measured.
            .child(
                div()
                    .flex_none()
                    .w(px(self.toast.text_width))
                    .text_size(px(toast::FONT))
                    .line_height(px(toast::LINE))
                    .child(self.toast.text.clone()),
            )
            .child(
                icon_button(&t, "x")
                    .size(px(toast::DISMISS))
                    .id("dismiss-notice")
                    .press(&t)
                    .tooltip(tip(t.dark, "Dismiss", ""))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        owner
                            .update(cx, |owner, cx| {
                                owner.notice = None;
                                cx.notify();
                            })
                            .ok();
                    })),
            );
        div()
            .size_full()
            .p(px(MENU_MARGIN))
            .font_family(crate::theme::SANS)
            .text_size(px(13.))
            .line_height(px(18.))
            .text_color(t.text)
            .child(self.reveal.apply(card, window))
    }
}

/// The "Undo" pill shown after a thread is removed (see `Nimbra::offer_undo`). It stays while
/// the pointer is on it, and gives a moment more once the pointer leaves.
pub(super) struct UndoToast {
    owner: gpui::WeakEntity<Nimbra>,
    label: SharedString,
    dark: bool,
    shown_gen: usize,
    reveal: Reveal,
    timer: Option<Task<()>>,
}

impl UndoToast {
    const SHOWN: Duration = Duration::from_secs(6);
    const AFTER_HOVER: Duration = Duration::from_secs(3);

    fn time_out(&mut self, after: Duration, window: &mut Window, cx: &mut Context<Self>) {
        self.timer = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(after).await;
            this.update_in(cx, |this, window, cx| {
                let shown_gen = this.shown_gen;
                this.owner
                    .update(cx, |owner, _| {
                        if owner.undo_gen == shown_gen {
                            owner.undo_window = None;
                        }
                    })
                    .ok();
                this.leave(window, cx);
            })
            .ok();
        }));
    }
}

impl Leave for UndoToast {
    fn leave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.timer = None;
        self.reveal.leave(motion::TOAST_OUT, window, cx);
    }
}

impl Render for UndoToast {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        let owner = self.owner.clone();
        // The button sits as far in from the right as from the top and bottom, and the
        // label as far in from the left as the button's own label is from the right.
        let (button_x, button_y) = (14., 7.);
        let card = popup_card(&t)
            .flex()
            .items_center()
            .gap(px(space::M))
            .size_full()
            .pl(px(button_y + button_x))
            .pr(px(button_y))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(self.label.clone()),
            )
            .child(
                button(&t, ButtonKind::Ghost, "Undo")
                    .id("undo")
                    .text_color(t.violet)
                    .press(&t)
                    .on_click(cx.listener(move |_, _, _, cx| {
                        owner.update(cx, |owner, cx| owner.undo(cx)).ok();
                    })),
            );
        div()
            .id("undo-pill")
            .size_full()
            .p(px(MENU_MARGIN))
            .font_family(crate::theme::SANS)
            .text_size(px(13.))
            .line_height(px(18.))
            .text_color(t.text)
            .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                if this.reveal.leaving() {
                    return;
                }
                if *hovered {
                    this.timer = None;
                } else {
                    this.time_out(Self::AFTER_HOVER, window, cx);
                }
            }))
            .child(self.reveal.apply(card, window))
    }
}

impl MenuPopup {
    /// Closes the menu without choosing anything, and tells the owner so a click on the
    /// menu's own button closes it and does not reopen it straight away. Escape closes it at
    /// once; a click elsewhere lets it fade.
    fn dismiss(&mut self, at_once: bool, window: &mut Window, cx: &mut Context<Self>) {
        let menu = self.menu;
        self.owner
            .update(cx, |owner, _| {
                owner.menu_window = None;
                owner.menu_dismissed = Some((menu, std::time::Instant::now()));
            })
            .ok();
        if at_once {
            window.remove_window();
        } else {
            self.leave(window, cx);
        }
    }

    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled || self.reveal.leaving() || index >= self.items.len() {
            return;
        }
        let menu = self.menu;
        self.owner
            .update(cx, |owner, cx| owner.menu_choice(menu, index, cx))
            .ok();
        self.leave(window, cx);
    }

    fn key(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.reveal.leaving() {
            return;
        }
        let rows = self.items.len();
        match event.keystroke.key.as_str() {
            "escape" => self.dismiss(true, window, cx),
            "down" if self.enabled && rows > 0 => {
                self.lit = Some(self.lit.map_or(0, |lit| (lit + 1) % rows));
                cx.notify();
            }
            "up" if self.enabled && rows > 0 => {
                self.lit = Some(self.lit.map_or(rows - 1, |lit| (lit + rows - 1) % rows));
                cx.notify();
            }
            "enter" | "space" => {
                if let Some(lit) = self.lit {
                    self.choose(lit, window, cx);
                }
            }
            _ => {}
        }
    }

    /// The window stopped being the key one: a click on the email, on the main window or on
    /// another app. Ignored while the click that opened it settles, then checked again.
    fn deactivated(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let left = SETTLING.saturating_sub(self.opened.elapsed());
        if left.is_zero() {
            self.dismiss(false, window, cx);
            return;
        }
        self._settling = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(left).await;
            this.update_in(cx, |this, window, cx| {
                if !window.is_window_active() {
                    this.dismiss(false, window, cx);
                }
            })
            .ok();
        }));
    }
}

impl Leave for MenuPopup {
    fn leave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reveal.leave(motion::MENU_OUT, window, cx);
    }
}

impl Render for MenuPopup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        let enabled = self.enabled;
        let rows = self.items.iter().enumerate().map(|(index, item)| {
            let lit = enabled && self.lit == Some(index);
            div()
                .id(("menu-item", index))
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(10.))
                .py(px(7.))
                .rounded(px(radius::CONTROL))
                .child(icon(
                    item.icon,
                    16.,
                    if enabled { item.tint } else { t.faint },
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .child(item.label.clone()),
                )
                .when(!item.key.is_empty(), |d| {
                    d.child(caption(item.key, t.text3))
                })
                .when(!enabled, |d| d.text_color(t.text3))
                .when(lit, |d| {
                    d.relative()
                        .bg(t.selected)
                        .child(lit_edge(&t, radius::CONTROL))
                })
                .when(enabled, |d| {
                    d.cursor_pointer()
                        .press_selected(&t)
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if *hovered && this.lit != Some(index) {
                                this.lit = Some(index);
                                cx.notify();
                            }
                        }))
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.choose(index, window, cx)),
                        )
                })
        });
        let card = popup_card(&t)
            .flex()
            .flex_col()
            .size_full()
            .p(px(CARD_PAD))
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                if !this.reveal.leaving() {
                    this.dismiss(false, window, cx)
                }
            }))
            .children(rows);
        div()
            .id("menu")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                this.key(event, window, cx)
            }))
            .size_full()
            .p(px(MENU_MARGIN))
            .font_family(crate::theme::SANS)
            .text_size(px(13.))
            .line_height(px(18.))
            .text_color(t.text)
            .child(self.reveal.apply(card, window))
    }
}

impl Nimbra {
    /// Opens a toolbar menu as a small borderless window of its own, placed under the
    /// button. A separate window floats above the email's native web view, so the menu can
    /// overlap the email without moving or hiding it.
    pub(super) fn toggle_menu(&mut self, menu: Menu, window: &mut Window, cx: &mut Context<Self>) {
        let just_dismissed = self.menu_dismissed.take().is_some_and(|(closed, at)| {
            closed == menu && at.elapsed() < Duration::from_millis(400)
        });
        self.close_menu_window(cx);
        if just_dismissed {
            return;
        }
        self.menu_for = match menu {
            Menu::Row(index) => Some(self.meant_for(&[index])),
            Menu::Labels | Menu::MoveTo | Menu::More => Some(self.meant_for(&self.targets())),
            _ => None,
        };
        self.menu_labels = match menu {
            Menu::Labels => self.labels.iter().take(12).cloned().collect(),
            Menu::Label(index) => self.labels.get(index).cloned().into_iter().collect(),
            _ => Vec::new(),
        };
        let t = self.theme();
        let tints = [t.violet, t.rose, t.amber, t.sky, t.mint];
        let items: Vec<MenuItem> = match menu {
            Menu::Labels if self.is_demo() => vec![item("tag", "No Account Connected", t.text3)],
            Menu::Labels => self
                .labels
                .iter()
                .enumerate()
                .take(12)
                .map(|(index, label)| {
                    item("tag", short(&label.name, 28), tints[index % tints.len()])
                })
                .chain(std::iter::once(item("plus", "New Label…", t.text2)))
                .collect(),
            Menu::MoveTo => {
                let mut targets = Vec::new();
                if matches!(
                    self.mailbox,
                    View::Done
                        | View::Spam
                        | View::Trash
                        | View::Label(_)
                        | View::Starred
                        | View::Important
                        | View::AllMail
                        | View::Search
                ) {
                    targets.push(MoveTarget::Inbox);
                }
                targets.extend(
                    self.labels
                        .iter()
                        .take(12)
                        .map(|l| MoveTarget::Label(l.id.clone())),
                );
                if self.mailbox != View::Spam && !self.is_demo() {
                    targets.push(MoveTarget::Spam);
                }
                let items = targets
                    .iter()
                    .map(|target| match target {
                        MoveTarget::Inbox => item("inbox", "Inbox", t.text2),
                        MoveTarget::Spam => item("ban", "Spam", t.text2),
                        MoveTarget::Label(id) => {
                            let index = self.labels.iter().position(|l| &l.id == id).unwrap_or(0);
                            let name = self
                                .labels
                                .get(index)
                                .map(|l| l.name.clone())
                                .unwrap_or_default();
                            item("folder", short(&name, 28), tints[index % tints.len()])
                        }
                    })
                    .collect::<Vec<_>>();
                self.move_targets = targets;
                if items.is_empty() {
                    let none = if self.own_servers() {
                        "No Folders"
                    } else {
                        "No Labels"
                    };
                    vec![item("folder", none, t.text3)]
                } else {
                    items
                }
            }
            Menu::From => {
                let from = self.compose_from();
                self.settings
                    .accounts
                    .iter()
                    .map(|a| {
                        if Some(a.id.as_str()) == from.as_deref() {
                            item("done", a.email.clone(), t.text)
                        } else {
                            item("mail", a.email.clone(), t.text2)
                        }
                    })
                    .collect()
            }
            Menu::Label(_) => vec![
                item("draft", "Rename…", t.text2),
                item("trash", "Delete Label…", t.rose),
            ],
            Menu::SendLater => {
                let choices = Self::send_later_choices();
                self.send_later_times = choices.iter().map(|(_, at)| *at).collect();
                choices
                    .into_iter()
                    .map(|(label, _)| item("clock", label, t.text2))
                    .collect()
            }
            Menu::More => {
                let actions = self.more_actions();
                // Keys act on the open thread: shown while that is what the menu is for.
                let keyed = self.picked.len() < 2;
                let items = actions
                    .iter()
                    .map(|action| {
                        let (name, label, key) = action.label();
                        MenuItem {
                            key: if keyed { key } else { "" },
                            ..item(name, label, t.text2)
                        }
                    })
                    .collect();
                self.more_actions = actions;
                items
            }
            Menu::Row(index) => {
                let Some(mail) = self.mails.get(index) else {
                    return;
                };
                let archivable = inbox_like(self.mailbox);
                let mut actions = vec![
                    if mail.unread {
                        RowAction::MarkRead
                    } else {
                        RowAction::MarkUnread
                    },
                    if mail.starred {
                        RowAction::Unstar
                    } else {
                        RowAction::Star
                    },
                ];
                if archivable {
                    actions.push(RowAction::Done);
                }
                if matches!(self.mailbox, View::Done | View::Trash | View::Spam) {
                    actions.push(RowAction::MoveToInbox);
                }
                if self.mailbox == View::Trash {
                    actions.push(RowAction::DeleteForever);
                } else {
                    actions.push(RowAction::Trash);
                }
                if !self.is_demo() && !matches!(self.mailbox, View::Spam | View::Trash) {
                    actions.push(RowAction::ReportSpam);
                }
                if !self.is_demo() && !self.own_servers() {
                    actions.push(RowAction::OpenInGmail);
                }
                // A key acts on the open thread: shown only when that is this row.
                let keyed = index == self.selected && self.picked.len() < 2;
                let items = actions
                    .iter()
                    .map(|action| {
                        let (name, label) = action.label();
                        MenuItem {
                            key: if keyed { action.key() } else { "" },
                            ..item(name, label, t.text2)
                        }
                    })
                    .collect();
                self.row_actions = actions;
                items
            }
        };
        let enabled = !(menu == Menu::Labels && self.is_demo())
            && !(menu == Menu::MoveTo && self.move_targets.is_empty());

        // Where the menu goes, in window coordinates: under its button.
        let viewport = window.viewport_size();
        let pane_left = self.pane_layout().1;
        // Addresses can be longer than the usual menu: From grows to fit its longest one,
        // so it keeps the same space on its right as on its left.
        let width = if menu == Menu::From {
            let longest = items
                .iter()
                .map(|item| text_width(&item.label, 13., crate::theme::SANS, cx))
                .fold(0., f32::max);
            MENU_WIDTH.max((longest + MENU_ROW_CHROME).ceil())
        } else {
            MENU_WIDTH
        };
        let height = items.len() as f32 * 32. + 14.;
        // Under the button it drops from, as last drawn: left-aligned with it, or for More
        // (at the toolbar's end) right-aligned.
        let anchored = |anchor: Anchor, fallback: (f32, f32)| {
            self.menu_anchors.get()[anchor as usize].map_or(fallback, |b| {
                let left = match anchor {
                    Anchor::More => f32::from(b.right()) - width,
                    Anchor::Labels | Anchor::MoveTo => f32::from(b.left()),
                };
                (left, f32::from(b.bottom()) + 6.)
            })
        };
        let (left, top) = match menu {
            Menu::Labels => anchored(Anchor::Labels, (pane_left + 150., crate::ui::GUTTER + 50.)),
            Menu::MoveTo => anchored(Anchor::MoveTo, (pane_left + 186., crate::ui::GUTTER + 50.)),
            Menu::More => anchored(
                Anchor::More,
                (
                    f32::from(viewport.width) - crate::ui::GUTTER - 20. - width,
                    crate::ui::GUTTER + 50.,
                ),
            ),
            // Under the composer's From: beside the title of a draft, and in the fields'
            // value column on a new message, which has no title.
            Menu::From => match &self.compose {
                Some(c) => {
                    let vw = f32::from(viewport.width);
                    let titled = c.draft_id.is_some();
                    let floating = !c.full && !self.compose_detached;
                    let (l, t) = if floating {
                        let (w, _) = c.size;
                        c.position.unwrap_or((((vw - w) / 2.).max(0.), 96.))
                    } else if c.full {
                        // The readable column, under the room left for the traffic lights.
                        (((vw - 860.) / 2.).max(0.), 36.)
                    } else {
                        (0., 0.)
                    };
                    match (titled, floating) {
                        (true, true) => (l + 20., t + 58.),
                        (true, false) => (24., 72.),
                        (false, _) => (l + 80., t + 47.),
                    }
                }
                None => (24., 72.),
            },
            // Above the composer's Send later button.
            Menu::SendLater => {
                let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
                match &self.compose {
                    Some(c) if !c.full => {
                        let (w, h) = c.size;
                        let (l, t) = c.position.unwrap_or((((vw - w) / 2.).max(0.), 96.));
                        (l + w - width - 90., t + h - height - 56.)
                    }
                    _ => (vw - width - 110., vh - height - 64.),
                }
            }
            // At the pointer, nudged back inside the window if it would hang over an edge.
            Menu::Row(_) | Menu::Label(_) => (
                self.menu_at.0.min(f32::from(viewport.width) - width - 8.),
                self.menu_at.1.min(f32::from(viewport.height) - height - 8.),
            ),
        };
        let origin = window.bounds().origin;
        let bounds = gpui::Bounds::new(
            gpui::point(
                origin.x + px(left - MENU_MARGIN),
                origin.y + px(top - MENU_MARGIN),
            ),
            gpui::size(px(width + 2. * MENU_MARGIN), px(height + 2. * MENU_MARGIN)),
        );
        let owner = cx.weak_entity();
        let dark = self.dark;
        // It drops in from its button, or rises from Send Later, which it opens above.
        let from = if menu == Menu::SendLater { 4. } else { -4. };
        let opened = cx.open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                titlebar: None,
                kind: gpui::WindowKind::PopUp,
                focus: true,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                window_background: gpui::WindowBackgroundAppearance::Transparent,
                ..Default::default()
            },
            |window, cx| {
                crate::blur::no_window_shadow(window);
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    window.focus(&focus);
                    MenuPopup {
                        owner,
                        menu,
                        items,
                        enabled,
                        dark,
                        lit: None,
                        focus,
                        reveal: Reveal::new(true, from, motion::MENU_IN),
                        was_active: false,
                        opened: std::time::Instant::now(),
                        // A click elsewhere (the email, the main window, another app) takes
                        // the key status away from the menu's window: that closes it.
                        _activation: cx.observe_window_activation(
                            window,
                            |this: &mut MenuPopup, window, cx| {
                                if window.is_window_active() {
                                    this.was_active = true;
                                } else if this.was_active && !this.reveal.leaving() {
                                    this.deactivated(window, cx);
                                }
                            },
                        ),
                        _settling: None,
                    }
                })
            },
        );
        match opened {
            Ok(handle) => self.menu_window = Some(handle),
            // No pop-up windows here: fall back to the menu drawn inside the pane.
            Err(_) => self.show_drawn_menu(menu, cx),
        }
    }

    pub(super) fn close_menu_window(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.menu_window.take() {
            let_go(handle, cx);
        }
    }

    fn menu_choice(&mut self, menu: Menu, index: usize, cx: &mut Context<Self>) {
        self.menu_window = None;
        self.menu = None;
        // Whatever a menu shows may have moved or gone while it was open (new mail, a label
        // made elsewhere): nothing here trusts a position from when it opened.
        let meant = self.menu_for.clone();
        if matches!(menu, Menu::Labels | Menu::MoveTo | Menu::More) && !self.still_meant(&meant, cx)
        {
            return;
        }
        match (menu, index) {
            (Menu::Labels, index) if index == self.menu_labels.len() => self.open_new_label(cx),
            (Menu::Labels, index) => {
                let id = self.menu_labels.get(index).map(|l| l.id.clone());
                match self.labels.iter().position(|l| Some(&l.id) == id.as_ref()) {
                    Some(now) => self.apply_label(now, cx),
                    None => self.list_changed(cx),
                }
            }
            (Menu::Label(_), choice) => {
                let Some(label) = self.menu_labels.first().cloned() else {
                    return;
                };
                let Some(now) = self.labels.iter().position(|l| l.id == label.id) else {
                    self.list_changed(cx);
                    return;
                };
                if choice == 0 {
                    self.open_rename_label(now, cx);
                } else {
                    self.confirm = Some(Ask::DeleteLabel(label.id, label.name));
                    cx.notify();
                }
            }
            (Menu::MoveTo, index) => {
                if let Some(target) = self.move_targets.get(index).cloned() {
                    self.move_selected(target, cx);
                }
            }
            (Menu::More, index) => {
                if let Some(action) = self.more_actions.get(index).copied() {
                    self.run_more(action, cx);
                }
            }
            (Menu::From, index) => {
                if let Some(id) = self.settings.accounts.get(index).map(|a| a.id.clone()) {
                    self.set_compose_account(id, cx);
                }
            }
            (Menu::SendLater, index) => {
                if let Some(at) = self.send_later_times.get(index).copied() {
                    self.schedule_send(at, cx);
                }
            }
            (Menu::Row(_), index) => {
                let Some(action) = self.row_actions.get(index).copied() else {
                    return;
                };
                let Some(meant) = meant else { return };
                // Its actions are those of the mailbox it was opened in.
                if meant.mailbox != self.mailbox {
                    self.list_changed(cx);
                    return;
                }
                if let Some((key, account)) = meant.threads.first()
                    && let Some(row) = self.row_to_act_on(key, account, cx)
                {
                    self.row_action(row, action, cx);
                }
            }
        }
    }

    fn show_drawn_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        let just_dismissed = self.menu_dismissed.take().is_some_and(|(closed, at)| {
            closed == menu && at.elapsed() < Duration::from_millis(400)
        });
        self.menu = if self.menu == Some(menu) || just_dismissed {
            None
        } else {
            Some(menu)
        };
        cx.notify();
    }

    /// Laid over a button that drops a menu: records where it is, so the menu drops from it.
    pub(super) fn menu_anchor(&self, which: Anchor) -> impl IntoElement + use<> {
        let anchors = self.menu_anchors.clone();
        gpui::canvas(
            move |bounds, _, _| {
                let mut all = anchors.get();
                all[which as usize] = Some(bounds);
                anchors.set(all);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }

    /// What the More menu offers for the open thread, in its order.
    pub(super) fn more_actions(&self) -> Vec<MoreAction> {
        let mail = self.mails.get(self.selected);
        let is_draft = self.mailbox == View::Drafts && !self.is_demo();
        let mut actions = vec![if mail.is_some_and(|m| m.unread) {
            MoreAction::MarkRead
        } else {
            MoreAction::MarkUnread
        }];
        if mail.is_some_and(|m| m.html.is_some()) && !is_draft && self.picked.len() < 2 {
            actions.push(if self.original {
                MoreAction::ReaderView
            } else {
                MoreAction::ShowOriginal
            });
        }
        if !self.is_demo() {
            if !self.own_servers() {
                actions.push(MoreAction::OpenInGmail);
            }
            actions.push(MoreAction::ReportSpam);
            actions.push(if self.sender_blocked(self.selected) {
                MoreAction::Unblock
            } else {
                MoreAction::Block
            });
        }
        actions
    }

    pub(super) fn run_more(&mut self, action: MoreAction, cx: &mut Context<Self>) {
        match action {
            MoreAction::MarkUnread => self.mark_unread(cx),
            MoreAction::MarkRead => self.toggle_unread(cx),
            MoreAction::ShowOriginal | MoreAction::ReaderView => self.toggle_original(cx),
            MoreAction::OpenInGmail => self.open_in_gmail(cx),
            MoreAction::ReportSpam => self.remove_selected(mail::Change::Spam, cx),
            MoreAction::Block | MoreAction::Unblock => self.toggle_block(cx),
        }
    }

    /// Opens the right-click menu for a row of the list.
    pub(super) fn open_row_menu(
        &mut self,
        index: usize,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu_at = (f32::from(at.x), f32::from(at.y));
        // A second right-click should always open the menu again, never just close it.
        self.menu_dismissed = None;
        self.toggle_menu(Menu::Row(index), window, cx);
    }

    /// Runs one action from a row's right-click menu on that row.
    fn row_action(&mut self, row: usize, action: RowAction, cx: &mut Context<Self>) {
        match action {
            RowAction::MarkRead => self.set_unread(row, false, cx),
            RowAction::MarkUnread => self.set_unread(row, true, cx),
            RowAction::Star | RowAction::Unstar => {
                let starred = action == RowAction::Star;
                self.mails[row].starred = starred;
                let change = if starred {
                    mail::Change::Star
                } else {
                    mail::Change::Unstar
                };
                self.sync(row, change, false, cx);
            }
            RowAction::Done
            | RowAction::MoveToInbox
            | RowAction::Trash
            | RowAction::DeleteForever
            | RowAction::ReportSpam => {
                // That row alone. Only if it is the open one does the next open, as with E;
                // any other leaves what is open as it is, and nothing else is marked read.
                let open = row == self.selected && self.picked.len() < 2;
                match action {
                    RowAction::Trash | RowAction::DeleteForever => {
                        self.trash_thread(row, cx);
                    }
                    RowAction::ReportSpam => self.remove_thread(row, mail::Change::Spam, cx),
                    _ => {
                        self.done_thread(row, cx);
                    }
                }
                if open && self.done[row] {
                    self.open_selected(cx);
                }
            }
            RowAction::OpenInGmail => {
                if let Some(id) = self.mails[row].id.as_ref()
                    && id.chars().all(|c| c.is_ascii_alphanumeric())
                {
                    cx.open_url(&format!("https://mail.google.com/mail/u/0/#all/{id}"));
                }
            }
        }
        cx.notify();
    }

    /// Shows the "Undo" pill at the bottom centre of the window for a few seconds.
    pub(super) fn offer_undo(&mut self, label: impl Into<SharedString>, cx: &mut Context<Self>) {
        let label: SharedString = label.into();
        // Wide enough for the label, the Undo button and the padding; 300 at least.
        let width = (text_width(&label, 13., crate::theme::SANS, cx) + 150.)
            .clamp(300., 460.)
            .ceil();
        let size = (width, 44.);
        self.close_undo(cx);
        let Some(frame) = self.frame else {
            return;
        };
        let origin = gpui::point(
            frame.origin.x + (frame.size.width - px(size.0)) / 2. - px(MENU_MARGIN),
            frame.origin.y + frame.size.height - px(size.1 + 24.) - px(MENU_MARGIN),
        );
        self.undo_gen += 1;
        let (owner, dark, shown_gen) = (cx.weak_entity(), self.dark, self.undo_gen);
        let opened = cx.open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::new(
                    origin,
                    gpui::size(px(size.0 + 2. * MENU_MARGIN), px(size.1 + 2. * MENU_MARGIN)),
                ))),
                titlebar: None,
                kind: gpui::WindowKind::PopUp,
                // Keyboard focus stays in the main window, so ⌘Z still reaches it.
                focus: false,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                window_background: gpui::WindowBackgroundAppearance::Transparent,
                ..Default::default()
            },
            |window, cx| {
                crate::blur::no_window_shadow(window);
                cx.new(|cx| {
                    let mut pill = UndoToast {
                        owner,
                        label,
                        dark,
                        shown_gen,
                        reveal: Reveal::new(true, TOAST_RISE, motion::TOAST_IN),
                        timer: None,
                    };
                    pill.time_out(UndoToast::SHOWN, window, cx);
                    pill
                })
            },
        );
        self.undo_window = opened.ok();
    }

    /// Shows the current message in a small window at the bottom centre, above the email
    /// (a native view that would cover anything drawn in the main window). It goes away by
    /// itself after a few seconds, longer for longer messages.
    pub(super) fn sync_notice_window(&mut self, toast: Option<Toast>, cx: &mut Context<Self>) {
        if let Some(handle) = self.notice_window.take() {
            let_go(handle, cx);
        }
        let (Some(toast), Some(frame)) = (toast, self.frame) else {
            return;
        };
        let (width, height) = (toast.width, toast.height);
        // Long enough to read it.
        let seconds = (4. + toast.text.chars().count() as f32 / 25.).min(20.);
        // Above the Undo pill when that is showing too.
        let lift = if self.undo_window.is_some() { 54. } else { 0. };
        let origin = gpui::point(
            frame.origin.x + (frame.size.width - px(width)) / 2. - px(MENU_MARGIN),
            frame.origin.y + frame.size.height - px(height + 24. + lift) - px(MENU_MARGIN),
        );
        self.notice_gen += 1;
        let (owner, dark, shown) = (cx.weak_entity(), self.dark, self.notice_gen);
        let opened = cx.open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::new(
                    origin,
                    gpui::size(px(width + 2. * MENU_MARGIN), px(height + 2. * MENU_MARGIN)),
                ))),
                titlebar: None,
                kind: gpui::WindowKind::PopUp,
                focus: false,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                window_background: gpui::WindowBackgroundAppearance::Transparent,
                ..Default::default()
            },
            |window, cx| {
                crate::blur::no_window_shadow(window);
                cx.new(|cx| {
                    // Its own clock: it goes with the window, so repeated notices never
                    // leave timers behind.
                    let weak = owner.clone();
                    let _timer = cx.spawn_in(window, async move |_, cx| {
                        cx.background_executor()
                            .timer(Duration::from_secs_f32(seconds))
                            .await;
                        weak.update(cx, |owner, cx| {
                            if owner.notice_gen == shown {
                                owner.notice = None;
                                cx.notify();
                            }
                        })
                        .ok();
                    });
                    NoticeToast {
                        owner,
                        toast,
                        dark,
                        reveal: Reveal::new(true, TOAST_RISE, motion::TOAST_IN),
                        _timer,
                    }
                })
            },
        );
        self.notice_window = opened.ok();
    }

    pub(super) fn close_undo(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.undo_window.take() {
            let_go(handle, cx);
        }
    }

    /// The Labels or More menu, dropped under its toolbar button.
    pub(super) fn menu_view(&self, t: &Theme, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let menu = self.menu?;
        let hover = t.hover;
        let item = move |id: usize, icon_name: &str, label: SharedString, tint: Hsla| {
            div()
                .id(("menu-item", id))
                .press(t)
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(10.))
                .py(px(7.))
                .rounded(px(radius::CONTROL))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(icon(icon_name, 16., tint))
                .child(div().flex_1().child(label))
        };
        let panel = div()
            .id("menu")
            .absolute()
            .top(px(50.))
            .flex()
            .flex_col()
            .min_w(px(MENU_WIDTH))
            .p(px(CARD_PAD))
            .rounded(px(radius::CARD))
            .bg(t.raised)
            .shadow(crate::tooltip::lift(t))
            .child(edge(t, radius::CARD))
            .occlude()
            // Any click outside the menu closes it.
            .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                this.menu = None;
                this.menu_dismissed = Some((menu, std::time::Instant::now()));
                cx.notify();
            }));
        Some(match menu {
            // These menus only exist as pop-up windows.
            Menu::Row(_) | Menu::SendLater | Menu::MoveTo | Menu::Label(_) | Menu::From => {
                return None;
            }
            Menu::Labels => {
                let tints = [t.violet, t.rose, t.amber, t.sky, t.mint];
                panel
                    .left(px(150.))
                    .when(self.labels.is_empty(), |d| {
                        d.child(div().px(px(10.)).py(px(7.)).text_color(t.text3).child(
                            if self.is_demo() {
                                "No Account Connected"
                            } else {
                                "No Labels"
                            },
                        ))
                    })
                    .children(
                        self.labels
                            .iter()
                            .enumerate()
                            .take(12)
                            .map(|(index, label)| {
                                item(
                                    index,
                                    "tag",
                                    short(&label.name, 28),
                                    tints[index % tints.len()],
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| this.menu_choice(Menu::Labels, index, cx),
                                ))
                            }),
                    )
            }
            Menu::More => panel.right(px(20.)).children(
                self.more_actions()
                    .into_iter()
                    .enumerate()
                    .map(|(index, action)| {
                        let (name, label, _) = action.label();
                        item(index, name, label.into(), t.text2).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.menu = None;
                                this.run_more(action, cx)
                            },
                        ))
                    }),
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_menu_shows_only_keys_that_exist() {
        let keys: Vec<&str> = crate::keyboard::SHORTCUTS
            .iter()
            .flat_map(|(_, rows)| rows.iter())
            .flat_map(|(keys, _)| keys.iter().copied())
            .collect();
        for action in [
            RowAction::MarkRead,
            RowAction::MarkUnread,
            RowAction::Star,
            RowAction::Unstar,
            RowAction::Done,
            RowAction::MoveToInbox,
            RowAction::Trash,
            RowAction::DeleteForever,
            RowAction::ReportSpam,
            RowAction::OpenInGmail,
        ] {
            let key = match action.key() {
                "" => continue,
                "⌫" => "Delete",
                key => key,
            };
            assert!(keys.contains(&key), "{key}");
        }
    }

    /// GPUI draws a shadow out to 3 times its blur, and cuts it off at the window's edge.
    #[test]
    fn every_layer_of_a_pop_ups_lift_fades_inside_its_window() {
        for (down, blur, spread, _) in crate::tooltip::LIFT {
            let below = down + spread + 3. * blur;
            assert!(below <= MENU_MARGIN && spread + 3. * blur <= MENU_MARGIN);
        }
    }
}
