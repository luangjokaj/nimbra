//! The list of threads: its header and rows, pulling it down to reload, the pane for several
//! picked threads, and what an empty or cleared mailbox shows.

use super::*;

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

/// How narrow and how wide the list can be dragged.
pub(super) const LIST_MIN: f32 = 300.;
pub(super) const LIST_MAX: f32 = 720.;

impl Nimbra {
    /// Whether nothing on screen means Inbox Zero, rather than nothing in this mailbox.
    pub(super) fn is_zero(&self) -> bool {
        match self.mailbox {
            View::Inbox if self.is_demo() => true,
            View::Inbox => self.all_cleared() && !self.next_pages.contains_key(&View::Inbox),
            // Not while mail is filtered away (Unread), unchecked or past an account's last page.
            View::ALL_INBOXES => self.all_inboxes_whole && self.all_cleared(),
            _ => false,
        }
    }

    /// Every thread that is listed was cleared, rather than some being filtered away.
    fn all_cleared(&self) -> bool {
        self.done.iter().all(|done| *done)
    }

    /// Relabels the rows' times ("9:41" becomes "Wed") and moves them from Today to Earlier
    /// once the day is over. Checked against the wall clock at each frame, a cheap read
    /// that also catches waking from sleep, and woken at midnight by one timer when
    /// nothing else is drawn.
    pub(super) fn roll_day(&mut self, cx: &mut Context<Self>) {
        if self.is_demo() {
            return;
        }
        let now = std::time::SystemTime::now();
        match self.day_ends {
            Some(end) if now < end => {
                if self.day_timer.is_none() {
                    self.wake_at_day_end(end.duration_since(now).unwrap_or_default(), cx);
                }
                return;
            }
            Some(_) => {
                for mail in self
                    .mails
                    .iter_mut()
                    .chain(self.cache.values_mut().flatten())
                {
                    mail.refresh_time();
                }
            }
            None => {}
        }
        let end = std::time::SystemTime::from(next_day_start(&chrono::Local::now()));
        self.day_ends = Some(end);
        self.wake_at_day_end(end.duration_since(now).unwrap_or_default(), cx);
    }

