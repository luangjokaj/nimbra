//! A message from Send until Gmail has it. It is kept in its account's store meanwhile, so
//! that quitting or switching accounts cannot lose it, and one that may already have gone
//! out is never sent again by itself. What was being written at quit is kept the same way,
//! for Drafts only.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::mail;
use crate::store::Store;

const RECORD: &str = "sending-";

#[derive(Clone, Serialize, Deserialize)]
pub struct QueuedMail {
    pub draft_id: Option<String>,
    pub outgoing: mail::Outgoing,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum Stage {
    /// Has not left this Mac: safe to send.
    Held,
    /// A request is out. Found like this at launch, it may or may not have been sent.
    Out,
    /// A request was out and its answer never came: it may or may not have been sent.
    Unsure,
    /// Gmail said no.
    Refused,
    /// Was still being written at quit: for Drafts, never to be sent.
    Writing,
    /// The same, with a time to send it that was not saved yet.
    Unscheduled,
    /// Discarded at quit: its draft is to be deleted, and nothing sent or saved.
    Discarded,
    /// Sent, taken back, or handed on.
    Over,
}

/// A message is not sent while what is on disk says it has not left, as a crash during
/// the request would have it sent again at the next launch.
#[derive(Debug)]
pub struct NotRecorded;

impl std::fmt::Display for NotRecorded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Nimbra could not save it on this Mac first.")
    }
}

impl std::error::Error for NotRecorded {}

#[derive(Clone, Serialize, Deserialize)]
struct Record {
    stage: Stage,
    mail: QueuedMail,
}

struct State {
    record: Record,
    store: Option<Arc<Store>>,
    /// Held for as long as this copy of Nimbra looks after the message, so that another
    /// does not send it as left behind. The system lets go of it however the app ends.
    claim: Option<std::fs::File>,
    /// Counts the changes to be written: a write of an older one never lands.
    version: u64,
    /// A write may have landed, so the disk may say it has not left.
    on_disk: bool,
    /// From a request going out until its answer is written down.
    out: bool,
}

pub struct Job {
    pub account: String,
    /// The composer it came from; none for one found at launch.
    pub composer: Option<u64>,
    client: Arc<mail::Client>,
    name: String,
    /// The last write landed. Outside the lock, which is only ever held for a moment.
    kept: AtomicBool,
    state: Mutex<State>,
    changed: Condvar,
}

impl Job {
    /// `store` is the account's when that is open; otherwise `keep` opens it.
    pub fn new(
        account: String,
        composer: u64,
        client: Arc<mail::Client>,
        store: Option<Arc<Store>>,
        mail: QueuedMail,
    ) -> anyhow::Result<Arc<Self>> {
        let name = format!("{RECORD}{}", mail::random_hex(8)?);
        let record = Record {
            stage: Stage::Held,
            mail,
        };
        Ok(Arc::new(Self::with(
            account,
            Some(composer),
            client,
            name,
            record,
            store,
            None,
        )))
    }

    fn with(
        account: String,
        composer: Option<u64>,
        client: Arc<mail::Client>,
        name: String,
        record: Record,
        store: Option<Arc<Store>>,
        claim: Option<std::fs::File>,
    ) -> Self {
        let found = claim.is_some();
        Self {
            account,
            composer,
            client,
            name,
            kept: AtomicBool::new(found),
            state: Mutex::new(State {
                record,
                store,
                claim,
                version: 0,
                on_disk: found,
                out: false,
            }),
            changed: Condvar::new(),
        }
    }

    /// One that was still open at quit: `stage` is `Writing`, `Unscheduled` or `Discarded`.
    pub fn unfinished(
        account: String,
        composer: u64,
        client: Arc<mail::Client>,
        store: Option<Arc<Store>>,
        mail: QueuedMail,
        stage: Stage,
    ) -> anyhow::Result<Arc<Self>> {
        let job = Job::new(account, composer, client, store, mail)?;
        job.locked().record.stage = stage;
        Ok(job)
    }

