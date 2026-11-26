//! The local mail store: what Nimbra has already fetched, kept on disk so the app opens
//! straight into your mail instead of waiting on Gmail.
//!
//! Everything is encrypted (ChaCha20-Poly1305) with a 256-bit key that is generated on
//! first use and kept only in the system keychain, so the files on disk (and in backups)
//! are unreadable on their own. Each record is one file, written atomically (its own
//! temporary file, then rename) with permissions limited to the user. Signing out deletes
//! both the files and the key.
//!
//! Records hold either mail fetched from the server, which can always be fetched again, or
//! the user's own work that exists nowhere else yet (`is_own`): a message being sent, the
//! offline queue, send later, snoozes, blocked senders. Those are flushed to the disk before
//! a save returns, and survive clearing the mail kept here.
//!
//! Writes never wait on each other across records: only two writes of the same record do,
//! and only for the rename. `save_later` and `update_later` hand a write to the store's own
//! writer thread, in order, so the UI thread never waits on the disk; `load` sees those
//! writes even before they land, and quitting waits for them (`settle`).

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::future::Future;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, Weak, mpsc};
use std::task::{Context, Poll};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context as _, Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use futures::channel::oneshot;
use serde::Serialize;
use serde::de::DeserializeOwned;

const KEYCHAIN_SERVICE: &str = "nimbra";
const KEYCHAIN_USER: &str = "store-key";
/// Bumped whenever the stored shapes change; older stores are discarded, not misread.
const FORMAT: u8 = 2;
/// The size of a saved empty list: format, nonce, `[]`, and the cipher's tag.
const EMPTY_LIST: u64 = 1 + 12 + 2 + 16;
const WRITER: &str = "nimbra-store";
/// A temporary file this old was left by a write that never finished (the app was killed).
const STALE_TEMP: Duration = Duration::from_secs(60 * 60);

/// One key is made at a time: a second would replace the one just written with.
static MAKING_KEY: Mutex<()> = Mutex::new(());
/// The accounts whose key could not be read when it was last asked for.
static UNREADABLE: Mutex<Option<HashSet<String>>> = Mutex::new(None);
/// One lock per record file, shared by every `Store` of the folder.
static RECORDS: Mutex<Option<HashMap<PathBuf, Weak<Mutex<()>>>>> = Mutex::new(None);
/// Writes handed to writer threads that have not landed yet, in every store.
static QUEUED: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The user's own work, as opposed to mail that can be fetched again: flushed to the disk
/// on every save, and kept when the mail kept on this Mac is cleared.
pub fn is_own(name: &str) -> bool {
    name.starts_with("sending-")
        || name.starts_with("due-")
        || matches!(name, "outbox" | "scheduled" | "snoozed" | "blocked")
}

/// The lock of one record: held while its file is replaced or removed, and by `update` for
/// as long as it reads, changes and writes it.
fn record_lock(path: &Path) -> Arc<Mutex<()>> {
    let mut all = locked(&RECORDS);
    let all = all.get_or_insert_default();
    if let Some(lock) = all.get(path).and_then(Weak::upgrade) {
        return lock;
    }
    if all.len() > 256 {
        all.retain(|_, lock| lock.strong_count() > 0);
    }
    let lock = Arc::new(Mutex::new(()));
    all.insert(path.to_path_buf(), Arc::downgrade(&lock));
    lock
}

type Job = Box<dyn FnOnce() + Send>;

/// What a write handed to the writer thread came to: `None` if it failed.
pub struct Later<T>(oneshot::Receiver<T>);

impl<T> Future for Later<T> {
    type Output = Option<T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        Pin::new(&mut self.0).poll(cx).map(Result::ok)
    }
}

/// Counts a queued write off when it has run, whether or not it worked (or panicked).
struct Queued {
    store: Arc<Store>,
    name: String,
}

impl Drop for Queued {
    fn drop(&mut self) {
        {
            let mut queued = locked(&self.store.queued);
            if let Some(count) = queued.get_mut(&self.name) {
                *count -= 1;
                if *count == 0 {
                    queued.remove(&self.name);
                }
            }
        }
        self.store.drained.notify_all();
        *locked(&QUEUED.0) -= 1;
        QUEUED.1.notify_all();
    }
}

