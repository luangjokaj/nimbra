//! The open mailbox: listing it from Gmail and from the local store, checking for changes,
//! loading more, opening a thread and fetching its content, and watching for new mail.

use super::*;
use nimbra_core::listing;

/// A mailbox clicked whose stored list is being read off the main thread. Kept by mailbox,
/// account and click, and checked when the list arrives.
pub(super) struct Opening {
    view: View,
    /// The mailbox on screen when it was clicked.
    from: View,
    account_gen: u64,
    serial: u64,
    /// It went on screen, loading, as the read took longer than a frame or two.
    shown: bool,
}

/// What to do with what was read for an `Opening`.
#[derive(Debug, PartialEq)]
enum Landing {
    /// Go to the mailbox and show it.
    Enter,
    /// It is on screen, loading: show it there.
    Fill,
    /// Not wanted any more.
    Stale,
}

impl Opening {
    /// Two frames at 60 Hz: a read that takes less shows no loading screen.
    const SKELETON_AFTER: Duration = Duration::from_millis(34);

    /// `waiting`: the screen is still empty and loading (nothing newer arrived).
    fn landing(&self, account_gen: u64, mailbox: View, waiting: bool) -> Landing {
        if account_gen != self.account_gen {
            Landing::Stale
        } else if !self.shown && mailbox == self.from {
            Landing::Enter
        } else if self.shown && mailbox == self.view && waiting {
            Landing::Fill
        } else {
            Landing::Stale
        }
    }
}

impl Nimbra {
    /// Saves the Inbox as soon as it empties or stops being empty. Waiting for the next
    /// check for mail would lose that when Nimbra quits first.
    pub(super) fn save_if_zero_changed(&mut self, cx: &mut Context<Self>) {
        self.save_inboxes_from_all();
        if self.mailbox == View::Inbox
            && self.status == Status::Ready
            // Not `visible`: with Unread on, read mail is still in the Inbox.
            && self.done.iter().all(|done| *done) != self.inbox_saved_empty
        {
            self.persist(cx);
        }
    }

    /// The same from All inboxes, for each account's Inbox. Only when it lists all of them
    /// whole: else what is not listed would be saved as gone.
    fn save_inboxes_from_all(&mut self) {
        if self.mailbox != View::ALL_INBOXES
            || self.status != Status::Ready
            || !self.all_inboxes_whole
        {
            return;
        }
        let accounts: Vec<String> = self
            .settings
            .accounts
            .iter()
            .map(|a| a.id.clone())
            .collect();
        for account in accounts {
            let left = |(mail, done): &(&Mail, &bool)| !**done && self.account_of(mail) == account;
            let empty = !self.mails.iter().zip(&self.done).any(|row| left(&row));
            // Not known yet: taken as changed, and saved, rather than asking the disk here.
            let saved = self
                .inboxes_saved_empty
                .get(&account)
                .copied()
                .or_else(|| (account == self.account_id).then_some(self.inbox_saved_empty))
                .unwrap_or(!empty);
            let Some(store) = self.store_of(&account).filter(|_| saved != empty) else {
                self.inboxes_saved_empty.insert(account, saved);
                continue;
            };
            let rows: Vec<Mail> = self
                .mails
                .iter()
                .zip(&self.done)
                .filter(left)
                .map(|(mail, _)| {
                    let mut mail = mail.clone();
                    mail.account = None; // as a store keeps its own account's
                    mail
                })
                .collect();
            if account == self.account_id {
                self.inbox_saved_empty = empty;
                self.cache.insert(View::Inbox, rows.clone());
                // It was listed whole: a page kept from before is none any more.
                self.next_pages.remove(&View::Inbox);
            }
            self.inboxes_saved_empty.insert(account, empty);
            store.save_soon(&record(View::Inbox), stored(&rows));
        }
    }

    /// The list as it is kept for the mailbox: without the threads cleared from it.
    pub(super) fn listed(&self) -> Vec<Mail> {
        self.mails
            .iter()
            .zip(&self.done)
            .filter(|(_, done)| !**done)
            .map(|(mail, _)| mail.clone())
            .collect()
    }

    /// Saves the current listing (minus threads cleared from it) for a later return.
    pub(super) fn remember_mailbox(&mut self) {
        if self.is_demo() || self.status != Status::Ready {
            return;
        }
        let kept = self.listed();
        if self.mailbox != View::Search {
            self.cache.insert(self.mailbox, kept);
        }
    }

    /// Switches the list to another mailbox.
    pub(super) fn select_mailbox(&mut self, view: View, cx: &mut Context<Self>) {
        self.switch_mailbox(view, false, cx);
    }

    /// The same, on screen before this returns (loading while its stored list is read), for
    /// what goes on to act in it.
    fn select_mailbox_now(&mut self, view: View, cx: &mut Context<Self>) {
        self.switch_mailbox(view, true, cx);
    }

    fn switch_mailbox(&mut self, view: View, now: bool, cx: &mut Context<Self>) {
        self.leave_sidebar(cx);
        // A mailbox clicked before, whose list is still being read, is not wanted now.
        self.opening = None;
        if self.mailbox == view || matches!(self.status, Status::SignedOut | Status::SigningIn) {
            return;
        }
        if self.is_demo() {
            self.enter_mailbox(view);
            // The sample data only has an inbox; Done shows what was cleared from it.
            self.selected = self.visible().first().copied().unwrap_or(0);
            cx.notify();
            return;
        }
        // Show what we already have for this mailbox, then refresh it quietly.
        // A mailbox seen before (this session or on disk) shows at once, even when it
        // was empty: "Nothing in …" is known, so there is no loading screen for it.
        // An empty All inboxes is out of date once mail has been seen in an Inbox.
        if view.is_every() {
            self.refresh_every_labels();
        }
        if view == View::ALL_INBOXES
            && self.cache.get(&view).is_some_and(Vec::is_empty)
            && self.inbox_has_mail()
        {
            self.cache.remove(&view);
        }
        match (self.cache.get(&view).cloned(), self.store.clone()) {
            (Some(mails), _) => {
                self.enter_mailbox(view);
                self.show_listed(view, Some(mails), cx);
            }
            // Not visited this session: start from what is stored on disk.
            (None, Some(store)) => {
                let serial = self.open_stored(view, store, cx);
                if now {
                    self.stored_slow(serial, cx);
                }
            }
            (None, None) => {
                self.enter_mailbox(view);
                self.show_listed(view, None, cx);
            }
        }
    }

    /// Goes to a mailbox, leaving the list of the one before for a later return.
    fn enter_mailbox(&mut self, view: View) {
        self.remember_mailbox();
        self.picked.clear();
        self.mailbox = view;
        self.back.clear();
        self.forward.clear();
        self.palette = false;
    }

    /// Puts what is known of the mailbox entered on screen (`None`: nothing, it is loaded)
    /// and brings it up to date.
    fn show_listed(&mut self, view: View, known: Option<Vec<Mail>>, cx: &mut Context<Self>) {
        if let Some(mails) = &known {
            self.cache.insert(view, mails.clone());
        }
        let found = known.is_some();
        self.mails = known.unwrap_or_default();
        self.done = vec![false; self.mails.len()];
        self.history.clear();
        self.history_groups.clear();
        self.selected = 0;
        if found {
            self.status = Status::Ready;
            self.load_selected(cx);
            self.look_again(cx);
        } else {
            self.refresh(cx);
        }
        cx.notify();
    }

