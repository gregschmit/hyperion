# m8r

m8r ("maintainer"): agentic repo maintainer automations using your personal development
environment.

You set up one Unix user the way you set up any computer of yours: dotfiles, agent CLIs, forge
logins, clones. m8r runs your commands and prompts there on a schedule, in the repos that you
name: to triage issues, to review PRs/MRs, to develop, or for anything else. It keeps a
watchlist of the issues and PRs/MRs that you follow, which you, your agents and Claude Duet
all keep current, and gives each run the items of its own repo. And it lists the runs, with
what each one printed.

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
 its report     what it prints, which the run's page shows
```

Everything runs as one user, `m8r`.

- **Small.** m8r schedules agent runs, keeps your watchlist, and shows what the agents report.
  The agents do everything else.
- **Your environment.** A run is a command of yours, as a rule your agent CLI. It starts as the
  `m8r` user, with the tools, logins and skills that you set up in that user's home. m8r
  installs none of them, and looks for none.
- **Your prompts.** A schedule's prompt is yours. m8r's own part and the watchlist go where
  the prompt says, and the pages show exactly what an agent gets.
- **m8r knows where, and the prompt says what.** m8r knows the repos, which items are in
  which, and which run gets which, so an agent starts in one repo with that repo's items.
  What to do with them is the prompt's to say.
- **The forge is the source of truth.** About an issue or a PR/MR, m8r stores only its link and
  what you say about it: your priority, your note, and whether someone asked you for it. The
  link says which repo it's in, and whether it's an issue or a PR/MR. m8r asks the forge for
  the rest.
- **One way in.** A person gets a session, and a program gets a token. Either opens the pages
  and the API by the same rules. A share link shows one run to someone who has neither.
- **Errors clear themselves.** What's wrong is in one list, on every page. Nobody dismisses an
  error: what raises one clears it too, when its cause is gone.
- **Idle until it has a schedule.** m8r installs up, like any other service. It runs nothing
  until you add a schedule.

## Deploying

On hyperion, [install](../install) does m8r's part. It:

- adds the user `m8r`, a normal user with a home and a login shell;
- builds `target/release/m8r-cli` as the clone's owner, with rustup's Rust;
- makes a secret in `m8r/.env`;
- migrates the database in `/var/lib/m8r`;
- enables and starts `m8r-web` (the pages) and `m8r-scheduler` (the runs).

m8r is then up, with nothing to do: it runs nothing until it has a schedule. Your part, in
this order:

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

5. In the pages: add the repos, add a schedule, and press "Run now". Until there's a schedule,
   every page says that it's missing.

The agent's command starts as `claude -p --dangerously-skip-permissions`: nobody is there to
approve a tool call. Each schedule has a command of its own. To start every new schedule with
another one, set `M8R_AGENT` in [/etc/default/m8r](../src/etc/default/m8r), and restart the
units.

## Using it

### Repos

A repo is a clone in the home of m8r's user, and the project on its forge. m8r works in the
repos that it has: a schedule runs in some of them, and a watchlist item is in one.

- **The path** is where the clone is, from that home: `~/git/group/project`. A full path works
  too. Clone the repo as `m8r` first: m8r adds a repo only for a directory that's there.
- **The URL** is the project's, on GitHub or GitLab: `https://git.example.com/group/project`.
  Leave it blank, and m8r reads it from the clone's `origin`. Write it yourself when the
  clone's origin isn't the project that you follow, as with a fork.
- **A repo with no URL** takes no watchlist items. A schedule can still run in it.
- **A repo that's in use stays:** take its items off the watchlist and its schedules off it
  before you remove it. Its runs stay in the history.

### Schedules

A schedule says when a command runs, in which repos, and with which prompt. "Run now" queues
its runs at once.

- **The name** says what the schedule is for, in your words: `nightly review`. It names the
  schedule's runs.
- **The repos** are the ones that the command starts in. Each time the schedule runs, a run
  starts in each of them, side by side.
- **The category** says which watchlist items a run's prompt gets: `all` the items of the
  run's repo, its `issues`, its `pull/merge requests`, or none. A run never gets another
  repo's items.