    fn wake_at_day_end(&mut self, wait: Duration, cx: &mut Context<Self>) {
        self.day_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(wait + Duration::from_millis(500))
                .await;
            this.update(cx, |this, cx| {
                this.day_timer = None;
                cx.notify();
            })
            .ok();
        }));
    }

    /// What `save_if_zero_changed` goes by, taken without walking the rows' accounts: it
    /// runs on every redraw request, animation frames included.
    pub(super) fn zero_inputs(&self) -> ZeroInputs {
        ZeroInputs {
            mailbox: self.mailbox,
            ready: self.status == Status::Ready,
            whole: self.all_inboxes_whole,
            account_gen: self.account_gen,
            rows: (self.mails.as_ptr() as usize, self.mails.len()),
            left: self.done.iter().filter(|done| !**done).count(),
            saved_empty: self.inbox_saved_empty,
            saved_empty_elsewhere: self.inboxes_saved_empty.values().filter(|e| **e).count(),
            accounts: self.settings.accounts.len(),
            stores: usize::from(self.store.is_some()) + self.other_stores.len(),
        }
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
                View::ALL_INBOXES => self.all_inboxes_saved_empty,
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

    /// Several threads picked: what can be done to all of them.
    pub(super) fn bulk_pane(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.targets().len();
        let action = |id: &'static str, name: &'static str, label: &'static str| {
            button(t, ButtonKind::Secondary, label, None)
                .id(id)
                .press_soft()
                .child(icon(name, 14., t.text2))
        };
        let row_of_actions = || div().flex().flex_wrap().justify_center().gap(dp(8.));
        let inbox_like = inbox_like(self.mailbox);
        div()
            .flex_1()
            .min_w(dp(0.))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(dp(18.))
            .bg(t.surface)
            .child(serif(
                &format!("{count} conversations, selected"),
                44.,
                t.text,
            ))
            // Two rows, what it is and where it goes, so a wrap never strands one button.
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(dp(8.))
                    .child(
                        row_of_actions()
                            .when(
                                inbox_like
                                    || matches!(
                                        self.mailbox.base(),
                                        View::Done
                                            | View::Spam
                                            | View::Trash
                                            | View::Snoozed
                                            | View::Reminders
                                    ),
                                |d| {
                                    d.child(
                                        action(
                                            "bulk-done",
                                            "done",
                                            if inbox_like {
                                                "Mark done"
                                            } else {
                                                "Move to inbox"
                                            },
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| this.mark_done(cx))),
                                    )
                                },
                            )
                            .child(
                                action("bulk-read", "mail", "Read or unread")
                                    .on_click(cx.listener(|this, _, _, cx| this.toggle_unread(cx))),
                            )
                            .child(
                                action("bulk-star", "star", "Star")
                                    .on_click(cx.listener(|this, _, _, cx| this.toggle_star(cx))),
                            ),
                    )
                    .child(
                        row_of_actions()
                            .child(action("bulk-label", "tag", "Label").on_click(cx.listener(
                                |this, _, window, cx| this.toggle_menu(Menu::Labels, window, cx),
                            )))
                            .child(
                                action("bulk-move", "folder", "Move to").on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.toggle_menu(Menu::MoveTo, window, cx)
                                    },
                                )),
                            )
                            .when(inbox_like && !self.is_demo(), |d| {
                                d.child(action("bulk-snooze", "clock", "Snooze").on_click(
                                    cx.listener(|this, event: &gpui::ClickEvent, window, cx| {
                                        this.open_snooze(Some(event.position()), window, cx)
                                    }),
                                ))
                            })
                            .when(self.mailbox.base() != View::Spam && !self.is_demo(), |d| {
                                d.child(action("bulk-spam", "ban", "Spam").on_click(cx.listener(
                                    |this, _, _, cx| this.remove_selected(mail::Change::Spam, cx),
                                )))
                            })
                            .child(
                                action("bulk-trash", "trash", "Trash")
                                    .on_click(cx.listener(|this, _, _, cx| this.trash(cx))),
                            ),
                    ),
            )
            .child(
                button(t, ButtonKind::Ghost, "Clear selection", Some("esc"))
                    .id("bulk-clear")
                    .press_soft()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.picked.clear();
                        cx.notify();
                    })),
            )
    }

    /// Pull to refresh, in the style of the system's: the list stretches with rubber-band
    /// resistance, letting go past the threshold starts the refresh, and the indicator
    /// springs back when it is done. Only a trackpad gesture that begins with the list
    /// already at the top counts, so the momentum of a normal scroll up never triggers it.
    fn pull_list(&mut self, event: &gpui::ScrollWheelEvent, cx: &mut Context<Self>) {
        let at_top = f32::from(self.list_scroll.offset().y) >= -0.5;
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
                self.pull_done = false;
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
        let dy = f32::from(event.delta.pixel_delta(to_screen(18.)).y);
        if (dy > 0. && at_top) || (dy < 0. && self.pull > 0.) {
            self.pull = (self.pull + dy).max(0.);
            self.pull_anim = None;
            // While the list is being pulled it must not scroll as well, or the two
            // movements fight each other.
            self.list_scroll.set_offset(gpui::point(px(0.), px(0.)));
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
                .gap(dp(8.))
                .h(dp(height.max(0.)))
                .overflow_hidden()
                .child(if spin {
                    arrow
                        .with_animation(
                            "pull-spin",
                            Animation::new(Duration::from_millis(800)).repeat(),
                            |svg, delta| {
                                svg.with_transformation(Transformation::rotate(percentage(delta)))
                            },
                        )
                        .into_any_element()
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
        } else if self.pull_done {
            "Done"
        } else {
            ""
        };
        Some(
            strip(settle(from, to, start), label, self.pull_hold, 0.75)
                .with_animation(
                    ("pull-settle", self.pull_gen),
                    // The animation only keeps frames coming; the height follows the spring.
                    Animation::new(Duration::from_secs_f32(SETTLE_SECONDS + 0.1)),
                    move |strip, _| strip.h(dp(settle(from, to, start))),
                )
                .into_any_element(),
        )
    }

    /// The message list in a wrapper whose width slides open or shut after a toggle, the
    /// same way the sidebar does. Once it has finished closing it is not drawn at all.
    pub(super) fn list_slot(
        &self,
        t: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        const SLIDE: Duration = Duration::from_millis((PANEL_SLIDE_SECONDS * 1000.) as u64);
        let moving = self.list_toggled.is_some_and(|at| at.elapsed() < SLIDE);
        if !self.list_open && !moving {
            return None;
        }
        let width = self.list_width;
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
                    .w(dp(width))
                    .h_full()
                    .child(self.list(t, window, cx)),
            );
        if !moving {
            return Some(slot.w(dp(width)).into_any_element());
        }
        let (from, to) = if self.list_open { (0., 1.) } else { (1., 0.) };
        Some(
            slot.with_animation(
                ("list-width", self.list_gen),
                Animation::new(SLIDE).with_easing(panel_ease),
                move |slot, delta| slot.w(dp(width * (from + (to - from) * delta))),
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
        self.list_toggled = (!crate::system::reduces_motion()).then(std::time::Instant::now);
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
        let el = el.relative().child(
            div()
                .absolute()
                .inset_0()
                .rounded(dp(radius))
                .border_1()
                .border_color(t.mint),
        );
        if !scrolled {
            return el;
        }
        el.child(reveal(
            self.sidebar_scroll.clone(),
            self.reveal_cursor.clone(),
            SIDEBAR_CLEAR,
        ))
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
        let hover = t.hover;
        // How many characters fit on a row at the list's current width (the text is cut
        // with an ellipsis by character count, so this follows the resize handle).
        let text =
            RowText::of(self.settings.reader_text).with_own_lines(self.settings.subject_own_line);
        let line = ((self.list_width - 120.) / text.char_w()).max(8.) as usize;
        // On a narrow list the tag gives way so the sender's name stays readable.
        let tag = m
            .tag
            .clone()
            .or_else(|| {
                let accents = [
                    Accent::Violet,
                    Accent::Rose,
                    Accent::Amber,
                    Accent::Sky,
                    Accent::Mint,
                ];
                self.first_label(m)
                    .map(|(index, label)| (short(&label.name, 18), accents[index % accents.len()]))
            })
            // By the list's width, not the text's: the pill keeps its own size.
            .filter(|_| self.list_width - 120. >= 36. * 7.4);
        let tag_chars = tag
            .as_ref()
            .map_or(0, |(label, _)| label.chars().count() + 6);
        // The time, the count, the star and the tag keep their own size whatever the mail
        // text's: their room is counted at the default size (7.4pt a character), then in
        // characters of the row's text.
        let extras =
            13 + tag_chars + if m.thread.is_some() { 3 } else { 0 } + if m.starred { 3 } else { 0 };
        let extras = (extras as f32 * 7.4 / text.char_w()).ceil() as usize;
        let sender_max = line.saturating_sub(extras).max(6);
        let subject = short(m.subject.split(',').next().unwrap_or(&m.subject), line);
        // On a line of its own the start of the email has the whole line; beside the
        // subject, what the subject leaves.
        let snippet_max = if text.own_lines {
            line
        } else {
            line.saturating_sub(subject.chars().count() + 2)
        };
        let snippet = if snippet_max >= 8 {
            short(&m.snippet, snippet_max)
        } else {
            SharedString::default()
        };

        div()
            .id(("row", row_id(m, self.account_of(m))))
            .group("row")
            .press(t)
            .relative()
            .flex()
            .flex_none()
            .h(dp(text.height()))
            .text_size(dp(text.size))
            .line_height(dp(text.line))
            .gap(dp(ROW_PAD))
            .px(dp(ROW_PAD))
            .py(dp(12.))
            .rounded(dp(12.))
            .cursor_pointer()
            .when(selected || picked, |d| d.bg(t.selected))
            // Drawn over the row rather than as its border, which would take up space and
            // make the row (and those below it) jump.
            .when(picked, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(dp(12.))
                        .border_1()
                        .border_color(t.violet.opacity(0.5)),
                )
            })
            .when(!selected && !picked, |d| d.hover(move |s| s.bg(hover)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    this.open_row_menu(i, event.position, window, cx)
                }),
            )
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                this.click_row(i, event.modifiers(), event.click_count(), cx)
            }))
            // The unread dot sits in the padding, so the avatar has the same space either side.
            .when(m.unread, |d| {
                d.child(
                    div()
                        .absolute()
                        .left(dp((ROW_PAD - 6.) / 2.))
                        // Level with the avatar's middle: the row's padding (12), then half
                        // of the sender's and the subject's lines and their gap.
                        .top(dp(12. + (text.first + LINE_GAP + text.second) / 2. - 3.))
                        .size(dp(6.))
                        .rounded_full()
                        .bg(t.violet),
                )
            })
            // In the middle of the sender's line and the subject's beside it, also when the
            // start of the email has a line of its own under them.
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(dp(text.first + LINE_GAP + text.second))
                    .child(self.sender_avatar(m, 28.)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(dp(0.))
                    .gap(dp(LINE_GAP))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(dp(8.))
                            .h(dp(text.first))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .min_w(dp(0.))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .child(short(&m.sender, sender_max)),
                            )
                            .children(m.thread.clone().map(|n| mono(n, t.text3)))
                            .when(m.starred, |d| d.child(icon("star", 13., t.amber)))
                            // Half a point up: measured, its box and letters sat a third of a
                            // point below the name's middle, and its lowercase looked lower.
                            .children(tag.map(|(label, a)| {
                                pill(t, &label, t.accent(a)).relative().top(dp(-0.5))
                            }))
                            .child(div().flex_1())
                            // Snoozed: when it comes back, in place of when it came. On the
                            // open row the quick actions take this place while it is hovered.
                            .child(
                                div()
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .children(snoozed.as_ref().map(|(name, when)| {
                                        super::threads::snooze_chip(t, name, when.clone())
                                    }))
                                    .when(snoozed.is_none(), |d| {
                                        d.child(mono(m.time.clone(), t.text3))
                                    })
                                    .when(selected, |d| d.group_hover("row", |s| s.invisible())),
                            )
                            // Quick actions on the open row, under the pointer: done, snooze,
                            // remind me. (The keyboard has E and H.)
                            .when(selected, |d| {
                                let quick = |id: &'static str, name: &'static str| {
                                    let (hover, bright) = (t.hover, t.text);
                                    div()
                                        .id((id, i))
                                        .group("quick")
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(dp(24.))
                                        .rounded(dp(6.))
                                        .cursor_pointer()
                                        .hover(move |s| s.bg(hover))
                                        .press(t)
                                        .child(
                                            icon(name, 15., t.text2)
                                                .group_hover("quick", move |s| {
                                                    s.text_color(bright)
                                                }),
                                        )
                                };
                                let archivable = self.done_change().is_some();
                                d.relative().child(
                                    div()
                                        .absolute()
                                        .right_0()
                                        .top(dp(-1.))
                                        .flex()
                                        .gap(dp(2.))
                                        .invisible()
                                        .group_hover("row", |s| s.visible())
                                        .when(archivable, |d| {
                                            d.child(
                                                quick("quick-done", "done")
                                                    .tooltip(tip(
                                                        t.dark,
                                                        if matches!(
                                                            self.mailbox,
                                                            View::Done
                                                                | View::Spam
                                                                | View::Trash
                                                                | View::Snoozed
                                                                | View::Reminders
                                                        ) {
                                                            "Move to inbox"
                                                        } else {
                                                            "Mark done"
                                                        },
                                                        "E",
                                                    ))
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            cx.stop_propagation();
                                                            this.selected = i;
                                                            this.mark_done(cx);
                                                        },
                                                    )),
                                            )
                                        })
                                        .child(
                                            quick("quick-snooze", "clock")
                                                .tooltip(tip(t.dark, "Snooze", "H"))
                                                .on_click(cx.listener(
                                                    move |this, event: &gpui::ClickEvent, window, cx| {
                                                        cx.stop_propagation();
                                                        this.selected = i;
                                                        this.picked.clear();
                                                        this.open_selected(cx);
                                                        this.open_snooze(
                                                            Some(event.position()),
                                                            window,
                                                            cx,
                                                        );
                                                    },
                                                )),
                                        )
                                        .child(
                                            quick("quick-remind", "bell")
                                                .tooltip(tip(
                                                    t.dark,
                                                    "Remind me if no reply in 3 days",
                                                    "",
                                                ))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    this.selected = i;
                                                    this.picked.clear();
                                                    if this.is_demo() {
                                                        this.open_palette();
                                                        this.palette_sel = 0;
                                                        cx.notify();
                                                        return;
                                                    }
                                                    if let Some(choice) = Self::snooze_choices()
                                                        .into_iter()
                                                        .find(|c| c.no_reply)
                                                    {
                                                        this.snooze(choice, cx);
                                                    }
                                                })),
                                        ),
                                )
                            }),
                    )
                    .map(|d| {
                        // Long subjects shrink with an ellipsis instead of spilling out of
                        // the row; the snippet takes what is left, or a line of its own.
                        let subject = div()
                            .min_w(dp(0.))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if selected { t.text } else { t.text2 })
                            .child(subject);
                        let snippet = div()
                            .flex_1()
                            .min_w(dp(0.))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_color(t.text3)
                            .child(snippet);
                        if text.own_lines {
                            d.child(div().flex().h(dp(text.second)).child(subject))
                                .child(div().flex().h(dp(text.second)).child(snippet))
                        } else {
                            d.child(
                                div()
                                    .flex()
                                    .h(dp(text.second))
                                    .gap(dp(6.))
                                    .child(subject)
                                    .child(snippet),
                            )
                        }
                    }),
            )
    }

    fn list(&self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = self.visible();
        let unread = visible.iter().filter(|i| self.mails[**i].unread).count();
        let slots = slots(
            visible.iter().map(|&i| (i, self.mails[i].yesterday)),
            self.more().is_some(),
        );

        // The filters are the toolbar's actions (an icon and a word, 30pt tall): each has a
        // fixed width, so the active look behind them can slide from one to the other (and
        // nothing shifts when it does).
        let unread_text = unread.to_string();
        let word = |label: &str| text_width(label, 13.5, crate::theme::SANS, cx);
        let widths = [
            9. + 16. + 6. + word("All") + 9.,
            9. + 16. + 6. + word("Unread") + 6. + 7. * unread_text.len() as f32 + 9.,
        ];
        // The filters and the 2pt between them.
        let mut switch_w = widths[0] + widths[1] + 2.;
        if self.is_demo() {
            switch_w += 2. + 140.;
        }
        // Dot, gap, padding and border around the pill's words.
        let offline_w = self
            .offline
            .then(|| 30. + text_width(&self.offline_label(), 12., crate::theme::SANS, cx));
        let fit = HeaderFit::new(
            self.list_width - 1. - LIST_LEFT - LIST_RIGHT,
            switch_w,
            offline_w,
        );
        let hover = t.hover;
        let segment =
            |name: &str, label: &str, count: Option<(&str, Hsla)>, active: bool, width: f32| {
                let color = if active { t.text } else { t.text2 };
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(dp(6.))
                    .w(dp(width))
                    .h(dp(30.))
                    .px(dp(9.))
                    .rounded(dp(8.))
                    .text_size(dp(13.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(color)
                    .when(!active, |d| d.hover(move |s| s.bg(hover)))
                    .child(icon(name, 16., color))
                    .child(label.to_string())
                    .children(count.map(|(n, color)| mono(n.to_string(), color)))
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
        // The active one has the look a toolbar action has while it is pressed.
        let pill = div()
            .absolute()
            .top_0()
            .h(dp(30.))
            .rounded(dp(8.))
            .bg(t.selected);
        // Only right after a switch (not again each time the list is drawn anew).
        let switching = self
            .filter_switched
            .is_some_and(|at| at.elapsed() < Duration::from_millis(260))
            && !crate::system::reduces_motion();
        let pill = if self.filter_gen == 0 || !switching {
            pill.left(dp(to_x)).w(dp(to_w)).into_any_element()
        } else {
            pill.with_animation(
                ("filter-pill", self.filter_gen),
                Animation::new(Duration::from_millis(260)).with_easing(ease_out_quint()),
                move |d, delta| {
                    d.left(dp(from_x + (to_x - from_x) * delta))
                        .w(dp(from_w + (to_w - from_w) * delta))
                },
            )
            .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .flex_none()
            .relative()
            .w(dp(self.list_width))
            .h_full()
            .bg(t.surface)
            .border_r_1()
            .border_color(t.border)
            // Drag the right edge to resize the list; double-click it to hide the list.
            .child(
                div()
                    .id("list-resize")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(dp(-3.))
                    .w(dp(7.))
                    .cursor(gpui::CursorStyle::ResizeLeftRight)
                    .hover(|s| s.bg(gpui::white().opacity(0.06)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            // Twice: back to its first width.
                            if event.click_count == 2 {
                                this.list_grab = None;
                                this.list_width = crate::settings::Settings::default().list_width;
                                this.keep_list_width(cx);
                                cx.notify();
                            } else {
                                this.list_grab =
                                    Some((from_screen(event.position.x), this.list_width));
                            }
                        }),
                    ),
            )
            .child(
                // One row on the top line (y = 25): the filters, then Offline and Compose.
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(dp(HEADER_GAP))
                    // With a line along its bottom, level with the reading toolbar's.
                    .h(dp(crate::ui::top_bar()))
                    .border_b_1()
                    .border_color(t.border)
                    .pl(dp(LIST_LEFT))
                    .pr(dp(LIST_RIGHT))
                    .on_mouse_down(MouseButton::Left, drag_or_zoom)
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_none()
                            .gap(dp(2.))
                            .child(pill)
                            .child(
                                segment("inbox", "All", None, !self.unread_only, widths[0])
                                    .id("filter-all")
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_unread_only(false, cx)
                                    })),
                            )
                            .child(
                                segment(
                                    "mail",
                                    "Unread",
                                    Some((&unread_text, t.faint)),
                                    self.unread_only,
                                    widths[1],
                                )
                                .id("filter-unread")
                                .cursor_pointer()
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.set_unread_only(true, cx)),
                                ),
                            )
                            .when(self.is_demo(), |d| {
                                d.child(segment(
                                    "reply",
                                    "Needs reply",
                                    Some(("2", t.rose)),
                                    false,
                                    140.,
                                ))
                            }),
                    )
                    .child(div().flex_1().min_w(dp(0.)).mx(dp(-HEADER_GAP / 2.)))
                    .when(self.offline, |d| d.child(self.offline_pill(t, fit.words)))
                    // Compose as the reading toolbar's actions are: its icon and word (just
                    // the icon where the list is narrow). The word ends as far from the line
                    // as the rest of the bar's edges.
                    .child({
                        let worded = fit.compose != ComposeLook::Icon;
                        super::reading::action_button(t, "draft", "Compose", worded)
                            .id("compose")
                            .mr(dp(if worded { -9. } else { -7. }))
                            .press(t)
                            .tooltip(tip(t.dark, "New message", "C"))
                            .on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.open_compose(ComposeKind::New, cx)
                                }),
                            )
                    }),
            )
            // While mail is checked, a thin line glides along the top of the list, on the
            // header's line (`LoadingLine`).
            .child(self.checking_line.view(t, "checking-mail"))
            .child(self.rows(t, &slots, window, cx))
    }

    /// Only the rows in and near view are built; spacers of the exact height stand in for
    /// the rest, so scrolling, the scroll position and `near_list_end` work as before.
    fn rows(
        &self,
        t: &Theme,
        slots: &[Slot],
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let pull = self.pull_indicator(t);
        let row_h = RowText::of(self.settings.reader_text)
            .with_own_lines(self.settings.subject_own_line)
            .height();
        let lead = self.pull_height().map_or(0., |h| h + SLOT_GAP);
        let view_h = from_screen(self.list_scroll.bounds().size.height);
        let target = self.pick_end.unwrap_or(self.selected);
        if view_h > 0.
            && self.reveal_row.replace(false)
            && let Some(k) = slots.iter().position(|s| *s == Slot::Row(target))
        {
            let top = lead + slot_top(slots, k, row_h);
            let scroll = -from_screen(self.list_scroll.offset().y);
            if let Some(to) = reveal_scroll(top, top + row_h, scroll, view_h) {
                self.list_scroll
                    .set_offset(gpui::point(px(0.), to_screen(-to)));
            }
        }
        // As GPUI will clamp it once laid out, so rows removed at the end of a scrolled
        // list never leave a frame with nothing drawn.
        let total = slot_top(slots, slots.len(), row_h);
        let scroll = (-from_screen(self.list_scroll.offset().y))
            .min((lead + total + 10. - view_h).max(0.))
            - lead;
        // The window bounds the list's height, so this holds even before the first layout
        // or right after a resize, when the list's own bounds are a frame old.
        let tallest = from_screen(window.viewport_size().height);
        let (range, above, below) =
            window_of(slots, scroll - OVERDRAW, scroll + tallest + OVERDRAW, row_h);
        let group = |label: &'static str| {
            div()
                .flex_none()
                .h(dp(HEADER_H))
                .pl(dp(14.))
                .pt(dp(14.))
                .pb(dp(6.))
                .child(overline(label, t.faint))
        };
        let first = range.start;
        div()
            .id("rows")
            .flex()
            .flex_col()
            .flex_1()
            // The rows (a picked one's highlight) start and end where the header's switch
            // and Compose do.
            .pl(dp(LIST_LEFT))
            .pr(dp(LIST_RIGHT))
            .pb(dp(10.))
            .overflow_y_scroll()
            .track_scroll(&self.list_scroll)
            .on_scroll_wheel(cx.listener(|this, event, _, cx| this.pull_list(event, cx)))
            .children(pull.map(|strip| div().flex_none().mb(dp(SLOT_GAP)).child(strip)))
            .child(div().flex_none().h(dp(above)))
            .children(slots[range].iter().enumerate().map(|(n, slot)| {
                let el = match *slot {
                    Slot::Header(false) => group("TODAY").into_any_element(),
                    Slot::Header(true) => group("EARLIER").into_any_element(),
                    Slot::Row(i) => self.row(t, i, cx).into_any_element(),
                    // More is on its way: placeholder rows that breathe while the next
                    // page loads (it starts on its own as the end of the list comes near).
                    Slot::Waiting(n) => waiting_row(t, n),
                };
                div()
                    .flex_none()
                    .when(first + n > 0, |d| d.pt(dp(SLOT_GAP)))
                    .child(el)
            }))
            .child(div().flex_none().h(dp(below)))
    }

    /// The pull-to-refresh strip's height now, when it is drawn.
    fn pull_height(&self) -> Option<f32> {
        if self.pull > 0. {
            return Some(rubber_band(self.pull));
        }
        self.pull_anim
            .map(|(from, to, start)| settle(from, to, start).max(0.))
    }

    /// An empty mailbox other than the inbox (which gets the Inbox Zero screen).
    pub(super) fn empty_mailbox(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let search = self.mailbox == View::Search;
        // Not empty at all: filtered, or the rest could not be had.
        let filtered = self.unread_only && !self.all_cleared();
        let unlisted = match self.mailbox {
            View::ALL_INBOXES => !self.all_inboxes_whole,
            view => self.next_pages.contains_key(&view),
        };
        let (title, line) = if filtered {
            (
                format!("Nothing unread in {}", self.inbox_name()),
                "What you have read is still here.".to_string(),
            )
        } else if search {
            (
                "No results".to_string(),
                format!("Nothing matches “{}”.", short(&self.query, 60)),
            )
        } else if unlisted {
            (
                "Not all here yet".to_string(),
                "The rest could not be checked. Reload to try again.".to_string(),
            )
        } else {
            (
                format!("Nothing in {}", self.inbox_name()),
                match self.mailbox.base() {
                    View::Snoozed => "Press H on a thread to put it away until later.",
                    View::Reminders => {
                        "Press H and pick “Remind me if no reply” to be told when nobody answers."
                    }
                    _ => "Threads will show up here.",
                }
                .to_string(),
            )
        };
        div()
            .flex_1()
            .min_w(dp(0.))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(dp(14.))
            .bg(t.surface)
            .child(serif(&title, 56., t.text))
            .child(
                div()
                    .text_size(dp(15.))
                    .line_height(dp(24.))
                    .text_color(t.text2)
                    .child(line),
            )
            // The filter's own switch is in the list's header, which is not drawn here.
            .when(filtered, |d| {
                d.child(
                    button(t, ButtonKind::Secondary, "Show all", None)
                        .id("show-all")
                        .press_soft()
                        .on_click(cx.listener(|this, _, _, cx| this.set_unread_only(false, cx))),
                )
            })
    }

    /// A star falls now and then while Inbox Zero is in front of you, drawn every frame
    /// while it falls; between falls one timer waits for the next and nothing is drawn.
    /// None with Reduce Motion, while the window is in the background, and while it comes up.
    pub(super) fn move_sky(
        &mut self,
        zero_shown: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !zero_shown {
            self.sky = None;
            self.sky_timer = None;
            return;
        }
        if self.arrival.is_some() {
            return;
        }
        let sky = self.sky.get_or_insert_with(crate::sky::Sky::new);
        let now = sky.now();
        if !window.is_window_active() || (!sky.falling() && crate::system::reduces_motion()) {
            sky.rest(now);
            self.sky_timer = None;
            return;
        }
        if sky.due(now) {
            sky.start(now);
        }
        if sky.falling() {
            self.sky_timer = None;
            window.request_animation_frame();
            return;
        }
        if self.sky_timer.is_some() {
            return;
        }
        let wait = Duration::from_secs_f32(sky.wait(now).unwrap_or(0.));
        self.sky_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| {
                this.sky_timer = None;
                cx.notify();
            })
            .ok();
        }));
    }

    pub(super) fn zero(&self, t: &Theme) -> impl IntoElement {
        let ink = c(0xf5f3ef, 1.);
        let soft = c(0xffffff, 0.7);
        let arrival = self.arrival.unwrap_or(Arrival::SETTLED);
        let settle = |d: Div, shown: f32| d.relative().top(dp(drop_for(shown))).opacity(shown);
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
                    .size(dp(size))
                    .rounded_full()
                    .bg(c(0xffffff, a * arrival.star(i)))
            })
            .collect();
        let falling = self
            .sky
            .as_ref()
            .and_then(|sky| sky.fall(sky.now()))
            .map(|(fall, dots)| {
                div()
                    .absolute()
                    .left(relative(fall.x))
                    .top(relative(fall.y))
                    .children(dots.into_iter().map(|dot| {
                        div()
                            .absolute()
                            .left(dp(dot.x - dot.size / 2.))
                            .top(dp(dot.y - dot.size / 2.))
                            .size(dp(dot.size))
                            .rounded_full()
                            .bg(c(0xfffaf0, dot.alpha))
                    }))
            });
        // The sample inbox keeps the day its mail is from.
        let today = if self.is_demo() {
            "THURSDAY, NOVEMBER 20".to_string()
        } else {
            crate::when::local(chrono::Local::now(), crate::when::FULL_DAY).to_uppercase()
        };
        let layer = |index: usize, angle: f32, color: u32, alpha: f32, end: f32| {
            let shown = arrival.glow(index);
            // Each reaches further from its edge as it brightens, so the light rises.
            let reach = end * (0.6 + 0.4 * shown);
            div()
                .absolute()
                .inset_0()
                .bg(fade(angle, color, alpha * shown, reach))
        };

        let below = if self.can_undo() {
            div()
                .flex()
                .items_center()
                .gap(dp(6.))
                .child(meta("Press", c(0xffffff, 0.5)))
                .child(kbd(&Theme::dark(), "Z"))
                .child(meta("to undo", c(0xffffff, 0.5)))
        } else {
            // The day's line, centred; a quote has its author set apart below it.
            let (line, author) = crate::inspiration::split(crate::inspiration::line_for(
                chrono::Local::now().date_naive(),
            ));
            div()
                .w_full()
                .px(dp(32.))
                .flex()
                .flex_col()
                .items_center()
                .gap(dp(12.))
                .child(
                    div()
                        .max_w(dp(620.))
                        .text_center()
                        .font_family(SERIF)
                        .italic()
                        .text_size(dp(28.))
                        .line_height(dp(34.))
                        .text_color(soft)
                        .child(line),
                )
                .children(author.map(|author| overline(author.to_uppercase(), c(0xffffff, 0.55))))
        };

        div().flex_1().min_w(dp(0.)).h_full().bg(t.surface).child(
            div()
                .relative()
                .flex()
                .flex_col()
                .items_center()
                .size_full()
                .overflow_hidden()
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
                        .child(settle(overline(today, c(0xffffff, 0.55)), arrival.date()))
                        .child(div().h(dp(18.)).flex_none())
                        .child(settle(serif("Inbox zero.", 120., ink), arrival.headline()))
                        .child(div().h(dp(14.)).flex_none())
                        // As a real account shows it: the date and the headline; the day's
                        // thought is below the horizon.
                        .child(settle(div(), arrival.line())),
                )
                // Drawn out from the middle, and fading towards both ends.
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .w(relative(arrival.horizon()))
                        .h(dp(1.))
                        .child(div().flex_1().h_full().bg(gradient(
                            90.,
                            c(0xffffff, 0.),
                            c(0xffffff, 0.28),
                        )))
                        .child(div().flex_1().h_full().bg(gradient(
                            90.,
                            c(0xffffff, 0.28),
                            c(0xffffff, 0.),
                        ))),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .w_full()
                        .gap(dp(18.))
                        .child(settle(below, arrival.below())),
                ),
        )
    }
}