    /// Reads a mailbox's stored list off the main thread. The list on screen stays until it
    /// is read, so a quick read shows no loading screen; a slow one shows it after a frame
    /// or two.
    fn open_stored(
        &mut self,
        view: View,
        store: Arc<crate::store::Store>,
        cx: &mut Context<Self>,
    ) -> u64 {
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        self.opening = Some(Opening {
            view,
            from: self.mailbox,
            account_gen: self.account_gen,
            serial,
            shown: false,
        });
        let record = self.record_for(view);
        cx.spawn(async move |this, cx| {
            let mails = cx
                .background_executor()
                .spawn(async move {
                    let mut mails: Vec<Mail> = loaded(store.load(&record)?);
                    for mail in &mut mails {
                        mail.refresh_time();
                    }
                    Some(mails)
                })
                .await;
            this.update(cx, |this, cx| this.stored_read(serial, mails, cx))
                .ok();
        })
        .detach();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Opening::SKELETON_AFTER)
                .await;
            this.update(cx, |this, cx| this.stored_slow(serial, cx))
                .ok();
        })
        .detach();
        serial
    }

    fn opening_now(&self, serial: u64) -> Option<Landing> {
        let opening = self.opening.as_ref().filter(|o| o.serial == serial)?;
        Some(opening.landing(
            self.account_gen,
            self.mailbox,
            self.status == Status::Loading && self.mails.is_empty(),
        ))
    }

    /// The stored list is not read yet: the mailbox goes on screen, loading.
    fn stored_slow(&mut self, serial: u64, cx: &mut Context<Self>) {
        let Some(Landing::Enter) = self.opening_now(serial) else {
            return;
        };
        let Some(opening) = &mut self.opening else {
            return;
        };
        opening.shown = true;
        let view = opening.view;
        self.enter_mailbox(view);
        self.mails.clear();
        self.done.clear();
        self.history.clear();
        self.history_groups.clear();
        self.selected = 0;
        self.status = Status::Loading;
        cx.notify();
    }

    /// The stored list is read: shown, if that mailbox and account are still the ones asked
    /// for and nothing newer has arrived for it meanwhile.
    fn stored_read(&mut self, serial: u64, mails: Option<Vec<Mail>>, cx: &mut Context<Self>) {
        let Some(landing) = self.opening_now(serial) else {
            return;
        };
        let Some(view) = self.opening.take().map(|o| o.view) else {
            return;
        };
        match landing {
            Landing::Enter => {
                self.enter_mailbox(view);
                self.show_listed(view, mails, cx);
            }
            Landing::Fill => self.show_listed(view, mails, cx),
            Landing::Stale => {}
        }
    }

    /// The mailbox on screen is being checked for mail.
    pub(super) fn fetching(&self) -> bool {
        self.checks.running_for(self.mailbox, &self.query)
    }

    /// A check has its answer, or ran out of time. False when that is no longer wanted.
    fn finish_check(&mut self, check: u64, cx: &mut Context<Self>) -> bool {
        if !self.checks.finish(check, self.mailbox, &self.query) {
            return false;
        }
        // Whatever the answer, rows that are already here are not hidden behind the skeleton.
        if self.status == Status::Loading && !self.mails.is_empty() {
            self.status = Status::Ready;
        }
        cx.notify();
        true
    }

    /// Where the open mailbox's next page starts, if it is to be loaded.
    pub(super) fn more(&self) -> Option<&String> {
        self.next_pages
            .get(&self.mailbox)
            .filter(|_| !self.more_failed.contains(&self.mailbox))
    }

    /// How much has been seen to change. It only grows while an account is shown.
    fn changes_seen(&self) -> u64 {
        let connected = self.connection.iter().chain(self.others.values());
        self.feed_changes + connected.map(|client| client.writes()).sum::<u64>()
    }

    /// A mailbox that is already on screen from what is kept. Gmail is asked only if
    /// something changed since it was last checked, or that was a while ago, and the screen
    /// does not say so.
    fn look_again(&mut self, cx: &mut Context<Self>) {
        let changes = self.changes_seen();
        let checked = self.checked.get(&self.record_for(self.mailbox));
        if self.fetching() || checked.is_some_and(|asked| asked.stands(changes)) {
            return;
        }
        self.refresh(cx);
        if self.fetching() {
            self.checks.quiet();
        }
    }

    /// Forgets what was checked and when: the lists it was true of are gone.
    pub(super) fn forget_checked(&mut self) {
        self.checked.clear();
        self.counted = None;
        self.paged.clear();
        self.all_inboxes_whole = false;
        self.all_inboxes_reached = false;
        self.all_next.clear();
        self.shifted.clear();
    }

    /// The open mailbox is being checked, and says so.
    pub(super) fn shows_checking(&self) -> bool {
        self.checks.shows_for(self.mailbox, &self.query)
    }

    /// Brings the open mailbox up to date: only what changed since it was last synced when
    /// that is known, otherwise a full fetch of its newest threads.
    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.more_failed.remove(&self.mailbox);
        if !self.sync_view(cx) {
            self.fetch_inbox(cx);
        }
    }

    /// Applies Gmail's change feed to the open mailbox. Returns false when it cannot (no
    /// starting point, nothing on screen, or a search-based mailbox) so the caller fetches.
    fn sync_view(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(client) = self.connection.clone() else {
            return false;
        };
        let view = self.mailbox;
        let label_id = match view {
            View::Label(index) => self.labels.get(index).map(|label| label.id.clone()),
            _ => None,
        };
        if !view.syncs(label_id.as_deref()) || self.mails.is_empty() {
            return false;
        }
        let Some(start) = self.synced_to.get(&self.record_for(view)).cloned() else {
            return false;
        };
        if self.fetching() {
            return true;
        }
        let check = self.checks.start(view, &self.query);
        let asked = Asked::now(self.changes_seen());
        let count = self
            .counted
            .is_none_or(|at| !at.stands(self.changes_seen()));
        // From this Mac, a list has no token for its next page.
        let (find_page, listed) = (!self.paged.contains(&view), self.mails.len());
        cx.notify();
        let count_ids = listing::count_ids(self.labels.iter().map(|l| l.id.as_str()));
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let result = within(cx.background_executor(), async move {
                let synced = listing::sync(
                    &*client,
                    &start,
                    view,
                    label_id.as_deref(),
                    find_page,
                    listed,
                    count,
                    &count_ids,
                )?;
                let listing::Synced {
                    latest,
                    updates,
                    counts,
                    page,
                } = synced;
                anyhow::Ok(((latest, updates, counts, page), label_id))
            })
            .await;
            this.update(cx, |this, cx| {
                // A different account was opened meanwhile: this answer is not for it.
                if this.account_gen != generation {
                    return;
                }
                if !this.finish_check(check, cx) {
                    return;
                }
                match result {
                    Ok(((latest, updates, counts, page), label_id)) => {
                        this.synced_to.insert(this.record_for(view), latest);
                        this.checked.insert(this.record_for(view), asked);
                        if let Some(next) = page {
                            this.paged.insert(view);
                            if let Some(next) = next {
                                this.next_pages.insert(view, next);
                            }
                        }
                        if let Some(counts) = counts.filter(|c| !c.is_empty()) {
                            this.counts = counts;
                            this.counted = Some(asked);
                        }
                        // Count only; never log subjects or addresses.
                        eprintln!(
                            "nimbra: {view:?} synced ({} changed threads)",
                            updates.len()
                        );
                        if updates.is_empty() {
                            this.persist(cx);
                            this.prefetch(cx);
                            cx.notify();
                            return;
                        }
                        let mut list: Vec<Mail> = this.mails.clone();
                        listing::apply_updates(&mut list, updates, view, label_id.as_deref());
                        this.install(list, cx);
                    }
                    Err(error) if error.is::<mail::HistoryExpired>() => this.fetch_inbox(cx),
                    Err(error) => this.fail(error, cx),
                }
            })
            .ok();
        })
        .detach();
        true
    }

    /// Brings the open mailbox up to date from the shown account's change feed, if that is
    /// owed and can be done now. Not while a list is loading (its answer may be from before
    /// the change) or a message is being written; then it stays owed, and the next look
    /// does it.
    fn keep_up(&mut self, cx: &mut Context<Self>) {
        if !self.sync_owed || self.compose.is_some() || self.fetching() {
            return;
        }
        self.sync_owed = false;
        // All inboxes cannot be patched from one feed: list it again.
        if self.mailbox.is_every() {
            self.fetch_every(cx);
            self.checks.quiet();
            return;
        }
        if self.sync_view(cx) {
            self.checks.quiet();
            return;
        }
        // An empty list (Inbox Zero) or one with no starting point is listed again, as long
        // as the feed can tell what is in it: a search is not run again by itself.
        let label_id = match self.mailbox {
            View::Label(index) => self.labels.get(index).map(|label| label.id.clone()),
            _ => None,
        };
        if self.connection.is_some() && self.mailbox.syncs(label_id.as_deref()) {
            self.fetch_inbox(cx);
            self.checks.quiet();
        }
    }

    /// Fetches, in the background, the content of the newest threads on screen that have not
    /// been downloaded yet, and saves it, so opening them is instant now and after a restart.
    pub(super) fn prefetch(&mut self, cx: &mut Context<Self>) {
        // A thread in full is 120 units of the 6,000 a minute: not the whole screen.
        const AHEAD: usize = 3;
        const WORKERS: usize = 4;
        if self.connection.is_none() {
            return;
        }
        if self.prefetching || self.mailbox == View::Search || self.status != Status::Ready {
            return;
        }
        // Each thread with its own account's connection (All inboxes mixes accounts).
        let mut ids = Vec::new();
        let mut accounts = Vec::new();
        let mut clients = Vec::new();
        // The top few only, not the next few that are missing: each check would fetch more.
        for i in self.visible().into_iter().take(AHEAD) {
            if i == self.selected {
                continue; // the one on show is loaded by itself
            }
            if let Some(mail) = self.mails.get(i).filter(|mail| !mail.loaded)
                && let (Some(id), Some(client)) = (mail.id.clone(), self.client_at(i))
            {
                ids.push(id);
                accounts.push(self.account_of(mail).to_string());
                clients.push(client);
            }
        }
        if ids.is_empty() {
            return;
        }
        self.prefetching = true;
        let view = self.mailbox;
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let fetched = within(cx.background_executor(), async move {
                let mut results: Vec<Option<anyhow::Result<Vec<mail::Message>>>> =
                    (0..ids.len()).map(|_| None).collect();
                let chunk = ids.len().div_ceil(WORKERS).max(1);
                std::thread::scope(|scope| {
                    for ((ids, clients), out) in ids
                        .chunks(chunk)
                        .zip(clients.chunks(chunk))
                        .zip(results.chunks_mut(chunk))
                    {
                        scope.spawn(move || {
                            for ((id, client), slot) in ids.iter().zip(clients).zip(out) {
                                *slot = Some(client.thread(id));
                            }
                        });
                    }
                });
                Ok(ids
                    .into_iter()
                    .zip(accounts)
                    .zip(results.into_iter().flatten())
                    .collect::<Vec<_>>())
            })
            .await
            .unwrap_or_default();
            this.update(cx, |this, cx| {
                // A different account was opened meanwhile: this answer is not for it.
                if this.account_gen != generation {
                    return;
                }
                this.prefetching = false;
                let mut filled = 0;
                for ((id, account), result) in fetched {
                    match result {
                        Ok(messages) => {
                            if let Some(row) = this.row_of(&id, &account)
                                && !this.mails[row].loaded
                            {
                                this.mails[row].fill(messages);
                                filled += 1;
                            }
                        }
                        Err(error) if error.is::<mail::SignedOut>() => {
                            this.fail(error, cx);
                            return;
                        }
                        // One thread failing (offline, deleted) does not stop the rest.
                        Err(_) => {}
                    }
                }
                if filled > 0 && this.mailbox == view {
                    // Count only; never log subjects or addresses.
                    eprintln!("nimbra: prefetched {filled} threads");
                    this.cache.insert(view, this.listed());
                    this.persist(cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Opens the encrypted mail store in the background and, if nothing has arrived from
    /// Gmail yet, shows what was saved last time straight away.
    pub(super) fn open_store(&mut self, cx: &mut Context<Self>) {
        // Ask once for permission to notify about new mail (macOS remembers the answer).
        static ASKED: std::sync::Once = std::sync::Once::new();
        ASKED.call_once(|| {
            crate::notify::show_while_active();
            crate::notify::request_permission();
            crate::notify::log_permission();
        });
        // NIMBRA_NOTIFY_TEST=1 posts one sample notification, to check the setup.
        if std::env::var_os("NIMBRA_NOTIFY_TEST").is_some() {
            crate::notify::show("test", "Nimbra", "Notifications are working.", true);
        }
        self.connect_others(cx);
        if self.store.is_some() {
            self.refresh(cx);
            return;
        }
        let view = self.mailbox;
        let view_record = self.record_for(view);
        let account = self.account_id.clone();
        let generation = self.account_gen;
        let client = self.connection.clone();
        let reading = self.read_keychain(cx);
        cx.spawn(async move |this, cx| {
            let opened = cx
                .background_executor()
                .spawn(async move {
                    let opened = crate::store::Store::open(&account);
                    drop(reading);
                    let store = Arc::new(opened?);
                    let saved: Option<Saved> = store.load("meta");
                    let mut mails: Option<Vec<Mail>> = store.load(&view_record).map(loaded);
                    for mail in mails.iter_mut().flatten() {
                        mail.refresh_time();
                    }
                    let later = scheduled::list(&store);
                    let snoozed: Vec<Snooze> =
                        store.load(nimbra_core::snooze::LIST).unwrap_or_default();
                    if let Some(client) = &client {
                        client.keep_rows_in(&store);
                    }
                    let unsent = client.map_or_else(Vec::new, |client| {
                        if let Some(mut older) = store.load::<Outbox>("outbox")
                            && !older.mail.is_empty()
                        {
                            older.mail.retain(|mail| {
                                !Job::queued(&account, &client, &store, mail.clone())
                                    .is_ok_and(|job| job.keep())
                            });
                            // Only the mail: the changes may have moved on meanwhile.
                            let left = older.mail;
                            let moved =
                                store.update("outbox", |outbox: &mut Outbox| outbox.mail = left);
                            if moved.is_err() {
                                eprintln!("nimbra: could not move the offline queue over");
                            }
                        }
                        Job::left_behind(&account, &client, &store)
                    });
                    anyhow::Ok((store, saved, mails, unsent, later, snoozed))
                })
                .await;
            let Ok((store, saved, mails, unsent, later, snoozed)) = opened else {
                eprintln!("nimbra: mail store unavailable; continuing without it");
                this.update(cx, |this, cx| {
                    if this.account_gen != generation {
                        return;
                    }
                    this.inbox_saved_empty = false;
                    this.tell(STORE_UNAVAILABLE);
                    if !this.outbox.changes.is_empty() {
                        this.went_offline(cx);
                    }
                    this.fetch_inbox(cx)
                })
                .ok();
                return;
            };
            this.update(cx, |this, cx| {
                if this.account_gen != generation {
                    return;
                }
                // Read after what was handed to the writer until now, not above: a
                // catch-up from before a switch may just have taken what it sent out of it.
                // One that does so from here on is counted (`drop_caught_up`).
                let reading = store.read_soon::<Outbox>("outbox");
                this.outbox_pending = Some(0);
                cx.spawn(async move |this, cx| {
                    let read = reading.await.ok().flatten();
                    this.update(cx, |this, cx| this.outbox_read(generation, read, cx))
                        .ok();
                })
                .detach();
                this.scheduled = later;
                this.snoozed = snoozed;
                this.store = Some(store);
                this.scan_contacts(this.account_id.clone(), cx);
                // Not only after a full listing: opening into what is saved often has none.
                this.load_photo(this.account_id.clone(), cx);
                if view == View::Inbox {
                    this.inbox_saved_empty = mails.as_ref().is_some_and(Vec::is_empty);
                }
                for job in unsent {
                    this.pick_up_send(job, cx);
                }
                if !this.outbox.changes.is_empty() || this.waiting_mail().next().is_some() {
                    this.went_offline(cx);
                }
                if !this.scheduled.is_empty() || !this.snoozed.is_empty() {
                    this.watch_schedule(cx);
                }
                // Something is already on screen (a sign-in in this session): just refresh.
                if !this.mails.is_empty() || this.mailbox != view {
                    this.refresh(cx);
                    return;
                }
                if let Some(saved) = saved {
                    if saved.account.contains('@') {
                        let generation = this.account_gen;
                        this.remember_account(&saved.account, cx);
                        if this.account_gen != generation {
                            return;
                        }
                    }
                    this.account = saved.account.into();
                    this.me_initials = saved.initials.into();
                    this.counts = saved.counts;
                    this.labels = saved.labels;
                    this.synced_to = saved.synced_to;
                }
                // The labels kept are as they were when last saved: ask Gmail how they
                // are now (one deleted or made since, here or elsewhere).
                this.refresh_labels(true, cx);
                if let Some(mails) = mails {
                    if mails.is_empty() {
                        // Known to be empty, so checking it again needs no loading screen.
                        this.cache.insert(view, Vec::new());
                    }
                    eprintln!("nimbra: opened {} stored threads", mails.len());
                    this.done = vec![false; mails.len()];
                    this.mails = mails;
                    this.selected = 0;
                    this.status = Status::Ready;
                    this.load_selected(cx);
                }
                // Bring it up to date: only what changed, when the store knows from where.
                this.refresh(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Saves the open mailbox and the sidebar state, in the background. Search results are
    /// never stored.
    pub(super) fn persist(&mut self, _cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        // All inboxes mixes accounts: it is never saved in any one account's store.
        if matches!(self.mailbox, View::Search | View::Every(_)) || self.status != Status::Ready {
            return;
        }
        // Keep the store small: the newest threads of a mailbox, and the full text of the
        // newest of those (`listing::kept`).
        let rows = listing::kept(
            self.mails
                .iter()
                .zip(&self.done)
                .filter(|(_, done)| !**done)
                .map(|(mail, _)| &mail.row),
        );
        if self.mailbox == View::Inbox {
            self.inbox_saved_empty = rows.is_empty();
            self.inboxes_saved_empty
                .insert(self.account_id.clone(), rows.is_empty());
        }
        let saved = Saved {
            account: self.account.to_string(),
            initials: self.me_initials.to_string(),
            counts: self.counts.clone(),
            labels: self.labels.clone(),
            synced_to: self.synced_to.clone(),
        };
        // Written in turn on the store's thread, which also turns them into JSON. The list
        // first: a sync point ahead of its list would miss what changed.
        store.save_soon(&self.record_for(self.mailbox), rows);
        store.save_soon("meta", saved);
    }

    /// Shows a thread by its Gmail id: switches to the Inbox, and opens the thread now if
    /// it is listed, or as soon as the list has caught up.
    pub(super) fn open_thread(&mut self, id: String, cx: &mut Context<Self>) {
        if self.compose.as_ref().is_some_and(|c| !c.full) {
            // Leave a message being written alone; just bring the app forward.
            return;
        }
        // Another account's notification ("acct:<account>:<thread>"): shown in All inboxes.
        let mut id = id;
        if let Some((account, thread)) = id
            .strip_prefix("acct:")
            .and_then(|rest| rest.split_once(':'))
            .map(|(a, t)| (a.to_string(), t.to_string()))
        {
            if account == self.account_id {
                id = thread;
            } else {
                if self.mailbox == View::ALL_INBOXES {
                    self.refresh(cx);
                } else {
                    self.select_mailbox_now(View::ALL_INBOXES, cx);
                }
                self.unread_only = false;
                if thread != "more" && !self.open_listed(&thread, cx) {
                    self.open_when_listed = Some(thread);
                    // Opened when it is listed, so it has to be listed again.
                    if !self.fetching() {
                        self.refresh(cx);
                    }
                }
                return;
            }
        }
        if self.mailbox != View::Inbox {
            self.select_mailbox_now(View::Inbox, cx);
        }
        self.unread_only = false;
        // "more" stands for several new threads: showing the Inbox is the answer.
        if id == "more" {
            return;
        }
        if !self.open_listed(&id, cx) {
            self.open_when_listed = Some(id);
            self.refresh(cx);
        }
    }

    /// Opens the thread with this id if it is in the list. Returns whether it was.
    pub(super) fn open_listed(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        let Some(index) = self.mails.iter().position(|m| m.id.as_deref() == Some(id)) else {
            return false;
        };
        if self.done.get(index) == Some(&true) {
            return false;
        }
        self.open(index);
        self.open_selected(cx);
        cx.notify();
        true
    }

    /// Reads Gmail's change feed every 30 seconds while the app is open: shows a
    /// notification for each thread with a new unread message in the inbox, and applies
    /// any change to the open mailbox.
    pub(super) fn watch_for_mail(&mut self, cx: &mut Context<Self>) {
        const EVERY: Duration = Duration::from_secs(30);
        cx.spawn(async move |this, cx| {
            // The first look happens as soon as the account is ready, so it only takes stock
            // of what is already unread; anything arriving after that is announced.
            let mut first = true;
            let mut failed = false;
            loop {
                let ready = this
                    .update(cx, |this, _| this.connection.is_some())
                    .unwrap_or(false);
                let wait = if first && !ready {
                    Duration::from_secs(3)
                } else if first && failed {
                    // Without a pause a first look that fails is made again and again.
                    Duration::from_secs(15)
                } else if first {
                    Duration::ZERO
                } else {
                    this.update(cx, |this, _| {
                        nimbra_core::watching::check_interval(this.settings.check_every)
                    })
                    .unwrap_or(EVERY)
                };
                // Until the next look, unless the server says sooner that something changed
                // (IMAP IDLE). Providers that cannot say return at once: the rest is waited.
                // In slices of at most half a minute, so that after a switch of account
                // it is the new account's server that is listened to, soon.
                let began = std::time::Instant::now();
                let start_gen = this.update(cx, |this, _| this.account_gen).ok();
                loop {
                    let left = wait.saturating_sub(began.elapsed());
                    if left.is_zero() {
                        break;
                    }
                    let slice = left.min(nimbra_core::watching::SLICE);
                    let current = this
                        .update(cx, |this, _| (this.account_gen, this.connection.clone()))
                        .ok();
                    let Some((generation, client)) = current else {
                        break;
                    };
                    if Some(generation) != start_gen {
                        break;
                    }
                    let sliced = std::time::Instant::now();
                    let woke = match client {
                        Some(client) => {
                            cx.background_executor()
                                .spawn(async move { client.wait_for_change(slice) })
                                .await
                        }
                        None => false,
                    };
                    if woke {
                        break;
                    }
                    // Cut short when the Mac wakes or the network is back.
                    if crate::integration::pause(
                        cx.background_executor()
                            .timer(slice.saturating_sub(sliced.elapsed())),
                    )
                    .await
                    {
                        break;
                    }
                }
                let Ok(client) = this.update(cx, |this, _| {
                    this.connection
                        .clone()
                        .map(|c| (c, this.account_gen, this.account_id.clone()))
                }) else {
                    break; // the window is gone
                };
                // The account checked, kept for its notifications: another may be shown by then.
                let Some((client, generation, account)) = client else {
                    continue; // not signed in
                };
                // The label list too, now and then: Gmail's change feed does not carry it.
                this.update(cx, |this, cx| this.refresh_labels(false, cx))
                    .ok();
                let from = this
                    .update(cx, |this, _| this.notify_from.clone())
                    .ok()
                    .flatten();
                let check = {
                    let client = client.clone();
                    cx.background_executor()
                        .spawn(async move {
                            match from {
                                // First look: just remember where the feed is now.
                                None => client.profile().map(|(_, at)| (at, None)),
                                Some(from) => match client.history(&from) {
                                    Ok(changes) => Ok((changes.latest.clone(), Some(changes))),
                                    Err(error) if error.is::<mail::HistoryExpired>() => {
                                        client.profile().map(|(_, at)| (at, None))
                                    }
                                    Err(error) => Err(error),
                                },
                            }
                        })
                        .await
                };
                failed = check.is_err();
                let Ok((at, changes)) = check else {
                    continue; // offline or signed out: try again next time
                };
                first = false;
                let (fresh, store) = this
                    .update(cx, |this, cx| {
                        let store = this.store.clone();
                        // Switched accounts meanwhile: this feed belongs to the other one.
                        if this.account_gen != generation {
                            return (Vec::new(), store);
                        }
                        this.notify_from = Some(at);
                        let changed = changes.as_ref().is_some_and(|c| !c.threads.is_empty());
                        if changed {
                            this.feed_changes += 1;
                            this.sync_owed = true;
                        }
                        // First look, just after the account was opened: mail that arrived
                        // while its list was loading is behind this point, and no later look
                        // would bring it. All inboxes is listed afresh anyway.
                        if changes.is_none() && !this.mailbox.is_every() {
                            this.sync_owed = true;
                        }
                        this.keep_up(cx);
                        let Some(changes) = changes else {
                            // First look: what blocked senders left in the inbox meanwhile.
                            this.sweep_blocked(this.account_id.clone(), cx);
                            return (Vec::new(), store);
                        };
                        (changes.new_unread_inbox, store)
                    })
                    .unwrap_or_default();
                if fresh.is_empty() {
                    continue;
                }
                // Blocked senders' mail is moved away first, and never notifies.
                let (fresh, summaries) = cx
                    .background_executor()
                    .spawn(async move {
                        // Sender and subject for the first few; the rest are summed up.
                        let news =
                            nimbra_core::notifying::new_mail(&*client, store.as_deref(), fresh);
                        (news.fresh, news.named)
                    })
                    .await;
                if fresh.is_empty() {
                    continue;
                }
                let extra = fresh.len().saturating_sub(summaries.len());
                this.update(cx, |this, _| {
                    if !this.settings.notifications {
                        return;
                    }
                    for mail in &summaries {
                        crate::notify::show(
                            &format!("acct:{account}:{}", mail.id),
                            &mail.sender,
                            &mail.subject,
                            this.settings.notify_sound,
                        );
                    }
                    if extra > 0 {
                        crate::notify::show(
                            &format!("acct:{account}:more"),
                            "Nimbra",
                            &format!("{extra} more new messages"),
                            this.settings.notify_sound,
                        );
                    }
                    // Count only; never log senders or subjects.
                    eprintln!("nimbra: {} new unread threads", fresh.len());
                })
                .ok();
            }
        })
        .detach();
    }

    /// The accounts that are not shown: their change feeds are read on the same schedule
    /// as the shown one's, for notifications, unread counts and All inboxes; and the
    /// snoozes and scheduled sends kept in their stores are carried out when due.
    pub(super) fn watch_other_accounts(&mut self, cx: &mut Context<Self>) {
        const EVERY: Duration = Duration::from_secs(30);
        cx.spawn(async move |this, cx| {
            // Where each account's feed was last read, by account id.
            let mut feeds: HashMap<String, String> = HashMap::new();
            loop {
                let wait = this
                    .update(cx, |this, _| {
                        let every =
                            nimbra_core::watching::check_interval(this.settings.check_every);
                        // Sooner while changes wait there for a connection.
                        let waiting = !this.away_waiting.is_empty();
                        if waiting {
                            crate::outbox::RETRY_EVERY
                        } else {
                            every
                        }
                    })
                    .unwrap_or(EVERY);
                crate::integration::pause(cx.background_executor().timer(wait)).await;
                let Ok((others, connected, writes)) = this.update(cx, |this, _| {
                    let others = this
                        .others
                        .iter()
                        .map(|(id, client)| (id.clone(), client.clone()))
                        .collect::<Vec<_>>();
                    // With the shown one's: a store can hold another account's threads.
                    let mut connected = this.others.clone();
                    if let Some(client) = &this.connection {
                        connected.insert(this.account_id.clone(), client.clone());
                    }
                    (others, connected, this.writes.clone())
                }) else {
                    break; // the window is gone
                };
                feeds.retain(|id, _| others.iter().any(|(other, _)| other == id));
                for (id, client) in others {
                    // First, so that reading the account does not bring back what was cleared.
                    Self::catch_up_away(&this, cx, &id, &client).await;
                    let from = feeds.get(&id).cloned();
                    let account = id.clone();
                    let connected = connected.clone();
                    let writes = writes.clone();
                    // Opened when it was connected. Opening it here could wait on a question
                    // from the keychain, and hold up every account's checks with it.
                    let store = this
                        .update(cx, |this, _| this.other_stores.get(&account).cloned())
                        .ok()
                        .flatten();
                    let checked = cx
                        .background_executor()
                        .spawn(async move {
                            let swept = from.is_none();
                            let (at, fresh, changed) = match from {
                                None => (client.profile()?.1, Vec::new(), false),
                                Some(from) => match client.history(&from) {
                                    Ok(changes) => (
                                        changes.latest.clone(),
                                        changes.new_unread_inbox.clone(),
                                        !changes.threads.is_empty(),
                                    ),
                                    Err(error) if error.is::<mail::HistoryExpired>() => {
                                        (client.profile()?.1, Vec::new(), true)
                                    }
                                    Err(error) => return Err(error),
                                },
                            };
                            // Blocked senders: moved away, and no notification.
                            if swept && let Some(store) = &store {
                                crate::blocking::sweep(&*client, store);
                            }
                            let news =
                                nimbra_core::notifying::new_mail(&*client, store.as_deref(), fresh);
                            let unread = listing::inbox_unread(&*client);
                            let (summaries, extra) = (news.named, news.more);
                            // Snoozes and scheduled sends kept with this account.
                            let woken = store.as_ref().map_or_else(Vec::new, |store| {
                                nimbra_core::snooze::carry_out_due(
                                    &account,
                                    store,
                                    &connected,
                                    || writes.begin(),
                                )
                            });
                            anyhow::Ok((at, summaries, extra, changed, unread, woken))
                        })
                        .await;
                    let Ok((at, summaries, extra, changed, unread, woken)) = checked else {
                        continue; // offline or signed out: next time
                    };
                    feeds.insert(id.clone(), at);
                    this.update(cx, |this, cx| {
                        if !this.others.contains_key(&id) {
                            return;
                        }
                        this.account_unread.insert(id.clone(), unread);
                        if !woken.is_empty() {
                            // Its store's list changed: the chips and Snoozed follow it.
                            this.reread_snoozed_of(id.clone(), cx);
                        }
                        if changed {
                            this.feed_changes += 1;
                        }
                        if this.settings.notifications {
                            for mail in &summaries {
                                crate::notify::show(
                                    &format!("acct:{id}:{}", mail.id),
                                    &mail.sender,
                                    &mail.subject,
                                    this.settings.notify_sound,
                                );
                            }
                            // Back from a snooze, as when the account is the one shown.
                            for item in &woken {
                                let whose = item.mail.account.as_deref().unwrap_or(&id);
                                crate::notify::show(
                                    &format!("acct:{whose}:{}", item.thread_id),
                                    if item.no_reply {
                                        "No reply yet"
                                    } else {
                                        "Back from snooze"
                                    },
                                    &item.mail.subject,
                                    this.settings.notify_sound,
                                );
                            }
                            if extra > 0 {
                                crate::notify::show(
                                    &format!("acct:{id}:more"),
                                    "Nimbra",
                                    &format!("{extra} more new messages"),
                                    this.settings.notify_sound,
                                );
                            }
                        }
                        if changed
                            && this.mailbox.is_every()
                            && this.compose.is_none()
                            && !this.fetching()
                        {
                            this.fetch_every(cx);
                            this.checks.quiet();
                        }
                        cx.notify();
                    })
                    .ok();
                }
            }
        })
        .detach();
    }

    pub(super) fn fetch_inbox(&mut self, cx: &mut Context<Self>) {
        if self.mailbox.is_every() {
            self.fetch_every(cx);
            return;
        }
        let Some(client) = self.connection.clone() else {
            return;
        };
        // The loading screen is only for a mailbox never listed before; one known to be
        // empty keeps showing as empty while it is checked again.
        if self.mails.is_empty() && !self.cache.contains_key(&self.mailbox) {
            self.status = Status::Loading;
        }
        let view = self.mailbox;
        let check = self.checks.start(view, &self.query);
        let asked = Asked::now(self.changes_seen());
        cx.notify();
        let label_id = match view {
            View::Label(index) => self.labels.get(index).map(|label| label.id.clone()),
            View::Search => Some(self.query.clone()),
            _ => None,
        };
        let need_labels = self.labels.is_empty();
        let known_labels: Vec<String> = self.labels.iter().map(|l| l.id.clone()).collect();
        let count = need_labels
            || self
                .counted
                .is_none_or(|at| !at.stands(self.changes_seen()));
        // A label is only known to hold by its id.
        let syncs = view.syncs(label_id.as_deref());
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let result = within(cx.background_executor(), async move {
                let page = listing::first_page(
                    &*client,
                    view,
                    label_id.as_deref(),
                    listing::PAGE,
                    need_labels,
                    &known_labels,
                    count,
                )?;
                // Rows are made here: each takes the local time and its formatting.
                let threads: Vec<Mail> = page.threads.into_iter().map(Mail::from_summary).collect();
                anyhow::Ok((
                    page.email,
                    page.history_id,
                    threads,
                    page.labels,
                    page.counts,
                    page.next_page,
                ))
            })
            .await;
            this.update(cx, |this, cx| {
                // A different account was opened meanwhile: this answer is not for it.
                if this.account_gen != generation {
                    return;
                }
                // Ignore the answer if the user has moved to another mailbox or search
                // meanwhile, or a newer check is out.
                if !this.finish_check(check, cx) {
                    return;
                }
                match result {
                    Ok((email, history_id, threads, labels, counts, next_page)) => {
                        this.checked.insert(this.record_for(view), asked);
                        this.paged.insert(view);
                        // Unless something was cleared while it was on its way.
                        if !this.writes.any() {
                            this.shifted.remove(&view);
                        }
                        match next_page {
                            Some(page) => this.next_pages.insert(view, page),
                            None => this.next_pages.remove(&view),
                        };
                        if syncs {
                            this.synced_to.insert(this.record_for(view), history_id);
                        }
                        if !counts.is_empty() {
                            this.counts = counts;
                            this.counted = Some(asked);
                        }
                        if let Some(labels) = labels {
                            this.labels = labels;
                        }
                        this.show_inbox(email, threads, cx);
                        // A change seen while this was loading may have come after it.
                        this.keep_up(cx);
                    }
                    Err(error) => this.fail(error, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// Every account's labels by name (`every_labels`): the one shown's, and the others' as
    /// their stores last kept them. An open label of every account's stays open by name
    /// when the list moves under it. Disk, small.
    pub(super) fn refresh_every_labels(&mut self) {
        let open = match self.mailbox.base() {
            View::Label(index) if self.mailbox.is_every() => {
                self.every_labels.get(index).map(|l| l.name.to_lowercase())
            }
            _ => None,
        };
        let labels: Vec<(String, Vec<mail::Label>)> = self
            .settings
            .accounts
            .iter()
            .map(|account| {
                let labels = if account.id == self.account_id {
                    self.labels.clone()
                } else {
                    self.other_stores
                        .get(&account.id)
                        .and_then(|store| store.load::<Saved>("meta"))
                        .map(|saved| saved.labels)
                        .unwrap_or_default()
                };
                (account.id.clone(), labels)
            })
            .collect();
        self.every_labels = listing::labels_by_name(
            labels
                .iter()
                .map(|(account, labels)| (account.as_str(), labels.as_slice())),
        );
        if let Some(name) = open {
            match self
                .every_labels
                .iter()
                .position(|l| l.name.to_lowercase() == name)
            {
                Some(index) => self.mailbox = View::every(View::Label(index)),
                None => self.mailbox = View::ALL_INBOXES,
            }
        }
    }

    /// The accounts that take part in every account's `view`, with their connections and,
    /// for a label, their label of that name: Gmail's own mailboxes leave out the accounts
    /// with their own servers, and a label those without one by its name.
    pub(super) fn every_accounts(
        &self,
        view: View,
    ) -> Vec<(String, Arc<mail::Client>, Option<String>)> {
        self.settings
            .accounts
            .iter()
            .filter(|account| listing::takes_part(view, account.servers.is_some()))
            .filter_map(|account| {
                let label = match view.base() {
                    View::Label(index) => Some(
                        self.every_labels
                            .get(index)?
                            .ids
                            .iter()
                            .find(|(id, _)| *id == account.id)?
                            .1
                            .clone(),
                    ),
                    _ => None,
                };
                Some((
                    account.id.clone(),
                    self.client_for(Some(&account.id))?,
                    label,
                ))
            })
            .collect()
    }

    /// Every account's mailbox (All inboxes is every account's inbox): the first pages of
    /// each account's, fetched side by side, merged newest first, each row marked with its
    /// account. Also each account's count of it. An account that fails is left out (with a
    /// note); all failing is an error.
    fn fetch_every(&mut self, cx: &mut Context<Self>) {
        if self.connection.is_none() {
            return;
        }
        self.refresh_every_labels();
        let view = self.mailbox;
        // As far as it was scrolled: listing again must not drop the pages below the first.
        // Cleared rows do not count, or each listing after a Done would reach further.
        let accounts: Vec<_> = self
            .every_accounts(view)
            .into_iter()
            .map(|(id, client, label)| {
                let listed = self
                    .mails
                    .iter()
                    .zip(&self.done)
                    .filter(|(mail, done)| !**done && self.account_of(mail) == id)
                    .count();
                (id, client, label, listing::all_inboxes_count(listed))
            })
            .collect();
        let inbox = view == View::ALL_INBOXES;
        if self.mails.is_empty() && !self.cache.contains_key(&view) {
            self.status = Status::Loading;
            // As when Nimbra opens: what was saved picks what to draw until Gmail answers.
            self.all_inboxes_saved_empty = inbox
                && !self.inbox_has_mail()
                && self.settings.accounts.iter().all(|account| {
                    crate::store::Store::saved_empty(&account.id, &record(View::Inbox))
                });
        }
        let check = self.checks.start(view, "");
        let seen = Asked::now(self.changes_seen());
        cx.notify();
        let generation = self.account_gen;
        let counted = self.every_count_ids();
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor();
            // A deadline each, so that one account cannot hold up the others.
            let results = futures::future::join_all(accounts.into_iter().map(
                |(id, client, label, count)| {
                    let account = id.clone();
                    let ids = counted.clone();
                    let listing = within(executor, async move {
                        let part = listing::account_view::<Mail>(
                            &*client,
                            &account,
                            view,
                            label.as_deref(),
                            count,
                        )?;
                        // The sidebar's counts of every account's mailboxes, while here.
                        Ok((part, client.counts(&ids)))
                    });
                    async move { (id, listing.await) }
                },
            ))
            .await;
            this.update(cx, |this, cx| {
                if this.account_gen != generation || !this.finish_check(check, cx) {
                    return;
                }
                let asked = results.len();
                let mut parts = Vec::new();
                for (id, result) in results {
                    let result: anyhow::Result<_> = result;
                    parts.push((
                        id.clone(),
                        result.map(|(part, counts)| {
                            if id != this.account_id {
                                this.account_counts.insert(id.clone(), counts);
                            }
                            part
                        }),
                    ));
                }
                let listing::AllInboxes {
                    rows: fresh,
                    pages,
                    unread,
                    failed,
                    first_error,
                } = listing::all_inboxes(parts);
                if inbox {
                    this.account_unread.extend(unread);
                    this.all_inboxes_reached = failed == 0 && asked >= this.settings.accounts.len();
                }
                this.set_all_pages(view, pages);
                if !this.writes.any() {
                    this.shifted.remove(&view);
                }
                if this.mailbox != view {
                    return;
                }
                if failed == asked
                    && let Some(error) = first_error
                {
                    this.fail(error, cx);
                    return;
                }
                this.install(fresh, cx);
                if failed == 0 {
                    this.checked.insert(this.record_for(view), seen);
                }
                // After the listing, which clears the notice.
                if failed > 0 {
                    this.notice = Some(
                        format!("{failed} account(s) could not be checked; showing the rest.")
                            .into(),
                    );
                }
                // Count only; never addresses or subjects.
                eprintln!(
                    "nimbra: every account's {:?} listed ({} threads)",
                    view.base(),
                    this.mails.len()
                );
            })
            .ok();
        })
        .detach();
    }

    /// The labels every account's mailboxes are counted by in the sidebar: the system ones
    /// with a count (see `listing::count_of`).
    fn every_count_ids(&self) -> Vec<String> {
        listing::count_ids(std::iter::empty())
    }

    /// Shows a freshly fetched listing, reusing thread contents already in memory and
    /// keeping the open thread and any just-cleared threads as they were.
    fn show_inbox(&mut self, email: String, fresh: Vec<Mail>, cx: &mut Context<Self>) {
        // A different Google account than the one the store was saved for: start clean.
        if self.account.contains('@') && self.account.as_ref() != email.as_str() {
            if let Some(store) = &self.store {
                store.clear_soon();
            }
            self.inboxes_saved_empty.remove(&self.account_id);
            self.cache.clear();
            self.outbox = Outbox::default();
            self.outbox_pending = None;
            self.counts.clear();
            self.labels.clear();
            self.forget_checked();
            if let Some(client) = &self.connection {
                client.forget_rows();
            }
        }
        self.me_initials = email
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default()
            .into();
        let generation = self.account_gen;
        self.remember_account(&email, cx);
        if self.account_gen != generation {
            return; // it was a second sign-in to an account already here
        }
        self.load_photo(self.account_id.clone(), cx);
        self.account = email.into();

        self.install(fresh, cx);
    }

    /// Fetches the messages of the open thread the first time it is shown.
    /// The user opened this thread themselves (clicked it, moved to it with the keyboard,
    /// or clicked its text): mark it read, here and in Gmail, and fetch its content.
    ///
    /// A thread that is only shown because a mailbox was just opened stays unread until
    /// then; see `load_selected`.
    pub(super) fn open_selected(&mut self, cx: &mut Context<Self>) {
        self.kept_unread = None;
        match self.settings.mark_read {
            crate::settings::MarkRead::Open => self.set_unread(self.selected, false, cx),
            crate::settings::MarkRead::Never => {}
            // Read once it has stayed open for two seconds.
            crate::settings::MarkRead::Delay => {
                let id = self.mails.get(self.selected).and_then(|m| m.id.clone());
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(nimbra_core::notifying::MARK_READ_DELAY)
                        .await;
                    this.update(cx, |this, cx| {
                        let still = this.mails.get(this.selected).and_then(|m| m.id.clone());
                        if still.is_some() && still == id && this.kept_unread.is_none() {
                            this.set_unread(this.selected, false, cx);
                        }
                    })
                    .ok();
                })
                .detach();
            }
        }
        self.load_selected(cx);
    }

    /// Fetches the content of the thread on show, without marking it read.
    pub(super) fn load_selected(&mut self, cx: &mut Context<Self>) {
        let Some(mail) = self.mails.get(self.selected) else {
            return;
        };
        let (false, Some(id)) = (mail.loaded, mail.id.clone()) else {
            return;
        };
        let account = self.account_of(mail).to_string();
        self.load_body(id, account, cx);
    }

    /// Fetches a thread's content.
    pub(super) fn load_body(&mut self, id: String, account: String, cx: &mut Context<Self>) {
        let key: ThreadKey = (id.clone(), account.clone());
        let Some(client) = self.client_for(Some(&account)) else {
            // Another account's, which is not connected: nothing is on its way.
            if self.connection.is_some() {
                self.body_failed.insert(key);
                cx.notify();
            }
            return;
        };
        let generation = self.account_gen;
        self.body_failed.remove(&key);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let thread_id = id.clone();
            let result = within(cx.background_executor(), async move {
                client.thread(&thread_id)
            })
            .await;
            this.update(cx, |this, cx| {
                // A different account was opened meanwhile: this answer is not for it.
                if this.account_gen != generation {
                    return;
                }
                match result {
                    Ok(messages) => {
                        // The inbox may have been reloaded meanwhile, so look the thread up again.
                        if let Some(row) = this.row_of(&id, &account) {
                            this.mails[row].fill(messages);
                            this.persist(cx);
                        } else if let Some(mail) = this.cache.values_mut().flatten().find(|m| {
                            m.id.as_deref() == Some(id.as_str())
                                && m.account.as_deref().unwrap_or(&this.account_id) == account
                        }) {
                            // Its mailbox was left; its own window may still be waiting for it.
                            mail.fill(messages);
                        }
                        cx.notify();
                    }
                    Err(error) => {
                        this.body_failed.insert(key);
                        this.fail(error, cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Sends a change to Gmail. If it removed the thread from the list and Gmail refuses,
    /// the thread is put back.
    pub(super) fn sync(
        &mut self,
        index: usize,
        change: mail::Change,
        removed: bool,
        cx: &mut Context<Self>,
    ) {
        let (Some(client), Some(id)) = (
            self.client_at(index),
            self.mails.get(index).and_then(|m| m.id.clone()),
        ) else {
            return;
        };
        let account = self.account_of(&self.mails[index]).to_string();
        self.sync_thread(client, account, id, change, removed, cx);
    }

    /// `sync` for a thread by its id and account, listed or not.
    pub(super) fn sync_thread(
        &mut self,
        client: Arc<mail::Client>,
        account: String,
        id: String,
        change: mail::Change,
        removed: bool,
        cx: &mut Context<Self>,
    ) {
        // Read or gone from the inbox here: its notification goes too.
        if matches!(
            change,
            mail::Change::MarkRead
                | mail::Change::Archive
                | mail::Change::Trash
                | mail::Change::Spam
        ) {
            crate::notify::withdraw(&account, &id);
        }
        // It follows what waits for the connection, so it waits too.
        if account == self.account_id {
            if self.outbox.changes.iter().any(|(queued, _)| *queued == id) {
                self.outbox.changes.push((id, change));
                self.save_outbox();
                self.went_offline(cx);
                return;
            }
            // What waits in its store is not known until that is read (`outbox_read`).
            if self.outbox_pending.is_some()
                && let Some(store) = &self.store
            {
                let read = store.read_soon::<Outbox>("outbox");
                cx.spawn(async move |this, cx| {
                    read.await.ok();
                    this.update(cx, |this, cx| {
                        if this.outbox_pending.is_some() {
                            this.start_step(client, account, id, change, removed, cx);
                        } else {
                            this.sync_thread(client, account, id, change, removed, cx);
                        }
                    })
                    .ok();
                })
                .detach();
                return;
            }
        } else if self.away_waiting.contains(&account)
            && let Some(store) = self.other_stores.get(&account).cloned()
        {
            // Queued after what waits for this thread there, if anything does. Asked on the
            // store's writer, which adds it in the same step.
            let queued = {
                let id = id.clone();
                store.update_soon("outbox", move |outbox: &mut Outbox| {
                    let waiting = outbox.changes.iter().any(|(queued, _)| *queued == id);
                    if waiting {
                        outbox.changes.push((id, change));
                    }
                    waiting
                })
            };
            self.away_queued += 1;
            cx.spawn(async move |this, cx| {
                let queued = matches!(queued.await, Ok(Ok(true)));
                this.update(cx, |this, cx| {
                    if !queued {
                        this.start_step(client, account, id, change, removed, cx);
                    }
                })
                .ok();
            })
            .detach();
            return;
        }
        self.start_step(client, account, id, change, removed, cx);
    }

    /// Makes a change to a thread with its account's connection.
    fn start_step(
        &mut self,
        client: Arc<mail::Client>,
        account: String,
        id: String,
        change: mail::Change,
        removed: bool,
        cx: &mut Context<Self>,
    ) {
        if removed {
            self.shifted.insert(self.mailbox);
        }
        let step = self.steps.start(account.clone(), id.clone(), change);
        let working = self.writes.begin();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _working = working;
                    step.run(&*client)
                })
                .await;
            this.update(cx, |this, cx| {
                let settled = this.steps.settle(&account, &id);
                // Offline: keep the change in its account's queue and send it when the
                // connection is back. Asked now, not when it was made: the account shown may
                // have changed.
                let shown = account == this.account_id;
                let waiting: Vec<_> = settled
                    .into_iter()
                    .filter(|(_, end)| *end == End::Offline)
                    .map(|(step, _)| (step.id.clone(), step.change))
                    .collect();
                // An account with no store has nowhere to keep it: undone, and said.
                let mut kept = shown || this.other_stores.contains_key(&account);
                if !waiting.is_empty() {
                    if shown {
                        this.outbox.changes.extend(waiting);
                        this.save_outbox();
                        this.went_offline(cx);
                    } else if this.queue_away(&account, waiting) {
                        this.notice = Some(
                            "That account is offline. The change is made when it is back.".into(),
                        );
                        cx.notify();
                    } else {
                        kept = false;
                    }
                }
                let Err(error) = result else {
                    return;
                };
                if kept && error.is::<mail::Offline>() {
                    return;
                }
                // Put back wherever the thread is by now.
                if removed && let Some(row) = this.row_of(&id, &account) {
                    this.done[row] = false;
                    if let Some(position) = this.history.iter().rposition(|i| *i == row) {
                        this.forget_cleared(position, cx);
                    }
                }
                this.fail(error, cx);
            })
            .ok();
        })
        .detach();
    }

    /// "Load more" at the end of the list: the next page of the mailbox.
    /// Where each account's next page in every account's `view` starts.
    fn set_all_pages(&mut self, view: View, pages: HashMap<String, String>) {
        if view == View::ALL_INBOXES {
            self.all_inboxes_whole = self.all_inboxes_reached && pages.is_empty();
        }
        if pages.is_empty() {
            self.next_pages.remove(&view);
            self.all_next.remove(&view);
        } else {
            // That there is a next page. The accounts' tokens are in `all_next`.
            self.next_pages.insert(view, String::new());
            self.all_next.insert(view, pages);
        }
    }

    /// The next page of every account that has one, merged into every account's mailbox.
    fn load_more_of_all(&mut self, from_top: bool, cx: &mut Context<Self>) {
        if self.loading_more || self.fetching() {
            return;
        }
        let view = self.mailbox;
        let pages = self.all_next.get(&view).cloned().unwrap_or_default();
        let labels: HashMap<String, Option<String>> = self
            .every_accounts(view)
            .into_iter()
            .map(|(id, _, label)| (id, label))
            .collect();
        let asked: Vec<_> = pages
            .iter()
            .filter_map(|(id, page)| {
                Some((
                    id.clone(),
                    self.client_for(Some(id))?,
                    labels.get(id)?.clone(),
                    page.clone(),
                ))
            })
            .collect();
        if asked.is_empty() {
            // Theirs are accounts that are not connected.
            self.more_failed.insert(view);
            return;
        }
        self.loading_more = true;
        cx.notify();
        let generation = self.account_gen;
        let count = if from_top { self.mails.len() + 30 } else { 30 };
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor();
            let results =
                futures::future::join_all(asked.into_iter().map(|(id, client, label, page)| {
                    let from = (!from_top).then(|| page.clone());
                    let listing = within(executor, async move {
                        client.threads_page(view.base(), label.as_deref(), count, from.as_deref())
                    });
                    async move { (id, page, listing.await) }
                }))
                .await;
            this.update(cx, |this, cx| {
                if this.account_gen != generation {
                    return;
                }
                this.loading_more = false;
                if this.mailbox != view {
                    cx.notify();
                    return;
                }
                let mut pages = this.all_next.get(&view).cloned().unwrap_or_default();
                let mut list = this.listed();
                let before = list.len();
                // A listing made meanwhile has other pages: what is not from them is left.
                let known: HashSet<ThreadKey> = this
                    .mails
                    .iter()
                    .map(|mail| this.thread_key(mail))
                    .collect();
                let first_error =
                    listing::more_of_all_inboxes(&mut list, &known, &mut pages, results);
                if from_top && list.len() == before && !pages.is_empty() {
                    this.more_failed.insert(view); // or it would ask without end
                }
                this.set_all_pages(view, pages);
                this.install(list, cx);
                if let Some(error) = first_error {
                    this.more_failed.insert(view);
                    this.fail(error, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Waits a moment with the next page: changes are still on their way.
    fn ask_again_soon(&mut self, cx: &mut Context<Self>) {
        self.loading_more = true;
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            this.update(cx, |this, cx| {
                if this.account_gen == generation {
                    this.loading_more = false;
                    cx.notify(); // drawing it asks again
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn near_list_end(&self) -> bool {
        let left = from_screen(self.list_scroll.max_offset().height)
            + from_screen(self.list_scroll.offset().y);
        left < 700.
    }

    pub(super) fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.loading_more || self.more().is_none() {
            return;
        }
        // With threads cleared from the list, what follows moved up, and a page token from
        // before could start past it: ask from the top, once the clearing is through, for
        // as many as are listed and a page. Rows already read cost nothing.
        let from_top = self.shifted.contains(&self.mailbox) && self.mails.len() + 30 <= 500;
        if from_top && self.writes.any() {
            self.ask_again_soon(cx);
            return;
        }
        if self.mailbox.is_every() {
            self.load_more_of_all(from_top, cx);
            return;
        }
        let count = if from_top { self.mails.len() + 30 } else { 30 };
        let (Some(client), Some(page)) = (self.connection.clone(), self.more().cloned()) else {
            return;
        };
        if self.loading_more {
            return;
        }
        self.loading_more = true;
        cx.notify();
        let view = self.mailbox;
        let label_id = match view {
            View::Label(index) => self.labels.get(index).map(|label| label.id.clone()),
            View::Search => Some(self.query.clone()),
            _ => None,
        };
        let generation = self.account_gen;
        let query = self.query.clone();
        cx.spawn(async move |this, cx| {
            let result = within(cx.background_executor(), async move {
                let page = (!from_top).then_some(page.as_str());
                client.threads_page(view, label_id.as_deref(), count, page)
            })
            .await;
            this.update(cx, |this, cx| {
                // A different account was opened meanwhile: this answer is not for it.
                if this.account_gen != generation {
                    return;
                }
                this.loading_more = false;
                if this.mailbox != view || (view == View::Search && this.query != query) {
                    cx.notify();
                    return;
                }
                match result {
                    Ok((threads, next)) => {
                        match next {
                            Some(page) => this.next_pages.insert(view, page),
                            None => this.next_pages.remove(&view),
                        };
                        let known: HashSet<String> =
                            this.mails.iter().filter_map(|m| m.id.clone()).collect();
                        let fresh: Vec<Mail> = threads
                            .into_iter()
                            .filter(|s| !known.contains(&s.id))
                            .map(Mail::from_summary)
                            .collect();
                        if from_top && fresh.is_empty() && this.next_pages.contains_key(&view) {
                            this.more_failed.insert(view); // or it would ask without end
                        }
                        if from_top {
                            // Mail that came meanwhile is in it too, and belongs on top.
                            let mut list = this.listed();
                            list.extend(fresh);
                            list.sort_by_key(|mail| std::cmp::Reverse(mail.timestamp_ms));
                            this.install(list, cx);
                        } else {
                            this.done.extend(std::iter::repeat_n(false, fresh.len()));
                            this.mails.extend(fresh);
                            this.cache.insert(this.mailbox, this.listed());
                            this.persist(cx);
                        }
                    }
                    Err(error) => {
                        this.more_failed.insert(view);
                        this.fail(error, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Checks for mail. With the sample data, everything that was cleared arrives again.
    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        self.reload_gen += 1;
        if !self.is_demo() {
            self.counted = None;
            self.refresh_labels(true, cx);
            self.refresh(cx);
            // One that was running quietly is what was asked for now.
            self.checks.show();
            return;
        }
        self.done.fill(false);
        self.history.clear();
        self.picked.clear();
        self.history_groups.clear();
        self.back.clear();
        self.forward.clear();
        self.selected = 0;
        self.palette = false;
        cx.notify();
    }
}

/// The rows a list is kept as in the store (the engine's `listing::Row`).
pub(super) fn stored(mails: &[Mail]) -> Vec<crate::data::Row> {
    mails.iter().map(|mail| mail.row.clone()).collect()
}

/// A list as it was kept, as rows to draw.
pub(super) fn loaded(rows: Vec<crate::data::Row>) -> Vec<Mail> {
    rows.into_iter().map(Mail::from).collect()
}

#[cfg(test)]
mod opening_tests {
    use super::*;

    fn opening(shown: bool) -> Opening {
        Opening {
            view: View::Sent,
            from: View::Inbox,
            account_gen: 3,
            serial: 1,
            shown,
        }
    }

    #[test]
    fn a_quick_read_goes_to_the_mailbox_from_the_one_it_was_clicked_in() {
        assert_eq!(
            opening(false).landing(3, View::Inbox, false),
            Landing::Enter
        );
    }

    #[test]
    fn a_read_for_another_account_is_dropped() {
        assert_eq!(
            opening(false).landing(4, View::Inbox, false),
            Landing::Stale
        );
        assert_eq!(opening(true).landing(4, View::Sent, true), Landing::Stale);
    }

    #[test]
    fn a_read_is_dropped_once_another_mailbox_is_on_screen() {
        assert_eq!(
            opening(false).landing(3, View::Drafts, false),
            Landing::Stale
        );
        assert_eq!(opening(true).landing(3, View::Inbox, true), Landing::Stale);
    }

    #[test]
    fn a_slow_read_fills_the_loading_mailbox_unless_newer_mail_came_first() {
        assert_eq!(opening(true).landing(3, View::Sent, true), Landing::Fill);
        assert_eq!(opening(true).landing(3, View::Sent, false), Landing::Stale);
    }
}
