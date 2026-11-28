//! The Settings window: its own window with a sidebar of sections. It reads and changes the
//! app's state (the `Nimbra` entity) and saves the preferences as they change.

use std::time::Instant;

use super::PhotoAnswer;
use super::*;
use crate::settings::{MarkRead, TextSize, ThemeChoice};

/// The app's icon on About, in points.
const ABOUT_ICON: f32 = 96.;
/// The Default mail app row's control, in every state (the Make default button, the badge,
/// or nothing yet): the height that keeps it a 48 pt row like the others (see `row`).
const MAIL_APP_CONTROL: f32 = 28.;

/// The app's icon in its light (Default) and Dark appearances, rendered from
/// `assets/AppIcon.icon` (README: how to render them again).
pub(super) static APP_ICON: std::sync::LazyLock<std::sync::Arc<gpui::Image>> =
    std::sync::LazyLock::new(|| {
        std::sync::Arc::new(gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            include_bytes!("../assets/app-icon.png").to_vec(),
        ))
    });
pub(super) static APP_ICON_DARK: std::sync::LazyLock<std::sync::Arc<gpui::Image>> =
    std::sync::LazyLock::new(|| {
        std::sync::Arc::new(gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            include_bytes!("../assets/app-icon-dark.png").to_vec(),
        ))
    });

/// The screenshot states that open Settings (`NIMBRA_SCREEN`), and the section each opens
/// on. One list, read both where the app opens the window and where it picks the section:
/// a state named in one place and not the other opened the window on General.
const SCREENS: &[(&str, Tab)] = &[
    ("settings", Tab::General),
    ("settings-account", Tab::Accounts),
    ("settings-accounts", Tab::Accounts),
    ("settings-remove", Tab::Accounts),
    ("settings-appearance", Tab::Appearance),
    ("settings-reading", Tab::Reading),
    ("settings-composing", Tab::Composing),
    ("settings-composing-html", Tab::Composing),
    ("settings-composing-preview", Tab::Composing),
    ("settings-notifications", Tab::Notifications),
    ("settings-appearance", Tab::Appearance),
    ("settings-privacy", Tab::Privacy),
    ("settings-shortcuts", Tab::Shortcuts),
    ("settings-about", Tab::About),
];

/// The section a screenshot state opens Settings on, when it opens Settings.
pub(super) fn screen_tab(screen: &str) -> Option<Tab> {
    SCREENS
        .iter()
        .find(|(name, _)| *name == screen)
        .map(|(_, tab)| *tab)
}

/// How a signature is edited in Settings › Composing.
#[derive(Clone, Copy, PartialEq)]
enum SigMode {
    /// Formatted text, in the composer's own editor.
    Text,
    /// The HTML source.
    Html,
    /// Drawn natively, read-only, as a reader would see it (images as placeholders).
    Preview,
}

/// The chip picked among an account's signatures.
#[derive(Clone, PartialEq)]
enum SigPick {
    Signature(String),
    /// "No signature".
    Nothing,
}

/// The signature editor's height: room for a long signature without the page scrolling.
const SIG_EDITOR_HEIGHT: f32 = 190.;

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
    pending_focus: Option<FocusHandle>,
    /// Signatures (Composing): the account shown, `None` for the default account (or the
    /// first).
    sig_account: Option<String>,
    /// The chip picked; `None` until one is, which shows the account's default.
    sig_pick: Option<SigPick>,
    sig_mode: SigMode,
    /// The picked signature's name, and which signature it holds.
    sig_name: Entity<TextInput>,
    sig_name_for: Option<String>,
    /// The HTML source as typed, for the signature it belongs to: what is kept is that,
    /// cleaned, and the source view is not loaded again with the cleaned copy while typing.
    sig_source: Option<(String, String)>,
    /// The signature asking to be deleted (id, name), in a dialog.
    confirm_delete: Option<(String, String)>,
    /// The formatted text and HTML editor: the composer's (a web view), a second one, made
    /// the first time it is needed.
    sig_editor: Option<std::rc::Rc<crate::editor::EditorView>>,
    sig_editor_failed: bool,
    sig_editor_ready: bool,
    /// The signature editor's page has drawn since it was made (hidden until then, so its
    /// blank first frames never flash).
    sig_painted: bool,
    sig_editor_dark: Option<bool>,
    /// What the editor shows (signature id, mode), and the session it was loaded as.
    sig_loaded: Option<(String, SigMode)>,
    sig_session: u64,
    sig_shown: bool,
    /// GPUI's keyboard focus while typing in the editor (its "Editor" key context), and
    /// whether it had it at the last frame.
    sig_editor_focus: FocusHandle,
    sig_keys: bool,
    /// The account field's place in the window (left, bottom, width), where its list opens.
    sig_anchor: std::rc::Rc<std::cell::Cell<(f32, f32, f32)>>,
    sig_picker: Option<gpui::WindowHandle<AccountPicker>>,
    /// A first click on a destructive button arms it; a second within 4 s does it.
    armed: Option<(&'static str, Instant)>,
    /// The account whose Remove asks for confirmation (id, address), in a dialog.
    confirm_remove: Option<(String, String)>,
    /// The dialog's keyboard focus, for Esc and Return.
    dialog_focus: FocusHandle,
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
    /// `dark` comes from the app (which is busy opening this window, so it cannot be read
    /// here). The window opens on General; `show` moves it to another section.
    pub(super) fn new(owner: Entity<Nimbra>, dark: bool, cx: &mut Context<Self>) -> Self {
        let t = if dark { Theme::dark() } else { Theme::light() };
        let subscriptions = vec![cx.observe(&owner, |_, _, cx| cx.notify())];
        let screen = std::env::var("NIMBRA_SCREEN").unwrap_or_default();
        // NIMBRA_SCREEN=settings-remove opens Accounts asking to remove the first one.
        let asking = (screen == "settings-remove")
            .then(|| owner.read(cx).settings.accounts.first().cloned())
            .flatten()
            .map(|a| (a.id, a.email));
        // The HTML and Preview screenshots show the sample's signature in HTML (its second).
        let sig_mode = match screen.as_str() {
            "settings-composing-html" => SigMode::Html,
            "settings-composing-preview" => SigMode::Preview,
            _ => SigMode::Text,
        };
        let sig_pick = (sig_mode != SigMode::Text)
            .then(|| owner.read(cx).settings.signatures.get(1).cloned())
            .flatten()
            .map(|s| SigPick::Signature(s.id));
        let sig_name = cx.new(|cx| TextInput::new(cx, "Untitled", t.faint, t.violet));
        let mut subscriptions = subscriptions;
        subscriptions.push(cx.subscribe(&sig_name, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Changed) {
                this.name_changed(cx);
            }
        }));
        SettingsWindow {
            owner,
            tab: Tab::General,
            pending_focus: None,
            sig_account: None,
            sig_pick,
            sig_mode,
            sig_name,
            sig_name_for: None,
            sig_source: None,
            confirm_delete: None,
            sig_editor: None,
            sig_editor_failed: false,
            sig_editor_ready: false,
            sig_painted: false,
            sig_editor_dark: None,
            sig_loaded: None,
            sig_session: 0,
            sig_shown: false,
            sig_editor_focus: cx.focus_handle(),
            sig_keys: false,
            sig_anchor: Default::default(),
            sig_picker: None,
            armed: None,
            confirm_remove: asking,
            dialog_focus: cx.focus_handle(),
            slides: std::collections::HashMap::new(),
            mail_app: None,
            making_default: false,
            _activation: None,
            _subscriptions: subscriptions,
        }
    }

    /// Moves to a section; for Composing, to `account`'s signatures when one is given.
    pub(super) fn show(&mut self, tab: Tab, account: Option<String>, cx: &mut Context<Self>) {
        self.tab = tab;
        self.armed = None;
        if let Some(account) = account
            && self.sig_account.as_ref() != Some(&account)
        {
            self.sig_account = Some(account);
            self.sig_pick = None;
        }
        cx.notify();
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

    /// Remove: asks first, in a dialog.
    fn ask_remove(&mut self, id: String, email: String, cx: &mut Context<Self>) {
        self.confirm_remove = Some((id, email));
        self.pending_focus = Some(self.dialog_focus.clone());
        cx.notify();
    }

    fn cancel_remove(&mut self, cx: &mut Context<Self>) {
        self.confirm_remove = None;
        cx.notify();
    }

    fn confirm_remove(&mut self, cx: &mut Context<Self>) {
        if let Some((id, _)) = self.confirm_remove.take() {
            self.owner.update(cx, |n, cx| n.remove_account(id, cx));
        }
        cx.notify();
    }

    /// The dialog over the window: what goes and what stays, Cancel and Remove.
    fn remove_dialog(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (_, email) = self.confirm_remove.as_ref()?;
        let danger = div()
            .id("confirm-remove")
            .flex()
            .flex_none()
            .items_center()
            .px(dp(14.))
            .py(dp(6.))
            .rounded(dp(9.))
            .bg(t.rose)
            .text_color(gpui::white())
            .font_weight(FontWeight::SEMIBOLD)
            .cursor_pointer()
            .hover(|s| s.opacity(0.9))
            .child("Sign out")
            .on_click(cx.listener(|this, _, _, cx| this.confirm_remove(cx)));
        Some(
            div()
                .id("remove-dialog-scrim")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(t.scrim)
                .occlude()
                .track_focus(&self.dialog_focus)
                .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => this.cancel_remove(cx),
                        "enter" => this.confirm_remove(cx),
                        _ => {}
                    }
                }))
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, _, cx| this.cancel_remove(cx)),
                )
                .child(
                    div()
                        .id("remove-dialog")
                        .w(dp(400.))
                        .flex()
                        .flex_col()
                        .gap(dp(10.))
                        .p(dp(24.))
                        .rounded(dp(16.))
                        .bg(t.raised)
                        .border_1()
                        .border_color(t.border_strong)
                        .shadow(vec![shadow(
                            c(0x000000, if t.dark { 0.6 } else { 0.2 }),
                            24.,
                            60.,
                            -10.,
                        )])
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .text_size(dp(15.))
                                .line_height(dp(20.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("Sign out of {email}?")),
                        )
                        .child(
                            div().text_color(t.text2).child(
                                "Nimbra signs out of it and deletes its mail from this Mac. \
                                 Its mail on the server, and your other accounts, stay as they are.",
                            ),
                        )
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap(dp(8.))
                                .pt(dp(10.))
                                .child(
                                    button(t, ButtonKind::Secondary, "Cancel", None)
                                        .id("cancel-remove")
                                        .press_soft()
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.cancel_remove(cx)),
                                        ),
                                )
                                .child(danger),
                        ),
                )
                .into_any_element(),
        )
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
        // Every window draws at the interface zoom.
        crate::ui::apply_zoom(window);
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
        self.sync_signature_editor(window, cx);
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
        let dialog = self
            .remove_dialog(&t, cx)
            .or_else(|| self.delete_dialog(&t, cx));
        div()
            .relative()
            .flex()
            .size_full()
            .bg(t.surface)
            .font_family(crate::theme::SANS)
            .text_size(dp(13.))
            .line_height(dp(18.))
            .text_color(t.text)
            // The sections.
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .w(dp(210.))
                    .h_full()
                    .px(dp(12.))
                    .pt(dp(52.))
                    .gap(dp(2.))
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
                            .gap(dp(10.))
                            .px(dp(10.))
                            .h(dp(32.))
                            .rounded(dp(8.))
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
                    .min_w(dp(0.))
                    .h_full()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(dp(18.))
                            // A definite width, so wrapped text is measured at its real width.
                            .w_full()
                            .px(dp(40.))
                            .map(|d| match tab {
                                // In the middle of the pane, whatever the window's size
                                // (and scrolling when it is too short).
                                // (A percentage height does not resolve inside a scroll.)
                                Tab::About => d
                                    .min_h(window.viewport_size().height)
                                    .justify_center()
                                    .py(dp(40.)),
                                _ => d.max_w(dp(680.)).pt(dp(44.)).pb(dp(40.)),
                            })
                            .when(tab != Tab::About, |d| {
                                d.child(serif(tab.label(), 34., t.text))
                            })
                            .children(content),
                    ),
            )
            .children(dialog)
    }
}

