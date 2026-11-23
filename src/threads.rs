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

    /// The times offered when snoozing.
    pub(super) fn snooze_choices() -> Vec<SnoozeChoice> {
        use chrono::{Datelike, Duration as Days, Local, TimeZone, Timelike, Weekday};
        let now = Local::now();
        let at = |date: chrono::NaiveDate, hour: u32| {
            Local
                .from_local_datetime(&date.and_hms_opt(hour, 0, 0).unwrap_or_default())
                .single()
        };
        let label = |when: chrono::DateTime<Local>| when.format("%a %-d %b, %H:%M").to_string();
        let mut choices = Vec::new();
        let mut push =
            |icon, name: &str, when: Option<chrono::DateTime<Local>>, header, no_reply| {
                if let Some(when) = when {
                    choices.push(SnoozeChoice {
                        icon,
                        label: name.to_string(),
                        hint: label(when),
                        header,
                        until_ms: when.timestamp_millis(),
                        no_reply,
                    });
                }
            };
        let today = now.date_naive();
        if now.hour() < 17 {
            push("clock", "Later today", at(today, 18), Some("SNOOZE"), false);
        } else {
            push(
                "clock",
                "In three hours",
                Some(now + Days::hours(3)),
                Some("SNOOZE"),
                false,
            );
        }
        let tomorrow = today + Days::days(1);
        push("clock", "Tomorrow morning", at(tomorrow, 8), None, false);
        if !matches!(now.weekday(), Weekday::Fri | Weekday::Sat | Weekday::Sun) {
            let mut saturday = today;
            while saturday.weekday() != Weekday::Sat {
                saturday += Days::days(1);
            }
            push("calendar", "This weekend", at(saturday, 9), None, false);
        }
        let mut monday = today + Days::days(1);
        while monday.weekday() != Weekday::Mon {
            monday += Days::days(1);
        }
        push("calendar", "Next week", at(monday, 8), None, false);
        push(
            "bell",
            "Remind me if no reply in 3 days",
            at(today + Days::days(3), 9),
            Some("REMINDER"),
            true,
        );
        push(
            "bell",
            "Remind me if no reply in a week",
            at(today + Days::days(7), 9),
            None,
            true,
        );
        choices
    }

    /// Snoozes the open (or picked) threads: out of the inbox now, back at the chosen time,
    /// unread and with a notification. A reminder only comes back if nobody has replied.
    pub(super) fn snooze(&mut self, choice: SnoozeChoice, cx: &mut Context<Self>) {
        if self.connection.is_some() && self.store.is_none() {
            // It would leave the Inbox with nothing to bring it back.
            self.notice = Some(STORE_UNAVAILABLE.into());
            cx.notify();
            return;
        }
        let mut new = Vec::new();
        for index in self.targets() {
            let mail = &self.mails[index];
            let Some(thread_id) = mail.id.clone() else {
                continue;
            };
            new.push(Snooze {
                thread_id,
                until_ms: choice.until_ms,
                no_reply: choice.no_reply,
                since_ms: mail.timestamp_ms,
                mail: mail.clone(),
            });
        }
        self.change_snoozed(|list| {
            list.retain(|kept| !new.iter().any(|s| s.thread_id == kept.thread_id));
            list.extend(new.iter().cloned());
        });
        self.watch_schedule(cx);
        if inbox_like(self.mailbox) {
            self.remove_selected(mail::Change::Archive, cx);
        }
        self.notice = Some(
            if choice.no_reply {
                format!("I’ll bring it back on {} if nobody replies.", choice.hint)
            } else {
                format!("Snoozed until {}.", choice.hint)
            }
            .into(),
        );
        cx.notify();
    }

    /// Changes the snoozed list here and in the store, each as it is: saving this copy
    /// could undo what a background check just took out.
    fn change_snoozed(&mut self, change: impl Fn(&mut Vec<Snooze>)) {
        change(&mut self.snoozed);
        if let Some(store) = &self.store
            && store.update("snoozed", |kept| change(kept)).is_err()
        {
            eprintln!("nimbra: could not save the snoozed list");
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
                        let wake = || {
                            if no_reply && client.summary(&id, false)?.timestamp_ms > since {
                                return anyhow::Ok(false); // someone wrote: no reminder needed
                            }
                            client.relabel(&id, &["INBOX", "UNREAD"], &[])?;
                            anyhow::Ok(true)
                        };
                        let woken = wake();
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
        if self.mailbox == View::Snoozed {
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
        if self.mailbox == View::Snoozed {
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
            View::Snoozed => mail::Change::Unarchive,
            _ => return None,
        })
    }

    fn forget_snoozes(&mut self, rows: &[usize]) {
        let threads: Vec<String> = rows
            .iter()
            .filter_map(|i| self.mails[*i].id.clone())
            .collect();
        if self.snoozed.iter().any(|s| threads.contains(&s.thread_id)) {
            self.change_snoozed(|list| list.retain(|s| !threads.contains(&s.thread_id)));
        }
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
        if let Some(thread) = self.mails[index].id.clone()
            && self.snoozed.iter().any(|s| s.thread_id == thread)
        {
            self.change_snoozed(|list| list.retain(|s| s.thread_id != thread));
        }
        if let Some(inverse) = change.inverse() {
            self.sync(index, inverse, false, cx);
        }
        if let Some(thread) = self.mails[index].id.clone()
            && let Some(label) = self.moved.remove(&thread)
        {
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
