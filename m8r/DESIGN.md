# m8r design

Status: built, and up on hyperion at `m8r.schmit.net` ([README.md](README.md) says how to deploy
it, use it and update it).

m8r ("maintainer"): agentic repo maintainer automations using your personal development
environment.

You set up one Unix user the way you set up any computer of yours: dotfiles, agent CLIs, forge
logins, clones. m8r runs your commands and prompts there on a schedule: to triage issues, to
review PRs/MRs, to develop, or for anything else. It keeps a watchlist of the issues and PRs/MRs
that you follow, which you, your agents and Claude Duet all keep current. And it shows what the
agents report back: each day's report is a list of links.

m8r first runs on hyperion, at `m8r.schmit.net`, for `rgnets/rxg` and `rgnets/infrastructure` on
`git.rgnets.com`.

## Principles

- **Small.** m8r schedules agent runs, keeps your watchlist, and shows what the agents report.
  The agents do everything else.
- **Your environment.** A run is a command of yours, as a rule your agent CLI. It starts as the
  `m8r` user, with the tools, logins and skills that you set up in that user's home. m8r
  installs none of them.
- **Your prompts.** A schedule's prompt is yours. m8r's own parts (a system message, its skill
  and the watchlist) go where the prompt says. You can change the system message too, and the
  UI shows exactly what an agent gets.
- **The forge is the source of truth.** About an issue or a PR/MR, m8r stores only its link and
  what you say about it: its category, your priority, your note, and whether its review is
  requested. It asks the forge for the rest.
- **One way in for each kind of caller.** A person gets a session, and a program gets a token.
  The pages, the API and the MCP endpoint all do the same things through the same models.
- **Idle until you configure it.** m8r installs up, like any other service. It runs nothing
  without an active configuration.
- **A missing tool is not fatal.** m8r installs none of the tools that it runs, so any of them
  can be missing or logged out. m8r says so, carries on, and stops saying so when the tool
  works.

## How it fits together

```
 you, in a browser                      Duet, or any program
      │ an access link, then a session       │ a token
      ▼                                      ▼
 nginx          m8r.schmit.net, TLS
      │
 m8r-web        m8r-cli start: the pages, the API ──┐
      ▲         and the MCP endpoint                ├── SQLite, /var/lib/m8r/m8r.sqlite
      │                                             │
 m8r-scheduler  m8r-cli scheduler: every minute, ───┘
      │         m8r-cli task tick
      ▼
 the command    $SHELL -lc 'claude -p', in a clone, with the compiled prompt on stdin
      │         and a token of its own, for the API
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
- **hron** reads when a schedule runs, in words such as `every weekday at 06:00`, and works out
  the times. It refuses what it can't read, and says where it went wrong. english-to-cron, which
  Loco's own scheduler uses, guesses: it reads `every weekday at 6:00` as every day, and a cron
  expression as every minute.
- **rmcp**, the Rust SDK of the Model Context Protocol, serves the MCP endpoint: it speaks the
  protocol's versions, and m8r writes the tools.
- **Server-rendered pages:** Tera templates and plain forms, with no frontend build. The one
  script, on a schedule's page, fills in an operation's command. Loco's generators write the
  models, the migrations, the tasks and JSON controllers. They write no HTML, so the page
  templates are written by hand.

`loco new --name m8r --db sqlite --bg async --assets serverside` makes the app. Its starter comes
with users, password logins and mailers, which m8r removes.

## Data model

```
categories       name (unique, 255 characters at most)

watchlist_items  url (unique), category → categories ON DELETE RESTRICT,
                 priority (medium|high), note, review_requested, title, checked_at

configurations   name, active, system_message

schedules        operation (triage|review|develop|other),
                 category → categories ON DELETE RESTRICT (optional),
                 repo, hron, command, prompt

runs             schedule → schedules ON DELETE SET NULL (optional), operation, repo, command,
                 status (queued|running|ok|failed), prompt, output, error,
                 started_at, finished_at

artifacts        run → runs ON DELETE CASCADE, name, url       unique(run, name)

tokens           name (unique), digest (unique), scope

used_links       token_id (unique), expires_at

