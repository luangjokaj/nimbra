//! Menus and the other small pop-ups (the Undo pill, notices). Each is a window of its own,
//! so it can lie over the email, which is a native view above everything GPUI draws.

use super::*;

/// What the reading pane's ⋯ menu can do (what the toolbar leaves out).
#[derive(Clone, Copy, PartialEq)]
pub(super) enum MoreAction {
    ToggleOriginal,
    MarkUnread,
    OpenInGmail,
    Spam,
    Block,
}

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
            RowAction::MarkRead => ("mail", "Mark as read"),
            RowAction::MarkUnread => ("mail", "Mark as unread"),
            RowAction::Star => ("star", "Star"),
            RowAction::Unstar => ("star", "Remove star"),
            RowAction::Done => ("done", "Mark done"),
            RowAction::MoveToInbox => ("inbox", "Move to inbox"),
            RowAction::Trash => ("trash", "Move to trash"),
            RowAction::DeleteForever => ("trash", "Delete permanently"),
            RowAction::ReportSpam => ("bell", "Report spam"),
            RowAction::OpenInGmail => ("link", "Open in Gmail"),
        }
    }
}

/// A small menu dropped from the reading toolbar.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
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
    /// Delete a label of every account's (under All inboxes), by index in `every_labels`.
    EveryLabel(usize),
    /// The right-click menu of a row in the list (by index into the loaded threads).
    Row(usize),
    /// The right-click menu of a mailbox in the sidebar: hide it, or reset its section.
    Mailbox(View),
    /// The From account's signatures, from the handle above the composer's signature.
    Signature,
}

/// What an item of the signature menu does.
#[derive(Clone, PartialEq)]
pub(super) enum SignatureChoice {
    Use(String),
    None,
    /// Settings › Composing, on the account's signatures.
    Edit,
}

pub(super) const MENU_WIDTH: f32 = 220.;

/// Closes one of the small pop-up windows (a menu, the Undo or notice pill), once it has
/// faded out.
pub(super) fn remove_popup<V: 'static>(handle: gpui::WindowHandle<V>, cx: &mut App) {
    // It fades out first (`popup::leave`), then goes. A window busy with its own button's
    // click (Undo, Restart) cannot be reached now: it fades as soon as that is over.
    let faded = handle
        .update(cx, |_, window, _| crate::popup::leave(window))
        .is_ok();
    cx.spawn(async move |cx| {
        if !faded {
            handle
                .update(cx, |_, window, _| crate::popup::leave(window))
                .ok();
        }
        cx.background_executor()
            .timer(std::time::Duration::from_secs_f64(crate::popup::FADE))
            .await;
        handle
            .update(cx, |_, window, _| window.remove_window())
            .ok();
    })
    .detach();
}

/// What a menu row has beside its label: the card's border (1) and padding (6) and the row's
/// padding (10) on each side, and the icon (16) with its gap (10).
pub(super) const MENU_ROW_CHROME: f32 = 2. * (1. + 6. + 10.) + 16. + 10.;

/// Transparent space around the menu inside its window, so its shadow is not cut off.
pub(super) const MENU_MARGIN: f32 = 28.;

/// The shadow of a pop-up window's card (menus, the Undo and notice pills). It has to fade
/// out inside the window's margin: GPUI draws a shadow out to 3 times its blur, and what
/// reaches the window's edge is cut off there, which shows as a box around the card.
pub(super) fn popup_shadow(t: &Theme) -> gpui::BoxShadow {
    shadow(t.shadow, POPUP_SHADOW.0, POPUP_SHADOW.1, POPUP_SHADOW.2)
}

/// Offset down, blur, spread.
pub(super) const POPUP_SHADOW: (f32, f32, f32) = (4., 8., -2.);

/// One account in a list of accounts to pick from (the composer's From, Settings ›
/// Composing's account field).
pub(super) struct AccountChoice {
    pub(super) id: String,
    pub(super) email: String,
    pub(super) photo: Option<Arc<gpui::Image>>,
    /// The one picked now, which has a check.
    pub(super) on: bool,
}

/// How far the photo sits in from the composer's From button's left edge (its padding).
pub(super) const FROM_PHOTO_INSET: f32 = 4.;

/// The height of a row in a list of accounts.
pub(super) const ACCOUNT_ROW: f32 = 40.;

/// What a list of accounts has around its rows, top and bottom: the card's border (1) and
/// padding (4) on each side.
pub(super) const ACCOUNT_LIST_CHROME: f32 = 2. * (1. + 4.);

/// What an account row has beside its address: the card's border (1) and padding (4) and the
/// row's padding (8) on each side, the photo (24) and the check (14) with their gaps (10).
pub(super) const ACCOUNT_ROW_CHROME: f32 = 2. * (1. + 4. + 8.) + 24. + 10. + 14. + 10.;

/// How far an account row's photo sits in from the list's left edge: the card's border (1)
/// and padding (4) and the row's padding (8).
pub(super) const ACCOUNT_ROW_INSET: f32 = 1. + 4. + 8.;

/// Where each menu's button is drawn, by window (a conversation's own window has the same
/// toolbar): left, bottom and right, in the window's points.
pub(super) type Anchors =
    std::rc::Rc<std::cell::RefCell<HashMap<(gpui::WindowId, Menu), (f32, f32, f32)>>>;

