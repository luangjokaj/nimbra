//! The reading pane: a thread and its messages, their headers and attachments, the original
//! HTML view beside the native one, printing and unsubscribing.

use super::*;
use crate::print;

/// "Fri, Nov 21, 2026 at 9:41 PM", as the Mac shows dates (`when`); empty when unknown.
pub(super) fn long_date(timestamp_ms: i64) -> String {
    if timestamp_ms <= 0 {
        return String::new();
    }
    crate::when::millis(timestamp_ms, crate::when::LONG)
}

fn downloads_dir() -> anyhow::Result<std::path::PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("no home folder"))?;
    Ok(std::path::PathBuf::from(home).join("Downloads"))
}

/// Writes `bytes` to a new file in `dir` named `name` or, when that is taken, "Plan 2.pdf"
/// and on. Only a file it creates is opened: never one already there, nor through a link.
pub(super) fn save_new(
    dir: &std::path::Path,
    name: &str,
    bytes: &[u8],
) -> anyhow::Result<std::path::PathBuf> {
    use std::io::Write as _;
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
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = match options.open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        };
        quarantine(&file);
        if let Err(e) = file.write_all(bytes) {
            let _ = std::fs::remove_file(&path);
            return Err(e.into());
        }
        return Ok(path);
    }
    anyhow::bail!("too many files with that name in Downloads")
}

/// Marks a file as downloaded, as browsers and Mail do, so Gatekeeper checks it when it is
/// opened (and Archive Utility passes that on to what it unpacks). Without it, an app
/// mailed in a zip would run unchecked. The value is flags (0x81: downloaded), the time
/// in hex, and the agent; the event id that may follow is optional.
#[cfg(target_os = "macos")]
fn quarantine(file: &std::fs::File) {
    use std::ffi::{c_char, c_int, c_void};
    use std::os::fd::AsRawFd as _;
    unsafe extern "C" {
        fn fsetxattr(
            fd: c_int,
            name: *const c_char,
            value: *const c_void,
            size: usize,
            position: u32,
            options: c_int,
        ) -> c_int;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let value = format!("0081;{now:08x};Nimbra;");
    // SAFETY: `file` keeps the descriptor open for the call, the name is NUL-terminated and
    // `value` lives through the call with exactly `value.len()` bytes.
    let failed = unsafe {
        fsetxattr(
            file.as_raw_fd(),
            c"com.apple.quarantine".as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
            0,
        )
    } != 0;
    if failed {
        eprintln!("nimbra: a saved file could not be marked as downloaded");
    }
}

#[cfg(not(target_os = "macos"))]
fn quarantine(_: &std::fs::File) {}

fn previews_root() -> std::path::PathBuf {
    std::env::temp_dir().join("nimbra-previews")
}

/// The folder of an account's Quick Look copies.
fn account_previews(root: &std::path::Path, account: &str) -> Option<std::path::PathBuf> {
    mail::is_account_id(account).then(|| root.join(format!("account-{account}")))
}

/// A private temporary folder for one Quick Look preview of `account`'s mail.
pub(super) fn preview_dir(account: &str) -> anyhow::Result<std::path::PathBuf> {
    previews_in(&previews_root(), account)
}

fn previews_in(root: &std::path::Path, account: &str) -> anyhow::Result<std::path::PathBuf> {
    let folder =
        account_previews(root, account).ok_or_else(|| anyhow::anyhow!("unexpected account id"))?;
    private(root)?;
    private_folder_in(&folder)
}

fn private_folder_in(root: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    let mut id = [0u8; 8];
    getrandom::getrandom(&mut id).map_err(|e| anyhow::anyhow!("{e}"))?;
    let dir = root.join(id.iter().map(|b| format!("{b:02x}")).collect::<String>());
    private(root)?;
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(&dir)?;
    Ok(dir)
}

/// Makes `folder` (and what leads to it) if it is not there, and only the user's.
fn private(folder: &std::path::Path) -> anyhow::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
        builder.mode(0o700);
        builder.create(folder)?;
        if std::fs::symlink_metadata(folder)?.file_type().is_symlink() {
            anyhow::bail!("the previews folder is a link");
        }
        // Made before this version kept it private.
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    builder.create(folder)?;
    Ok(())
}

/// The Quick Look copies there are now (they are mail, decrypted), to remove off the main
/// thread with `remove_previews`: none made after this is among them. At launch.
pub(crate) fn previews_now() -> Vec<std::path::PathBuf> {
    previews_now_in(&previews_root())
}

fn previews_now_in(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let children = |folder: &std::path::Path| -> Vec<std::path::PathBuf> {
        std::fs::read_dir(folder)
            .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
            .unwrap_or_default()
    };
    children(root)
        .into_iter()
        .flat_map(|path| {
            let is_folder = std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir());
            let of_account = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("account-"));
            // Earlier versions kept every preview right in the root.
            if is_folder && of_account {
                children(&path)
            } else {
                vec![path]
            }
        })
        .collect()
}

/// Blocking.
pub(crate) fn remove_previews(previews: Vec<std::path::PathBuf>) {
    for path in previews {
        let is_folder = std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir());
        let _ = if is_folder {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
    }
}

/// Removes an account's Quick Look copies, on signing out. Blocking.
pub(crate) fn forget_previews(account: &str) {
    forget_previews_in(&previews_root(), account);
}

fn forget_previews_in(root: &std::path::Path, account: &str) {
    if let Some(folder) = account_previews(root, account) {
        let _ = std::fs::remove_dir_all(folder);
    }
}

