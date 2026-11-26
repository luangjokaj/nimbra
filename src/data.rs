use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use gpui::SharedString;

use crate::mail;
use crate::rich::{self, Block};
use crate::theme::{Accent, Tone};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Mail {
    /// Gmail thread id; `None` for the built-in demo data.
    pub id: Option<String>,
    pub sender: SharedString,
    pub initials: SharedString,
    pub tone: Tone,
    pub subject: SharedString,
    pub snippet: SharedString,
    pub time: SharedString,
    pub thread: Option<SharedString>,
    pub unread: bool,
    pub starred: bool,
    pub tag: Option<(SharedString, Accent)>,
    /// Not from today; shown under the "Earlier" group.
    pub yesterday: bool,
    /// AI summary. Empty when there is none (real mail has none yet).
    pub brief: SharedString,
    pub participants: SharedString,
    /// Older messages in the thread, shown collapsed above the latest one.
    pub earlier: Shared<Vec<Earlier>>,
    pub body: Shared<Vec<Block>>,
    /// The demo hero thread shows action items and attachments.
    pub rich: bool,
    /// Whether `body` and `earlier` have been fetched.
    pub loaded: bool,
    /// Who and what a reply answers (the latest message), once the thread is loaded.
    pub reply: Option<(String, mail::ReplyTo)>,
    /// The other recipients of the latest message, for reply-all.
    pub reply_all: Vec<String>,
    /// The latest message's original HTML, for "Show original".
    pub html: Option<SharedString>,
    /// When the latest message arrived (Gmail's internal date), so a stored thread can
    /// relabel its time ("9:41", "Wed") when the app reopens on another day.
    #[serde(default)]
    pub timestamp_ms: i64,
    /// Headers of the latest message (sender, recipients, date, attachments).
    #[serde(default)]
    pub header: Header,
    /// Gmail labels on the thread (for the Spam warning).
    #[serde(default)]
    pub labels: Vec<String>,
    /// The account this thread belongs to, when the list mixes accounts (All inboxes).
    #[serde(default)]
    pub account: Option<String>,
}

/// A thread's messages, shared between the copies of its row (the list, the cache of each
/// mailbox, what is being saved) instead of copied for each. Changed in place only where no
/// other copy has it; stored as the value itself.
#[derive(Default, Debug, PartialEq)]
pub struct Shared<T>(Arc<T>);

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Shared(self.0.clone())
    }
}

impl<T> Deref for Shared<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: Clone> DerefMut for Shared<T> {
    fn deref_mut(&mut self) -> &mut T {
        Arc::make_mut(&mut self.0)
    }
}

impl<T> From<T> for Shared<T> {
    fn from(value: T) -> Self {
        Shared(Arc::new(value))
    }
}

impl<'a, T> IntoIterator for &'a Shared<T>
where
    &'a T: IntoIterator,
{
    type Item = <&'a T as IntoIterator>::Item;
    type IntoIter = <&'a T as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        (&*self.0).into_iter()
    }
}

impl<T: serde::Serialize> serde::Serialize for Shared<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for Shared<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::deserialize(deserializer).map(Shared::from)
    }
}

/// What a message says about itself, for the header block in the reading pane.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Header {
    pub from: mail::Person,
    pub to: Vec<mail::Person>,
    pub cc: Vec<mail::Person>,
    pub reply_to: Option<mail::Person>,
    pub date_ms: i64,
    pub attachments: Vec<mail::Attachment>,
    pub unsubscribe: Option<String>,
    /// The RFC 5322 Message-ID, to reply to this message specifically.
    pub message_id: String,
}

impl Header {
    fn from_message(message: &mail::Message) -> Self {
        Header {
            from: message.from.clone(),
            to: message.to.clone(),
            cc: message.cc.clone(),
            reply_to: message.reply_to.clone(),
            date_ms: message.timestamp_ms,
            attachments: message.attachments.clone(),
            unsubscribe: message.unsubscribe.clone(),
            message_id: message.message_id.clone(),
        }
    }

