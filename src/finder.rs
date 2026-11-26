//! Search: the field in the sidebar, what it suggests while you type (from the mail already
//! listed), and running the search on the account's server.

use super::*;
use crate::theme::{radius, space};

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
            sender: mail.sender.clone(),
            initials: mail.initials.clone(),
            tone: mail.tone,
            subject: mail.subject.clone(),
            snippet: mail.snippet.clone(),
            time: mail.time.clone(),
            unread: mail.unread,
            mail: Box::new(mail.clone()),
        }
    }
}

/// One thread of the mail on this Mac, as the search panel looks for words in it: made once
/// when the panel opens, not on every keystroke.
pub(super) struct Indexed {
    id: Option<String>,
    account: Option<String>,
    subject: SharedString,
    timestamp_ms: i64,
    /// Sender, address, subject, preview and participants, lowercased.
    words: String,
}

impl Indexed {
    fn of(mail: &Mail) -> Self {
        Self {
            id: mail.id.clone(),
            account: mail.account.clone(),
            subject: mail.subject.clone(),
            timestamp_ms: mail.timestamp_ms,
            words: format!(
                "{} {} {} {} {}",
                mail.sender,
                mail.header.from.address,
                mail.subject,
                mail.snippet,
                mail.participants
            )
            .to_lowercase(),
        }
    }

    fn matches(&self, words: &[String]) -> bool {
        words.iter().all(|w| self.words.contains(w.as_str()))
    }

    /// Whether this is that thread. Sample mail has no ids: matched by subject.
    fn is(&self, mail: &Mail) -> bool {
        match &self.id {
            Some(id) => mail.id.as_ref() == Some(id) && mail.account == self.account,
            None => mail.id.is_none() && mail.subject == self.subject,
        }
    }
}

/// The words of a query that are looked for in the mail on this Mac. Operators like `from:`
/// are left to the server.
/// A tip as it goes into the field: an operator waiting for its value, or a whole term and
/// a space.
fn tip_text(tip: &str) -> String {
    if tip.ends_with(':') {
        tip.to_string()
    } else {
        format!("{tip} ")
    }
}

fn query_words(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split_whitespace()
        .filter(|w| !w.contains(':'))
        .map(str::to_string)
        .collect()
}

/// Search tips shown while the search box is empty and nothing was searched yet: what to
/// type, and what it finds.
pub(super) const SEARCH_TIPS: &[(&str, &str)] = &[
    ("from:", "From someone"),
    ("has:attachment", "With attachments"),
    ("is:unread", "Unread only"),
    ("newer_than:7d", "From the last week"),
];

impl Nimbra {
    /// / or ⌘F: the search panel.
    pub(super) fn focus_search(&mut self, cx: &mut Context<Self>) {
        if self.compose.is_some() || matches!(self.status, Status::SignedOut | Status::SigningIn) {
            return;
        }
        self.menu = None;
        self.finder_open = true;
        self.finder_index = self.index_mail();
        let query = self.query.clone();
        // The field is made once; the theme may have changed since.
        let t = self.theme();
        self.finder.update(cx, |input, cx| {
            input.muted = t.text3;
            input.accent = t.violet;
            input.set(query, usize::MAX, cx);
        });
        self.finder_changed(cx);
        self.pending_focus = Some(self.finder.read(cx).focus_handle(cx));
        cx.notify();
    }

    pub(super) fn close_finder(&mut self, cx: &mut Context<Self>) {
        self.finder_open = false;
        self.finder_gen += 1;
        self.finder_loading = false;
        self.finder_task = None;
        self.finder_index = Vec::new();
        self.pending_focus = Some(self.focus.clone());
        cx.notify();
    }

