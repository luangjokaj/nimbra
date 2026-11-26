//! Sending: a message on its way out (the undo delay, the queue, retries and what happens
//! when Gmail cannot be reached), mail scheduled for later, and what must finish before the
//! app quits.

use super::*;

/// What a catch-up got done.
#[derive(Default)]
pub(super) struct CaughtUp {
    pub(super) changes: usize,
    pub(super) sent: usize,
    /// Not sent by itself: the subject, whether it may have arrived all the same, and
    /// whether Drafts has it.
    pub(super) kept: Vec<(String, bool, bool)>,
}

/// Sends what waited for the connection, in order. Blocking.
pub(super) fn catch_up(
    client: &mail::Client,
    changes: &[(String, mail::Change)],
    queue: &[Arc<Job>],
) -> anyhow::Result<CaughtUp> {
    // Is Gmail reachable again?
    client.profile()?;
    let mut done = CaughtUp::default();
    for (id, change) in changes {
        match client.change(id, *change) {
            Err(error) if error.is::<mail::Offline>() => return Ok(done),
            // Anything else (the thread is gone, say) is dropped.
            _ => done.changes += 1,
        }
    }
    for job in queue {
        match job.send() {
            None => {}
            Some(Ok(())) => done.sent += 1,
            Some(Err(_)) => {
                let unsure = match job.stage() {
                    Stage::Held => break, // still offline
                    stage => stage == Stage::Unsure,
                };
                done.kept.push((job.subject(), unsure, job.to_drafts()));
            }
        }
    }
    Ok(done)
}

/// The offline queue read from a store, without the `caught_up` changes at its start that
/// were made since it was read, and with what was queued meanwhile after it.
fn with_caught_up(
    mut read: Outbox,
    caught_up: usize,
    mut meanwhile: Vec<(String, mail::Change)>,
) -> Outbox {
    read.changes.drain(..caught_up.min(read.changes.len()));
    read.changes.append(&mut meanwhile);
    read
}

/// Send, while From is changing and the files are still being read from the old account.
const MOVING_ATTACHMENTS: &str =
    "The attachments are still moving to the account you picked. Try again in a moment.";

impl Nimbra {
    /// The reconnect loop is no longer wanted: its account was left, or Gmail is back.
    pub(super) fn stop_retrying(&mut self) {
        self.retrying = false;
        self.retry_gen += 1;
    }

    /// The shown account's messages that wait for the connection.
    pub(super) fn waiting_mail(&self) -> impl Iterator<Item = &Arc<Job>> {
        self.queue
            .iter()
            .filter(|job| job.account == self.account_id)
    }

    /// Adds to the queue of an account that is not shown, in its store (handed to its
    /// writer at once, so that it follows what is already there and quitting waits for it).
    /// Whether it has a store to keep it in.
    pub(super) fn queue_away(
        &mut self,
        account: &str,
        changes: Vec<(String, mail::Change)>,
    ) -> bool {
        let Some(store) = self.other_stores.get(account) else {
            return false;
        };
        drop(store.update_soon("outbox", move |outbox: &mut Outbox| {
            outbox.changes.extend(changes)
        }));
        self.away_waiting.insert(account.to_string());
        self.away_queued += 1;
        true
    }

    /// Makes the changes that wait for an account that is not shown.
    pub(super) async fn catch_up_away(
        this: &gpui::WeakEntity<Self>,
        cx: &mut gpui::AsyncApp,
        account: &str,
        client: &Arc<mail::Client>,
    ) {
        let claimed = this.update(cx, |this, _| {
            if !this.away_waiting.contains(account) {
                return None;
            }
            let store = this.other_stores.get(account).cloned()?;
            // One at a time per account, with the shown account's loop too.
            this.catching_up
                .insert(account.to_string())
                .then_some(store)
        });
        let Ok(Some(store)) = claimed else {
            return;
        };
        let (client, kept_in) = (client.clone(), store.clone());
        let made = cx
            .background_executor()
            .spawn(async move {
                let changes = kept_in.load::<Outbox>("outbox").unwrap_or_default().changes;
                catch_up(&*client, &changes, &[]).map_or(0, |done| done.changes)
            })
            .await;
        let Ok(left) = this.update(cx, |this, cx| {
            this.catching_up.remove(account);
            let queued = this.away_queued;
            let left = this.drop_caught_up(account, Some(&store), made, true, cx);
            cx.notify();
            (left, queued)
        }) else {
            return;
        };
        let (Some(left), queued) = left else {
            return;
        };
        let left = left.await;
        this.update(cx, |this, _| {
            // Not if something was put in it since, or it is shown (or gone) by now.
            let same = this
                .other_stores
                .get(account)
                .is_some_and(|now| Arc::ptr_eq(now, &store));
            if same && this.away_queued == queued && matches!(left, Ok(Ok(false))) {
                this.away_waiting.remove(account);
            }
        })
        .ok();
    }

