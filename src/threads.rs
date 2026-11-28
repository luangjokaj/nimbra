//! What you do to a thread: done, trash, star, read and unread, snooze, delete for ever, and
//! undoing it. Also picking several threads to act on at once.

use super::*;

// A thread put away until later, kept with its account: the engine's, as both apps keep it.
pub(super) use nimbra_core::snooze::Snooze;

/// One choice in the snooze palette.
#[derive(Clone)]
pub(super) struct SnoozeChoice {
    pub(super) icon: &'static str,
    pub(super) label: String,
    pub(super) hint: String,
    pub(super) header: Option<&'static str>,
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

    /// The threads in these rows, by (thread id, account), for `delete_forever`.
    pub(super) fn delete_keys(&self, rows: &[usize]) -> Vec<(String, String)> {
        rows.iter()
            .filter_map(|i| {
                let mail = self.mails.get(*i)?;
                Some((mail.id.clone()?, self.account_of(mail).to_string()))
            })
            .collect()
    }

    /// Deletes threads for good (after a yes), each through its own account (every
    /// account's Trash mixes them). If the account shown's sign-in cannot do that yet, asks
    /// to give Nimbra that permission first; another account's is opened to do that.
    pub(super) fn delete_forever(&mut self, ids: Vec<(String, String)>, cx: &mut Context<Self>) {
        let mut by_account: Vec<(String, Arc<mail::Client>, Vec<String>)> = Vec::new();
        for (id, account) in &ids {
            match by_account.iter_mut().find(|(a, _, _)| a == account) {
                Some((_, _, list)) => list.push(id.clone()),
                None => {
                    let Some(client) = self.client_for(Some(account)) else {
                        continue; // not connected: left as it is
                    };
                    by_account.push((account.clone(), client, vec![id.clone()]));
                }
            }
        }
        if by_account.is_empty() {
            return;
        }
        // Out of the list now; there is no undo for this.
        for index in 0..self.mails.len() {
            let mail = &self.mails[index];
            let account = self.account_of(mail);
            if mail
                .id
                .as_ref()
                .is_some_and(|id| ids.iter().any(|(i, a)| i == id && a == account))
            {
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
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _working = working;
                    let mut deleted = 0;
                    for (account, client, list) in &by_account {
                        for id in list {
                            if let Err(error) = client.delete_thread(id) {
                                return Err((deleted, account.clone(), error));
                            }
                            deleted += 1;
                        }
                    }
                    Ok(deleted)
                })
                .await;
            this.update(cx, |this, cx| {
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
                    Err((_, account, error)) => {
                        // Put back what was not deleted.
                        this.refresh(cx);
                        if !error.is::<gmail::NeedsFullAccess>() {
                            this.fail(error, cx);
                        } else if account == this.account_id {
                            let shown: Vec<(String, String)> =
                                ids.into_iter().filter(|(_, a)| *a == account).collect();
                            this.confirm = Some(Ask::GrantFullAccess(shown));
                        } else {
                            let email = this.email_of(&account).unwrap_or_default();
                            this.notice = Some(
                                format!(
                                    "To delete permanently from {email}, open that account and \
                                     delete there once."
                                )
                                .into(),
                            );
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

    /// The times offered when snoozing (`snooze::menu_times`), as the menu shows them.
    pub(super) fn snooze_choices() -> Vec<SnoozeChoice> {
        use nimbra_core::snooze::MenuTime;
        let label =
            |when: chrono::DateTime<chrono::Local>| when.format("%a %-d %b, %H:%M").to_string();
        nimbra_core::snooze::menu_times(chrono::Local::now())
            .into_iter()
            .map(|(kind, when)| {
                let (icon, name, header) = match kind {
                    MenuTime::LaterToday => ("clock", "Later today", Some("SNOOZE")),
                    MenuTime::InThreeHours => ("clock", "In three hours", Some("SNOOZE")),
                    MenuTime::TomorrowMorning => ("clock", "Tomorrow morning", None),
                    MenuTime::ThisWeekend => ("calendar", "This weekend", None),
                    MenuTime::NextWeek => ("calendar", "Next week", None),
                    MenuTime::NoReplyInThreeDays => {
                        ("bell", "Remind me if no reply in 3 days", Some("REMINDER"))
                    }
                    MenuTime::NoReplyInAWeek => ("bell", "Remind me if no reply in a week", None),
                };
                SnoozeChoice {
                    icon,
                    label: name.to_string(),
                    hint: label(when),
                    header,
                    until_ms: when.timestamp_millis(),
                    no_reply: kind.no_reply(),
                }
            })
            .collect()
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
            if mail.id.is_none() {
                continue;
            }
            new.push((
                self.account_of(mail).to_string(),
                Snooze::new(&mail.row, choice.until_ms, choice.no_reply),
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
            self.change_snoozed_of(&owner, move |list| nimbra_core::snooze::put(list, &theirs));
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
                if matches!(self.mailbox.base(), View::Snoozed | View::Reminders) {
                    if (self.mailbox.base() == View::Reminders) == choice.no_reply {
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

    /// Changes the snoozed list here and in the store, each as it is: saving this copy
    /// could undo what a background check just took out.
    fn change_snoozed(&mut self, change: impl Fn(&mut Vec<Snooze>) + Send + Sync + 'static) {
        change(&mut self.snoozed);
        if let Some(store) = &self.store {
            drop(store.update_soon(nimbra_core::snooze::LIST, move |kept| change(kept)));
        }
    }

    /// The same for any account: the shown one's as above, another's in its own store (or,
    /// with none open, the shown one's, which brings it back all the same).
    fn change_snoozed_of(
        &mut self,
        account: &str,
        change: impl Fn(&mut Vec<Snooze>) + Send + Sync + 'static,
    ) {
        match self.other_stores.get(account) {
            Some(store) if account != self.account_id => {
                let (changes, list) = self.snoozed_away.entry(account.to_string()).or_default();
                *changes += 1;
                change(list);
                drop(store.update_soon(nimbra_core::snooze::LIST, move |kept| change(kept)));
                self.read_snoozed_elsewhere();
            }
            _ => self.change_snoozed(change),
        }
    }

    /// Takes another account's snoozed list as its store has it now, unless it was changed
    /// here meanwhile.
    pub(super) fn reread_snoozed_of(&mut self, account: String, cx: &mut Context<Self>) {
        let Some(store) = self.other_stores.get(&account).cloned() else {
            return;
        };
        let changes = self.snoozed_away.get(&account).map_or(0, |(n, _)| *n);
        cx.spawn(async move |this, cx| {
            let reading = store.clone();
            let list: Vec<Snooze> = cx
                .background_executor()
                .spawn(async move { reading.load(nimbra_core::snooze::LIST).unwrap_or_default() })
                .await;
            this.update(cx, |this, cx| {
                let same_store = this
                    .other_stores
                    .get(&account)
                    .is_some_and(|now| Arc::ptr_eq(now, &store));
                let unchanged = this.snoozed_away.get(&account).map_or(0, |(n, _)| *n) == changes;
                if same_store && unchanged {
                    this.snoozed_away.insert(account, (changes, list));
                    this.read_snoozed_elsewhere();
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Forgets the snoozes of these threads, whichever account's list has them.
    pub(super) fn forget_snoozes_of(&mut self, threads: &[(String, String)]) {
        let accounts: std::collections::BTreeSet<&String> =
            threads.iter().map(|(account, _)| account).collect();
        for account in accounts {
            let ids: Arc<Vec<String>> = Arc::new(
                threads
                    .iter()
                    .filter(|(a, _)| a == account)
                    .map(|(_, id)| id.clone())
                    .collect(),
            );
            let account = account.clone();
            let theirs = ids.clone();
            self.change_snoozed_of(&account, move |list| {
                nimbra_core::snooze::forget(list, &theirs)
            });
            // One kept with the shown account from before (snoozed from All inboxes).
            if account != self.account_id && self.snoozed.iter().any(|s| ids.contains(&s.thread_id))
            {
                self.change_snoozed(move |list| nimbra_core::snooze::forget(list, &ids));
            }
        }
    }

    /// The shown account's snoozes (or reminders) as rows of Snoozed (or Reminders), with
    /// the time they come back where the time they arrived would be.
    pub(super) fn snoozed_rows(&mut self, reminders: bool) -> Vec<Mail> {
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
                let mut mail = Mail::from(s.mail.clone());
                mail.time = snooze_time(s.until_ms);
                mail
            })
            .collect()
    }

    /// The other signed-in accounts' snoozes, from their lists as last read, each marked
    /// with its account so its row acts there.
    pub(super) fn read_snoozed_elsewhere(&mut self) {
        let mut found = Vec::new();
        for account in self.other_stores.keys() {
            if *account == self.account_id || !self.others.contains_key(account) {
                continue;
            }
            let Some((_, list)) = self.snoozed_away.get(account) else {
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

    /// A reply came in while a thread was snoozed: it is back in the inbox by itself (as
    /// Gmail does), so it is not brought back again later. `fresh` is an inbox's listing.
    pub(super) fn unsnooze_answered(&mut self, fresh: &[Mail]) {
        let answered: Vec<(String, String)> = fresh
            .iter()
            .filter_map(|mail| {
                let id = mail.id.as_ref()?;
                let account = self.account_of(mail);
                let away = (account != self.account_id)
                    .then(|| self.snoozed_away.get(account))
                    .flatten()
                    .map(|(_, list)| list.as_slice())
                    .unwrap_or_default();
                self.snoozed
                    .iter()
                    .chain(away)
                    .any(|s| nimbra_core::snooze::is_answered(s, &mail.row))
                    .then(|| (account.to_string(), id.clone()))
            })
            .collect();
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
                let item_kept = item.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let _working = working;
                        // Out of its account's list here, whichever account is shown by now.
                        match &store {
                            Some(store) => {
                                nimbra_core::snooze::wake_kept(store, &*client, &item_kept)
                            }
                            None => wake(&*client, &id, since, no_reply),
                        }
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
                                    // Its own account's, as Archive and Open act on it there.
                                    let whose = item.mail.account.as_deref().unwrap_or(&owner);
                                    crate::notify::show(
                                        &format!("acct:{whose}:{}", item.thread_id),
                                        title,
                                        &item.mail.subject,
                                        this.settings.notify_sound,
                                    );
                                }
                                this.notice = Some(
                                    format!("{title}: {}", short(&item.mail.subject, 50)).into(),
                                );
                                if inbox_like(this.mailbox) || this.mailbox.base() == View::Snoozed
                                {
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
        // `counts` are the shown account's; the others have their inbox's number, and the
        // counts their last listing in every account's mailboxes found.
        if let Some(other) = mail.account.as_deref().filter(|a| *a != self.account_id) {
            if let Some(unread) = self.account_unread.get_mut(other) {
                *unread = (*unread as i32 + delta).max(0) as u32;
            }
            if let Some(counts) = self.account_counts.get_mut(other) {
                for id in mail.labels.iter().filter(|l| *l != "INBOX") {
                    if let Some((unread, _)) = counts.get_mut(id) {
                        *unread = (*unread as i32 + delta).max(0) as u32;
                    }
                }
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
        self.open_in_gmail_at(self.selected, cx);
    }

    /// Whether the thread in that row is a Gmail account's (so Gmail can open it), whichever
    /// account is shown: in All inboxes, threads are from every account.
    pub(super) fn gmail_at(&self, row: usize) -> bool {
        self.mails
            .get(row)
            .is_some_and(|mail| self.servers_of(self.account_of(mail)).is_none())
    }

    /// The thread in that row in Gmail, in the browser.
    pub(super) fn open_in_gmail_at(&mut self, row: usize, cx: &mut Context<Self>) {
        if !self.gmail_at(row) {
            return;
        }
        // Thread ids are validated as plain alphanumerics before they get here.
        if let Some(mail) = self.mails.get(row)
            && let Some(id) = mail.id.as_ref()
            && id.chars().all(|c| c.is_ascii_alphanumeric())
        {
            // In the browser's sign-in for the thread's own account, not just the first one.
            let account = self
                .email_of(self.account_of(mail))
                .filter(|email| {
                    email.chars().all(|c| {
                        c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '-' | '_' | '+')
                    })
                })
                .map(|email| {
                    format!(
                        "?authuser={}",
                        email.replace('@', "%40").replace('+', "%2B")
                    )
                })
                .unwrap_or_default();
            cx.open_url(&format!("https://mail.google.com/mail/{account}#all/{id}"));
        }
        cx.notify();
    }

    /// X: the open thread in or out of the picked ones.
    pub(super) fn pick_thread(&mut self, cx: &mut Context<Self>) {
        if self.palette || !self.visible().contains(&self.selected) {
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
        if self.palette || self.pane == Pane::Sidebar {
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
        if matches!(self.mailbox.base(), View::Snoozed | View::Reminders) {
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
        if matches!(self.mailbox.base(), View::Snoozed | View::Reminders) {
            self.forget_snoozes(&[index]);
        }
        self.remove_thread(index, change, cx);
        true
    }

    /// What Done does in the open mailbox.
    pub(super) fn done_change(&self) -> Option<mail::Change> {
        Some(match self.mailbox.base() {
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
        if self.mailbox.base() != View::Trash {
            self.remove_thread(index, mail::Change::Trash, cx);
            return true;
        }
        let ids = self.delete_keys(&[index]);
        if ids.is_empty() {
            return false;
        }
        self.confirm = Some(Ask::DeleteForever(ids));
        cx.notify();
        true
    }

    /// Moves the open thread to Gmail's trash (kept for 30 days, and undoable with `Z`).
    /// Moves the open (or picked) threads to Trash; in Trash itself, offers to delete them
    /// permanently.
    pub(super) fn trash(&mut self, cx: &mut Context<Self>) {
        if self.mailbox.base() == View::Trash {
            let ids = self.delete_keys(&self.targets());
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

    /// Pressing `Z` in the main window now would bring something back.
    pub(super) fn can_undo(&self) -> bool {
        // With the composer in this window, Z is a letter.
        let typing = self.compose.is_some() && self.compose_host.is_none();
        !typing && (self.held_back().is_some() || !self.history.is_empty())
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
        // The label it was moved to comes off first: going back to its mailbox (`sync`
        // below) shows it there at once as it is now, so without its label.
        if let Some(thread) = self.mails[index].id.clone()
            && let Some(label) = self.moved.remove(&thread)
        {
            self.undo_block(index, &label, cx);
            self.mails[index].labels.retain(|id| *id != label);
            // The thread's own account: in All inboxes it may be another than the one shown.
            if let Some(client) = self.client_at(index) {
                let working = self.writes.begin();
                cx.background_executor()
                    .spawn(async move {
                        let _working = working;
                        let _ = client.remove_label(&thread, &label);
                    })
                    .detach();
            }
        }
        if let Some(inverse) = change.inverse() {
            self.sync(index, inverse, false, cx);
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
        .gap(dp(4.))
        .px(dp(6.))
        .h(dp(18.))
        .rounded(dp(5.))
        .bg(t.amber.opacity(0.14))
        .child(icon(name, 11., t.amber))
        .child(mono(when, t.amber))
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

// `wake` moved to the core, with the snoozes the iOS app keeps.
pub(super) use nimbra_core::snooze::wake;

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
