//! The sidebar: accounts, mailboxes and labels, the window controls above it, folding its
//! sections, and showing, hiding and floating it.

use super::*;

impl Nimbra {
    fn section_open(&self, key: &str) -> bool {
        !self.settings.collapsed.iter().any(|k| k == key)
    }

    /// A sidebar row inside a section that can fold: shown, hidden, or (while the section
    /// folds or unfolds) growing or shrinking to and from nothing as it fades. The open
    /// mailbox always stays.
    fn fold_row(
        &self,
        key: &'static str,
        keep: bool,
        id: usize,
        row: AnyElement,
    ) -> Option<AnyElement> {
        let open = self.section_open(key);
        let moving = self
            .folding
            .get(key)
            .filter(|(at, _)| at.elapsed() < FOLD)
            .map(|(_, n)| *n);
        if keep || (open && moving.is_none()) {
            return Some(row);
        }
        let n = moving?;
        Some(
            div()
                .flex_none()
                .overflow_hidden()
                .child(row)
                .with_animation(
                    ("fold-row", n * 1000 + id),
                    Animation::new(FOLD).with_easing(ease_out_quint()),
                    move |d, delta| {
                        let shown = if open { delta } else { 1. - delta };
                        // The text fades well before the row gets thin, so no slivers show.
                        d.h(px(NAV_ROW * shown)).opacity(shown.powi(3))
                    },
                )
                .into_any_element(),
        )
    }

    /// A sidebar section title that folds its section away and back (remembered).
    fn section_toggle(
        &self,
        t: &Theme,
        key: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let open = self.section_open(key);
        let bright = t.text2;
        div()
            .id(("section", view_key_for_section(key)))
            .group("section")
            .flex()
            .items_center()
            .gap(px(4.))
            .pl(px(10.))
            .pt(px(18.))
            .pb(px(8.))
            .cursor_pointer()
            .child(overline(label, t.text3).group_hover("section", move |s| s.text_color(bright)))
            .child(
                icon(if open { "down" } else { "next" }, 11., t.text3)
                    .group_hover("section", move |s| s.text_color(bright)),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.section_open(key) {
                    this.settings.collapsed.push(key.to_string());
                } else {
                    this.settings.collapsed.retain(|k| k != key);
                }
                this.fold_gen += 1;
                this.folding
                    .insert(key, (std::time::Instant::now(), this.fold_gen));
                this.save_settings(cx);
                // Once folded, the rows leave the layout.
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(FOLD + Duration::from_millis(20))
                        .await;
                    this.update(cx, |this, cx| {
                        this.folding.retain(|_, (at, _)| at.elapsed() < FOLD);
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
                cx.notify();
            }))
    }

