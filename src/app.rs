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
use crate::checking::{Asked, Checks, within};
use crate::data::{COMMANDS, Mail, inbox};
use crate::gmail;
use crate::input::{InputEvent, TextInput};
use crate::keyboard::{self, Pane, Row as SidebarRow};
use crate::leaving::{self, End, Leaving, SavedAs, Steps, Underway};
use crate::mail::{self, View};
use crate::rich::{self, Block, Kind};
use crate::scheduled::{self, Outcome, Scheduled};
use crate::sending::{Job, QueuedMail, Stage};
use crate::theme::{Accent, SERIF, Theme, Tone, c, fade, gradient};
use crate::toast::{self, Toast};
use crate::ui::{
    ButtonKind, Press, avatar, button, dp, from_screen, icon, icon_button, kbd, meta, mono,
    overline, pill, shadow, tip, to_screen, zoom,
};
use crate::webview::HtmlView;

#[path = "message_window.rs"]
mod message_window;
use message_window::MessageWindow;

#[path = "skeleton.rs"]
mod skeleton;

#[path = "selection.rs"]
mod selection;
use selection::ReaderSelection;

#[path = "add_account.rs"]
mod add_account;
use add_account::AccountSheet;

#[path = "settings_window.rs"]
mod settings_window;
use settings_window::SettingsWindow;

#[path = "composer.rs"]
mod composer;
use composer::*;

#[path = "outgoing.rs"]
mod outgoing;

#[path = "menus.rs"]
mod menus;
use menus::*;

#[path = "accounts.rs"]
mod accounts;

#[path = "mailbox.rs"]
mod mailbox;

#[path = "finder.rs"]
mod finder;
use finder::*;

#[path = "labels.rs"]
mod labels;
use labels::*;

#[path = "threads.rs"]
mod threads;
use threads::*;

#[path = "sidebar.rs"]
mod sidebar;
use sidebar::*;

#[path = "list.rs"]
mod list;
use list::*;

#[path = "reading.rs"]
mod reading;
use reading::*;

#[path = "palette.rs"]
mod palette;

#[path = "shortcuts_sheet.rs"]
mod shortcuts_sheet;
use shortcuts_sheet::ShortcutsSheet;
#[path = "snooze_menu.rs"]
mod snooze_menu;
#[path = "suggest_popup.rs"]
mod suggest_popup;
use snooze_menu::SnoozeMenu;

#[path = "system_events.rs"]
mod system_events;
pub use system_events::{
    ActualSize, DockDrafts, DockInbox, DockSent, DockSnoozed, DockStarred, UseDarkAppearance,
    UseLightAppearance, UseSystemAppearance, ZoomIn, ZoomOut, install as install_system_actions,
};

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
        /// Nimbra › About Nimbra.
        AboutNimbra,
        /// Nimbra › Check for Updates…
        CheckForUpdates,
        /// Help › Report a Problem…: a new issue on GitHub.
        ReportProblem,
        /// Help › Release Notes: this version's, on the website.
        ReleaseNotes,
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
        MenuShortcuts,
        // Twins of actions whose keys can be changed, for when none of their keys works
        // anywhere in the app (see `shortcut_item` in main.rs).
        MenuCompose,
        MenuReload,
        MenuToggleList,
        MenuToggleTheme,
        Quit,
        // The keyboard (see `keyboard.rs`).
        FocusSidebar,
        OpenSidebarRow,
        First,
        Last,
        PickThread,
        PickDown,
        PickUp,
        OpenInWindow,
        PageDown,
        PageUp,
        ToggleShortcuts,
        GoInbox,
        GoStarred,
        GoSnoozed,
        GoSent,
        GoDrafts,
        GoAllMail
    ]
);

/// ⌘1 to ⌘9: All inboxes, then each account (`keyboard::Sidebar::numbered`).
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = nimbra, no_json)]
pub struct GoNumber(pub usize);

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

/// Something that needs a yes before it happens.
#[derive(Clone)]
enum Ask {
    /// Delete these threads for good, by (thread id, account).
    DeleteForever(Vec<(String, String)>),
    /// Ask Google for permanent deletion, then delete these threads (the account shown's).
    GrantFullAccess(Vec<(String, String)>),
    /// Delete a label (id, name); the mail stays.
    DeleteLabel(String, String),
}

// Changes and messages made while offline, sent once the server can be reached again.
pub(crate) use crate::outbox::Outbox;

/// What Google answered about an account's photo.
#[derive(Clone, Copy, PartialEq)]
enum PhotoAnswer {
    Theirs,
    Unchecked,
    None,
    NotAllowed,
}

/// How long a sent or discarded message can still be taken back.
const UNDO_DELAY: Duration = Duration::from_secs(6);
/// The longest quitting waits for what is on its way to Gmail. That is on disk as well.
const QUIT_WAIT: Duration = Duration::from_secs(30);
const KEYCHAIN_REFUSED: &str = "Nimbra could not read your sign-in from the Keychain. Nothing was deleted and you are still signed in. Try again, and allow access if macOS asks.";
const KEYCHAIN_WAIT: Duration = Duration::from_secs(8);
const KEYCHAIN_WAITING: &str = "macOS may be asking for your password or for permission, perhaps behind another window. Nimbra carries on as soon as it has an answer.";
const QUIT_WOULD_LOSE: &str = "Nimbra can’t keep your unsent mail: it can’t read its Keychain key. Quit again to quit and lose it.";
/// The store record of an account's profile photo.
const PHOTO: &str = "profile-photo";
const STORE_UNAVAILABLE: &str = "Nimbra could not open what it keeps on this Mac. Nothing was deleted. Allow Keychain access and reopen Nimbra.";
/// The account and sidebar state saved alongside the mailboxes (the engine's record).
type Saved = nimbra_core::listing::Meta;

