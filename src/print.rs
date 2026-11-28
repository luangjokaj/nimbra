//! The printed page: a whole conversation or one message, laid out by Nimbra (the Figma
//! file's "Print" frames) and handed to the hidden print web view.
//!
//! The page is always light. Its own type is the app's (Geist, Geist Mono and Instrument
//! Serif, embedded as `data:` fonts, since the web view cannot see the app's fonts). Each
//! email in its original HTML sits in a shadow root of its own, so its CSS styles only that
//! email: not Nimbra's headers, and not the other emails of the conversation. The same
//! Content-Security-Policy as the reading view blocks everything remote, and remote images
//! load only when the reading view shows them.

use std::sync::OnceLock;

use base64::Engine as _;
use nimbra_core::listing::Header;
use nimbra_core::mail::Person;
use nimbra_core::rich::{Block, Kind};

/// What to print, chosen in the More menu or with its shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Every message of the thread, oldest first.
    Conversation,
    /// The message being read.
    Message,
}

/// An email's text as the page shows it.
pub enum Body<'a> {
    /// Its own HTML, as in the original view.
    Html(&'a str),
    /// The reader's text.
    Blocks(&'a [Block]),
}

pub struct Message<'a> {
    pub header: &'a Header,
    /// Who sent it, when the header has no sender (sample mail).
    pub sender: String,
    pub initials: String,
    /// The avatar's gradient (`Tone::stops`).
    pub tone: (u32, u32),
    /// When it was sent, written out.
    pub date: String,
    pub body: Body<'a>,
}

pub struct Page<'a> {
    /// Told back by the page once it has loaded (`READY`), so an earlier page still
    /// loading is never the one printed.
    pub job: u64,
    pub subject: &'a str,
    /// One message with its full headers, or a conversation under compact ones.
    pub scope: Scope,
    /// Where the message sits in its thread: (place, of how many), oldest first.
    pub place: (usize, usize),
    pub messages: Vec<Message<'a>>,
    /// "Printed from Nimbra on …".
    pub printed: String,
    /// Remote images may load (the reading view shows them).
    pub images: bool,
}

/// Run by the app once the page has loaded: gives emails their shadow root where the
/// system's WebKit did not (declarative shadow DOM came with macOS 13.3), then answers the
/// page's job and how many it had to attach, or nothing while the page is still loading.
/// Fixed text; the page's own scripts never run.
pub const READY: &str = "(function(){if(document.readyState!=='complete')return '';\
var n=0;document.querySelectorAll('template[shadowrootmode]').forEach(function(t){\
var h=t.parentNode;if(h&&!h.shadowRoot){h.attachShadow({mode:'open'}).appendChild(t.content);n++;}\
t.remove();});var m=document.querySelector('meta[name=\"nimbra-print\"]');\
return m?m.content+' '+n:'';})()";

/// `READY`'s answer: the page's job, and whether emails were attached just now (their
/// images then still have to load).
pub fn ready(answer: &str) -> Option<(u64, bool)> {
    let answer = answer.trim().trim_matches('"');
    let (job, attached) = answer.split_once(' ')?;
    Some((job.parse().ok()?, attached.parse::<u32>().ok()? > 0))
}

