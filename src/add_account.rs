//! Adding an account that is not Gmail (Zoho, iCloud, Fastmail, a company's own server):
//! the sheet that asks for the address and password, finds the servers, checks that both
//! take the password, and only then keeps the account. The same sheet asks for the password
//! again when a server stops taking it. Microsoft accounts (Outlook.com, Microsoft 365) sign
//! in with Microsoft in the browser instead, from its button or when the address is found
//! to be one.

use super::*;
use crate::discover::Found;
use crate::imap_mail::Credential;
use crate::servers::{Secret, Security, Server, Servers};

/// Google's "G", in its colours, for the Continue with Google button. A PNG made from
/// `assets/google.svg` (`rsvg-convert -h 64 assets/google.svg -o assets/google.png`): GPUI
/// draws an SVG image with its red and blue swapped.
static GOOGLE_LOGO: std::sync::LazyLock<Arc<gpui::Image>> = std::sync::LazyLock::new(|| {
    Arc::new(gpui::Image::from_bytes(
        gpui::ImageFormat::Png,
        include_bytes!("../assets/google.png").to_vec(),
    ))
});

/// The open Add account sheet.
pub(super) struct AccountSheet {
    /// Asking again for an account that is already here (its password stopped working).
    pub(super) for_account: Option<String>,
    /// Shown in the Settings window (Accounts) rather than over the main window.
    pub(super) in_settings: bool,
    /// The field to put the keyboard in, for the window it is shown in to take.
    pub(super) focus: Option<FocusHandle>,
    email: Entity<TextInput>,
    password: Entity<TextInput>,
    /// The server fields are shown: nothing was found, or the ones found did not work.
    manual: bool,
    username: Entity<TextInput>,
    imap_host: Entity<TextInput>,
    imap_port: Entity<TextInput>,
    imap_security: Security,
    smtp_host: Entity<TextInput>,
    smtp_port: Entity<TextInput>,
    smtp_security: Security,
    /// Finding servers or checking the password.
    busy: bool,
    /// The security switches (incoming, outgoing): when each last changed, and how many
    /// times, so the highlight slides once per change.
    slid: [(Option<std::time::Instant>, u64); 2],
    /// Waiting for Microsoft's sign-in in the browser (with `busy`).
    microsoft: bool,
    error: Option<SharedString>,
    /// Tells a finished check apart from one for an older attempt.
    attempt: u64,
    _subscriptions: Vec<gpui::Subscription>,
}

impl Nimbra {
    /// Opens the sheet over the main window: empty for a new account, or for `account`'s
    /// password.
    pub(super) fn open_account_sheet(
        &mut self,
        for_account: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.open_account_form(for_account, false, cx);
    }

    /// The sheet is open over the main window (not in Settings).
    pub(super) fn account_sheet_over_main(&self) -> bool {
        self.account_sheet.as_ref().is_some_and(|s| !s.in_settings)
    }

    /// The form shown in Settings, if it is, and the field to focus there (once).
    pub(super) fn account_form_in_settings(&mut self) -> Option<Option<FocusHandle>> {
        self.account_sheet
            .as_mut()
            .filter(|s| s.in_settings)
            .map(|s| s.focus.take())
    }

    /// Opens the form in the Settings window's Accounts section.
    pub(super) fn open_account_form_in_settings(&mut self, cx: &mut Context<Self>) {
        self.open_account_form(None, true, cx);
    }

