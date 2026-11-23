//! What a run's command prints, as m8r reads it: text, or Claude Code's stream.

use std::fs;

use m8r::stream::{Printed, Reader, Step, Stream};

use crate::support::Sandbox;

/// What `claude -p --output-format stream-json --verbose` prints for a short review: one
/// JSON object to a line. The agent thinks, says what it will do, uses two tools, one of
/// which is another agent, and answers.
pub const STREAM: &str = r###"{"type":"system","subtype":"init","session_id":"s1","cwd":"/clones/r","model":"claude-test","tools":["Bash","Agent"]}
{"type":"rate_limit_event","rate_limit_info":{"status":"allowed"},"session_id":"s1"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"Let me look."},{"type":"text","text":"I'll read the MR first."},{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"glab mr view 12\n  --comments","description":"Read MR 12"},"caller":{"type":"direct"}}]},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_1","type":"tool_result","content":"title: Fix it","is_error":false}]},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_2","name":"Agent","input":{"description":"Review MR 12","prompt":"Review it."}}]},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Inside the other agent."}]},"parent_tool_use_id":"toolu_2","session_id":"s1"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"## Review\n\nLooks **good**."}]},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"result","subtype":"success","is_error":false,"result":"## Review\n\nLooks **good**.","session_id":"s1","total_cost_usd":0.01}
"###;

/// What the same review prints with `--include-partial-messages` too, in the order that Claude
/// Code prints it: each part of a message comes in pieces as the agent writes it, then whole
/// in a line of its own, and then its end. A tool gives its answer while the agent writes on,
/// and the other agent's pieces are among this one's.
pub const PARTS: &str = r###"{"type":"system","subtype":"init","session_id":"s1","cwd":"/clones/r","model":"claude-test","tools":["Bash","Agent"]}
{"type":"system","subtype":"status","status":"requesting","session_id":"s1"}
{"type":"stream_event","event":{"type":"message_start","message":{"id":"msg_1","role":"assistant","content":[]}},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me look."}},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"assistant","message":{"id":"msg_1","role":"assistant","content":[{"type":"thinking","thinking":"Let me look."}]},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"stream_event","event":{"type":"content_block_stop","index":0},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"I'll read "}},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"the MR first."}},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"assistant","message":{"id":"msg_1","role":"assistant","content":[{"type":"text","text":"I'll read the MR first."}]},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"stream_event","event":{"type":"content_block_stop","index":1},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"stream_event","event":{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_1","name":"Bash","input":{}}},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"stream_event","event":{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"command\": \"glab mr view"}},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"assistant","message":{"id":"msg_1","role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"glab mr view 12\n  --comments","description":"Read MR 12"}}]},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"stream_event","event":{"type":"content_block_stop","index":2},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"stream_event","event":{"type":"content_block_start","index":3,"content_block":{"type":"tool_use","id":"toolu_2","name":"Agent","input":{}}},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_1","type":"tool_result","content":"title: Fix it","is_error":false}]},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"assistant","message":{"id":"msg_1","role":"assistant","content":[{"type":"tool_use","id":"toolu_2","name":"Agent","input":{"description":"Review MR 12","prompt":"Review it."}}]},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"stream_event","event":{"type":"content_block_stop","index":3},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"stream_event","event":{"type":"message_stop"},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},"parent_tool_use_id":"toolu_2","session_id":"s1"}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Inside the "}},"parent_tool_use_id":"toolu_2","session_id":"s1"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Inside the other agent."}]},"parent_tool_use_id":"toolu_2","session_id":"s1"}
{"type":"stream_event","event":{"type":"message_start","message":{"id":"msg_2","role":"assistant","content":[]}},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"## Rev"}},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"iew\n\nLooks **go"}},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"od**."}},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"assistant","message":{"id":"msg_2","role":"assistant","content":[{"type":"text","text":"## Review\n\nLooks **good**."}]},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"stream_event","event":{"type":"content_block_stop","index":0},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"stream_event","event":{"type":"message_stop"},"parent_tool_use_id":null,"session_id":"s1"}
{"type":"result","subtype":"success","is_error":false,"result":"## Review\n\nLooks **good**.","session_id":"s1","total_cost_usd":0.01}
"###;

/// The steps of `STREAM`, as markdown.
pub const STEPS: &str = "I'll read the MR first.\n\n\
    - **Bash** ` command: glab mr view 12 --comments, description: Read MR 12 `\n\
    - **Agent** ` description: Review MR 12, prompt: Review it. `\n\n\
    ## Review\n\nLooks **good**.";

fn stream(printed: &str) -> Stream {
    match Printed::read(printed) {
        Printed::Stream(stream) => stream,
        Printed::Text(text) => panic!("not a stream: {text}"),
    }
}