/// When the day after `now` starts: its midnight, or the first moment of it where a
/// clock change skips midnight.
pub(super) fn next_day_start<Tz: chrono::TimeZone>(
    now: &chrono::DateTime<Tz>,
) -> chrono::DateTime<Tz> {
    let zone = now.timezone();
    now.date_naive()
        .succ_opt()
        .and_then(|day| {
            (0..24).find_map(|hour| {
                zone.from_local_datetime(&day.and_hms_opt(hour, 0, 0)?)
                    .earliest()
            })
        })
        .unwrap_or_else(|| now.clone() + chrono::Duration::days(1))
}

/// See `Nimbra::zero_inputs`. The rows are told apart by where they are kept, which changes
/// whenever a listing replaces them; marking done in place changes `left`.
#[derive(Clone, PartialEq)]
pub(super) struct ZeroInputs {
    mailbox: View,
    ready: bool,
    whole: bool,
    account_gen: u64,
    rows: (usize, usize),
    left: usize,
    saved_empty: bool,
    saved_empty_elsewhere: usize,
    accounts: usize,
    stores: usize,
}

/// The space either side of a row's avatar, and at the row's right.
pub(super) const ROW_PAD: f32 = 14.;

/// The list header's margins: All / Unread and Compose sit this far from the lines either
/// side, as far as the reading toolbar's first icon does from its edge.
pub(super) const LIST_LEFT: f32 = 18.;

