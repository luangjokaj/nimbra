//! The reading pane: a thread and its messages, their headers and attachments, the original
//! HTML view beside the native one, printing and unsubscribing.

use super::*;
use crate::theme::{GRADIENT_GLOW, radius, space};
use crate::ui::{Elevation, elevation};

/// "Fri, Nov 21, 2026 at 09:41", in local time; empty when unknown.
pub(super) fn long_date(timestamp_ms: i64) -> String {
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

/// Saves `bytes` as a new file in Downloads named after `name` ("Plan.pdf", then
/// "Plan 2.pdf", ...). Nothing already there is replaced or followed.
fn save_to_downloads(name: &str, bytes: &[u8]) -> anyhow::Result<std::path::PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("no home folder"))?;
    save_new_in(
        &std::path::PathBuf::from(home).join("Downloads"),
        name,
        bytes,
    )
}

/// Saves `bytes` in `dir` under the first free name of "name", "name 2", ...
fn save_new_in(
    dir: &std::path::Path,
    name: &str,
    bytes: &[u8],
) -> anyhow::Result<std::path::PathBuf> {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    for n in 1..1000 {
        let path = dir.join(if n == 1 {
            format!("{stem}{ext}")
        } else {
            format!("{stem} {n}{ext}")
        });
        match write_new(&path, bytes) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    anyhow::bail!("too many files with that name")
}

/// Saves `bytes` for Quick Look, in a private temporary folder of its own.
fn save_for_preview(name: &str, bytes: &[u8]) -> anyhow::Result<std::path::PathBuf> {
    let mut id = [0u8; 8];
    getrandom::fill(&mut id).map_err(|e| anyhow::anyhow!("{e}"))?;
    let parent = std::env::temp_dir().join("nimbra-previews");
    std::fs::create_dir_all(&parent)?;
    let dir = parent.join(id.iter().map(|b| format!("{b:02x}")).collect::<String>());
    let mut folder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut folder, 0o700);
    folder.create(&dir)?;
    let path = dir.join(name);
    write_new(&path, bytes)?;
    Ok(path)
}

/// Creates `path` (only if nothing is there, not even a link), readable by the user alone
/// and quarantined like any download, then writes `bytes` to it.
fn write_new(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;
    let written = quarantine(&file).and_then(|()| file.write_all(bytes));
    if written.is_err() {
        drop(file);
        let _ = std::fs::remove_file(path);
    }
    written
}

