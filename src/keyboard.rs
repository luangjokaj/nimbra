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
    /// Under All inboxes (several accounts): the mailboxes are every account's
    /// (`View::every`), and the labels are every account's by name.
    pub every: bool,
    /// The account has its own servers (IMAP): folders, and none of Gmail's categories or
    /// Important. Under All inboxes: every account has.
    pub folders: bool,
    pub labels: usize,
    /// The labels' ids, by index (`View::Label(i)` is `label_ids[i]`), for their order.
    /// Under All inboxes, their names in lower case.
    pub label_ids: Vec<String>,
    /// The folded sections ("accounts", "split", "mailbox", "labels").
    pub collapsed: &'a [String],
    /// The order dragged into each section, by section key ("split", "mailbox", and
    /// "labels:<account>"): mailbox names (`view_name`) or label ids.
    pub order: &'a std::collections::HashMap<String, Vec<String>>,
    /// Mailboxes hidden from the sidebar, by name.
    pub hidden: &'a [String],
    /// Sections showing their hidden mailboxes for now ("split", "mailbox").
    pub revealing: &'a [&'static str],
}

/// A mailbox's name in the settings (its place in the sidebar, or that it is hidden).
/// Labels go by their ids instead.
pub fn view_name(view: View) -> Option<&'static str> {
    Some(match view {
        View::Inbox => "inbox",
        View::Primary => "primary",
        View::Updates => "updates",
        View::Promotions => "promotions",
        View::Social => "social",
        View::Starred => "starred",
        View::Important => "important",
        View::Snoozed => "snoozed",
        View::Reminders => "reminders",
        View::Drafts => "drafts",
        View::Sent => "sent",
        View::Done => "done",
        View::AllMail => "all-mail",
        View::Spam => "spam",
        View::Trash => "trash",
        View::Every(_) | View::Search | View::Label(_) => return None,
    })
}

/// `defaults` in the order dragged (`order`, by name); any not named keep their default
/// order after those that are.
pub fn arrange(
    defaults: &[View],
    order: &[String],
    name: impl Fn(View) -> Option<String>,
) -> Vec<View> {
    let mut placed: Vec<View> = order
        .iter()
        .filter_map(|wanted| {
            defaults
                .iter()
                .copied()
                .find(|view| name(*view).as_deref() == Some(wanted.as_str()))
        })
        .collect();
    placed.dedup();
    for view in defaults {
        if !placed.contains(view) {
            placed.push(*view);
        }
    }
    placed
}

/// The most labels the sidebar lists.
pub use nimbra_core::listing::LABELS_SHOWN;