pub(super) const LIST_RIGHT: f32 = 18.;

pub(super) const HEADER_GAP: f32 = 8.;

/// How Compose is drawn, as the room for it shrinks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ComposeLook {
    Keyed,
    Plain,
    Icon,
}

impl ComposeLook {
    pub(super) fn width(self) -> f32 {
        match self {
            // The button, less the 9 or 7 it reaches into the margin.
            ComposeLook::Keyed | ComposeLook::Plain => 92.,
            ComposeLook::Icon => 24.,
        }
    }
}

/// What fits in the list's one-row header. The list can be 300pt narrow, so first the
/// Offline pill loses its words, then Compose its key, then Compose its label.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct HeaderFit {
    pub(super) words: bool,
    pub(super) compose: ComposeLook,
}

impl HeaderFit {
    pub(super) fn new(room: f32, switch: f32, offline: Option<f32>) -> Self {
        use ComposeLook::{Icon, Keyed, Plain};
        for (words, compose) in [(true, Keyed), (false, Keyed), (false, Plain)] {
            let pill = offline.map_or(0., |wide| HEADER_GAP + if words { wide } else { 22. });
            if switch + pill + HEADER_GAP + compose.width() <= room {
                return HeaderFit { words, compose };
            }
        }
        HeaderFit {
            words: false,
            compose: Icon,
        }
    }
}

