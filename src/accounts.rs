//! Accounts: signing in and out, connecting each one to Gmail (the keychain, its store, its
//! client), switching between them, and their profile photos.

use super::*;

impl Nimbra {
    /// Opens Google's consent page in the browser and waits for it in the background.
    pub(super) fn sign_in(&mut self, cx: &mut Context<Self>) {
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
                            this.gmail = Some(Arc::new(client));
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

    /// Add account: signs in to another Gmail account in the browser, then switches to it.
    /// The account shown stays as it is until that sign-in finishes.
    pub(super) fn add_account(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            self.notice = Some("Connecting Gmail needs the Google sign-in set up first.".into());
            cx.notify();
            return;
        };
        let Ok(account) = gmail::new_account_id() else {
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
                            this.gmail = Some(Arc::new(client));
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
        if self.config.is_none() || id == self.account_id || !gmail::is_account_id(&id) {
            return;
        }
        self.reset_account(id, cx);
        self.connect(false, cx);
    }

    /// Connects the account shown with its saved sign-in. `sign_in` is for when there is
    /// none: whether to go on to Google, as the button that asked for it says.
    pub(super) fn connect(&mut self, sign_in: bool, cx: &mut Context<Self>) {
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
                if this.account_gen != generation || this.account_id != id || this.gmail.is_some() {
                    return;
                }
                match token {
                    Ok(Some(token)) => {
                        this.gmail = Some(Arc::new(gmail::Client::new(config, token, id)));
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
        if self.gmail.is_none() {
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
        let Some(config) = self.config.clone() else {
            return;
        };
        let missing: Vec<String> = self
            .settings
            .accounts
            .iter()
            .map(|a| a.id.clone())
            .filter(|id| *id != self.account_id && !self.others.contains_key(id))
            .filter(|id| gmail::is_account_id(id))
            .collect();
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
                        this.others.insert(id.clone(), Arc::new(client));
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
                    anyhow::Ok((store, waiting))
                })
                .await;
            let Ok((store, waiting)) = opened else {
                return;
            };
            this.update(cx, |this, cx| {
                // Not if it was opened or signed out meanwhile.
                if account != this.account_id && this.others.contains_key(&account) {
                    if waiting {
                        this.away_waiting.insert(account.clone());
                    }
                    this.other_stores.insert(account.clone(), store);
                    this.load_photo(account, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// The connection for an account: the shown one, or another signed-in one.
    pub(super) fn client_for(&self, account: Option<&str>) -> Option<Arc<gmail::Client>> {
        match account {
            Some(id) if id != self.account_id => self.others.get(id).cloned(),
            _ => self.gmail.clone(),
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
    pub(super) fn client_at(&self, index: usize) -> Option<Arc<gmail::Client>> {
        let account = self.mails.get(index).and_then(|m| m.account.clone());
        self.client_for(account.as_deref())
    }

    /// Clears everything that belongs to the account shown, and makes `id` the account shown.
    fn reset_account(&mut self, id: String, cx: &mut Context<Self>) {
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
            .get(&View::AllInboxes)
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
        if listed(&self.account_id)
            && let Some((unread, _)) = self.counts.get("INBOX")
        {
            self.account_unread.insert(self.account_id.clone(), *unread);
        }
        self.account_gen += 1;
        // The account shown so far stays connected (All inboxes, new-mail checks).
        // The first account's id is "": it is an account like the others.
        if let Some(client) = self.gmail.take()
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
            }
        }
        self.others.remove(&id);
        self.other_stores.remove(&id);
        self.away_waiting.remove(&id);
        self.inboxes_saved_empty.clear();
        let left = self.account_id.clone();
        self.inbox_saved_empty = crate::store::Store::saved_empty(&id, &record(View::Inbox));
        self.account_id = id.clone();
        self.settings.active = id;
        self.save_settings(cx);
        self.gmail = None;
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
        self.print_view = None;
        self.account = "".into();
        self.me_initials = "".into();
        if let Some((mails, whole)) = all_inboxes {
            self.cache.insert(View::AllInboxes, mails);
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
        if let Some(other) = self
            .settings
            .accounts
            .iter()
            .find(|a| a.email.eq_ignore_ascii_case(email) && a.id != id)
            .map(|a| a.id.clone())
        {
            // The same Google account twice: drop the new sign-in, show the existing one.
            let duplicate = id.clone();
            std::thread::spawn(move || {
                gmail::forget_refresh_token(&duplicate);
                crate::store::wipe(&duplicate);
            });
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
                            gmail::Photo::Image(bytes) | gmail::Photo::Unchecked(bytes) => {
                                if store.save(PHOTO, bytes).is_err() {
                                    eprintln!("nimbra: could not keep the profile photo");
                                }
                            }
                            gmail::Photo::NoPhoto => store.remove(PHOTO),
                            gmail::Photo::NotAllowed => {}
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
                    gmail::Photo::Image(bytes) => (PhotoAnswer::Theirs, Some(bytes)),
                    gmail::Photo::Unchecked(bytes) => (PhotoAnswer::Unchecked, Some(bytes)),
                    gmail::Photo::NoPhoto => (PhotoAnswer::None, None),
                    gmail::Photo::NotAllowed => (PhotoAnswer::NotAllowed, None),
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
    fn email_of(&self, account: &str) -> Option<String> {
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
                            this.gmail = Some(Arc::new(client));
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
        // Snoozed lists Gmail's own snoozed threads and the ones snoozed here.
        if self.mailbox == View::Snoozed {
            for snooze in &self.snoozed {
                if fresh
                    .iter()
                    .any(|m| m.id.as_deref() == Some(snooze.thread_id.as_str()))
                {
                    continue;
                }
                let mut mail = snooze.mail.clone();
                mail.time = chrono::DateTime::from_timestamp_millis(snooze.until_ms)
                    .map(|t| {
                        t.with_timezone(&chrono::Local)
                            .format("%a %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default()
                    .into();
                fresh.push(mail);
            }
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
    }

    /// Signs in again in the browser, this time also allowing permanent deletion, then
    /// deletes the threads that were waiting.
    pub(super) fn grant_full_access(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
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
                            this.gmail = Some(Arc::new(client));
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
            store.clear();
        }
        self.cache.clear();
        self.synced_to.clear();
        self.notify_from = None;
        self.forget_checked();
        for client in self.gmail.iter().chain(self.others.values()) {
            client.forget_rows();
        }
        self.notice =
            Some("Cleared the mail kept on this Mac. It is downloaded again as needed.".into());
        self.fetch_inbox(cx);
        cx.notify();
    }

    /// Signs out: forgets the Google sign-in and removes the mail kept on this Mac and its
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
            gmail::forget_refresh_token(&gone);
            crate::store::wipe(&gone);
        });
        self.settings.accounts.retain(|a| a.id != account);
        self.others.remove(&account);
        self.forget_photo(&account);
        self.other_stores.remove(&account);
        self.away_waiting.remove(&account);
        self.account_unread.remove(&account);
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
        self.gmail = None;
        self.store = None;
        // An answer still on its way must not bring the removed mail back.
        self.checks.forget();
        self.offline = false;
        self.stop_retrying();
        // Whoever signs in next must not send this account's queue.
        self.outbox = Outbox::default();
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
            .downcast_ref::<gmail::SignedOut>()
            .is_some_and(|e| e.account.is_empty() || e.account == self.account_id)
    }
}