/// An invisible layer over a button that records where it is drawn, for `menu` to open
/// under it (`menu_anchors`).
pub(super) fn anchor_for(anchors: &Anchors, menu: Menu) -> impl IntoElement {
    let anchors = anchors.clone();
    gpui::canvas(
        move |bounds, window, _| {
            anchors.borrow_mut().insert(
                (window.window_handle().window_id(), menu),
                (
                    from_screen(bounds.left()),
                    from_screen(bounds.bottom()),
                    from_screen(bounds.right()),
                ),
            );
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

/// An account's photo, or its initial on its tone when it has none.
pub(super) fn account_face(email: &str, photo: Option<&Arc<gpui::Image>>, size: f32) -> AnyElement {
    let initials = email
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    match photo {
        Some(photo) => me_avatar(Some(photo), &initials, size),
        None => avatar(crate::data::tone_for(email), &initials, size).into_any_element(),
    }
}

/// A row in a list of accounts: its photo, its address, and a check on the one picked. The
/// caller adds what a click does.
pub(super) fn account_row(
    t: &Theme,
    id: impl Into<gpui::ElementId>,
    account: &AccountChoice,
) -> Stateful<Div> {
    let hover = t.hover;
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(dp(10.))
        .h(dp(ACCOUNT_ROW))
        .px(dp(8.))
        .rounded(dp(7.))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .press(t)
        .child(account_face(&account.email, account.photo.as_ref(), 24.))
        .child(
            div()
                .flex_1()
                .min_w(dp(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(account.email.clone()),
        )
        .when(account.on, |d| d.child(icon("done", 14., t.violet)))
}

/// The card a list of accounts sits on, inside its pop-up window's margin.
pub(super) fn account_list(t: &Theme, rows: impl IntoIterator<Item = Stateful<Div>>) -> Div {
    div()
        .size_full()
        .p(dp(MENU_MARGIN))
        .font_family(crate::theme::SANS)
        .text_size(dp(13.))
        .line_height(dp(18.))
        .text_color(t.text)
        .child(
            div()
                .size_full()
                .flex()
                .flex_col()
                .p(dp(4.))
                .rounded(dp(10.))
                .bg(t.raised)
                .border_1()
                .border_color(t.border_strong)
                .shadow(vec![popup_shadow(t)])
                .children(rows),
        )
}

/// A toolbar menu, shown in a small window of its own (see `Nimbra::toggle_menu`).
pub(super) struct MenuPopup {
    pub(super) owner: gpui::WeakEntity<Nimbra>,
    pub(super) menu: Menu,
    pub(super) items: Vec<(&'static str, SharedString, Hsla)>,
    /// The composer's From: the accounts, drawn with their photos in place of `items`.
    pub(super) accounts: Vec<AccountChoice>,
    /// False for a menu that only carries a message (no labels yet).
    pub(super) enabled: bool,
    pub(super) dark: bool,
    pub(super) was_active: bool,
    pub(super) opened: std::time::Instant,
    pub(super) _activation: gpui::Subscription,
}

/// Where an update's progress is shown: over the main window's `frame`, in its look, and
/// as big as the notice that takes its place when the update is ready.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct UpdatePlace {
    pub(super) frame: gpui::Bounds<gpui::Pixels>,
    pub(super) dark: bool,
    pub(super) size: (f32, f32),
}

/// A new version on its way in: what is happening and a bar, at the bottom of the window
/// until it is ready (see `Nimbra::sync_update_window`).
pub(super) struct UpdatePill {
    pub(super) owner: gpui::WeakEntity<Nimbra>,
    pub(super) progress: crate::update::Progress,
    pub(super) dark: bool,
    /// What it said before it moved on to another stage, fading out over it (`show`).
    pub(super) previous: Option<crate::update::Progress>,
    /// How many times it moved on, so each fade is an animation of its own.
    pub(super) changes: u64,
    /// When it opened: the checking bar moves by this clock, so it carries on unbroken
    /// while it fades out (an animation of its own would start over in the fading layer).
    pub(super) opened: std::time::Instant,
}

/// How long the card's content takes to fade from one stage to the next.
const UPDATE_FADE: Duration = Duration::from_millis(240);

impl UpdatePill {
    pub(super) fn new(
        owner: gpui::WeakEntity<Nimbra>,
        progress: crate::update::Progress,
        dark: bool,
    ) -> Self {
        Self {
            owner,
            progress,
            dark,
            previous: None,
            changes: 0,
            opened: std::time::Instant::now(),
        }
    }

    /// Shows `progress`. Moving on to another stage (checking to up to date, downloading
    /// to ready), what it said fades out over the new one; a download only counts up.
    pub(super) fn show(&mut self, progress: crate::update::Progress, cx: &mut Context<Self>) {
        let moved_on =
            std::mem::discriminant(&progress.stage) != std::mem::discriminant(&self.progress.stage);
        if moved_on {
            self.previous = Some(std::mem::replace(&mut self.progress, progress));
            self.changes += 1;
            let change = self.changes;
            // Gone once faded, so it never sits over the new one's Restart button.
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(UPDATE_FADE).await;
                this.update(cx, |this, cx| {
                    if this.changes == change {
                        this.previous = None;
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        } else {
            self.progress = progress;
        }
        cx.notify();
    }
}

impl Render for UpdatePill {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Every window draws at the interface zoom.
        crate::ui::apply_zoom(window);
        let progress = self.progress.clone();
        let now = self.card(&progress, window, cx);
        let Some(previous) = self.previous.clone() else {
            return now;
        };
        // The new stage underneath, whole; the old one over it, fading out.
        let before = self.card(&previous, window, cx);
        div()
            .size_full()
            .relative()
            .child(div().absolute().inset_0().child(now))
            .child(div().absolute().inset_0().child(before).with_animation(
                gpui::ElementId::NamedInteger("update-fade".into(), self.changes),
                gpui::Animation::new(UPDATE_FADE).with_easing(gpui::ease_in_out),
                |layer, delta| layer.opacity(1. - delta),
            ))
            .into_any_element()
    }
}

impl UpdatePill {
    /// The card as it is at `progress`.
    fn card(
        &self,
        progress: &crate::update::Progress,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use crate::update::Stage;
        let t = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        let version = &progress.version;
        let frame = div()
            .size_full()
            .p(dp(MENU_MARGIN))
            .font_family(crate::theme::SANS)
            .text_size(dp(13.))
            .line_height(dp(18.))
            .text_color(t.text);
        let card = div()
            .size_full()
            .flex()
            .rounded(dp(12.))
            .bg(t.raised)
            .border_1()
            .border_color(t.border_strong)
            .shadow(vec![popup_shadow(&t)]);
        // Asked for, and there is nothing newer: the same card says so, with the check
        // Settings shows beside Nimbra as the default mail app, and goes by itself.
        if progress.stage == Stage::Current {
            return frame
                .child(
                    card.items_center()
                        .gap(dp(10.))
                        .pl(dp(toast::LEFT))
                        .pr(dp(toast::LEFT))
                        .child(
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .size(dp(18.))
                                .rounded_full()
                                .bg(t.mint)
                                .child(icon("done", 11., t.surface)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(dp(0.))
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_ellipsis()
                                .text_size(dp(toast::FONT))
                                .line_height(dp(toast::LINE))
                                .child(system_events::UP_TO_DATE),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_color(t.text3)
                                .child(format!("Nimbra {version}")),
                        ),
                )
                .into_any_element();
        }
        // Ready: the same card says so, laid out as a notice with its Restart button.
        if progress.stage == Stage::Ready {
            let owner = self.owner.clone();
            return frame
                .child(
                    card.items_center()
                        .justify_between()
                        .gap(dp(toast::GAP))
                        .pl(dp(toast::LEFT))
                        .pr(dp(toast::RIGHT))
                        .child(
                            div()
                                .flex_1()
                                .min_w(dp(0.))
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_ellipsis()
                                .text_size(dp(toast::FONT))
                                .line_height(dp(toast::LINE))
                                .child(system_events::update_notice(version)),
                        )
                        .child(
                            button(&t, ButtonKind::Primary, "Restart", None)
                                .id("restart-to-update")
                                .justify_center()
                                .flex_none()
                                .w(dp(toast::DISMISS.0))
                                .h(dp(toast::DISMISS.1))
                                .press_soft()
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    owner
                                        .update(cx, |owner, cx| owner.restart_to_update(cx))
                                        .ok();
                                })),
                        ),
                )
                .into_any_element();
        }
        let (label, amount) = match progress.stage {
            Stage::Downloading { done, total } => (
                format!("Updating to Nimbra {version}…"),
                match progress.fraction() {
                    Some(f) if total.is_some() => format!("{}%", (f * 100.).floor() as u32),
                    _ => format!("{:.1} MB", done as f64 / (1024. * 1024.)),
                },
            ),
            Stage::Checking => (system_events::CHECKING.to_string(), String::new()),
            Stage::Verifying | Stage::Ready | Stage::Current => {
                (format!("Checking Nimbra {version}…"), String::new())
            }
        };
        let fraction = progress.fraction().unwrap_or(0.);
        // Checking has no amount yet: a short piece of the bar goes back and forth.
        let bar = if progress.stage == Stage::Checking {
            const ROUND: f32 = 1.1;
            let delta = (self.opened.elapsed().as_secs_f32() % ROUND) / ROUND;
            // There and back: 0 → 1 → 0 over one round, easing at each end.
            let at = gpui::ease_in_out(1. - (2. * delta - 1.).abs());
            window.request_animation_frame();
            div()
                .absolute()
                .top_0()
                .left(gpui::relative(0.7 * at))
                .h_full()
                .w(gpui::relative(0.3))
                .rounded_full()
                .bg(crate::ui::primary_fill(0.))
                .into_any_element()
        } else {
            div()
                .h_full()
                .w(gpui::relative(fraction))
                .rounded_full()
                .bg(crate::ui::primary_fill(0.))
                .into_any_element()
        };
        frame
            .child(
                card.flex_col()
                    .justify_center()
                    .gap(dp(10.))
                    .px(dp(toast::LEFT))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(dp(12.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(dp(0.))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(label),
                            )
                            .child(div().flex_none().text_color(t.text3).child(amount)),
                    )
                    .child(
                        div()
                            .relative()
                            .overflow_hidden()
                            .h(dp(4.))
                            .w_full()
                            .rounded_full()
                            .bg(t.hover)
                            .child(bar),
                    ),
            )
            .into_any_element()
    }
}

/// A short message at the bottom of the window (see `Nimbra::sync_notice_window`).
pub(super) struct NoticeToast {
    pub(super) owner: gpui::WeakEntity<Nimbra>,
    pub(super) toast: Toast,
    pub(super) dark: bool,
}

impl Render for NoticeToast {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Every window draws at the interface zoom.
        crate::ui::apply_zoom(window);
        let t = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        let owner = self.owner.clone();
        div()
            .size_full()
            .p(dp(MENU_MARGIN))
            .font_family(crate::theme::SANS)
            .text_size(dp(13.))
            .line_height(dp(18.))
            .text_color(t.text)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(dp(toast::GAP))
                    .size_full()
                    .pl(dp(toast::LEFT))
                    .pr(dp(toast::RIGHT))
                    .rounded(dp(12.))
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![popup_shadow(&t)])
                    // As wide as it was measured, so that it wraps as it was measured.
                    .child(
                        div()
                            .flex_none()
                            .w(dp(self.toast.text_width))
                            .text_size(dp(toast::FONT))
                            .line_height(dp(toast::LINE))
                            .child(self.toast.text.clone()),
                    )
                    .child(
                        button(&t, ButtonKind::Secondary, "Dismiss", None)
                            .id("dismiss-notice")
                            .justify_center()
                            .w(dp(toast::DISMISS.0))
                            .h(dp(toast::DISMISS.1))
                            .press_soft()
                            .on_click(cx.listener(move |_, _, _, cx| {
                                owner
                                    .update(cx, |owner, cx| {
                                        owner.notice = None;
                                        cx.notify();
                                    })
                                    .ok();
                            })),
                    ),
            )
    }
}

/// The "Undo" pill shown after a thread is removed (see `Nimbra::offer_undo`).
pub(super) struct UndoToast {
    pub(super) owner: gpui::WeakEntity<Nimbra>,
    pub(super) label: SharedString,
    pub(super) dark: bool,
    /// How long it can still be undone, shown as a bar filling up (`offer_undo_for`).
    pub(super) countdown: Option<Duration>,
    /// Undo still stops it (a message is on its way once its delay is over).
    pub(super) can_undo: bool,
    /// What happened, in place of `label`, with the bar full (`show_sent`).
    pub(super) sent: Option<SharedString>,
}

impl Render for UndoToast {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Every window draws at the interface zoom.
        crate::ui::apply_zoom(window);
        let t = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        let owner = self.owner.clone();
        div()
            .size_full()
            .p(dp(MENU_MARGIN))
            .font_family(crate::theme::SANS)
            .text_size(dp(13.))
            .line_height(dp(18.))
            .text_color(t.text)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap(dp(8.))
                    .size_full()
                    .rounded(dp(12.))
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![popup_shadow(&t)])
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(dp(12.))
                            .pl(dp(16.))
                            .pr(dp(6.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(dp(0.))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .child(self.sent.clone().unwrap_or_else(|| self.label.clone())),
                            )
                            // Kept in place but hidden once Undo can't stop it, so the words don't move.
                            .child({
                                let can_undo = self.can_undo;
                                button(&t, ButtonKind::Ghost, "Undo", Some("⌘Z"))
                                    .id("undo")
                                    .press_soft()
                                    .when(!can_undo, |b| b.invisible())
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        if can_undo {
                                            owner.update(cx, |owner, cx| owner.undo(cx)).ok();
                                        }
                                    }))
                            }),
                    )
                    // The update's bar: a track, and the time gone by filling it.
                    .children(self.countdown.map(|countdown| {
                        let fill = div()
                            .h_full()
                            .rounded_full()
                            .bg(crate::ui::primary_fill(0.));
                        div()
                            .mx(dp(16.))
                            .h(dp(4.))
                            .rounded_full()
                            .bg(t.hover)
                            .child(if self.sent.is_some() {
                                fill.w_full().into_any_element()
                            } else {
                                fill.with_animation(
                                    "undo-countdown",
                                    Animation::new(countdown),
                                    |fill, done| fill.w(gpui::relative(done)),
                                )
                                .into_any_element()
                            })
                    })),
            )
    }
}

