//! What a run printed, as its page shows it: markdown made into HTML that can do nothing.
//!
//! An agent wrote the text, maybe after it read a hostile issue, and the page opens inside the
//! viewer's session. So the HTML in the text shows as text, a link goes to a web or a mail
//! address and nowhere else, and a picture is a link. Nothing in it runs, and the page loads
//! nothing from another site, so nobody learns that it was read.
//!
//! The markdown is GitHub's kind: tables, task lists, struck text, footnotes, notes such as
//! `> [!WARNING]`, and a bare web address as a link.

use pulldown_cmark::{
    html, CodeBlockKind, CowStr, Event, LinkType, Options, Parser, Tag, TagEnd, TextMergeStream,
};

/// A link or a picture that's open, from its start to its end.
struct Open<'a> {
    /// Whether it's a link on the page. If not, only its text shows.
    kept: bool,
    /// Where a picture goes, which shows in its place while it has no text of its own.
    shown: Option<CowStr<'a>>,
}

/// `text` as HTML, which a page puts in as it is.
#[must_use]
pub fn render(text: &str) -> String {
    // `ENABLE_GFM` is for the notes such as `> [!WARNING]`.
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_GFM;
    let mut open: Vec<Open> = Vec::new();
    let events = Parser::new_ext(text, options).flat_map(|event| match event {
        Event::Html(html) | Event::InlineHtml(html) => vec![Event::Text(html)],
        // A block of HTML keeps its lines, as code does.
        Event::Start(Tag::HtmlBlock) => {
            vec![Event::Start(Tag::CodeBlock(CodeBlockKind::Indented))]
        }
        Event::End(TagEnd::HtmlBlock) => vec![Event::End(TagEnd::CodeBlock)],
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => {
            let kept = link_type == LinkType::Email || is_safe(&dest_url);
            open.push(Open { kept, shown: None });
            let link = Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            };
            started(kept, link)
        }
        Event::Start(Tag::Image {
            dest_url,
            title,
            id,
            ..
        }) => {
            // Inside a link, a picture is that link's text.
            let kept = is_safe(&dest_url) && !open.iter().any(|around| around.kept);
            let shown = Some(dest_url.clone());
            open.push(Open { kept, shown });
            let link = Tag::Link {
                link_type: LinkType::Inline,
                dest_url,
                title,
                id,
            };
            started(kept, link)
        }
        Event::End(TagEnd::Link | TagEnd::Image) => {
            let Some(Open { kept, shown }) = open.pop() else {
                return Vec::new();
            };
            let mut events: Vec<Event> = shown.map(Event::Text).into_iter().collect();
            if kept {
                events.push(Event::End(TagEnd::Link));
            }
            events
        }
        // A footnote's name is the agent's word, and it gets to be an `id` on the page.
        Event::FootnoteReference(name) => vec![Event::FootnoteReference(footnote(&name))],
        Event::Start(Tag::FootnoteDefinition(name)) => {
            vec![Event::Start(Tag::FootnoteDefinition(footnote(&name)))]
        }
        // What a command prints isn't always markdown, so a line's end shows as one, as in a
        // comment on a forge.
        Event::SoftBreak => vec![Event::HardBreak],
        Event::Text(_) | Event::Code(_) => {
            // What's open has text of its own.
            if let Some(open) = open.last_mut() {
                open.shown = None;
            }
            vec![event]
        }
        other => vec![other],
    });
    let mut page = String::new();
    // An address or a word that came in parts is one text again, for `links` and `tables`.
    html::push_html(&mut page, tables(links(TextMergeStream::new(events))));
    page
}

/// `events`, with each web address that's bare in the text as a link to itself, as on GitHub.
/// Not inside a link, where it's that link's text, and not in a block of code.
fn links<'a>(events: impl Iterator<Item = Event<'a>>) -> impl Iterator<Item = Event<'a>> {
    // The links and the blocks of code that are open.
    let mut around = 0_usize;
    events.flat_map(move |event| match event {
        Event::Start(Tag::Link { .. } | Tag::CodeBlock(_)) => {
            around += 1;
            vec![event]
        }
        Event::End(TagEnd::Link | TagEnd::CodeBlock) => {
            around = around.saturating_sub(1);
            vec![event]
        }
        Event::Text(text) if around == 0 => addresses(text),
        other => vec![other],
    })
}

