//! The mail window without a mouse: which pane has the keys, the sidebar's rows in the
//! order the arrows walk them, picking threads with Shift, and the shortcuts sheet.

use crate::mail::View;

/// Where Up, Down, Enter and Escape go.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Pane {
    List,
    Sidebar,
}

/// A sidebar row the keyboard can stop on.
#[derive(Clone, PartialEq, Debug)]
pub enum Row {
    Search,
    AllInboxes,
    /// An account's own row (several accounts), by its id.
    Account(String),
    Mailbox(View),
    Settings,
}

/// What the sidebar draws, as `Nimbra::sidebar` draws it.
pub struct Sidebar<'a> {
    /// The accounts' ids, in the sidebar's order.
    pub accounts: Vec<String>,
    /// The account shown.
    pub shown: &'a str,
    pub mailbox: View,
    pub demo: bool,
    /// The account has its own servers (IMAP): folders, and none of Gmail's categories or
    /// Important.
    pub folders: bool,
    pub labels: usize,
    /// The folded sections ("accounts", "split", "mailbox", "labels").
    pub collapsed: &'a [String],
}

/// The most labels the sidebar lists.
pub const LABELS_SHOWN: usize = 40;

impl Sidebar<'_> {
    pub fn several(&self) -> bool {
        !self.demo && self.accounts.len() >= 2
    }

    /// Top to bottom. A folded section still has the open mailbox, as on screen; the demo's
    /// sample rows, which do nothing, are left out.
    pub fn rows(&self) -> Vec<Row> {
        let open = |key: &str| !self.collapsed.iter().any(|k| k == key);
        let mut rows = vec![Row::Search];
        if self.several() {
            rows.push(Row::AllInboxes);
            for id in &self.accounts {
                if open("accounts") || (id == self.shown && self.mailbox != View::AllInboxes) {
                    rows.push(Row::Account(id.clone()));
                }
            }
        }
        let mut section = |key: &str, views: &[View]| {
            for view in views {
                if open(key) || *view == self.mailbox {
                    rows.push(Row::Mailbox(*view));
                }
            }
        };
        if self.demo {
            section("split", &[View::Inbox]);
            section(
                "mailbox",
                &[
                    View::Starred,
                    View::Snoozed,
                    View::Reminders,
                    View::Drafts,
                    View::Sent,
                    View::Done,
                ],
            );
        } else if self.folders {
            section(
                "mailbox",
                &[
                    View::Inbox,
                    View::Starred,
                    View::Snoozed,
                    View::Reminders,
                    View::Drafts,
                    View::Sent,
                    View::Done,
                    View::AllMail,
                    View::Spam,
                    View::Trash,
                ],
            );
        } else {
            section(
                "split",
                &[
                    View::Inbox,
                    View::Primary,
                    View::Updates,
                    View::Promotions,
                    View::Social,
                ],
            );
            section(
                "mailbox",
                &[
                    View::Starred,
                    View::Important,
                    View::Snoozed,
                    View::Reminders,
                    View::Drafts,
                    View::Sent,
                    View::Done,
                    View::AllMail,
                    View::Spam,
                    View::Trash,
                ],
            );
            let labels: Vec<View> = (0..self.labels.min(LABELS_SHOWN))
                .map(View::Label)
                .collect();
            section("labels", &labels);
        }
        rows.push(Row::Settings);
        rows
    }

    /// The row of what is open, where the cursor starts.
    pub fn here(&self) -> Row {
        match self.mailbox {
            View::Search => Row::Search,
            View::AllInboxes if !self.several() => Row::Mailbox(View::Inbox),
            View::AllInboxes => Row::AllInboxes,
            view => Row::Mailbox(view),
        }
    }

    /// Where ⌘1 to ⌘9 go: All inboxes and then each account, or with one account its Inbox.
    pub fn numbered(&self) -> Vec<Row> {
        if self.several() {
            std::iter::once(Row::AllInboxes)
                .chain(self.accounts.iter().cloned().map(Row::Account))
                .collect()
        } else {
            vec![Row::Mailbox(View::Inbox)]
        }
    }
}

/// Moves `at` by `delta` within `len` rows, stopping at either end.
pub fn step(at: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    (at.min(len - 1) as isize + delta).clamp(0, len as isize - 1) as usize
}

/// Shift with an arrow: the picked rows run from the open one to `end`, which moves by
/// `delta`. Positions are in the list as shown. Returns the new end and what is picked.
pub fn extend(open: usize, end: usize, delta: isize, len: usize) -> (usize, Vec<usize>) {
    let end = step(end, delta, len);
    let open = open.min(len.saturating_sub(1));
    (end, (open.min(end)..=open.max(end)).collect())
}