/// The whole page, ready for the web view as it is.
pub fn document(page: &Page) -> String {
    let img = if page.images { "data: https:" } else { "data:" };
    let mut out = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; \
         img-src {img}; style-src 'unsafe-inline'; font-src data:; form-action 'none'; \
         base-uri 'none'\">\
         <meta http-equiv=\"x-dns-prefetch-control\" content=\"off\">\
         <meta name=\"color-scheme\" content=\"light only\">\
         <meta name=\"nimbra-print\" content=\"{}\">\
         <title>{}</title><style>{}{STYLE}</style></head><body>",
        page.job,
        esc(page.subject),
        fonts(),
    );
    let (place, count) = page.place;
    let kind = match page.scope {
        Scope::Conversation => format!("Conversation · {count} messages"),
        Scope::Message if count > 1 => format!("Message {place} of {count}"),
        Scope::Message => "Message".to_string(),
    };
    out.push_str(&format!(
        "<header class=\"np-mast\"><span class=\"np-word\">Nimbra</span>\
         <span class=\"np-kind\">{}</span></header>",
        esc(&kind)
    ));
    let subject = if page.subject.trim().is_empty() {
        "(no subject)"
    } else {
        page.subject
    };
    out.push_str(&format!("<h1 class=\"np-subject\">{}</h1>", esc(subject)));
    match page.scope {
        Scope::Conversation => {
            let mut people: Vec<&str> = Vec::new();
            for m in &page.messages {
                let name = from_label(m);
                if !name.is_empty() && !people.contains(&name) {
                    people.push(name);
                }
            }
            if !people.is_empty() {
                out.push_str(&format!(
                    "<div class=\"np-people\">{}</div>",
                    esc(&people.join(", "))
                ));
            }
            out.push_str("<main class=\"np-thread\">");
            for m in &page.messages {
                out.push_str(&compact(m, page.images));
            }
            out.push_str("</main>");
        }
        Scope::Message => {
            if let Some(m) = page.messages.first() {
                out.push_str(&full(m, page.images));
            }
        }
    }
    out.push_str(&format!(
        "<footer class=\"np-foot\">{}</footer></body></html>",
        esc(&page.printed)
    ));
    out
}

/// A message in a conversation: avatar, sender, recipients and date over its text.
fn compact(m: &Message, images: bool) -> String {
    let (a, b) = m.tone;
    let from = &m.header.from;
    let address = if from.address.is_empty() || from.name.is_empty() {
        String::new()
    } else {
        format!(" <span class=\"np-addr\">{}</span>", esc(&from.address))
    };
    let mut to = Vec::new();
    if !m.header.to.is_empty() {
        to.push(format!("to {}", names(&m.header.to)));
    }
    if !m.header.cc.is_empty() {
        to.push(format!("cc {}", names(&m.header.cc)));
    }
    let to = if to.is_empty() {
        String::new()
    } else {
        format!("<div class=\"np-to\">{}</div>", esc(&to.join(", ")))
    };
    let wide = matches!(m.body, Body::Html(_));
    format!(
        "<article class=\"np-msg\"><div class=\"np-head\">\
         <span class=\"np-av\" style=\"background:linear-gradient(135deg,#{a:06x},#{b:06x})\">{}</span>\
         <div class=\"np-who\"><div><span class=\"np-name\">{}</span>{address}</div>{to}</div>\
         <time class=\"np-date\">{}</time></div>\
         <div class=\"np-body{}\">{}{}</div></article>",
        esc(&m.initials),
        esc(from_label(m)),
        esc(&m.date),
        if wide { " np-wide" } else { "" },
        body(&m.body, images),
        attachments(m.header),
    )
}

/// The one message printed: its full headers in a quiet grid, then its text.
fn full(m: &Message, images: bool) -> String {
    let mut rows = String::new();
    let mut row = |label: &str, value: String| {
        if !value.is_empty() {
            rows.push_str(&format!("<dt>{label}</dt><dd>{}</dd>", esc(&value)));
        }
    };
    let from = if m.header.from.address.is_empty() {
        m.sender.clone()
    } else {
        person(&m.header.from)
    };
    row("From", from);
    row("To", people(&m.header.to));
    row("Cc", people(&m.header.cc));
    if let Some(reply_to) = &m.header.reply_to
        && !reply_to
            .address
            .eq_ignore_ascii_case(&m.header.from.address)
    {
        row("Reply-To", person(reply_to));
    }
    row("Date", m.date.clone());
    format!(
        "<dl class=\"np-fields\">{rows}</dl><div class=\"np-body np-single\">{}{}</div>",
        body(&m.body, images),
        attachments(m.header),
    )
}

fn from_label<'a>(m: &'a Message) -> &'a str {
    let from = &m.header.from;
    if from.address.is_empty() && from.name.is_empty() {
        &m.sender
    } else {
        from.label()
    }
}

fn names(list: &[Person]) -> String {
    list.iter()
        .map(Person::label)
        .collect::<Vec<_>>()
        .join(", ")
}

fn person(p: &Person) -> String {
    if p.name.is_empty() {
        p.address.clone()
    } else {
        format!("{} <{}>", p.name, p.address)
    }
}