/// The store record a mailbox is saved under.
fn record(view: View) -> String {
    nimbra_core::listing::record(view, None)
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
    reply_to: Option<mail::ReplyTo>,
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
            body: mail.body.to_vec(),
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
            .map(|thread_id| mail::ReplyTo {
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
    /// All inboxes is being listed for the first time, and every account's Inbox was empty
    /// when last saved: what to draw until it is listed.
    all_inboxes_saved_empty: bool,
    /// Every account was reached by the last All inboxes listing.
    all_inboxes_reached: bool,
    /// Each account's next page in every account's mailboxes (All inboxes is one), by
    /// mailbox and then account.
    all_next: HashMap<View, HashMap<String, String>>,
    /// Every account's labels, one per name: what every account's `View::Label(i)` is.
    every_labels: Vec<nimbra_core::listing::NamedLabel>,
    /// The other accounts' labels, as their stores last kept them (`refresh_every_labels`),
    /// for their threads in every account's mailboxes (`labels_of`).
    account_labels: HashMap<String, Vec<mail::Label>>,
    /// Where the window was when Nimbra last closed (`place.rs`), until it is opened again
    /// there once the account's store is open, or another mailbox is chosen first.
    restore_place: Option<crate::place::Place>,
    /// Where the window is, as last saved.
    place_saved: Option<crate::place::Place>,
    /// Opening last time's place again waits for the other accounts' stores (once).
    place_waiting: bool,
    /// The other accounts' sidebar counts by label id, as the last listing of every
    /// account's mailboxes found them (the account shown has `counts`).
    account_counts: HashMap<String, HashMap<String, (u32, u32)>>,
    /// The mailboxes that threads were cleared from since they were last listed whole from
    /// the top: their page tokens may be off.
    shifted: HashSet<View>,
    /// The connected accounts' stores that are not the shown one's: what All inboxes does
    /// to their mail is kept in them.
    other_stores: HashMap<String, Arc<crate::store::Store>>,
    /// The accounts not shown that have changes waiting for a connection.
    away_waiting: HashSet<String>,
    /// Whether each account's saved Inbox is empty, as last saved here.
    inboxes_saved_empty: HashMap<String, bool>,
    /// Reads of the keychain on their way for the account shown.
    keychain: Underway,
    /// One of them is taking long enough to say so.
    keychain_slow: bool,
    /// Inbox Zero coming up, on a launch that opens on it.
    arrival: Option<Arrival>,
    /// The night over Inbox Zero once it has come up, where a star falls now and then.
    sky: Option<crate::sky::Sky>,
    /// Waits for the next falling star.
    sky_timer: Option<gpui::Task<()>>,
    /// When today ends, by the wall clock, and the timer that wakes the list then.
    day_ends: Option<std::time::SystemTime>,
    day_timer: Option<gpui::Task<()>>,
    /// What deciding whether the Inbox emptied went by last time (see `zero_inputs`).
    zero_seen: Option<ZeroInputs>,
    /// The message whose full header (all recipients, date) is expanded: thread id and
    /// message index (0 is the latest).
    details: Option<(String, usize)>,
    /// Earlier messages opening or folding, by thread and place (`reading::Fold`).
    folds: std::cell::RefCell<HashMap<(ThreadKey, usize), reading::Fold>>,
    /// Drafts fetched ahead as their rows are opened in Drafts (`prefetch_draft`), so
    /// Edit draft opens the composer at once.
    drafts_ready: HashMap<ThreadKey, mail::Draft>,
    /// Drafts being fetched now, so a row opened again or Edit draft never fetches twice.
    drafts_fetching: std::collections::HashSet<ThreadKey>,
    /// Edit draft was pressed while its draft was still on its way: opened when it is here
    /// (`true`: in a window of its own).
    draft_wanted: Option<(ThreadKey, bool)>,
    /// Closed composers whose new drafts show in Drafts as made-up rows (`show_in_drafts`)
    /// until Drafts is listed again, by row id: Edit draft on such a row brings its
    /// composer back, as Gmail does not know the row's id.
    closed_drafts: Vec<(String, Composer)>,
    /// Rows of discarded drafts, hidden through every listing (`install`) until a listing
    /// after their delete (`true` once it went through) no longer has them.
    hidden_drafts: HashMap<ThreadKey, bool>,
    /// The latest messages folded in their conversations: thread, account, Message-ID.
    latest_folded: std::collections::HashSet<(String, String, String)>,
    /// The height of an open earlier message as last laid out, by thread and place.
    fold_heights: reading::FoldHeights,
    /// A folded message's line, as last laid out: where one opens from and folds to.
    fold_line: std::rc::Rc<std::cell::Cell<f32>>,
    /// A message opening or folding, for the emails under it (`reading::FoldLead`).
    fold_lead: reading::FoldLead,
    /// The display the main window is on (`popup::display_of`).
    display: Option<gpui::DisplayId>,
    /// Changes to other mailboxes than the one on screen, until they list them (`pending`).
    pending: crate::pending::Pending,
    /// The formatted message editor (a web view), made the first time it is needed.
    editor: Option<std::rc::Rc<crate::editor::EditorView>>,
    /// No editor on this system: composers use the plain line fields.
    editor_failed: bool,
    /// The editor's page has loaded.
    editor_ready: bool,
    /// The editor page has drawn content since it was made: until then it stays hidden
    /// (its blank first frames would flash over the composer).
    editor_painted: bool,
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
    /// The details' copy buttons, opening (hovered) or closing, and since when, by key.
    copy_hover: HashMap<String, (bool, std::time::Instant)>,
    /// The details' chips' widths, and the room their lines have, as last laid out (design
    /// points), by key: the lines are broken from them (`reading::chip_lines`).
    chip_sizes: std::rc::Rc<std::cell::RefCell<HashMap<String, f32>>>,
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
    /// Where the editor was last placed in its window (its top left), for a menu opened
    /// from inside it.
    editor_origin: std::rc::Rc<std::cell::Cell<(f32, f32)>>,
    /// What each item of the open signature menu does, in order.
    signature_choices: Vec<SignatureChoice>,
    /// Connections to the signed-in accounts that are not the one shown (by account id).
    others: HashMap<String, Arc<mail::Client>>,
    /// Unread inbox conversations per account (for All inboxes and the Accounts list).
    account_unread: HashMap<String, u32>,
    /// Conversations open in windows of their own.
    message_windows: Vec<gpui::WindowHandle<MessageWindow>>,
    /// Archive or Mark as Read from a notification for an account not connected yet (the
    /// banner launched Nimbra): carried out once it is.
    pending_notes: Vec<(
        std::time::Instant,
        String,
        String,
        crate::integration::NoteAction,
    )>,
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
    /// The account shown: "" for the first one, else its id (see `mail::is_account_id`).
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
    /// Not sent, and neither Drafts nor this Mac has them: quitting would lose them.
    stranded: Vec<Arc<Job>>,
    /// The offline changes of accounts switched away from that had no store to keep them.
    homeless: HashMap<String, Vec<(String, mail::Change)>>,
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
    /// A newer Nimbra, downloaded and checked, waiting to replace this one (`update.rs`).
    update_ready: Option<String>,
    /// Quitting to restart into it: the new version opens once it is in place.
    restart_for_update: bool,
    /// This run is the first of a new version: the sidebar links to its release notes
    /// until dismissed.
    just_updated: Option<String>,
    /// A check for a new version is running (from the menu, Settings or the timer).
    update_checking: bool,
    /// How far the new version has got while it downloads and is checked (`update.rs`).
    update_progress: Option<crate::update::Progress>,
    /// The window showing `update_progress`, and where it was placed (`UpdatePlace`).
    update_window: Option<gpui::WindowHandle<UpdatePill>>,
    update_window_for: Option<UpdatePlace>,
    /// What the last check found, for Settings › About.
    update_status: Option<SharedString>,
    /// The address suggestions' window, where it was put, and for how many rows.
    /// The address suggestions' window, where it was put, its rows and its display.
    suggest_window: Option<suggest_popup::Shown>,
    /// The accounts whose first look for people to suggest runs (`contacts.rs`).
    contacts_scans: HashSet<String>,
    notice_shown: Option<SharedString>,
    notice_gen: usize,
    /// The search panel (opened with / or ⌘F): its field, the results so far (from the mail
    /// on this Mac, then Gmail's), the highlighted row (0 is "All results"), and whether
    /// Gmail is still being asked.
    finder_open: bool,
    finder: Entity<TextInput>,
    _finder_events: gpui::Subscription,
    finder_hits: Vec<Hit>,
    /// The mail on this Mac, ready to search, while the search panel is open.
    finder_index: Option<Arc<[finder::Searchable]>>,
    finder_sel: usize,
    finder_gen: u64,
    finder_loading: bool,
    /// The accounts the search panel is set to search, when picked there; otherwise where
    /// the window is decides (`search_scope_here`).
    finder_scope: Option<Scope>,
    /// The accounts the search on screen looks in.
    search_scope: Scope,
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
    /// The line over the list while mail is checked (`ui::LoadingLine`).
    checking_line: crate::ui::LoadingLine,
    /// The line under the reader's toolbar while its message loads.
    reader_line: crate::ui::LoadingLine,
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
    /// The other signed-in accounts' snoozes (each with its account), for Snoozed,
    /// Reminders and the rows' chips. Read from their stores (`read_snoozed_elsewhere`).
    snoozed_elsewhere: Vec<Snooze>,
    /// Each other account's own snoozed list as its store has it, and how often it was
    /// changed here (a read from before a change is not taken).
    snoozed_away: HashMap<String, (u64, Vec<Snooze>)>,
    /// The shown account's offline queue is being read from its store, and how many of its
    /// changes were made by a catch-up since that was asked for.
    outbox_pending: Option<usize>,
    /// Counts changes put in other accounts' queues, so that a catch-up that found its
    /// queue empty does not forget one put there since.
    away_queued: u64,
    /// A mailbox clicked whose stored list is still being read (`mailbox::Opening`).
    opening: Option<mailbox::Opening>,
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
    labels: Vec<mail::Label>,
    /// The check for mail that is running, and what it is for (see `fetching`).
    checks: Checks,
    /// The search field in the sidebar, and the query the results are for.
    search: Entity<TextInput>,
    query: String,
    _search_events: gpui::Subscription,
    /// The toolbar button under the mouse (what it does, its shortcut). Shown in the strip
    /// above an email in the original view, where a floating tooltip would be hidden
    /// behind the native web view.
    hint: Option<(&'static str, SharedString)>,
    /// Where a row's right-click menu opens (window coordinates), and what its items do.
    menu_at: (f32, f32),
    row_actions: Vec<RowAction>,
    /// How wide each message header's sender address could be when last drawn, by
    /// thread and message, so the address is cut with "…" to fit.
    address_room: std::rc::Rc<std::cell::RefCell<HashMap<String, f32>>>,
    /// Sidebar sections showing their hidden mailboxes for now.
    revealing: Vec<&'static str>,
    /// What each item of the open ⋯ menu does, in order.
    more_actions: Vec<menus::MoreAction>,
    /// What the open menu, the snooze panel and the label sheet were opened for. A refresh
    /// can move or remove rows under each, so they check before acting (see `still_meant`).
    menu_for: Option<MeantFor>,
    palette_for: Option<MeantFor>,
    label_for: Option<MeantFor>,
    /// The Add account sheet (an address that is not Gmail), when it is open.
    account_sheet: Option<AccountSheet>,
    /// Text selected in the reader.
    reader_selection: Option<ReaderSelection>,
    /// The reader's text blocks as last drawn, to find the one under the pointer while
    /// selecting.
    reader_layouts: std::rc::Rc<std::cell::RefCell<Vec<(usize, gpui::TextLayout)>>>,
    /// The labels the open menu lists, as it lists them.
    menu_labels: Vec<mail::Label>,
    /// When Gmail was last asked for the label list, and for which connection
    /// (`account_gen`): labels are made and renamed in other places too.
    labels_asked: Option<(std::time::Instant, u64)>,
    /// The "Undo" pill shown after a thread is removed, in a window of its own so it
    /// floats above the email's web view; `undo_gen` restarts its timer.
    undo_window: Option<gpui::WindowHandle<UndoToast>>,
    /// The pill in `undo_window` is a message's being sent: it turns into "Sent".
    undo_sending: bool,
    undo_gen: usize,
    /// The main window's frame, kept from the last draw, to place pop-ups without a window.
    frame: Option<gpui::Bounds<gpui::Pixels>>,
    /// The toolbar menu's own window, while it is open.
    menu_window: Option<gpui::WindowHandle<MenuPopup>>,
    /// The Snooze menu, while it is open (`snooze_menu.rs`).
    snooze_window: Option<gpui::WindowHandle<SnoozeMenu>>,
    /// Where the reader's Snooze button's menu drops from (its bottom-left), as last drawn.
    snooze_anchor: std::rc::Rc<std::cell::Cell<Option<gpui::Point<gpui::Pixels>>>>,
    /// The Snooze menu's "Only if nobody replies" switch, as it was last left.
    snooze_alert: bool,
    /// The time a day picked in the Snooze menu's calendar comes back at (hour, minute).
    snooze_time: (u32, u32),
    /// The day last picked in the snooze calendar, while the menu is open.
    snooze_day: Option<chrono::NaiveDate>,
    /// NIMBRA_SCREEN=snooze opens the Snooze menu once the window is drawn, and
    /// snooze-time its calendar with the hours open (screenshots).
    snooze_on_start: Option<bool>,
    /// NIMBRA_SCREEN=shortcuts opens the shortcuts sheet once the window is drawn.
    shortcuts_on_start: bool,
    /// NIMBRA_SCREEN=more: the reading pane's ⋯ menu opens once the window is up.
    more_on_start: bool,
    /// The open toolbar menu, if any.
    menu: Option<Menu>,
    /// The menu a click outside just closed, and when. Lets a click on the menu's own
    /// button close it instead of closing and instantly reopening it.
    menu_dismissed: Option<(Menu, std::time::Instant)>,
    /// Where the composer's From button is, in its window: left and bottom, for its menu.
    from_anchor: std::rc::Rc<std::cell::Cell<(f32, f32)>>,
    /// "Show original": the message's own HTML in a web view instead of the native reader.
    original: bool,
    /// Remote images allowed in the web view for the open thread. On by default; the
    /// strip above the message can block them.
    original_images: bool,
    /// The open thread's emails in their own formatting, one web view per open message.
    mail_views: crate::mail_views::MailViews,
    /// The hidden web view that print jobs are laid out in, always light.
    print_view: Option<HtmlView>,
    /// The thread the original view last showed, so another starts with its own images
    /// setting.
    html_shown: Option<String>,
    /// The message being written, if the composer is open.
    compose: Option<Composer>,
    /// Focus to hand over on the next frame (set where no window is at hand).
    pending_focus: Option<FocusHandle>,
    /// The list's Unread filter.
    unread_only: bool,
    /// What took each cleared thread out of the list, so undo can reverse it.
    removed_by: HashMap<String, mail::Change>,
    config: Option<gmail::Config>,
    /// Microsoft's sign-in (Outlook.com, Microsoft 365), when it is set up on this Mac.
    microsoft: Option<crate::microsoft::Config>,
    connection: Option<Arc<mail::Client>>,
    /// Shown in the sidebar footer.
    account: SharedString,
    me_initials: SharedString,
    /// The account's Google profile photo, shown in place of the initial.
    photos: HashMap<String, Arc<gpui::Image>>,
    /// Brand logos by sender domain (`bimi`), or that the domain has none.
    brand_logos: HashMap<String, Option<Arc<gpui::Image>>>,
    /// Domains whose logo is being looked up.
    brand_asking: HashSet<String>,
    /// Where logos are kept: one store for every account (`Store::open_shared`), opened on
    /// first need.
    brand_store: Option<Arc<crate::store::Store>>,
    /// What Google answered about each account's photo, once it has.
    photo_answers: HashMap<String, PhotoAnswer>,
    /// The accounts whose photo was asked for this session.
    photos_asked: HashSet<String>,
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
    /// Which pane Up, Down, Enter and Escape act in.
    pane: Pane,
    /// The sidebar row the keyboard is on (in `keyboard::Sidebar::rows`).
    sidebar_cursor: usize,
    /// The sidebar was hidden and came in for the keyboard: it goes again when left.
    sidebar_for_keys: bool,
    /// The far end of a Shift-arrow pick (a thread's index).
    pick_end: Option<usize>,
    /// The next frame scrolls the list to the open thread (or the pick's end), or the
    /// sidebar to its cursor: set when the keyboard moves them.
    reveal_row: std::rc::Rc<std::cell::Cell<bool>>,
    reveal_cursor: std::rc::Rc<std::cell::Cell<bool>>,
    /// The shortcuts window (?), while it is open.
    shortcuts_window: Option<gpui::WindowHandle<ShortcutsSheet>>,
    /// What the conversation is scrolled to, for Space.
    reader_scroll: gpui::ScrollHandle,
    /// The message list: shown or hidden, its width, and an in-progress resize
    /// (mouse x at the start, width at the start).
    list_open: bool,
    list_width: f32,
    list_grab: Option<(f32, f32)>,
    /// Dragging the sidebar's right edge: where the pointer started, and the width then.
    sidebar_grab: Option<(f32, f32)>,
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
    /// The open mailbox has to be brought up to date from the shown account's change feed,
    /// and that has not been done yet: it was busy loading, a message was being written, or
    /// the account was only just opened. Kept until it is done, so no new mail is left out.
    sync_owed: bool,
    /// The mailboxes whose next page is known, or known not to exist.
    paged: HashSet<View>,
    /// The search panel's results, kept scrolled to the highlighted one.
    finder_scroll: gpui::ScrollHandle,
    /// The search panel's row of accounts, scrolled sideways when they do not fit.
    finder_scope_scroll: gpui::ScrollHandle,
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
        let sample = !screen.is_empty() || std::env::var_os("NIMBRA_DEMO").is_some();
        let config = if sample { None } else { gmail::Config::load() };
        // Sample copies show both sign-in buttons, as a release build does: Microsoft's own
        // setting when it is there, otherwise a placeholder (signing in with it fails).
        let microsoft = crate::microsoft::Config::load().or_else(|| {
            sample
                .then(|| crate::microsoft::Config::new(crate::microsoft::Config::PLACEHOLDER))
                .flatten()
        });
        let (mail_views, mail_events) = crate::mail_views::MailViews::new();
        let mut settings = crate::settings::load();
        crate::ui::set_zoom(settings.zoom());
        crate::usage::opened(settings.usage_counts);
        // Screenshots show the sample account with signatures of its own.
        if !screen.is_empty() && settings.accounts.is_empty() && settings.signatures.is_empty() {
            sample_signatures(&mut settings);
        }
        // Without Google's sign-in set up, an account with its own servers still opens: the
        // sample mail is only for when there is nothing else to show.
        let demo =
            sample || (config.is_none() && !settings.accounts.iter().any(|a| a.servers.is_some()));
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
            InputEvent::Tab if this.several_accounts() => this.step_scope(1, cx),
            InputEvent::BackTab if this.several_accounts() => this.step_scope(-1, cx),
            InputEvent::Down | InputEvent::Tab => this.finder_step(1, cx),
            InputEvent::Up => this.finder_step(-1, cx),
            InputEvent::Enter => this.finder_open_selected(cx),
            _ => {}
        });
        // As it was left. NIMBRA_SIDEBAR=closed or open overrides that (screenshots).
        let sidebar_open = match std::env::var("NIMBRA_SIDEBAR").as_deref() {
            Ok("closed") => false,
            Ok("open") => true,
            _ => !settings.sidebar_auto_hide,
        };
        // As it was last dragged to; NIMBRA_LIST_WIDTH=<px> overrides it (screenshots).
        let list_width = std::env::var("NIMBRA_LIST_WIDTH")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(settings.list_width)
            .clamp(LIST_MIN, LIST_MAX);
        let mut this = Self {
            inbox_saved_empty: false,
            all_inboxes_whole: false,
            all_inboxes_saved_empty: false,
            all_inboxes_reached: false,
            all_next: HashMap::new(),
            shifted: HashSet::new(),
            other_stores: HashMap::new(),
            away_waiting: HashSet::new(),
            inboxes_saved_empty: HashMap::new(),
            keychain: Underway::default(),
            keychain_slow: false,
            arrival: None,
            sky: None,
            sky_timer: None,
            zero_seen: None,
            day_ends: None,
            day_timer: None,
            finder_open: false,
            finder,
            _finder_events: finder_events,
            finder_hits: Vec::new(),
            finder_index: None,
            finder_sel: 0,
            finder_gen: 0,
            finder_loading: false,
            finder_scope: None,
            search_scope: Scope::All,
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
                Ok("reminders") => View::Reminders,
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
            stranded: Vec::new(),
            homeless: HashMap::new(),
            steps: Steps::default(),
            must_show: None,
            saves: Underway::default(),
            writes: Underway::default(),
            kept_unread: None,
            attachment_busy: HashMap::new(),
            attachment_saved: HashMap::new(),
            notice_window: None,
            suggest_window: None,
            contacts_scans: HashSet::new(),
            update_ready: crate::update::staged(),
            restart_for_update: false,
            just_updated: None,
            update_checking: false,
            update_progress: None,
            update_window: None,
            update_window_for: None,
            update_status: None,
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
            checking_line: Default::default(),
            reader_line: Default::default(),
            badge: None,
            print_pending: None,
            login_item: false,
            picked: Default::default(),
            history_groups: Vec::new(),
            snoozed: Vec::new(),
            snoozed_elsewhere: Vec::new(),
            snoozed_away: HashMap::new(),
            outbox_pending: None,
            away_queued: 0,
            opening: None,
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
            editor_painted: false,
            editor_shown: None,
            editor_theme: None,
            editor_focus: cx.focus_handle(),
            editor_keys: false,
            settings,
            settings_window: None,
            message_windows: Vec::new(),
            pending_notes: Vec::new(),
            others: HashMap::new(),
            account_unread: HashMap::new(),
            every_labels: Vec::new(),
            account_labels: HashMap::new(),
            restore_place: crate::place::load(),
            place_saved: None,
            place_waiting: false,
            account_counts: HashMap::new(),
            compose_host: None,
            format_hint: None,
            compose_detached: false,
            compose_home: None,
            compose_ask: false,
            compose_ask_at: None,
            editor_window: None,
            editor_origin: Default::default(),
            signature_choices: Vec::new(),
            copied: None,
            copy_hover: HashMap::new(),
            chip_sizes: Default::default(),
            parked: Vec::new(),
            composer_serial: 0,
            scheduled: Vec::new(),
            sending_scheduled: HashSet::new(),
            schedule_watch: false,
            send_later_times: Vec::new(),
            details: None,
            folds: Default::default(),
            drafts_ready: HashMap::new(),
            drafts_fetching: Default::default(),
            draft_wanted: None,
            closed_drafts: Vec::new(),
            hidden_drafts: HashMap::new(),
            latest_folded: Default::default(),
            fold_heights: Default::default(),
            fold_line: std::rc::Rc::new(std::cell::Cell::new(53.)),
            fold_lead: Default::default(),
            display: None,
            pending: Default::default(),
            store: None,
            notify_from: None,
            synced_to: HashMap::new(),
            counts: HashMap::new(),
            labels: Vec::new(),
            checks: Checks::default(),
            hint: None,
            menu_at: (0., 0.),
            row_actions: Vec::new(),
            more_actions: Vec::new(),
            revealing: Vec::new(),
            address_room: Default::default(),
            menu_for: None,
            palette_for: None,
            label_for: None,
            menu_labels: Vec::new(),
            labels_asked: None,
            account_sheet: None,
            reader_selection: None,
            reader_layouts: Default::default(),
            menu_window: None,
            snooze_window: None,
            snooze_anchor: Default::default(),
            snooze_alert: false,
            snooze_time: (9, 0),
            snooze_day: None,
            shortcuts_on_start: screen == "shortcuts",
            more_on_start: screen == "more",
            snooze_on_start: match screen.as_str() {
                "snooze" => Some(false),
                "snooze-time" => Some(true),
                _ => None,
            },
            undo_window: None,
            undo_sending: false,
            undo_gen: 0,
            frame: None,
            menu: None,
            menu_dismissed: None,
            from_anchor: Default::default(),
            original: false,
            original_images: true,
            mail_views,
            html_shown: None,
            print_view: None,
            compose: None,
            pending_focus: None,
            unread_only: false,
            removed_by: HashMap::new(),
            config,
            microsoft,
            connection: None,
            account: if demo {
                "Luan Gjokaj".into()
            } else {
                "Gmail".into()
            },
            me_initials: if demo { "LG".into() } else { "ME".into() },
            photos: HashMap::new(),
            brand_logos: HashMap::new(),
            brand_asking: HashSet::new(),
            brand_store: None,
            photo_answers: HashMap::new(),
            photos_asked: HashSet::new(),
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
            list_width,
            list_grab: None,
            sidebar_grab: None,
            list_toggled: None,
            list_gen: 0,
            list_scroll: gpui::ScrollHandle::new(),
            sidebar_scroll: gpui::ScrollHandle::new(),
            checked: HashMap::new(),
            counted: None,
            feed_changes: 0,
            sync_owed: false,
            paged: HashSet::new(),
            finder_scroll: gpui::ScrollHandle::new(),
            finder_scope_scroll: gpui::ScrollHandle::new(),
            pull: 0.,
            pull_armed: false,
            pull_anim: None,
            pull_gen: 0,
            pull_hold: false,
            pull_done: false,
            sidebar_open,
            pane: Pane::List,
            sidebar_cursor: 0,
            sidebar_for_keys: false,
            pick_end: None,
            reveal_row: Default::default(),
            reveal_cursor: Default::default(),
            shortcuts_window: None,
            reader_scroll: gpui::ScrollHandle::new(),
            reload_gen: 0,
            images: HashMap::new(),
            image_requests: HashSet::new(),
            back: Vec::new(),
            forward: Vec::new(),
        };
        // NIMBRA_SCREEN=compose opens a new message on the sample data (screenshots), as
        // Compose does.
        if screen == "compose" || screen == "compose-full" {
            this.open_compose(ComposeKind::New, cx);
            if let Some(composer) = &mut this.compose {
                composer.full = screen == "compose-full";
            }
        }
        // NIMBRA_SCREEN=offline: as if the network were gone (screenshots).
        // NIMBRA_SCREEN=up-to-date: Check for Updates, a moment after the window is up (the
        // sample checks, then finds nothing newer).
        if screen == "up-to-date" {
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(800))
                    .await;
                // It shows while Nimbra is in front, so it comes to the front.
                this.update(cx, |this, cx| {
                    cx.activate(true);
                    this.check_for_updates(cx);
                })
                .ok();
            })
            .detach();
        }
        if screen == "offline" {
            this.offline = true;
        }
        // NIMBRA_SCREEN=window: the open thread in a window of its own (screenshots).
        if screen == "window" {
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(800))
                    .await;
                this.update(cx, |this, cx| this.open_message_window(this.selected, cx))
                    .ok();
            })
            .detach();
        }
        // NIMBRA_SCREEN=welcome: what a new user sees first (signed out, no account), with
        // nothing read from the keychain (screenshots).
        if screen == "welcome" {
            this.status = Status::SignedOut;
            this.mails.clear();
            this.settings.accounts.clear();
        }
        // NIMBRA_SCREEN=compose-suggest: a new message with "j" typed in To, so the address
        // suggestions show (screenshots).
        if screen == "compose-suggest" {
            this.open_compose(ComposeKind::New, cx);
            // Once the sample mail, where addresses are suggested from, is in.
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                this.update(cx, |this, cx| {
                    if let Some(to) = this.compose.as_ref().map(|c| c.to.clone()) {
                        to.update(cx, |input, cx| input.set("j".into(), usize::MAX, cx));
                        this.pending_focus = Some(to.read(cx).focus_handle(cx));
                        this.field_changed(Field::To, cx);
                    }
                })
                .ok();
            })
            .detach();
        }
        // Mail opens in its original formatting by default; NIMBRA_READER=1 starts in the
        // native reader instead. Messages without HTML always use the reader.
        this.original = this.settings.original_view && std::env::var_os("NIMBRA_READER").is_none();
        this.original_images = this.settings.load_images;
        // Each email's height, as its page works it out (`mail_views.rs`).
        cx.spawn(async move |this, cx| {
            use futures::StreamExt as _;
            let mut events = mail_events;
            while let Some(event) = events.next().await {
                let changed = this
                    .update(cx, |this, _| this.mail_views.handle(event))
                    .unwrap_or(false);
                if changed {
                    this.update(cx, |_, cx| cx.notify()).ok();
                }
            }
        })
        .detach();
        // NIMBRA_SCREEN=draft opens a sample draft in the composer (screenshots).
        if screen == "draft" {
            this.edit_draft(
                mail::Draft {
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
                None,
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
        // NIMBRA_SCREEN=update: a new version is ready, with its card (screenshots).
        if screen == "update" {
            this.update_ready = Some("0.1.4".into());
            this.show_update_ready("0.1.4", cx);
        }
        // NIMBRA_SCREEN=updated: the first run after an update, with the sidebar's link to
        // the release notes (screenshots).
        if screen == "updated" {
            this.just_updated = Some(env!("CARGO_PKG_VERSION").into());
        }
        // NIMBRA_SCREEN=update-download: a new version downloads, is checked, and is ready,
        // with nothing fetched (screenshots).
        if screen == "update-download" {
            this.pretend_update_download(cx);
        }
        // NIMBRA_SCREEN=about opens the About panel (screenshots).
        if screen == "about" {
            cx.defer(|_| crate::about::show());
        }
        // NIMBRA_SCREEN=settings (or settings-<section>, see `settings_window::SCREENS`)
        // opens the Settings window on that section (screenshots). settings-account opens
        // it on Add account, settings-remove asks to remove an account.
        if let Some(tab) = settings_window::screen_tab(&screen) {
            let form = screen == "settings-account";
            // After the main window is up.
            cx.spawn(async move |this, cx| {
                this.update(cx, |this, cx| {
                    if form {
                        this.open_account_form_in_settings(cx);
                    }
                    this.open_settings_on(Some(tab), None, cx)
                })
                .ok();
            })
            .detach();
        }
        // NIMBRA_SCREEN=account (and account-servers, with the server fields) opens the
        // Add account sheet (screenshots).
        if screen == "account" || screen == "account-servers" {
            let servers = screen == "account-servers";
            cx.spawn(async move |this, cx| {
                this.update(cx, |this, cx| {
                    this.open_account_sheet(None, cx);
                    if servers {
                        this.show_account_servers(cx);
                    }
                })
                .ok();
            })
            .detach();
        }
        // NIMBRA_SCREEN=loading holds the loading skeleton on screen (screenshots).
        if screen == "loading" {
            this.status = Status::Loading;
            this.checks.start(View::Inbox, "");
        }
        // Listed before a preview can be made, so that none made since is removed.
        let previews = previews_now();
        cx.background_spawn(async move { remove_previews(previews) })
            .detach();
        this.watch_for_mail(cx);
        this.watch_other_accounts(cx);
        this.watch_system(cx);
        // Every way out: ⌘Q, the Dock, logging out.
        cx.on_app_quit(|this, cx| {
            this.finish_before_quit(cx);
            // A new version waiting takes this one's place once it has quit.
            if this.update_ready.is_some() && crate::update::enabled() {
                crate::update::install_on_quit(this.restart_for_update);
            }
            async {}
        })
        .detach();
        this.watch_for_updates(cx);
        this.greet_new_version(cx);
        // Already signed in on a previous run: go straight to the inbox. The keychain is
        // read off the main thread because macOS may show a permission prompt for it, and
        // the window should not freeze (or fail to appear) while that is open.
        let active = this.settings.active.clone();
        if mail::is_account_id(&active)
            && (active.is_empty() || this.settings.accounts.iter().any(|a| a.id == active))
        {
            this.account_id = active;
        }
        // No Google sign-in to use: show an account with its own servers instead.
        if this.config.is_none()
            && !this.is_demo()
            && this.servers_of(&this.account_id).is_none()
            && let Some(id) = this
                .settings
                .accounts
                .iter()
                .find(|a| a.servers.is_some())
                .map(|a| a.id.clone())
        {
            this.account_id = id;
        }
        if this.config.is_some() || (!this.is_demo() && this.own_servers()) {
            this.status = Status::Loading;
            this.inbox_saved_empty =
                crate::store::Store::saved_empty(&this.account_id, &record(View::Inbox));
            if this.opening_to_zero() && !crate::system::reduces_motion() {
                this.arrival = Some(Arrival::START);
            }
            // On every change, whichever action made it: the next launch goes by this.
            cx.observe_self(|this, cx| {
                let now = this.zero_inputs();
                if this.zero_seen.as_ref() != Some(&now) {
                    this.zero_seen = Some(now);
                    this.save_if_zero_changed(cx);
                }
            })
            .detach();
            this.connect(false, cx);
        }
        // NIMBRA_SEARCH=<query> starts with that search already run.
        if let Ok(query) = std::env::var("NIMBRA_SEARCH") {
            this.search
                .update(cx, |input, cx| input.set(query, usize::MAX, cx));
            this.run_search(cx);
        }
        // NIMBRA_SCREEN=finder: the search panel open (holding NIMBRA_SEARCH, if given).
        if screen == "finder" {
            this.focus_search(cx);
        }
        this
    }

    /// Moves the keyboard to `focus`. GPUI's focus is its own: an email (a web view) that was
    /// clicked keeps the system's keyboard, and typing would go there, even with the email
    /// hidden under the search panel. So the email gives it back, unless the message editor
    /// (a web view too) is what gets it.
    fn apply_focus(&self, focus: &FocusHandle, window: &mut Window) {
        window.focus(focus);
        if *focus != self.editor_focus {
            self.mail_views.release_keyboard();
        }
    }

    fn is_demo(&self) -> bool {
        self.status == Status::Demo
    }

    fn inbox_name(&self) -> SharedString {
        self.title(self.mailbox)
    }

    fn title(&self, view: View) -> SharedString {
        match view {
            View::ALL_INBOXES => "All inboxes".into(),
            // Every account's: named as each account's is, a label by its shared name.
            View::Every(_) => match view.base() {
                View::Label(index) => self
                    .every_labels
                    .get(index)
                    .map(|label| SharedString::from(label.name.clone()))
                    .unwrap_or_else(|| "Label".into()),
                base => self.title(base),
            },
            View::Inbox if self.is_demo() => "Important".into(),
            View::Inbox => "Inbox".into(),
            View::Primary => "Primary".into(),
            View::Updates => "Updates".into(),
            View::Promotions => "Promotions".into(),
            View::Social => "Social".into(),
            View::Starred => "Starred".into(),
            View::Snoozed => "Snoozed".into(),
            View::Reminders => "Reminders".into(),
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

    fn set_hint(
        &mut self,
        hovered: bool,
        label: &'static str,
        key: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        if hovered {
            self.hint = Some((label, key.into()));
        } else if self
            .hint
            .as_ref()
            .is_some_and(|(current, _)| *current == label)
        {
            // Only clear our own hint: moving straight to the next button sets its hint
            // first, and that one must stay.
            self.hint = None;
        }
        cx.notify();
    }

    /// The store record (and sync point) of a mailbox. Labels are kept by their Gmail id,
    /// so creating or deleting a label never mixes up what another label holds.
    fn record_for(&self, view: View) -> String {
        let label_id = match view {
            View::Label(index) => self.labels.get(index).map(|label| label.id.as_str()),
            _ => None,
        };
        nimbra_core::listing::record(view, label_id)
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
                "They are deleted from the server for good. This can’t be undone.".into(),
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
                    .w(dp(440.))
                    .flex()
                    .flex_col()
                    .gap(dp(12.))
                    .p(dp(22.))
                    .rounded(dp(18.))
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
                            .gap(dp(8.))
                            .pt(dp(6.))
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
        let side_now = part(self.sidebar_open, self.sidebar_toggled, self.side_w());
        let list_now = part(self.list_open, self.list_toggled, self.list_width);
        let end = if self.sidebar_open { self.side_w() } else { 0. }
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
            let size = gpui::size(to_screen(900.), to_screen(780.));
            let mut bounds = gpui::Bounds::centered(None, size, cx);
            bounds.origin.x += to_screen(offset);
            bounds.origin.y += to_screen(offset);
            let me = owner.clone();
            let opened = cx.open_window(
                gpui::WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some(mail.subject.clone().into()),
                        appears_transparent: true,
                        traffic_light_position: Some(gpui::point(px(18.), px(18.))),
                    }),
                    window_min_size: Some(gpui::size(to_screen(520.), to_screen(420.))),
                    ..Default::default()
                },
                |window, cx| {
                    // Not seen until its emails are drawn (`MessageWindow::reveal`), so it
                    // opens whole, as the main window shows an email.
                    crate::blur::set_alpha(window, 0.);
                    crate::blur::track_pointer_always(window);
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
                // Shown by now whatever it drew (`MessageWindow::reveal`).
                cx.spawn(async move |cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(900))
                        .await;
                    handle
                        .update(cx, |view, window, _| view.reveal_now(window))
                        .ok();
                })
                .detach();
            }
        });
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

    /// ⌘⇧L and the sun/moon button: the other appearance, or following the Mac again
    /// when that is what it shows (`system_events::flipped`).
    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        let mac_dark = if self.settings.theme == crate::settings::ThemeChoice::System {
            self.dark
        } else {
            crate::integration::mac_is_dark()
        };
        self.set_theme(system_events::flipped(self.dark, mac_dark), cx);
    }

    /// The first screen's hello: played once, and remembered so it never plays again.
    fn play_welcome(&mut self, cx: &mut Context<Self>) {
        if self.settings.welcomed {
            return;
        }
        self.settings.welcomed = true;
        crate::notify::welcome_sound();
        self.save_settings(cx);
    }

    /// Keeps the list's width as it was dragged to, for the next launch.
    pub(super) fn keep_list_width(&mut self, cx: &mut Context<Self>) {
        if self.settings.list_width != self.list_width {
            self.settings.list_width = self.list_width;
            self.save_settings(cx);
        }
    }

    fn save_settings(&self, _cx: &mut Context<Self>) {
        // Written on the settings thread, newest last; quitting waits for it.
        if crate::settings::save(&self.settings).is_err() {
            eprintln!("nimbra: could not save settings");
        }
    }

    /// The accounts that can have signatures, as (id, address): every account, or with
    /// none, the sample one.
    pub(super) fn signature_accounts(&self) -> Vec<(String, String)> {
        if self.settings.accounts.is_empty() && self.is_demo() {
            return vec![(self.account_id.clone(), "luan@example.com".into())];
        }
        self.settings
            .accounts
            .iter()
            .map(|a| (a.id.clone(), a.email.clone()))
            .collect()
    }

    /// ⌘, or the account menu: the Settings window (brought forward if already open).
    fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.open_settings_on(None, None, cx);
    }

    /// The Settings window, on `tab` (and for Composing, on `account`'s signatures) when
    /// one is given; brought forward as it is if already open and none is.
    fn open_settings_on(
        &mut self,
        tab: Option<settings_window::Tab>,
        account: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        self.palette = false;
        self.login_item = crate::system::opens_at_login();
        if let Some(handle) = &self.settings_window
            && handle
                .update(cx, |settings, window, cx| {
                    if let Some(tab) = tab {
                        settings.show(tab, account.clone(), cx);
                    }
                    window.activate_window()
                })
                .is_ok()
        {
            return;
        }
        let owner = cx.entity();
        let dark = self.dark;
        // Opened once this update is over: the new window reads the app as it draws.
        cx.defer(move |cx| {
            let bounds =
                gpui::Bounds::centered(None, gpui::size(to_screen(860.), to_screen(620.)), cx);
            let me = owner.clone();
            let opened = cx.open_window(
                gpui::WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("Settings".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(gpui::point(px(16.), px(16.))),
                    }),
                    window_min_size: Some(gpui::size(to_screen(720.), to_screen(480.))),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|cx| {
                        let mut settings = SettingsWindow::new(owner, dark, cx);
                        if let Some(tab) = tab {
                            settings.show(tab, account, cx);
                        }
                        settings
                    })
                },
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

    fn fail(&mut self, error: anyhow::Error, cx: &mut Context<Self>) {
        // Offline is a state, not an error: keep showing what is here, and catch up later.
        if error.is::<mail::Offline>() {
            if self.mails.is_empty() && !matches!(self.status, Status::Ready) {
                self.status = Status::Failed(error.to_string().into());
            }
            self.went_offline(cx);
            cx.notify();
            return;
        }
        eprintln!("nimbra: {error}");
        if let Some(out) = error.downcast_ref::<mail::SignedOut>()
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
            self.connection = None;
            // The stored mail belongs to that sign-in: remove it, and its key.
            self.store = None;
            self.checks.forget();
            self.offline = false;
            self.stop_retrying();
            // Whoever signs in next must not send this account's mail.
            self.outbox = Outbox::default();
            self.outbox_pending = None;
            self.inboxes_saved_empty.remove(&self.account_id);
            let shown = self.account_id.clone();
            self.forget_photo(&shown);
            self.abandon_sends(&shown);
            self.parked
                .retain(|(_, c)| c.account.as_ref().is_some_and(|a| *a != shown));
            self.next_pages.clear();
            self.forget_checked();
            let account = self.account_id.clone();
            std::thread::spawn(move || {
                crate::store::wipe(&account);
                forget_previews(&account);
            });
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
            self.notice = Some(if self.is_microsoft(&self.account_id) {
                "Your Microsoft sign-in expired. Sign in again to continue.".into()
            } else if self.servers_of(&self.account_id).is_some() {
                "The server stopped taking this account’s password. Sign in again to continue."
                    .into()
            } else {
                "Your Google sign-in expired. Connect again to continue.".into()
            });
        // An error is no reason to hide a mailbox known to be empty.
        } else if self.mails.is_empty() && self.status != Status::Ready {
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
        let query = self.demo_query();
        (0..self.mails.len())
            .filter(|&i| self.shows(i, &query))
            .collect()
    }

    /// Whether any thread is listed, without collecting them.
    fn any_visible(&self) -> bool {
        let query = self.demo_query();
        (0..self.mails.len()).any(|i| self.shows(i, &query))
    }

    /// The sample data is searched locally, for this.
    fn demo_query(&self) -> String {
        if self.is_demo() && self.mailbox == View::Search {
            self.query.to_lowercase()
        } else {
            String::new()
        }
    }

    fn shows(&self, i: usize, demo_query: &str) -> bool {
        match (self.is_demo(), self.mailbox) {
            (true, View::Done) => self.done[i],
            (true, View::Search) => {
                let mail = &self.mails[i];
                !self.done[i]
                    && [&mail.sender, &mail.subject, &mail.snippet]
                        .iter()
                        .any(|field| field.to_lowercase().contains(demo_query))
            }
            (true, View::Inbox | View::ALL_INBOXES) | (false, _) => {
                // The open thread stays listed under Unread even once it is read.
                !self.done[i] && (!self.unread_only || self.mails[i].unread || i == self.selected)
            }
            (true, _) => false,
        }
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.palette {
            let max = self.palette_len() as isize - 1;
            self.palette_sel = (self.palette_sel as isize + delta).clamp(0, max) as usize;
        } else if self.pane == Pane::Sidebar {
            let len = self.sidebar_now().rows().len();
            self.sidebar_cursor = keyboard::step(self.sidebar_cursor, delta, len);
            self.reveal_cursor.set(true);
        } else {
            self.picked.clear();
            self.pick_end = None;
            let visible = self.visible();
            if let Some(pos) = visible.iter().position(|i| *i == self.selected) {
                let next = keyboard::step(pos, delta, visible.len());
                self.open(visible[next]);
                self.open_selected(cx);
                if next == 0 {
                    // The top, with the heading above the first thread.
                    self.list_scroll.set_offset(gpui::point(px(0.), px(0.)));
                } else {
                    self.reveal_row.set(true);
                }
            }
        }
        cx.notify();
    }

    /// Home and End: the first or last thread, palette row or sidebar row.
    fn step_to_end(&mut self, last: bool, cx: &mut Context<Self>) {
        let far = if last { isize::MAX / 2 } else { isize::MIN / 2 };
        self.step(far, cx);
    }

    /// G then a letter, from anywhere in the window.
    fn go_to(&mut self, view: View, cx: &mut Context<Self>) {
        if self.window_covered() {
            return;
        }
        self.leave_sidebar(cx);
        // Under All inboxes, to every account's: G then T goes to every account's Trash.
        let view = if self.every_mode() {
            View::every(view)
        } else {
            view
        };
        self.select_mailbox(view, cx);
    }

    /// ⌘1 to ⌘9.
    fn go_number(&mut self, n: usize, cx: &mut Context<Self>) {
        if self.window_covered() {
            return;
        }
        let Some(row) = n
            .checked_sub(1)
            .and_then(|i| self.sidebar_now().numbered().get(i).cloned())
        else {
            return;
        };
        self.leave_sidebar(cx);
        self.go_to_row(row, cx);
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
        .text_size(dp(size))
        .line_height(dp((size * 1.14).round()))
        .text_color(color)
        .child(StyledText::new(text.to_string()).with_highlights([(start..text.len(), italic)]))
}

/// Shortens text to `max` characters with an ellipsis.
///
/// Used instead of GPUI's own ellipsis truncation, which in 0.2.2 can panic when the cut
/// lands inside a multi-byte character such as an emoji. The element still clips whatever
/// does not fit.
/// Two signatures for the sample account (whose id is ""): a short formatted one, the
/// default, and one in HTML with a logo, a table and colours.
fn sample_signatures(settings: &mut crate::settings::Settings) {
    let simple = settings.add_signature("");
    let studio = settings.add_signature("");
    for (id, name, html) in [
        (
            simple,
            "Work",
            "<div><b>Luan Gjokaj</b></div><div>UX Engineer &amp; Designer</div>\
             <div><a href=\"https://nimbra.me\">nimbra.me</a></div>",
        ),
        (
            studio,
            "Studio",
            "<table cellpadding=\"0\" cellspacing=\"0\"><tr>\
             <td style=\"padding-right:14px\"><img src=\"https://nimbra.me/icons/android-chrome-192x192.png\" alt=\"Nimbra logo\" width=\"48\" height=\"48\"></td>\
             <td style=\"border-left:2px solid #8b7cff;padding-left:14px\">\
             <div style=\"font-weight:600;color:#16151c\">Luan Gjokaj</div>\
             <div style=\"color:#6b6878\">Design, Nimbra Studio</div>\
             <div><a href=\"https://nimbra.me\">nimbra.me</a> · <a href=\"mailto:luan@example.com\">luan@example.com</a></div>\
             </td></tr></table>",
        ),
    ] {
        let Some(signature) = id
            .as_deref()
            .and_then(|id| settings.signatures.iter_mut().find(|s| s.id == id))
        else {
            continue;
        };
        signature.name = name.into();
        signature.html = crate::signature::clean(html);
    }
}

fn short(text: &str, max: usize) -> SharedString {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{}…", head.trim_end()).into()
    } else {
        head.into()
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
/// The mailboxes that are an inbox, one account's or every account's: Done archives there,
/// Snooze and Move to take the thread out.
fn inbox_like(view: View) -> bool {
    nimbra_core::listing::inbox_like(view)
}

/// A stable number per mailbox, for element ids (the one its store record goes by).
fn view_key(view: View) -> usize {
    nimbra_core::listing::view_number(view)
}

fn card(t: &Theme) -> Div {
    div()
        .bg(t.hover)
        .border_1()
        .border_color(t.border)
        .rounded(dp(12.))
}

// ---------------------------------------------------------------- panes

impl Nimbra {
    /// Shown in place of the mail panes while signed out, signing in, loading or failed.
    fn status_screen(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let servers = self.servers_of(&self.account_id);
        let own_servers = servers.is_some();
        let gmail_password = servers.is_some_and(|s| s.is_gmail());
        let microsoft = self.is_microsoft(&self.account_id);
        let (title, message, action): (&str, SharedString, Option<&str>) = match &self.status {
            Status::SignedOut if microsoft => (
                "Sign in again",
                "Microsoft ended this account’s sign-in. Nimbra opens Microsoft in your \
                 browser to sign in again."
                    .into(),
                Some("Sign in"),
            ),
            Status::SigningIn if microsoft => (
                "Waiting for Microsoft",
                "Finish signing in in your browser, then come back here.".into(),
                None,
            ),
            Status::SignedOut if gmail_password => (
                "Sign in again",
                "Google stopped taking this account’s app password. Sign in again with a new \
                 one."
                    .into(),
                Some("Sign in"),
            ),
            Status::SignedOut if own_servers => (
                "Sign in again",
                "The mail server stopped taking this account’s password. Sign in again, or \
                 use an app password if the account has two-step sign-in."
                    .into(),
                Some("Sign in"),
            ),
            Status::SignedOut => (
                "Connect your inbox",
                "Nimbra opens Google in your browser to sign in. It can read and archive \
                 your mail, and send what you write. Deleting mail forever asks Google for \
                 your permission first."
                    .into(),
                Some("Connect Gmail"),
            ),
            Status::SigningIn => (
                "Waiting for Google",
                "Finish signing in in your browser, then come back here.".into(),
                None,
            ),
            Status::Failed(reason) => ("Something went wrong", reason.clone(), Some("Try again")),
            _ if self.waiting_for_keychain() => {
                ("Waiting for the Keychain", KEYCHAIN_WAITING.into(), None)
            }
            _ => (
                "Fetching your inbox",
                "This takes a few seconds.".into(),
                None,
            ),
        };
        // The first thing a new user sees (signed out, no account yet): the icon, and the
        // screen coming in one part after another.
        let welcome = self.status == Status::SignedOut && self.settings.accounts.is_empty();
        let motion = welcome && !crate::system::reduces_motion();
        // A new user's very first look: a hello with the animation (once, ever).
        if welcome && !self.settings.welcomed {
            cx.spawn(async move |this, cx| {
                this.update(cx, |this, cx| this.play_welcome(cx)).ok();
            })
            .detach();
        }
        let enter = |part: usize, element: AnyElement| -> AnyElement {
            if motion {
                welcome_entrance(part, element)
            } else {
                element
            }
        };
        let icon = welcome.then(|| enter(0, welcome_icon(t, motion)));
        let action_row = action.map(|label| {
            div()
                .pt(dp(10.))
                .flex()
                .gap(dp(8.))
                .child(crate::ui::glow(
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
                    "status-action",
                    true,
                ))
                // Signed out of Gmail (or never in): any other address works too.
                .when(self.status == Status::SignedOut && !own_servers, |d| {
                    d.child(
                        button(t, ButtonKind::Secondary, "Use another address", None)
                            .id("status-other")
                            .press_soft()
                            .on_click(
                                cx.listener(|this, _, _, cx| this.open_account_sheet(None, cx)),
                            ),
                    )
                })
                .into_any_element()
        });
        div()
            .flex_1()
            .min_w(dp(0.))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(dp(14.))
            .bg(t.surface)
            .children(icon)
            .child(enter(1, serif(title, 56., t.text).into_any_element()))
            .child(enter(
                2,
                div()
                    .max_w(dp(560.))
                    .text_center()
                    .text_size(dp(15.))
                    .line_height(dp(24.))
                    .text_color(t.text2)
                    .child(message)
                    .into_any_element(),
            ))
            .children(action_row.map(|row| enter(3, row)))
    }
}

/// How long the welcome screen takes to come in, and how far apart its parts start.
const WELCOME_IN: Duration = Duration::from_millis(2000);
const WELCOME_PART: f32 = 0.12;
/// One part's own rise, as a share of `WELCOME_IN`.
const WELCOME_RISE: f32 = 0.6;

/// A part of the welcome screen rising into place and fading in, after the ones above it.
fn welcome_entrance(part: usize, element: AnyElement) -> AnyElement {
    div()
        .relative()
        .child(element)
        .with_animation(
            ("welcome-in", part),
            Animation::new(WELCOME_IN),
            move |d, delta| {
                let start = part as f32 * WELCOME_PART;
                let local = ((delta - start) / WELCOME_RISE).clamp(0., 1.);
                // Ease out (cubic): it rises from a little way down and settles gently,
                // fading in on a slower curve (smoothstep) so it never flashes up.
                let eased = 1. - (1. - local).powi(3);
                let shown = local * local * (3. - 2. * local);
                d.opacity(shown).top(dp(24. * (1. - eased)))
            },
        )
        .into_any_element()
}

/// The app icon over a soft glow that slowly breathes (still when the Mac asks for less
/// motion).
fn welcome_icon(t: &Theme, motion: bool) -> AnyElement {
    const SIZE: f32 = 112.;
    let image = gpui::img(if t.dark {
        settings_window::APP_ICON_DARK.clone()
    } else {
        settings_window::APP_ICON.clone()
    })
    .size_full();
    let glow = |level: f32| shadow(c(0x8b7cff, 0.28 + 0.2 * level), 16., 40. + 16. * level, -8.);
    let icon = div()
        .size(dp(SIZE))
        .rounded(dp(SIZE / 4.))
        .shadow(vec![glow(0.)])
        .child(image);
    let icon = if motion {
        icon.with_animation(
            "welcome-glow",
            Animation::new(Duration::from_millis(4200)).repeat(),
            move |d, delta| {
                // Up and down once a cycle, smoothly.
                let level = 0.5 - 0.5 * (delta * std::f32::consts::TAU).cos();
                d.shadow(vec![glow(level)])
            },
        )
        .into_any_element()
    } else {
        icon.into_any_element()
    };
    div().pb(dp(10.)).child(icon).into_any_element()
}

impl Render for Nimbra {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The window the notices are attached to (`popup::enter`), and its display, which
        // they open on as they are placed by its frame.
        crate::popup::note_main(window);
        self.display = crate::popup::display_of(window, cx);
        // Every window draws at the interface zoom.
        crate::ui::apply_zoom(window);
        if std::mem::take(&mut self.more_on_start) {
            cx.defer_in(window, |this, window, cx| {
                this.toggle_menu(Menu::More, window, cx)
            });
        }
        if std::mem::take(&mut self.shortcuts_on_start) {
            cx.defer_in(window, |this, _, cx| this.open_shortcuts(cx));
        }
        self.save_place(cx);
        if let Some(hours) = self.snooze_on_start.take() {
            cx.defer_in(window, move |this, window, cx| {
                let mode = if hours {
                    let today = chrono::Local::now().date_naive();
                    snooze_menu::Mode::Calendar(
                        chrono::Datelike::with_day(&today, 1).unwrap_or(today),
                        true,
                    )
                } else {
                    snooze_menu::Mode::List
                };
                this.open_snooze_in(None, mode, window, cx)
            });
        }
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
            self.apply_focus(&focus, window);
        }
        let t = self.theme();
        let viewport = window.viewport_size();
        let viewport = (from_screen(viewport.width), from_screen(viewport.height));
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
        self.roll_day(cx);
        self.sync_html_view(window);
        if !composing_elsewhere {
            self.sync_editor(window, cx);
        }
        // Nothing to suggest any more (or no composer): the suggestions' window goes.
        if self.suggest_window.is_some()
            && !self
                .compose
                .as_ref()
                .is_some_and(|c| c.suggest.as_ref().is_some_and(|s| !s.people.is_empty()))
        {
            cx.spawn(async move |this, cx| {
                this.update(cx, |this, cx| this.sync_suggestions(cx)).ok();
            })
            .detach();
        }
        // A new message (or none): show it in its window, after this frame.
        if self.notice != self.notice_shown {
            self.notice_shown = self.notice.clone();
            let room = from_screen(window.bounds().size.width);
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
        if self.more().is_some()
            && !self.loading_more
            && !self.fetching()
            && (self.near_list_end() || self.next_page_coming())
        {
            // Asked again once this frame is laid out: until then a list that has just
            // arrived has no height, and each first listing fetched its second page too.
            let this = cx.weak_entity();
            window.on_next_frame(move |_, cx| {
                this.update(cx, |this, cx| {
                    if !this.fetching() && (this.near_list_end() || this.next_page_coming()) {
                        this.load_more(cx);
                    }
                })
                .ok();
            });
        }
        // The loading lines fade out when what they waited for is there, instead of
        // vanishing.
        let checking = self.shows_checking() && !self.pull_hold && self.pull == 0.;
        let reading = self.reader_loading();
        if self.checking_line.update(checking) | self.reader_line.update(reading) {
            window.request_animation_frame();
        }
        self.frame = Some(window.bounds());
        // The update's progress stays on screen while Nimbra is the app in use. Asked of the
        // app, not of its key window: opening or clicking the card itself moves the key
        // window for a moment, and the card would close under the pointer, then reopen.
        let shown = self
            .update_progress
            .clone()
            .filter(|_| crate::integration::app_is_active());
        // As big as the notice it turns into once the update is ready, so that notice takes
        // its place exactly.
        // Every stage is that size, so the card changes in place from one to the next
        // (checking, downloading, ready or up to date) rather than going and coming back.
        let placed = shown.as_ref().map(|p| {
            let room = from_screen(window.bounds().size.width);
            let ready = system_events::update_notice(&p.version);
            let ready = Toast::new(&ready, room, window.text_system());
            UpdatePlace {
                frame: self.popup_frame(window.bounds()),
                dark: self.dark,
                size: (ready.width, ready.height),
            }
        });
        let pushed = self
            .update_window
            .and_then(|h| h.read(cx).ok().map(|pill| pill.progress.clone()));
        if placed != self.update_window_for || shown != pushed {
            cx.spawn(async move |this, cx| {
                this.update(cx, |this, cx| this.sync_update_window(shown, placed, cx))
                    .ok();
            })
            .detach();
        }
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
                // The inset is macOS's, the sidebar's place is design points.
                place.map(|place| 18. + place.x * zoom())
            },
        );
        if self.peek.moving() || self.sidebar_sliding() {
            window.request_animation_frame();
        }
        let empty = !self.any_visible();
        let zero_shown = match self.status {
            Status::Demo | Status::Ready => empty && self.is_zero(),
            _ => self.opening_to_zero() && !self.waiting_for_keychain(),
        };
        self.arrival = self
            .arrival
            .filter(|_| zero_shown)
            .and_then(|arrival| arrival.after(since_last));
        if self.arrival.is_some() {
            window.request_animation_frame();
        }
        self.move_sky(zero_shown, window, cx);
        let list_shown =
            !empty && self.list_open && matches!(self.status, Status::Demo | Status::Ready);

        let enabled = self.menu_state();
        div()
            .id("nimbra")
            .key_context(if self.compose.is_some() && !composing_elsewhere {
                "Composer"
            } else if self.pane == Pane::Sidebar {
                "Mail Sidebar"
            } else {
                "Mail"
            })
            .track_focus(&self.focus)
            .on_action(
                cx.listener(|this, _: &crate::input::Copy, _, cx| this.copy_reader_selection(cx)),
            )
            .on_action(cx.listener(|this, _: &Next, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &Prev, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &First, _, cx| this.step_to_end(false, cx)))
            .on_action(cx.listener(|this, _: &Last, _, cx| this.step_to_end(true, cx)))
            .on_action(cx.listener(|this, _: &FocusSidebar, _, cx| this.focus_sidebar(cx)))
            .on_action(cx.listener(|this, _: &OpenSidebarRow, _, cx| {
                if this.pane == Pane::Sidebar && !this.palette {
                    this.open_sidebar_row(cx)
                }
            }))
            .on_action(cx.listener(|this, _: &PickThread, _, cx| this.pick_thread(cx)))
            .on_action(cx.listener(|this, _: &PickDown, _, cx| this.pick_more(1, cx)))
            .on_action(cx.listener(|this, _: &PickUp, _, cx| this.pick_more(-1, cx)))
            .on_action(cx.listener(|this, _: &OpenInWindow, _, cx| {
                if !this.palette && this.visible().contains(&this.selected) {
                    // A draft opens to be written (as a double-click on it does).
                    if this.mailbox.base() == View::Drafts && !this.is_demo() {
                        this.open_draft(this.selected, true, cx)
                    } else {
                        this.open_message_window(this.selected, cx)
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &PageDown, _, cx| this.page_message(true, cx)))
            .on_action(cx.listener(|this, _: &PageUp, _, cx| this.page_message(false, cx)))
            .on_action(cx.listener(|this, _: &ToggleShortcuts, _, cx| this.open_shortcuts(cx)))
            .on_action(cx.listener(|this, _: &GoInbox, _, cx| this.go_to(View::Inbox, cx)))
            .on_action(cx.listener(|this, _: &GoStarred, _, cx| this.go_to(View::Starred, cx)))
            .on_action(cx.listener(|this, _: &GoSnoozed, _, cx| this.go_to(View::Snoozed, cx)))
            .on_action(cx.listener(|this, _: &GoSent, _, cx| this.go_to(View::Sent, cx)))
            .on_action(cx.listener(|this, _: &GoDrafts, _, cx| this.go_to(View::Drafts, cx)))
            .on_action(cx.listener(|this, _: &GoAllMail, _, cx| {
                if !this.is_demo() {
                    this.go_to(View::AllMail, cx)
                }
            }))
            .on_action(cx.listener(|this, go: &GoNumber, _, cx| this.go_number(go.0, cx)))
            // A click anywhere but the sidebar gives the keys back to the list.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    if from_screen(event.position.x) > this.side_w() + FLOAT_INSET {
                        this.leave_sidebar(cx);
                    }
                }),
            )
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
            .on_action(cx.listener(|this, _: &TogglePalette, window, cx| {
                if this.palette {
                    this.palette = false;
                    cx.notify();
                } else if !this.visible().is_empty() {
                    this.leave_sidebar(cx);
                    this.open_snooze(None, window, cx);
                }
            }))
            // Menu items grey out when they would do nothing (`MenuState`).
            .when(enabled.open, |d| {
                d.on_action(cx.listener(|this, _: &Print, _, cx| this.print_selected(cx)))
            })
            .when(enabled.answer, |d| {
                d.on_action(cx.listener(|this, _: &MenuReply, _, cx| {
                    this.open_compose(ComposeKind::Reply, cx)
                }))
                .on_action(cx.listener(|this, _: &MenuReplyAll, _, cx| {
                    this.open_compose(ComposeKind::ReplyAll, cx)
                }))
                .on_action(cx.listener(|this, _: &MenuForwardMessage, _, cx| {
                    this.open_compose(ComposeKind::Forward, cx)
                }))
            })
            .when(enabled.done, |d| {
                d.on_action(cx.listener(|this, _: &MenuMarkDone, _, cx| this.mark_done(cx)))
            })
            .when(enabled.targets, |d| {
                d.on_action(cx.listener(|this, _: &MenuMarkUnread, _, cx| this.toggle_unread(cx)))
                    .on_action(cx.listener(|this, _: &MenuStar, _, cx| this.toggle_star(cx)))
                    .on_action(cx.listener(|this, _: &MenuTrash, _, cx| this.trash(cx)))
                    .on_action(cx.listener(|this, _: &MenuSnooze, window, cx| {
                        this.open_snooze(None, window, cx)
                    }))
            })
            .when(enabled.undo, |d| {
                d.on_action(cx.listener(|this, _: &MenuUndo, _, cx| this.undo(cx)))
            })
            .when(enabled.original, |d| {
                d.on_action(
                    cx.listener(|this, _: &MenuToggleOriginal, _, cx| this.toggle_original(cx)),
                )
            })
            .on_action(
                cx.listener(|this, _: &ReplyAll, _, cx| {
                    this.open_compose(ComposeKind::ReplyAll, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ForwardMessage, _, cx| {
                this.open_compose(ComposeKind::Forward, cx)
            }))
            .on_action(cx.listener(|this, _: &MenuToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &MenuSearch, _, cx| this.focus_search(cx)))
            .on_action(cx.listener(|this, _: &MenuShortcuts, _, cx| this.open_shortcuts(cx)))
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
                } else if this.account_sheet_over_main() {
                    this.close_account_sheet(cx);
                } else if this.pane == Pane::Sidebar && !this.palette {
                    this.leave_sidebar(cx);
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
                } else if this.mailbox.base() == View::Search
                    || !this.search.read(cx).text().is_empty()
                {
                    this.clear_search(cx);
                } else {
                    this.palette = false;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Confirm, _, cx| {
                if this.palette {
                    this.run_command(cx);
                } else if this.pane == Pane::Sidebar {
                    this.open_sidebar_row(cx);
                } else {
                    // Return: the open message in full view (and back).
                    this.toggle_full_view(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleTheme, _, cx| this.toggle_theme(cx)))
            .on_action(cx.listener(|this, _: &MenuToggleTheme, _, cx| this.toggle_theme(cx)))
            // Here too, as the theme: the app-wide handlers reach this window only from
            // the others (updating it from inside its own dispatch does nothing).
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.step_zoom(1, cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.step_zoom(-1, cx)))
            .on_action(cx.listener(|this, _: &ActualSize, _, cx| this.set_interface_zoom(100, cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, _, cx| this.open_settings(cx)))
            .on_action(cx.listener(|this, _: &CheckForUpdates, _, cx| this.check_for_updates(cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &Back, _, cx| this.go_back(cx)))
            .on_action(cx.listener(|this, _: &Forward, _, cx| this.go_forward(cx)))
            .on_action(cx.listener(|this, _: &Reload, _, cx| this.reload(cx)))
            .on_action(cx.listener(|this, _: &MenuReload, _, cx| this.reload(cx)))
            .on_action(|_: &HideWindow, window, _| crate::blur::set_visible(window, false))
            .on_action(|_: &Minimize, window, _| window.minimize_window())
            .on_action(|_: &Quit, _, cx: &mut App| cx.quit())
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                let Some((start, width)) = this.list_grab else {
                    return;
                };
                if event.pressed_button != Some(MouseButton::Left) {
                    this.list_grab = None;
                    this.keep_list_width(cx);
                    return;
                }
                this.list_width =
                    (width + from_screen(event.position.x) - start).clamp(LIST_MIN, LIST_MAX);
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.list_grab.take().is_some() {
                        this.keep_list_width(cx);
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &ToggleList, _, cx| this.toggle_list(cx)))
            .on_action(cx.listener(|this, _: &MenuToggleList, _, cx| this.toggle_list(cx)))
            .relative()
            .flex()
            .size_full()
            .font_family(crate::theme::SANS)
            .text_size(dp(13.))
            .line_height(dp(18.))
            .text_color(t.text)
            // The room the docked sidebar takes. The sidebar itself is drawn over it, so
            // floating in does not move or re-wrap anything. As it docks, what is around the
            // panel is filled (the panel's own shape is erased out of it).
            .child(
                div()
                    .flex_none()
                    .h_full()
                    .w(dp(self.sidebar_width(0.)))
                    .when(afloat > 0., |d| d.bg(t.surface)),
            )
            .map(|d| match self.status {
                _ if zero_shown => d.child(self.zero(&t)),
                Status::Ready if empty && self.next_page_coming() => d.child(self.skeleton(&t)),
                Status::Demo | Status::Ready if empty => d.child(self.empty_mailbox(&t, cx)),
                Status::Demo | Status::Ready => {
                    d.children(self.list_slot(&t, window, cx)).map(|d| {
                        if self.picked.len() >= 2 {
                            d.child(self.bulk_pane(&t, cx))
                        } else {
                            d.child(self.reading(&t, cx))
                        }
                    })
                }
                Status::Loading if self.waiting_for_keychain() => {
                    d.child(self.status_screen(&t, cx))
                }
                Status::Loading => d.child(self.skeleton(&t)),
                _ => d.child(self.status_screen(&t, cx)),
            })
            .when(self.offline && !list_shown && place.is_none(), |d| {
                // No list and no reload button to carry it.
                d.child(
                    div()
                        .id("offline-dot")
                        .absolute()
                        .left(dp(8.))
                        .top(dp(22.))
                        .size(dp(6.))
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
                            .left(dp(place.x))
                            .top(dp(place.inset))
                            .w(dp(self.side_w()))
                            .h(dp(height))
                            .rounded(dp(radius))
                            .shadow(vec![shadow(t.shadow.opacity(afloat), 10., 30., -6.)]),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(dp(place.x))
                            .top(dp(place.inset))
                            .w(dp(self.side_w()))
                            .h(dp(height))
                            .child(crate::erase::rounded(radius, t.surface)),
                    )
                })
                .child(
                    div()
                        .absolute()
                        .left(dp(place.x))
                        .top(dp(place.inset))
                        .w(dp(self.side_w()))
                        .h(dp(height))
                        .overflow_hidden()
                        .occlude()
                        .when(floating, |d| d.rounded(dp(radius)).bg(t.sidebar))
                        // Moved up by the gap, so that docking shifts nothing vertically.
                        .child(
                            div()
                                .absolute()
                                .left_0()
                                .top(dp(-place.inset))
                                .w(dp(self.side_w()))
                                .h(dp(viewport.1 - place.inset))
                                .child(self.sidebar(&t, floating, cx)),
                        )
                        // Over the contents, so it takes no width from them.
                        .when(floating, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .rounded(dp(radius))
                                    .border_1()
                                    .border_color(t.border_strong.opacity(afloat)),
                            )
                        }),
                )
                // Its right edge: drag to resize, docked or floating; double-click for the
                // usual width. Out here, over the sidebar and the pane beside it, where
                // nothing clips or covers it.
                // Dragging it follows the pointer over the whole window: the sidebar keeps its
                // moves from the window's own listeners, so shrinking would stop at once.
                .when(self.sidebar_grab.is_some(), |d| {
                    d.child(self.sidebar_drag(cx))
                })
                // Deferred: painted after the panes, which would cover its right half.
                .child(gpui::deferred(
                    div()
                        .id("sidebar-resize")
                        // Takes the press itself: what is under it (the list, under a
                        // floating sidebar) must not take it as a click.
                        .occlude()
                        .absolute()
                        .left(dp(place.x + self.side_w() - 4.))
                        .top(dp(place.inset))
                        .w(dp(8.))
                        .h(dp(height))
                        .cursor(gpui::CursorStyle::ResizeLeftRight)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                if event.click_count == 2 {
                                    this.sidebar_grab = None;
                                    this.settings.sidebar_width =
                                        crate::settings::Settings::default().sidebar_width;
                                    this.save_settings(cx);
                                } else {
                                    this.sidebar_grab =
                                        Some((from_screen(event.position.x), this.side_w()));
                                }
                                cx.notify();
                            }),
                        ),
                ))
                .child(self.window_controls(&t, place.x, list_shown, cx))
            })
            .when(self.palette && !empty, |d| d.child(self.palette(&t, cx)))
            .when(self.compose.is_some() && !composing_elsewhere, |d| {
                d.child(self.compose_view(&t, viewport, cx))
            })
            .when(self.new_label.is_some(), |d| {
                d.child(self.new_label_view(&t, cx))
            })
            .when(self.account_sheet_over_main(), |d| {
                d.child(self.account_sheet_view(&t, cx))
            })
            .when(self.confirm.is_some(), |d| {
                d.child(self.confirm_view(&t, cx))
            })
            .when(self.finder_open, |d| d.child(self.finder_view(&t, cx)))
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