pub struct Store {
    dir: PathBuf,
    cipher: ChaCha20Poly1305,
    snapshots: AtomicU64,
    /// The snapshot each record saved with `save_newest` has.
    newest: Mutex<HashMap<String, u64>>,
    /// The writer thread, started by the first write handed to it.
    writer: OnceLock<mpsc::Sender<Job>>,
    /// Writes of each record still waiting for the writer thread.
    queued: Mutex<HashMap<String, usize>>,
    drained: Condvar,
}

impl Store {
    /// Opens the store, creating the key (in the keychain) and the folder on first use.
    /// Blocking: may wait on the keychain. Call from a background thread.
    /// `account` is the account id ("" for the first account, as stored before accounts).
    pub fn open(account: &str) -> Result<Self> {
        let dir = default_dir(account)?;
        let key = load_or_create_key(account);
        let mut unreadable = locked(&UNREADABLE);
        let unreadable = unreadable.get_or_insert_default();
        match &key {
            Ok(_) => unreadable.remove(account),
            Err(_) => unreadable.insert(account.to_string()),
        };
        Self::open_at(dir, &key?)
    }

    /// The same for work nobody is waiting on. It does not ask the keychain again once that
    /// has said no, as macOS would put up its question every time.
    pub fn open_quietly(account: &str) -> Result<Self> {
        if Self::unreadable(account) {
            bail!("the key of this account's store could not be read");
        }
        Self::open(account)
    }

    /// Whether the account's key could not be read when it was last asked for, so that
    /// nothing of it can be kept.
    pub fn unreadable(account: &str) -> bool {
        locked(&UNREADABLE)
            .as_ref()
            .is_some_and(|all| all.contains(account))
    }

    fn open_at(dir: PathBuf, key: &[u8; 32]) -> Result<Self> {
        std::fs::create_dir_all(&dir).context("creating the mail store folder")?;
        restrict(&dir, 0o700);
        remove_stale_temps(&dir);
        Ok(Self {
            dir,
            cipher: ChaCha20Poly1305::new(&Key::from(*key)),
            snapshots: AtomicU64::new(0),
            newest: Mutex::new(HashMap::new()),
            writer: OnceLock::new(),
            queued: Mutex::new(HashMap::new()),
            drained: Condvar::new(),
        })
    }

    /// The folder, for tests elsewhere.
    #[cfg(test)]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// A store in a fresh temporary folder, for tests elsewhere.
    #[cfg(test)]
    pub fn temp() -> Self {
        let mut id = [0u8; 8];
        getrandom::fill(&mut id).unwrap();
        let name: String = id.iter().map(|b| format!("{b:02x}")).collect();
        Self::open_at(
            std::env::temp_dir().join(format!("nimbra-test-{name}")),
            &[7; 32],
        )
        .unwrap()
    }

    /// Saves a record under `name`, replacing any previous one. Blocking.
    pub fn save<T: Serialize + ?Sized>(&self, name: &str, value: &T) -> Result<()> {
        self.save_if(name, value, || true).map(drop)
    }

    /// Saves a record unless `wanted`, asked under the record's lock just before the new file
    /// takes the old one's place, says no: a write that lost a race, or whose record was
    /// removed meanwhile, never lands. Whether it landed. Blocking.
    pub fn save_if<T: Serialize + ?Sized>(
        &self,
        name: &str,
        value: &T,
        wanted: impl FnOnce() -> bool,
    ) -> Result<bool> {
        let path = self.path(name)?;
        let temp = self.stage(name, &path, &self.seal(name, value)?)?;
        let landed = {
            let lock = record_lock(&path);
            let _record = locked(&lock);
            wanted().then(|| std::fs::rename(&temp, &path))
        };
        self.landed(name, &temp, landed)
    }

    /// After a staged file took its record's place, or did not.
    fn landed(&self, name: &str, temp: &Path, rename: Option<std::io::Result<()>>) -> Result<bool> {
        match rename {
            Some(Ok(())) => {
                if is_own(name) {
                    sync_dir(&self.dir);
                }
                Ok(true)
            }
            Some(Err(error)) => {
                let _ = std::fs::remove_file(temp);
                Err(error).context("writing the mail store")
            }
            None => {
                let _ = std::fs::remove_file(temp);
                Ok(false)
            }
        }
    }