fn people(list: &[Person]) -> String {
    list.iter().map(person).collect::<Vec<_>>().join(", ")
}

fn body(body: &Body, images: bool) -> String {
    match body {
        Body::Html(html) => format!(
            "<div class=\"np-mail\"><template shadowrootmode=\"open\"><style>{MAIL}</style>{}</template></div>",
            contained(html)
        ),
        Body::Blocks(blocks) => format!("<div class=\"np-text\">{}</div>", text(blocks, images)),
    }
}

fn attachments(header: &Header) -> String {
    if header.attachments.is_empty() {
        return String::new();
    }
    let mut out = String::from("<ul class=\"np-atts\">");
    for a in &header.attachments {
        let size = crate::app::file_size(a.size);
        out.push_str(&format!(
            "<li>{CLIP}<span>{}</span>{}</li>",
            esc(&a.name),
            if size.is_empty() {
                String::new()
            } else {
                format!("<small>{}</small>", esc(&size))
            }
        ));
    }
    out.push_str("</ul>");
    out
}

/// The reader's blocks as HTML: paragraphs, headings, list items, code, rules and the
/// images it may show, quoted text indented with a bar per level.
fn text(blocks: &[Block], images: bool) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for block in blocks {
        while depth < block.quote {
            out.push_str("<blockquote>");
            depth += 1;
        }
        while depth > block.quote {
            out.push_str("</blockquote>");
            depth -= 1;
        }
        let inline = spans(block);
        match &block.kind {
            Kind::Paragraph => out.push_str(&format!("<p>{inline}</p>")),
            Kind::Heading(level) => {
                let level = (*level).clamp(1, 4) + 1;
                out.push_str(&format!("<h{level}>{inline}</h{level}>"));
            }
            Kind::Item { marker, depth } => out.push_str(&format!(
                "<div class=\"np-li\" style=\"padding-left:{}px\"><span>{}</span>{inline}</div>",
                18 * u32::from(*depth) + 18,
                esc(marker)
            )),
            Kind::Code => out.push_str(&format!("<pre>{}</pre>", esc(&block.text))),
            Kind::Rule => out.push_str("<hr>"),
            Kind::Image { src, alt } => {
                // Remote images only when the page may load them; otherwise their words.
                let fetchable = src.starts_with("data:") || (images && src.starts_with("https:"));
                if fetchable {
                    out.push_str(&format!("<img src=\"{}\" alt=\"{}\">", esc(src), esc(alt)));
                } else if !alt.trim().is_empty() {
                    out.push_str(&format!("<p class=\"np-alt\">[{}]</p>", esc(alt)));
                }
            }
        }
    }
    for _ in 0..depth {
        out.push_str("</blockquote>");
    }
    out
}

/// A block's text with its bold, italic, code and links. Spans are byte ranges, in order
/// and never overlapping; one that does not fall on the text's characters is left plain.
fn spans(block: &Block) -> String {
    let text = &block.text;
    let mut out = String::new();
    let mut at = 0;
    for span in &block.spans {
        let (start, end) = (span.range.start, span.range.end);
        if start < at || end <= start {
            continue;
        }
        let (Some(before), Some(inside)) = (text.get(at..start), text.get(start..end)) else {
            continue;
        };
        out.push_str(&lines(before));
        let mut piece = lines(inside);
        if span.code {
            piece = format!("<code>{piece}</code>");
        }
        if span.italic {
            piece = format!("<em>{piece}</em>");
        }
        if span.bold {
            piece = format!("<strong>{piece}</strong>");
        }
        if let Some(link) = &span.link {
            piece = format!("<a href=\"{}\">{piece}</a>", esc(link));
        }
        out.push_str(&piece);
        at = end;
    }
    out.push_str(&lines(text.get(at..).unwrap_or("")));
    out
}

fn lines(text: &str) -> String {
    esc(text).replace('\n', "<br>")
}