impl Nimbra {
    /// A sender's avatar: their brand's logo (`bimi`), for mail that passed DMARC from a
    /// domain that has one, else their initials.
    fn sender_avatar(&self, m: &Mail, size: f32) -> AnyElement {
        let logo = m
            .brand_domain
            .as_ref()
            .and_then(|domain| self.brand_logos.get(domain))
            .cloned()
            .flatten();
        match logo {
            // Square, made to be cut to a circle; white under any see-through parts.
            // GPUI clips children to a rectangle, so the image is rounded itself, as the
            // profile photo is (`me_avatar`), with the same faint edge as the initials.
            Some(logo) => div()
                .flex_none()
                .size(dp(size))
                .rounded_full()
                .overflow_hidden()
                .bg(gpui::white())
                .border_1()
                .border_color(c(0xffffff, 0.18))
                .child(gpui::img(logo).size_full().rounded_full())
                .into_any_element(),
            // No one named (a draft with no recipients yet): no initials made from a
            // stand-in such as "Unknown sender".
            None => avatar(
                m.tone,
                if m.names_someone() { &m.initials } else { "" },
                size,
            )
            .into_any_element(),
        }
    }

    /// Looks up the brand logos the listed rows can use and doesn't know yet, one domain at
    /// a time in the background, kept in the one store all accounts share (`bimi::logo`):
    /// a company writing to two accounts is fetched once.
    pub(super) fn ask_brand_logos(&mut self, cx: &mut Context<Self>) {
        let mut jobs: Vec<String> = Vec::new();
        for m in &self.mails {
            let Some(domain) = m.brand_domain.clone() else {
                continue;
            };
            if !self.brand_logos.contains_key(&domain)
                && !self.brand_asking.contains(&domain)
                && !jobs.contains(&domain)
            {
                jobs.push(domain);
            }
        }
        if jobs.is_empty() {
            return;
        }
        self.brand_asking.extend(jobs.iter().cloned());
        let kept = self.brand_store.clone();
        cx.spawn(async move |this, cx| {
            let store = match kept {
                Some(store) => store,
                None => {
                    let opened = cx
                        .background_executor()
                        .spawn(async { crate::store::Store::open_shared("brands") })
                        .await;
                    let Ok(store) = opened.map(Arc::new) else {
                        // No keychain: no logos this time, asked again next time.
                        this.update(cx, |this, _| this.brand_asking.clear()).ok();
                        return;
                    };
                    let shared = store.clone();
                    this.update(cx, |this, _| this.brand_store = Some(shared))
                        .ok();
                    store
                }
            };
            for domain in jobs {
                let store = store.clone();
                let asked = domain.clone();
                let found = cx
                    .background_executor()
                    .spawn(async move { nimbra_core::bimi::logo(&store, &asked) })
                    .await;
                let gone = this
                    .update(cx, |this, cx| {
                        this.brand_asking.remove(&domain);
                        // Offline, or a resolver that didn't answer: asked again next time.
                        if let Ok(svg) = found {
                            let logo = svg.map(|svg| {
                                Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Svg, svg))
                            });
                            this.brand_logos.insert(domain, logo);
                            cx.notify();
                        }
                    })
                    .is_err();
                if gone {
                    break;
                }
            }
        })
        .detach();
    }
}

