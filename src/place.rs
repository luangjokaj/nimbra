//! Where the main window was: the account shown and its mailbox, kept across launches next
//! to the window's frame, so Nimbra opens where it was left. Saved off the main thread.

use std::path::PathBuf;

use nimbra_core::mail::View;
use serde::{Deserialize, Serialize};

/// The account shown and the mailbox open in it. A label is kept by its id (its place in
/// the list moves as labels are made), and every account's label by its name, as the
/// sidebar shows those.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Place {
    pub account: String,
    /// Any other mailbox, by `View::number`.
    pub view: Option<usize>,
    pub label: Option<String>,
    pub every_label: Option<String>,
}

/// The mailboxes kept by number: each one, and each one of every account's.
const PLAIN: [View; 15] = [
    View::Inbox,
    View::Primary,
    View::Updates,
    View::Promotions,
    View::Social,
    View::Starred,
    View::Snoozed,
    View::Reminders,
    View::Drafts,
    View::Sent,
    View::Done,
    View::AllMail,
    View::Important,
    View::Spam,
    View::Trash,
];

impl Place {
    /// Where `view` is, with `label` the id of an account label, or the name of every
    /// account's label, it shows. `None` for a search, which is never reopened.
    pub fn of(account: &str, view: View, label: Option<String>) -> Option<Place> {
        let mut place = Place {
            account: account.to_string(),
            ..Place::default()
        };
        match view.base() {
            View::Search => return None,
            View::Label(_) if view.is_every() => place.every_label = Some(label?),
            View::Label(_) => place.label = Some(label?),
            _ => place.view = Some(view.number()),
        }
        Some(place)
    }

    /// The mailbox kept by number, if it is one Nimbra still has.
    pub fn plain_view(&self) -> Option<View> {
        let number = self.view?;
        PLAIN
            .into_iter()
            .flat_map(|view| [view, View::every(view)])
            .find(|view| view.number() == number)
    }
}

fn path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/Nimbra/place.json"))
}

pub fn load() -> Option<Place> {
    if !crate::window_frame::kept() {
        return None;
    }
    let bytes = std::fs::read(path()?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Blocking: call it off the main thread.
pub fn save(place: Place) {
    if !crate::window_frame::kept() {
        return;
    }
    let Some(path) = path() else {
        return;
    };
    let partial = path.with_extension("json.partial");
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| serde_json::to_vec(&place).map_err(std::io::Error::other))
        .and_then(|json| std::fs::write(&partial, json))
        .and_then(|()| std::fs::rename(&partial, &path));
    if written.is_err() {
        eprintln!("nimbra: could not save where the window was");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_plain_mailbox_comes_back_as_itself() {
        for view in PLAIN.into_iter().flat_map(|view| [view, View::every(view)]) {
            let place = Place::of("a1", view, None).unwrap();
            assert_eq!(place.plain_view(), Some(view), "{view:?}");
        }
    }

    #[test]
    fn labels_are_kept_by_id_and_every_accounts_by_name() {
        let label = Place::of("a1", View::Label(3), Some("Label_7".into())).unwrap();
        assert_eq!(label.label.as_deref(), Some("Label_7"));
        assert_eq!(label.plain_view(), None);
        let every = Place::of("a1", View::every(View::Label(0)), Some("Receipts".into())).unwrap();
        assert_eq!(every.every_label.as_deref(), Some("Receipts"));
        assert_eq!(Place::of("a1", View::Label(3), None), None);
    }

    #[test]
    fn a_search_is_not_kept() {
        assert_eq!(Place::of("a1", View::Search, None), None);
        assert_eq!(Place::of("a1", View::every(View::Search), None), None);
    }

    #[test]
    fn an_unknown_number_opens_nothing() {
        let place = Place {
            view: Some(9_999),
            ..Place::default()
        };
        assert_eq!(place.plain_view(), None);
    }
}