/// The email's HTML made safe to place inside its `<template>`: `<link>` tags inert (as in
/// the reading view), no `</template>` that would end it early and spill the email's
/// styles over the page, and no `<plaintext>` that would swallow the rest of the page.
/// Tag names can only be written literally, in any letter case, so no parser is needed.
fn contained(html: &str) -> String {
    let html = crate::webview::without_links(html);
    let mut out = String::with_capacity(html.len());
    let bytes = html.as_bytes();
    let mut done = 0;
    for (at, _) in html.match_indices('<') {
        if at < done {
            continue;
        }
        let rest = &bytes[at + 1..];
        let (closing, name) = match rest.first() {
            Some(b'/') => (true, &rest[1..]),
            _ => (false, rest),
        };
        let starts = |word: &[u8]| {
            name.len() >= word.len()
                && name[..word.len()].eq_ignore_ascii_case(word)
                && matches!(
                    name.get(word.len()),
                    None | Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'/' | b'>')
                )
        };
        let replacement = if starts(b"template") {
            Some((
                "template".len(),
                if closing {
                    "</nimbra-x"
                } else {
                    "<nimbra-x hidden"
                },
            ))
        } else if starts(b"plaintext") {
            Some(("plaintext".len(), if closing { "</pre" } else { "<pre" }))
        } else {
            None
        };
        if let Some((len, with)) = replacement {
            out.push_str(&html[done..at]);
            out.push_str(with);
            done = at + 1 + usize::from(closing) + len;
        }
    }
    out.push_str(&html[done..]);
    out
}

fn esc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The app's fonts as `@font-face` rules with `data:` sources, made once.
fn fonts() -> &'static str {
    static FONTS: OnceLock<String> = OnceLock::new();
    FONTS.get_or_init(|| {
        let faces: [(&str, u16, &[u8]); 5] = [
            (
                "Geist",
                400,
                include_bytes!("../assets/fonts/Geist-Regular.ttf"),
            ),
            (
                "Geist",
                500,
                include_bytes!("../assets/fonts/Geist-Medium.ttf"),
            ),
            (
                "Geist",
                600,
                include_bytes!("../assets/fonts/Geist-SemiBold.ttf"),
            ),
            (
                "Geist Mono",
                500,
                include_bytes!("../assets/fonts/GeistMono-Medium.ttf"),
            ),
            (
                "Instrument Serif",
                400,
                include_bytes!("../assets/fonts/InstrumentSerif-Regular.ttf"),
            ),
        ];
        faces
            .iter()
            .map(|(family, weight, bytes)| {
                format!(
                    "@font-face{{font-family:'{family}';font-weight:{weight};font-style:normal;\
                     src:url(data:font/ttf;base64,{}) format('truetype')}}",
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                )
            })
            .collect()
    })
}

/// The paper clip in front of each attachment (the app's `clip` icon).
const CLIP: &str = "<svg width=\"13\" height=\"13\" viewBox=\"0 0 24 24\" fill=\"none\" \
stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\" \
aria-hidden=\"true\"><path d=\"m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 \
18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48\"/></svg>";