    /// Who replies go to: Reply-To if set, else the sender.
    pub fn reply_address(&self) -> &str {
        self.reply_to
            .as_ref()
            .map_or(self.from.address.as_str(), |p| p.address.as_str())
    }

    /// Everyone on To and Cc, for reply-all.
    pub fn everyone(&self) -> Vec<String> {
        self.to
            .iter()
            .chain(&self.cc)
            .map(|p| p.address.clone())
            .collect()
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Earlier {
    pub tone: Tone,
    pub initials: SharedString,
    pub sender: SharedString,
    pub snippet: SharedString,
    pub when: SharedString,
    pub blocks: Vec<Block>,
    /// Collapsed to one line until clicked.
    pub expanded: bool,
    #[serde(default)]
    pub header: Header,
}

impl Mail {
    /// A list row for a real Gmail thread. The messages are filled in by [`Mail::fill`].
    pub fn from_summary(summary: mail::Summary) -> Self {
        let (time, earlier) = format_time(summary.timestamp_ms);
        Mail {
            account: None,
            id: Some(summary.id),
            initials: initials(&summary.sender).into(),
            tone: tone_for(&summary.sender),
            participants: summary.sender.clone().into(),
            sender: summary.sender.into(),
            subject: summary.subject.into(),
            snippet: summary.snippet.into(),
            time: time.into(),
            timestamp_ms: summary.timestamp_ms,
            thread: (summary.message_count > 1).then(|| summary.message_count.to_string().into()),
            unread: summary.unread,
            starred: summary.starred,
            labels: summary.labels.clone(),
            header: Header::default(),
            tag: None,
            yesterday: earlier,
            brief: "".into(),
            earlier: Shared::default(),
            body: Shared::default(),
            rich: false,
            loaded: false,
            reply: None,
            reply_all: Vec::new(),
            html: None,
        }
    }

    /// Recomputes the time label and the Today / Earlier grouping for a stored thread.
    pub fn refresh_time(&mut self) {
        if self.timestamp_ms > 0 {
            let (time, earlier) = format_time(self.timestamp_ms);
            self.time = time.into();
            self.yesterday = earlier;
        }
    }

    /// Fills in the thread once its messages (oldest first) have been fetched.
    pub fn fill(&mut self, mut messages: Vec<mail::Message>) {
        let count = messages.len();
        let mut senders: Vec<&str> = Vec::new();
        for message in &messages {
            if !senders.contains(&message.sender.as_str()) {
                senders.push(&message.sender);
            }
        }
        self.participants = if count > 1 {
            format!("{} · {count} messages", senders.join(", ")).into()
        } else {
            senders.join(", ").into()
        };
        if let Some(latest) = messages.pop() {
            self.header = Header::from_message(&latest);
            self.reply_all = latest.others.clone();
            self.reply = self.id.clone().map(|thread_id| {
                (
                    latest.address.clone(),
                    mail::ReplyTo {
                        thread_id,
                        message_id: latest.message_id.clone(),
                    },
                )
            });
            self.html = latest.html.map(Into::into);
            self.body = if latest.blocks.is_empty() {
                rich::from_text(vec![latest.snippet])
            } else {
                latest.blocks
            }
            .into();
        }
        self.earlier = messages
            .into_iter()
            .rev()
            .map(|message| Earlier {
                header: Header::from_message(&message),
                tone: tone_for(&message.sender),
                initials: initials(&message.sender).into(),
                when: format_time(message.timestamp_ms).0.into(),
                sender: message.sender.into(),
                snippet: message.snippet.into(),
                blocks: message.blocks,
                expanded: false,
            })
            .collect::<Vec<_>>()
            .into();
        self.loaded = true;
    }
}

/// Combines a freshly fetched mailbox listing with what is already in memory, so thread
/// contents that were already downloaded are not fetched again.
///
/// A thread keeps its cached messages only while its snippet and message count are
/// unchanged; a new reply changes both, which makes it load again.
pub fn merge(cached: &[Mail], fresh: Vec<Mail>) -> Vec<Mail> {
    // Thread ids are each account's own (All inboxes mixes accounts).
    let loaded: HashMap<(&str, Option<&str>), &Mail> = cached
        .iter()
        .filter(|old| old.loaded)
        .filter_map(|old| Some(((old.id.as_deref()?, old.account.as_deref()), old)))
        .collect();
    fresh
        .into_iter()
        .map(|mut mail| {
            let key = mail.id.as_deref().map(|id| (id, mail.account.as_deref()));
            if let Some(old) = key.and_then(|key| loaded.get(&key))
                && old.snippet == mail.snippet
                && old.thread == mail.thread
            {
                mail.body = old.body.clone();
                mail.earlier = old.earlier.clone();
                mail.participants = old.participants.clone();
                mail.reply = old.reply.clone();
                mail.header = old.header.clone();
                mail.reply_all = old.reply_all.clone();
                mail.html = old.html.clone();
                mail.loaded = true;
            }
            mail
        })
        .collect()
}

/// Up to two initials: "Mira Kovač" gives "MK", "stripe" gives "S".
pub fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|word| word.chars().find(|c| c.is_alphanumeric()))
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}

