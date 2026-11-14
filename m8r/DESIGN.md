# m8r design

Status: built. It runs on the Mac, and `install` builds and starts it in a Debian container. It
hasn't run on hyperion yet, or with a real agent (see [README.md](README.md) to deploy it).

m8r ("maintainer"): agentic repo maintainer automations using your personal development
environment.

You set up one Unix user the way you set up any computer of yours: dotfiles, agent CLIs, forge
logins, clones. m8r runs your prompts there on a schedule, to triage issues, review PRs/MRs, and
develop. It keeps a short list of the issues and PRs/MRs that you care about, and it shows what
the agents report back: each day's report is a list of links.

m8r first runs on hyperion, at `m8r.schmit.net`, for `rgnets/rxg` and `rgnets/infrastructure` on
`git.rgnets.com`.

## Principles

- **Small.** m8r schedules agent runs, keeps your priorities, and shows what the agents report.
  The agents do everything else.
- **Your environment.** A run is your agent CLI, started as the `m8r` user, with the tools,
  logins and skills that you set up in that user's home. m8r installs none of them.
- **Your prompts.** m8r adds one system message to a prompt, at the spot that the prompt marks.
  You can change that message too, and the UI shows exactly what an agent gets.
- **The forge is the source of truth.** About an issue or a PR/MR, m8r stores only its link and
  your priority. It asks the forge for the rest.
- **Idle until you configure it.** m8r installs up, like any other service. It runs nothing
  without an active configuration.
- **A missing tool is not fatal.** m8r installs none of the tools that it runs, so any of them
  can be missing or logged out. m8r says so, carries on, and stops saying so when the tool
  works.

## How it fits together

```
 you, in a browser
      │ an access link, then an 8-hour session
      ▼
 nginx          m8r.schmit.net, TLS
      │
 m8r-web        m8r-cli start: the pages ───────────┐
                                                    ├── SQLite, /var/lib/m8r/m8r.sqlite
 m8r-scheduler  m8r-cli scheduler: every minute, ───┘
      │         m8r-cli task tick
      ▼
 the agent      $SHELL -lc 'claude -p', in a clone, with the compiled prompt on stdin
      │
      ▼
 /var/lib/m8r/artifacts/<run>/    the links and files it reports, which the pages show
```

Everything runs as one user, `m8r`.

## Stack

- **Loco 1.3** (Rust), which builds one binary, `m8r-cli`. m8r uses Loco's models and migrations
  (SeaORM), views (Tera), tasks, scheduler, JWTs and cookie sessions. It uses no background
  workers and no mailers: a run happens inside a task.
- **SQLite**, one file. Loco sets it up for production: WAL, a busy timeout, and foreign keys on.
  A setup step that offers an existing database can come later: SeaORM also speaks Postgres, and
  the schema uses nothing that only SQLite has.
- **Server-rendered pages:** Tera templates and plain forms, with no JavaScript and no frontend
  build. Loco's generators write the models, the migrations, the tasks and JSON controllers.
  They write no HTML, so the page templates are written by hand.

`loco new --name m8r --db sqlite --bg async --assets serverside` makes the app. Its starter comes
with users, password logins and mailers, which m8r removes.

## Data model

```
configurations  name, active,
                system_message, triage_prompt, review_prompt, develop_prompt

schedules       operation (triage|review|develop), repo, cron

items           url (unique), priority (medium|high), note, title, checked_at

runs            operation, repo, status (queued|running|ok|failed),
                prompt, output, error, started_at, finished_at

artifacts       run → runs ON DELETE CASCADE, name, url       unique(run, name)

used_links      token_id (unique), expires_at

tool_errors     tool, host, message                           unique(tool, host)
```

Every table also has Loco's `id`, `created_at` and `updated_at`. Strings and booleans are
`NOT NULL`, with a default where a blank makes sense. One migration makes all seven.

- **Configurations** hold the prompts. A partial unique index allows at most one active row.
  With none active, nothing runs.
- **Schedules** say when an operation runs, and where. `repo` is the path of a clone in the
  `m8r` user's home. `cron` is a five-field cron expression in the server's time zone, such as
  `0 6 * * 1-5`.