/// The list's fixed heights, so the rows out of view can be left out and stood in for
/// exactly: a thread's row (its padding, its two lines and the gap between them), a day's
/// heading, a placeholder row while the next page loads, and the space between any two.
pub(super) const ROW_H: f32 = 12. + 22. + LINE_GAP + 18. + 12.;

/// Between a row's two lines: with the room above the subject's letters, as much as a
/// label on the first line has beside it (8).
pub(super) const LINE_GAP: f32 = 5.;

/// A thread row's text at the mail text size: its size and line height, and the heights
/// of its two lines (the sender's, and the subject's). Default is the one `ROW_H` is for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RowText {
    pub(super) size: f32,
    pub(super) line: f32,
    pub(super) first: f32,
    pub(super) second: f32,
    /// The subject and the start of the email on lines of their own (Settings ›
    /// Appearance): a third line, as tall as the second.
    pub(super) own_lines: bool,
}

impl RowText {
    pub(super) fn of(size: crate::settings::TextSize) -> Self {
        use crate::settings::TextSize::{Default, ExtraLarge, Large, Small};
        let (size, line, first, second) = match size {
            Small => (12., 17., 20., 17.),
            Default => (13., 18., 22., 18.),
            Large => (14.5, 20., 24., 20.),
            ExtraLarge => (16., 22., 26., 22.),
        };
        RowText {
            size,
            line,
            first,
            second,
            own_lines: false,
        }
    }

