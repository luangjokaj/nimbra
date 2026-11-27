//! Search: the field in the sidebar, what it suggests while you type (from the mail already
//! listed), and running the search in Gmail.

use super::*;

/// One result in the search panel.
#[derive(Clone)]
pub(super) struct Hit {
    pub(super) id: Option<String>,
    pub(super) sender: SharedString,
    pub(super) initials: SharedString,
    pub(super) tone: Tone,
    pub(super) subject: SharedString,
    pub(super) snippet: SharedString,
    pub(super) time: SharedString,
    pub(super) unread: bool,
    /// The conversation itself, so clicking it can open it straight away.
    pub(super) mail: Box<Mail>,
}

impl Hit {
    pub(super) fn from_mail(mail: &Mail) -> Self {
        Hit {
            id: mail.id.clone(),
            sender: mail.sender.clone().into(),
            initials: mail.initials.clone(),
            tone: mail.tone,
            subject: mail.subject.clone().into(),
            snippet: mail.snippet.clone().into(),
            time: mail.time.clone(),
            unread: mail.unread,
            mail: Box::new(mail.clone()),
        }
    }
}

/// Search tips shown while the search box is empty: what to type, and what it finds.
pub(super) const SEARCH_TIPS: &[(&str, &str)] = &[
    ("from:", "From someone"),
    ("has:attachment", "With attachments"),
    ("is:unread", "Unread only"),
    ("newer_than:7d", "From the last week"),
];

/// Local results shown at most.
const LOCAL_HITS: usize = 8;
/// Up to this many mails are searched on the spot; more go to a background thread.
const SEARCH_INLINE: usize = 2000;

/// A mail's text to search, lowercased once when the panel is first typed in, and where
/// the mail is: in the list on screen (`view` None) or kept for a mailbox.
pub(super) struct Searchable {
    text: String,
    view: Option<View>,
    row: usize,
    identity: String,
    timestamp_ms: i64,
}

/// What tells a thread apart in the search: its id, or the subject for sample mail.
fn identity(mail: &Mail) -> &str {
    mail.id.as_deref().unwrap_or(&mail.subject)
}

pub(super) fn search_text(mail: &Mail) -> String {
    format!(
        "{} {} {} {} {}",
        mail.sender, mail.header.from.address, mail.subject, mail.snippet, mail.participants
    )
    .to_lowercase()
}

/// The words of a query to look for here, lowercased; Gmail operators are left to Gmail.
pub(super) fn search_words(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split_whitespace()
        .filter(|w| !w.contains(':'))
        .map(str::to_string)
        .collect()
}

/// The entries holding every word, newest first, at most `limit`.
pub(super) fn matching(index: &[Searchable], words: &[String], limit: usize) -> Vec<usize> {
    let mut found: Vec<usize> = (0..index.len())
        .filter(|&k| words.iter().all(|w| index[k].text.contains(w.as_str())))
        .collect();
    found.sort_by(|a, b| index[*b].timestamp_ms.cmp(&index[*a].timestamp_ms));
    found.truncate(limit);
    found
}

impl Nimbra {
    /// / or ⌘F: the search panel.
    pub(super) fn focus_search(&mut self, cx: &mut Context<Self>) {
        if self.compose.is_some() || matches!(self.status, Status::SignedOut | Status::SigningIn) {
            return;
        }
        self.menu = None;
        self.palette = false;
        self.finder_open = true;
        self.finder_index = None;
        let query = self.query.clone();
        self.finder
            .update(cx, |input, cx| input.set(query, usize::MAX, cx));
        self.finder_changed(cx);
        self.pending_focus = Some(self.finder.read(cx).focus_handle(cx));
        cx.notify();
    }

    pub(super) fn close_finder(&mut self, cx: &mut Context<Self>) {
        self.finder_open = false;
        self.finder_index = None;
        self.finder_gen += 1;
        self.finder_loading = false;
        self.pending_focus = Some(self.focus.clone());
        cx.notify();
    }