// ---------------------------------------------------------------- building blocks

/// A group of rows on a card.
fn group(t: &Theme, title: Option<&str>, rows: Vec<AnyElement>) -> AnyElement {
    let count = rows.len();
    div()
        .flex()
        .flex_col()
        .gap(dp(8.))
        .children(title.map(|title| {
            div()
                .pl(dp(4.))
                .child(overline(title.to_uppercase(), t.text3))
        }))
        .child(
            div()
                .flex()
                .flex_col()
                .rounded(dp(14.))
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
    row_with(
        t,
        div()
            .font_weight(FontWeight::MEDIUM)
            .child(label.to_string())
            .into_any_element(),
        detail,
        control,
    )
}

/// A row whose label is more than its words (a button beside them, say).
fn row_with(t: &Theme, label: AnyElement, detail: &str, control: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(dp(16.))
        // One height for every row of a card, whatever its control.
        .min_h(dp(48.))
        .px(dp(16.))
        .py(dp(10.))
        .child(
            div()
                .flex_1()
                .min_w(dp(0.))
                .flex()
                .flex_col()
                .gap(dp(2.))
                .child(label)
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
        .w(dp(36.))
        .h(dp(21.))
        // A border either way, so the knob keeps the same inset on and off.
        .p(dp(1.))
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
                .size(dp(17.))
                .rounded_full()
                .bg(gpui::white())
                .shadow(vec![shadow(c(0x000000, 0.25), 1., 2., 0.)]),
        )
}

impl SettingsWindow {
    /// A switch row that flips a preference.
    #[allow(clippy::too_many_arguments)]
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

    /// A segmented choice of a preference; `pick` receives the index chosen and the
    /// preference is saved.
    fn choice(
        &self,
        t: &Theme,
        id: &'static str,
        options: &[&str],
        selected: usize,
        cx: &mut Context<Self>,
        pick: impl Fn(&mut Nimbra, usize) + Clone + 'static,
    ) -> Div {
        self.choice_with(t, id, options, selected, cx, move |this, i, cx| {
            let pick = pick.clone();
            this.set(cx, move |n| pick(n, i))
        })
    }

    /// A segmented choice; `pick` receives the index chosen. Like the inbox's All/Unread
    /// switch, the highlight slides from the old option to the new one.
    fn choice_with(
        &self,
        t: &Theme,
        id: &'static str,
        options: &[&str],
        selected: usize,
        cx: &mut Context<Self>,
        pick: impl Fn(&mut Self, usize, &mut Context<Self>) + Clone + 'static,
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
            .top(dp(2.))
            .h(dp(H))
            .rounded(dp(6.))
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
                        d.left(dp(2. + from_x + (to_x - from_x) * delta))
                            .w(dp(from_w + (to_w - from_w) * delta))
                    },
                )
                .into_any_element()
            }
            _ => pill.left(dp(2. + to_x)).w(dp(to_w)).into_any_element(),
        };
        let hover = t.hover;
        div()
            .relative()
            .flex()
            .flex_none()
            .gap(dp(GAP))
            .p(dp(2.))
            .rounded(dp(8.))
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
                    .w(dp(widths[i]))
                    .h(dp(H))
                    .rounded(dp(6.))
                    .cursor_pointer()
                    .text_size(dp(SIZE))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if on { t.text } else { t.text2 })
                    .when(!on, |d| d.hover(move |s| s.bg(hover)))
                    .child(label.to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if i != selected {
                            let n = this.slides.get(id).map_or(0, |s| s.2) + 1;
                            this.slides.insert(id, (selected, Instant::now(), n));
                        }
                        pick(this, i, cx)
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
                &["10 s", "30 s", "1 min", "5 min"],
                match every {
                    0..=20 => 0,
                    21..=45 => 1,
                    46..=150 => 2,
                    _ => 3,
                },
                cx,
                |n, i| n.settings.check_every = [10, 30, 60, 300][i],
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
            // Done: a soft mint badge, a check in a filled circle and the name.
            Some(app) if app.ours => div()
                .flex()
                .items_center()
                .gap(dp(7.))
                .h(dp(28.))
                .pl(dp(5.))
                .pr(dp(12.))
                .rounded_full()
                .bg(t.mint.opacity(0.1))
                .border_1()
                .border_color(t.mint.opacity(0.28))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(dp(18.))
                        .rounded_full()
                        .bg(t.mint)
                        .child(icon("done", 11., t.surface)),
                )
                .child(
                    div()
                        .text_size(dp(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.text)
                        .child("Nimbra"),
                ),
            Some(app) => div()
                .flex()
                .items_center()
                .gap(dp(12.))
                .when(!app.name.is_empty(), |d| {
                    d.child(meta(app.name.clone(), t.text3))
                })
                .child(
                    button(t, ButtonKind::Secondary, "Make default", None)
                        .id("default-mail-app")
                        .h(dp(MAIL_APP_CONTROL))
                        .press_soft()
                        .when(self.making_default, |b| b.opacity(0.5))
                        .on_click(cx.listener(|this, _, _, cx| this.make_default_mail_app(cx))),
                ),
        };
        // As tall as the button in every state, so the row keeps its height when it goes.
        let control = div()
            .flex()
            .items_center()
            .h(dp(MAIL_APP_CONTROL))
            .child(control);
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
        let photos = n.photos.clone();
        let photo_answer = n.photo_answers.get(&active).copied().filter(|_| connected);
        let several = accounts.len() >= 2;
        let default = n.settings.default_account().map(str::to_string);
        // A row about the photo goes under the accounts (see the end).
        let photo_row = matches!(
            photo_answer,
            Some(PhotoAnswer::NotAllowed | PhotoAnswer::Unchecked | PhotoAnswer::None)
        );
        let mut rows: Vec<AnyElement> = accounts
            .iter()
            .enumerate()
            .map(|(i, account)| {
                let shown = account.id == active;
                let is_default = several && default.as_deref() == Some(account.id.as_str());
                let id = account.id.clone();
                let make_default = account.id.clone();
                let remove = (account.id.clone(), account.email.clone());
                let initials = account
                    .email
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_default();
                let dragged = DraggedAccount {
                    id: account.id.clone(),
                    email: account.email.clone(),
                    initials: initials.clone(),
                    dark: t.dark,
                };
                let (kind, provider) = provider_of(account);
                // The photo, with where its mail comes from in a badge on its corner.
                let photo = div()
                    .relative()
                    .flex_none()
                    .size(dp(36.))
                    .child(if let Some(photo) = photos.get(&account.id) {
                        me_avatar(Some(photo), &initials, 36.)
                    } else if shown {
                        me_avatar(None, &initials, 36.)
                    } else {
                        avatar(crate::data::tone_for(&account.email), &initials, 36.)
                            .into_any_element()
                    })
                    .child(
                        div()
                            .id(("account-provider", i))
                            .absolute()
                            .right(dp(-4.))
                            .bottom(dp(-4.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(dp(18.))
                            .rounded_full()
                            .bg(t.raised)
                            .border_1()
                            .border_color(t.border_strong)
                            .tooltip(tip(t.dark, provider.clone(), ""))
                            .child(provider_logo(t, kind, 10.)),
                    );
                // Small text buttons, quieter than the card's other buttons.
                let quiet = |label: &'static str| {
                    let hover = t.hover;
                    let text = t.text;
                    div()
                        .flex()
                        .items_center()
                        .h(dp(24.))
                        .px(dp(8.))
                        .rounded(dp(6.))
                        .cursor_pointer()
                        .text_size(dp(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.text2)
                        .hover(move |s| s.bg(hover).text_color(text))
                        .child(label)
                };
                let details = div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(dp(2.))
                    // The provider in words, then what can be done with it.
                    .child(div().mr(dp(6.)).child(meta(provider, t.text3)))
                    .when(!shown, |d| {
                        d.child(
                            quiet("Switch to")
                                .id(("switch-account", i))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let id = id.clone();
                                    this.owner.update(cx, |n, cx| n.switch_account(id, cx));
                                })),
                        )
                    })
                    .when(several && !is_default, |d| {
                        d.child(
                            quiet("Make default")
                                .id(("default-account", i))
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
                    });
                let text = div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(dp(0.))
                    .gap(dp(4.))
                    // The address, with Default and Shown now as pills by it.
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .h(dp(22.))
                            .gap(dp(8.))
                            .min_w(dp(0.))
                            .child(
                                div()
                                    .min_w(dp(0.))
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(account.email.clone()),
                            )
                            .when(is_default, |d| d.child(pill(t, "Default", t.violet)))
                            .when(shown, |d| d.child(pill(t, "Shown now", t.mint))),
                    )
                    .child(details);
                let rose = t.rose;
                let ends = div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(dp(4.))
                    // Sign out asks first, in a dialog.
                    .child(
                        div()
                            .id(("remove-account", i))
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(dp(30.))
                            .rounded(dp(8.))
                            .cursor_pointer()
                            .hover(move |s| s.bg(rose.opacity(0.12)))
                            .tooltip(tip(t.dark, "Sign out", ""))
                            .group(SharedString::from(format!("remove-account-{i}")))
                            .child(icon("sign-out", 15., t.text3).group_hover(
                                SharedString::from(format!("remove-account-{i}")),
                                move |s| s.text_color(rose),
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let (id, email) = remove.clone();
                                this.ask_remove(id, email, cx);
                            })),
                    )
                    // A handle that says the row can be dragged to another place.
                    .when(several, |d| {
                        d.child(
                            div()
                                .id(("account-grip", i))
                                .flex_none()
                                .p(dp(6.))
                                .rounded(dp(6.))
                                .cursor(gpui::CursorStyle::OpenHand)
                                .tooltip(tip(t.dark, "Drag to reorder", ""))
                                .child(icon("grip", 16., t.text3)),
                        )
                    });
                let hover = t.hover;
                div()
                    .id(("account-row", i))
                    .flex()
                    .items_center()
                    .gap(dp(14.))
                    .px(dp(16.))
                    .py(dp(14.))
                    // Highlighted while another is dragged over it: square where it meets the
                    // rows around it, and rounded like the card (14, inside its border) where
                    // it is the card's own top or bottom.
                    .when(i == 0, |d| d.rounded_t(dp(13.)))
                    .when(i + 1 == accounts.len() && !photo_row, |d| {
                        d.rounded_b(dp(13.))
                    })
                    .child(photo)
                    .child(text)
                    .child(ends)
                    // Dragged onto another row, it takes that row's place.
                    .when(several, |d| {
                        d.on_drag(dragged, |dragged, _, window, cx| {
                            // A closed hand everywhere in the window until it is dropped (the
                            // drag starts once this returns).
                            window.defer(cx, |window, cx| {
                                cx.set_active_drag_cursor_style(
                                    gpui::CursorStyle::ClosedHand,
                                    window,
                                );
                            });
                            cx.new(|_| dragged.clone())
                        })
                        .drag_over::<DraggedAccount>(move |style, _, _, _| style.bg(hover))
                        .on_drop(cx.listener(
                            move |this, dragged: &DraggedAccount, _, cx| {
                                let id = dragged.id.clone();
                                this.owner.update(cx, |n, cx| n.move_account(&id, i, cx));
                            },
                        ))
                    })
                    .into_any_element()
            })
            .collect();
        if rows.is_empty() {
            rows.push(row(t, "No account yet", "", div()));
        }
        let actions = div().flex().gap(dp(8.)).when(can_add, |d| {
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
                .gap(dp(8.))
                .child(actions)
                // The consequence, at the moment it matters.
                .into_any_element(),
        ]
    }

    fn appearance(&mut self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let n = self.owner.read(cx);
        let (theme, size, zoom) = (
            n.settings.theme,
            n.settings.reader_text,
            (n.settings.zoom() * 100.).round() as u16,
        );
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
                        .min_w(dp(0.))
                        .overflow_hidden()
                        .h_full()
                        .flex()
                        .bg(th.surface)
                        .when(left, |d| d.rounded_l(dp(INNER)))
                        .when(right, |d| d.rounded_r(dp(INNER)))
                        .child(
                            div()
                                .flex_none()
                                .w(dp(22.))
                                .h_full()
                                .bg(th.sidebar)
                                .when(left, |d| d.rounded_l(dp(INNER)))
                                .border_r_1()
                                .border_color(th.border),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(dp(0.))
                                .flex()
                                .flex_col()
                                .gap(dp(5.))
                                .p(dp(8.))
                                .child(div().h(dp(5.)).w(dp(40.)).rounded_full().bg(th.text2))
                                .child(
                                    div()
                                        .h(dp(4.))
                                        .w(dp(56.))
                                        .rounded_full()
                                        .bg(th.border_strong),
                                )
                                .child(
                                    div()
                                        .h(dp(4.))
                                        .w(dp(48.))
                                        .rounded_full()
                                        .bg(th.border_strong),
                                ),
                        )
                };
                // An equal share of the row each, the preview keeping its shape as the
                // window grows or shrinks.
                let mut preview = div();
                preview.style().aspect_ratio = Some(150. / 92.);
                div()
                    .id(("theme-tile", id))
                    .flex_1()
                    .min_w(dp(0.))
                    .flex()
                    .flex_col()
                    .gap(dp(8.))
                    .cursor_pointer()
                    .child(
                        preview
                            .flex()
                            .w_full()
                            .rounded(dp(10.))
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
            .gap(dp(16.))
            .p(dp(16.))
            .child(tile(0, "System", ThemeChoice::System, true, true))
            .child(tile(1, "Light", ThemeChoice::Light, true, false))
            .child(tile(2, "Dark", ThemeChoice::Dark, false, true))
            .into_any_element();
        let size_row = row(
            t,
            "Mail text size",
            "Emails and the message list",
            self.choice(
                t,
                "text-size",
                &["Small", "Default", "Large", "Extra large"],
                match size {
                    TextSize::Small => 0,
                    TextSize::Default => 1,
                    TextSize::Large => 2,
                    TextSize::ExtraLarge => 3,
                },
                cx,
                |n, i| {
                    n.settings.reader_text = [
                        TextSize::Small,
                        TextSize::Default,
                        TextSize::Large,
                        TextSize::ExtraLarge,
                    ][i]
                },
            ),
        );
        let steps = crate::settings::ZOOM_STEPS;
        let zoom_row = row(
            t,
            "Interface size",
            "Everything in Nimbra's windows (⌘+ and ⌘-)",
            self.choice_with(
                t,
                "interface-size",
                &steps
                    .map(|s| format!("{s}%"))
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                steps.iter().position(|s| *s == zoom).unwrap_or(1),
                cx,
                move |this, i, cx| {
                    this.owner
                        .update(cx, |n, cx| n.set_interface_zoom(steps[i], cx));
                },
            ),
        );
        vec![
            group(t, Some("Theme"), vec![tiles]),
            group(t, None, vec![zoom_row, size_row]),
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
        let signatures = self.signatures(t, cx);
        vec![
            group(t, None, vec![undo_row, quote_row]),
            group(t, Some("Signatures"), signatures),
        ]
    }

    // ------------------------------------------------------------ signatures

    /// The account whose signatures are shown: the one picked, or the default account.
    fn sig_account(&self, cx: &App) -> Option<(String, String)> {
        let n = self.owner.read(cx);
        let accounts = n.signature_accounts();
        let wanted = self
            .sig_account
            .clone()
            .or_else(|| n.settings.default_account().map(str::to_string));
        accounts
            .iter()
            .find(|(id, _)| Some(id) == wanted.as_ref())
            .or(accounts.first())
            .cloned()
    }

    /// The chip shown as picked: the one clicked, or the account's default.
    fn sig_pick(&self, account: &str, cx: &App) -> SigPick {
        let settings = &self.owner.read(cx).settings;
        match &self.sig_pick {
            Some(SigPick::Signature(id))
                if settings.signature(id).is_some_and(|s| s.account == account) =>
            {
                SigPick::Signature(id.clone())
            }
            Some(SigPick::Nothing) => SigPick::Nothing,
            _ => match settings.default_signature(account) {
                Some(s) => SigPick::Signature(s.id.clone()),
                None => match settings.signatures_of(account).next() {
                    // "No signature" is the default, but there are signatures to edit.
                    Some(_) if settings.default_signature.contains_key(account) => SigPick::Nothing,
                    Some(s) => SigPick::Signature(s.id.clone()),
                    None => SigPick::Nothing,
                },
            },
        }
    }

    /// The signature being edited, if a signature (not "No signature") is picked.
    fn sig_editing(&self, cx: &App) -> Option<crate::settings::Signature> {
        let (account, _) = self.sig_account(cx)?;
        match self.sig_pick(&account, cx) {
            SigPick::Signature(id) => self.owner.read(cx).settings.signature(&id).cloned(),
            SigPick::Nothing => None,
        }
    }

    fn pick_signature(&mut self, pick: SigPick, cx: &mut Context<Self>) {
        self.sig_pick = Some(pick);
        self.sig_source = None;
        cx.notify();
    }

    fn pick_account(&mut self, id: String, cx: &mut Context<Self>) {
        self.sig_account = Some(id);
        self.sig_pick = None;
        self.sig_source = None;
        cx.notify();
    }

    /// Changes one signature and saves it.
    fn change_signature(
        &mut self,
        id: &str,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut crate::settings::Signature),
    ) {
        self.owner.update(cx, |n, cx| {
            if let Some(signature) = n.settings.signatures.iter_mut().find(|s| s.id == id) {
                let before = signature.clone();
                change(signature);
                if *signature != before {
                    n.save_settings(cx);
                }
            }
        });
        cx.notify();
    }

    fn new_signature(&mut self, cx: &mut Context<Self>) {
        let Some((account, _)) = self.sig_account(cx) else {
            return;
        };
        let added = self.owner.update(cx, |n, cx| {
            let id = n.settings.add_signature(&account);
            if id.is_some() {
                n.save_settings(cx);
            }
            id
        });
        match added {
            Some(id) => {
                self.sig_mode = SigMode::Text;
                self.pick_signature(SigPick::Signature(id), cx);
                // Named first: the name field takes the keyboard.
                self.pending_focus = Some(self.sig_name.read(cx).focus_handle(cx));
            }
            None => self.owner.update(cx, |n, cx| {
                n.notice = Some(
                    format!(
                        "An account can have {} signatures.",
                        crate::settings::SIGNATURES_PER_ACCOUNT
                    )
                    .into(),
                );
                cx.notify();
            }),
        }
    }

    fn make_default_signature(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        let Some((account, _)) = self.sig_account(cx) else {
            return;
        };
        self.set(cx, move |n| {
            n.settings.set_default_signature(&account, id.as_deref())
        });
    }

    fn confirm_delete_signature(&mut self, cx: &mut Context<Self>) {
        if let Some((id, _)) = self.confirm_delete.take() {
            self.set(cx, |n| n.settings.delete_signature(&id));
            self.sig_pick = None;
            self.sig_source = None;
        }
        cx.notify();
    }

    fn cancel_delete_signature(&mut self, cx: &mut Context<Self>) {
        self.confirm_delete = None;
        cx.notify();
    }

    fn name_changed(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.sig_name_for.clone() else {
            return;
        };
        let name = crate::signature::clean_name(&self.sig_name.read(cx).text());
        self.change_signature(&id, cx, |s| s.name = name);
    }

    /// The name field holds the picked signature's name.
    fn sync_name(&mut self, editing: Option<&crate::settings::Signature>, cx: &mut Context<Self>) {
        let id = editing.map(|s| s.id.clone());
        if self.sig_name_for == id {
            return;
        }
        self.sig_name_for = id;
        let name = editing.map(|s| s.name.clone()).unwrap_or_default();
        self.sig_name
            .update(cx, |input, cx| input.set(name, usize::MAX, cx));
    }

    /// Account, chips, the picked signature's name and actions, and its editor.
    fn signatures(&mut self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some((account, email)) = self.sig_account(cx) else {
            return vec![row(
                t,
                "No account yet",
                "Add an account to give it signatures.",
                div(),
            )];
        };
        let pick = self.sig_pick(&account, cx);
        let editing = self.sig_editing(cx);
        self.sync_name(editing.as_ref(), cx);
        let n = self.owner.read(cx);
        let several = n.signature_accounts().len() >= 2;
        let photo = n.photos.get(&account).cloned();
        let list: Vec<(String, String)> = n
            .settings
            .signatures_of(&account)
            .map(|s| (s.id.clone(), s.name.clone()))
            .collect();
        let default_id = n.settings.default_signature(&account).map(|s| s.id.clone());
        let hover = t.hover;

        // The account: its photo and address, and with several a list to pick another.
        let face = super::menus::account_face(&email, photo.as_ref(), 24.);
        let anchor = self.sig_anchor.clone();
        let field = div()
            .id("signature-account")
            .relative()
            .flex()
            .items_center()
            .gap(dp(10.))
            // As wide as the address, up to a limit (a longer one ends in "…").
            .max_w(dp(320.))
            .h(dp(36.))
            .pl(dp(6.))
            .pr(dp(10.))
            .rounded(dp(9.))
            .bg(t.hover)
            .border_1()
            .border_color(t.border)
            .child(face)
            .child(
                div()
                    .flex_initial()
                    .min_w(dp(0.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(email.clone()),
            )
            .when(several, |d| {
                d.cursor_pointer()
                    .hover(|s| s.border_color(t.border_strong))
                    .press(t)
                    .child(icon("down", 14., t.text2))
                    .child(
                        gpui::canvas(
                            move |bounds, _, _| {
                                anchor.set((
                                    from_screen(bounds.origin.x),
                                    from_screen(bounds.origin.y + bounds.size.height),
                                    from_screen(bounds.size.width),
                                ));
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .on_click(
                        cx.listener(|this, _, window, cx| this.toggle_account_picker(window, cx)),
                    )
            });
        let account_row = row(
            t,
            "Account",
            if several {
                "Each account has signatures of its own."
            } else {
                ""
            },
            field,
        );

        // The chips: "No signature", each signature (the default marked), and New.
        let chip = |id: SharedString, label: String, on: bool, default: bool| {
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .gap(dp(6.))
                .h(dp(28.))
                .px(dp(11.))
                .rounded_full()
                .border_1()
                .cursor_pointer()
                .text_size(dp(12.5))
                .font_weight(FontWeight::MEDIUM)
                .when(on, |d| {
                    d.bg(t.selected)
                        .border_color(t.violet.opacity(0.55))
                        .text_color(t.text)
                })
                .when(!on, |d| {
                    d.bg(t.hover)
                        .border_color(t.border)
                        .text_color(t.text2)
                        .hover(move |s| s.bg(hover).text_color(t.text))
                })
                .press(t)
                .when(default, |d| {
                    d.child(div().size(dp(6.)).rounded_full().bg(t.violet))
                })
                .child(label)
                .when(default, |d| d.child(meta("Default", t.text3)))
        };
        let none_is_default = default_id.is_none();
        let mut chips: Vec<AnyElement> = vec![
            chip(
                "sig-none".into(),
                "No signature".into(),
                pick == SigPick::Nothing,
                none_is_default,
            )
            .on_click(cx.listener(|this, _, _, cx| this.pick_signature(SigPick::Nothing, cx)))
            .into_any_element(),
        ];
        for (id, name) in &list {
            let on = pick == SigPick::Signature(id.clone());
            let default = default_id.as_ref() == Some(id);
            let picked = id.clone();
            chips.push(
                chip(
                    format!("sig-{id}").into(),
                    signature_name(name).to_string(),
                    on,
                    default,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.pick_signature(SigPick::Signature(picked.clone()), cx)
                }))
                .into_any_element(),
            );
        }
        chips.push(
            div()
                .id("sig-new")
                .flex()
                .flex_none()
                .items_center()
                .h(dp(28.))
                .px(dp(11.))
                .rounded_full()
                .cursor_pointer()
                .text_size(dp(12.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.violet)
                .hover(move |s| s.bg(hover))
                .press(t)
                .child("New signature")
                .on_click(cx.listener(|this, _, _, cx| this.new_signature(cx)))
                .into_any_element(),
        );
        let chips_row = div()
            .flex()
            .flex_wrap()
            .gap(dp(8.))
            .px(dp(16.))
            .py(dp(12.))
            .children(chips)
            .into_any_element();

        let mut rows = vec![account_row, chips_row];
        match &editing {
            None => {
                let detail = if list.is_empty() {
                    "Messages from this account have no signature. Make one with New signature."
                } else {
                    "Messages from this account start without a signature. Pick one in the composer whenever you want one."
                };
                let control = if none_is_default {
                    pill(t, "Default", t.violet).into_any_element()
                } else {
                    button(t, ButtonKind::Secondary, "Make default", None)
                        .id("sig-none-default")
                        .press_soft()
                        .on_click(
                            cx.listener(|this, _, _, cx| this.make_default_signature(None, cx)),
                        )
                        .into_any_element()
                };
                rows.push(row(t, "No signature", detail, control));
            }
            Some(signature) => {
                rows.push(self.signature_header(t, signature, default_id.as_ref(), cx));
                rows.push(self.signature_editor(t, signature, cx));
            }
        }
        rows
    }

    /// The picked signature's name, Make default and Delete.
    fn signature_header(
        &mut self,
        t: &Theme,
        signature: &crate::settings::Signature,
        default_id: Option<&String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_default = default_id == Some(&signature.id);
        let id = signature.id.clone();
        let delete = (signature.id.clone(), signature.name.clone());
        div()
            .flex()
            .items_center()
            .gap(dp(8.))
            .px(dp(16.))
            .py(dp(10.))
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::MEDIUM)
                    .child("Name"),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(dp(0.))
                    .max_w(dp(260.))
                    .px(dp(10.))
                    .py(dp(5.))
                    .rounded(dp(8.))
                    .bg(t.hover)
                    .border_1()
                    .border_color(t.border)
                    .child(self.sig_name.clone()),
            )
            .child(div().flex_1())
            .child(if is_default {
                pill(t, "Default", t.violet).into_any_element()
            } else {
                button(t, ButtonKind::Secondary, "Make default", None)
                    .id("sig-default")
                    .press_soft()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.make_default_signature(Some(id.clone()), cx)
                    }))
                    .into_any_element()
            })
            .child(
                div()
                    .id("sig-delete")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(dp(32.))
                    .rounded(dp(8.))
                    .border_1()
                    .border_color(t.border)
                    .cursor_pointer()
                    .hover({
                        let rose = t.rose;
                        move |s| s.bg(rose.opacity(0.1)).border_color(rose.opacity(0.35))
                    })
                    .press_soft()
                    .tooltip(tip(t.dark, "Delete signature", ""))
                    .child(icon("trash", 15., t.rose))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.confirm_delete = Some(delete.clone());
                        this.pending_focus = Some(this.dialog_focus.clone());
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// Text · HTML · Preview, and the signature in that form.
    fn signature_editor(
        &mut self,
        t: &Theme,
        signature: &crate::settings::Signature,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mode = self.sig_mode;
        let switch = self.choice_with(
            t,
            "sig-mode",
            &["Text", "HTML", "Preview"],
            match mode {
                SigMode::Text => 0,
                SigMode::Html => 1,
                SigMode::Preview => 2,
            },
            cx,
            |this, i, cx| {
                this.sig_mode = [SigMode::Text, SigMode::Html, SigMode::Preview][i];
                this.sig_source = None;
                cx.notify();
            },
        );
        let hint = match mode {
            SigMode::Text => "Formatted like a message: ⌘B, ⌘I, ⌘U and ⌘K for links.",
            SigMode::Html => "Scripts, forms and unsafe links are taken out as it is saved.",
            SigMode::Preview => "Images are kept on this Mac.",
        };
        // In Preview, pictures not downloaded yet are fetched, and the Preview redrawn once
        // they arrive.
        let sources = crate::signature::image_sources(&signature.html);
        if mode == SigMode::Preview {
            let this = cx.entity().downgrade();
            crate::signature_images::ensure(sources.clone(), cx, move |cx| {
                this.update(cx, |_, cx| cx.notify()).ok();
            });
        }
        let refresh = (mode == SigMode::Preview && !sources.is_empty()).then(|| {
            button(t, ButtonKind::Ghost, "Refresh images", None)
                .id("sig-refresh-images")
                .press_soft()
                .on_click(cx.listener(move |_, _, _, cx| {
                    crate::signature_images::forget(&sources);
                    cx.notify();
                }))
        });
        let fits = crate::signature::fits_text_editor(&signature.html);
        let body = match mode {
            // The editor is a native web view: this element only reserves its space and
            // tells it where that space is each frame (see `sync_signature_editor`).
            SigMode::Text | SigMode::Html if mode == SigMode::Html || fits => {
                let editor = self.sig_editor.clone();
                div()
                    .id("sig-editor")
                    .key_context("Editor")
                    .track_focus(&self.sig_editor_focus)
                    .h(dp(SIG_EDITOR_HEIGHT))
                    .child(
                        gpui::canvas(
                            move |bounds, _, _| {
                                if let Some(editor) = &editor {
                                    editor.set_bounds(
                                        f32::from(bounds.origin.x) + 1.,
                                        f32::from(bounds.origin.y) + 1.,
                                        f32::from(bounds.size.width) - 2.,
                                        f32::from(bounds.size.height) - 2.,
                                    );
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .size_full(),
                    )
                    .into_any_element()
            }
            // Kept whole: what the text editor would drop is never loaded into it.
            SigMode::Text => div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(dp(12.))
                .h(dp(SIG_EDITOR_HEIGHT))
                .px(dp(24.))
                .text_color(t.text2)
                .child("This signature has HTML the text editor can't keep. Edit it in HTML.")
                .child(
                    button(t, ButtonKind::Secondary, "Edit in HTML", None)
                        .id("sig-to-html")
                        .press_soft()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sig_mode = SigMode::Html;
                            cx.notify();
                        })),
                )
                .into_any_element(),
            SigMode::Html | SigMode::Preview => self.signature_preview(t, &signature.html),
        };
        div()
            .flex()
            .flex_col()
            .gap(dp(10.))
            .px(dp(16.))
            .pt(dp(10.))
            .pb(dp(16.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(dp(12.))
                    .child(switch)
                    .child(div().flex_1())
                    .children(refresh),
            )
            .child(
                div()
                    .rounded(dp(10.))
                    .border_1()
                    .border_color(t.border)
                    .bg(t.raised)
                    .overflow_hidden()
                    .child(body),
            )
            // As far from the box as from the bottom of the group (16).
            .child(meta(hint, t.text3).mt(dp(6.)))
            .into_any_element()
    }

    /// The signature drawn natively, as the reader draws mail. Nothing is fetched: images
    /// are placeholders with their description.
    fn signature_preview(&self, t: &Theme, html: &str) -> AnyElement {
        let blocks = crate::rich::from_html(html);
        div()
            .id("sig-preview")
            .h(dp(SIG_EDITOR_HEIGHT))
            .overflow_y_scroll()
            .px(dp(20.))
            .py(dp(14.))
            .text_size(dp(14.))
            .line_height(dp(22.))
            .when(blocks.is_empty(), |d| {
                d.child(div().text_color(t.text3).child("This signature is empty."))
            })
            .children(blocks.iter().map(|block| preview_block(t, block)))
            .into_any_element()
    }

    /// The editor is wanted for the picked signature in Text (when it fits) or HTML, while
    /// Composing is shown and no dialog covers it: (signature id, mode).
    fn sig_editor_wanted(&self, cx: &App) -> Option<(String, SigMode)> {
        if self.tab != Tab::Composing
            || self.confirm_delete.is_some()
            || self.confirm_remove.is_some()
            || self.sig_mode == SigMode::Preview
        {
            return None;
        }
        let signature = self.sig_editing(cx)?;
        (self.sig_mode == SigMode::Html || crate::signature::fits_text_editor(&signature.html))
            .then_some((signature.id, self.sig_mode))
    }

    /// Creates, fills, shows or hides the signature editor to match what is picked.
    fn sync_signature_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(wanted) = self.sig_editor_wanted(cx) else {
            if std::mem::take(&mut self.sig_shown)
                && let Some(editor) = &self.sig_editor
            {
                editor.set_visible(false);
                editor.blur();
            }
            return;
        };
        if self.sig_editor.is_none() && !self.sig_editor_failed {
            let (events, mut incoming) = futures::channel::mpsc::unbounded();
            match crate::editor::EditorView::new(window, events) {
                Some(view) => {
                    self.sig_editor = Some(std::rc::Rc::new(view));
                    cx.spawn(async move |this, cx| {
                        use futures::StreamExt as _;
                        while let Some(event) = incoming.next().await {
                            if this
                                .update(cx, |this, cx| this.signature_editor_event(event, cx))
                                .is_err()
                            {
                                break;
                            }
                        }
                    })
                    .detach();
                }
                None => self.sig_editor_failed = true,
            }
        }
        let Some(editor) = self.sig_editor.clone() else {
            return;
        };
        if !self.sig_editor_ready {
            return;
        }
        // Focus moved from the editor to one of the window's own fields: the web view must
        // give up the keyboard too, or typing would still go there.
        let focused = self.sig_editor_focus.is_focused(window);
        if self.sig_keys && !focused {
            editor.blur();
        }
        self.sig_keys = focused;
        let dark = self.owner.read(cx).dark;
        if self.sig_editor_dark != Some(dark) {
            let t = if dark { Theme::dark() } else { Theme::light() };
            let css = |color: Hsla| {
                let c = color.to_rgb();
                format!(
                    "rgba({},{},{},{:.3})",
                    (c.r * 255.).round(),
                    (c.g * 255.).round(),
                    (c.b * 255.).round(),
                    c.a
                )
            };
            editor.theme(serde_json::json!({
                "--bg": css(t.raised),
                "--text": css(t.text),
                "--dim": css(t.text3),
                "--accent": css(t.violet),
                "--line": css(t.border_strong),
                "--max": "none",
                "--pad": "12px",
            }));
            editor.placeholder("Write your signature…");
            self.sig_editor_dark = Some(dark);
        }
        if self.sig_loaded.as_ref() != Some(&wanted) {
            let html = self
                .owner
                .read(cx)
                .settings
                .signature(&wanted.0)
                .map(|s| s.html.clone())
                .unwrap_or_default();
            self.sig_session += 1;
            match wanted.1 {
                SigMode::Html => {
                    let text = match &self.sig_source {
                        Some((id, text)) if *id == wanted.0 => text.clone(),
                        _ => crate::signature::source_lines(&html),
                    };
                    editor.source(self.sig_session, &text);
                }
                _ => editor.load(self.sig_session, &html, "", false),
            }
            self.sig_loaded = Some(wanted);
        }
        editor.set_opaque(self.sig_painted);
        if !self.sig_shown {
            editor.set_visible(true);
            self.sig_shown = true;
        }
    }

    fn signature_editor_event(
        &mut self,
        event: crate::editor::EditorEvent,
        cx: &mut Context<Self>,
    ) {
        use crate::editor::EditorEvent;
        let loaded = self
            .sig_loaded
            .clone()
            .filter(|_| self.sig_editor_wanted(cx).as_ref() == self.sig_loaded.as_ref());
        match event {
            EditorEvent::Ready => {
                self.sig_editor_ready = true;
                self.sig_loaded = None;
                self.sig_editor_dark = None;
                self.sig_shown = false;
                self.sig_painted = false;
            }
            EditorEvent::Painted => {
                if !self.sig_painted {
                    self.sig_painted = true;
                    cx.notify();
                }
            }
            // What was typed is kept, cleaned (only what the text editor keeps, from there).
            EditorEvent::Content {
                session,
                html,
                edited: true,
                ..
            } if session == self.sig_session => {
                if let Some((id, SigMode::Text)) = loaded {
                    let html = crate::signature::clean_simple(&html);
                    self.change_signature(&id, cx, |s| s.html = html);
                }
            }
            EditorEvent::Source { session, text } if session == self.sig_session => {
                if let Some((id, SigMode::Html)) = loaded {
                    let html = crate::signature::clean(&text);
                    self.sig_source = Some((id.clone(), text));
                    self.change_signature(&id, cx, |s| s.html = html);
                }
            }
            EditorEvent::Focused => {
                self.pending_focus = Some(self.sig_editor_focus.clone());
            }
            // The composer's keys (send, save, Escape…) mean nothing here.
            _ => {}
        }
        cx.notify();
    }

    /// Opens (or closes) the list of accounts under the account field.
    fn toggle_account_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.sig_picker.take() {
            super::menus::remove_popup(handle, cx);
            return;
        }
        let n = self.owner.read(cx);
        let selected = self.sig_account(cx).map(|(id, _)| id).unwrap_or_default();
        let accounts: Vec<super::menus::AccountChoice> = n
            .signature_accounts()
            .into_iter()
            .map(|(id, email)| super::menus::AccountChoice {
                photo: n.photos.get(&id).cloned(),
                on: id == selected,
                id,
                email,
            })
            .collect();
        let dark = n.dark;
        let (left, bottom, width) = self.sig_anchor.get();
        let width = width.max(280.);
        let height =
            accounts.len() as f32 * super::menus::ACCOUNT_ROW + super::menus::ACCOUNT_LIST_CHROME;
        let margin = super::menus::MENU_MARGIN;
        let origin = window.bounds().origin;
        let bounds = gpui::Bounds::new(
            gpui::point(
                origin.x + to_screen(left - margin),
                origin.y + to_screen(bottom + 4. - margin),
            ),
            gpui::size(
                to_screen(width + 2. * margin),
                to_screen(height + 2. * margin),
            ),
        );
        let owner = cx.weak_entity();
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
                    // Closes on any click outside it, as the menus do (polled: clicks on
                    // another app never reach this one).
                    cx.spawn_in(window, async move |this, cx| {
                        let mut armed = false;
                        loop {
                            cx.background_executor()
                                .timer(Duration::from_millis(30))
                                .await;
                            if !crate::blur::mouse_down() {
                                armed = true;
                            }
                            let open =
                                this.update_in(cx, |this: &mut AccountPicker, window, cx| {
                                    if armed
                                        && crate::blur::mouse_pressed_outside(window, margin as f64)
                                    {
                                        this.close(window, cx);
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
                    AccountPicker {
                        owner,
                        accounts,
                        dark,
                    }
                })
            },
        );
        self.sig_picker = opened.ok();
    }

    /// The dialog over the window asking whether to delete a signature.
    fn delete_dialog(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (_, name) = self.confirm_delete.as_ref()?;
        let danger = div()
            .id("confirm-delete-signature")
            .flex()
            .flex_none()
            .items_center()
            .px(dp(14.))
            .py(dp(6.))
            .rounded(dp(9.))
            .bg(t.rose)
            .text_color(gpui::white())
            .font_weight(FontWeight::SEMIBOLD)
            .cursor_pointer()
            .hover(|s| s.opacity(0.9))
            .child("Delete")
            .on_click(cx.listener(|this, _, _, cx| this.confirm_delete_signature(cx)));
        Some(
            div()
                .id("delete-signature-scrim")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(t.scrim)
                .occlude()
                .track_focus(&self.dialog_focus)
                .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => this.cancel_delete_signature(cx),
                        "enter" => this.confirm_delete_signature(cx),
                        _ => {}
                    }
                }))
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, _, cx| this.cancel_delete_signature(cx)),
                )
                .child(
                    div()
                        .id("delete-signature")
                        .w(dp(400.))
                        .flex()
                        .flex_col()
                        .gap(dp(10.))
                        .p(dp(24.))
                        .rounded(dp(16.))
                        .bg(t.raised)
                        .border_1()
                        .border_color(t.border_strong)
                        .shadow(vec![shadow(
                            c(0x000000, if t.dark { 0.6 } else { 0.2 }),
                            24.,
                            60.,
                            -10.,
                        )])
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .text_size(dp(15.))
                                .line_height(dp(20.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("Delete “{}”?", signature_name(name))),
                        )
                        .child(div().text_color(t.text2).child(
                            "New messages can no longer use it. Messages already sent and \
                             drafts keep it as they are.",
                        ))
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap(dp(8.))
                                .pt(dp(10.))
                                .child(
                                    button(t, ButtonKind::Secondary, "Cancel", None)
                                        .id("cancel-delete-signature")
                                        .press_soft()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.cancel_delete_signature(cx)
                                        })),
                                )
                                .child(danger),
                        ),
                )
                .into_any_element(),
        )
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
        // "Play a sound", and a button beside it that plays the sound now.
        let hover = t.hover;
        let label = div()
            .flex()
            .items_center()
            .gap(dp(6.))
            .child(div().font_weight(FontWeight::MEDIUM).child("Play a sound"))
            .child(
                div()
                    .id("preview-sound")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(dp(24.))
                    .rounded(dp(6.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .tooltip(tip(t.dark, "Hear it", ""))
                    .child(icon("play", 12., t.text2))
                    .on_click(|_, _, _| crate::notify::preview_sound()),
            );
        let sound_row = if on {
            row_with(
                t,
                label.into_any_element(),
                "",
                switch(t, sound)
                    .id("sound")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set(cx, |n| n.settings.notify_sound = !n.settings.notify_sound)
                    })),
            )
        } else {
            // A sound belongs to a notification: with none, the switch is there but inert
            // (the preview still plays).
            row_with(
                t,
                label.into_any_element(),
                "",
                switch(t, sound).opacity(0.45).cursor_default(),
            )
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
        let updating = self.owner.read(cx).settings.auto_update;
        let update_row = self.toggle(
            t,
            "auto-update",
            "Update automatically",
            "Checks nimbra.me for a new version every few hours, downloads it in the background, and asks you to restart. The check sends nothing but the request.",
            updating,
            cx,
            |n| n.settings.auto_update = !n.settings.auto_update,
        );
        let fact = |name: &'static str, title: &'static str, text: &'static str| {
            div()
                .flex()
                .gap(dp(12.))
                .px(dp(16.))
                .py(dp(12.))
                // Level with the title's 18pt line.
                .child(div().pt(dp(1.)).child(icon(name, 16., t.violet)))
                .child(
                    div()
                        .flex_1()
                        .min_w(dp(0.))
                        .flex()
                        .flex_col()
                        .gap(dp(2.))
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
            group(t, None, vec![usage_row, update_row]),
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
                                .px(dp(16.))
                                .py(dp(9.))
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
        let n = self.owner.read(cx);
        let (update_ready, update_checking, update_status) = (
            n.update_ready.clone(),
            n.update_checking,
            n.update_status.clone(),
        );
        vec![
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(dp(14.))
                .child(
                    div()
                        .size(dp(ABOUT_ICON))
                        // The glow follows the icon's own shape, which the image brings.
                        .rounded(dp(ABOUT_ICON / 4.))
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
                    // The version, and what the last check for a new one found.
                    match &update_status {
                        Some(status) => {
                            format!("Version {} · {status}", env!("CARGO_PKG_VERSION"))
                        }
                        None => format!("Version {}", env!("CARGO_PKG_VERSION")),
                    },
                    t.text3,
                ))
                .child(
                    div()
                        .flex()
                        .gap(dp(8.))
                        .pt(dp(4.))
                        // Check for a new version now, or restart into the one that is ready.
                        .child(match &update_ready {
                            Some(_) => crate::ui::glow(
                                button(t, ButtonKind::Primary, "Restart to update", None)
                                    .id("about-update")
                                    .press_soft()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.owner.update(cx, |n, cx| n.restart_to_update(cx));
                                    })),
                                "about-update",
                                true,
                            )
                            .into_any_element(),
                            None => button(
                                t,
                                ButtonKind::Secondary,
                                if update_checking {
                                    "Checking…"
                                } else {
                                    "Check for updates"
                                },
                                None,
                            )
                            .id("about-update")
                            .press_soft()
                            .when(update_checking, |b| b.opacity(0.6))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.owner.update(cx, |n, cx| n.check_for_updates(cx));
                            }))
                            .into_any_element(),
                        })
                        .child(
                            button(t, ButtonKind::Secondary, "Visit website", None)
                                .id("about-homepage")
                                .press_soft()
                                .on_click(|_, _, cx| cx.open_url(HOMEPAGE)),
                        ),
                )
                // The site's policies, as small links.
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(dp(8.))
                        .text_size(dp(12.))
                        .text_color(t.text3)
                        .child(about_link(t, "about-privacy", "Privacy", PRIVACY))
                        .child("·")
                        .child(about_link(t, "about-terms", "Terms", TERMS)),
                )
                .into_any_element(),
        ]
    }
}