/// A stable avatar colour per sender.
pub fn tone_for(name: &str) -> Tone {
    const TONES: [Tone; 6] = [
        Tone::Violet,
        Tone::Rose,
        Tone::Amber,
        Tone::Mint,
        Tone::Sky,
        Tone::Graphite,
    ];
    let hash = name
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    TONES[hash as usize % TONES.len()]
}

/// A short local time label, and whether it falls before today.
fn format_time(timestamp_ms: i64) -> (String, bool) {
    use chrono::{Local, TimeZone};
    let Some(when) = Local.timestamp_millis_opt(timestamp_ms).single() else {
        return (String::new(), true);
    };
    let days = (Local::now().date_naive() - when.date_naive()).num_days();
    let label = match days {
        0 => when.format("%-H:%M"),
        1..=6 => when.format("%a"),
        _ => when.format("%b %-d"),
    };
    (label.to_string(), days != 0)
}

fn mail(
    sender: &'static str,
    initials: &'static str,
    tone: Tone,
    subject: &'static str,
    snippet: &'static str,
    time: &'static str,
) -> Mail {
    Mail {
        account: None,
        id: None,
        sender: sender.into(),
        initials: initials.into(),
        tone,
        subject: subject.into(),
        snippet: snippet.into(),
        time: time.into(),
        thread: None,
        unread: false,
        starred: false,
        tag: None,
        yesterday: false,
        brief: snippet.into(),
        participants: format!("{sender} and you").into(),
        earlier: Shared::default(),
        body: Shared::default(),
        rich: false,
        loaded: true,
        timestamp_ms: 0,
        header: Header::default(),
        labels: Vec::new(),
        reply: None,
        reply_all: Vec::new(),
        html: None,
    }
}

fn person(name: &str, address: &str) -> mail::Person {
    mail::Person {
        name: name.into(),
        address: address.into(),
    }
}

