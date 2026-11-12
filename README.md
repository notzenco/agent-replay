# agent-replay

Open a coding agent's session log, step through it, and export a shareable
HTML timeline.

![The HTML export of a session](docs/images/export.png)

`agent-replay` reads the transcripts that Claude Code, Codex and Axio write,
turns them into one timeline, and shows you what actually happened: every
prompt, every tool call, every result, every file change, with timings and
token counts. The exported HTML is a single file that loads nothing from
anywhere else, so it can be attached to a bug report or sent to someone
else.

It is the read-only counterpart to
[agent-tripwire](https://github.com/notzenco/agent-tripwire): tripwire watches
live transcripts for planted credentials that were leaked, agent-replay
reconstructs what a finished session did.

## What it does

- **Reads three formats** — Claude Code session JSONL, Codex rollout JSONL and
  Axio session JSONL — behind one interface, and works out which one it is
  from the contents of the file.
- **Shows a timeline** in the terminal: prompts, replies, reasoning, tool
  calls, results and file diffs, with search and a filter by tool.
- **Exports a self-contained HTML file** with no external requests, so it
  works offline and from a `file://` URL.
- **Redacts by default.** Secrets, your home directory and email addresses
  are replaced before anything is written out.
- **Finds agent-tripwire honeytokens** and highlights them, so an export can
  prove that a planted credential was leaked.

## Install

```sh
git clone https://github.com/notzenco/agent-replay
cd agent-replay
cargo install --locked --path .
```

Or run it without installing:

```sh
cargo run -- view ~/.claude/projects/<project>/<session>.jsonl
```

## Use

```sh
agent-replay list                        # what is on this machine
agent-replay view <session>               # open it in the terminal
agent-replay stats <session>              # what the session did
agent-replay export <session> -o out.html # write it out as HTML
```

`<session>` is either a path to a log file or a session id, which is looked up
in `~/.claude/projects`, `~/.codex/sessions` and `~/.axio/agent/sessions`.

### The terminal viewer

```
 agent-replay  claude-fixture-0001  claude-code  10 events  15950 tokens in  193 out
┌ Timeline ──────────────────────────────────┐┌ Detail · Edit ─────────────────────────────────────┐
│  00:00:00 user  add a health endpoint      ││Edit /work/demo-app/src/routes.rs                   │
│  00:00:02 agent I'll add a health endpoint ││                                                    │
│  00:00:02 tool  Read /work/demo-app/src/rou││file_path  /work/demo-app/src/routes.rs             │
│  00:00:02 out   ok                         ││new_string  pub fn routes() -> Router {             │
│▸ 00:00:04 tool  Edit /work/demo-app/src/rou││Router::new().route("/health", get(health))}        │
│  00:00:04 out   ok                         ││old_string  pub fn routes() {    health()}          │
│  00:00:06 tool  Bash cargo test            ││replace_all  false                                  │
│  00:00:08 out   error                      ││                                                    │
│  00:00:09 agent The test command failed. Pl││+2 −2 /work/demo-app/src/routes.rs                  │
│  00:00:09 note  turn took 9.1s             ││-pub fn routes() {                                  │
│                                            ││-    health()                                       │
│                                            ││+pub fn routes() -> Router {                        │
│                                            ││+    Router::new().route("/health", get(health))    │
│                                            ││ }                                                  │
│                                            ││                                                    │
│                                            ││in 200  out 90  cache read 3400  reasoning 0        │
└────────────────────────────────────────────┘└────────────────────────────────────────────────────┘
 /search  f filter  n/p tool  g/G ends  enter scroll  q quit  5/10
```

| Key | Action |
| --- | --- |
| `j` / `k`, `↓` / `↑` | move the selection |
| `g` / `G` | first / last event |
| `/` | search, `Esc` clears it |
| `f` | cycle the tool filter |
| `n` / `p` | next / previous tool call |
| `Enter`, `PageUp` / `PageDown` | scroll the detail pane |
| `q` | quit |

### Statistics

```
$ agent-replay stats tests/fixtures/claude-session.jsonl
session         claude-fixture-0001
format          claude-code
working dir     /work/demo-app
model           claude-sonnet-4-5
started         2026-03-29 8:02:27.0 +00:00:00
duration        9.1s
events          10

tool calls
  Bash  1 of 3
  Edit  1 of 3
  Read  1 of 3

files touched
  /work/demo-app/src/routes.rs

tokens
  input         750
  output        193
  cache read    13700
  cache create  1500
  reasoning     12
  total         16143
```

`--json` prints the same figures as JSON, for scripting.

## Privacy

**Redaction is on by default in exports.** The terminal viewer always shows
what really happened; only the exported file is cleaned up.

| What | Example | Becomes |
| --- | --- | --- |
| Home directory | `/Users/me/Developer/x`, `C:\Users\someone\x` | `~/Developer/x` |
| Email address | `someone@example.com` | `[email]` |
| Cloud and API keys | `AKIA…`, `ghp_…`, `glpat-…`, `xoxb-…`, `sk_live_…`, `sk-ant-…`, `AIza…`, `npm_…` | `[redacted:<kind>]` |
| Private key blocks | `-----BEGIN RSA PRIVATE KEY-----…` | `[redacted:private-key]` |
| JSON Web Tokens | `eyJhbGci….eyJ….…` | `[redacted:jwt]` |
| Passwords and secrets | `password = "…"`, `postgres://app:…@host/db` | `[redacted:password]` |
| High-entropy strings | 32+ characters of near-random text | `[redacted:high-entropy]` |

The high-entropy rule is deliberately strict: a candidate has to be at least
32 characters, mix upper case, lower case and digits, and score at least
3.5 bits of entropy per character. A git commit hash and an ordinary long
identifier are left alone, because redacting them would bury the timeline
without protecting anything.

Pass `--no-redact` to keep everything as it was.

### Honeytokens

A value planted by [agent-tripwire](https://github.com/notzenco/agent-tripwire)
looks like `TRIPWIRE` followed by an eight-character id whose last two
characters are a checksum over the first six. agent-replay reproduces that
checksum, so a candidate is only treated as a honeytoken when it is a real
one.

In an export a honeytoken is **highlighted, not removed**, and listed at the
top with the ids found. That is the point of a honeytoken: the export proves
the leak occurred. Every other secret is replaced.

Pass `--no-highlight-honeytokens` for a report that must not carry the token
itself.

## Formats

| Format | Where the logs live | Notes |
| --- | --- | --- |
| Claude Code | `~/.claude/projects/<project>/<session>.jsonl` | full tool calls, usage and per-turn timings |
| Codex | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` | function calls, exit codes, durations, running token totals |
| Axio | `~/.axio/agent/sessions/<project>/<id>.jsonl` | messages and tool calls; no timings or token usage |

The format is worked out from the contents of the file, so a log that has been
moved somewhere else still opens.

## How it is built

```
src/
  model.rs      the normalised event model every format is read into
  format/       one module per log format, behind the SessionReader trait
  diff.rs       turns a Write or Edit into a diff
  redact.rs     the redaction rules
  honeytoken.rs finds agent-tripwire honeytokens
  export.rs     renders the self-contained HTML
  stats.rs      what a session did
  tui.rs        the terminal viewer
  cli.rs        the command line
  discover.rs   finds session logs on this machine
```

The formats differ in how they spell a tool call, when they stamp a record
and whether they report token usage. Everything downstream of `model.rs` works
with the normalised model and never learns which agent produced a log.

`docs/design.md` has the details, including the shape of each format on disk.

## Development

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Every fixture under `tests/fixtures/` is hand-written and synthetic. The
repository contains no real session content and no real credentials.

## Licence

MIT. See [LICENSE](LICENSE).