    /// The same with the subject and the start of the email on lines of their own.
    pub(super) fn with_own_lines(self, own_lines: bool) -> Self {
        RowText { own_lines, ..self }
    }

    /// The row's height: its padding, its lines and the gaps between them.
    pub(super) fn height(self) -> f32 {
        let third = if self.own_lines {
            LINE_GAP + self.second
        } else {
            0.
        };
        12. + self.first + LINE_GAP + self.second + third + 12.
    }

    /// An average character's width, for cutting text to fit by count.
    pub(super) fn char_w(self) -> f32 {
        7.4 * self.size / 13.
    }
}
const HEADER_H: f32 = 14. + 12. + 6.;
const WAITING_H: f32 = 14. + 3. + 9. + 8. + 9. + 14.;
const SLOT_GAP: f32 = 2.;
/// How far past the visible rows they are built, so a fast scroll never shows a gap.
const OVERDRAW: f32 = 400.;

/// One entry of the list: a heading (Earlier when true), a thread by its row, or one of
/// the placeholder rows for the next page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Slot {
    Header(bool),
    Row(usize),
    Waiting(usize),
}

impl Slot {
    /// Its height, with thread rows `row_h` tall (`RowText::height`).
    fn height(self, row_h: f32) -> f32 {
        match self {
            Slot::Header(_) => HEADER_H,
            Slot::Row(_) => row_h,
            Slot::Waiting(_) => WAITING_H,
        }
    }
}

