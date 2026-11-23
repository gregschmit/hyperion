//! What a run's command prints, as it comes: for the run's page while the run is in progress.
//!
//! The command prints to a file (see `tasks::tick`). A follower reads what's new in the file
//! each time it's asked, and says what changed, in two ways that don't depend on each other.
//! One is the text, as it was written: of a stream, that's what the agent says, with a line
//! for each tool that it uses. The other is the steps that the text makes, which a page shows
//! as markdown. A watch asks its follower again and again, and looks at the run's row, until
//! the run is over.

use std::{
    collections::VecDeque,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    time::{Duration, Instant},
};

use loco_rs::prelude::*;

use crate::{
    models::runs,
    settings::Settings,
    stream::{self, Reader, Step, Stream, EARLIER},
};

/// How often a watch asks its follower for what's new. An agent writes a few words in that
/// time, so the page shows them as it writes them.
const POLL: Duration = Duration::from_millis(100);
/// How often a watch looks at the run's row.
const LOOK: Duration = Duration::from_secs(1);

/// One thing that changed in what a run has printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// What was said before no longer holds: all of it comes anew. A watch says this first.
    Start,
    /// More of the text, as it was written.
    Raw(String),
    /// A step that's new, or that changed: its number among the steps, the step as markdown,
    /// whether it's a tool that the agent used, and whether the agent is still writing it.
    /// Text that isn't a stream is one step.
    Step {
        id: usize,
        source: String,
        tool: bool,
        open: bool,
    },
    /// The run was waiting, and has started.
    Started,
    /// The run is over. A watch says this last.
    Over,
}

impl Change {
    /// How many bytes the change has to send.
    fn size(&self) -> usize {
        match self {
            Self::Raw(text) => text.len(),
            Self::Step { source, .. } => source.len(),
            Self::Start | Self::Started | Self::Over => 0,
        }
    }
}

/// What the file holds, and how much of it the follower has told.
#[derive(Debug)]
enum Printed {
    /// Text: all of it, and how many of its bytes were told.
    Text { text: String, told: usize },
    /// A stream: each step as it was told, and which one the agent was writing then.
    Stream {
        reader: Reader,
        told: Vec<String>,
        open: Option<usize>,
    },
}

/// Reads a run's file as it grows.
#[derive(Debug)]
pub struct Follower {
    path: PathBuf,
    /// The file, once it's there. It stays open, so its end can still be read when the tick
    /// has removed it.
    file: Option<File>,
    /// What was read and isn't whole yet: the start of a stream's line, or of a character.
    rest: Vec<u8>,
    /// Whether what comes before the first end of a line is to drop: a long file is read
    /// from its end, which starts inside a line.
    skip: bool,
    /// None until the file's start says what the file is.
    printed: Option<Printed>,
}