    /// Reads a record. Anything missing, from an older format, or that fails to decrypt
    /// (tampered with, or written with another key) reads as nothing. A write of it handed
    /// to the writer thread is waited for, so what was saved is what is read.
    pub fn load<T: DeserializeOwned>(&self, name: &str) -> Option<T> {
        // On the writer itself the write waited for could be the one running.
        if std::thread::current().name() == Some(WRITER) {
            return self.load_now(name);
        }
        drop(
            self.drained
                .wait_while(locked(&self.queued), |queued| queued.contains_key(name))
                .unwrap_or_else(PoisonError::into_inner),
        );
        self.load_now(name)
    }

    fn load_now<T: DeserializeOwned>(&self, name: &str) -> Option<T> {
        let file = std::fs::read(self.path(name).ok()?).ok()?;
        if file.len() < 13 || file[0] != FORMAT {
            return None;
        }
        let plain = self
            .cipher
            .decrypt(
                &Nonce::try_from(&file[1..13]).ok()?,
                chacha20poly1305::aead::Payload {
                    msg: &file[13..],
                    aad: name.as_bytes(),
                },
            )
            .ok()?;
        serde_json::from_slice(&plain).ok()
    }

    /// Numbers a copy that is about to be saved in the background, for `save_newest`.
    pub fn snapshot(&self) -> u64 {
        self.snapshots.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Saves a copy unless a newer one already is: background saves land in any order.
    pub fn save_newest<T: Serialize + ?Sized>(
        &self,
        name: &str,
        value: &T,
        taken: u64,
    ) -> Result<()> {
        self.save_if(name, value, || {
            let mut newest = locked(&self.newest);
            if newest.get(name).is_some_and(|saved| *saved > taken) {
                return false;
            }
            newest.insert(name.to_string(), taken);
            true
        })
        .map(drop)
    }

    /// Changes a record as it now is on disk, and gives back what it became. Blocking.
    pub fn update<T: Serialize + DeserializeOwned + Default>(
        &self,
        name: &str,
        change: impl FnOnce(&mut T),
    ) -> Result<T> {
        let path = self.path(name)?;
        let lock = record_lock(&path);
        let _record = locked(&lock);
        let mut value: T = self.load_now(name).unwrap_or_default();
        change(&mut value);
        let temp = self.stage(name, &path, &self.seal(name, &value)?)?;
        self.landed(name, &temp, Some(std::fs::rename(&temp, &path)))?;
        Ok(value)
    }

    /// Saves a record on the store's writer thread, after the writes handed to it before.
    /// Never waits on the disk.
    pub fn save_later<T: Serialize + Send + 'static>(
        self: &Arc<Self>,
        name: &str,
        value: T,
    ) -> Later<()> {
        let owned = name.to_string();
        self.later(name, move |store| store.save(&owned, &value))
    }

    /// `update` on the store's writer thread, after the writes handed to it before; gives
    /// back what `change` returns once the record is on disk. Never waits on the disk.
    pub fn update_later<T, R>(
        self: &Arc<Self>,
        name: &str,
        change: impl FnOnce(&mut T) -> R + Send + 'static,
    ) -> Later<R>
    where
        T: Serialize + DeserializeOwned + Default,
        R: Send + 'static,
    {
        let owned = name.to_string();
        self.later(name, move |store| {
            let mut out = None;
            store.update(&owned, |value: &mut T| out = Some(change(value)))?;
            out.ok_or_else(|| anyhow!("the change was not made"))
        })
    }

    /// Deletes the mail kept here on the writer thread: everything but the user's own work
    /// (`is_own`), which exists nowhere else.
    pub fn clear_mail_later(self: &Arc<Self>) -> Later<()> {
        self.later("", |store| {
            clear_dir(&store.dir, true);
            Ok(())
        })
    }