    /// What was typed changed: results from the mail on this Mac at once, then (after a
    /// short pause) the server's own search, merged in below.
    pub(super) fn finder_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.finder.read(cx).text().trim().to_string();
        self.finder_hits = self.local_hits(&query);
        self.finder_scroll.scroll_to_item(0);
        self.finder_sel = if self.finder_hits.is_empty() { 0 } else { 1 };
        self.finder_gen += 1;
        let generation = self.finder_gen;
        self.finder_loading = false;
        // A newer keystroke drops the wait (and the answer) of the one before.
        self.finder_task = None;
        cx.notify();
        let Some(client) = self
            .connection
            .clone()
            .filter(|_| query.chars().count() >= 2)
        else {
            return;
        };
        self.finder_loading = true;
        self.finder_task = Some(cx.spawn(async move |this, cx| {
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
        }));
    }

    /// The mail on this Mac (the open list and every mailbox kept in memory), each thread
    /// once.
    fn index_mail(&self) -> Vec<Indexed> {
        let mut seen = HashSet::new();
        self.mails
            .iter()
            .chain(self.cache.values().flatten())
            .filter(|mail| {
                seen.insert((
                    mail.id.clone().unwrap_or_else(|| mail.subject.to_string()),
                    mail.account.clone(),
                ))
            })
            .map(Indexed::of)
            .collect()
    }

    /// Mail on this Mac matching every word typed, newest first.
    fn local_hits(&self, query: &str) -> Vec<Hit> {
        let words = query_words(query);
        if words.is_empty() {
            return Vec::new();
        }
        let mut found: Vec<&Indexed> = self
            .finder_index
            .iter()
            .filter(|thread| thread.matches(&words))
            .collect();
        found.sort_by_key(|thread| std::cmp::Reverse(thread.timestamp_ms));
        // Looked up as they are now: a thread may have changed or gone since the panel opened.
        found
            .into_iter()
            .filter_map(|thread| {
                self.mails
                    .iter()
                    .chain(self.cache.values().flatten())
                    .find(|mail| thread.is(mail))
            })
            .take(8)
            .map(Hit::from_mail)
            .collect()
    }

    /// While nothing is typed: the recent searches, or the tips until there are some.
    fn finder_resting_rows(&self) -> usize {
        match self.settings.recent_searches.len() {
            0 => SEARCH_TIPS.len(),
            recent => recent,
        }
    }

    /// Removes one search from the recent ones.
    fn forget_search(&mut self, query: &str, cx: &mut Context<Self>) {
        self.settings.recent_searches.retain(|q| q != query);
        self.finder_sel = self
            .finder_sel
            .min(self.finder_resting_rows().saturating_sub(1));
        self.save_settings(cx);
        cx.notify();
    }

    pub(super) fn finder_step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let empty = self.finder.read(cx).text().trim().is_empty();
        let rows = if empty {
            self.finder_resting_rows()
        } else {
            self.finder_hits.len() + 1
        };
        if rows == 0 {
            return;
        }
        self.finder_sel = (self.finder_sel as isize + delta).rem_euclid(rows as isize) as usize;
        // Keep the highlighted row in view. Its place among the panel's children counts the
        // section title above the rows: one while nothing is typed ("Recent" or "Search
        // Tips"), and after "All results" otherwise. The first row brings the top into view.
        let child = match (empty, self.finder_sel) {
            (_, 0) | (false, 1) => 0,
            (_, sel) => sel + 1,
        };
        self.finder_scroll.scroll_to_item(child);
        cx.notify();
    }

    /// Return: open the highlighted result, or all results for what was typed.
    pub(super) fn finder_open_selected(&mut self, cx: &mut Context<Self>) {
        let query = self.finder.read(cx).text().trim().to_string();
        if query.is_empty() {
            let recent = &self.settings.recent_searches;
            let text = if recent.is_empty() {
                SEARCH_TIPS
                    .get(self.finder_sel)
                    .map(|(tip, _)| tip_text(tip))
            } else {
                recent.get(self.finder_sel).cloned()
            };
            if let Some(text) = text {
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
                .gap(px(space::M))
                .px(px(space::M))
                .py(px(space::S))
                .rounded(px(radius::ROW))
                .cursor_pointer()
                .when(active, |d| {
                    d.relative()
                        .bg(t.selected)
                        .child(lit_edge(t, radius::ROW))
                        .press_selected(t)
                })
                .when(!active, |d| d.hover(move |s| s.bg(hover)).press(t))
        };
        let section = |label: &str| {
            div()
                .pl(px(space::M))
                .pt(px(10.))
                .pb(px(6.))
                .child(overline(t, label.to_string()))
        };
        let mut rows: Vec<AnyElement> = Vec::new();
        if empty {
            let recent = &self.settings.recent_searches;
            if recent.is_empty() {
                rows.push(section("Search Tips").into_any_element());
                for (i, (tip, what)) in SEARCH_TIPS.iter().enumerate() {
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
                                let text = tip_text(tip);
                                this.finder
                                    .update(cx, |input, cx| input.set(text, usize::MAX, cx));
                                this.finder_changed(cx);
                                this.pending_focus = Some(this.finder.read(cx).focus_handle(cx));
                            }))
                            .into_any_element(),
                    );
                }
            } else {
                rows.push(
                    div()
                        .flex()
                        .items_end()
                        .pr(px(6.))
                        .child(section("Recent").flex_1())
                        .child(
                            div()
                                .id("finder-clear-recent")
                                .mb(px(3.))
                                .px(px(6.))
                                .py(px(2.))
                                .rounded(px(radius::CHIP))
                                .cursor_pointer()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(t.text3)
                                .hover(move |s| s.bg(hover).text_color(t.text))
                                .press(t)
                                .child("Clear")
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
                        .child(div().flex_1().min_w(px(0.)).truncate().child(text.clone()))
                        .child({
                            // Removes just this search.
                            let gone = text.clone();
                            div()
                                .id(("finder-forget", i))
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(22.))
                                .rounded(px(radius::CHIP))
                                .when(self.finder_sel != i, |d| {
                                    d.invisible().group_hover("finder-recent", |s| s.visible())
                                })
                                .hover(move |s| s.bg(hover))
                                .press(t)
                                .child(icon("x", 13., t.text3))
                                .tooltip(tip(t.dark, "Remove", ""))
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
                            .rounded(px(radius::CONTROL))
                            .bg(solid_violet(t).0)
                            .child(icon("search", 15., gpui::white())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .child(format!("All results for “{query}”")),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.search_for(&q, None, cx)))
                    .into_any_element(),
            );
            if !self.finder_hits.is_empty() {
                rows.push(section("Conversations").into_any_element());
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
                                        .gap(px(space::S))
                                        .child(
                                            div()
                                                .min_w(px(0.))
                                                .truncate()
                                                .font_weight(if hit.unread {
                                                    FontWeight::SEMIBOLD
                                                } else {
                                                    FontWeight::MEDIUM
                                                })
                                                .child(hit.sender.clone()),
                                        )
                                        .when(hit.unread, |d| {
                                            d.child(
                                                div()
                                                    .flex_none()
                                                    .size(px(6.))
                                                    .rounded_full()
                                                    .bg(t.violet),
                                            )
                                        })
                                        .child(div().flex_1())
                                        .child(caption(hit.time.clone(), t.text3)),
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
                // The server was asked too unless that was not possible.
                let asked = self.connection.is_some() && query.chars().count() >= 2;
                rows.push(
                    div()
                        .px(px(space::M))
                        .py(px(14.))
                        .text_color(t.text3)
                        .child(if asked {
                            "No Matches"
                        } else {
                            "No Matches on This Mac"
                        })
                        .into_any_element(),
                );
            }
        }
        // Asking the server: a turning mark beside the field, still under Reduce Motion.
        let busy = self.finder_loading.then(|| {
            let mark = icon("reload", 13., t.violet);
            if crate::ui::motion::reduced() {
                mark.opacity(0.7).into_any_element()
            } else {
                mark.with_animation(
                    "finder-busy",
                    Animation::new(Duration::from_millis(900)).repeat(),
                    |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
                )
                .into_any_element()
            }
        });
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
                    .relative()
                    .rounded(px(radius::SHEET))
                    .bg(t.raised)
                    .shadow(sheet_shadow(t))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::M))
                            .h(px(56.))
                            .pl(px(20.))
                            .pr(px(20.))
                            .child(icon("search", 18., t.text3))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .text_size(px(17.))
                                    .child(self.finder.clone()),
                            )
                            .children(busy),
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
                            .p(px(space::S))
                            .children(rows),
                    )
                    .child(edge(t, radius::SHEET)),
            )
    }

    /// Runs the typed query: against the server (Gmail's full search syntax works there), or
    /// locally on the sample data.
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

    fn mail(id: &str, account: Option<&str>, subject: &str) -> Mail {
        let mut mail = crate::data::inbox().remove(0);
        mail.id = Some(id.into());
        mail.account = account.map(Into::into);
        mail.subject = subject.to_string().into();
        mail
    }

    #[test]
    fn operators_are_left_to_the_server() {
        assert_eq!(query_words("From:Mira  Launch"), vec!["launch"]);
        assert!(query_words("has:attachment").is_empty());
    }

    #[test]
    fn every_word_must_appear_whatever_its_case() {
        let thread = Indexed::of(&mail("a", None, "Q4 Launch Plan"));
        assert!(thread.matches(&query_words("launch q4")));
        assert!(!thread.matches(&query_words("launch budget")));
    }

    #[test]
    fn the_same_thread_id_in_another_account_is_another_thread() {
        let thread = Indexed::of(&mail("a", Some("work"), "Plan"));
        assert!(thread.is(&mail("a", Some("work"), "Plan, renamed")));
        assert!(!thread.is(&mail("a", Some("home"), "Plan")));
    }

    #[test]
    fn a_tip_waits_for_its_value_or_ends_its_term() {
        assert_eq!(tip_text("from:"), "from:");
        assert_eq!(tip_text("is:unread"), "is:unread ");
    }
}