impl Follower {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            file: None,
            rest: Vec::new(),
            skip: false,
            printed: None,
        }
    }

    /// What changed since the last time. Nothing, while the file isn't there.
    pub fn poll(&mut self) -> Vec<Change> {
        self.read();
        self.take();
        match &mut self.printed {
            None => Vec::new(),
            Some(Printed::Text { text, told }) => {
                if text.len() == *told {
                    return Vec::new();
                }
                let raw = Change::Raw(text[*told..].to_string());
                *told = text.len();
                let step = Change::Step {
                    id: 0,
                    source: text.clone(),
                    tool: false,
                    open: true,
                };
                vec![raw, step]
            }
            Some(Printed::Stream { reader, told, open }) => {
                let stream = reader.stream();
                let writing = reader.writing();
                let changes = steps(stream, writing, told, *open).unwrap_or_else(|| {
                    // Something that was told changed in a way that no change says. So all
                    // of it again.
                    told.clear();
                    let mut all = vec![Change::Start];
                    all.extend(steps(stream, writing, told, None).unwrap_or_default());
                    all
                });
                *open = writing;
                changes
            }
        }
    }

    /// Read what's new in the file.
    fn read(&mut self) {
        if self.file.is_none() {
            let Ok(mut file) = File::open(&self.path) else {
                return;
            };
            self.start(&mut file);
            self.file = Some(file);
        }
        if let Some(file) = &mut self.file {
            let _ = file.read_to_end(&mut self.rest);
        }
    }

    /// Go to where the reading starts in a file that was just opened. A long file is read
    /// from its end, as `stream::Printed::read_file` reads it: its first line says what it
    /// is.
    fn start(&mut self, file: &mut File) {
        let size = file.metadata().map_or(0, |file| file.len());
        if size <= stream::END_BYTES {
            return;
        }
        let start = stream::text(file, stream::START_BYTES);
        let is_stream = start.lines().next().is_some_and(stream::starts_stream);
        if file
            .seek(SeekFrom::Start(size - stream::END_BYTES))
            .is_err()
        {
            return;
        }
        self.skip = true;
        self.printed = Some(if is_stream {
            Printed::stream(true)
        } else {
            Printed::text()
        });
    }

    /// Take what was read, as far as it's whole.
    fn take(&mut self) {
        if self.skip {
            let Some(end) = self.rest.iter().position(|byte| *byte == b'\n') else {
                return;
            };
            self.rest.drain(..=end);
            self.skip = false;
        }
        if self.printed.is_none() {
            self.printed = self.kind();
        }
        match &mut self.printed {
            None => {}
            Some(Printed::Stream { reader, .. }) => {
                // Up to the end of the last line. After that is a line that isn't all there.
                let Some(end) = self.rest.iter().rposition(|byte| *byte == b'\n') else {
                    return;
                };
                let lines: Vec<u8> = self.rest.drain(..=end).collect();
                for line in String::from_utf8_lossy(&lines).lines() {
                    reader.take(line);
                }
            }
            Some(Printed::Text { text, .. }) => {
                // Up to the last whole character.
                let whole = match std::str::from_utf8(&self.rest) {
                    Err(error) if error.error_len().is_none() => error.valid_up_to(),
                    _ => self.rest.len(),
                };
                let whole: Vec<u8> = self.rest.drain(..whole).collect();
                text.push_str(&String::from_utf8_lossy(&whole));
            }
        }
    }

    /// What the file is, once its start says so: a stream, if its first line starts one.
    /// That line is then taken.
    fn kind(&mut self) -> Option<Printed> {
        let Some(end) = self.rest.iter().position(|byte| *byte == b'\n') else {
            // No whole line yet. A stream's first line starts with `{`, and comes at once.
            let text = self.rest.first().is_some_and(|byte| *byte != b'{');
            return text.then(Printed::text);
        };
        if !stream::starts_stream(&String::from_utf8_lossy(&self.rest[..end])) {
            return Some(Printed::text());
        }
        self.rest.drain(..=end);
        Some(Printed::stream(false))
    }
}

impl Printed {
    fn text() -> Self {
        Self::Text {
            text: String::new(),
            told: 0,
        }
    }

    /// A stream, of which nothing is told yet. `cut`: its start was left out.
    fn stream(cut: bool) -> Self {
        Self::Stream {
            reader: Reader::new(cut),
            told: Vec::new(),
            open: None,
        }
    }
}

