# m8r

m8r ("maintainer"): agentic repo maintainer automations using your personal development
environment.

You set up one Unix user the way you set up any computer of yours: dotfiles, agent CLIs, forge
logins, clones. m8r runs your commands and prompts there on a schedule: to triage issues, to
review PRs/MRs, to develop, or for anything else. It keeps a watchlist of the issues and PRs/MRs
that you follow, which you, your agents and Claude Duet all keep current. And it shows what
each schedule's latest run reported.

It's a [Loco](https://loco.rs) app (Rust) with a SQLite database, up on hyperion at
`m8r.schmit.net`. [AGENTS.md](AGENTS.md) points coding agents at Loco's own skill, in
`.claude/skills/loco/`.

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
 the command    $SHELL -lc 'claude -p …', in a clone, with the compiled prompt on stdin
      │         and a token of its own, for the API
      ▼
 /var/lib/m8r/artifacts/<run>/    the links and files it reports, which the pages show
```

Everything runs as one user, `m8r`.

- **Small.** m8r schedules agent runs, keeps your watchlist, and shows what the agents report.
  The agents do everything else.
- **Your environment.** A run is a command of yours, as a rule your agent CLI. It starts as the
  `m8r` user, with the tools, logins and skills that you set up in that user's home. m8r
  installs none of them, and looks for none.
- **Your prompts.** A schedule's prompt is yours. m8r's own part and your configuration's go
  where the prompt says, and the pages show exactly what an agent gets.
- **The forge is the source of truth.** About an issue or a PR/MR, m8r stores only its link and
  what you say about it: its category, your priority, your note, and whether its review is
  requested. It asks the forge for the rest.
- **One way in for each kind of caller.** A person gets a session, and a program gets a token.
- **Errors clear themselves.** What's wrong is in one list, on every page. Nobody dismisses an
  error: what raises one clears it too, when its cause is gone.
- **Idle until you configure it.** m8r installs up, like any other service. It runs nothing
  without an active configuration and a schedule.

## Deploying

On hyperion, [install](../install) does m8r's part. It:

- adds the user `m8r`, a normal user with a home and a login shell;
- builds `target/release/m8r-cli` as the clone's owner, with rustup's Rust;
- makes a secret in `m8r/.env`;
- migrates the database in `/var/lib/m8r`;
- enables and starts `m8r-web` (the pages) and `m8r-scheduler` (the runs).

m8r is then up, and unconfigured: it runs nothing until you configure it. Your part, in this
order:

1. Add `m8r.schmit.net` to public DNS.
2. Load the new site and the firewall rules that keep the `m8r` user off the LAN:

   ```sh
   sudo systemctl reload nginx
   sudo nft -c -f /etc/nftables.conf && sudo systemctl reload-or-restart nftables
   ```

3. Get an access link, and open it in a browser. It starts an 8-hour session.

   ```sh
   m8r task link
   ```

4. Set up the environment. m8r installs no tools, dotfiles or logins. Become its user, and set
   its home up as you would a new computer:

   ```sh
   sudo -iu m8r
   ```

   - Install your dotfiles and tools: the agent CLI, `gh`, `glab`.
   - Log each of them in.
   - Clone the repos to maintain.

5. In the pages: write a configuration and make it active, add a schedule, and press "Run now".
   Until there's an active configuration and a schedule, every page says that each is missing.

The agent's command starts as `claude -p --dangerously-skip-permissions`: nobody is there to
approve a tool call. Each schedule has a command of its own. To start every new schedule with
another one, set `M8R_AGENT` in [/etc/default/m8r](../src/etc/default/m8r), and restart the
units.

## Using it

### Schedules

A schedule says when an operation runs, in which clones, with which command, and with which
prompt. "Run now" queues its runs at once.

- **The operation** says what the schedule is for: triage, review, develop, or other. It names
  the schedule's runs, and it fills in a new schedule's command.
- **The repos** are clones in the home of m8r's user, as paths from there:
  `~/git/group/project`. A full path works too. Write one to a line. Each time the schedule
  runs, a run starts in each of them, side by side.
- **The times** are in words: `every weekday at 06:00`, `every 12 hours`,
  `every monday, thursday at 9:00, 15:00`
  ([hron](https://github.com/simpllyf/hron#expression-syntax) has more). A five-field cron
  expression works too, and m8r puts it in words. Times are on the server's clock, unless you
  add a time zone: `in America/New_York`. One expression can name several times, as the third
  one does. For times that don't fit one expression, write several, one to a line: the
  schedule runs at each one's times.
- **The command** is the exact line that a run starts, through the login shell of m8r's user:
  the model, the effort and the permissions go there. Any command that reads a prompt on
  stdin fits, so codex or gemini can take claude's place. For `other`, you write the command,
  such as `claude -p "Update the changelog."` or `make release-notes`.
- **The category** says which watchlist items the prompt gets. With none, it gets no items.
- **The prompt** is what the command gets on stdin (see below). It can be blank, for a command
  that says what to do itself.

### Prompts

A schedule's prompt is a [Tera](https://keats.github.io/tera/) template, which m8r compiles
when the run starts. It can use these, and the schedule's page lists them:

- `{{ m8r_prompt }}`: m8r's own part, which is in the code. It says where the agent is, how it
  reports what it made, and how it adds, changes and removes watchlist items while it runs,
  with `curl` commands for m8r's API.
- `{{ config_prompt }}`: the active configuration's prompt. That's your part of every prompt:
  what every agent should know, whatever its schedule. It's a template too, and can use all of
  these but itself.
- `{{ watchlist }}`: the watchlist items in the schedule's category, one to a line, such as
  `- item 12 (Pull/Merge Request, high, review requested): https://… Ship by Friday`. The
  number is what the API's commands take. With no items, it's `(nothing)`.
