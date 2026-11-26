//! What you do to a thread: done, trash, star, read and unread, snooze, delete for ever, and
//! undoing it. Also picking several threads to act on at once.

use super::*;

/// A thread put away until later (snooze), or until later unless someone replies
/// (a reminder). Kept on this Mac: Gmail's API has no snooze.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct Snooze {
    pub(super) thread_id: String,
    /// When it comes back (or, for a reminder, when to check for a reply).
    pub(super) until_ms: i64,
    /// Only come back if nobody has written since.
    pub(super) no_reply: bool,
    /// The time of the thread's latest message when it was put away.
    pub(super) since_ms: i64,
    /// The row, to list it under Snoozed.
    pub(super) mail: Mail,
}

/// A time to snooze until.
#[derive(Clone)]
pub(super) struct SnoozeChoice {
    pub(super) icon: &'static str,
    pub(super) label: String,
    pub(super) hint: String,
    pub(super) until_ms: i64,
    pub(super) no_reply: bool,
}

impl Nimbra {
    /// The thread left the undo history by itself (back with a newer message, or Gmail
    /// refused the change).
    pub(super) fn forget_cleared(&mut self, position: usize, cx: &mut Context<Self>) {
        // The Undo pill is for the last one: it must not undo the one before instead.
        if position + 1 == self.history.len() {
            self.close_undo(cx);
        }
        crate::listing::forget(&mut self.history, &mut self.history_groups, position);
    }