/// Marks a file as downloaded from the internet, so that macOS asks before it opens or
/// runs anything in it (a `.terminal`, `.fileloc` or app would otherwise run unasked). A
/// file that cannot carry the mark is not kept.
#[cfg(target_os = "macos")]
fn quarantine(file: &std::fs::File) -> std::io::Result<()> {
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
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // Flags 0081: downloaded, and not yet approved by the user.
    let value = format!("0081;{since:x};Nimbra;");
    // SAFETY: a live descriptor, a NUL-terminated name and a buffer of `value.len()` bytes.
    let set = unsafe {
        fsetxattr(
            file.as_raw_fd(),
            c"com.apple.quarantine".as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
            0,
        )
    };
    if set == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn quarantine(_file: &std::fs::File) -> std::io::Result<()> {
    Ok(())
}

/// Whether `shown` (the web view's "account:thread") is this thread, without building it.
fn shows_thread(shown: &str, account: &str, thread: &str) -> bool {
    shown
        .strip_prefix(account)
        .and_then(|rest| rest.strip_prefix(':'))
        == Some(thread)
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
        .size(px(30.))
        .rounded(px(radius::CONTROL))
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
pub(super) fn esc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// "1.2 MB", "340 KB".
pub(super) fn file_size(bytes: u64) -> String {
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
    fn original_target(&self) -> Option<(&Mail, ThreadKey)> {
        let ready = matches!(self.status, Status::Demo | Status::Ready);
        // (A toolbar menu does not hide it: menus open in a window of their own, above it.)
        let covered = self.finder_open
            || self.picked.len() >= 2
            || self.compose.is_some()
            || self.new_label.is_some()
            || self.account_sheet_over_main()
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
        mail.html.as_ref()?;
        // Not the row: new mail would load the email again.
        Some((mail, self.thread_key(mail)))
    }

    /// Whether this thread is in Spam (Gmail's label, or an IMAP Junk folder).
    pub(super) fn in_spam(&self, mail: &Mail) -> bool {
        mail.labels.iter().any(|l| l == "SPAM")
            || (self.mailbox == View::Spam
                && self
                    .row_of(&message_window::key_of(mail), self.account_of(mail))
                    .is_some())
    }

    /// Whether the email's remote images load: as Settings › Reading says, and never for
    /// mail in Spam, which is where tracking pixels confirm an address is read.
    pub(super) fn remote_images(&self, mail: &Mail) -> bool {
        self.settings.load_images && !self.in_spam(mail)
    }

    /// Whether light emails are given a dark page right now (Settings › Reading).
    pub(super) fn darkens_mail(&self) -> bool {
        cfg!(target_os = "macos") && self.dark && self.settings.dark_emails
    }

    /// Creates, loads, shows or hides the web view to match the current state. Runs every
    /// frame, so it only talks to the view when something changed.
    pub(super) fn sync_html_view(&mut self, window: &mut Window) {
        // A print job has a view of its own, so the email shown does not replace it, and it
        // is light whatever the theme.
        if let Some(doc) = self.print_pending.take() {
            if self.print_view.is_none() {
                self.print_view = HtmlView::new(window);
            }
            let images = self
                .mails
                .get(self.selected)
                .is_some_and(|mail| self.remote_images(mail));
            if let Some(view) = &self.print_view {
                view.set_dark(false, false);
                // Never shown, but it needs a page's size to lay the document out.
                view.set_bounds(0., 0., 800., 1000.);
                view.print_when_loaded();
                view.show(&doc, images);
            }
        }
        let Some((mail, (thread, account))) = self.original_target() else {
            if let Some(view) = &self.html_view {
                view.set_visible(false);
            }
            return;
        };
        let (spam, html) = (self.in_spam(mail), mail.html.clone());
        let same_thread = self
            .html_shown
            .as_ref()
            .is_some_and(|(key, ..)| shows_thread(key, &account, &thread));
        // A different thread starts again from what Settings says.
        if !same_thread {
            self.original_images = self.settings.load_images;
        }
        let images = self.original_images && !spam;
        let darkened = self.darkens_mail();
        let unchanged = same_thread
            && self
                .html_shown
                .as_ref()
                .is_some_and(|(_, i, dark, d)| (*i, *dark, *d) == (images, self.dark, darkened));
        let html = html.filter(|_| !unchanged);
        if self.html_view.is_none() {
            self.html_view = HtmlView::new(window).map(std::rc::Rc::new);
        }
        let Some(view) = &self.html_view else {
            // No web view on this platform: fall back to the native reader.
            self.original = false;
            self.notice = Some("The original view isn’t available here.".into());
            return;
        };
        if let Some(html) = html {
            // The page follows Nimbra's theme. Both take effect as the mail loads, so a
            // change of either loads it again.
            view.set_dark(self.dark, darkened);
            view.show(&html, images);
            self.html_shown = Some((format!("{account}:{thread}"), images, self.dark, darkened));
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
                .map(|place| {
                    (
                        place.x,
                        place.inset,
                        self.side_w(),
                        height - 2. * place.inset,
                    )
                }),
        );
    }

    /// ⌘P: prints the open message with its sender, recipients and date.
    pub(super) fn print_selected(&mut self, cx: &mut Context<Self>) {
        let Some(mail) = self
            .mails
            .get(self.selected)
            .filter(|_| self.visible().contains(&self.selected))
        else {
            return;
        };
        if !mail.loaded {
            self.notice = Some("Still loading. Try again in a moment.".into());
            cx.notify();
            return;
        }
        let h = &mail.header;
        let person = |p: &mail::Person| {
            if p.name.is_empty() {
                p.address.clone()
            } else {
                format!("{} <{}>", p.name, p.address)
            }
        };
        let list =
            |people: &[mail::Person]| people.iter().map(person).collect::<Vec<_>>().join(", ");
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
        // Loaded on the next frame, and printed once the page has finished loading.
        self.print_pending = Some(doc);
        cx.notify();
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
        let shown_name = short(&att.name, 48);
        self.attachment_busy.insert(key.clone(), save);
        cx.notify();
        cx.spawn(async move |this, cx| {
            // No deadline: the spinner shows it is still coming, and one given up on would
            // still land in Downloads after saying it failed.
            let result = cx
                .background_executor()
                .spawn(async move {
                    let bytes = client.download(&att.message_id, &att.attachment_id)?;
                    let name = mail::safe_file_name(&att.name);
                    if save {
                        save_to_downloads(&name, &bytes)
                    } else {
                        save_for_preview(&name, &bytes)
                    }
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
                    Err(_) => {
                        this.notice =
                            Some(format!("Couldn’t download {shown_name}. Try again.").into())
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
        if self.compose.is_some() {
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

    /// Fetches the remote images of a message after the user asked for them. Never for mail
    /// in Spam.
    fn load_images(&mut self, sources: Vec<String>, cx: &mut Context<Self>) {
        let sources: Vec<String> = sources.into_iter().take(16).collect();
        self.forget_images(&sources, cx);
        for url in sources {
            if !self.image_requests.insert(url.clone()) {
                continue;
            }
            cx.spawn(async move |this, cx| {
                let fetch_url = url.clone();
                // Decoding and hashing the bytes is work for the background too.
                let image = cx
                    .background_executor()
                    .spawn(async move {
                        let (kind, bytes) = rich::fetch_image(&fetch_url)?;
                        let format = match kind {
                            rich::ImageKind::Png => gpui::ImageFormat::Png,
                            rich::ImageKind::Jpeg => gpui::ImageFormat::Jpeg,
                            rich::ImageKind::Gif => gpui::ImageFormat::Gif,
                            rich::ImageKind::Webp => gpui::ImageFormat::Webp,
                        };
                        anyhow::Ok(Arc::new(gpui::Image::from_bytes(format, bytes)))
                    })
                    .await;
                let Ok(image) = image else {
                    return;
                };
                this.update(cx, |this, cx| {
                    // Unless it was let go of while on its way.
                    if this.image_requests.contains(&url) {
                        this.images.insert(url, image);
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    /// Lets go of the images (and their decoded textures) of threads no longer open, once
    /// there are more than a few, keeping the open thread's and `wanted`.
    fn forget_images(&mut self, wanted: &[String], cx: &mut Context<Self>) {
        const KEPT: usize = 32;
        if self.image_requests.len() + wanted.len() <= KEPT {
            return;
        }
        let open: Vec<&String> = self.mails.get(self.selected).map_or_else(Vec::new, |mail| {
            mail.earlier
                .iter()
                .flat_map(|e| &e.blocks)
                .chain(&mail.body)
                .filter_map(|block| match &block.kind {
                    Kind::Image { src, .. } => Some(src),
                    _ => None,
                })
                .collect()
        });
        let keep = |url: &String| wanted.contains(url) || open.contains(&url);
        self.image_requests.retain(|url| keep(url));
        let gone: Vec<String> = self
            .images
            .keys()
            .filter(|url| !keep(url))
            .cloned()
            .collect();
        for url in gone {
            if let Some(image) = self.images.remove(&url) {
                image.remove_asset(cx);
            }
        }
    }

    /// Space and Shift-Space: the conversation a page further or back.
    pub(super) fn page_message(&mut self, down: bool, cx: &mut Context<Self>) {
        if self.pane == Pane::Sidebar || self.visible().is_empty() {
            return;
        }
        if self.original_target().is_some()
            && let Some(view) = &self.html_view
        {
            view.scroll_page(down);
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

    /// One message drawn natively from its blocks, its text selectable where it is drawn
    /// (`selectable`). `base` keeps element ids unique when several messages are on screen.
    /// `images` is whether its remote images may be loaded at all (never in Spam).
    pub(super) fn message_body(
        &self,
        t: &Theme,
        blocks: &[Block],
        base: usize,
        images: bool,
        selectable: &super::selection::Selectable,
        cx: &mut Context<Self>,
    ) -> Div {
        let sources = rich::image_sources(blocks);
        let pending = images && sources.iter().any(|src| !self.image_requests.contains(src));
        let (size, line) = self.settings.reader_text.metrics();
        div()
            .flex()
            .flex_col()
            .gap(px((line * 0.6).round()))
            // A comfortable measure: about 70 characters to a line.
            .max_w(px(size * 44.))
            .text_size(px(size))
            .line_height(px(line))
            .text_color(t.text)
            .when(pending, |d| {
                d.child(
                    div()
                        .id(("images-hidden", base))
                        .flex()
                        .items_center()
                        .gap(px(space::S))
                        .h(px(40.))
                        .pl(px(space::M))
                        .pr(px(space::XS))
                        .rounded(px(radius::CARD))
                        .bg(t.hover)
                        .border_1()
                        .border_color(t.border)
                        .text_size(px(13.))
                        .line_height(px(18.))
                        .tooltip(tip(t.dark, "Senders can tell when images load", ""))
                        .child(icon("image", 15., t.text3))
                        .child(div().flex_1().text_color(t.text2).child("Images hidden"))
                        .child(
                            button(t, ButtonKind::Ghost, "Load Images")
                                .id(("load-images", base))
                                .press(t)
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
                    .map(|(i, block)| self.block(t, block, base + i, selectable, cx)),
            )
    }

    fn block(
        &self,
        t: &Theme,
        block: &Block,
        id: usize,
        selectable: &super::selection::Selectable,
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
                gpui::InteractiveText::new(("link-text", id), styled)
                    .on_click(ranges, move |index, _, cx| cx.open_url(&urls[index]))
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
                .rounded(px(radius::ROW))
                .bg(t.hover)
                .border_1()
                .border_color(t.border)
                .font_family(crate::theme::MONO)
                .text_size(px(12.5))
                .line_height(px(20.))
                .child(text(t.text)),
            Kind::Rule => div().h(px(1.)).my(px(6.)).bg(t.border),
            Kind::Image { src, alt } => match self.images.get(src) {
                Some(image) => div().child(
                    gpui::img(image.clone())
                        .max_w_full()
                        .rounded(px(radius::ROW)),
                ),
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
                .flex()
                .gap(px(12.))
                .ml(px(14. * (block.quote.min(4) - 1) as f32))
                .text_color(t.text2)
                .child(
                    div()
                        .flex_none()
                        .w(px(2.))
                        .rounded_full()
                        .bg(t.violet.opacity(0.35)),
                )
                .child(div().flex_1().min_w(px(0.)).child(element))
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
                        .rounded(px(radius::CHIP))
                        .border_1()
                        .border_color(t.border_strong),
                )
                .child(div().flex_1().child(text.to_string()))
                .child(caption(due.to_string(), t.amber_text))
        };
        let glow = |hex: u32, a: f32| c(hex, a);
        // GPUI has no gradient strokes, so the edge is a 1pt gradient frame around the card.
        div()
            .p(px(1.))
            .rounded(px(radius::CARD))
            .bg(gradient(
                100.,
                glow(GRADIENT_GLOW.from, 0.7),
                glow(GRADIENT_GLOW.to, 0.25),
            ))
            .shadow(elevation(t, Elevation::Glow))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::M))
                    .px(px(20.))
                    .py(px(space::L))
                    .rounded(px(radius::CARD - 1.))
                    .bg(t.raised)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::S))
                            .child(icon("sparkles", 15., t.violet))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.text)
                                    .child("Brief"),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(15.))
                            .line_height(px(24.))
                            .text_color(t.text2)
                            .child(m.brief.clone()),
                    )
                    .when(m.rich, |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(space::S))
                                .child(todo("Confirm pricing tiers on slide 14", "Fri"))
                                .child(todo("Sign off on the press embargo date", "Fri")),
                        )
                    }),
            )
    }

    /// The reading pane's bar: what to do with the thread, in groups, and where to go next.
    /// Keys are in the tooltips, only while they would do something.
    fn reading_toolbar(
        &self,
        t: &Theme,
        m: &Mail,
        original: bool,
        is_draft: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let visible = self.visible();
        let pos = visible.iter().position(|i| *i == self.selected);
        let has_prev = pos.is_some_and(|p| p > 0);
        let has_next = pos.is_some_and(|p| p + 1 < visible.len());
        let rule = || {
            div()
                .flex_none()
                .w(px(1.))
                .h(px(16.))
                .mx(px(space::S))
                .bg(t.border_strong)
        };
        let tool = |name: &str, id: &'static str, label: SharedString, key: &'static str| {
            icon_button(t, name)
                .id(id)
                .press(t)
                .tooltip(tip(t.dark, label, key))
        };
        let done = self.done_change().map(|change| match change {
            mail::Change::Archive => ("done", "Mark Done"),
            mail::Change::NotSpam => ("inbox", "Not Spam"),
            mail::Change::Untrash => ("inbox", "Restore"),
            _ => ("inbox", "Move to Inbox"),
        });
        let in_trash = self.mailbox == View::Trash;
        div()
            .id("reading-toolbar")
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            // Nothing to clear: the window controls leave with the sidebar.
            .px(px(20.))
            // Its hairline is the line under it (`edge`).
            .h(px(TOP_BAR - 1.))
            .on_mouse_down(MouseButton::Left, drag_or_zoom)
            .child(
                tool(
                    "list",
                    "toggle-list",
                    if self.list_open {
                        "Hide Message List"
                    } else {
                        "Show Message List"
                    }
                    .into(),
                    "⌘⇧B",
                )
                .when(!self.list_open, |d| d.bg(t.selected))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_list(cx))),
            )
            .child(rule())
            .when(is_draft, |d| {
                d.child(crate::ui::glow(
                    button(t, ButtonKind::Primary, "Edit Draft")
                        .id("edit-draft")
                        .press(t)
                        .on_click(cx.listener(|this, _, _, cx| this.open_draft(this.selected, cx))),
                    "edit-draft",
                ))
                .child(div().w(px(space::S)))
            })
            .when_some(done, |d, (name, label)| {
                d.child(
                    tool(name, "mark-done", label.into(), "E")
                        .on_click(cx.listener(|this, _, _, cx| this.mark_done(cx))),
                )
            })
            .child({
                // Where the Snooze menu drops from, also when opened with H.
                let anchor = self.snooze_anchor.clone();
                tool("clock", "snooze", "Snooze".into(), "H")
                    .relative()
                    .child(
                        gpui::canvas(
                            move |bounds, _, _| {
                                anchor
                                    .set(Some(gpui::point(bounds.left(), bounds.bottom() + px(6.))))
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
            .child(rule())
            .child(
                tool("reply", "reply", "Reply".into(), "R").on_click(
                    cx.listener(|this, _, _, cx| this.open_compose(ComposeKind::Reply, cx)),
                ),
            )
            .child(
                tool("reply-all", "reply-all", "Reply All".into(), "A").on_click(
                    cx.listener(|this, _, _, cx| this.open_compose(ComposeKind::ReplyAll, cx)),
                ),
            )
            .child(tool("forward", "forward", "Forward".into(), "F").on_click(
                cx.listener(|this, _, _, cx| this.open_compose(ComposeKind::Forward, cx)),
            ))
            .child(rule())
            .child(
                tool(
                    "star",
                    "star",
                    if m.starred { "Unstar" } else { "Star" }.into(),
                    "S",
                )
                .when(m.starred, |d| d.bg(t.selected))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_star(cx))),
            )
            .child(
                tool(
                    "mail",
                    "toggle-unread",
                    if m.unread {
                        "Mark as Read"
                    } else {
                        "Mark as Unread"
                    }
                    .into(),
                    "U",
                )
                // Lit while the open thread is unread.
                .when(m.unread, |d| d.bg(t.selected))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_unread(cx))),
            )
            .child(
                tool("tag", "label", "Label".into(), "")
                    .when(self.menu == Some(Menu::Labels), |d| d.bg(t.selected))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_menu(Menu::Labels, window, cx)
                    })),
            )
            .child(tool("folder", "move-to", "Move To".into(), "").on_click(
                cx.listener(|this, _, window, cx| this.toggle_menu(Menu::MoveTo, window, cx)),
            ))
            .child(
                tool(
                    "trash",
                    "trash",
                    if in_trash {
                        "Delete Forever"
                    } else {
                        "Move to Trash"
                    }
                    .into(),
                    "⌫",
                )
                .on_click(cx.listener(|this, _, _, cx| this.trash(cx))),
            )
            .child(div().flex_1())
            .when(m.html.is_some() && !is_draft, |d| {
                d.child(
                    tool(
                        "eye",
                        "original",
                        if original {
                            "Reader View"
                        } else {
                            "Show Original"
                        }
                        .into(),
                        "V",
                    )
                    .when(original, |d| d.bg(t.selected))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_original(cx))),
                )
                .child(rule())
            })
            .child(
                tool(
                    "up",
                    "prev",
                    "Previous Thread".into(),
                    if has_prev { "K" } else { "" },
                )
                .when(!has_prev, |d| d.opacity(0.4))
                .on_click(cx.listener(|this, _, _, cx| this.step(-1, cx))),
            )
            .child(
                tool(
                    "down",
                    "next",
                    "Next Thread".into(),
                    if has_next { "J" } else { "" },
                )
                .when(!has_next, |d| d.opacity(0.4))
                .on_click(cx.listener(|this, _, _, cx| this.step(1, cx))),
            )
            .child(
                tool("more", "more", "More".into(), "")
                    .when(self.menu == Some(Menu::More), |d| d.bg(t.selected))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.toggle_menu(Menu::More, window, cx)),
                    ),
            )
    }

    pub(super) fn reading(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &self.mails[self.selected];
        // The subject is broken into lines by hand (see `balance_title`), for the width the
        // reader gives it.
        let pane_w = self.frame.map_or(0., |f| f32::from(f.size.width)) - self.pane_left();
        let column = if self.focus_restore.is_some() {
            pane_w.min(980.)
        } else {
            pane_w
        };
        // Only switch panes when this thread really has HTML to show.
        // Drafts always use the reader, so a click on the text can open the editor.
        let is_draft = self.mailbox == View::Drafts && !self.is_demo();
        let original = self.original && m.html.is_some() && !is_draft;
        // Until the message has been fetched we cannot know whether it has an original
        // (HTML) version, so show its outline, not the reader view that would flash by first.
        let loading = self.original && !is_draft && !m.loaded;
        let scrolled = !original && !loading && f32::from(self.reader_scroll.offset().y) < -0.5;
        let images = !self.in_spam(m);
        let divider = || div().flex_none().h(px(1.)).bg(t.border);

        let earlier = |tone: Tone, initials: &str, name: &str, snippet: &str, when: &str| {
            let hover = t.hover;
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .h(px(44.))
                // The hover fill reaches past the column; the content stays on it.
                .mx(px(-10.))
                .px(px(10.))
                .rounded(px(radius::ROW))
                .hover(move |s| s.bg(hover))
                .child(avatar(tone, initials, 24.))
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.text)
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
                .child(caption(when.to_string(), t.text3))
        };

        let attachment = |name: &str, info: &str, tone: Tone| {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .pl(px(8.))
                .pr(px(14.))
                .py(px(8.))
                .rounded(px(radius::CARD))
                .bg(t.hover)
                .border_1()
                .border_color(t.border)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(30.))
                        .rounded(px(radius::CONTROL))
                        .bg(tone.gradient())
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
        let pills = m.rich || m.tag.is_some();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .bg(t.surface)
            .relative()
            .child(self.selection_watch(self.reader_layouts.clone(), cx))
            // While a panel slides, the email keeps one width and the extra is cut off here.
            .overflow_hidden()
            .child(self.reading_toolbar(t, m, original, is_draft, cx))
            // Under the bar: there while something is under it, as the reader is once it
            // scrolls, and always over an email in its own formatting.
            .child(divider().when(!(original || loading || scrolled), |d| d.opacity(0.)))
            .when(loading, |d| d.child(self.loading_pane(t, m, cx.entity())))
            // Conversation (native reader)
            .when(!original && !loading, |d| {
                d.child(
                    div()
                        .id("conversation")
                        .flex_1()
                        .min_h(px(0.))
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
                            d.cursor_pointer().on_click(
                                cx.listener(|this, _, _, cx| this.open_draft(this.selected, cx)),
                            )
                        })
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(space::XL))
                                .px(px(64.))
                                // Full view keeps lines a comfortable length.
                                .when(self.focus_restore.is_some(), |d| d.w_full().max_w(px(980.)))
                                .pt(px(40.))
                                .pb(px(48.))
                                .when(pills, |d| {
                                    d.child(
                                        div()
                                            .flex()
                                            .flex_wrap()
                                            .gap(px(6.))
                                            .when(m.rich, |d| d.child(pill(t, "Product", t.violet)))
                                            .children(
                                                m.tag
                                                    .clone()
                                                    .map(|(label, a)| pill(t, &label, t.accent(a))),
                                            )
                                            .when(m.rich, |d| d.child(pill(t, "Due Fri", t.amber))),
                                    )
                                })
                                .child(self.subject_row(t, m, 30., column - 128., cx))
                                .children(self.thread_banners(t, m, cx))
                                // Who is in a conversation of several messages.
                                .when(!m.earlier.is_empty(), |d| {
                                    let people = [
                                        Some((m.tone, m.initials.clone())),
                                        m.rich.then(|| (Tone::Mint, SharedString::from("JR"))),
                                        Some((Tone::Graphite, self.me_initials.clone())),
                                    ];
                                    let count = people.iter().flatten().count();
                                    d.child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(10.))
                                            .child(
                                                div()
                                                    .relative()
                                                    .flex_none()
                                                    .h(px(28.))
                                                    .w(px(28. + 20. * (count - 1) as f32))
                                                    .children(
                                                        people
                                                            .into_iter()
                                                            .flatten()
                                                            .enumerate()
                                                            .map(|(n, (tone, initials))| {
                                                                // A ring of the page between them.
                                                                avatar(tone, &initials, 28.)
                                                                    .absolute()
                                                                    .top_0()
                                                                    .left(px(n as f32 * 20.))
                                                                    .border_2()
                                                                    .border_color(t.surface)
                                                            }),
                                                    ),
                                            )
                                            .child(meta(m.participants.clone(), t.text3).flex_1()),
                                    )
                                })
                                .when(!m.brief.is_empty(), |d| d.child(self.brief(t, m)))
                                .when(!m.earlier.is_empty(), |d| {
                                    d.child(div().flex().flex_col().gap(px(space::XS)).children(
                                        m.earlier.iter().enumerate().map(|(k, e)| {
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
                                                    .press(t)
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
                                                            .gap(px(space::M))
                                                            .pt(px(space::S))
                                                            .pb(px(space::L))
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
                                                                    images,
                                                                    &selectable,
                                                                    cx,
                                                                )
                                                                .ml(px(40.)),
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
                                        }),
                                    ))
                                })
                                .child(divider())
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(space::L))
                                        .child(self.message_header(t, m, 0, true, cx))
                                        .child(
                                            self.message_body(
                                                t,
                                                &m.body,
                                                0,
                                                images,
                                                &selectable,
                                                cx,
                                            )
                                            .ml(px(40.)),
                                        )
                                        .when(!m.loaded, |d| {
                                            d.child(
                                                self.unloaded_body(t, m, cx.entity()).ml(px(40.)),
                                            )
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
                                            .flex_wrap()
                                            .gap(px(10.))
                                            .pl(px(40.))
                                            .child(attachment(
                                                "Q4-Launch-Plan.pdf",
                                                "PDF · 4.2 MB",
                                                Tone::Rose,
                                            ))
                                            .child(attachment(
                                                "Pricing-tiers.fig",
                                                "Figma · 1.1 MB",
                                                Tone::Violet,
                                            )),
                                    )
                                }),
                        ),
                )
            })
            .when(original, |d| d.child(self.original_pane(t, m, cx)))
            .children(self.menu_view(t, cx))
    }

    /// Shown in place of an email while its content is on its way (its outline), or could
    /// not be fetched.
    pub(super) fn loading_pane(
        &self,
        t: &Theme,
        mail: &Mail,
        owner: gpui::Entity<Self>,
    ) -> impl IntoElement {
        let pane = div().flex().flex_col().flex_1().min_h(px(0.));
        if self.body_failed.contains(&self.thread_key(mail)) {
            return pane
                .items_center()
                .justify_center()
                .child(self.unloaded_body(t, mail, owner));
        }
        pane.px(px(28.))
            .pt(px(28.))
            .child(message_outline(t, self.settings.reader_text.metrics().1))
    }

    /// A message's body before it is here: its outline while it loads, or what went wrong
    /// and a way to try again.
    fn unloaded_body(&self, t: &Theme, mail: &Mail, owner: gpui::Entity<Self>) -> Div {
        let key = self.thread_key(mail);
        if !self.body_failed.contains(&key) {
            return div()
                .w_full()
                .max_w(px(self.settings.reader_text.metrics().0 * 44.))
                .child(message_outline(t, self.settings.reader_text.metrics().1));
        }
        div()
            .flex()
            .items_center()
            .gap(px(space::M))
            .text_color(t.text2)
            .child("Couldn’t load this message.")
            .child(
                button(t, ButtonKind::Secondary, "Try Again")
                    .id("message-retry")
                    .press(t)
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
                            // Name and address shrink (the address first) rather than run
                            // under the buttons beside them.
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .min_w(px(0.))
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
                                            meta(header.from.address.clone(), t.text3)
                                                .min_w(px(0.))
                                                .truncate(),
                                        )
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
                            "Reply",
                            ComposeKind::Reply,
                            index * 3,
                        ))
                        .child(reply_button(
                            "reply-all",
                            "Reply All",
                            ComposeKind::ReplyAll,
                            index * 3 + 1,
                        ))
                        .child(reply_button(
                            "forward",
                            "Forward",
                            ComposeKind::Forward,
                            index * 3 + 2,
                        ))
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
                                super::threads::snooze_chip(t, name, format!("Until {when}").into())
                            }),
                    )
                    .child(caption(time, t.text3).flex_none()),
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
                .rounded(px(radius::CHIP))
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
        // The label as wide as the longest ("Reply-To") needs, and level with the first line
        // of its value: one address is 24pt tall, and several wrap under it.
        let row = |name: &str, value: AnyElement| {
            div()
                .flex()
                .items_start()
                .gap(px(8.))
                .child(
                    div()
                        .w(px(58.))
                        .h(px(24.))
                        .flex()
                        .items_center()
                        .flex_none()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.text3)
                        .child(name.to_string()),
                )
                .child(div().flex_1().min_w(px(0.)).child(value))
        };
        let people = |list: &[mail::Person],
                      chip: &mut dyn FnMut(&mail::Person, &mut Context<Self>) -> Stateful<Div>,
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
        let from = people(std::slice::from_ref(&header.from), &mut chip, cx);
        let to = (!header.to.is_empty()).then(|| people(&header.to, &mut chip, cx));
        let cc = (!header.cc.is_empty()).then(|| people(&header.cc, &mut chip, cx));
        let reply_to = header
            .reply_to
            .as_ref()
            .map(|p| people(std::slice::from_ref(p), &mut chip, cx));
        // In line with the text under it: the sender's name and the message in the reader,
        // the avatar and the email itself in the original view (which starts at the left).
        let flush = self.original_target().is_some();
        div()
            .ml(px(if flush { 0. } else { 40. }))
            .flex()
            .flex_col()
            .gap(px(6.))
            .px(px(space::M))
            .py(px(space::M))
            .rounded(px(radius::CARD))
            .bg(t.hover)
            .border_1()
            .border_color(t.border)
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
                        .rounded(px(radius::CHIP))
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
                div().flex().justify_end().mb(px(-space::XS)).child(
                    div()
                        .id(("detail-copy-all", index))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(8.))
                        .py(px(4.))
                        .rounded(px(radius::CONTROL))
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
                        .child(if all_done { "Copied" } else { "Copy All" })
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
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .pl(px(8.))
                    .pr(px(6.))
                    .py(px(6.))
                    .max_w(px(360.))
                    .rounded(px(radius::CARD))
                    .bg(t.hover)
                    .border_1()
                    .border_color(t.border)
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(30.))
                            .rounded(px(radius::CONTROL))
                            .bg(Tone::Violet.gradient())
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
    pub(super) fn thread_banners(
        &self,
        t: &Theme,
        mail: &Mail,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
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
        if labels.is_empty() && !spam && !trash {
            return None;
        }
        let thread = (key, account);
        let (restore, not_spam) = (thread.clone(), thread.clone());
        let banners = div()
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
                                        .tooltip(tip(t.dark, "Remove Label", ""))
                                        .child(icon("x", 10., t.text3))
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
                        .h(px(44.))
                        .pl(px(14.))
                        .pr(px(6.))
                        .rounded(px(radius::CARD))
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
                            button(t, ButtonKind::Secondary, "Restore")
                                .id("restore")
                                .press(t)
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
                        .gap(px(10.))
                        .h(px(44.))
                        .pl(px(14.))
                        .pr(px(6.))
                        .rounded(px(radius::CARD))
                        .bg(t.rose.opacity(0.1))
                        .border_1()
                        .border_color(t.rose.opacity(0.35))
                        .child(icon("ban", 15., t.rose))
                        .child(div().flex_1().child("In Spam. Be careful with its links."))
                        .child(
                            button(t, ButtonKind::Secondary, "Not Spam")
                                .id("not-spam")
                                .press(t)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let (key, account) = &not_spam;
                                    if let Some(row) = this.row_to_act_on(key, account, cx) {
                                        this.remove_thread(row, mail::Change::NotSpam, cx)
                                    }
                                })),
                        ),
                )
            });
        Some(banners)
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
        let title = balance_title(&mail.subject, size, (width - taken).max(0.), cx);
        let account = mail.account.clone();
        div()
            .flex()
            .items_start()
            .gap(px(GAP))
            .child(display(title, size, t.text).flex_1().min_w(px(0.)))
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
                    button(t, ButtonKind::Secondary, "Unsubscribe")
                        .id("unsubscribe")
                        .justify_center()
                        .w(px(BUTTON_W))
                        .h(px(BUTTON_H))
                        // Centred on the subject's first line.
                        .mt(px(((size * 1.2).round() - BUTTON_H) / 2.))
                        .press(t)
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
                    .pt(px(18.))
                    .pb(px(14.))
                    .child(self.subject_row(t, m, 22., pane_w - 56., cx))
                    .children(self.thread_banners(t, m, cx))
                    .child(self.message_header(t, m, 0, false, cx)),
            )
            .child(div().flex_none().h(px(1.)).bg(t.border))
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

    /// The attachments of an email shown in its original formatting, on a card under it.
    pub(super) fn attachment_tray(
        &self,
        t: &Theme,
        attachments: &[mail::Attachment],
        account: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Div {
        let total: u64 = attachments.iter().map(|a| a.size).sum();
        let count = attachments.len();
        div().flex_none().p(px(space::M)).bg(t.surface).child(
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .p(px(space::M))
                .rounded(px(radius::CARD))
                .bg(t.raised)
                .border_1()
                .border_color(t.edge)
                .shadow(elevation(t, Elevation::Pop))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S))
                        .child(icon("clip", 14., t.text3))
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(t.text2)
                                .child(if count == 1 {
                                    "1 Attachment".to_string()
                                } else {
                                    format!("{count} Attachments")
                                }),
                        )
                        .child(caption(file_size(total), t.text3)),
                )
                .child(self.attachment_row(t, attachments, 0, account, cx)),
        )
    }
}