/// Nimbra's page, the Figma "Print" frames in light colours (`Nimbra` collection, Light).
/// WebKit lays a printed page out about a quarter wider than the paper and shrinks it, so
/// 14px text prints near 11pt.
const STYLE: &str = "\
:root{color-scheme:light only;-webkit-print-color-adjust:exact;print-color-adjust:exact}\
html,body{background:#fff}\
body{margin:0;font:14px/1.5 Geist,-apple-system,'Helvetica Neue',sans-serif;color:#17161c;\
-webkit-font-smoothing:antialiased;overflow-wrap:break-word}\
.np-mast{display:flex;justify-content:space-between;align-items:center;padding-bottom:14px;\
border-bottom:1px solid #dcdae0;margin-bottom:28px}\
.np-word{font:400 21px/1.2 'Instrument Serif',Georgia,serif;color:#17161c}\
.np-kind{font:500 10px/1.4 'Geist Mono',ui-monospace,monospace;letter-spacing:.06em;\
text-transform:uppercase;color:#8c8a95}\
.np-subject{font:400 30px/1.15 'Instrument Serif',Georgia,serif;margin:0;color:#17161c;\
break-after:avoid}\
.np-people{font-size:12px;color:#8c8a95;margin-top:6px}\
.np-thread{margin-top:4px}\
.np-msg{padding:24px 0;border-top:1px solid #ebe9ee}\
.np-msg:first-child{border-top:0}\
.np-head{display:flex;gap:14px;align-items:flex-start;margin-bottom:14px;break-inside:avoid;\
break-after:avoid}\
.np-av{flex:none;width:28px;height:28px;border-radius:50%;color:#fff;font:600 11px/28px Geist,\
sans-serif;text-align:center;letter-spacing:.02em}\
.np-who{flex:1;min-width:0;line-height:1.35}\
.np-name{font-weight:600;font-size:13px}\
.np-addr,.np-to{font-size:12px;color:#8c8a95}\
.np-date{flex:none;font:500 11px/1.7 'Geist Mono',ui-monospace,monospace;color:#8c8a95;\
white-space:nowrap}\
.np-body{margin-left:42px}\
.np-body.np-wide,.np-body.np-single{margin-left:0}\
.np-fields{display:grid;grid-template-columns:64px 1fr;row-gap:6px;column-gap:0;margin:24px 0 0;\
padding-bottom:20px;border-bottom:1px solid #ebe9ee;font-size:13px;break-inside:avoid}\
.np-fields dt{font-size:12px;line-height:19px;color:#8c8a95}\
.np-fields dd{margin:0;line-height:19px}\
.np-single{margin-top:24px}\
.np-text{font-size:14px;line-height:22px}\
.np-text p,.np-text pre,.np-text h2,.np-text h3,.np-text h4,.np-text h5{margin:0 0 12px}\
.np-text h2{font-size:18px;line-height:26px;font-weight:600}\
.np-text h3,.np-text h4,.np-text h5{font-size:15px;line-height:22px;font-weight:600}\
.np-text pre{font:13px/20px 'Geist Mono',ui-monospace,monospace;white-space:pre-wrap;\
background:#f6f5f3;border-radius:6px;padding:8px 10px}\
.np-text code{font-family:'Geist Mono',ui-monospace,monospace;font-size:.92em}\
.np-text hr{border:0;border-top:1px solid #ebe9ee;margin:16px 0}\
.np-text img{max-width:100%;height:auto;margin:0 0 12px}\
.np-text a{color:inherit}\
.np-text blockquote{margin:0 0 12px;padding-left:12px;border-left:2px solid #dcdae0;color:#8c8a95}\
.np-li{position:relative;margin:0 0 6px}\
.np-li>span{position:absolute;margin-left:-18px;color:#8c8a95}\
.np-alt{color:#8c8a95}\
.np-mail{display:block}\
.np-atts{display:flex;flex-wrap:wrap;gap:8px;list-style:none;margin:14px 0 0;padding:0;\
break-inside:avoid}\
.np-atts li{display:inline-flex;align-items:center;gap:7px;border:1px solid #dcdae0;\
border-radius:7px;padding:5px 11px 5px 9px;font-size:13px;font-weight:500;color:#17161c}\
.np-atts svg{color:#8c8a95;flex:none}\
.np-atts small{font-size:12px;font-weight:400;color:#8c8a95}\
.np-foot{margin-top:32px;padding-top:12px;border-top:1px solid #ebe9ee;font-size:12px;\
color:#8c8a95;break-inside:avoid}";

/// Inside each email's shadow root: what the reading view gives an email that brings no
/// styling of its own (`webview::DEFAULTS`), at zero specificity so the email's own rules
/// win, and nothing that changes how its tables size their columns.
const MAIL: &str = ":host{display:block;color:#17161c;overflow-wrap:break-word;\
font:14px/1.5 -apple-system,BlinkMacSystemFont,'Helvetica Neue',Arial,sans-serif;\
color-scheme:light only}\
:where(pre,code,tt){font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:0.92em}";

#[cfg(test)]
mod tests {
    use super::*;
    use nimbra_core::rich::Span;

    fn header(name: &str, address: &str) -> Header {
        Header {
            from: Person {
                name: name.into(),
                address: address.into(),
            },
            to: vec![Person {
                name: "Jonas Weber".into(),
                address: "jonas@example.com".into(),
            }],
            ..Header::default()
        }
    }

