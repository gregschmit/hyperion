//! What a run printed, as its page shows it: markdown made into HTML that can do nothing.
//!
//! An agent wrote the text, maybe after it read a hostile issue, and the page opens inside the
//! viewer's session. So the HTML in the text shows as text, a link goes to a web or a mail
//! address and nowhere else, and a picture is a link. Nothing in it runs, and the page loads
//! nothing from another site, so nobody learns that it was read.

use pulldown_cmark::{html, CodeBlockKind, CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};

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
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
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
    html::push_html(&mut page, events);
    page
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
