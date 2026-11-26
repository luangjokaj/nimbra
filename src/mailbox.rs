//! The open mailbox: listing it from Gmail and from the local store, checking for changes,
//! loading more, opening a thread and fetching its content, and watching for new mail.

use super::*;

use gpui::Task;

use crate::checking::{OUTSIDE, OnItsWay, Pace, on_own_thread};

/// How long a list stays still before it is saved: opening threads one after another
/// saves once.
const SAVE_AFTER: Duration = Duration::from_millis(400);

/// The store is kept small: the newest 300 threads of a mailbox, and the full text of the
/// newest 100 (older ones are downloaded again when opened).
const KEEP_THREADS: usize = 300;
const KEEP_BODIES: usize = 100;

fn forget_body(mail: &mut Mail) {
    if mail.loaded {
        mail.loaded = false;
        mail.body = Default::default();
        mail.earlier = Default::default();
        mail.html = None;
    }
}

/// An account's own lists in its store, read together when the store opens.
struct Kept {
    outbox: Outbox,
    scheduled: Vec<scheduled::Scheduled>,
    snoozed: Vec<Snooze>,
}

impl Kept {
    fn read(store: &crate::store::Store) -> Self {
        Self {
            outbox: store.load("outbox").unwrap_or_default(),
            scheduled: scheduled::list(store),
            snoozed: store.load("snoozed").unwrap_or_default(),
        }
    }
}

/// Mailbox work on its way. A task kept here is dropped, and so stopped, when newer work
/// takes its place.
#[derive(Default)]
pub(super) struct Flow {
    /// A mailbox opened for the first time, being read from the store.
    stored: Option<Task<()>>,
    /// Threads whose content is being fetched (by the reader or ahead of it).
    bodies: OnItsWay<ThreadKey>,
    /// The thread to fetch once the keys stop stepping through rows.
    stepped: Option<Task<()>>,
    pace: Pace,
    /// Saves waiting for their list to settle, by account and record.
    saves: HashMap<(String, String), Task<()>>,
    /// The other accounts' snoozed lists, by account, as their stores have them.
    pub(super) other_snoozed: HashMap<String, Vec<Snooze>>,
}

impl Nimbra {
    /// Saves the Inbox as soon as it empties or stops being empty. Waiting for the next
    /// check for mail would lose that when Nimbra quits first.
    pub(super) fn save_if_zero_changed(&mut self, cx: &mut Context<Self>) {
        // Here, as everything that changes what quitting would lose notifies.
        crate::quitting::set_would_lose(self.would_lose(cx));
        self.save_inboxes_from_all(cx);
        if self.mailbox == View::Inbox
            && self.status == Status::Ready
            // Not `visible`: with Unread on, read mail is still in the Inbox.
            && self.done.iter().all(|done| *done) != self.inbox_saved_empty
        {
            self.save_list(Duration::ZERO, cx);
        }
    }

