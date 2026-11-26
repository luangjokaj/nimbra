//! The Settings window: its own window with a sidebar of sections. It reads and changes the
//! app's state (the `Nimbra` entity) and saves the preferences as they change.

use std::time::Instant;

use super::PhotoAnswer;
use super::*;
use crate::settings::{MarkRead, TextSize, ThemeChoice};

/// The app's icon on About, in points.
const ABOUT_ICON: f32 = 96.;

/// The app's icon in its light (Default) and Dark appearances, rendered from
/// `assets/AppIcon.icon` (README: how to render them again).
static APP_ICON: std::sync::LazyLock<std::sync::Arc<gpui::Image>> =
    std::sync::LazyLock::new(|| {
        std::sync::Arc::new(gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            include_bytes!("../assets/app-icon.png").to_vec(),
        ))
    });
static APP_ICON_DARK: std::sync::LazyLock<std::sync::Arc<gpui::Image>> =
    std::sync::LazyLock::new(|| {
        std::sync::Arc::new(gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            include_bytes!("../assets/app-icon-dark.png").to_vec(),
        ))
    });

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Tab {
    General,
    Accounts,
    Appearance,
    Reading,
    Composing,
    Notifications,
    Privacy,
    Shortcuts,
    About,
}

impl Tab {
    const ALL: [Tab; 9] = [
        Tab::General,
        Tab::Accounts,
        Tab::Appearance,
        Tab::Reading,
        Tab::Composing,
        Tab::Notifications,
        Tab::Privacy,
        Tab::Shortcuts,
        Tab::About,
    ];

    fn label(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Accounts => "Accounts",
            Tab::Appearance => "Appearance",
            Tab::Reading => "Reading",
            Tab::Composing => "Composing",
            Tab::Notifications => "Notifications",
            Tab::Privacy => "Privacy & Security",
            Tab::Shortcuts => "Shortcuts",
            Tab::About => "About",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Tab::General => "zap",
            Tab::Accounts => "users",
            Tab::Appearance => "sun",
            Tab::Reading => "mail",
            Tab::Composing => "draft",
            Tab::Notifications => "bell",
            Tab::Privacy => "lock",
            Tab::Shortcuts => "list",
            Tab::About => "sparkles",
        }
    }
}