/// What changed in a stream's steps since `told`, which then has them as they are now.
/// `open` is the step that the agent was writing then. None when `told` has something that
/// no change can put right: it's then to tell anew.
fn steps(
    stream: &Stream,
    writing: Option<usize>,
    told: &mut Vec<String>,
    open: Option<usize>,
) -> Option<Vec<Change>> {
    // A note that steps were left out goes before the steps, and is told as one of them.
    let note = usize::from(stream.earlier);
    let mut raw = String::new();
    let mut changes = Vec::new();
    if stream.earlier && told.is_empty() {
        raw.push_str(EARLIER);
        told.push(EARLIER.to_string());
        changes.push(Change::Step {
            id: 0,
            source: EARLIER.to_string(),
            tool: false,
            open: false,
        });
    }
    if told.len() > note + stream.steps.len() {
        return None;
    }
    // A step changes only while the agent writes it. So look again at the one that it was
    // writing, and at what's new.
    let from = open.map_or(told.len() - note, |open| open.min(told.len() - note));
    for (at, step) in stream.steps.iter().enumerate().skip(from) {
        let id = at + note;
        let source = step.source();
        let is_open = writing == Some(at);
        match told.get(id) {
            // As it was told. If the agent has only stopped writing it, that's to tell.
            Some(before) if *before == source => {
                if (open == Some(at)) == is_open {
                    continue;
                }
            }
            // More of the last step: the text goes on.
            Some(before) if id + 1 == told.len() && source.starts_with(before.as_str()) => {
                raw.push_str(&source[before.len()..]);
                told[id].clone_from(&source);
            }
            Some(_) => return None,
            None => {
                if id > 0 {
                    raw.push_str(
                        at.checked_sub(1)
                            .map_or("\n\n", |before| step.gap(&stream.steps[before])),
                    );
                }
                raw.push_str(&source);
                told.push(source.clone());
            }
        }
        changes.push(Change::Step {
            id,
            source,
            tool: matches!(step, Step::Used { .. }),
            open: is_open,
        });
    }
    if !raw.is_empty() {
        changes.insert(0, Change::Raw(raw));
    }
    Some(changes)
}

/// Follows one run until it's over.
#[derive(Debug)]
pub struct Watch {
    db: DatabaseConnection,
    run_id: i64,
    follower: Follower,
    /// The changes to hand out next.
    ready: VecDeque<Change>,
    /// Whether the run was waiting when the watch last looked at its row.
    queued: bool,
    over: bool,
    /// How long to wait before the next look at the file.
    wait: Duration,
    /// When the watch last looked at the run's row.
    looked: Option<Instant>,
}

impl Watch {
    /// # Errors
    /// When m8r's settings are missing.
    pub fn new(ctx: &AppContext, run: &runs::Model) -> Result<Self> {
        let file = Settings::from_context(ctx)?.output_file(run.id);
        // A run that's over has its report in its row. Its file, if a tick left one, says
        // nothing more.
        let ready = if run.is_over() {
            VecDeque::from([Change::Start, Change::Over])
        } else {
            VecDeque::new()
        };
        Ok(Self {
            db: ctx.db.clone(),
            run_id: run.id,
            follower: Follower::new(file),
            ready,
            queued: run.status == runs::Status::Queued.as_str(),
            over: run.is_over(),
            wait: POLL,
            looked: None,
        })
    }

    /// The next change, when there is one. None after `Change::Over`.
    pub async fn next(&mut self) -> Option<Change> {
        loop {
            if let Some(change) = self.ready.pop_front() {
                return Some(change);
            }
            if self.over {
                return None;
            }
            match self.looked {
                None => self.ready.push_back(Change::Start),
                Some(_) => tokio::time::sleep(self.wait).await,
            }
            let changes = self.follower.poll();
            // What goes out takes time to send and to show. So after a lot of it, the next
            // look waits longer: a megabyte in a second, at most.
            let sent: usize = changes.iter().map(Change::size).sum();
            let sent = u64::try_from(sent).unwrap_or(u64::MAX);
            self.wait = POLL.max(Duration::from_micros(sent));
            self.ready.extend(changes);
            if self.looked.is_none_or(|at| at.elapsed() >= LOOK) {
                self.look().await;
                self.looked = Some(Instant::now());
            }
        }
    }

    /// Look at the run's row: whether the run has started, and whether it's over.
    async fn look(&mut self) {
        match runs::Model::find(&self.db, self.run_id).await {
            Ok(run) if !run.is_over() => {
                if self.queued && run.status == runs::Status::Running.as_str() {
                    self.queued = false;
                    self.ready.push_back(Change::Started);
                }
            }
            // Over. Or the run is gone, or the database doesn't say: nothing more comes.
            _ => {
                // The command may have printed more since the last look at the file.
                self.ready.extend(self.follower.poll());
                self.ready.push_back(Change::Over);
                self.over = true;
            }
        }
    }
}