fn used(tool: &str, input: &str) -> Step {
    Step::Used {
        tool: tool.to_string(),
        input: input.to_string(),
    }
}

#[test]
fn a_stream_has_the_agents_own_steps_and_its_answer() {
    let read = stream(STREAM);

    // What it said and the tools that it used, in their order. Not what it thought, not what
    // a tool gave it, and not the steps of the agent that it started.
    assert_eq!(
        read.steps,
        [
            Step::Said("I'll read the MR first.".to_string()),
            // A tool's input is one line.
            used(
                "Bash",
                "command: glab mr view 12 --comments, description: Read MR 12"
            ),
            used("Agent", "description: Review MR 12, prompt: Review it."),
            Step::Said("## Review\n\nLooks **good**.".to_string()),
        ]
    );
    assert!(!read.earlier);
    assert_eq!(read.markdown(), STEPS);
    // The report is the answer, as `claude -p` prints it alone.
    assert_eq!(read.result.as_deref(), Some("## Review\n\nLooks **good**."));
    assert_eq!(read.report(), "## Review\n\nLooks **good**.");
}

#[test]
fn a_stream_that_has_not_ended_has_its_steps_so_far() {
    // The agent is in the middle of a line.
    let lines: Vec<&str> = STREAM.lines().collect();
    let so_far = format!("{}\n{}", lines[..3].join("\n"), &lines[3][..40]);

    let read = stream(&so_far);

    assert_eq!(read.steps.len(), 2);
    assert_eq!(read.result, None);
    // With no answer, the steps are all there is to report: for a run that was killed, too.
    assert_eq!(
        read.report(),
        "I'll read the MR first.\n\n\
         - **Bash** ` command: glab mr view 12 --comments, description: Read MR 12 `"
    );
    // The first line alone starts a stream, with nothing in it yet.
    assert_eq!(stream(lines[0]), Stream::default());
}

/// One line of a stream that says a piece of what the agent writes.
pub fn piece(text: &str) -> String {
    let delta = serde_json::json!({ "type": "text_delta", "text": text });
    let event = serde_json::json!({ "type": "content_block_delta", "index": 0, "delta": delta });
    serde_json::json!({ "type": "stream_event", "event": event, "parent_tool_use_id": null })
        .to_string()
}

/// One line of a stream that says the whole of what the agent wrote.
pub fn whole(text: &str) -> String {
    let part = serde_json::json!({ "type": "text", "text": text });
    let message = serde_json::json!({ "content": [part] });
    serde_json::json!({ "type": "assistant", "message": message, "parent_tool_use_id": null })
        .to_string()
}

#[test]
fn a_stream_in_pieces_reads_as_the_whole_one_does() {
    // The pieces of a text are the same text as the whole of it, which follows them.
    assert_eq!(stream(PARTS), stream(STREAM));
    assert_eq!(stream(PARTS).markdown(), STEPS);
}

#[test]
fn a_reader_has_what_the_agent_is_writing_as_it_writes_it() {
    let said = |text: &str| Step::Said(text.to_string());
    let lines: Vec<&str> = PARTS.lines().collect();
    let mut reader = Reader::default();
    // What it thinks is no step, and nor is the start of a text.
    for line in &lines[1..8] {
        reader.take(line);
    }
    assert_eq!(reader.stream().steps, []);
    assert_eq!(reader.writing(), None);

    // Each piece adds to the step that it's writing.
    reader.take(lines[8]);
    assert_eq!(reader.stream().steps, [said("I'll read ")]);
    assert_eq!(reader.writing(), Some(0));
    reader.take(lines[9]);
    assert_eq!(reader.stream().steps, [said("I'll read the MR first.")]);
    assert_eq!(reader.writing(), Some(0));
    // The whole of it is the same step, and the agent writes it no more.
    reader.take(lines[10]);
    assert_eq!(reader.stream().steps, [said("I'll read the MR first.")]);
    assert_eq!(reader.writing(), None);

    // A tool is a step when the whole of its input is there. The pieces of the other
    // agent's text are none of this one's.
    for line in &lines[11..25] {
        reader.take(line);
    }
    assert_eq!(reader.stream().steps.len(), 3);
    assert_eq!(reader.writing(), None);
    for line in &lines[25..28] {
        reader.take(line);
    }
    assert_eq!(reader.stream().steps[3], said("## Review\n\nLooks **go"));
    assert_eq!(reader.writing(), Some(3));

    // A text has no step until it has a word: its blank start is part of that step. One
    // with no word at all makes none.
    let start = lines[7];
    let mut reader = Reader::default();
    for line in [start, &piece("\n\n"), &piece("  ")] {
        reader.take(line);
    }
    assert_eq!((reader.stream().steps.len(), reader.writing()), (0, None));
    reader.take(&piece("Done"));
    assert_eq!(reader.stream().steps, [said("\n\n  Done")]);
    for line in [&whole("\n\n  Done."), start, &piece(" \n"), &whole(" \n")] {
        reader.take(line);
    }
    assert_eq!(reader.stream().steps, [said("\n\n  Done.")]);
    assert_eq!(reader.writing(), None);

    // The end of a long file starts in the middle of a text: the pieces from there on are a
    // step, until the whole of the text takes their place.
    let mut reader = Reader::new(true);
    reader.take(&piece("good**."));
    assert_eq!(reader.stream().steps, [said("good**.")]);
    assert!(reader.stream().earlier);
    reader.take(&whole("Looks **good**."));
    assert_eq!(reader.stream().steps, [said("Looks **good**.")]);
}

