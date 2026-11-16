# m8r

m8r ("maintainer"): agentic repo maintainer automations using your personal development
environment. It runs your commands and prompts on a schedule, as a Unix user that you set up
like any computer of yours. It keeps a watchlist of the issues and PRs/MRs that you follow,
which the agents and Claude Duet help to keep, and it shows what the agents report.
[DESIGN.md](DESIGN.md) says how it works and why.

It's a [Loco](https://loco.rs) app (Rust) with a SQLite database. [AGENTS.md](AGENTS.md) points
coding agents at Loco's own skill, in `.claude/skills/loco/`.

## Deploying

On hyperion, [install](../install) does m8r's part. It:

- adds the user `m8r`;
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

   Every page lists what doesn't work yet: the agent's program, `gh` or `glab` once the
   watchlist needs it, m8r's own API if an agent couldn't reach it, and the scheduler if it
   stops ticking. m8r looks again every minute, and a row goes when its tool works.

5. Say how to start the agent. Nobody is there to approve a tool call, so give the agent what
   your prompts need: an `--allowedTools` list, or `--dangerously-skip-permissions`. To change
   the watchlist, an agent runs `curl`. Each schedule has a command of its own. To start every
   new schedule with the same one, set `M8R_AGENT` in
   [/etc/default/m8r](../src/etc/default/m8r), and restart the units.

6. In the pages: write a configuration and make it active, add a schedule, and press "Run now".

## Using it

- **Access.** `m8r task link` prints a link that works any number of times for 8 hours.
  `hours:72` changes that, and `once:true` makes a link that works one time. Each use starts an
  8-hour session. To end every link and session, delete the line in `m8r/.env`, run `install`,
  and restart the units.
- **Configurations** hold the system message: m8r's part of every prompt, which says where the
  agent is and how it reports. Runs use the active configuration. With none active, nothing
  runs.
- **Schedules** say when an operation runs, in which clone, with which command, and with which
  prompt. "Run now" queues the run at once.
  - Say when in words: `every weekday at 06:00`, `every 12 hours`,
    `every monday, thursday at 9:00, 15:00`
    ([hron](https://github.com/simpllyf/hron#expression-syntax) has more). A five-field cron
    expression works too, and m8r puts it in words.
  - Times are on the server's clock, unless you add a time zone: `in America/New_York`.
  - Choosing an operation fills in its command, which you can change: the model, the effort
    and the permissions go there.
  - The operation `other` is for anything else. You write its command, such as
    `claude -p "Update the changelog."`.
  - The prompt is a template, and its command gets it on stdin. The page lists what a prompt
    can use, and shows the prompt compiled: `{{ system_message }}`, `{{ skill }}` (how the
    agent changes the watchlist) and `{{ watchlist }}` (the items, one to a line) are in a new
    schedule's prompt from the start. After them, say what to do.
  - A schedule with a category gets that category's watchlist items. With none, it gets no
    items.
- **The watchlist** is the issues and PRs/MRs that you follow, by URL, each in a category, at
  medium or high priority. m8r asks the forge for each one's title through `gh` or `glab`, and
  one that closes or merges leaves the list.
  - **Categories** are on the same page. m8r starts with `Issue` and `Pull/Merge Request`, and
    you can add your own.
  - **Review requests.** When someone asks you to review a PR/MR, check "review requested" on
    its item. The prompts then say so. The agent that reviews it unchecks it, as the skill
    tells it to.
  - **Agents keep it too.** A prompt with `{{ skill }}` tells its agent how to add, change and
    remove items while it runs. So an agent that sees a merged MR takes it off the list.
- **Runs** happen one at a time. A run's page shows its command, the prompt it got, and what
  the command printed.
- **Report**, the home page, lists a day's artifacts. An agent reports an artifact by making a
  directory for it in the directory that the system message names. The directory holds the
  artifact's files, which m8r hosts, or one text file with a link.

## The API and Duet

Programs use m8r through its API (`/api`) or its MCP endpoint (`/mcp`). Each wants a token.

```sh
m8r task token name:duet scope:watchlist_items   # prints the token, this one time
m8r task tokens                                  # lists them
m8r task revoke name:duet                        # ends one
```

- **A scope** says what a token covers: `all`, or one or more of `watchlist_items`,
  `categories`, `schedules`, `configurations` and `runs`, with commas between them. A token
  for `schedules` can write a command, which runs as `m8r`: give each token the least it needs.
- **The API** takes and gives JSON:

  ```sh
  curl https://m8r.schmit.net/api/watchlist_items -H "Authorization: Bearer $TOKEN"
  curl https://m8r.schmit.net/api/watchlist_items -H "Authorization: Bearer $TOKEN" \
    --json '{"url": "https://github.com/o/r/pull/7", "category": "Pull/Merge Request"}'
  curl -X PATCH https://m8r.schmit.net/api/watchlist_items/1 -H "Authorization: Bearer $TOKEN" \
    --json '{"priority": "high"}'
  curl -X DELETE https://m8r.schmit.net/api/watchlist_items/1 -H "Authorization: Bearer $TOKEN"
  ```

  `categories`, `schedules` and `configurations` work the same way, and `runs` can be read.
  [DESIGN.md](DESIGN.md#the-api) has the rest.
- **Claude Duet** connects to the MCP endpoint. Make it a token for `watchlist_items`: it then
  sees four tools, which list, add, change and remove watchlist items, and nothing else of
  m8r. In Claude's settings, add a custom connector with the URL `https://m8r.schmit.net/mcp`
  and the request header `Authorization: Bearer <token>`. If the form has no place for a
  request header, Duet can't send the token yet (see "Later" in [DESIGN.md](DESIGN.md#later)).
- **Claude Code** on another computer connects the same way:

  ```sh
  claude mcp add --transport http m8r https://m8r.schmit.net/mcp \
    --header "Authorization: Bearer $TOKEN"
  ```

- **A run needs no token of yours.** m8r gives each run one of its own, for the watchlist, in
  `M8R_TOKEN`, with m8r's address in `M8R_URL`. It works until the run ends.

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

## When something is wrong

- `journalctl -u m8r-scheduler` shows the ticks, and `journalctl -u m8r-web` the requests.
- `journalctl -k -g "m8r blocked"` shows what the firewall turned away. The `m8r` user reaches
  the internet and the router's DNS, and nothing else, loopback included.
- `m8r doctor` checks the config and the database. It calls the loopback binding unsafe. Here
  that's on purpose: nginx is in front.
- `m8r task tick` does now what the scheduler does every minute.
- To stop the runs, make no configuration active, or `sudo systemctl stop m8r-scheduler`.
- A run's agent session is in the `m8r` user's home like any other: become `m8r`, go to the
  clone, and `claude --resume`.

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

Before a change is done:

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

The tests use stand-ins for the agent, `gh` and `glab`, so they need no logins and no network.
One stand-in agent runs the skill's commands against the API on loopback, with `curl` (7.82 or
newer, for `--json`).
New models, controllers and tasks start from `cargo loco generate`, which also wires them in.
