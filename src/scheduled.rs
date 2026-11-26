//! Send later. Each account's store has its list, which is the truth: whoever sends from
//! it takes the message out there and then, whichever account is shown by that time.

use serde::{Deserialize, Serialize};

use crate::mail;
use crate::store::Store;

const LIST: &str = "scheduled";

/// A message to send later: its saved draft and when.
#[derive(Clone, Serialize, Deserialize)]
pub struct Scheduled {
    pub draft_id: String,
    pub at_ms: i64,
    pub subject: String,
    /// The account the draft is in, when not the one whose store keeps this list.
    #[serde(default)]
    pub account: Option<String>,
    /// A request to send it went out, and how it ended is not known (yet).
    #[serde(default)]
    pub out: bool,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Sent,
    /// Its draft is gone: sent already, or deleted.
    Gone,
    /// The request went out and its answer never came: it may have been sent. It is off
    /// the list, never sent again by itself.
    Unsure,
}

pub fn list(store: &Store) -> Vec<Scheduled> {
    store.load(LIST).unwrap_or_default()
}

/// Puts a message on the list; gives back the list as it now is. Blocking.
pub fn add(store: &Store, item: Scheduled) -> anyhow::Result<Vec<Scheduled>> {
    store.update(LIST, |list: &mut Vec<Scheduled>| {
        list.retain(|kept| kept.draft_id != item.draft_id);
        list.push(item);
    })
}

/// Sends one, unless someone else is on it or has sent it. `None`: nothing came of it, and
/// it stays on the list. One whose request may have reached the server is never sent again:
/// with `can_tell` it comes off the list as `Unsure`; without, it waits on the list for
/// when it can be told about. Blocking.
pub fn send(
    store: &Store,
    client: &mail::Client,
    draft_id: &str,
    can_tell: bool,
) -> Option<Outcome> {
    let name = claim_name(draft_id);
    let _claim = store.claim(&name)?;
    let Some(item) = list(store)
        .into_iter()
        .find(|item| item.draft_id == draft_id)
    else {
        store.remove(&name);
        return None;
    };
    let outcome = if item.out {
        // Out when the app ended, or when the connection did.
        Outcome::Unsure
    } else {
        // On disk before the request: a crash during it must not have it sent again.
        mark_out(store, draft_id, true).ok()?;
        match client.send_saved_draft(draft_id) {
            Ok(()) => Outcome::Sent,
            Err(error) if error.is::<mail::MayHaveArrived>() => Outcome::Unsure,
            Err(error) if mail::is_gone(&error) => Outcome::Gone,
            Err(_) => {
                // Certainly not sent (offline, or refused): tried again on a later check.
                if mark_out(store, draft_id, false).is_err() {
                    eprintln!("nimbra: could not save the send-later list");
                }
                return None;
            }
        }
    };
    if outcome == Outcome::Unsure && !can_tell {
        return None;
    }
    let taken_out = store.update(LIST, |list: &mut Vec<Scheduled>| {
        list.retain(|item| item.draft_id != draft_id);
    });
    if taken_out.is_err() {
        eprintln!("nimbra: could not save the send-later list");
    }
    store.remove(&name);
    Some(outcome)
}

fn mark_out(store: &Store, draft_id: &str, out: bool) -> anyhow::Result<()> {
    store.update(LIST, |list: &mut Vec<Scheduled>| {
        for item in list.iter_mut().filter(|item| item.draft_id == draft_id) {
            item.out = out;
        }
    })?;
    Ok(())
}

fn claim_name(draft_id: &str) -> String {
    let id: String = draft_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(48)
        .collect();
    format!("due-{id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sending::tests::Fake;

    fn due(store: &Store) {
        add(
            store,
            Scheduled {
                draft_id: "r-1".into(),
                at_ms: 0,
                subject: "Later".into(),
                account: None,
                out: false,
            },
        )
        .unwrap();
    }

    #[test]
    fn one_whose_answer_never_came_is_not_sent_again() {
        let store = Store::temp();
        due(&store);
        let lost = Fake::new(|| Err(anyhow::anyhow!("lost").context(mail::MayHaveArrived)));
        assert_eq!(send(&store, &*lost, "r-1", true), Some(Outcome::Unsure));
        assert!(list(&store).is_empty());
        assert_eq!(send(&store, &*lost, "r-1", true), None);
        assert_eq!(lost.requests(), 1);
        std::fs::remove_dir_all(store.dir()).unwrap();
    }

    #[test]
    fn one_out_when_the_app_ended_is_told_about_not_sent() {
        let store = Store::temp();
        due(&store);
        // The app ended during the request: the list still says it was out.
        mark_out(&store, "r-1", true).unwrap();
        let client = Fake::new(|| Ok(()));
        assert_eq!(send(&store, &*client, "r-1", false), None);
        assert_eq!(list(&store).len(), 1, "kept until it can be told about");
        assert_eq!(send(&store, &*client, "r-1", true), Some(Outcome::Unsure));
        assert!(list(&store).is_empty());
        assert_eq!(client.requests(), 0);
        std::fs::remove_dir_all(store.dir()).unwrap();
    }

    #[test]
    fn one_that_certainly_did_not_leave_is_tried_again() {
        let store = Store::temp();
        due(&store);
        let offline = Fake::new(|| Err(mail::Offline.into()));
        assert_eq!(send(&store, &*offline, "r-1", true), None);
        assert!(!list(&store)[0].out);
        let online = Fake::new(|| Ok(()));
        assert_eq!(send(&store, &*online, "r-1", true), Some(Outcome::Sent));
        assert!(list(&store).is_empty());
        std::fs::remove_dir_all(store.dir()).unwrap();
    }

    #[test]
    fn lists_saved_before_the_out_mark_still_load() {
        let store = Store::temp();
        let legacy = serde_json::json!([
            { "draft_id": "r-1", "at_ms": 5, "subject": "Old", "account": null },
            { "draft_id": "r-2", "at_ms": 6, "subject": "Older" }
        ]);
        store.save(LIST, &legacy).unwrap();
        let loaded = list(&store);
        assert_eq!(loaded.len(), 2);
        assert!(
            loaded
                .iter()
                .all(|item| !item.out && item.account.is_none())
        );
        std::fs::remove_dir_all(store.dir()).unwrap();
    }
}