- `{{ category }}`: the schedule's category. Blank with none.
- `{{ artifacts_dir }}`: the directory where the run reports its artifacts.
- `{% for item in watchlist_items %}`: the same items, to write the lines yourself. Each has
  `id`, `url`, `category`, `priority`, `note` and `review_requested`.

A new schedule's prompt starts as this, and then you say what to do:

```
{{ m8r_prompt }}

{{ config_prompt }}

The watchlist now:
{{ watchlist }}

```

A schedule's page shows its prompt compiled, with the watchlist as it is now, and a run's page
shows the prompt it got. m8r refuses to save a template that doesn't compile, and shows Tera's
error.

**Configurations** hold `config_prompt`. One is active, or none, and then nothing runs.

### The watchlist

The watchlist is the issues and PRs/MRs that you follow, each by its URL.

- **Category.** Each item is in one. m8r starts with `Issue` and `Pull/Merge Request`, and you
  can add your own on the same page. A category that an item or a schedule uses stays.
- **Priority.** Medium means you care about it, and high means it's urgent.
- **m8r asks the forge** for each item's title and state every ten minutes: `gh` answers for
  github.com, and `glab` for any other host. An item whose issue or PR/MR closed or merged
  leaves the list by itself.
- **Review requests.** Check "review requested" on a PR/MR when someone asks you to review it.
  The prompts then say so, and the agent that reviews it unchecks it.
- **Everyone keeps it.** You do in the pages. An agent does while it runs, as `m8r_prompt`
  tells it to: an agent that sees a merged MR takes it off the list. Duet does through the MCP
  endpoint.

### Runs

The scheduler runs the `tick` task every minute, each time in a process of its own. A tick:

1. clears the scheduler's own error, and asks whether m8r is set up (see [Errors](#errors));
2. queues the runs of each schedule whose time comes in this minute, if a configuration is
   active: one run for each of the schedule's repos. It skips a repo where the schedule's run
   is already waiting;
3. asks the forges about the watchlist, and looks whether a run could reach m8r's API;
4. takes the run lock. If another tick holds it, runs are in progress, and this tick stops;
5. fails any run still marked `running`: the tick that held it died;
6. starts the oldest queued run and the other queued runs of its schedule, and waits for them
   to end.

A run:

1. takes its schedule as it is now, and compiles the prompt. It fails here if no configuration
   is active, if its schedule is gone, if its clone isn't there, or if a template doesn't
   compile;
2. starts the command through the login shell, in its clone, with the prompt on stdin. The
   login shell gives it the `PATH` and environment that you get when you become `m8r`;
3. ends when the command does. m8r kills a run that passes two hours, and whatever the run
   started goes with it;
4. keeps what the command printed, and what it reported. It's `ok` if the command exited
   with 0.

- **One schedule at a time,** in the order they were queued. A schedule's runs go together,
  one in each of its repos, so a schedule with ten repos starts ten agents at once. Two
  schedules never work in the same clone at the same time. Like cron, a minute that passes
  while the scheduler is off is skipped.
- **The API is at hand.** The command's environment has `M8R_URL`, the address of m8r's pages
  on this host, and `M8R_TOKEN`, a token for the watchlist's items that works while the run
  does. The commands in `m8r_prompt` use both.
- **The history keeps what ran:** a run's page shows its command, the prompt it got, and what
  the command printed. To look closer, become `m8r`, go to the clone, and `claude --resume`.

### The report and the artifacts

