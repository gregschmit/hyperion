# m8r

m8r ("maintainer"): agentic repo maintainer automations using your personal development
environment. It runs your prompts on a schedule, as a Unix user that you set up like any computer
of yours, and shows what the agents report. [DESIGN.md](DESIGN.md) says how it works and why.

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

   Every page lists what doesn't work yet: the agent's program, `gh` or `glab` once an item
   needs it, and the scheduler if it stops ticking. m8r looks again every minute, and a row goes
   when its tool works.

5. Say how to start the agent, in `M8R_AGENT` in [/etc/default/m8r](../src/etc/default/m8r), and
   restart the units. Nobody is there to approve a tool call, so give the agent what your prompts
   need: an `--allowedTools` list, or `--dangerously-skip-permissions`.

6. In the pages: write a configuration and make it active, add a schedule, and press "Run now".

## Using it

- **Access.** `m8r task link` prints a link that works any number of times for 8 hours.
  `hours:72` changes that, and `once:true` makes a link that works one time. Each use starts an
  8-hour session. To end every link and session, delete the line in `m8r/.env`, run `install`,
  and restart the units.
- **Configurations** hold the prompts: one each for triage, review and develop, and the system
  message that m8r puts where a prompt says `{{ system_message }}`. All four are templates, and
  a configuration's page shows each prompt compiled. Runs use the active configuration. With
  none active, nothing runs.
- **Schedules** say when an operation runs, and in which clone: a five-field cron expression in
  the server's time zone. "Run now" queues the run at once.
- **Items** are the issues and PRs/MRs that you care about, by URL, at medium or high priority.
  Every run gets the list. m8r asks the forge for each item's title through `gh` or `glab`, and
  an item that closes or merges leaves the list.
- **Runs** happen one at a time. A run's page shows the prompt it got and what the agent
  printed.
- **Report**, the home page, lists a day's artifacts. An agent reports an artifact by making a
  directory for it in the directory that the system message names. The directory holds the
  artifact's files, which m8r hosts, or one text file with a link.

## Updating

As `gns`, in `/srv/hyperion`: `git pull && sudo ./install`. `install` restarts nothing. When it
says that it rebuilt m8r, and nothing is running, restart it:

```sh
sudo systemctl restart m8r-web m8r-scheduler
```

A restart of `m8r-scheduler` ends the run in progress, and the next tick marks it failed.

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

In development the agent command is `cat`, which prints the prompt back, so a run costs nothing.
`M8R_AGENT='claude -p' cargo loco start --scheduler` uses a real agent. The database
(`m8r_development.sqlite`) and the data (`data/`) are in this directory, and git ignores them.

Before a change is done:

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

The tests use stand-ins for the agent, `gh` and `glab`, so they need no logins and no network.
New models, controllers and tasks start from `cargo loco generate`, which also wires them in.