/// The signed-in account's avatar: its profile photo, or its initial.
fn me_avatar(photo: Option<&Arc<gpui::Image>>, initials: &str, size: f32) -> AnyElement {
    match photo {
        Some(photo) => div()
            .flex_none()
            .size(dp(size))
            .rounded_full()
            .overflow_hidden()
            .border_1()
            .border_color(c(0xffffff, 0.18))
            .child(gpui::img(photo.clone()).size_full().rounded_full())
            .into_any_element(),
        None => avatar(Tone::Graphite, initials, size).into_any_element(),
    }
}

impl Nimbra {
    /// While the sidebar's edge is dragged: its width follows the pointer anywhere in the
    /// window, and is kept when the button is let go.
    fn sidebar_drag(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        gpui::canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let moved = entity.clone();
                window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, _, cx| {
                    if phase != gpui::DispatchPhase::Bubble {
                        return;
                    }
                    moved.update(cx, |this, cx| {
                        let Some((start, width)) = this.sidebar_grab else {
                            return;
                        };
                        if event.pressed_button != Some(MouseButton::Left) {
                            this.sidebar_grab = None;
                            this.save_settings(cx);
                        } else {
                            this.settings.sidebar_width = (width + from_screen(event.position.x)
                                - start)
                                .clamp(SIDEBAR_MIN, SIDEBAR_MAX);
                        }
                        cx.notify();
                    });
                });
                let released = entity.clone();
                window.on_mouse_event(move |_: &gpui::MouseUpEvent, phase, _, cx| {
                    if phase == gpui::DispatchPhase::Bubble {
                        released.update(cx, |this, cx| {
                            if this.sidebar_grab.take().is_some() {
                                this.save_settings(cx);
                                cx.notify();
                            }
                        });
                    }
                });
            },
        )
        .absolute()
        .size_0()
    }
}

#[cfg(test)]
mod header_tests {
    use super::*;

    #[test]
    fn a_pop_up_shadow_fades_inside_its_window() {
        let (down, blur, spread) = POPUP_SHADOW;
        // How far the shadow reaches past the card, below and to the sides.
        // GPUI's shader draws a shadow out to 3 times its blur (see `shaders.metal`).
        let below = down + spread + 3. * blur;
        let sides = spread + 3. * blur;
        assert!(
            below <= MENU_MARGIN && sides <= MENU_MARGIN,
            "{below} {sides}"
        );
    }

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