tool_errors      tool, host, message                           unique(tool, host)
```

Every table also has Loco's `id`, `created_at` and `updated_at`. Strings and booleans are
`NOT NULL`, with a default where a blank makes sense.

- **Categories** say what kind of thing a watchlist item is. Every m8r starts with two, `Issue`
  and `Pull/Merge Request`, and you can add your own. A category that an item or a schedule
  uses stays.
- **Watchlist items** are the watchlist: the issues and PRs/MRs that you follow, each by its URL
  (see [The watchlist](#the-watchlist)).
- **Configurations** hold the system message: m8r's part of every prompt. A partial unique index
  allows at most one active row. With none active, nothing runs.
- **Schedules** say when an operation runs, where, with which command and with which prompt
  (see [Schedules](#schedules)). A schedule's category says which watchlist items its prompt
  gets. With none, it gets no items.
- **Runs** are the queue and the history. A run takes its command and its prompt from its
  schedule when it starts, and keeps both, so the history outlives the schedule.
- **Artifacts** are what a run reported: a link, or files that m8r hosts (see
  [Artifacts](#artifacts)).
- **Tokens** let a program use the API and the MCP endpoint (see [Access](#access)). m8r keeps a
  token's SHA-256, and not the token.
- **Used links** are the single-use access links that were used. A row goes when its link would
  have expired anyway.
- **Tool errors** are what's wrong right now with the tools that m8r runs (see
  [Tools](#tools)). A row goes when its tool works again.

## The watchlist

The watchlist is the issues and PRs/MRs that you follow. An item is one of them, by its URL.

- **Category.** Each item is in one category. A schedule that names a category gives its agent
  that category's items: a review gets the PRs/MRs, and a triage the issues. A schedule with no
  category gives its agent no items.
- **Priority.** Medium means you care about it, and high means it's urgent.
- **m8r watches the forge.** It asks for each item's title and state every ten minutes (see
  [Tools](#tools)). `title` is a copy of the forge's, for the pages. An item whose issue or
  PR/MR closed or merged leaves the list by itself.
- **Review requests.** An item for a PR/MR has a switch, `review_requested`. It goes on when
  someone asks you to review the PR/MR. The prompts then say so, and a review run can take it.
- **Everyone keeps it.** You do in the pages. An agent does while it runs: m8r's skill tells it
  how to add an item, change one and remove one through the API, and to turn a review request
  off after its review. Duet does through the MCP endpoint, from what it reads in your chats.

## Prompts

A run's prompt is its schedule's `prompt`. It's a Tera template, which m8r compiles when the run
starts. The run's command gets the result on stdin.

A template can use these:

- `{{ system_message }}`: the active configuration's system message.
- `{{ skill }}`: m8r's skill, which says how the agent changes the watchlist while it runs.
- `{{ watchlist }}`: the watchlist items in the schedule's category, one to a line.
- `{{ category }}`: the schedule's category. Blank with none.
- `{{ artifacts_dir }}`: the directory where the run reports its artifacts.
- `{% for item in watchlist_items %}`: the same items, to write the lines yourself. Each has
  `id`, `url`, `category`, `priority`, `note` and `review_requested`.

- **The pages say so.** A schedule's page lists these under the prompt's field, and a new
  schedule's prompt starts with the first three in place.
- **The system message** is the part that every prompt shares: where the agent is, and how it
  reports. It's a field of the configuration, and a template itself, which can use all of the
  above but itself. A prompt without `{{ system_message }}` gets none.
- **The skill** is m8r's own text, the same for every run. It says what the watchlist is, gives
  the `curl` commands that list, add, change and remove an item, names the categories, and says
  when to do which. A run's environment has the address and the token that the commands use
  (see [A run](#a-run)), so the prompt holds neither.
- **The watchlist** is a line for each item, such as
  `- item 12 (Pull/Merge Request, high, review requested): https://… Ship by Friday`. The number
  is the item's `id`, which the skill's commands take. With no items, or with no category on
  the schedule, it's `(nothing)`.
- **You can see it.** A schedule's page shows its prompt compiled, with the watchlist as it is
  now, and a configuration's page shows the system message compiled, with no items. A run's
  page shows the prompt it got. m8r refuses to save a template that doesn't compile, and shows
  Tera's error.

The default system message:

```
You are running in m8r, a maintainer automation service. Nobody is here to answer questions, so
decide for yourself.

When you finish, report what you made as artifacts. For each one, make a directory in
{{ artifacts_dir }}, named for what it is. Put the artifact's files in it. If the artifact is
already online (a comment you posted, a PR/MR), put in one text file that holds only its link.
```