/// A piece of a shortcut on the sheet: a key cap, or a word between caps ("/", "then").
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cap {
    Key(&'static str),
    Word(&'static str),
}

const fn k(key: &'static str) -> Cap {
    Cap::Key(key)
}

const fn w(word: &'static str) -> Cap {
    Cap::Word(word)
}

/// One row of the shortcuts sheet: the keys, and what they do.
pub type Shortcut = (&'static [Cap], &'static str);

/// The shortcuts sheet (?), one tab each.
pub const SHORTCUT_TABS: &[(&str, &[Shortcut])] = &[
    (
        "Move",
        &[
            (
                &[k("J"), w("/"), k("K"), w("or arrows")],
                "Next / previous thread",
            ),
            (&[k("←")], "Move to the sidebar"),
            (
                &[
                    k("Home"),
                    w("/"),
                    k("End"),
                    w("(or"),
                    k("⌘↑"),
                    w("/"),
                    k("⌘↓"),
                    w(")"),
                ],
                "First / last thread",
            ),
            (&[k("X")], "Pick the open thread"),
            (
                &[
                    k("⇧↑"),
                    w("/"),
                    k("⇧↓"),
                    w("(or"),
                    k("⇧K"),
                    w("/"),
                    k("⇧J"),
                    w(")"),
                ],
                "Pick more threads",
            ),
            (&[k("O")], "Open in its own window"),
            (&[k("Space"), w("/"), k("⇧Space")], "Page down / up"),
            (
                &[
                    k("G"),
                    w("then"),
                    k("I"),
                    k("S"),
                    k("B"),
                    k("T"),
                    k("D"),
                    k("A"),
                ],
                "Go to a mailbox",
            ),
            (&[k("⌘1"), w("…"), k("⌘9")], "Switch account"),
            (&[k("⌘["), w("/"), k("⌘]")], "Back / forward"),
        ],
    ),
    (
        "Triage",
        &[
            (&[k("E")], "Mark done"),
            (&[k("H"), w("or"), k("⌘K")], "Snooze"),
            (&[k("S")], "Star or unstar"),
            (&[k("U")], "Mark unread or read"),
            (&[k("⌫"), w("or"), k("⌘⌫")], "Move to trash"),
            (&[k("Z")], "Undo"),
            (&[k("/"), w("or"), k("⌘F")], "Search"),
            (&[k("V")], "Original formatting or reader"),
            (&[k("↩")], "Read in full view"),
            (&[k("esc")], "Back to the list"),
        ],
    ),
    (
        "Write",
        &[
            (&[k("C"), w("or"), k("⌘N")], "New message"),
            (&[k("R")], "Reply"),
            (&[k("A")], "Reply all"),
            (&[k("F")], "Forward"),
            (&[k("⌘↩")], "Send"),
            (&[k("⌘⇧F")], "Full-page composer"),
            (&[k("⌘B"), k("⌘I"), k("⌘U")], "Bold, italic, underline"),
            (&[k("⌘K")], "Add a link (in a message)"),
            (&[k("⌘Z")], "Undo send"),
        ],
    ),
    (
        "Window",
        &[
            (&[k("⌘B")], "Show or hide the sidebar"),
            (&[k("⌘⇧B")], "Show or hide the message list"),
            (&[k("⌘R")], "Check for mail"),
            (&[k("⌘⇧L")], "Light or dark"),
            (&[k("⌘,")], "Settings"),
            (&[k("⌘W")], "Close the window (Nimbra keeps checking mail)"),
            (&[k("⌘Q")], "Quit"),
            (&[k("?")], "These shortcuts"),
        ],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| {
                if i == 0 {
                    String::new()
                } else {
                    format!("a{i}")
                }
            })
            .collect()
    }

    fn sidebar<'a>(accounts: &[String], collapsed: &'a [String], mailbox: View) -> Sidebar<'a> {
        Sidebar {
            accounts: accounts.to_vec(),
            shown: "",
            mailbox,
            demo: false,
            folders: false,
            labels: 3,
            collapsed,
        }
    }

    #[test]
    fn one_account_walks_search_mailboxes_labels_and_settings() {
        let accounts = ids(1);
        let rows = sidebar(&accounts, &[], View::Inbox).rows();
        assert_eq!(rows.first(), Some(&Row::Search));
        assert_eq!(rows[1], Row::Mailbox(View::Inbox));
        assert!(!rows.contains(&Row::AllInboxes));
        assert_eq!(rows[rows.len() - 2], Row::Mailbox(View::Label(2)));
        assert_eq!(rows.last(), Some(&Row::Settings));
        assert_eq!(rows.len(), 1 + 5 + 10 + 3 + 1);
    }

    #[test]
    fn several_accounts_come_first_after_search() {
        let accounts = ids(3);
        let rows = sidebar(&accounts, &[], View::Inbox).rows();
        assert_eq!(
            rows[1..5],
            [
                Row::AllInboxes,
                Row::Account(String::new()),
                Row::Account("a1".into()),
                Row::Account("a2".into()),
            ]
        );
    }

    #[test]
    fn a_folded_section_keeps_only_what_is_open() {
        let accounts = ids(3);
        let collapsed = [
            "mailbox".to_string(),
            "accounts".to_string(),
            "labels".into(),
        ];
        let rows = sidebar(&accounts, &collapsed, View::Sent).rows();
        assert!(rows.contains(&Row::Mailbox(View::Sent)));
        assert!(!rows.contains(&Row::Mailbox(View::Starred)));
        assert!(
            !rows
                .iter()
                .any(|r| matches!(r, Row::Mailbox(View::Label(_))))
        );
        // The account shown stays, as on screen; the others fold away.
        assert!(rows.contains(&Row::Account(String::new())));
        assert!(!rows.contains(&Row::Account("a1".into())));
        // In All inboxes no account is shown, so none stays.
        let all = sidebar(&accounts, &collapsed, View::AllInboxes).rows();
        assert!(!all.iter().any(|r| matches!(r, Row::Account(_))));
    }

    #[test]
    fn labels_stop_where_the_sidebar_does() {
        let accounts = ids(1);
        let mut many = sidebar(&accounts, &[], View::Inbox);
        many.labels = 90;
        let labels = many
            .rows()
            .iter()
            .filter(|r| matches!(r, Row::Mailbox(View::Label(_))))
            .count();
        assert_eq!(labels, LABELS_SHOWN);
    }

    #[test]
    fn the_demo_has_only_rows_that_open_something() {
        let accounts = ids(3);
        let mut demo = sidebar(&accounts, &[], View::Inbox);
        demo.demo = true;
        let rows = demo.rows();
        assert!(!rows.contains(&Row::AllInboxes));
        assert!(!rows.contains(&Row::Mailbox(View::Primary)));
        assert!(!rows.contains(&Row::Mailbox(View::AllMail)));
        assert!(rows.contains(&Row::Mailbox(View::Done)));
    }

    #[test]
    fn the_cursor_starts_on_what_is_open() {
        let one = ids(1);
        let several = ids(2);
        assert_eq!(
            sidebar(&one, &[], View::Drafts).here(),
            Row::Mailbox(View::Drafts)
        );
        assert_eq!(
            sidebar(&several, &[], View::AllInboxes).here(),
            Row::AllInboxes
        );
        assert_eq!(sidebar(&one, &[], View::Search).here(), Row::Search);
        for (accounts, mailbox) in [(&one, View::Starred), (&several, View::AllInboxes)] {
            let s = sidebar(accounts, &[], mailbox);
            assert!(s.rows().contains(&s.here()));
        }
    }

    #[test]
    fn numbers_go_to_all_inboxes_then_each_account() {
        let several = ids(2);
        assert_eq!(
            sidebar(&several, &[], View::Inbox).numbered(),
            [
                Row::AllInboxes,
                Row::Account(String::new()),
                Row::Account("a1".into())
            ]
        );
        let one = ids(1);
        assert_eq!(
            sidebar(&one, &[], View::Inbox).numbered(),
            [Row::Mailbox(View::Inbox)]
        );
    }

    #[test]
    fn every_tab_has_rows_and_each_row_a_key() {
        assert_eq!(SHORTCUT_TABS.len(), 4);
        for (name, rows) in SHORTCUT_TABS {
            assert!(!rows.is_empty(), "{name}");
            for (caps, action) in *rows {
                assert!(caps.iter().any(|c| matches!(c, Cap::Key(_))), "{action}");
            }
        }
    }

    #[test]
    fn steps_stop_at_the_ends() {
        assert_eq!(step(0, -1, 5), 0);
        assert_eq!(step(4, 1, 5), 4);
        assert_eq!(step(2, 1, 5), 3);
        // A cursor left past the end (rows went away) comes back in.
        assert_eq!(step(9, 0, 5), 4);
        assert_eq!(step(3, 1, 0), 0);
    }

    #[test]
    fn shift_picks_from_the_open_row_to_the_moving_end() {
        assert_eq!(extend(2, 2, 1, 6), (3, vec![2, 3]));
        assert_eq!(extend(2, 3, 1, 6), (4, vec![2, 3, 4]));
        // Back past the open row, it picks upwards instead.
        assert_eq!(extend(2, 2, -1, 6), (1, vec![1, 2]));
        assert_eq!(extend(2, 3, -1, 6), (2, vec![2]));
        assert_eq!(extend(5, 5, 1, 6), (5, vec![5]));
    }
}
