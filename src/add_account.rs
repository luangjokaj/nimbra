//! Adding an account that is not Gmail (Zoho, iCloud, Fastmail, a company's own server):
//! the sheet that asks for the address and password, finds the servers, checks that both
//! take the password, and only then keeps the account. The same sheet asks for the password
//! again when a server stops taking it.

use super::*;
use crate::discover::Found;
use crate::servers::{Security, Server, Servers};

/// Google's "G", in its colours, for the Continue with Google button.
static GOOGLE_LOGO: std::sync::LazyLock<Arc<gpui::Image>> = std::sync::LazyLock::new(|| {
    Arc::new(gpui::Image::from_bytes(
        gpui::ImageFormat::Svg,
        include_bytes!("../assets/google.svg").to_vec(),
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
        let enter =
            |this: &mut Self, _: Entity<TextInput>, event: &InputEvent, cx: &mut Context<Self>| {
                if matches!(event, InputEvent::Enter) {
                    this.account_sheet_continue(cx);
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
            Some("Nimbra could not find this account’s servers. Check them below.".into());
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
        if password.is_empty() {
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
                    Some("Nimbra could not find this account’s servers. Check them below.".into());
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
                self.keep_imap_account(for_account, email, password, servers, cx);
            }
        }
    }

    /// Keeps an account whose servers took the password: the password in the keychain, the
    /// servers in the settings, and the account shown.
    fn keep_imap_account(
        &mut self,
        for_account: Option<String>,
        email: String,
        password: String,
        servers: Servers,
        cx: &mut Context<Self>,
    ) {
        let existing = for_account.or_else(|| {
            self.settings
                .accounts
                .iter()
                .find(|a| a.email.eq_ignore_ascii_case(&email))
                .map(|a| a.id.clone())
        });
        let id = match existing {
            Some(id) if !id.is_empty() => id,
            // The first account of an old install is Gmail's and has no id of its own.
            Some(_) => {
                if let Some(sheet) = &mut self.account_sheet {
                    sheet.error = Some("That address is already in Nimbra, with Google.".into());
                }
                cx.notify();
                return;
            }
            None => match mail::new_account_id() {
                Ok(id) => id,
                Err(error) => {
                    if let Some(sheet) = &mut self.account_sheet {
                        sheet.error = Some(error.to_string().into());
                    }
                    cx.notify();
                    return;
                }
            },
        };
        if let Err(error) = crate::servers::store_password(&id, &password) {
            if let Some(sheet) = &mut self.account_sheet {
                sheet.error = Some(format!("{error:#}").into());
            }
            cx.notify();
            return;
        }
        match self.settings.accounts.iter_mut().find(|a| a.id == id) {
            Some(entry) => {
                entry.email = email.clone();
                entry.servers = Some(servers.clone());
            }
            None => self.settings.accounts.push(crate::settings::AccountRef {
                id: id.clone(),
                email: email.clone(),
                servers: Some(servers.clone()),
            }),
        }
        self.settings.active = id.clone();
        self.save_settings(cx);
        self.close_account_sheet(cx);
        // A connection to it from before (the old password) goes.
        self.others.remove(&id);
        if self.account_id != id || self.connection.is_none() || self.status == Status::SignedOut {
            self.reset_account(id.clone(), cx);
            self.connection = Some(Arc::new(crate::imap_mail::Client::new(
                id, email, servers, password,
            )));
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
                    .w(px(480.))
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .p(px(22.))
                    .rounded(px(18.))
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
                        d.child(self.google_button(t, cx).w_full()).child(meta(
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
                .gap(px(8.))
                .child(
                    div()
                        .pl(px(4.))
                        .child(overline(title.to_uppercase(), t.text3)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(14.))
                        .p(px(16.))
                        .rounded(px(14.))
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
        vec![
            card(
                "Google",
                "Gmail and Google Workspace. You sign in in your browser, with Google.",
                self.google_button(t, cx).into_any_element(),
            ),
            card(
                "Any other address",
                "Zoho, iCloud, Fastmail, Yahoo, or your own domain. Nimbra finds the servers.",
                self.account_fields(t, cx).into_any_element(),
            ),
        ]
    }

    /// Continue with Google, with Google's logo.
    fn google_button(&self, t: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        button(t, ButtonKind::Secondary, "Continue with Google", None)
            .id("account-google")
            .press_soft()
            .justify_center()
            .gap(px(10.))
            // The logo is added after the label: shown before it.
            .flex_row_reverse()
            .child(gpui::img(GOOGLE_LOGO.clone()).size(px(16.)).flex_none())
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
        let field = |label: &str, input: &Entity<TextInput>| {
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(meta(label.to_string(), t.text3))
                .child(
                    div()
                        .px(px(12.))
                        .py(px(8.))
                        .rounded(px(10.))
                        .bg(t.hover)
                        .border_1()
                        .border_color(t.border)
                        .text_size(px(14.))
                        .child(input.clone()),
                )
        };
        let security = |id: &'static str, value: Security, imap: bool, cx: &mut Context<Self>| {
            button(
                t,
                ButtonKind::Secondary,
                match value {
                    Security::Tls => "SSL/TLS",
                    Security::StartTls => "STARTTLS",
                },
                None,
            )
            .id(id)
            .press_soft()
            .tooltip(tip(t.dark, "How the connection is secured", ""))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(sheet) = &mut this.account_sheet {
                    let (slot, port, defaults) = if imap {
                        (&mut sheet.imap_security, &sheet.imap_port, ("993", "143"))
                    } else {
                        (&mut sheet.smtp_security, &sheet.smtp_port, ("465", "587"))
                    };
                    *slot = match *slot {
                        Security::Tls => Security::StartTls,
                        Security::StartTls => Security::Tls,
                    };
                    // The usual port follows, unless one of the others was typed.
                    let typed = port.read(cx).text();
                    if typed.is_empty() || typed == defaults.0 || typed == defaults.1 {
                        let next = if *slot == Security::Tls {
                            defaults.0
                        } else {
                            defaults.1
                        };
                        port.update(cx, |input, cx| input.set(next.to_string(), usize::MAX, cx));
                    }
                    cx.notify();
                }
            }))
        };
        let server_row = |label: &str,
                          host: &Entity<TextInput>,
                          port: &Entity<TextInput>,
                          secured: AnyElement| {
            div()
                .flex()
                .items_end()
                .gap(px(8.))
                .child(div().flex_1().min_w(px(0.)).child(field(label, host)))
                .child(div().w(px(76.)).flex_none().child(field("Port", port)))
                .child(div().flex_none().pb(px(1.)).child(secured))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
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
                    .child(meta(
                        match &sheet.error {
                            Some(error) => error.clone(),
                            None if sheet.busy => "Finding the servers and checking the password…".into(),
                            None => "The password is kept in your Mac’s keychain. With two-step sign-in on, use an app password from your provider."
                                .into(),
                        },
                        if sheet.error.is_some() { t.rose } else { t.text3 },
                    ))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.))
                            .child(
                                button(t, ButtonKind::Ghost, "Cancel", None)
                                    .id("account-cancel")
                                    .press_soft()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.close_account_sheet(cx)),
                                    ),
                            )
                            .child(
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
                                ),
                            ),
                    )
    }
}

/// What finding the servers and checking the password came to.
enum Checked {
    /// Gmail: signed in with Google instead.
    Google,
    /// A provider that only takes its own sign-in (Microsoft).
    OAuth(&'static str),
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
    if let Err(error) = crate::imap_mail::check_login(&servers, password) {
        return Checked::Failed(servers, explain(&error, "incoming (IMAP)"));
    }
    if let Err(error) = crate::smtp::check_login(&servers.smtp, &servers.username, password) {
        return Checked::Failed(servers, explain(&error, "outgoing (SMTP)"));
    }
    Checked::Works(servers)
}

/// A readable reason a server refused.
fn explain(error: &anyhow::Error, which: &str) -> SharedString {
    if error.is::<mail::SignedOut>() || error.is::<crate::smtp::AuthFailed>() {
        format!(
            "The {which} server did not take that password. With two-step sign-in, use an app password."
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
