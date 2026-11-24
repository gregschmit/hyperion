//! What a run printed, as its page shows it: markdown made into HTML that can do nothing.
//!
//! An agent wrote the text, maybe after it read a hostile issue, and the page opens inside the
//! viewer's session. So the HTML in the text shows as text, a link goes to a web or a mail
//! address and nowhere else, and a picture is a link. Nothing in it runs, and the page loads
//! nothing from another site, so nobody learns that it was read. A link to a web address
//! opens in a new window, which gets nothing of this one.
//!
//! The markdown is GitHub's kind: tables, task lists, struck text, footnotes, notes such as
//! `> [!WARNING]`, and a bare web address as a link. One thing in it is HTML, as on GitHub: a
//! part that folds, which is `<details>` with a `<summary>`. Those tags get to the page as
//! m8r writes them, with nothing of the text in them (see `folds`).

use pulldown_cmark::{
    html, CodeBlockKind, CowStr, Event, LinkType, Options, Parser, Tag, TagEnd, TextMergeStream,
};

/// What the start of a link to a web address ends with. `noreferrer` keeps where the reader
/// came from, from the site that the link goes to.
const NEW_WINDOW: &str = " target=\"_blank\" rel=\"noopener noreferrer\">";

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
    let events = folds(Parser::new_ext(text, options)).flat_map(|event| match event {
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
    let events = tables(links(TextMergeStream::new(events)));
    html::push_html(&mut page, new_windows(events));
    page
}

/// How far from its `<` a tag of a fold ends, at most. The longest is `<details open>`.
const TAG_BYTES: usize = 24;

/// A tag of a part that folds.
#[derive(Clone, Copy)]
enum FoldTag {
    /// `<details>`, or `<details open>`.
    Details {
        open: bool,
    },
    DetailsEnd,
    Summary,
    SummaryEnd,
}

/// What a fold has of its first line, which opens and closes it.
#[derive(PartialEq, Eq)]
enum Summary {
    None,
    Open,
    Closed,
}

/// A fold that's open.
struct Fold {
    /// How many parts of the text were open around its start. It ends there, and no deeper.
    level: usize,
    summary: Summary,
}

/// The folds that are open, and where the text is.
#[derive(Default)]
struct Folds {
    /// How many parts of the text are open: quotes, lists and so on.
    level: usize,
    open: Vec<Fold>,
}

impl Folds {
    /// What `tag` puts on the page. None when the tag has no place here: an end with no
    /// start in this part of the text, or a second first line.
    fn take(&mut self, tag: FoldTag) -> Option<String> {
        let level = self.level;
        let here = self.open.last_mut().filter(|fold| fold.level == level);
        match (tag, here) {
            (FoldTag::Details { open }, _) => {
                self.open.push(Fold {
                    level,
                    summary: Summary::None,
                });
                let start = if open { "<details open>" } else { "<details>" };
                Some(format!("{start}\n"))
            }
            (FoldTag::DetailsEnd, Some(_)) => Some(self.end()),
            (FoldTag::Summary, Some(fold)) if fold.summary == Summary::None => {
                fold.summary = Summary::Open;
                Some("<summary>".to_string())
            }
            (FoldTag::SummaryEnd, Some(fold)) if fold.summary == Summary::Open => {
                fold.summary = Summary::Closed;
                Some("</summary>\n".to_string())
            }
            _ => None,
        }
    }

    /// What ends the last fold, with its first line if that's still open.
    fn end(&mut self) -> String {
        let summary = self.open.pop().map(|fold| fold.summary);
        if summary == Some(Summary::Open) {
            return "</summary>\n</details>\n".to_string();
        }
        "</details>\n".to_string()
    }

    /// What ends the folds that started in the part of the text that ends now: a fold that
    /// the text leaves open ends with the part that it's in.
    fn part_ended<'a>(&mut self) -> Vec<Event<'a>> {
        let mut events = Vec::new();
        while self
            .open
            .last()
            .is_some_and(|fold| fold.level >= self.level)
        {
            events.push(Event::Html(self.end().into()));
        }
        self.level = self.level.saturating_sub(1);
        events
    }

    /// A block of HTML, as the page shows it. With no tag of a fold that has a place here,
    /// that's the block as text, with its lines, as code has them.
    fn block<'a>(&mut self, html: &str) -> Vec<Event<'a>> {
        let mut events = Vec::new();
        let mut kept = false;
        // What's between the tags, which is text.
        let mut text = String::new();
        let mut rest = html;
        // Only a block that starts with such a tag can have one: in a comment or a script,
        // `<details>` is a part of that.
        let mut at = fold_tag(html.trim_start()).and(rest.find('<'));
        while let Some(start) = at {
            text.push_str(&rest[..start]);
            rest = &rest[start..];
            // The text before a tag is in what was open before it.
            let first_line = self.in_first_line();
            let tag = fold_tag(rest).and_then(|(tag, len)| Some((self.take(tag)?, len)));
            let len = match tag {
                Some((tag, len)) => {
                    lines(&mut events, &text, first_line);
                    text.clear();
                    events.push(Event::Html(tag.into()));
                    kept = true;
                    len
                }
                None => {
                    text.push('<');
                    1
                }
            };
            rest = &rest[len..];
            at = rest.find('<');
        }
        if !kept {
            return vec![
                Event::Start(Tag::CodeBlock(CodeBlockKind::Indented)),
                Event::Text(html.to_string().into()),
                Event::End(TagEnd::CodeBlock),
            ];
        }
        text.push_str(rest);
        lines(&mut events, &text, self.in_first_line());
        events
    }

    /// Whether the text is in a fold's first line now.
    fn in_first_line(&self) -> bool {
        let fold = self.open.last();
        fold.is_some_and(|fold| fold.summary == Summary::Open)
    }
}

/// The tag of a fold that `html` starts with, and how long it is. Only the tag as it is, or
/// `<details open>`: one that says more is text, as all other HTML is.
fn fold_tag(html: &str) -> Option<(FoldTag, usize)> {
    let inside = html.strip_prefix('<')?;
    // Not far, so a text that's full of `<` is read once.
    let end = inside
        .bytes()
        .take(TAG_BYTES)
        .position(|byte| byte == b'>')?;
    let tag = match inside[..end].trim_end().to_ascii_lowercase().as_str() {
        "details" => FoldTag::Details { open: false },
        "/details" => FoldTag::DetailsEnd,
        "summary" => FoldTag::Summary,
        "/summary" => FoldTag::SummaryEnd,
        other => {
            let (name, says) = other.split_once(char::is_whitespace)?;
            if name != "details" || says.trim_start() != "open" {
                return None;
            }
            FoldTag::Details { open: true }
        }
    };
    // The `<` and the `>` too.
    Some((tag, end + 2))
}

/// Add `text` to `events`, with its lines as lines. Nothing, if it's only space. It's a
/// paragraph, but not in a fold's first line, which is one line beside the fold's mark.
fn lines<'a>(events: &mut Vec<Event<'a>>, text: &str, first_line: bool) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    if !first_line {
        events.push(Event::Start(Tag::Paragraph));
    }
    for (at, line) in text.lines().enumerate() {
        if at > 0 {
            events.push(Event::HardBreak);
        }
        events.push(Event::Text(line.trim_end().to_string().into()));
    }
    if !first_line {
        events.push(Event::End(TagEnd::Paragraph));
    }
}

