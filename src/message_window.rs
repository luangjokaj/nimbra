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
    /// Original formatting (a web view) rather than the reader.
    original: bool,
    html_view: Option<std::rc::Rc<HtmlView>>,
    html_shown: Option<(String, bool, bool, bool)>,
    /// Holds the keyboard so R, A, F, E and ⌫ work here.
    focus: FocusHandle,
    focused_once: bool,
    /// The text blocks as last drawn here, for selecting (`selection.rs`).
    layouts: super::selection::Layouts,
    scroll: gpui::ScrollHandle,
    _observe: gpui::Subscription,
}

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
        MessageWindow {
            owner,
            key: key_of(&mail),
            account,
            last: mail,
            original,
            html_view: None,
            html_shown: None,
            focus: cx.focus_handle(),
            focused_once: false,
            layouts: Default::default(),
            scroll: gpui::ScrollHandle::new(),
            _observe: observe,
        }
    }

    pub(super) fn key(&self) -> &str {
        &self.key
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

    fn sync_web_view(
        &mut self,
        mail: &Mail,
        dark: bool,
        darkened: bool,
        images: bool,
        window: &mut Window,
    ) {
        let wanted = self.original && mail.loaded && mail.html.is_some();
        if !wanted {
            if let Some(view) = &self.html_view {
                view.set_visible(false);
            }
            return;
        }
        if self.html_view.is_none() {
            self.html_view = HtmlView::new(window).map(std::rc::Rc::new);
        }
        let Some(view) = &self.html_view else {
            self.original = false;
            return;
        };
        let shown = (self.key.clone(), dark, darkened, images);
        if self.html_shown.as_ref() != Some(&shown) {
            view.set_dark(dark, darkened);
            if let Some(html) = &mail.html {
                view.show(html, images);
            }
            self.html_shown = Some(shown);
        }
        view.set_visible(true);
    }
}