    /// The same from All inboxes, for each account's Inbox. Only when it lists all of them
    /// whole: else what is not listed would be saved as gone.
    fn save_inboxes_from_all(&mut self, cx: &mut Context<Self>) {
        if self.mailbox != View::AllInboxes
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
            let saved = self
                .inboxes_saved_empty
                .get(&account)
                .copied()
                .unwrap_or_else(|| {
                    crate::store::Store::saved_empty(&account, &record(View::Inbox))
                });
            let Some(store) = self.store_of(&account).filter(|_| saved != empty) else {
                self.inboxes_saved_empty.insert(account, saved);
                continue;
            };
            let rows: Vec<Mail> = self
                .mails
                .iter()
                .zip(&self.done)
                .filter(left)
                .map(|(mail, _)| Mail {
                    account: None, // as a store keeps its own account's
                    ..mail.clone()
                })
                .collect();
            if account == self.account_id {
                self.inbox_saved_empty = empty;
                self.cache.insert(View::Inbox, rows.clone());
                // It was listed whole: a page kept from before is none any more.
                self.next_pages.remove(&View::Inbox);
            }
            self.inboxes_saved_empty.insert(account, empty);
            let taken = store.snapshot();
            cx.background_executor()
                .spawn(async move {
                    if store
                        .save_newest(&record(View::Inbox), &rows, taken)
                        .is_err()
                    {
                        eprintln!("nimbra: could not save an inbox emptied from All inboxes");
                    }
                })
                .detach();
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
        self.leave_sidebar(cx);
        if self.mailbox == view || matches!(self.status, Status::SignedOut | Status::SigningIn) {
            return;
        }
        self.remember_mailbox();
        self.picked.clear();
        self.mailbox = view;
        self.back.clear();
        self.forward.clear();
        if self.is_demo() {
            // The sample data only has an inbox; Done shows what was cleared from it.
            self.selected = self.visible().first().copied().unwrap_or(0);
            cx.notify();
        } else {
            // Show what we already have for this mailbox, then refresh it quietly.
            // A mailbox seen before (this session or on disk) shows at once, even when it
            // was empty: "Nothing in …" is known, so there is no loading screen for it.
            // An empty All inboxes is out of date once mail has been seen in an Inbox.
            if view == View::AllInboxes
                && self.cache.get(&view).is_some_and(Vec::is_empty)
                && self.inbox_has_mail()
            {
                self.cache.remove(&view);
            }
            self.flow.stored = None;
            self.history.clear();
            self.history_groups.clear();
            match (self.cache.get(&view).cloned(), self.store.clone()) {
                (Some(mails), _) => self.show_known(view, mails, cx),
                // Not visited this session: what is stored on disk, read off the main thread.
                // The skeleton has the list's layout, and usually goes within a frame.
                (None, Some(store)) => {
                    self.show_unknown();
                    self.status = Status::Loading;
                    self.read_stored(view, store, cx);
                    cx.notify();
                }
                (None, None) => {
                    self.show_unknown();
                    self.refresh(cx);
                }
            }
        }
    }

    /// A mailbox whose list is known (empty or not): shown at once, then checked quietly.
    fn show_known(&mut self, view: View, mails: Vec<Mail>, cx: &mut Context<Self>) {
        self.cache.insert(view, mails.clone());
        self.done = vec![false; mails.len()];
        self.mails = mails;
        self.selected = 0;
        self.status = Status::Ready;
        self.load_selected(cx);
        self.look_again(cx);
    }

    fn show_unknown(&mut self) {
        self.mails.clear();
        self.done.clear();
        self.selected = 0;
    }