An artifact is something that a run made and chose to report: a link, or a set of files.

- **How an agent reports.** `m8r_prompt` names the run's artifact directory,
  `/var/lib/m8r/artifacts/<run>/`. The agent makes one directory there for each artifact, named
  for what it is, and puts the artifact's files in it. For something that's already online (a
  comment it posted, a PR/MR), it puts in one text file that holds only the link.
- **What m8r records.** When the run ends, however it ends, m8r makes one artifact for each
  directory. Files that an agent left loose become one artifact named `report`.
- **The report** is the home page: for each schedule and each of its repos, what its latest
  run there reported. That stays until the schedule's next run there is over. A run that
  couldn't start takes nothing's place.
- **Hosted files are inert.** An agent wrote them, maybe after it read a hostile issue, and they
  open inside your session. So m8r serves them with a `Content-Security-Policy` that allows
  styles and images and nothing else, and shows text files, markdown too, as plain text.

### Errors

What's wrong is in one list. Every page shows the open errors at the top, one at a time, and
the Errors page lists them with the ones that were cleared in the last 30 days.

Nobody clears an error by hand. Whatever raises one clears it too:

- **`schedule <id> in <repo>`:** a run of the schedule failed in that repo. It clears when the
  schedule's next run there goes well, or when the schedule drops the repo or is removed. It
  links to the run that failed.
- **`configuration`:** no configuration is active. It clears when one is.
- **`schedules`:** there's no schedule. It clears when there's one.
- **`scheduler`:** a page found no tick for a few minutes. The next tick clears it.
- **`API`:** a tick couldn't reach m8r's pages as a run would, so an agent can't change the
  watchlist. A tick that can clears it.
- **`gh`, `glab`:** the tool is missing, or it failed for a host, when m8r asked a forge about
  the watchlist. It clears when the tool works, or when no item needs it.

### Access

Nothing answers without a session or a token.

- **An access link** starts a session: `m8r task link` prints one. It works any number of
  times for 8 hours. `hours:72` changes that, and `once:true` makes a link that works one time.
  A session lasts 8 hours and can do everything in the pages.
- **No revoking.** A link is a signed token that m8r doesn't store, so it can't end one. To end
  every link and session, delete the line in `m8r/.env`, run `install`, and restart the units.
- **A token** is for a program (see below). A session opens no part of the API, and a token
  opens no page.

## The API and Duet

Programs use m8r through its API (`/api`) or its MCP endpoint (`/mcp`). Each wants a token in
the `Authorization` header.

```sh
m8r task token name:duet scope:watchlist_items   # prints the token, this one time
m8r task tokens                                  # lists them
m8r task revoke name:duet                        # ends one
```

- **A scope** says what a token covers: `all`, or one or more of `watchlist_items`,
  `categories`, `schedules`, `configurations` and `runs`, with commas between them. A token
  for `schedules` can write a command, which runs as `m8r`: give each token the least it needs.
  m8r keeps a token's SHA-256, and not the token.
- **The API** takes and gives JSON:

  ```sh
  curl https://m8r.schmit.net/api/watchlist_items -H "Authorization: Bearer $TOKEN"
  curl https://m8r.schmit.net/api/watchlist_items -H "Authorization: Bearer $TOKEN" \
    --json '{"url": "https://github.com/o/r/pull/7", "category": "Pull/Merge Request"}'
  curl -X PATCH https://m8r.schmit.net/api/watchlist_items/1 -H "Authorization: Bearer $TOKEN" \
    --json '{"priority": "high"}'
  curl -X DELETE https://m8r.schmit.net/api/watchlist_items/1 -H "Authorization: Bearer $TOKEN"
  ```

  `categories`, `schedules` and `configurations` work the same way. A `POST` of a watchlist
  item whose URL is already there changes that item, and a `PATCH` changes what it names. A
  schedule's `repos` and `hron` are arrays. `POST /api/schedules/<id>/run` queues the
  schedule's runs and answers with them, `POST /api/configurations/<id>/activate` and
  `/deactivate` do what they say, and `/api/runs` can be read. What can't be saved comes back
  as a 400 with the reason in `description`.
- **The MCP endpoint** is the same work as tools (`list_watchlist_items`,
  `add_watchlist_item`, and so on), over streamable HTTP with no sessions. A client sees the
  tools of its token's resources, and no others. Each tool says whether it only reads and
  whether it removes something, which a client's approval step goes by.
