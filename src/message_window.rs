//! A conversation in a window of its own (double-click a row). It shows the thread as the
//! app knows it (the `Nimbra` entity), so it fills in when the content arrives, and acts on
//! it through the app.

use super::*;

pub(super) struct MessageWindow {
    owner: Entity<Nimbra>,
    /// The thread's id (or, for sample mail, its subject).
    key: String,
    /// The account it was opened from. The main window may switch to another; this stays.
    account: String,
    /// The thread as last seen, for when it is no longer in the open mailbox.
    last: Mail,
    /// Emails in their original formatting (web views) rather than drawn by the reader.
    original: bool,
    /// The emails' web views (`mail_views.rs`).
    mail_views: crate::mail_views::MailViews,
    /// Holds the keyboard so R, A, F, E and ⌫ work here.
    focus: FocusHandle,
    focused_once: bool,
    /// The text blocks as last drawn here, for selecting (`selection.rs`).
    layouts: super::selection::Layouts,
    /// The line under the toolbar while the thread or its emails load, as in the main
    /// window (`ui::LoadingLine`).
    loading_line: crate::ui::LoadingLine,
    /// When it opened, still unseen: it shows once its emails are drawn, or after
    /// `REVEAL_WITHIN` at most (`reveal`).
    unseen_since: Option<std::time::Instant>,
    /// The thread's content was asked for (`fetch_if_missing`), until it is here.
    asked: bool,
    _observe: gpui::Subscription,
}

/// The longest a message window waits, unseen, for its emails before it shows anyway.
const REVEAL_WITHIN: std::time::Duration = std::time::Duration::from_millis(800);

/// The reader's column padding on either side, in a message window.
const WINDOW_SIDE: f32 = 36.;

/// The key a thread is known by here.
pub(super) fn key_of(mail: &Mail) -> String {
    mail.id.clone().unwrap_or_else(|| mail.subject.to_string())
}