    /// One from the offline queue of earlier versions.
    pub fn queued(
        account: &str,
        client: &Arc<mail::Client>,
        store: &Arc<Store>,
        mail: QueuedMail,
    ) -> anyhow::Result<Arc<Self>> {
        Job::new(
            account.to_string(),
            0,
            client.clone(),
            Some(store.clone()),
            mail,
        )
    }

    /// What an earlier run, or a switch of accounts, left in this account's store.
    /// Blocking.
    pub fn left_behind(
        account: &str,
        client: &Arc<mail::Client>,
        store: &Arc<Store>,
    ) -> Vec<Arc<Self>> {
        let mut found = Vec::new();
        for name in store.names(RECORD) {
            let Some(claim) = store.claim(&name) else {
                continue; // being looked after
            };
            let Some(record) = store.load::<Record>(&name) else {
                continue;
            };
            found.push(Arc::new(Self::with(
                account.to_string(),
                None,
                client.clone(),
                name,
                record,
                Some(store.clone()),
                Some(claim),
            )));
        }
        found
    }

    fn locked(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn stage(&self) -> Stage {
        self.locked().record.stage
    }

    pub fn subject(&self) -> String {
        self.locked().record.mail.outgoing.subject.clone()
    }

    fn mail(&self) -> QueuedMail {
        self.locked().record.mail.clone()
    }

    /// Writes it down; whether that worked. Blocking.
    pub fn keep(&self) -> bool {
        self.write()
    }

    /// The same on Send, which is worth asking the keychain again for. Blocking.
    pub fn keep_asking(&self) -> bool {
        if Store::unreadable(&self.account)
            && let Ok(store) = Store::open(&self.account)
        {
            self.locked().store.get_or_insert(Arc::new(store));
        }
        self.keep()
    }

    /// Whether it is on disk.
    pub fn kept(&self) -> bool {
        self.kept.load(Ordering::Relaxed)
    }

    /// Writes down what it is now, with the lock held only to copy it: the main thread
    /// takes the lock too, and a record with attachments takes a while to write.
    fn write(&self) -> bool {
        let (record, store, claimed, version) = {
            let mut state = self.locked();
            if state.record.stage == Stage::Over {
                return false;
            }
            state.version += 1;
            let snapshot = state.record.clone();
            (
                snapshot,
                state.store.clone(),
                state.claim.is_some(),
                state.version,
            )
        };
        let Some(store) = store.or_else(|| self.open_store()) else {
            self.kept.store(false, Ordering::Relaxed);
            return false;
        };
        if !claimed && let Some(claim) = store.claim(&self.name) {
            self.locked().claim.get_or_insert(claim);
        }
        let landed = store.save_if(&self.name, &record, || {
            let mut state = self.locked();
            let current = state.version == version && state.record.stage != Stage::Over;
            state.on_disk |= current;
            current
        });
        match landed {
            Ok(true) => {
                self.kept.store(true, Ordering::Relaxed);
                true
            }
            // A newer write, or `forget`, came after it.
            Ok(false) => false,
            Err(error) => {
                eprintln!("nimbra: could not save the message being sent: {error:#}");
                self.kept.store(false, Ordering::Relaxed);
                false
            }
        }
    }

    fn open_store(&self) -> Option<Arc<Store>> {
        let store = Arc::new(Store::open_quietly(&self.account).ok()?);
        Some(self.locked().store.get_or_insert(store).clone())
    }

    /// Taken back or handed on: nothing is left of it on disk. This does not wait for a
    /// background thread, as quitting straight after Undo must not send it, nor for a write
    /// of it under way, which then never lands.
    pub fn forget(&self) {
        let store = {
            let mut state = self.locked();
            state.record.stage = Stage::Over;
            state.version += 1;
            state.out = false;
            state.store.clone()
        };
        if let Some(store) = store {
            store.remove(&self.name);
        }
        self.kept.store(false, Ordering::Relaxed);
        self.changed.notify_all();
    }

    /// The draft saved since Send, which sending then uses up.
    pub fn set_draft(&self, draft_id: Option<String>) {
        if draft_id.is_some() {
            self.locked().record.mail.draft_id = draft_id;
        }
    }

    /// Sends it, once however often it is called: `None` if it was not waiting. Only once
    /// the disk says it is out, wherever the disk had it as waiting. Blocking.
    pub fn send(&self) -> Option<anyhow::Result<()>> {
        let mail = {
            let mut state = self.locked();
            if state.record.stage != Stage::Held {
                return None;
            }
            state.record.stage = Stage::Out;
            state.out = true;
            state.record.mail.clone()
        };
        if !self.write() {
            let mut state = self.locked();
            match state.record.stage {
                Stage::Out if state.on_disk => {
                    state.record.stage = Stage::Held;
                    state.out = false;
                    drop(state);
                    self.changed.notify_all();
                    return Some(Err(NotRecorded.into()));
                }
                // Nowhere to keep it: sent all the same, as before it could be kept.
                Stage::Out => {}
                _ => return None,
            }
        }
        if self.stage() != Stage::Out {
            return None; // taken back meanwhile
        }
        let result = match &mail.draft_id {
            Some(id) => self.client.send_draft(id, &mail.outgoing),
            None => self.client.send_message(&mail.outgoing),
        };
        let (stage, store) = {
            let mut state = self.locked();
            if state.record.stage == Stage::Out {
                state.record.stage = match &result {
                    Ok(()) => Stage::Over,
                    Err(error) if error.is::<mail::MayHaveArrived>() => Stage::Unsure,
                    Err(error) if error.is::<mail::Offline>() => Stage::Held,
                    Err(_) => Stage::Refused,
                };
            }
            (state.record.stage, state.store.clone())
        };
        if stage == Stage::Over {
            if let Some(store) = store {
                store.remove(&self.name);
            }
            self.kept.store(false, Ordering::Relaxed);
        } else {
            self.write();
        }
        self.locked().out = false;
        self.changed.notify_all();
        Some(result)
    }

    /// Puts it in Gmail's Drafts. Until that works it stays on disk. Blocking.
    pub fn to_drafts(&self) -> bool {
        self.saved_as_draft().is_some()
    }

    /// The same, for who needs the draft's id.
    pub fn saved_as_draft(&self) -> Option<String> {
        let mail = self.mail();
        let id = self
            .client
            .keep_as_draft(mail.draft_id.as_deref(), &mail.outgoing)
            .ok()?;
        self.forget();
        Some(id)
    }

    /// Deletes what was saved of a discarded message. Until that works it stays on disk.
    /// Blocking.
    pub fn discard(&self) -> bool {
        let deleted = match self.mail().draft_id {
            None => true,
            Some(id) => match self.client.delete_draft(&id) {
                Ok(()) => true,
                Err(error) => mail::is_gone(&error),
            },
        };
        if deleted {
            self.forget();
        }
        deleted
    }

    /// Until the answer to its request is written down.
    fn wait(&self) {
        drop(
            self.changed
                .wait_while(self.locked(), |state| state.out)
                .unwrap_or_else(PoisonError::into_inner),
        );
    }
}

/// Quitting: sends what is waiting and waits for what is out, until `deadline`. Whatever is
/// not through by then is on disk for the next launch.
pub fn finish(jobs: &[Arc<Job>], deadline: Instant) {
    let (done, finished) = mpsc::channel();
    for job in jobs {
        let (job, done) = (job.clone(), done.clone());
        std::thread::spawn(move || {
            // It may already be out, on another thread.
            if job.send().is_none() {
                job.wait();
            }
            done.send(()).ok();
        });
    }
    for _ in jobs {
        let left = deadline.saturating_duration_since(Instant::now());
        if finished.recv_timeout(left).is_err() {
            break;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::mail::{Change, Changes, Draft, Label, Message, Outgoing, Provider, Summary, View};

    type Answer = Box<dyn Fn() -> anyhow::Result<()> + Send + Sync>;

    /// A connection that only sends: it counts the requests and answers with `answer`.
    pub(crate) struct Fake {
        pub(crate) requests: AtomicUsize,
        answer: Answer,
    }

    impl Fake {
        pub(crate) fn new(
            answer: impl Fn() -> anyhow::Result<()> + Send + Sync + 'static,
        ) -> Arc<Self> {
            Arc::new(Self {
                requests: AtomicUsize::new(0),
                answer: Box::new(answer),
            })
        }

        pub(crate) fn requests(&self) -> usize {
            self.requests.load(Ordering::SeqCst)
        }

        fn sent(&self) -> anyhow::Result<()> {
            self.requests.fetch_add(1, Ordering::SeqCst);
            (self.answer)()
        }
    }

    impl Provider for Fake {
        fn send_draft(&self, _: &str, _: &Outgoing) -> anyhow::Result<()> {
            self.sent()
        }
        fn send_message(&self, _: &Outgoing) -> anyhow::Result<()> {
            self.sent()
        }
        fn send_saved_draft(&self, _: &str) -> anyhow::Result<()> {
            self.sent()
        }
        fn profile(&self) -> anyhow::Result<(String, String)> {
            unreachable!()
        }
        fn writes(&self) -> u64 {
            unreachable!()
        }
        fn forget_rows(&self) {}
        fn keep_rows_in(&self, _: &Arc<Store>) {}
        fn history(&self, _: &str) -> anyhow::Result<Changes> {
            unreachable!()
        }
        fn threads_page(
            &self,
            _: View,
            _: Option<&str>,
            _: usize,
            _: Option<&str>,
        ) -> anyhow::Result<(Vec<Summary>, Option<String>)> {
            unreachable!()
        }
        fn page_after(&self, _: View, _: Option<&str>, _: usize) -> anyhow::Result<Option<String>> {
            unreachable!()
        }
        fn summary(&self, _: &str, _: bool) -> anyhow::Result<Summary> {
            unreachable!()
        }
        fn thread(&self, _: &str) -> anyhow::Result<Vec<Message>> {
            unreachable!()
        }
        fn download(&self, _: &str, _: &str) -> anyhow::Result<Vec<u8>> {
            unreachable!()
        }
        fn change(&self, _: &str, _: Change) -> anyhow::Result<()> {
            unreachable!()
        }
        fn draft_for_thread(&self, _: &str) -> anyhow::Result<Draft> {
            unreachable!()
        }
        fn save_draft(&self, _: Option<&str>, _: &Outgoing) -> anyhow::Result<String> {
            unreachable!()
        }
        fn keep_as_draft(&self, _: Option<&str>, _: &Outgoing) -> anyhow::Result<String> {
            unreachable!()
        }
        fn delete_draft(&self, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        fn delete_thread(&self, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        fn labels(&self) -> anyhow::Result<Vec<Label>> {
            unreachable!()
        }
        fn create_label(&self, _: &str) -> anyhow::Result<Label> {
            unreachable!()
        }
        fn rename_label(&self, _: &str, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        fn delete_label(&self, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        fn add_label(&self, _: &str, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        fn remove_label(&self, _: &str, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        fn relabel(&self, _: &str, _: &[&str], _: &[&str]) -> anyhow::Result<()> {
            unreachable!()
        }
        fn counts(&self, _: &[String]) -> std::collections::HashMap<String, (u32, u32)> {
            unreachable!()
        }
    }

    fn message() -> QueuedMail {
        QueuedMail {
            draft_id: None,
            outgoing: Outgoing {
                to: "ana@example.com".into(),
                subject: "Quarterly".into(),
                files: vec![mail::OutgoingFile {
                    name: "a.txt".into(),
                    mime: "text/plain".into(),
                    bytes: b"hi".as_slice().into(),
                }],
                ..Default::default()
            },
        }
    }

    fn stage_on_disk(store: &Store) -> Option<Stage> {
        let name = store.names(RECORD).pop()?;
        store.load::<Record>(&name).map(|record| record.stage)
    }

    #[test]
    fn the_disk_says_out_before_the_request_leaves() {
        let store = Arc::new(Store::temp());
        let seen = Arc::new(Mutex::new(None));
        let client = Fake::new({
            let (store, seen) = (store.clone(), seen.clone());
            move || {
                *seen.lock().unwrap() = stage_on_disk(&store);
                Ok(())
            }
        });
        let job = Job::new(
            String::new(),
            1,
            client.clone(),
            Some(store.clone()),
            message(),
        )
        .unwrap();
        assert!(job.keep());
        assert_eq!(stage_on_disk(&store), Some(Stage::Held));
        assert!(job.send().unwrap().is_ok());
        assert_eq!(*seen.lock().unwrap(), Some(Stage::Out));
        assert_eq!(stage_on_disk(&store), None, "sent: nothing is left of it");
        std::fs::remove_dir_all(store.dir()).unwrap();
    }

    #[test]
    fn a_message_out_when_the_app_ended_is_never_sent_again() {
        let store = Arc::new(Store::temp());
        let before = Fake::new(|| Ok(()));
        let job = Job::new(String::new(), 1, before, Some(store.clone()), message()).unwrap();
        assert!(job.keep());
        // The app ends during the request: the record is left as it was when it went out.
        let record = store.load::<Record>(&job.name).unwrap();
        store
            .save(
                &job.name,
                &Record {
                    stage: Stage::Out,
                    ..record
                },
            )
            .unwrap();
        drop(job);

        let client = Fake::new(|| Ok(()));
        let client_dyn: Arc<mail::Client> = client.clone();
        let found = Job::left_behind("", &client_dyn, &store);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].stage(), Stage::Out);
        assert!(found[0].send().is_none());
        assert_eq!(client.requests(), 0);
        std::fs::remove_dir_all(store.dir()).unwrap();
    }

    #[test]
    fn a_kept_message_is_not_sent_when_out_cannot_be_written() {
        let store = Arc::new(Store::temp());
        let client = Fake::new(|| Ok(()));
        let job = Job::new(
            String::new(),
            1,
            client.clone(),
            Some(store.clone()),
            message(),
        )
        .unwrap();
        assert!(job.keep());
        // The disk goes away (full, or failing) between Send and the request.
        std::fs::remove_dir_all(store.dir()).unwrap();
        let refused = job.send().unwrap().unwrap_err();
        assert!(refused.is::<NotRecorded>());
        assert_eq!(client.requests(), 0);
        assert_eq!(
            job.stage(),
            Stage::Held,
            "can be sent once the disk takes it"
        );
    }

    #[test]
    fn one_that_cannot_be_kept_anywhere_is_still_sent() {
        let store = Arc::new(Store::temp());
        let client = Fake::new(|| Ok(()));
        let job = Job::new(
            String::new(),
            1,
            client.clone(),
            Some(store.clone()),
            message(),
        )
        .unwrap();
        std::fs::remove_dir_all(store.dir()).unwrap();
        assert!(job.send().unwrap().is_ok());
        assert_eq!(client.requests(), 1);
    }

    #[test]
    fn a_message_taken_back_is_never_written_again() {
        let store = Arc::new(Store::temp());
        let client = Fake::new(|| Ok(()));
        let job = Job::new(
            String::new(),
            1,
            client.clone(),
            Some(store.clone()),
            message(),
        )
        .unwrap();
        assert!(job.keep());
        job.forget();
        assert!(store.names(RECORD).is_empty());
        assert!(!job.keep());
        assert!(job.send().is_none());
        assert!(store.names(RECORD).is_empty());
        assert_eq!(client.requests(), 0);
        std::fs::remove_dir_all(store.dir()).unwrap();
    }

    #[test]
    fn records_written_before_attachments_were_base64_still_load() {
        let store = Arc::new(Store::temp());
        let legacy = serde_json::json!({
            "stage": "Held",
            "mail": {
                "draft_id": null,
                "outgoing": {
                    "to": "ana@example.com", "cc": "", "bcc": "", "subject": "Old",
                    "body": "", "html": null, "kept": [], "reply": null,
                    "files": [{ "name": "a.txt", "mime": "text/plain", "bytes": [104, 105] }]
                }
            }
        });
        store.save("sending-0123456789abcdef", &legacy).unwrap();
        let client: Arc<mail::Client> = Fake::new(|| Ok(()));
        let found = Job::left_behind("", &client, &store);
        let files = found[0].mail().outgoing.files;
        assert_eq!(&*files[0].bytes, b"hi");

        assert!(found[0].keep());
        let rewritten: serde_json::Value = store.load("sending-0123456789abcdef").unwrap();
        assert_eq!(rewritten["mail"]["outgoing"]["files"][0]["bytes"], "aGk=");
        std::fs::remove_dir_all(store.dir()).unwrap();
    }
    fn imap_client() -> Arc<mail::Client> {
        let server =
            |port| crate::servers::Server::new("localhost", port, crate::servers::Security::Tls);
        Arc::new(crate::imap_mail::Client::new(
            None,
            String::new(),
            crate::servers::Servers {
                username: "me".into(),
                imap: server(1),
                smtp: server(1),
            },
            crate::imap_mail::Credential::Password(String::new()),
        ))
    }

    fn with_file(bytes: &[u8]) -> QueuedMail {
        QueuedMail {
            draft_id: None,
            outgoing: mail::Outgoing {
                subject: "Report".into(),
                files: vec![mail::OutgoingFile {
                    name: "a.bin".into(),
                    mime: "application/octet-stream".into(),
                    bytes: bytes.into(),
                }],
                ..mail::Outgoing::default()
            },
        }
    }

    #[test]
    fn taking_back_does_not_wait_for_a_write_and_leaves_nothing_on_disk() {
        let (store, release) = Store::temp_held();
        let job = Job::new(
            String::new(),
            1,
            imap_client(),
            Some(store.clone()),
            with_file(&[1; 64]),
        )
        .unwrap();
        // Its record is busy on the writer, so a write of it waits.
        drop(store.read_soon::<u32>(&job.name));
        let writing = std::thread::spawn({
            let job = job.clone();
            move || job.keep()
        });
        job.forget();
        assert_eq!(job.stage(), Stage::Over);
        release.send(()).unwrap();
        assert!(!writing.join().unwrap());
        assert!(store.load::<serde_json::Value>(&job.name).is_none());
        assert!(job.send().is_none(), "taken back: never sent");
    }

    #[test]
    fn a_kept_message_is_written_compactly_and_reads_back() {
        let store = Arc::new(Store::temp());
        let bytes: Vec<u8> = (0..=255).cycle().take(30_000).collect();
        let job = Job::new(
            String::new(),
            1,
            imap_client(),
            Some(store.clone()),
            with_file(&bytes),
        )
        .unwrap();
        assert!(job.keep() && job.kept());
        let raw: serde_json::Value = store.load(&job.name).unwrap();
        let written = raw["mail"]["outgoing"]["files"][0]["bytes"]
            .as_str()
            .unwrap();
        assert!(
            written.len() < bytes.len() * 3 / 2,
            "base64, not a list of numbers"
        );
        drop(job.locked().claim.take());
        let found = Job::left_behind("", &imap_client(), &store);
        assert_eq!(*found[0].mail().outgoing.files[0].bytes, *bytes);
        assert_eq!(found[0].stage(), Stage::Held);
    }

    #[test]
    fn a_message_kept_by_an_earlier_version_still_reads_back() {
        let store = Arc::new(Store::temp());
        // As written before files were base64: each byte a number.
        let old = serde_json::json!({
            "stage": "Held",
            "mail": {
                "draft_id": "r123",
                "outgoing": {
                    "to": "ana@example.com", "cc": "", "bcc": "", "subject": "Old",
                    "body": "Hi", "html": null, "kept": [], "reply": null,
                    "files": [{"name": "a.txt", "mime": "text/plain", "bytes": [104, 105, 0, 255]}]
                }
            }
        });
        store.save("sending-0123456789abcdef", &old).unwrap();
        let found = Job::left_behind("", &imap_client(), &store);
        let mail = found[0].mail();
        assert_eq!(mail.draft_id.as_deref(), Some("r123"));
        assert_eq!(*mail.outgoing.files[0].bytes, [104, 105, 0, 255]);
        assert_eq!(found[0].subject(), "Old");
    }
}