    /// Takes the changes a catch-up got through out of its account's queue, whichever is
    /// shown by now. `away`: it took them from the store, not from `outbox`. When they are
    /// taken out of a store, the answer says whether anything is left in it.
    fn drop_caught_up(
        &mut self,
        account: &str,
        store: Option<&Arc<crate::store::Store>>,
        changes: usize,
        away: bool,
        _cx: &mut Context<Self>,
    ) -> Option<futures::channel::oneshot::Receiver<anyhow::Result<bool>>> {
        let shown = account == self.account_id;
        if let (true, true, Some(read_since)) = (shown, away, self.outbox_pending.as_mut()) {
            // The queue being read is from before these were taken out: so is what it
            // finds. Taken out of that when it comes (`outbox_read`).
            *read_since += changes;
        } else if shown && (self.store.is_some() || store.is_none()) {
            if changes > 0 {
                self.outbox
                    .changes
                    .drain(..changes.min(self.outbox.changes.len()));
                self.save_outbox();
            }
            return None;
        }
        let store = store?;
        Some(store.update_soon("outbox", move |outbox: &mut Outbox| {
            outbox.changes.drain(..changes.min(outbox.changes.len()));
            !outbox.changes.is_empty()
        }))
    }

    /// The shown account's offline queue, read from its store as it opened: what waited
    /// there, without what a catch-up took out since, then what was queued meanwhile.
    pub(super) fn outbox_read(
        &mut self,
        generation: u64,
        read: Option<Outbox>,
        cx: &mut Context<Self>,
    ) {
        if self.account_gen != generation {
            return;
        }
        let Some(caught_up) = self.outbox_pending.take() else {
            return;
        };
        let queued_meanwhile = !self.outbox.changes.is_empty();
        let meanwhile = std::mem::take(&mut self.outbox.changes);
        self.outbox = with_caught_up(read.unwrap_or_default(), caught_up, meanwhile);
        if queued_meanwhile || caught_up > 0 {
            self.save_outbox();
        }
        if !self.outbox.changes.is_empty() {
            self.went_offline(cx);
        }
        cx.notify();
    }

