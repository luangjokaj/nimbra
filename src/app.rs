use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Context, Div, Entity, FocusHandle,
    Focusable as _, FontStyle, FontWeight, HighlightStyle, Hsla, MouseButton, SharedString,
    Stateful, StyledText, Transformation, Window, actions, div, ease_out_quint, percentage,
    prelude::*, px, relative,
};

use crate::arrival::{Arrival, drop_for};
use crate::checking::{Asked, Checks, TooSlow, within};
use crate::data::{COMMANDS, Mail, inbox};
use crate::gmail::{self, View};
use crate::input::{InputEvent, TextInput};
use crate::leaving::{self, End, Leaving, SavedAs, Steps, Underway};
use crate::rich::{self, Block, Kind};
use crate::scheduled::{self, Outcome, Scheduled};
use crate::sending::{Job, QueuedMail, Stage};
use crate::theme::{Accent, SERIF, Theme, Tone, c, fade, gradient};
use crate::toast::{self, Toast};
use crate::ui::{
    ButtonKind, Press, TOP_BAR, avatar, button, icon, icon_button, kbd, meta, mono, overline, pill,
    shadow, tip,
};
use crate::webview::HtmlView;

#[path = "message_window.rs"]
mod message_window;
use message_window::MessageWindow;

#[path = "skeleton.rs"]
mod skeleton;

#[path = "settings_window.rs"]
mod settings_window;
use settings_window::SettingsWindow;

actions!(
    nimbra,
    [
        Next,
        Prev,
        MarkDone,
        Undo,
        TogglePalette,
        Dismiss,
        Confirm,
        ToggleTheme,
        ToggleSidebar,
        ToggleList,
        ToggleStar,
        MarkUnread,
        Trash,
        Compose,
        Reply,
        ReplyAll,
        ForwardMessage,
        SendMail,
        SaveDraft,
        ToggleFullCompose,
        ToggleOriginal,
        FocusSearch,
        HideWindow,
        Back,
        Forward,
        Reload,
        OpenSettings,
        SelectAllThreads,
        Print,
        HideApp,
        HideOthers,
        ShowAll,
        Minimize,
        // Menu-bar twins of actions whose shortcut is a bare key (E, R, S…). A menu item
        // takes its action's shortcut as a system key equivalent, which would fire while
        // typing; these have no shortcut of their own.
        MenuReply,
        MenuReplyAll,
        MenuForwardMessage,
        MenuMarkDone,
        MenuMarkUnread,
        MenuStar,
        MenuTrash,
        MenuSnooze,
        MenuUndo,
        MenuToggleOriginal,
        MenuToggleSidebar,
        MenuSearch,
        Quit
    ]
);

/// Where the app is in connecting to a mailbox.
#[derive(Clone, PartialEq)]
enum Status {
    /// Built-in sample data, no network.
    Demo,
    SignedOut,
    SigningIn,
    Loading,
    Ready,
    Failed(SharedString),
}

/// The open composer: two single-line fields and a body made of one field per line.
struct Composer {
    /// Tells composers apart while one is still finishing in the background.
    serial: u64,
    to: Entity<TextInput>,
    cc: Entity<TextInput>,
    bcc: Entity<TextInput>,
    /// The Cc and Bcc fields are shown (after "Cc Bcc" is clicked, or when they have text).
    show_cc: bool,
    /// Finished addresses in To, Cc and Bcc, shown as pills; the fields hold only what is
    /// still being typed.
    chips: [Vec<String>; 3],
    /// Address suggestions under a recipient field.
    suggest: Option<Suggest>,
    /// Files picked or dropped in.
    files: Vec<gmail::OutgoingFile>,
    /// Files already attached to the draft being edited.
    kept: Vec<gmail::Attachment>,
    /// Counts edits; the draft is saved a moment after the last one.
    edits: u64,
    /// The edit count the saved draft has.
    saved_edits: u64,
    /// A save was asked for while one was running.
    resave: bool,
    /// What to do once the draft is saved.
    after_save: Option<After>,
    /// Written in the formatted editor (a web view) rather than the plain line fields.
    rich: bool,
    /// The editor's content, as HTML and as plain text.
    html: String,
    text: String,
    /// Content still to be put into the editor.
    load: Option<String>,
    /// Bold, italic, underline, bullets, numbers at the cursor.
    format: [bool; 5],
    /// Put the keyboard in the message body once the editor shows it.
    focus_body: bool,
    subject: Entity<TextInput>,
    lines: Vec<Entity<TextInput>>,
    reply: Option<gmail::ReplyTo>,
    /// The Gmail draft being edited, if this message was saved or opened from Drafts.
    draft_id: Option<String>,
    /// The account it is written from, when not the one shown (a reply in All inboxes).
    account: Option<String>,
    /// The earlier message a draft quotes or forwards. Kept out of the editor behind a
    /// "…" toggle and added back when the message is saved or sent.
    quoted: Option<String>,
    show_quoted: bool,
    saving: bool,
    /// What the last save left in Gmail, set by the thread that saved it (for quitting).
    saved_as: SavedAs,
    /// The last save succeeded (shown in the footer).
    saved: bool,
    /// Fills the window instead of floating over the mail.
    full: bool,
    /// Top-left corner of the floating card; `None` until it is first laid out, centred.
    position: Option<(f32, f32)>,
    size: (f32, f32),
    /// An in-progress move or resize: what, where the mouse started, and the card's
    /// position and size at that moment.
    grab: Option<(Grab, (f32, f32), (f32, f32), (f32, f32))>,
    sending: bool,
    error: Option<SharedString>,
    _subscriptions: Vec<gpui::Subscription>,
}

#[derive(Clone, Copy, PartialEq)]
enum ComposeKind {
    New,
    Reply,
    ReplyAll,
    Forward,
}

/// What a row's right-click menu can do.
#[derive(Clone, Copy, PartialEq)]
enum RowAction {
    MarkRead,
    MarkUnread,
    Star,
    Unstar,
    Done,
    MoveToInbox,
    Trash,
    DeleteForever,
    ReportSpam,
    OpenInGmail,
}

impl RowAction {
    /// Icon and label for the menu.
    fn label(self) -> (&'static str, &'static str) {
        match self {
            RowAction::MarkRead => ("mail", "Mark as read"),
            RowAction::MarkUnread => ("mail", "Mark as unread"),
            RowAction::Star => ("star", "Star"),
            RowAction::Unstar => ("star", "Remove star"),
            RowAction::Done => ("done", "Mark done"),
            RowAction::MoveToInbox => ("inbox", "Move to inbox"),
            RowAction::Trash => ("trash", "Move to trash"),
            RowAction::DeleteForever => ("trash", "Delete permanently"),
            RowAction::ReportSpam => ("bell", "Report spam"),
            RowAction::OpenInGmail => ("link", "Open in Gmail"),
        }
    }
}

/// A small menu dropped from the reading toolbar.
#[derive(Clone, Copy, PartialEq)]
enum Menu {
    Labels,
    More,
    /// Times to send the open composer's message at.
    SendLater,
    /// Inbox, a label or Spam, for the open thread.
    MoveTo,
    /// Rename or delete a label (right-click in the sidebar), by index.
    Label(usize),
    /// The right-click menu of a row in the list (by index into the loaded threads).
    Row(usize),
}

/// A thread put away until later (snooze), or until later unless someone replies
/// (a reminder). Kept on this Mac: Gmail's API has no snooze.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Snooze {
    thread_id: String,
    /// When it comes back (or, for a reminder, when to check for a reply).
    until_ms: i64,
    /// Only come back if nobody has written since.
    no_reply: bool,
    /// The time of the thread's latest message when it was put away.
    since_ms: i64,
    /// The row, to list it under Snoozed.
    mail: Mail,
}

/// One choice in the snooze palette.
#[derive(Clone)]
struct SnoozeChoice {
    icon: &'static str,
    label: String,
    hint: String,
    header: Option<&'static str>,
    until_ms: i64,
    no_reply: bool,
}

/// Something that needs a yes before it happens.
#[derive(Clone)]
enum Ask {
    /// Delete these threads for good.
    DeleteForever(Vec<String>),
    /// Ask Google for permanent deletion, then delete these threads.
    GrantFullAccess(Vec<String>),
    /// Delete a label (id, name); the mail stays.
    DeleteLabel(String, String),
}

/// What the label sheet does with the name typed in it.
#[derive(Clone, PartialEq)]
enum LabelSheet {
    /// Create a label; `apply` also puts it on the open thread.
    New { apply: bool },
    /// Rename the label with this id.
    Rename(String),
}

/// One result in the search panel.
#[derive(Clone)]
struct Hit {
    id: Option<String>,
    sender: SharedString,
    initials: SharedString,
    tone: Tone,
    subject: SharedString,
    snippet: SharedString,
    time: SharedString,
    unread: bool,
    timestamp_ms: i64,
    /// The conversation itself, so clicking it can open it straight away.
    mail: Box<Mail>,
}

impl Hit {
    fn from_mail(mail: &Mail) -> Self {
        Hit {
            id: mail.id.clone(),
            sender: mail.sender.clone(),
            initials: mail.initials.clone(),
            tone: mail.tone,
            subject: mail.subject.clone(),
            snippet: mail.snippet.clone(),
            time: mail.time.clone(),
            unread: mail.unread,
            timestamp_ms: mail.timestamp_ms,
            mail: Box::new(mail.clone()),
        }
    }
}

/// Search tips shown while the search box is empty: what to type, and what it finds.
const SEARCH_TIPS: &[(&str, &str)] = &[
    ("from:", "From someone"),
    ("has:attachment", "With attachments"),
    ("is:unread", "Unread only"),
    ("newer_than:7d", "From the last week"),
];

/// Where Move to sends the open thread.
#[derive(Clone, PartialEq)]
enum MoveTarget {
    Inbox,
    Label(String),
    Spam,
}

/// What the mouse is doing to the floating composer.
#[derive(Clone, Copy, PartialEq)]
enum Grab {
    Move,
    /// Resizing from an edge or corner: which way each side moves (-1 the left or top
    /// edge, 1 the right or bottom one, 0 that axis stays).
    Resize(i8, i8),
}

const COMPOSER_MIN: (f32, f32) = (460., 320.);

/// Address suggestions for one recipient field.
struct Suggest {
    field: Field,
    people: Vec<gmail::Person>,
    selected: usize,
}

/// Something to do once a composer's draft is saved.
#[derive(Clone, Copy, PartialEq)]
enum After {
    /// Send it at this time (milliseconds since the epoch).
    Schedule(i64),
}

/// A composer that is off screen but still has work to finish.
#[derive(Clone, Copy, PartialEq)]
enum Parked {
    /// Closed; kept until its last edits are saved.
    Closing,
    /// Sent, waiting out the undo delay.
    Sending,
    /// Being sent now; brought back if sending fails.
    InFlight,
    /// Discarded, waiting out the undo delay before its draft is deleted.
    Discarded,
}

/// Changes and messages made while offline, sent once Gmail can be reached again.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct Outbox {
    pub(crate) changes: Vec<(String, gmail::Change)>,
    /// Only as earlier versions saved it: waiting mail has records of its own now (see
    /// `sending`), and these become such records when the store opens.
    #[serde(default)]
    mail: Vec<QueuedMail>,
}

/// What a catch-up got done.
#[derive(Default)]
struct CaughtUp {
    changes: usize,
    sent: usize,
    /// Not sent by itself: the subject, whether it may have arrived all the same, and
    /// whether Drafts has it.
    kept: Vec<(String, bool, bool)>,
}

/// Sends what waited for the connection, in order. Blocking.
fn catch_up(
    client: &gmail::Client,
    changes: &[(String, gmail::Change)],
    queue: &[Arc<Job>],
) -> anyhow::Result<CaughtUp> {
    // Is Gmail reachable again?
    client.profile()?;
    let mut done = CaughtUp::default();
    for (id, change) in changes {
        match client.change(id, *change) {
            Err(error) if error.is::<gmail::Offline>() => return Ok(done),
            // Anything else (the thread is gone, say) is dropped.
            _ => done.changes += 1,
        }
    }
    for job in queue {
        match job.send() {
            None => {}
            Some(Ok(())) => done.sent += 1,
            Some(Err(_)) => {
                let unsure = match job.stage() {
                    Stage::Held => break, // still offline
                    stage => stage == Stage::Unsure,
                };
                done.kept.push((job.subject(), unsure, job.to_drafts()));
            }
        }
    }
    Ok(done)
}

/// How long a sent or discarded message can still be taken back.
const UNDO_DELAY: Duration = Duration::from_secs(6);
/// The longest quitting waits for what is on its way to Gmail. That is on disk as well.
const QUIT_WAIT: Duration = Duration::from_secs(30);
const KEYCHAIN_REFUSED: &str = "Nimbra could not read your sign-in from the Keychain. Nothing was deleted and you are still signed in. Try again, and allow access if macOS asks.";
const STORE_UNAVAILABLE: &str = "Nimbra could not open what it keeps on this Mac. Nothing was deleted. Allow Keychain access and reopen Nimbra.";

impl Composer {
    fn input(&self, field: Field) -> Option<Entity<TextInput>> {
        match field {
            Field::To => Some(self.to.clone()),
            Field::Cc => Some(self.cc.clone()),
            Field::Bcc => Some(self.bcc.clone()),
            Field::Subject => Some(self.subject.clone()),
            Field::Line(index) => self.lines.get(index).cloned(),
        }
    }

    /// Where a recipient field keeps its pills.
    fn slot(field: Field) -> Option<usize> {
        match field {
            Field::To => Some(0),
            Field::Cc => Some(1),
            Field::Bcc => Some(2),
            _ => None,
        }
    }

    /// A recipient field's pills and typed text, as one comma-separated list.
    fn recipients(&self, field: Field, cx: &App) -> String {
        let (Some(slot), Some(input)) = (Self::slot(field), self.input(field)) else {
            return String::new();
        };
        let typed = input.read(cx).text();
        self.chips[slot]
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(typed.trim()).filter(|t| !t.is_empty()))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Turns the finished addresses typed in a recipient field into pills. `all` also takes
    /// the part after the last comma (on Tab, Enter, or when filled in by the app). Anything
    /// that is not a valid address stays as text, to be corrected.
    fn chipify(&mut self, field: Field, all: bool, cx: &mut App) {
        let (Some(slot), Some(input)) = (Self::slot(field), self.input(field)) else {
            return;
        };
        let text = input.read(cx).text();
        let mut parts: Vec<&str> = text.split([',', ';']).collect();
        let last = if all { "" } else { parts.pop().unwrap_or("") };
        let mut keep: Vec<String> = Vec::new();
        for part in parts {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            match gmail::addresses(part) {
                Ok(found) if !found.is_empty() => {
                    for address in found {
                        if !self.chips[slot]
                            .iter()
                            .any(|c| c.eq_ignore_ascii_case(&address))
                        {
                            self.chips[slot].push(address);
                        }
                    }
                }
                _ => keep.push(part.to_string()),
            }
        }
        let mut rest = keep.join(", ");
        if !last.trim().is_empty() {
            if !rest.is_empty() {
                rest.push_str(", ");
            }
            rest.push_str(last.trim_start());
        }
        if rest != text {
            input.update(cx, |input, cx| input.set(rest, usize::MAX, cx));
        }
        self.sync_placeholder(field, cx);
    }

    /// The To field's hint shows only while it has no pills.
    fn sync_placeholder(&self, field: Field, cx: &mut App) {
        if let (Field::To, Some(input)) = (field, self.input(field)) {
            let hint = if self.chips[0].is_empty() {
                "name@example.com"
            } else {
                ""
            };
            input.update(cx, |input, cx| input.set_placeholder(hint, cx));
        }
    }

    /// The message as it would be saved or sent.
    fn outgoing(&self, cx: &App) -> gmail::Outgoing {
        let html = self
            .rich
            .then(|| crate::editor::message_html(&self.html, self.quoted.as_deref()));
        gmail::Outgoing {
            to: self.recipients(Field::To, cx),
            cc: self.recipients(Field::Cc, cx),
            bcc: self.recipients(Field::Bcc, cx),
            subject: self.subject.read(cx).text(),
            body: self.body(cx),
            html,
            files: self.files.clone(),
            kept: self.kept.clone(),
            reply: self.reply.clone(),
        }
    }

    /// Anything typed or attached.
    fn has_content(&self, cx: &App) -> bool {
        (self.rich && !self.text.trim().is_empty())
            || self.chips.iter().any(|c| !c.is_empty())
            || !self.files.is_empty()
            || !self.kept.is_empty()
            || [&self.to, &self.cc, &self.bcc, &self.subject]
                .iter()
                .chain(self.lines.iter().collect::<Vec<_>>().iter())
                .any(|input| !input.read(cx).text().trim().is_empty())
    }

    fn dirty(&self) -> bool {
        self.edits != self.saved_edits
    }

    /// The whole message text: the edited lines, then the quoted earlier message.
    fn body(&self, cx: &App) -> String {
        let mut body = if self.rich {
            self.text.trim_end().to_string()
        } else {
            self.lines
                .iter()
                .map(|line| line.read(cx).text())
                .collect::<Vec<_>>()
                .join("\n")
        };
        if let Some(quoted) = &self.quoted {
            body.push_str("\n\n");
            body.push_str(quoted);
        }
        body
    }
}

/// Pull-to-refresh distances, in points: where a release triggers a refresh, where the
/// indicator rests while refreshing, and the distance the resistance is scaled to.
const PULL_TRIGGER: f32 = 64.;
const PULL_REST: f32 = 46.;
const PULL_LIMIT: f32 = 420.;

/// Rubber-band resistance: the further the pull, the less the list follows.
fn rubber_band(pull: f32) -> f32 {
    (1. - 1. / (pull * 0.55 / PULL_LIMIT + 1.)) * PULL_LIMIT
}

/// Where the indicator is, `start.elapsed()` after being let go.
///
/// Settling at the "checking" height uses a critically damped spring (no overshoot).
/// Going back up (to 0) is a short, plain ease-out: quick, with no spring tail.
fn settle(from: f32, to: f32, start: std::time::Instant) -> f32 {
    let t = start.elapsed().as_secs_f32();
    if to == 0. {
        let p = (t / CLOSE_SECONDS).min(1.);
        return from * (1. - p).powi(3);
    }
    if t >= SETTLE_SECONDS {
        return to;
    }
    let k = 13. * t;
    to + (from - to) * (-k).exp() * (1. + k)
}

/// How long the pull-to-refresh strip takes to close.
const CLOSE_SECONDS: f32 = 0.22;

const SETTLE_SECONDS: f32 = 0.7;

const MENU_WIDTH: f32 = 220.;
/// Transparent space around the menu inside its window, so its shadow is not cut off.
const MENU_MARGIN: f32 = 28.;

/// A toolbar menu, shown in a small window of its own (see `Nimbra::toggle_menu`).
struct MenuPopup {
    owner: gpui::WeakEntity<Nimbra>,
    menu: Menu,
    items: Vec<(&'static str, SharedString, Hsla)>,
    /// False for a menu that only carries a message (no labels yet).
    enabled: bool,
    dark: bool,
    was_active: bool,
    opened: std::time::Instant,
    _activation: gpui::Subscription,
}

/// A short message at the bottom of the window (see `Nimbra::sync_notice_window`).
struct NoticeToast {
    owner: gpui::WeakEntity<Nimbra>,
    toast: Toast,
    dark: bool,
}

impl Render for NoticeToast {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        let owner = self.owner.clone();
        div()
            .size_full()
            .p(px(MENU_MARGIN))
            .font_family(crate::theme::SANS)
            .text_size(px(13.))
            .line_height(px(18.))
            .text_color(t.text)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(toast::GAP))
                    .size_full()
                    .pl(px(toast::LEFT))
                    .pr(px(toast::RIGHT))
                    .rounded(px(12.))
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![shadow(t.shadow, 12., 32., -6.)])
                    // As wide as it was measured, so that it wraps as it was measured.
                    .child(
                        div()
                            .flex_none()
                            .w(px(self.toast.text_width))
                            .text_size(px(toast::FONT))
                            .line_height(px(toast::LINE))
                            .child(self.toast.text.clone()),
                    )
                    .child(
                        button(&t, ButtonKind::Secondary, "Dismiss", None)
                            .id("dismiss-notice")
                            .justify_center()
                            .w(px(toast::DISMISS.0))
                            .h(px(toast::DISMISS.1))
                            .press_soft()
                            .on_click(cx.listener(move |_, _, _, cx| {
                                owner
                                    .update(cx, |owner, cx| {
                                        owner.notice = None;
                                        cx.notify();
                                    })
                                    .ok();
                            })),
                    ),
            )
    }
}

/// The "Undo" pill shown after a thread is removed (see `Nimbra::offer_undo`).
struct UndoToast {
    owner: gpui::WeakEntity<Nimbra>,
    label: &'static str,
    dark: bool,
}

impl Render for UndoToast {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        let owner = self.owner.clone();
        div()
            .size_full()
            .p(px(MENU_MARGIN))
            .font_family(crate::theme::SANS)
            .text_size(px(13.))
            .line_height(px(18.))
            .text_color(t.text)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .size_full()
                    .pl(px(16.))
                    .pr(px(6.))
                    .rounded(px(12.))
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![shadow(t.shadow, 12., 32., -6.)])
                    .child(div().flex_1().child(self.label))
                    .child(
                        button(&t, ButtonKind::Ghost, "Undo", Some("⌘Z"))
                            .id("undo")
                            .press_soft()
                            .on_click(cx.listener(move |_, _, _, cx| {
                                owner.update(cx, |owner, cx| owner.undo(cx)).ok();
                            })),
                    ),
            )
    }
}

impl MenuPopup {
    /// Closes the menu without choosing anything, and tells the owner so a click on the
    /// menu's own button closes it and does not reopen it straight away.
    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let menu = self.menu;
        self.owner
            .update(cx, |owner, _| {
                owner.menu_window = None;
                owner.menu_dismissed = Some((menu, std::time::Instant::now()));
            })
            .ok();
        window.remove_window();
    }
}

impl Render for MenuPopup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        let hover = t.hover;
        let enabled = self.enabled;
        div()
            .size_full()
            .p(px(MENU_MARGIN))
            .font_family(crate::theme::SANS)
            .text_size(px(13.))
            .line_height(px(18.))
            .text_color(t.text)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .p(px(6.))
                    .rounded(px(12.))
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![shadow(t.shadow, 10., 24., -6.)])
                    .children(
                        self.items
                            .iter()
                            .enumerate()
                            .map(|(index, (name, label, tint))| {
                                div()
                                    .id(("menu-item", index))
                                    .flex()
                                    .items_center()
                                    .gap(px(10.))
                                    .px(px(10.))
                                    .py(px(7.))
                                    .rounded(px(8.))
                                    .child(icon(name, 16., *tint))
                                    .child(div().flex_1().child(label.clone()))
                                    .when(!enabled, |d| d.text_color(t.text3))
                                    .when(enabled, |d| {
                                        d.cursor_pointer()
                                            .hover(move |s| s.bg(hover))
                                            .press(&t)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                let menu = this.menu;
                                                window.remove_window();
                                                this.owner
                                                    .update(cx, |owner, cx| {
                                                        owner.menu_choice(menu, index, cx)
                                                    })
                                                    .ok();
                                            }))
                                    })
                            }),
                    ),
            )
    }
}

/// The account and sidebar state saved alongside the mailboxes.
#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    account: String,
    initials: String,
    counts: HashMap<String, (u32, u32)>,
    labels: Vec<gmail::Label>,
    #[serde(default)]
    synced_to: HashMap<String, String>,
}

/// The store record a mailbox is saved under.
fn record(view: View) -> String {
    format!("view-{}", view_key(view))
}

/// A thread, wherever it is in a list: its key and its account.
type ThreadKey = (String, String);

/// What a menu or panel that acts on threads was opened for: the mailbox and the threads.
#[derive(Clone)]
struct MeantFor {
    mailbox: View,
    threads: Vec<ThreadKey>,
}

/// The message a reply or forward is about.
struct Source {
    subject: String,
    sender: String,
    header: crate::data::Header,
    body: Vec<Block>,
    reply_to: Option<gmail::ReplyTo>,
    /// For sample data, which has no real headers.
    fallback_to: Option<String>,
    fallback_all: Vec<String>,
    /// The account the message is in (All inboxes): the reply goes out from it.
    account: Option<String>,
}

impl Source {
    /// The latest message of a thread.
    fn latest(mail: &Mail) -> Self {
        Source {
            subject: mail.subject.to_string(),
            sender: mail.sender.to_string(),
            header: mail.header.clone(),
            body: mail.body.clone(),
            reply_to: mail.reply.as_ref().map(|(_, reply)| reply.clone()),
            fallback_to: mail.reply.as_ref().map(|(address, _)| address.clone()),
            fallback_all: mail.reply_all.clone(),
            account: mail.account.clone(),
        }
    }

    /// An earlier message in a thread.
    fn earlier(mail: &Mail, earlier: &crate::data::Earlier) -> Self {
        let reply_to = mail
            .id
            .clone()
            .filter(|_| !earlier.header.message_id.is_empty())
            .map(|thread_id| gmail::ReplyTo {
                thread_id,
                message_id: earlier.header.message_id.clone(),
            });
        Source {
            subject: mail.subject.to_string(),
            sender: earlier.sender.to_string(),
            header: earlier.header.clone(),
            body: earlier.blocks.clone(),
            reply_to,
            fallback_to: None,
            fallback_all: Vec::new(),
            account: mail.account.clone(),
        }
    }
}

/// "Fri, Nov 21, 2026 at 09:41", in local time; empty when unknown.
fn long_date(timestamp_ms: i64) -> String {
    use chrono::{Local, TimeZone};
    if timestamp_ms <= 0 {
        return String::new();
    }
    Local
        .timestamp_millis_opt(timestamp_ms)
        .single()
        .map(|when| when.format("%a, %b %-d, %Y at %H:%M").to_string())
        .unwrap_or_default()
}

/// A free path in Downloads for `name` ("Plan.pdf", then "Plan 2.pdf", ...).
fn unique_download_path(name: &str) -> anyhow::Result<std::path::PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("no home folder"))?;
    let dir = std::path::PathBuf::from(home).join("Downloads");
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (name.to_string(), String::new()),
    };
    for n in 1..1000 {
        let file = if n == 1 {
            format!("{stem}{ext}")
        } else {
            format!("{stem} {n}{ext}")
        };
        let path = dir.join(file);
        if !path.exists() {
            return Ok(path);
        }
    }
    anyhow::bail!("too many files with that name in Downloads")
}

/// A private temporary folder for Quick Look previews.
fn preview_path(name: &str) -> anyhow::Result<std::path::PathBuf> {
    let mut id = [0u8; 8];
    getrandom::getrandom(&mut id).map_err(|e| anyhow::anyhow!("{e}"))?;
    let dir = std::env::temp_dir()
        .join("nimbra-previews")
        .join(id.iter().map(|b| format!("{b:02x}")).collect::<String>());
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(dir.join(name))
}

fn restrict_file(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}

/// A button on an attachment: its icon, a turning arrow while it downloads, or a green
/// check mark once saved.
fn attachment_button(t: &Theme, name: &str, busy: bool, done: bool) -> Div {
    let hover = t.hover;
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(30.))
        .rounded(px(8.))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(if busy {
            icon("reload", 15., t.violet)
                .with_animation(
                    "attachment-busy",
                    Animation::new(Duration::from_millis(900)).repeat(),
                    |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
                )
                .into_any_element()
        } else {
            icon(name, 16., if done { t.mint } else { t.text2 }).into_any_element()
        })
}

/// Text made safe to put in HTML.
fn esc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// "1.2 MB", "340 KB".
fn file_size(bytes: u64) -> String {
    match bytes {
        0 => String::new(),
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{} KB", b / 1024),
        b => format!("{:.1} MB", b as f64 / (1024. * 1024.)),
    }
}

/// Which composer field an event came from.
#[derive(Clone, Copy, PartialEq)]
enum Field {
    To,
    Cc,
    Bcc,
    Subject,
    Line(usize),
}

/// Dragging the top strip moves the window; a double click zooms it, as macOS expects.
fn drag_or_zoom(event: &gpui::MouseDownEvent, window: &mut Window, _: &mut App) {
    if event.click_count == 2 {
        window.titlebar_double_click();
    } else {
        window.start_window_move();
    }
}

pub struct Nimbra {
    focus: FocusHandle,
    status: Status,
    /// Which mailbox the list is showing.
    mailbox: View,
    /// The last listing of each mailbox this session, with downloaded thread contents.
    /// Switching back shows it at once while a refresh runs in the background.
    /// Backed by the encrypted store on disk (see `persist`).
    cache: HashMap<View, Vec<Mail>>,
    /// The Inbox kept on this Mac is empty. Until the store is open this goes by the saved
    /// file's size, which is enough to open on Inbox Zero instead of the skeleton.
    inbox_saved_empty: bool,
    /// The last All inboxes listing had every account's whole Inbox: each was reached and
    /// none had a second page. Only then does empty mean Inbox Zero there.
    all_inboxes_whole: bool,
    /// Inbox Zero coming up, on a launch that opens on it.
    arrival: Option<Arrival>,
    /// The message whose full header (all recipients, date) is expanded: thread id and
    /// message index (0 is the latest).
    details: Option<(String, usize)>,
    /// The formatted message editor (a web view), made the first time it is needed.
    editor: Option<std::rc::Rc<crate::editor::EditorView>>,
    /// No editor on this system: composers use the plain line fields.
    editor_failed: bool,
    /// The editor's page has loaded.
    editor_ready: bool,
    /// Which composer the editor shows.
    editor_shown: Option<u64>,
    /// The colours and layout last given to the editor.
    editor_theme: Option<(bool, bool)>,
    /// GPUI's keyboard focus while typing in the editor (its "Editor" key context).
    editor_focus: FocusHandle,
    /// The editor had the keyboard at the last frame.
    editor_keys: bool,
    settings: crate::settings::Settings,
    /// What was copied last from a message's details, and when (shows "Copied").
    copied: Option<(String, std::time::Instant)>,
    /// Closing a composer with something in it: asking whether to save it as a draft.
    compose_ask: bool,
    /// When that question opened (a stray Esc right after it must not cancel it).
    compose_ask_at: Option<std::time::Instant>,
    /// The composer is in a window of its own (detached), so it always fills it.
    compose_detached: bool,
    /// Where a composer in a window of its own goes back to: the message window it came
    /// from, or the main window (None).
    compose_home: Option<gpui::AnyWindowHandle>,
    /// The formatting button under the mouse: its name and keys, shown in the toolbar (a
    /// tooltip would open under the editor, which is a native view and covers it).
    format_hint: Option<(&'static str, &'static str)>,
    /// The window showing the composer when it is not the main one (a message window it
    /// was started from, or a window of its own).
    compose_host: Option<gpui::AnyWindowHandle>,
    /// The window the editor web view was made in (it is native, so it lives in one window).
    editor_window: Option<gpui::AnyWindowHandle>,
    /// Connections to the signed-in accounts that are not the one shown (by account id).
    others: HashMap<String, Arc<gmail::Client>>,
    /// Unread inbox conversations per account (for All inboxes and the Accounts list).
    account_unread: HashMap<String, u32>,
    /// Conversations open in windows of their own.
    message_windows: Vec<gpui::WindowHandle<MessageWindow>>,
    /// The Settings window, while it is open.
    settings_window: Option<gpui::WindowHandle<SettingsWindow>>,
    /// Composers that are closed but still saving, waiting to send, or discarded.
    parked: Vec<(Parked, Composer)>,
    composer_serial: u64,
    /// Messages to send later, from this Mac.
    scheduled: Vec<Scheduled>,
    /// Scheduled drafts being sent right now.
    sending_scheduled: HashSet<String>,
    schedule_watch: bool,
    /// The times offered by the open Send later menu.
    send_later_times: Vec<i64>,
    /// The account shown: "" for the first one, else its id (see `gmail::is_account_id`).
    account_id: String,
    /// Bumped on every account switch, so answers for the previous account are dropped.
    account_gen: u64,
    /// Gmail could not be reached the last time we tried.
    offline: bool,
    /// What is waiting to go out once Gmail can be reached.
    outbox: Outbox,
    /// The offline retry loop is running.
    retrying: bool,
    /// Which reconnect loop is the current one; any older one stops.
    retry_gen: u64,
    /// The accounts whose offline queue is being sent right now.
    catching_up: HashSet<String>,
    /// Those of `sends` that could not leave this Mac and wait for the connection.
    queue: Vec<Arc<Job>>,
    /// Messages between Send and Gmail having them, of every account.
    sends: Vec<Arc<Job>>,
    /// Changes to threads, until they are settled.
    steps: Steps,
    /// The last notice about mail that was or was not kept or sent: a list arriving does
    /// not take it away, as it does a stale error.
    must_show: Option<SharedString>,
    /// Draft saves on their way: quitting needs their ids before it sends or saves.
    saves: Underway,
    /// Whatever else is changing something in Gmail, for quitting to wait for.
    writes: Underway,
    /// The open thread was just marked unread by the user: it stays unread until another
    /// thread is opened, even if its text is clicked.
    kept_unread: Option<String>,
    /// Attachments being downloaded (by attachment id; true when saving, false for a
    /// preview), and the ones just saved. Shown on the attachment itself, not as a message
    /// at the bottom (which would hide the email's web view while it shows).
    attachment_busy: HashMap<String, bool>,
    attachment_saved: HashMap<String, std::time::Instant>,
    /// The message window at the bottom (floats above the email, which is a native view)
    /// and the message it shows.
    notice_window: Option<gpui::WindowHandle<NoticeToast>>,
    notice_shown: Option<SharedString>,
    notice_gen: usize,
    /// The search panel (opened with / or ⌘F): its field, the results so far (from the mail
    /// on this Mac, then Gmail's), the highlighted row (0 is "All results"), and whether
    /// Gmail is still being asked.
    finder_open: bool,
    finder: Entity<TextInput>,
    _finder_events: gpui::Subscription,
    finder_hits: Vec<Hit>,
    finder_sel: usize,
    finder_gen: u64,
    finder_loading: bool,
    /// Counts All/Unread switches, to slide the highlight (0: never switched, no slide).
    filter_gen: usize,
    filter_switched: Option<std::time::Instant>,
    /// Sidebar sections folding or unfolding right now: when it started, and a number per
    /// fold (for its animation).
    folding: HashMap<&'static str, (std::time::Instant, usize)>,
    fold_gen: usize,
    /// The time between frames while something animates (measured: 60 Hz, 120 Hz…), and
    /// when the last frame was drawn.
    frame_interval: std::cell::Cell<f32>,
    last_frame: Option<std::time::Instant>,
    /// When the sidebar last started opening or closing (to keep the email still meanwhile).
    sidebar_toggled: Option<std::time::Instant>,
    /// The hidden sidebar, floating in from the window's edge (see `autohide`).
    peek: crate::autohide::Peek,
    /// The floating panel's left edge when it was docked: it slides to its place from there.
    docked_from: Option<f32>,
    /// The pointer is being watched for the hidden sidebar.
    watching_edge: bool,
    /// Full view: the sidebar and list were hidden by a double-click (or Return); this is
    /// whether each was open before, to put them back.
    focus_restore: Option<(bool, bool)>,
    /// The checking-for-mail bar was showing at the last frame.
    bar_shown: bool,
    /// When the bar started fading out (and a number per fade, for its animation).
    bar_fade: Option<(std::time::Instant, usize)>,
    /// The number shown on the Dock icon.
    badge: Option<u32>,
    /// A document waiting to be printed (it is loaded into the web view first).
    print_pending: Option<String>,
    /// Nimbra opens at login (read when Settings opens).
    login_item: bool,
    /// Rows picked with ⌘-click, Shift-click or ⌘A, for actions on several threads.
    picked: std::collections::BTreeSet<usize>,
    /// How many rows each entry of the undo history covers (one, or a whole batch).
    history_groups: Vec<usize>,
    /// Threads snoozed or waiting for a reply, kept on this Mac.
    snoozed: Vec<Snooze>,
    /// Snoozed threads being brought back right now.
    waking: HashSet<String>,
    /// The choices in the open snooze palette.
    snooze_choices: Vec<SnoozeChoice>,
    /// Where each mailbox's next page starts, when Gmail has more.
    next_pages: HashMap<View, String>,
    /// Mailboxes whose next page could not be loaded: not asked again until a refresh.
    more_failed: HashSet<View>,
    /// Threads whose content could not be loaded.
    body_failed: HashSet<ThreadKey>,
    loading_more: bool,
    /// Threads moved out of the inbox to a label, so Undo can take the label off again.
    moved: HashMap<String, String>,
    /// A question waiting for a yes (delete forever, delete a label).
    confirm: Option<Ask>,
    /// What the label sheet is for.
    label_sheet: LabelSheet,
    /// The New label sheet's name field, while it is open.
    new_label: Option<Entity<TextInput>>,
    _new_label_subscription: Option<gpui::Subscription>,
    /// What each item of the open Move to menu does.
    move_targets: Vec<MoveTarget>,
    /// A thread to open once it appears in the list (from a clicked notification).
    open_when_listed: Option<String>,
    /// The search result that was clicked, until the results it belongs to are listed.
    search_opened: Option<Mail>,
    /// A background fetch of thread contents is running.
    prefetching: bool,
    /// The encrypted on-disk mail store, once opened (real mail only).
    store: Option<Arc<crate::store::Store>>,
    /// Where the new-mail check reads Gmail's change feed from; `None` until its first
    /// look, which only takes stock and notifies about nothing.
    notify_from: Option<String>,
    /// Per mailbox (by store record name): the change-feed point its listing is current
    /// to, so a refresh only fetches what changed since.
    synced_to: HashMap<String, String>,
    /// Sidebar counts per label id: (unread threads still in the inbox, total threads for Drafts).
    counts: HashMap<String, (u32, u32)>,
    /// The user's own Gmail labels, shown in the sidebar.
    labels: Vec<gmail::Label>,
    /// The check for mail that is running, and what it is for (see `fetching`).
    checks: Checks,
    /// The search field in the sidebar, and the query the results are for.
    search: Entity<TextInput>,
    query: String,
    _search_events: gpui::Subscription,
    /// The toolbar button under the mouse (what it does, its shortcut). Shown in the strip
    /// above an email in the original view, where a floating tooltip would be hidden
    /// behind the native web view.
    hint: Option<(&'static str, &'static str)>,
    /// Where a row's right-click menu opens (window coordinates), and what its items do.
    menu_at: (f32, f32),
    row_actions: Vec<RowAction>,
    /// What the open menu, the snooze panel and the label sheet were opened for. A refresh
    /// can move or remove rows under each, so they check before acting (see `still_meant`).
    menu_for: Option<MeantFor>,
    palette_for: Option<MeantFor>,
    label_for: Option<MeantFor>,
    /// The labels the open menu lists, as it lists them.
    menu_labels: Vec<gmail::Label>,
    /// The "Undo" pill shown after a thread is removed, in a window of its own so it
    /// floats above the email's web view; `undo_gen` restarts its timer.
    undo_window: Option<gpui::WindowHandle<UndoToast>>,
    undo_gen: usize,
    /// The main window's frame, kept from the last draw, to place pop-ups without a window.
    frame: Option<gpui::Bounds<gpui::Pixels>>,
    /// The toolbar menu's own window, while it is open.
    menu_window: Option<gpui::WindowHandle<MenuPopup>>,
    /// The open toolbar menu, if any.
    menu: Option<Menu>,
    /// The menu a click outside just closed, and when. Lets a click on the menu's own
    /// button close it instead of closing and instantly reopening it.
    menu_dismissed: Option<(Menu, std::time::Instant)>,
    /// "Show original": the message's own HTML in a web view instead of the native reader.
    original: bool,
    /// Remote images allowed in the web view for the open thread. On by default; the
    /// strip above the message can block them.
    original_images: bool,
    /// The web view, created the first time it is needed.
    html_view: Option<std::rc::Rc<HtmlView>>,
    /// The hidden web view that print jobs are laid out in, always light.
    print_view: Option<HtmlView>,
    /// What the web view currently shows (thread, images allowed), to avoid reloading.
    html_shown: Option<(String, bool, bool, bool)>,
    /// The message being written, if the composer is open.
    compose: Option<Composer>,
    /// Focus to hand over on the next frame (set where no window is at hand).
    pending_focus: Option<FocusHandle>,
    /// The list's Unread filter.
    unread_only: bool,
    /// What took each cleared thread out of the list, so undo can reverse it.
    removed_by: HashMap<String, gmail::Change>,
    config: Option<gmail::Config>,
    gmail: Option<Arc<gmail::Client>>,
    /// Shown in the sidebar footer.
    account: SharedString,
    me_initials: SharedString,
    /// The account's Google profile photo, shown in place of the initial.
    me_photo: Option<Arc<gpui::Image>>,
    /// The sign-in does not include the profile (asked for in Settings › Accounts).
    photo_not_allowed: bool,
    /// The account generation the photo was last looked up for.
    photo_gen: Option<u64>,
    /// A short message shown at the bottom of the window (errors, hints).
    notice: Option<SharedString>,
    mails: Vec<Mail>,
    done: Vec<bool>,
    history: Vec<usize>,
    selected: usize,
    palette: bool,
    palette_sel: usize,
    dark: bool,
    sidebar_open: bool,
    /// The message list: shown or hidden, its width, and an in-progress resize
    /// (mouse x at the start, width at the start).
    list_open: bool,
    list_width: f32,
    list_grab: Option<(f32, f32)>,
    /// When the list was last shown or hidden, to animate it and restart the animation.
    list_toggled: Option<std::time::Instant>,
    list_gen: usize,
    /// Pull to refresh: the list's scroll position, how far it has been pulled past the
    /// top, and whether the current trackpad gesture started at the top.
    list_scroll: gpui::ScrollHandle,
    sidebar_scroll: gpui::ScrollHandle,
    /// When each mailbox was last checked, by its record.
    checked: HashMap<String, Asked>,
    /// When the sidebar's counts were last fetched (131 units, with 8 labels).
    counted: Option<Asked>,
    /// How often the change feeds have brought something.
    feed_changes: u64,
    /// The mailboxes whose next page is known, or known not to exist.
    paged: HashSet<View>,
    /// The search panel's results, kept scrolled to the highlighted one.
    finder_scroll: gpui::ScrollHandle,
    pull: f32,
    pull_armed: bool,
    /// The indicator settling after release: (from height, to height). `pull_gen` restarts it.
    pull_anim: Option<(f32, f32, std::time::Instant)>,
    pull_gen: usize,
    /// The indicator is held open while the refresh it started is running.
    pull_hold: bool,
    /// The strip is closing after a finished check: it says "Done" on the way up.
    pull_done: bool,
    /// Bumped on every toggle / reload so the matching animation replays.
    reload_gen: usize,
    /// Remote images the user asked to load, by URL. Nothing is fetched before that.
    images: HashMap<String, Arc<gpui::Image>>,
    image_requests: HashSet<String>,
    /// Threads visited before / after the current one, for the back and forward buttons.
    back: Vec<usize>,
    forward: Vec<usize>,
}

impl Nimbra {
    pub fn new(cx: &mut Context<Self>) -> Self {
        // NIMBRA_SCREEN=palette|zero and NIMBRA_THEME=light open the sample data in a given
        // state, which is handy for screenshots. NIMBRA_DEMO=1 forces the sample data.
        let screen = std::env::var("NIMBRA_SCREEN").unwrap_or_default();
        let config = if !screen.is_empty() || std::env::var_os("NIMBRA_DEMO").is_some() {
            None
        } else {
            gmail::Config::load()
        };
        let demo = config.is_none();
        let mails = if demo { inbox() } else { Vec::new() };
        let zero = screen == "zero";
        let search = cx
            .new(|cx| TextInput::new(cx, "Search mail", Theme::dark().text3, Theme::dark().violet));
        let search_events = cx.subscribe(&search, |this, _, event: &InputEvent, cx| match event {
            InputEvent::Enter => this.run_search(cx),
            // Down or Tab hands the keyboard back to the list.
            InputEvent::Down | InputEvent::Tab => {
                this.pending_focus = Some(this.focus.clone());
                cx.notify();
            }
            _ => {}
        });
        let finder = cx.new(|cx| {
            TextInput::new(
                cx,
                "Search mail, people and subjects",
                Theme::dark().text3,
                Theme::dark().violet,
            )
        });
        let finder_events = cx.subscribe(&finder, |this, _, event: &InputEvent, cx| match event {
            InputEvent::Changed => this.finder_changed(cx),
            InputEvent::Down | InputEvent::Tab => this.finder_step(1, cx),
            InputEvent::Up => this.finder_step(-1, cx),
            InputEvent::Enter => this.finder_open_selected(cx),
            _ => {}
        });
        let settings = crate::settings::load();
        // As it was left. NIMBRA_SIDEBAR=closed or open overrides that (screenshots).
        let sidebar_open = match std::env::var("NIMBRA_SIDEBAR").as_deref() {
            Ok("closed") => false,
            Ok("open") => true,
            _ => !settings.sidebar_auto_hide,
        };
        let mut this = Self {
            inbox_saved_empty: false,
            all_inboxes_whole: false,
            arrival: None,
            finder_open: false,
            finder,
            _finder_events: finder_events,
            finder_hits: Vec::new(),
            finder_sel: 0,
            finder_gen: 0,
            finder_loading: false,
            search,
            query: String::new(),
            _search_events: search_events,
            focus: cx.focus_handle(),
            status: if demo {
                Status::Demo
            } else {
                Status::SignedOut
            },
            // NIMBRA_MAILBOX=sent (or starred, snoozed, drafts, done, all, important, spam, trash)
            // opens that mailbox first.
            mailbox: match std::env::var("NIMBRA_MAILBOX").as_deref() {
                Ok("starred") => View::Starred,
                Ok("snoozed") => View::Snoozed,
                Ok("drafts") => View::Drafts,
                Ok("sent") => View::Sent,
                Ok("done") => View::Done,
                Ok("all") => View::AllMail,
                Ok("important") => View::Important,
                Ok("spam") => View::Spam,
                Ok("trash") => View::Trash,
                _ => View::Inbox,
            },
            cache: HashMap::new(),
            prefetching: false,
            open_when_listed: None,
            search_opened: None,
            account_id: String::new(),
            account_gen: 0,
            offline: false,
            outbox: Outbox::default(),
            retrying: false,
            retry_gen: 0,
            catching_up: HashSet::new(),
            queue: Vec::new(),
            sends: Vec::new(),
            steps: Steps::default(),
            must_show: None,
            saves: Underway::default(),
            writes: Underway::default(),
            kept_unread: None,
            attachment_busy: HashMap::new(),
            attachment_saved: HashMap::new(),
            notice_window: None,
            notice_shown: None,
            notice_gen: 0,
            focus_restore: None,
            sidebar_toggled: None,
            peek: crate::autohide::Peek::new(),
            docked_from: None,
            watching_edge: false,
            frame_interval: std::cell::Cell::new(1. / 60.),
            last_frame: None,
            folding: HashMap::new(),
            filter_gen: 0,
            filter_switched: None,
            fold_gen: 0,
            bar_shown: false,
            bar_fade: None,
            badge: None,
            print_pending: None,
            login_item: false,
            picked: Default::default(),
            history_groups: Vec::new(),
            snoozed: Vec::new(),
            waking: HashSet::new(),
            snooze_choices: Vec::new(),
            next_pages: HashMap::new(),
            more_failed: HashSet::new(),
            body_failed: HashSet::new(),
            loading_more: false,
            moved: HashMap::new(),
            confirm: None,
            label_sheet: LabelSheet::New { apply: true },
            new_label: None,
            _new_label_subscription: None,
            move_targets: Vec::new(),
            editor: None,
            editor_failed: false,
            editor_ready: false,
            editor_shown: None,
            editor_theme: None,
            editor_focus: cx.focus_handle(),
            editor_keys: false,
            settings,
            settings_window: None,
            message_windows: Vec::new(),
            others: HashMap::new(),
            account_unread: HashMap::new(),
            compose_host: None,
            format_hint: None,
            compose_detached: false,
            compose_home: None,
            compose_ask: false,
            compose_ask_at: None,
            editor_window: None,
            copied: None,
            parked: Vec::new(),
            composer_serial: 0,
            scheduled: Vec::new(),
            sending_scheduled: HashSet::new(),
            schedule_watch: false,
            send_later_times: Vec::new(),
            details: None,
            store: None,
            notify_from: None,
            synced_to: HashMap::new(),
            counts: HashMap::new(),
            labels: Vec::new(),
            checks: Checks::default(),
            hint: None,
            menu_at: (0., 0.),
            row_actions: Vec::new(),
            menu_for: None,
            palette_for: None,
            label_for: None,
            menu_labels: Vec::new(),
            menu_window: None,
            undo_window: None,
            undo_gen: 0,
            frame: None,
            menu: None,
            menu_dismissed: None,
            original: false,
            original_images: true,
            html_view: None,
            html_shown: None,
            print_view: None,
            compose: None,
            pending_focus: None,
            unread_only: false,
            removed_by: HashMap::new(),
            config,
            gmail: None,
            account: if demo {
                "Luan Gjokaj".into()
            } else {
                "Gmail".into()
            },
            me_initials: if demo { "LG".into() } else { "ME".into() },
            me_photo: None,
            photo_not_allowed: false,
            photo_gen: None,
            notice: None,
            done: vec![zero; mails.len()],
            history: if zero {
                (0..mails.len()).collect()
            } else {
                Vec::new()
            },
            // NIMBRA_OPEN=<row> opens that thread first (screenshots).
            selected: std::env::var("NIMBRA_OPEN")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .filter(|i| *i < mails.len())
                .unwrap_or(0),
            mails,
            palette: screen == "palette",
            palette_sel: 0,
            dark: std::env::var("NIMBRA_THEME").as_deref() != Ok("light"),
            list_open: std::env::var("NIMBRA_LIST").as_deref() != Ok("closed"),
            // NIMBRA_LIST_WIDTH=<px> starts with the list at that width (screenshots).
            list_width: std::env::var("NIMBRA_LIST_WIDTH")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .map_or(452., |w| w.clamp(300., 720.)),
            list_grab: None,
            list_toggled: None,
            list_gen: 0,
            list_scroll: gpui::ScrollHandle::new(),
            sidebar_scroll: gpui::ScrollHandle::new(),
            checked: HashMap::new(),
            counted: None,
            feed_changes: 0,
            paged: HashSet::new(),
            finder_scroll: gpui::ScrollHandle::new(),
            pull: 0.,
            pull_armed: false,
            pull_anim: None,
            pull_gen: 0,
            pull_hold: false,
            pull_done: false,
            sidebar_open,
            reload_gen: 0,
            images: HashMap::new(),
            image_requests: HashSet::new(),
            back: Vec::new(),
            forward: Vec::new(),
        };
        // NIMBRA_SCREEN=compose opens the composer on the sample data (screenshots).
        if screen == "compose" || screen == "compose-full" {
            this.open_compose(ComposeKind::Reply, cx);
            if let Some(composer) = &mut this.compose {
                composer.full = screen == "compose-full";
            }
        }
        // Mail opens in its original formatting by default; NIMBRA_READER=1 starts in the
        // native reader instead. Messages without HTML always use the reader.
        this.original = this.settings.original_view && std::env::var_os("NIMBRA_READER").is_none();
        this.original_images = this.settings.load_images;
        // NIMBRA_SCREEN=draft opens a sample draft in the composer (screenshots).
        if screen == "draft" {
            this.edit_draft(
                gmail::Draft {
                    id: "sample".into(),
                    to: "mira@example.com".into(),
                    cc: String::new(),
                    bcc: String::new(),
                    attachments: Vec::new(),
                    subject: "Fwd: Billing problem".into(),
                    body: "Hi Mira,\n\nCan you take a look at this one?\n\nLuan Gjokaj\nUX Engineer & Designer".into(),
                    quoted: Some("---------- Forwarded message ---------\nFrom: Stripe\nSubject: Billing problem\n\nYour last payment could not be processed.".into()),
                    html: None,
                    reply: None,
                },
                cx,
            );
            if let Some(composer) = &mut this.compose {
                composer.show_quoted = true;
            }
        }
        // NIMBRA_SCREEN=bulk picks three threads (screenshots).
        if screen == "bulk" {
            this.picked = this.visible().into_iter().take(3).collect();
        }
        // NIMBRA_SCREEN=settings opens the Settings sheet (screenshots).
        if screen == "settings" {
            // After the main window is up.
            cx.spawn(async move |this, cx| {
                this.update(cx, |this, cx| this.open_settings(cx)).ok();
            })
            .detach();
        }
        // NIMBRA_SCREEN=loading holds the loading skeleton on screen (screenshots).
        if screen == "loading" {
            this.status = Status::Loading;
            this.checks.start(View::Inbox, "");
        }
        this.watch_for_mail(cx);
        this.watch_other_accounts(cx);
        this.watch_notification_clicks(cx);
        // Every way out: ⌘Q, the Dock, logging out.
        cx.on_app_quit(|this, cx| {
            this.finish_before_quit(cx);
            async {}
        })
        .detach();
        // Already signed in on a previous run: go straight to the inbox. The keychain is
        // read off the main thread because macOS may show a permission prompt for it, and
        // the window should not freeze (or fail to appear) while that is open.
        let active = this.settings.active.clone();
        if gmail::is_account_id(&active)
            && (active.is_empty() || this.settings.accounts.iter().any(|a| a.id == active))
        {
            this.account_id = active;
        }
        if this.config.is_some() {
            this.status = Status::Loading;
            this.inbox_saved_empty =
                crate::store::Store::saved_empty(&this.account_id, &record(View::Inbox));
            if this.opening_to_zero() && !crate::system::reduces_motion() {
                this.arrival = Some(Arrival::START);
            }
            // On every change, whichever action made it: the next launch goes by this.
            cx.observe_self(Self::save_if_zero_changed).detach();
            this.connect(false, cx);
        }
        // NIMBRA_SEARCH=<query> starts with that search already run.
        if let Ok(query) = std::env::var("NIMBRA_SEARCH") {
            this.search
                .update(cx, |input, cx| input.set(query, usize::MAX, cx));
            this.run_search(cx);
        }
        this
    }

    fn is_demo(&self) -> bool {
        self.status == Status::Demo
    }

    /// Whether nothing on screen means Inbox Zero, rather than nothing in this mailbox.
    fn is_zero(&self) -> bool {
        match self.mailbox {
            View::Inbox => true,
            // Not while mail is filtered away (Unread), unchecked or past an account's first page.
            View::AllInboxes => self.all_inboxes_whole && self.done.iter().all(|done| *done),
            _ => false,
        }
    }

    /// The store is still opening, and the Inbox it holds is an empty one.
    fn opening_to_zero(&self) -> bool {
        self.status == Status::Loading
            && self.store.is_none()
            && self.mailbox == View::Inbox
            && self.inbox_saved_empty
    }

    /// Saves the Inbox as soon as it empties or stops being empty. Waiting for the next
    /// check for mail would lose that when Nimbra quits first.
    fn save_if_zero_changed(&mut self, cx: &mut Context<Self>) {
        if self.mailbox == View::Inbox
            && self.status == Status::Ready
            // Not `visible`: with Unread on, read mail is still in the Inbox.
            && self.done.iter().all(|done| *done) != self.inbox_saved_empty
        {
            self.persist(cx);
        }
    }

    fn inbox_name(&self) -> SharedString {
        self.title(self.mailbox)
    }

    fn title(&self, view: View) -> SharedString {
        match view {
            View::AllInboxes => "All inboxes".into(),
            View::Inbox if self.is_demo() => "Important".into(),
            View::Inbox => "Inbox".into(),
            View::Primary => "Primary".into(),
            View::Updates => "Updates".into(),
            View::Promotions => "Promotions".into(),
            View::Social => "Social".into(),
            View::Starred => "Starred".into(),
            View::Snoozed => "Snoozed".into(),
            View::Drafts => "Drafts".into(),
            View::Sent => "Sent".into(),
            View::Done => "Done".into(),
            View::AllMail => "All Mail".into(),
            View::Important => "Important".into(),
            View::Spam => "Spam".into(),
            View::Trash => "Trash".into(),
            View::Search => "Search".into(),
            View::Label(index) => self
                .labels
                .get(index)
                .map(|label| SharedString::from(label.name.clone()))
                .unwrap_or_else(|| "Label".into()),
        }
    }

    /// / or ⌘F: the search panel.
    fn focus_search(&mut self, cx: &mut Context<Self>) {
        if self.compose.is_some() || matches!(self.status, Status::SignedOut | Status::SigningIn) {
            return;
        }
        self.menu = None;
        self.palette = false;
        self.finder_open = true;
        let query = self.query.clone();
        self.finder
            .update(cx, |input, cx| input.set(query, usize::MAX, cx));
        self.finder_changed(cx);
        self.pending_focus = Some(self.finder.read(cx).focus_handle(cx));
        cx.notify();
    }

    fn close_finder(&mut self, cx: &mut Context<Self>) {
        self.finder_open = false;
        self.finder_gen += 1;
        self.finder_loading = false;
        self.pending_focus = Some(self.focus.clone());
        cx.notify();
    }

    /// What was typed changed: results from the mail on this Mac at once, then (after a
    /// short pause) Gmail's own search, merged in below.
    fn finder_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.finder.read(cx).text().trim().to_string();
        self.finder_hits = self.local_hits(&query);
        self.finder_scroll.scroll_to_item(0);
        self.finder_sel = if self.finder_hits.is_empty() { 0 } else { 1 };
        self.finder_gen += 1;
        let generation = self.finder_gen;
        self.finder_loading = false;
        cx.notify();
        let Some(client) = self.gmail.clone().filter(|_| query.chars().count() >= 2) else {
            return;
        };
        self.finder_loading = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(350))
                .await;
            if this
                .update(cx, |this, _| this.finder_gen != generation)
                .unwrap_or(true)
            {
                return;
            }
            let q = query.clone();
            let found = within(cx.background_executor(), async move {
                client.threads_page(View::Search, Some(&q), 10, None)
            })
            .await;
            this.update(cx, |this, cx| {
                if this.finder_gen != generation {
                    return;
                }
                this.finder_loading = false;
                if let Ok((summaries, _)) = found {
                    for summary in summaries {
                        if this.finder_hits.len() >= 12 {
                            break;
                        }
                        if this
                            .finder_hits
                            .iter()
                            .any(|h| h.id.as_deref() == Some(summary.id.as_str()))
                        {
                            continue;
                        }
                        let mail = Mail::from_summary(summary);
                        this.finder_hits.push(Hit::from_mail(&mail));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Mail on this Mac matching every word typed (Gmail operators like `from:` are left to
    /// Gmail), newest first.
    fn local_hits(&self, query: &str) -> Vec<Hit> {
        let words: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            .filter(|w| !w.contains(':'))
            .map(str::to_string)
            .collect();
        if words.is_empty() {
            return Vec::new();
        }
        let mut seen = HashSet::new();
        let mut hits: Vec<Hit> = Vec::new();
        for mail in self.mails.iter().chain(self.cache.values().flatten()) {
            let key = mail.id.clone().unwrap_or_else(|| mail.subject.to_string());
            if !seen.insert(key) {
                continue;
            }
            let text = format!(
                "{} {} {} {} {}",
                mail.sender,
                mail.header.from.address,
                mail.subject,
                mail.snippet,
                mail.participants
            )
            .to_lowercase();
            if words.iter().all(|w| text.contains(w.as_str())) {
                hits.push(Hit::from_mail(mail));
            }
        }
        hits.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms));
        hits.truncate(8);
        hits
    }

    /// Removes one search from the recent ones.
    fn forget_search(&mut self, query: &str, cx: &mut Context<Self>) {
        self.settings.recent_searches.retain(|q| q != query);
        let rows = self.settings.recent_searches.len() + SEARCH_TIPS.len();
        self.finder_sel = self.finder_sel.min(rows.saturating_sub(1));
        self.save_settings(cx);
        cx.notify();
    }

    fn finder_step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let empty = self.finder.read(cx).text().trim().is_empty();
        let rows = if empty {
            self.settings.recent_searches.len() + SEARCH_TIPS.len()
        } else {
            self.finder_hits.len() + 1
        };
        if rows == 0 {
            return;
        }
        self.finder_sel = (self.finder_sel as isize + delta).rem_euclid(rows as isize) as usize;
        // Keep the highlighted row in view. Its place among the panel's children counts the
        // section titles above it.
        let child = if empty {
            let recent = self.settings.recent_searches.len();
            let recent_title = usize::from(recent > 0);
            if self.finder_sel < recent {
                recent_title + self.finder_sel
            } else {
                recent_title + recent + 1 + (self.finder_sel - recent)
            }
        } else if self.finder_sel == 0 {
            0
        } else {
            1 + self.finder_sel // "All results", then the Conversations title
        };
        // The first row brings the section title above it into view too.
        let child = if self.finder_sel == 0 || (!empty && self.finder_sel == 1) {
            0
        } else {
            child
        };
        self.finder_scroll.scroll_to_item(child);
        cx.notify();
    }

    /// Return: open the highlighted result, or all results for what was typed.
    fn finder_open_selected(&mut self, cx: &mut Context<Self>) {
        let query = self.finder.read(cx).text().trim().to_string();
        if query.is_empty() {
            // A recent search or a tip.
            let recent = self.settings.recent_searches.clone();
            if let Some(text) = recent.get(self.finder_sel) {
                let text = text.clone();
                self.finder
                    .update(cx, |input, cx| input.set(text, usize::MAX, cx));
                self.finder_changed(cx);
            } else if let Some((tip, _)) = SEARCH_TIPS.get(self.finder_sel - recent.len()) {
                let text = tip.to_string();
                let text = if text.ends_with(':') {
                    text
                } else {
                    format!("{text} ")
                };
                self.finder
                    .update(cx, |input, cx| input.set(text, usize::MAX, cx));
                self.finder_changed(cx);
            }
            return;
        }
        let hit = self
            .finder_sel
            .checked_sub(1)
            .and_then(|i| self.finder_hits.get(i))
            .cloned();
        self.search_for(&query, hit.map(|h| *h.mail), cx);
    }

    /// Runs a search: the results list, opening `open` in it if given. `open` is shown at
    /// once, and stays in the list even if Gmail's results do not include it.
    fn search_for(&mut self, query: &str, open: Option<Mail>, cx: &mut Context<Self>) {
        let open_mail = open;
        let open = open_mail.as_ref().and_then(|m| m.id.clone());
        let recent = &mut self.settings.recent_searches;
        recent.retain(|q| !q.eq_ignore_ascii_case(query));
        recent.insert(0, query.to_string());
        recent.truncate(6);
        self.save_settings(cx);
        self.close_finder(cx);
        let text = query.to_string();
        self.search
            .update(cx, |input, cx| input.set(text, usize::MAX, cx));
        // Already in the list on screen: just open it.
        if let Some(id) = &open
            && self.mailbox == View::Search
            && self.query == query
            && let Some(index) = self.mails.iter().position(|m| m.id.as_ref() == Some(id))
        {
            self.open(index);
            self.open_selected(cx);
            return;
        }
        self.open_when_listed = open;
        self.search_opened = open_mail.clone();
        self.run_search(cx);
        // Sample mail is searched on the spot (and has no ids: matched by subject).
        if self.is_demo() {
            self.open_when_listed = None;
            if let Some(mail) = self.search_opened.take()
                && let Some(index) = self.mails.iter().position(|m| m.subject == mail.subject)
            {
                self.open(index);
                self.open_selected(cx);
            }
        }
        // Opened straight away (run_search listed it), while the rest of the results load.
        if open_mail.is_some() && !self.is_demo() && self.search_opened.is_some() {
            self.open(0);
            self.open_selected(cx);
            cx.notify();
        }
    }

    /// The search panel.
    fn finder_view(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.finder.read(cx).text().trim().to_string();
        let empty = query.is_empty();
        let hover = t.hover;
        let row_base = move |id: usize, active: bool| {
            div()
                .id(("finder-row", id))
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(12.))
                .py(px(8.))
                .rounded(px(10.))
                .cursor_pointer()
                .when(active, |d| d.bg(t.selected))
                .when(!active, |d| d.hover(move |s| s.bg(hover)))
                .press(t)
        };
        let section = |label: &str| {
            div()
                .pl(px(12.))
                .pt(px(10.))
                .pb(px(6.))
                .child(overline(label.to_string(), t.faint))
        };
        let mut rows: Vec<AnyElement> = Vec::new();
        if empty {
            let recent = &self.settings.recent_searches;
            if !recent.is_empty() {
                rows.push(
                    div()
                        .flex()
                        .items_end()
                        .pr(px(6.))
                        .child(section("RECENT").flex_1())
                        .child(
                            div()
                                .id("finder-clear-recent")
                                .mb(px(3.))
                                .px(px(6.))
                                .py(px(2.))
                                .rounded(px(6.))
                                .cursor_pointer()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(t.text3)
                                .hover(move |s| s.bg(hover).text_color(t.text))
                                .press(t)
                                .child("Clear all")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.settings.recent_searches.clear();
                                    this.finder_sel = 0;
                                    this.save_settings(cx);
                                    cx.notify();
                                })),
                        )
                        .into_any_element(),
                );
            }
            for (i, text) in recent.iter().enumerate() {
                let value = text.clone();
                rows.push(
                    row_base(i, self.finder_sel == i)
                        .group("finder-recent")
                        .child(icon("clock", 15., t.text3))
                        .child(div().flex_1().min_w(px(0.)).child(short(text, 70)))
                        .child({
                            // Removes just this search.
                            let gone = text.clone();
                            div()
                                .id(("finder-forget", i))
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(22.))
                                .rounded(px(6.))
                                .when(self.finder_sel != i, |d| {
                                    d.invisible().group_hover("finder-recent", |s| s.visible())
                                })
                                .hover(move |s| s.bg(hover))
                                .press(t)
                                .child(icon("x", 13., t.text3))
                                .tooltip(tip(t.dark, "Remove from recent", ""))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.forget_search(&gone, cx);
                                }))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let v = value.clone();
                            this.finder
                                .update(cx, |input, cx| input.set(v, usize::MAX, cx));
                            this.finder_changed(cx);
                            this.pending_focus = Some(this.finder.read(cx).focus_handle(cx));
                        }))
                        .into_any_element(),
                );
            }
            rows.push(section("SEARCH TIPS").into_any_element());
            for (k, (tip, what)) in SEARCH_TIPS.iter().enumerate() {
                let i = recent.len() + k;
                rows.push(
                    row_base(i, self.finder_sel == i)
                        .child(icon("zap", 15., t.text3))
                        .child(
                            div()
                                .font_family(crate::theme::MONO)
                                .text_size(px(12.5))
                                .child(*tip),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .text_color(t.text3)
                                .child(*what),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let text = if tip.ends_with(':') {
                                tip.to_string()
                            } else {
                                format!("{tip} ")
                            };
                            this.finder
                                .update(cx, |input, cx| input.set(text, usize::MAX, cx));
                            this.finder_changed(cx);
                            this.pending_focus = Some(this.finder.read(cx).focus_handle(cx));
                        }))
                        .into_any_element(),
                );
            }
        } else {
            let q = query.clone();
            rows.push(
                row_base(0, self.finder_sel == 0)
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(28.))
                            .rounded(px(8.))
                            .bg(gradient(135., c(0x8b7cff, 1.), c(0xff7ab6, 1.)))
                            .child(icon("search", 15., gpui::white())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(format!("All results for “{}”", short(&query, 50))),
                    )
                    .when(self.finder_sel == 0, |d| d.child(kbd(t, "↵")))
                    .on_click(cx.listener(move |this, _, _, cx| this.search_for(&q, None, cx)))
                    .into_any_element(),
            );
            if !self.finder_hits.is_empty() {
                rows.push(section("CONVERSATIONS").into_any_element());
            }
            for (k, hit) in self.finder_hits.iter().enumerate() {
                let i = k + 1;
                let (q, mail) = (query.clone(), Some((*hit.mail).clone()));
                rows.push(
                    row_base(i, self.finder_sel == i)
                        .child(avatar(hit.tone, &hit.initials, 28.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(8.))
                                        .child(
                                            div()
                                                .font_weight(if hit.unread {
                                                    FontWeight::SEMIBOLD
                                                } else {
                                                    FontWeight::MEDIUM
                                                })
                                                .child(short(&hit.sender, 30)),
                                        )
                                        .when(hit.unread, |d| {
                                            d.child(div().size(px(6.)).rounded_full().bg(t.mint))
                                        })
                                        .child(div().flex_1())
                                        .child(mono(hit.time.clone(), t.text3)),
                                )
                                // Subject and preview on a line each, cut at the edge, so
                                // every result is the same height.
                                .child(
                                    div()
                                        .truncate()
                                        .text_color(t.text2)
                                        .child(hit.subject.clone()),
                                )
                                .when(!hit.snippet.trim().is_empty(), |d| {
                                    d.child(
                                        div()
                                            .truncate()
                                            .text_size(px(12.))
                                            .text_color(t.text3)
                                            .child(hit.snippet.clone()),
                                    )
                                }),
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.search_for(&q, mail.clone(), cx)
                            }),
                        )
                        .into_any_element(),
                );
            }
            if self.finder_hits.is_empty() && !self.finder_loading {
                rows.push(
                    div()
                        .px(px(12.))
                        .py(px(14.))
                        .text_color(t.text3)
                        .child("Nothing here yet. Press Return to search all of Gmail.")
                        .into_any_element(),
                );
            }
        }
        let hint = |keys: &[&str], label: &str| {
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .children(keys.iter().map(|k| kbd(t, k)))
                .child(meta(label.to_string(), t.text3))
        };
        div()
            .id("finder-scrim")
            .absolute()
            .inset_0()
            .flex()
            .items_start()
            .justify_center()
            .pt(px(140.))
            .bg(t.scrim)
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close_finder(cx)),
            )
            .child(
                div()
                    .id("finder")
                    .w(px(680.))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded(px(18.))
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![
                        shadow(c(0x8b7cff, 0.22), 0., 120., 10.),
                        shadow(c(0x000000, if t.dark { 0.6 } else { 0.2 }), 30., 80., -10.),
                    ])
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .pl(px(20.))
                            .pr(px(16.))
                            .py(px(16.))
                            .child(icon("search", 18., t.text3))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .text_size(px(17.))
                                    .child(self.finder.clone()),
                            )
                            .when(self.finder_loading, |d| {
                                d.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(6.))
                                        .child(icon("reload", 13., t.violet).with_animation(
                                            "finder-busy",
                                            Animation::new(Duration::from_millis(900)).repeat(),
                                            |svg, delta| {
                                                svg.with_transformation(Transformation::rotate(
                                                    percentage(delta),
                                                ))
                                            },
                                        ))
                                        .child(meta("Searching Gmail", t.text3)),
                                )
                            }),
                    )
                    .child(div().h(px(1.)).bg(t.border))
                    .child(
                        div()
                            .id("finder-rows")
                            .flex()
                            .flex_col()
                            .max_h(px(420.))
                            .overflow_y_scroll()
                            .track_scroll(&self.finder_scroll)
                            .p(px(8.))
                            .children(rows),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(16.))
                            .px(px(20.))
                            .py(px(12.))
                            .border_t_1()
                            .border_color(t.border)
                            .bg(t.hover)
                            .child(hint(&["↑", "↓"], "Navigate"))
                            .child(hint(&["↵"], "Open"))
                            .child(hint(&["esc"], "Close"))
                            .child(div().flex_1())
                            .child(serif("Nimbra", 18., t.text2)),
                    ),
            )
    }

    /// Runs the typed query: against Gmail (its full search syntax works), or locally
    /// on the sample data.
    fn run_search(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).text().trim().to_string();
        if query.is_empty() {
            self.clear_search(cx);
            return;
        }
        if matches!(self.status, Status::SignedOut | Status::SigningIn) {
            return;
        }
        self.remember_mailbox();
        self.picked.clear();
        // Results are never cached: every search is fetched fresh.
        self.cache.remove(&View::Search);
        self.next_pages.remove(&View::Search);
        self.more_failed.remove(&View::Search);
        self.query = query;
        self.mailbox = View::Search;
        self.back.clear();
        self.forward.clear();
        self.palette = false;
        self.unread_only = false;
        self.pending_focus = Some(self.focus.clone());
        if self.is_demo() {
            self.selected = self.visible().first().copied().unwrap_or(0);
        } else {
            self.mails.clear();
            self.done.clear();
            self.history.clear();
            self.history_groups.clear();
            self.selected = 0;
            // A result clicked in the search panel is listed (and opened) straight away, so
            // only the reading pane loads instead of the whole window.
            if let Some(mail) = self.search_opened.clone() {
                self.mails.push(mail);
                self.done.push(false);
            }
            self.fetch_inbox(cx);
        }
        cx.notify();
    }

    /// Empties the search field and goes back to the inbox.
    fn clear_search(&mut self, cx: &mut Context<Self>) {
        self.search
            .update(cx, |input, cx| input.set(String::new(), 0, cx));
        self.query.clear();
        self.pending_focus = Some(self.focus.clone());
        if self.mailbox == View::Search {
            self.select_mailbox(View::Inbox, cx);
        }
        cx.notify();
    }

    /// The thread left the undo history by itself (back with a newer message, or Gmail
    /// refused the change).
    fn forget_cleared(&mut self, position: usize, cx: &mut Context<Self>) {
        // The Undo pill is for the last one: it must not undo the one before instead.
        if position + 1 == self.history.len() {
            self.close_undo(cx);
        }
        crate::listing::forget(&mut self.history, &mut self.history_groups, position);
    }

    /// The list as it is kept for the mailbox: without the threads cleared from it.
    fn listed(&self) -> Vec<Mail> {
        self.mails
            .iter()
            .zip(&self.done)
            .filter(|(_, done)| !**done)
            .map(|(mail, _)| mail.clone())
            .collect()
    }

    /// Saves the current listing (minus threads cleared from it) for a later return.
    fn remember_mailbox(&mut self) {
        if self.is_demo() || self.status != Status::Ready {
            return;
        }
        let kept = self.listed();
        if self.mailbox != View::Search {
            self.cache.insert(self.mailbox, kept);
        }
    }

    /// Switches the list to another mailbox.
    fn select_mailbox(&mut self, view: View, cx: &mut Context<Self>) {
        if self.mailbox == view || matches!(self.status, Status::SignedOut | Status::SigningIn) {
            return;
        }
        self.remember_mailbox();
        self.picked.clear();
        self.mailbox = view;
        self.back.clear();
        self.forward.clear();
        self.palette = false;
        if self.is_demo() {
            // The sample data only has an inbox; Done shows what was cleared from it.
            self.selected = self.visible().first().copied().unwrap_or(0);
            cx.notify();
        } else {
            // Show what we already have for this mailbox, then refresh it quietly.
            // A mailbox seen before (this session or on disk) shows at once, even when it
            // was empty: "Nothing in …" is known, so there is no loading screen for it.
            let known = match self.cache.get(&view) {
                Some(mails) => Some(mails.clone()),
                // Not visited this session: start from what is stored on disk.
                None => self.stored_mails(view),
            };
            if let Some(mails) = &known {
                self.cache.insert(view, mails.clone());
            }
            self.mails = known.clone().unwrap_or_default();
            self.done = vec![false; self.mails.len()];
            self.history.clear();
            self.history_groups.clear();
            self.selected = 0;
            if known.is_some() {
                self.status = Status::Ready;
                self.load_selected(cx);
            }
            if known.is_some() {
                self.look_again(cx);
            } else {
                self.refresh(cx);
            }
        }
    }

    // ------------------------------------------------------------ gmail

    // ------------------------------------------------------------ syncing

    /// The mailbox on screen is being checked for mail.
    fn fetching(&self) -> bool {
        self.checks.running_for(self.mailbox, &self.query)
    }

    /// A check has its answer, or ran out of time. False when that is no longer wanted.
    fn finish_check(&mut self, check: u64, cx: &mut Context<Self>) -> bool {
        if !self.checks.finish(check, self.mailbox, &self.query) {
            return false;
        }
        // Whatever the answer, rows that are already here are not hidden behind the skeleton.
        if self.status == Status::Loading && !self.mails.is_empty() {
            self.status = Status::Ready;
        }
        cx.notify();
        true
    }

    /// Where the open mailbox's next page starts, if it is to be loaded.
    fn more(&self) -> Option<&String> {
        self.next_pages
            .get(&self.mailbox)
            .filter(|_| !self.more_failed.contains(&self.mailbox))
    }

    /// How much has been seen to change. It only grows while an account is shown.
    fn changes_seen(&self) -> u64 {
        let connected = self.gmail.iter().chain(self.others.values());
        self.feed_changes + connected.map(|client| client.writes()).sum::<u64>()
    }

    /// A mailbox that is already on screen from what is kept. Gmail is asked only if
    /// something changed since it was last checked, or that was a while ago, and the screen
    /// does not say so.
    fn look_again(&mut self, cx: &mut Context<Self>) {
        let changes = self.changes_seen();
        let checked = self.checked.get(&self.record_for(self.mailbox));
        if self.fetching() || checked.is_some_and(|asked| asked.stands(changes)) {
            return;
        }
        self.refresh(cx);
        if self.fetching() {
            self.checks.quiet();
        }
    }

    /// Forgets what was checked and when: the lists it was true of are gone.
    fn forget_checked(&mut self) {
        self.checked.clear();
        self.counted = None;
        self.paged.clear();
        self.all_inboxes_whole = false;
    }

    /// The open mailbox is being checked, and says so.
    fn shows_checking(&self) -> bool {
        self.checks.shows_for(self.mailbox, &self.query)
    }

    /// Brings the open mailbox up to date: only what changed since it was last synced when
    /// that is known, otherwise a full fetch of its newest threads.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.more_failed.remove(&self.mailbox);
        if !self.sync_view(cx) {
            self.fetch_inbox(cx);
        }
    }

    /// Applies Gmail's change feed to the open mailbox. Returns false when it cannot (no
    /// starting point, nothing on screen, or a search-based mailbox) so the caller fetches.
    fn sync_view(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(client) = self.gmail.clone() else {
            return false;
        };
        let view = self.mailbox;
        let label_id = match view {
            View::Label(index) => self.labels.get(index).map(|label| label.id.clone()),
            _ => None,
        };
        if view.holds(&[], label_id.as_deref()).is_none() || self.mails.is_empty() {
            return false;
        }
        let Some(start) = self.synced_to.get(&self.record_for(view)).cloned() else {
            return false;
        };
        if self.fetching() {
            return true;
        }
        let check = self.checks.start(view, &self.query);
        let asked = Asked::now(self.changes_seen());
        let count = self
            .counted
            .is_none_or(|at| !at.stands(self.changes_seen()));
        // From this Mac, a list has no token for its next page.
        let (find_page, listed) = (!self.paged.contains(&view), self.mails.len());
        cx.notify();
        let recipient = matches!(view, View::Sent | View::Drafts);
        let count_ids: Vec<String> = [
            "INBOX",
            "DRAFT",
            "CATEGORY_PERSONAL",
            "CATEGORY_UPDATES",
            "CATEGORY_PROMOTIONS",
            "CATEGORY_SOCIAL",
        ]
        .map(String::from)
        .into_iter()
        .chain(self.labels.iter().take(8).map(|l| l.id.clone()))
        .collect();
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let result = within(cx.background_executor(), async move {
                let changes = client.history(&start)?;
                if changes.threads.len() > 40 {
                    return Err(gmail::HistoryExpired.into());
                }
                // Fresh details for each changed thread; `None` when it is gone.
                let mut updates = Vec::new();
                for id in &changes.threads {
                    match client.summary(id, recipient) {
                        Ok(summary) => updates.push((id.clone(), Some(summary))),
                        Err(error) if error.to_string().starts_with("Gmail error 404") => {
                            updates.push((id.clone(), None))
                        }
                        Err(error) => return Err(error),
                    }
                }
                let counts = (count && !updates.is_empty()).then(|| client.counts(&count_ids));
                let page = if !find_page {
                    None
                } else if (1..=500).contains(&listed) {
                    client.page_after(view, label_id.as_deref(), listed).ok()
                } else {
                    Some(None)
                };
                anyhow::Ok(((changes.latest, updates, counts, page), label_id))
            })
            .await;
            this.update(cx, |this, cx| {
                // A different account was opened meanwhile: this answer is not for it.
                if this.account_gen != generation {
                    return;
                }
                if !this.finish_check(check, cx) {
                    return;
                }
                match result {
                    Ok(((latest, updates, counts, page), label_id)) => {
                        this.synced_to.insert(this.record_for(view), latest);
                        this.checked.insert(this.record_for(view), asked);
                        if let Some(next) = page {
                            this.paged.insert(view);
                            if let Some(next) = next {
                                this.next_pages.insert(view, next);
                            }
                        }
                        if let Some(counts) = counts.filter(|c| !c.is_empty()) {
                            this.counts = counts;
                            this.counted = Some(asked);
                        }
                        // Count only; never log subjects or addresses.
                        eprintln!(
                            "nimbra: {view:?} synced ({} changed threads)",
                            updates.len()
                        );
                        if updates.is_empty() {
                            this.persist(cx);
                            this.prefetch(cx);
                            cx.notify();
                            return;
                        }
                        let mut list: Vec<Mail> = this.mails.clone();
                        for (id, summary) in updates {
                            list.retain(|mail| mail.id.as_deref() != Some(id.as_str()));
                            if let Some(summary) = summary
                                && view.holds(&summary.labels, label_id.as_deref()) == Some(true)
                            {
                                list.push(Mail::from_summary(summary));
                            }
                        }
                        list.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms));
                        this.install(list, cx);
                    }
                    Err(error) if error.is::<gmail::HistoryExpired>() => this.fetch_inbox(cx),
                    Err(error) => this.fail(error, cx),
                }
            })
            .ok();
        })
        .detach();
        true
    }

    /// Fetches, in the background, the content of the newest threads on screen that have not
    /// been downloaded yet, and saves it, so opening them is instant now and after a restart.
    fn prefetch(&mut self, cx: &mut Context<Self>) {
        // A thread in full is 120 units of the 6,000 a minute: not the whole screen.
        const AHEAD: usize = 3;
        const WORKERS: usize = 4;
        if self.gmail.is_none() {
            return;
        }
        if self.prefetching || self.mailbox == View::Search || self.status != Status::Ready {
            return;
        }
        // Each thread with its own account's connection (All inboxes mixes accounts).
        let mut ids = Vec::new();
        let mut accounts = Vec::new();
        let mut clients = Vec::new();
        // The top few only, not the next few that are missing: each check would fetch more.
        for i in self.visible().into_iter().take(AHEAD) {
            if i == self.selected {
                continue; // the one on show is loaded by itself
            }
            if let Some(mail) = self.mails.get(i).filter(|mail| !mail.loaded)
                && let (Some(id), Some(client)) = (mail.id.clone(), self.client_at(i))
            {
                ids.push(id);
                accounts.push(self.account_of(mail).to_string());
                clients.push(client);
            }
        }
        if ids.is_empty() {
            return;
        }
        self.prefetching = true;
        let view = self.mailbox;
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let fetched = within(cx.background_executor(), async move {
                let mut results: Vec<Option<anyhow::Result<Vec<gmail::Message>>>> =
                    (0..ids.len()).map(|_| None).collect();
                let chunk = ids.len().div_ceil(WORKERS).max(1);
                std::thread::scope(|scope| {
                    for ((ids, clients), out) in ids
                        .chunks(chunk)
                        .zip(clients.chunks(chunk))
                        .zip(results.chunks_mut(chunk))
                    {
                        scope.spawn(move || {
                            for ((id, client), slot) in ids.iter().zip(clients).zip(out) {
                                *slot = Some(client.thread(id));
                            }
                        });
                    }
                });
                Ok(ids
                    .into_iter()
                    .zip(accounts)
                    .zip(results.into_iter().flatten())
                    .collect::<Vec<_>>())
            })
            .await
            .unwrap_or_default();
            this.update(cx, |this, cx| {
                // A different account was opened meanwhile: this answer is not for it.
                if this.account_gen != generation {
                    return;
                }
                this.prefetching = false;
                let mut filled = 0;
                for ((id, account), result) in fetched {
                    match result {
                        Ok(messages) => {
                            if let Some(row) = this.row_of(&id, &account)
                                && !this.mails[row].loaded
                            {
                                this.mails[row].fill(messages);
                                filled += 1;
                            }
                        }
                        Err(error) if error.is::<gmail::SignedOut>() => {
                            this.fail(error, cx);
                            return;
                        }
                        // One thread failing (offline, deleted) does not stop the rest.
                        Err(_) => {}
                    }
                }
                if filled > 0 && this.mailbox == view {
                    // Count only; never log subjects or addresses.
                    eprintln!("nimbra: prefetched {filled} threads");
                    this.cache.insert(view, this.listed());
                    this.persist(cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    // ------------------------------------------------------------ local store

    /// Opens the encrypted mail store in the background and, if nothing has arrived from
    /// Gmail yet, shows what was saved last time straight away.
    fn open_store(&mut self, cx: &mut Context<Self>) {
        // Ask once for permission to notify about new mail (macOS remembers the answer).
        static ASKED: std::sync::Once = std::sync::Once::new();
        ASKED.call_once(|| {
            crate::notify::show_while_active();
            crate::notify::request_permission();
            crate::notify::log_permission();
        });
        // NIMBRA_NOTIFY_TEST=1 posts one sample notification, to check the setup.
        if std::env::var_os("NIMBRA_NOTIFY_TEST").is_some() {
            crate::notify::show("test", "Nimbra", "Notifications are working.", true);
        }
        self.connect_others(cx);
        if self.store.is_some() {
            self.refresh(cx);
            return;
        }
        let view = self.mailbox;
        let view_record = self.record_for(view);
        let account = self.account_id.clone();
        let generation = self.account_gen;
        let client = self.gmail.clone();
        cx.spawn(async move |this, cx| {
            let opened = cx
                .background_executor()
                .spawn(async move {
                    let store = Arc::new(crate::store::Store::open(&account)?);
                    let saved: Option<Saved> = store.load("meta");
                    let mails: Option<Vec<Mail>> = store.load(&view_record);
                    let unsent = client.map_or_else(Vec::new, |client| {
                        if let Some(mut older) = store.load::<Outbox>("outbox")
                            && !older.mail.is_empty()
                        {
                            older.mail.retain(|mail| {
                                !Job::queued(&account, &client, &store, mail.clone())
                                    .is_ok_and(|job| job.keep())
                            });
                            if store.save("outbox", &older).is_err() {
                                eprintln!("nimbra: could not move the offline queue over");
                            }
                        }
                        Job::left_behind(&account, &client, &store)
                    });
                    anyhow::Ok((store, saved, mails, unsent))
                })
                .await;
            let Ok((store, saved, mails, unsent)) = opened else {
                eprintln!("nimbra: mail store unavailable; continuing without it");
                this.update(cx, |this, cx| {
                    if this.account_gen != generation {
                        return;
                    }
                    this.inbox_saved_empty = false;
                    this.tell(STORE_UNAVAILABLE);
                    this.fetch_inbox(cx)
                })
                .ok();
                return;
            };
            this.update(cx, |this, cx| {
                if this.account_gen != generation {
                    return;
                }
                // Read here, not above: a catch-up from before a switch may just have
                // taken what it sent out of it.
                let mut outbox: Outbox = store.load("outbox").unwrap_or_default();
                this.scheduled = scheduled::list(&store);
                this.snoozed = store.load("snoozed").unwrap_or_default();
                let queued_meanwhile = !this.outbox.changes.is_empty();
                outbox.changes.append(&mut this.outbox.changes);
                this.outbox = outbox;
                this.store = Some(store);
                if queued_meanwhile {
                    this.save_outbox();
                }
                if view == View::Inbox {
                    this.inbox_saved_empty = mails.as_ref().is_some_and(Vec::is_empty);
                }
                for job in unsent {
                    this.pick_up_send(job, cx);
                }
                if !this.outbox.changes.is_empty() || this.waiting_mail().next().is_some() {
                    this.went_offline(cx);
                }
                if !this.scheduled.is_empty() || !this.snoozed.is_empty() {
                    this.watch_schedule(cx);
                }
                // Something is already on screen (a sign-in in this session): just refresh.
                if !this.mails.is_empty() || this.mailbox != view {
                    this.refresh(cx);
                    return;
                }
                if let Some(saved) = saved {
                    if saved.account.contains('@') {
                        let generation = this.account_gen;
                        this.remember_account(&saved.account, cx);
                        if this.account_gen != generation {
                            return;
                        }
                    }
                    this.account = saved.account.into();
                    this.me_initials = saved.initials.into();
                    this.counts = saved.counts;
                    this.labels = saved.labels;
                    this.synced_to = saved.synced_to;
                }
                if let Some(mut mails) = mails {
                    for mail in &mut mails {
                        mail.refresh_time();
                    }
                    if mails.is_empty() {
                        // Known to be empty, so checking it again needs no loading screen.
                        this.cache.insert(view, Vec::new());
                    }
                    eprintln!("nimbra: opened {} stored threads", mails.len());
                    this.done = vec![false; mails.len()];
                    this.mails = mails;
                    this.selected = 0;
                    this.status = Status::Ready;
                    this.load_selected(cx);
                }
                // Bring it up to date: only what changed, when the store knows from where.
                this.refresh(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// What the store holds for a mailbox: `None` if it was never listed, and an empty
    /// list for a mailbox that was listed and had nothing in it.
    fn stored_mails(&self, view: View) -> Option<Vec<Mail>> {
        let mut mails: Vec<Mail> = self.store.as_ref()?.load(&self.record_for(view))?;
        for mail in &mut mails {
            mail.refresh_time();
        }
        Some(mails)
    }

    /// Saves the open mailbox and the sidebar state, in the background. Search results are
    /// never stored.
    fn persist(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        // All inboxes mixes accounts: it is never saved in any one account's store.
        if matches!(self.mailbox, View::Search | View::AllInboxes) || self.status != Status::Ready {
            return;
        }
        // Keep the store small: the newest 300 threads of a mailbox, and the full text of
        // the newest 100 (older ones are downloaded again when opened).
        const KEEP_THREADS: usize = 300;
        const KEEP_BODIES: usize = 100;
        let mails: Vec<Mail> = self
            .mails
            .iter()
            .zip(&self.done)
            .filter(|(_, done)| !**done)
            .map(|(mail, _)| mail.clone())
            .take(KEEP_THREADS)
            .enumerate()
            .map(|(index, mut mail)| {
                if index >= KEEP_BODIES && mail.loaded {
                    mail.loaded = false;
                    mail.body.clear();
                    mail.earlier.clear();
                    mail.html = None;
                }
                mail
            })
            .collect();
        if self.mailbox == View::Inbox {
            self.inbox_saved_empty = mails.is_empty();
        }
        let name = self.record_for(self.mailbox);
        let taken = store.snapshot();
        let saved = Saved {
            account: self.account.to_string(),
            initials: self.me_initials.to_string(),
            counts: self.counts.clone(),
            labels: self.labels.clone(),
            synced_to: self.synced_to.clone(),
        };
        cx.background_executor()
            .spawn(async move {
                // The list first: a sync point ahead of its list would miss what changed.
                if let Err(error) = store
                    .save_newest(&name, &mails, taken)
                    .and_then(|_| store.save_newest("meta", &saved, taken))
                {
                    eprintln!("nimbra: saving the mail store failed: {error}");
                }
            })
            .detach();
    }

    /// Opens the thread behind a clicked notification. macOS brings the app forward itself.
    fn watch_notification_clicks(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                let clicked = crate::notify::take_clicked();
                let Some(id) = clicked.into_iter().rev().find(|id| id.as_str() != "test") else {
                    if this.update(cx, |_, _| ()).is_err() {
                        break; // the window is gone
                    }
                    continue;
                };
                // The window may be hidden (closed with ⌘W): bring it back first.
                cx.update(crate::show_main_window).ok();
                this.update(cx, |this, cx| this.open_thread(id, cx)).ok();
            }
        })
        .detach();
    }

    /// Shows a thread by its Gmail id: switches to the Inbox, and opens the thread now if
    /// it is listed, or as soon as the list has caught up.
    fn open_thread(&mut self, id: String, cx: &mut Context<Self>) {
        if self.compose.as_ref().is_some_and(|c| !c.full) {
            // Leave a message being written alone; just bring the app forward.
            return;
        }
        // Another account's notification ("acct:<account>:<thread>"): shown in All inboxes.
        let mut id = id;
        if let Some((account, thread)) = id
            .strip_prefix("acct:")
            .and_then(|rest| rest.split_once(':'))
            .map(|(a, t)| (a.to_string(), t.to_string()))
        {
            if account == self.account_id {
                id = thread;
            } else {
                if self.mailbox == View::AllInboxes {
                    self.refresh(cx);
                } else {
                    self.select_mailbox(View::AllInboxes, cx);
                }
                self.unread_only = false;
                if thread != "more" && !self.open_listed(&thread, cx) {
                    self.open_when_listed = Some(thread);
                    // Opened when it is listed, so it has to be listed again.
                    if !self.fetching() {
                        self.refresh(cx);
                    }
                }
                return;
            }
        }
        if self.mailbox != View::Inbox {
            self.select_mailbox(View::Inbox, cx);
        }
        self.unread_only = false;
        // "more" stands for several new threads: showing the Inbox is the answer.
        if id == "more" {
            return;
        }
        if !self.open_listed(&id, cx) {
            self.open_when_listed = Some(id);
            self.refresh(cx);
        }
    }

    /// Opens the thread with this id if it is in the list. Returns whether it was.
    fn open_listed(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        let Some(index) = self.mails.iter().position(|m| m.id.as_deref() == Some(id)) else {
            return false;
        };
        if self.done.get(index) == Some(&true) {
            return false;
        }
        self.open(index);
        self.open_selected(cx);
        cx.notify();
        true
    }

    /// Reads Gmail's change feed every 30 seconds while the app is open: shows a
    /// notification for each thread with a new unread message in the inbox, and applies
    /// any change to the open mailbox.
    fn watch_for_mail(&mut self, cx: &mut Context<Self>) {
        const EVERY: Duration = Duration::from_secs(30);
        cx.spawn(async move |this, cx| {
            // The first look happens as soon as the account is ready, so it only takes stock
            // of what is already unread; anything arriving after that is announced.
            let mut first = true;
            let mut failed = false;
            loop {
                let ready = this
                    .update(cx, |this, _| this.gmail.is_some())
                    .unwrap_or(false);
                let wait = if first && !ready {
                    Duration::from_secs(3)
                } else if first && failed {
                    // Without a pause a first look that fails is made again and again.
                    Duration::from_secs(15)
                } else if first {
                    Duration::ZERO
                } else {
                    this.update(cx, |this, _| {
                        Duration::from_secs(this.settings.check_every.clamp(15, 3600) as u64)
                    })
                    .unwrap_or(EVERY)
                };
                cx.background_executor().timer(wait).await;
                let Ok(client) = this.update(cx, |this, _| {
                    this.gmail.clone().map(|c| (c, this.account_gen))
                }) else {
                    break; // the window is gone
                };
                let Some((client, generation)) = client else {
                    continue; // not signed in
                };
                let from = this
                    .update(cx, |this, _| this.notify_from.clone())
                    .ok()
                    .flatten();
                let check = {
                    let client = client.clone();
                    cx.background_executor()
                        .spawn(async move {
                            match from {
                                // First look: just remember where the feed is now.
                                None => client.profile().map(|(_, at)| (at, None)),
                                Some(from) => match client.history(&from) {
                                    Ok(changes) => Ok((changes.latest.clone(), Some(changes))),
                                    Err(error) if error.is::<gmail::HistoryExpired>() => {
                                        client.profile().map(|(_, at)| (at, None))
                                    }
                                    Err(error) => Err(error),
                                },
                            }
                        })
                        .await
                };
                failed = check.is_err();
                let Ok((at, changes)) = check else {
                    continue; // offline or signed out: try again next time
                };
                first = false;
                let fresh = this
                    .update(cx, |this, cx| {
                        // Switched accounts meanwhile: this feed belongs to the other one.
                        if this.account_gen != generation {
                            return Vec::new();
                        }
                        this.notify_from = Some(at);
                        let Some(changes) = changes else {
                            return Vec::new();
                        };
                        if !changes.threads.is_empty() {
                            this.feed_changes += 1;
                        }
                        // Anything changed: bring the open mailbox up to date, cheaply.
                        if !changes.threads.is_empty() && this.compose.is_none() {
                            // All inboxes cannot be patched from one feed: list it again.
                            if this.mailbox == View::AllInboxes {
                                // Started again each time, a slow one would never finish.
                                if !this.fetching() {
                                    this.fetch_all_inboxes(cx);
                                    this.checks.quiet();
                                }
                            } else if !this.fetching() && this.sync_view(cx) {
                                this.checks.quiet();
                            }
                        }
                        changes.new_unread_inbox
                    })
                    .unwrap_or_default();
                if fresh.is_empty() {
                    continue;
                }
                // Sender and subject for the first few; the rest are summed up.
                let shown: Vec<String> = fresh.iter().take(3).cloned().collect();
                let summaries = cx
                    .background_executor()
                    .spawn(async move {
                        shown
                            .iter()
                            .filter_map(|id| client.summary(id, false).ok())
                            .collect::<Vec<_>>()
                    })
                    .await;
                let extra = fresh.len().saturating_sub(summaries.len());
                this.update(cx, |this, _| {
                    if !this.settings.notifications {
                        return;
                    }
                    for mail in &summaries {
                        crate::notify::show(
                            &mail.id,
                            &mail.sender,
                            &mail.subject,
                            this.settings.notify_sound,
                        );
                    }
                    if extra > 0 {
                        crate::notify::show(
                            "more",
                            "Nimbra",
                            &format!("{extra} more new messages"),
                            this.settings.notify_sound,
                        );
                    }
                    // Count only; never log senders or subjects.
                    eprintln!("nimbra: {} new unread threads", fresh.len());
                })
                .ok();
            }
        })
        .detach();
    }

    /// The accounts that are not shown: their change feeds are read on the same schedule
    /// as the shown one's, for notifications, unread counts and All inboxes; and the
    /// snoozes and scheduled sends kept in their stores are carried out when due.
    fn watch_other_accounts(&mut self, cx: &mut Context<Self>) {
        const EVERY: Duration = Duration::from_secs(30);
        cx.spawn(async move |this, cx| {
            // Where each account's feed was last read, by account id.
            let mut feeds: HashMap<String, String> = HashMap::new();
            loop {
                let wait = this
                    .update(cx, |this, _| {
                        Duration::from_secs(this.settings.check_every.clamp(15, 3600) as u64)
                    })
                    .unwrap_or(EVERY);
                cx.background_executor().timer(wait).await;
                let Ok((others, connected, writes)) = this.update(cx, |this, _| {
                    let others = this
                        .others
                        .iter()
                        .map(|(id, client)| (id.clone(), client.clone()))
                        .collect::<Vec<_>>();
                    // With the shown one's: a store can hold another account's threads.
                    let mut connected = this.others.clone();
                    if let Some(client) = &this.gmail {
                        connected.insert(this.account_id.clone(), client.clone());
                    }
                    (others, connected, this.writes.clone())
                }) else {
                    break; // the window is gone
                };
                feeds.retain(|id, _| others.iter().any(|(other, _)| other == id));
                for (id, client) in others {
                    let from = feeds.get(&id).cloned();
                    let account = id.clone();
                    let connected = connected.clone();
                    let writes = writes.clone();
                    let checked = cx
                        .background_executor()
                        .spawn(async move {
                            let (at, fresh, changed) = match from {
                                None => (client.profile()?.1, Vec::new(), false),
                                Some(from) => match client.history(&from) {
                                    Ok(changes) => (
                                        changes.latest.clone(),
                                        changes.new_unread_inbox.clone(),
                                        !changes.threads.is_empty(),
                                    ),
                                    Err(error) if error.is::<gmail::HistoryExpired>() => {
                                        (client.profile()?.1, Vec::new(), true)
                                    }
                                    Err(error) => return Err(error),
                                },
                            };
                            let unread = client
                                .counts(&["INBOX".to_string()])
                                .get("INBOX")
                                .map_or(0, |(unread, _)| *unread);
                            let summaries: Vec<gmail::Summary> = fresh
                                .iter()
                                .take(3)
                                .filter_map(|thread| client.summary(thread, false).ok())
                                .collect();
                            let extra = fresh.len().saturating_sub(summaries.len());
                            // Snoozes and scheduled sends kept with this account.
                            carry_out_due(&account, &connected, &writes);
                            anyhow::Ok((at, summaries, extra, changed, unread))
                        })
                        .await;
                    let Ok((at, summaries, extra, changed, unread)) = checked else {
                        continue; // offline or signed out: next time
                    };
                    feeds.insert(id.clone(), at);
                    this.update(cx, |this, cx| {
                        if !this.others.contains_key(&id) {
                            return;
                        }
                        this.account_unread.insert(id.clone(), unread);
                        if changed {
                            this.feed_changes += 1;
                        }
                        if this.settings.notifications {
                            for mail in &summaries {
                                crate::notify::show(
                                    &format!("acct:{id}:{}", mail.id),
                                    &mail.sender,
                                    &mail.subject,
                                    this.settings.notify_sound,
                                );
                            }
                            if extra > 0 {
                                crate::notify::show(
                                    &format!("acct:{id}:more"),
                                    "Nimbra",
                                    &format!("{extra} more new messages"),
                                    this.settings.notify_sound,
                                );
                            }
                        }
                        if changed
                            && this.mailbox == View::AllInboxes
                            && this.compose.is_none()
                            && !this.fetching()
                        {
                            this.fetch_all_inboxes(cx);
                            this.checks.quiet();
                        }
                        cx.notify();
                    })
                    .ok();
                }
            }
        })
        .detach();
    }

    /// Opens Google's consent page in the browser and waits for it in the background.
    fn sign_in(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            return;
        };
        let account = self.account_id.clone();
        match gmail::SignIn::start(&config, false) {
            Ok((pending, url)) => {
                cx.open_url(&url);
                self.status = Status::SigningIn;
                self.notice = None;
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move { pending.finish(&config, &account) })
                        .await;
                    this.update(cx, |this, cx| match result {
                        Ok(client) => {
                            this.gmail = Some(Arc::new(client));
                            this.open_store(cx);
                        }
                        Err(error) => {
                            this.status = Status::Failed(error.to_string().into());
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
            }
            Err(error) => self.status = Status::Failed(error.to_string().into()),
        }
        cx.notify();
    }

    /// Add account: signs in to another Gmail account in the browser, then switches to it.
    /// The account shown stays as it is until that sign-in finishes.
    fn add_account(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            self.notice = Some("Connecting Gmail needs the Google sign-in set up first.".into());
            cx.notify();
            return;
        };
        let Ok(account) = gmail::new_account_id() else {
            return;
        };
        match gmail::SignIn::start(&config, false) {
            Ok((pending, url)) => {
                cx.open_url(&url);
                self.notice = Some("Sign in to the other account in your browser.".into());
                cx.spawn(async move |this, cx| {
                    let id = account.clone();
                    let result = cx
                        .background_executor()
                        .spawn(async move { pending.finish(&config, &id) })
                        .await;
                    this.update(cx, |this, cx| match result {
                        Ok(client) => {
                            this.reset_account(account, cx);
                            this.gmail = Some(Arc::new(client));
                            this.open_store(cx);
                        }
                        Err(error) => {
                            this.notice = Some(format!("No account was added: {error}").into());
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
            }
            Err(error) => self.notice = Some(error.to_string().into()),
        }
        cx.notify();
    }

    /// Shows another of the accounts signed in on this Mac.
    fn switch_account(&mut self, id: String, cx: &mut Context<Self>) {
        if self.config.is_none() || id == self.account_id || !gmail::is_account_id(&id) {
            return;
        }
        self.reset_account(id, cx);
        self.connect(false, cx);
    }

    /// Connects the account shown with its saved sign-in. `sign_in` is for when there is
    /// none: whether to go on to Google, as the button that asked for it says.
    fn connect(&mut self, sign_in: bool, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            return;
        };
        let id = self.account_id.clone();
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let key = id.clone();
            let token = cx
                .background_executor()
                .spawn(async move { gmail::stored_refresh_token(&key) })
                .await;
            this.update(cx, |this, cx| {
                if this.account_gen != generation || this.account_id != id || this.gmail.is_some() {
                    return;
                }
                match token {
                    Ok(Some(token)) => {
                        this.gmail = Some(Arc::new(gmail::Client::new(config, token, id)));
                        this.open_store(cx);
                    }
                    Ok(None) if sign_in => this.sign_in(cx),
                    Ok(None) => this.status = Status::SignedOut,
                    // Not signed out: signing in over it could wipe this account, or send
                    // its mail from another.
                    Err(error) => {
                        eprintln!("nimbra: {error:#}");
                        this.status = Status::Failed(KEYCHAIN_REFUSED.into());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The status screen's Try again.
    fn try_again(&mut self, cx: &mut Context<Self>) {
        if self.gmail.is_none() {
            self.status = Status::Loading;
            self.connect(true, cx);
        } else if self.store.is_none() {
            // Asks the keychain again.
            self.open_store(cx);
        } else {
            self.fetch_inbox(cx);
        }
        cx.notify();
    }

    /// Opens a connection to every signed-in account that is not shown (their sign-ins are
    /// in the keychain). Accounts already connected are left as they are.
    fn connect_others(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            return;
        };
        let missing: Vec<String> = self
            .settings
            .accounts
            .iter()
            .map(|a| a.id.clone())
            .filter(|id| *id != self.account_id && !self.others.contains_key(id))
            .filter(|id| gmail::is_account_id(id))
            .collect();
        if missing.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async move {
                    missing
                        .into_iter()
                        .filter_map(|id| Some((gmail::stored_refresh_token(&id).ok()??, id)))
                        .map(|(token, id)| (id, token))
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| {
                for (id, token) in found {
                    if id != this.account_id && this.settings.accounts.iter().any(|a| a.id == id) {
                        let client = gmail::Client::new(config.clone(), token, id.clone());
                        this.others.insert(id, Arc::new(client));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The connection for an account: the shown one, or another signed-in one.
    fn client_for(&self, account: Option<&str>) -> Option<Arc<gmail::Client>> {
        match account {
            Some(id) if id != self.account_id => self.others.get(id).cloned(),
            _ => self.gmail.clone(),
        }
    }

    /// The thread in row `index` belongs to an account other than the one shown.
    fn is_foreign(&self, index: usize) -> bool {
        self.mails
            .get(index)
            .and_then(|m| m.account.as_deref())
            .is_some_and(|a| a != self.account_id)
    }

    /// The connection for the thread in row `index` (its own account in All inboxes).
    fn client_at(&self, index: usize) -> Option<Arc<gmail::Client>> {
        let account = self.mails.get(index).and_then(|m| m.account.clone());
        self.client_for(account.as_deref())
    }

    /// Clears everything that belongs to the account shown, and makes `id` the account shown.
    fn reset_account(&mut self, id: String, cx: &mut Context<Self>) {
        self.close_menu_window(cx);
        self.close_undo(cx);
        // What is being written is saved to its account's Drafts.
        self.close_compose(cx);
        // What is still finishing in the background stays with the account it is from.
        for (_, composer) in &mut self.parked {
            let from = composer
                .account
                .take()
                .unwrap_or_else(|| self.account_id.clone());
            composer.account = (from != id).then_some(from);
        }
        self.account_gen += 1;
        // The account shown so far stays connected (All inboxes, new-mail checks).
        if let Some(client) = self.gmail.take()
            && !self.account_id.is_empty()
            && self
                .settings
                .accounts
                .iter()
                .any(|a| a.id == self.account_id)
        {
            self.others.insert(self.account_id.clone(), client);
        }
        self.others.remove(&id);
        self.inbox_saved_empty = crate::store::Store::saved_empty(&id, &record(View::Inbox));
        self.account_id = id.clone();
        self.settings.active = id;
        self.save_settings(cx);
        self.gmail = None;
        self.store = None;
        self.status = Status::Loading;
        self.checks.forget();
        // What was on its way is for the other account, and its answer is dropped.
        self.prefetching = false;
        self.loading_more = false;
        self.body_failed.clear();
        self.mailbox = View::Inbox;
        self.mails.clear();
        self.done.clear();
        self.history.clear();
        self.history_groups.clear();
        self.cache.clear();
        self.counts.clear();
        self.labels.clear();
        self.synced_to.clear();
        self.scheduled.clear();
        self.sending_scheduled.clear();
        self.snoozed.clear();
        self.waking.clear();
        self.outbox = Outbox::default();
        self.offline = false;
        self.stop_retrying();
        self.picked.clear();
        // Asked about the other account's threads and labels, whose ids mean others here.
        self.confirm = None;
        self.new_label = None;
        self.next_pages.clear();
        self.forget_checked();
        self.more_failed.clear();
        self.back.clear();
        self.forward.clear();
        self.selected = 0;
        self.palette = false;
        self.removed_by.clear();
        self.moved.clear();
        self.notify_from = None;
        self.details = None;
        self.open_when_listed = None;
        self.search_opened = None;
        self.html_shown = None;
        self.print_view = None;
        self.account = "".into();
        self.me_initials = "".into();
        self.me_photo = None;
        self.photo_not_allowed = false;
        self.photo_gen = None;
        self.pending_focus = Some(self.focus.clone());
        cx.notify();
    }

    /// Remembers the account shown in the account list (once its address is known). Signing
    /// in to an account that is already listed keeps the existing entry.
    fn remember_account(&mut self, email: &str, cx: &mut Context<Self>) {
        let id = self.account_id.clone();
        if let Some(other) = self
            .settings
            .accounts
            .iter()
            .find(|a| a.email.eq_ignore_ascii_case(email) && a.id != id)
            .map(|a| a.id.clone())
        {
            // The same Google account twice: drop the new sign-in, show the existing one.
            let duplicate = id.clone();
            std::thread::spawn(move || {
                gmail::forget_refresh_token(&duplicate);
                crate::store::wipe(&duplicate);
            });
            self.notice = Some("That account is already in Nimbra.".into());
            self.switch_account(other, cx);
            return;
        }
        match self.settings.accounts.iter_mut().find(|a| a.id == id) {
            Some(entry) if entry.email == email => return,
            Some(entry) => entry.email = email.to_string(),
            None => self.settings.accounts.push(crate::settings::AccountRef {
                id: id.clone(),
                email: email.to_string(),
            }),
        }
        self.settings.active = id;
        self.save_settings(cx);
    }

    fn fetch_inbox(&mut self, cx: &mut Context<Self>) {
        if self.mailbox == View::AllInboxes {
            self.fetch_all_inboxes(cx);
            return;
        }
        let Some(client) = self.gmail.clone() else {
            return;
        };
        // The loading screen is only for a mailbox never listed before; one known to be
        // empty keeps showing as empty while it is checked again.
        if self.mails.is_empty() && !self.cache.contains_key(&self.mailbox) {
            self.status = Status::Loading;
        }
        let view = self.mailbox;
        let check = self.checks.start(view, &self.query);
        let asked = Asked::now(self.changes_seen());
        cx.notify();
        let label_id = match view {
            View::Label(index) => self.labels.get(index).map(|label| label.id.clone()),
            View::Search => Some(self.query.clone()),
            _ => None,
        };
        let need_labels = self.labels.is_empty();
        let known_labels: Vec<String> = self.labels.iter().map(|l| l.id.clone()).collect();
        let count = need_labels
            || self
                .counted
                .is_none_or(|at| !at.stands(self.changes_seen()));
        // A label is only known to hold by its id.
        let syncs = view.holds(&[], label_id.as_deref()).is_some();
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let result = within(cx.background_executor(), async move {
                // Read the history point first, so nothing that changes during the
                // fetch is missed by the next sync.
                let (email, history_id) = client.profile()?;
                let (threads, next_page) =
                    client.threads_page(view, label_id.as_deref(), 30, None)?;
                // Labels are a nicety; the mailbox still loads without them.
                let labels = need_labels.then(|| client.labels().unwrap_or_default());
                // Sidebar counts: the system mailboxes plus the labels that are listed.
                let mut ids: Vec<String> = [
                    "INBOX",
                    "DRAFT",
                    "CATEGORY_PERSONAL",
                    "CATEGORY_UPDATES",
                    "CATEGORY_PROMOTIONS",
                    "CATEGORY_SOCIAL",
                ]
                .map(String::from)
                .to_vec();
                ids.extend(match &labels {
                    Some(labels) => labels.iter().take(8).map(|l| l.id.clone()).collect(),
                    None => known_labels.into_iter().take(8).collect::<Vec<_>>(),
                });
                let counts = if count {
                    client.counts(&ids)
                } else {
                    HashMap::new()
                };
                anyhow::Ok((email, history_id, threads, labels, counts, next_page))
            })
            .await;
            this.update(cx, |this, cx| {
                // A different account was opened meanwhile: this answer is not for it.
                if this.account_gen != generation {
                    return;
                }
                // Ignore the answer if the user has moved to another mailbox or search
                // meanwhile, or a newer check is out.
                if !this.finish_check(check, cx) {
                    return;
                }
                match result {
                    Ok((email, history_id, threads, labels, counts, next_page)) => {
                        this.checked.insert(this.record_for(view), asked);
                        this.paged.insert(view);
                        match next_page {
                            Some(page) => this.next_pages.insert(view, page),
                            None => this.next_pages.remove(&view),
                        };
                        if syncs {
                            this.synced_to.insert(this.record_for(view), history_id);
                        }
                        if !counts.is_empty() {
                            this.counts = counts;
                            this.counted = Some(asked);
                        }
                        if let Some(labels) = labels {
                            this.labels = labels;
                        }
                        this.show_inbox(email, threads, cx)
                    }
                    Err(error) => this.fail(error, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// All inboxes: the first page of every signed-in account's inbox, fetched side by side,
    /// merged newest first, each row marked with its account. Also every account's unread
    /// count. An account that fails is left out (with a note); all failing is an error.
    fn fetch_all_inboxes(&mut self, cx: &mut Context<Self>) {
        let Some(shown) = self.gmail.clone() else {
            return;
        };
        let mut accounts: Vec<(String, Arc<gmail::Client>)> =
            vec![(self.account_id.clone(), shown)];
        for account in &self.settings.accounts {
            if let Some(client) = self.others.get(&account.id) {
                accounts.push((account.id.clone(), client.clone()));
            }
        }
        if self.mails.is_empty() && !self.cache.contains_key(&View::AllInboxes) {
            self.status = Status::Loading;
        }
        let check = self.checks.start(View::AllInboxes, "");
        let seen = Asked::now(self.changes_seen());
        cx.notify();
        let generation = self.account_gen;
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor();
            // A deadline each, so that one account cannot hold up the others.
            let results = futures::future::join_all(accounts.into_iter().map(|(id, client)| {
                let listing = within(executor, async move {
                    let (threads, next) = client.threads_page(View::Inbox, None, 30, None)?;
                    let unread = client
                        .counts(&["INBOX".to_string()])
                        .get("INBOX")
                        .map_or(0, |(unread, _)| *unread);
                    Ok((threads, next.is_some(), unread))
                });
                async move { (id, listing.await) }
            }))
            .await;
            this.update(cx, |this, cx| {
                if this.account_gen != generation || !this.finish_check(check, cx) {
                    return;
                }
                let mut fresh: Vec<Mail> = Vec::new();
                let mut first_error = None;
                let mut failed = 0;
                let mut more = false;
                let asked = results.len();
                for (id, result) in results {
                    match result {
                        Ok((threads, has_more, unread)) => {
                            more |= has_more;
                            this.account_unread.insert(id.clone(), unread);
                            fresh.extend(threads.into_iter().map(|summary| {
                                let mut mail = Mail::from_summary(summary);
                                mail.account = Some(id.clone());
                                mail
                            }));
                        }
                        Err(error) => {
                            failed += 1;
                            first_error.get_or_insert(error);
                        }
                    }
                }
                this.all_inboxes_whole =
                    failed == 0 && !more && asked >= this.settings.accounts.len();
                if failed == asked
                    && let Some(error) = first_error
                {
                    this.fail(error, cx);
                    return;
                }
                fresh.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms));
                this.install(fresh, cx);
                if failed == 0 {
                    this.checked.insert(this.record_for(View::AllInboxes), seen);
                }
                // After the listing, which clears the notice.
                if failed > 0 {
                    this.notice = Some(
                        format!("{failed} account(s) could not be checked; showing the rest.")
                            .into(),
                    );
                }
                // Count only; never addresses or subjects.
                eprintln!("nimbra: All inboxes listed ({} threads)", this.mails.len());
            })
            .ok();
        })
        .detach();
    }

    /// Shows a freshly fetched listing, reusing thread contents already in memory and
    /// keeping the open thread and any just-cleared threads as they were.
    fn show_inbox(&mut self, email: String, threads: Vec<gmail::Summary>, cx: &mut Context<Self>) {
        // A different Google account than the one the store was saved for: start clean.
        if self.account.contains('@') && self.account.as_ref() != email.as_str() {
            if let Some(store) = &self.store {
                store.clear();
            }
            self.cache.clear();
            self.outbox = Outbox::default();
            self.counts.clear();
            self.labels.clear();
            self.forget_checked();
        }
        self.me_initials = email
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default()
            .into();
        let generation = self.account_gen;
        self.remember_account(&email, cx);
        if self.account_gen != generation {
            return; // it was a second sign-in to an account already here
        }
        if self.photo_gen != Some(self.account_gen) {
            self.load_profile_photo(cx);
        }
        self.account = email.into();

        let fresh = threads.into_iter().map(Mail::from_summary).collect();
        self.install(fresh, cx);
    }

    /// The account's profile photo: the copy kept on this Mac first, then Google's current
    /// one (kept for next time). Once per account per session.
    fn load_profile_photo(&mut self, cx: &mut Context<Self>) {
        let generation = self.account_gen;
        self.photo_gen = Some(generation);
        self.me_photo = None;
        let (Some(client), store) = (self.gmail.clone(), self.store.clone()) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let kept: Option<Vec<u8>> = match &store {
                Some(store) => {
                    let store = store.clone();
                    cx.background_executor()
                        .spawn(async move { store.load::<Vec<u8>>("profile-photo") })
                        .await
                }
                None => None,
            };
            if let Some(bytes) = kept {
                this.update(cx, |this, cx| {
                    if this.account_gen == generation {
                        this.me_photo = photo_image(bytes);
                        cx.notify();
                    }
                })
                .ok();
            }
            let fetched = cx
                .background_executor()
                .spawn(async move { client.profile_photo() })
                .await;
            this.update(cx, |this, cx| {
                if this.account_gen != generation {
                    return;
                }
                match fetched {
                    Ok(gmail::Photo::Image(bytes)) => {
                        this.photo_not_allowed = false;
                        this.me_photo = photo_image(bytes.clone());
                        if let Some(store) = this.store.clone() {
                            cx.background_executor()
                                .spawn(async move {
                                    let _ = store.save("profile-photo", &bytes);
                                })
                                .detach();
                        }
                    }
                    Ok(gmail::Photo::NoPhoto) => {
                        this.photo_not_allowed = false;
                        this.me_photo = None;
                    }
                    Ok(gmail::Photo::NotAllowed) => this.photo_not_allowed = true,
                    Err(error) => {
                        // Counts only: never the address or the photo's link.
                        eprintln!("nimbra: profile photo not loaded ({error})");
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Settings › Accounts: signs in again to add the profile (name and photo) to this
    /// account's sign-in, then shows the photo.
    fn allow_profile_photo(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            return;
        };
        let account = self.account_id.clone();
        match gmail::SignIn::start(&config, false) {
            Ok((pending, url)) => {
                cx.open_url(&url);
                self.notice = Some("Allow your profile in your browser, then come back.".into());
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move { pending.finish(&config, &account) })
                        .await;
                    this.update(cx, |this, cx| match result {
                        Ok(client) => {
                            this.gmail = Some(Arc::new(client));
                            this.notice = None;
                            this.load_profile_photo(cx);
                        }
                        Err(error) => {
                            this.notice = Some(format!("The photo was not added: {error}").into());
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
            }
            Err(error) => self.notice = Some(error.to_string().into()),
        }
        cx.notify();
    }

    /// Puts a new listing on screen, keeping downloaded thread contents, the open thread and
    /// threads cleared in the meantime, then saves it.
    fn install(&mut self, fresh: Vec<Mail>, cx: &mut Context<Self>) {
        let mut fresh = fresh;
        // Snoozed lists Gmail's own snoozed threads and the ones snoozed here.
        if self.mailbox == View::Snoozed {
            for snooze in &self.snoozed {
                if fresh
                    .iter()
                    .any(|m| m.id.as_deref() == Some(snooze.thread_id.as_str()))
                {
                    continue;
                }
                let mut mail = snooze.mail.clone();
                mail.time = chrono::DateTime::from_timestamp_millis(snooze.until_ms)
                    .map(|t| {
                        t.with_timezone(&chrono::Local)
                            .format("%a %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default()
                    .into();
                fresh.push(mail);
            }
        }
        // Everything kept by position follows its thread into the new list.
        let relisted = crate::listing::relist(
            &self.mails,
            fresh,
            &self.history,
            |m| self.thread_key(m),
            |m| m.timestamp_ms,
        );
        self.mails = crate::data::merge(&self.mails, relisted.list);
        let now = |row: usize| relisted.moved.get(row).copied().flatten();
        for &position in relisted.back.iter().rev() {
            self.forget_cleared(position, cx);
        }
        // All found: a cleared thread is never dropped from the list.
        self.history = self.history.iter().filter_map(|&row| now(row)).collect();
        self.done = vec![false; self.mails.len()];
        for i in &self.history {
            self.done[*i] = true;
        }
        self.picked = self.picked.iter().filter_map(|&row| now(row)).collect();
        self.selected = now(self.selected).unwrap_or(0);
        // One picked row left is simply the open one (as when ⌘-click leaves one).
        if self.picked.len() < 2
            && let Some(only) = self.picked.pop_first()
        {
            self.selected = only;
        }
        if self.done.get(self.selected) == Some(&true) {
            self.selected = self.visible().first().copied().unwrap_or(0);
        }
        self.back.clear();
        self.forward.clear();
        if self.notice != self.must_show {
            self.notice = None;
        }
        self.status = Status::Ready;
        self.cache.insert(self.mailbox, self.listed());
        self.persist(cx);
        self.prefetch(cx);
        let opened = self.search_opened.take();
        if let Some(id) = self.open_when_listed.take()
            && !self.open_listed(&id, cx)
            && self.mailbox == View::Search
            && let Some(mail) = opened
            && mail.id.as_deref() == Some(id.as_str())
        {
            // Not in Gmail's results (another folder, or further down): kept at the top.
            self.mails.insert(0, mail);
            self.done.insert(0, false);
            self.history.iter_mut().for_each(|row| *row += 1);
            self.picked = self.picked.iter().map(|row| row + 1).collect();
            self.selected += 1;
            self.open(0);
            self.open_selected(cx);
        }
        // Counts only; never log addresses, subjects or message text.
        eprintln!(
            "nimbra: {:?} listed ({} threads, {} already in memory, {} sidebar counts)",
            self.mailbox,
            self.mails.len(),
            self.mails.iter().filter(|m| m.loaded).count(),
            self.counts.len()
        );
        self.load_selected(cx);
        cx.notify();
    }

    /// Fetches the messages of the open thread the first time it is shown.
    /// The user opened this thread themselves (clicked it, moved to it with the keyboard,
    /// or clicked its text): mark it read, here and in Gmail, and fetch its content.
    ///
    /// A thread that is only shown because a mailbox was just opened stays unread until
    /// then; see `load_selected`.
    fn open_selected(&mut self, cx: &mut Context<Self>) {
        self.kept_unread = None;
        match self.settings.mark_read {
            crate::settings::MarkRead::Open => self.set_unread(self.selected, false, cx),
            crate::settings::MarkRead::Never => {}
            // Read once it has stayed open for two seconds.
            crate::settings::MarkRead::Delay => {
                let id = self.mails.get(self.selected).and_then(|m| m.id.clone());
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_secs(2)).await;
                    this.update(cx, |this, cx| {
                        let still = this.mails.get(this.selected).and_then(|m| m.id.clone());
                        if still.is_some() && still == id && this.kept_unread.is_none() {
                            this.set_unread(this.selected, false, cx);
                        }
                    })
                    .ok();
                })
                .detach();
            }
        }
        self.load_selected(cx);
    }

    /// Fetches the content of the thread on show, without marking it read.
    fn load_selected(&mut self, cx: &mut Context<Self>) {
        let Some(mail) = self.mails.get(self.selected) else {
            return;
        };
        let (false, Some(id)) = (mail.loaded, mail.id.clone()) else {
            return;
        };
        let account = self.account_of(mail).to_string();
        self.load_body(id, account, cx);
    }

    /// Fetches a thread's content.
    fn load_body(&mut self, id: String, account: String, cx: &mut Context<Self>) {
        let key: ThreadKey = (id.clone(), account.clone());
        let Some(client) = self.client_for(Some(&account)) else {
            // Another account's, which is not connected: nothing is on its way.
            if self.gmail.is_some() {
                self.body_failed.insert(key);
                cx.notify();
            }
            return;
        };
        let generation = self.account_gen;
        self.body_failed.remove(&key);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let thread_id = id.clone();
            let result = within(cx.background_executor(), async move {
                client.thread(&thread_id)
            })
            .await;
            this.update(cx, |this, cx| {
                // A different account was opened meanwhile: this answer is not for it.
                if this.account_gen != generation {
                    return;
                }
                match result {
                    Ok(messages) => {
                        // The inbox may have been reloaded meanwhile, so look the thread up again.
                        if let Some(row) = this.row_of(&id, &account) {
                            this.mails[row].fill(messages);
                            this.persist(cx);
                        } else if let Some(mail) = this.cache.values_mut().flatten().find(|m| {
                            m.id.as_deref() == Some(id.as_str())
                                && m.account.as_deref().unwrap_or(&this.account_id) == account
                        }) {
                            // Its mailbox was left; its own window may still be waiting for it.
                            mail.fill(messages);
                        }
                        cx.notify();
                    }
                    Err(error) => {
                        this.body_failed.insert(key);
                        this.fail(error, cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Sends a change to Gmail. If it removed the thread from the list and Gmail refuses,
    /// the thread is put back.
    fn sync(&mut self, index: usize, change: gmail::Change, removed: bool, cx: &mut Context<Self>) {
        let (Some(client), Some(id)) = (
            self.client_at(index),
            self.mails.get(index).and_then(|m| m.id.clone()),
        ) else {
            return;
        };
        let account = self.account_of(&self.mails[index]).to_string();
        // It follows what waits for the connection, so it waits too.
        if account == self.account_id && self.outbox.changes.iter().any(|(queued, _)| *queued == id)
        {
            self.outbox.changes.push((id, change));
            self.save_outbox();
            self.went_offline(cx);
            return;
        }
        let step = self.steps.start(account.clone(), id.clone(), change);
        let working = self.writes.begin();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _working = working;
                    step.run(&client)
                })
                .await;
            this.update(cx, |this, cx| {
                let settled = this.steps.settle(&account, &id);
                // Offline: keep the change here and send it when the connection is back
                // (the queue is the shown account's; another account's change is undone).
                // Asked now, not when it was made: the account shown may have changed.
                let shown = account == this.account_id;
                let waiting: Vec<_> = settled
                    .into_iter()
                    .filter(|(_, end)| shown && *end == End::Offline)
                    .map(|(step, _)| (step.id.clone(), step.change))
                    .collect();
                if !waiting.is_empty() {
                    this.outbox.changes.extend(waiting);
                    this.save_outbox();
                    this.went_offline(cx);
                }
                let Err(error) = result else {
                    return;
                };
                if shown && error.is::<gmail::Offline>() {
                    return;
                }
                // Put back wherever the thread is by now.
                if removed && let Some(row) = this.row_of(&id, &account) {
                    this.done[row] = false;
                    if let Some(position) = this.history.iter().rposition(|i| *i == row) {
                        this.forget_cleared(position, cx);
                    }
                }
                this.fail(error, cx);
            })
            .ok();
        })
        .detach();
    }

    // ------------------------------------------------------------ composing

    fn new_field(
        &mut self,
        field: Field,
        placeholder: &str,
        cx: &mut Context<Self>,
    ) -> (Entity<TextInput>, gpui::Subscription) {
        let t = self.theme();
        let placeholder = placeholder.to_string();
        let input = cx.new(|cx| TextInput::new(cx, &placeholder, t.faint, t.violet));
        // Lines move around as the body is edited, so events look their line up by entity.
        let subscription = cx.subscribe(&input, move |this, input, event: &InputEvent, cx| {
            let field = match field {
                Field::Line(_) => this
                    .compose
                    .as_ref()
                    .and_then(|c| c.lines.iter().position(|line| *line == input))
                    .map(Field::Line),
                other => Some(other),
            };
            if let Some(field) = field {
                match event {
                    InputEvent::Changed => this.field_changed(field, cx),
                    event => this.field_event(field, event, cx),
                }
            }
        });
        (input, subscription)
    }

    /// Opens the composer, empty or as a reply to the open thread.
    fn open_compose(&mut self, kind: ComposeKind, cx: &mut Context<Self>) {
        let source = self
            .mails
            .get(self.selected)
            .filter(|_| !self.visible().is_empty())
            .map(|mail| Source::latest(mail));
        self.open_compose_from(kind, source, cx);
    }

    /// Opens the composer to reply to, reply to all on, or forward one message.
    fn open_compose_from(
        &mut self,
        kind: ComposeKind,
        source: Option<Source>,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        if self.compose.is_some() || self.palette {
            return;
        }
        let (to, s1) = self.new_field(Field::To, "name@example.com", cx);
        let (subject, s2) = self.new_field(Field::Subject, "Subject", cx);
        let (line, s3) = self.new_field(Field::Line(0), "Write your message…", cx);
        let (cc, s4) = self.new_field(Field::Cc, "", cx);
        let (bcc, s5) = self.new_field(Field::Bcc, "", cx);
        self.composer_serial += 1;
        let mut composer = Composer {
            serial: self.composer_serial,
            to,
            cc,
            bcc,
            show_cc: false,
            chips: Default::default(),
            suggest: None,
            files: Vec::new(),
            kept: Vec::new(),
            edits: 0,
            saved_edits: 0,
            resave: false,
            after_save: None,
            rich: !self.editor_failed && cfg!(any(target_os = "macos", target_os = "windows")),
            html: String::new(),
            text: String::new(),
            load: None,
            format: [false; 5],
            focus_body: false,
            subject,
            lines: vec![line],
            reply: None,
            full: false,
            position: None,
            size: (720., 520.),
            grab: None,
            draft_id: None,
            account: source.as_ref().and_then(|s| s.account.clone()),
            quoted: None,
            show_quoted: false,
            saving: false,
            saved_as: SavedAs::default(),
            saved: false,
            sending: false,
            error: None,
            _subscriptions: vec![s1, s2, s3, s4, s5],
        };
        let mut focus = composer.to.read(cx).focus_handle(cx);
        if kind != ComposeKind::New
            && let Some(source) = source
        {
            let prefix = if kind == ComposeKind::Forward {
                "Fwd: "
            } else {
                "Re: "
            };
            let subject = if source
                .subject
                .to_lowercase()
                .starts_with(&prefix.trim().to_lowercase())
            {
                source.subject.clone()
            } else {
                format!("{prefix}{}", source.subject)
            };
            composer
                .subject
                .update(cx, |input, cx| input.set(subject, usize::MAX, cx));
            let header = &source.header;
            let when = long_date(header.date_ms);
            let from = if header.from.address.is_empty() {
                source.sender.clone()
            } else if header.from.name.is_empty() {
                header.from.address.clone()
            } else {
                format!("{} <{}>", header.from.name, header.from.address)
            };
            let body: Vec<String> = source
                .body
                .iter()
                .flat_map(|block| {
                    block
                        .text
                        .lines()
                        .map(str::to_string)
                        .chain([String::new()])
                })
                .take(2000)
                .collect();
            if kind == ComposeKind::Forward {
                // The forwarded message is folded behind "•••" and added when sent.
                let to = header
                    .to
                    .iter()
                    .map(|p| p.address.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut quoted = vec![
                    "---------- Forwarded message ----------".to_string(),
                    format!("From: {from}"),
                ];
                if !when.is_empty() {
                    quoted.push(format!("Date: {when}"));
                }
                quoted.push(format!("Subject: {}", source.subject));
                if !to.is_empty() {
                    quoted.push(format!("To: {to}"));
                }
                quoted.push(String::new());
                quoted.extend(body);
                composer.quoted = Some(quoted.join("\n").trim_end().to_string());
            } else {
                let address = if header.from.address.is_empty() {
                    source.fallback_to.clone()
                } else {
                    Some(header.reply_address().to_string())
                };
                if let Some(address) = address {
                    let mut to = vec![address];
                    if kind == ComposeKind::ReplyAll {
                        let others = if header.from.address.is_empty() {
                            source.fallback_all.clone()
                        } else {
                            header.everyone()
                        };
                        for other in others {
                            let me = other.eq_ignore_ascii_case(&self.account);
                            if !me && !to.iter().any(|known| known.eq_ignore_ascii_case(&other)) {
                                to.push(other);
                            }
                        }
                    }
                    composer
                        .to
                        .update(cx, |input, cx| input.set(to.join(", "), usize::MAX, cx));
                    composer.chipify(Field::To, true, cx);
                }
                composer.reply = source.reply_to.clone();
                // The message being answered is quoted below, folded behind "•••".
                let intro = if when.is_empty() {
                    format!("{from} wrote:")
                } else {
                    format!("On {when}, {from} wrote:")
                };
                let quoted: Vec<String> = std::iter::once(intro)
                    .chain(body.iter().map(|line| format!("> {line}")))
                    .collect();
                if self.settings.quote_replies {
                    composer.quoted = Some(quoted.join("\n").trim_end().to_string());
                }
                focus = composer.lines[0].read(cx).focus_handle(cx);
            }
        }
        // The editor starts with the signature, below room to write.
        let signature = self.settings.signature.trim_end();
        let mut start = String::from("<div><br></div>");
        if !signature.is_empty() {
            start.push_str("<div><br></div><div class=\"sig\">");
            start.push_str(&crate::editor::text_to_html(signature));
            start.push_str("</div>");
        }
        composer.load = Some(start);
        if composer.rich
            && composer
                .lines
                .first()
                .is_some_and(|l| l.read(cx).focus_handle(cx) == focus)
        {
            composer.focus_body = true;
            focus = self.editor_focus.clone();
        }
        self.pending_focus = Some(focus);
        self.compose = Some(composer);
        cx.notify();
    }

    /// Creates, fills, shows or hides the message editor to match the composer.
    fn sync_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // While asking whether to save, the editor (a native view) steps aside so the
        // question shows over the composer.
        if self.compose_ask {
            if let Some(editor) = &self.editor {
                editor.set_visible(false);
                editor.blur();
            }
            return;
        }
        let Some(serial) = self.compose.as_ref().filter(|c| c.rich).map(|c| c.serial) else {
            if self.editor_shown.take().is_some()
                && let Some(editor) = &self.editor
            {
                editor.set_visible(false);
                editor.blur();
            }
            return;
        };
        // The composer moved to another window: the editor is made again there (its content
        // is kept in the composer and loaded once the new one is ready).
        if self.editor.is_some() && self.editor_window != Some(window.window_handle()) {
            if let Some(editor) = self.editor.take() {
                editor.set_visible(false);
            }
            self.editor_ready = false;
            self.editor_shown = None;
            self.editor_theme = None;
            self.editor_keys = false;
        }
        if self.editor.is_none() && !self.editor_failed {
            self.editor_window = Some(window.window_handle());
            let (events, mut incoming) = futures::channel::mpsc::unbounded();
            match crate::editor::EditorView::new(window, events) {
                Some(view) => {
                    self.editor = Some(std::rc::Rc::new(view));
                    cx.spawn(async move |this, cx| {
                        use futures::StreamExt as _;
                        while let Some(event) = incoming.next().await {
                            if this
                                .update(cx, |this, cx| this.editor_event(event, cx))
                                .is_err()
                            {
                                break;
                            }
                        }
                    })
                    .detach();
                }
                None => self.editor_failed = true,
            }
        }
        let Some(editor) = self.editor.clone().filter(|_| !self.editor_failed) else {
            // No editor here: fall back to the plain fields, which hold the same text.
            if let Some(composer) = &mut self.compose {
                composer.rich = false;
            }
            return;
        };
        if !self.editor_ready {
            return;
        }
        // Focus moved from the editor to one of the app's own fields (a click, Tab, ↑):
        // the web view must also give up the keyboard, or typing would still go there.
        let focused = self.editor_focus.is_focused(window);
        if self.editor_keys && !focused {
            editor.blur();
        }
        self.editor_keys = focused;
        let t = self.theme();
        let full = self.compose.as_ref().is_some_and(|c| c.full);
        if self.editor_theme != Some((self.dark, full)) {
            let css = |color: Hsla| {
                let c = color.to_rgb();
                format!(
                    "rgba({},{},{},{:.3})",
                    (c.r * 255.).round(),
                    (c.g * 255.).round(),
                    (c.b * 255.).round(),
                    c.a
                )
            };
            editor.theme(serde_json::json!({
                "--bg": css(if full { t.surface } else { t.raised }),
                "--text": css(t.text),
                "--dim": css(t.text3),
                "--accent": css(t.violet),
                "--line": css(t.border_strong),
                "--max": if full { "860px" } else { "none" },
                "--pad": if full { "24px" } else { "14px" },
            }));
            self.editor_theme = Some((self.dark, full));
        }
        let shown = self.editor_shown;
        let Some(composer) = self.compose.as_mut() else {
            return;
        };
        let focus = std::mem::take(&mut composer.focus_body);
        if shown != Some(serial) || composer.load.is_some() {
            let html = composer
                .load
                .take()
                .unwrap_or_else(|| composer.html.clone());
            editor.load(
                serial,
                &html,
                composer.quoted.as_deref().unwrap_or(""),
                focus,
            );
            self.editor_shown = Some(serial);
        } else if focus {
            editor.focus();
        }
        editor.set_visible(true);
    }

    fn editor_event(&mut self, event: crate::editor::EditorEvent, cx: &mut Context<Self>) {
        use crate::editor::EditorEvent;
        match event {
            EditorEvent::Ready => {
                self.editor_ready = true;
                self.editor_shown = None;
                self.editor_theme = None;
            }
            EditorEvent::Content {
                session,
                html,
                text,
                edited,
            } => {
                let on_screen = self.compose.as_ref().is_some_and(|c| c.serial == session);
                let Some(composer) = self.composer_mut(session) else {
                    return;
                };
                let differs = composer.html != html;
                composer.html = html;
                composer.text = text;
                if edited && differs && on_screen {
                    if let Some(composer) = &mut self.compose {
                        composer.suggest = None;
                    }
                    self.composer_changed(cx);
                }
            }
            EditorEvent::Format { session, state } => {
                if let Some(composer) = self.compose.as_mut().filter(|c| c.serial == session) {
                    composer.format = state;
                }
            }
            EditorEvent::Focused => {
                if let Some(composer) = &mut self.compose {
                    composer.suggest = None;
                }
                self.pending_focus = Some(self.editor_focus.clone());
            }
            EditorEvent::Up => self.focus_field(Field::Subject, None, cx),
            EditorEvent::Escape => self.request_close_compose(cx),
            EditorEvent::Send => self.send_mail(cx),
            EditorEvent::Save => self.save_draft(cx),
            EditorEvent::ToggleFull => self.toggle_full_compose(cx),
            EditorEvent::Dropped(paths) => self.attach_files(paths, cx),
        }
        cx.notify();
    }

    /// Starts moving or resizing the floating composer.
    fn grab_composer(&mut self, grab: Grab, event: &gpui::MouseDownEvent, cx: &mut Context<Self>) {
        if let Some(composer) = &mut self.compose
            && let Some(position) = composer.position
            && !composer.full
        {
            let mouse = (f32::from(event.position.x), f32::from(event.position.y));
            composer.grab = Some((grab, mouse, position, composer.size));
            cx.notify();
        }
    }

    /// Follows the mouse while the composer is grabbed, keeping it reachable on screen.
    fn drag_composer(
        &mut self,
        event: &gpui::MouseMoveEvent,
        viewport: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        let Some(composer) = &mut self.compose else {
            return;
        };
        let Some((grab, start, position, size)) = composer.grab else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            composer.grab = None;
            return;
        }
        let dx = f32::from(event.position.x) - start.0;
        let dy = f32::from(event.position.y) - start.1;
        match grab {
            Grab::Move => {
                composer.position = Some((
                    (position.0 + dx).clamp(80. - size.0, viewport.0 - 80.),
                    (position.1 + dy).clamp(0., viewport.1 - 60.),
                ));
            }
            Grab::Resize(hx, hy) => {
                // One axis: the far edge stays put, the grabbed one follows the mouse,
                // within the window and never below the smallest size.
                let axis =
                    |side: i8, start: f32, length: f32, delta: f32, room: f32, min: f32| match side
                    {
                        1 => (start, (length + delta).clamp(min, (room - start).max(min))),
                        -1 => {
                            let end = start + length;
                            let begin = (start + delta).clamp(0., end - min);
                            (begin, end - begin)
                        }
                        _ => (start, length),
                    };
                let (x, w) = axis(hx, position.0, size.0, dx, viewport.0, COMPOSER_MIN.0);
                let (y, h) = axis(hy, position.1, size.1, dy, viewport.1, COMPOSER_MIN.1);
                composer.position = Some((x, y));
                composer.size = (w, h);
            }
        }
        cx.notify();
    }

    /// Switches the open thread between the native reader and its original HTML.
    fn toggle_original(&mut self, cx: &mut Context<Self>) {
        let has_html = self
            .mails
            .get(self.selected)
            .is_some_and(|mail| mail.html.is_some());
        if !self.original && !has_html {
            self.notice = Some("This message has no HTML version to show.".into());
        } else {
            self.original = !self.original;
        }
        cx.notify();
    }

    /// The thread the web view should be showing right now, if any. It stays hidden
    /// whenever something is drawn over the reading pane, because a native web view
    /// always sits on top of everything GPUI paints.
    fn original_target(&self) -> Option<(String, SharedString)> {
        let ready = matches!(self.status, Status::Demo | Status::Ready);
        // (A toolbar menu does not hide it: menus open in a window of their own, above it.)
        let covered = self.palette
            || self.finder_open
            || self.picked.len() >= 2
            || self.compose.is_some()
            || self.new_label.is_some()
            || self.confirm.is_some();
        if !self.original
            || !ready
            || covered
            || self.mailbox == View::Drafts
            || !self.visible().contains(&self.selected)
        {
            return None;
        }
        let mail = self.mails.get(self.selected)?;
        // Not the row: new mail would load the email again, undoing Block images.
        let (thread, account) = self.thread_key(mail);
        let key = format!("{account}:{thread}");
        Some((key, mail.html.clone()?))
    }

    /// Whether light emails are given a dark page right now (Settings › Reading).
    fn darkens_mail(&self) -> bool {
        cfg!(target_os = "macos") && self.dark && self.settings.dark_emails
    }

    /// Creates, loads, shows or hides the web view to match the current state.
    fn sync_html_view(&mut self, window: &mut Window) {
        // A print job has a view of its own, so the email shown does not replace it, and it
        // is light whatever the theme.
        if let Some(doc) = self.print_pending.take() {
            if self.print_view.is_none() {
                self.print_view = HtmlView::new(window);
            }
            if let Some(view) = &self.print_view {
                view.set_dark(false, false);
                // Never shown, but it needs a page's size to lay the document out.
                view.set_bounds(0., 0., 800., 1000.);
                view.show(&doc, self.original_images);
            }
        }
        let Some((key, html)) = self.original_target() else {
            if let Some(view) = &self.html_view {
                view.set_visible(false);
            }
            return;
        };
        if self.html_view.is_none() {
            self.html_view = HtmlView::new(window).map(std::rc::Rc::new);
        }
        let Some(view) = &self.html_view else {
            // No web view on this platform: fall back to the native reader.
            self.original = false;
            self.notice = Some("The original view is not available on this system.".into());
            return;
        };
        // A different thread starts with remote images on again.
        if self
            .html_shown
            .as_ref()
            .is_some_and(|(shown, ..)| *shown != key)
        {
            self.original_images = self.settings.load_images;
        }
        let darkened = self.darkens_mail();
        let wanted = (key, self.original_images, self.dark, darkened);
        if self.html_shown.as_ref() != Some(&wanted) {
            // The page follows Nimbra's theme. Both take effect as the mail loads, so a
            // change of either loads it again.
            view.set_dark(self.dark, darkened);
            view.show(&html, self.original_images);
            self.html_shown = Some(wanted);
        }
        view.set_visible(true);
        // The web view is above all the app draws, so the floating sidebar gets a hole in
        // it, a frame ahead while it moves (see `pane_lead`).
        let ahead = if self.peek.moving() || self.sidebar_sliding() {
            self.frame_interval.get()
        } else {
            0.
        };
        let height = f32::from(window.viewport_size().height);
        view.set_hole(
            self.sidebar_place(ahead)
                .filter(|place| place.inset > 0.)
                .map(|place| (place.x, place.inset, SIDEBAR_W, height - 2. * place.inset)),
        );
    }

    fn set_hint(
        &mut self,
        hovered: bool,
        label: &'static str,
        key: &'static str,
        cx: &mut Context<Self>,
    ) {
        if hovered {
            self.hint = Some((label, key));
        } else if self.hint.is_some_and(|(current, _)| current == label) {
            // Only clear our own hint: moving straight to the next button sets its hint
            // first, and that one must stay.
            self.hint = None;
        }
        cx.notify();
    }

    /// Opens a toolbar menu as a small borderless window of its own, placed under the
    /// button. A separate window floats above the email's native web view, so the menu can
    /// overlap the email without moving or hiding it.
    fn toggle_menu(&mut self, menu: Menu, window: &mut Window, cx: &mut Context<Self>) {
        let just_dismissed = self.menu_dismissed.take().is_some_and(|(closed, at)| {
            closed == menu && at.elapsed() < Duration::from_millis(400)
        });
        self.close_menu_window(cx);
        if just_dismissed {
            return;
        }
        self.menu_for = match menu {
            Menu::Row(index) => Some(self.meant_for(&[index])),
            Menu::Labels | Menu::MoveTo | Menu::More => Some(self.meant_for(&self.targets())),
            _ => None,
        };
        self.menu_labels = match menu {
            Menu::Labels => self.labels.iter().take(12).cloned().collect(),
            Menu::Label(index) => self.labels.get(index).cloned().into_iter().collect(),
            _ => Vec::new(),
        };
        let t = self.theme();
        let tints = [t.violet, t.rose, t.amber, t.sky, t.mint];
        let items: Vec<(&'static str, SharedString, Hsla)> = match menu {
            Menu::Labels if self.is_demo() => vec![(
                "tag",
                "Labels need a connected Gmail account.".into(),
                t.text3,
            )],
            Menu::Labels => self
                .labels
                .iter()
                .enumerate()
                .take(12)
                .map(|(index, label)| ("tag", short(&label.name, 28), tints[index % tints.len()]))
                .chain(std::iter::once(("tag", "New label…".into(), t.text2)))
                .collect(),
            Menu::MoveTo => {
                let mut targets = Vec::new();
                if matches!(
                    self.mailbox,
                    View::Done
                        | View::Spam
                        | View::Trash
                        | View::Label(_)
                        | View::Starred
                        | View::Important
                        | View::AllMail
                        | View::Search
                ) {
                    targets.push(MoveTarget::Inbox);
                }
                targets.extend(
                    self.labels
                        .iter()
                        .take(12)
                        .map(|l| MoveTarget::Label(l.id.clone())),
                );
                if self.mailbox != View::Spam && !self.is_demo() {
                    targets.push(MoveTarget::Spam);
                }
                let items = targets
                    .iter()
                    .map(|target| match target {
                        MoveTarget::Inbox => ("inbox", "Inbox".into(), t.text2),
                        MoveTarget::Spam => ("ban", "Spam".into(), t.text2),
                        MoveTarget::Label(id) => {
                            let index = self.labels.iter().position(|l| &l.id == id).unwrap_or(0);
                            let name = self
                                .labels
                                .get(index)
                                .map(|l| l.name.clone())
                                .unwrap_or_default();
                            ("folder", short(&name, 28), tints[index % tints.len()])
                        }
                    })
                    .collect::<Vec<_>>();
                self.move_targets = targets;
                if items.is_empty() {
                    vec![("folder", "Nowhere to move this to yet.".into(), t.text3)]
                } else {
                    items
                }
            }
            Menu::Label(_) => vec![
                ("tag", "Rename…".into(), t.text2),
                ("trash", "Delete label…".into(), t.rose),
            ],
            Menu::SendLater => {
                let choices = Self::send_later_choices();
                self.send_later_times = choices.iter().map(|(_, at)| *at).collect();
                choices
                    .into_iter()
                    .map(|(label, _)| ("clock", label.into(), t.text2))
                    .collect()
            }
            Menu::More if self.is_demo() => vec![("inbox", "Mark as unread".into(), t.text2)],
            Menu::More => vec![
                ("inbox", "Mark as unread".into(), t.text2),
                ("link", "Open in Gmail".into(), t.text2),
                ("bell", "Report spam".into(), t.text2),
            ],
            Menu::Row(index) => {
                let Some(mail) = self.mails.get(index) else {
                    return;
                };
                let archivable = inbox_like(self.mailbox);
                let mut actions = vec![
                    if mail.unread {
                        RowAction::MarkRead
                    } else {
                        RowAction::MarkUnread
                    },
                    if mail.starred {
                        RowAction::Unstar
                    } else {
                        RowAction::Star
                    },
                ];
                if archivable {
                    actions.push(RowAction::Done);
                }
                if matches!(self.mailbox, View::Done | View::Trash | View::Spam) {
                    actions.push(RowAction::MoveToInbox);
                }
                if self.mailbox == View::Trash {
                    actions.push(RowAction::DeleteForever);
                } else {
                    actions.push(RowAction::Trash);
                }
                if !self.is_demo() && !matches!(self.mailbox, View::Spam | View::Trash) {
                    actions.push(RowAction::ReportSpam);
                }
                if !self.is_demo() {
                    actions.push(RowAction::OpenInGmail);
                }
                let items = actions
                    .iter()
                    .map(|action| {
                        let (name, label) = action.label();
                        (name, label.into(), t.text2)
                    })
                    .collect();
                self.row_actions = actions;
                items
            }
        };
        let enabled = !(menu == Menu::Labels && self.is_demo())
            && !(menu == Menu::MoveTo && self.move_targets.is_empty());

        // Where the menu goes, in window coordinates: under its button.
        let viewport = window.viewport_size();
        let pane_left = if self.sidebar_open { 236. } else { 0. }
            + if self.list_open { self.list_width } else { 0. };
        let (width, height) = (MENU_WIDTH, items.len() as f32 * 32. + 14.);
        let (left, top) = match menu {
            Menu::Labels => (pane_left + 150., 50.),
            Menu::MoveTo => (pane_left + 186., 50.),
            Menu::More => (f32::from(viewport.width) - 20. - width, 50.),
            // Above the composer's Send later button.
            Menu::SendLater => {
                let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
                match &self.compose {
                    Some(c) if !c.full => {
                        let (w, h) = c.size;
                        let (l, t) = c.position.unwrap_or((((vw - w) / 2.).max(0.), 96.));
                        (l + w - width - 90., t + h - height - 56.)
                    }
                    _ => (vw - width - 110., vh - height - 64.),
                }
            }
            // At the pointer, nudged back inside the window if it would hang over an edge.
            Menu::Row(_) | Menu::Label(_) => (
                self.menu_at.0.min(f32::from(viewport.width) - width - 8.),
                self.menu_at.1.min(f32::from(viewport.height) - height - 8.),
            ),
        };
        let origin = window.bounds().origin;
        let bounds = gpui::Bounds::new(
            gpui::point(
                origin.x + px(left - MENU_MARGIN),
                origin.y + px(top - MENU_MARGIN),
            ),
            gpui::size(px(width + 2. * MENU_MARGIN), px(height + 2. * MENU_MARGIN)),
        );
        let owner = cx.weak_entity();
        let dark = self.dark;
        let opened = cx.open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                titlebar: None,
                kind: gpui::WindowKind::PopUp,
                focus: true,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                window_background: gpui::WindowBackgroundAppearance::Transparent,
                ..Default::default()
            },
            |window, cx| {
                crate::blur::no_window_shadow(window);
                cx.new(|cx| {
                    // Close on any click outside the menu. Polled, because clicks on the
                    // email's web view or on another app never reach this app's handlers.
                    cx.spawn_in(window, async move |this, cx| {
                        // The click that opened the menu may still be held down (a
                        // right-click opens it on the way down). Only a press that starts
                        // after every button has been let go counts as an outside click.
                        let mut armed = false;
                        loop {
                            cx.background_executor()
                                .timer(Duration::from_millis(30))
                                .await;
                            if !crate::blur::mouse_down() {
                                armed = true;
                            }
                            let open = this.update_in(cx, |this: &mut MenuPopup, window, cx| {
                                if armed
                                    && crate::blur::mouse_pressed_outside(
                                        window,
                                        MENU_MARGIN as f64,
                                    )
                                {
                                    this.dismiss(window, cx);
                                    false
                                } else {
                                    true
                                }
                            });
                            if !matches!(open, Ok(true)) {
                                break;
                            }
                        }
                    })
                    .detach();
                    MenuPopup {
                        owner,
                        menu,
                        items,
                        enabled,
                        dark,
                        was_active: false,
                        opened: std::time::Instant::now(),
                        // Losing focus (switching apps with the keyboard, say) closes it too.
                        _activation: cx.observe_window_activation(
                            window,
                            |this: &mut MenuPopup, window, cx| {
                                if window.is_window_active() {
                                    this.was_active = true;
                                } else if this.was_active
                                    && this.opened.elapsed() > Duration::from_millis(500)
                                {
                                    // Focus moving away right as the menu opens is part of
                                    // the click that opened it, not a reason to close.
                                    this.dismiss(window, cx);
                                }
                            },
                        ),
                    }
                })
            },
        );
        match opened {
            Ok(handle) => self.menu_window = Some(handle),
            // No pop-up windows here: fall back to the menu drawn inside the pane.
            Err(_) => self.show_drawn_menu(menu, cx),
        }
    }

    fn close_menu_window(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.menu_window.take() {
            handle
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
    }

    fn menu_choice(&mut self, menu: Menu, index: usize, cx: &mut Context<Self>) {
        self.menu_window = None;
        self.menu = None;
        // Whatever a menu shows may have moved or gone while it was open (new mail, a label
        // made elsewhere): nothing here trusts a position from when it opened.
        let meant = self.menu_for.clone();
        if matches!(menu, Menu::Labels | Menu::MoveTo | Menu::More) && !self.still_meant(&meant, cx)
        {
            return;
        }
        match (menu, index) {
            (Menu::Labels, index) if index == self.menu_labels.len() => self.open_new_label(cx),
            (Menu::Labels, index) => {
                let id = self.menu_labels.get(index).map(|l| l.id.clone());
                match self.labels.iter().position(|l| Some(&l.id) == id.as_ref()) {
                    Some(now) => self.apply_label(now, cx),
                    None => self.list_changed(cx),
                }
            }
            (Menu::Label(_), choice) => {
                let Some(label) = self.menu_labels.first().cloned() else {
                    return;
                };
                let Some(now) = self.labels.iter().position(|l| l.id == label.id) else {
                    self.list_changed(cx);
                    return;
                };
                if choice == 0 {
                    self.open_rename_label(now, cx);
                } else {
                    self.confirm = Some(Ask::DeleteLabel(label.id, label.name));
                    cx.notify();
                }
            }
            (Menu::MoveTo, index) => {
                if let Some(target) = self.move_targets.get(index).cloned() {
                    self.move_selected(target, cx);
                }
            }
            (Menu::More, 0) => self.mark_unread(cx),
            (Menu::More, 1) => self.open_in_gmail(cx),
            (Menu::More, _) => self.remove_selected(gmail::Change::Spam, cx),
            (Menu::SendLater, index) => {
                if let Some(at) = self.send_later_times.get(index).copied() {
                    self.schedule_send(at, cx);
                }
            }
            (Menu::Row(_), index) => {
                let Some(action) = self.row_actions.get(index).copied() else {
                    return;
                };
                let Some(meant) = meant else { return };
                // Its actions are those of the mailbox it was opened in.
                if meant.mailbox != self.mailbox {
                    self.list_changed(cx);
                    return;
                }
                if let Some((key, account)) = meant.threads.first()
                    && let Some(row) = self.row_to_act_on(key, account, cx)
                {
                    self.row_action(row, action, cx);
                }
            }
        }
    }

    fn show_drawn_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        let just_dismissed = self.menu_dismissed.take().is_some_and(|(closed, at)| {
            closed == menu && at.elapsed() < Duration::from_millis(400)
        });
        self.menu = if self.menu == Some(menu) || just_dismissed {
            None
        } else {
            Some(menu)
        };
        cx.notify();
    }

    /// The reconnect loop is no longer wanted: its account was left, or Gmail is back.
    fn stop_retrying(&mut self) {
        self.retrying = false;
        self.retry_gen += 1;
    }

    /// The shown account's messages that wait for the connection.
    fn waiting_mail(&self) -> impl Iterator<Item = &Arc<Job>> {
        self.queue
            .iter()
            .filter(|job| job.account == self.account_id)
    }

    /// Takes the changes a catch-up got through out of its account's queue, whichever is
    /// shown by now.
    fn drop_caught_up(
        &mut self,
        account: &str,
        store: Option<&Arc<crate::store::Store>>,
        changes: usize,
    ) {
        if changes == 0 {
            return;
        }
        if account == self.account_id && (self.store.is_some() || store.is_none()) {
            self.outbox
                .changes
                .drain(..changes.min(self.outbox.changes.len()));
            self.save_outbox();
        } else if let Some(store) = store
            && let Some(mut outbox) = store.load::<Outbox>("outbox")
        {
            outbox.changes.drain(..changes.min(outbox.changes.len()));
            if store.save("outbox", &outbox).is_err() {
                eprintln!("nimbra: could not save the offline queue");
            }
        }
    }

    /// Gmail could not be reached: keep checking, then send what is waiting.
    fn went_offline(&mut self, cx: &mut Context<Self>) {
        if self.gmail.is_none() {
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
                cx.background_executor().timer(Duration::from_secs(15)).await;
                let next = this.update(cx, |this, _| {
                    if this.retry_gen != attempt {
                        return Err(()); // another loop's turn, or none's
                    }
                    let Some(client) = this.gmail.clone() else {
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
                    .spawn(async move { catch_up(&client, &changes, &queue) })
                    .await;
                let done = this.update(cx, |this, cx| {
                    // First, and whoever is shown by now.
                    this.catching_up.remove(&account);
                    let changed = result.as_ref().map_or(0, |done| done.changes);
                    this.drop_caught_up(&account, store.as_ref(), changed);
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
                        Err(error) if error.is::<gmail::SignedOut>() => {
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

    /// Saves the offline queue at once: saved in the background, an older copy could land
    /// last, or not before the app quits.
    fn save_outbox(&self) {
        if let Some(store) = &self.store
            && store.save("outbox", &self.outbox).is_err()
        {
            eprintln!("nimbra: could not save the offline queue");
        }
    }

    /// ⌘P: prints the open message with its sender, recipients and date.
    fn print_selected(&mut self, cx: &mut Context<Self>) {
        let Some(mail) = self
            .mails
            .get(self.selected)
            .filter(|_| self.visible().contains(&self.selected))
        else {
            return;
        };
        if !mail.loaded {
            self.notice = Some("The message is still loading. Try again in a moment.".into());
            cx.notify();
            return;
        }
        let h = &mail.header;
        let person = |p: &gmail::Person| {
            if p.name.is_empty() {
                p.address.clone()
            } else {
                format!("{} <{}>", p.name, p.address)
            }
        };
        let list =
            |people: &[gmail::Person]| people.iter().map(person).collect::<Vec<_>>().join(", ");
        let mut doc = format!(
            "<div style=\"font:13px/1.5 -apple-system,sans-serif;border-bottom:1px solid #ccc;padding-bottom:10px;margin-bottom:16px\"><div style=\"font-size:18px;font-weight:600;margin-bottom:6px\">{}</div>",
            esc(&mail.subject)
        );
        let from = if h.from.address.is_empty() {
            mail.sender.to_string()
        } else {
            person(&h.from)
        };
        let mut row = |name: &str, value: String| {
            if !value.is_empty() {
                doc.push_str(&format!("<div><b>{name}:</b> {}</div>", esc(&value)));
            }
        };
        row("From", from);
        row("To", list(&h.to));
        row("Cc", list(&h.cc));
        row("Date", long_date(h.date_ms));
        doc.push_str("</div>");
        match &mail.html {
            Some(html) => doc.push_str(html),
            None => {
                for block in &mail.body {
                    doc.push_str(&format!(
                        "<p>{}</p>",
                        esc(&block.text).replace('\n', "<br>")
                    ));
                }
            }
        }
        self.print_pending = Some(doc);
        cx.notify();
        // The document is loaded on the next frame; give it a moment, then print.
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(700))
                .await;
            this.update(cx, |this, _| {
                if let Some(view) = &this.print_view {
                    view.print();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The store record (and sync point) of a mailbox. Labels are kept by their Gmail id,
    /// so creating or deleting a label never mixes up what another label holds.
    fn record_for(&self, view: View) -> String {
        match view {
            View::Label(index) => match self.labels.get(index) {
                Some(label) => format!(
                    "label-{}",
                    label
                        .id
                        .chars()
                        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                        .take(50)
                        .collect::<String>()
                ),
                None => record(view),
            },
            other => record(other),
        }
    }

    /// Changes the label list, keeping the open label mailbox pointing at the same label.
    fn change_labels(
        &mut self,
        change: impl FnOnce(&mut Vec<gmail::Label>),
        cx: &mut Context<Self>,
    ) {
        let checking = self.fetching();
        let open = match self.mailbox {
            View::Label(index) => self.labels.get(index).map(|l| l.id.clone()),
            _ => None,
        };
        change(&mut self.labels);
        self.labels.sort_by_key(|l| l.name.to_lowercase());
        // Label mailboxes in memory are kept by position, which may have moved.
        self.cache.retain(|view, _| !matches!(view, View::Label(_)));
        if let Some(id) = open {
            match self.labels.iter().position(|l| l.id == id) {
                Some(index) => self.mailbox = View::Label(index),
                None => self.mailbox = View::Inbox,
            }
        }
        self.next_pages
            .retain(|view, _| !matches!(view, View::Label(_)));
        self.paged.retain(|view| !matches!(view, View::Label(_)));
        // So is a check, by the label's old position: ask again.
        self.checks.forget();
        if checking {
            self.refresh(cx);
        }
    }

    fn rename_label(&mut self, id: String, name: String, cx: &mut Context<Self>) {
        let Some(client) = self.gmail.clone() else {
            return;
        };
        let working = self.writes.begin();
        cx.spawn(async move |this, cx| {
            let (label, new_name) = (id.clone(), name.clone());
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _working = working;
                    client.rename_label(&label, &new_name)
                })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(()) => {
                    this.change_labels(
                        |labels| {
                            if let Some(label) = labels.iter_mut().find(|l| l.id == id) {
                                label.name = name;
                            }
                        },
                        cx,
                    );
                    cx.notify();
                }
                Err(error) => this.fail(error, cx),
            })
            .ok();
        })
        .detach();
    }

    fn delete_label(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(client) = self.gmail.clone() else {
            return;
        };
        let working = self.writes.begin();
        cx.spawn(async move |this, cx| {
            let label = id.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _working = working;
                    client.delete_label(&label)
                })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(()) => {
                    let was_open = matches!(this.mailbox, View::Label(i) if this.labels.get(i).is_some_and(|l| l.id == id));
                    this.change_labels(|labels| labels.retain(|l| l.id != id), cx);
                    for mail in &mut this.mails {
                        mail.labels.retain(|l| *l != id);
                    }
                    if was_open {
                        this.mailbox = View::Search; // forces the switch below to reload
                        this.select_mailbox(View::Inbox, cx);
                    }
                    this.notice = Some("Label deleted. The mail it was on is still there.".into());
                    cx.notify();
                }
                Err(error) => this.fail(error, cx),
            })
            .ok();
        })
        .detach();
    }

    /// Deletes threads for good (after a yes). If the sign-in cannot do that yet, asks to
    /// give Nimbra that permission first.
    fn delete_forever(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        let Some(client) = self.gmail.clone() else {
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

    /// Signs in again in the browser, this time also allowing permanent deletion, then
    /// deletes the threads that were waiting.
    fn grant_full_access(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            return;
        };
        let account = self.account_id.clone();
        match gmail::SignIn::start(&config, true) {
            Ok((pending, url)) => {
                cx.open_url(&url);
                self.notice =
                    Some("Allow permanent deletion in your browser, then come back.".into());
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move { pending.finish(&config, &account) })
                        .await;
                    this.update(cx, |this, cx| match result {
                        Ok(client) => {
                            this.gmail = Some(Arc::new(client));
                            this.delete_forever(ids, cx);
                        }
                        Err(error) => {
                            this.notice = Some(format!("Nothing was deleted: {error}").into());
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
            }
            Err(error) => self.notice = Some(error.to_string().into()),
        }
        cx.notify();
    }

    /// The yes-or-no sheet for permanent actions.
    fn confirm_view(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, body, yes): (String, String, &str) = match &self.confirm {
            Some(Ask::DeleteForever(ids)) => (
                if ids.len() == 1 {
                    "Delete forever?".into()
                } else {
                    format!("Delete {} forever?", ids.len())
                },
                "They are removed from Gmail completely. This can’t be undone.".into(),
                "Delete forever",
            ),
            Some(Ask::GrantFullAccess(_)) => (
                "One more permission".into(),
                "Deleting permanently needs Google’s full mail access, which Nimbra does not ask for until it is needed. Google opens in your browser to allow it; nothing is deleted until you do."
                    .into(),
                "Continue in browser",
            ),
            Some(Ask::DeleteLabel(_, name)) => (
                format!("Delete “{}”?", short(name, 30)),
                "The label is removed from Gmail. The mail it was on stays where it is.".into(),
                "Delete label",
            ),
            None => (String::new(), String::new(), ""),
        };
        div()
            .id("confirm-scrim")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(t.scrim)
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.confirm = None;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .id("confirm")
                    .w(px(440.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .p(px(22.))
                    .rounded(px(18.))
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![shadow(
                        c(0x000000, if t.dark { 0.6 } else { 0.2 }),
                        30.,
                        80.,
                        -10.,
                    )])
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(serif(&title, 28., t.text))
                    .child(div().text_color(t.text2).child(body))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.))
                            .pt(px(6.))
                            .child(
                                button(t, ButtonKind::Ghost, "Cancel", Some("esc"))
                                    .id("confirm-cancel")
                                    .press_soft()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.confirm = None;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                button(t, ButtonKind::Secondary, yes, None)
                                    .id("confirm-yes")
                                    .press_soft()
                                    .text_color(t.rose)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        match this.confirm.take() {
                                            Some(Ask::DeleteForever(ids)) => {
                                                this.delete_forever(ids, cx)
                                            }
                                            Some(Ask::GrantFullAccess(ids)) => {
                                                this.grant_full_access(ids, cx)
                                            }
                                            Some(Ask::DeleteLabel(id, _)) => {
                                                this.delete_label(id, cx)
                                            }
                                            None => {}
                                        }
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
    }

    /// The rows an action applies to: the picked ones (in list order), or the open one.
    fn targets(&self) -> Vec<usize> {
        let visible = self.visible();
        if self.picked.is_empty() {
            visible
                .into_iter()
                .filter(|i| *i == self.selected)
                .collect()
        } else {
            visible
                .into_iter()
                .filter(|i| self.picked.contains(i))
                .collect()
        }
    }

    /// Where the reading pane starts right now (following a sidebar or list slide), and
    /// where it will start once the slide ends.
    fn pane_layout(&self) -> (f32, f32) {
        self.pane_layout_at(0.)
    }

    /// `pane_layout` as it will be `ahead` seconds from now.
    fn pane_layout_at(&self, ahead: f32) -> (f32, f32) {
        const SLIDE: f32 = PANEL_SLIDE_SECONDS;
        // The same curve as the slide itself.
        let ease = panel_ease;
        let part = |open: bool, at: Option<std::time::Instant>, size: f32| {
            let to = if open { 1. } else { 0. };
            match at.map(|at| at.elapsed().as_secs_f32() + ahead) {
                Some(elapsed) if elapsed < SLIDE => {
                    let from = 1. - to;
                    size * (from + (to - from) * ease(elapsed / SLIDE))
                }
                _ => size * to,
            }
        };
        let side_now = part(self.sidebar_open, self.sidebar_toggled, SIDEBAR_W);
        let list_now = part(self.list_open, self.list_toggled, self.list_width);
        let end = if self.sidebar_open { 236. } else { 0. }
            + if self.list_open { self.list_width } else { 0. };
        (side_now + list_now, end)
    }

    /// How far the reading pane's left edge will move during the next frame of a sidebar or
    /// list slide. The email is a native view that is placed one frame after the rest of the
    /// window is drawn; moving it by this much in advance keeps it in step with the slide.
    fn pane_lead(&self) -> f32 {
        let (now, end) = self.pane_layout();
        if (now - end).abs() < 0.5 {
            return 0.;
        }
        self.pane_layout_at(self.frame_interval.get()).0 - now
    }

    /// Where the reading pane starts right now, following any slide in progress.
    fn pane_left(&self) -> f32 {
        self.pane_layout().0
    }

    fn section_open(&self, key: &str) -> bool {
        !self.settings.collapsed.iter().any(|k| k == key)
    }

    /// A sidebar row inside a section that can fold: shown, hidden, or (while the section
    /// folds or unfolds) growing or shrinking to and from nothing as it fades. The open
    /// mailbox always stays.
    fn fold_row(
        &self,
        key: &'static str,
        keep: bool,
        id: usize,
        row: AnyElement,
    ) -> Option<AnyElement> {
        let open = self.section_open(key);
        let moving = self
            .folding
            .get(key)
            .filter(|(at, _)| at.elapsed() < FOLD)
            .map(|(_, n)| *n);
        if keep || (open && moving.is_none()) {
            return Some(row);
        }
        let n = moving?;
        Some(
            div()
                .flex_none()
                .overflow_hidden()
                .child(row)
                .with_animation(
                    ("fold-row", n * 1000 + id),
                    Animation::new(FOLD).with_easing(ease_out_quint()),
                    move |d, delta| {
                        let shown = if open { delta } else { 1. - delta };
                        // The text fades well before the row gets thin, so no slivers show.
                        d.h(px(NAV_ROW * shown)).opacity(shown.powi(3))
                    },
                )
                .into_any_element(),
        )
    }

    /// A sidebar section title that folds its section away and back (remembered).
    fn section_toggle(
        &self,
        t: &Theme,
        key: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let open = self.section_open(key);
        let bright = t.text2;
        div()
            .id(("section", view_key_for_section(key)))
            .group("section")
            .flex()
            .items_center()
            .gap(px(4.))
            .pl(px(10.))
            .pt(px(18.))
            .pb(px(8.))
            .cursor_pointer()
            .child(overline(label, t.text3).group_hover("section", move |s| s.text_color(bright)))
            .child(
                icon(if open { "down" } else { "next" }, 11., t.text3)
                    .group_hover("section", move |s| s.text_color(bright)),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.section_open(key) {
                    this.settings.collapsed.push(key.to_string());
                } else {
                    this.settings.collapsed.retain(|k| k != key);
                }
                this.fold_gen += 1;
                this.folding
                    .insert(key, (std::time::Instant::now(), this.fold_gen));
                this.save_settings(cx);
                // Once folded, the rows leave the layout.
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(FOLD + Duration::from_millis(20))
                        .await;
                    this.update(cx, |this, cx| {
                        this.folding.retain(|_, (at, _)| at.elapsed() < FOLD);
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
                cx.notify();
            }))
    }

    /// Full view: the open message across the whole window, without the sidebar and list
    /// (they slide away and come back with their usual animation).
    fn toggle_full_view(&mut self, cx: &mut Context<Self>) {
        match self.focus_restore.take() {
            Some((sidebar, list)) => {
                if self.sidebar_open != sidebar {
                    self.slide_sidebar(cx);
                }
                if self.list_open != list {
                    self.toggle_list(cx);
                }
            }
            None => {
                if self.visible().is_empty() {
                    return;
                }
                self.focus_restore = Some((self.sidebar_open, self.list_open));
                if self.sidebar_open {
                    self.slide_sidebar(cx);
                }
                if self.list_open {
                    self.toggle_list(cx);
                }
            }
        }
        cx.notify();
    }

    /// A conversation in a window of its own (brought forward if it already has one).
    fn open_message_window(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(mail) = self.mails.get(i).cloned() else {
            return;
        };
        let key = message_window::key_of(&mail);
        self.message_windows
            .retain(|handle| handle.read(cx).is_ok());
        for handle in &self.message_windows {
            if handle.read(cx).is_ok_and(|w| w.key() == key) {
                handle
                    .update(cx, |_, window, _| window.activate_window())
                    .ok();
                return;
            }
        }
        let account = self.account_of(&mail).to_string();
        let owner = cx.entity();
        let original = self.settings.original_view;
        let offset = self.message_windows.len() as f32 * 24.;
        // Opened after this update: the new window reads the app as it draws.
        cx.defer(move |cx| {
            let size = gpui::size(px(900.), px(780.));
            let mut bounds = gpui::Bounds::centered(None, size, cx);
            bounds.origin.x += px(offset);
            bounds.origin.y += px(offset);
            let me = owner.clone();
            let opened = cx.open_window(
                gpui::WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some(mail.subject.clone()),
                        appears_transparent: true,
                        traffic_light_position: Some(gpui::point(px(18.), px(18.))),
                    }),
                    window_min_size: Some(gpui::size(px(520.), px(420.))),
                    ..Default::default()
                },
                |window, cx| {
                    let guard = owner.clone();
                    window.on_window_should_close(cx, move |window, cx| {
                        let handle = window.window_handle();
                        guard.update(cx, |n, cx| n.may_close_window(handle, cx))
                    });
                    cx.new(|cx| MessageWindow::new(owner, mail, account, original, cx))
                },
            );
            if let Ok(handle) = opened {
                me.update(cx, |n, _| n.message_windows.push(handle));
            }
        });
    }

    /// A click on a row: plain opens it, a double-click opens it in its own window; ⌘ adds or
    /// removes it from the picked rows; Shift picks everything from the open row to this one.
    fn click_row(
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
    fn remove_picked(&mut self, change: gmail::Change, cx: &mut Context<Self>) {
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
                gmail::Change::Trash => "Moved to trash",
                gmail::Change::Unarchive | gmail::Change::NotSpam => "Moved to inbox",
                gmail::Change::Spam => "Reported as spam",
                gmail::Change::Untrash => "Restored",
                _ => "Marked done",
            },
            cx,
        );
        cx.notify();
    }

    /// The times offered when snoozing.
    fn snooze_choices() -> Vec<SnoozeChoice> {
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
    fn snooze(&mut self, choice: SnoozeChoice, cx: &mut Context<Self>) {
        if self.gmail.is_some() && self.store.is_none() {
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
            self.remove_selected(gmail::Change::Archive, cx);
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
    fn wake_due(&mut self, cx: &mut Context<Self>) {
        if self.gmail.is_none() {
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
                        if woken.as_ref().map_or_else(gmail::is_gone, |_| true)
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
                        Err(error) => gmail::is_gone(error),
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

    /// Several threads picked: what can be done to all of them.
    fn bulk_pane(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.targets().len();
        let action = |id: &'static str, name: &'static str, label: &'static str| {
            button(t, ButtonKind::Secondary, label, None)
                .id(id)
                .press_soft()
                .child(icon(name, 14., t.text2))
        };
        let inbox_like = inbox_like(self.mailbox);
        div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(18.))
            .bg(t.surface)
            .child(serif(
                &format!("{count} conversations, selected"),
                44.,
                t.text,
            ))
            .child(meta(
                "⌘-click adds or removes one, Shift-click picks a range, Esc clears.",
                t.text3,
            ))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap(px(8.))
                    .max_w(px(620.))
                    .when(
                        inbox_like
                            || matches!(
                                self.mailbox,
                                View::Done | View::Spam | View::Trash | View::Snoozed
                            ),
                        |d| {
                            d.child(
                                action(
                                    "bulk-done",
                                    "done",
                                    if inbox_like {
                                        "Mark done"
                                    } else {
                                        "Move to inbox"
                                    },
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.mark_done(cx))),
                            )
                        },
                    )
                    .child(
                        action("bulk-read", "mail", "Read or unread")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_unread(cx))),
                    )
                    .child(
                        action("bulk-star", "star", "Star")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_star(cx))),
                    )
                    .child(action("bulk-label", "tag", "Label").on_click(cx.listener(
                        |this, _, window, cx| this.toggle_menu(Menu::Labels, window, cx),
                    )))
                    .child(
                        action("bulk-move", "folder", "Move to").on_click(cx.listener(
                            |this, _, window, cx| this.toggle_menu(Menu::MoveTo, window, cx),
                        )),
                    )
                    .when(inbox_like && !self.is_demo(), |d| {
                        d.child(
                            action("bulk-snooze", "clock", "Snooze").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.snooze_choices = Self::snooze_choices();
                                    this.open_palette();
                                    this.palette_sel = 0;
                                    cx.notify();
                                },
                            )),
                        )
                    })
                    .when(self.mailbox != View::Spam && !self.is_demo(), |d| {
                        d.child(action("bulk-spam", "ban", "Spam").on_click(cx.listener(
                            |this, _, _, cx| this.remove_selected(gmail::Change::Spam, cx),
                        )))
                    })
                    .child(
                        action("bulk-trash", "trash", "Trash")
                            .on_click(cx.listener(|this, _, _, cx| this.trash(cx))),
                    ),
            )
            .child(
                button(t, ButtonKind::Ghost, "Clear selection", Some("esc"))
                    .id("bulk-clear")
                    .press_soft()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.picked.clear();
                        cx.notify();
                    })),
            )
    }

    /// "Load more" at the end of the list: the next page of the mailbox.
    fn near_list_end(&self) -> bool {
        let left = f32::from(self.list_scroll.max_offset().height)
            + f32::from(self.list_scroll.offset().y);
        left < 700.
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        let (Some(client), Some(page)) = (self.gmail.clone(), self.more().cloned()) else {
            return;
        };
        if self.loading_more {
            return;
        }
        self.loading_more = true;
        cx.notify();
        let view = self.mailbox;
        let label_id = match view {
            View::Label(index) => self.labels.get(index).map(|label| label.id.clone()),
            View::Search => Some(self.query.clone()),
            _ => None,
        };
        let generation = self.account_gen;
        let query = self.query.clone();
        cx.spawn(async move |this, cx| {
            let result = within(cx.background_executor(), async move {
                client.threads_page(view, label_id.as_deref(), 30, Some(&page))
            })
            .await;
            this.update(cx, |this, cx| {
                // A different account was opened meanwhile: this answer is not for it.
                if this.account_gen != generation {
                    return;
                }
                this.loading_more = false;
                if this.mailbox != view || (view == View::Search && this.query != query) {
                    cx.notify();
                    return;
                }
                match result {
                    Ok((threads, next)) => {
                        match next {
                            Some(page) => this.next_pages.insert(view, page),
                            None => this.next_pages.remove(&view),
                        };
                        let known: HashSet<String> =
                            this.mails.iter().filter_map(|m| m.id.clone()).collect();
                        let fresh: Vec<Mail> = threads
                            .into_iter()
                            .filter(|s| !known.contains(&s.id))
                            .map(Mail::from_summary)
                            .collect();
                        this.done.extend(std::iter::repeat_n(false, fresh.len()));
                        this.mails.extend(fresh);
                        this.cache.insert(this.mailbox, this.listed());
                        this.persist(cx);
                    }
                    Err(error) => {
                        this.more_failed.insert(view);
                        this.fail(error, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The user labels on a thread, with their position in the label list (for a tint).
    fn thread_labels(&self, mail: &Mail) -> Vec<(usize, gmail::Label)> {
        self.labels
            .iter()
            .enumerate()
            .filter(|(_, label)| mail.labels.iter().any(|id| *id == label.id))
            .map(|(index, label)| {
                (
                    index,
                    gmail::Label {
                        id: label.id.clone(),
                        name: label.name.clone(),
                    },
                )
            })
            .collect()
    }

    /// The account a thread is from (`None` on a mail means the one shown).
    fn account_of<'a>(&'a self, mail: &'a Mail) -> &'a str {
        mail.account.as_deref().unwrap_or(&self.account_id)
    }

    /// Where a thread is in the open mailbox's list.
    fn row_of(&self, key: &str, account: &str) -> Option<usize> {
        self.mails
            .iter()
            .position(|m| message_window::key_of(m) == key && self.account_of(m) == account)
    }

    fn thread_key(&self, mail: &Mail) -> ThreadKey {
        (
            message_window::key_of(mail),
            self.account_of(mail).to_string(),
        )
    }

    /// These rows' threads, to find or compare later, when rows may have moved.
    fn meant_for(&self, rows: &[usize]) -> MeantFor {
        let mut threads: Vec<ThreadKey> = rows
            .iter()
            .filter_map(|&i| Some(self.thread_key(self.mails.get(i)?)))
            .collect();
        threads.sort();
        MeantFor {
            mailbox: self.mailbox,
            threads,
        }
    }

    /// Whether an action on the open or picked threads would still reach the ones `meant`.
    /// If not, a notice says so, and nothing is to be done.
    fn still_meant(&mut self, meant: &Option<MeantFor>, cx: &mut Context<Self>) -> bool {
        let now = self.meant_for(&self.targets());
        let same = meant
            .as_ref()
            .is_some_and(|m| m.mailbox == now.mailbox && m.threads == now.threads);
        if !same {
            self.list_changed(cx);
        }
        same
    }

    fn list_changed(&mut self, cx: &mut Context<Self>) {
        self.notice = Some("The list changed meanwhile, so nothing was done.".into());
        cx.notify();
    }

    /// The snooze panel, for the threads open or picked now.
    fn open_palette(&mut self) {
        self.palette = true;
        self.palette_for = Some(self.meant_for(&self.targets()));
    }

    /// The same, for acting on it: it has to still be listed, or a notice says why not.
    /// Whatever a message window does starts here, not from `selected`, which is only what
    /// the main window shows.
    fn row_to_act_on(&mut self, key: &str, account: &str, cx: &mut Context<Self>) -> Option<usize> {
        let row = self.row_of(key, account).filter(|i| !self.done[*i]);
        if row.is_none() {
            self.notice = Some("This conversation is no longer in the open mailbox.".into());
            cx.notify();
        }
        row
    }

    /// Takes a label off the thread in that row.
    fn remove_label(&mut self, index: usize, label_id: String, cx: &mut Context<Self>) {
        if self.is_foreign(index) {
            return;
        }
        let Some(mail) = self.mails.get_mut(index) else {
            return;
        };
        mail.labels.retain(|id| *id != label_id);
        let (Some(client), Some(id)) = (self.gmail.clone(), mail.id.clone()) else {
            cx.notify();
            return;
        };
        let working = self.writes.begin();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _working = working;
                    client.remove_label(&id, &label_id)
                })
                .await;
            if let Err(error) = result {
                this.update(cx, |this, cx| this.fail(error, cx)).ok();
            }
        })
        .detach();
        cx.notify();
    }

    /// Opens the label sheet to rename a label.
    fn open_rename_label(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(label) = self.labels.get(index).cloned() else {
            return;
        };
        self.open_label_sheet(LabelSheet::Rename(label.id), cx);
        if let Some(input) = &self.new_label {
            input.update(cx, |input, cx| input.set(label.name, usize::MAX, cx));
        }
    }

    /// Opens the New label sheet (from the Labels menu: also labels the open thread).
    fn open_new_label(&mut self, cx: &mut Context<Self>) {
        self.label_for = Some(self.meant_for(&self.targets()));
        self.open_label_sheet(LabelSheet::New { apply: true }, cx);
    }

    fn open_label_sheet(&mut self, sheet: LabelSheet, cx: &mut Context<Self>) {
        self.label_sheet = sheet;
        if self.gmail.is_none() {
            self.notice = Some("Labels need a connected Gmail account.".into());
            cx.notify();
            return;
        }
        let t = self.theme();
        let input = cx.new(|cx| TextInput::new(cx, "Label name", t.faint, t.violet));
        let subscription = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Enter) {
                this.create_label(cx);
            }
        });
        self.pending_focus = Some(input.read(cx).focus_handle(cx));
        self.new_label = Some(input);
        self._new_label_subscription = Some(subscription);
        if let Some(view) = &self.html_view {
            view.set_visible(false);
        }
        cx.notify();
    }

    fn close_new_label(&mut self, cx: &mut Context<Self>) {
        self.new_label = None;
        self._new_label_subscription = None;
        self.pending_focus = Some(self.focus.clone());
        cx.notify();
    }

    /// Creates the label typed in the sheet and puts it on the open thread.
    fn create_label(&mut self, cx: &mut Context<Self>) {
        let (Some(input), Some(client)) = (self.new_label.clone(), self.gmail.clone()) else {
            return;
        };
        let name = input.read(cx).text().trim().to_string();
        if name.is_empty() {
            return;
        }
        let apply = match self.label_sheet.clone() {
            LabelSheet::Rename(id) => {
                self.close_new_label(cx);
                self.rename_label(id, name, cx);
                return;
            }
            LabelSheet::New { apply } => apply,
        };
        if let Some(existing) = self
            .labels
            .iter()
            .position(|l| l.name.eq_ignore_ascii_case(&name))
        {
            self.close_new_label(cx);
            let meant = self.label_for.clone();
            if apply && self.still_meant(&meant, cx) {
                self.apply_label(existing, cx);
            }
            return;
        }
        self.close_new_label(cx);
        let (meant, generation) = (self.label_for.clone(), self.account_gen);
        let working = self.writes.begin();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _working = working;
                    client.create_label(&name)
                })
                .await;
            this.update(cx, |this, cx| match result {
                // The label is in the account it was made in.
                Ok(_) if this.account_gen != generation => {}
                Ok(label) => {
                    let id = label.id.clone();
                    let name = label.name.clone();
                    this.change_labels(|labels| labels.push(label), cx);
                    if apply && let Some(index) = this.labels.iter().position(|l| l.id == id) {
                        if this.still_meant(&meant, cx) {
                            this.apply_label(index, cx);
                        }
                    } else {
                        this.notice = Some(format!("Created “{}”.", short(&name, 30)).into());
                    }
                    cx.notify();
                }
                Err(error) => this.fail(error, cx),
            })
            .ok();
        })
        .detach();
    }

    /// The New label sheet.
    fn new_label_view(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let input = self.new_label.clone();
        div()
            .id("new-label-scrim")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(t.scrim)
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close_new_label(cx)),
            )
            .child(
                div()
                    .id("new-label")
                    .w(px(400.))
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .p(px(22.))
                    .rounded(px(18.))
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![shadow(
                        c(0x000000, if t.dark { 0.6 } else { 0.2 }),
                        30.,
                        80.,
                        -10.,
                    )])
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(serif(
                        if matches!(self.label_sheet, LabelSheet::Rename(_)) {
                            "Rename label"
                        } else {
                            "New label"
                        },
                        28.,
                        t.text,
                    ))
                    .child(
                        div()
                            .px(px(12.))
                            .py(px(8.))
                            .rounded(px(10.))
                            .bg(t.hover)
                            .border_1()
                            .border_color(t.border)
                            .text_size(px(14.))
                            .children(input),
                    )
                    .child(meta(
                        match self.label_sheet {
                            LabelSheet::Rename(_) => "Renamed in Gmail too. Use “Parent/Child” to put it inside another label.",
                            LabelSheet::New { apply: true } => "It is added to this conversation and to Gmail’s label list. Use “Parent/Child” to nest it.",
                            LabelSheet::New { apply: false } => "It is added to Gmail’s label list. Use “Parent/Child” to put it inside another label.",
                        },
                        t.text3,
                    ))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.))
                            .child(
                                button(t, ButtonKind::Ghost, "Cancel", None)
                                    .id("label-cancel").press_soft()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.close_new_label(cx)),
                                    ),
                            )
                            .child(
                                button(
                                    t,
                                    ButtonKind::Primary,
                                    if matches!(self.label_sheet, LabelSheet::Rename(_)) { "Rename" } else { "Create" },
                                    Some("↵"),
                                )
                                    .id("label-create").press_soft()
                                    .on_click(cx.listener(|this, _, _, cx| this.create_label(cx))),
                            ),
                    ),
            )
    }

    /// Move to: back to the inbox, into a label (out of the inbox), or to Spam.
    fn move_selected(&mut self, target: MoveTarget, cx: &mut Context<Self>) {
        let leaves = inbox_like(self.mailbox);
        match target {
            MoveTarget::Spam => self.remove_selected(gmail::Change::Spam, cx),
            MoveTarget::Inbox => match self.mailbox {
                View::Spam => self.remove_selected(gmail::Change::NotSpam, cx),
                View::Trash => self.remove_selected(gmail::Change::Untrash, cx),
                _ => self.remove_selected(gmail::Change::Unarchive, cx),
            },
            MoveTarget::Label(label_id) => {
                let Some(index) = self.labels.iter().position(|l| l.id == label_id) else {
                    return;
                };
                let rows = self.targets();
                // Labels are each account's own: without this it would be archived unlabelled.
                if rows.iter().any(|&row| self.is_foreign(row)) {
                    self.notice = Some(
                        "Labels belong to each account: open this conversation's account to move it to one."
                            .into(),
                    );
                    cx.notify();
                    return;
                }
                let threads: Vec<String> = rows
                    .iter()
                    .filter_map(|&row| self.mails[row].id.clone())
                    .collect();
                self.apply_label(index, cx);
                if leaves {
                    for thread in threads {
                        self.moved.insert(thread, label_id.clone());
                    }
                    self.remove_selected(gmail::Change::Archive, cx);
                }
            }
        }
    }

    /// Adds one of the user's labels to the open thread.
    fn apply_label(&mut self, label: usize, cx: &mut Context<Self>) {
        self.menu = None;
        if self.picked.len() >= 2 {
            let selected = self.selected;
            let targets = self.targets();
            let picked = std::mem::take(&mut self.picked);
            for index in targets {
                self.selected = index;
                self.apply_label(label, cx);
            }
            self.selected = selected;
            self.picked = picked;
            return;
        }
        if let (Some(label), Some(mail)) =
            (self.labels.get(label), self.mails.get_mut(self.selected))
            && !mail.labels.contains(&label.id)
        {
            mail.labels.push(label.id.clone());
        }
        let (Some(label), Some(mail)) = (
            self.labels.get(label).cloned(),
            self.mails.get(self.selected),
        ) else {
            return;
        };
        if self.is_foreign(self.selected) {
            if let Some(mail) = self.mails.get_mut(self.selected) {
                mail.labels.retain(|id| *id != label.id);
            }
            self.notice = Some(
                "Labels belong to each account: open this conversation's account to label it."
                    .into(),
            );
            cx.notify();
            return;
        }
        let (Some(client), Some(id)) = (self.gmail.clone(), mail.id.clone()) else {
            return;
        };
        let name = label.name.clone();
        let working = self.writes.begin();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _working = working;
                    client.add_label(&id, &label.id)
                })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(()) => {
                    this.notice = Some(format!("Labelled “{}”.", short(&name, 30)).into());
                    cx.notify();
                }
                Err(error) => this.fail(error, cx),
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// `U`, or "Mark as unread" in a menu: the open thread goes back to unread.
    fn mark_unread(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        self.set_unread(self.selected, true, cx);
    }

    /// The toolbar button and `U`: flips the open thread between unread and read.
    fn toggle_unread(&mut self, cx: &mut Context<Self>) {
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
    fn count_change(change: gmail::Change) -> i32 {
        match change {
            gmail::Change::Archive | gmail::Change::Trash | gmail::Change::Spam => -1,
            _ => 0,
        }
    }

    /// Marks a thread read or unread, here and in Gmail.
    fn set_unread(&mut self, index: usize, unread: bool, cx: &mut Context<Self>) {
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
                gmail::Change::MarkUnread
            } else {
                gmail::Change::MarkRead
            };
            self.sync(index, change, false, cx);
        }
        cx.notify();
    }

    /// Downloads an attachment, then either previews it with Quick Look (from a private
    /// temporary folder) or saves it to Downloads and shows it in Finder. Nothing is opened
    /// with another app automatically.
    fn fetch_attachment(
        &mut self,
        att: gmail::Attachment,
        account: Option<&str>,
        save: bool,
        cx: &mut Context<Self>,
    ) {
        // Through the account of the conversation it is in.
        let Some(client) = self.client_for(account) else {
            self.notice = Some("Attachments need a connected Gmail account.".into());
            cx.notify();
            return;
        };
        if self.attachment_busy.contains_key(&att.attachment_id) {
            return;
        }
        let key = att.attachment_id.clone();
        self.attachment_busy.insert(key.clone(), save);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = within(cx.background_executor(), async move {
                let bytes = client.download(&att.message_id, &att.attachment_id)?;
                let name = gmail::safe_file_name(&att.name);
                let path = if save {
                    unique_download_path(&name)?
                } else {
                    preview_path(&name)?
                };
                std::fs::write(&path, &bytes)?;
                restrict_file(&path);
                anyhow::Ok(path)
            })
            .await;
            this.update(cx, |this, cx| {
                this.attachment_busy.remove(&key);
                match result {
                    Ok(path) => {
                        let tool = if save {
                            ("/usr/bin/open", "-R")
                        } else {
                            ("/usr/bin/qlmanage", "-p")
                        };
                        let _ = std::process::Command::new(tool.0)
                            .arg(tool.1)
                            .arg(&path)
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .spawn();
                        if save {
                            // A check mark on the attachment for a moment (Finder also shows it).
                            this.attachment_saved
                                .insert(key.clone(), std::time::Instant::now());
                            cx.spawn(async move |this, cx| {
                                cx.background_executor().timer(Duration::from_secs(3)).await;
                                this.update(cx, |this, cx| {
                                    this.attachment_saved.remove(&key);
                                    cx.notify();
                                })
                                .ok();
                            })
                            .detach();
                        }
                    }
                    Err(error) => {
                        this.notice = Some(format!("Could not get the attachment: {error}").into())
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Leaves a mailing list: an https link opens in the browser (where the list asks for
    /// confirmation); a mailto address opens a prepared message, from the account the mail
    /// came to, in the window the button was in (`host`; `None` is the main window).
    fn unsubscribe(
        &mut self,
        link: &str,
        account: Option<String>,
        host: Option<gpui::AnyWindowHandle>,
        cx: &mut Context<Self>,
    ) {
        if link.to_ascii_lowercase().starts_with("https://") {
            cx.open_url(link);
            return;
        }
        let Some(mail) = gmail::unsubscribe_mail(link) else {
            self.notice = Some("This list’s unsubscribe address is not valid.".into());
            cx.notify();
            return;
        };
        // Never into a message that is being written: it would gain the list as a recipient.
        if self.compose.is_some() || self.palette {
            self.notice =
                Some("Finish or close what you are writing, then click Unsubscribe again.".into());
            cx.notify();
            return;
        }
        self.open_compose_from(ComposeKind::New, None, cx);
        let Some(composer) = &mut self.compose else {
            return;
        };
        composer.account = account;
        composer
            .to
            .update(cx, |input, cx| input.set(mail.to, usize::MAX, cx));
        composer.chipify(Field::To, true, cx);
        composer
            .subject
            .update(cx, |input, cx| input.set(mail.subject, usize::MAX, cx));
        // A program reads it: exactly what the list asked for, and no signature.
        if composer.rich {
            composer.load = Some(if mail.body.is_empty() {
                "<div><br></div>".to_string()
            } else {
                crate::editor::text_to_html(&mail.body)
            });
        } else if let Some(line) = composer.lines.first() {
            line.update(cx, |input, cx| {
                input.set(mail.body.replace('\n', " "), usize::MAX, cx)
            });
        }
        if host.is_some() {
            self.compose_host = host;
            self.compose_detached = false;
        }
    }

    /// Opens the right-click menu for a row of the list.
    fn open_row_menu(
        &mut self,
        index: usize,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu_at = (f32::from(at.x), f32::from(at.y));
        // A second right-click should always open the menu again, never just close it.
        self.menu_dismissed = None;
        self.toggle_menu(Menu::Row(index), window, cx);
    }

    /// Runs one action from a row's right-click menu on that row.
    fn row_action(&mut self, row: usize, action: RowAction, cx: &mut Context<Self>) {
        match action {
            RowAction::MarkRead => self.set_unread(row, false, cx),
            RowAction::MarkUnread => self.set_unread(row, true, cx),
            RowAction::Star | RowAction::Unstar => {
                let starred = action == RowAction::Star;
                self.mails[row].starred = starred;
                let change = if starred {
                    gmail::Change::Star
                } else {
                    gmail::Change::Unstar
                };
                self.sync(row, change, false, cx);
            }
            RowAction::Done
            | RowAction::MoveToInbox
            | RowAction::Trash
            | RowAction::DeleteForever
            | RowAction::ReportSpam => {
                // That row alone. Only if it is the open one does the next open, as with E;
                // any other leaves what is open as it is, and nothing else is marked read.
                let open = row == self.selected && self.picked.len() < 2;
                match action {
                    RowAction::Trash | RowAction::DeleteForever => {
                        self.trash_thread(row, cx);
                    }
                    RowAction::ReportSpam => self.remove_thread(row, gmail::Change::Spam, cx),
                    _ => {
                        self.done_thread(row, cx);
                    }
                }
                if open && self.done[row] {
                    self.open_selected(cx);
                }
            }
            RowAction::OpenInGmail => {
                if let Some(id) = self.mails[row].id.as_ref()
                    && id.chars().all(|c| c.is_ascii_alphanumeric())
                {
                    cx.open_url(&format!("https://mail.google.com/mail/u/0/#all/{id}"));
                }
            }
        }
        cx.notify();
    }

    fn open_in_gmail(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        // Thread ids are validated as plain alphanumerics before they get here.
        if let Some(id) = self.mails.get(self.selected).and_then(|m| m.id.as_ref())
            && id.chars().all(|c| c.is_ascii_alphanumeric())
        {
            cx.open_url(&format!("https://mail.google.com/mail/u/0/#all/{id}"));
        }
        cx.notify();
    }

    /// The composer, shown in `window` when that is where it lives: full page, with the
    /// composer's keys, the editor and the keyboard focus. None anywhere else.
    fn hosted_compose(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.compose.is_none() || self.compose_host != Some(window.window_handle()) {
            return None;
        }
        let size = window.viewport_size();
        let viewport = (f32::from(size.width), f32::from(size.height));
        let detached = self.compose_detached;
        if let Some(composer) = &mut self.compose {
            // A window of its own is all composer; elsewhere it floats as in the main window.
            if detached {
                composer.full = true;
            } else if composer.position.is_none() {
                // In the bottom-right corner, sized to fit the window with a margin.
                const MARGIN: f32 = 20.;
                composer.size = (
                    composer
                        .size
                        .0
                        .min(620.)
                        .min(viewport.0 - 2. * MARGIN)
                        .max(COMPOSER_MIN.0),
                    composer.size.1.min(viewport.1 - 92.).max(COMPOSER_MIN.1),
                );
                composer.position = Some((
                    (viewport.0 - composer.size.0 - MARGIN).max(0.),
                    (viewport.1 - composer.size.1 - MARGIN).max(0.),
                ));
            }
        }
        if let Some(focus) = self.pending_focus.take() {
            window.focus(&focus);
        }
        self.sync_editor(window, cx);
        let t = self.theme();
        Some(
            div()
                .absolute()
                .inset_0()
                .key_context("Composer")
                .on_action(cx.listener(|this, _: &SendMail, _, cx| this.send_mail(cx)))
                .on_action(cx.listener(|this, _: &SaveDraft, _, cx| this.save_draft(cx)))
                .on_action(
                    cx.listener(|this, _: &ToggleFullCompose, _, cx| this.toggle_full_compose(cx)),
                )
                .on_action(cx.listener(|this, _: &Dismiss, _, cx| {
                    if let Some(composer) = this.compose.as_mut()
                        && composer.suggest.take().is_some()
                    {
                        cx.notify();
                    } else {
                        this.request_close_compose(cx);
                    }
                }))
                .child(self.compose_view(&t, viewport, cx))
                .into_any_element(),
        )
    }

    /// Moves the composer into a window of its own.
    fn detach_compose(&mut self, cx: &mut Context<Self>) {
        if self.compose.is_none() {
            return;
        }
        let owner = cx.entity();
        self.compose_home = self.compose_host;
        cx.defer(move |cx| {
            let bounds = gpui::Bounds::centered(None, gpui::size(px(820.), px(700.)), cx);
            let me = owner.clone();
            let opened = cx.open_window(
                gpui::WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("New message".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(gpui::point(px(18.), px(18.))),
                    }),
                    window_min_size: Some(gpui::size(px(520.), px(420.))),
                    ..Default::default()
                },
                |window, cx| {
                    let guard = owner.clone();
                    window.on_window_should_close(cx, move |window, cx| {
                        let handle = window.window_handle();
                        guard.update(cx, |n, cx| n.may_close_window(handle, cx))
                    });
                    cx.new(|cx| ComposeWindow {
                        _observe: cx.observe(&owner, |_, _, cx| cx.notify()),
                        owner,
                        shown: false,
                    })
                },
            );
            if let Ok(handle) = opened {
                me.update(cx, |n, cx| {
                    n.compose_host = Some(handle.into());
                    n.compose_detached = true;
                    if let Some(composer) = &n.compose {
                        n.pending_focus = Some(composer.to.read(cx).focus_handle(cx));
                    }
                    cx.notify();
                });
            }
        });
    }

    /// Puts a composer in a window of its own back where it came from (the main window if
    /// that message window has closed), floating in its corner. Its window then closes.
    fn attach_compose(&mut self, cx: &mut Context<Self>) {
        if !self.compose_detached || self.compose.is_none() {
            return;
        }
        let home = self
            .compose_home
            .take()
            .filter(|home| cx.windows().contains(home));
        self.compose_host = home;
        self.compose_detached = false;
        if let Some(composer) = &mut self.compose {
            composer.full = false;
            composer.position = None;
            composer.grab = None;
            self.pending_focus = Some(composer.to.read(cx).focus_handle(cx));
        }
        if let Some(home) = home {
            cx.defer(move |cx| {
                home.update(cx, |_, window, _| window.activate_window())
                    .ok();
            });
        } else {
            crate::show_main_window(cx);
        }
        cx.notify();
    }

    fn toggle_full_compose(&mut self, cx: &mut Context<Self>) {
        if self.compose_detached {
            return;
        }
        if let Some(composer) = &mut self.compose {
            composer.full = !composer.full;
            cx.notify();
        }
    }

    /// Opens the draft behind a row of the Drafts mailbox in the composer.
    fn open_draft(&mut self, index: usize, cx: &mut Context<Self>) {
        let (Some(client), Some(thread_id)) = (
            self.gmail.clone(),
            self.mails.get(index).and_then(|m| m.id.clone()),
        ) else {
            return;
        };
        if self.compose.is_some() {
            return;
        }
        self.notice = None;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { client.draft_for_thread(&thread_id) })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(draft) => this.edit_draft(draft, cx),
                Err(error) => this.fail(error, cx),
            })
            .ok();
        })
        .detach();
    }

    fn edit_draft(&mut self, draft: gmail::Draft, cx: &mut Context<Self>) {
        self.open_compose(ComposeKind::New, cx);
        let Some(mut composer) = self.compose.take() else {
            return;
        };
        composer
            .to
            .update(cx, |input, cx| input.set(draft.to, usize::MAX, cx));
        composer
            .subject
            .update(cx, |input, cx| input.set(draft.subject, usize::MAX, cx));
        // One field per line of the part the user wrote.
        for (n, text) in draft.body.lines().take(400).enumerate() {
            let text = text.to_string();
            if n == 0 {
                composer.lines[0].update(cx, |input, cx| input.set(text, 0, cx));
            } else {
                let (line, subscription) = self.new_field(Field::Line(n), "", cx);
                line.update(cx, |input, cx| input.set(text, 0, cx));
                composer.lines.push(line);
                composer._subscriptions.push(subscription);
            }
        }
        composer.show_cc = !draft.cc.trim().is_empty() || !draft.bcc.trim().is_empty();
        let (cc, bcc) = (draft.cc, draft.bcc);
        composer
            .cc
            .update(cx, |input, cx| input.set(cc, usize::MAX, cx));
        composer
            .bcc
            .update(cx, |input, cx| input.set(bcc, usize::MAX, cx));
        for field in [Field::To, Field::Cc, Field::Bcc] {
            composer.chipify(field, true, cx);
        }
        composer.kept = draft.attachments;
        let html = draft
            .html
            .as_deref()
            .and_then(|html| {
                if draft.quoted.is_some() {
                    crate::editor::own_html(html)
                } else {
                    Some(html.to_string())
                }
            })
            .unwrap_or_else(|| crate::editor::text_to_html(&draft.body));
        composer.load = Some(html);
        composer.focus_body = composer.rich;
        composer.draft_id = Some(draft.id);
        composer.quoted = draft.quoted;
        composer.reply = draft.reply;
        composer.full = true;
        self.pending_focus = Some(if composer.rich {
            self.editor_focus.clone()
        } else {
            composer.lines[0].read(cx).focus_handle(cx)
        });
        self.compose = Some(composer);
        cx.notify();
    }

    /// Saves the composer's contents to Gmail's Drafts without sending anything.
    /// ⌘S: saves the draft now.
    fn save_draft(&mut self, cx: &mut Context<Self>) {
        if let Some(serial) = self.compose.as_ref().map(|c| c.serial) {
            self.save_composer(serial, true, cx);
        }
    }

    /// The composer with this serial, on screen or parked.
    fn composer_mut(&mut self, serial: u64) -> Option<&mut Composer> {
        if self.compose.as_ref().is_some_and(|c| c.serial == serial) {
            return self.compose.as_mut();
        }
        self.parked
            .iter_mut()
            .find(|(_, c)| c.serial == serial)
            .map(|(_, c)| c)
    }

    /// Something in the composer changed: save the draft once typing pauses.
    fn composer_changed(&mut self, cx: &mut Context<Self>) {
        let Some(composer) = &mut self.compose else {
            return;
        };
        composer.edits += 1;
        composer.saved = false;
        let (serial, edits) = (composer.serial, composer.edits);
        // Attachments make every save an upload, so those wait longer.
        let pause = if composer.files.is_empty() && composer.kept.is_empty() {
            2500
        } else {
            10_000
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(pause))
                .await;
            this.update(cx, |this, cx| {
                if this
                    .compose
                    .as_ref()
                    .is_some_and(|c| c.serial == serial && c.edits == edits)
                {
                    this.save_composer(serial, false, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Saves a composer's message as a Gmail draft (new, or over the one it has).
    /// `explicit` is ⌘S or a step that needs the draft (Send later); otherwise only unsaved
    /// edits with something in them are saved.
    fn save_composer(&mut self, serial: u64, explicit: bool, cx: &mut Context<Self>) {
        let from = self.composer_mut(serial).and_then(|c| c.account.clone());
        let client = self.client_for(from.as_deref());
        let Some(client) = client else {
            if explicit && let Some(composer) = self.composer_mut(serial) {
                composer.error = Some("Sample mode: drafts are not saved.".into());
                cx.notify();
            }
            return;
        };
        let Some(composer) = self.composer_mut(serial) else {
            return;
        };
        if composer.sending {
            return;
        }
        if composer.saving {
            composer.resave = true;
            return;
        }
        if !explicit
            && composer.after_save.is_none()
            && (!composer.dirty() || !composer.has_content(cx))
        {
            return;
        }
        let outgoing = composer.outgoing(cx);
        let draft_id = composer.draft_id.clone();
        let edits = composer.edits;
        composer.saving = true;
        composer.resave = false;
        if explicit {
            composer.error = None;
        }
        let saved_as = composer.saved_as.clone();
        cx.notify();
        let working = self.saves.begin();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _working = working;
                    let saved = client.save_draft(draft_id.as_deref(), &outgoing);
                    if let Ok(id) = &saved {
                        *saved_as.lock().unwrap() = Some((id.clone(), edits));
                    }
                    saved
                })
                .await;
            this.update(cx, |this, cx| {
                this.composer_saved(serial, edits, explicit, result, cx)
            })
            .ok();
        })
        .detach();
    }

    fn composer_saved(
        &mut self,
        serial: u64,
        edits: u64,
        explicit: bool,
        result: anyhow::Result<String>,
        cx: &mut Context<Self>,
    ) {
        let result = match result {
            Err(error) if self.shown_signed_out(&error) => {
                self.compose = None;
                self.fail(error, cx);
                return;
            }
            other => other,
        };
        let kind = self
            .parked
            .iter()
            .find(|(_, c)| c.serial == serial)
            .map(|(k, _)| *k);
        let Some(composer) = self.composer_mut(serial) else {
            return;
        };
        composer.saving = false;
        let ok = match result {
            Ok(id) => {
                composer.draft_id = Some(id);
                composer.saved_edits = composer.saved_edits.max(edits);
                composer.saved = !composer.dirty();
                true
            }
            Err(error) => {
                composer.error = Some(format!("The draft was not saved: {error}").into());
                composer.after_save = None;
                false
            }
        };
        let again = ok && (composer.dirty() || std::mem::take(&mut composer.resave));
        let after = if ok && !again {
            composer.after_save.take()
        } else {
            None
        };
        let mut refresh = explicit && ok;
        match (kind, after) {
            (Some(_), Some(After::Schedule(at))) => {
                if let Some(pos) = self.parked.iter().position(|(_, c)| c.serial == serial) {
                    let (_, composer) = self.parked.remove(pos);
                    if let Some(draft_id) = composer.draft_id.clone() {
                        let subject = composer.subject.read(cx).text();
                        let item = Scheduled {
                            draft_id,
                            at_ms: at,
                            subject,
                            account: composer.account.clone(),
                        };
                        self.add_scheduled(item, cx);
                    }
                    refresh = true;
                }
            }
            (Some(Parked::Closing), _) if !ok => self.unpark(serial, cx),
            (Some(Parked::Closing), _) if again => self.save_composer(serial, false, cx),
            (Some(Parked::Closing), _) => {
                self.parked.retain(|(_, c)| c.serial != serial);
                refresh = true;
            }
            (None, _) if again => self.save_composer(serial, false, cx),
            _ => {}
        }
        if refresh && self.mailbox == View::Drafts {
            self.refresh(cx);
        }
        cx.notify();
    }

    /// Brings a parked composer back on screen (when nothing else is open there).
    fn unpark(&mut self, serial: u64, cx: &mut Context<Self>) {
        let Some(pos) = self.parked.iter().position(|(_, c)| c.serial == serial) else {
            return;
        };
        if self.compose.is_some() {
            let (_, composer) = self.parked.remove(pos);
            let subject = composer.subject.read(cx).text();
            self.notice = Some(
                format!(
                    "“{}” needs attention: {}",
                    short(&subject, 40),
                    composer.error.clone().unwrap_or_default()
                )
                .into(),
            );
            return;
        }
        let (_, mut composer) = self.parked.remove(pos);
        composer.sending = false;
        if composer.rich {
            composer.focus_body = true;
            self.pending_focus = Some(self.editor_focus.clone());
        } else if let Some(first) = composer.lines.first() {
            self.pending_focus = Some(first.read(cx).focus_handle(cx));
        }
        self.compose = Some(composer);
        cx.notify();
    }

    /// Esc, ⌘W or a window's close button on the composer: asks whether to keep what was
    /// written as a draft or throw it away. An empty composer just closes; asked again
    /// (Esc while asking), it goes back to writing.
    fn request_close_compose(&mut self, cx: &mut Context<Self>) {
        let Some(composer) = &self.compose else {
            return;
        };
        if self.compose_ask {
            // Hiding the editor (a native view with the keyboard) for the question makes
            // macOS send a cancel (Esc) straight after the click that asked: not an answer.
            if self
                .compose_ask_at
                .is_some_and(|at| at.elapsed() < Duration::from_millis(400))
            {
                return;
            }
            self.compose_ask = false;
            self.pending_focus = Some(self.editor_focus.clone());
        } else if composer.has_content(cx) {
            self.compose_ask = true;
            self.compose_ask_at = Some(std::time::Instant::now());
        } else {
            self.close_compose(cx);
            return;
        }
        cx.notify();
    }

    /// A window's close button or ⌘W: a composer with something in it, shown in that
    /// window, asks first (false keeps the window open).
    fn may_close_window(&mut self, window: gpui::AnyWindowHandle, cx: &mut Context<Self>) -> bool {
        let here = self.compose.is_some() && self.compose_host == Some(window);
        if here && self.compose.as_ref().is_some_and(|c| c.has_content(cx)) {
            if !self.compose_ask {
                self.compose_ask = true;
                self.compose_ask_at = Some(std::time::Instant::now());
                cx.notify();
            }
            return false;
        }
        true
    }

    /// The answer to "Save this draft?".
    fn answer_close_compose(&mut self, save: bool, cx: &mut Context<Self>) {
        self.compose_ask = false;
        if save {
            self.close_compose(cx);
        } else {
            self.discard_compose(cx);
        }
    }

    /// Closes the composer. Its last edits are still saved to Drafts.
    fn close_compose(&mut self, cx: &mut Context<Self>) {
        self.compose_ask = false;
        self.pending_focus = Some(self.focus.clone());
        let Some(mut composer) = self.compose.take() else {
            return;
        };
        composer.suggest = None;
        let unsaved = composer.saving || (composer.dirty() && composer.has_content(cx));
        if self.gmail.is_some() && unsaved {
            let serial = composer.serial;
            self.parked.push((Parked::Closing, composer));
            self.save_composer(serial, false, cx);
        }
        cx.notify();
    }

    /// The Discard button: closes the composer and deletes its draft, unless undone.
    fn discard_compose(&mut self, cx: &mut Context<Self>) {
        self.compose_ask = false;
        self.pending_focus = Some(self.focus.clone());
        let Some(mut composer) = self.compose.take() else {
            return;
        };
        composer.suggest = None;
        if !composer.has_content(cx) && composer.draft_id.is_none() && !composer.saving {
            cx.notify();
            return;
        }
        let serial = composer.serial;
        self.parked.push((Parked::Discarded, composer));
        self.offer_undo("Draft discarded", cx);
        self.finish_parked_later(serial, UNDO_DELAY, cx);
        cx.notify();
    }

    fn finish_parked_later(&mut self, serial: u64, delay: Duration, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |this, cx| this.finish_parked(serial, cx))
                .ok();
        })
        .detach();
    }

    /// The undo delay of a sent or discarded message is over: send it, or delete its draft.
    fn finish_parked(&mut self, serial: u64, cx: &mut Context<Self>) {
        let Some(pos) = self.parked.iter().position(|(_, c)| c.serial == serial) else {
            return; // undone
        };
        if self.parked[pos].1.saving {
            // Wait for the draft save that is running, so nothing is left behind.
            self.finish_parked_later(serial, Duration::from_millis(400), cx);
            return;
        }
        match self.parked[pos].0 {
            Parked::Sending => {
                let (_, composer) = self.parked.remove(pos);
                self.dispatch(composer, cx);
            }
            Parked::Discarded => {
                let (_, composer) = self.parked.remove(pos);
                if let (Some(id), Some(client)) = (
                    composer.draft_id.clone(),
                    self.client_for(composer.account.as_deref()),
                ) {
                    let working = self.writes.begin();
                    cx.spawn(async move |this, cx| {
                        let result = cx
                            .background_executor()
                            .spawn(async move {
                                let _working = working;
                                client.delete_draft(&id)
                            })
                            .await;
                        this.update(cx, |this, cx| {
                            if let Err(error) = result {
                                this.notice =
                                    Some(format!("The draft was not deleted: {error}").into());
                            }
                            if this.mailbox == View::Drafts {
                                this.refresh(cx);
                            }
                            cx.notify();
                        })
                        .ok();
                    })
                    .detach();
                }
            }
            Parked::Closing | Parked::InFlight => {}
        }
        // Inbox Zero offers Z only until now.
        cx.notify();
    }

    /// Sends a message whose undo delay is over.
    fn dispatch(&mut self, mut composer: Composer, cx: &mut Context<Self>) {
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
                self.notice = Some(match serial {
                    Some(_) => "Message sent.".into(),
                    None => format!("Sent “{subject}”, which was waiting.").into(),
                });
                if matches!(self.mailbox, View::Sent | View::Drafts) {
                    self.refresh(cx);
                }
                cx.notify();
                return;
            }
            Err(error) => error,
        };
        let offline = error.is::<gmail::Offline>();
        let unsure = error.is::<gmail::MayHaveArrived>();
        let room = parked.is_some() && self.compose.is_none();
        if self.shown_signed_out(&error) {
            self.compose = None;
            self.fail(error, cx);
        } else if offline && !unsure && (shown || !room) {
            if let Some(at) = parked {
                self.parked.remove(at);
            }
            self.queue.push(job);
            if shown {
                self.notice = Some(
                    "You’re offline. The message will be sent when the connection is back.".into(),
                );
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
    fn abandon_sends(&mut self, account: &str) {
        self.queue.retain(|job| job.account != account);
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
    fn tell(&mut self, text: impl Into<SharedString>) {
        let text = text.into();
        self.must_show = Some(text.clone());
        self.notice = Some(text);
    }

    /// A message that is not to be sent by itself goes to Drafts, and `why` says so.
    fn send_to_drafts(&self, job: Arc<Job>, why: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let saved = cx
                .background_executor()
                .spawn(async move { job.to_drafts() })
                .await;
            this.update(cx, |this, cx| {
                this.tell(if saved {
                    format!("{why} It is in Drafts.")
                } else {
                    format!("{why} It is kept on this Mac.")
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A message found in the store: quit, a crash or a switch of accounts got in its way.
    fn pick_up_send(&mut self, job: Arc<Job>, cx: &mut Context<Self>) {
        let subject = short(&job.subject(), 40);
        match job.stage() {
            Stage::Held => {
                self.sends.push(job.clone());
                self.send_job(job, cx);
            }
            Stage::Out | Stage::Unsure => self.send_to_drafts(
                job,
                format!("“{subject}” may already be sent: check Sent before sending it again."),
                cx,
            ),
            Stage::Refused => self.send_to_drafts(job, format!("“{subject}” was not sent."), cx),
            Stage::Writing | Stage::Unscheduled => {
                let what = if subject.trim().is_empty() {
                    "A message".to_string()
                } else {
                    format!("“{subject}”")
                };
                let why = if job.stage() == Stage::Writing {
                    format!("{what} was being written when Nimbra quit.")
                } else {
                    format!("{what} was not scheduled before Nimbra quit.")
                };
                self.send_to_drafts(job, why, cx);
            }
            Stage::Over => job.forget(),
        }
    }

    /// Quitting: what was sent goes now, undo delay or not; what is being written goes to
    /// Drafts; and what is on its way is waited for.
    fn finish_before_quit(&mut self, cx: &mut Context<Self>) {
        let leaving = self.leaving(cx);
        if leaving.is_empty() {
            return;
        }
        // Nothing on screen can answer while this waits.
        cx.hide();
        leaving.finish(std::time::Instant::now() + QUIT_WAIT);
    }

    /// Whether an account is still signed in here: nothing is written down for one that is
    /// not, as its store was deleted.
    fn keeps(&self, account: &str) -> bool {
        self.settings.accounts.iter().any(|a| a.id == account)
            || (self.gmail.is_some() && account == self.account_id)
    }

    fn store_of(&self, account: &str) -> Option<Arc<crate::store::Store>> {
        self.store.clone().filter(|_| account == self.account_id)
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
        let queued = (self.store.is_none()
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

    /// A recipient field or the message changed.
    fn field_changed(&mut self, field: Field, cx: &mut Context<Self>) {
        // A comma (or semicolon) finishes an address: it becomes a pill.
        if let Some(composer) = &mut self.compose
            && Composer::slot(field).is_some()
            && composer
                .input(field)
                .is_some_and(|input| input.read(cx).text().contains([',', ';']))
        {
            composer.chipify(field, false, cx);
        }
        let suggest = match field {
            Field::To | Field::Cc | Field::Bcc => self
                .compose
                .as_ref()
                .and_then(|c| c.input(field))
                .and_then(|input| {
                    let text = input.read(cx).text();
                    let token = text
                        .rsplit([',', ';'])
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_lowercase();
                    if token.is_empty() {
                        return None;
                    }
                    let present: Vec<String> = text
                        .split([',', ';'])
                        .map(|p| p.trim().to_lowercase())
                        .collect();
                    let people = self.match_contacts(&token, &present);
                    (!people.is_empty()).then_some(Suggest {
                        field,
                        people,
                        selected: 0,
                    })
                }),
            _ => None,
        };
        if let Some(composer) = &mut self.compose {
            composer.suggest = suggest;
        }
        self.composer_changed(cx);
        cx.notify();
    }

    /// People from the mail on this Mac whose name or address starts with `token`, most
    /// often seen first.
    fn match_contacts(&self, token: &str, present: &[String]) -> Vec<gmail::Person> {
        let me = self.account.to_lowercase();
        let mut seen: HashMap<String, (gmail::Person, u32)> = HashMap::new();
        let mut add = |p: &gmail::Person| {
            let key = p.address.to_lowercase();
            if key.is_empty() || key == me {
                return;
            }
            let entry = seen.entry(key).or_insert_with(|| (p.clone(), 0));
            entry.1 += 1;
            if entry.0.name.is_empty() && !p.name.is_empty() {
                entry.0.name = p.name.clone();
            }
        };
        for mail in self.mails.iter().chain(self.cache.values().flatten()) {
            for header in
                std::iter::once(&mail.header).chain(mail.earlier.iter().map(|e| &e.header))
            {
                add(&header.from);
                header.to.iter().for_each(&mut add);
                header.cc.iter().for_each(&mut add);
            }
        }
        let mut found: Vec<(gmail::Person, u32)> = seen
            .into_values()
            .filter(|(p, _)| {
                let address = p.address.to_lowercase();
                let name = p.name.to_lowercase();
                !present.contains(&address)
                    && (address.starts_with(token)
                        || name.starts_with(token)
                        || name.split_whitespace().any(|w| w.starts_with(token)))
            })
            .collect();
        found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.address.cmp(&b.0.address)));
        found.into_iter().take(5).map(|(p, _)| p).collect()
    }

    /// Puts the chosen suggestion in place of what was being typed.
    fn accept_suggestion(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        let Some(composer) = &mut self.compose else {
            return;
        };
        let Some(suggest) = composer.suggest.take() else {
            return;
        };
        let (Some(person), Some(input)) = (
            suggest
                .people
                .get(index.unwrap_or(suggest.selected))
                .cloned(),
            composer.input(suggest.field),
        ) else {
            return;
        };
        let text = input.read(cx).text();
        let cut = text.rfind([',', ';']).unwrap_or(0);
        let head = text[..cut].trim().to_string();
        input.update(cx, |input, cx| input.set(head, usize::MAX, cx));
        if let Some(slot) = Composer::slot(suggest.field)
            && !composer.chips[slot]
                .iter()
                .any(|c| c.eq_ignore_ascii_case(&person.address))
        {
            composer.chips[slot].push(person.address.clone());
        }
        composer.sync_placeholder(suggest.field, cx);
        self.pending_focus = Some(input.read(cx).focus_handle(cx));
        self.composer_changed(cx);
        cx.notify();
    }

    fn move_suggestion(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(suggest) = self.compose.as_mut().and_then(|c| c.suggest.as_mut()) {
            let len = suggest.people.len() as isize;
            suggest.selected = (suggest.selected as isize + delta).rem_euclid(len) as usize;
            cx.notify();
        }
    }

    /// Opens the file picker to attach files.
    fn pick_files(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                this.update(cx, |this, cx| this.attach_files(paths, cx))
                    .ok();
            }
        })
        .detach();
    }

    /// Reads picked or dropped files into the composer, within Gmail's 25 MB limit.
    fn attach_files(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        let Some(composer) = &self.compose else {
            return;
        };
        let serial = composer.serial;
        let used = composer
            .files
            .iter()
            .map(|f| f.bytes.len() as u64)
            .sum::<u64>()
            + composer.kept.iter().map(|a| a.size).sum::<u64>();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut files = Vec::new();
                    let mut total = used;
                    for path in paths.into_iter().take(20) {
                        let meta = std::fs::metadata(&path)?;
                        if !meta.is_file() {
                            anyhow::bail!("Folders can’t be attached. Zip them first.");
                        }
                        total += meta.len();
                        if total > gmail::ATTACHMENT_LIMIT {
                            anyhow::bail!("Attachments can be 25 MB in total.");
                        }
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let bytes = std::fs::read(&path)?;
                        files.push(gmail::OutgoingFile {
                            mime: gmail::mime_for(&name).into(),
                            name: gmail::safe_file_name(&name),
                            bytes,
                        });
                    }
                    anyhow::Ok(files)
                })
                .await;
            this.update(cx, |this, cx| {
                let Some(composer) = this.composer_mut(serial) else {
                    return;
                };
                match result {
                    Ok(files) => {
                        composer.files.extend(files);
                        composer.error = None;
                    }
                    Err(error) => composer.error = Some(error.to_string().into()),
                }
                if this.compose.as_ref().is_some_and(|c| c.serial == serial) {
                    this.composer_changed(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The times offered by Send later, with their labels.
    fn send_later_choices() -> Vec<(String, i64)> {
        use chrono::{Datelike, Duration as Days, Local, TimeZone, Timelike, Weekday};
        let now = Local::now();
        let at = |date: chrono::NaiveDate, hour: u32| {
            Local
                .from_local_datetime(&date.and_hms_opt(hour, 0, 0).unwrap_or_default())
                .single()
                .map(|t| t.timestamp_millis())
        };
        let mut choices = Vec::new();
        let hour = (now + Days::hours(1)).with_second(0).unwrap_or(now);
        choices.push((
            format!("In an hour · {}", hour.format("%H:%M")),
            hour.timestamp_millis(),
        ));
        if now.hour() < 17
            && let Some(ms) = at(now.date_naive(), 18)
        {
            choices.push(("This evening · 18:00".into(), ms));
        }
        let tomorrow = now.date_naive() + Days::days(1);
        if let Some(ms) = at(tomorrow, 8) {
            choices.push((
                format!("Tomorrow morning · {} 8:00", tomorrow.format("%a")),
                ms,
            ));
        }
        let mut monday = now.date_naive() + Days::days(1);
        while monday.weekday() != Weekday::Mon {
            monday += Days::days(1);
        }
        if monday != tomorrow
            && let Some(ms) = at(monday, 8)
        {
            choices.push((
                format!("Monday morning · {} 8:00", monday.format("%b %-d")),
                ms,
            ));
        }
        choices
    }

    /// Send later: saves the draft, then sends it from this Mac at `at`.
    fn schedule_send(&mut self, at: i64, cx: &mut Context<Self>) {
        let demo = self.gmail.is_none();
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
    fn add_scheduled(&mut self, item: Scheduled, cx: &mut Context<Self>) {
        let (owner, store) = (self.account_id.clone(), self.store.clone());
        let (at, subject) = (item.at_ms, short(&item.subject, 40));
        if let Some(store) = store {
            let added = scheduled::add(&store, item).is_ok();
            self.scheduled_added(&owner, at, &subject, added, cx);
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
            self.read_scheduled();
            self.watch_schedule(cx);
        }
        cx.notify();
    }

    /// Takes the send-later list as it now is in the store.
    fn read_scheduled(&mut self) {
        if let Some(store) = &self.store {
            self.scheduled = scheduled::list(store);
        }
    }

    /// Checks for scheduled messages that are due, every 20 seconds while Nimbra runs.
    fn watch_schedule(&mut self, cx: &mut Context<Self>) {
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
        let (Some(store), Some(_)) = (self.store.clone(), &self.gmail) else {
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
                        scheduled::send(&store, &client, &id)
                    })
                    .await;
                this.update(cx, |this, cx| {
                    this.sending_scheduled.remove(&item.draft_id);
                    // Whose list it was may no longer be the one shown.
                    if this.account_id == owner {
                        this.read_scheduled();
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
                    if outcome.is_some() && matches!(this.mailbox, View::Sent | View::Drafts) {
                        this.refresh(cx);
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    fn focus_field(&mut self, field: Field, cursor: Option<usize>, cx: &mut Context<Self>) {
        let Some(composer) = &self.compose else {
            return;
        };
        if composer.rich && matches!(field, Field::Line(_)) {
            if let Some(editor) = &self.editor {
                editor.focus();
            }
            self.pending_focus = Some(self.editor_focus.clone());
            cx.notify();
            return;
        }
        if let Some(editor) = &self.editor {
            editor.blur();
        }
        let Some(input) = composer.input(field) else {
            return;
        };
        if let Some(composer) = &mut self.compose
            && composer.suggest.as_ref().is_some_and(|s| s.field != field)
        {
            composer.suggest = None;
        }
        if let Some(cursor) = cursor {
            input.update(cx, |input, cx| {
                let text = input.text();
                input.set(text, cursor, cx)
            });
        }
        self.pending_focus = Some(input.read(cx).focus_handle(cx));
        cx.notify();
    }

    /// Enter, arrows, Tab and Backspace-at-start make the separate fields behave like one
    /// form with a multi-line body.
    fn field_event(&mut self, field: Field, event: &InputEvent, cx: &mut Context<Self>) {
        let Some(line_count) = self.compose.as_ref().map(|c| c.lines.len()) else {
            return;
        };
        let show_cc = self.compose.as_ref().is_some_and(|c| c.show_cc);
        let suggesting = self
            .compose
            .as_ref()
            .and_then(|c| c.suggest.as_ref())
            .is_some_and(|s| s.field == field);
        match (field, event) {
            (_, InputEvent::Enter | InputEvent::Tab) if suggesting => {
                self.accept_suggestion(None, cx)
            }
            (_, InputEvent::Down) if suggesting => self.move_suggestion(1, cx),
            (_, InputEvent::Up) if suggesting => self.move_suggestion(-1, cx),
            // Backspace in an empty recipient field takes the last pill back as text.
            (Field::To | Field::Cc | Field::Bcc, InputEvent::BackspaceAtStart) => {
                if let Some(composer) = &mut self.compose
                    && let (Some(slot), Some(input)) =
                        (Composer::slot(field), composer.input(field))
                    && input.read(cx).text().is_empty()
                    && let Some(address) = composer.chips[slot].pop()
                {
                    input.update(cx, |input, cx| input.set(address, usize::MAX, cx));
                    composer.sync_placeholder(field, cx);
                    self.composer_changed(cx);
                    cx.notify();
                }
            }
            (
                Field::To | Field::Cc | Field::Bcc,
                InputEvent::Enter | InputEvent::Tab | InputEvent::Down,
            ) => {
                if let Some(composer) = &mut self.compose {
                    composer.chipify(field, true, cx);
                }
                let next = match field {
                    Field::To if show_cc => Field::Cc,
                    Field::Cc => Field::Bcc,
                    _ => Field::Subject,
                };
                self.focus_field(next, None, cx)
            }
            (Field::Cc, InputEvent::Up) => self.focus_field(Field::To, None, cx),
            (Field::Bcc, InputEvent::Up) => self.focus_field(Field::Cc, None, cx),
            (Field::Subject, InputEvent::Enter | InputEvent::Tab | InputEvent::Down) => {
                self.focus_field(Field::Line(0), None, cx)
            }
            (Field::Subject, InputEvent::Up) => {
                self.focus_field(if show_cc { Field::Bcc } else { Field::To }, None, cx)
            }
            (Field::Line(0), InputEvent::Up) => self.focus_field(Field::Subject, None, cx),
            (Field::Line(index), InputEvent::Up) => {
                let cursor = self
                    .compose
                    .as_ref()
                    .map(|c| c.lines[index].read(cx).cursor());
                self.focus_field(Field::Line(index - 1), cursor, cx)
            }
            (Field::Line(index), InputEvent::Down) if index + 1 < line_count => {
                let cursor = self
                    .compose
                    .as_ref()
                    .map(|c| c.lines[index].read(cx).cursor());
                self.focus_field(Field::Line(index + 1), cursor, cx)
            }
            // Enter splits the line at the cursor.
            (Field::Line(index), InputEvent::Enter) => {
                let (new_line, subscription) = self.new_field(Field::Line(index + 1), "", cx);
                let Some(composer) = &mut self.compose else {
                    return;
                };
                let current = composer.lines[index].clone();
                let (text, cursor) = {
                    let input = current.read(cx);
                    (input.text(), input.cursor())
                };
                let tail = text[cursor..].to_string();
                current.update(cx, |input, cx| {
                    input.set(text[..cursor].to_string(), cursor, cx)
                });
                new_line.update(cx, |input, cx| input.set(tail, 0, cx));
                composer.lines.insert(index + 1, new_line);
                composer._subscriptions.push(subscription);
                self.focus_field(Field::Line(index + 1), None, cx);
            }
            // Backspace at the start joins the line onto the one above.
            (Field::Line(index), InputEvent::BackspaceAtStart) if index > 0 => {
                let Some(composer) = &mut self.compose else {
                    return;
                };
                let removed = composer.lines.remove(index);
                let tail = removed.read(cx).text();
                let previous = composer.lines[index - 1].clone();
                let head = previous.read(cx).text();
                let join = head.len();
                previous.update(cx, |input, cx| input.set(format!("{head}{tail}"), join, cx));
                self.focus_field(Field::Line(index - 1), None, cx);
            }
            _ => {}
        }
    }

    /// Sends what was written, after a few seconds in which it can be undone. Only ever
    /// runs from the Send button or its shortcut.
    fn send_mail(&mut self, cx: &mut Context<Self>) {
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
        let outgoing = composer.outgoing(cx);
        if let Err(error) = outgoing.recipients() {
            composer.error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        if outgoing.subject.trim().is_empty()
            && outgoing.body.trim().is_empty()
            && outgoing.files.is_empty()
        {
            composer.error = Some("Write a subject or a message first.".into());
            cx.notify();
            return;
        }
        if outgoing.attachment_size() > gmail::ATTACHMENT_LIMIT {
            composer.error = Some("Attachments can be 25 MB in total.".into());
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
            .spawn(async move { job.keep() })
            .detach();
        composer.suggest = None;
        composer.error = None;
        let serial = composer.serial;
        self.parked.push((Parked::Sending, composer));
        self.pending_focus = Some(self.focus.clone());
        // Settings › Composing: how long a sent message can still be taken back.
        let hold = Duration::from_secs(self.settings.undo_send.min(60) as u64);
        if hold.is_zero() {
            self.finish_parked(serial, cx);
        } else {
            self.offer_undo("Sending…", cx);
            self.finish_parked_later(serial, hold, cx);
        }
        cx.notify();
    }

    /// Fetches the remote images of a message after the user asked for them.
    fn load_images(&mut self, sources: Vec<String>, cx: &mut Context<Self>) {
        for url in sources.into_iter().take(16) {
            if !self.image_requests.insert(url.clone()) {
                continue;
            }
            cx.spawn(async move |this, cx| {
                let fetch_url = url.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move { rich::fetch_image(&fetch_url) })
                    .await;
                let Ok((kind, bytes)) = result else {
                    return;
                };
                let format = match kind {
                    rich::ImageKind::Png => gpui::ImageFormat::Png,
                    rich::ImageKind::Jpeg => gpui::ImageFormat::Jpeg,
                    rich::ImageKind::Gif => gpui::ImageFormat::Gif,
                    rich::ImageKind::Webp => gpui::ImageFormat::Webp,
                };
                this.update(cx, |this, cx| {
                    this.images
                        .insert(url, Arc::new(gpui::Image::from_bytes(format, bytes)));
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    /// How many rows the palette has.
    fn palette_len(&self) -> usize {
        if self.is_demo() {
            COMMANDS.len()
        } else {
            self.snooze_choices.len()
        }
    }

    /// Runs the highlighted palette command. Snoozing is only simulated on the sample data;
    /// on a real mailbox it must not archive anything, so it just says so.
    fn run_command(&mut self, cx: &mut Context<Self>) {
        self.palette = false;
        let meant = self.palette_for.clone();
        if meant.is_some() && !self.still_meant(&meant, cx) {
            return;
        }
        if self.is_demo() {
            self.mark_done(cx);
        } else if let Some(choice) = self.snooze_choices.get(self.palette_sel).cloned() {
            self.snooze(choice, cx);
        }
    }

    /// ⌘⇧L and the sun/moon button: switch to the other theme, and remember it.
    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        self.dark = !self.dark;
        self.settings.theme = if self.dark {
            crate::settings::ThemeChoice::Dark
        } else {
            crate::settings::ThemeChoice::Light
        };
        self.save_settings(cx);
        cx.notify();
    }

    fn save_settings(&self, _cx: &mut Context<Self>) {
        // Not in the background: the last change has to be the one on disk, also when two
        // come quickly or the app quits straight after. It is a tiny file, moved into place.
        if crate::settings::save(&self.settings).is_err() {
            eprintln!("nimbra: could not save settings");
        }
    }

    /// ⌘, or the account menu: the Settings window (brought forward if already open).
    fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        self.palette = false;
        self.login_item = crate::system::opens_at_login();
        if let Some(handle) = &self.settings_window
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let owner = cx.entity();
        let (signature, dark) = (self.settings.signature.clone(), self.dark);
        // Opened once this update is over: the new window reads the app as it draws.
        cx.defer(move |cx| {
            let bounds = gpui::Bounds::centered(None, gpui::size(px(860.), px(620.)), cx);
            let me = owner.clone();
            let opened = cx.open_window(
                gpui::WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("Settings".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(gpui::point(px(16.), px(16.))),
                    }),
                    window_min_size: Some(gpui::size(px(720.), px(480.))),
                    ..Default::default()
                },
                |_, cx| cx.new(|cx| SettingsWindow::new(owner, signature, dark, cx)),
            );
            me.update(cx, |n, _| n.settings_window = opened.ok());
        });
    }

    /// Puts `text` on the clipboard and marks `key` as just copied for a moment.
    fn copy_text(&mut self, key: String, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        self.copied = Some((key.clone(), std::time::Instant::now()));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1400))
                .await;
            this.update(cx, |this, cx| {
                if this.copied.as_ref().is_some_and(|(k, _)| *k == key) {
                    this.copied = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Settings › Privacy: forgets the mail kept on this Mac (Gmail is untouched) and loads
    /// the open mailbox again.
    fn clear_local_mail(&mut self, cx: &mut Context<Self>) {
        if let Some(store) = &self.store {
            store.clear();
        }
        self.cache.clear();
        self.synced_to.clear();
        self.notify_from = None;
        self.forget_checked();
        for client in self.gmail.iter().chain(self.others.values()) {
            client.forget_rows();
        }
        self.notice =
            Some("Cleared the mail kept on this Mac. It is downloaded again as needed.".into());
        self.fetch_inbox(cx);
        cx.notify();
    }

    /// Signs out: forgets the Google sign-in and removes the mail kept on this Mac and its
    /// key. Settings (signature, theme) stay.
    fn sign_out(&mut self, cx: &mut Context<Self>) {
        let account = self.account_id.clone();
        // Signing out deletes what is kept of it.
        // (Not what waits for a connection: that would hold signing out up for as long.)
        let on_its_way = self.sends.iter().any(|job| {
            job.account == account && !self.queue.iter().any(|waiting| Arc::ptr_eq(waiting, job))
        });
        if on_its_way {
            self.notice = Some(
                "A message from this account is still being sent. Sign out again in a moment."
                    .into(),
            );
            cx.notify();
            return;
        }
        self.abandon_sends(&account);
        let gone = account.clone();
        std::thread::spawn(move || {
            gmail::forget_refresh_token(&gone);
            crate::store::wipe(&gone);
        });
        self.settings.accounts.retain(|a| a.id != account);
        self.others.remove(&account);
        self.account_unread.remove(&account);
        // Another account is signed in: show that one.
        if let Some(next) = self.settings.accounts.first().map(|a| a.id.clone()) {
            self.switch_account(next, cx);
            self.notice =
                Some("Signed out of that account. Its mail was removed from this Mac.".into());
            return;
        }
        self.settings.active = String::new();
        self.account_id = String::new();
        self.save_settings(cx);
        self.gmail = None;
        self.store = None;
        // An answer still on its way must not bring the removed mail back.
        self.checks.forget();
        self.offline = false;
        self.stop_retrying();
        // Whoever signs in next must not send this account's queue.
        self.outbox = Outbox::default();
        self.next_pages.clear();
        self.forget_checked();
        self.cache.clear();
        self.mails.clear();
        self.done.clear();
        self.history.clear();
        self.picked.clear();
        self.history_groups.clear();
        self.back.clear();
        self.forward.clear();
        self.scheduled.clear();
        self.parked.clear();
        self.compose = None;
        self.selected = 0;
        self.palette = false;
        self.account = "".into();
        self.status = Status::SignedOut;
        self.notice = Some("Signed out. The mail kept on this Mac was removed.".into());
        cx.notify();
    }

    /// Whether an error ends the shown account's sign-in. Other accounts' connections are in
    /// use too (All inboxes, message windows, requests from before a switch): theirs do not.
    fn shown_signed_out(&self, error: &anyhow::Error) -> bool {
        error
            .downcast_ref::<gmail::SignedOut>()
            .is_some_and(|e| e.account.is_empty() || e.account == self.account_id)
    }

    fn fail(&mut self, error: anyhow::Error, cx: &mut Context<Self>) {
        // Offline is a state, not an error: keep showing what is here, and catch up later.
        if error.is::<gmail::Offline>() {
            if self.mails.is_empty() && !matches!(self.status, Status::Ready) {
                self.status = Status::Failed(error.to_string().into());
            }
            self.went_offline(cx);
            cx.notify();
            return;
        }
        eprintln!("nimbra: {error}");
        if let Some(out) = error.downcast_ref::<gmail::SignedOut>()
            && !self.shown_signed_out(&error)
        {
            // Another account's: the mail, snoozes and sign-in of the one shown are not its.
            let whose = self
                .settings
                .accounts
                .iter()
                .find(|a| a.id == out.account)
                .map_or("another account".to_string(), |a| a.email.clone());
            self.notice = Some(
                format!("The sign-in for {whose} expired. Switch to it to connect again.").into(),
            );
            cx.notify();
            return;
        }
        if self.shown_signed_out(&error) {
            self.gmail = None;
            // The stored mail belongs to that sign-in: remove it, and its key.
            self.store = None;
            self.checks.forget();
            self.offline = false;
            self.stop_retrying();
            // Whoever signs in next must not send this account's mail.
            self.outbox = Outbox::default();
            let shown = self.account_id.clone();
            self.abandon_sends(&shown);
            self.parked
                .retain(|(_, c)| c.account.as_ref().is_some_and(|a| *a != shown));
            self.next_pages.clear();
            self.forget_checked();
            let account = self.account_id.clone();
            std::thread::spawn(move || crate::store::wipe(&account));
            self.cache.clear();
            self.mails.clear();
            self.done.clear();
            self.history.clear();
            self.picked.clear();
            self.history_groups.clear();
            self.back.clear();
            self.forward.clear();
            self.selected = 0;
            self.palette = false;
            self.status = Status::SignedOut;
            self.notice = Some("Your Google sign-in expired. Connect again to continue.".into());
        } else if self.mails.is_empty()
            // Slow is no reason to hide a mailbox known to be empty.
            && !(error.is::<TooSlow>() && self.status == Status::Ready)
        {
            self.status = Status::Failed(error.to_string().into());
        } else {
            self.notice = Some(error.to_string().into());
        }
        cx.notify();
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn theme(&self) -> Theme {
        if self.dark {
            Theme::dark()
        } else {
            Theme::light()
        }
    }

    fn visible(&self) -> Vec<usize> {
        (0..self.mails.len())
            .filter(|i| match (self.is_demo(), self.mailbox) {
                (true, View::Done) => self.done[*i],
                // The sample data is searched locally.
                (true, View::Search) => {
                    let mail = &self.mails[*i];
                    let query = self.query.to_lowercase();
                    !self.done[*i]
                        && [&mail.sender, &mail.subject, &mail.snippet]
                            .iter()
                            .any(|field| field.to_lowercase().contains(&query))
                }
                (true, View::Inbox | View::AllInboxes) | (false, _) => {
                    // The open thread stays listed under Unread even once it is read.
                    !self.done[*i]
                        && (!self.unread_only || self.mails[*i].unread || *i == self.selected)
                }
                (true, _) => false,
            })
            .collect()
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.palette {
            let max = self.palette_len() as isize - 1;
            self.palette_sel = (self.palette_sel as isize + delta).clamp(0, max) as usize;
        } else {
            self.picked.clear();
            let visible = self.visible();
            if let Some(pos) = visible.iter().position(|i| *i == self.selected) {
                let next = (pos as isize + delta).clamp(0, visible.len() as isize - 1);
                self.open(visible[next as usize]);
                self.open_selected(cx);
            }
        }
        cx.notify();
    }

    /// Opens a thread and records the previous one so Back can return to it.
    fn open(&mut self, i: usize) {
        if i != self.selected {
            self.back.push(self.selected);
            self.forward.clear();
            self.selected = i;
        }
    }

    fn can_go_back(&self) -> bool {
        self.back.iter().any(|i| !self.done[*i])
    }

    fn can_go_forward(&self) -> bool {
        self.forward.iter().any(|i| !self.done[*i])
    }

    fn go_back(&mut self, cx: &mut Context<Self>) {
        // Skip threads that were marked done in the meantime.
        while let Some(i) = self.back.pop() {
            if !self.done[i] {
                self.forward.push(self.selected);
                self.selected = i;
                break;
            }
        }
        self.open_selected(cx);
        cx.notify();
    }

    fn go_forward(&mut self, cx: &mut Context<Self>) {
        while let Some(i) = self.forward.pop() {
            if !self.done[i] {
                self.back.push(self.selected);
                self.selected = i;
                break;
            }
        }
        self.open_selected(cx);
        cx.notify();
    }

    /// Pull to refresh, in the style of the system's: the list stretches with rubber-band
    /// resistance, letting go past the threshold starts the refresh, and the indicator
    /// springs back when it is done. Only a trackpad gesture that begins with the list
    /// already at the top counts, so the momentum of a normal scroll up never triggers it.
    fn pull_list(&mut self, event: &gpui::ScrollWheelEvent, cx: &mut Context<Self>) {
        let at_top = f32::from(self.list_scroll.offset().y) >= -0.5;
        match event.touch_phase {
            gpui::TouchPhase::Started => {
                self.pull_armed = at_top && !self.pull_hold;
                return;
            }
            gpui::TouchPhase::Ended => {
                self.pull_armed = false;
                if self.pull <= 0. {
                    return;
                }
                let shown = rubber_band(self.pull);
                self.pull = 0.;
                self.pull_gen += 1;
                self.pull_done = false;
                if shown >= PULL_TRIGGER && !self.fetching() {
                    // Settle at the resting height and stay there while mail is fetched.
                    self.pull_anim = Some((shown, PULL_REST, std::time::Instant::now()));
                    self.pull_hold = true;
                    self.reload(cx);
                } else {
                    self.pull_anim = Some((shown, 0., std::time::Instant::now()));
                }
                cx.notify();
                return;
            }
            gpui::TouchPhase::Moved => {}
        }
        if !self.pull_armed {
            return;
        }
        let dy = f32::from(event.delta.pixel_delta(px(18.)).y);
        if (dy > 0. && at_top) || (dy < 0. && self.pull > 0.) {
            self.pull = (self.pull + dy).max(0.);
            self.pull_anim = None;
            // While the list is being pulled it must not scroll as well, or the two
            // movements fight each other.
            self.list_scroll.set_offset(gpui::point(px(0.), px(0.)));
            cx.notify();
        }
    }

    /// The strip at the top of the list while pulling, refreshing or settling.
    fn pull_indicator(&self, t: &Theme) -> Option<AnyElement> {
        let strip = |height: f32, label: &'static str, spin: bool, turn: f32| {
            let arrow = icon("reload", 15., t.violet);
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .h(px(height.max(0.)))
                .overflow_hidden()
                .child(if spin {
                    arrow
                        .with_animation(
                            "pull-spin",
                            Animation::new(Duration::from_millis(800)).repeat(),
                            |svg, delta| {
                                svg.with_transformation(Transformation::rotate(percentage(delta)))
                            },
                        )
                        .into_any_element()
                } else {
                    arrow
                        .with_transformation(Transformation::rotate(percentage(turn)))
                        .into_any_element()
                })
                .child(meta(label, t.text3))
        };
        if self.pull > 0. {
            // Following the finger: no animation, just the stretched height.
            let shown = rubber_band(self.pull);
            let ready = shown >= PULL_TRIGGER;
            return Some(
                strip(
                    shown,
                    if ready {
                        "Release to refresh"
                    } else {
                        "Pull to refresh"
                    },
                    false,
                    (shown / PULL_TRIGGER).min(1.) * 0.75,
                )
                .opacity(0.45 + 0.55 * (shown / PULL_TRIGGER).min(1.))
                .into_any_element(),
            );
        }
        let (from, to, start) = self.pull_anim?;
        let label = if self.pull_hold {
            "Checking for mail…"
        } else if self.pull_done {
            "Done"
        } else {
            ""
        };
        Some(
            strip(settle(from, to, start), label, self.pull_hold, 0.75)
                .with_animation(
                    ("pull-settle", self.pull_gen),
                    // The animation only keeps frames coming; the height follows the spring.
                    Animation::new(Duration::from_secs_f32(SETTLE_SECONDS + 0.1)),
                    move |strip, _| strip.h(px(settle(from, to, start))),
                )
                .into_any_element(),
        )
    }

    /// The message list in a wrapper whose width slides open or shut after a toggle, the
    /// same way the sidebar does. Once it has finished closing it is not drawn at all.
    fn list_slot(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        const SLIDE: Duration = Duration::from_millis((PANEL_SLIDE_SECONDS * 1000.) as u64);
        let moving = self.list_toggled.is_some_and(|at| at.elapsed() < SLIDE);
        if !self.list_open && !moving {
            return None;
        }
        let width = self.list_width;
        // The list keeps its width while the slot around it grows or shrinks, so its rows
        // slide instead of re-wrapping on every frame.
        let slot = div()
            .flex()
            .justify_end()
            .flex_none()
            .h_full()
            .overflow_hidden()
            .child(
                div()
                    .flex_none()
                    .w(px(width))
                    .h_full()
                    .child(self.list(t, cx)),
            );
        if !moving {
            return Some(slot.w(px(width)).into_any_element());
        }
        let (from, to) = if self.list_open { (0., 1.) } else { (1., 0.) };
        Some(
            slot.with_animation(
                ("list-width", self.list_gen),
                Animation::new(SLIDE).with_easing(panel_ease),
                move |slot, delta| slot.w(px(width * (from + (to - from) * delta))),
            )
            .into_any_element(),
        )
    }

    /// Hides or shows the message list, leaving the reading pane the full width.
    fn toggle_list(&mut self, cx: &mut Context<Self>) {
        self.list_open = !self.list_open;
        if self.list_open {
            self.focus_restore = None;
        }
        self.list_grab = None;
        self.list_toggled = Some(std::time::Instant::now());
        self.list_gen += 1;
        cx.notify();
    }

    /// ⌘B, the toggle and the menu: the choice is kept for the next launch. Full view hides
    /// the sidebar only for as long as it lasts, so it calls `slide_sidebar` instead.
    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.slide_sidebar(cx);
        self.settings.sidebar_auto_hide = !self.sidebar_open;
        self.save_settings(cx);
    }

    fn slide_sidebar(&mut self, cx: &mut Context<Self>) {
        // Opened while it floats: it docks from where it is. Shut: away until the edge is
        // reached again, though the pointer is still on it.
        self.docked_from = self
            .sidebar_place(0.)
            .filter(|place| !self.sidebar_open && place.inset > 0.)
            .map(|place| place.x);
        self.peek.reset();
        self.sidebar_open = !self.sidebar_open;
        self.sidebar_toggled = Some(std::time::Instant::now());
        if self.sidebar_open {
            self.focus_restore = None;
        }
        cx.notify();
    }

    fn sidebar_sliding(&self) -> bool {
        self.sidebar_toggled
            .is_some_and(|at| at.elapsed().as_secs_f32() < PANEL_SLIDE_SECONDS)
    }

    /// Where the sidebar is drawn `ahead` seconds from now: docked, sliding, floating over
    /// the content, or (`None`) nowhere.
    fn sidebar_place(&self, ahead: f32) -> Option<SidebarPlace> {
        let sliding = self.sidebar_sliding();
        if self.sidebar_open || sliding {
            if sliding
                && self.sidebar_open
                && let (Some(from), Some(at)) = (self.docked_from, self.sidebar_toggled)
            {
                let left =
                    1. - panel_ease((at.elapsed().as_secs_f32() + ahead) / PANEL_SLIDE_SECONDS);
                return Some(SidebarPlace {
                    x: from * left,
                    inset: FLOAT_INSET * left,
                });
            }
            let width = self.sidebar_width(ahead);
            return (width > 0.).then_some(SidebarPlace {
                x: width - SIDEBAR_W,
                inset: 0.,
            });
        }
        let shown = self.peek.progress(ahead);
        (shown > 0.).then_some(SidebarPlace {
            x: FLOAT_INSET - (SIDEBAR_W + 2. * FLOAT_INSET) * (1. - shown),
            inset: FLOAT_INSET,
        })
    }

    /// The room the panes leave for the sidebar, following its slide.
    fn sidebar_width(&self, ahead: f32) -> f32 {
        let to = if self.sidebar_open { 1. } else { 0. };
        match self
            .sidebar_toggled
            .map(|at| at.elapsed().as_secs_f32() + ahead)
        {
            Some(elapsed) if elapsed < PANEL_SLIDE_SECONDS => {
                let from = 1. - to;
                SIDEBAR_W * (from + (to - from) * panel_ease(elapsed / PANEL_SLIDE_SECONDS))
            }
            _ => SIDEBAR_W * to,
        }
    }

    /// Something is drawn over the whole window, so the hidden sidebar stays away.
    fn window_covered(&self) -> bool {
        self.palette
            || self.finder_open
            || self.new_label.is_some()
            || self.confirm.is_some()
            || (self.compose.is_some() && self.compose_host.is_none())
    }

    /// While the sidebar is hidden, follows the pointer to bring it in and send it away.
    fn watch_edge(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.sidebar_open || self.watching_edge {
            return;
        }
        self.watching_edge = true;
        cx.spawn_in(window, async move |this, cx| {
            let mut wait = Duration::from_millis(16);
            loop {
                cx.background_executor().timer(wait).await;
                let Ok(next) = this.update_in(cx, |this, window, cx| this.look_at_edge(window, cx))
                else {
                    break;
                };
                match next {
                    Some(next) => wait = next,
                    None => break,
                }
            }
        })
        .detach();
    }

    /// One look at the pointer. Returns when to take the next, or `None` when done.
    fn look_at_edge(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<Duration> {
        if self.sidebar_open {
            self.watching_edge = false;
            return None;
        }
        // An open menu is a window of its own, which takes the focus from this one.
        let hold = self.menu_window.is_some();
        let right = FLOAT_INSET + SIDEBAR_W;
        let pointer = if self.sidebar_sliding() || self.window_covered() {
            None
        } else {
            crate::autohide::pointer(window, right)
        };
        if self
            .peek
            .look(pointer, right, hold && !self.window_covered())
        {
            cx.notify();
        }
        // Nothing can happen while inactive except becoming active: look less often.
        Some(Duration::from_millis(if window.is_window_active() {
            16
        } else {
            250
        }))
    }

    /// Checks for mail. With the sample data, everything that was cleared arrives again.
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.reload_gen += 1;
        if !self.is_demo() {
            self.counted = None;
            self.refresh(cx);
            // One that was running quietly is what was asked for now.
            self.checks.show();
            return;
        }
        self.done.fill(false);
        self.history.clear();
        self.picked.clear();
        self.history_groups.clear();
        self.back.clear();
        self.forward.clear();
        self.selected = 0;
        self.palette = false;
        cx.notify();
    }

    /// `E`: archive from the inbox, or move back to the inbox from Done.
    fn mark_done(&mut self, cx: &mut Context<Self>) {
        let Some(change) = self.done_change() else {
            return;
        };
        if self.mailbox == View::Snoozed {
            self.forget_snoozes(&self.targets());
        }
        self.remove_selected(change, cx);
    }

    /// Done from a message window. Whether it was done.
    fn done_thread(&mut self, index: usize, cx: &mut Context<Self>) -> bool {
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
    fn done_change(&self) -> Option<gmail::Change> {
        Some(match self.mailbox {
            view if inbox_like(view) => gmail::Change::Archive,
            View::Done => gmail::Change::Unarchive,
            View::Spam => gmail::Change::NotSpam,
            View::Trash => gmail::Change::Untrash,
            // Back to the inbox now, instead of at the snooze time.
            View::Snoozed => gmail::Change::Unarchive,
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
    fn trash_thread(&mut self, index: usize, cx: &mut Context<Self>) -> bool {
        if self.mailbox != View::Trash {
            self.remove_thread(index, gmail::Change::Trash, cx);
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
    fn trash(&mut self, cx: &mut Context<Self>) {
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
        self.remove_selected(gmail::Change::Trash, cx);
    }

    /// Takes the open thread out of the list and applies `change` to it in Gmail.
    fn remove_selected(&mut self, change: gmail::Change, cx: &mut Context<Self>) {
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
    fn remove_thread(&mut self, index: usize, change: gmail::Change, cx: &mut Context<Self>) {
        let shown = index == self.selected;
        self.remove_row(index, change, cx);
        if shown {
            self.load_selected(cx);
        }
    }

    /// Takes one thread off the list and sends the change. Opens nothing.
    fn remove_row(&mut self, index: usize, change: gmail::Change, cx: &mut Context<Self>) {
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
                gmail::Change::Trash => "Moved to trash",
                gmail::Change::Unarchive | gmail::Change::NotSpam => "Moved to inbox",
                gmail::Change::Spam => "Reported as spam",
                gmail::Change::Untrash => "Restored",
                _ => "Marked done",
            },
            cx,
        );
        cx.notify();
    }

    /// Shows the "Undo" pill at the bottom centre of the window for a few seconds.
    fn offer_undo(&mut self, label: &'static str, cx: &mut Context<Self>) {
        const SHOWN: Duration = Duration::from_secs(6);
        const SIZE: (f32, f32) = (300., 44.);
        self.close_undo(cx);
        let Some(frame) = self.frame else {
            return;
        };
        let origin = gpui::point(
            frame.origin.x + (frame.size.width - px(SIZE.0)) / 2. - px(MENU_MARGIN),
            frame.origin.y + frame.size.height - px(SIZE.1 + 24.) - px(MENU_MARGIN),
        );
        let owner = cx.weak_entity();
        let dark = self.dark;
        let opened = cx.open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::new(
                    origin,
                    gpui::size(px(SIZE.0 + 2. * MENU_MARGIN), px(SIZE.1 + 2. * MENU_MARGIN)),
                ))),
                titlebar: None,
                kind: gpui::WindowKind::PopUp,
                // Keyboard focus stays in the main window, so ⌘Z still reaches it.
                focus: false,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                window_background: gpui::WindowBackgroundAppearance::Transparent,
                ..Default::default()
            },
            |window, cx| {
                crate::blur::no_window_shadow(window);
                cx.new(|_| UndoToast { owner, label, dark })
            },
        );
        let Ok(handle) = opened else {
            return;
        };
        self.undo_window = Some(handle);
        self.undo_gen += 1;
        let shown_gen = self.undo_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SHOWN).await;
            this.update(cx, |this, cx| {
                if this.undo_gen == shown_gen {
                    this.close_undo(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Shows the current message in a small window at the bottom centre, above the email
    /// (a native view that would cover anything drawn in the main window). It goes away by
    /// itself after a few seconds, longer for longer messages.
    fn sync_notice_window(&mut self, toast: Option<Toast>, cx: &mut Context<Self>) {
        if let Some(handle) = self.notice_window.take() {
            handle
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
        let (Some(toast), Some(frame)) = (toast, self.frame) else {
            return;
        };
        let (width, height) = (toast.width, toast.height);
        // Long enough to read it.
        let seconds = (4. + toast.text.chars().count() as f32 / 25.).min(20.);
        // Above the Undo pill when that is showing too.
        let lift = if self.undo_window.is_some() { 54. } else { 0. };
        let origin = gpui::point(
            frame.origin.x + (frame.size.width - px(width)) / 2. - px(MENU_MARGIN),
            frame.origin.y + frame.size.height - px(height + 24. + lift) - px(MENU_MARGIN),
        );
        let owner = cx.weak_entity();
        let dark = self.dark;
        let opened = cx.open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::new(
                    origin,
                    gpui::size(px(width + 2. * MENU_MARGIN), px(height + 2. * MENU_MARGIN)),
                ))),
                titlebar: None,
                kind: gpui::WindowKind::PopUp,
                focus: false,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                window_background: gpui::WindowBackgroundAppearance::Transparent,
                ..Default::default()
            },
            |window, cx| {
                crate::blur::no_window_shadow(window);
                cx.new(|_| NoticeToast { owner, toast, dark })
            },
        );
        let Ok(handle) = opened else {
            return;
        };
        self.notice_window = Some(handle);
        self.notice_gen += 1;
        let shown = self.notice_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f32(seconds))
                .await;
            this.update(cx, |this, cx| {
                if this.notice_gen == shown {
                    this.notice = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn close_undo(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.undo_window.take() {
            handle
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
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
    fn can_undo(&self) -> bool {
        // With the composer in this window, Z is a letter.
        let typing = self.compose.is_some() && self.compose_host.is_none();
        !typing && (self.held_back().is_some() || !self.history.is_empty())
    }

    /// `Z`: brings back the last thread that was archived, moved or trashed.
    fn undo(&mut self, cx: &mut Context<Self>) {
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
            .unwrap_or(gmail::Change::Archive);
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
            if let Some(client) = self.gmail.clone() {
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

    fn toggle_star(&mut self, cx: &mut Context<Self>) {
        if self.picked.len() >= 2 {
            let targets = self.targets();
            let starred = !targets.iter().all(|i| self.mails[*i].starred);
            for index in targets {
                if self.mails[index].starred != starred {
                    self.mails[index].starred = starred;
                    let change = if starred {
                        gmail::Change::Star
                    } else {
                        gmail::Change::Unstar
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
            gmail::Change::Star
        } else {
            gmail::Change::Unstar
        };
        self.sync(index, change, false, cx);
        cx.notify();
    }

    fn set_unread_only(&mut self, unread_only: bool, cx: &mut Context<Self>) {
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

// ---------------------------------------------------------------- helpers

/// Breaks a title into lines for `width` (in points) at `size`, the way a typesetter
/// would: only at spaces (so "(PR #3345)" or "owner/repo" never split), and balanced, so
/// the lines are about even and the last is never a lone word. The breaks are returned as
/// newlines in place of those spaces, so character positions do not change.
fn balance_title(text: &str, size: f32, width: f32, cx: &App) -> String {
    if width <= 0. || !text.contains(' ') {
        return text.to_string();
    }
    let system = cx.text_system();
    let font_id = system.resolve_font(&gpui::font(crate::theme::SERIF));
    let measure = |s: &str| -> f32 {
        s.chars()
            .map(|c| {
                system
                    .advance(font_id, px(size), c)
                    .map_or(size * 0.45, |a| f32::from(a.width))
            })
            .sum()
    };
    let words: Vec<&str> = text.split(' ').collect();
    let widths: Vec<f32> = words.iter().map(|w| measure(w)).collect();
    let space = measure(" ");
    // Greedy line filling at a given width: the index of the first word of each line.
    let fill = |limit: f32| -> Vec<usize> {
        let mut starts = vec![0];
        let mut line = 0.;
        for (i, w) in widths.iter().enumerate() {
            let next = if line == 0. { *w } else { line + space + w };
            if next > limit && line > 0. {
                starts.push(i);
                line = *w;
            } else {
                line = next;
            }
        }
        starts
    };
    // A little slack: the italic tail and kerning make real lines slightly wider.
    let limit = width * 0.97;
    let lines = fill(limit).len();
    if lines == 1 {
        return text.to_string();
    }
    // The narrowest width that still needs no more lines than that: balanced lines.
    let (mut lo, mut hi) = (limit * 0.4, limit);
    for _ in 0..14 {
        let mid = (lo + hi) / 2.;
        if fill(mid).len() <= lines {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let starts = fill(hi);
    let mut out = String::with_capacity(text.len());
    for (i, word) in words.iter().enumerate() {
        if i > 0 {
            out.push(if starts.contains(&i) { '\n' } else { ' ' });
        }
        out.push_str(word);
    }
    out
}

fn serif(text: &str, size: f32, color: Hsla) -> Div {
    // The tail of the title (after the last comma, or the last word) is set in italic.
    let start = text
        .rfind(", ")
        .map(|i| i + 2)
        .or(text.rfind(' ').map(|i| i + 1))
        .unwrap_or(text.len());
    let italic = HighlightStyle {
        font_style: Some(FontStyle::Italic),
        ..Default::default()
    };
    div()
        .font_family(SERIF)
        .text_size(px(size))
        .line_height(px((size * 1.14).round()))
        .text_color(color)
        .child(StyledText::new(text.to_string()).with_highlights([(start..text.len(), italic)]))
}

/// Shortens text to `max` characters with an ellipsis.
///
/// Used instead of GPUI's own ellipsis truncation, which in 0.2.2 can panic when the cut
/// lands inside a multi-byte character such as an emoji. The element still clips whatever
/// does not fit.
fn short(text: &str, max: usize) -> SharedString {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{}…", head.trim_end()).into()
    } else {
        head.into()
    }
}

const SIDEBAR_W: f32 = 236.;
const NAV_RADIUS: f32 = 8.;
/// Between the search field and the first row under it.
const SEARCH_GAP: f32 = 10.;
/// The sidebar's row of window controls. Nothing scrolls into it, so nothing shows through
/// the buttons or outside the floating panel's round corners.
const SIDEBAR_TOP: f32 = 40.;
/// The room from there to the search field.
const SIDEBAR_CLEAR: f32 = 12.;
/// Below that row, where what scrolls up fades out.
const SIDEBAR_FADE: f32 = 28.;
/// The gap around the sidebar while it floats over the content.
const FLOAT_INSET: f32 = 6.;
/// Its corners then. The hole it cuts in the email's web view is rounded the same.
const FLOAT_RADIUS: f32 = 18.;

/// Where the sidebar is drawn: its left edge, and its gap from the window's top and bottom
/// (none when docked).
#[derive(Clone, Copy)]
struct SidebarPlace {
    x: f32,
    inset: f32,
}

/// How long the sidebar and the message list take to slide open or shut.
const PANEL_SLIDE_SECONDS: f32 = 0.3;

/// The curve of those slides: a symmetric ease-in-out (cubic). It starts gently, so the
/// first frame after a click is not already halfway, and settles softly.
fn panel_ease(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    if t < 0.5 {
        4. * t * t * t
    } else {
        1. - (-2. * t + 2.).powi(3) / 2.
    }
}

/// The space either side of a row's avatar, and at the row's right. With the list's own
/// 10pt margin it puts the avatars on `LIST_LEFT`.
const ROW_PAD: f32 = 14.;

/// The list header's margins: the list's text starts and ends on them.
const LIST_LEFT: f32 = 24.;
const LIST_RIGHT: f32 = 18.;
const HEADER_GAP: f32 = 8.;

/// How Compose is drawn, as the room for it shrinks.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ComposeLook {
    Keyed,
    Plain,
    Icon,
}

impl ComposeLook {
    fn width(self) -> f32 {
        match self {
            ComposeLook::Keyed => 112.,
            ComposeLook::Plain => 88.,
            ComposeLook::Icon => 34.,
        }
    }
}

/// What fits in the list's one-row header. The list can be 300pt narrow, so first the
/// Offline pill loses its words, then Compose its key, then Compose its label.
#[derive(Clone, Copy, Debug, PartialEq)]
struct HeaderFit {
    words: bool,
    compose: ComposeLook,
}

impl HeaderFit {
    fn new(room: f32, switch: f32, offline: Option<f32>) -> Self {
        use ComposeLook::{Icon, Keyed, Plain};
        for (words, compose) in [(true, Keyed), (false, Keyed), (false, Plain)] {
            let pill = offline.map_or(0., |wide| HEADER_GAP + if words { wide } else { 22. });
            if switch + pill + HEADER_GAP + compose.width() <= room {
                return HeaderFit { words, compose };
            }
        }
        HeaderFit {
            words: false,
            compose: Icon,
        }
    }
}

/// The width of `text` on one line.
fn text_width(text: &str, size: f32, family: &'static str, cx: &App) -> f32 {
    let system = cx.text_system();
    let font = system.resolve_font(&gpui::font(family));
    text.chars()
        .map(|c| {
            system
                .advance(font, px(size), c)
                .map_or(size * 0.6, |a| f32::from(a.width))
        })
        .sum()
}

/// How far down Inbox Zero's horizon is.
const HORIZON: f32 = 0.72;

/// The height of an option in the All / Unread switch.
const SEGMENT_H: f32 = 26.;
/// The height of a sidebar row (folding animates rows to and from it).
const NAV_ROW: f32 = 30.;
/// How long a sidebar section takes to fold or unfold.
const FOLD: Duration = Duration::from_millis(240);

fn view_key_for_section(key: &str) -> usize {
    match key {
        "split" => 0,
        "mailbox" => 1,
        _ => 2,
    }
}

/// The mailboxes that are an inbox, one account's or every account's: Done archives there,
/// Snooze and Move to take the thread out.
fn inbox_like(view: View) -> bool {
    matches!(
        view,
        View::Inbox
            | View::AllInboxes
            | View::Primary
            | View::Updates
            | View::Promotions
            | View::Social
    )
}

/// A stable number per mailbox, for element ids.
fn view_key(view: View) -> usize {
    match view {
        View::AllInboxes => 99,
        View::Inbox => 0,
        View::Primary => 1,
        View::Updates => 2,
        View::Promotions => 3,
        View::Social => 4,
        View::Starred => 5,
        View::Snoozed => 6,
        View::Drafts => 7,
        View::Sent => 8,
        View::Done => 9,
        View::Search => 10,
        View::AllMail => 11,
        View::Important => 12,
        View::Spam => 13,
        View::Trash => 14,
        View::Label(index) => 100 + index,
    }
}

fn nav(
    t: &Theme,
    name: &str,
    label: &str,
    count: Option<String>,
    selected: bool,
    tint: Option<Hsla>,
) -> Div {
    let hover = t.hover;
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(10.))
        .h(px(NAV_ROW))
        .flex_none()
        .rounded(px(NAV_RADIUS))
        .cursor_pointer()
        .when(selected, |d| {
            d.relative()
                .font_weight(FontWeight::MEDIUM)
                .child(active_pill(t))
        })
        .when(!selected, |d| d.hover(move |s| s.bg(hover)))
        .child(icon(
            name,
            16.,
            tint.unwrap_or(if selected { t.text } else { t.text2 }),
        ))
        .child(
            div()
                .flex_1()
                .text_color(if selected { t.text } else { t.text2 })
                .child(label.to_string()),
        )
        // Not violet, nor is the icon: on glass over a bright desktop it cannot be seen.
        .children(count.map(|n| mono(n, if selected { t.text } else { t.text3 })))
}

/// What is behind the sidebar's open row.
fn active_pill(t: &Theme) -> impl IntoElement {
    crate::erase::glass(NAV_RADIUS, t.sidebar, t.glass, t.glass_opacity)
}

fn card(t: &Theme) -> Div {
    div()
        .bg(t.hover)
        .border_1()
        .border_color(t.border)
        .rounded(px(12.))
}

// ---------------------------------------------------------------- panes

impl Nimbra {
    /// Sidebar toggle, back, forward and reload, sitting beside the traffic lights.
    /// Drawn above the panes so it stays put when the sidebar is hidden.
    fn window_controls(
        &self,
        t: &Theme,
        x: f32,
        list_shown: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let hover = t.hover;
        let control = |name: &str, enabled: bool| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(28.))
                .rounded(px(7.))
                .when(enabled, |d| d.cursor_pointer().hover(move |s| s.bg(hover)))
                .child(icon(name, 17., if enabled { t.text2 } else { t.faint }))
        };
        div()
            .absolute()
            .top(px(11.))
            .left(px(x + 84.))
            .flex()
            .items_center()
            .gap(px(4.))
            .child(
                control("sidebar", true)
                    .id("toggle-sidebar")
                    .press(t)
                    // Highlighted while the sidebar is hidden, like the list button.
                    .when(!self.sidebar_open, |d| d.bg(t.selected))
                    .tooltip(tip(t.dark, "Show or hide sidebar", "⌘B"))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            )
            .child(div().w(px(6.)))
            .child(
                control("back", self.can_go_back())
                    .id("go-back")
                    .press(t)
                    .tooltip(tip(t.dark, "Back", "⌘["))
                    .on_click(cx.listener(|this, _, _, cx| this.go_back(cx))),
            )
            .child(
                control("next", self.can_go_forward())
                    .id("go-forward")
                    .press(t)
                    .tooltip(tip(t.dark, "Forward", "⌘]"))
                    .on_click(cx.listener(|this, _, _, cx| this.go_forward(cx))),
            )
            .child(div().w(px(6.)))
            .child(
                div()
                    .id("reload")
                    .tooltip(tip(
                        t.dark,
                        if self.offline {
                            self.offline_label().into()
                        } else {
                            SharedString::from("Check for mail")
                        },
                        "⌘R",
                    ))
                    .relative()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(28.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx)))
                    .child(if self.shows_checking() {
                        // Keeps turning for as long as a mailbox request is in flight.
                        icon("reload", 17., t.violet)
                            .with_animation(
                                "reload-loop",
                                Animation::new(Duration::from_millis(900)).repeat(),
                                |svg, delta| {
                                    svg.with_transformation(Transformation::rotate(percentage(
                                        delta,
                                    )))
                                },
                            )
                            .into_any_element()
                    } else if self.reload_gen == 0 {
                        icon("reload", 17., t.text2).into_any_element()
                    } else {
                        // One full turn each time mail is checked.
                        icon("reload", 17., t.text2)
                            .with_animation(
                                ("reload-spin", self.reload_gen),
                                Animation::new(Duration::from_millis(700))
                                    .with_easing(ease_out_quint()),
                                |svg, delta| {
                                    svg.with_transformation(Transformation::rotate(percentage(
                                        delta,
                                    )))
                                },
                            )
                            .into_any_element()
                    })
                    // The list's header has the Offline pill; without a list, a dot here.
                    .when(self.offline && !list_shown, |d| {
                        d.child(
                            div()
                                .absolute()
                                .top(px(3.))
                                .right(px(3.))
                                .size(px(6.))
                                .rounded_full()
                                .bg(t.amber),
                        )
                    }),
            )
    }

    /// "Offline", and how much is waiting to go out.
    fn offline_label(&self) -> String {
        match self.outbox.changes.len() + self.waiting_mail().count() {
            0 => "Offline".to_string(),
            waiting => format!("Offline · {waiting} waiting"),
        }
    }

    /// Without room for the words it is only the dot, and the tooltip says the rest.
    fn offline_pill(&self, t: &Theme, words: bool) -> impl IntoElement {
        div()
            .id("offline")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .py(px(2.))
            .rounded_full()
            .bg(t.amber.opacity(0.14))
            .border_1()
            .border_color(t.amber.opacity(0.4))
            .h(px(22.))
            .when(!words, |d| d.w(px(22.)).justify_center())
            .tooltip(tip(
                t.dark,
                if words {
                    SharedString::from("Changes are kept and sent when the connection is back")
                } else {
                    self.offline_label().into()
                },
                "",
            ))
            .child(div().size(px(6.)).flex_none().rounded_full().bg(t.amber))
            .when(words, |d| d.child(meta(self.offline_label(), t.text2)))
    }

    fn sidebar(&self, t: &Theme, floating: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let left = self.visible().len();
        let unread = self
            .visible()
            .iter()
            .filter(|i| self.mails[**i].unread)
            .count();
        let bars = [9., 14., 8., 17., 12., 20., 15.];
        // The other mailboxes are not connected yet, so their counts only show in the demo.
        let demo = self.is_demo();
        let fake = |n: &str| demo.then(|| n.to_string());
        // Real counts from Gmail: unread in the inbox for mailboxes and labels (archived
        // "Done" threads left out), total for Drafts.
        let unread_in = |id: &str| {
            self.counts
                .get(id)
                .map(|(unread, _)| *unread)
                .filter(|n| *n > 0)
                .map(|n| n.to_string())
        };
        let drafts = self
            .counts
            .get("DRAFT")
            .map(|(_, total)| *total)
            .filter(|n| *n > 0)
            .map(|n| n.to_string());
        let split_toggle = self.section_toggle(t, "split", "SPLIT INBOX", cx);
        // Several accounts: All inboxes, and each account on its own.
        let many = !demo && self.settings.accounts.len() >= 2;
        let unread_of = |id: &str| -> u32 {
            if id == self.account_id {
                self.counts.get("INBOX").map_or(0, |(unread, _)| *unread)
            } else {
                self.account_unread.get(id).copied().unwrap_or(0)
            }
        };
        let all_unread: u32 = self
            .settings
            .accounts
            .iter()
            .map(|a| unread_of(&a.id))
            .sum();
        let accounts_toggle = many.then(|| self.section_toggle(t, "accounts", "ACCOUNTS", cx));
        let account_rows: Vec<AnyElement> = if many {
            self.settings
                .accounts
                .iter()
                .enumerate()
                .filter_map(|(i, account)| {
                    let shown = account.id == self.account_id && self.mailbox != View::AllInboxes;
                    let id = account.id.clone();
                    let initial = account
                        .email
                        .chars()
                        .next()
                        .map(|c| c.to_uppercase().to_string())
                        .unwrap_or_default();
                    let count = Some(unread_of(&account.id))
                        .filter(|n| *n > 0)
                        .map(|n| n.to_string());
                    let hover = t.hover;
                    let row = div()
                        .id(("account-row", i))
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(10.))
                        .px(px(10.))
                        .h(px(30.))
                        .rounded(px(NAV_RADIUS))
                        .cursor_pointer()
                        .when(shown, |d| d.relative().child(active_pill(t)))
                        .when(!shown, |d| d.hover(move |s| s.bg(hover)))
                        .press(t)
                        .child(avatar(crate::data::tone_for(&account.email), &initial, 18.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .text_color(if shown { t.text } else { t.text2 })
                                .child(account.email.clone()),
                        )
                        .children(count.map(|n| meta(n, if shown { t.text } else { t.text3 })))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if id == this.account_id {
                                this.select_mailbox(View::Inbox, cx);
                            } else {
                                this.switch_account(id.clone(), cx);
                            }
                        }))
                        .into_any_element();
                    self.fold_row("accounts", shown, 950 + i, row)
                })
                .collect()
        } else {
            Vec::new()
        };
        let mailbox_toggle = self.section_toggle(t, "mailbox", "MAILBOX", cx);
        let labels_toggle = self.section_toggle(t, "labels", "LABELS", cx);
        let mailbox = |view: View, icon_name: &'static str, count: Option<String>| {
            // A folded section still shows the mailbox that is open.
            let section = match view {
                View::Inbox | View::Primary | View::Updates | View::Promotions | View::Social => {
                    "split"
                }
                View::Label(_) => "labels",
                _ => "mailbox",
            };
            let row = nav(
                t,
                icon_name,
                &self.title(view),
                count,
                self.mailbox == view,
                None,
            )
            .id(("mailbox", view_key(view)))
            .press(t)
            .on_click(cx.listener(move |this, _, _, cx| this.select_mailbox(view, cx)))
            .into_any_element();
            self.fold_row(section, self.mailbox == view, view_key(view), row)
        };

        let scrolled =
            div()
                .id("sidebar")
                .track_scroll(&self.sidebar_scroll)
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                // With several accounts it can be taller than the window: it scrolls.
                .overflow_y_scroll()
                .px(px(12.))
                .pb(px(10.))
                .child(
                    div()
                        .h(px(SIDEBAR_CLEAR))
                        .flex_none()
                        .on_mouse_down(MouseButton::Left, drag_or_zoom),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .pl(px(10.))
                        .pr(px(6.))
                        .py(px(6.))
                        .rounded(px(9.))
                        .bg(t.hover)
                        .border_1()
                        .border_color(t.border)
                        .cursor_pointer()
                        .id("jump")
                        .cursor(gpui::CursorStyle::IBeam)
                        .on_click(cx.listener(|this, _, _, cx| this.focus_search(cx)))
                        .when(self.mailbox == View::Search, |d| d.border_color(t.mint))
                        .child(icon("search", 16., t.text3))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_color(if self.query.is_empty() {
                                    t.text3
                                } else {
                                    t.text
                                })
                                .child(if self.query.is_empty() {
                                    SharedString::from("Search mail")
                                } else {
                                    short(&self.query, 24)
                                }),
                        )
                        .child(kbd(t, "/")),
                )
                .when(many, |d| {
                    d.child(
                        nav(
                            t,
                            "mail",
                            "All inboxes",
                            Some(all_unread).filter(|n| *n > 0).map(|n| n.to_string()),
                            self.mailbox == View::AllInboxes,
                            None,
                        )
                        .mt(px(SEARCH_GAP))
                        .id("all-inboxes")
                        .press(t)
                        .on_click(
                            cx.listener(|this, _, _, cx| this.select_mailbox(View::AllInboxes, cx)),
                        ),
                    )
                    .children(accounts_toggle)
                    .children(account_rows)
                })
                .child(split_toggle)
                .children(mailbox(
                    View::Inbox,
                    "inbox",
                    if demo {
                        (unread > 0 && self.mailbox == View::Inbox).then(|| unread.to_string())
                    } else {
                        unread_in("INBOX")
                    },
                ))
                .when(demo, |d| {
                    d.children(self.fold_row(
                        "split",
                        false,
                        910,
                        nav(t, "users", "Team", fake("4"), false, None).into_any_element(),
                    ))
                    .children(self.fold_row(
                        "split",
                        false,
                        911,
                        nav(t, "calendar", "Calendar", fake("2"), false, None).into_any_element(),
                    ))
                    .children(self.fold_row(
                        "split",
                        false,
                        912,
                        nav(t, "news", "Newsletters", fake("31"), false, None).into_any_element(),
                    ))
                    .children(self.fold_row(
                        "split",
                        false,
                        913,
                        nav(t, "zap", "Other", fake("23"), false, None).into_any_element(),
                    ))
                })
                // With real mail the split inbox is Gmail's own categories.
                .when(!demo, |d| {
                    d.children(mailbox(
                        View::Primary,
                        "users",
                        unread_in("CATEGORY_PERSONAL"),
                    ))
                    .children(mailbox(
                        View::Updates,
                        "bell",
                        unread_in("CATEGORY_UPDATES"),
                    ))
                    .children(mailbox(
                        View::Promotions,
                        "tag",
                        unread_in("CATEGORY_PROMOTIONS"),
                    ))
                    .children(mailbox(
                        View::Social,
                        "news",
                        unread_in("CATEGORY_SOCIAL"),
                    ))
                })
                .child(mailbox_toggle)
                .children(mailbox(View::Starred, "star", None))
                .when(!demo, |d| {
                    d.children(mailbox(View::Important, "pulse", None))
                })
                .children(mailbox(View::Snoozed, "clock", fake("3")))
                .children(self.fold_row(
                    "mailbox",
                    false,
                    900,
                    nav(t, "bell", "Reminders", fake("2"), false, None).into_any_element(),
                ))
                .children(mailbox(
                    View::Drafts,
                    "draft",
                    if demo { fake("1") } else { drafts },
                ))
                .children(mailbox(View::Sent, "send", None))
                .children(mailbox(
                    View::Done,
                    "done",
                    (!self.history.is_empty()).then(|| self.history.len().to_string()),
                ))
                .when(!demo, |d| {
                    d.children(mailbox(View::AllMail, "archive", None))
                        .children(mailbox(View::Spam, "ban", None))
                        .children(mailbox(View::Trash, "trash", None))
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(labels_toggle.flex_1())
                        .when(!demo && self.gmail.is_some(), |d| {
                            let hover = t.hover;
                            d.child(
                                div()
                                    .id("new-label-sidebar")
                                    .mt(px(10.))
                                    .mr(px(4.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(22.))
                                    .rounded(px(6.))
                                    .cursor_pointer()
                                    .text_color(t.text3)
                                    .hover(move |s| s.bg(hover))
                                    .tooltip(tip(t.dark, "New label", ""))
                                    .child("+")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_label_sheet(LabelSheet::New { apply: false }, cx)
                                    })),
                            )
                        }),
                )
                .when(demo, |d| {
                    d.children(self.fold_row(
                        "labels",
                        false,
                        920,
                        nav(t, "tag", "Product", None, false, Some(t.violet)).into_any_element(),
                    ))
                    .children(self.fold_row(
                        "labels",
                        false,
                        921,
                        nav(t, "tag", "Design", None, false, Some(t.rose)).into_any_element(),
                    ))
                    .children(self.fold_row(
                        "labels",
                        false,
                        922,
                        nav(t, "tag", "Finance", None, false, Some(t.amber)).into_any_element(),
                    ))
                    .children(self.fold_row(
                        "labels",
                        false,
                        923,
                        nav(t, "tag", "Travel", None, false, Some(t.sky)).into_any_element(),
                    ))
                })
                .when(!demo, |d| {
                    let tints = [t.violet, t.rose, t.amber, t.sky, t.mint];
                    d.children(self.labels.iter().enumerate().take(40).filter_map(
                        |(index, label)| {
                            let view = View::Label(index);
                            let row = nav(
                                t,
                                "tag",
                                &short(&label.name, 22),
                                unread_in(&label.id),
                                self.mailbox == view,
                                Some(tints[index % tints.len()]),
                            )
                            .id(("mailbox", view_key(view)))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.select_mailbox(view, cx)),
                            )
                            // Right-click: rename or delete the label.
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(
                                    move |this, event: &gpui::MouseDownEvent, window, cx| {
                                        this.menu_at = (
                                            f32::from(event.position.x),
                                            f32::from(event.position.y),
                                        );
                                        this.menu_dismissed = None;
                                        this.toggle_menu(Menu::Label(index), window, cx);
                                    },
                                ),
                            )
                            .press(t)
                            .into_any_element();
                            self.fold_row("labels", self.mailbox == view, view_key(view), row)
                        },
                    ))
                })
                .child(div().flex_1())
                // The streak card is sample content, so it only shows with the demo data.
                .when(demo, |d| {
                    d.child(
                        card(t)
                            .flex()
                            .flex_col()
                            .gap(px(10.))
                            .p(px(12.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(icon("zap", 16., t.amber))
                                    .child(
                                        div()
                                            .flex_1()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child("Inbox zero streak"),
                                    )
                                    .child(mono("7d", t.text3)),
                            )
                            .child(div().flex().items_end().gap(px(5.)).h(px(22.)).children(
                                bars.map(|h: f32| {
                                    div().flex_1().h(px(h)).rounded(px(2.)).bg(gradient(
                                        180.,
                                        c(0xff7ab6, 1.),
                                        c(0x8b7cff, 1.),
                                    ))
                                }),
                            ))
                            .child(meta(
                                if left == 0 {
                                    "Done at 10:52 · 28 min faster".to_string()
                                } else {
                                    format!("{left} left · usually done by 11:20")
                                },
                                t.text3,
                            )),
                    )
                });

        // Settings stays at the bottom, whatever is scrolled above it.
        let footer = div()
            .flex_none()
            .px(px(12.))
            .py(px(8.))
            .border_t_1()
            .border_color(t.border_strong)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .p(px(6.))
                    .rounded(px(10.))
                    .child(
                        div()
                            .id("account")
                            .flex()
                            .flex_1()
                            .min_w(px(0.))
                            .items_center()
                            .gap(px(10.))
                            .cursor_pointer()
                            .tooltip(tip(t.dark, "Settings", "⌘,"))
                            .on_click(cx.listener(|this, _, _, cx| this.open_settings(cx)))
                            .child(me_avatar(self.me_photo.as_ref(), &self.me_initials, 28.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .whitespace_nowrap()
                                            .overflow_hidden()
                                            .child("Settings"),
                                    )
                                    .child(meta(
                                        match (demo, self.settings.accounts.len()) {
                                            (true, _) => "Work · 3 accounts".to_string(),
                                            // The one account's address: nothing else here shows it.
                                            (false, 0 | 1) if !self.account.is_empty() => {
                                                short(&self.account, 22).to_string()
                                            }
                                            (false, 0 | 1) => "Gmail".to_string(),
                                            (false, n) => format!("Gmail · {n} accounts"),
                                        },
                                        t.text3,
                                    )),
                            ),
                    )
                    .child(
                        icon_button(t, if self.dark { "sun" } else { "moon" })
                            .id("theme")
                            .press(t)
                            .tooltip(tip(t.dark, "Light or dark theme", "⌘⇧L"))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_theme(cx))),
                    ),
            );

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(SIDEBAR_W))
            .h_full()
            // Floating, the panel around it has the colour and the outline. The border keeps
            // its 1pt, so that docking moves nothing inside.
            .border_r_1()
            .border_color(if floating {
                gpui::transparent_black()
            } else {
                t.border
            })
            .when(!floating, |d| d.bg(t.sidebar))
            // Space for the native traffic lights; dragging here moves the window.
            .child(
                div()
                    .h(px(SIDEBAR_TOP))
                    .flex_none()
                    .on_mouse_down(MouseButton::Left, drag_or_zoom),
            )
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.))
                    .child(scrolled)
                    .child(crate::erase::fade(
                        SIDEBAR_FADE,
                        SIDEBAR_CLEAR,
                        t.sidebar,
                        self.sidebar_scroll.clone(),
                    )),
            )
            .child(footer)
    }

    fn row(&self, t: &Theme, i: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let m = &self.mails[i];
        let picked = self.picked.contains(&i);
        let selected = i == self.selected && self.picked.len() < 2;
        let hover = t.hover;
        // How many characters fit on a row at the list's current width (the text is cut
        // with an ellipsis by character count, so this follows the resize handle).
        let line = ((self.list_width - 104.) / 7.4).max(8.) as usize;
        // On a narrow list the tag gives way so the sender's name stays readable.
        let tag = m
            .tag
            .clone()
            .or_else(|| {
                let accents = [
                    Accent::Violet,
                    Accent::Rose,
                    Accent::Amber,
                    Accent::Sky,
                    Accent::Mint,
                ];
                self.thread_labels(m)
                    .into_iter()
                    .next()
                    .map(|(index, label)| (short(&label.name, 18), accents[index % accents.len()]))
            })
            .filter(|_| line >= 36);
        let tag_chars = tag
            .as_ref()
            .map_or(0, |(label, _)| label.chars().count() + 6);
        let extras =
            13 + tag_chars + if m.thread.is_some() { 3 } else { 0 } + if m.starred { 3 } else { 0 };
        let sender_max = line.saturating_sub(extras).max(6);
        let subject = short(m.subject.split(',').next().unwrap_or(&m.subject), line);
        let snippet_max = line.saturating_sub(subject.chars().count() + 2);
        let snippet = if snippet_max >= 8 {
            short(&m.snippet, snippet_max)
        } else {
            SharedString::default()
        };

        let (key, account) = self.thread_key(m);
        div()
            .id(SharedString::from(format!("row:{account}:{key}")))
            .press(t)
            .relative()
            .flex()
            .gap(px(ROW_PAD))
            .px(px(ROW_PAD))
            .py(px(12.))
            .rounded(px(12.))
            .cursor_pointer()
            .when(selected || picked, |d| d.bg(t.selected))
            // Drawn over the row rather than as its border, which would take up space and
            // make the row (and those below it) jump.
            .when(picked, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(px(12.))
                        .border_1()
                        .border_color(t.violet.opacity(0.5)),
                )
            })
            .when(!selected && !picked, |d| d.hover(move |s| s.bg(hover)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    this.open_row_menu(i, event.position, window, cx)
                }),
            )
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                this.click_row(i, event.modifiers(), event.click_count(), cx)
            }))
            // The unread dot sits in the padding, so the avatar has the same space either side.
            .when(m.unread, |d| {
                d.child(
                    div()
                        .absolute()
                        .left(px((ROW_PAD - 6.) / 2.))
                        .top(px(23.))
                        .size(px(6.))
                        .rounded_full()
                        .bg(t.violet),
                )
            })
            .child({
                // In All inboxes, a small badge in the account's colour says whose it is.
                let badge = m
                    .account
                    .as_deref()
                    .filter(|_| self.mailbox == View::AllInboxes)
                    .and_then(|id| self.settings.accounts.iter().find(|a| a.id == id))
                    .map(|a| crate::data::tone_for(&a.email));
                div()
                    .relative()
                    .flex_none()
                    .child(avatar(m.tone, &m.initials, 28.))
                    .children(badge.map(|tone| {
                        div()
                            .absolute()
                            .right(px(-2.))
                            .bottom(px(-2.))
                            .size(px(11.))
                            .rounded_full()
                            .bg(tone.gradient())
                            .border_2()
                            .border_color(t.surface)
                    }))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.))
                    .gap(px(3.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .h(px(22.))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .min_w(px(0.))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .child(short(&m.sender, sender_max)),
                            )
                            .children(m.thread.clone().map(|n| mono(n, t.text3)))
                            .when(m.starred, |d| d.child(icon("star", 13., t.amber)))
                            .children(tag.map(|(label, a)| pill(t, &label, t.accent(a))))
                            .child(div().flex_1())
                            .when(!selected, |d| d.child(mono(m.time.clone(), t.text3)))
                            // Quick actions on the open row: done, snooze, remind me.
                            .when(selected, |d| {
                                let quick = |id: &'static str, name: &'static str| {
                                    let (hover, bright) = (t.hover, t.text);
                                    div()
                                        .id((id, i))
                                        .group("quick")
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(px(24.))
                                        .rounded(px(6.))
                                        .cursor_pointer()
                                        .hover(move |s| s.bg(hover))
                                        .press(t)
                                        .child(
                                            icon(name, 15., t.text2)
                                                .group_hover("quick", move |s| {
                                                    s.text_color(bright)
                                                }),
                                        )
                                };
                                let archivable = self.done_change().is_some();
                                d.child(
                                    div()
                                        .flex()
                                        .gap(px(2.))
                                        .when(archivable, |d| {
                                            d.child(
                                                quick("quick-done", "done")
                                                    .tooltip(tip(
                                                        t.dark,
                                                        if matches!(
                                                            self.mailbox,
                                                            View::Done
                                                                | View::Spam
                                                                | View::Trash
                                                                | View::Snoozed
                                                        ) {
                                                            "Move to inbox"
                                                        } else {
                                                            "Mark done"
                                                        },
                                                        "E",
                                                    ))
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            cx.stop_propagation();
                                                            this.selected = i;
                                                            this.mark_done(cx);
                                                        },
                                                    )),
                                            )
                                        })
                                        .child(
                                            quick("quick-snooze", "clock")
                                                .tooltip(tip(t.dark, "Snooze", "H"))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    this.selected = i;
                                                    this.picked.clear();
                                                    this.snooze_choices = Self::snooze_choices();
                                                    this.palette_sel = 0;
                                                    this.open_palette();
                                                    cx.notify();
                                                })),
                                        )
                                        .child(
                                            quick("quick-remind", "bell")
                                                .tooltip(tip(
                                                    t.dark,
                                                    "Remind me if no reply in 3 days",
                                                    "",
                                                ))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    this.selected = i;
                                                    this.picked.clear();
                                                    if this.is_demo() {
                                                        this.open_palette();
                                                        this.palette_sel = 0;
                                                        cx.notify();
                                                        return;
                                                    }
                                                    if let Some(choice) = Self::snooze_choices()
                                                        .into_iter()
                                                        .find(|c| c.no_reply)
                                                    {
                                                        this.snooze(choice, cx);
                                                    }
                                                })),
                                        ),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .child(
                                // Long subjects shrink with an ellipsis instead of
                                // spilling out of the row; the snippet takes what is left.
                                div()
                                    .min_w(px(0.))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(if selected { t.text } else { t.text2 })
                                    .child(subject),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .text_color(t.text3)
                                    .child(snippet),
                            ),
                    ),
            )
    }

    fn list(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = self.visible();
        let unread = visible.iter().filter(|i| self.mails[**i].unread).count();
        let group = |label: &str| {
            div()
                .pl(px(14.))
                .pt(px(14.))
                .pb(px(6.))
                .child(overline(label.to_string(), t.faint))
        };
        let today: Vec<usize> = visible
            .iter()
            .copied()
            .filter(|i| !self.mails[*i].yesterday)
            .collect();
        let earlier: Vec<usize> = visible
            .iter()
            .copied()
            .filter(|i| self.mails[*i].yesterday)
            .collect();

        // Each option has a fixed width, so the highlight behind them can slide from one to
        // the other (and nothing shifts when it does).
        let unread_text = unread.to_string();
        let widths = [44., 70. + 7. * unread_text.len() as f32];
        // The switch: its options, the 2pt between them, its padding and its border.
        let mut switch_w = widths[0] + widths[1] + 2. + 8.;
        if self.is_demo() {
            switch_w += 2. + 112.;
        }
        // Dot, gap, padding and border around the pill's words.
        let offline_w = self
            .offline
            .then(|| 30. + text_width(&self.offline_label(), 12., crate::theme::SANS, cx));
        let fit = HeaderFit::new(
            self.list_width - 1. - LIST_LEFT - LIST_RIGHT,
            switch_w,
            offline_w,
        );
        let segment = |label: &str, count: Option<(&str, Hsla)>, active: bool, width: f32| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .w(px(width))
                .h(px(SEGMENT_H))
                .rounded(px(7.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if active { t.text } else { t.text3 })
                .child(label.to_string())
                .children(count.map(|(n, color)| mono(n.to_string(), color)))
        };
        // The highlight: where it is, and where it was before the last switch.
        let spot = |unread: bool| {
            if unread {
                (widths[0] + 2., widths[1])
            } else {
                (0., widths[0])
            }
        };
        let (to_x, to_w) = spot(self.unread_only);
        let (from_x, from_w) = spot(!self.unread_only);
        let pill = div()
            .absolute()
            .top(px(3.))
            .h(px(SEGMENT_H))
            .rounded(px(7.))
            .bg(t.raised)
            .border_1()
            .border_color(t.border)
            .shadow(vec![shadow(
                c(0x000000, if t.dark { 0.25 } else { 0.08 }),
                1.,
                3.,
                0.,
            )]);
        // Only right after a switch (not again each time the list is drawn anew).
        let switching = self
            .filter_switched
            .is_some_and(|at| at.elapsed() < Duration::from_millis(260));
        let pill = if self.filter_gen == 0 || !switching {
            pill.left(px(3. + to_x)).w(px(to_w)).into_any_element()
        } else {
            pill.with_animation(
                ("filter-pill", self.filter_gen),
                Animation::new(Duration::from_millis(260)).with_easing(ease_out_quint()),
                move |d, delta| {
                    d.left(px(3. + from_x + (to_x - from_x) * delta))
                        .w(px(from_w + (to_w - from_w) * delta))
                },
            )
            .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .flex_none()
            .relative()
            .w(px(self.list_width))
            .h_full()
            .bg(t.surface)
            .border_r_1()
            .border_color(t.border)
            // Drag the right edge to resize the list; double-click it to hide the list.
            .child(
                div()
                    .id("list-resize")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(px(-3.))
                    .w(px(7.))
                    .cursor(gpui::CursorStyle::ResizeLeftRight)
                    .hover(|s| s.bg(gpui::white().opacity(0.06)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            if event.click_count == 2 {
                                this.toggle_list(cx);
                            } else {
                                this.list_grab =
                                    Some((f32::from(event.position.x), this.list_width));
                            }
                        }),
                    ),
            )
            .child(
                // One row on the top line (y = 25): the filters, then Offline and Compose.
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(HEADER_GAP))
                    .h(px(TOP_BAR - 1.))
                    .pl(px(LIST_LEFT))
                    .pr(px(LIST_RIGHT))
                    .on_mouse_down(MouseButton::Left, drag_or_zoom)
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_none()
                            .gap(px(2.))
                            .p(px(3.))
                            .rounded(px(10.))
                            .bg(t.hover)
                            .border_1()
                            .border_color(t.border)
                            .child(pill)
                            .child(
                                segment("All", None, !self.unread_only, widths[0])
                                    .id("filter-all")
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_unread_only(false, cx)
                                    })),
                            )
                            .child(
                                segment(
                                    "Unread",
                                    Some((&unread_text, t.faint)),
                                    self.unread_only,
                                    widths[1],
                                )
                                .id("filter-unread")
                                .cursor_pointer()
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.set_unread_only(true, cx)),
                                ),
                            )
                            .when(self.is_demo(), |d| {
                                d.child(segment("Needs reply", Some(("2", t.rose)), false, 112.))
                            }),
                    )
                    .child(div().flex_1().min_w(px(0.)).mx(px(-HEADER_GAP / 2.)))
                    .when(self.offline, |d| d.child(self.offline_pill(t, fit.words)))
                    .child(
                        match fit.compose {
                            ComposeLook::Keyed => {
                                button(t, ButtonKind::Secondary, "Compose", Some("C"))
                            }
                            ComposeLook::Plain => button(t, ButtonKind::Secondary, "Compose", None),
                            ComposeLook::Icon => icon_button(t, "draft")
                                .size(px(34.))
                                .bg(t.hover)
                                .border_1()
                                .border_color(t.border_strong),
                        }
                        .id("compose")
                        .h(px(34.))
                        .press_soft()
                        .tooltip(tip(t.dark, "New message", "C"))
                        .on_click(
                            cx.listener(|this, _, _, cx| this.open_compose(ComposeKind::New, cx)),
                        ),
                    ),
            )
            // While mail is checked, a thin bar glides along the top of the list. Its 2px
            // strip is always there, so nothing below it moves.
            .child(
                div()
                    .relative()
                    .flex_none()
                    .h(px(2.))
                    .mx(px(18.))
                    .rounded_full()
                    .overflow_hidden()
                    .when(self.bar_shown || self.bar_fade.is_some(), |d| {
                        d.child(
                            div()
                                .absolute()
                                .inset_0()
                                .bg(t.border)
                                .child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .h_full()
                                        .w(relative(0.45))
                                        .rounded_full()
                                        .bg(gradient(90., c(0x8b7cff, 0.), c(0xff7ab6, 1.)))
                                        .with_animation(
                                            "checking-mail",
                                            Animation::new(Duration::from_millis(1150))
                                                .repeat()
                                                .with_easing(gpui::ease_in_out),
                                            |bar, delta| bar.left(relative(-0.45 + 1.45 * delta)),
                                        ),
                                )
                                .map(|strip| match self.bar_fade {
                                    // Done: fade the whole strip out.
                                    Some((_, n)) => strip
                                        .with_animation(
                                            ("checking-mail-fade", n),
                                            Animation::new(Duration::from_millis(450))
                                                .with_easing(ease_out_quint()),
                                            |strip, delta| strip.opacity(1. - delta),
                                        )
                                        .into_any_element(),
                                    None => strip.into_any_element(),
                                }),
                        )
                    }),
            )
            .child(
                div()
                    .id("rows")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap(px(2.))
                    .px(px(10.))
                    .pb(px(10.))
                    .overflow_y_scroll()
                    .track_scroll(&self.list_scroll)
                    .on_scroll_wheel(cx.listener(|this, event, _, cx| this.pull_list(event, cx)))
                    .children(self.pull_indicator(t))
                    .when(!today.is_empty(), |d| d.child(group("TODAY")))
                    .children(today.into_iter().map(|i| self.row(t, i, cx)))
                    .when(!earlier.is_empty(), |d| d.child(group("EARLIER")))
                    .children(earlier.into_iter().map(|i| self.row(t, i, cx)))
                    // More is on its way: placeholder rows that breathe while the next
                    // page loads (it starts on its own as the end of the list comes near).
                    .when(self.more().is_some(), |d| {
                        d.children((0..2usize).map(|n| {
                            let bar = |w: f32| div().h(px(9.)).w(px(w)).rounded_full().bg(t.hover);
                            div()
                                .flex()
                                .flex_none()
                                .gap(px(ROW_PAD))
                                .px(px(ROW_PAD))
                                .py(px(14.))
                                .child(div().size(px(28.)).flex_none().rounded_full().bg(t.hover))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(8.))
                                        .pt(px(3.))
                                        .child(bar(if n == 0 { 120. } else { 96. }))
                                        .child(bar(if n == 0 { 220. } else { 180. })),
                                )
                                .with_animation(
                                    ("more-loading", n),
                                    Animation::new(Duration::from_millis(1400)).repeat(),
                                    move |row, delta| {
                                        let wave = (std::f32::consts::TAU
                                            * (delta - n as f32 * 0.15))
                                            .cos();
                                        row.opacity(0.55 + 0.35 * wave)
                                    },
                                )
                        }))
                    }),
            )
    }

    /// One message drawn natively from its blocks. `base` keeps element ids unique when
    /// several messages are on screen.
    fn message_body(
        &self,
        t: &Theme,
        blocks: &[Block],
        base: usize,
        cx: &mut Context<Self>,
    ) -> Div {
        let sources = rich::image_sources(blocks);
        let pending = sources.iter().any(|src| !self.image_requests.contains(src));
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .text_size(px(self.settings.reader_text.metrics().0))
            .line_height(px(self.settings.reader_text.metrics().1))
            .text_color(t.text2)
            .when(pending, |d| {
                d.child(
                    card(t)
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .pl(px(12.))
                        .pr(px(6.))
                        .py(px(6.))
                        .text_size(px(13.))
                        .line_height(px(18.))
                        .child(icon("image", 16., t.text3))
                        .child(
                            div()
                                .flex_1()
                                .text_color(t.text3)
                                .child("Remote images are hidden to protect your privacy."),
                        )
                        .child(
                            button(t, ButtonKind::Secondary, "Load images", None)
                                .id(("load-images", base))
                                .press_soft()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.load_images(sources.clone(), cx)
                                })),
                        ),
                )
            })
            .children(
                blocks
                    .iter()
                    .enumerate()
                    .map(|(i, block)| self.block(t, block, base + i)),
            )
    }

    fn block(&self, t: &Theme, block: &Block, id: usize) -> AnyElement {
        let text = |color: Hsla| {
            let highlights: Vec<_> = block
                .spans
                .iter()
                .map(|span| {
                    let style = HighlightStyle {
                        font_weight: span.bold.then_some(FontWeight::SEMIBOLD),
                        font_style: span.italic.then_some(FontStyle::Italic),
                        color: span
                            .link
                            .is_some()
                            .then_some(t.violet)
                            .or(span.bold.then_some(color)),
                        background_color: span.code.then_some(t.hover),
                        underline: span.link.is_some().then(|| gpui::UnderlineStyle {
                            thickness: px(1.),
                            color: Some(t.violet.opacity(0.5)),
                            wavy: false,
                        }),
                        ..Default::default()
                    };
                    (span.range.clone(), style)
                })
                .collect();
            let styled = StyledText::new(block.text.clone()).with_highlights(highlights);
            let links: Vec<(std::ops::Range<usize>, String)> = block
                .spans
                .iter()
                .filter_map(|span| Some((span.range.clone(), span.link.clone()?)))
                .collect();
            if links.is_empty() {
                return styled.into_any_element();
            }
            // Links open in the system browser; only http, https and mailto get this far.
            let (ranges, urls): (Vec<_>, Vec<_>) = links.into_iter().unzip();
            gpui::InteractiveText::new(("link-text", id), styled)
                .on_click(ranges, move |index, _, cx| cx.open_url(&urls[index]))
                .into_any_element()
        };

        let element = match &block.kind {
            Kind::Paragraph => div().child(text(t.text)),
            Kind::Heading(level) => {
                let size = match level {
                    1 => 24.,
                    2 => 20.,
                    _ => 17.,
                };
                div()
                    .pt(px(6.))
                    .text_size(px(size))
                    .line_height(px(size * 1.35))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text)
                    .child(text(t.text))
            }
            Kind::Item { marker, depth } => div()
                .flex()
                .gap(px(10.))
                .pl(px(4. + 20. * *depth as f32))
                .child(
                    div()
                        .flex_none()
                        .min_w(px(14.))
                        .text_color(t.text3)
                        .child(marker.clone()),
                )
                .child(div().flex_1().min_w(px(0.)).child(text(t.text))),
            Kind::Code => div()
                .px(px(14.))
                .py(px(10.))
                .rounded(px(10.))
                .bg(t.hover)
                .border_1()
                .border_color(t.border)
                .font_family(crate::theme::MONO)
                .text_size(px(12.5))
                .line_height(px(20.))
                .child(text(t.text)),
            Kind::Rule => div().h(px(1.)).my(px(6.)).bg(t.border),
            Kind::Image { src, alt } => match self.images.get(src) {
                Some(image) => {
                    div().child(gpui::img(image.clone()).max_w(px(560.)).rounded(px(10.)))
                }
                None => div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(13.))
                    .line_height(px(18.))
                    .text_color(t.text3)
                    .child(icon("image", 15., t.text3))
                    .child(if alt.is_empty() {
                        "Image".to_string()
                    } else {
                        format!("Image: {alt}")
                    }),
            },
        };
        // Quoted text gets a bar on the left, one step per nesting level.
        if block.quote > 0 {
            div()
                .ml(px(14. * (block.quote.min(4) - 1) as f32))
                .pl(px(14.))
                .border_l_2()
                .border_color(t.border_strong)
                .text_color(t.text3)
                .child(element)
                .into_any_element()
        } else {
            element.into_any_element()
        }
    }

    fn brief(&self, t: &Theme, m: &Mail) -> impl IntoElement {
        let todo = |text: &str, due: &str| {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    div()
                        .size(px(16.))
                        .flex_none()
                        .rounded(px(5.))
                        .border_1()
                        .border_color(t.border_strong),
                )
                .child(div().flex_1().child(text.to_string()))
                .child(mono(due.to_string(), t.amber))
        };
        // GPUI has no gradient strokes, so the border is a 1px gradient frame around the card.
        div()
            .p(px(1.))
            .rounded(px(14.))
            .bg(gradient(100., c(0x8b7cff, 0.75), c(0xffb86b, 0.3)))
            .shadow(vec![shadow(c(0x8b7cff, 0.16), 12., 40., -12.)])
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .px(px(20.))
                    .pt(px(16.))
                    .pb(px(18.))
                    .rounded(px(13.))
                    .bg(t.raised)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(icon("sparkles", 16., t.mint))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("Brief"))
                            .child(
                                meta(
                                    match &m.thread {
                                        Some(n) => format!("from {n} messages"),
                                        None => "from 1 message".to_string(),
                                    },
                                    t.text3,
                                )
                                .flex_1(),
                            )
                            .child(kbd(t, "⌘"))
                            .child(kbd(t, "J")),
                    )
                    .child(
                        div()
                            .text_size(px(15.))
                            .line_height(px(26.))
                            .text_color(t.text2)
                            .child(m.brief.clone()),
                    )
                    .when(m.rich, |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(8.))
                                .child(todo("Confirm pricing tiers on slide 14", "Fri"))
                                .child(todo("Sign off on the press embargo date", "Fri")),
                        )
                    }),
            )
    }

    fn reading(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &self.mails[self.selected];
        // The subject is broken into lines by hand (see `balance_title`), for the width the
        // reader gives it.
        let pane_w = self.frame.map_or(0., |f| f32::from(f.size.width)) - self.pane_left();
        let column = if self.focus_restore.is_some() {
            pane_w.min(980.)
        } else {
            pane_w
        };
        let visible = self.visible();
        let pos = visible
            .iter()
            .position(|i| *i == self.selected)
            .unwrap_or(0)
            + 1;
        // Only switch panes when this thread really has HTML to show.
        // Drafts always use the reader, so a click on the text can open the editor.
        let is_draft = self.mailbox == View::Drafts && !self.is_demo();
        let original = self.original && m.html.is_some() && !is_draft;
        // Until the message has been fetched we cannot know whether it has an original
        // (HTML) version, so show a loader, not the reader view that would flash by first.
        let loading = self.original && !is_draft && !m.loaded;
        let divider = || div().h(px(1.)).bg(t.border);

        let earlier = |tone: Tone, initials: &str, name: &str, snippet: &str, when: &str| {
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .py(px(12.))
                .border_t_1()
                .border_color(t.border)
                .child(avatar(tone, initials, 24.))
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.text2)
                        .child(name.to_string()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_color(t.text3)
                        .child(short(snippet, 110)),
                )
                .child(mono(when.to_string(), t.faint))
        };

        let attachment = |name: &str, info: &str, from: u32, to: u32| {
            card(t)
                .flex()
                .items_center()
                .gap(px(10.))
                .pl(px(8.))
                .pr(px(14.))
                .py(px(8.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(30.))
                        .rounded(px(8.))
                        .bg(gradient(135., c(from, 1.), c(to, 1.)))
                        .child(icon("file", 16., gpui::white())),
                )
                .child(
                    div()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .child(name.to_string()),
                        )
                        .child(meta(info.to_string(), t.text3)),
                )
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .bg(t.surface)
            .relative()
            // While a panel slides, the email keeps one width and the extra is cut off here.
            .overflow_hidden()
            // Toolbar
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(2.))
                    // Nothing to clear: the window controls leave with the sidebar.
                    .pl(px(20.))
                    .pr(px(20.))
                    .h(px(TOP_BAR))
                    .border_b_1()
                    .border_color(t.border)
                    .on_mouse_down(MouseButton::Left, drag_or_zoom)
                    .child(
                        icon_button(t, "list")
                            .id("toggle-list")
                            .press(t)
                            .tooltip(tip(t.dark, "Show or hide message list", "⌘⇧B"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Show or hide message list", "⌘⇧B", cx)
                            }))
                            .when(!self.list_open, |d| d.bg(t.selected))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_list(cx))),
                    )
                    .child(div().w(px(6.)))
                    .when(is_draft, |d| {
                        d.child(
                            button(t, ButtonKind::Primary, "Edit draft", None)
                                .id("edit-draft")
                                .press_soft()
                                .tooltip(tip(t.dark, "Open this draft in the editor", ""))
                                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                    this.set_hint(*hovered, "Open this draft in the editor", "", cx)
                                }))
                                .on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.open_draft(this.selected, cx)
                                    }),
                                ),
                        )
                        .child(div().w(px(8.)))
                    })
                    .child(div().w(px(6.)))
                    .child(
                        icon_button(t, "done")
                            .id("mark-done")
                            .press(t)
                            .tooltip(tip(t.dark, "Mark done (archive)", "E"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Mark done (archive)", "E", cx)
                            }))
                            .on_click(cx.listener(|this, _, _, cx| this.mark_done(cx))),
                    )
                    .child(kbd(t, "E"))
                    .child(div().w(px(6.)))
                    .child(
                        icon_button(t, "clock")
                            .id("snooze")
                            .press(t)
                            .tooltip(tip(t.dark, "Snooze", "H"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Snooze", "H", cx)
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_palette();
                                cx.notify();
                            })),
                    )
                    .child(
                        icon_button(t, "star")
                            .id("star")
                            .press(t)
                            .tooltip(tip(t.dark, "Star", "S"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Star", "S", cx)
                            }))
                            .when(m.starred, |d| d.bg(t.selected))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_star(cx))),
                    )
                    .child(
                        icon_button(t, "mail")
                            .id("toggle-unread")
                            .press(t)
                            .tooltip(tip(t.dark, "Mark as unread or read", "U"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Mark as unread or read", "U", cx)
                            }))
                            // Lit while the open thread is unread.
                            .when(m.unread, |d| d.bg(t.selected))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_unread(cx))),
                    )
                    .child(
                        icon_button(t, "tag")
                            .id("label")
                            .press(t)
                            .tooltip(tip(t.dark, "Add a label", ""))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Add a label", "", cx)
                            }))
                            .when(self.menu == Some(Menu::Labels), |d| d.bg(t.selected))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_menu(Menu::Labels, window, cx)
                            })),
                    )
                    .child(
                        icon_button(t, "folder")
                            .id("move-to")
                            .press(t)
                            .tooltip(tip(t.dark, "Move to", ""))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Move to", "", cx)
                            }))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_menu(Menu::MoveTo, window, cx)
                            })),
                    )
                    .child(
                        icon_button(t, "trash")
                            .id("trash")
                            .press(t)
                            .tooltip(tip(t.dark, "Move to trash", "⌫"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Move to trash", "⌫", cx)
                            }))
                            .on_click(cx.listener(|this, _, _, cx| this.trash(cx))),
                    )
                    .child(div().w(px(1.)).h(px(16.)).mx(px(6.)).bg(t.border_strong))
                    .child(
                        icon_button(t, "reply")
                            .id("reply")
                            .press(t)
                            .tooltip(tip(t.dark, "Reply", "R"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Reply", "R", cx)
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_compose(ComposeKind::Reply, cx)
                            })),
                    )
                    .child(
                        icon_button(t, "reply-all")
                            .id("reply-all")
                            .press(t)
                            .tooltip(tip(t.dark, "Reply all", "A"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Reply all", "A", cx)
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_compose(ComposeKind::ReplyAll, cx)
                            })),
                    )
                    .child(
                        icon_button(t, "forward")
                            .id("forward")
                            .press(t)
                            .tooltip(tip(t.dark, "Forward", "F"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Forward", "F", cx)
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_compose(ComposeKind::Forward, cx)
                            })),
                    )
                    .child(div().w(px(1.)).h(px(16.)).mx(px(6.)).bg(t.border_strong))
                    .child(
                        button(
                            t,
                            ButtonKind::Ghost,
                            if original {
                                "Reader view"
                            } else {
                                "Show original"
                            },
                            Some("V"),
                        )
                        .id("original")
                        .press_soft()
                        .tooltip(tip(t.dark, "Switch between original and reader view", "V"))
                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                            this.set_hint(
                                *hovered,
                                "Switch between original and reader view",
                                "V",
                                cx,
                            )
                        }))
                        .when(original, |d| d.bg(t.selected))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_original(cx))),
                    )
                    .child(div().flex_1())
                    .child(mono(format!("{pos} / {}", visible.len()), t.text3).mr(px(6.)))
                    .child(
                        icon_button(t, "up")
                            .id("prev")
                            .press(t)
                            .tooltip(tip(t.dark, "Previous thread", "K"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Previous thread", "K", cx)
                            }))
                            .on_click(cx.listener(|this, _, _, cx| this.step(-1, cx))),
                    )
                    .child(
                        icon_button(t, "down")
                            .id("next")
                            .press(t)
                            .tooltip(tip(t.dark, "Next thread", "J"))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "Next thread", "J", cx)
                            }))
                            .on_click(cx.listener(|this, _, _, cx| this.step(1, cx))),
                    )
                    .child(
                        icon_button(t, "more")
                            .id("more")
                            .press(t)
                            .tooltip(tip(t.dark, "More actions", ""))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "More actions", "", cx)
                            }))
                            .when(self.menu == Some(Menu::More), |d| d.bg(t.selected))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_menu(Menu::More, window, cx)
                            })),
                    ),
            )
            // Conversation (native reader)
            .when(loading, |d| d.child(self.loading_pane(t, m, cx.entity())))
            .when(!original && !loading, |d| {
                d.child(
                    div()
                        .id("conversation")
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        // Full view centres the column it keeps narrow.
                        .when(self.focus_restore.is_some(), |d| {
                            d.flex().flex_col().items_center()
                        })
                        // Clicking the text of a thread counts as opening it: it is marked read.
                        // (Not when it was just marked unread: that stays until you move on.)
                        .when(!is_draft, |d| {
                            d.on_click(cx.listener(|this, _, _, cx| {
                                let id = this.mails.get(this.selected).and_then(|m| m.id.clone());
                                if id.is_none() || this.kept_unread != id {
                                    this.open_selected(cx)
                                }
                            }))
                        })
                        // Clicking a draft's text opens it in the editor.
                        .when(is_draft, |d| {
                            d.cursor_pointer().on_click(
                                cx.listener(|this, _, _, cx| this.open_draft(this.selected, cx)),
                            )
                        })
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(22.))
                                .px(px(64.))
                                // Full view keeps lines a comfortable length.
                                .when(self.focus_restore.is_some(), |d| d.w_full().max_w(px(980.)))
                                .pt(px(40.))
                                .pb(px(24.))
                                .child(
                                    div()
                                        .flex()
                                        .gap(px(6.))
                                        .when(m.rich, |d| d.child(pill(t, "Product", t.violet)))
                                        .children(
                                            m.tag
                                                .clone()
                                                .map(|(label, a)| pill(t, &label, t.accent(a))),
                                        )
                                        .when(m.rich, |d| d.child(pill(t, "Due Fri", t.amber)))
                                        .when(!m.rich && m.tag.is_none(), |d| {
                                            d.child(pill(
                                                t,
                                                &self.inbox_name(),
                                                t.accent(Accent::Violet),
                                            ))
                                        }),
                                )
                                .child(self.subject_row(t, m, 38., column - 128., cx))
                                .child(self.thread_banners(t, m, cx))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(10.))
                                        .child(
                                            div()
                                                .relative()
                                                .flex_none()
                                                .h(px(28.))
                                                .w(px(if m.rich { 68. } else { 48. }))
                                                .children(
                                                    [
                                                        Some((m.tone, m.initials.clone())),
                                                        m.rich.then(|| {
                                                            (Tone::Mint, SharedString::from("JR"))
                                                        }),
                                                        Some((
                                                            Tone::Graphite,
                                                            self.me_initials.clone(),
                                                        )),
                                                    ]
                                                    .into_iter()
                                                    .flatten()
                                                    .enumerate()
                                                    .map(|(n, (tone, initials))| {
                                                        avatar(tone, &initials, 28.)
                                                            .absolute()
                                                            .top_0()
                                                            .left(px(n as f32 * 20.))
                                                    }),
                                                ),
                                        )
                                        .child(meta(m.participants.clone(), t.text3).flex_1())
                                        .child(mono(m.time.clone(), t.text3)),
                                )
                                .when(!m.brief.is_empty(), |d| d.child(self.brief(t, m)))
                                .when(!m.earlier.is_empty(), |d| {
                                    d.child(div().children(m.earlier.iter().enumerate().map(
                                        |(k, e)| {
                                            // Click a collapsed message to read it in full.
                                            div()
                                                .child(
                                                    earlier(
                                                        e.tone,
                                                        &e.initials,
                                                        &e.sender,
                                                        if e.expanded { "" } else { &e.snippet },
                                                        &e.when,
                                                    )
                                                    .id(("earlier", k))
                                                    .cursor_pointer()
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        let selected = this.selected;
                                                        if let Some(e) = this
                                                            .mails
                                                            .get_mut(selected)
                                                            .and_then(|m| m.earlier.get_mut(k))
                                                        {
                                                            e.expanded = !e.expanded;
                                                            cx.notify();
                                                        }
                                                    })),
                                                )
                                                .when(e.expanded, |d| {
                                                    d.child(
                                                        div()
                                                            .flex()
                                                            .flex_col()
                                                            .gap(px(12.))
                                                            .pb(px(14.))
                                                            .child(self.message_header(
                                                                t,
                                                                m,
                                                                k + 1,
                                                                true,
                                                                cx,
                                                            ))
                                                            .child(
                                                                self.message_body(
                                                                    t,
                                                                    &e.blocks,
                                                                    (k + 1) * 1000,
                                                                    cx,
                                                                )
                                                                .pl(px(40.)),
                                                            )
                                                            .when(
                                                                !e.header.attachments.is_empty(),
                                                                |d| {
                                                                    d.child(
                                                                        self.attachment_row(
                                                                            t,
                                                                            &e.header.attachments,
                                                                            (k + 1) * 100,
                                                                            m.account.as_deref(),
                                                                            cx,
                                                                        )
                                                                        .pl(px(40.)),
                                                                    )
                                                                },
                                                            ),
                                                    )
                                                })
                                        },
                                    )))
                                })
                                .child(divider())
                                .child(self.message_header(t, m, 0, true, cx))
                                .child(
                                    self.message_body(t, &m.body, 0, cx)
                                        .pl(px(40.))
                                        .when(!m.loaded, |d| {
                                            d.child(self.loading_line(t, m, cx.entity()))
                                        }),
                                )
                                .when(!m.header.attachments.is_empty(), |d| {
                                    d.child(
                                        self.attachment_row(
                                            t,
                                            &m.header.attachments,
                                            0,
                                            m.account.as_deref(),
                                            cx,
                                        )
                                        .pl(px(40.)),
                                    )
                                })
                                .when(m.rich, |d| {
                                    d.child(
                                        div()
                                            .flex()
                                            .gap(px(10.))
                                            .pl(px(40.))
                                            .child(attachment(
                                                "Q4-Launch-Plan.pdf",
                                                "PDF · 4.2 MB",
                                                0xff7ab6,
                                                0xff4f7b,
                                            ))
                                            .child(attachment(
                                                "Pricing-tiers.fig",
                                                "Figma · 1.1 MB",
                                                0x8b7cff,
                                                0x5b47e0,
                                            )),
                                    )
                                }),
                        ),
                )
            })
            .when(original, |d| d.child(self.original_pane(t, m, cx)))
            .children(self.menu_view(t, cx))
    }

    /// Shown in place of the mail panes while signed out, signing in, loading or failed.
    fn status_screen(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, message, action): (&str, SharedString, Option<&str>) = match &self.status {
            Status::SignedOut => (
                "Connect your inbox",
                "Nimbra opens Google in your browser to sign in. It can read and archive \
                 your mail, and send what you write. It cannot permanently delete anything."
                    .into(),
                Some("Connect Gmail"),
            ),
            Status::SigningIn => (
                "Waiting for Google",
                "Finish signing in in your browser, then come back here.".into(),
                None,
            ),
            Status::Failed(reason) => ("Something went wrong", reason.clone(), Some("Try again")),
            _ => (
                "Fetching your inbox",
                "This takes a few seconds.".into(),
                None,
            ),
        };
        div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(14.))
            .bg(t.surface)
            .child(serif(title, 56., t.text))
            .child(
                div()
                    .max_w(px(460.))
                    .text_center()
                    .text_size(px(15.))
                    .line_height(px(24.))
                    .text_color(t.text2)
                    .child(message),
            )
            .children(action.map(|label| {
                div().pt(px(10.)).child(
                    button(t, ButtonKind::Primary, label, None)
                        .id("status-action")
                        .press_soft()
                        .on_click(cx.listener(|this, _, _, cx| {
                            if matches!(this.status, Status::Failed(_)) {
                                this.try_again(cx);
                            } else {
                                this.sign_in(cx);
                            }
                        })),
                )
            }))
    }

    /// Shown while an email's content is on its way, or could not be fetched.
    fn loading_pane(&self, t: &Theme, mail: &Mail, owner: gpui::Entity<Self>) -> impl IntoElement {
        let pane = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .items_center()
            .justify_center()
            .gap(px(12.));
        if self.body_failed.contains(&self.thread_key(mail)) {
            return pane.child(self.loading_line(t, mail, owner));
        }
        pane.child(icon("reload", 22., t.violet).with_animation(
            "message-loading",
            Animation::new(Duration::from_millis(900)).repeat(),
            |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
        ))
        .child(meta("Loading message…", t.text3))
    }

    /// The same in words, with a way to try again when it failed.
    fn loading_line(&self, t: &Theme, mail: &Mail, owner: gpui::Entity<Self>) -> Div {
        let line = div().flex().items_center().gap(px(12.)).text_color(t.text3);
        let key = self.thread_key(mail);
        if !self.body_failed.contains(&key) {
            return line.child("Loading message…");
        }
        line.child("This message could not be loaded.").child(
            button(t, ButtonKind::Secondary, "Try again", None)
                .id("message-retry")
                .press_soft()
                .on_click(move |_, _, cx| {
                    let (id, account) = key.clone();
                    owner.update(cx, |n, cx| n.load_body(id, account, cx));
                }),
        )
    }

    /// One message's header: sender, a one-line recipient summary that expands into every
    /// field (From, To, Cc, Reply-To, Date), and, for earlier messages, reply actions.
    fn message_header(
        &self,
        t: &Theme,
        mail: &Mail,
        index: usize,
        actions: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let (header, sender, tone, initials, time) = match index {
            0 => (
                &mail.header,
                mail.sender.clone(),
                mail.tone,
                mail.initials.clone(),
                mail.time.clone(),
            ),
            k => {
                let e = &mail.earlier[k - 1];
                (
                    &e.header,
                    e.sender.clone(),
                    e.tone,
                    e.initials.clone(),
                    e.when.clone(),
                )
            }
        };
        let thread = mail.id.clone().unwrap_or_default();
        let open = self.details.as_ref() == Some(&(thread.clone(), index));
        let card_thread = thread.clone();
        let me = self.account.to_string();
        let label = |p: &gmail::Person| {
            if p.address.eq_ignore_ascii_case(&me) {
                "me".to_string()
            } else {
                p.label().to_string()
            }
        };
        let recipients: Vec<String> = header.to.iter().chain(&header.cc).map(label).collect();
        let summary = match recipients.len() {
            0 if header.from.address.is_empty() => "to you".to_string(),
            0 => "to undisclosed recipients".to_string(),
            1..=3 => format!("to {}", recipients.join(", ")),
            n => format!("to {}, {} and {} more", recipients[0], recipients[1], n - 2),
        };
        let person = |p: &gmail::Person| {
            if p.name.is_empty() {
                p.address.clone()
            } else {
                format!("{} <{}>", p.name, p.address)
            }
        };
        let source_index = index;
        let reply_button =
            |name: &'static str, tip_text: &'static str, kind: ComposeKind, id: usize| {
                icon_button(t, name)
                    .id(("message-action", id))
                    .press(t)
                    .tooltip(tip(t.dark, tip_text, ""))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let source =
                            this.mails
                                .get(this.selected)
                                .and_then(|mail| match source_index {
                                    0 => Some(Source::latest(mail)),
                                    k => mail.earlier.get(k - 1).map(|e| Source::earlier(mail, e)),
                                });
                        this.open_compose_from(kind, source, cx);
                    }))
            };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(avatar(tone, &initials, 28.))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(div().font_weight(FontWeight::SEMIBOLD).child(sender))
                                    .when(!header.from.address.is_empty(), |d| {
                                        d.child(meta(header.from.address.clone(), t.text3))
                                    }),
                            )
                            .child(
                                div()
                                    .id(("recipients", index))
                                    .flex()
                                    .items_center()
                                    .gap(px(4.))
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let key = (thread.clone(), index);
                                        this.details = if this.details.as_ref() == Some(&key) {
                                            None
                                        } else {
                                            Some(key)
                                        };
                                        cx.notify();
                                    }))
                                    .child(meta(short(&summary, 90), t.text3))
                                    .child(icon(if open { "up" } else { "down" }, 12., t.text3)),
                            ),
                    )
                    .when(actions, |d| {
                        d.child(reply_button(
                            "reply",
                            "Reply to this message",
                            ComposeKind::Reply,
                            index * 3,
                        ))
                        .child(reply_button(
                            "reply-all",
                            "Reply all to this message",
                            ComposeKind::ReplyAll,
                            index * 3 + 1,
                        ))
                        .child(reply_button(
                            "forward",
                            "Forward this message",
                            ComposeKind::Forward,
                            index * 3 + 2,
                        ))
                    })
                    .child(mono(time, t.text3)),
            )
            .when(open && !header.from.address.is_empty(), |d| {
                d.child(self.details_card(t, header, &card_thread, index, &person, cx))
            })
    }

    /// A message's full header (From, To, Cc, Reply-To, Date). Each address copies on
    /// click; "Copy all" copies the lot as text.
    fn details_card(
        &self,
        t: &Theme,
        header: &crate::data::Header,
        thread: &str,
        index: usize,
        person: &dyn Fn(&gmail::Person) -> String,
        cx: &mut Context<Self>,
    ) -> Div {
        let me = self.account.to_string();
        let copied = |key: &str| {
            self.copied
                .as_ref()
                .is_some_and(|(k, at)| k == key && at.elapsed().as_millis() < 1400)
        };
        let base = format!("{thread}/{index}");
        let mut serial = index * 1000;
        let mut chip = |p: &gmail::Person, cx: &mut Context<Self>| {
            serial += 1;
            let key = format!("{base}/{serial}");
            let done = copied(&key);
            let address = p.address.clone();
            let mine = p.address.eq_ignore_ascii_case(&me);
            let hover = t.hover;
            div()
                .id(("detail-person", serial))
                .group("detail-person")
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(px(6.))
                .max_w_full()
                .px(px(7.))
                .py(px(3.))
                .rounded(px(7.))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .press(t)
                .when(!p.name.is_empty(), |d| {
                    d.child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.text)
                            .child(p.name.clone()),
                    )
                })
                .child(
                    div()
                        .min_w(px(0.))
                        .text_color(if p.name.is_empty() { t.text } else { t.text3 })
                        .child(p.address.clone()),
                )
                .when(mine, |d| d.child(pill(t, "You", t.violet)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .when(done, |d| {
                            d.child(icon("done", 12., t.mint))
                                .child(div().text_color(t.mint).child("Copied"))
                        })
                        .when(!done, |d| {
                            d.invisible()
                                .group_hover("detail-person", |s| s.visible())
                                .child(icon("copy", 12., t.text3))
                        }),
                )
                .tooltip(tip(t.dark, "Copy address", ""))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.copy_text(key.clone(), address.clone(), cx)
                }))
        };
        let row = |name: &str, value: AnyElement| {
            div()
                .flex()
                .items_start()
                .gap(px(8.))
                .child(
                    div()
                        .w(px(72.))
                        .flex_none()
                        .pt(px(3.))
                        .text_size(px(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.text3)
                        .child(name.to_string()),
                )
                .child(div().flex_1().min_w(px(0.)).child(value))
        };
        let people = |list: &[gmail::Person],
                      chip: &mut dyn FnMut(&gmail::Person, &mut Context<Self>) -> Stateful<Div>,
                      cx: &mut Context<Self>| {
            div()
                .flex()
                .flex_wrap()
                .gap(px(2.))
                .ml(px(-7.))
                .children(list.iter().map(|p| chip(p, cx)).collect::<Vec<_>>())
                .into_any_element()
        };
        // The whole header as text, for "Copy all".
        let mut all = vec![format!("From: {}", person(&header.from))];
        let join = |list: &[gmail::Person]| list.iter().map(person).collect::<Vec<_>>().join(", ");
        if !header.to.is_empty() {
            all.push(format!("To: {}", join(&header.to)));
        }
        if !header.cc.is_empty() {
            all.push(format!("Cc: {}", join(&header.cc)));
        }
        if let Some(p) = &header.reply_to {
            all.push(format!("Reply-To: {}", person(p)));
        }
        let date = (header.date_ms > 0).then(|| long_date(header.date_ms));
        if let Some(date) = &date {
            all.push(format!("Date: {date}"));
        }
        let all = all.join("\n");
        let all_key = format!("{base}/all");
        let all_done = copied(&all_key);
        let date_key = format!("{base}/date");
        let date_done = copied(&date_key);
        let hover = t.hover;
        let from = people(std::slice::from_ref(&header.from), &mut chip, cx);
        let to = (!header.to.is_empty()).then(|| people(&header.to, &mut chip, cx));
        let cc = (!header.cc.is_empty()).then(|| people(&header.cc, &mut chip, cx));
        let reply_to = header
            .reply_to
            .as_ref()
            .map(|p| people(std::slice::from_ref(p), &mut chip, cx));
        div()
            .ml(px(40.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .pl(px(14.))
            .pr(px(10.))
            .pt(px(10.))
            .pb(px(8.))
            .rounded(px(12.))
            .bg(t.raised)
            .border_1()
            .border_color(t.border)
            .shadow(vec![shadow(c(0x000000, 0.06), 1., 3., 0.)])
            .text_size(px(12.5))
            .line_height(px(18.))
            .child(row("From", from))
            .children(to.map(|v| row("To", v)))
            .children(cc.map(|v| row("Cc", v)))
            .children(reply_to.map(|v| row("Reply-To", v)))
            .children(date.map(|date| {
                let text = date.clone();
                row(
                    "Date",
                    div()
                        .id(("detail-date", index))
                        .group("detail-date")
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .ml(px(-7.))
                        .px(px(7.))
                        .py(px(3.))
                        .rounded(px(7.))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .press(t)
                        .child(div().text_color(t.text2).child(date))
                        .child(if date_done {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .child(icon("done", 12., t.mint))
                                .child(div().text_color(t.mint).child("Copied"))
                        } else {
                            div()
                                .invisible()
                                .group_hover("detail-date", |s| s.visible())
                                .child(icon("copy", 12., t.text3))
                        })
                        .tooltip(tip(t.dark, "Copy date", ""))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_text(date_key.clone(), text.clone(), cx)
                        }))
                        .into_any_element(),
                )
            }))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .pt(px(4.))
                    .mt(px(2.))
                    .border_t_1()
                    .border_color(t.border)
                    .child(
                        div()
                            .id(("detail-copy-all", index))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .px(px(8.))
                            .py(px(4.))
                            .rounded(px(7.))
                            .cursor_pointer()
                            .text_color(if all_done { t.mint } else { t.text2 })
                            .font_weight(FontWeight::MEDIUM)
                            .hover(move |s| s.bg(hover))
                            .press(t)
                            .child(icon(
                                if all_done { "done" } else { "copy" },
                                13.,
                                if all_done { t.mint } else { t.text2 },
                            ))
                            .child(if all_done { "Copied" } else { "Copy all" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.copy_text(all_key.clone(), all.clone(), cx)
                            })),
                    ),
            )
    }

    /// A message's attachments, each with Quick Look and Save.
    fn attachment_row(
        &self,
        t: &Theme,
        attachments: &[gmail::Attachment],
        base: usize,
        account: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .flex()
            .flex_wrap()
            .gap(px(10.))
            .children(attachments.iter().enumerate().map(|(i, att)| {
                let (preview, save) = (att.clone(), att.clone());
                let from = account.map(str::to_string);
                let save_from = from.clone();
                let kind = att
                    .name
                    .rsplit('.')
                    .next()
                    .filter(|e| *e != att.name)
                    .unwrap_or("file")
                    .to_uppercase();
                card(t)
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .pl(px(8.))
                    .pr(px(6.))
                    .py(px(6.))
                    .max_w(px(360.))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(30.))
                            .rounded(px(8.))
                            .bg(gradient(135., c(0x8b7cff, 1.), c(0x5b47e0, 1.)))
                            .child(icon("file", 16., gpui::white())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(short(&att.name, 34)),
                            )
                            .child(meta(
                                [kind, file_size(att.size)]
                                    .into_iter()
                                    .filter(|s| !s.is_empty())
                                    .collect::<Vec<_>>()
                                    .join(" · "),
                                t.text3,
                            )),
                    )
                    .child({
                        let busy = self.attachment_busy.get(&att.attachment_id) == Some(&false);
                        attachment_button(t, "eye", busy, false)
                            .id(("attachment-preview", base + i))
                            .press(t)
                            .tooltip(tip(t.dark, "Quick Look", ""))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.fetch_attachment(preview.clone(), from.as_deref(), false, cx)
                            }))
                    })
                    .child({
                        let busy = self.attachment_busy.get(&att.attachment_id) == Some(&true);
                        let saved = self.attachment_saved.contains_key(&att.attachment_id);
                        attachment_button(t, if saved { "done" } else { "down" }, busy, saved)
                            .id(("attachment-save", base + i))
                            .press(t)
                            .tooltip(tip(
                                t.dark,
                                if saved {
                                    "Saved to Downloads"
                                } else {
                                    "Save to Downloads"
                                },
                                "",
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.fetch_attachment(save.clone(), save_from.as_deref(), true, cx)
                            }))
                    })
            }))
    }

    /// The thread's labels, and the Trash and Spam notes, when they apply.
    fn thread_banners(&self, t: &Theme, mail: &Mail, cx: &mut Context<Self>) -> Div {
        // Labels are gathered from every message in the thread, so one deleted (or spam)
        // message would mark the whole conversation. Like Gmail, only a conversation that is
        // in that mailbox (or no longer in the inbox) gets the banner.
        let has = |l: &str| mail.labels.iter().any(|x| x == l);
        let inbox = has("INBOX");
        let key = message_window::key_of(mail);
        let account = self.account_of(mail).to_string();
        // The open mailbox says where a thread is only if the thread is in its list: a
        // message window can show one from another mailbox or account.
        let listed = self.row_of(&key, &account).is_some();
        let spam = (listed && self.mailbox == View::Spam) || (has("SPAM") && !inbox);
        let trash = (listed && self.mailbox == View::Trash) || (has("TRASH") && !inbox);
        let tints = [t.violet, t.rose, t.amber, t.sky, t.mint];
        // Label ids are each account's own: another account's would get the wrong names.
        let labels = if account == self.account_id {
            self.thread_labels(mail)
        } else {
            Vec::new()
        };
        let thread = (key, account);
        let (restore, not_spam) = (thread.clone(), thread.clone());
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .when(!labels.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .children(labels.into_iter().map(|(index, label)| {
                            let hover = t.hover;
                            let id = label.id.clone();
                            let (key, account) = thread.clone();
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .pl(px(8.))
                                .pr(px(3.))
                                .py(px(2.))
                                .rounded_full()
                                .bg(t.hover)
                                .border_1()
                                .border_color(t.border)
                                .child(
                                    div()
                                        .size(px(6.))
                                        .rounded_full()
                                        .bg(tints[index % tints.len()]),
                                )
                                .child(meta(short(&label.name, 30), t.text2))
                                .child(
                                    div()
                                        .id(("remove-label", index))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(px(16.))
                                        .rounded_full()
                                        .cursor_pointer()
                                        .text_color(t.text3)
                                        .hover(move |s| s.bg(hover))
                                        .tooltip(tip(t.dark, "Remove label", ""))
                                        .child("×")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(row) =
                                                this.row_to_act_on(&key, &account, cx)
                                            {
                                                this.remove_label(row, id.clone(), cx)
                                            }
                                        })),
                                )
                        })),
                )
            })
            .when(trash, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(14.))
                        .py(px(8.))
                        .rounded(px(10.))
                        .bg(t.hover)
                        .border_1()
                        .border_color(t.border)
                        .child(icon("trash", 15., t.text3))
                        .child(
                            div()
                                .flex_1()
                                .child("In Trash. Gmail deletes it for good after 30 days."),
                        )
                        .child(
                            button(t, ButtonKind::Secondary, "Restore", None)
                                .id("restore")
                                .press_soft()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let (key, account) = &restore;
                                    if let Some(row) = this.row_to_act_on(key, account, cx) {
                                        this.remove_thread(row, gmail::Change::Untrash, cx)
                                    }
                                })),
                        ),
                )
            })
            .when(spam, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(14.))
                        .py(px(8.))
                        .rounded(px(10.))
                        .bg(t.rose.opacity(0.12))
                        .border_1()
                        .border_color(t.rose.opacity(0.4))
                        .child(icon("bell", 15., t.rose))
                        .child(div().flex_1().child(
                            "This message is in Spam. Be careful with its links and attachments.",
                        ))
                        .child(
                            button(t, ButtonKind::Secondary, "Not spam", None)
                                .id("not-spam")
                                .press_soft()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let (key, account) = &not_spam;
                                    if let Some(row) = this.row_to_act_on(key, account, cx) {
                                        this.remove_thread(row, gmail::Change::NotSpam, cx)
                                    }
                                })),
                        ),
                )
            })
    }

    /// A thread's subject, with Unsubscribe at its top right when it is from a mailing list.
    /// `width` is what the two have together.
    fn subject_row(
        &self,
        t: &Theme,
        mail: &Mail,
        size: f32,
        width: f32,
        cx: &mut Context<Self>,
    ) -> Div {
        const BUTTON_W: f32 = 112.;
        const BUTTON_H: f32 = 32.;
        const GAP: f32 = 16.;
        let link = mail.header.unsubscribe.clone();
        let taken = if link.is_some() { BUTTON_W + GAP } else { 0. };
        let title = balance_title(&mail.subject, size, (width - taken).max(0.), cx);
        let account = mail.account.clone();
        div()
            .flex()
            .items_start()
            .gap(px(GAP))
            .child(serif(&title, size, t.text).flex_1().min_w(px(0.)))
            .when_some(link, |d, link| {
                // Where it goes is the sender's choice, so the tooltip says.
                let hint = match url::Url::parse(&link).ok() {
                    Some(url) if url.scheme() == "https" => format!(
                        "Opens {} in your browser to unsubscribe",
                        short(url.host_str().unwrap_or("the list’s page"), 48)
                    ),
                    _ => "Prepares an unsubscribe email for you to send".to_string(),
                };
                d.child(
                    button(t, ButtonKind::Secondary, "Unsubscribe", None)
                        .id("unsubscribe")
                        .justify_center()
                        .w(px(BUTTON_W))
                        .h(px(BUTTON_H))
                        // Centred on the subject's first line.
                        .mt(px(((size * 1.14).round() - BUTTON_H) / 2.))
                        .press_soft()
                        .tooltip(tip(t.dark, hint, ""))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let host = window
                                .root::<MessageWindow>()
                                .flatten()
                                .map(|_| window.window_handle());
                            this.unsubscribe(&link, account.clone(), host, cx)
                        })),
                )
            })
    }

    /// The area the web view covers, with a strip above it explaining what is blocked.
    fn original_pane(&self, t: &Theme, m: &Mail, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.html_view.clone();
        let pane_w = self.frame.map_or(0., |f| f32::from(f.size.width)) - self.pane_left();
        // The email is a native view: it is moved one frame ahead while a panel slides
        // (see `pane_lead`), so it keeps up with the rest of the window.
        let lead = self.pane_lead();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            // Subject, sender, recipients and attachments stay native above the email.
            .child(
                div()
                    .flex()
                    .flex_none()
                    .flex_col()
                    .gap(px(12.))
                    .px(px(28.))
                    .pt(px(16.))
                    .pb(px(12.))
                    .border_b_1()
                    .border_color(t.border)
                    .child(self.subject_row(t, m, 26., pane_w - 56., cx))
                    .child(self.thread_banners(t, m, cx))
                    .child(self.message_header(t, m, 0, false, cx)),
            )
            // The web view is a native layer; this element only reserves its space and
            // tells it where that space is each frame.
            .child(
                gpui::canvas(
                    move |bounds, _, _| {
                        if let Some(view) = &view {
                            view.set_bounds(
                                f32::from(bounds.origin.x) + lead,
                                f32::from(bounds.origin.y),
                                f32::from(bounds.size.width) - lead,
                                f32::from(bounds.size.height),
                            );
                        }
                    },
                    |_, _, _, _| {},
                )
                .flex_1()
                .w_full()
                .bg(if self.darkens_mail() {
                    t.surface
                } else {
                    gpui::white()
                }),
            )
            .when(!m.header.attachments.is_empty(), |d| {
                d.child(self.attachment_tray(t, &m.header.attachments, m.account.as_deref(), cx))
            })
    }

    /// The attachments of an email shown in its original formatting, in a tray under it.
    fn attachment_tray(
        &self,
        t: &Theme,
        attachments: &[gmail::Attachment],
        account: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Div {
        let total: u64 = attachments.iter().map(|a| a.size).sum();
        let count = attachments.len();
        div()
            .flex()
            .flex_none()
            .flex_col()
            .gap(px(10.))
            .px(px(28.))
            .pt(px(12.))
            .pb(px(14.))
            .bg(t.surface)
            .border_t_1()
            .border_color(t.border)
            .shadow(vec![shadow(c(0x000000, 0.12), -4., 12., 0.)])
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon("clip", 14., t.text3))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.text2)
                            .child(if count == 1 {
                                "1 attachment".to_string()
                            } else {
                                format!("{count} attachments")
                            }),
                    )
                    .child(meta(format!("· {}", file_size(total)), t.text3)),
            )
            .child(self.attachment_row(t, attachments, 0, account, cx))
    }

    /// The Labels or More menu, dropped under its toolbar button.
    fn menu_view(&self, t: &Theme, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let menu = self.menu?;
        let hover = t.hover;
        let item = move |id: usize, icon_name: &str, label: SharedString, tint: Hsla| {
            div()
                .id(("menu-item", id))
                .press(t)
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(10.))
                .py(px(7.))
                .rounded(px(8.))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(icon(icon_name, 16., tint))
                .child(div().flex_1().child(label))
        };
        let panel = div()
            .id("menu")
            .absolute()
            .top(px(50.))
            .flex()
            .flex_col()
            .min_w(px(220.))
            .p(px(6.))
            .rounded(px(12.))
            .bg(t.raised)
            .border_1()
            .border_color(t.border_strong)
            .shadow(vec![shadow(t.shadow, 14., 36., -6.)])
            .occlude()
            // Any click outside the menu closes it.
            .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                this.menu = None;
                this.menu_dismissed = Some((menu, std::time::Instant::now()));
                cx.notify();
            }));
        Some(match menu {
            // These menus only exist as pop-up windows.
            Menu::Row(_) | Menu::SendLater | Menu::MoveTo | Menu::Label(_) => {
                return None;
            }
            Menu::Labels => {
                let tints = [t.violet, t.rose, t.amber, t.sky, t.mint];
                panel
                    .left(px(150.))
                    .when(self.labels.is_empty(), |d| {
                        d.child(div().px(px(10.)).py(px(7.)).text_color(t.text3).child(
                            if self.is_demo() {
                                "Labels need a connected Gmail account."
                            } else {
                                "You have no labels in Gmail yet."
                            },
                        ))
                    })
                    .children(
                        self.labels
                            .iter()
                            .enumerate()
                            .take(12)
                            .map(|(index, label)| {
                                item(
                                    index,
                                    "tag",
                                    short(&label.name, 28),
                                    tints[index % tints.len()],
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| this.menu_choice(Menu::Labels, index, cx),
                                ))
                            }),
                    )
            }
            Menu::More => {
                panel
                    .right(px(20.))
                    .child(item(0, "inbox", "Mark as unread".into(), t.text2).on_click(
                        cx.listener(|this, _, _, cx| this.menu_choice(Menu::More, 0, cx)),
                    ))
                    .when(!self.is_demo(), |d| {
                        d.child(item(1, "link", "Open in Gmail".into(), t.text2).on_click(
                            cx.listener(|this, _, _, cx| this.menu_choice(Menu::More, 1, cx)),
                        ))
                    })
            }
        })
    }

    fn compose_view(
        &self,
        t: &Theme,
        viewport: (f32, f32),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some(composer) = &self.compose else {
            return div().into_any_element();
        };
        let full = composer.full;
        let (width, height) = composer.size;
        let (left, top) = composer
            .position
            .unwrap_or((((viewport.0 - width) / 2.).max(0.), 96.));
        // Full page keeps the text in a readable column instead of stretching it edge to edge.
        let column = |d: Div| {
            if full {
                d.w_full().max_w(px(860.))
            } else {
                d.w_full()
            }
        };
        let field = |label: &str,
                     input: &Entity<TextInput>,
                     lead: Vec<AnyElement>,
                     trailing: Option<AnyElement>,
                     below: Option<AnyElement>| {
            div()
                .flex()
                .justify_center()
                .border_b_1()
                .border_color(t.border)
                .child(
                    column(div())
                        .relative()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .px(px(20.))
                        .py(px(10.))
                        .child(
                            div()
                                .w(px(56.))
                                .flex_none()
                                .text_color(t.text3)
                                .child(label.to_string()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(px(6.))
                                .text_size(px(14.))
                                .children(lead)
                                .child(div().flex_1().min_w(px(120.)).child(input.clone())),
                        )
                        .children(trailing)
                        .children(below),
                )
        };
        // Each finished address is a pill with a button to take it off.
        let pills = |field: Field, cx: &mut Context<Self>| -> Vec<AnyElement> {
            let Some(slot) = Composer::slot(field) else {
                return Vec::new();
            };
            composer.chips[slot]
                .iter()
                .enumerate()
                .map(|(i, address)| {
                    let hover = t.hover;
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .pl(px(9.))
                        .pr(px(3.))
                        .py(px(1.))
                        .rounded_full()
                        .bg(t.hover)
                        .border_1()
                        .border_color(t.border_strong)
                        .text_size(px(13.))
                        .child(short(address, 34))
                        .child(
                            div()
                                .id(("remove-recipient", slot * 1000 + i))
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(16.))
                                .rounded_full()
                                .cursor_pointer()
                                .text_color(t.text3)
                                .hover(move |s| s.bg(hover))
                                .child("×")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(composer) = &mut this.compose
                                        && i < composer.chips[slot].len()
                                    {
                                        composer.chips[slot].remove(i);
                                        composer.sync_placeholder(field, cx);
                                        this.composer_changed(cx);
                                        cx.notify();
                                    }
                                })),
                        )
                        .into_any_element()
                })
                .collect()
        };
        // "Cc Bcc" opens the two extra recipient fields.
        let cc_toggle = (!composer.show_cc).then(|| {
            let (dim, bright) = (t.text3, t.text);
            div()
                .id("show-cc")
                .flex_none()
                .text_color(dim)
                .cursor_pointer()
                .hover(move |s| s.text_color(bright))
                .child("Cc Bcc")
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(composer) = &mut this.compose {
                        composer.show_cc = true;
                        let cc = composer.cc.clone();
                        this.pending_focus = Some(cc.read(cx).focus_handle(cx));
                        cx.notify();
                    }
                }))
                .into_any_element()
        });
        // Address suggestions, in a strip under the field. (Not floating over the fields
        // below: the message editor is a native view that would cover it.)
        let mut dropdown: Option<(Field, AnyElement)> = composer.suggest.as_ref().map(|s| {
            let list = div()
                .w(px(380.))
                .flex()
                .flex_col()
                .p(px(4.))
                .rounded(px(10.))
                .bg(t.raised)
                .border_1()
                .border_color(t.border_strong)
                .shadow(vec![shadow(t.shadow, 12., 32., -6.)])
                .children(s.people.iter().enumerate().map(|(i, p)| {
                    let hover = t.hover;
                    div()
                        .id(("suggest", i))
                        .press(t)
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(8.))
                        .py(px(6.))
                        .rounded(px(7.))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .when(i == s.selected, |d| d.bg(t.hover))
                        .child(avatar(
                            crate::data::tone_for(p.label()),
                            &crate::data::initials(p.label()),
                            24.,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .child(short(p.label(), 44))
                                .when(!p.name.is_empty(), |d| {
                                    d.child(meta(short(&p.address, 50), t.text3))
                                }),
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.accept_suggestion(Some(i), cx)),
                        )
                }));
            {
                let strip = div()
                    .flex()
                    .justify_center()
                    .border_b_1()
                    .border_color(t.border)
                    .child(column(div()).pl(px(84.)).pr(px(20.)).pb(px(8.)).child(list));
                (s.field, strip.into_any_element())
            }
        });
        let mut drop_for = |f: Field| -> Option<AnyElement> {
            if dropdown.as_ref().is_some_and(|(field, _)| *field == f) {
                dropdown.take().map(|(_, el)| el)
            } else {
                None
            }
        };
        // Attached files, each with a button to take it off.
        let chip = |id: usize, name: &str, size: u64| {
            let hover = t.hover;
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .pl(px(10.))
                .pr(px(4.))
                .py(px(4.))
                .rounded(px(8.))
                .bg(t.hover)
                .border_1()
                .border_color(t.border)
                .child(icon("clip", 13., t.text3))
                .child(short(name, 32))
                .child(meta(file_size(size), t.text3))
                .child(
                    div()
                        .id(("remove-file", id))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(18.))
                        .rounded(px(5.))
                        .cursor_pointer()
                        .text_color(t.text3)
                        .hover(move |s| s.bg(hover))
                        .child("×"),
                )
        };
        let chips: Vec<AnyElement> = composer
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| {
                chip(i, &f.name, f.bytes.len() as u64)
                    .id(("file-chip", i))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(composer) = &mut this.compose
                            && i < composer.files.len()
                        {
                            composer.files.remove(i);
                            this.composer_changed(cx);
                            cx.notify();
                        }
                    }))
                    .into_any_element()
            })
            .chain(composer.kept.iter().enumerate().map(|(i, att)| {
                chip(1000 + i, &att.name, att.size)
                    .id(("kept-chip", i))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(composer) = &mut this.compose
                            && i < composer.kept.len()
                        {
                            composer.kept.remove(i);
                            this.composer_changed(cx);
                            cx.notify();
                        }
                    }))
                    .into_any_element()
            }))
            .collect();
        let accent = t.violet;
        let card = div()
            .id("compose-card")
            .flex()
            .flex_col()
            .bg(t.raised)
            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _, cx| {
                this.attach_files(paths.paths().to_vec(), cx)
            }))
            .drag_over::<gpui::ExternalPaths>(move |style, _, _, _| style.border_color(accent))
            .when(full, |d| d.size_full().bg(t.surface))
            .when(!full, |d| {
                d.absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(width))
                    .h(px(height))
                    .rounded(px(18.))
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![
                        shadow(c(0x8b7cff, 0.18), 0., 120., 10.),
                        shadow(c(0x000000, if t.dark { 0.6 } else { 0.2 }), 30., 80., -10.),
                    ])
            })
            .child(
                div()
                    .flex()
                    .justify_center()
                    // In full page the header also clears the traffic lights and is the
                    // strip you drag or double-click.
                    .when(full, |d| {
                        d.pt(px(36.)).on_mouse_down(MouseButton::Left, drag_or_zoom)
                    })
                    .when(!full, |d| {
                        d.cursor(gpui::CursorStyle::OpenHand).on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, event, _, cx| {
                                this.grab_composer(Grab::Move, event, cx)
                            }),
                        )
                    })
                    .child(
                        column(div())
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .px(px(20.))
                            .pt(px(16.))
                            .pb(px(8.))
                            .child(serif(
                                if composer.draft_id.is_some() {
                                    "Draft"
                                } else if composer.reply.is_some() {
                                    "Reply"
                                } else {
                                    "New message"
                                },
                                if full { 38. } else { 26. },
                                t.text,
                            ))
                            // Written from another account than the one shown (All inboxes).
                            .children(
                                composer
                                    .account
                                    .as_deref()
                                    .filter(|a| *a != self.account_id)
                                    .and_then(|id| {
                                        self.settings.accounts.iter().find(|a| a.id == id)
                                    })
                                    .map(|a| meta(format!("from {}", a.email), t.text3)),
                            )
                            .child(div().flex_1())
                            .when(!self.compose_detached, |d| {
                                d.child(
                                    icon_button(t, "detach")
                                        .id("compose-detach")
                                        .press(t)
                                        .tooltip(tip(t.dark, "Open in its own window", ""))
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(|this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.detach_compose(cx)
                                            }),
                                        ),
                                )
                            })
                            .when(!self.compose_detached, |d| {
                                d.child(
                                    icon_button(t, if full { "shrink" } else { "expand" })
                                        .id("compose-size")
                                        .press(t)
                                        .tooltip(tip(t.dark, "Full page or floating", "⌘⇧F"))
                                        // On press, as the close button (see there).
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(|this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.toggle_full_compose(cx)
                                            }),
                                        ),
                                )
                            })
                            // Close (asks about the draft if there is one). A window of
                            // its own has its close button in the title bar instead.
                            .when(!self.compose_detached, |d| {
                                d.child(
                                    icon_button(t, "x")
                                        .id("compose-close")
                                        .press(t)
                                        .tooltip(tip(t.dark, "Close", "esc"))
                                        // On press, not release: a press that takes the
                                        // keyboard from the editor (a native view) can lose
                                        // its release, which made a click need doing twice.
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(|this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.request_close_compose(cx)
                                            }),
                                        ),
                                )
                            })
                            .when(self.compose_detached, |d| {
                                d.child(
                                    icon_button(t, "attach")
                                        .id("compose-attach")
                                        .press(t)
                                        .tooltip(tip(t.dark, "Back into the window", ""))
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(|this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.attach_compose(cx)
                                            }),
                                        ),
                                )
                                .child(kbd(t, "esc"))
                            }),
                    ),
            )
            .child(field(
                "To",
                &composer.to,
                pills(Field::To, cx),
                cc_toggle,
                None,
            ))
            .children(drop_for(Field::To))
            .when(composer.show_cc, |d| {
                d.child(field("Cc", &composer.cc, pills(Field::Cc, cx), None, None))
                    .children(drop_for(Field::Cc))
                    .child(field(
                        "Bcc",
                        &composer.bcc,
                        pills(Field::Bcc, cx),
                        None,
                        None,
                    ))
                    .children(drop_for(Field::Bcc))
            })
            .child(field("Subject", &composer.subject, Vec::new(), None, None))
            .when(!chips.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .justify_center()
                        .border_b_1()
                        .border_color(t.border)
                        .child(
                            column(div())
                                .flex()
                                .flex_wrap()
                                .gap(px(8.))
                                .px(px(20.))
                                .py(px(10.))
                                .children(chips),
                        ),
                )
            })
            .when(composer.rich, |d| {
                let editor = self.editor.clone();
                let tool = |id: usize,
                            name: &'static str,
                            label: &'static str,
                            key: &'static str,
                            command: &'static str,
                            active: bool| {
                    let hover = t.hover;
                    div()
                        .id(("format", id))
                        .press(t)
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(px(28.))
                        .rounded(px(7.))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .when(active, |d| d.bg(t.hover).border_1().border_color(t.border))
                        .child(icon(name, 15., if active { t.text } else { t.text2 }))
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if *hovered {
                                this.format_hint = Some((label, key));
                            } else if this.format_hint.is_some_and(|(l, _)| l == label) {
                                this.format_hint = None;
                            }
                            cx.notify();
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(editor) = &this.editor {
                                editor.exec(command);
                            }
                            this.pending_focus = Some(this.editor_focus.clone());
                            cx.notify();
                        }))
                };
                let f = composer.format;
                let gap = || div().w(px(1.)).h(px(16.)).mx(px(4.)).bg(t.border);
                d.child(
                    div()
                        .flex()
                        .justify_center()
                        .border_b_1()
                        .border_color(t.border)
                        .child(
                            column(div())
                                .flex()
                                .items_center()
                                .gap(px(2.))
                                .px(px(14.))
                                .py(px(5.))
                                .child(tool(0, "bold", "Bold", "⌘B", "bold", f[0]))
                                .child(tool(1, "italic", "Italic", "⌘I", "italic", f[1]))
                                .child(tool(2, "underline", "Underline", "⌘U", "underline", f[2]))
                                .child(gap())
                                .child(tool(3, "link", "Link", "⌘K", "link", false))
                                .child(gap())
                                .child(tool(
                                    4,
                                    "bullets",
                                    "Bulleted list",
                                    "⌘⇧8",
                                    "insertUnorderedList",
                                    f[3],
                                ))
                                .child(tool(
                                    5,
                                    "numbers",
                                    "Numbered list",
                                    "⌘⇧7",
                                    "insertOrderedList",
                                    f[4],
                                ))
                                .child(tool(6, "quote", "Quote", "", "blockquote", false))
                                .child(gap())
                                .child(tool(
                                    7,
                                    "clear-format",
                                    "Clear formatting",
                                    "",
                                    "removeFormat",
                                    false,
                                ))
                                .child(div().flex_1())
                                .when_some(self.format_hint, |d, (label, key)| {
                                    d.child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(8.))
                                            .text_size(px(12.))
                                            .text_color(t.text2)
                                            .child(label)
                                            .when(!key.is_empty(), |d| d.child(kbd(t, key))),
                                    )
                                }),
                        ),
                )
                // The editor is a native web view: this element only reserves its space and
                // tells it where that space is each frame.
                .child(
                    div()
                        .id("compose-body")
                        .flex_1()
                        .min_h(px(0.))
                        .key_context("Editor")
                        .track_focus(&self.editor_focus)
                        .child(
                            gpui::canvas(
                                move |bounds, _, _| {
                                    // Floating, the editor (a native view) leaves the card's
                                    // side edges free for resizing.
                                    let inset = if full { 0. } else { 6. };
                                    if let Some(editor) = &editor {
                                        editor.set_bounds(
                                            f32::from(bounds.origin.x) + inset,
                                            f32::from(bounds.origin.y),
                                            f32::from(bounds.size.width) - 2. * inset,
                                            f32::from(bounds.size.height),
                                        );
                                    }
                                },
                                |_, _, _, _| {},
                            )
                            .size_full(),
                        ),
                )
            })
            .when(!composer.rich, |d| {
                d.child(
                    div()
                        .id("compose-body")
                        .flex()
                        .justify_center()
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        .child(
                            column(div())
                                .flex()
                                .flex_col()
                                .px(px(20.))
                                .py(px(if full { 24. } else { 14. }))
                                .text_size(px(15.))
                                .children(composer.lines.iter().cloned())
                                // The quoted earlier message stays folded behind "…".
                                .when_some(composer.quoted.clone(), |d, quoted| {
                                    d.child(
                                        div().flex().pt(px(10.)).child(
                                            div()
                                                .id("toggle-quoted")
                                                .px(px(8.))
                                                .rounded(px(6.))
                                                .bg(t.hover)
                                                .border_1()
                                                .border_color(t.border)
                                                .text_color(t.text3)
                                                .cursor_pointer()
                                                .child("•••")
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    if let Some(composer) = &mut this.compose {
                                                        composer.show_quoted =
                                                            !composer.show_quoted;
                                                        cx.notify();
                                                    }
                                                })),
                                        ),
                                    )
                                    .when(
                                        composer.show_quoted,
                                        |d| {
                                            d.child(
                                                div()
                                                    .mt(px(10.))
                                                    .pl(px(14.))
                                                    .border_l_2()
                                                    .border_color(t.border_strong)
                                                    .text_size(px(14.))
                                                    .line_height(px(22.))
                                                    .text_color(t.text3)
                                                    .child(short(&quoted, 6000)),
                                            )
                                        },
                                    )
                                }),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_center()
                    .border_t_1()
                    .border_color(t.border)
                    .child(
                        column(div())
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .px(px(14.))
                            .py(px(12.))
                            .child(
                                icon_button(t, "clip")
                                    .id("attach")
                                    .press(t)
                                    .tooltip(tip(t.dark, "Attach files", ""))
                                    .on_click(cx.listener(|this, _, _, cx| this.pick_files(cx))),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .pl(px(4.))
                                    .text_color(if composer.error.is_some() {
                                        t.rose
                                    } else {
                                        t.text3
                                    })
                                    .child(match &composer.error {
                                        Some(error) => short(error, 90),
                                        None if composer.sending => "Sending…".into(),
                                        None if composer.saving => "Saving…".into(),
                                        None if composer.saved => "Saved to Drafts".into(),
                                        None if self.gmail.is_none() => {
                                            "Sample mode: nothing is sent.".into()
                                        }
                                        None => "Drop files here to attach them.".into(),
                                    }),
                            )
                            .child(
                                icon_button(t, "trash")
                                    .id("discard")
                                    .press(t)
                                    .tooltip(tip(t.dark, "Discard draft", ""))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.discard_compose(cx)),
                                    ),
                            )
                            .child(
                                button(t, ButtonKind::Secondary, "Send later", None)
                                    .id("send-later")
                                    .press_soft()
                                    .tooltip(tip(
                                        t.dark,
                                        "Sent from this Mac while Nimbra is running",
                                        "",
                                    ))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_menu(Menu::SendLater, window, cx)
                                    })),
                            )
                            .child(
                                button(t, ButtonKind::Primary, "Send", Some("⌘↵"))
                                    .id("send")
                                    .press_soft()
                                    .when(composer.sending, |d| d.opacity(0.5))
                                    .on_click(cx.listener(|this, _, _, cx| this.send_mail(cx))),
                            ),
                    ),
            )
            // Every edge and corner resizes the floating card (no visible grip).
            .when(!full, |d| {
                use gpui::CursorStyle as C;
                // Each handle reaches a little past the card (where the mouse also comes to
                // the app over a message window's email) and a little inside it.
                const EDGE: f32 = 6.;
                const CORNER: f32 = 16.;
                const OUT: f32 = 8.;
                let handles: [(i8, i8, C); 8] = [
                    (-1, -1, C::ResizeUpLeftDownRight),
                    (1, 1, C::ResizeUpLeftDownRight),
                    (1, -1, C::ResizeUpRightDownLeft),
                    (-1, 1, C::ResizeUpRightDownLeft),
                    (0, -1, C::ResizeUpDown),
                    (0, 1, C::ResizeUpDown),
                    (-1, 0, C::ResizeLeftRight),
                    (1, 0, C::ResizeLeftRight),
                ];
                // Edges first, so the corners sit on top of them.
                d.children(handles.into_iter().rev().map(|(hx, hy, cursor)| {
                    let corner = hx != 0 && hy != 0;
                    let mut h = div().absolute().cursor(cursor);
                    h = match hx {
                        -1 => h.left(px(-OUT)),
                        1 => h.right(px(-OUT)),
                        _ => h.left(px(CORNER)).right(px(CORNER)),
                    };
                    h = match hy {
                        -1 => h.top(px(-OUT)),
                        1 => h.bottom(px(-OUT)),
                        _ => h.top(px(CORNER)).bottom(px(CORNER)),
                    };
                    if corner {
                        h = h.size(px(CORNER + OUT));
                    } else if hx == 0 {
                        h = h.h(px(EDGE + OUT));
                    } else {
                        h = h.w(px(EDGE + OUT));
                    }
                    h.on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event, _, cx| {
                            cx.stop_propagation();
                            this.grab_composer(Grab::Resize(hx, hy), event, cx)
                        }),
                    )
                }))
            })
            // "Save this draft?" over the composer.
            .when(self.compose_ask, |d| {
                d.child(
                    div()
                        .id("compose-ask")
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(!full, |d| d.rounded(px(18.)))
                        .bg(t.scrim)
                        .occlude()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(14.))
                                .w(px(340.))
                                .p(px(22.))
                                .rounded(px(16.))
                                .bg(t.raised)
                                .border_1()
                                .border_color(t.border_strong)
                                .shadow(vec![shadow(c(0x000000, 0.35), 18., 50., -8.)])
                                .child(serif("Save this draft?", 24., t.text))
                                .child(meta(
                                    "Keep it in Drafts to finish later, or discard what you wrote.",
                                    t.text2,
                                ))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(8.))
                                        .pt(px(4.))
                                        .child(
                                            button(t, ButtonKind::Ghost, "Discard", None)
                                                .id("ask-discard")
                                                .press_soft()
                                                .text_color(t.rose)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.answer_close_compose(false, cx)
                                                })),
                                        )
                                        .child(div().flex_1())
                                        .child(
                                            button(t, ButtonKind::Secondary, "Cancel", Some("esc"))
                                                .id("ask-cancel")
                                                .press_soft()
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.request_close_compose(cx)
                                                })),
                                        )
                                        .child(
                                            button(t, ButtonKind::Primary, "Save draft", None)
                                                .id("ask-save")
                                                .press_soft()
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.answer_close_compose(true, cx)
                                                })),
                                        ),
                                ),
                        ),
                )
            });
        div()
            .id("compose-scrim")
            .absolute()
            .inset_0()
            .flex()
            .items_start()
            .justify_center()
            // Dimmed in the main window; over a message window the email stays readable.
            .when(!full && self.compose_host.is_none(), |d| d.bg(t.scrim))
            // A grabbed composer follows the mouse anywhere in the window.
            .on_mouse_move(
                cx.listener(move |this, event, _, cx| this.drag_composer(event, viewport, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if let Some(composer) = &mut this.compose {
                        composer.grab = None;
                        cx.notify();
                    }
                }),
            )
            // Over a message window the email behind can still be scrolled.
            .when(self.compose_host.is_none(), |d| d.occlude())
            .child(card)
            .into_any_element()
    }

    /// An empty mailbox other than the inbox (which gets the Inbox Zero screen).
    fn empty_mailbox(&self, t: &Theme) -> impl IntoElement {
        div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(14.))
            .bg(t.surface)
            .child(serif(
                &if self.mailbox == View::Search {
                    "No results".to_string()
                } else {
                    format!("Nothing in {}", self.inbox_name())
                },
                56.,
                t.text,
            ))
            .child(
                div()
                    .text_size(px(15.))
                    .line_height(px(24.))
                    .text_color(t.text2)
                    .child(if self.mailbox == View::Search {
                        format!("Nothing matches “{}”.", short(&self.query, 60))
                    } else {
                        "Threads will show up here.".to_string()
                    }),
            )
    }

    fn zero(&self, t: &Theme) -> impl IntoElement {
        let ink = c(0xf5f3ef, 1.);
        let soft = c(0xffffff, 0.7);
        let glass = |d: Div| {
            d.bg(c(0xffffff, 0.07))
                .border_1()
                .border_color(c(0xffffff, 0.14))
                .rounded(px(14.))
        };
        let stat = |name: &str, color: Hsla, label: &str, value: &str| {
            glass(div())
                .flex()
                .flex_col()
                .gap(px(6.))
                .pl(px(20.))
                .pr(px(28.))
                .py(px(16.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(icon(name, 16., color))
                        .child(meta(label.to_string(), soft)),
                )
                .child(
                    div()
                        .font_family(SERIF)
                        .text_size(px(40.))
                        .line_height(px(44.))
                        .text_color(ink)
                        .child(value.to_string()),
                )
        };
        let arrival = self.arrival.unwrap_or(Arrival::SETTLED);
        let settle = |d: Div, shown: f32| d.relative().top(px(drop_for(shown))).opacity(shown);
        // Deterministic star field.
        let mut seed = 7u32;
        let mut rnd = move || {
            seed = (seed.wrapping_mul(9301).wrapping_add(49297)) % 233280;
            seed as f32 / 233280.
        };
        let stars: Vec<Div> = (0..48)
            .map(|i| {
                let (x, y, a) = (rnd(), rnd() * 0.42, 0.2 + rnd() * 0.6);
                div()
                    .absolute()
                    .left(relative(x))
                    .top(relative(y))
                    .size(px(if a > 0.7 { 2.5 } else { 1.5 }))
                    .rounded_full()
                    .bg(c(0xffffff, a * arrival.star(i)))
            })
            .collect();
        // The sample inbox keeps the day its mail is from.
        let today = if self.is_demo() {
            "THURSDAY, NOVEMBER 20".to_string()
        } else {
            chrono::Local::now()
                .format("%A, %B %-d")
                .to_string()
                .to_uppercase()
        };
        let layer = |index: usize, angle: f32, color: u32, alpha: f32, end: f32| {
            let shown = arrival.glow(index);
            // Each reaches further from its edge as it brightens, so the light rises.
            let reach = end * (0.6 + 0.4 * shown);
            div()
                .absolute()
                .inset_0()
                .bg(fade(angle, color, alpha * shown, reach))
        };

        let below = if self.can_undo() {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(meta("Press", c(0xffffff, 0.5)))
                .child(kbd(&Theme::dark(), "Z"))
                .child(meta("to undo", c(0xffffff, 0.5)))
        } else {
            div()
                .w_full()
                .px(px(32.))
                .text_center()
                .font_family(SERIF)
                .italic()
                .text_size(px(24.))
                .line_height(px(30.))
                .text_color(soft)
                .child(crate::inspiration::line_for(
                    chrono::Local::now().date_naive(),
                ))
        };

        div().flex_1().min_w(px(0.)).h_full().bg(t.surface).child(
            div()
                .relative()
                .flex()
                .flex_col()
                .items_center()
                .size_full()
                .overflow_hidden()
                .bg(c(0x0a0912, 1.))
                // Aurora: GPUI has no blur filter, so the glow is layered two-stop gradients.
                .child(layer(0, 45., 0x6b4dff, 0.9, 0.62))
                .child(layer(2, 315., 0xffa94d, 0.6, 0.5))
                .child(layer(1, 0., 0xff4fa0, 0.55, 0.42))
                .child(layer(3, 300., 0x3fa9ff, 0.22, 0.4))
                .children(stars)
                // Two halves with the horizon between: each holds its content in its middle.
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .w_full()
                        .h(relative(HORIZON))
                        .child(settle(overline(today, c(0xffffff, 0.55)), arrival.date()))
                        .child(div().h(px(18.)).flex_none())
                        .child(settle(serif("Inbox zero.", 120., ink), arrival.headline()))
                        .child(div().h(px(14.)).flex_none())
                        .child(
                            settle(div(), arrival.line())
                                .text_size(px(17.))
                                .line_height(px(26.))
                                .text_color(soft)
                                .child(if self.is_demo() {
                                    "Every important thread is handled. Go make something wonderful."
                                } else if self.mailbox == View::AllInboxes {
                                    "Your inboxes are empty. Go make something wonderful."
                                } else {
                                    "Your inbox is empty. Go make something wonderful."
                                }),
                        )
                        // Sample content: real mail shows the headline only.
                        .when(self.is_demo(), |d| {
                            d.child(
                                div()
                                    .flex()
                                    .mt(px(40.))
                                    .gap(px(12.))
                                    .child(stat(
                                        "done",
                                        t.mint,
                                        "threads handled",
                                        &(32 + self.history.len()).to_string(),
                                    ))
                                    .child(stat("clock", t.sky, "time in inbox", "38m"))
                                    .child(stat("zap", t.amber, "zero streak", "8 days")),
                            )
                        })
                )
                // Drawn out from the middle.
                .child(
                    div()
                        .flex_none()
                        .w(relative(arrival.horizon()))
                        .h(px(1.))
                        .bg(c(0xffffff, 0.22)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .w_full()
                        .gap(px(18.))
                        .when(self.is_demo(), |d| {
                            d.child(
                                glass(div())
                                    .bg(c(0x14131c, 0.72))
                                    .flex()
                                    .items_center()
                                    .gap(px(14.))
                                    .pl(px(14.))
                                    .pr(px(10.))
                                    .py(px(10.))
                                    .text_color(ink)
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .size(px(34.))
                                            .rounded(px(9.))
                                            .bg(Tone::Sky.gradient())
                                            .child(icon("calendar", 16., gpui::white())),
                                    )
                                    .child(
                                        div()
                                            .mr(px(40.))
                                            .child(
                                                div()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .child("Up next · Design crit"),
                                            )
                                            .child(meta(
                                                "2:00 PM · Studio B · in 42 min",
                                                c(0xffffff, 0.55),
                                            )),
                                    )
                                    .child(button(
                                        &Theme::dark(),
                                        ButtonKind::Secondary,
                                        "Join",
                                        Some("J"),
                                    )),
                            )
                        })
                        .child(settle(below, arrival.below())),
                ),
        )
    }

    fn palette(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &self.mails[self.selected];
        // The sample data shows the designed palette; a real mailbox gets real snooze times.
        let rows: Vec<(
            &'static str,
            SharedString,
            SharedString,
            &'static str,
            Option<&'static str>,
        )> = if self.is_demo() {
            COMMANDS
                .iter()
                .map(|(name, label, hint, key, header)| {
                    (*name, (*label).into(), (*hint).into(), *key, *header)
                })
                .collect()
        } else {
            self.snooze_choices
                .iter()
                .map(|c| {
                    (
                        c.icon,
                        c.label.clone().into(),
                        c.hint.clone().into(),
                        "",
                        c.header,
                    )
                })
                .collect()
        };
        let items =
            rows.into_iter()
                .enumerate()
                .flat_map(|(i, (name, label, hint, key, header))| {
                    let active = i == self.palette_sel;
                    let hover = t.hover;
                    let head = header.map(|h| {
                        div()
                            .pl(px(12.))
                            .pt(px(12.))
                            .pb(px(6.))
                            .child(overline(h, t.faint))
                            .into_any_element()
                    });
                    let row = div()
                        .id(("command", i))
                        .press(t)
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .pl(px(10.))
                        .pr(px(12.))
                        .py(px(8.))
                        .rounded(px(10.))
                        .cursor_pointer()
                        .when(active, |d| d.bg(t.selected))
                        .when(!active, |d| d.hover(move |s| s.bg(hover)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.palette_sel = i;
                            this.run_command(cx)
                        }))
                        .child(
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .size(px(28.))
                                .rounded(px(8.))
                                .when(active, |d| {
                                    d.bg(gradient(135., c(0x8b7cff, 1.), c(0xff7ab6, 1.)))
                                })
                                .when(!active, |d| d.bg(t.hover).border_1().border_color(t.border))
                                .child(icon(
                                    name,
                                    16.,
                                    if active { gpui::white() } else { t.text2 },
                                )),
                        )
                        .child(
                            div()
                                .flex_none()
                                .font_weight(FontWeight::MEDIUM)
                                .child(label.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_color(t.text3)
                                .child(hint.clone()),
                        )
                        .when(!key.is_empty(), |d| d.child(kbd(t, key)))
                        .into_any_element();
                    head.into_iter().chain(std::iter::once(row))
                });

        let hint = |keys: &[&str], label: &str| {
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .children(keys.iter().map(|k| kbd(t, k)))
                .child(meta(label.to_string(), t.text3))
        };

        div()
            .id("scrim")
            .absolute()
            .inset_0()
            .flex()
            .items_start()
            .justify_center()
            .pt(px(150.))
            .bg(t.scrim)
            .occlude()
            .on_click(cx.listener(|this, _, _, cx| {
                this.palette = false;
                cx.notify();
            }))
            .child(
                div()
                    .id("palette")
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .flex()
                    .flex_col()
                    .w(px(660.))
                    .h_auto()
                    .max_h(px(520.))
                    .overflow_hidden()
                    .rounded(px(18.))
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.border_strong)
                    .shadow(vec![
                        shadow(c(0x8b7cff, 0.22), 0., 120., 10.),
                        shadow(c(0x000000, if t.dark { 0.6 } else { 0.2 }), 30., 80., -10.),
                    ])
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .pl(px(20.))
                            .pr(px(16.))
                            .py(px(18.))
                            .border_b_1()
                            .border_color(t.border)
                            .child(icon("search", 18., t.text3))
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .items_center()
                                    .gap(px(2.))
                                    .text_size(px(17.))
                                    .line_height(px(24.))
                                    .child("snooze until i’m free")
                                    .child(div().w(px(2.)).h(px(20.)).rounded(px(1.)).bg(t.violet)),
                            )
                            .child(pill(
                                t,
                                m.subject.split(',').next().unwrap_or(&m.subject),
                                t.violet,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .px(px(8.))
                            .pt(px(6.))
                            .pb(px(8.))
                            .children(items),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(16.))
                            .pl(px(20.))
                            .pr(px(18.))
                            .py(px(12.))
                            .bg(t.hover)
                            .border_t_1()
                            .border_color(t.border)
                            .child(hint(&["↑", "↓"], "Navigate"))
                            .child(hint(&["↵"], "Snooze"))
                            .child(hint(&["esc"], "Close"))
                            .child(div().flex_1())
                            .child(
                                div()
                                    .font_family(SERIF)
                                    .italic()
                                    .text_size(px(18.))
                                    .line_height(px(20.))
                                    .text_color(t.text3)
                                    .child("Nimbra"),
                            ),
                    ),
            )
    }
}

impl Render for Nimbra {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The composer shown in another window: closing that window closes the composer
        // (keeping the draft), and a finished composer frees it.
        if let Some(host) = self.compose_host
            && (self.compose.is_none() || !cx.windows().contains(&host))
        {
            self.compose_host = None;
            self.compose_detached = false;
            if self.compose.is_some() {
                self.close_compose(cx);
            }
        }
        let composing_elsewhere = self.compose.is_some() && self.compose_host.is_some();
        if !composing_elsewhere && let Some(focus) = self.pending_focus.take() {
            window.focus(&focus);
        }
        // "System" follows the Mac's light or dark appearance.
        if self.settings.theme == crate::settings::ThemeChoice::System
            && std::env::var_os("NIMBRA_THEME").is_none()
        {
            self.dark = matches!(
                window.appearance(),
                gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
            );
        }
        let t = self.theme();
        let viewport = window.viewport_size();
        let viewport = (f32::from(viewport.width), f32::from(viewport.height));
        // The floating composer needs a concrete position before it can be dragged.
        if let Some(composer) = &mut self.compose
            && composer.position.is_none()
            && !composing_elsewhere
        {
            composer.position = Some((((viewport.0 - composer.size.0) / 2.).max(0.), 96.));
        }
        // Measure the display's frame interval from back-to-back frames (an animation keeps
        // them coming), smoothed; long gaps between idle frames are ignored.
        let now = std::time::Instant::now();
        let mut since_last = 0.;
        if let Some(last) = self.last_frame {
            let dt = now.duration_since(last).as_secs_f32();
            since_last = dt;
            if (0.004..0.035).contains(&dt) {
                let smooth = self.frame_interval.get() * 0.8 + dt * 0.2;
                self.frame_interval.set(smooth);
            }
        }
        self.last_frame = Some(now);
        self.sync_html_view(window);
        if !composing_elsewhere {
            self.sync_editor(window, cx);
        }
        // A new message (or none): show it in its window, after this frame.
        if self.notice != self.notice_shown {
            self.notice_shown = self.notice.clone();
            let room = f32::from(window.bounds().size.width);
            let toast = self
                .notice
                .as_ref()
                .map(|text| Toast::new(text, room, window.text_system()));
            cx.spawn(async move |this, cx| {
                this.update(cx, |this, cx| this.sync_notice_window(toast, cx))
                    .ok();
            })
            .detach();
        }
        // Infinite list: the next page starts loading as the end of the list comes near
        // (or straight away when the list does not fill the pane yet).
        if self.more().is_some() && !self.loading_more && !self.fetching() && self.near_list_end() {
            // Asked again once this frame is laid out: until then a list that has just
            // arrived has no height, and each first listing fetched its second page too.
            let this = cx.weak_entity();
            window.on_next_frame(move |_, cx| {
                this.update(cx, |this, cx| {
                    if !this.fetching() && this.near_list_end() {
                        this.load_more(cx);
                    }
                })
                .ok();
            });
        }
        // The checking-for-mail bar fades out when the check is done, instead of vanishing.
        let bar = self.shows_checking() && !self.pull_hold && self.pull == 0.;
        if self.bar_shown && !bar {
            let n = self.bar_fade.map_or(0, |(_, n)| n + 1);
            self.bar_fade = Some((std::time::Instant::now(), n));
        } else if bar {
            self.bar_fade = None;
        }
        self.bar_shown = bar;
        if self
            .bar_fade
            .is_some_and(|(at, _)| at.elapsed() > Duration::from_millis(600))
        {
            self.bar_fade = None;
        }
        // The inbox's unread count on the Dock icon.
        let unread = if self.gmail.is_some() && self.settings.dock_badge {
            // Every signed-in account's inbox.
            self.counts.get("INBOX").map_or(0, |(unread, _)| *unread)
                + self
                    .account_unread
                    .iter()
                    .filter(|(id, _)| **id != self.account_id && self.others.contains_key(*id))
                    .map(|(_, n)| *n)
                    .sum::<u32>()
        } else {
            0
        };
        if self.badge != Some(unread) {
            crate::system::set_badge(unread);
            self.badge = Some(unread);
        }
        self.frame = Some(window.bounds());
        if self.pull_hold && !self.fetching() {
            self.pull_hold = false;
            self.pull_done = true;
            // Close from wherever the strip is right now, so a fast refresh does not jump.
            let now = self
                .pull_anim
                .map_or(PULL_REST, |(from, to, start)| settle(from, to, start));
            self.pull_anim = Some((now, 0., std::time::Instant::now()));
            self.pull_gen += 1;
        }
        self.watch_edge(window, cx);
        let place = self.sidebar_place(0.);
        // 1 while the sidebar floats, down to 0 as it docks.
        let afloat = place.map_or(0., |place| place.inset / FLOAT_INSET);
        // The traffic lights travel with the sidebar. A full-page composer has room of its
        // own for them in its header.
        let full_compose = !composing_elsewhere && self.compose.as_ref().is_some_and(|c| c.full);
        crate::autohide::place_traffic_lights(
            window,
            if full_compose {
                Some(18.)
            } else {
                place.map(|place| 18. + place.x)
            },
        );
        if self.peek.moving() || self.sidebar_sliding() {
            window.request_animation_frame();
        }
        let empty = self.visible().is_empty();
        let zero_shown = match self.status {
            Status::Demo | Status::Ready => empty && self.is_zero(),
            _ => self.opening_to_zero(),
        };
        self.arrival = self
            .arrival
            .filter(|_| zero_shown)
            .and_then(|arrival| arrival.after(since_last));
        if self.arrival.is_some() {
            window.request_animation_frame();
        }
        let list_shown =
            !empty && self.list_open && matches!(self.status, Status::Demo | Status::Ready);

        div()
            .id("nimbra")
            .key_context(if self.compose.is_some() && !composing_elsewhere {
                "Composer"
            } else {
                "Mail"
            })
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Next, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &Prev, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &MarkDone, _, cx| {
                if !this.palette {
                    this.mark_done(cx)
                }
            }))
            .on_action(cx.listener(|this, _: &Undo, _, cx| this.undo(cx)))
            .on_action(
                cx.listener(|this, _: &Compose, _, cx| this.open_compose(ComposeKind::New, cx)),
            )
            .on_action(
                cx.listener(|this, _: &Reply, _, cx| this.open_compose(ComposeKind::Reply, cx)),
            )
            .on_action(cx.listener(|this, _: &SendMail, _, cx| this.send_mail(cx)))
            .on_action(cx.listener(|this, _: &SaveDraft, _, cx| this.save_draft(cx)))
            .on_action(cx.listener(|this, _: &ToggleOriginal, _, cx| this.toggle_original(cx)))
            .on_action(cx.listener(|this, _: &FocusSearch, _, cx| this.focus_search(cx)))
            .on_action(
                cx.listener(|this, _: &ToggleFullCompose, _, cx| this.toggle_full_compose(cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleStar, _, cx| this.toggle_star(cx)))
            .on_action(cx.listener(|this, _: &MarkUnread, _, cx| this.toggle_unread(cx)))
            .on_action(cx.listener(|this, _: &Trash, _, cx| {
                if !this.palette {
                    this.trash(cx)
                }
            }))
            .on_action(cx.listener(|this, _: &TogglePalette, _, cx| {
                if this.palette {
                    this.palette = false;
                } else if !this.visible().is_empty() {
                    this.open_palette();
                }
                this.palette_sel = 0;
                this.snooze_choices = Self::snooze_choices();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Print, _, cx| this.print_selected(cx)))
            .on_action(
                cx.listener(|this, _: &MenuReply, _, cx| this.open_compose(ComposeKind::Reply, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ReplyAll, _, cx| {
                    this.open_compose(ComposeKind::ReplyAll, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &MenuReplyAll, _, cx| {
                this.open_compose(ComposeKind::ReplyAll, cx)
            }))
            .on_action(cx.listener(|this, _: &ForwardMessage, _, cx| {
                this.open_compose(ComposeKind::Forward, cx)
            }))
            .on_action(cx.listener(|this, _: &MenuForwardMessage, _, cx| {
                this.open_compose(ComposeKind::Forward, cx)
            }))
            .on_action(cx.listener(|this, _: &MenuMarkDone, _, cx| this.mark_done(cx)))
            .on_action(cx.listener(|this, _: &MenuMarkUnread, _, cx| this.toggle_unread(cx)))
            .on_action(cx.listener(|this, _: &MenuStar, _, cx| this.toggle_star(cx)))
            .on_action(cx.listener(|this, _: &MenuTrash, _, cx| this.trash(cx)))
            .on_action(cx.listener(|this, _: &MenuUndo, _, cx| this.undo(cx)))
            .on_action(cx.listener(|this, _: &MenuToggleOriginal, _, cx| this.toggle_original(cx)))
            .on_action(cx.listener(|this, _: &MenuToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &MenuSearch, _, cx| this.focus_search(cx)))
            .on_action(cx.listener(|this, _: &MenuSnooze, _, cx| {
                if this.compose.is_none() && !this.visible().is_empty() {
                    this.open_palette();
                    this.palette_sel = 0;
                    this.snooze_choices = Self::snooze_choices();
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &SelectAllThreads, _, cx| {
                let visible = this.visible();
                if visible.len() >= 2 {
                    this.picked = visible.into_iter().collect();
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| {
                if this.menu.take().is_some() {
                    cx.notify();
                } else if this.finder_open {
                    this.close_finder(cx);
                } else if this.confirm.take().is_some() {
                    cx.notify();
                } else if this.new_label.is_some() {
                    this.close_new_label(cx);
                } else if !this.picked.is_empty() && !this.palette {
                    this.picked.clear();
                    cx.notify();
                } else if this.focus_restore.is_some() && !this.palette && this.compose.is_none() {
                    this.toggle_full_view(cx);
                } else if let Some(composer) = this.compose.as_mut()
                    && composer.suggest.take().is_some()
                {
                    cx.notify();
                } else if this.compose.is_some() {
                    this.request_close_compose(cx);
                } else if this.mailbox == View::Search || !this.search.read(cx).text().is_empty() {
                    this.clear_search(cx);
                } else {
                    this.palette = false;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Confirm, _, cx| {
                if this.palette {
                    this.run_command(cx);
                } else {
                    // Return: the open message in full view (and back).
                    this.toggle_full_view(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleTheme, _, cx| this.toggle_theme(cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, _, cx| this.open_settings(cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &Back, _, cx| this.go_back(cx)))
            .on_action(cx.listener(|this, _: &Forward, _, cx| this.go_forward(cx)))
            .on_action(cx.listener(|this, _: &Reload, _, cx| this.reload(cx)))
            .on_action(|_: &HideWindow, window, _| crate::blur::set_visible(window, false))
            .on_action(|_: &Minimize, window, _| window.minimize_window())
            .on_action(|_: &Quit, _, cx: &mut App| cx.quit())
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                let Some((start, width)) = this.list_grab else {
                    return;
                };
                if event.pressed_button != Some(MouseButton::Left) {
                    this.list_grab = None;
                    return;
                }
                this.list_width = (width + f32::from(event.position.x) - start).clamp(300., 720.);
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.list_grab = None),
            )
            .on_action(cx.listener(|this, _: &ToggleList, _, cx| this.toggle_list(cx)))
            .relative()
            .flex()
            .size_full()
            .font_family(crate::theme::SANS)
            .text_size(px(13.))
            .line_height(px(18.))
            .text_color(t.text)
            // The room the docked sidebar takes. The sidebar itself is drawn over it, so
            // floating in does not move or re-wrap anything. As it docks, what is around the
            // panel is filled (the panel's own shape is erased out of it).
            .child(
                div()
                    .flex_none()
                    .h_full()
                    .w(px(self.sidebar_width(0.)))
                    .when(afloat > 0., |d| d.bg(t.surface)),
            )
            .map(|d| match self.status {
                _ if zero_shown => d.child(self.zero(&t)),
                Status::Demo | Status::Ready if empty => d.child(self.empty_mailbox(&t)),
                Status::Demo | Status::Ready => d.children(self.list_slot(&t, cx)).map(|d| {
                    if self.picked.len() >= 2 {
                        d.child(self.bulk_pane(&t, cx))
                    } else {
                        d.child(self.reading(&t, cx))
                    }
                }),
                Status::Loading => d.child(self.skeleton(&t)),
                _ => d.child(self.status_screen(&t, cx)),
            })
            .when(self.offline && !list_shown && place.is_none(), |d| {
                // No list and no reload button to carry it.
                d.child(
                    div()
                        .id("offline-dot")
                        .absolute()
                        .left(px(8.))
                        .top(px(22.))
                        .size(px(6.))
                        .rounded_full()
                        .bg(t.amber)
                        .tooltip(tip(t.dark, self.offline_label(), "")),
                )
            })
            .when_some(place, |d, place| {
                let floating = afloat > 0.;
                let radius = FLOAT_RADIUS * afloat;
                let height = viewport.1 - 2. * place.inset;
                d.when(floating, |d| {
                    // The panel is the docked sidebar's material, see-through to the blurred
                    // desktop. The window's blur is only of what is behind the window, so
                    // what is painted under the panel is erased in its exact shape: its
                    // shadow (which is filled) too, leaving it only around the outside.
                    d.child(
                        div()
                            .absolute()
                            .left(px(place.x))
                            .top(px(place.inset))
                            .w(px(SIDEBAR_W))
                            .h(px(height))
                            .rounded(px(radius))
                            .shadow(vec![shadow(t.shadow.opacity(afloat), 10., 30., -6.)]),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(place.x))
                            .top(px(place.inset))
                            .w(px(SIDEBAR_W))
                            .h(px(height))
                            .child(crate::erase::rounded(radius, t.surface)),
                    )
                })
                .child(
                    div()
                        .absolute()
                        .left(px(place.x))
                        .top(px(place.inset))
                        .w(px(SIDEBAR_W))
                        .h(px(height))
                        .overflow_hidden()
                        .occlude()
                        .when(floating, |d| d.rounded(px(radius)).bg(t.sidebar))
                        // Moved up by the gap, so that docking shifts nothing vertically.
                        .child(
                            div()
                                .absolute()
                                .left_0()
                                .top(px(-place.inset))
                                .w(px(SIDEBAR_W))
                                .h(px(viewport.1 - place.inset))
                                .child(self.sidebar(&t, floating, cx)),
                        )
                        // Over the contents, so it takes no width from them.
                        .when(floating, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .rounded(px(radius))
                                    .border_1()
                                    .border_color(t.border_strong.opacity(afloat)),
                            )
                        }),
                )
                .child(self.window_controls(&t, place.x, list_shown, cx))
            })
            .when(self.palette && !empty, |d| d.child(self.palette(&t, cx)))
            .when(self.compose.is_some() && !composing_elsewhere, |d| {
                d.child(self.compose_view(&t, viewport, cx))
            })
            .when(self.new_label.is_some(), |d| {
                d.child(self.new_label_view(&t, cx))
            })
            .when(self.confirm.is_some(), |d| {
                d.child(self.confirm_view(&t, cx))
            })
            .when(self.finder_open, |d| d.child(self.finder_view(&t, cx)))
    }
}

/// A window holding only the composer (the composer's detach button).
struct ComposeWindow {
    owner: Entity<Nimbra>,
    /// Has shown the composer (until then it may not be registered as its window yet).
    shown: bool,
    _observe: gpui::Subscription,
}

impl Render for ComposeWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = self.owner.clone();
        let content = owner.update(cx, |n, cx| n.hosted_compose(window, cx));
        let t = owner.read(cx).theme();
        if content.is_some() {
            self.shown = true;
        }
        if content.is_none() && self.shown {
            // Sent, closed or discarded: the window goes with it.
            window.defer(cx, |window, _| window.remove_window());
        } else if let Some(composer) = &owner.read(cx).compose {
            let title = if composer.reply.is_some() {
                "Reply"
            } else {
                "New message"
            };
            window.set_window_title(title);
        }
        div()
            .relative()
            .size_full()
            .bg(t.surface)
            .font_family(crate::theme::SANS)
            .text_size(px(13.))
            .line_height(px(18.))
            .text_color(t.text)
            // ⌘W: asks about the draft (the window closes with the composer).
            .on_action(cx.listener(|this, _: &HideWindow, window, cx| {
                let handle = window.window_handle();
                this.owner.update(cx, |n, cx| {
                    if n.may_close_window(handle, cx) {
                        n.close_compose(cx);
                    }
                });
            }))
            .children(content)
    }
}

/// A profile photo's bytes as an image, if they are one of the formats Google serves.
fn photo_image(bytes: Vec<u8>) -> Option<Arc<gpui::Image>> {
    let format = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        gpui::ImageFormat::Png
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        gpui::ImageFormat::Jpeg
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        gpui::ImageFormat::Webp
    } else if bytes.starts_with(b"GIF8") {
        gpui::ImageFormat::Gif
    } else {
        return None;
    };
    Some(Arc::new(gpui::Image::from_bytes(format, bytes)))
}

/// The signed-in account's avatar: its profile photo, or its initial.
fn me_avatar(photo: Option<&Arc<gpui::Image>>, initials: &str, size: f32) -> AnyElement {
    match photo {
        Some(photo) => div()
            .flex_none()
            .size(px(size))
            .rounded_full()
            .overflow_hidden()
            .border_1()
            .border_color(c(0xffffff, 0.18))
            .child(gpui::img(photo.clone()).size_full().rounded_full())
            .into_any_element(),
        None => avatar(Tone::Graphite, initials, size).into_any_element(),
    }
}

/// For an account that is not shown: wakes its due snoozes (back to the inbox, unread) and
/// sends its due scheduled messages, as kept in its own store. One set from All inboxes can
/// be another account's: it goes through that account's connection. Runs off the main thread.
fn carry_out_due(
    account: &str,
    connected: &HashMap<String, Arc<gmail::Client>>,
    writes: &Underway,
) {
    let Ok(store) = crate::store::Store::open_quietly(account) else {
        return;
    };
    let now = chrono::Utc::now().timestamp_millis();
    let client_of = |whose: Option<&str>| connected.get(whose.unwrap_or(account));
    for snooze in store.load::<Vec<Snooze>>("snoozed").unwrap_or_default() {
        if snooze.until_ms > now {
            continue;
        }
        let Some(client) = client_of(snooze.mail.account.as_deref()) else {
            continue; // that account is not connected: later
        };
        let _working = writes.begin();
        // A reminder comes back only if nobody wrote since; either way it is done.
        let replied = snooze.no_reply
            && client
                .summary(&snooze.thread_id, false)
                .is_ok_and(|s| s.timestamp_ms > snooze.since_ms);
        let done = replied
            || client
                .relabel(&snooze.thread_id, &["INBOX", "UNREAD"], &[])
                .is_ok();
        // The list as it is by now: the account may have been opened and changed meanwhile.
        if done
            && store
                .update("snoozed", |list: &mut Vec<Snooze>| {
                    list.retain(|s| {
                        s.thread_id != snooze.thread_id || s.until_ms != snooze.until_ms
                    });
                })
                .is_err()
        {
            eprintln!("nimbra: could not save the snoozed list");
        }
    }
    for item in scheduled::list(&store) {
        if item.at_ms > now {
            continue;
        }
        if let Some(client) = client_of(item.account.as_deref()) {
            let _working = writes.begin();
            scheduled::send(&store, client, &item.draft_id);
        }
    }
}

#[cfg(test)]
mod header_tests {
    use super::*;

    #[test]
    fn the_list_header_fits_at_every_width() {
        let used = |fit: HeaderFit, switch: f32, offline: Option<f32>| {
            let pill = offline.map_or(0., |w| HEADER_GAP + if fit.words { w } else { 22. });
            switch + pill + HEADER_GAP + fit.compose.width()
        };
        for list in 300..=720 {
            let room = list as f32 - 1. - LIST_LEFT - LIST_RIGHT;
            for digits in 1..=4 {
                let switch = 124. + 7. * digits as f32;
                for offline in [None, Some(67.), Some(140.)] {
                    let fit = HeaderFit::new(room, switch, offline);
                    assert!(
                        used(fit, switch, offline) <= room,
                        "{list} {digits} {offline:?}"
                    );
                }
            }
        }
        // At the default width there is room for all of it.
        let full = HeaderFit::new(452. - 1. - LIST_LEFT - LIST_RIGHT, 131., Some(126.));
        assert_eq!((full.words, full.compose), (true, ComposeLook::Keyed));
    }
}