- **Claude Duet** connects to the MCP endpoint. Make it a token for `watchlist_items`: it then
  sees four tools, which list, add, change and remove watchlist items, and nothing else of
  m8r. In Claude's settings, add a custom connector with the URL `https://m8r.schmit.net/mcp`
  and the request header `Authorization: Bearer <token>`. If the form has no place for a
  request header, Duet can't send the token (see [Not built](#not-built)).
- **Claude Code** on another computer connects the same way:

  ```sh
  claude mcp add --transport http m8r https://m8r.schmit.net/mcp \
    --header "Authorization: Bearer $TOKEN"
  ```

- **A run needs no token of yours.** m8r gives each run one of its own, in `M8R_TOKEN`.

## Updating

As `gns`, in `/srv/hyperion`: `git pull && sudo ./install`. `install` restarts nothing. When it
says that it rebuilt m8r, and nothing is running, restart it:

```sh
sudo systemctl restart m8r-web m8r-scheduler
```

Don't wait long. `install` also migrates the database, and the m8r that's still running may
not know the new tables: its pages fail until the restart. A restart of `m8r-scheduler` ends
the run in progress, and the next tick marks it failed.

If the update changed the firewall or the nginx site, load them as in "Deploying", step 2. An
agent that the firewall keeps from m8r's API can't change the watchlist, and every page then
says so.

A migration can move data so that it can't be taken back. To be able to go back, copy
`/var/lib/m8r/m8r.sqlite` before `install`, with m8r stopped.

## On hyperion

The app is in `m8r/` in this repo, and its host files are in `src/` like everything else.

- **Two units, one binary.** `m8r-web` runs `m8r-cli start`, and `m8r-scheduler` runs
  `m8r-cli scheduler`. Both run as `m8r`, and keep the database, the artifacts and the locks in
  `/var/lib/m8r`. The agents are children of the scheduler unit, so its `MemoryMax=`,
  `CPUQuota=` and `TasksMax=` bound them, and not the pages: this box routes the house.
- **The settings** are in [/etc/default/m8r](../src/etc/default/m8r): where m8r is reached,
  where it keeps its data, and the agent's command (`M8R_AGENT`). Both units load it.
- **The secret,** `M8R_SECRET` in `m8r/.env`, signs the links, the sessions and the runs'
  tokens. The file is ignored by git and root's alone.
- **The `m8r` command** (`src/usr/bin/m8r`) runs `m8r-cli` as the `m8r` user, with the same
  settings and secret: `m8r task link`, `m8r task tick`, `m8r doctor`.
- **nginx** serves `m8r.schmit.net` with a Let's Encrypt certificate, and passes requests to
  `m8r-web` on `127.0.0.1:5150`. It keeps requests that carry a link's token out of its log.
- **The firewall** lets the `m8r` user reach the internet, the router's DNS and m8r's own
  pages, and nothing else: not the LAN, the guest networks, the rXg's network, or the router's
  other services. The `m8r` chain in [nftables.conf](../src/etc/nftables.conf) does it, and
  `test/check` proves it. It rejects rather than drops, so a blocked call fails at once.

## When something is wrong

- The Errors page says what m8r knows is wrong, and since when.
- `journalctl -u m8r-scheduler` shows the ticks, and `journalctl -u m8r-web` the requests.
- `journalctl -k -g "m8r blocked"` shows what the firewall turned away.
- `m8r doctor` checks the config and the database. It calls the loopback binding unsafe. Here
  that's on purpose: nginx is in front.
- `m8r task tick` does now what the scheduler does every minute.
- To stop the runs, make no configuration active, or `sudo systemctl stop m8r-scheduler`.

## Security

Hyperion is also the home router, and the agents read issues and PRs/MRs that other people
wrote. Expect a prompt injection someday.

- **An agent is `m8r`.** It runs as the same user as m8r, with no questions asked of anyone, so
  it can use every login in that home, change m8r's database, and read its secret. m8r adds no
  sandbox: the limits are what the `m8r` user can do on the box, and what its logins can do
  elsewhere. Use forge tokens that cover only the repos to maintain, and don't put your own
  SSH key there.
- **The firewall** keeps that user off the LAN and the router's other services.
- **A session is the keys.** Whoever holds a live link or session can change the prompts and
  the schedules' commands, and so run anything as `m8r`. So can a token whose scope has
  `schedules`. Keep links short-lived, and give a token the least that its holder needs.
- **The watchlist is text from outside.** An agent or Duet can write an item's note, and the
  note goes into the next prompts. A token for `watchlist_items` can do that and no more.
- **The site is public,** with no IP limits, and every request needs a session or a token. Keep
  Loco, rmcp and their dependencies current.

Know these too, before you rely on it:

- **The agents decide what to report, and what to keep on the watchlist.** One that removes the
  wrong item loses your note with it. m8r keeps no history of the watchlist.