pub(super) struct SettingsWindow {
    owner: Entity<Nimbra>,
    tab: Tab,
    /// One field per line of the signature.
    signature: Vec<Entity<TextInput>>,
    pending_focus: Option<FocusHandle>,
    /// A first click on a destructive button arms it; a second within 4 s does it.
    armed: Option<(&'static str, Instant)>,
    /// A segmented choice that just changed: the option it slides from, when, and a
    /// count that keys the slide.
    slides: std::collections::HashMap<&'static str, (usize, Instant, usize)>,
    /// Which app opens email links, as last read (read again whenever this window is
    /// brought forward: it is changed in other apps too).
    mail_app: Option<crate::integration::MailApp>,
    making_default: bool,
    _activation: Option<gpui::Subscription>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl SettingsWindow {
    /// `text` and `dark` come from the app (which is busy opening this window, so it
    /// cannot be read here).
    pub(super) fn new(
        owner: Entity<Nimbra>,
        text: String,
        dark: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let t = if dark { Theme::dark() } else { Theme::light() };
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        let mut signature = Vec::new();
        let mut subscriptions = vec![cx.observe(&owner, |_, _, cx| cx.notify())];
        for index in 0..6 {
            let placeholder = if index == 0 { "Your name" } else { "" };
            let input = cx.new(|cx| TextInput::new(cx, placeholder, t.faint, t.violet));
            if let Some(line) = lines.get(index).cloned() {
                input.update(cx, |input, cx| input.set(line, usize::MAX, cx));
            }
            subscriptions.push(
                cx.subscribe(&input, move |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Changed => this.signature_changed(cx),
                    InputEvent::Enter | InputEvent::Down | InputEvent::Tab => {
                        if let Some(next) = this.signature.get(index + 1) {
                            this.pending_focus = Some(next.read(cx).focus_handle(cx));
                            cx.notify();
                        }
                    }
                    InputEvent::Up | InputEvent::BackspaceAtStart if index > 0 => {
                        let previous = this.signature[index - 1].clone();
                        this.pending_focus = Some(previous.read(cx).focus_handle(cx));
                        cx.notify();
                    }
                    _ => {}
                }),
            );
            signature.push(input);
        }
        SettingsWindow {
            owner,
            // Screenshots of Add account open on Accounts.
            tab: if std::env::var("NIMBRA_SCREEN").as_deref() == Ok("settings-account") {
                Tab::Accounts
            } else {
                Tab::General
            },
            signature,
            pending_focus: None,
            armed: None,
            slides: std::collections::HashMap::new(),
            mail_app: None,
            making_default: false,
            _activation: None,
            _subscriptions: subscriptions,
        }
    }

    /// Asks macOS which app opens email links, off the main thread.
    fn read_mail_app(&mut self, cx: &mut Context<Self>) {
        if !bundled() {
            return;
        }
        // Badges may have been turned on or off in System Settings meanwhile too.
        crate::notify::refresh_settings();
        cx.spawn(async move |this, cx| {
            let app = cx
                .background_spawn(async { crate::integration::default_mail_app() })
                .await;
            this.update(cx, |this, cx| {
                if this.mail_app != app {
                    this.mail_app = app;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn make_default_mail_app(&mut self, cx: &mut Context<Self>) {
        if self.making_default {
            return;
        }
        self.making_default = true;
        cx.notify();
        // This copy, by its own path: GPUI's register_url_scheme takes whichever copy macOS
        // finds for the bundle id, which may be an older one.
        let done = crate::integration::make_default_mail_app();
        cx.spawn(async move |this, cx| {
            let result = done.await.unwrap_or_else(|_| Err("failed".into()));
            this.update(cx, |this, cx| {
                this.making_default = false;
                // Empty: the user chose to keep their current app when macOS asked.
                if matches!(&result, Err(message) if !message.is_empty()) {
                    this.owner.update(cx, |n, cx| {
                        n.notice = Some(
                            "macOS did not make Nimbra the default mail app. Try again in a moment."
                                .into(),
                        );
                        cx.notify();
                    });
                }
                this.read_mail_app(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn signature_changed(&mut self, cx: &mut Context<Self>) {
        let text = self
            .signature
            .iter()
            .map(|line| line.read(cx).text().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .to_string();
        self.owner.update(cx, |n, cx| {
            if n.settings.signature != text {
                n.settings.signature = text;
                n.save_settings(cx);
            }
        });
    }

    /// Changes a preference and saves it.
    fn set(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Nimbra)) {
        self.owner.update(cx, |n, cx| {
            change(n);
            n.save_settings(cx);
            cx.notify();
        });
    }

    fn is_armed(&self, key: &str) -> bool {
        self.armed
            .is_some_and(|(k, at)| k == key && at.elapsed().as_secs() < 4)
    }

    fn arm(&mut self, key: &'static str, cx: &mut Context<Self>) -> bool {
        if self.is_armed(key) {
            self.armed = None;
            return true;
        }
        self.armed = Some((key, Instant::now()));
        cx.notify();
        false
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self._activation.is_none() {
            self.read_mail_app(cx);
            self._activation = Some(cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.read_mail_app(cx);
                }
            }));
        }
        if let Some(focus) = self.pending_focus.take() {
            window.focus(&focus);
        }
        // The Add account form (it belongs to the app) wants the keyboard.
        let form = self.owner.update(cx, |n, _| n.account_form_in_settings());
        if let Some(Some(focus)) = &form {
            window.focus(focus);
        }
        let dark = self.owner.read(cx).dark;
        let t = if dark { Theme::dark() } else { Theme::light() };
        let tab = self.tab;
        let content = match tab {
            Tab::General => self.general(&t, cx),
            Tab::Accounts => self.accounts(&t, cx),
            Tab::Appearance => self.appearance(&t, cx),
            Tab::Reading => self.reading(&t, cx),
            Tab::Composing => self.composing(&t, cx),
            Tab::Notifications => self.notifications(&t, cx),
            Tab::Privacy => self.privacy(&t, cx),
            Tab::Shortcuts => self.shortcuts(&t),
            Tab::About => self.about(&t, cx),
        };
        let hover = t.hover;
        div()
            .flex()
            .size_full()
            .bg(t.surface)
            .font_family(crate::theme::SANS)
            .text_size(px(13.))
            .line_height(px(18.))
            .text_color(t.text)
            // The sections.
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .w(px(210.))
                    .h_full()
                    .px(px(12.))
                    .pt(px(52.))
                    .gap(px(2.))
                    .bg(t.sidebar)
                    .border_r_1()
                    .border_color(t.border)
                    .children(Tab::ALL.iter().enumerate().map(|(i, item)| {
                        let item = *item;
                        let on = item == tab;
                        div()
                            .id(("settings-tab", i))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .px(px(10.))
                            .h(px(32.))
                            .rounded(px(8.))
                            .cursor_pointer()
                            .when(on, |d| d.bg(t.selected).font_weight(FontWeight::MEDIUM))
                            .when(!on, |d| d.hover(move |s| s.bg(hover)))
                            .press(&t)
                            .child(icon(item.icon(), 16., if on { t.violet } else { t.text2 }))
                            .child(
                                div()
                                    .text_color(if on { t.text } else { t.text2 })
                                    .child(item.label()),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.tab = item;
                                this.armed = None;
                                cx.notify();
                            }))
                    })),
            )
            // The section itself.
            .child(
                div()
                    .id("settings-content")
                    .flex_1()
                    .min_w(px(0.))
                    .h_full()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(18.))
                            // A definite width, so wrapped text is measured at its real width.
                            .w_full()
                            .px(px(40.))
                            .map(|d| match tab {
                                // In the middle of the pane, whatever the window's size
                                // (and scrolling when it is too short).
                                // (A percentage height does not resolve inside a scroll.)
                                Tab::About => d
                                    .min_h(window.viewport_size().height)
                                    .justify_center()
                                    .py(px(40.)),
                                _ => d.max_w(px(680.)).pt(px(44.)).pb(px(40.)),
                            })
                            .when(tab != Tab::About, |d| {
                                d.child(serif(tab.label(), 34., t.text))
                            })
                            .children(content),
                    ),
            )
    }
}

// ---------------------------------------------------------------- building blocks

/// A group of rows on a card.
fn group(t: &Theme, title: Option<&str>, rows: Vec<AnyElement>) -> AnyElement {
    let count = rows.len();
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .children(title.map(|title| {
            div()
                .pl(px(4.))
                .child(overline(title.to_uppercase(), t.text3))
        }))
        .child(
            div()
                .flex()
                .flex_col()
                .rounded(px(14.))
                .bg(t.raised)
                .border_1()
                .border_color(t.border)
                .children(rows.into_iter().enumerate().map(move |(i, row)| {
                    div()
                        .when(i + 1 < count, |d| d.border_b_1().border_color(t.border))
                        .child(row)
                })),
        )
        .into_any_element()
}

/// A row: label and explanation on the left, the control on the right.
fn row(t: &Theme, label: &str, detail: &str, control: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(16.))
        // One height for every row of a card, whatever its control.
        .min_h(px(48.))
        .px(px(16.))
        .py(px(10.))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(label.to_string()),
                )
                .when(!detail.is_empty(), |d| {
                    d.child(meta(detail.to_string(), t.text3))
                }),
        )
        .child(control)
        .into_any_element()
}

/// An on/off switch.
fn switch(t: &Theme, on: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .w(px(36.))
        .h(px(21.))
        // A border either way, so the knob keeps the same inset on and off.
        .p(px(1.))
        .border_1()
        .rounded_full()
        .cursor_pointer()
        .when(on, |d| {
            d.justify_end()
                .bg(t.violet)
                .border_color(gpui::transparent_black())
        })
        .when(!on, |d| d.bg(t.hover).border_color(t.border_strong))
        .child(
            div()
                .size(px(17.))
                .rounded_full()
                .bg(gpui::white())
                .shadow(vec![shadow(c(0x000000, 0.25), 1., 2., 0.)]),
        )
}

impl SettingsWindow {
    /// A switch row that flips a preference.
    fn toggle(
        &self,
        t: &Theme,
        id: &'static str,
        label: &str,
        detail: &str,
        on: bool,
        cx: &mut Context<Self>,
        flip: impl Fn(&mut Nimbra) + 'static,
    ) -> AnyElement {
        row(
            t,
            label,
            detail,
            switch(t, on)
                .id(id)
                .on_click(cx.listener(move |this, _, _, cx| this.set(cx, |n| flip(n)))),
        )
    }

    /// A segmented choice; `pick` receives the index chosen. Like the inbox's All/Unread
    /// switch, the highlight slides from the old option to the new one.
    fn choice(
        &self,
        t: &Theme,
        id: &'static str,
        options: &[&str],
        selected: usize,
        cx: &mut Context<Self>,
        pick: impl Fn(&mut Nimbra, usize) + Clone + 'static,
    ) -> Div {
        const SIZE: f32 = 12.5;
        const H: f32 = 22.;
        const GAP: f32 = 2.;
        const SLIDE: Duration = Duration::from_millis(260);
        // Each option is as wide as its label (measured) plus padding, so the highlight
        // can move between known places.
        let widths: Vec<f32> = options
            .iter()
            .map(|label| (label_width(cx, label, SIZE) + 22.).ceil())
            .collect();
        let spot = |i: usize| -> (f32, f32) {
            let x: f32 = widths[..i].iter().map(|w| w + GAP).sum();
            (x, widths[i])
        };
        let (to_x, to_w) = spot(selected.min(options.len() - 1));
        let pill = div()
            .absolute()
            .top(px(2.))
            .h(px(H))
            .rounded(px(6.))
            .bg(t.raised)
            .border_1()
            .border_color(t.border_strong)
            .shadow(vec![shadow(
                c(0x000000, if t.dark { 0.25 } else { 0.08 }),
                1.,
                3.,
                0.,
            )]);
        let pill = match self.slides.get(id) {
            Some(&(from, at, n))
                if at.elapsed() < SLIDE && from != selected && !crate::system::reduces_motion() =>
            {
                let (from_x, from_w) = spot(from.min(options.len() - 1));
                pill.with_animation(
                    (id, n),
                    Animation::new(SLIDE).with_easing(ease_out_quint()),
                    move |d, delta| {
                        d.left(px(2. + from_x + (to_x - from_x) * delta))
                            .w(px(from_w + (to_w - from_w) * delta))
                    },
                )
                .into_any_element()
            }
            _ => pill.left(px(2. + to_x)).w(px(to_w)).into_any_element(),
        };
        let hover = t.hover;
        div()
            .relative()
            .flex()
            .flex_none()
            .gap(px(GAP))
            .p(px(2.))
            .rounded(px(8.))
            .bg(t.hover)
            .border_1()
            .border_color(t.border)
            .child(pill)
            .children(options.iter().enumerate().map(|(i, label)| {
                let on = i == selected;
                let pick = pick.clone();
                div()
                    .id((id, i))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .w(px(widths[i]))
                    .h(px(H))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .text_size(px(SIZE))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if on { t.text } else { t.text2 })
                    .when(!on, |d| d.hover(move |s| s.bg(hover)))
                    .child(label.to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if i != selected {
                            let n = this.slides.get(id).map_or(0, |s| s.2) + 1;
                            this.slides.insert(id, (selected, Instant::now(), n));
                        }
                        let pick = pick.clone();
                        this.set(cx, move |n| pick(n, i))
                    }))
            }))
    }

    // ------------------------------------------------------------ sections

    fn general(&mut self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let n = self.owner.read(cx);
        let (login, mark, every) = (n.login_item, n.settings.mark_read, n.settings.check_every);
        let login_row = row(
            t,
            "Open Nimbra at login",
            "",
            switch(t, login)
                .id("login")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.owner.update(cx, |n, cx| {
                        match crate::system::set_open_at_login(!n.login_item) {
                            Ok(()) => n.login_item = !n.login_item,
                            Err(message) => n.notice = Some(message.into()),
                        }
                        cx.notify();
                    });
                })),
        );
        let mark_row = row(
            t,
            "Mark as read",
            "",
            self.choice(
                t,
                "mark",
                &["When opened", "After 2 seconds", "Manually"],
                match mark {
                    MarkRead::Open => 0,
                    MarkRead::Delay => 1,
                    MarkRead::Never => 2,
                },
                cx,
                |n, i| n.settings.mark_read = [MarkRead::Open, MarkRead::Delay, MarkRead::Never][i],
            ),
        );
        let every_row = row(
            t,
            "Check for new mail",
            "",
            self.choice(
                t,
                "every",
                &["30 s", "1 min", "5 min"],
                match every {
                    0..=45 => 0,
                    46..=150 => 1,
                    _ => 2,
                },
                cx,
                |n, i| n.settings.check_every = [30, 60, 300][i],
            ),
        );
        let mut rows = vec![login_row, mark_row, every_row];
        if bundled() {
            rows.insert(0, self.mail_app_row(t, cx));
        }
        vec![group(t, None, rows)]
    }

    fn mail_app_row(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let control = match &self.mail_app {
            None => div(),
            Some(app) if app.ours => div().child(meta("Nimbra", t.text2)),
            Some(app) => div()
                .flex()
                .items_center()
                .gap(px(12.))
                .when(!app.name.is_empty(), |d| {
                    d.child(meta(app.name.clone(), t.text3))
                })
                .child(
                    button(t, ButtonKind::Secondary, "Make default", None)
                        .id("default-mail-app")
                        .press_soft()
                        .when(self.making_default, |b| b.opacity(0.5))
                        .on_click(cx.listener(|this, _, _, cx| this.make_default_mail_app(cx))),
                ),
        };
        row(t, "Default mail app", "", control)
    }

    fn accounts(&mut self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        // Adding an account: the form, here in Settings, instead of the list.
        if self
            .owner
            .read(cx)
            .account_sheet
            .as_ref()
            .is_some_and(|s| s.in_settings)
        {
            return self
                .owner
                .update(cx, |n, cx| n.account_settings_page(t, cx));
        }
        let n = self.owner.read(cx);
        let accounts = n.settings.accounts.clone();
        let active = n.account_id.clone();
        let connected = n.connection.is_some();
        // Google's sign-in needs its setup; any other address does not.
        let can_add = true;
        let active_email = n.account.to_string();
        let photos = n.photos.clone();
        let photo_answer = n.photo_answers.get(&active).copied().filter(|_| connected);
        let several = accounts.len() >= 2;
        let default = n.settings.default_account().map(str::to_string);
        let mut rows: Vec<AnyElement> = accounts
            .iter()
            .enumerate()
            .map(|(i, account)| {
                let shown = account.id == active;
                let is_default = several && default.as_deref() == Some(account.id.as_str());
                let id = account.id.clone();
                let make_default = account.id.clone();
                let initials = account
                    .email
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_default();
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(16.))
                    .py(px(12.))
                    .child(if let Some(photo) = photos.get(&account.id) {
                        me_avatar(Some(photo), &initials, 32.)
                    } else if shown {
                        me_avatar(None, &initials, 32.)
                    } else {
                        avatar(crate::data::tone_for(&account.email), &initials, 32.)
                            .into_any_element()
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(account.email.clone()),
                            )
                            .child(meta(
                                {
                                    // Gmail, or the IMAP server it reads from.
                                    let provider = account
                                        .servers
                                        .as_ref()
                                        .map_or("Gmail".to_string(), |s| s.imap.host.clone());
                                    match (is_default, shown) {
                                        (true, true) => format!("Default · Shown now · {provider}"),
                                        (true, false) => format!("Default · {provider}"),
                                        (false, true) => format!("Shown now · {provider}"),
                                        (false, false) => provider,
                                    }
                                },
                                if shown || is_default {
                                    t.violet
                                } else {
                                    t.text3
                                },
                            )),
                    )
                    .when(several && !is_default, |d| {
                        d.child(
                            button(t, ButtonKind::Ghost, "Make default", None)
                                .id(("default-account", i))
                                .press_soft()
                                .tooltip(tip(
                                    t.dark,
                                    "Its profile in the sidebar, and new messages from it",
                                    "",
                                ))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let id = make_default.clone();
                                    this.owner.update(cx, |n, cx| n.set_default_account(id, cx));
                                })),
                        )
                    })
                    .when(!shown, |d| {
                        d.child(
                            button(t, ButtonKind::Secondary, "Switch to", None)
                                .id(("switch-account", i))
                                .press_soft()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let id = id.clone();
                                    this.owner.update(cx, |n, cx| n.switch_account(id, cx));
                                })),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        if rows.is_empty() {
            rows.push(row(t, "No account yet", "", div()));
        }
        let armed = self.is_armed("sign-out");
        let actions = div()
            .flex()
            .gap(px(8.))
            .when(can_add, |d| {
                d.child(
                    button(t, ButtonKind::Secondary, "Add account", None)
                        .id("add-account")
                        .press_soft()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.tab = Tab::Accounts;
                            this.owner
                                .update(cx, |n, cx| n.open_account_form_in_settings(cx));
                        })),
                )
            })
            .when(connected, |d| {
                d.child(
                    button(
                        t,
                        ButtonKind::Ghost,
                        if armed {
                            "Click again to sign out"
                        } else {
                            "Sign out of this account"
                        },
                        None,
                    )
                    .id("sign-out")
                    .press_soft()
                    .when(armed, |d| d.text_color(t.rose))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.arm("sign-out", cx) {
                            this.owner.update(cx, |n, cx| n.sign_out(cx));
                        }
                    })),
                )
            });
        let check_again = |cx: &mut Context<Self>| {
            button(t, ButtonKind::Secondary, "Check again", None)
                .id("check-photo")
                .press_soft()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.owner.update(cx, |n, cx| n.check_photo_again(cx));
                }))
        };
        match photo_answer {
            Some(PhotoAnswer::NotAllowed) => rows.push(row(
                t,
                "Show your Google photo",
                "Allow your basic profile once in your browser, with this same account.",
                button(t, ButtonKind::Secondary, "Allow", None)
                    .id("allow-photo")
                    .press_soft()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.owner.update(cx, |n, cx| n.allow_profile_photo(cx));
                    })),
            )),
            Some(PhotoAnswer::Unchecked) => rows.push(row(
                t,
                "A letter instead of your photo?",
                "Google gives every app this picture. Your own needs the People API on in Nimbra's Google Cloud project.",
                check_again(cx),
            )),
            Some(PhotoAnswer::None) => rows.push(row(
                t,
                "Google has no photo for this account",
                "If you set one, your Workspace admin may be keeping it from apps.",
                check_again(cx),
            )),
            Some(PhotoAnswer::Theirs) | None => {}
        }
        vec![
            group(t, None, rows),
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(actions)
                // The consequence, at the moment it matters.
                .when(connected && armed, |d| {
                    d.child(meta(
                        format!(
                            "Signing out of {} removes its mail from this Mac. Other accounts stay.",
                            if active_email.is_empty() { "this account".to_string() } else { active_email }
                        ),
                        t.text3,
                    ))
                })
                .into_any_element(),
        ]
    }

    fn appearance(&mut self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let n = self.owner.read(cx);
        let (theme, size) = (n.settings.theme, n.settings.reader_text);
        // Small previews of each look.
        let tile =
            |id: usize, label: &'static str, choice: ThemeChoice, light: bool, dark: bool| {
                let on = theme == choice;
                // GPUI clips to a rectangle, not to the rounded tile: the preview's own outer
                // corners are rounded to fit inside the border (10 less its 2).
                const INNER: f32 = 8.;
                let half = |th: Theme, left: bool, right: bool| {
                    div()
                        .flex_1()
                        // Its share of the tile, whatever it holds: wider, its rounded
                        // corner would fall outside the tile and the square middle show.
                        .min_w(px(0.))
                        .overflow_hidden()
                        .h_full()
                        .flex()
                        .bg(th.surface)
                        .when(left, |d| d.rounded_l(px(INNER)))
                        .when(right, |d| d.rounded_r(px(INNER)))
                        .child(
                            div()
                                .flex_none()
                                .w(px(22.))
                                .h_full()
                                .bg(th.sidebar)
                                .when(left, |d| d.rounded_l(px(INNER)))
                                .border_r_1()
                                .border_color(th.border),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .flex()
                                .flex_col()
                                .gap(px(5.))
                                .p(px(8.))
                                .child(div().h(px(5.)).w(px(40.)).rounded_full().bg(th.text2))
                                .child(
                                    div()
                                        .h(px(4.))
                                        .w(px(56.))
                                        .rounded_full()
                                        .bg(th.border_strong),
                                )
                                .child(
                                    div()
                                        .h(px(4.))
                                        .w(px(48.))
                                        .rounded_full()
                                        .bg(th.border_strong),
                                ),
                        )
                };
                div()
                    .id(("theme-tile", id))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .cursor_pointer()
                    .child(
                        div()
                            .flex()
                            .w(px(150.))
                            .h(px(92.))
                            .rounded(px(10.))
                            .overflow_hidden()
                            .border_2()
                            .border_color(if on { t.violet } else { t.border })
                            .when(light, |d| d.child(half(Theme::light(), true, !dark)))
                            .when(dark, |d| d.child(half(Theme::dark(), !light, true))),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_center()
                            .text_color(if on { t.text } else { t.text2 })
                            .when(on, |d| d.font_weight(FontWeight::MEDIUM))
                            .child(label),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.owner.update(cx, |n, cx| n.set_theme(choice, cx));
                    }))
            };
        let tiles = div()
            .flex()
            .gap(px(16.))
            .p(px(16.))
            .child(tile(0, "System", ThemeChoice::System, true, true))
            .child(tile(1, "Light", ThemeChoice::Light, true, false))
            .child(tile(2, "Dark", ThemeChoice::Dark, false, true))
            .into_any_element();
        let size_row = row(
            t,
            "Reader text size",
            "",
            self.choice(
                t,
                "text-size",
                &["Small", "Default", "Large"],
                match size {
                    TextSize::Small => 0,
                    TextSize::Default => 1,
                    TextSize::Large => 2,
                },
                cx,
                |n, i| {
                    n.settings.reader_text =
                        [TextSize::Small, TextSize::Default, TextSize::Large][i]
                },
            ),
        );
        vec![
            group(t, Some("Theme"), vec![tiles]),
            group(t, None, vec![size_row]),
        ]
    }

    fn reading(&mut self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let n = self.owner.read(cx);
        let (original, images, dark_emails) = (
            n.settings.original_view,
            n.settings.load_images,
            n.settings.dark_emails,
        );
        let mut rows = vec![
            self.toggle(
                t,
                "original",
                "Show original formatting",
                "Press V to switch one message.",
                original,
                cx,
                |n| {
                    n.settings.original_view = !n.settings.original_view;
                    n.original = n.settings.original_view;
                },
            ),
            self.toggle(
                t,
                "images",
                "Load remote images",
                "Images can tell senders you opened their mail.",
                images,
                cx,
                |n| {
                    n.settings.load_images = !n.settings.load_images;
                    n.original_images = n.settings.load_images;
                },
            ),
        ];
        // Darkening needs WebKit (see webview.rs), so the switch is only where it works.
        if cfg!(target_os = "macos") {
            rows.insert(
                1,
                self.toggle(
                    t,
                    "dark-emails",
                    "Darken emails in dark theme",
                    "Pictures keep their colours.",
                    dark_emails,
                    cx,
                    |n| n.settings.dark_emails = !n.settings.dark_emails,
                ),
            );
        }
        vec![group(t, None, rows)]
    }

    fn composing(&mut self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let n = self.owner.read(cx);
        let (undo, quote) = (n.settings.undo_send, n.settings.quote_replies);
        let used = self
            .signature
            .iter()
            .rposition(|line| !line.read(cx).text().is_empty())
            .map_or(0, |last| last + 1);
        let signature = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(16.))
            .child(div().font_weight(FontWeight::MEDIUM).child("Signature"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(10.))
                    .bg(t.hover)
                    .border_1()
                    .border_color(t.border)
                    .text_size(px(14.))
                    .line_height(px(22.))
                    .children(self.signature.iter().take((used + 1).max(3)).cloned()),
            )
            .into_any_element();
        let undo_row = row(
            t,
            "Undo send",
            "",
            self.choice(
                t,
                "undo",
                &["Off", "5 s", "10 s", "20 s", "30 s"],
                match undo {
                    0 => 0,
                    1..=7 => 1,
                    8..=14 => 2,
                    15..=24 => 3,
                    _ => 4,
                },
                cx,
                |n, i| n.settings.undo_send = [0, 6, 10, 20, 30][i],
            ),
        );
        let quote_row = self.toggle(
            t,
            "quote",
            "Include the original in replies",
            "",
            quote,
            cx,
            |n| n.settings.quote_replies = !n.settings.quote_replies,
        );
        vec![
            group(t, None, vec![signature]),
            group(t, None, vec![undo_row, quote_row]),
        ]
    }

    fn notifications(&mut self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let n = self.owner.read(cx);
        let (on, sound, badge) = (
            n.settings.notifications,
            n.settings.notify_sound,
            n.settings.dock_badge,
        );
        // Said only when macOS is the reason nothing shows (read again on activation).
        const BLOCKED: &str = "Turned off for Nimbra in System Settings › Notifications.";
        let notify_row = self.toggle(
            t,
            "notify",
            "Notify me about new mail",
            if crate::notify::alerts_allowed() {
                ""
            } else {
                BLOCKED
            },
            on,
            cx,
            |n| n.settings.notifications = !n.settings.notifications,
        );
        // A sound belongs to a notification: with none, the switch is there but inert.
        let sound_row = if on {
            self.toggle(t, "sound", "Play a sound", "", sound, cx, |n| {
                n.settings.notify_sound = !n.settings.notify_sound
            })
        } else {
            div()
                .opacity(0.45)
                .child(row(
                    t,
                    "Play a sound",
                    "",
                    switch(t, sound).cursor_default(),
                ))
                .into_any_element()
        };
        let badge_row = self.toggle(
            t,
            "dock-badge",
            "Badge app icon",
            if crate::notify::badges_allowed() {
                ""
            } else {
                BLOCKED
            },
            badge,
            cx,
            |n| n.settings.dock_badge = !n.settings.dock_badge,
        );
        vec![group(t, None, vec![notify_row, sound_row, badge_row])]
    }

    fn privacy(&mut self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let armed = self.is_armed("clear");
        let counting = self.owner.read(cx).settings.usage_counts;
        let usage_row = self.toggle(
            t,
            "usage-counts",
            "Anonymous usage counts",
            "Each launch sends one count with the app and macOS versions: no ID, no address, nothing about your mail. It tells us how many people use Nimbra.",
            counting,
            cx,
            |n| n.settings.usage_counts = !n.settings.usage_counts,
        );
        let fact = |name: &'static str, title: &'static str, text: &'static str| {
            div()
                .flex()
                .gap(px(12.))
                .px(px(16.))
                .py(px(12.))
                // Level with the title's 18pt line.
                .child(div().pt(px(1.)).child(icon(name, 16., t.violet)))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(div().font_weight(FontWeight::MEDIUM).child(title))
                        .child(meta(text, t.text3)),
                )
                .into_any_element()
        };
        vec![
            group(
                t,
                None,
                vec![
                    fact(
                        "lock",
                        "Encrypted on disk",
                        "Mail on this Mac is locked with a key kept only in your Keychain.",
                    ),
                    fact(
                        "key",
                        "Sign-ins in the Keychain",
                        "Passwords and sign-ins are never written to a file.",
                    ),
                    fact(
                        "mail",
                        "Only what it needs",
                        "Nimbra asks to read, organise and send. Deleting forever asks once.",
                    ),
                    fact(
                        "eye",
                        "Mail is shown safely",
                        "Scripts never run, images wait until allowed, links open in your browser.",
                    ),
                ],
            ),
            group(
                t,
                None,
                vec![row(
                    t,
                    "Clear mail kept on this Mac",
                    "Your mail stays on the server and downloads again as needed.",
                    button(
                        t,
                        ButtonKind::Secondary,
                        if armed {
                            "Click again to clear"
                        } else {
                            "Clear"
                        },
                        None,
                    )
                    .id("clear-store")
                    .press_soft()
                    .when(armed, |d| d.text_color(t.rose))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.arm("clear", cx) {
                            this.owner.update(cx, |n, cx| n.clear_local_mail(cx));
                        }
                    })),
                )],
            ),
            group(t, None, vec![usage_row]),
        ]
    }

    fn shortcuts(&self, t: &Theme) -> Vec<AnyElement> {
        let keys: &[(&str, &[(&str, &str)])] = &[
            (
                "Reading",
                &[
                    ("J / ↓", "Next conversation"),
                    ("K / ↑", "Previous conversation"),
                    ("Return", "Full view"),
                    ("V", "Original formatting or reader"),
                    ("⌘[ / ⌘]", "Back / forward"),
                ],
            ),
            (
                "Organising",
                &[
                    ("E", "Mark done"),
                    ("U", "Read or unread"),
                    ("S", "Star"),
                    ("H", "Snooze"),
                    ("⌫", "Move to trash"),
                    ("Z / ⌘Z", "Undo"),
                    ("⌘A", "Select all"),
                ],
            ),
            (
                "Writing",
                &[
                    ("C / ⌘N", "New message"),
                    ("R", "Reply"),
                    ("A", "Reply all"),
                    ("F", "Forward"),
                    ("⌘↵", "Send"),
                    ("⌘S", "Save draft"),
                    ("⌘⇧F", "Full page composer"),
                    ("⌘B ⌘I ⌘U ⌘K", "Bold, italic, underline, link"),
                ],
            ),
            (
                "App",
                &[
                    ("/ or ⌘F", "Search"),
                    ("⌘K", "Snooze"),
                    ("⌘R", "Check for mail"),
                    ("⌘B / ⌘⇧B", "Sidebar (auto-hide) / message list"),
                    ("⌘⇧L", "Switch light and dark"),
                    ("⌘,", "Settings"),
                    ("⌘P", "Print"),
                    ("⌘H / ⌘M / ⌘W", "Hide / minimise / close window"),
                ],
            ),
        ];
        keys.iter()
            .map(|(title, items)| {
                group(
                    t,
                    Some(title),
                    items
                        .iter()
                        .map(|(key, what)| {
                            div()
                                .flex()
                                .items_center()
                                .px(px(16.))
                                .py(px(9.))
                                .child(div().flex_1().text_color(t.text2).child(*what))
                                .child(kbd(t, key))
                                .into_any_element()
                        })
                        .collect(),
                )
            })
            .collect()
    }

    fn about(&mut self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        vec![
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(14.))
                .child(
                    div()
                        .size(px(ABOUT_ICON))
                        // The glow follows the icon's own shape, which the image brings.
                        .rounded(px(ABOUT_ICON / 4.))
                        .shadow(vec![shadow(c(0x8b7cff, 0.45), 14., 40., -6.)])
                        // The icon as macOS shows it in the same appearance.
                        .child(
                            gpui::img(if t.dark {
                                APP_ICON_DARK.clone()
                            } else {
                                APP_ICON.clone()
                            })
                            .size_full(),
                        ),
                )
                .child(serif("Nimbra", 44., t.text))
                .child(meta(
                    format!("Version {}", env!("CARGO_PKG_VERSION")),
                    t.text3,
                ))
                .child(
                    button(t, ButtonKind::Secondary, "Keyboard shortcuts", None)
                        .id("about-shortcuts")
                        .press_soft()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.tab = Tab::Shortcuts;
                            cx.notify();
                        })),
                )
                .into_any_element(),
        ]
    }
}

/// How wide `text` is in the app's medium-weight sans at `size` points.
fn label_width(cx: &App, text: &str, size: f32) -> f32 {
    let system = cx.text_system();
    let mut font = gpui::font(crate::theme::SANS);
    font.weight = FontWeight::MEDIUM;
    let font_id = system.resolve_font(&font);
    text.chars()
        .map(|c| {
            system
                .advance(font_id, px(size), c)
                .map_or(size * 0.55, |a| f32::from(a.width))
        })
        .sum()
}

/// Running as Nimbra.app, which is what macOS registers handlers and defaults for.
fn bundled() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::notify::available()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}
