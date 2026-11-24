# What's inside GOD OF THE VOID

This page lists every part of the agent, what it does and how far we got with it. The
code is all in this repository: 158,698 lines of Rust in 302 files, plus 11,497 lines of
interface code. Most of the agent lives in `crates/gotv-core`, which has 117 modules.

Most parts were tested twice. First with their own unit tests, then with a real task on a local
model, a 35B mixture-of-experts model (a Qwen3.6-35B-A3B variant, 4-bit). A task counts as
done when a hidden judge checks the result, not when the model says it finished. The
numbers below come from those runs, on an M1 Max with 64 GB.

✅ means built and measured on a real task. ◐ means built and now in bug fixing on
real hardware.

[Español](CUERPO.md) · [Back to the README](README.md)

---

## The base

✅ The tool loop, edits that tolerate small mismatches, and long outputs sent to a file
instead of the context. Long jobs get compacted so the model doesn't lose the thread.

With the 35B model a long job finished in 17 steps and 3 minutes, without needing to
compact at all. With a 9B model it no longer claims to be done early, but it compacts
about every 15 steps and hit our 30-minute limit. Small models still struggle here.

## Understanding code

✅ Language servers for many languages, started and managed by the agent. Servers that
aren't on your Mac are downloaded from their official release and checked against a
fixed SHA-256 (zls, lua-language-server, taplo and marksman, for example). After every
edit the agent reads the new diagnostics, grouped by file.

✅ A map of the repository, so the model can find things without reading every file. In
our runs it helped with searching but cost context while editing.

✅ Per-language packs that know how each ecosystem runs its tests. We measured 8 "run
test X" tasks across languages.

## Testing and debugging

✅ A `pruebas` tool with 14 test runners. Before the agent can close a task, the tests
that touch the changed files have to pass.

✅ A debugger over DAP. Measured with debugpy for Python and lldb-dap for Rust. Go
(dlv) is wired in but not measured.

## Safety

✅ Every command runs inside a macOS sandbox (Seatbelt) that can only write inside your
project. Permission rules decide what runs, asks or gets refused.

✅ For commands the rules can't decide, a judge model looks at them. On our test
set of 100 safe and 100 dangerous commands, the rules alone let 0 dangerous ones through
and 99 safe ones. With the judge on top, all 100 safe commands ran without asking and no
dangerous one was approved.

✅ Secrets stay out of cloud models, and permissions can be scoped to a path or a tool.

✅ New dependencies are checked before they're installed. Lookalike package names and
known vulnerable versions were stopped in our runs.

## Git and pull requests

✅ Git reads go through gix, which we measured faster than calling the `git` binary.
Writes use the real binary. Pushing always asks first. The agent can open a PR, read its
comments and CI, and fix a red build, but it never merges.

## Data

✅ SQLite, Postgres, MySQL/MariaDB and DuckDB, plus CSV and Parquet files through DuckDB.
Connections can go through an SSH tunnel, passwords go to the Keychain, and read-only is
enforced by the engine, not by the prompt. We measured SQLite, Postgres, a remote database
and an attack test against it.

## Connections

✅ MCP client and server, over stdio, HTTP, SSE and WebSocket, with its own OAuth. We
measured a server with 40 tools, images and OAuth. Local MCP servers run inside the same
sandbox.

✅ ACP, so editors like Zed can drive the agent. With the local engine, a change asked from
Zed came back as a diff, was accepted and landed in the file in 12 seconds.

✅ Hooks, its own plugins and Claude Code plugins. Remote control from your phone through
a QR code, over a private tunnel.

## Containers

✅ Docker and docker compose, outside the sandbox but with fixed limits. Compose files are
normalized first, and we measured a compose project end to end.

## Several agents

✅ One turn can start several subagents. Each one works on its own branch, and a separate
verifier, which can't edit, checks the result. With cloud models they run in parallel.
With local models they take turns.

On a local run, a team of 3 subagents finished in 11 steps and 300 seconds, with the
verifier's PASS and the hidden judge green. With a small cloud model, the same job took
86 seconds in parallel against 118 seconds one after another. Creating and merging a
branch takes about 63 ms on a 300-file project, without starting `git`.

## Ways of working

✅ Rules per folder, goals checked by text or by tests, an interview mode and a learning
mode. They cost no context when you don't use them.

✅ You can send a message while it works and it changes course without stopping. Custom
agents, projects on another machine over SSH (with the language server running over
there), `/revisar` to review a change, and following a PR until CI is green.

✅ Automations and notifications: schedule a prompt, run it without the window open, get
a desktop notice when it's done.

✅ Watching files, logs and sockets and reacting to them. In our test, an error written to
a log from outside was found and fixed in 9 seconds.

## Notebooks

✅ Jupyter notebooks with the project's own kernel, talked to over ZeroMQ from Rust. In our
test the agent fixed a broken cell, added a bar chart that stays inside the `.ipynb` and
looked at it, in 23 seconds.

## Browser and research

✅ Its own browser and Chrome control, plans you can comment line by line, and a recording
of what the agent did in the browser.

✅ Web research across several search engines at once, with citations. In "local only"
mode it doesn't go online at all, and we tested that too.

## Mobile

◐ iOS Simulator and Android emulator: build, install, launch and catch an app that crashes
on open. Measured in 144 seconds on both. It also creates devices and installs the Android
SDK from Rust. It's now in bug fixing on real phones.

## Connected devices

◐ USB, disks, printers, cameras, Raspberry Pi and ESP32. The camera is measured (a 364 KB
JPEG in 5.1 seconds). The rest is in bug fixing on real hardware.

## The window

✅ Three zones built from Rust, a work panel with tabs, chats per project, a changes view
that works without git, mentions, several folders in one project, chats without a project
and ⌘K search. Rewind undoes a whole turn, even across folders.

---

Something here doesn't match what you see? Please [open an issue](https://github.com/CHAOSX-SPACE/god-of-the-void/issues).