/// `text`, with each web address in it as a link to itself.
fn addresses(text: CowStr) -> Vec<Event> {
    let mut events = Vec::new();
    // How much of `text` is in `events`.
    let mut done = 0;
    for (start, _) in text.match_indices("http") {
        // One inside the address before it is part of that.
        if start < done {
            continue;
        }
        let Some(end) = address_end(&text, start) else {
            continue;
        };
        let address: CowStr = text[start..end].to_string().into();
        if start > done {
            events.push(Event::Text(text[done..start].to_string().into()));
        }
        events.push(Event::Start(Tag::Link {
            link_type: LinkType::Autolink,
            dest_url: address.clone(),
            title: "".into(),
            id: "".into(),
        }));
        events.push(Event::Text(address));
        events.push(Event::End(TagEnd::Link));
        done = end;
    }
    if done == 0 {
        return vec![Event::Text(text)];
    }
    if done < text.len() {
        events.push(Event::Text(text[done..].to_string().into()));
    }
    events
}

/// Where the web address that starts at `start` in `text` ends, if one starts there. The rules
/// are GitHub's, more or less: an address starts a word, it has no space in it, and what ends
/// the sentence or closes the brackets around it isn't part of it.
fn address_end(text: &str, start: usize) -> Option<usize> {
    let (before, tail) = text.split_at(start);
    let scheme = ["https://", "http://"]
        .into_iter()
        .find(|scheme| tail.starts_with(scheme))?;
    if before.ends_with(char::is_alphanumeric) {
        return None;
    }
    let len = tail
        .find(|c: char| c.is_whitespace() || "<>\"".contains(c))
        .unwrap_or(tail.len());
    let mut address = &tail[..len];
    while let Some(last) = address.chars().next_back() {
        let extra = match last {
            ')' => address.matches(')').count() > address.matches('(').count(),
            ']' => address.matches(']').count() > address.matches('[').count(),
            _ => "?!.,:;*_~'".contains(last),
        };
        if !extra {
            break;
        }
        address = &address[..address.len() - last.len_utf8()];
    }
    address
        .get(scheme.len()..)?
        .starts_with(char::is_alphanumeric)
        .then_some(start + address.len())
}

/// `events`, with what a table needs to show well at any width. A line in a cell breaks
/// between words and nowhere else, so each word is in a box that a style keeps on one line: a
/// browser breaks a line after a hyphen too, and no style can stop that for a cell as a whole.
/// Words may then need more width than the page has, so the table is in a box that scrolls.
fn tables<'a>(events: impl Iterator<Item = Event<'a>>) -> impl Iterator<Item = Event<'a>> {
    let mut in_cell = false;
    events.flat_map(move |event| match event {
        Event::Start(Tag::Table(_)) => vec![Event::Html("<div class=\"scroll\">".into()), event],
        Event::End(TagEnd::Table) => vec![event, Event::Html("</div>\n".into())],
        Event::Start(Tag::TableCell) => {
            in_cell = true;
            vec![event]
        }
        Event::End(TagEnd::TableCell) => {
            in_cell = false;
            vec![event]
        }
        Event::Text(text) if in_cell => words(&text),
        Event::Code(code) if in_cell => {
            let mut events = vec![Event::Html("<code>".into())];
            events.extend(words(&code));
            events.push(Event::Html("</code>".into()));
            events
        }
        other => vec![other],
    })
}

/// `text`, with each of its words in a box of its own. The spaces stay between the boxes,
/// where a line may break.
fn words<'a>(text: &str) -> Vec<Event<'a>> {
    let mut events = Vec::new();
    for piece in text.split_inclusive(char::is_whitespace) {
        let (word, space) = piece.split_at(piece.trim_end().len());
        if !word.is_empty() {
            events.push(Event::Html("<span class=\"word\">".into()));
            events.push(Event::Text(word.to_string().into()));
            events.push(Event::Html("</span>".into()));
        }
        if !space.is_empty() {
            events.push(Event::Text(space.to_string().into()));
        }
    }
    events
}

/// The name on the page of the footnote that the text calls `name`. With this start, it can't
/// be the name of something else there.
fn footnote<'a>(name: &str) -> CowStr<'a> {
    format!("footnote-{name}").into()
}

/// The start of a link that's kept. One that isn't has no start, and no end.
fn started(kept: bool, link: Tag) -> Vec<Event> {
    if kept {
        vec![Event::Start(link)]
    } else {
        Vec::new()
    }
}

/// Whether a link may go to `destination`: a web address or a mail address. Not a path on
/// this site, and not something that a browser would run.
fn is_safe(destination: &str) -> bool {
    let destination = destination.trim().to_ascii_lowercase();
    ["http://", "https://", "mailto:"]
        .iter()
        .any(|start| destination.starts_with(start))
}