    /// Reads a mailbox's stored list in the background, for when it is still the one shown.
    fn read_stored(&mut self, view: View, store: Arc<crate::store::Store>, cx: &mut Context<Self>) {
        let record = self.record_for(view);
        let generation = self.account_gen;
        self.flow.stored = Some(cx.spawn(async move |this, cx| {
            let mails = cx
                .background_executor()
                .spawn(async move {
                    let mut mails: Option<Vec<Mail>> = store.load(&record);
                    mails.iter_mut().flatten().for_each(Mail::refresh_time);
                    mails
                })
                .await;
            this.update(cx, |this, cx| {
                if this.account_gen != generation || this.mailbox != view {
                    return;
                }
                this.flow.stored = None;
                match mails {
                    // Unless a listing came first: it is newer.
                    Some(mails) if this.mails.is_empty() => this.show_known(view, mails, cx),
                    Some(_) => {}
                    None => this.refresh(cx),
                }
                cx.notify();
            })
            .ok();
        }));
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
        let recipient = matches!(view, View::Sent | View::Drafts);
        let count_ids: Vec<String> = [
            "INBOX",
            "DRAFT",
            "CATEGORY_PERSONAL",
            "CATEGORY_UPDATES",
            "CATEGORY_PROMOTIONS",
            "CATEGORY_SOCIAL",
        ]
        .map(String::from)
        .into_iter()
        .chain(self.labels.iter().take(8).map(|l| l.id.clone()))
        .collect();
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let result = within(cx.background_executor(), async move {
                let changes = client.history(&start)?;
                if changes.threads.len() > 40 {
                    return Err(mail::HistoryExpired.into());
                }
                // Fresh details for each changed thread; `None` when it is gone.
                let mut updates = Vec::new();
                for id in &changes.threads {
                    match client.summary(id, recipient) {
                        Ok(summary) => updates.push((id.clone(), Some(summary))),
                        Err(error) if error.to_string().starts_with("Gmail error 404") => {
                            updates.push((id.clone(), None))
                        }
                        Err(error) => return Err(error),
                    }
                }
                let counts = (count && !updates.is_empty()).then(|| client.counts(&count_ids));
                let page = if !find_page {
                    None
                } else if (1..=500).contains(&listed) {
                    client.page_after(view, label_id.as_deref(), listed).ok()
                } else {
                    Some(None)
                };
                anyhow::Ok(((changes.latest, updates, counts, page), label_id))
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
                        for (id, summary) in updates {
                            list.retain(|mail| mail.id.as_deref() != Some(id.as_str()));
                            if let Some(summary) = summary
                                && view.takes(&summary, label_id.as_deref())
                            {
                                list.push(Mail::from_summary(summary));
                            }
                        }
                        list.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms));
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

    /// Fetches, in the background, the content of the newest threads on screen that have not
    /// been downloaded yet, and saves it, so opening them is instant now and after a restart.
    pub(super) fn prefetch(&mut self, cx: &mut Context<Self>) {
        // A thread in full is 120 units of the 6,000 a minute: not the whole screen.
        const AHEAD: usize = 3;
        if self.connection.is_none() {
            return;
        }
        if self.prefetching || self.mailbox == View::Search || self.status != Status::Ready {
            return;
        }
        // Each thread with its own account's connection (All inboxes mixes accounts).
        let mut wanted = Vec::new();
        // The top few only, not the next few that are missing: each check would fetch more.
        for i in self.visible().into_iter().take(AHEAD) {
            if i == self.selected {
                continue; // the one on show is loaded by itself
            }
            if let Some(mail) = self.mails.get(i).filter(|mail| !mail.loaded)
                && let (Some(id), Some(client)) = (mail.id.clone(), self.client_at(i))
            {
                let key: ThreadKey = (id, self.account_of(mail).to_string());
                if self.flow.bodies.begin(key.clone()) {
                    wanted.push((key, client));
                }
            }
        }
        if wanted.is_empty() {
            return;
        }
        self.prefetching = true;
        let view = self.mailbox;
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            // Side by side, each a task of its own: nothing waits on a thread for the others.
            let fetches = wanted.into_iter().map(|(key, client)| {
                let id = key.0.clone();
                let fetch = within(&executor, async move { client.thread(&id) });
                async move { (key, fetch.await) }
            });
            let fetched = futures::future::join_all(fetches).await;
            this.update(cx, |this, cx| {
                for (key, _) in &fetched {
                    this.flow.bodies.end(key);
                }
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
                }
                // Opened while it was on its way here, and that way failed: asked for itself.
                this.load_shown(cx);
                cx.notify();
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
                    let mails: Option<Vec<Mail>> = store.load(&view_record);
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
                            if store.save("outbox", &older).is_err() {
                                eprintln!("nimbra: could not move the offline queue over");
                            }
                        }
                        Job::left_behind(&account, &client, &store)
                    });
                    let kept = Kept::read(&store);
                    anyhow::Ok((store, saved, mails, unsent, kept))
                })
                .await;
            let Ok((store, saved, mails, unsent, kept)) = opened else {
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
                // A catch-up from before a switch may be taking what it sent out of them
                // right now: then they are read again, after it.
                let kept = if crate::store::writing() || this.catching_up.contains(&this.account_id)
                {
                    Kept::read(&store)
                } else {
                    kept
                };
                let mut outbox = kept.outbox;
                this.scheduled = kept.scheduled;
                this.snoozed = kept.snoozed;
                let queued_meanwhile = !this.outbox.changes.is_empty();
                outbox.changes.append(&mut this.outbox.changes);
                this.outbox = outbox;
                this.store = Some(store);
                // Not only after a full listing: opening into what is saved often has none.
                this.load_photo(this.account_id.clone(), cx);
                if queued_meanwhile {
                    this.save_outbox();
                }
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
                if let Some(mut mails) = mails {
                    for mail in &mut mails {
                        mail.refresh_time();
                    }
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
    pub(super) fn persist(&mut self, cx: &mut Context<Self>) {
        self.save_list(SAVE_AFTER, cx);
    }

    fn save_list(&mut self, after: Duration, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        // All inboxes mixes accounts: it is never saved in any one account's store.
        if matches!(self.mailbox, View::Search | View::AllInboxes) || self.status != Status::Ready {
            return;
        }
        let mut mails: Vec<Mail> = self
            .mails
            .iter()
            .zip(&self.done)
            .filter(|(_, done)| !**done)
            .map(|(mail, _)| mail.clone())
            .take(KEEP_THREADS)
            .collect();
        mails.iter_mut().skip(KEEP_BODIES).for_each(forget_body);
        if self.mailbox == View::Inbox {
            self.inbox_saved_empty = mails.is_empty();
            self.inboxes_saved_empty
                .insert(self.account_id.clone(), mails.is_empty());
        }
        let name = self.record_for(self.mailbox);
        let taken = store.snapshot();
        let saved = Saved {
            account: self.account.to_string(),
            initials: self.me_initials.to_string(),
            counts: self.counts.clone(),
            labels: self.labels.clone(),
            synced_to: self.synced_to.clone(),
        };
        // A newer copy of the same list takes the place of one still waiting.
        let key = (self.account_id.clone(), name.clone());
        let save = cx.spawn(async move |_, cx| {
            cx.background_executor().timer(after).await;
            cx.background_executor()
                .spawn(async move {
                    // The list first: a sync point ahead of its list would miss what changed.
                    if let Err(error) = store
                        .save_newest(&name, &mails, taken)
                        .and_then(|_| store.save_newest("meta", &saved, taken))
                    {
                        eprintln!("nimbra: saving the mail store failed: {error}");
                    }
                })
                .await;
        });
        self.flow.saves.insert(key, save);
    }

    /// Opens the thread behind a clicked notification (macOS brings the app forward itself),
    /// and says why quitting was put off. Both are rung from outside GPUI, so this waits for
    /// the bell rather than looking every so often.
    pub(super) fn watch_notification_clicks(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let mut heard = 0;
            loop {
                heard = OUTSIDE.after(heard).await;
                if crate::quitting::take_put_off() {
                    // It may be hidden, and whoever quit has to see why it did not.
                    cx.update(crate::show_main_window).ok();
                    let shown = this.update(cx, |this, cx| {
                        this.tell(QUIT_WOULD_LOSE);
                        cx.notify();
                    });
                    if shown.is_err() {
                        break; // the window is gone
                    }
                }
                let clicked = crate::notify::take_clicked();
                let Some(id) = clicked.into_iter().rev().find(|id| id.as_str() != "test") else {
                    continue;
                };
                // The window may be hidden (closed with ⌘W): bring it back first.
                cx.update(crate::show_main_window).ok();
                if this
                    .update(cx, |this, cx| this.open_thread(id, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// Shows a thread by its Gmail id: switches to the Inbox, and opens the thread now if
    /// it is listed, or as soon as the list has caught up.
    fn open_thread(&mut self, id: String, cx: &mut Context<Self>) {
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
                if self.mailbox == View::AllInboxes {
                    self.refresh(cx);
                } else {
                    self.select_mailbox(View::AllInboxes, cx);
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
            self.select_mailbox(View::Inbox, cx);
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
                        Duration::from_secs(this.settings.check_every.clamp(15, 3600) as u64)
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
                    let slice = left.min(Duration::from_secs(30));
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
                            on_own_thread("nimbra-mail-wait", move || client.wait_for_change(slice))
                                .await
                                .unwrap_or(false)
                        }
                        None => false,
                    };
                    if woke {
                        break;
                    }
                    cx.background_executor()
                        .timer(slice.saturating_sub(sliced.elapsed()))
                        .await;
                }
                let Ok(client) = this.update(cx, |this, _| {
                    this.connection.clone().map(|c| (c, this.account_gen))
                }) else {
                    break; // the window is gone
                };
                let Some((client, generation)) = client else {
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
                        let Some(changes) = changes else {
                            // First look: what blocked senders left in the inbox meanwhile.
                            this.sweep_blocked(this.account_id.clone(), cx);
                            return (Vec::new(), store);
                        };
                        if !changes.threads.is_empty() {
                            this.feed_changes += 1;
                        }
                        // Anything changed: bring the open mailbox up to date, cheaply.
                        if !changes.threads.is_empty() && this.compose.is_none() {
                            // All inboxes cannot be patched from one feed: list it again.
                            if this.mailbox == View::AllInboxes {
                                // Started again each time, a slow one would never finish.
                                if !this.fetching() {
                                    this.fetch_all_inboxes(cx);
                                    this.checks.quiet();
                                }
                            } else if !this.fetching() && this.sync_view(cx) {
                                this.checks.quiet();
                            }
                        }
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
                        let fresh = match &store {
                            Some(store) => crate::blocking::screen(&*client, store, fresh),
                            None => fresh,
                        };
                        // Sender and subject for the first few; the rest are summed up.
                        let summaries = fresh
                            .iter()
                            .take(3)
                            .filter_map(|id| client.summary(id, false).ok())
                            .collect::<Vec<_>>();
                        (fresh, summaries)
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
                            &mail.id,
                            &mail.sender,
                            &mail.subject,
                            this.settings.notify_sound,
                        );
                    }
                    if extra > 0 {
                        crate::notify::show(
                            "more",
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
                        let every = this.settings.check_every.clamp(15, 3600) as u64;
                        // Sooner while changes wait there for a connection.
                        let waiting = !this.away_waiting.is_empty();
                        Duration::from_secs(if waiting { 15 } else { every })
                    })
                    .unwrap_or(EVERY);
                cx.background_executor().timer(wait).await;
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
                            let fresh = match &store {
                                Some(store) => {
                                    if swept {
                                        crate::blocking::sweep(&*client, store);
                                    }
                                    crate::blocking::screen(&*client, store, fresh)
                                }
                                None => fresh,
                            };
                            let unread = client
                                .counts(&["INBOX".to_string()])
                                .get("INBOX")
                                .map_or(0, |(unread, _)| *unread);
                            let summaries: Vec<mail::Summary> = fresh
                                .iter()
                                .take(3)
                                .filter_map(|thread| client.summary(thread, false).ok())
                                .collect();
                            let extra = fresh.len().saturating_sub(summaries.len());
                            // Snoozes and scheduled sends kept with this account.
                            let woken = store.as_ref().map_or_else(Vec::new, |store| {
                                carry_out_due(&account, store, &connected, &writes)
                            });
                            let snoozed = store.as_ref().map(|store| {
                                store.load::<Vec<Snooze>>("snoozed").unwrap_or_default()
                            });
                            anyhow::Ok((at, summaries, extra, changed, unread, woken, snoozed))
                        })
                        .await;
                    let Ok((at, summaries, extra, changed, unread, woken, snoozed)) = checked
                    else {
                        continue; // offline or signed out: next time
                    };
                    feeds.insert(id.clone(), at);
                    this.update(cx, |this, cx| {
                        if !this.others.contains_key(&id) {
                            return;
                        }
                        if let Some(snoozed) = snoozed.filter(|_| id != this.account_id) {
                            this.took_snoozed(id.clone(), snoozed);
                        }
                        this.account_unread.insert(id.clone(), unread);
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
                            && this.mailbox == View::AllInboxes
                            && this.compose.is_none()
                            && !this.fetching()
                        {
                            this.fetch_all_inboxes(cx);
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
        if self.mailbox == View::AllInboxes {
            self.fetch_all_inboxes(cx);
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
                // Read the history point first, so nothing that changes during the
                // fetch is missed by the next sync.
                let (email, history_id) = client.profile()?;
                let (threads, next_page) =
                    client.threads_page(view, label_id.as_deref(), 30, None)?;
                // Labels are a nicety; the mailbox still loads without them.
                let labels = need_labels.then(|| client.labels().unwrap_or_default());
                // Sidebar counts: the system mailboxes plus the labels that are listed.
                let mut ids: Vec<String> = [
                    "INBOX",
                    "DRAFT",
                    "CATEGORY_PERSONAL",
                    "CATEGORY_UPDATES",
                    "CATEGORY_PROMOTIONS",
                    "CATEGORY_SOCIAL",
                ]
                .map(String::from)
                .to_vec();
                ids.extend(match &labels {
                    Some(labels) => labels.iter().take(8).map(|l| l.id.clone()).collect(),
                    None => known_labels.into_iter().take(8).collect::<Vec<_>>(),
                });
                let counts = if count {
                    client.counts(&ids)
                } else {
                    HashMap::new()
                };
                anyhow::Ok((email, history_id, threads, labels, counts, next_page))
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
                        this.show_inbox(email, threads, cx)
                    }
                    Err(error) => this.fail(error, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// All inboxes: the first page of every signed-in account's inbox, fetched side by side,
    /// merged newest first, each row marked with its account. Also every account's unread
    /// count. An account that fails is left out (with a note); all failing is an error.
    fn fetch_all_inboxes(&mut self, cx: &mut Context<Self>) {
        let Some(shown) = self.connection.clone() else {
            return;
        };
        let mut accounts: Vec<(String, Arc<mail::Client>)> = vec![(self.account_id.clone(), shown)];
        for account in &self.settings.accounts {
            if let Some(client) = self.others.get(&account.id) {
                accounts.push((account.id.clone(), client.clone()));
            }
        }
        // As far as it was scrolled: listing again must not drop the pages below the first.
        // Cleared rows do not count, or each listing after a Done would reach further.
        let accounts: Vec<(String, Arc<mail::Client>, usize)> = accounts
            .into_iter()
            .map(|(id, client)| {
                let listed = self
                    .mails
                    .iter()
                    .zip(&self.done)
                    .filter(|(mail, done)| !**done && self.account_of(mail) == id)
                    .count();
                (id, client, listed.next_multiple_of(30).clamp(30, 480))
            })
            .collect();
        if self.mails.is_empty() && !self.cache.contains_key(&View::AllInboxes) {
            self.status = Status::Loading;
            // As when Nimbra opens: what was saved picks what to draw until Gmail answers.
            self.all_inboxes_saved_empty = !self.inbox_has_mail()
                && self.settings.accounts.iter().all(|account| {
                    crate::store::Store::saved_empty(&account.id, &record(View::Inbox))
                });
        }
        let check = self.checks.start(View::AllInboxes, "");
        let seen = Asked::now(self.changes_seen());
        cx.notify();
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor();
            // A deadline each, so that one account cannot hold up the others.
            let results =
                futures::future::join_all(accounts.into_iter().map(|(id, client, count)| {
                    let listing = within(executor, async move {
                        let (threads, next) =
                            client.threads_page(View::Inbox, None, count, None)?;
                        let unread = client
                            .counts(&["INBOX".to_string()])
                            .get("INBOX")
                            .map_or(0, |(unread, _)| *unread);
                        Ok((threads, next, unread))
                    });
                    async move { (id, listing.await) }
                }))
                .await;
            this.update(cx, |this, cx| {
                if this.account_gen != generation || !this.finish_check(check, cx) {
                    return;
                }
                let mut fresh: Vec<Mail> = Vec::new();
                let mut first_error = None;
                let mut failed = 0;
                let mut pages = HashMap::new();
                let asked = results.len();
                for (id, result) in results {
                    match result {
                        Ok((threads, next, unread)) => {
                            if let Some(next) = next {
                                pages.insert(id.clone(), next);
                            }
                            this.account_unread.insert(id.clone(), unread);
                            fresh.extend(threads.into_iter().map(|summary| {
                                let mut mail = Mail::from_summary(summary);
                                mail.account = Some(id.clone());
                                mail
                            }));
                        }
                        Err(error) => {
                            failed += 1;
                            first_error.get_or_insert(error);
                        }
                    }
                }
                this.all_inboxes_reached = failed == 0 && asked >= this.settings.accounts.len();
                this.set_all_pages(pages);
                if !this.writes.any() {
                    this.shifted.remove(&View::AllInboxes);
                }
                if failed == asked
                    && let Some(error) = first_error
                {
                    this.fail(error, cx);
                    return;
                }
                fresh.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms));
                this.install(fresh, cx);
                if failed == 0 {
                    this.checked.insert(this.record_for(View::AllInboxes), seen);
                }
                // After the listing, which clears the notice.
                if failed > 0 {
                    this.notice = Some(match failed {
                        1 => "One account could not be checked. Reload to try again.".into(),
                        n => format!("{n} accounts could not be checked. Reload to try again.")
                            .into(),
                    });
                }
                // Count only; never addresses or subjects.
                eprintln!("nimbra: All inboxes listed ({} threads)", this.mails.len());
            })
            .ok();
        })
        .detach();
    }

    /// Shows a freshly fetched listing, reusing thread contents already in memory and
    /// keeping the open thread and any just-cleared threads as they were.
    fn show_inbox(&mut self, email: String, threads: Vec<mail::Summary>, cx: &mut Context<Self>) {
        // A different Google account than the one the store was saved for: start clean.
        if self.account.contains('@') && self.account.as_ref() != email.as_str() {
            if let Some(store) = self.store.clone() {
                // The other account's lists still waiting to be saved are not saved now.
                let shown = self.account_id.clone();
                self.flow.saves.retain(|(of, _), _| *of != shown);
                cx.background_executor()
                    .spawn(async move { store.clear() })
                    .detach();
            }
            self.cache.clear();
            self.outbox = Outbox::default();
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

        let fresh = threads.into_iter().map(Mail::from_summary).collect();
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
                    cx.background_executor().timer(Duration::from_secs(2)).await;
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

    /// Fetches the content of the thread on show, without marking it read. While keys step
    /// through the rows faster than threads load, only the row they stop on is fetched.
    pub(super) fn load_selected(&mut self, cx: &mut Context<Self>) {
        self.flow.stepped = None;
        if self.flow.pace.at_once(std::time::Instant::now()) {
            self.load_shown(cx);
            return;
        }
        self.flow.stepped = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Pace::GAP).await;
            this.update(cx, |this, cx| this.load_shown(cx)).ok();
        }));
    }

    fn load_shown(&mut self, cx: &mut Context<Self>) {
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
        if !self.flow.bodies.begin(key.clone()) {
            return;
        }
        cx.spawn(async move |this, cx| {
            let thread_id = id.clone();
            let result = within(cx.background_executor(), async move {
                client.thread(&thread_id)
            })
            .await;
            this.update(cx, |this, cx| {
                this.flow.bodies.end(&key);
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
        // It follows what waits for the connection, so it waits too.
        if account == self.account_id {
            if self.outbox.changes.iter().any(|(queued, _)| *queued == id) {
                self.outbox.changes.push((id, change));
                self.save_outbox();
                self.went_offline(cx);
                return;
            }
        } else if self.away_waiting.contains(&account)
            && let Some(store) = self.other_stores.get(&account).cloned()
        {
            // Read and queued on the store's own thread, in order with its other writes.
            let queued = store.update_later("outbox", {
                let id = id.clone();
                move |outbox: &mut Outbox| {
                    let waiting = outbox.changes.iter().any(|(queued, _)| *queued == id);
                    if waiting {
                        outbox.changes.push((id, change));
                    }
                    waiting
                }
            });
            cx.spawn(async move |this, cx| {
                if queued.await != Some(true) {
                    this.update(cx, |this, cx| {
                        this.make_change(client, account, id, change, removed, cx)
                    })
                    .ok();
                }
            })
            .detach();
            return;
        }
        self.make_change(client, account, id, change, removed, cx);
    }

    /// Makes a change to a thread on the server, keeping it for later when offline.
    fn make_change(
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
    /// Where each account's next page in All inboxes starts.
    fn set_all_pages(&mut self, pages: HashMap<String, String>) {
        self.all_next = pages;
        self.all_inboxes_whole = self.all_inboxes_reached && self.all_next.is_empty();
        if self.all_next.is_empty() {
            self.next_pages.remove(&View::AllInboxes);
        } else {
            // That there is a next page. The accounts' tokens are in `all_next`.
            self.next_pages.insert(View::AllInboxes, String::new());
        }
    }

    /// The next page of every account that has one, merged into All inboxes.
    fn load_more_of_all(&mut self, from_top: bool, cx: &mut Context<Self>) {
        if self.loading_more || self.fetching() {
            return;
        }
        let asked: Vec<_> = self
            .all_next
            .iter()
            .filter_map(|(id, page)| Some((id.clone(), self.client_for(Some(id))?, page.clone())))
            .collect();
        if asked.is_empty() {
            // Theirs are accounts that are not connected.
            self.more_failed.insert(View::AllInboxes);
            return;
        }
        self.loading_more = true;
        cx.notify();
        let generation = self.account_gen;
        let count = if from_top { self.mails.len() + 30 } else { 30 };
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor();
            let results = futures::future::join_all(asked.into_iter().map(|(id, client, page)| {
                let from = (!from_top).then(|| page.clone());
                let listing = within(executor, async move {
                    client.threads_page(View::Inbox, None, count, from.as_deref())
                });
                async move { (id, page, listing.await) }
            }))
            .await;
            this.update(cx, |this, cx| {
                if this.account_gen != generation {
                    return;
                }
                this.loading_more = false;
                if this.mailbox != View::AllInboxes {
                    cx.notify();
                    return;
                }
                let mut pages = this.all_next.clone();
                let mut list = this.listed();
                let before = list.len();
                let known: HashSet<ThreadKey> = this
                    .mails
                    .iter()
                    .map(|mail| this.thread_key(mail))
                    .collect();
                let mut first_error = None;
                for (id, page, result) in results {
                    // A listing made meanwhile has other pages: this one is not from them.
                    if pages.get(&id) != Some(&page) {
                        continue;
                    }
                    match result {
                        Ok((threads, next)) => {
                            match next {
                                Some(next) => pages.insert(id.clone(), next),
                                None => pages.remove(&id),
                            };
                            list.extend(threads.into_iter().filter_map(|summary| {
                                let mut mail = Mail::from_summary(summary);
                                mail.account = Some(id.clone());
                                (!known.contains(&this.thread_key(&mail))).then_some(mail)
                            }));
                        }
                        Err(error) => {
                            first_error.get_or_insert(error);
                        }
                    }
                }
                if from_top && list.len() == before && !pages.is_empty() {
                    this.more_failed.insert(View::AllInboxes); // or it would ask without end
                }
                this.set_all_pages(pages);
                list.sort_by_key(|mail| std::cmp::Reverse(mail.timestamp_ms));
                this.install(list, cx);
                if let Some(error) = first_error {
                    this.more_failed.insert(View::AllInboxes);
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
        let left = f32::from(self.list_scroll.max_offset().height)
            + f32::from(self.list_scroll.offset().y);
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
        if self.mailbox == View::AllInboxes {
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
        cx.notify();
    }
}