/// The list's entries for these rows (each with whether it is from before today): Today's,
/// then Earlier's, each under its heading, and two placeholders when more is coming.
pub(super) fn slots(rows: impl Iterator<Item = (usize, bool)> + Clone, more: bool) -> Vec<Slot> {
    let mut slots = Vec::new();
    for earlier in [false, true] {
        let mut group = rows.clone().filter(|(_, e)| *e == earlier).peekable();
        if group.peek().is_some() {
            slots.push(Slot::Header(earlier));
            slots.extend(group.map(|(i, _)| Slot::Row(i)));
        }
    }
    if more {
        slots.extend([Slot::Waiting(0), Slot::Waiting(1)]);
    }
    slots
}

/// Where entry `k` starts, from the top of the first.
pub(super) fn slot_top(slots: &[Slot], k: usize, row_h: f32) -> f32 {
    slots[..k].iter().map(|s| s.height(row_h) + SLOT_GAP).sum()
}

/// The entries that reach into `from..to` (from the top of the first), and the space above
/// and below them that stands in for the rest.
pub(super) fn window_of(
    slots: &[Slot],
    from: f32,
    to: f32,
    row_h: f32,
) -> (std::ops::Range<usize>, f32, f32) {
    let mut top = 0.;
    let mut start = slots.len();
    let mut end = slots.len();
    let mut above = 0.;
    for (k, slot) in slots.iter().enumerate() {
        let gap = if k == 0 { 0. } else { SLOT_GAP };
        let bottom = top + gap + slot.height(row_h);
        if start == slots.len() && bottom > from {
            start = k;
            above = top;
        }
        if top + gap >= to {
            end = k;
            break;
        }
        top = bottom;
    }
    let total = slot_top(slots, slots.len(), row_h) - if slots.is_empty() { 0. } else { SLOT_GAP };
    if start == slots.len() {
        return (start..start, total, 0.);
    }
    let end = end.max(start);
    let rendered_end = if end == 0 {
        0.
    } else {
        slot_top(slots, end, row_h) - SLOT_GAP
    };
    (start..end, above, (total - rendered_end).max(0.))
}

/// The scroll position that brings `top..bottom` into a view `height` tall, scrolled to
/// `scroll`: the least move, or none when it is in view already.
pub(super) fn reveal_scroll(top: f32, bottom: f32, scroll: f32, height: f32) -> Option<f32> {
    if top < scroll {
        Some(top)
    } else if bottom > scroll + height {
        Some(bottom - height)
    } else {
        None
    }
}