#[test]
fn what_is_not_a_stream_is_text() {
    for printed in [
        "# Review\n\nLooks good.",
        "",
        // JSON, and not Claude Code's.
        "{\"type\":\"thread.started\"}\n{\"type\":\"turn.started\"}",
        "{\"level\":\"info\"}",
    ] {
        assert_eq!(
            Printed::read(printed),
            Printed::Text(printed.to_string()),
            "{printed}"
        );
    }
}

#[test]
fn a_tools_input_cannot_break_out_of_its_line() {
    let init = STREAM.lines().next().unwrap();
    let tool = |name: &str, input: serde_json::Value| {
        let part = serde_json::json!({ "type": "tool_use", "name": name, "input": input });
        let event = serde_json::json!({ "type": "assistant", "message": { "content": [part] } });
        stream(&format!("{init}\n{event}")).markdown()
    };

    // Code marks in the input are inside longer ones.
    assert_eq!(
        tool(
            "Bash",
            serde_json::json!({ "command": "echo `date` ``x``" })
        ),
        "- **Bash** ``` command: echo `date` ``x`` ```"
    );
    // A name is a name, and no markdown of its own.
    assert_eq!(
        tool("mcp__m8r__add_watchlist_item*[x](y)", serde_json::json!({})),
        "- **mcp__m8r__add_watchlist_itemxy**"
    );
    // A long input is cut, and one that isn't a list of fields shows as it is.
    let long = tool(
        "Write",
        serde_json::json!({ "content": "word ".repeat(200) }),
    );
    let around = "- **Write** ` … `".chars().count();
    assert_eq!(long.chars().count(), around + 300);
    assert!(long.ends_with("… `"), "{long}");
    assert_eq!(
        tool("Odd", serde_json::json!(["a", 1])),
        "- **Odd** ` [\"a\",1] `"
    );
}

#[test]
fn a_long_file_is_read_from_its_end() {
    let sandbox = Sandbox::new("stream-long");
    let file = sandbox.path("run.out");
    assert_eq!(Printed::read_file(&file), None);
    fs::write(&file, "").unwrap();
    assert_eq!(Printed::read_file(&file), None);

    // A short one is read whole.
    fs::write(&file, STREAM).unwrap();
    assert_eq!(
        Printed::read_file(&file),
        Some(Printed::Stream(stream(STREAM)))
    );
    fs::write(&file, "plain\ntext\n").unwrap();
    assert_eq!(
        Printed::read_file(&file),
        Some(Printed::Text("plain\ntext\n".to_string()))
    );

    // Some 6 MB of steps: m8r reads the end, which has the latest steps and the answer.
    let lines: Vec<&str> = STREAM.lines().collect();
    let filler = "x".repeat(2000);
    let mut long = format!("{}\n", lines[0]);
    for number in 0..3000 {
        let text = format!("step {number} {filler}");
        let part = serde_json::json!({ "type": "text", "text": text });
        let event = serde_json::json!({ "type": "assistant", "message": { "content": [part] } });
        long.push_str(&format!("{event}\n"));
    }
    long.push_str(lines[7]);
    long.push('\n');
    fs::write(&file, &long).unwrap();
    let Some(Printed::Stream(read)) = Printed::read_file(&file) else {
        panic!("not a stream");
    };
    assert!(read.earlier);
    assert_eq!(read.steps.len(), 200);
    assert!(matches!(&read.steps[199], Step::Said(text) if text.starts_with("step 2999 ")));
    assert_eq!(read.report(), "## Review\n\nLooks **good**.");
    assert!(read
        .markdown()
        .starts_with("*The steps before these are left out.*\n\nstep 2800 "));

    // Long text is its end, from the start of a line.
    let text: String = (0..700_000).map(|line| format!("line {line}\n")).collect();
    fs::write(&file, &text).unwrap();
    let Some(Printed::Text(end)) = Printed::read_file(&file) else {
        panic!("not text");
    };
    assert!(
        end.len() < text.len() && end.starts_with("line "),
        "{}",
        &end[..40]
    );
    assert!(end.ends_with("line 699999\n"));
}