    fn later<R: Send + 'static>(
        self: &Arc<Self>,
        name: &str,
        write: impl FnOnce(&Store) -> Result<R> + Send + 'static,
    ) -> Later<R> {
        *locked(&self.queued).entry(name.to_string()).or_default() += 1;
        *locked(&QUEUED.0) += 1;
        let queued = Queued {
            store: self.clone(),
            name: name.to_string(),
        };
        let (done, answer) = oneshot::channel();
        let job: Job = Box::new(move || {
            match write(&queued.store) {
                Ok(value) => drop(done.send(value)),
                Err(error) => eprintln!("nimbra: could not save {}: {error:#}", queued.name),
            }
            drop(queued);
        });
        let writer = self.writer.get_or_init(|| {
            let (sender, jobs) = mpsc::channel::<Job>();
            // Ends once the store is gone: every queued job holds it, so none is dropped.
            let started = std::thread::Builder::new()
                .name(WRITER.into())
                .spawn(move || jobs.into_iter().for_each(|job| job()));
            if started.is_err() {
                eprintln!("nimbra: the store's writer did not start; writing in place");
            }
            sender
        });
        if let Err(mpsc::SendError(job)) = writer.send(job) {
            job();
        }
        Later(answer)
    }

    /// The records whose name starts with `prefix`.
    pub fn names(&self, prefix: &str) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let file = entry.file_name();
                let name = file.to_str()?.strip_suffix(".bin")?;
                name.starts_with(prefix).then(|| name.to_string())
            })
            .collect()
    }

    /// Claims a record for this process for as long as the file returned is open. `None`
    /// while someone has it.
    pub fn claim(&self, name: &str) -> Option<File> {
        let path = self.path(name).ok()?.with_extension("lock");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(false);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let file = options.open(path).ok()?;
        file.try_lock().ok()?;
        Some(file)
    }

    /// Deletes a record and its claim. Waits only for a write of it that is landing right
    /// now, so that none lands after this.
    pub fn remove(&self, name: &str) {
        let Ok(path) = self.path(name) else {
            return;
        };
        {
            let lock = record_lock(&path);
            let _record = locked(&lock);
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(path.with_extension("lock"));
        }
        if is_own(name) {
            let dir = self.dir.clone();
            std::thread::spawn(move || sync_dir(&dir));
        }
    }

    /// Whether the list saved under `name` is empty, as far as can be told without the key
    /// (which waits on the keychain): from its size. Good for choosing what to draw until
    /// the store is open and `load` has the real answer, and for nothing else.
    pub fn saved_empty(account: &str, name: &str) -> bool {
        default_dir(account).is_ok_and(|dir| empty_list_at(&dir, name))
    }

    /// Deletes every stored record, the user's own work included (keeps the key, so the
    /// store can be reused). Blocking.
    pub fn clear(&self) {
        clear_dir(&self.dir, false);
    }

    fn path(&self, name: &str) -> Result<PathBuf> {
        record_path(&self.dir, name)
    }

    fn seal<T: Serialize + ?Sized>(&self, name: &str, value: &T) -> Result<Vec<u8>> {
        let plain = serde_json::to_vec(value)?;
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).map_err(|e| anyhow!("no secure random source: {e}"))?;
        // The record name is bound in as associated data, so one record cannot be swapped
        // in for another.
        let sealed = self
            .cipher
            .encrypt(
                &Nonce::from(nonce),
                chacha20poly1305::aead::Payload {
                    msg: &plain,
                    aad: name.as_bytes(),
                },
            )
            .map_err(|_| anyhow!("encrypting the mail store failed"))?;
        let mut file = Vec::with_capacity(1 + nonce.len() + sealed.len());
        file.push(FORMAT);
        file.extend_from_slice(&nonce);
        file.extend_from_slice(&sealed);
        Ok(file)
    }

    /// Writes the new file beside the record, under a name of its own, so that writes of
    /// the same record at once never share one. The user's own work is flushed to the disk.
    fn stage(&self, name: &str, path: &Path, sealed: &[u8]) -> Result<PathBuf> {
        let mut nonce = [0u8; 8];
        getrandom::fill(&mut nonce).map_err(|e| anyhow!("no secure random source: {e}"))?;
        let nonce: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let temp = path.with_extension(format!("{nonce}.tmp"));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let written = options.open(&temp).and_then(|mut file| {
            file.write_all(sealed)?;
            if is_own(name) {
                file.sync_all()?;
            }
            Ok(())
        });
        if let Err(error) = written {
            let _ = std::fs::remove_file(&temp);
            return Err(error).context("writing the mail store");
        }
        Ok(temp)
    }
}