/// A message's text before it arrives: lines of ink at the reader's line height, with a few
/// slow passes of light, then still.
fn message_outline(t: &Theme, line: f32) -> AnyElement {
    const PARAGRAPHS: [&[f32]; 3] = [&[0.22], &[0.97, 0.92, 0.99, 0.58], &[0.94, 0.98, 0.41]];
    let ink = t.border;
    let outline = div()
        .flex()
        .flex_col()
        .gap(px((line * 0.6).round()))
        .children(PARAGRAPHS.iter().map(|lines| {
            div().flex().flex_col().children(lines.iter().map(|&share| {
                div()
                    .flex()
                    .items_center()
                    .h(px(line))
                    .w(relative(share))
                    .child(div().w_full().h(px(8.)).rounded_full().bg(ink))
            }))
        }));
    if crate::ui::motion::reduced() {
        return outline.into_any_element();
    }
    let pass = Animation::new(Duration::from_millis(1700));
    outline
        .with_animations(
            "message-outline",
            vec![pass.clone(), pass.clone(), pass],
            |outline, _, delta| outline.opacity(0.8 + 0.2 * (delta * std::f32::consts::TAU).cos()),
        )
        .into_any_element()
}

/// The reading pane's bar as the loading screen draws it: the same controls, in the same
/// places, out of reach.
pub(super) fn toolbar_outline(t: &Theme, original: bool) -> Div {
    let rule = || {
        div()
            .flex_none()
            .w(px(1.))
            .h(px(16.))
            .mx(px(space::S))
            .bg(t.border_strong)
    };
    div()
        .flex()
        .items_center()
        .gap(px(2.))
        .size_full()
        .overflow_hidden()
        .px(px(20.))
        .child(icon_button(t, "list"))
        .child(rule())
        .children(["done", "clock"].map(|name| icon_button(t, name)))
        .child(rule())
        .children(["reply", "reply-all", "forward"].map(|name| icon_button(t, name)))
        .child(rule())
        .children(["star", "mail", "tag", "folder", "trash"].map(|name| icon_button(t, name)))
        .child(div().flex_1())
        .child(icon_button(t, "eye").when(original, |d| d.bg(t.selected)))
        .child(rule())
        .children(["up", "down", "more"].map(|name| icon_button(t, name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> std::path::PathBuf {
        let mut id = [0u8; 8];
        getrandom::fill(&mut id).unwrap();
        let dir = std::env::temp_dir().join(format!("nimbra-test-{}", u64::from_le_bytes(id)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_second_save_of_the_same_name_gets_a_new_file() {
        let dir = scratch();
        let first = save_new_in(&dir, "Plan.pdf", b"one").unwrap();
        let second = save_new_in(&dir, "Plan.pdf", b"two").unwrap();
        assert_eq!(second.file_name().unwrap(), "Plan 2.pdf");
        assert_eq!(std::fs::read(first).unwrap(), b"one");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_save_never_writes_through_a_link_already_there() {
        let dir = scratch();
        let target = dir.join("target");
        std::fs::write(&target, b"keep").unwrap();
        std::os::unix::fs::symlink(&target, dir.join("Plan.pdf")).unwrap();
        let saved = save_new_in(&dir, "Plan.pdf", b"mail").unwrap();
        assert_eq!(saved.file_name().unwrap(), "Plan 2.pdf");
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn saved_attachments_are_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = scratch();
        let saved = save_new_in(&dir, "a.txt", b"x").unwrap();
        let mode = std::fs::metadata(saved).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_shown_thread_is_matched_by_account_and_thread() {
        assert!(shows_thread("a@x.co:123", "a@x.co", "123"));
        assert!(shows_thread(":123", "", "123"));
        assert!(!shows_thread("a@x.co:1234", "a@x.co", "123"));
        assert!(!shows_thread("b@x.co:123", "a@x.co", "123"));
        assert!(!shows_thread("a@x.co:123", "a@x.c", "o:123"));
    }
}