    fn page<'a>(scope: Scope, messages: Vec<Message<'a>>) -> Page<'a> {
        Page {
            job: 7,
            subject: "Q4 <roadmap>",
            scope,
            place: (1, messages.len()),
            messages,
            printed: "Printed from Nimbra".into(),
            images: false,
        }
    }

    fn plain(text: &str) -> Block {
        Block {
            kind: Kind::Paragraph,
            quote: 0,
            text: text.into(),
            spans: Vec::new(),
        }
    }

    #[test]
    fn a_conversation_has_every_message_under_a_compact_header() {
        let (a, b) = (
            header("Maya", "maya@example.com"),
            header("", "ben@example.com"),
        );
        let text = [plain("Hi <both>")];
        let html = "<p>Notes</p>";
        let page = page(
            Scope::Conversation,
            vec![
                Message {
                    header: &a,
                    sender: "Maya".into(),
                    initials: "M".into(),
                    tone: (0x9b8cff, 0x5b47e0),
                    date: "Tue".into(),
                    body: Body::Blocks(&text),
                },
                Message {
                    header: &b,
                    sender: "ben".into(),
                    initials: "B".into(),
                    tone: (0, 0),
                    date: "Wed".into(),
                    body: Body::Html(html),
                },
            ],
        );
        let doc = document(&page);
        assert!(doc.contains("Conversation · 2 messages"));
        assert_eq!(doc.matches("<article class=\"np-msg\">").count(), 2);
        assert!(doc.contains("Q4 &lt;roadmap&gt;"));
        assert!(doc.contains("Hi &lt;both&gt;"));
        assert!(doc.contains("Maya, ben@example.com"), "participants");
        assert!(doc.contains("linear-gradient(135deg,#9b8cff,#5b47e0)"));
        assert!(doc.contains("<meta name=\"nimbra-print\" content=\"7\">"));
        // Its HTML in a shadow root of its own, full width.
        assert!(doc.contains(
            "np-body np-wide\"><div class=\"np-mail\"><template shadowrootmode=\"open\">"
        ));
        assert!(doc.contains("<p>Notes</p></template>"));
    }

    #[test]
    fn one_message_has_its_full_headers() {
        let mut h = header("Maya", "maya@example.com");
        h.cc = vec![Person {
            name: String::new(),
            address: "alex@example.com".into(),
        }];
        let text = [plain("Hello")];
        let mut page = page(
            Scope::Message,
            vec![Message {
                header: &h,
                sender: "Maya".into(),
                initials: "M".into(),
                tone: (0, 0),
                date: "Tue, 24 Nov 2026".into(),
                body: Body::Blocks(&text),
            }],
        );
        page.place = (2, 3);
        let doc = document(&page);
        assert!(doc.contains("Message 2 of 3"));
        assert!(doc.contains("<dt>From</dt><dd>Maya &lt;maya@example.com&gt;</dd>"));
        assert!(doc.contains("<dt>To</dt><dd>Jonas Weber &lt;jonas@example.com&gt;</dd>"));
        assert!(doc.contains("<dt>Cc</dt><dd>alex@example.com</dd>"));
        assert!(!doc.contains("<dt>Reply-To</dt>"));
        assert!(!doc.contains("<article"));
    }

    #[test]
    fn remote_images_follow_the_reading_view() {
        let mut p = page(Scope::Message, vec![]);
        assert!(document(&p).contains("img-src data:;"));
        p.images = true;
        assert!(document(&p).contains("img-src data: https:;"));
        let remote = Block {
            kind: Kind::Image {
                src: "https://example.com/a.png".into(),
                alt: "Chart".into(),
            },
            ..plain("")
        };
        assert!(text(std::slice::from_ref(&remote), false).contains("[Chart]"));
        assert!(text(&[remote], true).contains("<img src=\"https://example.com/a.png\""));
    }

    #[test]
    fn an_email_cannot_leave_its_template() {
        let out = contained(
            "<style>p{}</style></TEMPLATE ><h1>x</h1><template><b></b></template>\
             <PlainText>rest<link rel=preconnect href=https://t.example>",
        );
        assert!(!out.to_ascii_lowercase().contains("template"));
        assert!(!out.to_ascii_lowercase().contains("plaintext"));
        assert!(out.contains("</nimbra-x ><h1>x</h1><nimbra-x hidden><b></b></nimbra-x>"));
        assert!(out.contains("<pre>rest<meta rel=preconnect"));
        // Words that only start the same are left alone.
        assert_eq!(contained("<templates><linked>"), "<templates><linked>");
    }

    #[test]
    fn reader_text_keeps_its_styles_quotes_and_lists() {
        let mut styled = plain("Read the docs\nnow");
        styled.spans = vec![
            Span {
                range: 0..4,
                bold: true,
                italic: false,
                code: false,
                link: None,
            },
            Span {
                range: 9..13,
                bold: false,
                italic: false,
                code: false,
                link: Some("https://example.com/?a=1&b=2".into()),
            },
            // Not on a character boundary or out of order: left plain.
            Span {
                range: 2..3,
                bold: true,
                italic: false,
                code: false,
                link: None,
            },
        ];
        let mut quoted = plain("earlier");
        quoted.quote = 2;
        let item = Block {
            kind: Kind::Item {
                marker: "•".into(),
                depth: 1,
            },
            ..plain("one")
        };
        let image = Block {
            kind: Kind::Image {
                src: "cid:logo".into(),
                alt: "Logo".into(),
            },
            ..plain("")
        };
        let out = text(&[styled, quoted, plain("after"), item, image], false);
        assert!(out.starts_with(
            "<p><strong>Read</strong> the <a href=\"https://example.com/?a=1&amp;b=2\">docs</a><br>now</p>"
        ));
        assert!(out.contains(
            "<blockquote><blockquote><p>earlier</p></blockquote></blockquote><p>after</p>"
        ));
        assert!(out.contains("padding-left:36px\"><span>•</span>one</div>"));
        assert!(out.contains("<p class=\"np-alt\">[Logo]</p>"));
    }

    #[test]
    fn ready_reads_the_pages_answer() {
        assert_eq!(ready("\"12 0\""), Some((12, false)));
        assert_eq!(ready("3 2"), Some((3, true)));
        assert_eq!(ready(""), None);
        assert_eq!(ready("\"\""), None);
    }
}

