//! A conversation in a window of its own (double-click a row). It shows the thread as the
//! app knows it (the `Nimbra` entity), so it fills in when the content arrives, and acts on
//! it through the app.

use super::*;

pub(super) struct MessageWindow {
    owner: Entity<Nimbra>,
    /// The thread's id (or, for sample mail, its subject).
    key: String,
    /// The thread as last seen, for when it is no longer in the open mailbox.
    last: Mail,
    /// Original formatting (a web view) rather than the reader.
    original: bool,
    html_view: Option<std::rc::Rc<HtmlView>>,
    html_shown: Option<(String, bool, bool)>,
    /// Holds the keyboard so R, A, F, E and ⌫ work here.
    focus: FocusHandle,
    focused_once: bool,
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
        original: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let observe = cx.observe(&owner, |_, _, cx| cx.notify());
        MessageWindow {
            owner,
            key: key_of(&mail),
            last: mail,
            original,
            html_view: None,
            html_shown: None,
            focus: cx.focus_handle(),
            focused_once: false,
            _observe: observe,
        }
    }

    pub(super) fn key(&self) -> &str {
        &self.key
    }

    /// The thread, fresh from the app when it still has it.
    fn mail(&mut self, cx: &App) -> Mail {
        let n = self.owner.read(cx);
        if let Some(mail) = n
            .mails
            .iter()
            .chain(n.cache.values().flatten())
            .find(|m| key_of(m) == self.key)
        {
            self.last = mail.clone();
        }
        self.last.clone()
    }

    /// Runs `act` on the thread in the main window (selected there first).
    fn act(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        act: impl FnOnce(&mut Nimbra, &mut Context<Nimbra>) + 'static,
        close: bool,
    ) {
        let key = self.key.clone();
        let done = self.owner.update(cx, |n, cx| {
            let Some(index) = n.mails.iter().position(|m| key_of(m) == key) else {
                n.notice = Some("This conversation is no longer in the open mailbox.".into());
                cx.notify();
                return false;
            };
            n.picked.clear();
            n.open(index);
            act(n, cx);
            cx.notify();
            true
        });
        if done && close {
            window.remove_window();
        }
    }

    /// Reply, reply all or forward, written right here in this window.
    pub(super) fn reply(&mut self, kind: ComposeKind, window: &mut Window, cx: &mut Context<Self>) {
        let mail = self.mail(cx);
        let here = window.window_handle();
        self.owner.update(cx, |n, cx| {
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

    fn sync_web_view(&mut self, mail: &Mail, dark: bool, images: bool, window: &mut Window) {
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
        let shown = (self.key.clone(), dark, images);
        if self.html_shown.as_ref() != Some(&shown) {
            view.set_dark(dark);
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
        let (dark, images) = {
            let n = self.owner.read(cx);
            (n.dark, n.settings.load_images)
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
            self.sync_web_view(&m, dark, images, window);
            if let Some(view) = &self.html_view {
                view.set_hole(if compose.is_some() { reply_rect } else { None });
            }
        }
        let original = self.original && m.html.is_some();
        let width = f32::from(window.viewport_size().width);

        let tool = |name: &'static str, label: &'static str, id: usize| {
            let key = match id {
                0 => "R",
                1 => "A",
                2 => "F",
                3 => "E",
                _ => "⌫",
            };
            icon_button(&t, name)
                .id(("message-window-tool", id))
                .press(&t)
                .tooltip(tip(t.dark, label, key))
        };
        let toolbar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .h(px(52.))
            .pl(px(84.))
            .pr(px(14.))
            .border_b_1()
            .border_color(t.border)
            .on_mouse_down(MouseButton::Left, |event, window, cx| {
                drag_or_zoom(event, window, cx)
            })
            .child(tool("reply", "Reply", 0).on_click(
                cx.listener(|this, _, window, cx| this.reply(ComposeKind::Reply, window, cx)),
            ))
            .child(tool("reply-all", "Reply all", 1).on_click(
                cx.listener(|this, _, window, cx| this.reply(ComposeKind::ReplyAll, window, cx)),
            ))
            .child(tool("forward", "Forward", 2).on_click(
                cx.listener(|this, _, window, cx| this.reply(ComposeKind::Forward, window, cx)),
            ))
            .child(div().w(px(1.)).h(px(18.)).mx(px(6.)).bg(t.border))
            .child(
                tool("done", "Done", 3).on_click(cx.listener(|this, _, window, cx| {
                    this.act(window, cx, |n, cx| n.mark_done(cx), true)
                })),
            )
            .child(tool("trash", "Move to trash", 4).on_click(
                cx.listener(|this, _, window, cx| this.act(window, cx, |n, cx| n.trash(cx), true)),
            ))
            .child(div().flex_1())
            .when(m.html.is_some(), |d| {
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
                .gap(px(12.))
                .child(serif(
                    &balance_title(&m.subject, 28., title_w, cx),
                    28.,
                    t.text,
                ))
                .child(n.thread_banners(&t, &m, cx))
        };

        let body: AnyElement = if !m.loaded && m.body.is_empty() {
            self.owner.read(cx).loading_pane(&t).into_any_element()
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
                    .pb(px(12.))
                    .border_b_1()
                    .border_color(t.border)
                    .child(header(n, cx, (width - 56.).max(0.)))
                    .child(n.message_header(&t, &m, 0, false, cx))
            });
            let tray = (!m.header.attachments.is_empty()).then(|| {
                self.owner
                    .update(cx, |n, cx| n.attachment_tray(&t, &m.header.attachments, cx))
            });
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .child(top)
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
            let content = self.owner.update(cx, |n, cx| {
                let mut d = div()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .w_full()
                    .max_w(px(860.))
                    .mx_auto()
                    .px(px(36.))
                    .pt(px(26.))
                    .pb(px(48.))
                    .child(header(n, cx, (column - 72.).max(0.)));
                // Every message, oldest first, the latest last.
                for (k, e) in m.earlier.iter().enumerate() {
                    d = d.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .pt(px(14.))
                            .border_t_1()
                            .border_color(t.border)
                            .child(n.message_header(&t, &m, k + 1, false, cx))
                            .child(
                                n.message_body(&t, &e.blocks, (k + 1) * 1000, cx)
                                    .pl(px(40.)),
                            )
                            .when(!e.header.attachments.is_empty(), |d| {
                                d.child(
                                    n.attachment_row(&t, &e.header.attachments, (k + 1) * 100, cx)
                                        .pl(px(40.)),
                                )
                            }),
                    );
                }
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(12.))
                        .pt(px(14.))
                        .border_t_1()
                        .border_color(t.border)
                        .child(n.message_header(&t, &m, 0, false, cx))
                        .child(n.message_body(&t, &m.body, 0, cx).pl(px(40.)))
                        .when(!m.header.attachments.is_empty(), |d| {
                            d.child(
                                n.attachment_row(&t, &m.header.attachments, 0, cx)
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
                .child(content)
                .into_any_element()
        };

        if !self.focused_once {
            self.focused_once = true;
            window.focus(&self.focus);
        }
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
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
                this.act(window, cx, |n, cx| n.mark_done(cx), true)
            }))
            .on_action(cx.listener(|this, _: &Trash, window, cx| {
                this.act(window, cx, |n, cx| n.trash(cx), true)
            }))
            .bg(t.surface)
            .font_family(crate::theme::SANS)
            .text_size(px(13.))
            .line_height(px(18.))
            .text_color(t.text)
            .child(toolbar)
            .child(body)
            .children(compose)
    }
}
