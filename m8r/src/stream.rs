//! What a run's command has printed so far, for the run's page to show while the run is in
//! progress, and for the run's report when it ends.
//!
//! Most commands print text. Claude Code with `--output-format stream-json --verbose` prints a
//! stream: one JSON object to a line, for each thing that the agent says and does as it does
//! it, and at the end its result, which is the answer that `claude -p` prints alone without
//! that option. m8r reads a stream, so that the page shows the agent's steps as they come,
//! and so that the report is the answer and not the lines of JSON.
//!
//! With `--include-partial-messages` too, the stream has what the agent says in parts, a few
//! words to a line, as it writes them. Then the whole of it follows in a line of its own,
//! which takes the place of the parts. So a page can show the agent as it writes.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use serde_json::Value;

/// How much m8r reads of a file: its end. A stream has everything that a tool gave the agent
/// too, so it gets long.
pub(crate) const END_BYTES: u64 = 4 * 1024 * 1024;
/// How much m8r reads of a long file's start, to see what the file is. A stream's first line
/// names every tool that the agent has.
pub(crate) const START_BYTES: u64 = 1024 * 1024;
/// What stands for the steps that m8r left out, before the ones that it kept.
pub(crate) const EARLIER: &str = "*The steps before these are left out.*";
/// How many of a stream's steps m8r keeps: the latest ones.
const STEPS: usize = 200;
/// How much of a tool's input a step shows.
const INPUT_CHARS: usize = 300;

/// What a command has printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Printed {
    /// Text: the end of it, if it's long.
    Text(String),
    Stream(Stream),
}

/// One thing that an agent did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// What it said, which is markdown.
    Said(String),
    /// A tool that it used, and the tool's input in one line.
    Used { tool: String, input: String },
}

/// What a stream says so far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stream {
    /// The agent's own steps, the oldest first. Those of the agents that it started for a
    /// part of its work aren't here: its own step says that it started them.
    pub steps: Vec<Step>,
    /// Whether there were steps before these, which m8r left out.
    pub earlier: bool,
    /// The agent's answer, once it has ended.
    pub result: Option<String>,
}

impl Printed {
    /// Read what a command printed: a stream, if its first line starts one.
    #[must_use]
    pub fn read(printed: &str) -> Self {
        let mut lines = printed.lines();
        if !lines.next().is_some_and(starts_stream) {
            return Self::Text(printed.to_string());
        }
        Self::Stream(Stream::of(lines, false))
    }

    /// Read the file that a run's command prints to: all of it, or of a long one, its end.
    /// None when the file isn't there, or has nothing yet.
    #[must_use]
    pub fn read_file(path: &Path) -> Option<Self> {
        let mut file = File::open(path).ok()?;
        let size = file.metadata().ok()?.len();
        if size == 0 {
            return None;
        }
        if size <= END_BYTES {
            return Some(Self::read(&text(&mut file, size)));
        }
        // The first line says what the file is. Then the end, from the start of a line.
        let start = text(&mut file, START_BYTES);
        let stream = start.lines().next().is_some_and(starts_stream);
        file.seek(SeekFrom::Start(size - END_BYTES)).ok()?;
        let end = text(&mut file, END_BYTES);
        let end = end.split_once('\n').map_or("", |(_, whole)| whole);
        Some(if stream {
            Self::Stream(Stream::of(end.lines(), true))
        } else {
            Self::Text(end.to_string())
        })
    }
}

/// The next `bytes` of the file, as text.
pub(crate) fn text(file: &mut File, bytes: u64) -> String {
    let mut read = Vec::new();
    let _ = file.take(bytes).read_to_end(&mut read);
    String::from_utf8_lossy(&read).into_owned()
}

/// Whether this line starts a stream of Claude Code's.
pub(crate) fn starts_stream(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .is_ok_and(|event| event["type"] == "system" && event["subtype"] == "init")
}

/// Reads a stream a line at a time, as the lines come.
#[derive(Debug, Default)]
pub struct Reader {
    stream: Stream,
    /// The step that the agent is writing, whose text comes in parts.
    writing: Option<usize>,
    /// The blank start of a text that has no word yet. A step starts with its first word.
    blank: String,
}

impl Reader {
    /// A reader of a stream's end: `cut` says that lines before it were left out.
    #[must_use]
    pub fn new(cut: bool) -> Self {
        Self {
            stream: Stream {
                earlier: cut,
                ..Stream::default()
            },
            ..Self::default()
        }
    }

    /// What the lines so far say.
    #[must_use]
    pub fn stream(&self) -> &Stream {
        &self.stream
    }

    /// Which of the stream's steps the agent is writing now, if it's writing one.
    #[must_use]
    pub fn writing(&self) -> Option<usize> {
        self.writing
    }