/// An id for a thread's row that stays with the thread when rows move, made without
/// building a string each frame.
fn row_id(mail: &Mail, account: &str) -> usize {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    match &mail.id {
        Some(id) => id.hash(&mut hash),
        None => mail.subject.hash(&mut hash),
    }
    account.hash(&mut hash);
    hash.finish() as usize
}

/// A placeholder row while the next page loads.
fn waiting_row(t: &Theme, n: usize) -> AnyElement {
    let bar = |w: f32| div().h(dp(9.)).w(dp(w)).rounded_full().bg(t.hover);
    div()
        .flex()
        .flex_none()
        .h(dp(WAITING_H))
        .gap(dp(ROW_PAD))
        .px(dp(ROW_PAD))
        .py(dp(14.))
        .child(div().size(dp(28.)).flex_none().rounded_full().bg(t.hover))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(dp(8.))
                .pt(dp(3.))
                .child(bar(if n == 0 { 120. } else { 96. }))
                .child(bar(if n == 0 { 220. } else { 180. })),
        )
        .with_animation(
            ("more-loading", n),
            Animation::new(Duration::from_millis(1400)).repeat(),
            move |row, delta| {
                let wave = (std::f32::consts::TAU * (delta - n as f32 * 0.15)).cos();
                row.opacity(0.55 + 0.35 * wave)
            },
        )
        .into_any_element()
}

/// How far down Inbox Zero's horizon is.
pub(super) const HORIZON: f32 = 0.72;

/// The height of an option in the All / Unread switch.
pub(super) const SEGMENT_H: f32 = 26.;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, TimeZone, Utc};

    fn sample() -> Vec<Slot> {
        // Rows 0..3 today, 3..50 earlier, more to come.
        slots((0..50).map(|i| (i, i >= 3)), true)
    }

    #[test]
    fn slots_put_today_then_earlier_under_headings_and_placeholders_last() {
        let s = sample();
        assert_eq!(s[0], Slot::Header(false));
        assert_eq!(s[1..4], [Slot::Row(0), Slot::Row(1), Slot::Row(2)]);
        assert_eq!(s[4], Slot::Header(true));
        assert_eq!(s[s.len() - 2..], [Slot::Waiting(0), Slot::Waiting(1)]);
        assert_eq!(
            slots((0..2).map(|i| (i, true)), false)[0],
            Slot::Header(true)
        );
        assert!(slots(std::iter::empty(), false).is_empty());
    }

    #[test]
    fn the_window_and_its_spacers_add_up_to_the_whole_list_wherever_it_is_scrolled() {
        let s = sample();
        let total = slot_top(&s, s.len(), ROW_H) - SLOT_GAP;
        for from in [-500., 0., 33., 700., 2000., total - 100., total + 300.] {
            let to = from + 900.;
            let (range, above, below) = window_of(&s, from, to, ROW_H);
            let drawn: f32 = s[range.clone()]
                .iter()
                .map(|x| x.height(ROW_H))
                .sum::<f32>()
                + SLOT_GAP * range.len().saturating_sub(1) as f32
                + if range.start > 0 && !range.is_empty() {
                    SLOT_GAP
                } else {
                    0.
                };
            assert!(
                (above + drawn + below - total).abs() < 0.01,
                "{from}: {range:?}"
            );
            // Every entry that reaches into the band is drawn.
            for k in 0..s.len() {
                let top = slot_top(&s, k, ROW_H);
                let bottom = top + s[k].height(ROW_H);
                if bottom > from && top < to {
                    assert!(range.contains(&k), "{from}: {k} missing from {range:?}");
                }
            }
        }
    }

    #[test]
    fn rows_grow_with_the_mail_text_size_and_default_is_row_h() {
        use crate::settings::TextSize;
        assert_eq!(RowText::of(TextSize::Default).height(), ROW_H);
        // The subject and the start of the email on lines of their own: one line more.
        let own = RowText::of(TextSize::Default).with_own_lines(true);
        assert_eq!(own.height(), ROW_H + LINE_GAP + own.second);
        let sizes = [
            TextSize::Small,
            TextSize::Default,
            TextSize::Large,
            TextSize::ExtraLarge,
        ]
        .map(RowText::of);
        for pair in sizes.windows(2) {
            assert!(pair[0].height() < pair[1].height());
            assert!(pair[0].char_w() < pair[1].char_w());
            // Each line is tall enough for its text.
            assert!(pair[1].first >= pair[1].line && pair[1].second >= pair[1].line);
        }
    }

    #[test]
    fn the_window_holds_with_taller_rows() {
        let s = sample();
        let row_h = RowText::of(crate::settings::TextSize::ExtraLarge).height();
        let total = slot_top(&s, s.len(), row_h) - SLOT_GAP;
        let (range, above, below) = window_of(&s, 500., 1400., row_h);
        let drawn: f32 = s[range.clone()]
            .iter()
            .map(|x| x.height(row_h))
            .sum::<f32>()
            + SLOT_GAP * range.len().saturating_sub(1) as f32
            + if range.start > 0 { SLOT_GAP } else { 0. };
        assert!((above + drawn + below - total).abs() < 0.01);
    }

    #[test]
    fn revealing_moves_the_least_or_not_at_all() {
        assert_eq!(reveal_scroll(100., 167., 0., 800.), None);
        assert_eq!(reveal_scroll(900., 967., 0., 800.), Some(167.));
        assert_eq!(reveal_scroll(50., 117., 400., 800.), Some(50.));
    }

    #[test]
    fn the_day_ends_at_the_next_midnight_in_its_own_zone() {
        let late = Utc.with_ymd_and_hms(2026, 11, 26, 23, 59, 30).unwrap();
        assert_eq!((next_day_start(&late) - late).num_seconds(), 30);
        let midnight = Utc.with_ymd_and_hms(2026, 11, 27, 0, 0, 0).unwrap();
        assert_eq!((next_day_start(&midnight) - midnight).num_hours(), 24);
        let zone = FixedOffset::east_opt(9 * 3600).unwrap();
        let tokyo = zone.with_ymd_and_hms(2026, 12, 31, 18, 0, 0).unwrap();
        assert_eq!(
            next_day_start(&tokyo),
            zone.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap()
        );
    }
}
