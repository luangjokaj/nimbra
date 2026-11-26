//! Preferences, kept in a small JSON file next to the mail store (not in it: they are not
//! mail, and they stay when you sign out).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Added under new messages, replies and forwards. Plain text, one line per line.
    pub signature: String,
    /// Notify about new mail in the inbox.
    pub notifications: bool,
    /// Open mail in its original formatting (the web view) rather than the reader.
    pub original_view: bool,
    /// Show remote images in the original view.
    pub load_images: bool,
    /// In dark theme, give light emails a dark page in the original view.
    pub dark_emails: bool,
    /// Light, dark, or follow the system.
    pub theme: ThemeChoice,
    /// The Gmail accounts signed in on this Mac (ids and addresses; the sign-ins themselves
    /// are in the keychain).
    pub accounts: Vec<AccountRef>,
    /// Accounts a newer Nimbra wrote that this one cannot read: kept as they are and saved
    /// again, so that running an older copy never loses them.
    #[serde(skip)]
    pub unreadable_accounts: Vec<serde_json::Value>,
    /// The account shown.
    pub active: String,
    /// The account that is yours by default: its profile at the bottom of the sidebar, and
    /// new messages are from it. `None`, or one signed out since: the first account.
    pub default_account: Option<String>,
    /// Sidebar sections folded away ("split", "mailbox", "labels").
    pub collapsed: Vec<String>,
    /// The sidebar was left in auto-hide (⌘B).
    pub sidebar_auto_hide: bool,
    /// The sidebar's width, as its right edge was dragged to (docked and floating alike).
    pub sidebar_width: f32,
    /// The last few searches, newest first.
    pub recent_searches: Vec<String>,
    /// When an opened thread counts as read.
    pub mark_read: MarkRead,
    /// How often to check Gmail for new mail, in seconds.
    pub check_every: u32,
    /// How long a sent message can still be taken back, in seconds (0: sent at once).
    pub undo_send: u32,
    /// Replies and forwards include the original message (folded behind •••).
    pub quote_replies: bool,
    /// New-mail notifications play a sound.
    pub notify_sound: bool,
    /// The Dock icon shows the inbox's unread count.
    pub dock_badge: bool,
    /// The size of text in the reader.
    pub reader_text: TextSize,
    /// The Zero Streak shows in the sidebar and on Inbox Zero.
    pub show_streak: bool,
    /// The eclipse at the sidebar's foot has been pointed at once, so it needs no caption.
    pub eclipse_seen: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkRead {
    /// As soon as it is opened.
    Open,
    /// After it has been open for two seconds.
    Delay,
    /// Only with Mark as read (or U).
    Never,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextSize {
    Small,
    Default,
    Large,
}

impl TextSize {
    /// Body text size and line height in the reader, in points.
    pub fn metrics(self) -> (f32, f32) {
        match self {
            TextSize::Small => (13.5, 23.),
            TextSize::Default => (15., 26.),
            TextSize::Large => (17., 29.),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountRef {
    pub id: String,
    pub email: String,
    /// For an account that is not Gmail: its IMAP and SMTP servers (its password is in
    /// the keychain). None for Gmail, which is all an older settings file has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub servers: Option<crate::servers::Servers>,
    /// For an account with servers that signs in with its provider rather than a password:
    /// "microsoft" (its sign-in is in the keychain). None for a password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    System,
    Light,
    Dark,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            signature: String::new(),
            notifications: true,
            original_view: true,
            load_images: true,
            dark_emails: true,
            theme: ThemeChoice::System,
            accounts: Vec::new(),
            unreadable_accounts: Vec::new(),
            active: String::new(),
            default_account: None,
            collapsed: Vec::new(),
            sidebar_auto_hide: false,
            sidebar_width: 236.,
            recent_searches: Vec::new(),
            mark_read: MarkRead::Open,
            check_every: 30,
            undo_send: 6,
            quote_replies: true,
            notify_sound: true,
            dock_badge: true,
            reader_text: TextSize::Default,
            show_streak: true,
            eclipse_seen: false,
        }
    }
}

impl Settings {
    /// The default account's id, when any account is signed in. The first account's id is
    /// "", so this is never tested for emptiness.
    pub fn default_account(&self) -> Option<&str> {
        self.default_account
            .as_deref()
            .filter(|id| self.accounts.iter().any(|a| a.id == *id))
            .or_else(|| self.accounts.first().map(|a| a.id.as_str()))
    }

    /// The default account's address.
    pub fn default_email(&self) -> Option<&str> {
        let id = self.default_account()?;
        self.accounts
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.email.as_str())
    }
}

/// The signature can be a few lines; anything longer is cut.
pub const SIGNATURE_LIMIT: usize = 2000;

fn path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/Nimbra/settings.json"))
}