impl MenuPopup {
    /// Closes the menu without choosing anything, and tells the owner so a click on the
    /// menu's own button closes it and does not reopen it straight away.
    pub(super) fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let menu = self.menu;
        self.owner
            .update(cx, |owner, _| {
                owner.menu_window = None;
                owner.menu_dismissed = Some((menu, std::time::Instant::now()));
            })
            .ok();
        window.remove_window();
    }
}

impl Render for MenuPopup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Every window draws at the interface zoom.
        crate::ui::apply_zoom(window);
        let t = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        if !self.accounts.is_empty() {
            let rows = self.accounts.iter().enumerate().map(|(index, account)| {
                account_row(&t, ("menu-account", index), account).on_click(cx.listener(
                    move |this, _, window, cx| {
                        let menu = this.menu;
                        window.remove_window();
                        this.owner
                            .update(cx, |owner, cx| owner.menu_choice(menu, index, cx))
                            .ok();
                    },
                ))
            });
            return account_list(&t, rows).into_any_element();
        }
        let hover = t.hover;
        let enabled = self.enabled;
        div()
            .size_full()
            .p(dp(MENU_MARGIN))
            .font_family(crate::theme::SANS)
            .text_size(dp(13.))
            .line_height(dp(18.))
            .text_color(t.text)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .p(dp(6.))
                    .rounded(dp(12.))
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![popup_shadow(&t)])
                    .children(
                        self.items
                            .iter()
                            .enumerate()
                            .map(|(index, (name, label, tint))| {
                                div()
                                    .id(("menu-item", index))
                                    .flex()
                                    .items_center()
                                    .gap(dp(10.))
                                    .px(dp(10.))
                                    .py(dp(7.))
                                    .rounded(dp(8.))
                                    .child(icon(name, 16., *tint))
                                    .child(div().flex_1().child(label.clone()))
                                    .when(!enabled, |d| d.text_color(t.text3))
                                    .when(enabled, |d| {
                                        d.cursor_pointer()
                                            .hover(move |s| s.bg(hover))
                                            .press(&t)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                let menu = this.menu;
                                                window.remove_window();
                                                this.owner
                                                    .update(cx, |owner, cx| {
                                                        owner.menu_choice(menu, index, cx)
                                                    })
                                                    .ok();
                                            }))
                                    })
                            }),
                    ),
            )
            .into_any_element()
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
        // The labels of the threads' own account (in every account's mailboxes, maybe not
        // the one shown), or none when they are from several.
        let targets_account = self.targets_account();
        let account_labels: Vec<mail::Label> = targets_account
            .as_deref()
            .map(|account| self.labels_of(account).iter().take(12).cloned().collect())
            .unwrap_or_default();
        self.menu_labels = match menu {
            Menu::Labels => account_labels.clone(),
            Menu::Label(index) => self.labels.get(index).cloned().into_iter().collect(),
            _ => Vec::new(),
        };
        let t = self.theme();
        let tints = [t.violet, t.rose, t.amber, t.sky, t.mint];
        let items: Vec<(&'static str, SharedString, Hsla)> = match menu {
            Menu::Labels if self.is_demo() => {
                vec![("tag", "Labels need a connected account.".into(), t.text3)]
            }
            Menu::Labels if targets_account.is_none() => {
                vec![(
                    "tag",
                    "Pick conversations from one account.".into(),
                    t.text3,
                )]
            }
            Menu::Labels => account_labels
                .iter()
                .enumerate()
                .map(|(index, label)| ("tag", short(&label.name, 28), tints[index % tints.len()]))
                .chain(std::iter::once(("tag", "New label…".into(), t.text2)))
                .collect(),
            Menu::MoveTo => {
                let mut targets = Vec::new();
                if matches!(
                    self.mailbox.base(),
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
                    account_labels
                        .iter()
                        .map(|l| MoveTarget::Label(l.id.clone())),
                );
                if self.mailbox.base() != View::Spam && !self.is_demo() {
                    targets.push(MoveTarget::Spam);
                }
                let items = targets
                    .iter()
                    .map(|target| match target {
                        MoveTarget::Inbox => ("inbox", "Inbox".into(), t.text2),
                        MoveTarget::Spam => ("ban", "Spam".into(), t.text2),
                        MoveTarget::Label(id) => {
                            let index =
                                account_labels.iter().position(|l| &l.id == id).unwrap_or(0);
                            let name = account_labels
                                .get(index)
                                .map(|l| l.name.clone())
                                .unwrap_or_default();
                            ("folder", short(&name, 28), tints[index % tints.len()])
                        }
                    })
                    .collect::<Vec<_>>();
                self.move_targets = targets;
                if items.is_empty() {
                    vec![("folder", "Nowhere to move this to yet.".into(), t.text3)]
                } else {
                    items
                }
            }
            // Drawn from `accounts` below, with their photos.
            Menu::From => Vec::new(),
            Menu::Signature => {
                let from = self.compose_from().unwrap_or_default();
                let current = self
                    .compose
                    .as_ref()
                    .and_then(|c| c.signature.as_ref().map(|(id, _)| id.clone()));
                let mut choices: Vec<SignatureChoice> = self
                    .settings
                    .signatures_of(&from)
                    .map(|s| SignatureChoice::Use(s.id.clone()))
                    .collect();
                choices.push(SignatureChoice::None);
                choices.push(SignatureChoice::Edit);
                let items = choices
                    .iter()
                    .map(|choice| match choice {
                        SignatureChoice::Use(id) => {
                            let name = self
                                .settings
                                .signature(id)
                                .map(|s| s.name.clone())
                                .unwrap_or_default();
                            let name = short(signature_name(&name), 40);
                            if current.as_deref() == Some(id.as_str()) {
                                ("done", name, t.text)
                            } else {
                                ("draft", name, t.text2)
                            }
                        }
                        SignatureChoice::None if current.is_none() => {
                            ("done", "No signature".into(), t.text)
                        }
                        SignatureChoice::None => ("x", "No signature".into(), t.text2),
                        SignatureChoice::Edit => ("list", "Edit signatures…".into(), t.text2),
                    })
                    .collect();
                self.signature_choices = choices;
                items
            }
            Menu::Label(_) => vec![
                ("tag", "Rename…".into(), t.text2),
                ("trash", "Delete label…".into(), t.rose),
            ],
            Menu::EveryLabel(_) => vec![("trash", "Delete label…".into(), t.rose)],
            Menu::SendLater => {
                let choices = Self::send_later_choices();
                self.send_later_times = choices.iter().map(|(_, at)| *at).collect();
                choices
                    .into_iter()
                    .map(|(label, _)| ("clock", label.into(), t.text2))
                    .collect()
            }
            Menu::Mailbox(view) => {
                let section = if matches!(
                    view,
                    View::Inbox | View::Primary | View::Updates | View::Promotions | View::Social
                ) && !self.own_servers()
                {
                    "split"
                } else {
                    "mailbox"
                };
                let mut items = Vec::new();
                if view != View::Inbox {
                    if self.sidebar_now().is_hidden(view) {
                        items.push(("eye", "Show in sidebar".into(), t.text2));
                    } else {
                        items.push(("eye", "Hide from sidebar".into(), t.text2));
                    }
                }
                items.push((
                    "reload",
                    format!("Reset {} order", section_name(section)).into(),
                    t.text2,
                ));
                items
            }
            Menu::More => {
                let menu = self.more_menu();
                self.more_actions = menu.iter().map(|(action, _, _)| *action).collect();
                menu.into_iter()
                    .map(|(_, icon, label)| (icon, label, t.text2))
                    .collect()
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
                if matches!(self.mailbox.base(), View::Done | View::Trash | View::Spam) {
                    actions.push(RowAction::MoveToInbox);
                }
                if self.mailbox.base() == View::Trash {
                    actions.push(RowAction::DeleteForever);
                } else {
                    actions.push(RowAction::Trash);
                }
                if !self.is_demo() && !matches!(self.mailbox.base(), View::Spam | View::Trash) {
                    actions.push(RowAction::ReportSpam);
                }
                if !self.is_demo() && self.gmail_at(index) {
                    actions.push(RowAction::OpenInGmail);
                }
                let items = actions
                    .iter()
                    .map(|action| {
                        let (name, label) = action.label();
                        (name, label.into(), t.text2)
                    })
                    .collect();
                self.row_actions = actions;
                items
            }
        };
        // The composer's From: every account (or the sample's one), as in Settings › Composing.
        let accounts: Vec<AccountChoice> = if menu == Menu::From {
            let from = self.compose_from();
            self.signature_accounts()
                .into_iter()
                .map(|(id, email)| AccountChoice {
                    photo: self.photos.get(&id).cloned(),
                    on: Some(id.as_str()) == from.as_deref(),
                    id,
                    email,
                })
                .collect()
        } else {
            Vec::new()
        };
        let enabled = !(menu == Menu::Labels && self.is_demo())
            && !(menu == Menu::MoveTo && self.move_targets.is_empty());

        // Where the menu goes, in window coordinates: under its button.
        let viewport = window.viewport_size();
        let pane_left = if self.sidebar_open { self.side_w() } else { 0. }
            + if self.list_open { self.list_width } else { 0. };
        // Addresses can be longer than the usual menu: From grows to fit its longest one,
        // so it keeps the same space on its right as on its left.
        let width = if menu == Menu::From {
            let longest = accounts
                .iter()
                .map(|a| text_width(&a.email, 13., crate::theme::SANS, cx))
                .fold(0., f32::max);
            MENU_WIDTH.max((longest + ACCOUNT_ROW_CHROME).ceil())
        } else {
            // Any other grows to fit its longest item too (Send later's times, a long
            // signature's name), rather than cutting it off.
            let longest = items
                .iter()
                .map(|(_, label, _)| text_width(label, 13., crate::theme::SANS, cx))
                .fold(0., f32::max);
            MENU_WIDTH.max((longest + MENU_ROW_CHROME).ceil())
        };
        let height = if menu == Menu::From {
            accounts.len() as f32 * ACCOUNT_ROW + ACCOUNT_LIST_CHROME
        } else {
            items.len() as f32 * 32. + 14.
        };
        let anchored = self
            .menu_anchors
            .borrow()
            .get(&(window.window_handle().window_id(), menu))
            .copied();
        let (left, top) = match menu {
            // Under its button, wherever the toolbar has it: Label and Move to start at the
            // button's left edge, ⋯ (at the far right) ends at its right one.
            Menu::Labels | Menu::MoveTo => match anchored {
                Some((left, bottom, _)) => (left, bottom + 4.),
                None => (pane_left + 150., 50.),
            },
            Menu::More => match anchored {
                Some((_, bottom, right)) => (right - width, bottom + 4.),
                None => (from_screen(viewport.width) - 20. - width, 50.),
            },
            // Under the composer's From button, its photos over the button's.
            Menu::From => {
                let (left, bottom) = self.from_anchor.get();
                (left - ACCOUNT_ROW_INSET + FROM_PHOTO_INSET, bottom + 4.)
            }
            // Above the composer's Send later button.
            Menu::SendLater => {
                let (vw, vh) = (from_screen(viewport.width), from_screen(viewport.height));
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
            Menu::Row(_)
            | Menu::Label(_)
            | Menu::EveryLabel(_)
            | Menu::Mailbox(_)
            | Menu::Signature => (
                self.menu_at.0.min(from_screen(viewport.width) - width - 8.),
                self.menu_at
                    .1
                    .min(from_screen(viewport.height) - height - 8.),
            ),
        };
        // A menu under its button stays inside the window, however narrow.
        let left = if matches!(menu, Menu::Labels | Menu::MoveTo | Menu::More) {
            left.min(from_screen(viewport.width) - width - 8.).max(8.)
        } else {
            left
        };
        let origin = window.bounds().origin;
        let bounds = gpui::Bounds::new(
            gpui::point(
                origin.x + to_screen(left - MENU_MARGIN),
                origin.y + to_screen(top - MENU_MARGIN),
            ),
            gpui::size(
                to_screen(width + 2. * MENU_MARGIN),
                to_screen(height + 2. * MENU_MARGIN),
            ),
        );
        let owner = cx.weak_entity();
        let dark = self.dark;
        let display_id = crate::popup::display_of(window, cx);
        let opened = cx.open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                titlebar: None,
                kind: gpui::WindowKind::PopUp,
                display_id,
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
                    // Close on any click outside the menu. Polled, because clicks on the
                    // email's web view or on another app never reach this app's handlers.
                    cx.spawn_in(window, async move |this, cx| {
                        // The click that opened the menu may still be held down (a
                        // right-click opens it on the way down). Only a press that starts
                        // after every button has been let go counts as an outside click.
                        let mut armed = false;
                        loop {
                            cx.background_executor()
                                .timer(Duration::from_millis(30))
                                .await;
                            if !crate::blur::mouse_down() {
                                armed = true;
                            }
                            let open = this.update_in(cx, |this: &mut MenuPopup, window, cx| {
                                if armed
                                    && crate::blur::mouse_pressed_outside(
                                        window,
                                        MENU_MARGIN as f64,
                                    )
                                {
                                    this.dismiss(window, cx);
                                    false
                                } else {
                                    true
                                }
                            });
                            if !matches!(open, Ok(true)) {
                                break;
                            }
                        }
                    })
                    .detach();
                    MenuPopup {
                        owner,
                        menu,
                        items,
                        accounts,
                        enabled,
                        dark,
                        was_active: false,
                        opened: std::time::Instant::now(),
                        // Losing focus (switching apps with the keyboard, say) closes it too.
                        _activation: cx.observe_window_activation(
                            window,
                            |this: &mut MenuPopup, window, cx| {
                                if window.is_window_active() {
                                    this.was_active = true;
                                } else if this.was_active
                                    && this.opened.elapsed() > Duration::from_millis(500)
                                {
                                    // Focus moving away right as the menu opens is part of
                                    // the click that opened it, not a reason to close.
                                    this.dismiss(window, cx);
                                }
                            },
                        ),
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
            remove_popup(handle, cx);
        }
    }

    /// The ⋯ menu's items, in order: what the toolbar leaves out.
    pub(super) fn more_menu(&self) -> Vec<(MoreAction, &'static str, SharedString)> {
        let demo = self.is_demo();
        let html = self.mails.get(self.selected).is_some_and(|m| m.has_html());
        let draft = self.mailbox.base() == View::Drafts && !demo;
        let mut items = Vec::new();
        // First, where there is an original to show (not a draft, not plain text): the
        // switch between the email's own design and the reader.
        if html && !draft {
            let label = if self.original {
                "Switch to reader view"
            } else {
                "Switch to original"
            };
            items.push((MoreAction::ToggleOriginal, "eye", label.into()));
        }
        items.push((MoreAction::MarkUnread, "inbox", "Mark as unread".into()));
        if !demo {
            if self.gmail_at(self.selected) {
                items.push((MoreAction::OpenInGmail, "link", "Open in Gmail".into()));
            }
            items.push((MoreAction::Spam, "bell", "Report spam".into()));
            let block = if self.sender_blocked(self.selected) {
                "Unblock sender"
            } else {
                "Block sender"
            };
            items.push((MoreAction::Block, "ban", block.into()));
        }
        items
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
            // From several accounts, the menu only said to pick from one.
            (Menu::Labels, _) if self.targets_account().is_none() => {}
            (Menu::Labels, index) if index == self.menu_labels.len() => self.open_new_label(cx),
            (Menu::Labels, index) => {
                let id = self.menu_labels.get(index).map(|l| l.id.clone());
                let account = self.targets_account().unwrap_or_default();
                match id.filter(|id| self.labels_of(&account).iter().any(|l| &l.id == id)) {
                    Some(id) => self.apply_label(id, cx),
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
            (Menu::EveryLabel(index), _) => {
                if let Some(label) = self.every_labels.get(index).cloned() {
                    self.ask_delete_every_label(label, cx);
                }
            }
            (Menu::MoveTo, index) => {
                if let Some(target) = self.move_targets.get(index).cloned() {
                    self.move_selected(target, cx);
                }
            }
            (Menu::Mailbox(view), index) => {
                let section = if matches!(
                    view,
                    View::Inbox | View::Primary | View::Updates | View::Promotions | View::Social
                ) && !self.own_servers()
                {
                    "split"
                } else {
                    "mailbox"
                };
                if view != View::Inbox && index == 0 {
                    let hidden = self.sidebar_now().is_hidden(view);
                    self.set_hidden(view, !hidden, cx);
                } else {
                    self.reset_sidebar_section(section, cx);
                }
            }
            (Menu::More, index) => match self.more_actions.get(index).copied() {
                Some(MoreAction::ToggleOriginal) => self.toggle_original(cx),
                Some(MoreAction::MarkUnread) => self.mark_unread(cx),
                Some(MoreAction::OpenInGmail) => self.open_in_gmail(cx),
                Some(MoreAction::Spam) => self.remove_selected(mail::Change::Spam, cx),
                Some(MoreAction::Block) => self.toggle_block(cx),
                None => {}
            },
            (Menu::From, index) => {
                // The same list the menu showed (`toggle_menu`).
                if let Some((id, _)) = self.signature_accounts().into_iter().nth(index) {
                    self.set_compose_account(id, cx);
                }
            }
            (Menu::Signature, index) => match self.signature_choices.get(index).cloned() {
                Some(SignatureChoice::Use(id)) => self.set_compose_signature(Some(id), cx),
                Some(SignatureChoice::None) => self.set_compose_signature(None, cx),
                Some(SignatureChoice::Edit) => {
                    let from = self.compose_from().unwrap_or_default();
                    self.open_settings_on(Some(settings_window::Tab::Composing), Some(from), cx);
                }
                None => {}
            },
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

    /// Opens the right-click menu for a row of the list.
    pub(super) fn open_row_menu(
        &mut self,
        index: usize,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu_at = (from_screen(at.x), from_screen(at.y));
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
            RowAction::OpenInGmail => self.open_in_gmail_at(row, cx),
        }
        cx.notify();
    }

    /// Shows the "Undo" pill at the bottom centre of the window for a few seconds.
    pub(super) fn offer_undo(&mut self, label: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.offer_undo_for(label, None, cx);
    }

    /// `offer_undo`, with a bar that fills for `countdown` (and the pill shown as long), as
    /// the update's progress bar does: for a message being sent, the time left to take it
    /// back.
    pub(super) fn offer_undo_for(
        &mut self,
        label: impl Into<SharedString>,
        countdown: Option<Duration>,
        cx: &mut Context<Self>,
    ) {
        // A message being sent stays until it is sent (`show_sent`); this is in case it
        // never says.
        let shown = countdown.map_or(Duration::from_secs(6), |c| c + Duration::from_secs(60));
        let label: SharedString = label.into();
        // Wide enough for the label, the Undo button and the padding; 300 at least.
        let width = (text_width(&label, 13., crate::theme::SANS, cx) + 150.)
            .clamp(300., 460.)
            .ceil();
        // Room for the bar under the label and the button.
        let size = (width, if countdown.is_some() { 62. } else { 44. });
        self.close_undo(cx);
        let Some(frame) = self.frame.map(|f| self.popup_frame(f)) else {
            return;
        };
        let lift = self.update_lift();
        let origin = gpui::point(
            frame.origin.x + (frame.size.width - to_screen(size.0)) / 2. - to_screen(MENU_MARGIN),
            frame.origin.y + frame.size.height
                - to_screen(size.1 + 24. + lift)
                - to_screen(MENU_MARGIN),
        );
        let owner = cx.weak_entity();
        let dark = self.dark;
        let opened = cx.open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::new(
                    origin,
                    gpui::size(
                        to_screen(size.0 + 2. * MENU_MARGIN),
                        to_screen(size.1 + 2. * MENU_MARGIN),
                    ),
                ))),
                titlebar: None,
                kind: gpui::WindowKind::PopUp,
                display_id: self.display,
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
                // Comes in smoothly, and stays with the main window.
                crate::popup::enter(window, true);
                cx.new(|cx| {
                    // It goes away by itself too, so it can never be left behind.
                    cx.spawn_in(window, async move |_, cx| {
                        cx.background_executor().timer(shown).await;
                        cx.update(|window, _| crate::popup::leave(window)).ok();
                        cx.background_executor()
                            .timer(std::time::Duration::from_secs_f64(crate::popup::FADE))
                            .await;
                        cx.update(|window, _| window.remove_window()).ok();
                    })
                    .detach();
                    UndoToast {
                        owner,
                        label,
                        dark,
                        countdown,
                        can_undo: true,
                        sent: None,
                    }
                })
            },
        );
        let Ok(handle) = opened else {
            return;
        };
        self.undo_window = Some(handle);
        self.undo_sending = countdown.is_some();
        self.undo_gen += 1;
        let shown_gen = self.undo_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(shown).await;
            this.update(cx, |this, cx| {
                if this.undo_gen == shown_gen {
                    this.close_undo(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Shows the current message in a small window at the bottom centre, above the email
    /// (a native view that would cover anything drawn in the main window). It goes away by
    /// itself after a few seconds, longer for longer messages.
    pub(super) fn sync_notice_window(&mut self, toast: Option<Toast>, cx: &mut Context<Self>) {
        if let Some(handle) = self.notice_window.take() {
            remove_popup(handle, cx);
        }
        let (Some(toast), Some(frame)) = (toast, self.frame.map(|f| self.popup_frame(f))) else {
            return;
        };
        let (width, height) = (toast.width, toast.height);
        // Long enough to read it.
        let seconds = (4. + toast.text.chars().count() as f32 / 25.).min(20.);
        // Above the Undo pill when that is showing too.
        let lift = if self.undo_window.is_some() { 54. } else { 0. } + self.update_lift();
        let origin = gpui::point(
            frame.origin.x + (frame.size.width - to_screen(width)) / 2. - to_screen(MENU_MARGIN),
            frame.origin.y + frame.size.height
                - to_screen(height + 24. + lift)
                - to_screen(MENU_MARGIN),
        );
        let owner = cx.weak_entity();
        let dark = self.dark;
        let opened = cx.open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::new(
                    origin,
                    gpui::size(
                        to_screen(width + 2. * MENU_MARGIN),
                        to_screen(height + 2. * MENU_MARGIN),
                    ),
                ))),
                titlebar: None,
                kind: gpui::WindowKind::PopUp,
                display_id: self.display,
                focus: false,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                window_background: gpui::WindowBackgroundAppearance::Transparent,
                ..Default::default()
            },
            |window, cx| {
                crate::blur::no_window_shadow(window);
                // Comes in smoothly, and stays with the main window.
                crate::popup::enter(window, true);
                cx.new(|_| NoticeToast { owner, toast, dark })
            },
        );
        let Ok(handle) = opened else {
            return;
        };
        self.notice_window = Some(handle);
        self.notice_gen += 1;
        let shown = self.notice_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f32(seconds))
                .await;
            this.update(cx, |this, cx| {
                if this.notice_gen == shown {
                    this.notice = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The part of the main window (`frame`) the bottom popups centre in: the mail, right
    /// of the sidebar while it is fixed; the whole window while it hides.
    pub(super) fn popup_frame(
        &self,
        frame: gpui::Bounds<gpui::Pixels>,
    ) -> gpui::Bounds<gpui::Pixels> {
        if !self.sidebar_open {
            return frame;
        }
        let side = to_screen(self.side_w());
        gpui::Bounds::new(
            gpui::point(frame.origin.x + side, frame.origin.y),
            gpui::size(frame.size.width - side, frame.size.height),
        )
    }

    /// Shows `progress` in its window at the bottom of the main window (`placed`: its frame
    /// and look), or closes that window when there is nothing to show. The window is placed
    /// again when the main window moves or changes look, and otherwise only redrawn.
    pub(super) fn sync_update_window(
        &mut self,
        progress: Option<crate::update::Progress>,
        placed: Option<UpdatePlace>,
        cx: &mut Context<Self>,
    ) {
        let (Some(progress), Some(place)) = (progress, placed) else {
            if let Some(handle) = self.update_window.take() {
                remove_popup(handle, cx);
            }
            self.update_window_for = None;
            return;
        };
        if let Some(handle) = self.update_window
            && self.update_window_for == placed
        {
            let redrawn = handle.update(cx, |pill, _, cx| {
                pill.show(progress.clone(), cx);
            });
            if redrawn.is_ok() {
                return;
            }
        }
        if let Some(handle) = self.update_window.take() {
            remove_popup(handle, cx);
        }
        let owner = cx.weak_entity();
        let UpdatePlace {
            frame,
            dark,
            size: (width, height),
        } = place;
        let origin = gpui::point(
            frame.origin.x + (frame.size.width - to_screen(width)) / 2. - to_screen(MENU_MARGIN),
            frame.origin.y + frame.size.height - to_screen(height + 24.) - to_screen(MENU_MARGIN),
        );
        let opened = cx.open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::new(
                    origin,
                    gpui::size(
                        to_screen(width + 2. * MENU_MARGIN),
                        to_screen(height + 2. * MENU_MARGIN),
                    ),
                ))),
                titlebar: None,
                kind: gpui::WindowKind::PopUp,
                display_id: self.display,
                // Keyboard focus stays in the main window.
                focus: false,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                window_background: gpui::WindowBackgroundAppearance::Transparent,
                ..Default::default()
            },
            |window, cx| {
                crate::blur::no_window_shadow(window);
                // Comes in smoothly, and stays with the main window.
                crate::popup::enter(window, true);
                cx.new(|_| UpdatePill::new(owner, progress, dark))
            },
        );
        self.update_window = opened.ok();
        self.update_window_for = self.update_window.and(placed);
    }

    /// How far the pop-ups at the bottom of the window (Undo, notices) move up to clear an
    /// update's progress. Not at all once the update is done: its notice then takes the
    /// progress's place while that window is still on its way out.
    fn update_lift(&self) -> f32 {
        match (&self.update_progress, &self.update_window_for) {
            (Some(_), Some(place)) if self.update_window.is_some() => place.size.1 + 10.,
            _ => 0.,
        }
    }

    pub(super) fn close_undo(&mut self, cx: &mut Context<Self>) {
        self.undo_sending = false;
        if let Some(handle) = self.undo_window.take() {
            remove_popup(handle, cx);
        }
    }

    /// Changes the pill of a message being sent, if it is on screen.
    pub(super) fn update_sending_toast(
        &mut self,
        change: impl FnOnce(&mut UndoToast),
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(handle) = self.undo_window.filter(|_| self.undo_sending) else {
            return false;
        };
        handle
            .update(cx, |toast, _, cx| {
                change(toast);
                cx.notify();
            })
            .is_ok()
    }

    /// The message went: its pill says "Sent" in place of a notice of its own, then goes
    /// by itself. False when there is no pill to say it (the caller says it another way).
    pub(super) fn show_sent(&mut self, cx: &mut Context<Self>) -> bool {
        const SHOWN: Duration = Duration::from_millis(2800);
        let said = self.update_sending_toast(
            |toast| {
                toast.can_undo = false;
                toast.sent = Some("Sent".into());
            },
            cx,
        );
        if !said {
            return false;
        }
        let shown_gen = self.undo_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SHOWN).await;
            this.update(cx, |this, cx| {
                if this.undo_gen == shown_gen {
                    this.close_undo(cx);
                }
            })
            .ok();
        })
        .detach();
        true
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
                .gap(dp(10.))
                .px(dp(10.))
                .py(dp(7.))
                .rounded(dp(8.))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(icon(icon_name, 16., tint))
                .child(div().flex_1().child(label))
        };
        let panel = div()
            .id("menu")
            .absolute()
            .top(dp(50.))
            .flex()
            .flex_col()
            .min_w(dp(220.))
            .p(dp(6.))
            .rounded(dp(12.))
            .bg(t.raised)
            .border_1()
            .border_color(t.border_strong)
            .shadow(vec![shadow(t.shadow, 14., 36., -6.)])
            .occlude()
            // Any click outside the menu closes it.
            .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                this.menu = None;
                this.menu_dismissed = Some((menu, std::time::Instant::now()));
                cx.notify();
            }));
        Some(match menu {
            // These menus only exist as pop-up windows.
            Menu::Row(_)
            | Menu::SendLater
            | Menu::MoveTo
            | Menu::Label(_)
            | Menu::EveryLabel(_)
            | Menu::From
            | Menu::Signature
            | Menu::Mailbox(_) => {
                return None;
            }
            Menu::Labels => {
                let tints = [t.violet, t.rose, t.amber, t.sky, t.mint];
                panel
                    .left(dp(150.))
                    .when(self.labels.is_empty(), |d| {
                        d.child(div().px(dp(10.)).py(dp(7.)).text_color(t.text3).child(
                            if self.is_demo() {
                                "Labels need a connected account."
                            } else {
                                "You have no labels yet."
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
            Menu::More => {
                panel
                    .right(dp(20.))
                    .children(self.more_menu().into_iter().enumerate().map(
                        |(index, (_, icon, label))| {
                            item(index, icon, label, t.text2).on_click(cx.listener(
                                move |this, _, _, cx| this.menu_choice(Menu::More, index, cx),
                            ))
                        },
                    ))
            }
        })
    }
}

/// A signature's name as shown: one without a name still has something to click.
pub(super) fn signature_name(name: &str) -> &str {
    if name.trim().is_empty() {
        "Untitled"
    } else {
        name
    }
}

/// How a sidebar section is called in its menu.
fn section_name(section: &str) -> &'static str {
    match section {
        "split" => "Split Inbox",
        "labels" => "Labels",
        _ => "Mailbox",
    }
}