fn paragraphs(items: &[&'static str]) -> Vec<Block> {
    rich::from_text(items.iter().map(|p| p.to_string()).collect())
}

/// A formatted message for the sample data.
const SAMPLE_HTML: &str = r#"<html><head><meta name="color-scheme" content="light dark">
                <style>body{font-family:-apple-system,sans-serif;margin:24px}
                @media (prefers-color-scheme: dark){body{background:#22252a;color:#e6e6e6}a{color:#8ab4ff}}</style></head>
                <body><div><p>Hi Luan,</p>
                <p>Pulled a few references for the <b>aurora wallpaper</b> and the serif. The
                <i>italic accent</i> on the last words of a subject line is my favourite detail.</p>
                <h2>What I would keep</h2>
                <ul><li>The violet to rose gradient on the primary button</li>
                <li>Mono for timestamps and <code>keyboard hints</code></li>
                <li>One accent colour per label, nothing more</li></ul>
                <h3>Next steps</h3>
                <ol><li>Review the board: <a href="https://example.com/nimbra-moodboard">nimbra-moodboard</a></li>
                <li>Pick two directions for light mode</li></ol>
                <blockquote><p>Can the unread counts be a touch quieter?</p></blockquote>
                <p>I think that note from last week still stands.</p>
                <img src="https://example.com/aurora-reference.png" alt="Aurora reference" width="560">
                <table width="100%" cellpadding="8"><tr><th align="left">Status</th><th align="left">Direction</th><th align="right">Votes</th></tr>
                <tr><td>Kept</td><td><b>Aurora</b> gradient</td><td align="right">4</td></tr>
                <tr><td>Open</td><td><b>Paper</b> light mode</td><td align="right">2</td></tr></table>
                <hr><pre>--color-accent: #8b7cff;
--color-rose:   #ff7ab6;</pre>
                <p>Thanks,<br>Elena</p></div></body></html>"#;

pub fn inbox() -> Vec<Mail> {
    vec![
        Mail {
            thread: Some("3".into()),
            unread: true,
            tag: Some(("Needs reply".into(), Accent::Rose)),
            rich: true,
            header: Header {
                from: person("Mira Kovač", "mira@northwind.studio"),
                to: vec![person("Luan Gjokaj", "luan@example.com"), person("Jonah Reyes", "jonah@northwind.studio")],
                cc: vec![person("", "launch@northwind.studio")],
                reply_to: None,
                date_ms: 0,
                attachments: Vec::new(),
                unsubscribe: None,
                message_id: String::new(),
            },
            participants: "Mira, Jonah and you · 3 messages".into(),
            earlier: vec![
                Earlier {
                    tone: Tone::Mint,
                    initials: "JR".into(),
                    sender: "Jonah Reyes".into(),
                    snippet: "Numbers look right to me. Looping Luan in for final sign-off.".into(),
                    when: "Yesterday".into(),
                    blocks: paragraphs(&[
                        "Numbers look right to me. Looping Luan in for final sign-off.",
                        "The only open question on my side is the embargo date.",
                    ]),
                    expanded: false,
                    header: Header::default(),
                },
                Earlier {
                    tone: Tone::Violet,
                    initials: "MK".into(),
                    sender: "Mira Kovač".into(),
                    snippet: "First pass of the deck. Timeline still says Nov 28, fixing now.".into(),
                    when: "Tue".into(),
                    blocks: paragraphs(&[
                        "First pass of the deck. Timeline still says Nov 28, fixing now.",
                        "Draft is here: https://example.com/q4-launch-plan",
                    ]),
                    expanded: false,
                    header: Header::default(),
                },
            ].into(),
            brief: "Mira needs your sign-off by Friday. Public launch moves to Dec 9 so onboarding gets one more test round; budget is unchanged.".into(),
            body: paragraphs(&[
                "Hi Luan,",
                "Final deck is attached with the updated timeline. We pushed the public launch to December 9 so the new onboarding flow gets one more round of testing. Budget stays exactly where it was.",
                "Two things before Friday: the pricing tiers on slide 14, and a date for the press embargo.",
                "Thanks,\nMira",
            ]).into(),
            ..mail(
                "Mira Kovač",
                "MK",
                Tone::Violet,
                "Q4 launch plan, final review",
                "Attached the deck with the updated timeline. Can you sign off before Friday?",
                "9:41",
            )
        },
        Mail {
            unread: true,
            brief: "Your November payout of €24,180.00 lands on Nov 22. Nothing to do.".into(),
            body: paragraphs(&[
                "Hi Luan,",
                "Your payout of €24,180.00 is on its way and should arrive on November 22 in the account ending 4421.",
                "You can see the full breakdown in your dashboard.",
            ]).into(),
            ..mail(
                "Stripe",
                "S",
                Tone::Graphite,
                "November payout",
                "€24,180.00 arrives Nov 22 in the account ending 4421.",
                "9:12",
            )
        },
        Mail {
            thread: Some("5".into()),
            unread: true,
            brief: "Jonah finished the component audit. Two focus-ring edge cases in dark mode need a decision; everything else passes.".into(),
            body: paragraphs(&[
                "Hey,",
                "Went through Button and Input end to end. Two edge cases with focus rings in dark mode, otherwise solid.",
                "I can pair on it tomorrow morning if that works.",
            ]).into(),
            ..mail(
                "Jonah Reyes",
                "JR",
                Tone::Mint,
                "Component audit",
                "Two edge cases with focus rings in dark mode, otherwise solid.",
                "8:47",
            )
        },
        Mail {
            tag: Some(("Travel".into(), Accent::Sky)),
            brief: "Flights for the Lisbon offsite are booked. Everyone lands Thursday afternoon; hotel confirmation is attached.".into(),
            body: paragraphs(&[
                "Hi all,",
                "Flights are booked. Everyone lands Thursday afternoon and the hotel confirmation is inside.",
                "Dinner on the first night is at 8:30, a short walk from the hotel.",
            ]).into(),
            ..mail(
                "Ana Petrović",
                "AP",
                Tone::Amber,
                "Lisbon offsite",
                "Flights booked. Everyone lands Thursday afternoon.",
                "7:30",
            )
        },
        Mail {
            brief: "Three issues were assigned to you: fuzzy matching in the command palette, snooze presets, and a keyboard hint bug.".into(),
            body: paragraphs(&[
                "NIM-214 Command palette fuzzy matching",
                "NIM-219 Snooze presets",
                "NIM-223 Keyboard hints overlap on narrow windows",
            ]).into(),
            // An HTML message with no styling of its own, to check the web view's defaults.
            html: Some(
                "<p><b>3 issues</b> were assigned to you in <a href=\"https://example.com\">Nimbra</a>.</p>\
                 <ol><li><b>NIM-214.</b> Command palette fuzzy matching, see <code>palette.rs</code></li>\
                 <li><b>NIM-219.</b> Snooze presets</li><li><b>NIM-223.</b> Keyboard hints overlap on narrow windows</li></ol>\
                 <p>Reply to this email directly or view it in Linear.</p>"
                    .into(),
            ),
            ..mail(
                "Linear",
                "L",
                Tone::Sky,
                "3 issues assigned",
                "NIM-214 Command palette fuzzy matching, NIM-219 Snooze presets",
                "7:02",
            )
        },
        Mail {
            thread: Some("2".into()),
            yesterday: true,
            // An HTML message, to exercise the rich-text viewer.
            body: rich::from_html(SAMPLE_HTML).into(),
            html: Some(SAMPLE_HTML.into()),
            ..mail(
                "Elena Brooks",
                "EB",
                Tone::Rose,
                "Nimbra moodboard",
                "Pulled a few references for the aurora wallpaper and the serif.",
                "Wed",
            )
        },
        Mail {
            yesterday: true,
            body: paragraphs(&["Clause 4.2 is updated per legal. Ready to countersign when you are."]).into(),
            ..mail(
                "Daniel Osei",
                "DO",
                Tone::Graphite,
                "Contract v3",
                "Clause 4.2 updated per legal. Ready to countersign when you are.",
                "Wed",
            )
        },
        Mail {
            yesterday: true,
            body: paragraphs(&["“Can the unread counts be a touch quieter?”"]).into(),
            ..mail(
                "Figma",
                "F",
                Tone::Violet,
                "Elena mentioned you",
                "“Can the unread counts be a touch quieter?”",
                "Wed",
            )
        },
        Mail {
            yesterday: true,
            body: paragraphs(&["Design crit is now Thursday 2:00 PM in Studio B. Agenda attached."]).into(),
            ..mail(
                "Google Calendar",
                "GC",
                Tone::Sky,
                "Design crit moved",
                "Now Thursday 2:00 PM, Studio B. Agenda attached.",
                "Wed",
            )
        },
        Mail {
            header: Header {
                from: person("Notion", "notify@mail.notion.so"),
                to: vec![person("", "luan@example.com")],
                unsubscribe: Some("https://example.com/unsubscribe".into()),
                ..Header::default()
            },
            yesterday: true,
            body: paragraphs(&["12 pages updated and 4 new comments in Roadmap 2027."]).into(),
            ..mail(
                "Notion",
                "N",
                Tone::Amber,
                "Weekly digest: everything that changed across the Design workspace this week",
                "12 pages updated, 4 new comments in Roadmap 2027.",
                "Tue",
            )
        },
    ]
}

/// (icon, label, hint, key, section header shown above this row)
pub const COMMANDS: &[(&str, &str, &str, &str, Option<&str>)] = &[
    (
        "sparkles",
        "Snooze until I’m free",
        "After Design crit · Thu 3:30 PM",
        "↵",
        Some("SMART SNOOZE"),
    ),
    (
        "bell",
        "Until Mira replies",
        "Returns to Important automatically",
        "R",
        None,
    ),
    ("clock", "Later today", "Thu 6:00 PM", "H", Some("SNOOZE")),
    ("clock", "Tomorrow morning", "Fri 9:00 AM", "T", None),
    ("calendar", "Next week", "Mon 9:00 AM", "W", None),
    (
        "zap",
        "Pick a date & time…",
        "Try “next tue 3pm”",
        "/",
        None,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(id: &str, snippet: &'static str) -> Mail {
        Mail {
            id: Some(id.into()),
            loaded: false,
            ..mail("Ana", "A", Tone::Mint, "Subject", snippet, "9:00")
        }
    }

    #[test]
    fn merge_keeps_downloaded_threads_and_refetches_changed_ones() {
        let mut same = thread("a", "hello");
        same.body = rich::from_text(vec!["Downloaded body".into()]).into();
        same.loaded = true;
        let mut changed = thread("b", "old reply");
        changed.body = rich::from_text(vec!["Stale body".into()]).into();
        changed.loaded = true;
        let cached = vec![same, changed];

        let merged = merge(
            &cached,
            vec![
                thread("c", "brand new"),
                thread("a", "hello"),
                thread("b", "new reply"),
            ],
        );

        // Order follows the fresh listing.
        let ids: Vec<_> = merged.iter().map(|m| m.id.clone().unwrap()).collect();
        assert_eq!(ids, ["c", "a", "b"]);
        // Unchanged thread keeps its body; new and changed ones will load again.
        assert!(merged[1].loaded);
        assert_eq!(merged[1].body[0].text, "Downloaded body");
        assert!(!merged[0].loaded && merged[0].body.is_empty());
        assert!(!merged[2].loaded && merged[2].body.is_empty());
    }

    #[test]
    fn merge_keeps_each_accounts_thread_apart() {
        let mut mine = thread("a", "hello");
        mine.body = rich::from_text(vec!["Mine".into()]).into();
        mine.loaded = true;
        let mut fresh_theirs = thread("a", "hello");
        fresh_theirs.account = Some("bbbbbbbbbbbb".into());
        let merged = merge(&[mine], vec![fresh_theirs, thread("a", "hello")]);
        assert!(
            !merged[0].loaded,
            "another account's thread with the same id"
        );
        assert_eq!(merged[1].body[0].text, "Mine");
    }

    #[test]
    fn a_copied_row_shares_its_messages_until_one_copy_changes_them() {
        let mut row = thread("a", "hello");
        row.body = rich::from_text(vec!["Body".into()]).into();
        let copy = row.clone();
        assert!(std::ptr::eq(row.body.as_ptr(), copy.body.as_ptr()));
        row.body[0].text = "Changed".into();
        assert_eq!(copy.body[0].text, "Body");
    }

    #[test]
    fn rows_stored_before_messages_were_shared_still_load() {
        let mut row = thread("a", "hello");
        row.body = rich::from_text(vec!["Body".into()]).into();
        let stored = serde_json::to_value(&row).unwrap();
        assert!(stored["body"].is_array(), "stored as the list itself");
        let back: Mail = serde_json::from_value(stored).unwrap();
        assert_eq!(back.body[0].text, "Body");
    }
}