/// What loading found other than a good file, for the app to say once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Recovery {
    /// The file was unreadable; the last good copy was used. It was set aside.
    FromBackup,
    /// The file and its backup were both unreadable: defaults, with the file set aside.
    Reset,
}

static RECOVERY: std::sync::OnceLock<Recovery> = std::sync::OnceLock::new();

/// Set when the settings had to be recovered at launch.
pub fn recovery() -> Option<Recovery> {
    RECOVERY.get().copied()
}

/// The saved settings, or the defaults.
pub fn load() -> Settings {
    let Some(path) = path() else {
        return Settings::default();
    };
    let (settings, recovery) = load_from(&path);
    if let Some(recovery) = recovery {
        eprintln!("nimbra: settings were unreadable ({recovery:?})");
        let _ = RECOVERY.set(recovery);
    }
    settings
}

fn backup_of(path: &Path) -> PathBuf {
    path.with_extension("json.bak")
}

fn partial_of(path: &Path) -> PathBuf {
    path.with_extension("json.partial")
}

fn parse(path: &Path) -> std::io::Result<Option<Settings>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(read(&bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(error),
        // Unreadable counts as damaged: never as "there are none".
        Err(_) => Ok(None),
    }
}

/// The settings in a file's bytes. A field this build cannot read (a newer build's choice,
/// say) takes its default and the others are kept; so are the accounts it can read. `None`
/// when the file is not settings at all, or its list of accounts is not a list.
fn read(bytes: &[u8]) -> Option<Settings> {
    use serde_json::Value;
    let Ok(Value::Object(mut file)) = serde_json::from_slice(bytes) else {
        return None;
    };
    let accounts = match file.remove("accounts") {
        Some(Value::Array(accounts)) => accounts,
        Some(_) => return None,
        None => Vec::new(),
    };
    file.retain(|key, value| {
        let alone = serde_json::Map::from_iter([(key.clone(), value.clone())]);
        serde_json::from_value::<Settings>(Value::Object(alone)).is_ok()
    });
    let mut settings: Settings = serde_json::from_value(Value::Object(file)).ok()?;
    for account in accounts {
        match serde_json::from_value(account.clone()) {
            Ok(known) => settings.accounts.push(known),
            Err(_) => settings.unreadable_accounts.push(account),
        }
    }
    Some(settings)
}

/// A damaged file is moved aside (never saved over) and the last good copy used instead.
fn load_from(path: &Path) -> (Settings, Option<Recovery>) {
    let damaged = match parse(path) {
        Ok(Some(settings)) => return (settings, None),
        Ok(None) => {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let aside = path.with_extension(format!("json.corrupt-{stamp}"));
            if let Err(error) = std::fs::rename(path, &aside) {
                eprintln!("nimbra: could not set damaged settings aside: {error}");
            }
            true
        }
        Err(_) => false,
    };
    // A save cut off between its two moves left its new copy, complete and on disk.
    if let Ok(Some(settings)) = parse(&partial_of(path)) {
        return (settings, damaged.then_some(Recovery::FromBackup));
    }
    match parse(&backup_of(path)) {
        Ok(Some(settings)) => (settings, damaged.then_some(Recovery::FromBackup)),
        Ok(None) => (Settings::default(), Some(Recovery::Reset)),
        Err(_) => (Settings::default(), damaged.then_some(Recovery::Reset)),
    }
}

