//! Accounts: signing in and out, connecting each one to its mail (Gmail with Google's
//! sign-in, or IMAP and SMTP with a password or Microsoft's sign-in: the keychain, its
//! store, its connection), switching between them, and their profile photos.

use super::*;
use crate::imap_mail::Credential;

// How an account with its own servers signs in, from the keychain: the engine's.
use nimbra_core::accounts::stored_credential;

/// Shown for a Microsoft account when Microsoft's sign-in is not set up on this Mac.
const NO_MICROSOFT: &str = "This account signs in with Microsoft, which is not set up on this Mac (NIMBRA_MICROSOFT_CLIENT_ID).";

impl Nimbra {
    /// Opens Google's consent page in the browser and waits for it in the background. For
    /// an account with its own servers, asks for its password again instead.
    pub(super) fn sign_in(&mut self, cx: &mut Context<Self>) {
        if self.is_microsoft(&self.account_id) {
            self.sign_in_to_microsoft_again(cx);
            return;
        }
        if self.servers_of(&self.account_id).is_some() {
            let account = self.account_id.clone();
            self.open_account_sheet(Some(account), cx);
            return;
        }
        let Some(config) = self.config.clone() else {
            return;
        };
        let account = self.account_id.clone();
        let generation = self.account_gen;
        let expected = self.email_of(&account);
        match gmail::SignIn::start(&config, false, expected.as_deref()) {
            Ok((pending, url)) => {
                cx.open_url(&url);
                self.status = Status::SigningIn;
                self.notice = None;
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move { pending.finish(&config, &account) })
                        .await;
                    this.update(cx, |this, cx| match result {
                        // Saved for its account, which is not the one shown any more.
                        _ if this.account_gen != generation => {}
                        Ok(client) => {
                            this.connection = Some(this.signed_in_again(client));
                            this.open_store(cx);
                        }
                        Err(error) => {
                            this.status = Status::Failed(error.to_string().into());
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
            }
            Err(error) => self.status = Status::Failed(error.to_string().into()),
        }
        cx.notify();
    }

    /// Sign in again for a Microsoft account whose sign-in ended: Microsoft in the browser,
    /// pointed at its address, which is the only one accepted. Nothing is saved until it
    /// comes back.
    fn sign_in_to_microsoft_again(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.microsoft.clone() else {
            self.status = Status::Failed(NO_MICROSOFT.into());
            cx.notify();
            return;
        };
        let account = self.account_id.clone();
        let generation = self.account_gen;
        let expected = self.email_of(&account);
        match crate::microsoft::SignIn::start(&config, expected.as_deref()) {
            Ok((pending, url)) => {
                cx.open_url(&url);
                self.status = Status::SigningIn;
                self.notice = None;
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            let (email, tokens) = pending.finish(&config)?;
                            if let Some(expected) = &expected {
                                crate::microsoft::check_same_account(expected, &email)?;
                            }
                            anyhow::Ok((email, tokens))
                        })
                        .await;
                    this.update(cx, |this, cx| {
                        // Another account was switched to meanwhile: nothing is kept.
                        if this.account_gen != generation {
                            return;
                        }
                        let kept = match result {
                            Ok((email, tokens)) => this.keep_imap_account(
                                Some(account),
                                email.clone(),
                                Credential::Microsoft(Arc::new(tokens)),
                                crate::microsoft::servers(&email),
                                false,
                                cx,
                            ),
                            Err(error) => Err(error.to_string().into()),
                        };
                        if let Err(error) = kept {
                            this.status = Status::SignedOut;
                            this.notice = Some(error);
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            Err(error) => self.status = Status::Failed(error.to_string().into()),
        }
        cx.notify();
    }

    /// The account shown has its own servers (IMAP and SMTP), not Gmail.
    pub(super) fn own_servers(&self) -> bool {
        self.servers_of(&self.account_id).is_some()
    }

    /// An account's servers, if it is not Gmail.
    pub(super) fn servers_of(&self, id: &str) -> Option<crate::servers::Servers> {
        self.settings
            .accounts
            .iter()
            .find(|a| a.id == id)
            .and_then(|a| a.servers.clone())
    }

    /// The account signs in with Microsoft (its servers are Microsoft's).
    pub(super) fn is_microsoft(&self, id: &str) -> bool {
        self.settings
            .accounts
            .iter()
            .any(|a| a.id == id && nimbra_core::accounts::is_microsoft(a))
    }

    /// What reading an account's credential needs (see `stored_credential`): Microsoft's
    /// setup for a Microsoft account. `Err` when it is one and that is not set up here.
    fn credential_setup(&self, id: &str) -> Result<Option<crate::microsoft::Config>, ()> {
        if !self.is_microsoft(id) {
            return Ok(None);
        }
        self.microsoft.clone().map(Some).ok_or(())
    }

    /// The connection to an account with its own servers, from its saved password or
    /// sign-in: `None` when there is none (yet, or any more).
    fn imap_connection(
        &self,
        id: &str,
        credential: Option<Credential>,
    ) -> Option<Arc<mail::Client>> {
        let account = self.settings.accounts.iter().find(|a| a.id == id)?;
        nimbra_core::accounts::imap_connection(account, credential)
    }

    /// Signs in to another Gmail account in the browser, then switches to it. The account
    /// shown stays as it is until that sign-in finishes.
    pub(super) fn add_google_account(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            self.notice =
                Some("Adding a Gmail account needs the Google sign-in set up first.".into());
            cx.notify();
            return;
        };
        let Ok(account) = mail::new_account_id() else {
            return;
        };
        match gmail::SignIn::start(&config, false, None) {
            Ok((pending, url)) => {
                cx.open_url(&url);
                self.notice = Some("Sign in to the other account in your browser.".into());
                cx.spawn(async move |this, cx| {
                    let id = account.clone();
                    let result = cx
                        .background_executor()
                        .spawn(async move { pending.finish(&config, &id) })
                        .await;
                    this.update(cx, |this, cx| match result {
                        Ok(client) => {
                            this.reset_account(account, cx);
                            // A new account: its address is known once its profile is read.
                            this.connection = Some(crate::contacts::wrap(client, ""));
                            this.open_store(cx);
                        }
                        Err(error) => {
                            this.notice = Some(format!("No account was added: {error}").into());
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
            }
            Err(error) => self.notice = Some(error.to_string().into()),
        }
        cx.notify();
    }

    /// Shows another of the accounts signed in on this Mac.
    pub(super) fn switch_account(&mut self, id: String, cx: &mut Context<Self>) {
        self.leave_sidebar(cx);
        let reachable = self.config.is_some() || self.servers_of(&id).is_some();
        if !reachable || id == self.account_id || !mail::is_account_id(&id) {
            return;
        }
        self.reset_account(id, cx);
        self.connect(false, cx);
    }

    /// Connects the account shown with its saved sign-in. `sign_in` is for when there is
    /// none: whether to go on to Google, as the button that asked for it says.
    pub(super) fn connect(&mut self, sign_in: bool, cx: &mut Context<Self>) {
        if self.servers_of(&self.account_id).is_some() {
            self.connect_imap(sign_in, cx);
            return;
        }
        let Some(config) = self.config.clone() else {
            return;
        };
        let id = self.account_id.clone();
        let generation = self.account_gen;
        let reading = self.read_keychain(cx);
        cx.spawn(async move |this, cx| {
            let key = id.clone();
            let token = cx
                .background_executor()
                .spawn(async move {
                    let _reading = reading;
                    gmail::stored_refresh_token(&key)
                })
                .await;
            this.update(cx, |this, cx| {
                if this.account_gen != generation
                    || this.account_id != id
                    || this.connection.is_some()
                {
                    return;
                }
                match token {
                    Ok(Some(token)) => {
                        let email = this.email_of(&id).unwrap_or_default();
                        this.connection = Some(crate::contacts::wrap(
                            gmail::Client::new(config, token, id),
                            &email,
                        ));
                        this.open_store(cx);
                    }
                    Ok(None) if sign_in => this.sign_in(cx),
                    Ok(None) => this.status = Status::SignedOut,
                    // Not signed out: signing in over it could wipe this account, or send
                    // its mail from another.
                    Err(error) => {
                        eprintln!("nimbra: {error:#}");
                        this.status = Status::Failed(KEYCHAIN_REFUSED.into());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// `connect` for an account with its own servers: its password, or its Microsoft
    /// sign-in, from the keychain.
    fn connect_imap(&mut self, sign_in: bool, cx: &mut Context<Self>) {
        let id = self.account_id.clone();
        let generation = self.account_gen;
        let Ok(microsoft) = self.credential_setup(&id) else {
            self.status = Status::Failed(NO_MICROSOFT.into());
            cx.notify();
            return;
        };
        let reading = self.read_keychain(cx);
        cx.spawn(async move |this, cx| {
            let key = id.clone();
            let credential = cx
                .background_executor()
                .spawn(async move {
                    let _reading = reading;
                    stored_credential(&key, microsoft.as_ref())
                })
                .await;
            this.update(cx, |this, cx| {
                if this.account_gen != generation
                    || this.account_id != id
                    || this.connection.is_some()
                {
                    return;
                }
                match credential {
                    Ok(credential) => match this.imap_connection(&id, credential) {
                        Some(connection) => {
                            this.connection = Some(connection);
                            this.open_store(cx);
                        }
                        None if sign_in => this.sign_in(cx),
                        None => this.status = Status::SignedOut,
                    },
                    Err(error) => {
                        eprintln!("nimbra: {error:#}");
                        this.status = Status::Failed(KEYCHAIN_REFUSED.into());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Counts a read of the keychain. One with no answer is never given up on, as macOS
    /// can be asking for the password, but after a while the screen says what is waited for.
    pub(super) fn read_keychain(&mut self, cx: &mut Context<Self>) -> leaving::Working {
        self.keychain_slow = false;
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(KEYCHAIN_WAIT).await;
            this.update(cx, |this, cx| {
                if this.account_gen == generation && this.keychain.any() {
                    this.keychain_slow = true;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        self.keychain.begin()
    }

    pub(super) fn waiting_for_keychain(&self) -> bool {
        self.status == Status::Loading
            && self.store.is_none()
            && self.keychain_slow
            && self.keychain.any()
    }

    /// The status screen's Try again.
    pub(super) fn try_again(&mut self, cx: &mut Context<Self>) {
        if self.connection.is_none() {
            self.status = Status::Loading;
            self.connect(true, cx);
        } else if self.store.is_none() {
            // Asks the keychain again.
            self.open_store(cx);
        } else {
            self.fetch_inbox(cx);
        }
        cx.notify();
    }

    /// Opens a connection to every signed-in account that is not shown (their sign-ins are
    /// in the keychain). Accounts already connected are left as they are.
    pub(super) fn connect_others(&mut self, cx: &mut Context<Self>) {
        let config = self.config.clone();
        let missing: Vec<String> = self
            .settings
            .accounts
            .iter()
            .map(|a| a.id.clone())
            .filter(|id| *id != self.account_id && !self.others.contains_key(id))
            .filter(|id| mail::is_account_id(id))
            .collect();
        if missing.is_empty() {
            return;
        }
        // Accounts with their own servers sign in with a password or Microsoft; Gmail ones
        // with Google.
        let (imap, missing): (Vec<String>, Vec<String>) = missing
            .into_iter()
            .partition(|id| self.servers_of(id).is_some());
        // A Microsoft account without Microsoft's setup here waits until it is shown.
        let imap: Vec<(String, Option<crate::microsoft::Config>)> = imap
            .into_iter()
            .filter_map(|id| Some((id.clone(), self.credential_setup(&id).ok()?)))
            .collect();
        if !imap.is_empty() {
            cx.spawn(async move |this, cx| {
                let found = cx
                    .background_executor()
                    .spawn(async move {
                        imap.into_iter()
                            .filter_map(|(id, microsoft)| {
                                Some((stored_credential(&id, microsoft.as_ref()).ok()??, id))
                            })
                            .collect::<Vec<_>>()
                    })
                    .await;
                this.update(cx, |this, cx| {
                    for (credential, id) in found {
                        if id != this.account_id
                            && !this.others.contains_key(&id)
                            && let Some(connection) = this.imap_connection(&id, Some(credential))
                        {
                            this.others.insert(id.clone(), connection);
                            this.open_other_store(id, cx);
                        }
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        let Some(config) = config else {
            return;
        };
        if missing.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async move {
                    missing
                        .into_iter()
                        .filter_map(|id| Some((gmail::stored_refresh_token(&id).ok()??, id)))
                        .map(|(token, id)| (id, token))
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                for (id, token) in found {
                    if id != this.account_id && this.settings.accounts.iter().any(|a| a.id == id) {
                        let client = gmail::Client::new(config.clone(), token, id.clone());
                        let email = this.email_of(&id).unwrap_or_default();
                        this.others
                            .insert(id.clone(), crate::contacts::wrap(client, &email));
                        this.open_other_store(id, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Opens the store of a connected account that is not shown.
    fn open_other_store(&self, account: String, cx: &mut Context<Self>) {
        let Some(client) = self.others.get(&account).cloned() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let key = account.clone();
            let opened = cx
                .background_executor()
                .spawn(async move {
                    let store = Arc::new(crate::store::Store::open_quietly(&key)?);
                    client.keep_rows_in(&store);
                    let waiting = store
                        .load::<Outbox>("outbox")
                        .is_some_and(|outbox| !outbox.changes.is_empty());
                    let snoozed: Vec<Snooze> =
                        store.load(nimbra_core::snooze::LIST).unwrap_or_default();
                    let inbox_empty = crate::store::Store::saved_empty(&key, &record(View::Inbox));
                    anyhow::Ok((store, waiting, snoozed, inbox_empty))
                })
                .await;
            let Ok((store, waiting, snoozed, inbox_empty)) = opened else {
                return;
            };
            this.update(cx, |this, cx| {
                // Not if it was opened or signed out meanwhile.
                if account != this.account_id && this.others.contains_key(&account) {
                    if waiting {
                        this.away_waiting.insert(account.clone());
                    }
                    this.other_stores.insert(account.clone(), store);
                    // What this app saved since, if anything, knows better.
                    this.inboxes_saved_empty
                        .entry(account.clone())
                        .or_insert(inbox_empty);
                    this.snoozed_away.insert(account.clone(), (0, snoozed));
                    this.read_snoozed_elsewhere();
                    this.scan_contacts(account.clone(), cx);
                    this.load_photo(account, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// The connection for an account: the shown one, or another signed-in one.
    /// Reads the people of the account's newest sent and received mail for address
    /// suggestions, once per account and session, a page every `SCAN_PAUSE` (Gmail's
    /// quota is the user's). It carries on from where it got to before, and stops for the
    /// session at the first failure (offline: the next launch tries again), or once the
    /// account's connection is not this one any more (signed out, removed).
    pub(super) fn scan_contacts(&mut self, account: String, cx: &mut Context<Self>) {
        let Some(client) = self.client_for(Some(&account)) else {
            return;
        };
        if client.contacts().is_none() || !self.contacts_scans.insert(account.clone()) {
            return;
        }
        cx.spawn(async move |this, cx| {
            loop {
                // Also before the first page: the mailbox loading at launch goes first.
                cx.background_executor()
                    .timer(crate::contacts::SCAN_PAUSE)
                    .await;
                let same = this
                    .update(cx, |this, _| {
                        this.client_for(Some(&account)).is_some_and(|now| {
                            std::ptr::addr_eq(Arc::as_ptr(&now), Arc::as_ptr(&client))
                        })
                    })
                    .unwrap_or(false);
                if !same {
                    break;
                }
                let step = {
                    let client = client.clone();
                    cx.background_executor()
                        .spawn(async move {
                            match client.contacts() {
                                Some(contacts) => contacts.scan_step(&*client),
                                None => Ok(false),
                            }
                        })
                        .await
                };
                match step {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(error) => {
                        eprintln!("nimbra: could not read the people of past mail: {error:#}");
                        break;
                    }
                }
            }
            this.update(cx, |this, _| this.contacts_scans.remove(&account))
                .ok();
        })
        .detach();
    }

    /// A new Gmail sign-in to the account shown, as the connection to keep: it carries on
    /// with the people the one it replaces learnt (`contacts.rs`).
    fn signed_in_again(&self, client: gmail::Client) -> Arc<mail::Client> {
        match self.connection.as_ref().and_then(|c| c.contacts()) {
            Some(contacts) => crate::contacts::wrap_sharing(client, contacts.clone()),
            None => {
                crate::contacts::wrap(client, &self.email_of(&self.account_id).unwrap_or_default())
            }
        }
    }

    pub(super) fn client_for(&self, account: Option<&str>) -> Option<Arc<mail::Client>> {
        match account {
            Some(id) if id != self.account_id => self.others.get(id).cloned(),
            _ => self.connection.clone(),
        }
    }

    /// The thread in row `index` belongs to an account other than the one shown.
    pub(super) fn is_foreign(&self, index: usize) -> bool {
        self.mails
            .get(index)
            .and_then(|m| m.account.as_deref())
            .is_some_and(|a| a != self.account_id)
    }

    /// The connection for the thread in row `index` (its own account in All inboxes).
    pub(super) fn client_at(&self, index: usize) -> Option<Arc<mail::Client>> {
        let account = self.mails.get(index).and_then(|m| m.account.clone());
        self.client_for(account.as_deref())
    }

    /// Clears everything that belongs to the account shown, and makes `id` the account shown.
    pub(super) fn reset_account(&mut self, id: String, cx: &mut Context<Self>) {
        self.close_menu_window(cx);
        self.close_undo(cx);
        // What is being written is saved to its account's Drafts.
        self.close_compose(cx);
        // What is still finishing in the background stays with the account it is from.
        for (_, composer) in &mut self.parked {
            let from = composer
                .account
                .take()
                .unwrap_or_else(|| self.account_id.clone());
            composer.account = (from != id).then_some(from);
        }
        // All inboxes is the same whichever account is shown: it outlives a switch, so that
        // it does not load again each time. Not for an account being added (its mail is not
        // in it), and without the mail of one signed out.
        self.remember_mailbox();
        let listed = |account: &str| self.settings.accounts.iter().any(|a| a.id == account);
        let all_inboxes = self
            .cache
            .get(&View::ALL_INBOXES)
            .filter(|_| listed(&id))
            .map(|mails| {
                let kept: Vec<Mail> = mails
                    .iter()
                    .filter(|mail| mail.account.as_deref().is_some_and(listed))
                    .cloned()
                    .collect();
                (kept, self.all_inboxes_whole)
            });
        // While it was shown its count was kept in `counts`: hand that on, or the sidebar
        // and the Dock go back to the number from before it was shown.
        if listed(&self.account_id) && self.counts.contains_key("INBOX") {
            let unread = self.inbox_unread();
            self.account_unread.insert(self.account_id.clone(), unread);
        }
        // Its other counts too, for the sidebar's counts of every account's mailboxes.
        if listed(&self.account_id) && !self.counts.is_empty() {
            self.account_counts
                .insert(self.account_id.clone(), self.counts.clone());
        }
        self.account_gen += 1;
        // The account shown so far stays connected (All inboxes, new-mail checks).
        // The first account's id is "": it is an account like the others.
        if let Some(client) = self.connection.take()
            && self
                .settings
                .accounts
                .iter()
                .any(|a| a.id == self.account_id)
        {
            self.others.insert(self.account_id.clone(), client);
            if let Some(store) = self.store.take() {
                if !self.outbox.changes.is_empty() {
                    self.away_waiting.insert(self.account_id.clone());
                }
                self.other_stores.insert(self.account_id.clone(), store);
                // Its list as kept here, which is its store's (changes are handed over).
                let snoozed = std::mem::take(&mut self.snoozed);
                self.snoozed_away
                    .insert(self.account_id.clone(), (0, snoozed));
            }
        }
        self.others.remove(&id);
        self.other_stores.remove(&id);
        self.snoozed_away.remove(&id);
        self.away_waiting.remove(&id);
        let reading_queue = self.outbox_pending.take().is_some();
        self.opening = None;
        let left = self.account_id.clone();
        self.inbox_saved_empty = crate::store::Store::saved_empty(&id, &record(View::Inbox));
        self.account_id = id.clone();
        self.settings.active = id;
        self.save_settings(cx);
        self.connection = None;
        self.store = None;
        self.status = Status::Loading;
        self.checks.forget();
        // A question that is still up for the account left is not this one's.
        self.keychain = Underway::default();
        self.keychain_slow = false;
        // What was on its way is for the other account, and its answer is dropped.
        self.prefetching = false;
        self.loading_more = false;
        self.body_failed.clear();
        self.mailbox = View::Inbox;
        self.mails.clear();
        self.done.clear();
        self.history.clear();
        self.history_groups.clear();
        self.cache.clear();
        self.counts.clear();
        self.labels.clear();
        self.synced_to.clear();
        self.scheduled.clear();
        self.sending_scheduled.clear();
        self.snoozed.clear();
        self.waking.clear();
        let unkept = std::mem::take(&mut self.outbox.changes);
        if !self.other_stores.contains_key(&left) && !unkept.is_empty() {
            self.homeless.insert(left.clone(), unkept);
        } else if reading_queue && !unkept.is_empty() {
            // Queued while its store's queue was read, so in no store yet: after that one.
            self.queue_away(&left, unkept);
        }
        self.outbox = Outbox {
            changes: self.homeless.remove(&self.account_id).unwrap_or_default(),
            ..Outbox::default()
        };
        self.offline = false;
        self.stop_retrying();
        self.picked.clear();
        // Asked about the other account's threads and labels, whose ids mean others here.
        self.confirm = None;
        self.new_label = None;
        self.next_pages.clear();
        self.forget_checked();
        self.more_failed.clear();
        self.back.clear();
        self.forward.clear();
        self.selected = 0;
        self.palette = false;
        self.removed_by.clear();
        self.moved.clear();
        self.notify_from = None;
        self.details = None;
        self.open_when_listed = None;
        self.search_opened = None;
        self.html_shown = None;
        self.mail_views.clear();
        self.pending.clear();
        self.print_view = None;
        self.account = "".into();
        self.me_initials = "".into();
        if let Some((mails, whole)) = all_inboxes {
            self.cache.insert(View::ALL_INBOXES, mails);
            self.all_inboxes_whole = whole;
        }
        // Left before its store had opened.
        if !self.other_stores.contains_key(&left) {
            self.open_other_store(left, cx);
        }
        self.pending_focus = Some(self.focus.clone());
        cx.notify();
    }

    /// Remembers the account shown in the account list (once its address is known). Signing
    /// in to an account that is already listed keeps the existing entry.
    pub(super) fn remember_account(&mut self, email: &str, cx: &mut Context<Self>) {
        let id = self.account_id.clone();
        if let Some(other) =
            nimbra_core::accounts::duplicate_of(&self.settings, email, &id).map(|a| a.id.clone())
        {
            // The same Google account twice: drop the new sign-in, show the existing one.
            let duplicate = id.clone();
            std::thread::spawn(move || nimbra_core::accounts::forget_secrets_and_mail(&duplicate));
            self.notice = Some("That account is already in Nimbra.".into());
            self.switch_account(other, cx);
            return;
        }
        match self.settings.accounts.iter_mut().find(|a| a.id == id) {
            Some(entry) if entry.email == email => return,
            Some(entry) => entry.email = email.to_string(),
            None => self.settings.accounts.push(crate::settings::AccountRef {
                id: id.clone(),
                email: email.to_string(),
                servers: None,
                oauth: None,
            }),
        }
        self.settings.active = id;
        self.save_settings(cx);
    }

    /// An account's profile photo: the copy kept on this Mac first, then Google's current
    /// one (kept for next time). Once per account per session, shown or not.
    pub(super) fn load_photo(&mut self, account: String, cx: &mut Context<Self>) {
        let Some(client) = self.client_for(Some(&account)) else {
            return;
        };
        if !self.photos_asked.insert(account.clone()) {
            return;
        }
        let store = self.store_of(&account);
        cx.spawn(async move |this, cx| {
            if let Some(store) = store.clone() {
                let kept = cx
                    .background_executor()
                    .spawn(async move { store.load::<Vec<u8>>(PHOTO) })
                    .await;
                if let Some(image) = kept.and_then(photo_image) {
                    this.update(cx, |this, cx| {
                        if this.keeps(&account) {
                            this.photos.insert(account.clone(), image);
                            cx.notify();
                        }
                    })
                    .ok();
                }
            }
            let fetched = cx
                .background_executor()
                .spawn(async move {
                    let photo = client.profile_photo()?;
                    if let Some(store) = &store {
                        match &photo {
                            mail::Photo::Image(bytes) | mail::Photo::Unchecked(bytes) => {
                                if store.save(PHOTO, bytes).is_err() {
                                    eprintln!("nimbra: could not keep the profile photo");
                                }
                            }
                            mail::Photo::NoPhoto => store.remove(PHOTO),
                            mail::Photo::NotAllowed => {}
                        }
                    }
                    anyhow::Ok(photo)
                })
                .await;
            this.update(cx, |this, cx| {
                // Signed out meanwhile: nothing of it stays.
                if !this.keeps(&account) {
                    return;
                }
                let photo = match fetched {
                    Ok(photo) => photo,
                    Err(error) => {
                        // Never the address or the photo's link.
                        eprintln!("nimbra: profile photo not loaded ({error})");
                        this.photos_asked.remove(&account); // asked again next time
                        return;
                    }
                };
                let (answer, bytes) = match photo {
                    mail::Photo::Image(bytes) => (PhotoAnswer::Theirs, Some(bytes)),
                    mail::Photo::Unchecked(bytes) => (PhotoAnswer::Unchecked, Some(bytes)),
                    mail::Photo::NoPhoto => (PhotoAnswer::None, None),
                    mail::Photo::NotAllowed => (PhotoAnswer::NotAllowed, None),
                };
                if let Some(image) = bytes.and_then(photo_image) {
                    this.photos.insert(account.clone(), image);
                } else if answer == PhotoAnswer::None {
                    this.photos.remove(&account);
                }
                this.photo_answers.insert(account, answer);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Settings › Accounts › Check again.
    pub(super) fn check_photo_again(&mut self, cx: &mut Context<Self>) {
        let account = self.account_id.clone();
        self.photo_answers.remove(&account);
        self.photos_asked.remove(&account);
        self.load_photo(account, cx);
        cx.notify();
    }

    /// Forgets an account's photo, as it is signed out.
    pub(super) fn forget_photo(&mut self, account: &str) {
        self.photos.remove(account);
        self.photo_answers.remove(account);
        self.photos_asked.remove(account);
    }

    /// The address an account has, to point signing in again at and to check it by.
    pub(super) fn email_of(&self, account: &str) -> Option<String> {
        self.settings
            .accounts
            .iter()
            .find(|a| a.id == account && a.email.contains('@'))
            .map(|a| a.email.clone())
    }

    /// Settings › Accounts: signs in again to add the profile (name and photo) to this
    /// account's sign-in, then shows the photo.
    pub(super) fn allow_profile_photo(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            return;
        };
        let account = self.account_id.clone();
        let generation = self.account_gen;
        let expected = self.email_of(&account);
        match gmail::SignIn::start(&config, false, expected.as_deref()) {
            Ok((pending, url)) => {
                cx.open_url(&url);
                self.notice = Some("Allow your profile in your browser, then come back.".into());
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move { pending.finish(&config, &account) })
                        .await;
                    this.update(cx, |this, cx| match result {
                        // Another account was switched to while the browser was open.
                        Ok(_) if this.account_gen != generation => {}
                        Ok(client) => {
                            this.connection = Some(this.signed_in_again(client));
                            this.notice = None;
                            this.check_photo_again(cx);
                        }
                        Err(error) => {
                            this.notice = Some(format!("The photo was not added: {error}").into());
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
            }
            Err(error) => self.notice = Some(error.to_string().into()),
        }
        cx.notify();
    }

    /// Puts a new listing on screen, keeping downloaded thread contents, the open thread and
    /// threads cleared in the meantime, then saves it.
    pub(super) fn install(&mut self, fresh: Vec<Mail>, cx: &mut Context<Self>) {
        let mut fresh = fresh;
        // With what was changed here and is not listed yet (`pending`).
        let shown = self.account_id.clone();
        self.pending
            .apply(self.mailbox, &shown, &mut fresh, true, false, |m| {
                m.account.clone().unwrap_or_else(|| shown.clone())
            });
        // For the chips of other accounts' snoozed threads (All inboxes), and Snoozed.
        self.read_snoozed_elsewhere();
        // Snoozed lists Gmail's own snoozed threads and the ones snoozed here; Reminders,
        // only the ones here.
        if matches!(self.mailbox.base(), View::Snoozed | View::Reminders) {
            for mail in self.snoozed_rows(self.mailbox.base() == View::Reminders) {
                if !fresh.iter().any(|m| m.id.is_some() && m.id == mail.id) {
                    fresh.push(mail);
                }
            }
            fresh.sort_by_key(|m| std::cmp::Reverse(m.timestamp_ms));
        }
        if inbox_like(self.mailbox) {
            self.unsnooze_answered(&fresh);
        }
        // Everything kept by position follows its thread into the new list.
        let relisted = crate::listing::relist(
            &self.mails,
            fresh,
            &self.history,
            |m| self.thread_key(m),
            |m| m.timestamp_ms,
        );
        self.mails = crate::data::merge(&self.mails, relisted.list);
        let now = |row: usize| relisted.moved.get(row).copied().flatten();
        for &position in relisted.back.iter().rev() {
            self.forget_cleared(position, cx);
        }
        // All found: a cleared thread is never dropped from the list.
        self.history = self.history.iter().filter_map(|&row| now(row)).collect();
        self.done = vec![false; self.mails.len()];
        for i in &self.history {
            self.done[*i] = true;
        }
        self.picked = self.picked.iter().filter_map(|&row| now(row)).collect();
        self.selected = now(self.selected).unwrap_or(0);
        // One picked row left is simply the open one (as when ⌘-click leaves one).
        if self.picked.len() < 2
            && let Some(only) = self.picked.pop_first()
        {
            self.selected = only;
        }
        if self.done.get(self.selected) == Some(&true) {
            self.selected = self.visible().first().copied().unwrap_or(0);
        }
        self.back.clear();
        self.forward.clear();
        if self.notice != self.must_show {
            self.notice = None;
        }
        self.status = Status::Ready;
        self.cache.insert(self.mailbox, self.listed());
        self.persist(cx);
        self.prefetch(cx);
        let opened = self.search_opened.take();
        if let Some(id) = self.open_when_listed.take()
            && !self.open_listed(&id, cx)
            && self.mailbox == View::Search
            && let Some(mail) = opened
            && mail.id.as_deref() == Some(id.as_str())
        {
            // Not in Gmail's results (another folder, or further down): kept at the top.
            self.mails.insert(0, mail);
            self.done.insert(0, false);
            self.history.iter_mut().for_each(|row| *row += 1);
            self.picked = self.picked.iter().map(|row| row + 1).collect();
            self.selected += 1;
            self.open(0);
            self.open_selected(cx);
        }
        // Counts only; never log addresses, subjects or message text.
        eprintln!(
            "nimbra: {:?} listed ({} threads, {} already in memory, {} sidebar counts)",
            self.mailbox,
            self.mails.len(),
            self.mails.iter().filter(|m| m.loaded).count(),
            self.counts.len()
        );
        self.load_selected(cx);
        cx.notify();
        // Logos for the senders listed (each domain once).
        self.ask_brand_logos(cx);
    }

    /// Signs in again in the browser, this time also allowing permanent deletion, then
    /// deletes the threads that were waiting.
    pub(super) fn grant_full_access(&mut self, ids: Vec<(String, String)>, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            return;
        };
        let account = self.account_id.clone();
        let generation = self.account_gen;
        let expected = self.email_of(&account);
        match gmail::SignIn::start(&config, true, expected.as_deref()) {
            Ok((pending, url)) => {
                cx.open_url(&url);
                self.notice =
                    Some("Allow permanent deletion in your browser, then come back.".into());
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move { pending.finish(&config, &account) })
                        .await;
                    this.update(cx, |this, cx| match result {
                        // Those threads are not the account's that is shown by now.
                        Ok(_) if this.account_gen != generation => {}
                        Ok(client) => {
                            this.connection = Some(this.signed_in_again(client));
                            this.delete_forever(ids, cx);
                        }
                        Err(error) => {
                            this.notice = Some(format!("Nothing was deleted: {error}").into());
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
            }
            Err(error) => self.notice = Some(error.to_string().into()),
        }
        cx.notify();
    }

    /// Settings › Accounts: moves an account to another place in the list (by dragging).
    /// The order is the sidebar's, All inboxes' and From's.
    pub(super) fn move_account(&mut self, id: &str, to: usize, cx: &mut Context<Self>) {
        if self.settings.move_account(id, to) {
            self.save_settings(cx);
            cx.notify();
        }
    }

    /// Settings › Accounts › Make default.
    pub(super) fn set_default_account(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.settings.accounts.iter().any(|a| a.id == id) {
            return;
        }
        self.settings.default_account = Some(id.clone());
        self.save_settings(cx);
        self.load_photo(id, cx);
        cx.notify();
    }

    /// Settings › Privacy: forgets the mail kept on this Mac (Gmail is untouched) and loads
    /// the open mailbox again.
    pub(super) fn clear_local_mail(&mut self, cx: &mut Context<Self>) {
        if let Some(store) = &self.store {
            store.clear_soon();
        }
        self.cache.clear();
        self.synced_to.clear();
        self.notify_from = None;
        self.forget_checked();
        for client in self.connection.iter().chain(self.others.values()) {
            client.forget_rows();
        }
        self.notice =
            Some("Cleared the mail kept on this Mac. It is downloaded again as needed.".into());
        self.fetch_inbox(cx);
        cx.notify();
    }

    /// Settings › Accounts › Remove: takes an account off this Mac. Its sign-in (Google's, the
    /// password, or Microsoft's), the mail kept for it and its key are deleted; the mail on
    /// its server is not touched. The account shown signs out as before; any other goes
    /// without switching away from what is open.
    pub(super) fn remove_account(&mut self, account: String, cx: &mut Context<Self>) {
        if account == self.account_id {
            self.sign_out(cx);
            return;
        }
        let Some(email) = self
            .settings
            .accounts
            .iter()
            .find(|a| a.id == account)
            .map(|a| a.email.clone())
        else {
            return;
        };
        // As signing out: a message on its way waits to finish first.
        let on_its_way = self.sends.iter().any(|job| {
            job.account == account && !self.queue.iter().any(|waiting| Arc::ptr_eq(waiting, job))
        });
        if on_its_way {
            self.notice = Some(
                "A message from this account is still being sent. Sign out again in a moment."
                    .into(),
            );
            cx.notify();
            return;
        }
        self.abandon_sends(&account);
        let gone = account.clone();
        std::thread::spawn(move || {
            nimbra_core::accounts::forget_secrets_and_mail(&gone);
            forget_previews(&gone);
        });
        nimbra_core::accounts::remove(&mut self.settings, &account);
        self.others.remove(&account);
        self.forget_photo(&account);
        self.other_stores.remove(&account);
        self.snoozed_away.remove(&account);
        self.away_waiting.remove(&account);
        self.account_unread.remove(&account);
        self.account_counts.remove(&account);
        self.inboxes_saved_empty.remove(&account);
        self.snoozed_elsewhere
            .retain(|s| s.mail.account.as_deref() != Some(account.as_str()));
        self.save_settings(cx);
        // Every account's mailboxes kept so far still have its mail.
        self.cache.retain(|view, _| !view.is_every());
        // The one open: listed again, or, with one account left, its inbox.
        if self.mailbox.is_every() {
            if self.settings.accounts.len() < 2 {
                self.select_mailbox(View::Inbox, cx);
            } else {
                self.refresh(cx);
            }
        }
        self.notice = Some(format!("Removed {email}. Its mail was removed from this Mac.").into());
        cx.notify();
    }

    /// Signs out: forgets the sign-in (Google's, the password, or Microsoft's) and removes the mail kept on this Mac and its
    /// key. Settings (signature, theme) stay.
    pub(super) fn sign_out(&mut self, cx: &mut Context<Self>) {
        let account = self.account_id.clone();
        // Signing out deletes what is kept of it.
        // (Not what waits for a connection: that would hold signing out up for as long.)
        let on_its_way = self.sends.iter().any(|job| {
            job.account == account && !self.queue.iter().any(|waiting| Arc::ptr_eq(waiting, job))
        });
        if on_its_way {
            self.notice = Some(
                "A message from this account is still being sent. Sign out again in a moment."
                    .into(),
            );
            cx.notify();
            return;
        }
        self.abandon_sends(&account);
        let gone = account.clone();
        std::thread::spawn(move || {
            nimbra_core::accounts::forget_secrets_and_mail(&gone);
            forget_previews(&gone);
        });
        self.settings.accounts.retain(|a| a.id != account);
        self.others.remove(&account);
        self.forget_photo(&account);
        self.other_stores.remove(&account);
        self.snoozed_away.remove(&account);
        self.away_waiting.remove(&account);
        self.account_unread.remove(&account);
        self.inboxes_saved_empty.remove(&account);
        // Another account is signed in: show that one.
        if let Some(next) = self.settings.accounts.first().map(|a| a.id.clone()) {
            self.switch_account(next, cx);
            self.notice =
                Some("Signed out of that account. Its mail was removed from this Mac.".into());
            return;
        }
        self.settings.active = String::new();
        self.account_id = String::new();
        self.save_settings(cx);
        self.connection = None;
        self.store = None;
        // An answer still on its way must not bring the removed mail back.
        self.checks.forget();
        self.offline = false;
        self.stop_retrying();
        // Whoever signs in next must not send this account's queue.
        self.outbox = Outbox::default();
        self.outbox_pending = None;
        self.next_pages.clear();
        self.forget_checked();
        self.cache.clear();
        self.mails.clear();
        self.done.clear();
        self.history.clear();
        self.picked.clear();
        self.history_groups.clear();
        self.back.clear();
        self.forward.clear();
        self.scheduled.clear();
        self.parked.clear();
        self.compose = None;
        self.selected = 0;
        self.palette = false;
        self.account = "".into();
        self.status = Status::SignedOut;
        self.notice = Some("Signed out. The mail kept on this Mac was removed.".into());
        cx.notify();
    }

    /// Whether an error ends the shown account's sign-in. Other accounts' connections are in
    /// use too (All inboxes, message windows, requests from before a switch): theirs do not.
    pub(super) fn shown_signed_out(&self, error: &anyhow::Error) -> bool {
        error
            .downcast_ref::<mail::SignedOut>()
            .is_some_and(|e| e.account.is_empty() || e.account == self.account_id)
    }
}
