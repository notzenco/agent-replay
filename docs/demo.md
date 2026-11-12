# Demo plan

A script for recording a short asciinema cast, and a list of the things
worth showing if the cast is not wanted. The screenshots in the README were
taken from the synthetic fixture, so the demo works the same on any machine.

```sh
asciinema rec -c "cargo run -- view tests/fixtures/claude-session.jsonl" demo.cast
```

## The cast

1. **Open a session.** `agent-replay list` shows what is on the machine. Pick
   a session and open it. The timeline fills with the prompts, the replies and
   the tool calls.

2. **Step through the tool calls.** Press `n` to jump to the next one. The
   detail pane shows the tool, its input and, for a file edit, the diff.

3. **Filter to one tool.** Press `f` to cycle the filter. The timeline narrows
   to the calls of that tool only; the status bar shows the filter.

4. **Search.** Press `/` and type a word. The timeline narrows as you type.
   `Esc` clears it.

5. **Show the statistics.** Leave the viewer and run
   `agent-replay stats <session>`.

6. **Export.** Run `agent-replay export <session> -o out.html` and open it in
   a browser. Show that the file works offline: disconnect the network and
   reload.

## The parts worth pointing at

- The diff in the detail pane is derived from what the agent sent, so it shows
  the change rather than the whole file.
- The token counts in the status bar and in `stats` come from the assistant
  messages, not from a separate tally.
- The exported file loads nothing: no stylesheet, no script, no font. It works
  from a `file://` URL.
- Secrets, home paths and email addresses are replaced in the export. The
  terminal viewer is not touched, so you can still see what really happened.
- A planted agent-tripwire credential shows up highlighted in the export and
  is named at the top of the file.

## The fixtures

The demo uses `tests/fixtures/claude-session.jsonl`, a hand-written session
with a prompt, a reply, a `Read`, an `Edit`, a `Bash`, a failed command and a
turn timing. `tests/fixtures/secret-session.jsonl` has a credential in it, for
showing the redaction.