    fn open_account_form(
        &mut self,
        for_account: Option<String>,
        in_settings: bool,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        let t = self.theme();
        let field = |cx: &mut Context<Self>, hint: &str| {
            cx.new(|cx| TextInput::new(cx, hint, t.faint, t.violet))
        };
        let email = field(cx, "you@example.com");
        let password = field(cx, "Password or app password");
        password.update(cx, |input, _| input.set_secret(true));
        let username = field(cx, "Usually your address");
        let imap_host = field(cx, "imap.example.com");
        let imap_port = field(cx, "993");
        let smtp_host = field(cx, "smtp.example.com");
        let smtp_port = field(cx, "465");
        let mut manual = false;
        let mut security = (Security::Tls, Security::Tls);
        if let Some(account) = for_account
            .as_deref()
            .and_then(|id| self.settings.accounts.iter().find(|a| a.id == id))
        {
            let address = account.email.clone();
            email.update(cx, |input, cx| input.set(address, usize::MAX, cx));
            if let Some(servers) = &account.servers {
                manual = true;
                security = (servers.imap.security, servers.smtp.security);
                for (input, text) in [
                    (&username, servers.username.clone()),
                    (&imap_host, servers.imap.host.clone()),
                    (&imap_port, servers.imap.port.to_string()),
                    (&smtp_host, servers.smtp.host.clone()),
                    (&smtp_port, servers.smtp.port.to_string()),
                ] {
                    input.update(cx, |input, cx| input.set(text, usize::MAX, cx));
                }
            }
        }
        let enter = |this: &mut Self,
                     input: Entity<TextInput>,
                     event: &InputEvent,
                     cx: &mut Context<Self>| {
            match event {
                InputEvent::Enter => this.account_sheet_continue(cx),
                InputEvent::Tab => this.account_sheet_tab(&input, false, cx),
                InputEvent::BackTab => this.account_sheet_tab(&input, true, cx),
                _ => {}
            }
        };
        let subscriptions = [
            &email, &password, &username, &imap_host, &imap_port, &smtp_host, &smtp_port,
        ]
        .into_iter()
        .map(|input| cx.subscribe(input, enter))
        .collect();
        let focus = if for_account.is_some() {
            password.read(cx).focus_handle(cx)
        } else {
            email.read(cx).focus_handle(cx)
        };
        if !in_settings {
            self.pending_focus = Some(focus.clone());
        }
        self.account_sheet = Some(AccountSheet {
            for_account,
            in_settings,
            focus: in_settings.then_some(focus),
            email,
            password,
            manual,
            username,
            imap_host,
            imap_port,
            imap_security: security.0,
            smtp_host,
            smtp_port,
            smtp_security: security.1,
            busy: false,
            slid: [(None, 0); 2],
            microsoft: false,
            error: None,
            attempt: 0,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    /// Shows the server fields with an example filled in (screenshots).
    pub(super) fn show_account_servers(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = &mut self.account_sheet else {
            return;
        };
        sheet.email.update(cx, |input, cx| {
            input.set("ana@example.com".into(), usize::MAX, cx)
        });
        sheet.password.update(cx, |input, cx| {
            input.set("app-password".into(), usize::MAX, cx)
        });
        fill(sheet, &guess("ana@example.com"), cx);
        sheet.manual = true;
        sheet.error =
            Some("Nimbra could not find this account’s servers. Check them above.".into());
        cx.notify();
    }

    /// Tab: the keyboard goes to the next field shown (from the last, back to the first).
    fn account_sheet_tab(&mut self, from: &Entity<TextInput>, back: bool, cx: &mut Context<Self>) {
        let Some(sheet) = &mut self.account_sheet else {
            return;
        };
        let mut order = vec![&sheet.email, &sheet.password];
        if sheet.manual {
            order.extend([
                &sheet.username,
                &sheet.imap_host,
                &sheet.imap_port,
                &sheet.smtp_host,
                &sheet.smtp_port,
            ]);
        }
        let Some(at) = order.iter().position(|input| *input == from) else {
            return;
        };
        // Shift-Tab (`back`) goes to the field before, round from the first to the last.
        let step = if back { order.len() - 1 } else { 1 };
        let next = order[(at + step) % order.len()].read(cx).focus_handle(cx);
        // The window the form is in takes it (see `focus`).
        if sheet.in_settings {
            sheet.focus = Some(next);
        } else {
            self.pending_focus = Some(next);
        }
        cx.notify();
    }

    pub(super) fn close_account_sheet(&mut self, cx: &mut Context<Self>) {
        let in_settings = self
            .account_sheet
            .take()
            .is_some_and(|sheet| sheet.in_settings);
        if !in_settings {
            self.pending_focus = Some(self.focus.clone());
        }
        cx.notify();
    }

    /// Continue: finds the servers (unless they are typed in), checks the password with
    /// both, and keeps the account.
    fn account_sheet_continue(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = &mut self.account_sheet else {
            return;
        };
        if sheet.busy {
            return;
        }
        let email = sheet.email.read(cx).text().trim().to_string();
        let password = sheet.password.read(cx).text();
        if !mail::is_address(&email) {
            sheet.error = Some("Type your whole email address.".into());
            cx.notify();
            return;
        }
        // Without a password, what the address is decides (Google and Microsoft sign in
        // in the browser); typed servers always need one.
        if password.is_empty() && sheet.manual {
            sheet.error = Some("Type the account’s password.".into());
            cx.notify();
            return;
        }
        let typed = if sheet.manual {
            match typed_servers(sheet, cx) {
                Ok(servers) => Some(servers),
                Err(error) => {
                    sheet.error = Some(error.to_string().into());
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        sheet.busy = true;
        sheet.microsoft = false;
        sheet.error = None;
        sheet.attempt += 1;
        let attempt = sheet.attempt;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let address = email.clone();
            let secret = password.clone();
            let outcome = cx
                .background_executor()
                .spawn(async move { find_and_check(&address, &secret, typed) })
                .await;
            this.update(cx, |this, cx| {
                this.account_checked(attempt, email, password, outcome, cx)
            })
            .ok();
        })
        .detach();
    }

    fn account_checked(
        &mut self,
        attempt: u64,
        email: String,
        password: String,
        outcome: Checked,
        cx: &mut Context<Self>,
    ) {
        let Some(sheet) = self.account_sheet.as_mut().filter(|s| s.attempt == attempt) else {
            return;
        };
        sheet.busy = false;
        match outcome {
            Checked::Google => {
                self.close_account_sheet(cx);
                self.add_google_account(cx);
            }
            Checked::NoPassword => {
                sheet.error = Some("Type the account’s password.".into());
                cx.notify();
            }
            Checked::OAuth("Microsoft") if self.microsoft.is_some() => {
                self.continue_with_microsoft(cx);
            }
            Checked::OAuth(provider) => {
                sheet.error = Some(
                    format!(
                        "{provider} accounts need {provider}’s own sign-in, which Nimbra does not have yet."
                    )
                    .into(),
                );
                cx.notify();
            }
            // Nothing found, or the servers found did not take it: shown, to fix.
            Checked::Unknown(servers) => {
                if !sheet.manual {
                    fill(sheet, &servers, cx);
                    sheet.manual = true;
                }
                sheet.error =
                    Some("Nimbra could not find this account’s servers. Check them above.".into());
                cx.notify();
            }
            Checked::Failed(servers, why) => {
                if !sheet.manual {
                    fill(sheet, &servers, cx);
                    sheet.manual = true;
                }
                sheet.error = Some(why);
                cx.notify();
            }
            Checked::Works(servers) => {
                let for_account = sheet.for_account.clone();
                let credential = Credential::Password(password);
                if let Err(error) =
                    self.keep_imap_account(for_account, email, credential, servers, true, cx)
                    && let Some(sheet) = &mut self.account_sheet
                {
                    sheet.error = Some(error);
                    cx.notify();
                }
            }
        }
    }

    /// Continue with Microsoft: signs in in the browser (to the address typed, if any),
    /// checks that the mailbox lets Nimbra in over IMAP, and keeps the account. The sheet
    /// stays open, saying what it waits for; closing it drops the sign-in.
    pub(super) fn continue_with_microsoft(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.microsoft.clone() else {
            return;
        };
        let Some(sheet) = &mut self.account_sheet else {
            return;
        };
        if config.is_placeholder() {
            sheet.error = Some("Signing in with Microsoft is off in this preview copy.".into());
            cx.notify();
            return;
        }
        let typed = sheet.email.read(cx).text().trim().to_string();
        let hint = mail::is_address(&typed).then_some(typed);
        let (pending, url) = match crate::microsoft::SignIn::start(&config, hint.as_deref()) {
            Ok(started) => started,
            Err(error) => {
                sheet.error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        cx.open_url(&url);
        sheet.busy = true;
        sheet.microsoft = true;
        sheet.error = None;
        sheet.attempt += 1;
        let attempt = sheet.attempt;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let (email, tokens) = pending.finish(&config)?;
                    let tokens = Arc::new(tokens);
                    let servers = crate::microsoft::servers(&email);
                    crate::imap_mail::check_login(&servers, Credential::Microsoft(tokens.clone()))
                        .map_err(|error| {
                            if error.is::<mail::Offline>() {
                                anyhow::anyhow!(
                                    "Microsoft’s mail server could not be reached. Check the connection and try again."
                                )
                            } else {
                                error
                            }
                        })?;
                    anyhow::Ok((email, tokens, servers))
                })
                .await;
            this.update(cx, |this, cx| {
                // Closed, or started again, meanwhile: nothing is kept.
                let Some(sheet) = this.account_sheet.as_mut().filter(|s| s.attempt == attempt)
                else {
                    return;
                };
                sheet.busy = false;
                sheet.microsoft = false;
                let kept = match outcome {
                    Ok((email, tokens, servers)) => {
                        let for_account = sheet.for_account.clone();
                        let credential = Credential::Microsoft(tokens);
                        this.keep_imap_account(for_account, email, credential, servers, true, cx)
                    }
                    Err(error) => Err(error.to_string().into()),
                };
                if let Err(error) = kept
                    && let Some(sheet) = &mut this.account_sheet
                {
                    sheet.error = Some(error);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Keeps an account whose servers took its password or Microsoft sign-in: that in the
    /// keychain (off the main thread, as macOS may ask about it; the sheet says it is busy
    /// meanwhile), the servers in the settings, and the account shown. Why not, if it is
    /// not: at once, or where it was asked from once the keychain has answered.
    pub(super) fn keep_imap_account(
        &mut self,
        for_account: Option<String>,
        email: String,
        credential: Credential,
        servers: Servers,
        from_sheet: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), SharedString> {
        let existing = for_account.or_else(|| {
            self.settings
                .accounts
                .iter()
                .find(|a| a.email.eq_ignore_ascii_case(&email))
                .map(|a| a.id.clone())
        });
        let new = existing.is_none();
        let id = match existing {
            Some(id) if !id.is_empty() => id,
            // The first account of an old install is Gmail's and has no id of its own.
            Some(_) => return Err("That address is already in Nimbra, with Google.".into()),
            None => mail::new_account_id().map_err(|error| error.to_string())?,
        };
        let asked_in = self
            .account_sheet
            .as_mut()
            .filter(|_| from_sheet)
            .map(|sheet| {
                sheet.busy = true;
                sheet.error = None;
                sheet.attempt += 1;
                sheet.attempt
            });
        let generation = self.account_gen;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let key = id.clone();
            let saved = cx
                .background_executor()
                .spawn(async move { save_credential(&key, credential) })
                .await;
            this.update(cx, |this, cx| {
                // Closed or started again (the sheet), or another account shown meanwhile:
                // nothing is kept.
                let wanted = match asked_in {
                    Some(attempt) => this
                        .account_sheet
                        .as_ref()
                        .is_some_and(|sheet| sheet.attempt == attempt),
                    None => this.account_gen == generation,
                };
                if !wanted {
                    if new && saved.is_ok() {
                        std::thread::spawn(move || {
                            crate::servers::forget_password(&id);
                            crate::microsoft::forget_refresh_token(&id);
                        });
                    }
                    return;
                }
                match saved {
                    Ok((credential, oauth)) => {
                        this.imap_account_kept(id, email, credential, oauth, servers, cx)
                    }
                    Err(error) => {
                        match &mut this.account_sheet {
                            Some(sheet) if asked_in.is_some() => {
                                sheet.busy = false;
                                sheet.error = Some(error);
                            }
                            _ => {
                                this.status = Status::SignedOut;
                                this.notice = Some(error);
                            }
                        }
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
        Ok(())
    }

    fn imap_account_kept(
        &mut self,
        id: String,
        email: String,
        credential: Credential,
        oauth: Option<String>,
        servers: Servers,
        cx: &mut Context<Self>,
    ) {
        match self.settings.accounts.iter_mut().find(|a| a.id == id) {
            Some(entry) => {
                entry.email = email.clone();
                entry.servers = Some(servers.clone());
                entry.oauth = oauth;
            }
            None => self.settings.accounts.push(crate::settings::AccountRef {
                id: id.clone(),
                email: email.clone(),
                servers: Some(servers.clone()),
                oauth,
            }),
        }
        self.settings.active = id.clone();
        self.save_settings(cx);
        self.close_account_sheet(cx);
        // A connection to it from before (the old password or sign-in) goes.
        self.others.remove(&id);
        if self.account_id != id
            || self.connection.is_none()
            || matches!(self.status, Status::SignedOut | Status::SigningIn)
        {
            self.reset_account(id.clone(), cx);
            self.connection = Some(crate::contacts::wrap(
                crate::imap_mail::Client::new(id, email.clone(), servers, credential),
                &email,
            ));
            self.status = Status::Loading;
            self.open_store(cx);
        }
        cx.notify();
    }

    /// The sheet over the main window: a card with the title, Google, and the form.
    pub(super) fn account_sheet_view(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(sheet) = self.account_sheet.as_ref().filter(|s| !s.in_settings) else {
            return div().into_any_element();
        };
        let asking_again = sheet.for_account.is_some();
        div()
            .id("account-scrim")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(t.scrim)
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close_account_sheet(cx)),
            )
            .child(
                div()
                    .id("account-sheet")
                    .w(dp(480.))
                    .flex()
                    .flex_col()
                    .gap(dp(14.))
                    .p(dp(22.))
                    .rounded(dp(18.))
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![shadow(
                        c(0x000000, if t.dark { 0.6 } else { 0.2 }),
                        30.,
                        80.,
                        -10.,
                    )])
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(serif(
                        if asking_again {
                            "Sign in again"
                        } else {
                            "Add account"
                        },
                        28.,
                        t.text,
                    ))
                    .when(!asking_again, |d| {
                        d.child(self.google_button(t, cx).w_full())
                            .when(self.microsoft.is_some(), |d| {
                                d.child(self.microsoft_button(t, cx).w_full())
                            })
                            .child(meta(
                                "Or any other address (Zoho, iCloud, Fastmail, your own domain):"
                                    .to_string(),
                                t.text3,
                            ))
                    })
                    .child(self.account_fields(t, cx)),
            )
            .into_any_element()
    }

    /// The form as Settings shows it, in its own cards: Google, then any other address.
    pub(super) fn account_settings_page(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let card = |title: &str, detail: &str, body: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap(dp(8.))
                .child(
                    div()
                        .pl(dp(4.))
                        .child(overline(title.to_uppercase(), t.text3)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(dp(14.))
                        .p(dp(16.))
                        .rounded(dp(14.))
                        .bg(t.raised)
                        .border_1()
                        .border_color(t.border)
                        .when(!detail.is_empty(), |d| {
                            d.child(meta(detail.to_string(), t.text3))
                        })
                        .child(body),
                )
                .into_any_element()
        };
        let mut cards = vec![card(
            "Google",
            "Gmail and Google Workspace. You sign in in your browser, with Google.",
            self.google_button(t, cx).into_any_element(),
        )];
        if self.microsoft.is_some() {
            cards.push(card(
                "Microsoft",
                "Outlook.com, Hotmail and Microsoft 365. You sign in in your browser, with Microsoft.",
                self.microsoft_button(t, cx).into_any_element(),
            ));
        }
        cards.push(card(
            "Any other address",
            "Zoho, iCloud, Fastmail, Yahoo, or your own domain. Nimbra finds the servers.",
            self.account_fields(t, cx).into_any_element(),
        ));
        cards
    }

    /// Continue with Microsoft, with its four squares.
    fn microsoft_button(&self, t: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let square = |hex: u32| div().size(dp(7.5)).bg(gpui::rgb(hex));
        let logo = div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(dp(1.))
            .child(
                div()
                    .flex()
                    .gap(dp(1.))
                    .child(square(0xF25022))
                    .child(square(0x7FBA00)),
            )
            .child(
                div()
                    .flex()
                    .gap(dp(1.))
                    .child(square(0x00A4EF))
                    .child(square(0xFFB900)),
            );
        button(t, ButtonKind::Secondary, "Continue with Microsoft", None)
            .id("account-microsoft")
            .press_soft()
            .justify_center()
            .gap(dp(10.))
            // The logo is added after the label: shown before it.
            .flex_row_reverse()
            .child(logo)
            .on_click(cx.listener(|this, _, _, cx| this.continue_with_microsoft(cx)))
    }

    /// Continue with Google, with Google's logo.
    fn google_button(&self, t: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        button(t, ButtonKind::Secondary, "Continue with Google", None)
            .id("account-google")
            .press_soft()
            .justify_center()
            .gap(dp(10.))
            // The logo is added after the label: shown before it.
            .flex_row_reverse()
            .child(gpui::img(GOOGLE_LOGO.clone()).size(dp(16.)).flex_none())
            .on_click(cx.listener(|this, _, _, cx| {
                this.close_account_sheet(cx);
                this.add_google_account(cx);
            }))
    }

    /// Email, password, the servers when they are shown, what is happening, and the
    /// buttons: the same wherever the form is.
    fn account_fields(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let Some(sheet) = &self.account_sheet else {
            return div();
        };
        let asking_again = sheet.for_account.is_some();
        // Every box in the form is this tall: fields and the security switch line up.
        const BOX_H: f32 = 40.;
        const SWITCH_W: f32 = 176.;
        let field = |label: &str, input: &Entity<TextInput>| {
            div()
                .flex()
                .flex_col()
                .gap(dp(6.))
                .child(meta(label.to_string(), t.text3))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .h(dp(BOX_H))
                        .px(dp(12.))
                        .rounded(dp(10.))
                        .bg(t.hover)
                        .border_1()
                        .border_color(t.border)
                        .text_size(dp(14.))
                        .child(div().w_full().child(input.clone())),
                )
        };
        // How the connection is secured: the list's All / Unread switch, with its sliding
        // highlight.
        let slid = sheet.slid;
        let security = |id: &'static str, value: Security, imap: bool, cx: &mut Context<Self>| {
            // The switch's border and padding, two options and the 2 between them.
            let option_w = (SWITCH_W - 2. - 6. - 2.) / 2.;
            let option =
                |index: usize, choice: Security, label: &'static str, cx: &mut Context<Self>| {
                    div()
                        .id((id, index))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .w(dp(option_w))
                        .h_full()
                        .rounded(dp(7.))
                        .text_size(dp(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .cursor_pointer()
                        .text_color(if value == choice { t.text } else { t.text3 })
                        .child(label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let Some(sheet) = &mut this.account_sheet else {
                                return;
                            };
                            let which = usize::from(!imap);
                            let (slot, port, defaults) = if imap {
                                (&mut sheet.imap_security, &sheet.imap_port, ("993", "143"))
                            } else {
                                (&mut sheet.smtp_security, &sheet.smtp_port, ("465", "587"))
                            };
                            if *slot == choice {
                                return;
                            }
                            *slot = choice;
                            sheet.slid[which] =
                                (Some(std::time::Instant::now()), sheet.slid[which].1 + 1);
                            // The usual port follows, unless one of the others was typed.
                            let typed = port.read(cx).text();
                            if typed.is_empty() || typed == defaults.0 || typed == defaults.1 {
                                let next = if choice == Security::Tls {
                                    defaults.0
                                } else {
                                    defaults.1
                                };
                                port.update(cx, |input, cx| {
                                    input.set(next.to_string(), usize::MAX, cx)
                                });
                            }
                            cx.notify();
                        }))
                };
            // The highlight: under the option chosen, sliding there right after a change.
            let x = |security: Security| {
                if security == Security::Tls {
                    0.
                } else {
                    option_w + 2.
                }
            };
            let other = if value == Security::Tls {
                Security::StartTls
            } else {
                Security::Tls
            };
            let (to, from) = (x(value), x(other));
            let (changed, generation) = slid[usize::from(!imap)];
            let sliding = !crate::system::reduces_motion()
                && changed.is_some_and(|at| at.elapsed() < Duration::from_millis(260));
            let pill = div()
                .absolute()
                .top(dp(3.))
                .w(dp(option_w))
                .h(dp(BOX_H - 2. - 6.))
                .rounded(dp(7.))
                .bg(t.raised)
                .border_1()
                .border_color(t.border)
                .shadow(vec![shadow(
                    c(0x000000, if t.dark { 0.25 } else { 0.08 }),
                    1.,
                    3.,
                    0.,
                )]);
            let pill = if sliding {
                pill.with_animation(
                    (id, generation as usize),
                    Animation::new(Duration::from_millis(260)).with_easing(ease_out_quint()),
                    move |d, delta| d.left(dp(3. + from + (to - from) * delta)),
                )
                .into_any_element()
            } else {
                pill.left(dp(3. + to)).into_any_element()
            };
            div()
                .flex()
                .flex_col()
                .gap(dp(6.))
                .child(meta("Security".to_string(), t.text3))
                .child(
                    div()
                        .relative()
                        .flex()
                        .gap(dp(2.))
                        .w(dp(SWITCH_W))
                        .h(dp(BOX_H))
                        .p(dp(3.))
                        .rounded(dp(10.))
                        .bg(t.hover)
                        .border_1()
                        .border_color(t.border)
                        .child(pill)
                        .child(option(0, Security::Tls, "SSL/TLS", cx))
                        .child(option(1, Security::StartTls, "STARTTLS", cx)),
                )
        };
        let server_row = |label: &str,
                          host: &Entity<TextInput>,
                          port: &Entity<TextInput>,
                          secured: AnyElement| {
            div()
                .flex()
                .gap(dp(8.))
                .child(div().flex_1().min_w(dp(0.)).child(field(label, host)))
                .child(div().w(dp(76.)).flex_none().child(field("Port", port)))
                .child(div().w(dp(SWITCH_W)).flex_none().child(secured))
        };
        div()
            .flex()
            .flex_col()
            .gap(dp(14.))
                    .child(field("Email", &sheet.email))
                    .child(field("Password", &sheet.password))
                    .when(sheet.manual, |d| {
                        d.child(field("User name", &sheet.username))
                            .child(server_row(
                                "Incoming (IMAP)",
                                &sheet.imap_host,
                                &sheet.imap_port,
                                security("imap-security", sheet.imap_security, true, cx)
                                    .into_any_element(),
                            ))
                            .child(server_row(
                                "Outgoing (SMTP)",
                                &sheet.smtp_host,
                                &sheet.smtp_port,
                                security("smtp-security", sheet.smtp_security, false, cx)
                                    .into_any_element(),
                            ))
                    })
                    .child(match &sheet.error {
                        Some(error) => error_callout(t, error).into_any_element(),
                        None => meta(
                            match () {
                                _ if sheet.microsoft => "Finish signing in with Microsoft in your browser…",
                                _ if sheet.busy => "Finding the servers and checking the password…",
                                _ => "The password is kept in your Mac’s keychain. With two-step sign-in on, use an app password from your provider.",
                            },
                            t.text3,
                        )
                        .into_any_element(),
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(dp(8.))
                            .child(
                                button(t, ButtonKind::Ghost, "Cancel", None)
                                    .id("account-cancel")
                                    .press_soft()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.close_account_sheet(cx)),
                                    ),
                            )
                            .child(crate::ui::glow(

                                button(
                                    t,
                                    ButtonKind::Primary,
                                    if sheet.busy { "Checking…" } else if asking_again { "Sign in" } else { "Add" },
                                    Some("↵"),
                                )
                                .id("account-continue")
                                .press_soft()
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.account_sheet_continue(cx)),
                                ), "account-continue", true)),
                    )
    }
}

/// What finding the servers and checking the password came to.
enum Checked {
    /// Gmail: signed in with Google instead.
    Google,
    /// A provider that only takes its own sign-in (Microsoft).
    OAuth(&'static str),
    /// Servers found, but no password was typed to check them with.
    NoPassword,
    /// Nothing found: these guesses (imap. and smtp. of the domain) to start from.
    Unknown(Servers),
    /// These servers did not take it, and why.
    Failed(Servers, SharedString),
    Works(Servers),
}

/// Runs off the main thread: discovery (unless the servers were typed), then a sign-in to
/// each server.
fn find_and_check(email: &str, password: &str, typed: Option<Servers>) -> Checked {
    let servers = match typed {
        Some(servers) => servers,
        None => match crate::discover::discover(email) {
            Ok(Found::Servers(servers)) => servers,
            Ok(Found::NeedsOAuth { provider: "Google" }) => return Checked::Google,
            Ok(Found::NeedsOAuth { provider }) => return Checked::OAuth(provider),
            Ok(Found::Unknown) | Err(_) => return Checked::Unknown(guess(email)),
        },
    };
    if password.is_empty() {
        return Checked::NoPassword;
    }
    let credential = Credential::Password(password.to_string());
    if let Err(error) = crate::imap_mail::check_login(&servers, credential) {
        return Checked::Failed(servers, explain(&error, "incoming (IMAP)"));
    }
    let secret = Secret::Password(password.to_string());
    if let Err(error) = crate::smtp::check_login(&servers.smtp, &servers.username, &secret) {
        return Checked::Failed(servers, explain(&error, "outgoing (SMTP)"));
    }
    Checked::Works(servers)
}

/// A readable reason a server refused.
fn explain(error: &anyhow::Error, which: &str) -> SharedString {
    if error.is::<mail::SignedOut>() || error.is::<crate::smtp::AuthFailed>() {
        // The server's own words, when it gave a reason ("enable IMAP first").
        let reason = error
            .downcast_ref::<crate::imap_mail::ServerSaid>()
            .map(|said| said.0.trim().to_string())
            .filter(|said| !said.is_empty())
            .map(|said| format!(" It said: “{said}”."))
            .unwrap_or_default();
        format!(
            "The {which} server did not take that password.{reason} Check that IMAP is on for the account, and with two-step sign-in, use an app password."
        )
        .into()
    } else if error.is::<mail::Offline>() {
        format!("The {which} server could not be reached. Check its name and port.").into()
    } else {
        format!("The {which} server: {error}").into()
    }
}

/// Servers to start from when none were found: imap. and smtp. of the address's domain.
fn guess(email: &str) -> Servers {
    let domain = email.rsplit_once('@').map_or("", |(_, d)| d);
    Servers {
        username: email.to_string(),
        imap: Server::new(&format!("imap.{domain}"), 993, Security::Tls),
        smtp: Server::new(&format!("smtp.{domain}"), 465, Security::Tls),
    }
}

/// Puts servers in the sheet's fields.
fn fill(sheet: &mut AccountSheet, servers: &Servers, cx: &mut Context<Nimbra>) {
    sheet.imap_security = servers.imap.security;
    sheet.smtp_security = servers.smtp.security;
    for (input, text) in [
        (&sheet.username, servers.username.clone()),
        (&sheet.imap_host, servers.imap.host.clone()),
        (&sheet.imap_port, servers.imap.port.to_string()),
        (&sheet.smtp_host, servers.smtp.host.clone()),
        (&sheet.smtp_port, servers.smtp.port.to_string()),
    ] {
        input.update(cx, |input, cx| input.set(text, usize::MAX, cx));
    }
}

/// The servers typed in the sheet, checked.
fn typed_servers(sheet: &AccountSheet, cx: &mut Context<Nimbra>) -> anyhow::Result<Servers> {
    let port = |input: &Entity<TextInput>, which: &str| -> anyhow::Result<u16> {
        input
            .read(cx)
            .text()
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|p| *p > 0)
            .ok_or_else(|| anyhow::anyhow!("The {which} port is a number, such as 993 or 465."))
    };
    let servers = Servers {
        username: sheet.username.read(cx).text().trim().to_string(),
        imap: Server::new(
            &sheet.imap_host.read(cx).text(),
            port(&sheet.imap_port, "incoming")?,
            sheet.imap_security,
        ),
        smtp: Server::new(
            &sheet.smtp_host.read(cx).text(),
            port(&sheet.smtp_port, "outgoing")?,
            sheet.smtp_security,
        ),
    };
    servers.check()?;
    Ok(servers)
}

/// Saves an account's sign-in in the keychain, and forgets whichever it signed in with
/// before. Blocking: macOS may ask first.
fn save_credential(
    id: &str,
    credential: Credential,
) -> Result<(Credential, Option<String>), SharedString> {
    match credential {
        Credential::Password(password) => {
            crate::servers::store_password(id, &password)
                .map_err(|error| SharedString::from(format!("{error:#}")))?;
            crate::microsoft::forget_refresh_token(id);
            Ok((Credential::Password(password), None))
        }
        Credential::Microsoft(tokens) => {
            let saved = tokens
                .saved_as(id)
                .map_err(|error| SharedString::from(format!("{error:#}")))?;
            crate::servers::forget_password(id);
            Ok((
                Credential::Microsoft(Arc::new(saved)),
                Some(crate::microsoft::PROVIDER.to_string()),
            ))
        }
    }
}

/// What went wrong, in a soft rose box with an alert icon. It opens over a moment (its height
/// and opacity) so the sheet, which is centred, grows smoothly instead of jumping.
fn error_callout(t: &Theme, error: &SharedString) -> AnyElement {
    let callout = div()
        .flex()
        .items_start()
        .gap(dp(10.))
        .px(dp(12.))
        .py(dp(10.))
        .rounded(dp(10.))
        .bg(t.rose.opacity(0.1))
        .border_1()
        .border_color(t.rose.opacity(0.3))
        .child(div().pt(dp(1.)).child(icon("alert", 15., t.rose)))
        .child(
            div()
                .flex_1()
                .min_w(dp(0.))
                .text_size(dp(12.5))
                .line_height(dp(18.))
                .text_color(t.text)
                .child(error.clone()),
        );
    if crate::system::reduces_motion() {
        return callout.into_any_element();
    }
    // Each new message opens again (keyed by its text).
    let key = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        error.hash(&mut hasher);
        hasher.finish() as usize
    };
    div()
        .overflow_hidden()
        .child(callout)
        .with_animation(
            ("account-error", key),
            Animation::new(Duration::from_millis(280)).with_easing(ease_out_quint()),
            // Taller than any message, so the box is never cut once open.
            |d, delta| d.max_h(dp(160. * delta)).opacity(delta),
        )
        .into_any_element()
}

/// Against the local test servers (see `imap_mail::live`): `cargo test -- --ignored live`.
#[cfg(test)]
mod live {
    use super::*;

    fn local() -> Servers {
        Servers {
            username: "sheet@example.org".into(),
            imap: Server::new("localhost", 10993, Security::Tls),
            smtp: Server::new("localhost", 10587, Security::StartTls),
        }
    }

    #[test]
    #[ignore]
    fn live_the_sheet_checks_both_servers() {
        assert!(matches!(
            find_and_check("sheet@example.org", "pass", Some(local())),
            Checked::Works(_)
        ));
        match find_and_check("sheet@example.org", "wrong", Some(local())) {
            Checked::Failed(_, why) => assert!(why.contains("did not take that password"), "{why}"),
            _ => panic!("a wrong password must not be kept"),
        }
        let mut closed = local();
        closed.imap.port = 1;
        match find_and_check("sheet@example.org", "pass", Some(closed)) {
            Checked::Failed(_, why) => assert!(why.contains("could not be reached"), "{why}"),
            _ => panic!("an unreachable server must not be kept"),
        }
    }
}