- **Prompts post and push as you.** m8r has no switch for that: a prompt that says "post"
  posts. Start with prompts that keep their results as files.
- **Disk and cost.** Clones, worktrees and artifacts grow on the router's root filesystem, and
  each run is a full agent session. Set the model and the effort in each schedule's command.
- **Subscription terms.** Check that your plan allows scheduled `claude -p` runs. An
  `ANTHROPIC_API_KEY` in the `m8r` user's environment works in its place.
- **`develop` needs more than the firewall allows:** a server on localhost for tests, or a lab
  rXg. Each needs a rule of its own.

## Development

```sh
cargo install loco sea-orm-cli   # once: the generators need them
cargo loco start --scheduler     # the pages and the scheduler, at http://localhost:5150
cargo loco task link             # an access link for them
```

In development a new schedule's command is `cat`, which prints the prompt back, so a run costs
nothing. For a real agent, write its command on the schedule, or start with
`M8R_AGENT='claude -p' cargo loco start --scheduler`. For the API and the MCP endpoint,
`cargo loco task token name:me scope:all` prints a token. The database
(`m8r_development.sqlite`) and the data (`data/`) are in this directory, and git ignores them.

The scheduler runs your own tools here: it asks the forges about the watchlist with your `gh`
and `glab` logins. Leave `--scheduler` out to keep it from that.

Before a change is done:

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

The tests use stand-ins for the agent, `gh` and `glab`, so they need no logins and no network.
One stand-in agent runs the commands of `m8r_prompt` against the API on loopback, with `curl`
(7.82 or newer, for `--json`). New models, controllers and tasks start from
`cargo loco generate`, which also wires them in.

- **Loco 1.3** builds one binary, `m8r-cli`. m8r uses its models and migrations (SeaORM), views
  (Tera), tasks, scheduler, JWTs and cookie sessions. A run happens inside a task, so there are
  no background workers.
- **SQLite**, one file: WAL, a busy timeout, and foreign keys on.
- **A migration** runs in one transaction (`use_transaction`), so one that fails changes
  nothing. When it reads a column, it names the table too: a connection of the pool may know an
  older schema, and SQLite takes a bare quoted name that it doesn't know for text.
- **`migration/schema.sql`** is what the migrations make, in one place to read. In development,
  the migrator writes it each time it runs: `cargo loco db migrate` does, and so does a start.
  The schema is always SQLite's, from a new database in memory, so it's the same whatever
  database m8r uses. A test fails when the file is behind the migrations.
- **hron** reads the schedules' times. It refuses what it can't read, and says where it went
  wrong. english-to-cron, which Loco's own scheduler uses, guesses.
- **rmcp**, the Rust SDK of the Model Context Protocol, serves the MCP endpoint. It and hron
  are young: `Cargo.lock` pins the versions that the tests ran with.
- **The pages** are Tera templates and plain forms, with no frontend build, and two small
  scripts: one fills in an operation's command, and one gives the errors their turns.

`migration/schema.sql` has the schema itself. In short, what the database holds, beside Loco's
`id`, `created_at` and `updated_at`:

```
categories       name (unique, 255 characters at most)

watchlist_items  url (unique), category → categories ON DELETE RESTRICT,
                 priority (medium|high), note, review_requested, title, checked_at

configurations   name, active (one at most), prompt

schedules        operation (triage|review|develop|other),
                 category → categories ON DELETE RESTRICT (optional),
                 repos and hron (each a list, one to a line), command, prompt

runs             schedule → schedules ON DELETE SET NULL (optional), operation, repo, command,
                 status (queued|running|ok|failed), prompt, output, error,
                 started_at, finished_at

artifacts        run → runs ON DELETE CASCADE, name, url       unique(run, name)

errors           subject (one open error at most), message,
                 run → runs ON DELETE SET NULL (optional), cleared_at

tokens           name (unique), digest (unique), scope

used_links       token_id (unique), expires_at
```

## Not built

- **OAuth for the MCP endpoint.** Claude's connectors sign in with OAuth by default, and take a
  fixed `Authorization` header only where Anthropic turned that on (see
  [its connector docs](https://claude.com/docs/connectors/building)). If Duet can't send m8r's
  token, m8r needs the MCP specification's OAuth flow.
- **Review requests from the forge.** m8r could ask each forge which PRs/MRs wait for your
  review, and put them on the watchlist by itself. Until then, you or Duet turn a review
  request on.
- **Setup in the app:** a dotfiles repo and credentials that m8r installs for you, and the
  choice of an existing database.
- **The report by email,** with an access link.