    /// What the sidebar shows now, for the keyboard.
    pub(super) fn sidebar_now(&self) -> keyboard::Sidebar<'_> {
        keyboard::Sidebar {
            accounts: self
                .settings
                .accounts
                .iter()
                .map(|a| a.id.clone())
                .collect(),
            shown: &self.account_id,
            mailbox: self.mailbox,
            demo: self.is_demo(),
            labels: self.labels.len(),
            collapsed: &self.settings.collapsed,
        }
    }

    /// ←: the keys go to the sidebar, its cursor on what is open. A hidden sidebar (or
    /// full view) comes in for as long as the keys are there.
    pub(super) fn focus_sidebar(&mut self, cx: &mut Context<Self>) {
        if self.pane == Pane::Sidebar
            || self.window_covered()
            || self.shortcuts
            || matches!(self.status, Status::SignedOut | Status::SigningIn)
        {
            return;
        }
        if self.focus_restore.is_some() {
            self.toggle_full_view(cx);
        }
        if !self.sidebar_open {
            self.slide_sidebar(cx);
            self.sidebar_for_keys = true;
        }
        let sidebar = self.sidebar_now();
        let here = sidebar.here();
        self.sidebar_cursor = sidebar.rows().iter().position(|r| *r == here).unwrap_or(0);
        self.pane = Pane::Sidebar;
        self.reveal_cursor.set(true);
        cx.notify();
    }

    /// The keys back on the list; a sidebar that came in for them goes again.
    pub(super) fn leave_sidebar(&mut self, cx: &mut Context<Self>) {
        if self.pane != Pane::Sidebar {
            return;
        }
        self.pane = Pane::List;
        if std::mem::take(&mut self.sidebar_for_keys) && self.sidebar_open {
            self.slide_sidebar(cx);
        }
        cx.notify();
    }

    /// → or Return in the sidebar: what the row under the cursor stands for, as a click.
    pub(super) fn open_sidebar_row(&mut self, cx: &mut Context<Self>) {
        let rows = self.sidebar_now().rows();
        let Some(row) = rows.get(self.sidebar_cursor.min(rows.len().saturating_sub(1))) else {
            return;
        };
        let row = row.clone();
        self.leave_sidebar(cx);
        self.go_to_row(row, cx);
    }

    pub(super) fn go_to_row(&mut self, row: SidebarRow, cx: &mut Context<Self>) {
        match row {
            SidebarRow::Search => self.focus_search(cx),
            SidebarRow::AllInboxes => self.select_mailbox(View::AllInboxes, cx),
            SidebarRow::Mailbox(view) => self.select_mailbox(view, cx),
            SidebarRow::Account(id) if id == self.account_id => {
                self.select_mailbox(View::Inbox, cx)
            }
            SidebarRow::Account(id) => self.switch_account(id, cx),
            SidebarRow::Settings => self.open_settings(cx),
        }
    }

    /// ⌘B, the toggle and the menu: the choice is kept for the next launch. Full view hides
    /// the sidebar only for as long as it lasts, so it calls `slide_sidebar` instead.
    pub(super) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.slide_sidebar(cx);
        self.sidebar_for_keys = false;
        if !self.sidebar_open {
            self.pane = Pane::List;
        }
        self.settings.sidebar_auto_hide = !self.sidebar_open;
        self.save_settings(cx);
    }

    pub(super) fn slide_sidebar(&mut self, cx: &mut Context<Self>) {
        // Opened while it floats: it docks from where it is. Shut: away until the edge is
        // reached again, though the pointer is still on it.
        self.docked_from = self
            .sidebar_place(0.)
            .filter(|place| !self.sidebar_open && place.inset > 0.)
            .map(|place| place.x);
        self.peek.reset();
        self.sidebar_open = !self.sidebar_open;
        self.sidebar_toggled = Some(std::time::Instant::now());
        if self.sidebar_open {
            self.focus_restore = None;
        }
        cx.notify();
    }

    pub(super) fn sidebar_sliding(&self) -> bool {
        self.sidebar_toggled
            .is_some_and(|at| at.elapsed().as_secs_f32() < PANEL_SLIDE_SECONDS)
    }

    /// Where the sidebar is drawn `ahead` seconds from now: docked, sliding, floating over
    /// the content, or (`None`) nowhere.
    pub(super) fn sidebar_place(&self, ahead: f32) -> Option<SidebarPlace> {
        let sliding = self.sidebar_sliding();
        if self.sidebar_open || sliding {
            if sliding
                && self.sidebar_open
                && let (Some(from), Some(at)) = (self.docked_from, self.sidebar_toggled)
            {
                let left =
                    1. - panel_ease((at.elapsed().as_secs_f32() + ahead) / PANEL_SLIDE_SECONDS);
                return Some(SidebarPlace {
                    x: from * left,
                    inset: FLOAT_INSET * left,
                });
            }
            let width = self.sidebar_width(ahead);
            return (width > 0.).then_some(SidebarPlace {
                x: width - SIDEBAR_W,
                inset: 0.,
            });
        }
        let shown = self.peek.progress(ahead);
        (shown > 0.).then_some(SidebarPlace {
            x: FLOAT_INSET - (SIDEBAR_W + 2. * FLOAT_INSET) * (1. - shown),
            inset: FLOAT_INSET,
        })
    }

    /// The room the panes leave for the sidebar, following its slide.
    pub(super) fn sidebar_width(&self, ahead: f32) -> f32 {
        let to = if self.sidebar_open { 1. } else { 0. };
        match self
            .sidebar_toggled
            .map(|at| at.elapsed().as_secs_f32() + ahead)
        {
            Some(elapsed) if elapsed < PANEL_SLIDE_SECONDS => {
                let from = 1. - to;
                SIDEBAR_W * (from + (to - from) * panel_ease(elapsed / PANEL_SLIDE_SECONDS))
            }
            _ => SIDEBAR_W * to,
        }
    }

    /// Something is drawn over the whole window, so the hidden sidebar stays away.
    pub(super) fn window_covered(&self) -> bool {
        self.palette
            || self.finder_open
            || self.new_label.is_some()
            || self.confirm.is_some()
            || (self.compose.is_some() && self.compose_host.is_none())
    }

    /// While the sidebar is hidden, follows the pointer to bring it in and send it away.
    pub(super) fn watch_edge(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.sidebar_open || self.watching_edge {
            return;
        }
        self.watching_edge = true;
        cx.spawn_in(window, async move |this, cx| {
            let mut wait = Duration::from_millis(16);
            loop {
                cx.background_executor().timer(wait).await;
                let Ok(next) = this.update_in(cx, |this, window, cx| this.look_at_edge(window, cx))
                else {
                    break;
                };
                match next {
                    Some(next) => wait = next,
                    None => break,
                }
            }
        })
        .detach();
    }

    /// One look at the pointer. Returns when to take the next, or `None` when done.
    fn look_at_edge(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<Duration> {
        if self.sidebar_open {
            self.watching_edge = false;
            return None;
        }
        // An open menu is a window of its own, which takes the focus from this one.
        let hold = self.menu_window.is_some();
        let right = FLOAT_INSET + SIDEBAR_W;
        let pointer = if self.sidebar_sliding() || self.window_covered() {
            None
        } else {
            crate::autohide::pointer(window, right)
        };
        if self
            .peek
            .look(pointer, right, hold && !self.window_covered())
        {
            cx.notify();
        }
        // Nothing can happen while inactive except becoming active: look less often.
        Some(Duration::from_millis(if window.is_window_active() {
            16
        } else {
            250
        }))
    }

    /// Sidebar toggle, back, forward and reload, sitting beside the traffic lights.
    /// Drawn above the panes so it stays put when the sidebar is hidden.
    pub(super) fn window_controls(
        &self,
        t: &Theme,
        x: f32,
        list_shown: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let hover = t.hover;
        let control = |name: &str, enabled: bool| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(28.))
                .rounded(px(7.))
                .when(enabled, |d| d.cursor_pointer().hover(move |s| s.bg(hover)))
                .child(icon(name, 17., if enabled { t.text2 } else { t.faint }))
        };
        div()
            .absolute()
            .top(px(11.))
            .left(px(x + 84.))
            .flex()
            .items_center()
            .gap(px(4.))
            .child(
                control("sidebar", true)
                    .id("toggle-sidebar")
                    .press(t)
                    // Highlighted while the sidebar is hidden, like the list button.
                    .when(!self.sidebar_open, |d| d.bg(t.selected))
                    .tooltip(tip(t.dark, "Show or hide sidebar", "⌘B"))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            )
            .child(div().w(px(6.)))
            .child(
                control("back", self.can_go_back())
                    .id("go-back")
                    .press(t)
                    .tooltip(tip(t.dark, "Back", "⌘["))
                    .on_click(cx.listener(|this, _, _, cx| this.go_back(cx))),
            )
            .child(
                control("next", self.can_go_forward())
                    .id("go-forward")
                    .press(t)
                    .tooltip(tip(t.dark, "Forward", "⌘]"))
                    .on_click(cx.listener(|this, _, _, cx| this.go_forward(cx))),
            )
            .child(div().w(px(6.)))
            .child(
                div()
                    .id("reload")
                    .tooltip(tip(
                        t.dark,
                        if self.offline {
                            self.offline_label().into()
                        } else {
                            SharedString::from("Check for mail")
                        },
                        "⌘R",
                    ))
                    .relative()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(28.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx)))
                    .child(if self.shows_checking() {
                        // Keeps turning for as long as a mailbox request is in flight.
                        icon("reload", 17., t.violet)
                            .with_animation(
                                "reload-loop",
                                Animation::new(Duration::from_millis(900)).repeat(),
                                |svg, delta| {
                                    svg.with_transformation(Transformation::rotate(percentage(
                                        delta,
                                    )))
                                },
                            )
                            .into_any_element()
                    } else if self.reload_gen == 0 {
                        icon("reload", 17., t.text2).into_any_element()
                    } else {
                        // One full turn each time mail is checked.
                        icon("reload", 17., t.text2)
                            .with_animation(
                                ("reload-spin", self.reload_gen),
                                Animation::new(Duration::from_millis(700))
                                    .with_easing(ease_out_quint()),
                                |svg, delta| {
                                    svg.with_transformation(Transformation::rotate(percentage(
                                        delta,
                                    )))
                                },
                            )
                            .into_any_element()
                    })
                    // The list's header has the Offline pill; without a list, a dot here.
                    .when(self.offline && !list_shown, |d| {
                        d.child(
                            div()
                                .absolute()
                                .top(px(3.))
                                .right(px(3.))
                                .size(px(6.))
                                .rounded_full()
                                .bg(t.amber),
                        )
                    }),
            )
    }

    /// "Offline", and how much is waiting to go out.
    pub(super) fn offline_label(&self) -> String {
        match self.outbox.changes.len() + self.waiting_mail().count() {
            0 => "Offline".to_string(),
            waiting => format!("Offline · {waiting} waiting"),
        }
    }

    /// Without room for the words it is only the dot, and the tooltip says the rest.
    pub(super) fn offline_pill(&self, t: &Theme, words: bool) -> impl IntoElement {
        div()
            .id("offline")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .py(px(2.))
            .rounded_full()
            .bg(t.amber.opacity(0.14))
            .border_1()
            .border_color(t.amber.opacity(0.4))
            .h(px(22.))
            .when(!words, |d| d.w(px(22.)).justify_center())
            .tooltip(tip(
                t.dark,
                if words {
                    SharedString::from("Changes are kept and sent when the connection is back")
                } else {
                    self.offline_label().into()
                },
                "",
            ))
            .child(div().size(px(6.)).flex_none().rounded_full().bg(t.amber))
            .when(words, |d| d.child(meta(self.offline_label(), t.text2)))
    }

    pub(super) fn sidebar(
        &self,
        t: &Theme,
        floating: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let cursor = (self.pane == Pane::Sidebar)
            .then(|| {
                let rows = self.sidebar_now().rows();
                rows.get(self.sidebar_cursor.min(rows.len().saturating_sub(1)))
                    .cloned()
            })
            .flatten();
        let left = self.visible().len();
        let unread = self
            .visible()
            .iter()
            .filter(|i| self.mails[**i].unread)
            .count();
        let bars = [9., 14., 8., 17., 12., 20., 15.];
        // The other mailboxes are not connected yet, so their counts only show in the demo.
        let demo = self.is_demo();
        let fake = |n: &str| demo.then(|| n.to_string());
        // Real counts from Gmail: unread in the inbox for mailboxes and labels (archived
        // "Done" threads left out), total for Drafts.
        let unread_in = |id: &str| {
            self.counts
                .get(id)
                .map(|(unread, _)| *unread)
                .filter(|n| *n > 0)
                .map(|n| n.to_string())
        };
        let drafts = self
            .counts
            .get("DRAFT")
            .map(|(_, total)| *total)
            .filter(|n| *n > 0)
            .map(|n| n.to_string());
        let split_toggle = self.section_toggle(t, "split", "SPLIT INBOX", cx);
        // Several accounts: All inboxes, and each account on its own.
        let many = !demo && self.settings.accounts.len() >= 2;
        let unread_of = |id: &str| -> u32 {
            if id == self.account_id {
                self.counts.get("INBOX").map_or(0, |(unread, _)| *unread)
            } else {
                self.account_unread.get(id).copied().unwrap_or(0)
            }
        };
        let all_unread: u32 = self
            .settings
            .accounts
            .iter()
            .map(|a| unread_of(&a.id))
            .sum();
        let accounts_toggle = many.then(|| self.section_toggle(t, "accounts", "ACCOUNTS", cx));
        let account_rows: Vec<AnyElement> = if many {
            self.settings
                .accounts
                .iter()
                .enumerate()
                .filter_map(|(i, account)| {
                    let shown = account.id == self.account_id && self.mailbox != View::AllInboxes;
                    let id = account.id.clone();
                    let initial = account
                        .email
                        .chars()
                        .next()
                        .map(|c| c.to_uppercase().to_string())
                        .unwrap_or_default();
                    let count = Some(unread_of(&account.id))
                        .filter(|n| *n > 0)
                        .map(|n| n.to_string());
                    let hover = t.hover;
                    let row = div()
                        .id(("account-row", i))
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(10.))
                        .px(px(10.))
                        .h(px(30.))
                        .rounded(px(NAV_RADIUS))
                        .cursor_pointer()
                        .when(shown, |d| d.relative().child(active_pill(t)))
                        .when(!shown, |d| d.hover(move |s| s.bg(hover)))
                        .press(t)
                        .child(match self.photos.get(&account.id) {
                            Some(photo) => me_avatar(Some(photo), &initial, 18.),
                            None => avatar(crate::data::tone_for(&account.email), &initial, 18.)
                                .into_any_element(),
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .text_color(if shown { t.text } else { t.text2 })
                                .child(account.email.clone()),
                        )
                        .children(count.map(|n| meta(n, if shown { t.text } else { t.text3 })))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if id == this.account_id {
                                this.select_mailbox(View::Inbox, cx);
                            } else {
                                this.switch_account(id.clone(), cx);
                            }
                        }));
                    let row = self
                        .cursor_on(
                            row,
                            t,
                            &cursor,
                            SidebarRow::Account(account.id.clone()),
                            shown,
                            NAV_RADIUS,
                        )
                        .into_any_element();
                    self.fold_row("accounts", shown, 950 + i, row)
                })
                .collect()
        } else {
            Vec::new()
        };
        let mailbox_toggle = self.section_toggle(t, "mailbox", "MAILBOX", cx);
        let labels_toggle = self.section_toggle(t, "labels", "LABELS", cx);
        let mailbox = |view: View, icon_name: &'static str, count: Option<String>| {
            // A folded section still shows the mailbox that is open.
            let section = match view {
                View::Inbox | View::Primary | View::Updates | View::Promotions | View::Social => {
                    "split"
                }
                View::Label(_) => "labels",
                _ => "mailbox",
            };
            let row = nav(
                t,
                icon_name,
                &self.title(view),
                count,
                self.mailbox == view,
                None,
            )
            .id(("mailbox", view_key(view)))
            .press(t)
            .on_click(cx.listener(move |this, _, _, cx| this.select_mailbox(view, cx)));
            let row = self
                .cursor_on(
                    row,
                    t,
                    &cursor,
                    SidebarRow::Mailbox(view),
                    self.mailbox == view,
                    NAV_RADIUS,
                )
                .into_any_element();
            self.fold_row(section, self.mailbox == view, view_key(view), row)
        };

        let scrolled =
            div()
                .id("sidebar")
                .track_scroll(&self.sidebar_scroll)
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                // With several accounts it can be taller than the window: it scrolls.
                .overflow_y_scroll()
                .px(px(12.))
                .pb(px(10.))
                .child(
                    div()
                        .h(px(SIDEBAR_CLEAR))
                        .flex_none()
                        .on_mouse_down(MouseButton::Left, drag_or_zoom),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .pl(px(10.))
                        .pr(px(6.))
                        .py(px(6.))
                        .rounded(px(9.))
                        .bg(t.hover)
                        .border_1()
                        .border_color(t.border)
                        .cursor_pointer()
                        .id("jump")
                        .cursor(gpui::CursorStyle::IBeam)
                        .on_click(cx.listener(|this, _, _, cx| this.focus_search(cx)))
                        .when(self.mailbox == View::Search, |d| d.border_color(t.mint))
                        .when(cursor == Some(SidebarRow::Search), |d| {
                            d.border_color(t.mint).relative().child(reveal(
                                self.sidebar_scroll.clone(),
                                self.reveal_cursor.clone(),
                                SIDEBAR_CLEAR,
                            ))
                        })
                        .child(icon("search", 16., t.text3))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_color(if self.query.is_empty() {
                                    t.text3
                                } else {
                                    t.text
                                })
                                .child(if self.query.is_empty() {
                                    SharedString::from("Search mail")
                                } else {
                                    short(&self.query, 24)
                                }),
                        )
                        .child(kbd(t, "/")),
                )
                .when(many, |d| {
                    d.child(
                        nav(
                            t,
                            "mail",
                            "All inboxes",
                            Some(all_unread).filter(|n| *n > 0).map(|n| n.to_string()),
                            self.mailbox == View::AllInboxes,
                            None,
                        )
                        .mt(px(SEARCH_GAP))
                        .id("all-inboxes")
                        .press(t)
                        .on_click(
                            cx.listener(|this, _, _, cx| this.select_mailbox(View::AllInboxes, cx)),
                        )
                        .map(|row| {
                            self.cursor_on(
                                row,
                                t,
                                &cursor,
                                SidebarRow::AllInboxes,
                                self.mailbox == View::AllInboxes,
                                NAV_RADIUS,
                            )
                        }),
                    )
                    .children(accounts_toggle)
                    .children(account_rows)
                })
                .child(split_toggle)
                .children(mailbox(
                    View::Inbox,
                    "inbox",
                    if demo {
                        (unread > 0 && self.mailbox == View::Inbox).then(|| unread.to_string())
                    } else {
                        unread_in("INBOX")
                    },
                ))
                .when(demo, |d| {
                    d.children(self.fold_row(
                        "split",
                        false,
                        910,
                        nav(t, "users", "Team", fake("4"), false, None).into_any_element(),
                    ))
                    .children(self.fold_row(
                        "split",
                        false,
                        911,
                        nav(t, "calendar", "Calendar", fake("2"), false, None).into_any_element(),
                    ))
                    .children(self.fold_row(
                        "split",
                        false,
                        912,
                        nav(t, "news", "Newsletters", fake("31"), false, None).into_any_element(),
                    ))
                    .children(self.fold_row(
                        "split",
                        false,
                        913,
                        nav(t, "zap", "Other", fake("23"), false, None).into_any_element(),
                    ))
                })
                // With real mail the split inbox is Gmail's own categories.
                .when(!demo, |d| {
                    d.children(mailbox(
                        View::Primary,
                        "users",
                        unread_in("CATEGORY_PERSONAL"),
                    ))
                    .children(mailbox(
                        View::Updates,
                        "bell",
                        unread_in("CATEGORY_UPDATES"),
                    ))
                    .children(mailbox(
                        View::Promotions,
                        "tag",
                        unread_in("CATEGORY_PROMOTIONS"),
                    ))
                    .children(mailbox(
                        View::Social,
                        "news",
                        unread_in("CATEGORY_SOCIAL"),
                    ))
                })
                .child(mailbox_toggle)
                .children(mailbox(View::Starred, "star", None))
                .when(!demo, |d| {
                    d.children(mailbox(View::Important, "pulse", None))
                })
                .children(mailbox(View::Snoozed, "clock", fake("3")))
                .children(self.fold_row(
                    "mailbox",
                    false,
                    900,
                    nav(t, "bell", "Reminders", fake("2"), false, None).into_any_element(),
                ))
                .children(mailbox(
                    View::Drafts,
                    "draft",
                    if demo { fake("1") } else { drafts },
                ))
                .children(mailbox(View::Sent, "send", None))
                .children(mailbox(
                    View::Done,
                    "done",
                    (!self.history.is_empty()).then(|| self.history.len().to_string()),
                ))
                .when(!demo, |d| {
                    d.children(mailbox(View::AllMail, "archive", None))
                        .children(mailbox(View::Spam, "ban", None))
                        .children(mailbox(View::Trash, "trash", None))
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(labels_toggle.flex_1())
                        .when(!demo && self.gmail.is_some(), |d| {
                            let hover = t.hover;
                            d.child(
                                div()
                                    .id("new-label-sidebar")
                                    .mt(px(10.))
                                    .mr(px(4.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(22.))
                                    .rounded(px(6.))
                                    .cursor_pointer()
                                    .text_color(t.text3)
                                    .hover(move |s| s.bg(hover))
                                    .tooltip(tip(t.dark, "New label", ""))
                                    .child("+")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_label_sheet(LabelSheet::New { apply: false }, cx)
                                    })),
                            )
                        }),
                )
                .when(demo, |d| {
                    d.children(self.fold_row(
                        "labels",
                        false,
                        920,
                        nav(t, "tag", "Product", None, false, Some(t.violet)).into_any_element(),
                    ))
                    .children(self.fold_row(
                        "labels",
                        false,
                        921,
                        nav(t, "tag", "Design", None, false, Some(t.rose)).into_any_element(),
                    ))
                    .children(self.fold_row(
                        "labels",
                        false,
                        922,
                        nav(t, "tag", "Finance", None, false, Some(t.amber)).into_any_element(),
                    ))
                    .children(self.fold_row(
                        "labels",
                        false,
                        923,
                        nav(t, "tag", "Travel", None, false, Some(t.sky)).into_any_element(),
                    ))
                })
                .when(!demo, |d| {
                    let tints = [t.violet, t.rose, t.amber, t.sky, t.mint];
                    d.children(self.labels.iter().enumerate().take(40).filter_map(
                        |(index, label)| {
                            let view = View::Label(index);
                            let row = nav(
                                t,
                                "tag",
                                &short(&label.name, 22),
                                unread_in(&label.id),
                                self.mailbox == view,
                                Some(tints[index % tints.len()]),
                            )
                            .id(("mailbox", view_key(view)))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.select_mailbox(view, cx)),
                            )
                            // Right-click: rename or delete the label.
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(
                                    move |this, event: &gpui::MouseDownEvent, window, cx| {
                                        this.menu_at = (
                                            f32::from(event.position.x),
                                            f32::from(event.position.y),
                                        );
                                        this.menu_dismissed = None;
                                        this.toggle_menu(Menu::Label(index), window, cx);
                                    },
                                ),
                            )
                            .press(t);
                            let row = self
                                .cursor_on(
                                    row,
                                    t,
                                    &cursor,
                                    SidebarRow::Mailbox(view),
                                    self.mailbox == view,
                                    NAV_RADIUS,
                                )
                                .into_any_element();
                            self.fold_row("labels", self.mailbox == view, view_key(view), row)
                        },
                    ))
                })
                .child(div().flex_1())
                // The streak card is sample content, so it only shows with the demo data.
                .when(demo, |d| {
                    d.child(
                        card(t)
                            .flex()
                            .flex_col()
                            .gap(px(10.))
                            .p(px(12.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(icon("zap", 16., t.amber))
                                    .child(
                                        div()
                                            .flex_1()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child("Inbox zero streak"),
                                    )
                                    .child(mono("7d", t.text3)),
                            )
                            .child(div().flex().items_end().gap(px(5.)).h(px(22.)).children(
                                bars.map(|h: f32| {
                                    div().flex_1().h(px(h)).rounded(px(2.)).bg(gradient(
                                        180.,
                                        c(0xff7ab6, 1.),
                                        c(0x8b7cff, 1.),
                                    ))
                                }),
                            ))
                            .child(meta(
                                if left == 0 {
                                    "Done at 10:52 · 28 min faster".to_string()
                                } else {
                                    format!("{left} left · usually done by 11:20")
                                },
                                t.text3,
                            )),
                    )
                });

        // Settings stays at the bottom, whatever is scrolled above it.
        let footer = div()
            .flex_none()
            .px(px(12.))
            .py(px(8.))
            .border_t_1()
            .border_color(t.border_strong)
            .child(
                self.cursor_on(div(), t, &cursor, SidebarRow::Settings, false, 10.)
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .p(px(6.))
                    .rounded(px(10.))
                    .child(
                        div()
                            .id("account")
                            .flex()
                            .flex_1()
                            .min_w(px(0.))
                            .items_center()
                            .gap(px(10.))
                            .cursor_pointer()
                            .tooltip(tip(t.dark, "Settings", "⌘,"))
                            .on_click(cx.listener(|this, _, _, cx| this.open_settings(cx)))
                            .child({
                                // With several accounts, the default one's profile.
                                let default = (!demo && self.settings.accounts.len() >= 2)
                                    .then(|| self.settings.default_account())
                                    .flatten()
                                    .filter(|id| *id != self.account_id);
                                match default {
                                    Some(id) => {
                                        let initial = self
                                            .settings
                                            .default_email()
                                            .and_then(|e| e.chars().next())
                                            .map(|c| c.to_uppercase().to_string())
                                            .unwrap_or_default();
                                        match self.photos.get(id) {
                                            Some(photo) => me_avatar(Some(photo), &initial, 28.),
                                            None => avatar(
                                                crate::data::tone_for(
                                                    self.settings.default_email().unwrap_or(""),
                                                ),
                                                &initial,
                                                28.,
                                            )
                                            .into_any_element(),
                                        }
                                    }
                                    None => me_avatar(
                                        self.photos.get(&self.account_id),
                                        &self.me_initials,
                                        28.,
                                    ),
                                }
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .whitespace_nowrap()
                                            .overflow_hidden()
                                            .child("Settings"),
                                    )
                                    .child(meta(
                                        match (demo, self.settings.accounts.len()) {
                                            (true, _) => "Work · 3 accounts".to_string(),
                                            // The one account's address: nothing else here shows it.
                                            (false, 0 | 1) if !self.account.is_empty() => {
                                                short(&self.account, 22).to_string()
                                            }
                                            (false, 0 | 1) => "Gmail".to_string(),
                                            // Several: the default account's address.
                                            (false, n) => self
                                                .settings
                                                .default_email()
                                                .map(|e| short(e, 22).to_string())
                                                .unwrap_or_else(|| format!("Gmail · {n} accounts")),
                                        },
                                        t.text3,
                                    )),
                            ),
                    )
                    .child(
                        icon_button(t, if self.dark { "sun" } else { "moon" })
                            .id("theme")
                            .press(t)
                            .tooltip(tip(t.dark, "Light or dark theme", "⌘⇧L"))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_theme(cx))),
                    ),
            );

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(SIDEBAR_W))
            .h_full()
            // Floating, the panel around it has the colour and the outline. The border keeps
            // its 1pt, so that docking moves nothing inside.
            .border_r_1()
            .border_color(if floating {
                gpui::transparent_black()
            } else {
                t.border
            })
            .when(!floating, |d| d.bg(t.sidebar))
            // Space for the native traffic lights; dragging here moves the window.
            .child(
                div()
                    .h(px(SIDEBAR_TOP))
                    .flex_none()
                    .on_mouse_down(MouseButton::Left, drag_or_zoom),
            )
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.))
                    .child(scrolled)
                    .child(crate::erase::fade(
                        SIDEBAR_FADE,
                        SIDEBAR_CLEAR,
                        t.sidebar,
                        self.sidebar_scroll.clone(),
                    )),
            )
            .child(footer)
    }
}

