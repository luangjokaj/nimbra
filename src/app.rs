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
    ButtonKind, Press, TOP_BAR, avatar, button, icon, icon_button, kbd, meta, mono, overline, pill,
    shadow, tip,
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
use outgoing::*;

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
use snooze_menu::SnoozeMenu;

#[path = "system_events.rs"]
mod system_events;
pub use system_events::{
    DockDrafts, DockInbox, DockSent, DockSnoozed, DockStarred, UseDarkAppearance,
    UseLightAppearance, UseSystemAppearance, install as install_system_actions,
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
    /// Delete these threads for good.
    DeleteForever(Vec<String>),
    /// Ask Google for permanent deletion, then delete these threads.
    GrantFullAccess(Vec<String>),
    /// Delete a label (id, name); the mail stays.
    DeleteLabel(String, String),
}

/// Changes and messages made while offline, sent once Gmail can be reached again.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct Outbox {
    pub(crate) changes: Vec<(String, mail::Change)>,
    /// Only as earlier versions saved it: waiting mail has records of its own now (see
    /// `sending`), and these become such records when the store opens.
    #[serde(default)]
    mail: Vec<QueuedMail>,
}

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
/// The account and sidebar state saved alongside the mailboxes.
#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    account: String,
    initials: String,
    counts: HashMap<String, (u32, u32)>,
    labels: Vec<mail::Label>,
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
    /// Each account's next page in All inboxes.
    all_next: HashMap<String, String>,
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
    hint: Option<(&'static str, &'static str)>,
    /// Where a row's right-click menu opens (window coordinates), and what its items do.
    menu_at: (f32, f32),
    row_actions: Vec<RowAction>,
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
    /// The shortcuts sheet (?).
    shortcuts: bool,
    /// The shortcuts sheet's window, while it is open.
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
        // The Add account screenshots show the Microsoft button when it is set up.
        let account_screen = matches!(
            screen.as_str(),
            "account" | "account-servers" | "settings-account"
        );
        let microsoft = if sample && !account_screen {
            None
        } else {
            crate::microsoft::Config::load()
        };
        let settings = crate::settings::load();
        crate::usage::opened(settings.usage_counts);
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
            snooze_on_start: match screen.as_str() {
                "snooze" => Some(false),
                "snooze-time" => Some(true),
                _ => None,
            },
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
            microsoft,
            connection: None,
            account: if demo {
                "Luan Gjokaj".into()
            } else {
                "Gmail".into()
            },
            me_initials: if demo { "LG".into() } else { "ME".into() },
            photos: HashMap::new(),
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
            // NIMBRA_LIST_WIDTH=<px> starts with the list at that width (screenshots).
            list_width: std::env::var("NIMBRA_LIST_WIDTH")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .map_or(452., |w| w.clamp(300., 720.)),
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
            shortcuts: false,
            shortcuts_window: None,
            reader_scroll: gpui::ScrollHandle::new(),
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
        // NIMBRA_SCREEN=about opens the About panel (screenshots).
        if screen == "about" {
            cx.defer(|_| crate::about::show());
        }
        // NIMBRA_SCREEN=settings opens the Settings sheet (screenshots).
        // NIMBRA_SCREEN=settings-account opens it on Add account.
        // NIMBRA_SCREEN=settings-remove opens it on Accounts, asking to remove one.
        if screen == "settings"
            || screen == "settings-account"
            || screen == "settings-accounts"
            || screen == "settings-remove"
            || screen == "settings-about"
        {
            let form = screen == "settings-account";
            // After the main window is up.
            cx.spawn(async move |this, cx| {
                this.update(cx, |this, cx| {
                    if form {
                        this.open_account_form_in_settings(cx);
                    }
                    this.open_settings(cx)
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
            async {}
        })
        .detach();
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
        this
    }

    fn is_demo(&self) -> bool {
        self.status == Status::Demo
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

    fn save_settings(&self, _cx: &mut Context<Self>) {
        // Written on the settings thread, newest last; quitting waits for it.
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
            (true, View::Inbox | View::AllInboxes) | (false, _) => {
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
        self.close_shortcuts(cx);
        self.leave_sidebar(cx);
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
        self.close_shortcuts(cx);
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
        View::Reminders => 15,
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

fn card(t: &Theme) -> Div {
    div()
        .bg(t.hover)
        .border_1()
        .border_color(t.border)
        .rounded(px(12.))
}

// ---------------------------------------------------------------- panes

impl Nimbra {
    /// Shown in place of the mail panes while signed out, signing in, loading or failed.
    fn status_screen(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let own_servers = self.servers_of(&self.account_id).is_some();
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
            _ if self.waiting_for_keychain() => {
                ("Waiting for the Keychain", KEYCHAIN_WAITING.into(), None)
            }
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
                div()
                    .pt(px(10.))
                    .flex()
                    .gap(px(8.))
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
            }))
    }
}

impl Render for Nimbra {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.shortcuts_on_start) {
            cx.defer_in(window, |this, window, cx| this.toggle_shortcuts(window, cx));
        }
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
            window.focus(&focus);
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
        self.roll_day(cx);
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
                    this.open_message_window(this.selected, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &PageDown, _, cx| this.page_message(true, cx)))
            .on_action(cx.listener(|this, _: &PageUp, _, cx| this.page_message(false, cx)))
            .on_action(cx.listener(|this, _: &ToggleShortcuts, window, cx| {
                this.toggle_shortcuts(window, cx)
            }))
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
                    this.close_shortcuts(cx);
                    if f32::from(event.position.x) > this.side_w() + FLOAT_INSET {
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
                } else if this.shortcuts {
                    this.close_shortcuts(cx);
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
                } else if this.pane == Pane::Sidebar {
                    this.open_sidebar_row(cx);
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
                            .w(px(self.side_w()))
                            .h(px(height))
                            .rounded(px(radius))
                            .shadow(vec![shadow(t.shadow.opacity(afloat), 10., 30., -6.)]),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(place.x))
                            .top(px(place.inset))
                            .w(px(self.side_w()))
                            .h(px(height))
                            .child(crate::erase::rounded(radius, t.surface)),
                    )
                })
                .child(
                    div()
                        .absolute()
                        .left(px(place.x))
                        .top(px(place.inset))
                        .w(px(self.side_w()))
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
                                .w(px(self.side_w()))
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
                        .left(px(place.x + self.side_w() - 4.))
                        .top(px(place.inset))
                        .w(px(8.))
                        .h(px(height))
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
                                        Some((f32::from(event.position.x), this.side_w()));
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
                            this.settings.sidebar_width = (width + f32::from(event.position.x)
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