- **Items** are the issues and PRs/MRs that you care about, each by its URL. Medium means you
  care about it, and high means it's urgent. Every run gets the list (see [Prompts](#prompts)).
  `title` is a copy of the forge's, for the pages. An item that closes or merges leaves the list
  by itself (see [Tools](#tools)).
- **Runs** are the queue and the history. A run keeps the prompt it got and what the agent
  printed.
- **Artifacts** are what a run reported: a link, or files that m8r hosts (see
  [Artifacts](#artifacts)).
- **Used links** are the single-use access links that were used (see [Access](#access)). A row
  goes when its link would have expired anyway.
- **Tool errors** are what's wrong right now with the tools that m8r runs (see
  [Tools](#tools)). A row goes when its tool works again.

## Prompts

A run's prompt comes from the active configuration: `triage_prompt`, `review_prompt` or
`develop_prompt`, by the run's operation. Each is a Tera template, which m8r compiles when the
run starts.

- **The system message** is m8r's own part of a prompt. It tells the agent where it is, what has
  priority, and how to report. A prompt marks the spot for it with `{{ system_message }}`. A
  prompt without the spot gets none.
- **You can change it.** The system message is a field of the configuration, and a template
  itself. A new configuration starts with the default below, and with blank prompts.
- **What a template can use:** `items` (each has `url`, `priority` and `note`; high comes first)
  and `artifacts_dir` (where this run reports).
- **You can see it.** A configuration's page shows each prompt compiled, with the current items.
  A run's page shows the prompt it got. m8r refuses to save a template that doesn't compile, and
  shows Tera's error.

The default system message:

```
You are running in m8r, a maintainer automation service. Nobody is here to answer questions, so
decide for yourself.

These issues and PRs/MRs have priority, and high means urgent. Take the ones in this repository
first, and keep following each one until it closes:
{% for item in items %}- {{ item.priority }}: {{ item.url }} {{ item.note }}
{% else %}(none)
{% endfor %}
When you finish, report what you made as artifacts. For each one, make a directory in
{{ artifacts_dir }}, named for what it is. Put the artifact's files in it. If the artifact is
already online (a comment you posted, a PR/MR), put in one text file that holds only its link.
```

A triage prompt can then be this short:

```
{{ system_message }}

Use the `triage-issues` skill on this repository, in `untriaged` mode. Take them all, post each
result as a comment, add no labels, and reuse the worktree.
```

## Runs

### Schedules and the tick

Loco's scheduler takes its jobs from the config file, and m8r's schedules are rows that you edit
in the UI. So the config has one job, which runs the `tick` task every minute, each time in a
process of its own:

```yaml
scheduler:
  jobs:
    tick:
      run: tick
      schedule: run every minute
```

A tick does this:

1. If a configuration is active, it queues a run for each schedule whose `cron` matches this
   minute. It skips a schedule that already has a run waiting (the same operation and repo).
2. It checks the tools and the items (see [Tools](#tools)). One tick at a time does this, so a
   forge that answers slowly doesn't pile ticks up.
3. It takes the run lock, which is a lock file. If another tick holds it, a run is in progress,
   and this tick stops here.
4. It fails any run still marked `running`: the tick that held it died.
5. It starts the oldest queued run, and waits for it to end.

- **One run at a time,** in the order they were queued. The skills fan out to subagents of their
  own, so usage limits are the bottleneck, not concurrency.
- **Like cron,** a minute that passes while the scheduler is off is skipped.
- **Run now.** Each schedule has a button that queues its run at once, to test a prompt.

### A run

1. m8r compiles the prompt from the active configuration, and makes the run's artifact
   directory. With no active configuration, or a blank prompt, the run fails.
2. It starts the agent command through the `m8r` user's login shell, in the repo's directory,
   with the prompt on stdin: `$SHELL -lc 'claude -p'`. The login shell gives the agent the same
   `PATH` and environment that you get when you become `m8r`.
3. It waits. It kills a run that passes the time limit (`settings.run_limit`, two hours), and
   whatever the run started goes with it: the agent has a process group of its own.
4. It stores what the agent printed, and records the artifacts. The run is `ok` if the command
   exited with 0.

- **The agent command** is a setting, `settings.agent`, which production reads from
  `M8R_AGENT`; at first, `claude -p`. It's where the model, the effort and the permissions go.
  Any command that reads a prompt on stdin fits, so codex or gemini can take claude's place
  there.
- **m8r reads nothing from the agent** but its exit status. The run's page shows what it
  printed, as plain text.
- **To look closer,** become `m8r` and resume the session in the repo's directory
  (`claude --resume`). It's in that home like any other session.

## Tools

m8r runs three tools from the `m8r` user's environment, each through the login shell: the agent
command, and `gh` and `glab` to ask the forges about the items. It installs none of them, so any
of them can be missing, or logged out.

- **Items.** `gh` answers for github.com, and `glab` for any other host. m8r asks for an item's
  title and state every ten minutes. An item that closed or merged leaves the list.
- **Errors.** A tool that fails is not fatal. m8r puts what's wrong in `tool_errors`: one row
  for a tool that isn't installed, and one for each host that a tool can't work with (no login,
  no network, no such item). Every page shows the rows.
- **Clearing.** While a tool has a row, each tick tries it again, and the row goes as soon as
  the tool works. It also goes when nothing needs the tool any more.
- **The agent.** Each tick looks for the agent command's program. While it's missing, a run
  still starts, fails at once, and says so.
- **The scheduler.** Each tick touches a file. When the file gets a few minutes old, every page
  says that the scheduler isn't running.

## Artifacts

An artifact is something that a run made and chose to report: a link, or a set of files.

- **How an agent reports.** The system message names the run's artifact directory,
  `/var/lib/m8r/artifacts/<run>/`. The agent makes one directory there for each artifact, named
  for what it is, and puts the artifact's files in it. For something that's already online (a
  comment it posted, a PR/MR), it puts in one text file that holds only the link.
- **What m8r records.** When the run ends, however it ends, m8r makes one artifact for each
  directory: its name, and its link if a link is all that it holds. A link must start with
  `https://` or `http://`. The files stay where they are. Files that an agent left loose, with
  no directory, become one artifact named `report`.
- **Hosting.** `/artifacts/<id>` opens a hosted artifact: `index.html` if there is one, else
  the only file, else a list of the files. m8r serves only what's inside the artifact's
  directory.
- **Hosted files are inert.** An agent wrote them, maybe after it read a hostile issue, and they
  open inside your session. So m8r serves them with a `Content-Security-Policy` that allows
  styles and images and nothing else: no scripts, no forms, no frames. Text files, markdown too,
  show as plain text.
- **The daily report** is the home page: the day's artifacts by run, each one a link, to the
  forge or to m8r's copy.

## Access

Every request needs a session, and the only way to get one is an access link.

- **An access link.** `m8r task link` prints `https://m8r.schmit.net/?token=…`. The token is a
  JWT: m8r signs the time when the link stops working, and stores nothing. That time is 8 hours
  away unless you say otherwise (`hours:72`). Until then the link works any number of times, in
  any browser.
- **A session.** Opening a link starts a session, and redirects to the same page without the
  token. The session is a signed cookie that the server stops accepting after 8 hours.
- **Single use.** `m8r task link once:true` makes a link that works one time. m8r puts the
  token's ID in `used_links` when the link is used, and refuses it after that. Paste such a link
  straight into a browser: an app that previews links would use it up.
- **Full access.** A session can do everything in the UI. There are no roles.
- **No revoking.** m8r keeps no list of links or sessions, so it can't end one. A new secret
  ends them all.
- **Loco does the work.** Its JWT signer makes and checks the tokens, and its session (the
  `auth_session` feature) is the cookie: `HttpOnly`, `Secure`, `SameSite=Lax`.
- **No tokens in logs.** nginx keeps requests that carry a token out of its access log.

## Pages

- **Report** (the home page): a day's artifacts.
- **Runs:** the queue and the history, and a run's prompt, output and artifacts.
- **Items:** add one by URL, set medium or high, write a note, remove.
- **Schedules:** add, change, remove, and run now.
- **Configurations:** edit the system message and the three prompts, see them compiled, and make
  one active, or none.

Every page also lists what doesn't work: a tool, or the scheduler.

Every change is a form `POST`, and a browser doesn't send a `SameSite=Lax` cookie with another
site's `POST`. So another site can't act through your session.

## Setup

m8r sets up no dotfiles, tools or logins, and doesn't know which ones you use. Its part is to
tell you what to do: the home page shows these steps until there's a configuration, and every
page lists the tools that don't work yet. Become the `m8r` user, and set up its home as you
would a new computer:

1. Install your dotfiles and tools: the agent CLIs, `gh`, `glab`.
2. Log each of them in.
3. Clone the repos to maintain.

Then, in the UI: write a configuration, make it active, add a schedule, and run it now.

- **Permissions.** Nobody is there to approve a tool call. Give the agent what your prompts need
  in the agent command: an `--allowedTools` list, or `--dangerously-skip-permissions`.
- **Credentials.** An agent can use every login in that home. Use forge tokens that cover only
  the repos to maintain, and don't put your own SSH key there.

## On hyperion

The app is in `m8r/` in this repo, and its host files are in `src/` like everything else.
[install](../install) has an `install_m8r` step.

- **The user.** `install` creates `m8r`, a normal user with a home and a login shell, before it
  does anything else: nftables.conf names the user.
- **The binary.** `install_m8r` builds `m8r/target/release/m8r-cli` as the repo's owner, with
  rustup's toolchain (Loco needs a newer Rust than Debian's), and runs the migrations.
- **Two units, one binary.** `m8r-web` runs `m8r-cli start`, and `m8r-scheduler` runs
  `m8r-cli scheduler`. Both run as `m8r`, and keep the database, the artifacts and the locks in
  `/var/lib/m8r`. `install` enables and starts both: a new m8r is up, and idle only because it
  has no configuration. The agents are children of the scheduler unit, so its `MemoryMax=`,
  `CPUQuota=` and `TasksMax=` bound them, and not the pages: this box routes the house.
- **The settings** are in [/etc/default/m8r](../src/etc/default/m8r): where m8r is reached,
  where it keeps its data, and the agent command (`M8R_AGENT`). Both units load it.
- **The secret.** `install_m8r` writes `M8R_SECRET` to `m8r/.env` when it's missing. The file is
  ignored by git and root's alone, and both units load it too. To end every link and session,
  delete the line, run `install`, and restart the units.
- **The `m8r` command** (`src/usr/bin/m8r`, like `ipv6`) runs `m8r-cli` as the `m8r` user, with
  the same settings and secret: `m8r task link`.
- **nginx** serves `m8r.schmit.net` with a Let's Encrypt certificate, like the other sites, and
  passes requests to `m8r-web` on `127.0.0.1:5150`.

**The firewall** lets the `m8r` user reach the internet and the router's DNS, and nothing else:
not the LAN, the guest networks, the rXg's network, or the router's other services. An `output`
chain in [nftables.conf](../src/etc/nftables.conf) does it, and `test/check` proves it with a
process run as `m8r`:

```nft
	chain output {
		type filter hook output priority filter; policy accept;
		meta skuid "m8r" jump m8r
	}

	chain m8r {
		ct state established,related accept
		oifname "lo" meta l4proto { tcp, udp } th dport 53 accept   # dnsmasq
		oifname $WAN accept                                          # the internet
		limit rate 10/second burst 20 packets log prefix "m8r blocked: "
		counter reject
	}
```

It rejects rather than drops, so a blocked call fails at once, and `journalctl -k -g "m8r blocked"`
shows it. Loopback is closed too, so an agent can't yet use a server that it starts on localhost.

`install` restarts nothing: after an update that rebuilt m8r, it says so, and you restart the
units. Your part of a first install: add `m8r.schmit.net` to public DNS, reload nginx and
nftables, get an access link, and set up the environment ([README.md](README.md) has the
steps).

## Security

Hyperion is also the home router, and the agents read issues and PRs/MRs that other people
wrote. Expect a prompt injection someday.

- **An agent is `m8r`.** It runs as the same user as m8r, so it can use every login in that
  home, change m8r's database, and read its secret. m8r adds no sandbox: the limits are what the
  `m8r` user can do on the box, and what its logins can do elsewhere. (m8r does keep its secret
  out of the environment that it gives the tools. That stops an accident, not an attacker.)
- **The firewall** keeps that user off the LAN and the router's own services.
- **A session is the keys.** Whoever holds a live link or session can change the prompts, and
  so run anything as `m8r`. Keep links short-lived, or single-use.
- **Hosted artifacts** can't run scripts or send forms.
- **The site is public,** with no IP limits, and every request needs a session. Keep Loco and
  its dependencies current.

## Development

On the Mac, in `m8r/`, `cargo loco start --scheduler` runs the pages and the scheduler in one
process, with a SQLite file in the project. Install `loco` and `sea-orm-cli` with cargo first.

In development and in tests, the agent command is `cat`, which prints the prompt back. So a run
costs nothing, and its output is its compiled prompt. The tests use stand-ins for the agent,
`gh` and `glab`, so they need no logins and no network.

`loco new` also wrote Loco's own skill for coding agents to `m8r/.claude/skills/loco/`. Read it
before you write Loco code.

## Build order

1. **The app** (built): `loco new`, without the starter's users, logins, mailers and worker; the
   models and their one migration; access links and sessions.
2. **The pages** (built): configurations with compiled prompts, schedules, items, runs, the
   report, and hosted artifacts.
3. **Runs** (built): the `tick` task, the locks, the tools and their errors, the time limit, and
   the artifacts.
4. **Hyperion** (written, and its firewall rules pass `test/run`): `install_m8r`, the units,
   nginx, the `m8r` command. To do on the box: `install`, DNS.
5. **On the box:** the environment, one configuration, "run now" for each operation, and then
   schedules.

## Later

- **Duet.** An MCP endpoint (`/mcp`, with the Rust SDK, `rmcp`) that lists, adds and removes
  items, so Duet can set priorities from what it reads in Google Chat. A long-lived JWT of its
  own kind lets Duet in. The notes that Duet writes go into every prompt, so they need the care
  that any text from outside needs.
- **Setup in the app:** a dotfiles repo and credentials that m8r installs for you, and the
  choice of an existing database.
- **Repos and jobs as records,** in place of a path on each schedule.
- **Other agents:** codex and gemini runs.
- **The report by email,** with an access link.

## Risks

- **The agents decide what to report,** so the report is only as good as their judgment, and as
  the system message. Tune it when artifacts are missing, or too many.
- **Prompts post and push as you.** m8r has no switch for that: a prompt that says "post" posts.
  Start with prompts that keep their results as files.
- **Disk.** Clones, worktrees and artifacts grow on the router's root filesystem, and an agent
  cleans up only if a prompt says so. m8r doesn't watch the disk.
- **Cost.** Each run is a full agent session with subagents. Set the model and the effort in the
  agent command, and add schedules slowly.
- **Subscription terms.** Check that your plan allows scheduled `claude -p` runs. An
  `ANTHROPIC_API_KEY` in the `m8r` user's environment works in its place.
- **`develop` needs more than the firewall allows:** a server on localhost for tests, or a lab
  rXg. Each needs a rule of its own.
- **Duet may not take custom connectors.** Try it early, before the MCP endpoint is built.
- **Prove these early on the box:**
  - the release build: its time and memory on the router;
  - `claude -p` under `m8r-scheduler`: the login shell's `PATH`, and a login that stays valid
    with nobody there;
  - the firewall rules, with `test/check`.

## References

- Loco: [docs](https://loco.rs/docs/), [tasks](https://loco.rs/docs/how-to/write-task/),
  [the scheduler](https://loco.rs/docs/how-to/schedule-jobs/),
  [JWTs](https://loco.rs/docs/how-to/jwt-auth/),
  [sessions](https://github.com/loco-rs/loco/blob/master/website/src/content/docs/docs/how-to/oauth-sign-in.md),
  [views](https://loco.rs/docs/how-to/render-views/),
  [SQLite in production](https://github.com/loco-rs/loco/blob/master/website/src/content/docs/docs/how-to/run-sqlite-in-production.md),
  and [its skill for coding agents](https://loco.rs/skills/loco/SKILL.md)
- [Tera](https://keats.github.io/tera/)
- Claude Code: [headless](https://code.claude.com/docs/en/headless),
  [CLI reference](https://code.claude.com/docs/en/cli-reference),
  [permission modes](https://code.claude.com/docs/en/permission-modes)
- [Introducing Claude Duet](https://www.claude.com/resources/articles/claude-duet)
- [Getting started with custom connectors](https://support.claude.com/en/articles/11175166-getting-started-with-custom-connectors-using-remote-mcp)