/// Saves the settings on a writer thread and returns at once. Changes that come faster than
/// the disk are written as one, the newest; `flush` waits for it.
pub fn save(settings: &Settings) -> Result<()> {
    writer().put(settings.clone());
    Ok(())
}

/// Waits until the last settings saved are on disk (quitting), for a few seconds at most.
/// Whether they are.
pub fn flush() -> bool {
    writer().flush(Instant::now() + Duration::from_secs(3))
}

fn writer() -> &'static Arc<Latest<Settings>> {
    static WRITER: OnceLock<Arc<Latest<Settings>>> = OnceLock::new();
    WRITER.get_or_init(|| Latest::new(write))
}

fn write(settings: &Settings) -> Result<()> {
    save_to(&path().context("no home folder")?, settings)
}

fn save_to(path: &Path, settings: &Settings) -> Result<()> {
    let dir = path.parent().context("no settings folder")?;
    std::fs::create_dir_all(dir)?;
    let mut settings = settings.clone();
    settings.signature = settings.signature.chars().take(SIGNATURE_LIMIT).collect();
    let mut json = serde_json::to_value(&settings)?;
    if let Some(serde_json::Value::Array(accounts)) = json.get_mut("accounts") {
        accounts.extend(settings.unreadable_accounts);
    }
    let json = serde_json::to_vec_pretty(&json)?;
    // Written in full next to the file and on disk before it takes its place; the copy it
    // replaces stays as the backup.
    let partial = partial_of(path);
    {
        use std::io::Write as _;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&partial)?;
        // One left by an earlier run keeps its old permissions.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(&json)?;
        file.sync_all()?;
    }
    if path.exists() {
        std::fs::rename(path, backup_of(path))?;
    }
    std::fs::rename(&partial, path)?;
    std::fs::File::open(dir)?.sync_all()?;
    Ok(())
}

/// Writes the newest of what it is given on a thread of its own, one at a time.
struct Latest<T> {
    state: Mutex<Waiting<T>>,
    changed: Condvar,
    write: fn(&T) -> Result<()>,
}

struct Waiting<T> {
    next: Option<T>,
    /// How many were given, and how many of those are written (or replaced by a newer one
    /// that is).
    given: u64,
    written: u64,
    started: bool,
}