/// A button on an attachment: its icon, a turning arrow while it downloads, or a green
/// check mark once saved.
pub(super) fn attachment_button(t: &Theme, name: &str, busy: bool, done: bool) -> Div {
    let hover = t.hover;
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(dp(30.))
        .rounded(dp(8.))
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
/// What the hidden print view tells the app.
pub(super) enum PrintEvent {
    /// A page has loaded.
    Loaded,
    /// The page's answer to `print::READY`.
    Ready(String),
}

/// "1.2 MB", "340 KB".
pub(crate) fn file_size(bytes: u64) -> String {
    match bytes {
        0 => String::new(),
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{} KB", b / 1024),
        b => format!("{:.1} MB", b as f64 / (1024. * 1024.)),
    }
}

impl Nimbra {
    /// Switches the open thread between the native reader and its original HTML.
    pub(super) fn toggle_original(&mut self, cx: &mut Context<Self>) {
        let has_html = self
            .mails
            .get(self.selected)
            .is_some_and(|mail| mail.has_html());
        if !self.original && !has_html {
            self.notice = Some("This message has no HTML version to show.".into());
        } else {
            self.original = !self.original;
        }
        cx.notify();
    }

    /// Whether the open thread's emails show in their own formatting right now. They are
    /// hidden whenever something is drawn over the reading pane, because a native web view
    /// always sits on top of everything GPUI paints.
    fn original_shown(&self) -> bool {
        // (A toolbar menu does not hide them: menus open in a window of their own.)
        let covered = self.palette
            || self.finder_open
            || self.picked.len() >= 2
            || self.compose.is_some()
            || self.new_label.is_some()
            || self.account_sheet_over_main()
            || self.confirm.is_some();
        !covered && self.visible().contains(&self.selected)
    }

    /// Whether the open thread is read in its original formatting (its messages with HTML
    /// as web views), shown or not.
    pub(super) fn original_on(&self) -> bool {
        self.original
            && matches!(self.status, Status::Demo | Status::Ready)
            && self.mailbox.base() != View::Drafts
            && self.mails.get(self.selected).is_some_and(Mail::has_html)
    }

    /// The open thread's key among the web views: its account and id, not its row (new
    /// mail would load the emails again, undoing Block images).
    fn original_thread(&self, mail: &Mail) -> String {
        let (thread, account) = self.thread_key(mail);
        format!("{account}:{thread}")
    }

    /// Whether light emails are given a dark page right now (Settings › Reading).
    pub(super) fn darkens_mail(&self) -> bool {
        cfg!(target_os = "macos") && self.dark && self.settings.dark_emails
    }

    /// Creates, loads, shows or hides the web view to match the current state.
    pub(super) fn sync_html_view(&mut self, window: &mut Window) {
        self.settle_folds();
        self.fold_lead.set(None);
        // A print job has a view of its own, so the email shown does not replace it, and it
        // is light whatever the theme.
        if let Some(doc) = self.print_pending.take() {
            if self.print_view.is_none() {
                let events = self.print_events.clone();
                self.print_view = HtmlView::with_loaded(window, move || {
                    let _ = events.unbounded_send(PrintEvent::Loaded);
                });
            }
            if let Some(view) = &self.print_view {
                view.set_dark(false, false);
                // Never shown, but it needs a page's size to lay the document out.
                view.set_bounds(0., 0., 800., 1000.);
                view.show_page(&doc);
            }
        }
        let Some(mail) = self.mails.get(self.selected).filter(|_| self.original_on()) else {
            self.mail_views.clear();
            return;
        };
        let thread = self.original_thread(mail);
        // A different thread starts with its own images setting.
        if self.html_shown.as_ref() != Some(&thread) {
            self.original_images = self.settings.load_images;
            self.html_shown = Some(thread.clone());
        }
        let mut wanted = crate::mail_views::wanted(mail, &thread, false, !self.latest_folded(mail));
        // The conversations beside it in the list: their latest email loaded and laid out,
        // hidden, so that moving to one (J, K, a click) shows it at once.
        let visible = self.visible();
        if let Some(at) = visible.iter().position(|row| *row == self.selected) {
            let near = [at.checked_sub(1), at.checked_add(1)];
            for row in near.into_iter().flatten().filter_map(|at| visible.get(at)) {
                let Some(other) = self.mails.get(*row) else {
                    continue;
                };
                let Some(html) = other.html.as_deref() else {
                    continue;
                };
                wanted.push(crate::mail_views::Wanted {
                    key: crate::mail_views::message_key(
                        &self.original_thread(other),
                        &other.header.message_id,
                        0,
                    ),
                    html,
                    open: false,
                    latest: true,
                    // By field (not `account_of`): the views are changed below.
                    account: Some(other.account.as_deref().unwrap_or(&self.account_id)),
                });
            }
        }
        let look = crate::mail_views::Look {
            images: self.original_images,
            dark: self.dark,
            darkened: self.darkens_mail(),
        };
        // The emails are above all the app draws, so the floating sidebar gets a hole in
        // them, a frame ahead while it moves (see `pane_lead`).
        let ahead = if self.peek.moving() || self.sidebar_sliding() {
            self.frame_interval.get()
        } else {
            0.
        };
        let height = from_screen(window.viewport_size().height);
        let hole = self
            .sidebar_place(ahead)
            .filter(|place| place.inset > 0.)
            .map(|place| {
                (
                    place.x,
                    place.inset,
                    self.side_w(),
                    height - 2. * place.inset,
                )
            });
        let account = self.account_of(mail).to_string();
        let shown = self.original_shown();
        // The mail text size reaches emails in their own formatting too.
        let zoom = crate::webview::zoom_for(self.settings.reader_text);
        if !self
            .mail_views
            .sync(wanted, look, &account, zoom, shown, hole, window)
        {
            // No web view on this platform: the native reader.
            self.original = false;
            self.mail_views.clear();
            self.notice = Some("The original view is not available on this system.".into());
            return;
        }
        // An email shows once its text is laid out, without waiting for its images.
        self.mail_views.measure_waiting();
        // Pictures of the open emails, for while the conversation moves (`email_frame`).
        let moving = self.folds.borrow().values().any(|fold| !fold.done);
        self.mail_views.take_snapshots(moving);
    }

    /// ⌘P, ⇧⌘P and the More menu: prints the open conversation, or the message being read,
    /// on Nimbra's page (`print.rs`). Each email prints as it is shown: its own HTML in the
    /// original view, the reader's text otherwise. A thread of one message prints it with
    /// its full headers either way.
    pub(super) fn print_selected(&mut self, scope: print::Scope, cx: &mut Context<Self>) {
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
        // Messages by their place in the reader: 0 is the latest, k + 1 is `earlier[k]`.
        let count = mail.earlier.len() + 1;
        let chosen: Vec<usize> = match scope {
            // Oldest first: `earlier` is newest first, and the latest ends it.
            print::Scope::Conversation if count > 1 => (1..count).rev().chain([0]).collect(),
            _ => vec![self.message_read(mail)],
        };
        let scope = if chosen.len() > 1 {
            print::Scope::Conversation
        } else {
            print::Scope::Message
        };
        let original = self.original_on();
        let messages: Vec<print::Message> = chosen
            .iter()
            .map(|&index| self.print_message(mail, index, original))
            .collect();
        // Remote images as the reading view has them: the original view's setting, or in
        // the reader only when every one printed has been loaded already.
        let images = if original {
            self.original_images
        } else {
            let mut remote = messages
                .iter()
                .filter_map(|m| match m.body {
                    print::Body::Blocks(blocks) => Some(blocks),
                    print::Body::Html(_) => None,
                })
                .flatten()
                .filter_map(|block| match &block.kind {
                    Kind::Image { src, .. } if src.starts_with("https:") => Some(src),
                    _ => None,
                })
                .peekable();
            remote.peek().is_some() && remote.all(|src| self.images.contains_key(src))
        };
        self.print_job += 1;
        let job = self.print_job;
        let subject = mail.shown_subject();
        let page = print::Page {
            job,
            subject: &subject,
            scope,
            place: (count - chosen[0], count),
            messages,
            printed: format!(
                "Printed from Nimbra on {}",
                crate::when::local(chrono::Local::now(), crate::when::LONG)
            ),
            images,
        };
        self.print_pending = Some(print::document(&page));
        self.print_waiting = Some(job);
        cx.notify();
        // The page says when it has loaded (`print_event`). One that never does (an image
        // that never arrives) is printed as it is after a while.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(8)).await;
            this.update(cx, |this, _| {
                if this.print_waiting == Some(job) {
                    this.print_waiting = None;
                    if let Some(view) = &this.print_view {
                        view.print();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// The message being read in the open thread: the latest, or when it is folded the
    /// newest one opened.
    fn message_read(&self, mail: &Mail) -> usize {
        if !self.latest_folded(mail) {
            return 0;
        }
        mail.earlier
            .iter()
            .position(|e| e.expanded)
            .map_or(0, |k| k + 1)
    }

    /// One message of `mail` for the printed page (`index` as in `print_selected`).
    fn print_message<'a>(
        &self,
        mail: &'a Mail,
        index: usize,
        original: bool,
    ) -> print::Message<'a> {
        let (header, blocks, html, sender, tone, initials, time) = match index {
            0 => (
                &mail.header,
                &mail.body[..],
                mail.html.as_deref(),
                self.latest_from(mail).to_string(),
                mail.tone,
                mail.initials.to_string(),
                mail.time.to_string(),
            ),
            k => {
                let e = &mail.earlier[k - 1];
                let (tone, initials, when) = mail.earlier_look(k - 1);
                (
                    &e.header,
                    &e.blocks[..],
                    e.html.as_deref(),
                    e.sender.clone(),
                    tone,
                    initials.to_string(),
                    when.to_string(),
                )
            }
        };
        let body = match html.filter(|_| original) {
            Some(html) => print::Body::Html(html),
            None => print::Body::Blocks(blocks),
        };
        let date = long_date(header.date_ms);
        print::Message {
            header,
            sender,
            initials,
            tone: tone.stops(),
            date: if date.is_empty() { time } else { date },
            body,
        }
    }

    /// What the print view says. Once the page of the job waiting has loaded and given its
    /// emails their shadow roots, the print panel opens.
    pub(super) fn print_event(&mut self, event: PrintEvent, cx: &mut Context<Self>) {
        let (Some(job), Some(view)) = (self.print_waiting, &self.print_view) else {
            return;
        };
        match event {
            PrintEvent::Loaded => {
                let events = self.print_events.clone();
                view.when_ready(move |answer| {
                    let _ = events.unbounded_send(PrintEvent::Ready(answer));
                });
            }
            PrintEvent::Ready(answer) => {
                // Another page (the empty one a new view starts with, or an earlier job).
                if print::ready(&answer).map(|(ready, _)| ready) != Some(job) {
                    return;
                }
                self.print_waiting = None;
                let attached = print::ready(&answer).is_some_and(|(_, attached)| attached);
                if !attached {
                    view.print();
                    return;
                }
                // Emails given their shadow roots just now still load their images.
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(600))
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
        }
    }

    /// Downloads an attachment, then either previews it with Quick Look (from a private
    /// temporary folder) or saves it to Downloads and shows it in Finder. Nothing is opened
    /// with another app automatically.
    fn fetch_attachment(
        &mut self,
        att: mail::Attachment,
        account: Option<&str>,
        save: bool,
        cx: &mut Context<Self>,
    ) {
        // Through the account of the conversation it is in.
        let Some(client) = self.client_for(account) else {
            self.notice = Some("Attachments need a connected account.".into());
            cx.notify();
            return;
        };
        if self.attachment_busy.contains_key(&att.attachment_id) {
            return;
        }
        let key = att.attachment_id.clone();
        let from = account.unwrap_or(&self.account_id).to_string();
        self.attachment_busy.insert(key.clone(), save);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = within(cx.background_executor(), async move {
                let bytes = client.download(&att.message_id, &att.attachment_id)?;
                let name = mail::safe_file_name(&att.name);
                let dir = if save {
                    downloads_dir()?
                } else {
                    preview_dir(&from)?
                };
                save_new(&dir, &name, &bytes)
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
        let Some(mail) = mail::unsubscribe_mail(link) else {
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
        composer.signature = None;
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

    /// Space and Shift-Space: the conversation a page further or back.
    pub(super) fn page_message(&mut self, down: bool, cx: &mut Context<Self>) {
        if self.palette || self.pane == Pane::Sidebar || self.visible().is_empty() {
            return;
        }
        let handle = &self.reader_scroll;
        let page = f32::from(handle.bounds().size.height) * 0.9;
        let max = f32::from(handle.max_offset().height);
        let offset = handle.offset();
        let y = (f32::from(offset.y) + if down { -page } else { page }).clamp(-max, 0.);
        handle.set_offset(gpui::point(offset.x, px(y)));
        cx.notify();
    }

    /// One message drawn natively from its blocks. `base` keeps element ids unique when
    /// several messages are on screen.
    /// A message's text, selectable where it is drawn (`selectable`).
    pub(super) fn message_body(
        &self,
        t: &Theme,
        blocks: &[Block],
        base: usize,
        selectable: &super::selection::Selectable,
        account: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let sources = rich::image_sources(blocks);
        let pending = sources.iter().any(|src| !self.image_requests.contains(src));
        div()
            .flex()
            .flex_col()
            .gap(dp(12.))
            .text_size(dp(self.settings.reader_text.metrics().0))
            .line_height(dp(self.settings.reader_text.metrics().1))
            .text_color(t.text2)
            .when(pending, |d| {
                d.child(
                    card(t)
                        .flex()
                        .items_center()
                        .gap(dp(10.))
                        .pl(dp(12.))
                        .pr(dp(6.))
                        .py(dp(6.))
                        .text_size(dp(13.))
                        .line_height(dp(18.))
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
                    .map(|(i, block)| self.block(t, block, base + i, selectable, account, cx)),
            )
    }

    fn block(
        &self,
        t: &Theme,
        block: &Block,
        id: usize,
        selectable: &super::selection::Selectable,
        account: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
            // The selection (see `selection.rs`), over the spans.
            let selected = self
                .selection_in(&selectable.place)
                .and_then(|s| s.in_block(id, block.text.len()));
            let highlights =
                super::selection::with_selection(highlights, selected, t.violet.opacity(0.3));
            let styled = StyledText::new(block.text.clone()).with_highlights(highlights);
            let layout = styled.layout().clone();
            let links: Vec<(std::ops::Range<usize>, String)> = block
                .spans
                .iter()
                .filter_map(|span| Some((span.range.clone(), span.link.clone()?)))
                .collect();
            let inner = if links.is_empty() {
                styled.into_any_element()
            } else {
                // Links open in the system browser; only http, https and mailto get this far.
                let (ranges, urls): (Vec<_>, Vec<_>) = links.into_iter().unzip();
                let account = account.to_string();
                gpui::InteractiveText::new(("link-text", id), styled)
                    .on_click(ranges, move |index, window, cx| {
                        let link = &urls[index];
                        let from = Some(account.clone());
                        let here = Some(window.window_handle());
                        if !crate::integration::mailto_from_mail(link, from, here) {
                            cx.open_url(link);
                        }
                    })
                    .into_any_element()
            };
            // Selecting: press on the text, drag (across blocks too, see `selection.rs`),
            // Shift to extend.
            selectable.layouts.borrow_mut().push((id, layout.clone()));
            let pressed = layout;
            let place = selectable.place.clone();
            div()
                .cursor(gpui::CursorStyle::IBeam)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                        let offset = pressed
                            .index_for_position(event.position)
                            .unwrap_or_else(|nearest| nearest);
                        let point = super::selection::Point { block: id, offset };
                        this.select_from(place.clone(), point, event.modifiers.shift, cx);
                    }),
                )
                .child(inner)
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
                    .pt(dp(6.))
                    .text_size(dp(size))
                    .line_height(dp(size * 1.35))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text)
                    .child(text(t.text))
            }
            Kind::Item { marker, depth } => div()
                .flex()
                .gap(dp(10.))
                .pl(dp(4. + 20. * *depth as f32))
                .child(
                    div()
                        .flex_none()
                        .min_w(dp(14.))
                        .text_color(t.text3)
                        .child(marker.clone()),
                )
                .child(div().flex_1().min_w(dp(0.)).child(text(t.text))),
            Kind::Code => div()
                .px(dp(14.))
                .py(dp(10.))
                .rounded(dp(10.))
                .bg(t.hover)
                .border_1()
                .border_color(t.border)
                .font_family(crate::theme::MONO)
                .text_size(dp(12.5))
                .line_height(dp(20.))
                .child(text(t.text)),
            Kind::Rule => div().h(dp(1.)).my(dp(6.)).bg(t.border),
            Kind::Image { src, alt } => match self.images.get(src) {
                Some(image) => {
                    div().child(gpui::img(image.clone()).max_w(dp(560.)).rounded(dp(10.)))
                }
                None => div()
                    .flex()
                    .items_center()
                    .gap(dp(8.))
                    .text_size(dp(13.))
                    .line_height(dp(18.))
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
                .ml(dp(14. * (block.quote.min(4) - 1) as f32))
                .pl(dp(14.))
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
                .gap(dp(10.))
                .child(
                    div()
                        .size(dp(16.))
                        .flex_none()
                        .rounded(dp(5.))
                        .border_1()
                        .border_color(t.border_strong),
                )
                .child(div().flex_1().child(text.to_string()))
                .child(mono(due.to_string(), t.amber))
        };
        // GPUI has no gradient strokes, so the border is a 1px gradient frame around the card.
        div()
            .p(dp(1.))
            .rounded(dp(14.))
            .bg(gradient(100., c(0x8b7cff, 0.75), c(0xffb86b, 0.3)))
            .shadow(vec![shadow(c(0x8b7cff, 0.16), 12., 40., -12.)])
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(dp(12.))
                    .px(dp(20.))
                    .pt(dp(16.))
                    .pb(dp(18.))
                    .rounded(dp(13.))
                    .bg(t.raised)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(dp(8.))
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
                            .text_size(dp(15.))
                            .line_height(dp(26.))
                            .text_color(t.text2)
                            .child(m.brief.clone()),
                    )
                    .when(m.rich, |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(dp(8.))
                                .child(todo("Confirm pricing tiers on slide 14", "Fri"))
                                .child(todo("Sign off on the press embargo date", "Fri")),
                        )
                    }),
            )
    }

    /// A message folded to one line, as in a thread: avatar, sender, the start of its text
    /// and its time. Mail text, sized as the list's rows are; `column` is the width it has.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn folded_line(
        &self,
        t: &Theme,
        column: f32,
        tone: Tone,
        initials: &str,
        name: &str,
        snippet: &str,
        when: &str,
        turn: f32,
    ) -> Div {
        let row_text = super::list::RowText::of(self.settings.reader_text);
        div()
            .flex()
            .items_center()
            .gap(dp(12.))
            .py(dp(FOLD_LINE_PAD))
            .text_size(dp(row_text.size))
            .line_height(dp(row_text.line))
            .border_t_1()
            .border_color(t.border)
            // As large as an open message's, so the names line up.
            .child(avatar(tone, initials, 28.))
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
                    .min_w(dp(0.))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_color(t.text3)
                    // Cut by characters to what the column leaves beside the name and
                    // the time, so it ends with an ellipsis rather than mid-letter.
                    .child(short(snippet, {
                        let scale = row_text.size / 13.;
                        let taken = 8. * scale * name.chars().count() as f32
                            + 7.5 * when.chars().count() as f32;
                        ((column - 160. - taken) / (7. * scale)).clamp(8., 110.) as usize
                    })),
            )
            .child(mono(when.to_string(), t.faint))
            .child(fold_arrow(turn, t.faint))
    }

    /// How far a message's arrow has turned: 0 folded, 1 open, and in between while it
    /// opens or folds, on the same curve (`fold_block`).
    pub(super) fn fold_turn(&self, key: &(ThreadKey, usize), open: bool) -> f32 {
        let folds = self.folds.borrow();
        match folds.get(key).filter(|f| !f.done) {
            Some(fold) => {
                let eased =
                    ease_in_out((fold.start.elapsed().as_secs_f32() / FOLD_TIME).clamp(0., 1.));
                if fold.closing { 1. - eased } else { eased }
            }
            None if open => 1.,
            None => 0.,
        }
    }

    pub(super) fn reading(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &self.mails[self.selected];
        // The subject is broken into lines by hand (see `balance_title`), for the width the
        // reader gives it.
        let pane_w = self.frame.map_or(0., |f| from_screen(f.size.width)) - self.pane_left();
        let column = if self.focus_restore.is_some() {
            pane_w.min(980.)
        } else {
            pane_w
        };
        // Reply is the one answer with its word (a narrow pane keeps every action but drops
        // their words); Reply all and Forward are icons beside it. A narrow pane also drops
        // the previous and next arrows (the list, J and K do that too). Show original is in
        // More actions (and V).
        let narrow = pane_w < 600.;
        let visible = self.visible();
        let pos = visible
            .iter()
            .position(|i| *i == self.selected)
            .unwrap_or(0)
            + 1;
        // Only switch panes when this thread really has HTML to show.
        // Drafts always use the reader, so a click on the text can open the editor.
        let is_draft = self.mailbox.base() == View::Drafts && !self.is_demo();
        let original = self.original_on();
        let thread = self.original_thread(m);
        // In the original view the latest email runs to the bottom of the pane unless
        // something follows it (its attachments, the sample's extras).
        let trailing = !m.header.attachments.is_empty() || m.rich;
        let email_last = original && !trailing;
        // Until the message has been fetched we cannot know whether it has an original
        // (HTML) version, so show a loader, not the reader view that would flash by first.
        let loading = self.original && !is_draft && !m.loaded;
        let divider = || div().h(dp(1.)).bg(t.border);
        // The latest message folded to its line (only in a conversation).
        let latest_folded = self.latest_folded(m);
        // The conversation's order (Settings › Appearance), and whether the message the
        // stack ends with is folded: the latest's, or the oldest's when it comes first.
        let newest_first = self.settings.newest_first;
        let last_folded = if newest_first {
            m.earlier.last().is_some_and(|e| !e.expanded)
        } else {
            latest_folded
        };
        let latest_line = || {
            self.folded_line(
                t,
                column,
                m.tone,
                &m.initials,
                &self.latest_from(m),
                &m.snippet,
                &m.time,
                self.fold_turn(&(self.thread_key(m), LATEST), !latest_folded),
            )
        };

        let earlier =
            |tone: Tone, initials: &str, name: &str, snippet: &str, when: &str, turn: f32| {
                self.folded_line(t, column, tone, initials, name, snippet, when, turn)
            };

        let attachment = |name: &str, info: &str, from: u32, to: u32| {
            card(t)
                .flex()
                .items_center()
                .gap(dp(10.))
                .pl(dp(8.))
                .pr(dp(14.))
                .py(dp(8.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(dp(30.))
                        .rounded(dp(8.))
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

        // The text blocks drawn this time, for selecting (`selection.rs`).
        self.reader_layouts.borrow_mut().clear();
        let selectable = super::selection::Selectable {
            place: super::selection::Place::Main(self.thread_key(m)),
            layouts: self.reader_layouts.clone(),
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(dp(0.))
            .h_full()
            .bg(t.surface)
            .relative()
            .child(self.selection_watch(self.reader_layouts.clone(), cx))
            // While a panel slides, the email keeps one width and the extra is cut off here.
            .overflow_hidden()
            // Toolbar
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(dp(2.))
                    // Nothing to clear: the window controls leave with the sidebar. The first
                    // and last icons' glyphs sit as far from the edges as Compose does from
                    // the list's line (LIST_RIGHT), past their buttons' own 7pt.
                    .pl(dp(LIST_RIGHT - 7.))
                    .pr(dp(LIST_RIGHT - 7.))
                    .h(dp(crate::ui::top_bar()))
                    .border_b_1()
                    .border_color(t.border)
                    .on_mouse_down(MouseButton::Left, drag_or_zoom)
                    .child(
                        icon_button(t, "list")
                            .id("toggle-list")
                            .press(t)
                            .tooltip(tip(
                                t.dark,
                                "Show or hide message list",
                                crate::shortcuts::label("toggle-list"),
                            ))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(
                                    *hovered,
                                    "Show or hide message list",
                                    crate::shortcuts::label("toggle-list"),
                                    cx,
                                )
                            }))
                            .when(!self.list_open, |d| d.bg(t.selected))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_list(cx))),
                    )
                    .when(is_draft, |d| {
                        d.child(div().w(dp(6.)))
                            .child(crate::ui::glow(
                                button(t, ButtonKind::Primary, "Edit draft", None)
                                    .id("edit-draft")
                                    .press_soft()
                                    .tooltip(tip(t.dark, "Open this draft in the editor", ""))
                                    .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                                        crate::ui::glow_hovered("edit-draft", *hovered, window);
                                        this.set_hint(
                                            *hovered,
                                            "Open this draft in the editor",
                                            "",
                                            cx,
                                        )
                                    }))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_draft(this.selected, false, cx)
                                    })),
                                "edit-draft",
                                false,
                            ))
                            .child(div().w(dp(8.)))
                    })
                    .child(toolbar_rule(t, 7., if narrow { 7. } else { 9. }))
                    // What is done to a thread: as words, which the narrow pane drops.
                    .child(
                        action_button(t, "done", "Done", !narrow)
                            .id("mark-done")
                            .press(t)
                            .tooltip(tip(
                                t.dark,
                                "Mark done (archive)",
                                crate::shortcuts::label("mark-done"),
                            ))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(
                                    *hovered,
                                    "Mark done (archive)",
                                    crate::shortcuts::label("mark-done"),
                                    cx,
                                )
                            }))
                            .on_click(cx.listener(|this, _, _, cx| this.mark_done(cx))),
                    )
                    .child({
                        // Where the Snooze menu drops from, also when opened with H.
                        let anchor = self.snooze_anchor.clone();
                        action_button(t, "clock", "Snooze", !narrow)
                            .id("snooze")
                            .relative()
                            .press(t)
                            .tooltip(tip(t.dark, "Snooze", crate::shortcuts::label("snooze")))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(
                                    *hovered,
                                    "Snooze",
                                    crate::shortcuts::label("snooze"),
                                    cx,
                                )
                            }))
                            .child(
                                gpui::canvas(
                                    move |bounds, _, _| {
                                        anchor.set(Some(gpui::point(
                                            bounds.left(),
                                            bounds.bottom() + to_screen(6.),
                                        )))
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .size_full(),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_snooze(this.snooze_anchor.get(), window, cx)
                            }))
                    })
                    .child(
                        action_button(t, "reply", "Reply", !narrow)
                            .id("reply")
                            .press(t)
                            .tooltip(tip(t.dark, "Reply", crate::shortcuts::label("reply")))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(
                                    *hovered,
                                    "Reply",
                                    crate::shortcuts::label("reply"),
                                    cx,
                                )
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_compose(ComposeKind::Reply, cx)
                            })),
                    )
                    .child(
                        action_button(t, "reply-all", "Reply all", false)
                            .id("reply-all")
                            .press(t)
                            .tooltip(tip(
                                t.dark,
                                "Reply all",
                                crate::shortcuts::label("reply-all"),
                            ))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(
                                    *hovered,
                                    "Reply all",
                                    crate::shortcuts::label("reply-all"),
                                    cx,
                                )
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_compose(ComposeKind::ReplyAll, cx)
                            })),
                    )
                    .child(
                        action_button(t, "forward", "Forward", false)
                            .id("forward")
                            .press(t)
                            .tooltip(tip(
                                t.dark,
                                "Forward",
                                crate::shortcuts::label("forward-message"),
                            ))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(
                                    *hovered,
                                    "Forward",
                                    crate::shortcuts::label("forward-message"),
                                    cx,
                                )
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_compose(ComposeKind::Forward, cx)
                            })),
                    )
                    .child(div().flex_1())
                    // Where it goes and what it is, as icons.
                    .child(
                        icon_button(t, "star")
                            .id("star")
                            .press(t)
                            .tooltip(tip(t.dark, "Star", crate::shortcuts::label("toggle-star")))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(
                                    *hovered,
                                    "Star",
                                    crate::shortcuts::label("toggle-star"),
                                    cx,
                                )
                            }))
                            .when(m.starred, |d| d.bg(t.selected))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_star(cx))),
                    )
                    .child(
                        icon_button(t, "tag")
                            .id("label")
                            .relative()
                            .child(menus::anchor_for(&self.menu_anchors, Menu::Labels))
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
                            .relative()
                            .child(menus::anchor_for(&self.menu_anchors, Menu::MoveTo))
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
                            .tooltip(tip(
                                t.dark,
                                "Move to trash",
                                crate::shortcuts::label("trash"),
                            ))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(
                                    *hovered,
                                    "Move to trash",
                                    crate::shortcuts::label("trash"),
                                    cx,
                                )
                            }))
                            .on_click(cx.listener(|this, _, _, cx| this.trash(cx))),
                    )
                    .child(
                        icon_button(t, "more")
                            .id("more")
                            .relative()
                            .child(menus::anchor_for(&self.menu_anchors, Menu::More))
                            .press(t)
                            .tooltip(tip(t.dark, "More actions", ""))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.set_hint(*hovered, "More actions", "", cx)
                            }))
                            .when(self.menu == Some(Menu::More), |d| d.bg(t.selected))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_menu(Menu::More, window, cx)
                            })),
                    )
                    // The previous and next thread (also J and K, and the list).
                    .when(!narrow, |d| {
                        d.child(toolbar_rule(t, 7., 7.))
                            .child(
                                icon_button(t, "up")
                                    .id("prev")
                                    .press(t)
                                    .tooltip(tip(
                                        t.dark,
                                        "Previous thread",
                                        crate::shortcuts::label("prev"),
                                    ))
                                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                        this.set_hint(
                                            *hovered,
                                            "Previous thread",
                                            crate::shortcuts::label("prev"),
                                            cx,
                                        )
                                    }))
                                    .when(pos <= 1, |d| d.opacity(0.4))
                                    .on_click(cx.listener(|this, _, _, cx| this.step(-1, cx))),
                            )
                            .child(
                                icon_button(t, "down")
                                    .id("next")
                                    .press(t)
                                    .tooltip(tip(
                                        t.dark,
                                        "Next thread",
                                        crate::shortcuts::label("next"),
                                    ))
                                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                        this.set_hint(
                                            *hovered,
                                            "Next thread",
                                            crate::shortcuts::label("next"),
                                            cx,
                                        )
                                    }))
                                    .when(pos >= visible.len(), |d| d.opacity(0.4))
                                    .on_click(cx.listener(|this, _, _, cx| this.step(1, cx))),
                            )
                    }),
            )
            // While the message loads, the line the list has while mail is checked.
            .child(self.reader_line.view(t, "reader-loading"))
            // Conversation (native reader)
            .when(loading, |d| d.child(self.loading_pane(t, m, cx.entity())))
            .when(!loading, |d| {
                d.child(
                    div()
                        .id("conversation")
                        .flex_1()
                        .min_h(dp(0.))
                        .overflow_y_scroll()
                        .track_scroll(&self.reader_scroll)
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
                            d.cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
                                this.open_draft(this.selected, false, cx)
                            }))
                        })
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(dp(if original { ORIGINAL_GAP } else { 22. }))
                                // The original view takes the pane's whole width, as an email
                                // in its own formatting does, under a compact head.
                                .px(dp(if original { ORIGINAL_SIDE } else { READER_SIDE }))
                                // Full view keeps lines a comfortable length.
                                .when(self.focus_restore.is_some(), |d| d.w_full().max_w(dp(980.)))
                                .pt(dp(if original { 16. } else { 40. }))
                                .pb(dp(if email_last {
                                    0.
                                } else if original {
                                    ORIGINAL_SIDE
                                } else {
                                    24.
                                }))
                                .when(!original, |d| {
                                    d.child(
                                        div()
                                            .flex()
                                            .gap(dp(6.))
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
                                })
                                .child(if original {
                                    self.subject_row(t, m, 26., column - 2. * ORIGINAL_SIDE, cx)
                                } else {
                                    self.subject_row(t, m, 38., column - 128., cx)
                                })
                                .child(self.thread_banners(t, m, cx))
                                .when(!original, |d| {
                                    d.child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(dp(10.))
                                            .child(
                                                div()
                                                    .relative()
                                                    .flex_none()
                                                    .h(dp(28.))
                                                    .w(dp(if m.rich { 68. } else { 48. }))
                                                    .children(
                                                        [
                                                            Some((m.tone, m.initials.clone())),
                                                            m.rich.then(|| {
                                                                (
                                                                    Tone::Mint,
                                                                    SharedString::from("JR"),
                                                                )
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
                                                                .left(dp(n as f32 * 20.))
                                                        }),
                                                    ),
                                            )
                                            .child(meta(m.participants.clone(), t.text3).flex_1())
                                            .child(mono(m.time.clone(), t.text3)),
                                    )
                                })
                                .when(!original && !m.brief.is_empty(), |d| {
                                    d.child(self.brief(t, m))
                                })
                                // A conversation's messages, each under a line, oldest first
                                // and the latest last (or the latest first, Settings ›
                                // Appearance), which fold to their lines and open again alike
                                // (a single message is drawn on its own below).
                                .when(!m.earlier.is_empty(), |d| {
                                    // In the original view the lines run across the pane,
                                    // as the one over each email does.
                                    let across = if original { ORIGINAL_SIDE } else { 0. };
                                    d.child(
                                        div()
                                            .mx(dp(-across))
                                            // A line closes the stack when the message it
                                            // ends with is folded too.
                                            .when(last_folded, |d| d.border_b_1())
                                            .border_color(t.border)
                                            .children({
                                                // `earlier` is newest first.
                                                let mut earlier_entries: Vec<AnyElement> = m
                                                    .earlier
                                                    .iter()
                                                    .enumerate()
                                                    .map(|(k, e)| {
                                                        let (tone, initials, when) =
                                                            m.earlier_look(k);
                                                        // Click a folded message to read it in
                                                        // full; open, its header stands in for its
                                                        // line, and the change is animated.
                                                        let turn = self.fold_turn(
                                                            &(self.thread_key(m), k),
                                                            e.expanded,
                                                        );
                                                        let line = earlier(
                                                            tone, &initials, &e.sender, &e.snippet,
                                                            &when, turn,
                                                        )
                                                        .px(dp(across));
                                                        if !e.expanded {
                                                            return self
                                                                .folded_message(line, k, cx)
                                                                .into_any_element();
                                                        }
                                                        let body = div()
                                                            .flex()
                                                            .flex_col()
                                                            .gap(dp(12.))
                                                            // Where its line has its text, so it opens
                                                            // without anything in it moving.
                                                            .pt(dp(FOLD_LINE_PAD))
                                                            .pb(dp(14.))
                                                            .px(dp(across))
                                                            .border_t_1()
                                                            .border_color(t.border)
                                                            .child(self.message_header(
                                                                t,
                                                                m,
                                                                k + 1,
                                                                true,
                                                                cx,
                                                            ))
                                                            .child(
                                                                self.message_text(
                                                                    t,
                                                                    m,
                                                                    k + 1,
                                                                    original.then_some(&thread),
                                                                    // The gap over it, and the gap or
                                                                    // padding under it.
                                                                    (
                                                                        12.,
                                                                        if e.header
                                                                            .attachments
                                                                            .is_empty()
                                                                        {
                                                                            14.
                                                                        } else {
                                                                            12. - ORIGINAL_SIDE
                                                                        },
                                                                    ),
                                                                    &selectable,
                                                                    cx,
                                                                ),
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
                                                                        .pl(dp(attachments_indent(
                                                                            original
                                                                                && e.html.is_some(),
                                                                        ))),
                                                                    )
                                                                },
                                                            );
                                                        self.fold_block(
                                                            (self.thread_key(m), k),
                                                            body,
                                                            line,
                                                        )
                                                    })
                                                    .collect();
                                                // The latest, laid out as the earlier ones are, so
                                                // it folds and opens the same way and nothing
                                                // around it moves.
                                                let latest_entry: AnyElement = {
                                                    let line = latest_line().px(dp(across));
                                                    if latest_folded {
                                                        self.folded_message(line, LATEST, cx)
                                                            .into_any_element()
                                                    } else {
                                                        let body = div()
                                                            .flex()
                                                            .flex_col()
                                                            .gap(dp(12.))
                                                            .pt(dp(FOLD_LINE_PAD))
                                                            .pb(dp(14.))
                                                            .px(dp(across))
                                                            .border_t_1()
                                                            .border_color(t.border)
                                                            .child(
                                                                self.message_header(
                                                                    t, m, 0, true, cx,
                                                                ),
                                                            )
                                                            .child(
                                                                self.message_text(
                                                                    t,
                                                                    m,
                                                                    0,
                                                                    original.then_some(&thread),
                                                                    (
                                                                        12.,
                                                                        if trailing {
                                                                            12. - ORIGINAL_SIDE
                                                                        } else {
                                                                            14.
                                                                        },
                                                                    ),
                                                                    &selectable,
                                                                    cx,
                                                                )
                                                                .when(!m.loaded, |d| {
                                                                    d.child(self.loading_line(
                                                                        t,
                                                                        m,
                                                                        cx.entity(),
                                                                    ))
                                                                })
                                                                .when(no_content(m), |d| {
                                                                    d.child(no_content_line(t))
                                                                }),
                                                            )
                                                            .when(
                                                                !m.header.attachments.is_empty(),
                                                                |d| {
                                                                    d.child(
                                                                        self.attachment_row(
                                                                            t,
                                                                            &m.header.attachments,
                                                                            0,
                                                                            m.account.as_deref(),
                                                                            cx,
                                                                        )
                                                                        .pl(dp(attachments_indent(
                                                                            original
                                                                                && m.html.is_some(),
                                                                        ))),
                                                                    )
                                                                },
                                                            )
                                                            .when(m.rich, |d| {
                                                                d.child(
                                                                    div()
                                                                        .flex()
                                                                        .gap(dp(10.))
                                                                        .pl(dp(attachments_indent(
                                                                            original
                                                                                && m.html.is_some(),
                                                                        )))
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
                                                            });
                                                        self.fold_block(
                                                            (self.thread_key(m), LATEST),
                                                            body,
                                                            line,
                                                        )
                                                    }
                                                };
                                                let mut entries =
                                                    Vec::with_capacity(earlier_entries.len() + 1);
                                                if newest_first {
                                                    entries.push(latest_entry);
                                                    entries.extend(earlier_entries);
                                                } else {
                                                    earlier_entries.reverse();
                                                    entries.extend(earlier_entries);
                                                    entries.push(latest_entry);
                                                }
                                                entries
                                            }),
                                    )
                                })
                                .when(m.earlier.is_empty() && !original, |d| d.child(divider()))
                                .when(m.earlier.is_empty(), |d| {
                                    let body = div()
                                        .flex()
                                        .flex_col()
                                        .gap(dp(if original { ORIGINAL_GAP } else { 22. }))
                                        .child(self.message_header(t, m, 0, true, cx))
                                        .child(
                                            self.message_text(
                                                t,
                                                m,
                                                0,
                                                original.then_some(&thread),
                                                // The column's gap over and under it
                                                // (emails show only in the original view).
                                                (
                                                    ORIGINAL_GAP,
                                                    if trailing {
                                                        ORIGINAL_GAP - ORIGINAL_SIDE
                                                    } else {
                                                        0.
                                                    },
                                                ),
                                                &selectable,
                                                cx,
                                            )
                                            .when(!m.loaded, |d| {
                                                d.child(self.loading_line(t, m, cx.entity()))
                                            })
                                            .when(no_content(m), |d| d.child(no_content_line(t))),
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
                                                .pl(dp(attachments_indent(
                                                    original && m.html.is_some(),
                                                ))),
                                            )
                                        })
                                        .when(m.rich, |d| {
                                            d.child(
                                                div()
                                                    .flex()
                                                    .gap(dp(10.))
                                                    .pl(dp(attachments_indent(
                                                        original && m.html.is_some(),
                                                    )))
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
                                        });
                                    // A single message does not fold.
                                    d.child(body)
                                }),
                        ),
                )
            })
            .children(self.menu_view(t, cx))
    }

    /// Shown while an email's content is on its way, or could not be fetched.
    pub(super) fn loading_pane(&self, t: &Theme, mail: &Mail, owner: gpui::Entity<Self>) -> Div {
        let pane = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(dp(0.))
            .items_center()
            .justify_center()
            .gap(dp(12.));
        // On its way, the line under the toolbar says so (`reader_loading`).
        if self.body_failed.contains(&self.thread_key(mail)) {
            return pane.child(self.loading_line(t, mail, owner));
        }
        pane
    }

    /// Whether the open message is on its way: its content not fetched yet, or its email
    /// (the latest, in its own formatting) not drawn yet. The line under the reader's
    /// toolbar shows while it is.
    pub(super) fn reader_loading(&self) -> bool {
        let Some(m) = self.mails.get(self.selected) else {
            return false;
        };
        if self.body_failed.contains(&self.thread_key(m)) {
            return false;
        }
        if !m.loaded || self.draft_wanted.is_some() {
            return true;
        }
        self.original_on()
            && !self.latest_folded(m)
            && self
                .mail_views
                .slot(&crate::mail_views::message_key(
                    &self.original_thread(m),
                    &m.header.message_id,
                    0,
                ))
                .is_some_and(|slot| slot.waiting())
    }

    /// The same in words, with a way to try again when it failed.
    fn loading_line(&self, t: &Theme, mail: &Mail, owner: gpui::Entity<Self>) -> Div {
        let line = div().flex().items_center().gap(dp(12.)).text_color(t.text3);
        let key = self.thread_key(mail);
        // Still on its way: the line under the toolbar says so.
        if !self.body_failed.contains(&key) {
            return line;
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
    pub(super) fn message_header(
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
                self.latest_from(mail),
                mail.tone,
                mail.initials.clone(),
                mail.time.clone(),
            ),
            k => {
                let e = &mail.earlier[k - 1];
                let (tone, initials, when) = mail.earlier_look(k - 1);
                (
                    &e.header,
                    SharedString::from(e.sender.clone()),
                    tone,
                    initials,
                    when,
                )
            }
        };
        let thread = mail.id.clone().unwrap_or_default();
        let open = self.details.as_ref() == Some(&(thread.clone(), index));
        let card_thread = thread.clone();
        let me = self.account.to_string();
        let label = |p: &mail::Person| {
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
        let person = |p: &mail::Person| {
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
                        // Not a click on the header row (which folds the message).
                        cx.stop_propagation();
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
        // An open earlier message in the main window folds again from its header row, as it
        // opened from its line (its buttons and recipients keep their own clicks).
        let foldable = actions && (index > 0 || !mail.earlier.is_empty());
        // The details card needs an address to show.
        let has_details = !header.from.address.is_empty();
        div()
            .flex()
            .flex_col()
            .gap(dp(8.))
            .child(
                div()
                    .id(("header-row", index))
                    .flex()
                    // The avatar, the buttons and the time are in line with the name, as in
                    // a folded message's line, so a message opens without them moving.
                    .items_start()
                    .gap(dp(12.))
                    .when(foldable, |d| {
                        d.cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.fold(index.checked_sub(1).unwrap_or(LATEST), true, cx)
                            }))
                    })
                    .child(if index == 0 {
                        // The latest message is the one DMARC was checked for.
                        self.sender_avatar(mail, 28.)
                    } else {
                        avatar(tone, &initials, 28.).into_any_element()
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(dp(0.))
                            // Name and address shrink (the address first) rather than run
                            // under the buttons beside them.
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .h(dp(HEADER_LINE))
                                    .gap(dp(8.))
                                    .min_w(dp(0.))
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .max_w(relative(0.7))
                                            .truncate()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(sender),
                                    )
                                    .when(!header.from.address.is_empty(), |d| {
                                        d.child(
                                            // Cut to fit with "…": GPUI's own truncation lays
                                            // the text out before the row has its width, and
                                            // crops it. The width is the one it had when last
                                            // drawn (see `address_room`).
                                            {
                                                let rooms = self.address_room.clone();
                                                let key = format!("{thread}/{index}");
                                                let room = rooms.borrow().get(&key).copied();
                                                let shown = match room {
                                                    Some(room) => {
                                                        fit_meta(&header.from.address, room, cx)
                                                    }
                                                    None => header.from.address.clone(),
                                                };
                                                div()
                                                    .relative()
                                                    .flex_1()
                                                    .min_w(dp(0.))
                                                    .overflow_hidden()
                                                    .whitespace_nowrap()
                                                    .child(meta(shown, t.text3))
                                                    .child(
                                                        gpui::canvas(
                                                            move |bounds, window, _| {
                                                                let width =
                                                                    from_screen(bounds.size.width);
                                                                let old = rooms
                                                                    .borrow_mut()
                                                                    .insert(key, width);
                                                                if old.is_none_or(|old| {
                                                                    (old - width).abs() > 0.5
                                                                }) {
                                                                    window.refresh();
                                                                }
                                                            },
                                                            |_, _, _, _| {},
                                                        )
                                                        .absolute()
                                                        .size_full(),
                                                    )
                                            },
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .id(("recipients", index))
                                    .flex()
                                    .items_center()
                                    .gap(dp(4.))
                                    // Shrinks with the column (a narrow pane, a large interface
                                    // size) instead of running under the reply buttons.
                                    .min_w(dp(0.))
                                    .overflow_hidden()
                                    .when(has_details, |d| d.cursor_pointer())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if !has_details {
                                            return;
                                        }
                                        cx.stop_propagation();
                                        let key = (thread.clone(), index);
                                        this.details = if this.details.as_ref() == Some(&key) {
                                            None
                                        } else {
                                            Some(key)
                                        };
                                        cx.notify();
                                    }))
                                    .child(
                                        div()
                                            .min_w(dp(0.))
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .child(meta(short(&summary, 90), t.text3)),
                                    )
                                    .when(has_details, |d| {
                                        d.child(icon(
                                            if open { "up" } else { "down" },
                                            12.,
                                            t.text3,
                                        ))
                                    }),
                            ),
                    )
                    // As close together as the toolbar's icons.
                    .when(actions, |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_none()
                                .h(dp(HEADER_LINE))
                                .items_center()
                                .gap(dp(2.))
                                .child(reply_button(
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
                                )),
                        )
                    })
                    // The latest message of a snoozed thread says when it comes back.
                    .children(
                        (index == 0)
                            .then(|| {
                                super::threads::snooze_of(
                                    self.snoozed.iter().chain(&self.snoozed_elsewhere),
                                    mail.id.as_deref(),
                                )
                            })
                            .flatten()
                            .map(|(name, when)| {
                                div().flex().items_center().h(dp(HEADER_LINE)).child(
                                    super::threads::snooze_chip(
                                        t,
                                        name,
                                        format!("Until {when}").into(),
                                    ),
                                )
                            }),
                    )
                    .child(
                        mono(time, t.text3)
                            .flex_none()
                            .flex()
                            .items_center()
                            .h(dp(HEADER_LINE)),
                    )
                    // Folds as its line opened it: the arrow turns back as it does.
                    .when(foldable, |d| {
                        let key = (
                            self.thread_key(mail),
                            index.checked_sub(1).unwrap_or(LATEST),
                        );
                        d.child(
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .h(dp(HEADER_LINE))
                                .child(fold_arrow(self.fold_turn(&key, true), t.text3)),
                        )
                    }),
            )
            // As far from what is over and under it as from the pane's side: 8 is the
            // header's own gap over it, and 12 the gap under the header to the email's line.
            .when(open && !header.from.address.is_empty(), |d| {
                d.child(
                    self.details_card(t, header, &card_thread, index, &person, cx)
                        .mt(dp(ORIGINAL_SIDE - 8.))
                        .when(self.original_on(), |card| {
                            card.mb(dp(ORIGINAL_SIDE - ORIGINAL_GAP))
                        }),
                )
            })
    }

    /// A details chip was hovered or left: its copy button opens or closes (`copy_button`).
    fn hover_copy(&mut self, key: &str, hovered: bool, cx: &mut Context<Self>) {
        let now = std::time::Instant::now();
        // Turned back part-way: from where it is, so it does not jump.
        let started = match self.copy_hover.get(key) {
            Some((was, since)) if *was != hovered => {
                let gone = since.elapsed().as_secs_f32().min(COPY_TIME);
                now.checked_sub(std::time::Duration::from_secs_f32(COPY_TIME - gone))
                    .unwrap_or(now)
            }
            _ => now,
        };
        self.copy_hover.insert(key.to_string(), (hovered, started));
        cx.notify();
    }

    /// A details chip's copy button, in the chip after what it copies: the copy icon, which
    /// opens as the chip is hovered (and closes as it is left), or "Copied" for a moment
    /// after a copy. The chip grows for it, which pushes the chips after it along its line
    /// (each line has room for it: `chip_lines`). `None` while closed.
    fn copy_button(&self, t: &Theme, key: &str, done: bool, said: &str) -> Option<Div> {
        if done {
            // Its width noted, for breaking the lines while one shows (`COPIED_KEY`).
            let sizes = self.chip_sizes.clone();
            let button = div()
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .gap(dp(4.))
                .pl(dp(COPY_GAP))
                .child(icon("done", 12., t.mint))
                .child(div().text_color(t.mint).child(said.to_string()))
                .child(
                    gpui::canvas(
                        move |bounds, _, _| {
                            sizes
                                .borrow_mut()
                                .insert(COPIED_KEY.into(), from_screen(bounds.size.width));
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                );
            return Some(button);
        }
        let (hovered, since) = self.copy_hover.get(key).copied()?;
        let t_ = (since.elapsed().as_secs_f32() / COPY_TIME).min(1.);
        let open = if hovered {
            ease_in_out(t_)
        } else {
            1. - ease_in_out(t_)
        };
        if open <= 0. {
            return None;
        }
        let moving = t_ < 1.;
        let grow = COPY_ROOM * open;
        // It grows from nothing to its space and the icon's width, the icon kept whole.
        let button = div()
            .flex()
            .flex_none()
            .items_center()
            .overflow_hidden()
            .w(dp(grow))
            .opacity(open)
            .child(
                div()
                    .flex_none()
                    .pl(dp(COPY_GAP))
                    .child(icon("copy", 12., t.text3)),
            )
            .when(moving, |d| {
                d.child(
                    gpui::canvas(
                        |_, window, _| window.request_animation_frame(),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_0(),
                )
            });
        Some(button)
    }

    /// A message's full header (From, To, Cc, Reply-To, Date). Each address copies on
    /// click; "Copy all" copies the lot as text.
    fn details_card(
        &self,
        t: &Theme,
        header: &crate::data::Header,
        thread: &str,
        index: usize,
        person: &dyn Fn(&mail::Person) -> String,
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
        let mut chip = |p: &mail::Person, cx: &mut Context<Self>| {
            serial += 1;
            let key = format!("{base}/{serial}");
            let done = copied(&key);
            let address = p.address.clone();
            let mine = p.address.eq_ignore_ascii_case(&me);
            let hover = t.hover;
            let hover_key = key.clone();
            let copy_button = self.copy_button(t, &key, done, "Copied");
            // Its width without the copy button (what is in it and its padding), which a
            // click or a hover does not change, for breaking the lines.
            let sizes = self.chip_sizes.clone();
            let size_key = key.clone();
            let measure = gpui::canvas(
                move |bounds, window, _| {
                    let width = from_screen(bounds.size.width) + 2. * CHIP_PAD;
                    let old = sizes.borrow_mut().insert(size_key.clone(), width);
                    if old.is_none_or(|old| (old - width).abs() > 0.5) {
                        window.refresh();
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full();
            let chip_key = key.clone();
            // Name, address and "You", then the copy button, which brings its own space.
            // On one line; where the card is too narrow for it, the address ends in "…" (the
            // name and "You" keep their room).
            let who = div()
                .relative()
                .flex()
                .items_center()
                .min_w(dp(0.))
                .gap_x(dp(6.))
                .whitespace_nowrap()
                .when(!p.name.is_empty(), |d| {
                    d.child(
                        div()
                            .flex_none()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.text)
                            .child(p.name.clone()),
                    )
                })
                .child(
                    div()
                        .min_w(dp(0.))
                        .truncate()
                        .text_color(if p.name.is_empty() { t.text } else { t.text3 })
                        .child(p.address.clone()),
                )
                .when(mine, |d| d.child(pill(t, "You", t.violet).flex_none()));
            let chip = div()
                .id(("detail-person", serial))
                .relative()
                .flex()
                .flex_none()
                // No wider than its line: what is in it shortens instead.
                .max_w_full()
                .items_center()
                .px(dp(CHIP_PAD))
                .py(dp(3.))
                .rounded(dp(7.))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .press(t)
                .child(who.child(measure))
                .children(copy_button)
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    this.hover_copy(&hover_key, *hovered, cx)
                }))
                .tooltip(tip(t.dark, "Copy address", ""))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.copy_text(key.clone(), address.clone(), cx)
                }));
            (chip_key, chip)
        };
        // The label as wide as the longest ("Reply-To") needs, and level with the first line
        // of its value: one address is 24pt tall, and several wrap under it.
        let row = |name: &str, value: AnyElement| {
            div()
                .flex()
                .items_start()
                .gap(dp(8.))
                .child(
                    div()
                        .w(dp(58.))
                        .h(dp(24.))
                        .flex()
                        .items_center()
                        .flex_none()
                        .text_size(dp(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.text3)
                        .child(name.to_string()),
                )
                .child(div().flex_1().min_w(dp(0.)).child(value))
        };
        let sizes = self.chip_sizes.clone();
        let people =
            |list: &[mail::Person], field: &str, chip: &mut ChipFn, cx: &mut Context<Self>| {
                let chips: Vec<_> = list.iter().map(|p| chip(p, cx)).collect();
                let room_key = format!("{base}/{field}");
                let lines = {
                    let known = sizes.borrow();
                    known.get(&room_key).copied().and_then(|room| {
                        // A chip showing "Copied" is that much wider, from the frame it shows.
                        let copied_w = known.get(COPIED_KEY).copied().unwrap_or(COPIED_GUESS);
                        let widths = chips
                            .iter()
                            .map(|(key, _)| {
                                let extra = if copied(key) { copied_w } else { 0. };
                                known.get(key).map(|w| w + extra)
                            })
                            .collect::<Option<Vec<_>>>()?;
                        Some(chip_lines(&widths, room))
                    })
                };
                // The room the lines have: the whole width the value has (not what the chips
                // take, which would only ever fit them), and the padding they start into.
                let noted = sizes.clone();
                let measure = gpui::canvas(
                    move |bounds, window, _| {
                        let room = from_screen(bounds.size.width) + CHIP_PAD;
                        let old = noted.borrow_mut().insert(room_key.clone(), room);
                        if old.is_none_or(|old| (old - room).abs() > 0.5) {
                            window.refresh();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full();
                let base = div().ml(dp(-CHIP_PAD));
                let chips_el = match lines {
                    // Broken by hand, each line with room for a copy button: a chip that opens
                    // pushes the ones after it along, and never off its line or out of the card.
                    Some(lines) => {
                        let mut chips = chips.into_iter().map(|(_, chip)| chip);
                        base.flex()
                            .flex_col()
                            .gap_y(dp(CHIP_GAP))
                            .children(lines.into_iter().map(|count| {
                                div()
                                    .flex()
                                    .gap_x(dp(CHIP_GAP))
                                    .children(chips.by_ref().take(count).collect::<Vec<_>>())
                            }))
                            .into_any_element()
                    }
                    // Not laid out yet: as they wrap.
                    None => base
                        .flex()
                        .flex_wrap()
                        .gap(dp(CHIP_GAP))
                        .children(chips.into_iter().map(|(_, chip)| chip))
                        .into_any_element(),
                };
                div()
                    .relative()
                    .w_full()
                    .child(measure)
                    .child(chips_el)
                    .into_any_element()
            };
        // The whole header as text, for "Copy all".
        let mut all = vec![format!("From: {}", person(&header.from))];
        let join = |list: &[mail::Person]| list.iter().map(person).collect::<Vec<_>>().join(", ");
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
        let from = people(std::slice::from_ref(&header.from), "from", &mut chip, cx);
        let to = (!header.to.is_empty()).then(|| people(&header.to, "to", &mut chip, cx));
        let cc = (!header.cc.is_empty()).then(|| people(&header.cc, "cc", &mut chip, cx));
        let reply_to = header
            .reply_to
            .as_ref()
            .map(|p| people(std::slice::from_ref(p), "reply-to", &mut chip, cx));
        // In line with the text under it: the sender's name and the message.
        div()
            .ml(dp(40.))
            .flex()
            .flex_col()
            .gap(dp(6.))
            .pl(dp(14.))
            .pr(dp(10.))
            .pt(dp(10.))
            .pb(dp(8.))
            .rounded(dp(12.))
            .bg(t.raised)
            .border_1()
            .border_color(t.border)
            .shadow(vec![shadow(c(0x000000, 0.06), 1., 3., 0.)])
            .text_size(dp(12.5))
            .line_height(dp(18.))
            .child(row("From", from))
            .children(to.map(|v| row("To", v)))
            .children(cc.map(|v| row("Cc", v)))
            .children(reply_to.map(|v| row("Reply-To", v)))
            .children(date.map(|date| {
                let text = date.clone();
                let hover_key = date_key.clone();
                let copy_button = self.copy_button(t, &date_key, date_done, "Copied");
                row(
                    "Date",
                    // As wide as the date, as an address is.
                    div()
                        .flex()
                        .child(
                            div()
                                .id(("detail-date", index))
                                .flex()
                                .items_center()
                                .whitespace_nowrap()
                                .ml(dp(-7.))
                                .px(dp(7.))
                                .py(dp(3.))
                                .rounded(dp(7.))
                                .cursor_pointer()
                                .hover(move |s| s.bg(hover))
                                .press(t)
                                .child(div().text_color(t.text2).child(date))
                                .children(copy_button)
                                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                    this.hover_copy(&hover_key, *hovered, cx)
                                }))
                                .tooltip(tip(t.dark, "Copy date", ""))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.copy_text(date_key.clone(), text.clone(), cx)
                                })),
                        )
                        .into_any_element(),
                )
            }))
            .child(
                div()
                    .flex()
                    .justify_end()
                    // As much room above Copy all as the card leaves below it.
                    .pt(dp(8.))
                    .mt(dp(2.))
                    .border_t_1()
                    .border_color(t.border)
                    .child(
                        div()
                            .id(("detail-copy-all", index))
                            .flex()
                            .items_center()
                            .gap(dp(6.))
                            .px(dp(8.))
                            .py(dp(4.))
                            .rounded(dp(7.))
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
    pub(super) fn attachment_row(
        &self,
        t: &Theme,
        attachments: &[mail::Attachment],
        base: usize,
        account: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .flex()
            .flex_wrap()
            .gap(dp(10.))
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
                    .gap(dp(10.))
                    .pl(dp(8.))
                    .pr(dp(6.))
                    .py(dp(6.))
                    .max_w(dp(360.))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(dp(30.))
                            .rounded(dp(8.))
                            .bg(gradient(135., c(0x8b7cff, 1.), c(0x5b47e0, 1.)))
                            .child(icon("file", 16., gpui::white())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(dp(0.))
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
    pub(super) fn thread_banners(&self, t: &Theme, mail: &Mail, cx: &mut Context<Self>) -> Div {
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
        let spam = (listed && self.mailbox.base() == View::Spam) || (has("SPAM") && !inbox);
        let trash = (listed && self.mailbox.base() == View::Trash) || (has("TRASH") && !inbox);
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
            .gap(dp(10.))
            .when(!labels.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(dp(6.))
                        .children(labels.into_iter().map(|(index, label)| {
                            let hover = t.hover;
                            let id = label.id.clone();
                            let (key, account) = thread.clone();
                            div()
                                .flex()
                                .items_center()
                                .gap(dp(6.))
                                .pl(dp(8.))
                                .pr(dp(3.))
                                .py(dp(2.))
                                .rounded_full()
                                .bg(t.hover)
                                .border_1()
                                .border_color(t.border)
                                .child(
                                    div()
                                        .size(dp(6.))
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
                                        .size(dp(16.))
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
                        .gap(dp(10.))
                        .px(dp(14.))
                        .py(dp(8.))
                        .rounded(dp(10.))
                        .bg(t.hover)
                        .border_1()
                        .border_color(t.border)
                        .child(icon("trash", 15., t.text3))
                        .child(div().flex_1().child(if self.own_servers() {
                            "In Trash. Your mail server may delete it after a while."
                        } else {
                            "In Trash. Gmail deletes it for good after 30 days."
                        }))
                        .child(
                            button(t, ButtonKind::Secondary, "Restore", None)
                                .id("restore")
                                .press_soft()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let (key, account) = &restore;
                                    if let Some(row) = this.row_to_act_on(key, account, cx) {
                                        this.remove_thread(row, mail::Change::Untrash, cx)
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
                        .gap(dp(10.))
                        .px(dp(14.))
                        .py(dp(8.))
                        .rounded(dp(10.))
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
                                        this.remove_thread(row, mail::Change::NotSpam, cx)
                                    }
                                })),
                        ),
                )
            })
    }

    /// A thread's subject, with Unsubscribe at its top right when it is from a mailing list.
    /// `width` is what the two have together.
    pub(super) fn subject_row(
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
        let title = balance_title(&mail.shown_subject(), size, (width - taken).max(0.), cx);
        let account = mail.account.clone();
        div()
            .flex()
            .items_start()
            .gap(dp(GAP))
            .child(serif(&title, size, t.text).flex_1().min_w(dp(0.)))
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
                        .w(dp(BUTTON_W))
                        .h(dp(BUTTON_H))
                        // Centred on the subject's first line.
                        .mt(dp(((size * 1.14).round() - BUTTON_H) / 2.))
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

    /// A message's body in the conversation: its email in its own formatting (`thread`,
    /// the original view's key for it) when it has HTML and its web view is up, else its
    /// text drawn natively. `index` is 0 for the latest, `k + 1` for `earlier[k]`; `gaps`
    /// are the column's space over and under it (see `email_frame`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn message_text(
        &self,
        t: &Theme,
        m: &Mail,
        index: usize,
        thread: Option<&String>,
        gaps: (f32, f32),
        selectable: &super::selection::Selectable,
        cx: &mut Context<Self>,
    ) -> Div {
        let (blocks, message_id) = match index {
            0 => (&m.body[..], m.header.message_id.as_str()),
            k => {
                let e = &m.earlier[k - 1];
                (&e.blocks[..], e.header.message_id.as_str())
            }
        };
        let slot = thread.and_then(|thread| {
            self.mail_views
                .slot(&crate::mail_views::message_key(thread, message_id, index))
        });
        match slot {
            Some(slot) => email_frame(
                slot,
                ORIGINAL_SIDE,
                gaps,
                self.pane_lead(),
                Some(self.fold_lead.clone()),
                self.darkens_mail(),
                self.folds.borrow().values().any(|fold| !fold.done),
                t,
            ),
            None => self
                .message_body(t, blocks, index * 1000, selectable, self.account_of(m), cx)
                .pl(dp(40.)),
        }
    }

    /// Opens (`folded` false) or folds `earlier[k]` of the open thread. Unless motion is
    /// reduced, the message grows from its line or shrinks back to it (`fold_block`); a
    /// folding message is folded once it is back to its line (`settle_folds`).
    pub(super) fn fold(&mut self, k: usize, folded: bool, cx: &mut Context<Self>) {
        let selected = self.selected;
        let Some(mail) = self.mails.get(selected) else {
            return;
        };
        let key = (self.thread_key(mail), k);
        let animate = !crate::integration::display_options().reduce_motion;
        let line = self.fold_line.get();
        let now = std::time::Instant::now();
        if folded && animate {
            // From where it is: as laid out, or part-way through opening.
            let shown = self
                .folds
                .borrow()
                .get(&key)
                .map(|f| f.shown)
                .unwrap_or_else(|| {
                    self.fold_heights
                        .borrow()
                        .get(&key)
                        .map_or(line, |h| h.get().max(line))
                });
            self.folds.borrow_mut().insert(
                key,
                Fold {
                    from: shown,
                    shown,
                    start: now,
                    closing: true,
                    done: false,
                },
            );
            cx.notify();
            return;
        }
        self.set_open(selected, k, !folded);
        if folded || !animate {
            self.folds.borrow_mut().remove(&key);
        } else {
            let shown = self.folds.borrow().get(&key).map_or(line, |f| f.shown);
            self.folds.borrow_mut().insert(
                key,
                Fold {
                    from: shown,
                    shown,
                    start: now,
                    closing: false,
                    done: false,
                },
            );
        }
        cx.notify();
    }

    /// Opens or folds message `k` of the thread in row `row`: `earlier[k]`, or the latest
    /// (`LATEST`), which is folded by its Message-ID so that a newer message comes in open.
    fn set_open(&mut self, row: usize, k: usize, open: bool) {
        let Some(m) = self.mails.get_mut(row) else {
            return;
        };
        if k != LATEST {
            if let Some(e) = m.earlier.get_mut(k) {
                e.expanded = open;
            }
            return;
        }
        let latest = (
            message_window::key_of(m),
            m.account.clone().unwrap_or_else(|| self.account_id.clone()),
            m.header.message_id.clone(),
        );
        if open {
            self.latest_folded.remove(&latest);
        } else {
            self.latest_folded.insert(latest);
        }
    }

    /// Who the latest message is from, as its header says: the row names the recipient in
    /// Drafts and Sent. "Me" for one of the user's own accounts sent without a name.
    pub(super) fn latest_from(&self, mail: &Mail) -> SharedString {
        let from = &mail.header.from;
        if !from.name.trim().is_empty() {
            return from.name.trim().to_string().into();
        }
        let own = self
            .settings
            .accounts
            .iter()
            .any(|account| account.email.eq_ignore_ascii_case(from.address.trim()));
        if own {
            "Me".into()
        } else if !from.address.trim().is_empty() {
            from.address.trim().to_string().into()
        } else {
            mail.shown_sender(self.mailbox.base() == View::Drafts)
        }
    }

    /// Whether the open thread's latest message is folded: only in a conversation.
    pub(super) fn latest_folded(&self, m: &Mail) -> bool {
        !m.earlier.is_empty()
            && self.latest_folded.contains(&(
                message_window::key_of(m),
                self.account_of(m).to_string(),
                m.header.message_id.clone(),
            ))
    }

    /// Folds the messages whose folding has ended (`fold_block` cannot: it only reads).
    pub(super) fn settle_folds(&mut self) {
        let done: Vec<_> = self
            .folds
            .borrow()
            .iter()
            .filter(|(_, f)| f.done)
            .map(|(key, f)| (key.clone(), f.closing))
            .collect();
        for (key, closing) in done {
            self.folds.borrow_mut().remove(&key);
            if !closing {
                continue;
            }
            let (thread, k) = &key;
            if let Some(at) = self
                .mails
                .iter()
                .position(|m| self.thread_key(m) == *thread)
            {
                self.set_open(at, *k, false);
            }
        }
    }

    /// A folded earlier message: its line, which opens it on a click, and whose height is
    /// noted for opening from and folding to.
    fn folded_message(&self, line: Div, k: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let height = self.fold_line.clone();
        line.relative()
            .id(("earlier", k))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.fold(k, false, cx)))
            .child(
                gpui::canvas(
                    move |bounds, _, _| height.set(f32::from(bounds.size.height)),
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
    }

    /// An open earlier message (`body`), noting its height; while it opens or folds, only
    /// as much of it shows as it has grown to, with its `line` fading out over it, and what
    /// is under it moves along. The height follows where it is going smoothly, also when
    /// that changes on the way (its email's height arriving).
    fn fold_block(&self, key: (ThreadKey, usize), body: Div, line: Div) -> AnyElement {
        let height = self
            .fold_heights
            .borrow_mut()
            .entry(key.clone())
            .or_insert_with(|| std::rc::Rc::new(std::cell::Cell::new(0.)))
            .clone();
        let noted = height.clone();
        let body = div().relative().flex_none().child(body).child(
            gpui::canvas(
                move |bounds, _, _| noted.set(f32::from(bounds.size.height)),
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );
        let mut folds = self.folds.borrow_mut();
        let Some(fold) = folds.get_mut(&key).filter(|f| !f.done) else {
            return body.into_any_element();
        };
        let line_h = self.fold_line.get();
        let open = height.get().max(line_h);
        let target = if fold.closing { line_h } else { open };
        // Where the curve has it now, by the clock (not by frames, which can come late).
        let at = |t: f32| fold.from + (target - fold.from) * ease_in_out(t.clamp(0., 1.));
        let t = fold.start.elapsed().as_secs_f32() / FOLD_TIME;
        let shown = at(t);
        fold.shown = shown;
        fold.done = t >= 1.;
        // How much it grows (or shrinks) by the next frame, for what is under it.
        let next = if fold.done {
            0.
        } else {
            at(t + self.frame_interval.get() / FOLD_TIME) - shown
        };
        let lead = self.fold_lead.clone();
        let progress = if open <= line_h + 0.5 {
            0.
        } else {
            ((shown - line_h) / (open - line_h)).clamp(0., 1.)
        };
        // The pointer stays the hand it was over the line, all the way.
        div()
            .relative()
            .flex()
            .flex_col()
            .h(px(shown))
            .overflow_hidden()
            .cursor_pointer()
            .child(body.opacity(progress))
            .child(
                line.absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .cursor_pointer()
                    .opacity(1. - progress),
            )
            .child(
                gpui::canvas(
                    move |bounds, window, _| {
                        window.request_animation_frame();
                        lead.set(Some((f32::from(bounds.bottom()), next)));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .into_any_element()
    }
}

/// The reader's column padding on either side (the conversation's `px`).
pub(super) const READER_SIDE: f32 = 64.;

/// The original view's: in line with the toolbar's first icon, whose glyph starts this far
/// in, and with the text of an email's page, whose padding is as wide.
pub(super) const ORIGINAL_SIDE: f32 = 20.;

/// Where a message opening or folding ends (window px), and how much it will have grown by
/// the next frame: what under it moves by then. Set while it is drawn, cleared each frame.
pub(super) type FoldLead = std::rc::Rc<std::cell::Cell<Option<(f32, f32)>>>;

/// Open earlier messages' heights as last laid out, by thread and place.
pub(super) type FoldHeights =
    std::cell::RefCell<HashMap<(ThreadKey, usize), std::rc::Rc<std::cell::Cell<f32>>>>;

/// An earlier message opening or folding: how tall it shows (px), as `fold_block` moves it.
pub(super) struct Fold {
    /// The height it started from, and since when (`FOLD_TIME`).
    from: f32,
    start: std::time::Instant,
    /// How tall it showed when last drawn: where it starts from if turned back.
    shown: f32,
    closing: bool,
    /// There: `settle_folds` takes it away, and folds the message if it was folding.
    done: bool,
}

/// The space before a details chip's copy button.
const COPY_GAP: f32 = 6.;

/// A details chip's padding at its sides.
const CHIP_PAD: f32 = 7.;

/// Makes a details chip for a person: its key and the chip.
type ChipFn<'a> = dyn FnMut(&mail::Person, &mut Context<Nimbra>) -> (String, Stateful<Div>) + 'a;

/// Where the width of a details chip's "Copied" is noted among `chip_sizes`, and what it
/// is taken to be before it was ever shown (design points).
const COPIED_KEY: &str = "copied";
const COPIED_GUESS: f32 = 66.;

/// The space between details chips.
const CHIP_GAP: f32 = 2.;

/// How many details chips go on each line, given their widths (without a copy button) and
/// the line's room: each line keeps room for one copy button to open (`COPY_ROOM`), and
/// has at least one chip.
fn chip_lines(widths: &[f32], room: f32) -> Vec<usize> {
    let mut lines = Vec::new();
    let (mut count, mut used) = (0, 0.);
    for &width in widths {
        let next = if count == 0 {
            width
        } else {
            used + CHIP_GAP + width
        };
        if count > 0 && next + COPY_ROOM > room {
            lines.push(count);
            (count, used) = (1, width);
        } else {
            (count, used) = (count + 1, next);
        }
    }
    if count > 0 {
        lines.push(count);
    }
    lines
}

/// How much a details chip grows for its copy button: its space and the icon.
const COPY_ROOM: f32 = COPY_GAP + 12.;

/// How long a details chip takes to make room for its copy button (seconds).
const COPY_TIME: f32 = 0.15;

/// How long a message takes to open or fold (seconds).
const FOLD_TIME: f32 = 0.28;

/// A foldable message's arrow, after its time: down when folded, turned up (by `turn`, 0
/// to 1) as it opens.
fn fold_arrow(turn: f32, color: Hsla) -> gpui::Svg {
    icon("down", 14., color).with_transformation(Transformation::rotate(gpui::radians(
        turn * std::f32::consts::PI,
    )))
}

/// The latest message, among a thread's folds (the earlier ones are by their place).
pub(super) const LATEST: usize = usize::MAX;

/// Slow out, slow in (cubic): no jump at the start, none at the end.
fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 {
        4. * t * t * t
    } else {
        1. - (-2. * t + 2.).powi(3) / 2.
    }
}

/// A message header's first line (avatar, name, buttons, time), as tall as the avatar: as
/// a folded message's line is.
const HEADER_LINE: f32 = 28.;

/// The space over and under a folded message's line.
const FOLD_LINE_PAD: f32 = 12.;

/// The original view's gap between the parts of a conversation.
const ORIGINAL_GAP: f32 = 12.;

/// Where a message's attachments start: under its email, in line with the email's text
/// (the column's edge, where the email's page padding ends), else in line with the
/// reader's text, past the sender's avatar. Under an email they are as far below it as its
/// text is from its sides: `email_frame` is given `gap - ORIGINAL_SIDE` under it for that.
pub(super) fn attachments_indent(under_email: bool) -> f32 {
    if under_email { 0. } else { 40. }
}

/// The blank edge inside a 16pt icon (Lucide draws in 2 of its 24 units from each side).
const GLYPH_INSET: f32 = 16. * 2. / 24.;

/// A divider in a toolbar, with as much room on each side as Compose has from the list's
/// line (LIST_RIGHT), counted to the glyphs: `before` and `after` are the inner padding of
/// the buttons beside it, and the toolbar's 2pt gap is on both sides too.
/// Where a message's email shows in the conversation: room as tall as the email, which
/// places its web view each frame inside what of the pane is on screen. The email takes
/// the pane's whole width, as in its own formatting it was made to, past the column's
/// `side` padding (its page has padding of its own). `lead` moves it a frame ahead while a
/// panel slides (`pane_lead`); under it is the page's own background, while it loads.
///
/// `gaps` are the space the column leaves over and under it: the email takes it, so the
/// room around its text is its page's padding, as at its sides, and over it a line across
/// the pane, as far under the header as the header is under the line over it.
///
/// `below` is where a message over it is opening or folding (`FoldLead`): an email under
/// it moves with it a frame ahead, as for `lead`, so it does not shake.
#[allow(clippy::too_many_arguments)]
pub(super) fn email_frame(
    slot: std::rc::Rc<crate::mail_views::Slot>,
    side: f32,
    gaps: (f32, f32),
    lead: f32,
    below: Option<FoldLead>,
    darkened: bool,
    moving: bool,
    t: &Theme,
) -> Div {
    // While it opens, the web view keeps its full height and only that much shows.
    let (height, opening) = slot.shown_height();
    let full = slot.height();
    // Its picture, under the web view, hidden by it. While the conversation moves (a
    // message opening or folding), the web view steps aside and the picture moves with
    // the app's own drawing, in step: the web view reaches the screen sooner than the
    // app's frames, so it would run ahead of them.
    let picture = slot.snapshot();
    let stand_in = moving && picture.is_some();
    div()
        .ml(dp(-side))
        .mr(dp(-side))
        // Under its header, as much room as over it (a folded line's), then a line across
        // the pane; the email's page puts its own padding between the line and its text.
        .mt(dp(FOLD_LINE_PAD - gaps.0))
        .mb(dp(-gaps.1))
        .border_t_1()
        .border_color(t.border)
        .child(
            div()
                .relative()
                .w_full()
                .h(px(height))
                .overflow_hidden()
                .bg(if darkened || height <= 0. {
                    t.surface
                } else {
                    gpui::white()
                })
                .children(picture.map(|picture| {
                    gpui::img(picture)
                        .absolute()
                        .top_0()
                        .left_0()
                        .w_full()
                        .h(px(full))
                }))
                .child(
                    gpui::canvas(
                        move |bounds, window, _| {
                            if opening {
                                window.request_animation_frame();
                            }
                            if stand_in {
                                // Out of sight while its picture stands in for it.
                                let none = (
                                    f32::from(bounds.origin.x),
                                    f32::from(bounds.origin.y),
                                    0.,
                                    0.,
                                );
                                slot.place(
                                    (
                                        f32::from(bounds.origin.x) + lead,
                                        f32::from(bounds.origin.y),
                                        f32::from(bounds.size.width) - lead,
                                        full,
                                    ),
                                    none,
                                );
                                return;
                            }
                            // Under a message opening or folding: where the next frame has it.
                            let ahead = below
                                .as_ref()
                                .and_then(|b| b.get())
                                .filter(|(bottom, _)| f32::from(bounds.origin.y) >= bottom - 1.)
                                .map_or(0., |(_, dy)| dy);
                            let bounds = gpui::Bounds {
                                origin: gpui::point(bounds.origin.x, bounds.origin.y + px(ahead)),
                                size: bounds.size,
                            };
                            // What of the pane is on screen, and of the email has opened.
                            let clip = window.content_mask().bounds.intersect(&bounds);
                            slot.place(
                                (
                                    f32::from(bounds.origin.x) + lead,
                                    f32::from(bounds.origin.y),
                                    f32::from(bounds.size.width) - lead,
                                    full,
                                ),
                                (
                                    f32::from(clip.origin.x) + lead,
                                    f32::from(clip.origin.y),
                                    f32::from(clip.size.width) - lead,
                                    f32::from(clip.size.height),
                                ),
                            );
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                ),
        )
}

pub(super) fn toolbar_rule(t: &Theme, before: f32, after: f32) -> Div {
    div()
        .flex_none()
        .w(dp(1.))
        .h(dp(16.))
        .ml(dp(LIST_RIGHT - before - 2. - GLYPH_INSET))
        .mr(dp(LIST_RIGHT - after - 2. - GLYPH_INSET))
        .bg(t.border_strong)
}

/// The text, cut with "…" so that it fits `room` (12 pt, `meta`'s size).
fn fit_meta(text: &str, room: f32, cx: &App) -> String {
    let width = |s: &str| text_width(s, 12., crate::theme::SANS, cx);
    if width(text) <= room {
        return text.to_string();
    }
    let mut cut: Vec<char> = text.chars().collect();
    while !cut.is_empty() {
        cut.pop();
        let shown = format!("{}…", cut.iter().collect::<String>().trim_end());
        if width(&shown) <= room {
            return shown;
        }
    }
    "…".to_string()
}

/// A toolbar action as an icon and a word (Done, Snooze, Reply, Forward); just the icon
/// where the pane is narrow. Also drawn, dimmed, on the loading screen.
pub(super) fn action_button(t: &Theme, name: &str, label: &str, worded: bool) -> Div {
    let hover = t.hover;
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(dp(6.))
        .h(dp(30.))
        .px(dp(if worded { 9. } else { 7. }))
        .rounded(dp(8.))
        .cursor_pointer()
        .text_color(t.text2)
        .hover(move |s| s.bg(hover))
        .child(icon(name, 16., t.text2))
        .when(worded, |d| {
            d.child(
                div()
                    .text_size(dp(13.5))
                    .font_weight(FontWeight::MEDIUM)
                    .child(label.to_string()),
            )
        })
}

/// A message fetched with nothing in it (an empty draft): said so, rather than a blank.
fn no_content(m: &Mail) -> bool {
    m.loaded
        && m.body.is_empty()
        && m.html
            .as_deref()
            .is_none_or(|html| nimbra_core::rich::from_html(html).is_empty())
}

fn no_content_line(t: &Theme) -> Div {
    div().text_color(t.text3).child("No content")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chip_lines_keep_room_for_a_copy_button() {
        // 100 + 2 + 100 + the button's 18 fits 220, not 219.
        assert_eq!(chip_lines(&[100., 100.], 220.), [2]);
        assert_eq!(chip_lines(&[100., 100.], 219.), [1, 1]);
        assert_eq!(chip_lines(&[50., 50., 50., 50.], 160.), [2, 2]);
        // One wider than the room still gets a line of its own.
        assert_eq!(chip_lines(&[500., 10.], 100.), [1, 1]);
        assert!(chip_lines(&[], 100.).is_empty());
    }

    #[test]
    fn opening_starts_and_ends_gently() {
        assert_eq!(ease_in_out(0.), 0.);
        assert_eq!(ease_in_out(1.), 1.);
        assert!((ease_in_out(0.5) - 0.5).abs() < 1e-6);
        // Barely moving at either end, and always forward.
        assert!(ease_in_out(0.05) < 0.001 && ease_in_out(0.95) > 0.999);
        let steps: Vec<f32> = (0..=20).map(|i| ease_in_out(i as f32 / 20.)).collect();
        assert!(steps.windows(2).all(|w| w[1] >= w[0]));
    }
    use std::path::{Path, PathBuf};

    /// An empty folder of the test's own, removed when it ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let mut id = [0u8; 8];
            getrandom::getrandom(&mut id).unwrap();
            let dir = std::env::temp_dir().join(format!("nimbra-test-{}", u64::from_le_bytes(id)));
            std::fs::create_dir(&dir).unwrap();
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::symlink_metadata(path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    }

    #[test]
    fn save_new_writes_the_bytes_under_the_name() {
        let scratch = Scratch::new();
        let path = save_new(&scratch.0, "Plan.pdf", b"pdf").unwrap();
        assert_eq!(
            (path.file_name().unwrap(), std::fs::read(&path).unwrap()),
            ("Plan.pdf".as_ref(), b"pdf".to_vec())
        );
    }

    #[test]
    fn save_new_numbers_the_name_when_it_is_taken() {
        let scratch = Scratch::new();
        std::fs::write(scratch.0.join("Plan.pdf"), b"old").unwrap();
        let path = save_new(&scratch.0, "Plan.pdf", b"new").unwrap();
        assert_eq!(path, scratch.0.join("Plan 2.pdf"));
    }

    #[test]
    fn save_new_never_writes_through_a_dangling_link() {
        let scratch = Scratch::new();
        let target = scratch.0.join("elsewhere.plist");
        std::os::unix::fs::symlink(&target, scratch.0.join("report.pdf")).unwrap();
        save_new(&scratch.0, "report.pdf", b"payload").unwrap();
        assert!(!target.exists());
    }

    #[test]
    fn save_new_skips_a_dangling_link_for_the_next_name() {
        let scratch = Scratch::new();
        std::os::unix::fs::symlink(scratch.0.join("gone"), scratch.0.join("report.pdf")).unwrap();
        let path = save_new(&scratch.0, "report.pdf", b"payload").unwrap();
        assert_eq!(path, scratch.0.join("report 2.pdf"));
    }

    #[test]
    fn save_new_makes_the_file_private() {
        let scratch = Scratch::new();
        let path = save_new(&scratch.0, "a.txt", b"x").unwrap();
        assert_eq!(mode(&path), 0o600);
    }

    #[test]
    fn preview_folders_are_private() {
        let scratch = Scratch::new();
        let root = scratch.0.join("previews");
        let dir = private_folder_in(&root).unwrap();
        assert_eq!((mode(&root), mode(&dir)), (0o700, 0o700));
    }

    #[test]
    fn an_existing_preview_root_is_made_private() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new();
        let root = scratch.0.join("previews");
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        private_folder_in(&root).unwrap();
        assert_eq!(mode(&root), 0o700);
    }

    #[test]
    fn signing_out_forgets_that_accounts_previews_only() {
        let scratch = Scratch::new();
        let root = scratch.0.join("previews");
        let gone = previews_in(&root, "").unwrap();
        let kept = previews_in(&root, "a1").unwrap();
        std::fs::write(gone.join("mail.pdf"), b"secret").unwrap();
        forget_previews_in(&root, "");
        assert!(!gone.exists());
        assert!(kept.exists());
    }

    #[test]
    fn previews_are_kept_per_account_and_private() {
        let scratch = Scratch::new();
        let root = scratch.0.join("previews");
        let dir = previews_in(&root, "a1").unwrap();
        let folder = dir.parent().unwrap();
        assert_eq!(folder, root.join("account-a1"));
        assert_eq!(
            (mode(&root), mode(folder), mode(&dir)),
            (0o700, 0o700, 0o700)
        );
        assert!(previews_in(&root, "../x").is_err());
    }

    #[test]
    fn a_launch_forgets_the_previews_there_were_and_not_one_made_since() {
        let scratch = Scratch::new();
        let root = scratch.0.join("previews");
        let older = previews_in(&root, "").unwrap();
        let earlier_version = private_folder_in(&root).unwrap();
        let there = previews_now_in(&root);
        let since = previews_in(&root, "").unwrap();
        remove_previews(there);
        assert!(!older.exists() && !earlier_version.exists());
        assert!(since.exists());
    }
}
