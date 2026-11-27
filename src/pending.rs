//! What a change here does to mailboxes other than the one on screen, before their own
//! listing shows it: a thread starred shows in Starred at once, one moved to Trash or
//! Spam, archived or labelled shows there, and goes from where it left.
//!
//! A mailbox is shown from memory, from its store or freshly listed, and any of those can
//! be from before the change. So the changes are kept here and applied to each of them
//! (`apply`), until a fresh listing agrees with one or a while has passed.

use std::time::{Duration, Instant};

use crate::data::Mail;
use crate::mail::View;

/// How long a change is applied for, at most: the listing has caught up long before.
const KEPT: Duration = Duration::from_secs(120);

struct Entry {
    /// The mailbox it is in or out of (its `base`: one account's Starred, or every one's).
    view: View,
    account: String,
    thread: String,
    /// The thread as it is now, when it went in; `None` when it went out.
    mail: Option<Mail>,
    at: Instant,
}

#[derive(Default)]
pub struct Pending {
    entries: Vec<Entry>,
}

impl Pending {
    /// `mail` (of `account`) is now in `view`.
    pub fn add(&mut self, view: View, account: &str, mail: &Mail) {
        let Some(thread) = mail.id.clone() else {
            return;
        };
        self.forget(view, account, &thread);
        self.entries.push(Entry {
            view: view.base(),
            account: account.to_string(),
            thread,
            mail: Some(mail.clone()),
            at: Instant::now(),
        });
    }

    /// The thread (of `account`) is no longer in `view`.
    pub fn remove(&mut self, view: View, account: &str, thread: &str) {
        self.forget(view, account, thread);
        self.entries.push(Entry {
            view: view.base(),
            account: account.to_string(),
            thread: thread.to_string(),
            mail: None,
            at: Instant::now(),
        });
    }

    fn forget(&mut self, view: View, account: &str, thread: &str) {
        let view = view.base();
        self.entries
            .retain(|e| !(e.view == view && e.account == account && e.thread == thread));
    }

    /// Applies the changes to `mails`, the listing of `view` with `shown` the account on
    /// screen (`account_of` says each row's). `listed`: freshly listed by the server, so a
    /// change it already shows is no longer kept. `remove`: threads that left are taken
    /// out; the list on screen keeps its own (they stay in it, cleared, for Undo).
    pub fn apply(
        &mut self,
        view: View,
        shown: &str,
        mails: &mut Vec<Mail>,
        listed: bool,
        remove: bool,
        account_of: impl Fn(&Mail) -> String,
    ) {
        self.entries.retain(|e| e.at.elapsed() < KEPT);
        if self.entries.is_empty() {
            return;
        }
        let every = matches!(view, View::Every(_));
        let base = view.base();
        let mut agreed = Vec::new();
        for (k, entry) in self.entries.iter().enumerate() {
            if entry.view != base || !(every || entry.account == shown) {
                continue;
            }
            let at = mails.iter().position(|m| {
                m.id.as_deref() == Some(entry.thread.as_str()) && account_of(m) == entry.account
            });
            match (&entry.mail, at) {
                (Some(_), Some(_)) | (None, None) => {
                    if listed {
                        agreed.push(k);
                    }
                }
                (Some(mail), None) => {
                    let mut mail = mail.clone();
                    // A row names its account when it is not the one on screen.
                    mail.account = (every || entry.account != shown).then(|| entry.account.clone());
                    let at = mails
                        .iter()
                        .position(|m| m.timestamp_ms < mail.timestamp_ms)
                        .unwrap_or(mails.len());
                    mails.insert(at, mail);
                }
                (None, Some(at)) => {
                    if remove {
                        mails.remove(at);
                    }
                }
            }
        }
        for k in agreed.into_iter().rev() {
            self.entries.remove(k);
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(id: &str, at: i64) -> Mail {
        Mail {
            row: crate::data::Row {
                id: Some(id.into()),
                timestamp_ms: at,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn ids(mails: &[Mail]) -> Vec<&str> {
        mails.iter().filter_map(|m| m.id.as_deref()).collect()
    }

    #[test]
    fn a_change_shows_in_its_mailbox_until_the_listing_agrees() {
        let mut pending = Pending::default();
        pending.add(View::Starred, "a", &mail("t2", 20));
        pending.remove(View::Starred, "a", "t9");
        let account = |_: &Mail| "a".to_string();
        // A stale listing: the starred one goes in by its date, the other comes out.
        let mut shown = vec![mail("t9", 90), mail("t3", 30), mail("t1", 10)];
        pending.apply(View::Starred, "a", &mut shown, false, true, account);
        assert_eq!(ids(&shown), ["t3", "t2", "t1"]);
        // Another mailbox, and another account's Starred, are left alone.
        let mut inbox = vec![mail("t9", 90)];
        pending.apply(View::Inbox, "a", &mut inbox, false, true, account);
        assert_eq!(ids(&inbox), ["t9"]);
        let mut other = vec![mail("t9", 90)];
        pending.apply(View::Starred, "b", &mut other, false, true, |_| {
            "b".to_string()
        });
        assert_eq!(ids(&other), ["t9"]);
        // A fresh listing that shows both: nothing is kept any more.
        let mut listed = vec![mail("t2", 20)];
        pending.apply(View::Starred, "a", &mut listed, true, true, account);
        assert!(pending.entries.is_empty());
        // The list on screen keeps its own leavers: a removal is only agreed with there.
        pending.remove(View::Inbox, "a", "t5");
        let mut on_screen = vec![mail("t5", 50)];
        pending.apply(View::Inbox, "a", &mut on_screen, true, false, account);
        assert_eq!(ids(&on_screen), ["t5"]);
        pending.clear();
        // Starred again after being unstarred: only the last change counts.
        pending.remove(View::Starred, "a", "t2");
        pending.add(View::Starred, "a", &mail("t2", 20));
        assert_eq!(pending.entries.len(), 1);
    }
}