/// Whether a write handed to a writer thread has not landed yet.
pub fn writing() -> bool {
    *locked(&QUEUED.0) > 0
}

/// Waits until every write handed to a writer thread has landed, or the deadline.
pub fn settle(deadline: Instant) {
    let left = deadline.saturating_duration_since(Instant::now());
    drop(
        QUEUED
            .1
            .wait_timeout_while(locked(&QUEUED.0), left, |queued| *queued > 0),
    );
}

fn record_path(dir: &Path, name: &str) -> Result<PathBuf> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("bad record name");
    }
    Ok(dir.join(format!("{name}.bin")))
}

fn empty_list_at(dir: &Path, name: &str) -> bool {
    let Ok(path) = record_path(dir, name) else {
        return false;
    };
    // The size first: a full mailbox is megabytes, and this runs before the first frame.
    std::fs::metadata(&path).is_ok_and(|file| file.is_file() && file.len() == EMPTY_LIST)
        && std::fs::read(&path).is_ok_and(|file| file.first() == Some(&FORMAT))
}

/// Makes a rename or a removal in the folder survive a power cut, where the system can.
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(folder) = File::open(dir) {
        let _ = folder.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Deletes the files in a store's folder; with `keep_own`, not the user's own work, nor a
/// write that is landing right now.
fn clear_dir(dir: &Path, keep_own: bool) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file = entry.file_name();
        let file = file.to_string_lossy();
        let record = file.split('.').next().unwrap_or_default();
        if keep_own && (is_own(record) || file.ends_with(".tmp")) {
            continue;
        }
        let _ = std::fs::remove_file(entry.path());
    }
}

fn remove_stale_temps(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .is_ok_and(|at| now.duration_since(at).is_ok_and(|age| age > STALE_TEMP));
        if stale && entry.file_name().to_string_lossy().ends_with(".tmp") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Signing out because the server stopped taking the sign-in: removes the mail kept here,
/// but not the user's own work nor the key that protects it, so that signing in again to
/// the same account carries on with what was waiting.
pub fn wipe_mail(account: &str) {
    if let Ok(dir) = default_dir(account) {
        clear_dir(&dir, true);
    }
}

/// Signing out: removes the stored mail and the key that protects it.
pub fn wipe(account: &str) {
    if let Ok(dir) = default_dir(account) {
        let _ = std::fs::remove_dir_all(dir);
    }
    if let Ok(user) = key_user(account)
        && let Ok(entry) = keyring::Entry::new(KEYCHAIN_SERVICE, &user)
    {
        let _ = entry.delete_credential();
    }
}

/// The keychain entry of an account's store key.
fn key_user(account: &str) -> Result<String> {
    if !crate::mail::is_account_id(account) {
        bail!("unexpected account id");
    }
    Ok(if account.is_empty() {
        KEYCHAIN_USER.to_string()
    } else {
        format!("{KEYCHAIN_USER}-{account}")
    })
}

fn default_dir(account: &str) -> Result<PathBuf> {
    if !crate::mail::is_account_id(account) {
        bail!("unexpected account id");
    }
    let folder = if account.is_empty() {
        "store".to_string()
    } else {
        format!("store-{account}")
    };
    Ok(base_dir()?.join(folder))
}

fn base_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("no home folder"))?;
    let base = if cfg!(target_os = "macos") {
        PathBuf::from(home).join("Library/Application Support/Nimbra")
    } else {
        PathBuf::from(home).join(".local/share/nimbra")
    };
    Ok(base)
}

fn load_or_create_key(account: &str) -> Result<[u8; 32]> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, &key_user(account)?)
        .context("opening the system keychain")?;
    if let Some(key) = key_in(entry.get_password())? {
        return Ok(key);
    }
    let _making = locked(&MAKING_KEY);
    if let Some(key) = key_in(entry.get_password())? {
        return Ok(key); // made meanwhile
    }
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|e| anyhow!("no secure random source: {e}"))?;
    entry
        .set_password(&STANDARD.encode(key))
        .context("saving the mail store key to the keychain")?;
    // A new key cannot read anything written with the old one.
    if let Ok(dir) = default_dir(account) {
        let _ = std::fs::remove_dir_all(dir);
    }
    Ok(key)
}