impl<T: Send + 'static> Latest<T> {
    fn new(write: fn(&T) -> Result<()>) -> Arc<Self> {
        Arc::new(Latest {
            state: Mutex::new(Waiting {
                next: None,
                given: 0,
                written: 0,
                started: false,
            }),
            changed: Condvar::new(),
            write,
        })
    }

    fn lock(&self) -> MutexGuard<'_, Waiting<T>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn put(self: &Arc<Self>, value: T) {
        let mut state = self.lock();
        state.next = Some(value);
        state.given += 1;
        if !state.started {
            let latest = self.clone();
            state.started = std::thread::Builder::new()
                .name("nimbra-settings".into())
                .spawn(move || latest.run())
                .is_ok();
            if !state.started {
                // No thread to write on: written here rather than lost.
                let value = state.next.take();
                let given = state.given;
                drop(state);
                if let Some(value) = value {
                    self.write_one(&value, given);
                }
                return;
            }
        }
        self.changed.notify_all();
    }

    fn run(&self) {
        loop {
            let (value, given) = {
                let mut state = self.lock();
                while state.next.is_none() {
                    state = self
                        .changed
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                }
                (state.next.take(), state.given)
            };
            if let Some(value) = value {
                self.write_one(&value, given);
            }
        }
    }

    fn write_one(&self, value: &T, given: u64) {
        if let Err(error) = (self.write)(value) {
            eprintln!("nimbra: could not save settings: {error:#}");
        }
        let mut state = self.lock();
        state.written = state.written.max(given);
        self.changed.notify_all();
    }

    fn flush(&self, deadline: Instant) -> bool {
        let state = self.lock();
        let given = state.given;
        let left = deadline.saturating_duration_since(Instant::now());
        let (state, _) = self
            .changed
            .wait_timeout_while(state, left, |state| state.written < given)
            .unwrap_or_else(PoisonError::into_inner);
        state.written >= given
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// What a test writer saw, and a gate that holds up the first write.
    struct Probe {
        seen: Mutex<Vec<u32>>,
        entered: Mutex<Option<mpsc::Sender<()>>>,
        gate: Mutex<Option<mpsc::Receiver<()>>>,
    }

    fn probe_write(value: &(u32, Arc<Probe>)) -> Result<()> {
        if let Some(entered) = value.1.entered.lock().unwrap().take() {
            entered.send(()).unwrap();
            let gate = value.1.gate.lock().unwrap().take().unwrap();
            gate.recv().ok();
        }
        value.1.seen.lock().unwrap().push(value.0);
        Ok(())
    }

    #[test]
    fn changes_made_while_one_is_written_are_written_as_the_newest_only() {
        let (entered, has_entered) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let probe = Arc::new(Probe {
            seen: Mutex::new(Vec::new()),
            entered: Mutex::new(Some(entered)),
            gate: Mutex::new(Some(gate)),
        });
        let latest = Latest::new(probe_write);
        latest.put((1, probe.clone()));
        has_entered.recv().unwrap();
        for n in 2..=4 {
            latest.put((n, probe.clone()));
        }
        assert!(
            !latest.flush(Instant::now()),
            "the first is still being written"
        );
        release.send(()).unwrap();
        assert!(latest.flush(Instant::now() + Duration::from_secs(30)));
        assert_eq!(*probe.seen.lock().unwrap(), [1, 4]);
    }

    #[test]
    fn flushing_waits_for_the_last_change() {
        let probe = Arc::new(Probe {
            seen: Mutex::new(Vec::new()),
            entered: Mutex::new(None),
            gate: Mutex::new(None),
        });
        let latest = Latest::new(probe_write);
        for n in 1..=50 {
            latest.put((n, probe.clone()));
            if n % 10 == 0 {
                assert!(latest.flush(Instant::now() + Duration::from_secs(30)));
                assert_eq!(probe.seen.lock().unwrap().last(), Some(&n));
            }
        }
    }

    #[test]
    fn settings_are_moved_into_place_whole_and_private() {
        let dir = std::env::temp_dir().join(format!("nimbra-settings-{}", std::process::id()));
        let path = dir.join("settings.json");
        for signature in ["one", "two"] {
            let settings = Settings {
                signature: signature.into(),
                ..Settings::default()
            };
            save_to(&path, &settings).unwrap();
        }
        let back: Settings = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(back.signature, "two");
        assert!(!path.with_extension("json.partial").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_settings_file_from_before_the_streak_switch_shows_the_streak() {
        let settings: Settings =
            serde_json::from_str(r#"{"signature":"Luan","reader_text":"large"}"#).unwrap();
        assert!(settings.show_streak);
    }

    #[test]
    fn a_hidden_streak_stays_hidden_after_a_save() {
        let hidden = Settings {
            show_streak: false,
            ..Settings::default()
        };
        let again: Settings =
            serde_json::from_str(&serde_json::to_string(&hidden).unwrap()).unwrap();
        assert!(!again.show_streak);
    }

    #[test]
    fn missing_fields_take_their_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"signature":"Luan"}"#).unwrap();
        assert_eq!(settings.signature, "Luan");
        assert!(settings.notifications && settings.original_view);
        // The sidebar starts docked until it has been put in auto-hide.
        assert!(!settings.sidebar_auto_hide);
        let hidden: Settings = serde_json::from_str(r#"{"sidebar_auto_hide":true}"#).unwrap();
        assert!(hidden.sidebar_auto_hide);
        // On for settings saved before it existed; turning it off sticks.
        assert!(settings.dark_emails);
        let off: Settings = serde_json::from_str(r#"{"dark_emails":false}"#).unwrap();
        assert!(!off.dark_emails);
        assert_eq!(settings.theme, ThemeChoice::System);
        assert_eq!(settings.default_account, None);
    }

    fn two() -> Settings {
        Settings {
            accounts: vec![
                AccountRef {
                    id: String::new(),
                    email: "a@example.com".into(),
                    servers: None,
                    oauth: None,
                },
                AccountRef {
                    id: "bbbbbbbbbbbb".into(),
                    email: "b@example.com".into(),
                    servers: None,
                    oauth: None,
                },
            ],
            ..Settings::default()
        }
    }

    #[test]
    fn the_default_account_is_the_chosen_one_or_else_the_first() {
        let mut settings = two();
        // Never chosen: the first, whose id is "".
        assert_eq!(settings.default_account(), Some(""));
        assert_eq!(settings.default_email(), Some("a@example.com"));
        settings.default_account = Some("bbbbbbbbbbbb".into());
        assert_eq!(settings.default_account(), Some("bbbbbbbbbbbb"));
        assert_eq!(settings.default_email(), Some("b@example.com"));
        // Chosen and then signed out: the first again.
        settings.accounts.pop();
        assert_eq!(settings.default_account(), Some(""));
        // Choosing the first one is kept as such ("" is an id, not "none").
        settings.default_account = Some(String::new());
        assert_eq!(settings.default_account(), Some(""));
        settings.accounts.clear();
        assert_eq!(settings.default_account(), None);
        // Saved and read back.
        let mut chosen = two();
        chosen.default_account = Some("bbbbbbbbbbbb".into());
        let back: Settings = serde_json::from_slice(&serde_json::to_vec(&chosen).unwrap()).unwrap();
        assert_eq!(back.default_account(), Some("bbbbbbbbbbbb"));
    }

    #[test]
    fn an_account_says_how_it_signs_in_only_when_it_is_not_a_password() {
        let mut settings = two();
        let plain = serde_json::to_string(&settings).unwrap();
        assert!(!plain.contains("oauth"));
        settings.accounts[1].oauth = Some("microsoft".into());
        let back: Settings =
            serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        assert_eq!(back.accounts[1].oauth.as_deref(), Some("microsoft"));
        assert_eq!(back.accounts[0].oauth, None);
    }

    /// A settings file in a folder of its own, removed when dropped.
    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("nimbra-settings-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Folder(dir)
        }

        fn file(&self) -> PathBuf {
            self.0.join("settings.json")
        }

        fn names(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&self.0)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_torn_file_loads_the_last_good_copy_and_is_kept_aside() {
        let folder = Folder::new("torn");
        save_to(&folder.file(), &two()).unwrap();
        let mut newer = two();
        newer.signature = "Newer".into();
        save_to(&folder.file(), &newer).unwrap();
        std::fs::write(folder.file(), b"").unwrap();

        let (settings, recovery) = load_from(&folder.file());

        assert_eq!(recovery, Some(Recovery::FromBackup));
        assert_eq!(settings.accounts.len(), 2);
        assert!(
            folder
                .names()
                .iter()
                .any(|n| n.starts_with("settings.json.corrupt-")),
            "{:?}",
            folder.names()
        );
    }

    #[test]
    fn a_damaged_file_without_a_backup_is_reported_not_silently_reset() {
        let folder = Folder::new("reset");
        std::fs::write(folder.file(), b"{\"accounts\": [").unwrap();

        let (_, recovery) = load_from(&folder.file());

        assert_eq!(recovery, Some(Recovery::Reset));
        assert!(
            !folder.file().exists(),
            "the damaged file must not be saved over"
        );
    }

    #[test]
    fn a_save_cut_off_between_its_moves_keeps_the_newest_copy() {
        let folder = Folder::new("between");
        save_to(&folder.file(), &Settings::default()).unwrap();
        let mut newer = two();
        newer.signature = "Newest".into();
        std::fs::write(
            partial_of(&folder.file()),
            serde_json::to_vec(&newer).unwrap(),
        )
        .unwrap();
        std::fs::rename(folder.file(), backup_of(&folder.file())).unwrap();

        let (settings, recovery) = load_from(&folder.file());

        assert_eq!((settings.signature.as_str(), recovery), ("Newest", None));
    }

    /// As a newer build might write it: a theme, a mark-read choice, a field and an
    /// account's security that this build does not know.
    const FROM_A_NEWER_BUILD: &str = r#"{
        "signature": "Luan",
        "theme": "midnight",
        "mark_read": "on_scroll",
        "check_every": 60,
        "future": {"x": 1},
        "accounts": [
            {"id": "", "email": "a@example.com"},
            {"id": "cccccccccccc", "email": "c@example.com", "servers": {
                "username": "c@example.com",
                "imap": {"host": "imap.example.com", "port": 993, "security": "quic"},
                "smtp": {"host": "smtp.example.com", "port": 465, "security": "tls"}
            }}
        ]
    }"#;

    #[test]
    fn a_newer_builds_settings_keep_what_this_build_can_read() {
        let folder = Folder::new("newer");
        std::fs::write(folder.file(), FROM_A_NEWER_BUILD).unwrap();

        let (settings, recovery) = load_from(&folder.file());

        assert_eq!(recovery, None);
        assert_eq!(settings.signature, "Luan");
        assert_eq!(settings.check_every, 60);
        assert_eq!(settings.theme, ThemeChoice::System);
        assert_eq!(settings.mark_read, MarkRead::Open);
        assert_eq!(settings.accounts.len(), 1);
        assert_eq!(settings.accounts[0].email, "a@example.com");
    }

    #[test]
    fn an_account_this_build_cannot_read_survives_a_save() {
        let folder = Folder::new("unread-account");
        std::fs::write(folder.file(), FROM_A_NEWER_BUILD).unwrap();
        let (mut settings, _) = load_from(&folder.file());
        settings.signature = "Changed".into();

        save_to(&folder.file(), &settings).unwrap();

        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(folder.file()).unwrap()).unwrap();
        let ids: Vec<&str> = saved["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|a| a["id"].as_str())
            .collect();
        assert_eq!(ids, ["", "cccccccccccc"]);
        assert_eq!(saved["accounts"][1]["servers"]["imap"]["security"], "quic");
    }

    #[test]
    fn accounts_that_are_not_a_list_count_as_damaged() {
        let folder = Folder::new("accounts-damaged");
        save_to(&folder.file(), &two()).unwrap();
        save_to(&folder.file(), &two()).unwrap();
        std::fs::write(folder.file(), br#"{"accounts": 3}"#).unwrap();

        let (settings, recovery) = load_from(&folder.file());

        assert_eq!(recovery, Some(Recovery::FromBackup));
        assert_eq!(settings.accounts.len(), 2);
    }

    #[test]
    fn no_file_yet_is_a_first_launch() {
        let folder = Folder::new("fresh");
        assert_eq!(load_from(&folder.file()), (Settings::default(), None));
    }

    #[test]
    fn saving_keeps_the_previous_copy_as_the_backup() {
        let folder = Folder::new("backup");
        save_to(&folder.file(), &two()).unwrap();
        save_to(&folder.file(), &Settings::default()).unwrap();

        let backup: Settings =
            serde_json::from_slice(&std::fs::read(backup_of(&folder.file())).unwrap()).unwrap();

        assert_eq!(backup.accounts.len(), 2);
        assert_eq!(folder.names(), ["settings.json", "settings.json.bak"]);
    }
}