pub(super) const SIDEBAR_W: f32 = 236.;

pub(super) const NAV_RADIUS: f32 = 8.;

/// Between the search field and the first row under it.
pub(super) const SEARCH_GAP: f32 = 10.;

/// The sidebar's row of window controls. Nothing scrolls into it, so nothing shows through
/// the buttons or outside the floating panel's round corners.
pub(super) const SIDEBAR_TOP: f32 = 40.;

/// The room from there to the search field.
pub(super) const SIDEBAR_CLEAR: f32 = 12.;

/// Below that row, where what scrolls up fades out.
pub(super) const SIDEBAR_FADE: f32 = 28.;

/// The gap around the sidebar while it floats over the content.
pub(super) const FLOAT_INSET: f32 = 6.;

/// Its corners then. The hole it cuts in the email's web view is rounded the same.
pub(super) const FLOAT_RADIUS: f32 = 18.;

/// Where the sidebar is drawn: its left edge, and its gap from the window's top and bottom
/// (none when docked).
#[derive(Clone, Copy)]
pub(super) struct SidebarPlace {
    pub(super) x: f32,
    pub(super) inset: f32,
}

/// How long the sidebar and the message list take to slide open or shut.
pub(super) const PANEL_SLIDE_SECONDS: f32 = 0.3;