/// `events`, with the HTML in them as text, but for the parts that fold. This is the only
/// place where HTML that the text has gets to the page, and all that gets there is five tags
/// that m8r writes: `<details>`, `<details open>`, `<summary>` and the ends of the two. They
/// run nothing and load nothing.
///
/// A tag counts where GitHub's markdown reads it as a block: at the start of a line. A fold
/// ends in the part of the text where it started (a quote, an item of a list), or with that
/// part, or with the text. So the page's HTML is whole, whatever the text has.
fn folds<'a>(events: impl Iterator<Item = Event<'a>>) -> impl Iterator<Item = Event<'a>> {
    let mut folds = Folds::default();
    // The block of HTML that's open, as far as it came.
    let mut block: Option<String> = None;
    // After the last event comes None: the text's end, which ends the folds that are open.
    let events = events.map(Some).chain(std::iter::once(None));
    events.flat_map(move |event| match event {
        None => folds.part_ended(),
        Some(Event::Start(Tag::HtmlBlock)) => {
            block = Some(String::new());
            Vec::new()
        }
        Some(Event::Html(html) | Event::InlineHtml(html)) => match &mut block {
            Some(block) => {
                block.push_str(&html);
                Vec::new()
            }
            None => vec![Event::Text(html)],
        },
        Some(Event::End(TagEnd::HtmlBlock)) => folds.block(&block.take().unwrap_or_default()),
        Some(event @ Event::Start(_)) => {
            folds.level += 1;
            vec![event]
        }
        Some(event @ Event::End(_)) => {
            let mut events = folds.part_ended();
            events.push(event);
            events
        }
        Some(other) => vec![other],
    })
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

/// `events`, with each link to a web address as one that opens in a new window. The run's
/// page stays open that way, and one of a run in progress keeps up with the run. A mail
/// address opens no page, so its link stays as it is.
fn new_windows<'a>(events: impl Iterator<Item = Event<'a>>) -> impl Iterator<Item = Event<'a>> {
    events.map(|event| match event {
        Event::Start(Tag::Link { ref dest_url, .. }) if is_web(dest_url) => {
            // pulldown-cmark writes the start, so the address is escaped as in any link.
            let mut start = String::new();
            html::push_html(&mut start, std::iter::once(event));
            // Its `>`, which `NEW_WINDOW` has.
            start.pop();
            Event::Html(format!("{start}{NEW_WINDOW}").into())
        }
        other => other,
    })
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
    is_web(destination) || starts(destination, "mailto:")
}

/// Whether `destination` is a web address.
fn is_web(destination: &str) -> bool {
    starts(destination, "http://") || starts(destination, "https://")
}

/// Whether `destination` starts with `start`, as a browser reads it.
fn starts(destination: &str, start: &str) -> bool {
    destination.trim().to_ascii_lowercase().starts_with(start)
}