    /// Take what one line says, after the stream's first one. A line that isn't JSON says
    /// nothing: the last one may be only half there, while the agent prints it.
    pub fn take(&mut self, line: &str) {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return;
        };
        // An agent that this one started has a parent. Its steps aren't this one's.
        let own = event["parent_tool_use_id"].is_null();
        match event["type"].as_str() {
            Some("stream_event") if own => self.part(&event["event"]),
            Some("assistant") if own => {
                let parts = event["message"]["content"].as_array();
                for part in parts.into_iter().flatten() {
                    self.whole(part);
                }
            }
            Some("result") => {
                let answer = event["result"].as_str().filter(|text| !text.is_empty());
                self.stream.result = answer.map(str::to_string);
            }
            _ => {}
        }
    }

    /// Take a part of a message, which comes while the agent writes the message.
    fn part(&mut self, part: &Value) {
        match part["type"].as_str() {
            // What comes now is no more of the text before.
            Some("content_block_start") => {
                self.writing = None;
                self.blank.clear();
            }
            Some("content_block_delta") if part["delta"]["type"] == "text_delta" => {
                self.write(part["delta"]["text"].as_str().unwrap_or_default());
            }
            _ => {}
        }
    }

    /// Add `text` to what the agent is writing.
    fn write(&mut self, text: &str) {
        if let Some(Step::Said(said)) = self.writing.and_then(|at| self.stream.steps.get_mut(at)) {
            said.push_str(text);
            return;
        }
        self.blank.push_str(text);
        if self.blank.trim().is_empty() {
            return;
        }
        let said = std::mem::take(&mut self.blank);
        self.writing = Some(self.stream.steps.len());
        self.stream.steps.push(Step::Said(said));
    }

    /// Take one whole part of a message. A text that came in parts first is here again, all
    /// of it: it takes the place of the step that the parts made.
    fn whole(&mut self, part: &Value) {
        let step = step(part);
        if part["type"] != "text" {
            self.stream.steps.extend(step);
            return;
        }
        self.blank.clear();
        match (self.writing.take(), step) {
            (Some(at), Some(step)) => self.stream.steps[at] = step,
            (Some(at), None) => drop(self.stream.steps.remove(at)),
            (None, step) => self.stream.steps.extend(step),
        }
    }
}

impl Step {
    /// The step as markdown: what the agent said, as it said it, or the tool that it used,
    /// in a line of a list.
    #[must_use]
    pub fn source(&self) -> String {
        match self {
            Self::Said(text) => text.clone(),
            Self::Used { tool, input } => format!("- **{tool}** {}", code(input))
                .trim_end()
                .to_string(),
        }
    }

    /// What goes between the step before this one, `before`, and this one, in markdown.
    /// The tools that follow each other are one list.
    #[must_use]
    pub fn gap(&self, before: &Self) -> &'static str {
        match (before, self) {
            (Self::Used { .. }, Self::Used { .. }) => "\n",
            _ => "\n\n",
        }
    }
}

impl Stream {
    /// The stream that these lines are, after the first one. `cut` says that lines before
    /// them were left out.
    fn of<'a>(lines: impl Iterator<Item = &'a str>, cut: bool) -> Self {
        let mut reader = Reader::new(cut);
        for line in lines {
            reader.take(line);
        }
        let mut stream = reader.stream;
        if stream.steps.len() > STEPS {
            stream.steps.drain(..stream.steps.len() - STEPS);
            stream.earlier = true;
        }
        stream
    }

    /// The steps as markdown: what the agent said as it said it, and each tool that it used
    /// in a line of a list.
    #[must_use]
    pub fn markdown(&self) -> String {
        let mut markdown = String::new();
        if self.earlier {
            markdown.push_str(EARLIER);
        }
        let mut before = None;
        for step in &self.steps {
            let gap = before.map_or("\n\n", |before| step.gap(before));
            if !markdown.is_empty() {
                markdown.push_str(gap);
            }
            markdown.push_str(step.source().trim());
            before = Some(step);
        }
        markdown
    }

    /// The run's report: the agent's answer. For an agent that ended with none, such as one
    /// that m8r killed, the steps that it got to.
    #[must_use]
    pub fn report(&self) -> String {
        self.result.clone().unwrap_or_else(|| self.markdown())
    }
}

/// The step that one part of an agent's message is, if it's one to show. What it thought is
/// not.
fn step(part: &Value) -> Option<Step> {
    match part["type"].as_str()? {
        "text" => {
            let text = part["text"].as_str()?;
            (!text.trim().is_empty()).then(|| Step::Said(text.to_string()))
        }
        "tool_use" => Some(Step::Used {
            // A name is letters, digits and a few marks. Anything else could be markdown.
            tool: part["name"]
                .as_str()?
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || "_-.:".contains(*c))
                .collect(),
            input: one_line(&part["input"]),
        }),
        _ => None,
    }
}

/// A tool's input in one line: each value with its name, and as much of that as fits.
fn one_line(input: &Value) -> String {
    let words = |value: &Value| match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let text = match input {
        Value::Object(fields) => {
            let fields = fields
                .iter()
                .map(|(name, value)| format!("{name}: {}", words(value)));
            fields.collect::<Vec<_>>().join(", ")
        }
        other => words(other),
    };
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= INPUT_CHARS {
        return line;
    }
    let start: String = line.chars().take(INPUT_CHARS).collect();
    format!("{start}…")
}

/// `text` as code in a line of markdown, whatever is in it: the marks around it are longer
/// than any run of them inside.
fn code(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let longest = text
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    let marks = "`".repeat(longest + 1);
    format!("{marks} {text} {marks}")
}