impl Render for MessageWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let m = self.mail(cx);
        let (dark, darkened, images) = {
            let n = self.owner.read(cx);
            (n.dark, n.darkens_mail(), n.remote_images(&m))
        };
        let t = if dark { Theme::dark() } else { Theme::light() };
        window.set_window_title(&m.subject);
        // A reply written here covers the message (and its web view, which is native).
        let compose = self.owner.update(cx, |n, cx| n.hosted_compose(window, cx));
        // A reply floating here: the email's web view (native, so above everything the app
        // draws) keeps showing, with a hole where the reply is. A full-page reply hides it.
        let reply_rect = self.owner.read(cx).compose.as_ref().and_then(|c| {
            let (x, y) = c.position?;
            (!c.full).then_some((x, y, c.size.0, c.size.1))
        });
        let full_reply = compose.is_some() && reply_rect.is_none();
        if full_reply {
            if let Some(view) = &self.html_view {
                view.set_visible(false);
            }
        } else {
            self.sync_web_view(&m, dark, darkened, images, window);
            if let Some(view) = &self.html_view {
                view.set_hole(if compose.is_some() { reply_rect } else { None });
            }
        }
        let original = self.original && m.html.is_some();
        let width = f32::from(window.viewport_size().width);

        let (done, in_trash) = {
            let n = self.owner.read(cx);
            let done = n.done_change().map(|change| match change {
                mail::Change::Archive => ("done", "Mark Done"),
                mail::Change::NotSpam => ("inbox", "Not Spam"),
                mail::Change::Untrash => ("inbox", "Restore"),
                _ => ("inbox", "Move to Inbox"),
            });
            (done, n.mailbox == View::Trash)
        };
        let tool = |name: &'static str, label: &'static str, key: &'static str, id: usize| {
            icon_button(&t, name)
                .id(("message-window-tool", id))
                .press(&t)
                .tooltip(tip(t.dark, label, key))
        };
        let rule = || {
            div()
                .flex_none()
                .w(px(1.))
                .h(px(16.))
                .mx(px(8.))
                .bg(t.border_strong)
        };
        let toolbar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            // Its hairline is the line under it.
            .h(px(crate::ui::TOP_BAR - 1.))
            .pl(px(84.))
            .pr(px(14.))
            .on_mouse_down(MouseButton::Left, |event, window, cx| {
                drag_or_zoom(event, window, cx)
            })
            .when_some(done, |d, (name, label)| {
                d.child(
                    tool(name, label, "E", 3).on_click(cx.listener(|this, _, window, cx| {
                        this.act(window, cx, |n, row, cx| n.done_thread(row, cx))
                    })),
                )
                .child(rule())
            })
            .child(tool("reply", "Reply", "R", 0).on_click(
                cx.listener(|this, _, window, cx| this.reply(ComposeKind::Reply, window, cx)),
            ))
            .child(tool("reply-all", "Reply All", "A", 1).on_click(
                cx.listener(|this, _, window, cx| this.reply(ComposeKind::ReplyAll, window, cx)),
            ))
            .child(tool("forward", "Forward", "F", 2).on_click(
                cx.listener(|this, _, window, cx| this.reply(ComposeKind::Forward, window, cx)),
            ))
            .child(rule())
            .child(
                tool(
                    "trash",
                    if in_trash {
                        "Delete Forever"
                    } else {
                        "Move to Trash"
                    },
                    "⌫",
                    4,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.act(window, cx, |n, row, cx| n.trash_thread(row, cx))
                })),
            )
            .child(div().flex_1())
            // V is the main window's: no key here.
            .when(m.html.is_some(), |d| {
                d.child(
                    tool(
                        "eye",
                        if self.original {
                            "Reader View"
                        } else {
                            "Show Original"
                        },
                        "",
                        5,
                    )
                    .when(self.original, |d| d.bg(t.selected))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.original = !this.original;
                        cx.notify();
                    })),
                )
            });
        let divider = || div().flex_none().h(px(1.)).bg(t.border);
        let scrolled = !original && f32::from(self.scroll.offset().y) < -0.5;
        let loading = !m.loaded && m.body.is_empty();

        let header = |n: &mut Nimbra, cx: &mut Context<Nimbra>, title_w: f32| {
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(n.subject_row(&t, &m, 24., title_w, cx))
                .children(n.thread_banners(&t, &m, cx))
        };

        let images = self.owner.read(cx).remote_images(&m);
        let body: AnyElement = if loading {
            self.owner
                .read(cx)
                .loading_pane(&t, &m, self.owner.clone())
                .into_any_element()
        } else if original {
            let view = self.html_view.clone();
            let top = self.owner.update(cx, |n, cx| {
                div()
                    .flex()
                    .flex_none()
                    .flex_col()
                    .gap(px(12.))
                    .px(px(28.))
                    .pt(px(18.))
                    .pb(px(14.))
                    .child(header(n, cx, (width - 56.).max(0.)))
                    .child(n.message_header(&t, &m, 0, false, cx))
            });
            let tray = (!m.header.attachments.is_empty()).then(|| {
                self.owner.update(cx, |n, cx| {
                    n.attachment_tray(&t, &m.header.attachments, m.account.as_deref(), cx)
                })
            });
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .child(top)
                .child(divider())
                .child(
                    gpui::canvas(
                        move |bounds, _, _| {
                            if let Some(view) = &view {
                                view.set_bounds(
                                    f32::from(bounds.origin.x),
                                    f32::from(bounds.origin.y),
                                    f32::from(bounds.size.width),
                                    f32::from(bounds.size.height),
                                );
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .flex_1()
                    .size_full(),
                )
                .children(tray)
                .into_any_element()
        } else {
            let column = width.min(860.);
            self.layouts.borrow_mut().clear();
            let selectable = super::selection::Selectable {
                place: super::selection::Place::Window(self.key.clone()),
                layouts: self.layouts.clone(),
            };
            let content = self.owner.update(cx, |n, cx| {
                let mut d = div()
                    .flex()
                    .flex_col()
                    .gap(px(24.))
                    .w_full()
                    .max_w(px(860.))
                    .mx_auto()
                    .px(px(36.))
                    .pt(px(28.))
                    .pb(px(48.))
                    .child(header(n, cx, (column - 72.).max(0.)));
                // Every message, oldest first, the latest last.
                for (k, e) in m.earlier.iter().enumerate() {
                    d = d.child(divider()).child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(16.))
                            .child(n.message_header(&t, &m, k + 1, false, cx))
                            .child(
                                n.message_body(
                                    &t,
                                    &e.blocks,
                                    (k + 1) * 1000,
                                    images,
                                    &selectable,
                                    cx,
                                )
                                .ml(px(40.)),
                            )
                            .when(!e.header.attachments.is_empty(), |d| {
                                d.child(
                                    n.attachment_row(
                                        &t,
                                        &e.header.attachments,
                                        (k + 1) * 100,
                                        m.account.as_deref(),
                                        cx,
                                    )
                                    .pl(px(40.)),
                                )
                            }),
                    );
                }
                d.child(divider()).child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(16.))
                        .child(n.message_header(&t, &m, 0, false, cx))
                        .child(
                            n.message_body(&t, &m.body, 0, images, &selectable, cx)
                                .ml(px(40.)),
                        )
                        .when(!m.header.attachments.is_empty(), |d| {
                            d.child(
                                n.attachment_row(
                                    &t,
                                    &m.header.attachments,
                                    0,
                                    m.account.as_deref(),
                                    cx,
                                )
                                .pl(px(40.)),
                            )
                        }),
                )
            });
            div()
                .id("message-window-scroll")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
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
                let mail = this.mail(cx);
                this.owner
                    .update(cx, |n, cx| n.copy_selection(&place, &mail, cx));
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
            .text_size(px(13.))
            .line_height(px(18.))
            .text_color(t.text)
            .child(toolbar)
            .child(divider().when(!(original || loading || scrolled), |d| d.opacity(0.)))
            .child(body)
            .children(compose)
    }
}