/// The curve of those slides: a symmetric ease-in-out (cubic). It starts gently, so the
/// first frame after a click is not already halfway, and settles softly.
pub(super) fn panel_ease(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    if t < 0.5 {
        4. * t * t * t
    } else {
        1. - (-2. * t + 2.).powi(3) / 2.
    }
}

/// The height of a sidebar row (folding animates rows to and from it).
pub(super) const NAV_ROW: f32 = 30.;

/// How long a sidebar section takes to fold or unfold.
pub(super) const FOLD: Duration = Duration::from_millis(240);

pub(super) fn view_key_for_section(key: &str) -> usize {
    match key {
        "split" => 0,
        "mailbox" => 1,
        _ => 2,
    }
}

pub(super) fn nav(
    t: &Theme,
    name: &str,
    label: &str,
    count: Option<String>,
    selected: bool,
    tint: Option<Hsla>,
) -> Div {
    let hover = t.hover;
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(10.))
        .h(px(NAV_ROW))
        .flex_none()
        .rounded(px(NAV_RADIUS))
        .cursor_pointer()
        .when(selected, |d| {
            d.relative()
                .font_weight(FontWeight::MEDIUM)
                .child(active_pill(t))
        })
        .when(!selected, |d| d.hover(move |s| s.bg(hover)))
        .child(icon(
            name,
            16.,
            tint.unwrap_or(if selected { t.text } else { t.text2 }),
        ))
        .child(
            div()
                .flex_1()
                .text_color(if selected { t.text } else { t.text2 })
                .child(label.to_string()),
        )
        // Not violet, nor is the icon: on glass over a bright desktop it cannot be seen.
        .children(count.map(|n| mono(n, if selected { t.text } else { t.text3 })))
}

/// Laid over a row: once `armed`, scrolls `handle` so the row is in view, at least `top`
/// below the top (what a fade covers there).
pub(super) fn reveal(
    handle: gpui::ScrollHandle,
    armed: std::rc::Rc<std::cell::Cell<bool>>,
    top: f32,
) -> impl IntoElement {
    gpui::canvas(
        move |bounds, window, _| {
            if !armed.replace(false) {
                return;
            }
            let view = handle.bounds();
            let above = f32::from(view.top()) + top - f32::from(bounds.top());
            let below = f32::from(bounds.bottom()) - f32::from(view.bottom());
            let shift = if above > 0. {
                above
            } else if below > 0. {
                -below
            } else {
                return;
            };
            let offset = handle.offset();
            handle.set_offset(gpui::point(offset.x, offset.y + px(shift)));
            window.refresh();
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

/// What is behind the sidebar's open row.
pub(super) fn active_pill(t: &Theme) -> impl IntoElement {
    crate::erase::glass(NAV_RADIUS, t.sidebar, t.glass, t.glass_opacity)
}