impl Sidebar<'_> {
    pub fn several(&self) -> bool {
        !self.demo && self.accounts.len() >= 2
    }

    /// The section's mailboxes in their default order. Under All inboxes, every account's,
    /// without the inbox and the Split Inbox: All inboxes is every account's inbox.
    fn defaults(&self, section: &str) -> Vec<View> {
        let views = self.own_defaults(section);
        if !self.every {
            return views;
        }
        if section == "split" {
            return Vec::new();
        }
        views
            .into_iter()
            .filter(|view| *view != View::Inbox)
            .map(View::every)
            .collect()
    }

    fn own_defaults(&self, section: &str) -> Vec<View> {
        match section {
            "split" if self.demo => vec![View::Inbox],
            "split" => nimbra_core::listing::split_mailboxes(self.folders),
            "mailbox" if self.demo => vec![
                View::Starred,
                View::Snoozed,
                View::Reminders,
                View::Drafts,
                View::Sent,
                View::Done,
            ],
            "mailbox" => nimbra_core::listing::mailboxes(self.folders),
            // Every account's labels by name (folders too) are listed under All inboxes.
            "labels" if self.demo || (self.folders && !self.every) => Vec::new(),
            "labels" => (0..self.labels.min(LABELS_SHOWN))
                .map(View::Label)
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The key the section's order is kept under: labels are each account's own.
    pub fn order_key(&self, section: &str) -> String {
        if section == "labels" && self.every {
            "labels:every".to_string()
        } else if section == "labels" {
            format!("labels:{}", self.shown)
        } else {
            section.to_string()
        }
    }

    fn name(&self, view: View) -> Option<String> {
        match view.base() {
            View::Label(index) => self.label_ids.get(index).cloned(),
            view => view_name(view).map(str::to_string),
        }
    }

    /// The section's mailboxes as the sidebar shows them: in the order dragged, without
    /// those hidden (unless the section is showing them). The open mailbox and the inbox
    /// are never hidden.
    pub fn section(&self, section: &'static str) -> Vec<View> {
        let order = self
            .order
            .get(&self.order_key(section))
            .map(Vec::as_slice)
            .unwrap_or_default();
        arrange(&self.defaults(section), order, |view| self.name(view))
            .into_iter()
            .filter(|view| {
                self.revealing.contains(&section) || !self.is_hidden(*view) || *view == self.mailbox
            })
            .collect()
    }

    /// Whether the mailbox is hidden (the inbox never is).
    pub fn is_hidden(&self, view: View) -> bool {
        let view = view.base();
        view != View::Inbox
            && view_name(view).is_some_and(|name| self.hidden.iter().any(|h| h == name))
    }

    /// How many of the section's mailboxes are hidden.
    pub fn hidden_in(&self, section: &str) -> usize {
        self.defaults(section)
            .into_iter()
            .filter(|view| self.is_hidden(*view))
            .count()
    }

    /// Top to bottom. A folded section still has the open mailbox, as on screen; the demo's
    /// sample rows, which do nothing, are left out.
    pub fn rows(&self) -> Vec<Row> {
        let open = |key: &str| !self.collapsed.iter().any(|k| k == key);
        let mut rows = vec![Row::Search];
        if self.several() {
            rows.push(Row::AllInboxes);
            for id in &self.accounts {
                if open("accounts") || (id == self.shown && !self.mailbox.is_every()) {
                    rows.push(Row::Account(id.clone()));
                }
            }
        }
        for key in ["split", "mailbox", "labels"] {
            for view in self.section(key) {
                if open(key) || view == self.mailbox {
                    rows.push(Row::Mailbox(view));
                }
            }
        }
        rows.push(Row::Settings);
        rows
    }

    /// The row of what is open, where the cursor starts.
    pub fn here(&self) -> Row {
        match self.mailbox {
            view if view.base() == View::Search => Row::Search,
            View::ALL_INBOXES if !self.several() => Row::Mailbox(View::Inbox),
            View::ALL_INBOXES => Row::AllInboxes,
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
            every: accounts.len() >= 2 && mailbox.is_every(),
            folders: false,
            labels: 3,
            label_ids: vec!["L0".into(), "L1".into(), "L2".into()],
            collapsed,
            order: &NO_ORDER,
            hidden: &[],
            revealing: &[],
        }
    }

    #[test]
    fn all_inboxes_lists_every_accounts_mailboxes() {
        let several = ids(2);
        let all = sidebar(&several, &[], View::ALL_INBOXES);
        let rows = all.rows();
        // The inbox is All inboxes itself; the rest are every account's.
        assert!(!rows.contains(&Row::Mailbox(View::Inbox)));
        assert!(!rows.contains(&Row::Mailbox(View::every(View::Inbox))));
        for view in [View::Trash, View::Sent, View::Important, View::Label(0)] {
            assert!(rows.contains(&Row::Mailbox(View::every(view))), "{view:?}");
            assert!(!rows.contains(&Row::Mailbox(view)), "{view:?}");
        }
        // No Split Inbox: All inboxes is every account's inbox.
        for view in [View::Primary, View::Updates, View::Promotions, View::Social] {
            assert!(!rows.contains(&Row::Mailbox(View::every(view))), "{view:?}");
            assert!(!rows.contains(&Row::Mailbox(view)), "{view:?}");
        }
        assert_eq!(all.here(), Row::AllInboxes);
        // Every account's Trash open: still every account's mailboxes, Trash the one here.
        let trash = sidebar(&several, &[], View::every(View::Trash));
        assert_eq!(trash.here(), Row::Mailbox(View::every(View::Trash)));
        assert!(
            trash
                .rows()
                .contains(&Row::Mailbox(View::every(View::Sent)))
        );
        // One account's own mailbox: its own again.
        let own = sidebar(&several, &[], View::Trash).rows();
        assert!(own.contains(&Row::Mailbox(View::Inbox)));
        assert!(own.contains(&Row::Mailbox(View::Sent)));
        // Hiding a mailbox hides every account's too.
        let hidden = ["sent".to_string()];
        let bar = Sidebar {
            hidden: &hidden,
            ..sidebar(&several, &[], View::ALL_INBOXES)
        };
        assert!(!bar.rows().contains(&Row::Mailbox(View::every(View::Sent))));
    }

    static NO_ORDER: std::sync::LazyLock<std::collections::HashMap<String, Vec<String>>> =
        std::sync::LazyLock::new(Default::default);

    #[test]
    fn mailboxes_follow_the_order_dragged_and_leave_out_the_hidden() {
        let accounts = ids(1);
        let mut order = std::collections::HashMap::new();
        order.insert("mailbox".to_string(), vec!["sent".into(), "starred".into()]);
        order.insert("labels:".to_string(), vec!["L2".into()]);
        let hidden = ["spam".to_string(), "inbox".to_string()];
        let bar = Sidebar {
            order: &order,
            hidden: &hidden,
            ..sidebar(&accounts, &[], View::Inbox)
        };
        let mailbox = bar.section("mailbox");
        assert_eq!(mailbox[..3], [View::Sent, View::Starred, View::Important]);
        assert!(!mailbox.contains(&View::Spam));
        assert_eq!(bar.hidden_in("mailbox"), 1);
        // The inbox is never hidden; labels have their own order.
        assert_eq!(bar.section("split")[0], View::Inbox);
        assert_eq!(
            bar.section("labels"),
            [View::Label(2), View::Label(0), View::Label(1)]
        );
        // Showing the hidden ones, or having one open, brings it back.
        let revealing = Sidebar {
            revealing: &["mailbox"],
            ..bar
        };
        assert!(revealing.section("mailbox").contains(&View::Spam));
        let open = Sidebar {
            mailbox: View::Spam,
            order: &order,
            hidden: &hidden,
            ..sidebar(&accounts, &[], View::Spam)
        };
        assert!(open.section("mailbox").contains(&View::Spam));
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
        let all = sidebar(&accounts, &collapsed, View::ALL_INBOXES).rows();
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
            sidebar(&several, &[], View::ALL_INBOXES).here(),
            Row::AllInboxes
        );
        assert_eq!(sidebar(&one, &[], View::Search).here(), Row::Search);
        for (accounts, mailbox) in [(&one, View::Starred), (&several, View::ALL_INBOXES)] {
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
