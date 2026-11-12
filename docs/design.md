# agent-replay design

`agent-replay` opens a coding agent's session log, lets you step through it in a
terminal, and exports a self-contained HTML timeline you can share.

It is the read-only counterpart to
[agent-tripwire](https://github.com/notzenco/agent-tripwire): tripwire watches
live transcripts for honeytokens that were leaked, agent-replay reconstructs
what a finished session actually did.

The formats below were learned by inspecting real session logs on the author's
machine. No real session content is copied into this repository; every fixture
in `tests/fixtures/` is hand-written and synthetic.

## 1. Supported formats

Three formats are supported behind one trait. The format is detected from the
file's contents, not its path, because the same agent writes to different
locations depending on how it was launched.

Detection reads the first few lines of the file and looks for a discriminator:

| Order | Signal | Format |
| --- | --- | --- |
| 1 | a top-level `payload` object plus a `type` of `session_meta` / `response_item` / `event_msg` / `turn_context` | Codex |
| 2 | a first line with `"type":"session"` and a `"version"` | Axio |
| 3 | a top-level `type` of `user` / `assistant` / `system` alongside a `message` object | Claude Code |
| 4 | filename matches a known agent layout (`~/.claude/projects/<project>/<uuid>.jsonl`, `~/.codex/sessions/...`, `~/.axio/agent/sessions/...`) | matching format |
| — | none of the above | error listing the supported formats |

Rule 4 is a fallback for truncated or unusual files; content wins whenever it
is available.

### 1.1 Claude Code

`~/.claude/projects/<encoded-project-path>/<session-uuid>.jsonl`, one JSON
object per line, append-only. The project path is the working directory with
`/` replaced by `-`.

Lines are discriminated by a top-level `type`:

| `type` | Shape | Handling |
| --- | --- | --- |
| `user` | `message.role == "user"`, `message.content` is a string or a block array | string → user turn; `text` block → user turn; `tool_result` block → tool result |
| `assistant` | `message.content` is a block array | `text` → assistant turn, `thinking` → reasoning, `tool_use` → tool call |
| `system` | `subtype`, `durationMs`, `level` | notice |
| `attachment` | `attachment`, `renderedRole` | ignored (environment context) |
| `file-history-snapshot` | `snapshot.trackedFileBackups` | ignored (diffs are derived from tool input) |
| `cost-state` | cumulative totals | ignored (usage comes from assistant messages) |
| `last-prompt`, `mode`, `permission-mode`, `atis-latch`, `bridge-session`, `ai-title`, `tools-used`, `api-request*`, `queue-operation`, `file-history-delta` | assorted | ignored |

Relevant assistant fields:

```jsonc
{
  "type": "assistant",
  "timestamp": "2026-03-29T08:02:27.123Z",
  "uuid": "…", "parentUuid": "…", "sessionId": "…",
  "cwd": "/Users/me/Developer/personal/agent-replay",
  "gitBranch": "main",
  "isSidechain": false,
  "message": {
    "role": "assistant",
    "model": "claude-opus-5-5",
    "stop_reason": "tool_use",
    "content": [
      { "type": "text", "text": "…" },
      { "type": "thinking", "thinking": "…", "signature": "…" },
      { "type": "tool_use", "id": "toolu_01…", "name": "Edit",
        "input": { "file_path": "…", "old_string": "…", "new_string": "…", "replace_all": false } }
    ],
    "usage": {
      "input_tokens": 2,
      "output_tokens": 317,
      "cache_creation_input_tokens": 51227,
      "cache_read_input_tokens": 28663,
      "output_tokens_details": { "thinking_tokens": 174 }
    }
  }
}
```

Tool inputs the tool understands, by name:

| Tool | Input keys |
| --- | --- |
| `Read` | `file_path`, `offset`, `limit` |
| `Write` | `file_path`, `content` |
| `Edit` | `file_path`, `old_string`, `new_string`, `replace_all` |
| `Bash` | `command`, `description`, `timeout` |

Any other tool name is carried through generically with its raw JSON input.

A `tool_result` block is `{ "type": "tool_result", "tool_use_id": "toolu_01…",
"content": "…" | [ … ], "is_error": false }`. It is matched back to its
`tool_use` by `tool_use_id`.

A streamed assistant message is written as one record per content block, all
sharing `message.id` and each repeating the `usage` block with the output
count still growing. The reader keeps only the last reading per message id,
so a message's tokens are counted once, at their final value.

### 1.2 Codex

`~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<uuid>.jsonl`. Each line is
`{ "timestamp": "…", "type": "…", "payload": { … } }`.

| `type` | `payload.type` | Handling |
| --- | --- | --- |
| `session_meta` | — | session id, `cwd`, `cli_version`, `originator`, `model_provider` |
| `turn_context` | — | `model`, `cwd`, `turn_id`, `effort` |
| `response_item` | `message` | `role` (`user` / `assistant` / `developer`), `content` array of `{type, text}` |
| `response_item` | `reasoning` | reasoning; `summary` array of `{type:"summary_text", text}` |
| `response_item` | `function_call` | tool call: `call_id`, `name`, `arguments` (a JSON **string**) |
| `response_item` | `function_call_output` | tool result: `call_id`, `output` |
| `response_item` | `custom_tool_call` | built-in tool call (`exec`, `apply_patch`): `call_id`, `name`, `input` (one **string**) |
| `response_item` | `custom_tool_call_output` | tool result: `call_id`, `output` |
| `response_item` | `tool_search_call` / `tool_search_output` | tool call / result: `arguments` (an object) / `tools` |
| `response_item` | `web_search_call` | tool call: `action` (`type`, `query`, `queries`) |
| `response_item` | `image_generation_call` | notice naming the `revised_prompt`; the base64 image data is dropped |
| `event_msg` | `user_message` | user turn (`message`) |
| `event_msg` | `agent_message` | assistant turn (`message`) |
| `event_msg` | `token_count` | usage: `info.total_token_usage` and `info.last_token_usage` |
| `event_msg` | `exec_command_end` | exit code, stdout/stderr, duration — matched to the call by `call_id` |
| `event_msg` | `task_started`, `turn_aborted` | notice |

Codex tool names seen in the wild: `exec_command`, `shell_command`,
`write_stdin`, `update_plan`, `view_image`, `request_user_input`, and the
`custom_tool_call` built-ins `exec` and `apply_patch`.

A tool output is either a string or a list of content blocks
(`input_text`, `input_image`, …); blocks are rendered as text, with an image
shown as `[image]` so its data never reaches the timeline.

Codex splits its turn text across two places: `event_msg/user_message` and
`event_msg/agent_message` carry the user-visible prose, while
`response_item/message` carries the same content plus the `developer` and
system-injected messages. To avoid duplicates, the reader keeps the
`event_msg` copy and drops the `response_item` copy when the two match.
`developer` messages are the system prompt rather than something the session
did, so they are skipped.

Codex reports usage only as a running total in `token_count` events. The
reader treats the first reading as the usage so far and each later reading as
the difference from the one before it, so the readings add up to the session
total. Codex counts cached input inside `input_tokens`, so the cached part is
split out to match how the other formats report it.

Codex also writes an `exec_command_end` event for each command, carrying its
exit code and duration. That event is matched to the tool result by `call_id`
and enriches it, rather than becoming a second event of its own.

### 1.3 Axio

`~/.axio/agent/sessions/<encoded-cwd>/<epoch-ms>-<pid>.jsonl` and
`…/<encoded-cwd>/agents/<epoch-ms>-<pid>.jsonl`. Append-only JSONL.

The first line is a header:

```json
{ "type": "session", "version": 1, "created_ms": 1794477932897,
  "cwd": "/Users/me/Developer/…/axio" }
```

Every following line is a message:

```json
{ "type": "message",
  "message": { "role": "assistant",
    "content": [ { "type": "text", "text": "…" },
                 { "type": "tool_call", "id": "…", "name": "Bash", "input": { … } },
                 { "type": "tool_result", "id": "…", "output": "…", "is_error": false },
                 { "type": "opaque", "provider": "…", "value": "…" } ] } }
```

`opaque` blocks are provider-specific payloads with no stable shape; they are
rendered as a notice rather than being interpreted.

Axio carries no usage information, so token counts for Axio sessions are
always zero.

## 2. Normalised event model

Every format is read into one model, so the TUI, the stats command and the
HTML export never know which agent produced a log.

```rust
pub struct Session {
    pub id: String,
    pub source: SessionSource,     // ClaudeCode | Codex | Axio
    pub cwd: Option<PathBuf>,
    pub model: Option<String>,
    pub started_at: Option<OffsetDateTime>,
    pub events: Vec<Event>,
}

pub struct Event {
    pub index: usize,                    // position in the timeline
    pub kind: EventKind,
    pub ts: Option<OffsetDateTime>,
    pub duration_ms: Option<u64>,        // tool duration, where the format reports one
    pub usage: Option<Usage>,            // assistant messages only
    pub model: Option<String>,
    pub tool_call_id: Option<String>,    // links a call to its result
}

pub enum EventKind {
    User { text: String },
    Assistant { text: String },
    Reasoning { text: String },
    ToolCall { name: String, input: serde_json::Value },
    ToolResult { output: String, is_error: bool },
    Notice { text: String },
}

pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub reasoning_tokens: u64,
}
```

Design decisions:

- **A tool call and its result are separate events.** The timeline reads like
  the log did, and a long-running command shows its result arriving later.
  `tool_call_id` links them.
- **Diffs are derived, not stored.** A `Write` or `Edit` diff is computed from
  the tool input on demand, so the model stays a faithful copy of what the
  agent did and there is one implementation of diffing for both the TUI and
  the export.
- **Timestamps are optional.** Not every format stamps every record; `ts` is
  `None` rather than a sentinel.
- **Usage is per assistant message.** Totals are summed for `stats`, which
  keeps per-message deltas available in the TUI.

The reader trait:

```rust
pub trait SessionReader {
    /// Parse a whole session file into the normalised model.
    fn read(path: &Path) -> anyhow::Result<Session>;
    /// Cheap check used by format detection.
    fn detect(path: &Path) -> bool;
}
```

Parsing is line-at-a-time and tolerant: a line that fails to parse is counted
as a malformed line and skipped rather than aborting the whole session. A
session that yields zero usable lines is an error.

## 3. TUI layout

A ratatui terminal UI with two panes and a status bar.

```
┌ agent-replay · claude-code · 2026-03-29 08:02 ─────────────┬──────────────────────────┐
│ Timeline — 12 of 214                                        │ Detail                  │
│                                                            │                          │
│ ▸ 00:00:00  user       add a health endpoint                │                          │
│   00:00:04  assistant  I'll add it to the router.           │ Tool call: Edit         │
│   00:00:05  tool       Edit  src/api/routes.rs              │ Path: src/api/routes.rs │
│   00:00:09  result     ok                                   │                          │
│   00:00:09  tool       Bash  cargo test                     │ -  fn routes() {         │
│   00:00:21  result     12 passed                            │ +  fn routes() -> … {    │
│                                                            │                          │
├────────────────────────────────────────────────────────────┴──────────────────────────┤
│ /search · f filter · n/p next tool · g/G ends · q quit                             │
└───────────────────────────────────────────────────────────────────────────────────────┘
```

- **Left pane** — the timeline. One line per event: relative offset, a
  two-letter kind tag, a short label, and the tool name for tool calls.
  Long lines are truncated with an ellipsis at the pane edge.
- **Right pane** — the detail for the selected event. For a tool call it shows
  the tool name, the path or command, the formatted input and, for `Write` and
  `Edit`, a colourised diff. For a tool result it shows the output.
- **Status bar** — the key hints, the search term, the active filters and the
  position in the filtered list.

Keys:

| Key | Action |
| --- | --- |
| `j` / `k`, `↓` / `↑` | move the selection |
| `g` / `G` | first / last event |
| `/` | incremental search over the event text, the tool name and the tool's input; `Esc` clears it |
| `f` | cycle the tool filter (`all` → the tools the session used, in alphabetical order) |
| `n` / `p` | next / previous tool call |
| `Enter` | scroll the detail pane down; `BackTab` scrolls back up |
| `Esc` | clear the search and the filter |
| `q` | quit |

The filter and the search term combine: an event is shown when it matches the
search text *and* passes the tool filter. The counts in the status bar always
report both the filtered and the total.

## 4. HTML export

`agent-replay export <session> -o out.html` writes one file with **no
external requests** — no CDN, no web fonts, no analytics, no images loaded
from elsewhere. The file works offline, from a `file://` URL, and can be
attached to a bug report.

Structure:

- `<style>` block with every rule inline. System font stack, no `@import`.
- A header with the session id, source format, working directory, model,
  start time, duration and token totals.
- A summary bar: tool call counts, files touched, tokens.
- A timeline of cards, one per event, each with a time badge, a kind badge and
  the body.
- Tool calls render their input as a formatted key/value list.
- `Write` and `Edit` render a diff table with `+`/`−`/` ` prefixes and
  green/red backgrounds.
- Tool results render inside a `<pre>` block, with errors in red.
- No JavaScript. Interactivity comes from CSS only (`<details>` for
  collapsible sections, `:hover` for highlighting). This removes the largest
  class of injection and escaping problems.

Escaping is explicit: every dynamic value goes through an HTML escaper for
`&`, `<`, `>`, `"` and `'`. Because there is no `<script>` and no `style`
attribute carrying user data, an escaped session cannot break out of its
context.

The export is deterministic: the same session and the same flags produce
byte-identical output, so exports can be diffed in review.

## 5. Privacy and redaction

**Redaction is on by default for exports.** `--no-redact` turns it off. The
TUI always shows the raw content; only the exported artefact is redacted, so
an operator can still see what happened locally.

### 5.1 What is redacted

| Category | Example | Replacement |
| --- | --- | --- |
| Home directory | `/Users/me/Developer/x`, `/home/runner/x`, `C:\Users\someone\x` | `~/Developer/x` |
| Email address | `user@example.com` | `[email]` |
| AWS access key id | `AKIAIOSFODNN7EXAMPLE` | `[redacted:aws-key-id]` |
| AWS secret key | value of `aws_secret_access_key` | `[redacted:aws-secret-key]` |
| GitHub token | `ghp_…`, `gho_…`, `ghs_…`, `ghr_…`, `github_pat_…` | `[redacted:github-token]` |
| GitLab token | `glpat-…` | `[redacted:gitlab-token]` |
| Slack token | `xoxb-…`, `xoxp-…`, `xoxs-…` | `[redacted:slack-token]` |
| Stripe secret | `sk_live_…`, `rk_live_…` | `[redacted:stripe-key]` |
| Anthropic key | `sk-ant-…` | `[redacted:anthropic-key]` |
| OpenAI key | `sk-…`, `sk-proj-…` | `[redacted:openai-key]` |
| Google API key | `AIza…` | `[redacted:google-api-key]` |
| npm token | `npm_…` | `[redacted:npm-token]` |
| Private key block | `-----BEGIN RSA PRIVATE KEY-----…` | `[redacted:private-key]` |
| JSON Web Token | `eyJhbGci….eyJ….…` | `[redacted:jwt]` |
| Bearer header | `Authorization: Bearer <token>` | `[redacted:bearer-token]` |
| URL password | `postgres://app:…@host/db` | `[redacted:url-password]` |
| Password assignment | `password = "…"` | `[redacted:password]` |
| Generic secret assignment | `api_key = "…"` | `[redacted:secret]` |
| High-entropy string | 32+ chars, mixed case and digits, ≥ 3.5 bits/char | `[redacted:high-entropy]` |

The home directory is replaced by the literal `~/`, which keeps paths
readable and still identifies the shape of what was happening. Any user's
home counts, in Unix, WSL (`/mnt/c/Users/…`) and Windows (`C:\Users\…`)
spellings, because a session recorded on another machine — or a CI log it
quotes — names users this machine has never heard of.

Only the password of a URL is replaced, and only when it starts with a letter
or a digit: `postgres://app:${DB_PASSWORD}@host/db` is a template, not a
secret, and it is left as it is.

The high-entropy rule is deliberately conservative. A candidate must be at
least 32 characters, contain an uppercase letter, a lowercase letter and a
digit, and score at least 3.5 bits of Shannon entropy per character. Pure
lowercase hex (a git SHA), pure base64 without mixed case, and long
identifier-like words are not redacted, because redacting them would bury the
timeline in noise without protecting anything.

### 5.2 Honeytokens

`agent-tripwire` plants values shaped as `TRIPWIRE` followed by an 8-character
base32 id, where the last two characters are a checksum over the first six.
The checksum algorithm is reproduced here, so a candidate is only treated as a
honeytoken when it is a real tripwire token and not a coincidence.

In an export, a honeytoken is **highlighted**, not removed: it is wrapped in a
`<mark>` element and listed in a banner at the top with the count and the ids
found. That is the point of a honeytoken — the export proves the leak
occurred. Every other secret is replaced, because a honeytoken's only value is
that it was noticed.

`--no-highlight-honeytokens` suppresses the highlighting for a report that
must not carry the token itself.

### 5.3 Scope

Redaction applies to every string that reaches the export: event text, tool
input values, tool output, file paths, the working directory and the session
header. The file path shown in a `Write` or `Edit` detail is redacted the same
way as any other text.

Redaction is applied at render time, not at parse time, so the TUI and the
`stats` command still operate on the real values.

## 6. CLI surface

```
agent-replay list [--format claude|codex|axio] [--cwd <path>]
agent-replay view <session>
agent-replay stats <session> [--json]
agent-replay export <session> -o <file.html> [--no-redact] [--no-highlight-honeytokens]
```

`<session>` is either a path to a log file or a session id, which is resolved
against the known roots (`~/.claude/projects`, `~/.codex/sessions`,
`~/.axio/agent/sessions`).

`stats` prints tool call counts, the files touched, the wall-clock duration
and the token breakdown, and `--json` emits the same data as JSON for
scripting.

## 7. Tests

Every fixture is hand-written and synthetic. The repository contains no real
session content.

- `tests/fixtures/claude-session.jsonl` — a Claude Code session with a user
  turn, an assistant turn, a `Read`, an `Edit`, a `Bash`, a tool error and
  usage.
- `tests/fixtures/claude-split-message.jsonl` — one assistant message split
  over three records with repeated usage, plus a user image block.
- `tests/fixtures/codex-session.jsonl` — a Codex rollout with `session_meta`,
  a `function_call`, a `function_call_output`, an `exec_command_end` and
  `token_count` events.
- `tests/fixtures/codex-custom-tools.jsonl` — a Codex rollout with
  `custom_tool_call` records, block-shaped tool outputs, a tool search, a web
  search and an image generation.
- `tests/fixtures/axio-session.jsonl` — an Axio session with a header, a
  user turn, a `tool_call` and a `tool_result`.
- `tests/fixtures/secret-session.jsonl` — a session holding a cloud key and an
  email address, for checking that redaction works.
- `tests/fixtures/not-a-session.txt` — a file that is not a session log, for
  checking that the format is refused rather than guessed.

The test suite covers format detection, the normalisation of each format, diff
rendering, every redaction rule, honeytoken detection and validation, the
stats arithmetic, and the guarantee that an exported file makes no external
requests.