    /// Deletes threads for good (after a yes). If the sign-in cannot do that yet, asks to
    /// give Nimbra that permission first.
    pub(super) fn delete_forever(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        let Some(client) = self.connection.clone() else {
            return;
        };
        // Out of the list now; there is no undo for this.
        for (index, mail) in self.mails.iter().enumerate() {
            if mail.id.as_ref().is_some_and(|id| ids.contains(id)) {
                self.done[index] = true;
            }
        }
        // Asked from a message window, it need not be what is picked or shown here.
        self.picked.retain(|i| !self.done[*i]);
        if self.picked.len() < 2 {
            self.picked.clear();
        }
        let visible = self.visible();
        if !visible.contains(&self.selected) {
            self.selected = visible.first().copied().unwrap_or(0);
        }
        self.load_selected(cx);
        cx.notify();
        let working = self.writes.begin();
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let list = ids.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _working = working;
                    let mut deleted = 0;
                    for id in &list {
                        client.delete_thread(id)?;
                        deleted += 1;
                    }
                    anyhow::Ok(deleted)
                })
                .await;
            this.update(cx, |this, cx| {
                // Another account is shown now: its list, sign-in and permissions are not
                // the ones these threads were deleted with.
                if this.account_gen != generation {
                    return;
                }
                match result {
                    Ok(deleted) => {
                        this.notice = Some(
                            format!(
                                "Deleted {deleted} conversation{} permanently.",
                                if deleted == 1 { "" } else { "s" }
                            )
                            .into(),
                        );
                        this.persist(cx);
                    }
                    Err(error) => {
                        // Put back what was not deleted.
                        this.refresh(cx);
                        if error.is::<gmail::NeedsFullAccess>() {
                            this.confirm = Some(Ask::GrantFullAccess(ids));
                        } else {
                            this.fail(error, cx);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A click on a row: plain opens it, a double-click opens it in its own window; ⌘ adds or
    /// removes it from the picked rows; Shift picks everything from the open row to this one.
    pub(super) fn click_row(
        &mut self,
        i: usize,
        modifiers: gpui::Modifiers,
        clicks: usize,
        cx: &mut Context<Self>,
    ) {
        if clicks >= 2 && !modifiers.platform && !modifiers.shift && !modifiers.control {
            self.picked.clear();
            self.open(i);
            self.open_selected(cx);
            self.open_message_window(i, cx);
            return;
        }
        if modifiers.platform || modifiers.control {
            if self.picked.is_empty() && self.visible().contains(&self.selected) {
                self.picked.insert(self.selected);
            }
            if !self.picked.remove(&i) {
                self.picked.insert(i);
            }
            if self.picked.len() == 1 {
                let only = *self.picked.iter().next().unwrap_or(&i);
                self.picked.clear();
                self.open(only);
                self.open_selected(cx);
            }
        } else if modifiers.shift {
            let visible = self.visible();
            let (Some(a), Some(b)) = (
                visible.iter().position(|x| *x == self.selected),
                visible.iter().position(|x| *x == i),
            ) else {
                return;
            };
            self.picked = visible[a.min(b)..=a.max(b)].iter().copied().collect();
        } else {
            self.picked.clear();
            self.open(i);
            self.open_selected(cx);
        }
        cx.notify();
    }

    /// Takes every picked thread out of the list at once; one Undo brings them all back.
    fn remove_picked(&mut self, change: mail::Change, cx: &mut Context<Self>) {
        let targets = self.targets();
        if targets.is_empty() {
            return;
        }
        for &index in &targets {
            if self.mails[index].unread {
                self.adjust_counts(index, Self::count_change(change));
            }
            self.done[index] = true;
            self.history.push(index);
            if let Some(id) = self.mails[index].id.clone() {
                self.removed_by.insert(id, change);
            }
            self.sync(index, change, true, cx);
        }
        self.history_groups.push(targets.len());
        self.picked.clear();
        self.selected = self.visible().first().copied().unwrap_or(0);
        self.open_selected(cx);
        self.offer_undo(
            match change {
                mail::Change::Trash => "Moved to trash",
                mail::Change::Unarchive | mail::Change::NotSpam => "Moved to inbox",
                mail::Change::Spam => "Reported as spam",
                mail::Change::Untrash => "Restored",
                _ => "Marked done",
            },
            cx,
        );
        cx.notify();
    }

    /// Snoozes the open (or picked) threads: out of the inbox now, back at the chosen time,
    /// unread and with a notification. A reminder only comes back if nobody has replied.
    /// Each is kept with its own account (a thread snoozed from All inboxes may be another's).
    pub(super) fn snooze(&mut self, choice: SnoozeChoice, cx: &mut Context<Self>) {
        if self.connection.is_some() && self.store.is_none() {
            // It would leave the Inbox with nothing to bring it back.
            self.notice = Some(STORE_UNAVAILABLE.into());
            cx.notify();
            return;
        }
        let targets = self.targets();
        let mut new: Vec<(String, Snooze)> = Vec::new();
        for &index in &targets {
            let mail = &self.mails[index];
            let Some(thread_id) = mail.id.clone() else {
                continue;
            };
            new.push((
                self.account_of(mail).to_string(),
                Snooze {
                    thread_id,
                    until_ms: choice.until_ms,
                    no_reply: choice.no_reply,
                    since_ms: mail.timestamp_ms,
                    // The row only: its messages are fetched again when it is opened.
                    mail: Mail {
                        body: Default::default(),
                        earlier: Default::default(),
                        html: None,
                        loaded: false,
                        ..mail.clone()
                    },
                },
            ));
        }
        let owners: std::collections::BTreeSet<String> =
            new.iter().map(|(owner, _)| owner.clone()).collect();
        for owner in owners {
            let theirs: Vec<Snooze> = new
                .iter()
                .filter(|(o, _)| *o == owner)
                .map(|(_, snooze)| snooze.clone())
                .collect();
            self.change_snoozed_of(&owner, move |list| {
                list.retain(|kept| !theirs.iter().any(|s| s.thread_id == kept.thread_id));
                list.extend(theirs.iter().cloned());
            });
        }
        self.watch_schedule(cx);
        let said = if choice.no_reply {
            format!("I’ll bring it back on {} if nobody replies", choice.hint)
        } else {
            format!("Snoozed until {}", choice.hint)
        };
        if inbox_like(self.mailbox) {
            self.remove_selected(mail::Change::Archive, cx);
            // It left the inbox by being snoozed, not marked done: the Undo pill says so,
            // and Undo cancels the snooze.
            self.offer_undo(said, cx);
            cx.notify();
            return;
        } else {
            for &index in &targets {
                // Snoozed from another mailbox: it still leaves the inbox, and stays listed here.
                if self.mails[index].labels.iter().any(|l| l == "INBOX") {
                    self.mails[index].labels.retain(|l| l != "INBOX");
                    self.sync(index, mail::Change::Archive, false, cx);
                }
                // Snoozed again from Snoozed or Reminders: the new time, or off to the other.
                if matches!(self.mailbox, View::Snoozed | View::Reminders) {
                    if (self.mailbox == View::Reminders) == choice.no_reply {
                        self.mails[index].time = snooze_time(choice.until_ms);
                    } else {
                        self.done[index] = true;
                    }
                }
            }
            self.picked.clear();
            if self.done.get(self.selected) == Some(&true) {
                self.selected = self.visible().first().copied().unwrap_or(0);
                self.open_selected(cx);
            }
        }
        self.notice = Some(format!("{said}.").into());
        cx.notify();
    }

    /// Changes the snoozed list here at once, and in the store off the main thread, each as
    /// it is: saving this copy could undo what a background check just took out.
    fn change_snoozed(&mut self, change: impl Fn(&mut Vec<Snooze>) + Send + Sync + 'static) {
        change(&mut self.snoozed);
        if let Some(store) = self.store.clone() {
            self.change_stored_snoozed(store, change);
        }
    }

    /// The same for any account: the shown one's as above, another's in its own store (or,
    /// with none open, the shown one's, which brings it back all the same).
    fn change_snoozed_of(
        &mut self,
        account: &str,
        change: impl Fn(&mut Vec<Snooze>) + Send + Sync + 'static,
    ) {
        match self.other_stores.get(account).cloned() {
            Some(store) if account != self.account_id => {
                if let Some(kept) = self.flow.other_snoozed.get_mut(account) {
                    change(kept);
                }
                self.change_stored_snoozed(store, change);
                self.read_snoozed_elsewhere();
            }
            _ => self.change_snoozed(change),
        }
    }

    /// A store's snoozed list, changed off the main thread. Quitting waits for it.
    fn change_stored_snoozed(
        &self,
        store: Arc<crate::store::Store>,
        change: impl FnOnce(&mut Vec<Snooze>) + Send + 'static,
    ) {
        drop(store.update_later("snoozed", change));
    }

    /// Forgets the snoozes of these threads, whichever account's list has them. Nothing is
    /// written for threads that were not snoozed (every Undo asks).
    pub(super) fn forget_snoozes_of(&mut self, threads: &[(String, String)]) {
        let accounts: std::collections::BTreeSet<&String> =
            threads.iter().map(|(account, _)| account).collect();
        for account in accounts {
            let ids: Vec<String> = threads
                .iter()
                .filter(|(a, _)| a == account)
                .map(|(_, id)| id.clone())
                .collect();
            let listed = |list: &[Snooze]| list.iter().any(|s| ids.contains(&s.thread_id));
            let theirs = if *account == self.account_id {
                listed(&self.snoozed)
            } else {
                self.flow
                    .other_snoozed
                    .get(account)
                    .is_none_or(|list| listed(list))
            };
            let here = *account != self.account_id && listed(&self.snoozed);
            if !theirs && !here {
                continue;
            }
            let account = account.clone();
            if theirs {
                let ids = ids.clone();
                self.change_snoozed_of(&account, move |list| {
                    list.retain(|s| !ids.contains(&s.thread_id))
                });
            }
            // One kept with the shown account from before (snoozed from All inboxes).
            if here {
                self.change_snoozed(move |list| list.retain(|s| !ids.contains(&s.thread_id)));
            }
        }
    }

    /// The shown account's snoozes (or reminders) as rows of Snoozed (or Reminders), with
    /// the time they come back where the time they arrived would be.
    pub(super) fn snoozed_rows(&mut self, reminders: bool) -> Vec<Mail> {
        self.read_snoozed_elsewhere();
        self.snoozed
            .iter()
            .filter(|s| {
                s.mail
                    .account
                    .as_deref()
                    .is_none_or(|a| a == self.account_id)
            })
            // Every account's: one snoozed from All inboxes is kept with its own account.
            .chain(&self.snoozed_elsewhere)
            .filter(|s| s.no_reply == reminders)
            .map(|s| {
                let mut mail = s.mail.clone();
                mail.time = snooze_time(s.until_ms);
                mail
            })
            .collect()
    }

    /// The other signed-in accounts' snoozes, each marked with its account so its row acts
    /// there. From memory: each list is read off the main thread when its store opens, and
    /// kept up to date by the checks that change it there.
    pub(super) fn read_snoozed_elsewhere(&mut self) {
        // The shown account's list is `snoozed`; a copy from before it was shown is old.
        self.flow.other_snoozed.remove(&self.account_id);
        let mut found = Vec::new();
        for account in self.other_stores.keys() {
            if *account == self.account_id || !self.others.contains_key(account) {
                continue;
            }
            let Some(list) = self.flow.other_snoozed.get(account) else {
                continue;
            };
            for snooze in list {
                // Kept with its own account, or with this one from All inboxes (shown here).
                let mut snooze = match snooze.mail.account.as_deref() {
                    Some(owner) if owner == self.account_id => continue,
                    _ => snooze.clone(),
                };
                snooze.mail.account.get_or_insert_with(|| account.clone());
                found.push(snooze);
            }
        }
        self.snoozed_elsewhere = found;
    }

    /// Another account's snoozed list as its store now has it, read off the main thread.
    pub(super) fn took_snoozed(&mut self, account: String, list: Vec<Snooze>) {
        self.flow.other_snoozed.insert(account, list);
        self.read_snoozed_elsewhere();
    }

    /// A reply came in while a thread was snoozed: it is back in the inbox by itself (as
    /// Gmail does), so it is not brought back again later. `fresh` is an inbox's listing.
    pub(super) fn unsnooze_answered(&mut self, fresh: &[Mail]) {
        self.read_snoozed_elsewhere();
        let mut answered = Vec::new();
        for mail in fresh {
            let Some(id) = &mail.id else {
                continue;
            };
            let account = self.account_of(mail);
            let elsewhere = (account != self.account_id)
                .then(|| self.flow.other_snoozed.get(account))
                .flatten();
            let snoozed = self
                .snoozed
                .iter()
                .chain(elsewhere.into_iter().flatten())
                .any(|s| s.thread_id == *id && mail.timestamp_ms > s.since_ms);
            if snoozed {
                answered.push((account.to_string(), id.clone()));
            }
        }
        if !answered.is_empty() {
            self.forget_snoozes_of(&answered);
        }
    }

    /// Brings back snoozed threads whose time has come (checked with the send-later list).
    pub(super) fn wake_due(&mut self, cx: &mut Context<Self>) {
        if self.connection.is_none() {
            return;
        }
        let now = chrono::Utc::now().timestamp_millis();
        let due: Vec<Snooze> = self
            .snoozed
            .iter()
            .filter(|s| s.until_ms <= now && !self.waking.contains(&s.thread_id))
            .cloned()
            .collect();
        for item in due {
            // Woken in its own account (a snooze set from All inboxes may be another one's).
            let Some(client) = self.client_for(item.mail.account.as_deref()) else {
                continue;
            };
            self.waking.insert(item.thread_id.clone());
            let (owner, store) = (self.account_id.clone(), self.store.clone());
            let working = self.writes.begin();
            cx.spawn(async move |this, cx| {
                let (id, since, no_reply) = (item.thread_id.clone(), item.since_ms, item.no_reply);
                let until = item.until_ms;
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let _working = working;
                        let woken = wake(&*client, &id, since, no_reply);
                        // Out of its account's list here, whichever account is shown by now.
                        if woken.as_ref().map_or_else(mail::is_gone, |_| true)
                            && let Some(store) = &store
                            && store
                                .update("snoozed", |list: &mut Vec<Snooze>| {
                                    list.retain(|s| s.thread_id != id || s.until_ms != until);
                                })
                                .is_err()
                        {
                            eprintln!("nimbra: could not save the snoozed list");
                        }
                        woken
                    })
                    .await;
                this.update(cx, |this, cx| {
                    this.waking.remove(&item.thread_id);
                    let done = match &result {
                        Ok(back) => {
                            if *back {
                                let title = if item.no_reply {
                                    "No reply yet"
                                } else {
                                    "Back from snooze"
                                };
                                if this.settings.notifications {
                                    crate::notify::show(
                                        &item.thread_id,
                                        title,
                                        &item.mail.subject,
                                        this.settings.notify_sound,
                                    );
                                }
                                this.notice = Some(
                                    format!("{title}: {}", short(&item.mail.subject, 50)).into(),
                                );
                                if inbox_like(this.mailbox) || this.mailbox == View::Snoozed {
                                    this.refresh(cx);
                                }
                            }
                            true
                        }
                        Err(error) => mail::is_gone(error),
                    };
                    if done && this.account_id == owner {
                        this.snoozed.retain(|s| {
                            s.thread_id != item.thread_id || s.until_ms != item.until_ms
                        });
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    /// The same, for acting on it: it has to still be listed, or a notice says why not.
    /// Whatever a message window does starts here, not from `selected`, which is only what
    /// the main window shows.
    pub(super) fn row_to_act_on(
        &mut self,
        key: &str,
        account: &str,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let row = self.row_of(key, account).filter(|i| !self.done[*i]);
        if row.is_none() {
            self.notice = Some("This conversation is no longer in the open mailbox.".into());
            cx.notify();
        }
        row
    }

    /// `U`, or "Mark as unread" in a menu: the open thread goes back to unread.
    pub(super) fn mark_unread(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        self.set_unread(self.selected, true, cx);
    }

    /// The toolbar button and `U`: flips the open thread between unread and read.
    pub(super) fn toggle_unread(&mut self, cx: &mut Context<Self>) {
        if self.picked.len() >= 2 {
            let targets = self.targets();
            let unread = !targets.iter().all(|i| self.mails[*i].unread);
            for index in targets {
                self.set_unread(index, unread, cx);
            }
            return;
        }
        let index = self.selected;
        if let Some(unread) = self.mails.get(index).map(|m| m.unread) {
            self.set_unread(index, !unread, cx);
        }
    }

    /// Moves the sidebar's unread counts (and the Dock badge) at once, before Gmail's own
    /// numbers come back with the next sync. `delta` is +1 or -1 for one unread thread.
    fn adjust_counts(&mut self, index: usize, delta: i32) {
        let Some(mail) = self.mails.get(index) else {
            return;
        };
        let inbox = mail.labels.iter().any(|l| l == "INBOX")
            || (mail.labels.is_empty() && inbox_like(self.mailbox));
        if !inbox {
            return;
        }
        // `counts` are the shown account's; the others have only their inbox's number.
        if let Some(other) = mail.account.as_deref().filter(|a| *a != self.account_id) {
            if let Some(unread) = self.account_unread.get_mut(other) {
                *unread = (*unread as i32 + delta).max(0) as u32;
            }
            return;
        }
        let mut ids: Vec<String> = mail.labels.clone();
        if !ids.iter().any(|l| l == "INBOX") {
            ids.push("INBOX".into());
        }
        for id in ids {
            if let Some((unread, _)) = self.counts.get_mut(&id) {
                *unread = (*unread as i32 + delta).max(0) as u32;
            }
        }
    }

    /// How a change moves an unread thread's inbox counts: leaving the inbox takes it off.
    fn count_change(change: mail::Change) -> i32 {
        match change {
            mail::Change::Archive | mail::Change::Trash | mail::Change::Spam => -1,
            _ => 0,
        }
    }

    /// Marks a thread read or unread, here and in Gmail.
    pub(super) fn set_unread(&mut self, index: usize, unread: bool, cx: &mut Context<Self>) {
        if index == self.selected {
            self.kept_unread = if unread {
                self.mails.get(index).and_then(|m| m.id.clone())
            } else {
                None
            };
        }
        if self.visible().contains(&index) && self.mails[index].unread != unread {
            self.mails[index].unread = unread;
            self.adjust_counts(index, if unread { 1 } else { -1 });
            let change = if unread {
                mail::Change::MarkUnread
            } else {
                mail::Change::MarkRead
            };
            self.sync(index, change, false, cx);
        }
        cx.notify();
    }

    pub(super) fn open_in_gmail(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        // Thread ids are validated as plain alphanumerics before they get here.
        if let Some(id) = self.mails.get(self.selected).and_then(|m| m.id.as_ref())
            && id.chars().all(|c| c.is_ascii_alphanumeric())
        {
            cx.open_url(&format!("https://mail.google.com/mail/u/0/#all/{id}"));
        }
        cx.notify();
    }

    /// X: the open thread in or out of the picked ones.
    pub(super) fn pick_thread(&mut self, cx: &mut Context<Self>) {
        if !self.visible().contains(&self.selected) {
            return;
        }
        if !self.picked.remove(&self.selected) {
            self.picked.insert(self.selected);
        }
        self.pick_end = None;
        cx.notify();
    }

    /// Shift with Up or Down: picks from the open thread to one further along.
    pub(super) fn pick_more(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.pane == Pane::Sidebar {
            return;
        }
        let visible = self.visible();
        let Some(open) = visible.iter().position(|i| *i == self.selected) else {
            return;
        };
        let end = self
            .pick_end
            .filter(|end| self.picked.contains(end))
            .and_then(|end| visible.iter().position(|i| *i == end))
            .unwrap_or(open);
        let (end, picked) = keyboard::extend(open, end, delta, visible.len());
        self.picked = picked.into_iter().map(|p| visible[p]).collect();
        self.pick_end = Some(visible[end]);
        self.reveal_row.set(true);
        cx.notify();
    }

    /// `E`: archive from the inbox, or move back to the inbox from Done.
    pub(super) fn mark_done(&mut self, cx: &mut Context<Self>) {
        let Some(change) = self.done_change() else {
            return;
        };
        if matches!(self.mailbox, View::Snoozed | View::Reminders) {
            self.forget_snoozes(&self.targets());
        }
        self.remove_selected(change, cx);
    }

    /// Done from a message window. Whether it was done.
    pub(super) fn done_thread(&mut self, index: usize, cx: &mut Context<Self>) -> bool {
        let Some(change) = self.done_change() else {
            self.notice = Some("Done is not available in the mailbox that is open.".into());
            cx.notify();
            return false;
        };
        if matches!(self.mailbox, View::Snoozed | View::Reminders) {
            self.forget_snoozes(&[index]);
        }
        self.remove_thread(index, change, cx);
        true
    }

    /// What Done does in the open mailbox.
    pub(super) fn done_change(&self) -> Option<mail::Change> {
        Some(match self.mailbox {
            view if inbox_like(view) => mail::Change::Archive,
            View::Done => mail::Change::Unarchive,
            View::Spam => mail::Change::NotSpam,
            View::Trash => mail::Change::Untrash,
            // Back to the inbox now, instead of at the snooze time.
            View::Snoozed | View::Reminders => mail::Change::Unarchive,
            _ => return None,
        })
    }

    fn forget_snoozes(&mut self, rows: &[usize]) {
        let threads: Vec<(String, String)> = rows
            .iter()
            .filter_map(|i| {
                let mail = &self.mails[*i];
                Some((self.account_of(mail).to_string(), mail.id.clone()?))
            })
            .collect();
        self.forget_snoozes_of(&threads);
    }

    /// Trash from a message window (in Trash: asks before deleting forever). Whether it
    /// was taken on.
    pub(super) fn trash_thread(&mut self, index: usize, cx: &mut Context<Self>) -> bool {
        if self.mailbox != View::Trash {
            self.remove_thread(index, mail::Change::Trash, cx);
            return true;
        }
        let Some(id) = self.mails[index].id.clone() else {
            return false;
        };
        self.confirm = Some(Ask::DeleteForever(vec![id]));
        cx.notify();
        true
    }

    /// Moves the open thread to Gmail's trash (kept for 30 days, and undoable with `Z`).
    /// Moves the open (or picked) threads to Trash; in Trash itself, offers to delete them
    /// permanently.
    pub(super) fn trash(&mut self, cx: &mut Context<Self>) {
        if self.mailbox == View::Trash {
            let ids: Vec<String> = self
                .targets()
                .iter()
                .filter_map(|i| self.mails[*i].id.clone())
                .collect();
            if !ids.is_empty() {
                self.confirm = Some(Ask::DeleteForever(ids));
                cx.notify();
            }
            return;
        }
        self.remove_selected(mail::Change::Trash, cx);
    }

    /// Takes the open thread out of the list and applies `change` to it in Gmail.
    pub(super) fn remove_selected(&mut self, change: mail::Change, cx: &mut Context<Self>) {
        if self.picked.len() >= 2 {
            self.remove_picked(change, cx);
            return;
        }
        self.picked.clear();
        if !self.visible().contains(&self.selected) {
            return;
        }
        self.remove_row(self.selected, change, cx);
        self.open_selected(cx);
    }

    /// The same from a message window. The main window keeps what it shows and what is
    /// picked; only if it showed this thread does it move on, and the thread it moves to is
    /// not marked read, as nobody opened it.
    pub(super) fn remove_thread(
        &mut self,
        index: usize,
        change: mail::Change,
        cx: &mut Context<Self>,
    ) {
        let shown = index == self.selected;
        self.remove_row(index, change, cx);
        if shown {
            self.load_selected(cx);
        }
    }

    /// Takes one thread off the list and sends the change. Opens nothing.
    fn remove_row(&mut self, index: usize, change: mail::Change, cx: &mut Context<Self>) {
        let visible = self.visible();
        if self.mails[index].unread {
            self.adjust_counts(index, Self::count_change(change));
        }
        self.done[index] = true;
        self.history.push(index);
        self.history_groups.push(1);
        if let Some(id) = self.mails[index].id.clone() {
            self.removed_by.insert(id, change);
        }
        if index == self.selected
            && let Some(pos) = visible.iter().position(|i| *i == index)
            && let Some(next) = visible
                .get(pos + 1)
                .or(pos.checked_sub(1).map(|p| &visible[p]))
        {
            self.selected = *next;
        }
        self.picked.remove(&index);
        if self.picked.len() < 2 {
            self.picked.clear();
        }
        self.sync(index, change, true, cx);
        self.offer_undo(
            match change {
                mail::Change::Trash => "Moved to trash",
                mail::Change::Unarchive | mail::Change::NotSpam => "Moved to inbox",
                mail::Change::Spam => "Reported as spam",
                mail::Change::Untrash => "Restored",
                _ => "Marked done",
            },
            cx,
        );
        cx.notify();
    }

    /// The message being sent or the discarded draft that `Z` would bring back.
    fn held_back(&self) -> Option<u64> {
        if self.compose.is_some() {
            return None;
        }
        self.parked
            .iter()
            .rev()
            .find(|(kind, _)| matches!(kind, Parked::Sending | Parked::Discarded))
            .map(|(_, c)| c.serial)
    }

    /// `Z`: brings back the last thread that was archived, moved or trashed.
    pub(super) fn undo(&mut self, cx: &mut Context<Self>) {
        self.close_undo(cx);
        // A message being sent or a discarded draft comes back first.
        if let Some(serial) = self.held_back() {
            if let Some(at) = self.sends.iter().position(|j| j.composer == Some(serial)) {
                self.sends.remove(at).forget();
            }
            if let Some(composer) = self.composer_mut(serial) {
                composer.error = None;
            }
            self.unpark(serial, cx);
            return;
        }
        // A batch (several threads at once) comes back together.
        let group = self
            .history_groups
            .pop()
            .unwrap_or(1)
            .clamp(1, self.history.len().max(1));
        for _ in 1..group {
            if let Some(index) = self.history.pop() {
                self.undo_one(index, cx);
            }
        }
        let Some(index) = self.history.pop() else {
            return;
        };
        self.undo_one(index, cx);
        self.selected = index;
        self.load_selected(cx);
        cx.notify();
    }

    /// Puts one removed thread back.
    fn undo_one(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.mails.len() {
            return;
        }
        self.done[index] = false;
        let change = self.mails[index]
            .id
            .as_ref()
            .and_then(|id| self.removed_by.remove(id))
            .unwrap_or(mail::Change::Archive);
        if self.mails[index].unread {
            self.adjust_counts(index, -Self::count_change(change));
        }
        // Undoing a snooze cancels it.
        self.forget_snoozes(&[index]);
        if let Some(inverse) = change.inverse() {
            self.sync(index, inverse, false, cx);
        }
        if let Some(thread) = self.mails[index].id.clone()
            && let Some(label) = self.moved.remove(&thread)
        {
            self.undo_block(index, &label, cx);
            self.mails[index].labels.retain(|id| *id != label);
            if let Some(client) = self.connection.clone() {
                let working = self.writes.begin();
                cx.background_executor()
                    .spawn(async move {
                        let _working = working;
                        let _ = client.remove_label(&thread, &label);
                    })
                    .detach();
            }
        }
    }

    pub(super) fn toggle_star(&mut self, cx: &mut Context<Self>) {
        if self.picked.len() >= 2 {
            let targets = self.targets();
            let starred = !targets.iter().all(|i| self.mails[*i].starred);
            for index in targets {
                if self.mails[index].starred != starred {
                    self.mails[index].starred = starred;
                    let change = if starred {
                        mail::Change::Star
                    } else {
                        mail::Change::Unstar
                    };
                    self.sync(index, change, false, cx);
                }
            }
            cx.notify();
            return;
        }
        let index = self.selected;
        if !self.visible().contains(&index) {
            return;
        }
        let starred = !self.mails[index].starred;
        self.mails[index].starred = starred;
        let change = if starred {
            mail::Change::Star
        } else {
            mail::Change::Unstar
        };
        self.sync(index, change, false, cx);
        cx.notify();
    }

    pub(super) fn set_unread_only(&mut self, unread_only: bool, cx: &mut Context<Self>) {
        if self.unread_only != unread_only {
            self.filter_gen += 1;
            self.filter_switched = Some(std::time::Instant::now());
        }
        self.unread_only = unread_only;
        if !self.visible().contains(&self.selected) {
            self.selected = self.visible().first().copied().unwrap_or(0);
        }
        self.load_selected(cx);
        cx.notify();
    }
}

/// When a snoozed thread comes back, as its row shows it: the weekday and time within the
/// week, the date after that.
pub(super) fn snooze_time(until_ms: i64) -> SharedString {
    let Some(when) = chrono::DateTime::from_timestamp_millis(until_ms) else {
        return SharedString::default();
    };
    let when = when.with_timezone(&chrono::Local);
    let soon = when - chrono::Local::now() < chrono::Duration::days(6);
    when.format(if soon { "%a %H:%M" } else { "%-d %b" })
        .to_string()
        .into()
}

/// A snoozed thread's chip: a clock (a bell for a reminder) and when it comes back, in amber.
pub(super) fn snooze_chip(t: &Theme, name: &'static str, when: SharedString) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .px(px(6.))
        .h(px(18.))
        .rounded(px(5.))
        .bg(t.amber.opacity(0.14))
        .child(icon(name, 11., t.amber_text))
        .child(caption(when, t.amber_text))
}

/// The snooze of the thread with this id, if one is kept here: its chip's icon and text.
pub(super) fn snooze_of<'a>(
    list: impl IntoIterator<Item = &'a Snooze>,
    id: Option<&str>,
) -> Option<(&'static str, SharedString)> {
    let id = id?;
    list.into_iter().find(|s| s.thread_id == id).map(|s| {
        (
            if s.no_reply { "bell" } else { "clock" },
            snooze_day(s.until_ms),
        )
    })
}

/// The day a snoozed thread comes back, for its row's chip: the time if that is today,
/// else the date ("Nov 26").
pub(super) fn snooze_day(until_ms: i64) -> SharedString {
    let Some(when) = chrono::DateTime::from_timestamp_millis(until_ms) else {
        return SharedString::default();
    };
    let when = when.with_timezone(&chrono::Local);
    let today = when.date_naive() == chrono::Local::now().date_naive();
    when.format(if today { "%-I:%M %p" } else { "%b %-d" })
        .to_string()
        .into()
}

/// Brings a snoozed thread back if it is still away: into the inbox, unread. Whether it
/// came back. A reminder is not needed once someone wrote, and a snooze is over by itself
/// once a reply put the thread back in the inbox.
pub(super) fn wake(
    client: &mail::Client,
    id: &str,
    since_ms: i64,
    no_reply: bool,
) -> anyhow::Result<bool> {
    let now = client.summary(id, false)?;
    let answered = now.timestamp_ms > since_ms;
    if answered && (no_reply || now.labels.iter().any(|l| l == "INBOX")) {
        return Ok(false);
    }
    client.relabel(id, &["INBOX", "UNREAD"], &[])?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snooze_chip_shows_the_day_or_todays_time() {
        let at =
            |days: i64| (chrono::Local::now() + chrono::Duration::days(days)).timestamp_millis();
        let later = snooze_day(at(3));
        assert!(
            !later.contains(':') && later.split(' ').count() == 2,
            "{later}"
        );
        let soon = (chrono::Local::now() + chrono::Duration::minutes(1)).timestamp_millis();
        // Today (unless that minute is already tomorrow): a time.
        let today = snooze_day(soon);
        assert!(
            today.contains(':') || today.split(' ').count() == 2,
            "{today}"
        );
    }

    #[test]
    fn a_snooze_shows_its_weekday_soon_and_its_date_later() {
        let at =
            |days: i64| (chrono::Local::now() + chrono::Duration::days(days)).timestamp_millis();
        let soon = snooze_time(at(2));
        assert!(soon.contains(':'), "{soon}");
        let later = snooze_time(at(20));
        assert!(!later.contains(':'), "{later}");
    }
}