/// Nimbra's site and its policies, linked from About.
const HOMEPAGE: &str = "https://nimbra.me";
const PRIVACY: &str = "https://nimbra.me/privacy";
const TERMS: &str = "https://nimbra.me/terms";

/// A small text link that opens `url` in the browser.
fn about_link(
    t: &Theme,
    id: &'static str,
    label: &'static str,
    url: &'static str,
) -> impl IntoElement {
    let bright = t.text;
    div()
        .id(id)
        .cursor_pointer()
        .hover(move |s| s.text_color(bright).underline())
        .child(label)
        .on_click(move |_, _, cx| cx.open_url(url))
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

/// An account being dragged to another place in Settings › Accounts, and what follows the
/// pointer meanwhile.
#[derive(Clone)]
struct DraggedAccount {
    id: String,
    email: String,
    initials: String,
    dark: bool,
}

impl Render for DraggedAccount {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        div()
            .flex()
            .items_center()
            .gap(dp(10.))
            .px(dp(12.))
            .py(dp(8.))
            .rounded(dp(10.))
            .bg(t.raised)
            .border_1()
            .border_color(t.border_strong)
            .shadow(vec![shadow(t.shadow, 8., 24., -6.)])
            .font_family(crate::theme::SANS)
            .text_size(dp(13.))
            .text_color(t.text)
            .cursor(gpui::CursorStyle::ClosedHand)
            .child(avatar(
                crate::data::tone_for(&self.email),
                &self.initials,
                24.,
            ))
            .child(self.email.clone())
    }
}