- **Max items** cuts that list: a run gets the first ones, and no more than this many. The
  requested items come first, then the most pressing ones: urgent, high, then medium. 0 means
  all of them.
- **When** is the schedule's times, in words: `every weekday at 06:00`, `every 12 hours`,
  `every monday, thursday at 9:00, 15:00`
  ([hron](https://github.com/simpllyf/hron#expression-syntax) has more). A five-field cron
  expression works too, and m8r puts it in words. Times are on the server's clock, unless you
  add a time zone: `in America/New_York`. One expression can name several times, as the third
  one does. For times that don't fit one expression, write several, one to a line: the
  schedule runs at each one's times.
- **Enabled** says whether the schedule runs at its times. Turn it off to stop a schedule for
  a while: it then starts nothing by itself, and its runs that wait are dropped. A run in
  progress goes on, and "Run now" still works.
- **The command** is the exact line that a run starts, through the login shell of m8r's user:
  the model, the effort and the permissions go there. It starts as the agent's command. Any
  command that reads a prompt on stdin fits, so codex or gemini can take claude's place, and
  so does one that needs no prompt, such as `make release-notes`.
- **The prompt** is what the command gets on stdin (see below). It can be blank, for a command
  that says what to do itself.

### Prompts

A schedule's prompt is a [Tera](https://keats.github.io/tera/) template, which m8r compiles
when the run starts. It can use these, and the schedule's page lists them:

- `{{ m8r_prompt }}`: m8r's own part, which is in the code. It says where the agent is, that
  what it prints is its report, with the links to what it made, and how it adds, changes and
  removes watchlist items while it runs, with `curl` commands for m8r's API.
- `{{ watchlist }}`: the watchlist items that the run gets, one to a line, such as
  `- item 12 (high, requested): https://… Ship by Friday`. They're the items of the run's
  repo, of the schedule's category, in the order above, and no more than its max items. An
  item's number is what the API's commands take. With no items, it's `(nothing)`.
- `{{ requested_watchlist }}`: only the requested ones of those items, the same way. They're
  in `watchlist` too, at its top.
- `{{ repo }}` and `{{ repo_url }}`: the run's repo, as the path of its clone and as the URL of
  its project. The URL is blank if m8r doesn't know it.
- `{{ category }}`: the schedule's category: `all`, `issues` or `pull/merge requests`. Blank
  with none.
- `{{ max_items }}`: the schedule's max items. It's 0 when a run gets all of them, so a prompt
  can say `{% if max_items %}Do no more than {{ max_items }} of them.{% endif %}`.
- `{% for item in watchlist_items %}` and `{% for item in requested_watchlist_items %}`: the
  same two lists, to write the lines yourself. Each item has `id`, `url`, `category`,
  `priority`, `note` and `requested`.

A new schedule's prompt starts as this, and then you say what to do:

```
{{ m8r_prompt }}

The watchlist now:
{{ watchlist }}

```

A schedule's page shows its prompt compiled as a run in the first of its repos would get it,
with the watchlist as it is now, and a run's page shows the prompt it got. m8r refuses to save
a prompt that doesn't compile, and shows Tera's error.

### The watchlist

The watchlist is the issues and PRs/MRs that you follow, each by its URL. The page shows all
of it, or a category's or a repo's part. The most pressing items are at the top: urgent, high,
then medium. In each of those, the requested items come first. Then the list goes repo by
repo, with a repo's issues before its PRs/MRs, by their numbers. The API and the MCP tools
list the items in the same order.

- **The URL says where and what.** An item is in the repo whose project its URL names, and
  it's an issue or a pull/merge request. You say neither. m8r takes an item only for a repo
  that it has: add the repo first.
- **Priority.** Medium means you care about it, high means it matters more, and urgent means
  it can't wait. An item is medium until someone says more.
- **Requested.** Check it when someone asks you for an item: for a PR/MR, a review. A run gets
  the requested items first, so a schedule that takes only some items takes those. The agent
  that does what was asked unchecks it.
- **The note** says why you follow the item. It's one line, as a prompt has a line for each
  item: m8r makes one line of a note that has more.
- **m8r asks the forge** for each item's title and state every ten minutes: `gh` answers for
  github.com, and `glab` for any other host. An item whose issue or PR/MR closed or merged
  leaves the list by itself.
- **Everyone keeps it.** You do in the pages. An agent does while it runs, as `m8r_prompt`
  tells it to: an agent that sees a merged MR takes it off the list. Duet does through the MCP
  endpoint.

### Runs

The scheduler runs the `tick` task every minute, each time in a process of its own. A tick:

1. clears the scheduler's own error, asks whether m8r is set up, and looks at the repos (see
   [Errors](#errors));
2. queues the runs of each schedule whose time comes in this minute: one run for each of the
   schedule's repos. It skips a repo where the schedule's run is already waiting;
3. asks the forges about the watchlist, and looks whether a run could reach m8r's API;
4. takes the run lock. If another tick holds it, runs are in progress, and this tick stops;
5. fails any run still marked `running`: the tick that held it died;
6. starts the oldest queued run and the other queued runs of its schedule, and waits for them
   to end.

A run:

1. takes its schedule and its repo as they are now, and compiles the prompt with that repo's
   items. It fails here if its schedule or its repo is gone, if its clone isn't there, or if
   the prompt doesn't compile;
2. starts the command through the login shell, in its clone, with the prompt on stdin. The
   login shell gives it the `PATH` and environment that you get when you become `m8r`. It
   gets nothing of m8r's own environment: not its secret, and not a setting such as
   `DATABASE_URL`, which a tool in a repo would take for its own;
3. ends when the command does. m8r kills a run that passes two hours, and whatever the run
   started goes with it;
4. keeps what the command printed. It's `done` if the command exited with 0, and `failed`
   if not. `done` says that the command ended well, and not that its work is good.

- **One schedule at a time,** in the order they were queued. A schedule's runs go together,
  one in each of its repos, so a schedule with ten repos starts ten agents at once. Two
  schedules never work in the same clone at the same time. Like cron, a minute that passes
  while the scheduler is off is skipped.
- **The API is at hand.** The command's environment has `M8R_URL`, the address of m8r's pages
  on this host, and `M8R_TOKEN`, a token for the watchlist's items that works while the run
  does. The commands in `m8r_prompt` use both.
- **What a run prints is its report.** m8r keeps that, and no files. `m8r_prompt` tells the
  agent to put the link to what it made online, such as a comment or a PR/MR, in its answer.
- **A stream is read.** With `--output-format stream-json --verbose` in a schedule's command,
  `claude -p` prints a stream: a line of JSON for each thing that the agent says and does, as
  it does it, and at the end its answer. m8r knows that stream by its first line. The run's
  report is then the answer, as `claude -p` prints it alone, and not the lines of JSON. A run
  that ends with no answer, such as one that m8r killed, leaves the steps that it got to. Any
  other output is text: m8r reads no other tool's stream. With `--include-partial-messages`
  too, the stream has what the agent says a few words at a time, as it writes them, and the
  run's page shows the agent writing:

  ```
  claude -p --dangerously-skip-permissions --output-format stream-json --verbose --include-partial-messages
  ```
- **The list says when, and for how long:** `2026-11-23 06:00 – 06:14 (14 min)`, with the day
  once unless the run ended on another day. It says when a run was created only if the run
  then waited two minutes or more: a tick starts a run within a minute, so that one waited
  for another schedule's runs.
- **The history keeps what ran:** a run's page shows its schedule's name, its command, the
  prompt it got, and what the command printed. That shows as markdown of GitHub's kind
  (tables, task lists, footnotes, notes such as `> [!WARNING]`, and a bare web address as a
  link), and a tab away as it was printed, to copy. A link in it opens in a new window, so
  the run's page stays open. A part of it can fold, as on GitHub: `<details>` with a
  `<summary>`, each at the start of a line. Its first line shows, and a click opens the
  rest. `<details open>` starts open. To look closer, become `m8r`, go to the clone, and resume the
  agent's session: `claude --resume <id>`. Claude Code leaves a `-p` session out of the list
  that `claude --resume` shows, so take the id from the name of the session's file in
  `~/.claude/projects/`.
- **A run in progress can be watched.** What its command prints goes to a file as it comes,
  and into the run's row when the run ends. While the run is in progress, m8r sends its page
  what's new in the file some ten times a second, over one connection that stays open
  (`GET /api/runs/<id>/output`, server-sent events). The page has two tabs for it, and m8r
  sends each on its own. "Raw" is the text as it was written: of a stream, what the agent
  says, and a line for each tool that it uses. "Markdown" is the same as the agent's steps,
  which m8r renders: what it said, and each tool that it used, in one line. Not what a tool
  gave it, and not the steps of the agents that it started. The page adds to what it shows
  and never loads again, so it stays where you scrolled to. A part that folds is open while
  the agent writes its step. When the run ends, its report takes the steps' place. A share link shows the same. In a terminal, `m8r task runs` lists
  the runs that aren't over, and `m8r task output id:12` prints what run 12 has printed, as
  it was printed, and goes on until the run ends. The page and the terminal show only what
  the command prints: `claude -p` prints its answer, when it's done, unless its command
  asks for a stream.
- **A run can be shared.** "Share" on a run's page shows a link to that page, and copies it.
  For 72 hours, the link opens the page to anyone who has it (see [Access](#access)).

### Errors

What's wrong is in one list. Every page shows the open errors at the top, one at a time, and
the Errors page lists them with the ones that were cleared in the last 30 days.

Nobody clears an error by hand. Whatever raises one clears it too:

- **`schedule <id> in <repo>`:** a run of the schedule failed in that repo. It clears when the
  schedule's next run there goes well, or when the schedule drops the repo or is removed. It
  links to the run that failed.
- **`repo <path>`:** the repo's clone isn't there, or its `origin` names a project that
  another repo has. A tick raises it, and clears it when it's put right.
- **`schedules`:** there's no schedule. It clears when there's one.
- **`scheduler`:** a page found no tick for a few minutes. The next tick clears it.
- **`API`:** a tick couldn't reach m8r's pages as a run would, so an agent can't change the
  watchlist. A tick that can clears it.
- **`gh`, `glab`:** the tool is missing, or it failed for a host, when m8r asked a forge about
  the watchlist. It clears when the tool works, or when no item needs it.

### Access

Nothing answers without a session, a token or a share link. One guard decides for every path,
and the rules are the same for a page and for the API.

- **An access link** starts a session: `m8r task link` prints one. It works any number of
  times for 8 hours. `hours:72` changes that, and `once:true` makes a link that works one time.
  A session lasts 8 hours and covers everything: the pages, and the API.
- **A token** is for a program (see below). It covers its scope: the API's part of each
  resource that the scope names, and that resource's pages too. `/schedules` and
  `/api/schedules` are both `schedules`. Only `all` covers the errors, which tell of every
  part of m8r: a page shows them to no other token.
- **A share link** makes a guest of whoever opens it, for one run and for 72 hours: "Share"
  on the run's page makes one. A guest reads what that page shows: the run's command, the
  prompt it got and what it printed, also as the API has them. The link starts no session,
  so only the link itself opens the run. A guest gets no other run and nothing else of m8r,
  and changes nothing.
- **A change on a session must come from m8r's own pages.** A browser sends a session's
  cookie with every request to m8r, whichever site's page makes the request. So m8r looks at
  where a request comes from, as the browser says it (`Sec-Fetch-Site`, or `Origin`), before
  a session changes anything. A token needs none of this: it's in a header, which no other
  site can make a browser send. With `curl` and a session's cookie, send
  `-H 'Sec-Fetch-Site: same-origin'`.
- **No revoking.** A link is a signed token that m8r doesn't store, so it can't end one. To end
  every link and session, delete the line in `m8r/.env`, run `install`, and restart the units.

## The API and Duet

Programs use m8r through its API (`/api`) or its MCP endpoint (`/mcp`), with a token in the
`Authorization` header. A page's script uses the API too, with the page's session. The MCP
endpoint takes a token and nothing else.

```sh
m8r task token name:duet scope:watchlist_items,repos,runs   # prints the token, this one time
m8r task tokens                                             # lists them
m8r task revoke name:duet                                   # ends one
```

- **A scope** says what a token covers: `all`, or one or more of `watchlist_items`, `repos`,
  `schedules` and `runs`, with commas between them. A token for `schedules` can write a
  command, which runs as `m8r`: give each token the least it needs. m8r keeps a token's
  SHA-256, and not the token.
- **The API** takes and gives JSON:

  ```sh
  curl https://m8r.schmit.net/api/watchlist_items -H "Authorization: Bearer $TOKEN"
  curl https://m8r.schmit.net/api/watchlist_items -H "Authorization: Bearer $TOKEN" \
    --json '{"url": "https://github.com/o/r/pull/7", "requested": true}'
  curl -X PATCH https://m8r.schmit.net/api/watchlist_items/1 -H "Authorization: Bearer $TOKEN" \
    --json '{"priority": "high"}'
  curl -X DELETE https://m8r.schmit.net/api/watchlist_items/1 -H "Authorization: Bearer $TOKEN"
  ```

  `repos` and `schedules` work the same way. A column goes by its own name. A `POST` of a
  watchlist item whose URL is already there changes that item, and a `PATCH` changes what it
  names. An item's answer has its `repo` and its `category`, which its URL says, and
  `?category=issues` and `?repo_id=1` narrow the list. A repo has a `path` and a `url`. A
  schedule's `repos` and `when` are arrays: the repos by the paths of their clones. Its
  `enabled` is true or false, and its `next` is null while it's false.
  `POST /api/schedules/<id>/run` queues the schedule's runs and answers with them, and
  `/api/runs` can be read. `GET /api/runs/<id>/output` sends what a run prints as it comes,
  as server-sent events, until the run is over: `start`, then `raw` (more of the text) and
  `step` (a step as HTML, with its `id`, and `open` while the agent writes it), `started`
  when a run that waited starts, and `over`. What can't be saved comes back as a 400 with
  the reason in `description`.
- **The MCP endpoint** keeps the watchlist, as tools: `list_watchlist_items`,
  `add_watchlist_item`, `update_watchlist_item` and `remove_watchlist_item`. Three more tools
  only read: `list_repos` shows the repos that m8r works in, and `list_runs` and `get_run`
  show the runs and what they printed. No tool changes a repo, a schedule or a run: the pages
  and the API do that. It speaks streamable HTTP with no sessions. A client sees the tools of
  its token's resources, and no others. Each tool says whether it only reads and whether it
  removes something, which a client's approval step goes by.
- **Claude Duet** connects to the MCP endpoint. Make it a token for
  `watchlist_items,repos,runs`: it then keeps the watchlist, and reads the repos and the runs.
  With `watchlist_items` alone, it sees the watchlist's four tools and nothing else of m8r. A
  scope counts for the API too, where `repos` also changes the repos: Duet has only the tools.
  In Claude's settings, add a custom connector with the URL `https://m8r.schmit.net/mcp` and
  the request header `Authorization: Bearer <token>`. If the form has no place for a request
  header, Duet can't send the token (see [Not built](#not-built)).
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
  `m8r-cli scheduler`. Both run as `m8r`, and keep the database and the locks in
  `/var/lib/m8r`. The agents are children of the scheduler unit. It sets them no limits: the
  system shares the box's memory and CPU between them and everything else. To see how much
  memory they took at most since the unit started: `systemctl show m8r-scheduler -p MemoryPeak`.
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
- `m8r task runs` lists the runs that aren't over, and `m8r task output id:12` prints what one
  prints, as it comes.
- `systemctl status m8r-scheduler` shows the processes of the runs in progress: the agents are
  the unit's children.
- To stop one schedule's runs, disable the schedule. To stop them all:
  `sudo systemctl stop m8r-scheduler`.

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
- **Another site can't use your session.** Its cookie is `SameSite=Lax`, and m8r also refuses
  a change on a session unless the request comes from one of m8r's own pages. The second
  rule covers the other hosts of `schmit.net`, which a browser counts as the same site.
- **The watchlist is text from outside.** An agent or Duet can write an item's note, and the
  note goes into the next prompts. A token for `watchlist_items` can do that and no more.
- **So is what a run printed.** An agent wrote it, maybe after it read a hostile issue. Its
  page shows it as markdown, with the HTML in it as text and each picture as a link, so
  nothing in it runs, and the page loads nothing from another site. The one HTML that counts
  is a part that folds, and m8r writes its tags itself: `<details>`, `<details open>` and
  `<summary>`, with nothing of the text in them. Duet reads it too, if its
  token covers `runs`.
- **A share link shows its run to whoever holds it,** for 72 hours. The prompt on the run's
  page has the watchlist items that the run got, with your notes. The link opens nothing
  else, and changes nothing.
- **The site is public,** with no IP limits, and every request needs a session, a token or a
  share link. Keep Loco, rmcp and their dependencies current.

Know these too, before you rely on it:

- **The agents decide what to report, and what to keep on the watchlist.** One that removes the
  wrong item loses your note with it. m8r keeps no history of the watchlist.
- **Prompts post and push as you.** m8r has no switch for that: a prompt that says "post"
  posts. Start with prompts that only print their results.
- **Disk and cost.** Clones and worktrees grow on the router's root filesystem, and each run
  is a full agent session. Set the model and the effort in each schedule's command.
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
- **pulldown-cmark** reads the markdown that a run printed, for its page. Before that's made
  into HTML, m8r makes text of the HTML in it and of the links that aren't web or mail
  addresses, and links of its pictures (see `views::markdown`). Of the HTML, only the tags
  of a part that folds get to the page, and m8r ends each part that the text leaves open, so
  the page's HTML is whole. m8r also makes a link of each
  bare web address, which pulldown-cmark leaves as text, makes each link to a web address
  open in a new window, and puts `footnote-` before a footnote's name, so that it can't be
  the name of something else on the page. In a table, a line breaks between words and never
  inside one, and a table that's too wide for the page that way scrolls sideways.
- **The pages** are Tera templates and plain forms, with no frontend build, and three small
  scripts: one gives the errors their turns, one copies a run's share link, and one shows
  what a run prints as it comes. That one reads server-sent events, which a browser's
  `EventSource` gets over plain HTTP and connects to again by itself. m8r only sends there,
  so it needs no WebSocket. The markdown is rendered on the server, where the rules that keep
  it harmless are tested: the script puts each step's HTML in its place. A page asks for the
  stylesheet at an address that changes when the stylesheet does, so a browser never uses an
  old one for a new page.
- **A name is the column's.** What a page calls a field is the name of its column, and so is
  what the API and the MCP tools call it: `max_items` is "Max items" on a schedule's page. To
  rename one, rename them all.

`migration/schema.sql` has the schema itself. In short, what the database holds, beside Loco's
`id`, `created_at` and `updated_at`:

```
repos            path (unique), url (blank, or one repo's)

watchlist_items  url (unique), repo → repos ON DELETE RESTRICT,
                 category (issues|pull/merge requests), priority (medium|high|urgent),
                 note, requested, title, checked_at

schedules        name (unique), category (blank|all|issues|pull/merge requests), max_items,
                 when (a list, one to a line), enabled, command, prompt

schedule_repos   schedule → schedules ON DELETE CASCADE,
                 repo → repos ON DELETE RESTRICT            unique(schedule, repo)

runs             schedule → schedules ON DELETE SET NULL (optional),
                 repo → repos ON DELETE SET NULL (optional), name, repo (the clone's path),
                 command, status (queued|running|done|failed), prompt, output, error,
                 started_at, finished_at

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
- **The rest of m8r as MCP tools.** The MCP endpoint changes the watchlist, and nothing else.
  A client such as Duet can't add a repo, or write or start a schedule: the pages and the API
  do that.
- **Requests from the forge.** m8r could ask each forge which PRs/MRs wait for your review, and
  put them on the watchlist by itself, as requested. Until then, you or Duet check
  "requested".
- **A run for each item.** A run gets its repo's items in one prompt, and one agent works
  through them. m8r could start an agent for each item, with a limit on how many run at once
  and a worktree for each.
- **Setup in the app:** a dotfiles repo and credentials that m8r installs for you, and the
  choice of an existing database.
- **A digest by email:** what the day's runs reported, with an access link.