/// The key in a keychain's answer. `None` only when there certainly is none: a read that
/// failed (denied, locked) is an error, as making a key then would delete what the real one
/// protects.
fn key_in(read: keyring::Result<String>) -> Result<Option<[u8; 32]>> {
    match read {
        Ok(saved) => Ok(STANDARD
            .decode(saved.trim())
            .ok()
            .and_then(|bytes| <[u8; 32]>::try_from(bytes.as_slice()).ok())),
        Err(keyring::Error::NoEntry | keyring::Error::BadEncoding(_)) => Ok(None),
        Err(error) => Err(error).context("reading the mail store key from the keychain"),
    }
}

#[cfg(unix)]
fn restrict(path: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn restrict(_path: &std::path::Path, _mode: u32) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(key: u8) -> Store {
        let mut dir = std::env::temp_dir();
        let mut id = [0u8; 8];
        getrandom::fill(&mut id).unwrap();
        dir.push(format!(
            "nimbra-store-test-{}",
            STANDARD.encode(id).replace(['/', '+', '='], "")
        ));
        Store::open_at(dir, &[key; 32]).unwrap()
    }

    #[test]
    fn a_claimed_record_cannot_be_claimed_again_until_let_go() {
        let store = temp_store(5);
        store.save("sending-1", &1).unwrap();
        store.save("view-0", &2).unwrap();
        assert_eq!(store.names("sending-"), ["sending-1"]);
        let claim = store.claim("sending-1").unwrap();
        assert!(store.claim("sending-1").is_none());
        drop(claim);
        assert!(store.claim("sending-1").is_some());
        store.remove("sending-1");
        assert!(store.names("sending-").is_empty());
        std::fs::remove_dir_all(&store.dir).unwrap();
    }

    #[test]
    fn a_key_is_only_made_when_there_is_none() {
        use keyring::Error;
        let failed = || Box::new(std::io::Error::other("denied"));
        assert!(key_in(Err(Error::PlatformFailure(failed()))).is_err());
        assert!(key_in(Err(Error::NoStorageAccess(failed()))).is_err());
        assert_eq!(key_in(Err(Error::NoEntry)).unwrap(), None);
        assert_eq!(key_in(Ok("not a key".into())).unwrap(), None);
        let saved = STANDARD.encode([4; 32]);
        assert_eq!(key_in(Ok(saved)).unwrap(), Some([4; 32]));
    }

    #[test]
    fn updates_at_the_same_time_all_land() {
        let store = temp_store(6);
        std::thread::scope(|scope| {
            for n in 0..8 {
                let store = &store;
                scope.spawn(move || store.update::<Vec<u32>>("list", |list| list.push(n)));
            }
        });
        assert_eq!(store.load::<Vec<u32>>("list").unwrap().len(), 8);
        std::fs::remove_dir_all(&store.dir).unwrap();
    }

    #[test]
    fn saves_at_the_same_time_all_work_and_the_newest_stays() {
        let store = temp_store(8);
        let (older, newer) = (store.snapshot(), store.snapshot());
        store.save_newest("view-0", &"newer", newer).unwrap();
        store.save_newest("view-0", &"older", older).unwrap();
        assert_eq!(store.load::<String>("view-0").as_deref(), Some("newer"));
        std::thread::scope(|scope| {
            for n in 0..16 {
                let store = &store;
                scope.spawn(move || store.save("meta", &vec![n; 20_000]).unwrap());
            }
        });
        assert_eq!(store.load::<Vec<u32>>("meta").unwrap().len(), 20_000);
        std::fs::remove_dir_all(&store.dir).unwrap();
    }

    #[test]
    fn a_removal_waits_for_a_write_landing_so_none_lands_after_it() {
        let store = temp_store(9);
        let (inside, landing) = mpsc::channel();
        std::thread::scope(|scope| {
            let writer = scope.spawn(|| {
                store.save_if("sending-1", &"held", || {
                    inside.send(()).unwrap();
                    std::thread::sleep(Duration::from_millis(100));
                    true
                })
            });
            landing.recv().unwrap();
            store.remove("sending-1");
            assert!(writer.join().unwrap().unwrap());
        });
        assert_eq!(store.load::<String>("sending-1"), None);
        std::fs::remove_dir_all(&store.dir).unwrap();
    }

    #[test]
    fn writes_handed_to_the_writer_land_in_order_and_read_back() {
        let store = Arc::new(temp_store(10));
        for n in 0..50 {
            drop(store.save_later("outbox", vec![n; 1000]));
        }
        let length = store.update_later("outbox", |list: &mut Vec<u32>| {
            list.push(7);
            list.len()
        });
        assert_eq!(store.load::<Vec<u32>>("outbox").unwrap()[..2], [49, 49]);
        assert_eq!(futures::executor::block_on(length), Some(1001));
        settle(Instant::now() + Duration::from_secs(5));
        assert!(!writing());
        std::fs::remove_dir_all(&store.dir).unwrap();
    }

    #[test]
    fn clearing_the_mail_keeps_the_users_own_work() {
        let store = Arc::new(temp_store(11));
        for name in [
            "view-0",
            "rows",
            "outbox",
            "scheduled",
            "snoozed",
            "sending-ab",
        ] {
            store.save(name, &name).unwrap();
        }
        let claim = store.claim("sending-ab").unwrap();
        futures::executor::block_on(store.clear_mail_later()).unwrap();
        let mut left = store.names("");
        left.sort();
        assert_eq!(left, ["outbox", "scheduled", "sending-ab", "snoozed"]);
        assert!(store.claim("sending-ab").is_none(), "its claim is kept too");
        drop(claim);
        std::fs::remove_dir_all(&store.dir).unwrap();
    }

    #[test]
    fn round_trip_and_files_are_not_plaintext() {
        let store = temp_store(7);
        store
            .save("view-0", &vec!["Quarterly report", "secret words"])
            .unwrap();
        let back: Vec<String> = store.load("view-0").unwrap();
        assert_eq!(back, ["Quarterly report", "secret words"]);
        let raw = std::fs::read(store.dir.join("view-0.bin")).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("secret words"));
        std::fs::remove_dir_all(&store.dir).unwrap();
    }

    #[test]
    fn wrong_key_tampering_and_swapped_records_read_as_nothing() {
        let store = temp_store(1);
        store.save("a", &"hello").unwrap();
        store.save("b", &"other").unwrap();
        let other_key = Store::open_at(store.dir.clone(), &[2; 32]).unwrap();
        assert_eq!(other_key.load::<String>("a"), None);
        // Flip one byte of the ciphertext.
        let path = store.dir.join("a.bin");
        let mut raw = std::fs::read(&path).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 1;
        std::fs::write(&path, &raw).unwrap();
        assert_eq!(store.load::<String>("a"), None);
        // A record copied under another name does not decrypt.
        std::fs::copy(store.dir.join("b.bin"), store.dir.join("c.bin")).unwrap();
        assert_eq!(store.load::<String>("c"), None);
        assert_eq!(store.load::<String>("b").as_deref(), Some("other"));
        std::fs::remove_dir_all(&store.dir).unwrap();
    }

    #[test]
    fn an_empty_list_is_known_without_the_key() {
        let store = temp_store(5);
        let empty = || empty_list_at(&store.dir, "view-0");
        assert!(!empty(), "never saved");
        store.save("view-0", &Vec::<String>::new()).unwrap();
        assert!(empty());
        // The shortest list that has anything in it.
        store.save("view-0", &vec![0]).unwrap();
        assert!(!empty());
        store.save("view-0", &Vec::<String>::new()).unwrap();
        let path = store.dir.join("view-0.bin");
        let mut raw = std::fs::read(&path).unwrap();
        raw[0] = FORMAT + 1;
        std::fs::write(&path, &raw).unwrap();
        assert!(!empty(), "another format");
        assert!(!empty_list_at(&store.dir, "../view-0"));
        std::fs::remove_dir_all(&store.dir).unwrap();
    }

    #[test]
    fn record_names_cannot_escape_the_folder() {
        let store = temp_store(3);
        for bad in ["../x", "a/b", "", "a.b", &"x".repeat(65)] {
            assert!(store.save(bad, &1).is_err(), "{bad:?}");
        }
        std::fs::remove_dir_all(&store.dir).unwrap();
    }
}