/// Where an account's mail comes from, for the logo at the end of its row.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Provider {
    Gmail,
    Microsoft,
    Other,
}

/// The account's provider, and the words for its tooltip.
fn provider_of(account: &crate::settings::AccountRef) -> (Provider, String) {
    let Some(servers) = &account.servers else {
        return (Provider::Gmail, "Gmail".into());
    };
    let host = servers.imap.host.to_lowercase();
    if account.oauth.as_deref() == Some("microsoft")
        || host.ends_with("outlook.com")
        || host.ends_with("office365.com")
    {
        (Provider::Microsoft, "Microsoft · Outlook".into())
    } else if host == "imap.gmail.com" || host == "imap.googlemail.com" {
        (Provider::Gmail, "Gmail · IMAP".into())
    } else {
        (Provider::Other, format!("IMAP · {}", servers.imap.host))
    }
}

/// Gmail's "M" in its colours: a PNG made from `assets/gmail.svg`
/// (`rsvg-convert -w 64 assets/gmail.svg -o assets/gmail.png`), as GPUI draws an SVG
/// image with its red and blue swapped.
static GMAIL_LOGO: std::sync::LazyLock<std::sync::Arc<gpui::Image>> =
    std::sync::LazyLock::new(|| {
        std::sync::Arc::new(gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            include_bytes!("../assets/gmail.png").to_vec(),
        ))
    });