impl MessageWindow {
    pub(super) fn new(
        owner: Entity<Nimbra>,
        mail: Mail,
        account: String,
        original: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let observe = cx.observe(&owner, |_, _, cx| cx.notify());
        let (mail_views, mut events) = crate::mail_views::MailViews::new();
        // Each email's height, as its page works it out.
        cx.spawn(async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(event) = events.next().await {
                let changed = this
                    .update(cx, |this, _| this.mail_views.handle(event))
                    .unwrap_or(false);
                if changed {
                    this.update(cx, |_, cx| cx.notify()).ok();
                }
            }
        })
        .detach();
        MessageWindow {
            owner,
            key: key_of(&mail),
            account,
            last: mail,
            original,
            mail_views,
            focus: cx.focus_handle(),
            focused_once: false,
            layouts: Default::default(),
            loading_line: Default::default(),
            unseen_since: Some(std::time::Instant::now()),
            asked: false,
            _observe: observe,
        }
    }

    /// Shows the window once nothing in it is still being drawn (its content not fetched
    /// yet does not count: that can take a while, and the loading line says so), or once it
    /// has waited `REVEAL_WITHIN`. Until then it is drawn unseen.
    fn reveal(&mut self, drawing: bool, window: &mut Window) {
        let Some(since) = self.unseen_since else {
            return;
        };
        if drawing && since.elapsed() < REVEAL_WITHIN {
            window.request_animation_frame();
            return;
        }
        self.unseen_since = None;
        crate::blur::set_alpha(window, 1.);
    }

    /// Asks the app for the thread's content once while it is not here. True when that
    /// failed (offline, say): then nothing is on its way.
    fn fetch_if_missing(&mut self, m: &Mail, cx: &mut Context<Self>) -> bool {
        if m.loaded {
            self.asked = false;
            return false;
        }
        let key = (self.key.clone(), self.account.clone());
        let failed = self.owner.read(cx).body_failed.contains(&key);
        if !self.asked && !failed && m.id.is_some() {
            self.asked = true;
            let (id, account) = key;
            self.owner.update(cx, |n, cx| n.load_body(id, account, cx));
        }
        failed
    }

    /// Shows the window now if it is still unseen: the guard for a window that draws no
    /// frames while unseen (macOS may count a transparent window as hidden).
    pub(super) fn reveal_now(&mut self, window: &mut Window) {
        if self.unseen_since.take().is_some() {
            crate::blur::set_alpha(window, 1.);
        }
    }

    pub(super) fn key(&self) -> &str {
        &self.key
    }

    pub(super) fn account(&self) -> &str {
        &self.account
    }

    /// The thread, fresh from the app when it still has it.
    fn mail(&mut self, cx: &App) -> Mail {
        let n = self.owner.read(cx);
        // Opened before the account's id was known (a first sign-in): it is the one shown.
        if self.account.is_empty() {
            self.account = n.account_id.clone();
        }
        if let Some(mail) = n
            .mails
            .iter()
            .chain(n.cache.values().flatten())
            .find(|m| key_of(m) == self.key && n.account_of(m) == self.account)
        {
            self.last = mail.clone();
        }
        let mut mail = self.last.clone();
        // On a mail no account means the one shown, which may no longer be this one's.
        mail.account = (self.account != n.account_id).then(|| self.account.clone());
        mail
    }

    /// Runs `act` on this window's thread, and closes the window if it was done.
    fn act(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        act: impl FnOnce(&mut Nimbra, usize, &mut Context<Nimbra>) -> bool,
    ) {
        let done = self.owner.update(cx, |n, cx| {
            n.row_to_act_on(&self.key, &self.account, cx)
                .is_some_and(|row| act(n, row, cx))
        });
        if done {
            window.remove_window();
        }
    }

    /// Reply, reply all or forward, written right here in this window.
    pub(super) fn reply(&mut self, kind: ComposeKind, window: &mut Window, cx: &mut Context<Self>) {
        let mail = self.mail(cx);
        let here = window.window_handle();
        self.owner.update(cx, |n, cx| {
            // `open_compose_from` would leave that message as it is, and what follows would
            // pull it in here, where it looks like the reply to this one.
            if n.compose.is_some() {
                if n.compose_host != Some(here) {
                    n.notice =
                        Some("Finish or close the message you are writing, then reply.".into());
                    cx.notify();
                }
                return;
            }
            n.open_compose_from(kind, Some(Source::latest(&mail)), cx);
            if let Some(composer) = &mut n.compose {
                // Floating over the message, as in the main window.
                composer.full = false;
                composer.position = None;
                n.compose_host = Some(here);
                n.compose_detached = false;
            }
            cx.notify();
        });
    }

    /// The thread's key among its web views.
    fn thread(&self) -> String {
        format!("{}:{}", self.account, self.key)
    }
}

