<div align="center">

<img src="crates/gotv-gui/icons/icon.png" alt="GOD OF THE VOID" width="130">

# GOD OF THE VOID

## A local AI studio for the Mac, written in Rust

*An app and an engine, made by [**CHAOS-X**](https://chaosx.space). Everything runs on your machine, and the cloud is only used if you turn it on.*

GOD OF THE VOID, or GOTV, is a coding agent and a voice assistant. It also comes with
engines for images, video, music and sound. We wrote it from scratch in Rust on top of
Apple's MLX.
There is no Python in it, no server to rent and no telemetry.

| You want | Take this | Size |
|---|---|---|
| **The app** | `GOD OF THE VOID.app` from [**Releases**](https://github.com/CHAOSX-SPACE/god-of-the-void/releases) | 478 MB |
| **Only the engine** (no window) | `god-of-the-void-engine-…-macos-arm64.tar.gz` from [**Releases**](https://github.com/CHAOSX-SPACE/god-of-the-void/releases) · [what it does](#the-engine-without-the-app) | 78 MB |
| **To build it yourself** | `cargo run --release -p gotv-empaquetar` (see [BUILDING.md](BUILDING.md)) | |

![status](https://img.shields.io/badge/status-0.1%20early-F0A23F?style=for-the-badge)
![license](https://img.shields.io/badge/license-Apache--2.0-8A7CF7?style=for-the-badge)
![platform](https://img.shields.io/badge/macOS%2014%2B-Apple%20Silicon-67E8F9?style=for-the-badge)
![rust](https://img.shields.io/badge/100%25-Rust-F0523F?style=for-the-badge)

[**User manual**](manual/README.md) · [Models on Hugging Face](https://huggingface.co/god-of-the-void) · [chaosx.space](https://chaosx.space) · [Español](README.es.md)

</div>

---

## See it in two minutes

https://github.com/user-attachments/assets/63ddbbd2-36a3-4ddd-bad0-4c79c6be3e58

*A narrated tour of the window. [En español](README.es.md#míralo-en-dos-minutos) · the full [user manual](manual/README.md).*

---

Other AI studios run on someone else's servers. GOTV runs on your Mac.

It can read your code, run your tests and fix what breaks. You can talk to it and it
answers out loud. You can also ask it for an image, a video, a song or the sound of rain.
It makes them locally, with models you picked and whose licenses you accepted.

---

> [!IMPORTANT]
> This is version 0.1 and it is early. We use it every day, but it still has known bugs,
> and they are listed [below](#known-errors-and-rough-edges). GOTV is open source and
> free. Nothing here is for sale.

---

## In numbers you can verify

All numbers were measured on an Apple M1 Max with 64 GB, using the project's own
benchmarks and test judges. None of them come from a vendor's spec sheet.

| | | how |
|---|---|---|
| **14 crates** | one Rust workspace: engine, core, app, terminal, voice, sound, design, data | `ls crates` |
| **No Python at runtime** | the models run inside the process; MLX is called through FFI | `crates/gotv-mlx` |
| **0.5 s** | first answer of a conversation, down from 9.8 s: the system prompt's cache is read from disk | `prefijo_del_sistema_desde_disco` |
| **−30 % / +13 %** | prefill time / generation speed of a 35B mixture-of-experts model, against the reference server | same model, best of 3, alternated |
| **2.0× real time** | its own voice; first audio in 0.36 s | `gotv say "…"` |
| **25.6 s** | a 1024×1024 image with FLUX.2 klein 4B (512×512 in 8.5–9.8 s) | `gotv image "…"` |
| **6.7 s** | 30 s of music with ACE-Step 1.5 turbo (240 s in 46 s, ~10 GB flat) | `gotv music "…"` |
| **47.8 min** | 2 s of 720p video, 50 steps, with Wan 2.2 TI2V 5B (peak 28.7 GB) | `gotv video "…"` |
| **1,240 tests** | all green, from a fresh clone of this repository ([how](#every-claim-is-tested)) | `cargo test --release --workspace` |

We only list what we measured. Before an engine was allowed in, we checked it against the
official reference implementation. The tokens had to match and the latents had to agree
within float rounding. In several cases our output was closer to the exact float64 result
than the reference was.

---

## Two ways in

### The app

The app is a coding agent with real tools. It can read, write, edit and search files, run
commands, talk to language servers and show diagnostics. It keeps a map of your code base,
does semantic search over it, and looks up packages at the exact version *your* project
uses.

- Permissions: you pick a mode (plan, ask, accept edits or bypass) and add allow and deny
  rules. Some things are refused even in bypass mode. Every command runs inside a macOS
  sandbox (Seatbelt) that can only write inside your project.
- Rewind undoes the last step, including the edits and anything the shell deleted.
- It can reach further when you allow it. That covers MCP client and server, ACP for
  editors, projects over SSH, containers, its own browser and the other apps on your Mac.
  Databases work too (SQLite, Postgres, MySQL, DuckDB, CSV/Parquet), with read-only
  enforced by the engine.
- Voice is hands-free. You talk, you can interrupt it, and you keep talking.
- The Expert studio is a multitrack audio studio where the AI handles clips, automation,
  the mixer, effects and analysis. It exports WAV, FLAC, stems and `.dawproject` files for
  Bitwig, Studio One or Cubase.
- It makes Word and PDF documents with themes, charts inside the apps it builds, and REST
  APIs in Hono, FastAPI or Axum with tests.
- Everything is in English and Spanish. A test fails if any text is left in only one
  language.

### The engine: `gotv-mlx`

`gotv-mlx` is the engine inside the app, and it also works on its own. It's a multimodal
inference engine written in Rust on Apple's MLX, and every model architecture in it was
written by hand. It only depends on `gotv-engine` and the MLX bindings, not on any part of
the app.

| Sense | Model it runs today | Inside |
|---|---|---|
| **Language** | Qwen 3.5 family, its mixture-of-experts models, Gemma 4 | expert routing, sliding windows, per-layer embeddings, prompt cache on disk |
| **Vision** | Gemma 4 vision | images into the conversation |
| **Hearing** | Whisper large-v3-turbo | voice-activity detection + transcription |
| **Voice** | Qwen3-TTS 1.7B | streaming speech, interruptible |
| **Image** | FLUX.2 klein 4B | guided by a sketch, an isometric scene or up to 4 references |
| **Video** | Wan 2.2 TI2V 5B | text → video and image → video, H.264 through AVFoundation |
| **Music** | ACE-Step 1.5 | instrumentals, songs with lyrics, covers and repaints of your own audio |
| **Sound** | MOSS-SoundEffect 2.0 | rain, doors, explosions |
| **Stems** | HTDemucs (4 / 6 / ft) + [our fine-tunes](https://huggingface.co/god-of-the-void) | vocals, drums, bass, guitar and piano, plus the drums split into kick, snare, cymbals and toms |
| **Search** | Qwen3-Embedding 0.6B | semantic search over your code |

---

## What's inside

GOTV is bigger than the list above. The repository holds 158,698 lines of Rust in 302
files, and most of the agent lives in `gotv-core`, which has 117 modules. A ✅ means the
part was also tried on a real task with a local 35B model, and a hidden judge checked the
result.

| Part | What it covers | |
|---|---|---|
| Base | tool loop, tolerant edits, long outputs to files, context compaction | ✅ |
| Understanding code | language servers with pinned downloads, repository map, per-language packs | ✅ |
| Testing and debugging | 14 test runners, tests must pass before a task closes, DAP debugger | ✅ |
| Safety | macOS sandbox, permission rules, a judge for unclear commands, dependency checks | ✅ |
| Git and PRs | reads through gix, PRs with comments and CI, never merges | ✅ |
| Data | SQLite, Postgres, MySQL, DuckDB, CSV/Parquet, SSH tunnels, read-only enforced by the engine | ✅ |
| Connections | MCP over stdio, HTTP, SSE and WebSocket with OAuth, ACP for editors, hooks, plugins, phone remote | ✅ |
| Containers | Docker and docker compose | ✅ |
| Several agents | subagents on their own branches, a verifier that can't edit | ✅ |
| Ways of working | rules per folder, goals, automations, watching files and logs, remote projects | ✅ |
| Notebooks | Jupyter with the project's own kernel | ✅ |
| Browser and research | its own browser, Chrome control, web research with citations | ✅ |
| Mobile | iOS Simulator and Android emulator | ◐ bug fixing on real hardware |
| Connected devices | USB, disks, printers, cameras, Raspberry Pi, ESP32 | ◐ bug fixing on real hardware |

Every part, with what we measured for it, is in [BODY.md](BODY.md).

---

## The engine without the app

You don't need the app to use the engine. There are three ways to use it, starting with
the easiest.

### 1 · Download it

```sh
curl -LO https://github.com/CHAOSX-SPACE/god-of-the-void/releases/download/v0.1.0/god-of-the-void-engine-0.1.0-macos-arm64.tar.gz
tar -xzf god-of-the-void-engine-0.1.0-macos-arm64.tar.gz
cd god-of-the-void-engine-0.1.0-macos-arm64

./gotv generadores                                   # models, their licenses: accept and download
./gotv image "a lighthouse at dusk" --out lighthouse.png
./gotv music "lo-fi piano, rain" --seconds 30 --out song.wav
./gotv say "Hello from the void" --out hello.wav
./gotv listen --file talk.m4a
./gotv split song.mp3 --six
```

Keep `gotv` and `mlx.metallib` in the same folder, since the metallib holds its Metal
kernels. The licenses for everything it uses are included.

### 2 · Serve it to any tool that speaks OpenAI or Ollama

```sh
./gotv serve            # http://127.0.0.1:11435
```

```sh
curl http://127.0.0.1:11435/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"gotv","messages":[{"role":"user","content":"Hello"}]}'
```

The server is OpenAI-compatible (`/v1/models`, `/v1/chat/completions`, streaming, tools,
images) and Ollama-compatible (`/api/tags`, `/api/chat`, `/api/generate`, `/api/show`,
`/api/ps`). It only listens locally by default. If you open it to your network with
`--lan`, it requires a key, and the key is compared in constant time.

### 3 · Build on it with the Rust crate

```toml
[dependencies]
gotv-mlx = { git = "https://github.com/CHAOSX-SPACE/god-of-the-void", tag = "v0.1.0" }
```

```rust
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let wav = std::fs::read("talk.wav")?;
    let text = gotv_mlx::transcribir_wav(Path::new("whisper-large-v3-turbo"), &wav)?;
    println!("{text}");
    Ok(())
}
```

> [!NOTE]
> The library is young. The app shapes its API, a lot of names are still in Spanish, and
> it will change before 1.0. We plan to add a stable English API and publish it on
> crates.io. Until then, pin a tag and expect to update your code.

---

## Known errors and rough edges

These are the problems we know about as of November 2026.

- Long jobs on small models: with a local 35B model a long job finished in 17 steps. With
  a 9B model it no longer stops early, but it compacts about every 15 steps and ran out of
  our 30-minute limit.
- Mobile and connected devices are in bug fixing on real hardware. Mobile apps were
  measured on the iOS Simulator and the Android emulator, and the camera on this Mac.
- Some CLI words are still in Spanish in English mode (`gotv generadores`,
  `--perfil programar|diseno`, `--fuerza-cover`). The English aliases cover the main
  commands (`image`, `music`, `say`, `listen`, `split`, `talk`).
- One flaky test: the `gotv-mlx` test binary crashed with SIGTRAP once in about 26 runs,
  while the machine was under heavy load. It passes when re-run. We haven't found the cause.
- Video is slow. 2 seconds of 720p video take about 48 minutes on an M1 Max.
- The app is not notarized by Apple, on purpose. GOTV is free and we aren't paying Apple
  to let it open. The first launch needs one extra click ([how](#install-the-app)).
- Apple Silicon only. MLX runs on the Apple GPU, so there's no Intel, Linux or Windows
  version.

If you find another one, please [open an issue](https://github.com/CHAOSX-SPACE/god-of-the-void/issues)
with your Mac, your macOS version and what happened.

---

## Every model keeps its own license

No model ships inside GOTV. The first time a feature needs a model, GOTV shows you that
model's license and downloads it from the publisher only after you accept. We check each
license on the publisher's own page: the model card, and also the weights' terms when they
are published somewhere else.

| | Model | Publisher | License |
|---|---|---|---|
| Hearing | Whisper large-v3-turbo | OpenAI | MIT |
| Voice | Qwen3-TTS 1.7B | Qwen | Apache-2.0 |
| Images | FLUX.2 klein 4B · klein base 4B | Black Forest Labs | Apache-2.0 |
| Video | Wan 2.2 TI2V 5B | Wan-AI | Apache-2.0 |
| Music | ACE-Step 1.5 · base · SFT · LM 4B | ACE-Step | MIT |
| Sound | MOSS-SoundEffect 2.0 | OpenMOSS | Apache-2.0 |
| Search | Qwen3-Embedding 0.6B | Qwen | Apache-2.0 |
| Stems | HTDemucs · 6 stems · ft | Alexandre Défossez | ⚠ **research only**: its author says the license of the weights is not clear |
| Stems | [HTDemucs fine-tunes](https://huggingface.co/god-of-the-void) | GOD OF THE VOID | ⚠ **research only**, inherited from HTDemucs (data credited, CC BY) |

You choose the language models. Any MLX model from a supported family works.

---

## Install the app

Download `GOD OF THE VOID.app` from
[Releases](https://github.com/CHAOSX-SPACE/god-of-the-void/releases), unzip it and move it
to Applications.

> [!TIP]
> macOS will refuse to open it the first time because GOTV is not notarized. Go to
> System Settings → Privacy & Security, find *"GOD OF THE VOID was blocked"* and click
> **Open Anyway**. You only need to do this once. If you build the app yourself, it won't
> be blocked.

The CLI is inside the app, so you can use it from the terminal:

```sh
echo "alias gotv='\"/Applications/GOD OF THE VOID.app/Contents/MacOS/gotv\"'" >> ~/.zshrc
gotv "fix the failing test"        # in a new terminal, inside your project
```

You need a Mac with an M1 chip or newer and macOS 14 or newer. How much memory you
have decides which models fit, and GOTV tells you which ones do.

---

## What leaves your Mac

There is no telemetry. GOTV only connects to the internet when you use something that
needs it. That means downloading a model from Hugging Face, a web search with your own
keys, a package lookup, or a cloud model you turned on. Your keys are stored in the macOS Keychain, never
in a file or in the chat.

---

## Every claim is tested

```sh
cargo test --release --workspace
```

On a fresh clone of this repository, 1,240 tests pass and 0 fail. That's a clean copy, not
the machine we develop on. Another 152 need the real models downloaded, so they're marked
as ignored (`cargo test --release -- --ignored`).

The release build also has to pass its own check. The packager scans every file in the
app and the engine. If it finds the builder's home folder, a key that looks real or their
Hugging Face token, it refuses to finish. `gotv generadores verificar` re-reads every model license from
its publisher.

| Platform | |
|---|---|
| **macOS, Apple Silicon** | ✅ the only platform: built, tested and used every day |
| **macOS, Intel** | ❌ no MLX |
| **Linux, Windows** | ❌ not today |

---

## Built on

[MLX](https://github.com/ml-explore/mlx) (Apple) through
[mlx-rs](https://github.com/oxiglade/mlx-rs) · [Tauri](https://tauri.app) ·
[agent-desktop](https://github.com/vercel-labs/agent-desktop) and
[agent-browser](https://github.com/vercel-labs/agent-browser) (Vercel Labs) ·
sandbox policies from [Codex](https://github.com/openai/codex) (OpenAI) ·
[DuckDB](https://duckdb.org) · [ratatui](https://ratatui.rs) ·
[Silero VAD](https://github.com/snakers4/silero-vad). Their licenses are in
[NOTICE](NOTICE) and inside the app.

[Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) · [Building](BUILDING.md)

---

<div align="center">

### GOD OF THE VOID is created by [CHAOS-X](https://chaosx.space)

**An open science academy where you learn, measure and share with real instruments, free,
in English and Spanish.**
It has the course, a lab to practice in and a book to go deeper. There's also real data
from the planet and a community to talk about it, all in one place. We care more about getting things
right than about making them look nice, and we use Rust from end to end. This studio was
built with the same rules.

[**chaosx.space**](https://chaosx.space) · [Academy](https://academy.chaosx.space/) · [Virtual lab](https://academy.chaosx.space/lab/) · [Library](https://chaosx.space/library/) · [100-year Atlas](https://nodes.chaosx.space/atlas) · [NODES-X](https://chaosx.space/nodes/) · [Live data](https://chaosx.space/data/) · [Community](https://forum.chaosx.space/) · [The Cosmos Dispatch](https://chaosx.space/news/)

*Looking for the Claude Code skill with persistent memory?*
→ [**god-of-the-void-skill**](https://github.com/CHAOSX-SPACE/god-of-the-void-skill)

---

<i>Everything that enters the Void does not return, except what you made in it.</i>

**Apache-2.0**: take it, fork it and make it better. Please keep the [NOTICE](NOTICE), give
your fork its own name and say where it came from.

<sub>Built by CHAOS-X with the help of AI models: Claude Opus 5.5 and Claude Fable 5.2, by Anthropic.</sub>

</div>