/// The provider's logo, about `size` wide.
fn provider_logo(t: &Theme, kind: Provider, size: f32) -> AnyElement {
    match kind {
        Provider::Gmail => gpui::img(GMAIL_LOGO.clone())
            .w(dp(size))
            .h(dp(size * 0.75))
            .into_any_element(),
        // Microsoft's four squares.
        Provider::Microsoft => {
            let side = (size - 1.) / 2.;
            let square = |hex: u32| div().size(dp(side)).bg(c(hex, 1.));
            div()
                .flex()
                .flex_col()
                .gap(dp(1.))
                .child(
                    div()
                        .flex()
                        .gap(dp(1.))
                        .child(square(0xf25022))
                        .child(square(0x7fba00)),
                )
                .child(
                    div()
                        .flex()
                        .gap(dp(1.))
                        .child(square(0x00a4ef))
                        .child(square(0xffb900)),
                )
                .into_any_element()
        }
        Provider::Other => icon("mail", size, t.text2).into_any_element(),
    }
}

/// Each account's row in the signature account list.
/// The list of accounts under Settings › Composing's account field: a window of its own,
/// like the menus, so it can hang over the signature editor (a native view).
pub(super) struct AccountPicker {
    owner: gpui::WeakEntity<SettingsWindow>,
    accounts: Vec<super::menus::AccountChoice>,
    dark: bool,
}