A new schedule's prompt starts as this, and then you say what to do:

```
{{ system_message }}

{{ skill }}

The watchlist now:
{{ watchlist }}

```

So a review schedule, with the category `Pull/Merge Request`, can add one line:

```
Use the `review-branches` skill on the PRs/MRs above whose review is requested. Post each
review, and then turn its review request off.
```

## Schedules

A schedule is an operation, a category, a clone, a time, a command and a prompt.

- **The operation** says what the schedule is for: triage, review, develop, or other. It names
  the schedule's runs in the pages, and it fills in a new schedule's command.
- **The category** says which watchlist items the prompt gets (see [Prompts](#prompts)). With
  no category, the prompt gets none.
- **The time** is in words: an [hron](https://github.com/simpllyf/hron#expression-syntax)
  expression such as `every weekday at 06:00`, `every monday, thursday at 9:00, 15:00` or
  `every 30 min from 09:00 to 17:00 on weekdays`. m8r keeps it as hron writes it back, so the
  page shows how m8r read it.
- **An interval needs no window.** hron wants one: `every 12 hours from 00:00 to 23:59`. Without
  one, m8r takes the whole day, so `every 12 hours` and `every hour` work.
- **Cron works too.** hron puts a five-field cron expression in its own words: `0 6 * * 1-5`
  becomes `every weekday at 06:00`.
- **The clock is the server's,** unless the expression names a time zone:
  `every day at 09:00 in America/New_York`.
- **The command** is the exact line that a run starts (see [A run](#a-run)). The model, the
  effort and the permissions go there.
- **The form fills the command in** when you choose the operation. For triage, review and
  develop, that's the agent's command from the settings: `settings.agent`, which production
  reads from `M8R_AGENT`; at first, `claude -p`. A command of your own stays as you wrote it.
- **Other** is the operation for anything else. It has no default command: you write it, such
  as `claude -p "Update the changelog."` or `make release-notes`.
- **The prompt** can be blank. Then the command gets nothing on stdin, which suits a command
  that says what to do itself.

## Runs

### The tick

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

1. If a configuration is active, it queues a run for each schedule whose time comes in this
   minute. It skips a schedule whose run is already waiting.
2. It checks the tools and the watchlist (see [Tools](#tools)). One tick at a time does this,
   so a forge that answers slowly doesn't pile ticks up.
3. It takes the run lock, which is a lock file. If another tick holds it, a run is in progress,
   and this tick stops here.
4. It fails any run still marked `running`: the tick that held it died.
5. It starts the oldest queued run, and waits for it to end.

- **One run at a time,** in the order they were queued. The skills fan out to subagents of their
  own, so usage limits are the bottleneck, not concurrency.
- **Like cron,** a minute that passes while the scheduler is off is skipped.
- **Run now.** Each schedule has a button that queues its run at once, to try its command and
  prompt.

### A run

1. m8r takes the run's schedule as it is now, compiles its prompt, and makes the run's artifact
   directory. The run fails here if no configuration is active, if its schedule is gone, if the
   clone isn't there, or if a template doesn't compile.
2. It starts the schedule's command through the `m8r` user's login shell, in the repo's
   directory, with the prompt on stdin: `$SHELL -lc 'claude -p'`. The login shell gives the
   command the same `PATH` and environment that you get when you become `m8r`.
3. It waits. It kills a run that passes the time limit (`settings.run_limit`, two hours), and
   whatever the run started goes with it: the command has a process group of its own.
4. It stores what the command printed, and records the artifacts. The run is `ok` if the
   command exited with 0.

- **The command** is the run's own: a copy of its schedule's, so the history keeps what ran. Any
  command that reads a prompt on stdin fits, so codex or gemini can take claude's place.
- **The API is at hand.** m8r puts two variables in the command's environment: `M8R_URL`, the
  address of m8r's pages on this host, and `M8R_TOKEN`, a token that covers the watchlist's
  items and works while the run does. The skill's commands use both.
- **m8r doesn't read what the command prints.** It goes by the exit status, and by what's in
  the artifact directory. The run's page shows the output, as plain text.
- **To look closer,** become `m8r` and resume the session in the repo's directory
  (`claude --resume`). It's in that home like any other session.

## Tools

m8r runs tools from the `m8r` user's environment, each through the login shell: the programs
that the schedules' commands start, and `gh` and `glab` to ask the forges about the watchlist.
It installs none of them, so any of them can be missing, or logged out.

- **The watchlist.** `gh` answers for github.com, and `glab` for any other host. m8r asks for an
  item's title and state every ten minutes. One that closed or merged leaves the list.
- **Errors.** A tool that fails is not fatal. m8r puts what's wrong in `tool_errors`: one row
  for a tool that isn't installed, and one for each host that a tool can't work with (no login,
  no network, no such issue). Every page shows the rows.
- **Clearing.** While a tool has a row, each tick tries it again, and the row goes as soon as
  the tool works. It also goes when nothing needs the tool any more.
- **The commands.** Each tick looks for the program that each schedule's command starts, such
  as `claude`. It looks for the one in `settings.agent` too, so setup knows before there's a
  schedule. While a program is missing, a run still starts, fails at once, and says so. m8r
  looks only when a command starts with a plain word: anything else is the shell's to read.
- **m8r's own API.** Each tick connects to the pages as a run would. If it can't, a row for
  `m8r-web` says that an agent can't change the watchlist: the pages are down, or a firewall is
  in the way.
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

Nothing answers without a session or a token. The pages want a session, and the API and the MCP
endpoint want a token.

### Sessions

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
- **No tokens in logs.** nginx keeps requests that carry a link's token out of its access log.

### Tokens

- **A token** goes in a request's header: `Authorization: Bearer m8r_…`. A session opens no
  part of the API, and a token opens no page.
- **Made on the server,** like the links: `m8r task token name:duet scope:watchlist_items`
  prints a token, one time. m8r keeps its SHA-256. `m8r task tokens` lists the tokens, and
  `m8r task revoke name:duet` ends one.
- **A scope** says what a token covers: `all`, or one or more of `watchlist_items`,
  `categories`, `schedules`, `configurations` and `runs`, with commas between them. Each is a
  part of the API, `/api/<resource>`, and a set of the MCP endpoint's tools. So Duet's token
  covers `watchlist_items` and nothing else.
- **A run's token** is a JWT that names the run. It covers `watchlist_items`, and works while
  the run is `running`. m8r stores nothing for it.
- **One guard** checks all of this, in front of everything: a path under `/api/<resource>`
  needs a token that covers the resource, and `/mcp` needs any token.

## The API

`/api` is the pages' work as JSON, for programs.

```
GET /api/watchlist_items        POST /api/watchlist_items
GET /api/watchlist_items/<id>   PATCH /api/watchlist_items/<id>   DELETE /api/watchlist_items/<id>
```

The same five are there for `categories`, `schedules` and `configurations`. Beside them:

```
POST /api/schedules/<id>/run          queue the schedule's run now
POST /api/configurations/<id>/activate
POST /api/configurations/<id>/deactivate
GET /api/runs      GET /api/runs/<id>
```

- **A watchlist item** goes in by its URL and its category's name:
  `{"url": "https://…", "category": "Issue"}`. A `POST` with a URL that's already there changes
  that item. `GET /api/watchlist_items?category=Issue` lists one category.
- **A `PATCH`** changes what it names, and keeps the rest.
- **What can't be saved** comes back as a 400 with the reason in `description`, in the words
  that the pages use: `There's no category "Epic". The categories are: Issue, Pull/Merge
  Request.`
- **The runs** are read-only. A list leaves out each run's prompt and output, and one run has
  both, with the artifacts that it reported.

## The MCP endpoint

`/mcp` is the same work as tools, for an agent that isn't one of m8r's runs: Claude Duet, or
Claude Code on another computer.

- **Streamable HTTP, with no sessions.** Each `POST` stands alone and gets JSON back, so a
  restart loses nothing.
- **The tools** are the API's actions: `list_watchlist_items`, `add_watchlist_item`,
  `update_watchlist_item`, `remove_watchlist_item`, and the like for categories, schedules,
  configurations and runs.
- **A token sees what it covers.** A client gets the tools of its token's resources, and no
  others: to a token for `watchlist_items`, there are four tools.
- **The schemas say what's there.** A tool's `category` argument lists the categories as they
  are now.
- **Hints.** Each tool says whether it only reads, and whether it removes something, which a
  client's approval step goes by.
- **Errors.** What the model got wrong (no such category, no such id) comes back as a tool
  error with the reason, so it can try again.
- **The server's instructions** say what m8r and the watchlist are, and when to add, change and
  remove an item.

## Pages

- **Report** (the home page): a day's artifacts.
- **Runs:** the queue and the history, and a run's command, prompt, output and artifacts.
- **Watchlist:** add an issue or a PR/MR by URL, choose its category, set medium or high, say
  that its review is requested, write a note, remove it. The categories are on this page too.
- **Schedules:** add, change, remove, and run now. A schedule's page has its prompt, what the
  prompt can use, and the prompt compiled.
- **Configurations:** edit the system message, see it compiled, and make one active, or none.

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
  in its command: an `--allowedTools` list, or `--dangerously-skip-permissions`. The skill's
  commands are `curl`. Put it in a schedule's command, or in `M8R_AGENT` for every new schedule.
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
  where it keeps its data, and the agent's command (`M8R_AGENT`), which a new schedule starts
  with. Both units load it.
- **The secret.** `install_m8r` writes `M8R_SECRET` to `m8r/.env` when it's missing. The file is
  ignored by git and root's alone, and both units load it too. It signs the links, the sessions
  and the runs' tokens. To end them all, delete the line, run `install`, and restart the units.
- **The `m8r` command** (`src/usr/bin/m8r`, like `ipv6`) runs `m8r-cli` as the `m8r` user, with
  the same settings and secret: `m8r task link`, `m8r task token name:duet scope:watchlist_items`.
- **nginx** serves `m8r.schmit.net` with a Let's Encrypt certificate, like the other sites, and
  passes requests to `m8r-web` on `127.0.0.1:5150`.

**The firewall** lets the `m8r` user reach the internet, the router's DNS and m8r's own pages,
and nothing else: not the LAN, the guest networks, the rXg's network, or the router's other
services. An `output` chain in [nftables.conf](../src/etc/nftables.conf) does it, and
`test/check` proves it with a process run as `m8r`:

```nft
	chain output {
		type filter hook output priority filter; policy accept;
		meta skuid "m8r" jump m8r
	}

	chain m8r {
		ct state established,related accept
		oifname "lo" meta l4proto { tcp, udp } th dport 53 accept   # dnsmasq
		oifname "lo" ip daddr 127.0.0.1 tcp dport 5150 accept       # m8r's API, for the agents
		oifname $WAN accept                                          # the internet
		limit rate 10/second burst 20 packets log prefix "m8r blocked: "
		counter reject
	}
```

It rejects rather than drops, so a blocked call fails at once, and `journalctl -k -g "m8r blocked"`
shows it. The rest of loopback is closed, so an agent can't yet use a server that it starts on
localhost.

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
- **The firewall** keeps that user off the LAN and the router's other services.
- **A session is the keys.** Whoever holds a live link or session can change the prompts and
  the schedules' commands, and so run anything as `m8r`. Keep links short-lived, or single-use.
- **So is a token for schedules.** A token whose scope has `schedules` or `all` can write a
  command, and so run anything as `m8r` too. Give a token the least that its holder needs: for
  Duet, `watchlist_items`.
- **The watchlist is text from outside.** An agent or Duet can write an item's note, and the
  note goes into the next prompts. A token for `watchlist_items` can do that and no more: it
  can't change a prompt, a command or a schedule.
- **Tokens are long-lived.** m8r keeps only their digests, and a token works until you revoke
  it. A request carries its token in a header, which nginx doesn't log.
- **Hosted artifacts** can't run scripts or send forms.
- **The site is public,** with no IP limits, and every request needs a session or a token. Keep
  Loco, rmcp and their dependencies current.

## Development

On the Mac, in `m8r/`, `cargo loco start --scheduler` runs the pages and the scheduler in one
process, with a SQLite file in the project. Install `loco` and `sea-orm-cli` with cargo first.

In development and in tests, the agent's command is `cat`, which prints the prompt back. So a
run costs nothing, and its output is its compiled prompt. The tests use stand-ins for the agent,
`gh` and `glab`, so they need no logins and no network. One stand-in agent takes the skill's
commands from its prompt and runs them against the API, so the skill's text is tested as it is.

`loco new` also wrote Loco's own skill for coding agents to `m8r/.claude/skills/loco/`. Read it
before you write Loco code.

## Build order

1. **The app** (built): `loco new`, without the starter's users, logins, mailers and worker; the
   models and their migrations; access links and sessions.
2. **The pages** (built): configurations, schedules with their times, commands and prompts, the
   watchlist and its categories, runs, the report, and hosted artifacts.
3. **Runs** (built): the `tick` task, the locks, the tools and their errors, the time limit,
   and the artifacts.
4. **Hyperion** (built, and its firewall rules pass `test/run`): `install_m8r`, the units,
   nginx, the `m8r` command, DNS.
5. **Programs** (built): tokens, the API, the MCP endpoint, and m8r's skill with a run's own
   token.
6. **On the box:** the environment, one configuration, "run now" for each schedule, and then
   Duet.

## Later

- **OAuth for the MCP endpoint.** Claude's connectors sign in with OAuth by default, and take a
  fixed `Authorization` header only where Anthropic turned that on. If Duet can't send m8r's
  token, m8r needs the MCP specification's OAuth flow: its metadata, a client registration, and
  a consent page behind a session.
- **Review requests from the forge.** m8r asks each forge which PRs/MRs wait for your review,
  and puts them on the watchlist by itself. Until then, you or Duet turn a review request on.
- **Setup in the app:** a dotfiles repo and credentials that m8r installs for you, and the
  choice of an existing database.
- **Repos as records,** in place of a path on each schedule.
- **The report by email,** with an access link.

## Risks

- **The agents decide what to report,** so the report is only as good as their judgment, and as
  the system message. Tune it when artifacts are missing, or too many.
- **The agents decide what to keep on the watchlist** too. One that removes the wrong item
  loses your note with it, and one that reviews a PR/MR and doesn't turn the request off leaves
  it for the next review run. m8r keeps no history of the watchlist.
- **A command can be anything,** and it gets a prompt on stdin. One that runs what it reads,
  such as a bare `sh`, runs the prompt.
- **Prompts post and push as you.** m8r has no switch for that: a prompt that says "post" posts.
  Start with prompts that keep their results as files.
- **Disk.** Clones, worktrees and artifacts grow on the router's root filesystem, and an agent
  cleans up only if a prompt says so. m8r doesn't watch the disk.
- **Cost.** Each run is a full agent session with subagents. Set the model and the effort in
  each schedule's command, and add schedules slowly.
- **hron and rmcp are young.** hron's first release was in February 2026, and rmcp gets a
  release about every week, with new names. `Cargo.lock` pins the versions that m8r's tests ran
  with.
- **Subscription terms.** Check that your plan allows scheduled `claude -p` runs. An
  `ANTHROPIC_API_KEY` in the `m8r` user's environment works in its place.
- **`develop` needs more than the firewall allows:** a server on localhost for tests, or a lab
  rXg. Each needs a rule of its own.
- **Duet may not take m8r's connector.** Its launch page names the Marketplace's connectors,
  and not your own. Try it early (see [Later](#later) for what's next if it can't send a token).
- **Prove these early on the box:**
  - the release build: its time and memory on the router;
  - `claude -p` under `m8r-scheduler`: the login shell's `PATH`, a login that stays valid with
    nobody there, and the skill's `curl` commands with the agent's permissions;
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
- hron: [the expressions](https://github.com/simpllyf/hron#expression-syntax),
  [the spec](https://github.com/simpllyf/hron/blob/main/spec/README.md), and
  [the Rust crate](https://docs.rs/hron)
- MCP: [the specification](https://modelcontextprotocol.io/specification/latest) and
  [rmcp, the Rust SDK](https://docs.rs/rmcp)
- Claude Code: [headless](https://code.claude.com/docs/en/headless),
  [CLI reference](https://code.claude.com/docs/en/cli-reference),
  [permission modes](https://code.claude.com/docs/en/permission-modes),
  [MCP](https://code.claude.com/docs/en/mcp)
- [Introducing Claude Duet](https://www.claude.com/resources/articles/claude-duet)
- Claude's connectors:
  [custom connectors](https://support.claude.com/en/articles/11175166-getting-started-with-custom-connectors-using-remote-mcp)
  and [building one](https://claude.com/docs/connectors/building)