impl Render for MessageWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Every window draws at the interface zoom.
        crate::ui::apply_zoom(window);
        let m = self.mail(cx);
        let (dark, darkened, images, text_scale) = {
            let n = self.owner.read(cx);
            (
                n.dark,
                n.darkens_mail(),
                n.settings.load_images,
                crate::webview::zoom_for(n.settings.reader_text),
            )
        };
        let t = if dark { Theme::dark() } else { Theme::light() };
        window.set_window_title(&m.shown_subject());
        // A reply written here covers the message (and its web view, which is native).
        let compose = self.owner.update(cx, |n, cx| n.hosted_compose(window, cx));
        // A reply floating here: the email's web view (native, so above everything the app
        // draws) keeps showing, with a hole where the reply is. A full-page reply hides it.
        let reply_rect = self.owner.read(cx).compose.as_ref().and_then(|c| {
            let (x, y) = c.position?;
            (!c.full).then_some((x, y, c.size.0, c.size.1))
        });
        let full_reply = compose.is_some() && reply_rect.is_none();
        let original = self.original && m.has_html() && m.loaded;
        let thread = self.thread();
        let wanted = if original {
            crate::mail_views::wanted(&m, &thread, true, true)
        } else {
            Vec::new()
        };
        let look = crate::mail_views::Look {
            images,
            dark,
            darkened,
        };
        let hole = if compose.is_some() { reply_rect } else { None };
        if !self.mail_views.sync(
            wanted,
            look,
            &self.account,
            text_scale,
            !full_reply,
            hole,
            window,
        ) {
            self.original = false;
            self.mail_views.clear();
        }
        // Each email shows once its text is laid out, without waiting for its images.
        self.mail_views.measure_waiting();
        // Each message's email, where it has one (0 the latest, k + 1 `earlier[k]`).
        let emails: Vec<_> = (0..=m.earlier.len())
            .map(|index| {
                let id = match index {
                    0 => m.header.message_id.as_str(),
                    k => m.earlier[k - 1].header.message_id.as_str(),
                };
                original
                    .then(|| {
                        self.mail_views
                            .slot(&crate::mail_views::message_key(&thread, id, index))
                    })
                    .flatten()
            })
            .collect();
        let body_of = |n: &Nimbra,
                       cx: &mut Context<Nimbra>,
                       index: usize,
                       gaps: (f32, f32),
                       blocks: &[crate::rich::Block],
                       selectable: &super::selection::Selectable|
         -> Div {
            match emails[index].clone() {
                Some(slot) => super::reading::email_frame(
                    slot,
                    super::reading::ORIGINAL_SIDE,
                    gaps,
                    0.,
                    None,
                    darkened,
                    false,
                    &t,
                ),
                None => n
                    .message_body(&t, blocks, index * 1000, selectable, n.account_of(&m), cx)
                    .pl(dp(40.)),
            }
        };
        // Its content, when the app does not have it (dropped since, or never fetched):
        // asked for here, as the main window only fetches the thread selected in it.
        let failed = self.fetch_if_missing(&m, cx);
        // Loading: the thread's content, or an email not drawn yet. Not once it failed.
        let loading = !failed && (!m.loaded || emails.iter().flatten().any(|slot| slot.waiting()));
        self.reveal(loading && m.loaded, window);
        if self.loading_line.update(loading) {
            window.request_animation_frame();
        }
        let width = from_screen(window.viewport_size().width);

        // As the main window's toolbar (`reading.rs`): the actions as words, then icons.
        let worded = width >= 560.;
        let tool = |name: &'static str, label: &'static str, id: usize| {
            let key = crate::shortcuts::label(match id {
                0 => "reply",
                1 => "reply-all",
                2 => "forward-message",
                3 => "mark-done",
                _ => "trash",
            });
            icon_button(&t, name)
                .id(("message-window-tool", id))
                .press(&t)
                .tooltip(tip(t.dark, label, key))
        };
        let action = |name: &'static str, word: &'static str, label: &'static str, id: usize| {
            super::reading::action_button(&t, name, word, worded)
                .id(("message-window-action", id))
                .press(&t)
                .tooltip(tip(
                    t.dark,
                    label,
                    crate::shortcuts::label(match id {
                        0 => "reply",
                        2 => "forward-message",
                        _ => "mark-done",
                    }),
                ))
        };
        let toolbar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(dp(2.))
            .h(dp(crate::ui::top_bar()))
            .pl(dp(84.))
            .pr(dp(14.))
            .border_b_1()
            .border_color(t.border)
            .on_mouse_down(MouseButton::Left, |event, window, cx| {
                drag_or_zoom(event, window, cx)
            })
            .child(
                action("done", "Done", "Mark done (archive)", 3).on_click(cx.listener(
                    |this, _, window, cx| this.act(window, cx, |n, row, cx| n.done_thread(row, cx)),
                )),
            )
            // Reply with its word, Reply all and Forward as icons beside it, as in the main
            // window.
            .child(action("reply", "Reply", "Reply", 0).on_click(
                cx.listener(|this, _, window, cx| this.reply(ComposeKind::Reply, window, cx)),
            ))
            .child(tool("reply-all", "Reply all", 1).on_click(
                cx.listener(|this, _, window, cx| this.reply(ComposeKind::ReplyAll, window, cx)),
            ))
            .child(tool("forward", "Forward", 2).on_click(
                cx.listener(|this, _, window, cx| this.reply(ComposeKind::Forward, window, cx)),
            ))
            .child(div().flex_1())
            .child(tool("trash", "Move to trash", 4).on_click(cx.listener(
                |this, _, window, cx| this.act(window, cx, |n, row, cx| n.trash_thread(row, cx)),
            )))
            .when(m.has_html(), |d| {
                d.child(div().w(dp(1.)).h(dp(16.)).mx(dp(6.)).bg(t.border_strong))
            })
            .when(m.has_html(), |d| {
                d.child(
                    button(
                        &t,
                        ButtonKind::Ghost,
                        if self.original {
                            "Reader view"
                        } else {
                            "Show original"
                        },
                        None,
                    )
                    .id("message-window-view")
                    .press_soft()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.original = !this.original;
                        cx.notify();
                    })),
                )
            });

        let header = |n: &mut Nimbra, cx: &mut Context<Nimbra>, title_w: f32| {
            div()
                .flex()
                .flex_col()
                .gap(dp(12.))
                .child(n.subject_row(&t, &m, 28., title_w, cx))
                .child(n.thread_banners(&t, &m, cx))
        };

        let body: AnyElement = if !m.loaded && m.body.is_empty() {
            self.owner
                .read(cx)
                .loading_pane(&t, &m, self.owner.clone())
                .into_any_element()
        } else {
            let column = width.min(860.);
            self.layouts.borrow_mut().clear();
            let selectable = super::selection::Selectable {
                place: super::selection::Place::Window(self.key.clone()),
                layouts: self.layouts.clone(),
            };
            // The original view's lines run across the window, past the column's padding.
            let across = if original {
                super::reading::ORIGINAL_SIDE
            } else {
                0.
            };
            let content = self.owner.update(cx, |n, cx| {
                let d = div()
                    .flex()
                    .flex_col()
                    .gap(dp(16.))
                    .w_full()
                    // Emails in their own formatting take the window's whole width.
                    .when(!original, |d| d.max_w(dp(860.)).mx_auto())
                    .px(dp(if original {
                        super::reading::ORIGINAL_SIDE
                    } else {
                        WINDOW_SIDE
                    }))
                    .pt(dp(26.))
                    // An email last in the window runs to its bottom.
                    .pb(dp(if !original || emails[0].is_none() {
                        48.
                    } else if m.header.attachments.is_empty() {
                        0.
                    } else {
                        super::reading::ORIGINAL_SIDE
                    }))
                    .child(header(n, cx, (column - 72.).max(0.)));
                // Every message, oldest first and the latest last, or the latest first
                // (Settings › Appearance). `earlier` is newest first.
                let newest_first = n.settings.newest_first;
                let mut earlier: Vec<Div> = Vec::with_capacity(m.earlier.len());
                for (k, e) in m.earlier.iter().enumerate() {
                    // The message at the bottom has the column's padding under it instead.
                    let bottom = newest_first && k + 1 == m.earlier.len();
                    earlier.push(
                        div()
                            .flex()
                            .flex_col()
                            .gap(dp(12.))
                            .pt(dp(14.))
                            // Lines across the window in the original view.
                            .mx(dp(-across))
                            .px(dp(across))
                            .border_t_1()
                            .border_color(t.border)
                            .child(n.message_header(&t, &m, k + 1, false, cx))
                            .child(body_of(
                                n,
                                cx,
                                k + 1,
                                // The message's gap over it; under it, its gap to the
                                // attachments, or the column's to the next message.
                                (
                                    12.,
                                    if !e.header.attachments.is_empty() {
                                        12. - super::reading::ORIGINAL_SIDE
                                    } else if bottom {
                                        0.
                                    } else {
                                        16.
                                    },
                                ),
                                &e.blocks,
                                &selectable,
                            ))
                            .when(!e.header.attachments.is_empty(), |d| {
                                d.child(
                                    n.attachment_row(
                                        &t,
                                        &e.header.attachments,
                                        (k + 1) * 100,
                                        m.account.as_deref(),
                                        cx,
                                    )
                                    .pl(dp(
                                        super::reading::attachments_indent(emails[k + 1].is_some()),
                                    )),
                                )
                            }),
                    );
                }
                let latest = div()
                    .flex()
                    .flex_col()
                    .gap(dp(12.))
                    .pt(dp(14.))
                    .mx(dp(-across))
                    .px(dp(across))
                    .border_t_1()
                    .border_color(t.border)
                    .child(n.message_header(&t, &m, 0, false, cx))
                    .child(body_of(
                        n,
                        cx,
                        0,
                        (
                            12.,
                            if !m.header.attachments.is_empty() {
                                12. - super::reading::ORIGINAL_SIDE
                            } else if newest_first {
                                16.
                            } else {
                                0.
                            },
                        ),
                        &m.body,
                        &selectable,
                    ))
                    .when(!m.header.attachments.is_empty(), |d| {
                        d.child(
                            n.attachment_row(
                                &t,
                                &m.header.attachments,
                                0,
                                m.account.as_deref(),
                                cx,
                            )
                            .pl(dp(super::reading::attachments_indent(emails[0].is_some()))),
                        )
                    });
                if newest_first {
                    d.child(latest).children(earlier)
                } else {
                    d.children(earlier.into_iter().rev()).child(latest)
                }
            });
            div()
                .id("message-window-scroll")
                .flex_1()
                .min_h(dp(0.))
                .overflow_y_scroll()
                .child(content)
                .into_any_element()
        };

        if !self.focused_once {
            self.focused_once = true;
            window.focus(&self.focus);
        }
        // Dragging to select follows the pointer over the whole window.
        let watch = self.owner.update(cx, |n, cx| {
            n.selection_watch(self.layouts.clone(), cx)
                .into_any_element()
        });
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .child(watch)
            .on_action(cx.listener(|this, _: &crate::input::Copy, _, cx| {
                let place = super::selection::Place::Window(this.key.clone());
                // The text picked in this window's reader, or else the email's own selection.
                let picked = this.owner.read(cx).selection_in(&place).is_some();
                if picked {
                    let mail = this.mail(cx);
                    this.owner
                        .update(cx, |n, cx| n.copy_selection(&place, &mail, cx));
                } else {
                    this.mail_views.copy();
                }
            }))
            // The same single-letter keys as in the main window, unless a reply is open here.
            .key_context(if compose.is_some() {
                "Composer"
            } else {
                "Mail"
            })
            .track_focus(&self.focus)
            // ⌘W: with a reply open here that has something in it, asks about the draft
            // first; otherwise closes the window.
            .on_action(cx.listener(|this, _: &HideWindow, window, cx| {
                let handle = window.window_handle();
                if this
                    .owner
                    .update(cx, |n, cx| n.may_close_window(handle, cx))
                {
                    window.defer(cx, |window, _| window.remove_window());
                }
            }))
            .on_action(
                cx.listener(|this, _: &Reply, window, cx| {
                    this.reply(ComposeKind::Reply, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ReplyAll, window, cx| {
                this.reply(ComposeKind::ReplyAll, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ForwardMessage, window, cx| {
                this.reply(ComposeKind::Forward, window, cx)
            }))
            .on_action(cx.listener(|this, _: &MarkDone, window, cx| {
                this.act(window, cx, |n, row, cx| n.done_thread(row, cx))
            }))
            .on_action(cx.listener(|this, _: &Trash, window, cx| {
                this.act(window, cx, |n, row, cx| n.trash_thread(row, cx))
            }))
            .bg(t.surface)
            .font_family(crate::theme::SANS)
            .text_size(dp(13.))
            .line_height(dp(18.))
            .text_color(t.text)
            .child(toolbar)
            .child(self.loading_line.view(&t, "window-loading"))
            .child(body)
            .children(compose)
    }
}