    /// What was typed changed: results from the mail on this Mac at once, then (after a
    /// short pause) Gmail's own search, merged in below.
    pub(super) fn finder_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.finder.read(cx).text().trim().to_string();
        self.finder_gen += 1;
        let generation = self.finder_gen;
        self.finder_hits = self.local_hits(&query, generation, cx);
        self.finder_scroll.scroll_to_item(0);
        self.finder_sel = if self.finder_hits.is_empty() { 0 } else { 1 };
        self.finder_loading = false;
        cx.notify();
        let Some(client) = self
            .connection
            .clone()
            .filter(|_| query.chars().count() >= 2)
        else {
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
    /// Gmail), newest first. A small corpus is searched at once; a large one off the main
    /// thread, its hits put in when they come if nothing was typed since.
    fn local_hits(&mut self, query: &str, generation: u64, cx: &mut Context<Self>) -> Vec<Hit> {
        let words = search_words(query);
        if words.is_empty() {
            return Vec::new();
        }
        let index = match &self.finder_index {
            Some(index) => index.clone(),
            None => {
                let index: Arc<[Searchable]> = self.searchables().into();
                self.finder_index = Some(index.clone());
                index
            }
        };
        if index.len() <= SEARCH_INLINE {
            return self.hits_at(&index, &matching(&index, &words, LOCAL_HITS));
        }
        cx.spawn(async move |this, cx| {
            let found = {
                let index = index.clone();
                cx.background_spawn(async move { matching(&index, &words, LOCAL_HITS) })
                    .await
            };
            this.update(cx, |this, cx| {
                if this.finder_gen != generation {
                    return;
                }
                let mut hits = this.hits_at(&index, &found);
                hits.retain(|h| {
                    h.id.is_none() || !this.finder_hits.iter().any(|old| old.id == h.id)
                });
                let fetched = std::mem::take(&mut this.finder_hits);
                hits.extend(fetched);
                hits.truncate(12);
                this.finder_hits = hits;
                if this.finder_sel == 0 && !this.finder_hits.is_empty() {
                    this.finder_sel = 1;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        Vec::new()
    }

    /// Every mail on this Mac once, with its text to search.
    fn searchables(&self) -> Vec<Searchable> {
        let listed = self
            .mails
            .iter()
            .enumerate()
            .map(|(row, mail)| (None, row, mail));
        let kept = self.cache.iter().flat_map(|(view, mails)| {
            mails
                .iter()
                .enumerate()
                .map(move |(row, mail)| (Some(*view), row, mail))
        });
        let mut seen = HashSet::new();
        listed
            .chain(kept)
            .filter(|(_, _, mail)| seen.insert(identity(mail)))
            .map(|(view, row, mail)| Searchable {
                text: search_text(mail),
                view,
                row,
                identity: identity(mail).to_string(),
                timestamp_ms: mail.timestamp_ms,
            })
            .collect()
    }

    /// The hits for these entries, leaving out any whose mail has moved since.
    fn hits_at(&self, index: &[Searchable], found: &[usize]) -> Vec<Hit> {
        found
            .iter()
            .filter_map(|&k| {
                let entry = &index[k];
                let mail = match entry.view {
                    None => self.mails.get(entry.row),
                    Some(view) => self.cache.get(&view)?.get(entry.row),
                }?;
                (identity(mail) == entry.identity).then(|| Hit::from_mail(mail))
            })
            .collect()
    }

    /// Removes one search from the recent ones.
    fn forget_search(&mut self, query: &str, cx: &mut Context<Self>) {
        self.settings.recent_searches.retain(|q| q != query);
        let rows = self.settings.recent_searches.len() + SEARCH_TIPS.len();
        self.finder_sel = self.finder_sel.min(rows.saturating_sub(1));
        self.save_settings(cx);
        cx.notify();
    }

    pub(super) fn finder_step(&mut self, delta: isize, cx: &mut Context<Self>) {
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
    pub(super) fn finder_open_selected(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn finder_view(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.finder.read(cx).text().trim().to_string();
        let empty = query.is_empty();
        let hover = t.hover;
        let row_base = move |id: usize, active: bool| {
            div()
                .id(("finder-row", id))
                .flex()
                .items_center()
                .gap(dp(12.))
                .px(dp(12.))
                .py(dp(8.))
                .rounded(dp(10.))
                .cursor_pointer()
                .when(active, |d| d.bg(t.selected))
                .when(!active, |d| d.hover(move |s| s.bg(hover)))
                .press(t)
        };
        let section = |label: &str| {
            div()
                .pl(dp(12.))
                .pt(dp(10.))
                .pb(dp(6.))
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
                        .pr(dp(6.))
                        .child(section("RECENT").flex_1())
                        .child(
                            div()
                                .id("finder-clear-recent")
                                .mb(dp(3.))
                                .px(dp(6.))
                                .py(dp(2.))
                                .rounded(dp(6.))
                                .cursor_pointer()
                                .text_size(dp(11.5))
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
                        .child(div().flex_1().min_w(dp(0.)).child(short(text, 70)))
                        .child({
                            // Removes just this search.
                            let gone = text.clone();
                            div()
                                .id(("finder-forget", i))
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(dp(22.))
                                .rounded(dp(6.))
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
                                .text_size(dp(12.5))
                                .child(*tip),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(dp(0.))
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
                            .size(dp(28.))
                            .rounded(dp(8.))
                            .bg(gradient(135., c(0x8b7cff, 1.), c(0xff7ab6, 1.)))
                            .child(icon("search", 15., gpui::white())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(dp(0.))
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
                                .min_w(dp(0.))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(dp(8.))
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
                                            d.child(div().size(dp(6.)).rounded_full().bg(t.mint))
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
                                            .text_size(dp(12.))
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
                        .px(dp(12.))
                        .py(dp(14.))
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
                .gap(dp(5.))
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
            .pt(dp(140.))
            .bg(t.scrim)
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close_finder(cx)),
            )
            .child(
                div()
                    .id("finder")
                    .w(dp(680.))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded(dp(18.))
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
                            .gap(dp(12.))
                            .pl(dp(20.))
                            .pr(dp(16.))
                            .py(dp(16.))
                            .child(icon("search", 18., t.text3))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(dp(0.))
                                    .text_size(dp(17.))
                                    .child(self.finder.clone()),
                            )
                            .when(self.finder_loading, |d| {
                                d.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(dp(6.))
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
                            })
                            // Something typed: ✕ empties the field, keeping it ready to type.
                            .when(!empty, |d| {
                                d.child(
                                    div()
                                        .id("finder-clear")
                                        .flex()
                                        .flex_none()
                                        .items_center()
                                        .justify_center()
                                        .size(dp(24.))
                                        .rounded(dp(6.))
                                        .cursor_pointer()
                                        .hover(move |s| s.bg(hover))
                                        .tooltip(tip(t.dark, "Clear", ""))
                                        .child(icon("x", 15., t.text3))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.finder.update(cx, |input, cx| {
                                                input.set(String::new(), 0, cx)
                                            });
                                            this.finder_changed(cx);
                                            this.pending_focus =
                                                Some(this.finder.read(cx).focus_handle(cx));
                                            cx.notify();
                                        })),
                                )
                            }),
                    )
                    .child(div().h(dp(1.)).bg(t.border))
                    .child(
                        div()
                            .id("finder-rows")
                            .flex()
                            .flex_col()
                            .max_h(dp(420.))
                            .overflow_y_scroll()
                            .track_scroll(&self.finder_scroll)
                            .p(dp(8.))
                            .children(rows),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(dp(16.))
                            .px(dp(20.))
                            .py(dp(12.))
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
    pub(super) fn run_search(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn clear_search(&mut self, cx: &mut Context<Self>) {
        self.search
            .update(cx, |input, cx| input.set(String::new(), 0, cx));
        self.query.clear();
        self.pending_focus = Some(self.focus.clone());
        if self.mailbox == View::Search {
            self.select_mailbox(View::Inbox, cx);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(text: &str, timestamp_ms: i64) -> Searchable {
        Searchable {
            text: text.to_lowercase(),
            view: None,
            row: 0,
            identity: String::new(),
            timestamp_ms,
        }
    }

    #[test]
    fn a_query_keeps_its_words_lowercased_and_leaves_operators_to_gmail() {
        assert_eq!(search_words("  Mira  from:ana DECK "), ["mira", "deck"]);
        assert!(search_words("is:unread").is_empty());
    }

    #[test]
    fn every_word_must_match_and_the_newest_come_first_up_to_the_limit() {
        let index = [
            entry("Mira Q4 launch plan deck", 10),
            entry("Stripe payout", 30),
            entry("mira deck review", 20),
            entry("deck only", 40),
        ];
        let words = search_words("DECK mira");
        assert_eq!(matching(&index, &words, 8), [2, 0]);
        assert_eq!(matching(&index, &search_words("deck"), 2), [3, 2]);
        assert!(matching(&index, &search_words("nothing"), 8).is_empty());
    }

    #[test]
    fn a_mail_is_found_by_sender_address_subject_snippet_and_participants() {
        let mut mail = crate::data::inbox().remove(0);
        mail.header.from.address = "Someone@Example.org".into();
        let text = search_text(&mail);
        assert!(text.contains("someone@example.org"));
        assert!(text.contains(&mail.subject.to_lowercase()));
        assert_eq!(text, text.to_lowercase());
    }
}
