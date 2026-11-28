//! Sending: a message on its way out (the undo delay, the queue, retries and what happens
//! when Gmail cannot be reached), mail scheduled for later, and what must finish before the
//! app quits.

use super::*;

use crate::outbox::{CaughtUp, catch_up, with_caught_up};

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
                crate::integration::pause(
                    cx.background_executor().timer(crate::outbox::RETRY_EVERY),
                )
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
                    this.drop_caught_up(&account, store.as_ref(), changed, false, cx);
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

    /// Lists `account`'s Sent again in the background, after a message went from it, and
    /// keeps what it finds for when Sent is opened (from memory, or from its store after a
    /// restart). Nothing is shown if it fails: Sent is listed when it is opened anyway.
    fn refresh_sent(&mut self, account: String, cx: &mut Context<Self>) {
        if self.is_demo() {
            return;
        }
        let shown = account == self.account_id;
        let store = if shown {
            self.store.clone()
        } else {
            self.other_stores.get(&account).cloned()
        };
        let (Some(client), Some(store)) = (self.client_for(Some(&account)), store) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let listed =
                cx.background_executor()
                    .spawn(async move {
                        nimbra_core::listing::refresh(&*client, &store, View::Sent, None)
                    })
                    .await;
            let Ok(listed) = listed else {
                return;
            };
            this.update(cx, |this, _| {
                // Kept for the account on screen; another account's is in its store.
                if account != this.account_id {
                    return;
                }
                let fresh: Vec<Mail> = listed
                    .rows
                    .into_iter()
                    .map(|row| {
                        let mut mail = Mail::from(row);
                        mail.refresh_time();
                        mail
                    })
                    .collect();
                let kept = this.cache.get(&View::Sent).cloned().unwrap_or_default();
                this.cache
                    .insert(View::Sent, crate::data::merge(&kept, fresh));
            })
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
                match serial {
                    // Its sending pill says it, if it's still there.
                    Some(_) if self.show_sent(cx) => {}
                    Some(_) => self.notice = Some("Message sent.".into()),
                    None => {
                        self.notice = Some(format!("Sent “{subject}”, which was waiting.").into())
                    }
                }
                if matches!(self.mailbox.base(), View::Sent | View::Drafts) {
                    self.refresh(cx);
                } else {
                    // Sent is listed again now, so it shows the message when it is opened.
                    self.refresh_sent(job.account.clone(), cx);
                }
                cx.notify();
                return;
            }
            Err(error) => error,
        };
        // Not sent: the sending pill makes way for what happened instead.
        if self.undo_sending {
            self.close_undo(cx);
        }
        let offline = error.is::<mail::Offline>();
        let unsure = crate::outbox::failed(&error) == crate::outbox::Failed::Unsure;
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

    /// Forgets an account's unsent mail: whoever signs in next must not send it.
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
        use crate::outbox::PickUp;
        let subject = short(&job.subject(), 40);
        let what = if subject.trim().is_empty() {
            "A message".to_string()
        } else {
            format!("“{subject}”")
        };
        match crate::outbox::pick_up(job.stage()) {
            PickUp::Send => {
                self.sends.push(job.clone());
                self.send_job(job, cx);
            }
            PickUp::MaybeSent => self.send_to_drafts(
                job,
                format!("“{subject}” may already be sent: check Sent before sending it again."),
                cx,
            ),
            PickUp::Refused => self.send_to_drafts(job, format!("“{subject}” was not sent."), cx),
            PickUp::Writing => self.send_to_drafts(
                job,
                format!("{what} was being written when Nimbra quit."),
                cx,
            ),
            PickUp::Unscheduled => self.send_to_drafts(
                job,
                format!("{what} was not scheduled before Nimbra quit."),
                cx,
            ),
            PickUp::Discard => {
                cx.background_executor()
                    .spawn(async move { job.discard() })
                    .detach();
            }
            PickUp::Forget => job.forget(),
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

    /// The times offered by Send later (`scheduled::later_times`), with their labels.
    pub(super) fn send_later_choices() -> Vec<(String, i64)> {
        use crate::scheduled::Later;
        crate::scheduled::later_times(chrono::Local::now())
            .into_iter()
            .map(|(kind, when)| {
                let label = match kind {
                    Later::InAnHour => {
                        format!(
                            "In an hour · {}",
                            crate::when::local(when, crate::when::TIME)
                        )
                    }
                    Later::ThisEvening => {
                        format!(
                            "This evening · {}",
                            crate::when::local(when, crate::when::TIME)
                        )
                    }
                    Later::TomorrowMorning => format!(
                        "Tomorrow morning · {}",
                        crate::when::local(when, crate::when::WEEKDAY_TIME)
                    ),
                    Later::MondayMorning => format!(
                        "Monday morning · {}, {}",
                        crate::when::local(when, crate::when::DAY_MONTH),
                        crate::when::local(when, crate::when::TIME)
                    ),
                };
                (label, when.timestamp_millis())
            })
            .collect()
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
                let added = matches!(adding.await, Ok(Ok(())));
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
                    crate::store::Store::open_quietly(&key)
                        .and_then(|store| scheduled::add(&store, item))
                        .is_ok()
                })
                .await;
            this.update(cx, |this, cx| {
                this.scheduled_added(&owner, at, &subject, added, cx)
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
        added: bool,
        cx: &mut Context<Self>,
    ) {
        if added {
            self.notice = Some(format!("Scheduled for {}.", long_date(at)).into());
        } else {
            self.tell(format!("“{subject}” was not scheduled. It is in Drafts."));
        }
        if added && self.account_id == owner {
            self.read_scheduled(true, cx);
        }
        cx.notify();
    }

    /// Takes the send-later list as it now is in the store, read off the main thread, and
    /// then watches it if `watch`.
    fn read_scheduled(&mut self, watch: bool, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let reading = store.clone();
            let list = cx
                .background_executor()
                .spawn(async move { scheduled::list(&reading) })
                .await;
            this.update(cx, |this, cx| {
                let same = this
                    .store
                    .as_ref()
                    .is_some_and(|now| Arc::ptr_eq(now, &store));
                if this.account_gen == generation && same {
                    this.scheduled = list;
                    if watch {
                        this.watch_schedule(cx);
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Checks for scheduled messages that are due, every 20 seconds while Nimbra runs.
    pub(super) fn watch_schedule(&mut self, cx: &mut Context<Self>) {
        self.send_due(cx);
        self.wake_due(cx);
        if self.schedule_watch {
            return;
        }
        self.schedule_watch = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(20))
                    .await;
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
                let outcome = cx
                    .background_executor()
                    .spawn(async move {
                        let _working = working;
                        scheduled::send(&store, &*client, &id)
                    })
                    .await;
                this.update(cx, |this, cx| {
                    this.sending_scheduled.remove(&item.draft_id);
                    // Whose list it was may no longer be the one shown.
                    if this.account_id == owner {
                        this.read_scheduled(false, cx);
                    }
                    let subject = short(&item.subject, 40);
                    match &outcome {
                        Some(Outcome::Sent) => {
                            this.notice = Some(format!("Scheduled message sent: {subject}").into());
                        }
                        Some(Outcome::Gone) => this.tell(format!(
                            "“{subject}” is no longer in Drafts: check Sent to see if it went out."
                        )),
                        // Offline and the like: tried again on the next check.
                        None => {}
                    }
                    if outcome.is_some() && matches!(this.mailbox.base(), View::Sent | View::Drafts)
                    {
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
        if let Err(not_ready) = crate::outbox::ready(&outgoing) {
            composer.error = Some(match not_ready {
                crate::outbox::NotReady::Recipients(error) => error.into(),
                crate::outbox::NotReady::Empty => "Write a subject or a message first.".into(),
                crate::outbox::NotReady::TooLarge => "Attachments can be 25 MB in total.".into(),
            });
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
        let hold = crate::outbox::undo_delay(self.settings.undo_send);
        if hold.is_zero() {
            self.finish_parked(serial, cx);
        } else {
            self.offer_undo_for("Sending…", Some(hold), cx);
            self.finish_parked_later(serial, hold, cx);
        }
        cx.notify();
    }
}