impl AccountPicker {
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.owner
            .update(cx, |settings, _| settings.sig_picker = None)
            .ok();
        window.remove_window();
    }
}

impl Render for AccountPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Every window draws at the interface zoom.
        crate::ui::apply_zoom(window);
        let t = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        let rows = self.accounts.iter().enumerate().map(|(i, account)| {
            let picked = account.id.clone();
            super::menus::account_row(&t, ("signature-account", i), account).on_click(cx.listener(
                move |this, _, window, cx| {
                    let picked = picked.clone();
                    this.owner
                        .update(cx, |settings, cx| settings.pick_account(picked, cx))
                        .ok();
                    this.close(window, cx);
                },
            ))
        });
        super::menus::account_list(&t, rows)
    }
}

/// One block of a signature's Preview: the reader's look, without its selection and links
/// (nothing in Settings opens a link), and images as placeholders.
fn preview_block(t: &Theme, block: &Block) -> AnyElement {
    let highlights: Vec<_> = block
        .spans
        .iter()
        .map(|span| {
            let style = gpui::HighlightStyle {
                font_weight: span.bold.then_some(FontWeight::SEMIBOLD),
                font_style: span.italic.then_some(gpui::FontStyle::Italic),
                color: span.link.is_some().then_some(t.violet),
                background_color: span.code.then_some(t.hover),
                underline: span.link.is_some().then(|| gpui::UnderlineStyle {
                    thickness: px(1.),
                    color: Some(t.violet.opacity(0.5)),
                    wavy: false,
                }),
                ..Default::default()
            };
            (span.range.clone(), style)
        })
        .collect();
    let text = || StyledText::new(block.text.clone()).with_highlights(highlights.clone());
    let element = match &block.kind {
        Kind::Paragraph => div().child(text()),
        Kind::Heading(level) => {
            let size = match level {
                1 => 20.,
                2 => 17.,
                _ => 15.,
            };
            div()
                .text_size(dp(size))
                .line_height(dp(size * 1.35))
                .font_weight(FontWeight::SEMIBOLD)
                .child(text())
        }
        Kind::Item { marker, depth } => div()
            .flex()
            .gap(dp(10.))
            .pl(dp(4. + 20. * *depth as f32))
            .child(
                div()
                    .flex_none()
                    .min_w(dp(14.))
                    .text_color(t.text3)
                    .child(marker.clone()),
            )
            .child(div().flex_1().min_w(dp(0.)).child(text())),
        Kind::Code => div()
            .font_family(crate::theme::MONO)
            .text_size(dp(12.5))
            .child(text()),
        Kind::Rule => div().h(dp(1.)).my(dp(6.)).bg(t.border),
        Kind::Image { src, .. } if let Some(image) = crate::signature_images::image(src) => {
            div().py(dp(2.)).child(
                gpui::img(image)
                    .max_w_full()
                    .max_h(dp(160.))
                    .object_fit(gpui::ObjectFit::ScaleDown),
            )
        }
        Kind::Image { alt, .. } => div()
            .flex()
            .items_center()
            .gap(dp(8.))
            .py(dp(2.))
            .text_size(dp(12.))
            .text_color(t.text3)
            .child(icon("image", 14., t.text3))
            .child(if alt.is_empty() {
                "Image".to_string()
            } else {
                format!("Image: {alt}")
            }),
    };
    if block.quote > 0 {
        div()
            .pl(dp(12.))
            .border_l_2()
            .border_color(t.border_strong)
            .text_color(t.text3)
            .child(element)
            .into_any_element()
    } else {
        element.into_any_element()
    }
}

#[cfg(test)]
mod provider_tests {
    use super::*;
    use crate::servers::{Security, Server, Servers};

    fn account(host: Option<&str>, oauth: Option<&str>) -> crate::settings::AccountRef {
        let server = |host: &str| Server {
            host: host.into(),
            port: 993,
            security: Security::Tls,
        };
        crate::settings::AccountRef {
            id: "a".into(),
            email: "a@example.com".into(),
            servers: host.map(|host| Servers {
                username: "a@example.com".into(),
                imap: server(host),
                smtp: server(host),
            }),
            oauth: oauth.map(str::to_string),
        }
    }

    #[test]
    fn each_account_shows_where_its_mail_comes_from() {
        assert_eq!(provider_of(&account(None, None)).0, Provider::Gmail);
        assert_eq!(
            provider_of(&account(Some("imap.gmail.com"), None)).0,
            Provider::Gmail
        );
        assert_eq!(
            provider_of(&account(Some("outlook.office365.com"), Some("microsoft"))).0,
            Provider::Microsoft
        );
        let zoho = provider_of(&account(Some("imap.zoho.com"), None));
        assert_eq!(zoho, (Provider::Other, "IMAP · imap.zoho.com".to_string()));
    }
}