/// Writes the sample inbox's first thread as both pages, to look at them in WebKit:
/// `NIMBRA_PRINT_OUT=<folder> cargo test print_sample -- --ignored`.
#[cfg(test)]
#[test]
#[ignore]
fn print_sample() {
    let Some(dir) = std::env::var_os("NIMBRA_PRINT_OUT") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let inbox = crate::data::inbox();
    let mail = &inbox[0];
    let count = mail.earlier.len() + 1;
    let message = |index: usize, original: bool| {
        let (header, blocks, html, sender, tone, initials) = match index {
            0 => (
                &mail.header,
                &mail.body[..],
                mail.html.as_deref(),
                mail.sender.clone(),
                mail.tone,
                mail.initials.to_string(),
            ),
            k => {
                let e = &mail.earlier[k - 1];
                let (tone, initials, _) = mail.earlier_look(k - 1);
                (
                    &e.header,
                    &e.blocks[..],
                    e.html.as_deref(),
                    e.sender.clone(),
                    tone,
                    initials.to_string(),
                )
            }
        };
        Message {
            header,
            sender,
            initials,
            tone: tone.stops(),
            date: "Tue, Nov 24, 2026 at 9:12 AM".into(),
            body: match html.filter(|_| original) {
                Some(html) => Body::Html(html),
                None => Body::Blocks(blocks),
            },
        }
    };
    for (name, scope, original) in [
        ("conversation-reader", Scope::Conversation, false),
        ("conversation-original", Scope::Conversation, true),
        ("message", Scope::Message, true),
    ] {
        let chosen: Vec<usize> = match scope {
            Scope::Conversation => (1..count).rev().chain([0]).collect(),
            Scope::Message => vec![0],
        };
        let page = Page {
            job: 1,
            subject: &mail.subject,
            scope,
            place: (count - chosen[0], count),
            messages: chosen.iter().map(|&i| message(i, original)).collect(),
            printed: "Printed from Nimbra on Sat, Nov 28, 2026 at 2:32 PM".into(),
            images: false,
        };
        std::fs::write(dir.join(format!("{name}.html")), document(&page)).unwrap();
    }
}
