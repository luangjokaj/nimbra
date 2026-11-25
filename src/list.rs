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

    /// Several threads picked: what can be done to all of them.
    pub(super) fn bulk_pane(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.targets().len();
        let action = |id: &'static str, name: &'static str, label: &'static str| {
            button(t, ButtonKind::Secondary, label, None)
                .id(id)
                .press_soft()
                .child(icon(name, 14., t.text2))
        };
        let inbox_like = inbox_like(self.mailbox);
        div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(18.))
            .bg(t.surface)
            .child(serif(
                &format!("{count} conversations, selected"),
                44.,
                t.text,
            ))
            .child(meta(
                "⌘-click adds or removes one, Shift-click picks a range, Esc clears.",
                t.text3,
            ))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap(px(8.))
                    .max_w(px(620.))
                    .when(
                        inbox_like
                            || matches!(
                                self.mailbox,
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
                    )
                    .child(action("bulk-label", "tag", "Label").on_click(cx.listener(
                        |this, _, window, cx| this.toggle_menu(Menu::Labels, window, cx),
                    )))
                    .child(
                        action("bulk-move", "folder", "Move to").on_click(cx.listener(
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
        let dy = f32::from(event.delta.pixel_delta(px(18.)).y);
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
                .gap(px(8.))
                .h(px(height.max(0.)))
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
                    move |strip, _| strip.h(px(settle(from, to, start))),
                )
                .into_any_element(),
        )
    }

    /// The message list in a wrapper whose width slides open or shut after a toggle, the
    /// same way the sidebar does. Once it has finished closing it is not drawn at all.
    pub(super) fn list_slot(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
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
                    .w(px(width))
                    .h_full()
                    .child(self.list(t, cx)),
            );
        if !moving {
            return Some(slot.w(px(width)).into_any_element());
        }
        let (from, to) = if self.list_open { (0., 1.) } else { (1., 0.) };
        Some(
            slot.with_animation(
                ("list-width", self.list_gen),
                Animation::new(SLIDE).with_easing(panel_ease),
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
        self.list_toggled = Some(std::time::Instant::now());
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
                .rounded(px(radius))
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
        let line = ((self.list_width - 104.) / 7.4).max(8.) as usize;
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
                self.thread_labels(m)
                    .into_iter()
                    .next()
                    .map(|(index, label)| (short(&label.name, 18), accents[index % accents.len()]))
            })
            .filter(|_| line >= 36);
        let tag_chars = tag
            .as_ref()
            .map_or(0, |(label, _)| label.chars().count() + 6);
        let extras =
            13 + tag_chars + if m.thread.is_some() { 3 } else { 0 } + if m.starred { 3 } else { 0 };
        let sender_max = line.saturating_sub(extras).max(6);
        let subject = short(m.subject.split(',').next().unwrap_or(&m.subject), line);
        let snippet_max = line.saturating_sub(subject.chars().count() + 2);
        let snippet = if snippet_max >= 8 {
            short(&m.snippet, snippet_max)
        } else {
            SharedString::default()
        };

        let (key, account) = self.thread_key(m);
        div()
            .id(SharedString::from(format!("row:{account}:{key}")))
            .press(t)
            .relative()
            .when(self.pick_end.unwrap_or(self.selected) == i, |d| {
                d.child(reveal(
                    self.list_scroll.clone(),
                    self.reveal_row.clone(),
                    0.,
                ))
            })
            .flex()
            .gap(px(ROW_PAD))
            .px(px(ROW_PAD))
            .py(px(12.))
            .rounded(px(12.))
            .cursor_pointer()
            .when(selected || picked, |d| d.bg(t.selected))
            // Drawn over the row rather than as its border, which would take up space and
            // make the row (and those below it) jump.
            .when(picked, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(px(12.))
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
                        .left(px((ROW_PAD - 6.) / 2.))
                        // Level with the avatar's middle: the row's padding (12), then half
                        // of its two lines and their gap (22 + 3 + 18).
                        .top(px(12. + 43. / 2. - 3.))
                        .size(px(6.))
                        .rounded_full()
                        .bg(t.violet),
                )
            })
            // Centred on the two lines of text beside it.
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .child(avatar(m.tone, &m.initials, 28.)),
            )
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
                            .gap(px(8.))
                            .h(px(22.))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .min_w(px(0.))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .child(short(&m.sender, sender_max)),
                            )
                            .children(m.thread.clone().map(|n| mono(n, t.text3)))
                            .when(m.starred, |d| d.child(icon("star", 13., t.amber)))
                            .children(tag.map(|(label, a)| pill(t, &label, t.accent(a))))
                            .child(div().flex_1())
                            // Snoozed: when it comes back, in place of when it came (and
                            // beside the quick actions on the open row).
                            .children(snoozed.as_ref().map(|(name, when)| {
                                super::threads::snooze_chip(t, name, when.clone())
                            }))
                            .when(!selected && snoozed.is_none(), |d| {
                                d.child(mono(m.time.clone(), t.text3))
                            })
                            // Quick actions on the open row: done, snooze, remind me.
                            .when(selected, |d| {
                                let quick = |id: &'static str, name: &'static str| {
                                    let (hover, bright) = (t.hover, t.text);
                                    div()
                                        .id((id, i))
                                        .group("quick")
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(px(24.))
                                        .rounded(px(6.))
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
                                d.child(
                                    div()
                                        .flex()
                                        .gap(px(2.))
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
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .child(
                                // Long subjects shrink with an ellipsis instead of
                                // spilling out of the row; the snippet takes what is left.
                                div()
                                    .min_w(px(0.))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(if selected { t.text } else { t.text2 })
                                    .child(subject),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .text_color(t.text3)
                                    .child(snippet),
                            ),
                    ),
            )
    }

    fn list(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = self.visible();
        let unread = visible.iter().filter(|i| self.mails[**i].unread).count();
        let group = |label: &str| {
            div()
                .pl(px(14.))
                .pt(px(14.))
                .pb(px(6.))
                .child(overline(label.to_string(), t.faint))
        };
        let today: Vec<usize> = visible
            .iter()
            .copied()
            .filter(|i| !self.mails[*i].yesterday)
            .collect();
        let earlier: Vec<usize> = visible
            .iter()
            .copied()
            .filter(|i| self.mails[*i].yesterday)
            .collect();

        // Each option has a fixed width, so the highlight behind them can slide from one to
        // the other (and nothing shifts when it does).
        let unread_text = unread.to_string();
        let widths = [44., 70. + 7. * unread_text.len() as f32];
        // The switch: its options, the 2pt between them, its padding and its border.
        let mut switch_w = widths[0] + widths[1] + 2. + 8.;
        if self.is_demo() {
            switch_w += 2. + 112.;
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
        let segment = |label: &str, count: Option<(&str, Hsla)>, active: bool, width: f32| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .w(px(width))
                .h(px(SEGMENT_H))
                .rounded(px(7.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if active { t.text } else { t.text3 })
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
        let pill = div()
            .absolute()
            .top(px(3.))
            .h(px(SEGMENT_H))
            .rounded(px(7.))
            .bg(t.raised)
            .border_1()
            .border_color(t.border)
            .shadow(vec![shadow(
                c(0x000000, if t.dark { 0.25 } else { 0.08 }),
                1.,
                3.,
                0.,
            )]);
        // Only right after a switch (not again each time the list is drawn anew).
        let switching = self
            .filter_switched
            .is_some_and(|at| at.elapsed() < Duration::from_millis(260));
        let pill = if self.filter_gen == 0 || !switching {
            pill.left(px(3. + to_x)).w(px(to_w)).into_any_element()
        } else {
            pill.with_animation(
                ("filter-pill", self.filter_gen),
                Animation::new(Duration::from_millis(260)).with_easing(ease_out_quint()),
                move |d, delta| {
                    d.left(px(3. + from_x + (to_x - from_x) * delta))
                        .w(px(from_w + (to_w - from_w) * delta))
                },
            )
            .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .flex_none()
            .relative()
            .w(px(self.list_width))
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
                    .right(px(-3.))
                    .w(px(7.))
                    .cursor(gpui::CursorStyle::ResizeLeftRight)
                    .hover(|s| s.bg(gpui::white().opacity(0.06)))
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
                    ),
            )
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
                            .rounded(px(10.))
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
                                segment(
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
                                d.child(segment("Needs reply", Some(("2", t.rose)), false, 112.))
                            }),
                    )
                    .child(div().flex_1().min_w(px(0.)).mx(px(-HEADER_GAP / 2.)))
                    .when(self.offline, |d| d.child(self.offline_pill(t, fit.words)))
                    .child(
                        match fit.compose {
                            ComposeLook::Keyed => {
                                button(t, ButtonKind::Secondary, "Compose", Some("C"))
                            }
                            ComposeLook::Plain => button(t, ButtonKind::Secondary, "Compose", None),
                            ComposeLook::Icon => icon_button(t, "draft")
                                .size(px(34.))
                                .bg(t.hover)
                                .border_1()
                                .border_color(t.border_strong),
                        }
                        .id("compose")
                        .h(px(34.))
                        .press_soft()
                        .tooltip(tip(t.dark, "New message", "C"))
                        .on_click(
                            cx.listener(|this, _, _, cx| this.open_compose(ComposeKind::New, cx)),
                        ),
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
                        d.child(
                            div()
                                .absolute()
                                .inset_0()
                                .bg(t.border)
                                .child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .h_full()
                                        .w(relative(0.45))
                                        .rounded_full()
                                        .bg(gradient(90., c(0x8b7cff, 0.), c(0xff7ab6, 1.)))
                                        .with_animation(
                                            "checking-mail",
                                            Animation::new(Duration::from_millis(1150))
                                                .repeat()
                                                .with_easing(gpui::ease_in_out),
                                            |bar, delta| bar.left(relative(-0.45 + 1.45 * delta)),
                                        ),
                                )
                                .map(|strip| match self.bar_fade {
                                    // Done: fade the whole strip out.
                                    Some((_, n)) => strip
                                        .with_animation(
                                            ("checking-mail-fade", n),
                                            Animation::new(Duration::from_millis(450))
                                                .with_easing(ease_out_quint()),
                                            |strip, delta| strip.opacity(1. - delta),
                                        )
                                        .into_any_element(),
                                    None => strip.into_any_element(),
                                }),
                        )
                    }),
            )
            .child(
                div()
                    .id("rows")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap(px(2.))
                    .px(px(10.))
                    .pb(px(10.))
                    .overflow_y_scroll()
                    .track_scroll(&self.list_scroll)
                    .on_scroll_wheel(cx.listener(|this, event, _, cx| this.pull_list(event, cx)))
                    .children(self.pull_indicator(t))
                    .when(!today.is_empty(), |d| d.child(group("TODAY")))
                    .children(today.into_iter().map(|i| self.row(t, i, cx)))
                    .when(!earlier.is_empty(), |d| d.child(group("EARLIER")))
                    .children(earlier.into_iter().map(|i| self.row(t, i, cx)))
                    // More is on its way: placeholder rows that breathe while the next
                    // page loads (it starts on its own as the end of the list comes near).
                    .when(self.more().is_some(), |d| {
                        d.children((0..2usize).map(|n| {
                            let bar = |w: f32| div().h(px(9.)).w(px(w)).rounded_full().bg(t.hover);
                            div()
                                .flex()
                                .flex_none()
                                .gap(px(ROW_PAD))
                                .px(px(ROW_PAD))
                                .py(px(14.))
                                .child(div().size(px(28.)).flex_none().rounded_full().bg(t.hover))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(8.))
                                        .pt(px(3.))
                                        .child(bar(if n == 0 { 120. } else { 96. }))
                                        .child(bar(if n == 0 { 220. } else { 180. })),
                                )
                                .with_animation(
                                    ("more-loading", n),
                                    Animation::new(Duration::from_millis(1400)).repeat(),
                                    move |row, delta| {
                                        let wave = (std::f32::consts::TAU
                                            * (delta - n as f32 * 0.15))
                                            .cos();
                                        row.opacity(0.55 + 0.35 * wave)
                                    },
                                )
                        }))
                    }),
            )
    }

    /// An empty mailbox other than the inbox (which gets the Inbox Zero screen).
    pub(super) fn empty_mailbox(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let search = self.mailbox == View::Search;
        // Not empty at all: filtered, or the rest could not be had.
        let filtered = self.unread_only && !self.all_cleared();
        let unlisted = match self.mailbox {
            View::AllInboxes => !self.all_inboxes_whole,
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
                match self.mailbox {
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
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(14.))
            .bg(t.surface)
            .child(serif(&title, 56., t.text))
            .child(
                div()
                    .text_size(px(15.))
                    .line_height(px(24.))
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

    /// The sky moves on while Inbox Zero is in front of you: a slow frame for the twinkling
    /// (it changes little from one to the next), every frame while a star falls. Still with
    /// Reduce Motion, while the window is in the background, and while it comes up.
    pub(super) fn move_sky(
        &mut self,
        zero_shown: bool,
        since_last: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !zero_shown {
            self.sky = None;
            return;
        }
        if self.arrival.is_some() || crate::system::reduces_motion() {
            return;
        }
        let sky = self.sky.get_or_insert_with(crate::sky::Sky::new);
        if !window.is_window_active() {
            sky.drop_fall();
            return;
        }
        sky.advance(since_last);
        if sky.falling() {
            window.request_animation_frame();
        } else if !self.sky_tick {
            self.sky_tick = true;
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                this.update(cx, |this, cx| {
                    this.sky_tick = false;
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    pub(super) fn zero(&self, t: &Theme) -> impl IntoElement {
        let ink = c(0xf5f3ef, 1.);
        let soft = c(0xffffff, 0.7);
        let glass = |d: Div| {
            d.bg(c(0xffffff, 0.07))
                .border_1()
                .border_color(c(0xffffff, 0.14))
                .rounded(px(14.))
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
                .child(
                    div()
                        .font_family(SERIF)
                        .text_size(px(40.))
                        .line_height(px(44.))
                        .text_color(ink)
                        .child(value.to_string()),
                )
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
                let (lit, sparkle) = self
                    .sky
                    .as_ref()
                    .map_or((1., 0.), |sky| (sky.twinkle(i), sky.sparkle(i)));
                let alpha = a * arrival.star(i);
                // A soft glow as it comes back to full, centred on the star.
                let halo = size * 3.;
                div()
                    .absolute()
                    .left(relative(x))
                    .top(relative(y))
                    .when(sparkle > 0., |d| {
                        d.child(
                            div()
                                .absolute()
                                .left(px((size - halo) / 2.))
                                .top(px((size - halo) / 2.))
                                .size(px(halo))
                                .rounded_full()
                                .bg(c(0xffffff, alpha * 0.18 * sparkle)),
                        )
                    })
                    .child(
                        div()
                            .absolute()
                            .size(px(size))
                            .rounded_full()
                            .bg(c(0xffffff, alpha * lit)),
                    )
            })
            .collect();
        let falling = self
            .sky
            .as_ref()
            .and_then(|sky| sky.fall())
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
            div()
                .absolute()
                .inset_0()
                .bg(fade(angle, color, alpha * shown, reach))
        };

        let below = if self.can_undo() {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(meta("Press", c(0xffffff, 0.5)))
                .child(kbd(&Theme::dark(), "Z"))
                .child(meta("to undo", c(0xffffff, 0.5)))
        } else {
            div()
                .w_full()
                .px(px(32.))
                .text_center()
                .font_family(SERIF)
                .italic()
                .text_size(px(24.))
                .line_height(px(30.))
                .text_color(soft)
                .child(crate::inspiration::line_for(
                    chrono::Local::now().date_naive(),
                ))
        };

        div().flex_1().min_w(px(0.)).h_full().bg(t.surface).child(
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
                        .child(div().h(px(18.)).flex_none())
                        .child(settle(serif("Inbox zero.", 120., ink), arrival.headline()))
                        .child(div().h(px(14.)).flex_none())
                        .child(
                            settle(div(), arrival.line())
                                .text_size(px(17.))
                                .line_height(px(26.))
                                .text_color(soft)
                                .child(if self.is_demo() {
                                    "Every important thread is handled. Go make something wonderful."
                                } else if self.mailbox == View::AllInboxes {
                                    "Your inboxes are empty. Go make something wonderful."
                                } else {
                                    "Your inbox is empty. Go make something wonderful."
                                }),
                        )
                        // Sample content: real mail shows the headline only.
                        .when(self.is_demo(), |d| {
                            d.child(
                                div()
                                    .flex()
                                    .mt(px(40.))
                                    .gap(px(12.))
                                    .child(stat(
                                        "done",
                                        t.mint,
                                        "threads handled",
                                        &(32 + self.history.len()).to_string(),
                                    ))
                                    .child(stat("clock", t.sky, "time in inbox", "38m"))
                                    .child(stat("zap", t.amber, "zero streak", "8 days")),
                            )
                        })
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
                                            .rounded(px(9.))
                                            .bg(Tone::Sky.gradient())
                                            .child(icon("calendar", 16., gpui::white())),
                                    )
                                    .child(
                                        div()
                                            .mr(px(40.))
                                            .child(
                                                div()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .child("Up next · Design crit"),
                                            )
                                            .child(meta(
                                                "2:00 PM · Studio B · in 42 min",
                                                c(0xffffff, 0.55),
                                            )),
                                    )
                                    .child(button(
                                        &Theme::dark(),
                                        ButtonKind::Secondary,
                                        "Join",
                                        Some("J"),
                                    )),
                            )
                        })
                        .child(settle(below, arrival.below())),
                ),
        )
    }
}

/// The space either side of a row's avatar, and at the row's right. With the list's own
/// 10pt margin it puts the avatars on `LIST_LEFT`.
pub(super) const ROW_PAD: f32 = 14.;

/// The list header's margins: the list's text starts and ends on them.
pub(super) const LIST_LEFT: f32 = 24.;

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
            ComposeLook::Keyed => 112.,
            ComposeLook::Plain => 88.,
            ComposeLook::Icon => 34.,
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

/// How far down Inbox Zero's horizon is.
pub(super) const HORIZON: f32 = 0.72;

/// The height of an option in the All / Unread switch.
pub(super) const SEGMENT_H: f32 = 26.;