    /// Gmail could not be reached: keep checking, then send what is waiting.
    pub(super) fn went_offline(&mut self, cx: &mut Context<Self>) {
        if self.connection.is_none() {
            // No connection to check with: opening the account starts this again.
            return;
        }
        self.offline = true;
        cx.notify();
        if self.retrying {
            return;
        }
        self.retrying = true;
        self.retry_gen += 1;
        let attempt = self.retry_gen;
        eprintln!("nimbra: offline");
        cx.spawn(async move |this, cx| {
            loop {
                // Sooner when the Mac wakes or the network comes back.
                crate::integration::pause(cx.background_executor().timer(Duration::from_secs(15)))
                    .await;
                let next = this.update(cx, |this, _| {
                    if this.retry_gen != attempt {
                        return Err(()); // another loop's turn, or none's
                    }
                    let Some(client) = this.connection.clone() else {
                        this.stop_retrying();
                        return Err(());
                    };
                    let account = this.account_id.clone();
                    // One at a time per account, or two would each count off the same changes.
                    if !this.catching_up.insert(account.clone()) {
                        return Ok(None);
                    }
                    let changes = this.outbox.changes.clone();
                    let queue: Vec<_> = this.waiting_mail().cloned().collect();
                    Ok(Some((client, account, this.store.clone(), changes, queue)))
                });
                let (client, account, store, changes, queue) = match next {
                    Ok(Ok(Some(next))) => next,
                    Ok(Ok(None)) => continue,
                    _ => break,
                };
                let result = cx
                    .background_executor()
                    .spawn(async move { catch_up(&*client, &changes, &queue) })
                    .await;
                let done = this.update(cx, |this, cx| {
                    // First, and whoever is shown by now.
                    this.catching_up.remove(&account);
                    let changed = result.as_ref().map_or(0, |done| done.changes);
                    if changed > 0 {
                        drop(this.drop_caught_up(&account, store.as_ref(), changed, false, cx));
                    }
                    let left = |job: &Arc<Job>| matches!(job.stage(), Stage::Held | Stage::Out);
                    let through: Vec<_> =
                        this.queue.iter().filter(|job| !left(job)).cloned().collect();
                    this.queue.retain(left);
                    this.sends
                        .retain(|job| !through.iter().any(|t| Arc::ptr_eq(t, job)));
                    if this.retry_gen != attempt {
                        return true;
                    }
                    match result {
                        Ok(CaughtUp { changes, sent, kept }) => {
                            if this.outbox.changes.is_empty() && this.waiting_mail().next().is_none() {
                                this.offline = false;
                                this.stop_retrying();
                                eprintln!("nimbra: back online ({changes} changes, {sent} messages caught up)");
                                this.tell(if let Some((subject, unsure, saved)) = kept.first() {
                                    let subject = short(subject, 40);
                                    let what = if *unsure { "may already be sent: check Sent" } else { "could not be sent" };
                                    let where_now = if *saved { "It is in Drafts." } else { "It is kept on this Mac." };
                                    format!("Back online. “{subject}” {what}. {where_now}")
                                } else if sent > 0 {
                                    format!("Back online. {sent} waiting message{} sent.", if sent == 1 { " was" } else { "s were" })
                                } else {
                                    "Back online.".to_string()
                                });
                                this.refresh(cx);
                                cx.notify();
                                return true;
                            }
                        }
                        Err(error) if error.is::<mail::SignedOut>() => {
                            this.stop_retrying();
                            this.fail(error, cx);
                            return true;
                        }
                        Err(_) => {}
                    }
                    cx.notify();
                    false
                });
                if !matches!(done, Ok(false)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Hands the offline queue to the store's writer now: written in order with the rest,
    /// and quitting waits for it.
    pub(super) fn save_outbox(&self) {
        // While it is being read, what is here is only part of it (`outbox_read` saves).
        if let Some(store) = self
            .store
            .as_ref()
            .filter(|_| self.outbox_pending.is_none())
        {
            store.save_soon("outbox", self.outbox.clone());
        }
    }

    /// Sends a message whose undo delay is over.
    pub(super) fn dispatch(&mut self, mut composer: Composer, cx: &mut Context<Self>) {
        let serial = composer.serial;
        let Some(job) = self
            .sends
            .iter()
            .find(|job| job.composer == Some(serial))
            .cloned()
        else {
            return;
        };
        job.set_draft(composer.draft_id.clone());
        composer.sending = true;
        self.parked.push((Parked::InFlight, composer));
        self.send_job(job, cx);
    }

    fn send_job(&self, job: Arc<Job>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let sender = job.clone();
            let result = cx
                .background_executor()
                .spawn(async move { sender.send() })
                .await;
            let Some(result) = result else {
                return;
            };
            this.update(cx, |this, cx| this.send_ended(job, result, cx))
                .ok();
        })
        .detach();
    }

    /// Gmail answered a send, or could not be reached. Its account may not be the one
    /// shown, and may not have been when it was sent.
    fn send_ended(&mut self, job: Arc<Job>, result: anyhow::Result<()>, cx: &mut Context<Self>) {
        let serial = job.composer;
        let shown = job.account == self.account_id;
        let subject = short(&job.subject(), 40);
        let parked = self
            .parked
            .iter()
            .position(|(_, c)| Some(c.serial) == serial);
        let error = match result {
            Ok(()) => {
                self.sends.retain(|j| !Arc::ptr_eq(j, &job));
                if let Some(at) = parked {
                    self.parked.remove(at);
                }
                self.notice = Some(match serial {
                    Some(_) => "Message sent.".into(),
                    None => format!("Sent “{subject}”, which was waiting.").into(),
                });
                if matches!(self.mailbox, View::Sent | View::Drafts) {
                    self.refresh(cx);
                }
                cx.notify();
                return;
            }
            Err(error) => error,
        };
        let offline = error.is::<mail::Offline>();
        let unsure = error.is::<mail::MayHaveArrived>();
        let room = parked.is_some() && self.compose.is_none();
        if self.shown_signed_out(&error) {
            self.compose = None;
            self.fail(error, cx);
        } else if offline && !unsure && (shown || !room) {
            if let Some(at) = parked {
                self.parked.remove(at);
            }
            let kept = job.kept();
            self.queue.push(job);
            if shown {
                if kept {
                    self.notice = Some(
                        "You’re offline. The message will be sent when the connection is back."
                            .into(),
                    );
                } else {
                    self.tell("You’re offline, and the message can’t be kept on this Mac. Don’t quit until it has been sent.");
                }
                self.went_offline(cx);
            } else {
                self.notice = Some(
                    format!("“{subject}” will be sent when you next open its account.").into(),
                );
            }
        } else {
            self.sends.retain(|j| !Arc::ptr_eq(j, &job));
            match parked {
                // Back on screen to look at.
                Some(at) if room && !unsure => {
                    job.forget();
                    let composer = &mut self.parked[at].1;
                    composer.error = Some(format!("Not sent: {error}").into());
                    let serial = composer.serial;
                    self.unpark(serial, cx);
                }
                // Not one to send again with a keystroke, or no room to show it.
                _ => {
                    if let Some(at) = parked {
                        self.parked.remove(at);
                    }
                    let why = if unsure {
                        format!(
                            "“{subject}” may already be sent: check Sent before sending it again."
                        )
                    } else {
                        format!("“{subject}” was not sent.")
                    };
                    self.send_to_drafts(job, why, cx);
                    if unsure && shown && offline {
                        self.went_offline(cx);
                    }
                }
            }
        }
        cx.notify();
    }

    /// Messages of an account that wait for the connection, for asking before signing out
    /// drops them.
    pub(super) fn waiting_to_send(&self, account: &str) -> usize {
        self.queue
            .iter()
            .filter(|job| job.account == account)
            .count()
    }

    /// The account stopped taking its sign-in: nothing of it is sent meanwhile, and what
    /// waited stays on disk (`Job::left_behind`) for when it is signed in again. What could
    /// not be kept stays in memory, for quitting to warn about.
    pub(super) fn hold_sends(&mut self, account: &str) {
        self.steps.forget(account);
        let held = |job: &Arc<Job>| job.account == account;
        let unkept: Vec<_> = self
            .sends
            .iter()
            .chain(&self.queue)
            .filter(|job| held(job) && !job.kept() && job.stage() == Stage::Held)
            .cloned()
            .collect();
        self.queue.retain(|job| !held(job));
        self.sends.retain(|job| !held(job));
        for job in unkept {
            if !self.stranded.iter().any(|known| Arc::ptr_eq(known, &job)) {
                self.stranded.push(job);
            }
        }
    }

    /// Forgets an account's unsent mail when the user signs out of it: whoever signs in
    /// next must not send it.
    pub(super) fn abandon_sends(&mut self, account: &str) {
        self.queue.retain(|job| job.account != account);
        self.stranded.retain(|job| job.account != account);
        self.homeless.remove(account);
        self.steps.forget(account);
        self.sends.retain(|job| {
            let keep = job.account != account;
            if !keep {
                job.forget();
            }
            keep
        });
    }

    /// A notice that stays for its time: what it says happened to the user's mail.
    pub(super) fn tell(&mut self, text: impl Into<SharedString>) {
        let text = text.into();
        self.must_show = Some(text.clone());
        self.notice = Some(text);
    }

    /// A message that is not to be sent by itself goes to Drafts, and `why` says so.
    fn send_to_drafts(&self, job: Arc<Job>, why: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let saving = job.clone();
            let saved = cx
                .background_executor()
                .spawn(async move { saving.to_drafts() })
                .await;
            this.update(cx, |this, cx| {
                let said = if saved {
                    format!("{why} It is in Drafts.")
                } else if job.kept() {
                    format!("{why} It is kept on this Mac.")
                } else {
                    this.stranded.push(job);
                    format!("{why} It could not be saved anywhere, so quitting loses it.")
                };
                this.tell(said);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A message found in the store: quit, a crash or a switch of accounts got in its way.
    pub(super) fn pick_up_send(&mut self, job: Arc<Job>, cx: &mut Context<Self>) {
        let subject = short(&job.subject(), 40);
        match job.stage() {
            Stage::Held => {
                self.sends.push(job.clone());
                self.send_job(job, cx);
            }
            Stage::Out | Stage::Unsure => self.send_to_drafts(
                job,
                format!("“{subject}” may already be sent: check Sent before sending it again."),
                cx,
            ),
            Stage::Refused => self.send_to_drafts(job, format!("“{subject}” was not sent."), cx),
            Stage::Writing | Stage::Unscheduled => {
                let what = if subject.trim().is_empty() {
                    "A message".to_string()
                } else {
                    format!("“{subject}”")
                };
                let why = if job.stage() == Stage::Writing {
                    format!("{what} was being written when Nimbra quit.")
                } else {
                    format!("{what} was not scheduled before Nimbra quit.")
                };
                self.send_to_drafts(job, why, cx);
            }
            Stage::Discarded => {
                cx.background_executor()
                    .spawn(async move { job.discard() })
                    .detach();
            }
            Stage::Over => job.forget(),
        }
    }

    /// Quitting: what was sent goes now, undo delay or not; what is being written goes to
    /// Drafts; and what is on its way is waited for.
    pub(super) fn finish_before_quit(&mut self, cx: &mut Context<Self>) {
        let deadline = std::time::Instant::now() + QUIT_WAIT;
        let leaving = self.leaving(cx);
        let written = crate::store::flush(std::time::Instant::now());
        if !leaving.is_empty() || !written {
            // Nothing on screen can answer while this waits.
            cx.hide();
        }
        if !leaving.is_empty() {
            leaving.finish(deadline);
        }
        // What was handed to the writers, the settings and the records, is on disk first.
        if !crate::settings::flush() {
            eprintln!("nimbra: the settings may not be saved");
        }
        let at_least = std::time::Instant::now() + Duration::from_secs(5);
        if !crate::store::flush(deadline.max(at_least)) {
            eprintln!("nimbra: some of the mail store may not be saved");
        }
    }

    /// Whether an account is still signed in here: nothing is written down for one that is
    /// not, as its store was deleted.
    pub(super) fn keeps(&self, account: &str) -> bool {
        self.settings.accounts.iter().any(|a| a.id == account)
            || (self.connection.is_some() && account == self.account_id)
    }

    /// Whether quitting would lose mail or changes that could not be written down.
    pub(super) fn would_lose(&self, cx: &App) -> bool {
        let unkeepable = |account: &str| {
            self.store_of(account).is_none() && crate::store::Store::unreadable(account)
        };
        let sending = !self.stranded.is_empty()
            || self
                .sends
                .iter()
                .any(|job| !job.kept() && unkeepable(&job.account));
        let writing = self
            .compose
            .iter()
            .chain(
                self.parked
                    .iter()
                    .filter(|(kind, _)| *kind == Parked::Closing)
                    .map(|(_, composer)| composer),
            )
            .any(|composer| {
                let account = composer.account.as_deref().unwrap_or(&self.account_id);
                composer.dirty() && unkeepable(account) && composer.has_content(cx)
            });
        let changing = !self.outbox.changes.is_empty() && unkeepable(&self.account_id);
        sending || writing || changing || !self.homeless.is_empty()
    }

    pub(super) fn store_of(&self, account: &str) -> Option<Arc<crate::store::Store>> {
        if account == self.account_id {
            self.store.clone()
        } else {
            self.other_stores.get(account).cloned()
        }
    }

    /// Everything that quitting has to finish or write down.
    fn leaving(&self, cx: &App) -> Leaving {
        let sends = self
            .sends
            .iter()
            .map(|job| {
                let composer = self
                    .parked
                    .iter()
                    .find(|(_, c)| Some(c.serial) == job.composer)
                    .map(|(_, c)| c);
                if let Some(composer) = composer {
                    job.set_draft(composer.draft_id.clone());
                }
                let saved = composer.map(|c| c.saved_as.clone()).unwrap_or_default();
                (job.clone(), saved)
            })
            .collect();

        let closing = self
            .parked
            .iter()
            .filter(|(kind, _)| *kind == Parked::Closing)
            .map(|(_, composer)| composer);
        let mut unfinished = Vec::new();
        for composer in self.compose.iter().chain(closing) {
            let at = match composer.after_save {
                Some(After::Schedule(at)) => Some(at),
                _ => None,
            };
            let unsaved = composer.saving || (composer.dirty() && composer.has_content(cx));
            if at.is_none() && !unsaved {
                continue;
            }
            let account = composer.account.clone().unwrap_or(self.account_id.clone());
            let client = self.client_for(composer.account.as_deref());
            let Some(client) = client.filter(|_| self.keeps(&account)) else {
                continue;
            };
            let outgoing = composer.outgoing(cx);
            let later = at.map(|at_ms| {
                let item = Scheduled {
                    draft_id: String::new(), // known once it is saved
                    at_ms,
                    subject: outgoing.subject.clone(),
                    account: composer.account.clone(),
                    out: false,
                };
                (self.account_id.clone(), self.store.clone(), item)
            });
            let stage = match at {
                Some(_) => Stage::Unscheduled,
                None => Stage::Writing,
            };
            let mail = QueuedMail {
                draft_id: composer.draft_id.clone(),
                outgoing,
            };
            let store = self.store_of(&account);
            match Job::unfinished(account, composer.serial, client, store, mail, stage) {
                Ok(job) => unfinished.push(leaving::Unfinished {
                    job,
                    edits: composer.edits,
                    saved: composer.saved_as.clone(),
                    later,
                }),
                Err(error) => eprintln!("nimbra: could not keep what was being written: {error}"),
            }
        }

        // Discarded, with the time to undo that not over yet: quitting ends it.
        for (kind, composer) in &self.parked {
            if *kind != Parked::Discarded || (composer.draft_id.is_none() && !composer.saving) {
                continue;
            }
            let account = composer.account.clone().unwrap_or(self.account_id.clone());
            let client = self.client_for(composer.account.as_deref());
            let Some(client) = client.filter(|_| self.keeps(&account)) else {
                continue;
            };
            let mail = QueuedMail {
                draft_id: composer.draft_id.clone(),
                outgoing: composer.outgoing(cx),
            };
            let store = self.store_of(&account);
            match Job::unfinished(
                account,
                composer.serial,
                client,
                store,
                mail,
                Stage::Discarded,
            ) {
                Ok(job) => unfinished.push(leaving::Unfinished {
                    job,
                    edits: composer.edits,
                    saved: composer.saved_as.clone(),
                    later: None,
                }),
                Err(error) => eprintln!("nimbra: could not note the discarded draft: {error}"),
            }
        }

        let mut changes: Vec<leaving::Changes> = Vec::new();
        let mut of = |account: &str| {
            if let Some(at) = changes.iter().position(|c| c.account == account) {
                return at;
            }
            changes.push(leaving::Changes {
                account: account.to_string(),
                store: self.store_of(account),
                unsaved: Vec::new(),
                steps: Vec::new(),
            });
            changes.len() - 1
        };
        let steps: Vec<_> = self
            .steps
            .all()
            .into_iter()
            .filter(|step| self.keeps(&step.account))
            .map(|step| (of(&step.account), step))
            .collect();
        // Not in the store yet: it has none, or what is here waits for its queue to be read
        // (`outbox_read`), and goes after that.
        let queued = ((self.store.is_none() || self.outbox_pending.is_some())
            && !self.outbox.changes.is_empty()
            && self.keeps(&self.account_id))
        .then(|| of(&self.account_id));
        for (at, step) in steps {
            changes[at].steps.push(step);
        }
        if let Some(at) = queued {
            changes[at].unsaved = self.outbox.changes.clone();
        }

        Leaving {
            sends,
            unfinished,
            changes,
            saves: self.saves.clone(),
            writes: self.writes.clone(),
        }
    }

    /// The times offered by Send later, with their labels.
    pub(super) fn send_later_choices() -> Vec<(String, i64)> {
        use chrono::{Datelike, Duration as Days, Local, TimeZone, Timelike, Weekday};
        let now = Local::now();
        let at = |date: chrono::NaiveDate, hour: u32| {
            Local
                .from_local_datetime(&date.and_hms_opt(hour, 0, 0).unwrap_or_default())
                .single()
                .map(|t| t.timestamp_millis())
        };
        let mut choices = Vec::new();
        let hour = (now + Days::hours(1)).with_second(0).unwrap_or(now);
        choices.push((
            format!("In an hour · {}", hour.format("%H:%M")),
            hour.timestamp_millis(),
        ));
        if now.hour() < 17
            && let Some(ms) = at(now.date_naive(), 18)
        {
            choices.push(("This evening · 18:00".into(), ms));
        }
        let tomorrow = now.date_naive() + Days::days(1);
        if let Some(ms) = at(tomorrow, 8) {
            choices.push((
                format!("Tomorrow morning · {} 8:00", tomorrow.format("%a")),
                ms,
            ));
        }
        let mut monday = now.date_naive() + Days::days(1);
        while monday.weekday() != Weekday::Mon {
            monday += Days::days(1);
        }
        if monday != tomorrow
            && let Some(ms) = at(monday, 8)
        {
            choices.push((
                format!("Monday morning · {} 8:00", monday.format("%b %-d")),
                ms,
            ));
        }
        choices
    }

    /// Send later: saves the draft, then sends it from this Mac at `at`.
    pub(super) fn schedule_send(&mut self, at: i64, cx: &mut Context<Self>) {
        let demo = self.connection.is_none();
        let nowhere_to_keep = self.store.is_none();
        let Some(composer) = &mut self.compose else {
            return;
        };
        if demo {
            composer.error = Some("Sample mode: nothing is scheduled.".into());
            cx.notify();
            return;
        }
        if nowhere_to_keep {
            composer.error = Some(STORE_UNAVAILABLE.into());
            cx.notify();
            return;
        }
        if composer.moving_to.is_some() {
            composer.error = Some(MOVING_ATTACHMENTS.into());
            cx.notify();
            return;
        }
        let outgoing = composer.outgoing(cx);
        if let Err(error) = outgoing.recipients() {
            composer.error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        let Some(mut composer) = self.compose.take() else {
            return;
        };
        composer.suggest = None;
        composer.after_save = Some(After::Schedule(at));
        let (serial, saving) = (composer.serial, composer.saving);
        self.parked.push((Parked::Closing, composer));
        if !saving {
            self.save_composer(serial, true, cx);
        }
        self.pending_focus = Some(self.focus.clone());
        cx.notify();
    }

    /// Puts a message in the send-later list of the account shown.
    pub(super) fn add_scheduled(&mut self, item: Scheduled, cx: &mut Context<Self>) {
        let (owner, store) = (self.account_id.clone(), self.store.clone());
        let (at, subject) = (item.at_ms, short(&item.subject, 40));
        if let Some(store) = store {
            let adding = scheduled::add_soon(&store, item);
            cx.spawn(async move |this, cx| {
                let added = adding.await.ok().and_then(Result::ok);
                this.update(cx, |this, cx| {
                    this.scheduled_added(&owner, at, &subject, added, cx)
                })
                .ok();
            })
            .detach();
            return;
        }
        // Just switched to, and its store is still opening.
        cx.spawn(async move |this, cx| {
            let key = owner.clone();
            let added = cx
                .background_executor()
                .spawn(async move {
                    let store = crate::store::Store::open_quietly(&key)?;
                    scheduled::add(&store, item)
                })
                .await;
            this.update(cx, |this, cx| {
                this.scheduled_added(&owner, at, &subject, added.ok(), cx)
            })
            .ok();
        })
        .detach();
    }

    fn scheduled_added(
        &mut self,
        owner: &str,
        at: i64,
        subject: &str,
        list: Option<Vec<Scheduled>>,
        cx: &mut Context<Self>,
    ) {
        let Some(list) = list else {
            self.tell(format!("“{subject}” was not scheduled. It is in Drafts."));
            cx.notify();
            return;
        };
        self.notice = Some(format!("Scheduled for {}.", long_date(at)).into());
        if self.account_id == owner {
            self.scheduled = list;
            self.watch_schedule(cx);
        }
        cx.notify();
    }

    /// Checks for scheduled messages and snoozes that are due: when the next one is, at
    /// least every 20 seconds (the clock can be changed), for as long as there are any.
    pub(super) fn watch_schedule(&mut self, cx: &mut Context<Self>) {
        self.send_due(cx);
        self.wake_due(cx);
        if self.schedule_watch {
            return;
        }
        self.schedule_watch = true;
        cx.spawn(async move |this, cx| {
            while let Ok(Some(wait)) = this.update(cx, |this, _| this.next_due_in()) {
                cx.background_executor().timer(wait).await;
                if this
                    .update(cx, |this, cx| {
                        this.send_due(cx);
                        this.wake_due(cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// How long until the next scheduled message or snooze is due; `None`, and the watch
    /// ends, once there are none.
    fn next_due_in(&mut self) -> Option<Duration> {
        const AT_MOST: Duration = Duration::from_secs(20);
        let next = self
            .scheduled
            .iter()
            .map(|item| item.at_ms)
            .chain(self.snoozed.iter().map(|snooze| snooze.until_ms))
            .min();
        let Some(next) = next else {
            self.schedule_watch = false;
            return None;
        };
        let now = chrono::Utc::now().timestamp_millis();
        // One already due waits for its connection, or is on its way: checked again later.
        Some(match u64::try_from(next - now) {
            Ok(ms) if ms > 0 => Duration::from_millis(ms).min(AT_MOST),
            _ => AT_MOST,
        })
    }

    fn send_due(&mut self, cx: &mut Context<Self>) {
        let (Some(store), Some(_)) = (self.store.clone(), &self.connection) else {
            return;
        };
        let now = chrono::Utc::now().timestamp_millis();
        let due: Vec<Scheduled> = self
            .scheduled
            .iter()
            .filter(|s| s.at_ms <= now && !self.sending_scheduled.contains(&s.draft_id))
            .cloned()
            .collect();
        for item in due {
            // Sent from the draft's own account.
            let Some(client) = self.client_for(item.account.as_deref()) else {
                continue;
            };
            self.sending_scheduled.insert(item.draft_id.clone());
            let (owner, store) = (self.account_id.clone(), store.clone());
            let working = self.writes.begin();
            cx.spawn(async move |this, cx| {
                let id = item.draft_id.clone();
                let (outcome, list) = cx
                    .background_executor()
                    .spawn(async move {
                        let _working = working;
                        let outcome = scheduled::send(&store, &*client, &id, true);
                        (outcome, scheduled::list(&store))
                    })
                    .await;
                this.update(cx, |this, cx| {
                    this.sending_scheduled.remove(&item.draft_id);
                    // Whose list it was may no longer be the one shown.
                    if this.account_id == owner {
                        this.scheduled = list;
                    }
                    let subject = short(&item.subject, 40);
                    match &outcome {
                        Some(Outcome::Sent) => {
                            this.notice = Some(format!("Scheduled message sent: {subject}").into());
                        }
                        Some(Outcome::Gone) => this.tell(format!(
                            "“{subject}” is no longer in Drafts: check Sent to see if it went out."
                        )),
                        Some(Outcome::Unsure) => this.tell(format!(
                            "“{subject}” may already be sent: check Sent before sending it again."
                        )),
                        // Offline and the like: tried again on the next check.
                        None => {}
                    }
                    if outcome.is_some() && matches!(this.mailbox, View::Sent | View::Drafts) {
                        this.refresh(cx);
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    /// Sends what was written, after a few seconds in which it can be undone. Only ever
    /// runs from the Send button or its shortcut.
    pub(super) fn send_mail(&mut self, cx: &mut Context<Self>) {
        let demo = self.is_demo();
        let from = self.compose.as_ref().and_then(|c| c.account.clone());
        let client = self.client_for(from.as_deref());
        let account = from.unwrap_or_else(|| self.account_id.clone());
        let store = self.store.clone().filter(|_| account == self.account_id);
        let Some(composer) = &mut self.compose else {
            return;
        };
        if composer.sending {
            return;
        }
        if composer.moving_to.is_some() {
            composer.error = Some(MOVING_ATTACHMENTS.into());
            cx.notify();
            return;
        }
        let outgoing = composer.outgoing(cx);
        if let Err(error) = outgoing.recipients() {
            composer.error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        if outgoing.subject.trim().is_empty()
            && outgoing.body.trim().is_empty()
            && outgoing.files.is_empty()
        {
            composer.error = Some("Write a subject or a message first.".into());
            cx.notify();
            return;
        }
        if outgoing.attachment_size() > mail::ATTACHMENT_LIMIT {
            composer.error = Some("Attachments can be 25 MB in total.".into());
            cx.notify();
            return;
        }
        if demo {
            // Sample data: nothing leaves this computer.
            self.compose = None;
            self.pending_focus = Some(self.focus.clone());
            self.notice = Some("Sample mode: the message was not sent anywhere.".into());
            cx.notify();
            return;
        }
        let Some(client) = client else {
            composer.error = Some("That account is not connected. Try again in a moment.".into());
            cx.notify();
            return;
        };
        let mail = QueuedMail {
            draft_id: composer.draft_id.clone(),
            outgoing,
        };
        let job = match Job::new(account, composer.serial, client, store, mail) {
            Ok(job) => job,
            Err(error) => {
                composer.error = Some(format!("Not sent: {error}").into());
                cx.notify();
                return;
            }
        };
        let Some(mut composer) = self.compose.take() else {
            return;
        };
        self.sends.push(job.clone());
        cx.background_executor()
            .spawn(async move { job.keep_asking() })
            .detach();
        composer.suggest = None;
        composer.error = None;
        let serial = composer.serial;
        self.parked.push((Parked::Sending, composer));
        self.pending_focus = Some(self.focus.clone());
        // Settings › Composing: how long a sent message can still be taken back.
        let hold = Duration::from_secs(self.settings.undo_send.min(60) as u64);
        if hold.is_zero() {
            self.finish_parked(serial, cx);
        } else {
            self.offer_undo("Sending…", cx);
            self.finish_parked_later(serial, hold, cx);
        }
        cx.notify();
    }
}

/// For an account that is not shown: wakes its due snoozes (back to the inbox, unread) and
/// sends its due scheduled messages, as kept in its own store. One set from All inboxes can
/// be another account's: it goes through that account's connection. Runs off the main thread.
/// Gives back the snoozes it brought back, to notify about.
pub(super) fn carry_out_due(
    account: &str,
    store: &crate::store::Store,
    connected: &HashMap<String, Arc<mail::Client>>,
    writes: &Underway,
) -> Vec<Snooze> {
    let now = chrono::Utc::now().timestamp_millis();
    let mut back = Vec::new();
    let client_of = |whose: Option<&str>| connected.get(whose.unwrap_or(account));
    for snooze in store.load::<Vec<Snooze>>("snoozed").unwrap_or_default() {
        if snooze.until_ms > now {
            continue;
        }
        let Some(client) = client_of(snooze.mail.account.as_deref()) else {
            continue; // that account is not connected: later
        };
        let _working = writes.begin();
        // Brought back, or not needed any more: either way it is done.
        let woken = super::threads::wake(
            &**client,
            &snooze.thread_id,
            snooze.since_ms,
            snooze.no_reply,
        );
        let done = woken.as_ref().map_or_else(mail::is_gone, |_| true);
        // The list as it is by now: the account may have been opened and changed meanwhile.
        if done
            && store
                .update("snoozed", |list: &mut Vec<Snooze>| {
                    list.retain(|s| {
                        s.thread_id != snooze.thread_id || s.until_ms != snooze.until_ms
                    });
                })
                .is_err()
        {
            eprintln!("nimbra: could not save the snoozed list");
        }
        if matches!(woken, Ok(true)) {
            back.push(snooze);
        }
    }
    for item in scheduled::list(store) {
        if item.at_ms > now {
            continue;
        }
        if let Some(client) = client_of(item.account.as_deref()) {
            let _working = writes.begin();
            // One that may have gone out waits to be told about when its account is shown.
            scheduled::send(store, &**client, &item.draft_id, false);
        }
    }
    back
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(id: &str) -> (String, mail::Change) {
        (id.to_string(), mail::Change::Archive)
    }

    #[test]
    fn the_queue_read_as_the_store_opens_loses_what_was_caught_up_and_keeps_the_rest() {
        let read = Outbox {
            changes: vec![change("a"), change("b"), change("c")],
            ..Outbox::default()
        };
        let merged = with_caught_up(read, 2, vec![change("d")]);
        let ids: Vec<&str> = merged.changes.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["c", "d"]);
    }

    #[test]
    fn a_queue_handed_to_the_writer_is_on_disk_once_quitting_has_flushed() {
        let (store, release) = crate::store::Store::temp_held();
        let outbox = Outbox {
            changes: vec![change("a"), change("b")],
            ..Outbox::default()
        };
        store.save_soon("outbox", outbox.clone());
        drop(release);
        // What a later launch reads, through a store of its own.
        let reader = crate::store::Store::temp_like(&store);
        assert!(crate::store::flush_store(
            &store,
            std::time::Instant::now() + Duration::from_secs(30)
        ));
        let back: Outbox = reader.load("outbox").unwrap();
        assert_eq!(back.changes.len(), 2);
    }
}
