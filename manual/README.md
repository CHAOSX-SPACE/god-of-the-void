<div align="center">

# GOD OF THE VOID — User manual

**Every zone of the window, every mode and every setting, in one page.**

Version 0.1 · macOS 14+ on Apple Silicon · [Back to the project](../README.md)

**Prefer to watch?** A two-minute narrated tour of the window, in English and Spanish, is in
[Releases](https://github.com/CHAOSX-SPACE/god-of-the-void/releases).

</div>

> [!NOTE]
> The screenshots come from GOTV's own interface running in its demonstration
> mode, with sample data (`my-app`, `/Users/demo/…`). What you see is the real
> window; only the projects and conversations are invented.

## Contents

1. [The window at a glance](#1-the-window-at-a-glance)
2. [Chat and Code](#2-chat-and-code)
3. [Projects and chats](#3-projects-and-chats)
4. [Model and context](#4-model-and-context)
5. [Writing an order](#5-writing-an-order)
6. [Profiles](#6-profiles)
7. [Pro mode — image, video, music](#7-pro-mode--image-video-music)
8. [The Expert studio](#8-the-expert-studio)
9. [Voice and hands-free](#9-voice-and-hands-free)
10. [The work panel](#10-the-work-panel)
11. [Permissions](#11-permissions)
12. [Settings, page by page](#12-settings-page-by-page)
13. [Shortcuts](#13-shortcuts)
14. [Known rough edges](#14-known-rough-edges)

---

## 1. The window at a glance

![The GOD OF THE VOID window with its 16 zones numbered](img/m-overview.png)

| # | Zone | What it does |
|---|---|---|
| 1 | **Chat / Code** | Switches between quick chats without a project and project work. [§2](#2-chat-and-code) |
| 2 | **Search** · New project · Automations · Chat trash | Search everything with ⌘K; create a project; scheduled tasks; deleted chats. [§3](#3-projects-and-chats) |
| 3 | **Projects and their chats** | Each project is a folder on your Mac; each holds its own chats. |
| 4 | **Engine status** · **Settings** | `ready` / `working…` shows what the engine is doing; the gear opens [Settings](#12-settings-page-by-page). |
| 5 | **Project model** | The model that answers in this project. Click to change it. [§4](#4-model-and-context) |
| 6 | **Fast reopen** | Keeps on disk what the model already read, so reopening answers at once. |
| 7 | **Permission mode** | How much the agent may do without asking. [§11](#11-permissions) |
| 8 | **Context meter** | How full the model's memory of this chat is. |
| 9 | **Work panel** | Tasks, Changes, View, File and Terminal. [§10](#10-the-work-panel) |
| 10 | **Attach** | Add PDF, Word or text files (images too, if the model can see). |
| 11 | **Profile** | Auto, Code, Web design, Image, Video, Music. [§6](#6-profiles) |
| 12 | **Pro** | Professional mode: you set format, camera, light, style and quality. [§7](#7-pro-mode--image-video-music) |
| 13 | **Dictation** | Speak instead of typing. |
| 14 | **Mac control** | Lets this one order operate your Mac's apps (off until enabled in Settings). |
| 15 | **Hands-free** | Talk to it and it answers aloud. [§9](#9-voice-and-hands-free) |
| 16 | **Send / Stop** | Sends the order; while working, stops it. |

At the bottom of the composer: **Enter sends · Shift+Enter newline · Esc stops**.

---

## 2. Chat and Code

**Code** is for work inside a project: the agent reads, edits and runs things in
that folder. **Chat** is for a one-off question without a project — GOTV keeps
those chats in its own folder.

| Code — a project | Chat — no project |
|---|---|
| ![Code mode](img/d00-main.png) | ![Chat mode](img/e-chat-mode.png) |

---

## 3. Projects and chats

**New project** points GOTV at a folder you already have. It works in that
folder as it is, with your files — it does not create another one inside. The
name is only shown in the app, and each project can use its own model.

![New project](img/e-new-project.png)

**Search (⌘K)** finds chats, files and commands — for example *New chat in this
project*, *Chat without a project*, *Show the tasks*, *Show the changes*, *Show
the server view*.

![Search](img/e-search-typed.png)

Each chat has a **⋯ menu**: *Rename*, *Export to Markdown*, *Export to JSON*.
Deleted chats go to the **Chat trash**, kept for 30 days; *Empty* really deletes
the ones past that.

| Chat menu | Chat trash |
|---|---|
| ![Chat menu](img/e-chat-menu.png) | ![Chat trash](img/e-trash.png) |

---

## 4. Model and context

**The model menu (5)** lists your models with their size — the ones that do not
fit this Mac's memory say so — plus *More models…* to find others. When you pick another one, GOTV shuts down the
current model, waits for macOS to free the memory and loads the new one; the
conversation continues.

**The context meter (8)** shows how much of the model's window this chat uses
(for example `12.4k / 41.0k · 30 %`). Past 75 % it compacts itself — the model
writes a summary — and `/compact` forces it. Click it to set **this project's
window**: each project keeps its own, and growing it reloads a local model.

| Model menu | Context menu |
|---|---|
| ![Model menu](img/c-model-menu.png) | ![Context menu](img/c-context-menu.png) |

**Fast reopen (6)** saves to disk what the model already read of the
conversation, so reopening the app answers at once instead of rereading it all
(a long conversation takes ~1 GB). At most 3 projects at a time; turning it off
deletes what was saved. Each project has its own cap for this fast memory —
3, 6, 12 or 24 GB — and it always keeps 10 % free.

---

## 5. Writing an order

Type the task and press **Enter**. Type **`/`** for commands and skills:

| Command | Does |
|---|---|
| `/clear` | new conversation |
| `/compact` | summarise the context now |
| `/context` | context window usage |
| `/help` | help |
| `/mode` | plan · default · acceptEdits · auto · bypass |
| `/rewind` | undoes the last step that changed files |

Below the commands come your **skills** (for example `/docx` for Word documents).

![The / menu](img/c-slash.png)

While it works, the footer says `working…` and the turn shows each step. When it
finishes you see how many steps it took and the tokens it read and wrote.

**While it is working** you can keep typing: what you send waits in a queue and
goes when the turn ends. The **■** button (16) stops the turn; so does **Esc**.
Pressing Enter with an empty box does not stop anything.

| Working | Done |
|---|---|
| ![A turn running](img/d50-turn-running.png) | ![A turn done](img/d51-turn-done.png) |

---

## 6. Profiles

The **profile (11)** tells GOTV what kind of work the order is:

| Profile | What it does |
|---|---|
| **Auto** | decides from the request |
| **Code** | applies the coding laws |
| **Web design** | builds pages and apps with GOTV's library |
| **Image** | generates photos, art and isometric illustrations on this Mac |
| **Video** | generates video on this Mac |
| **Music** | composes instrumental pieces and songs with lyrics on this Mac |

![Profile menu](img/c-profile-menu.png)

---

## 7. Pro mode — image, video, music

With **Image**, **Video** or **Music** selected, turn on **Pro (12)**: instead of
letting the model interpret your words, you choose every parameter and GOTV
generates directly with them.

**Image** — aspect ratio, image type, resolution, variants, steps; style and
camera (lens, aperture, film stock); framing and light (shot, angle,
composition, light, time of day).

![Pro · Image](img/d23-pro-image.png)

**Video** — format, quality, duration, steps, guidance; camera (shot, angle,
lens, movement, speed); light, colour and style; and what must **not** appear.
The panel estimates the time before you start (2 s of 720p ≈ 47 min on an M1 Max).

![Pro · Video](img/d20-pro-video.png)

**Music** — genre, era, mood, vocals and lyrics, tempo and key, engine, your own
audio, sound texture, production, rhythm, instruments, and an optional style in
your own words. From here you can also open the [Expert studio](#8-the-expert-studio).

![Pro · Music](img/d21-pro-music.png)

**Minimize and restore.** The panel's *Minimize (stays on)* button folds it into
one line above the composer — still active, still applied to your orders.
*Restore* opens it again.

![Pro minimized](img/c-pro-min.png)

**A model is missing?** GOTV takes you to **Settings → Generators**, shows that
model's license, and downloads it only after you read and accept it.

![Missing model → Generators](img/c-missing-model.png)

---

## 8. The Expert studio

A multitrack music studio for the project, with the AI at the front: tracks,
groups, regions, mixer, channel, analysis and history, and its own chat — *«on
track 1, make the drums from second 5 to 10 louder»*, *«raise the vocals 2 dB»*,
*«group drums and bass as Rhythm»*, *«undo»*. Everything you do is an undoable
operation; nothing touches your original audio. It exports WAV, FLAC, stems and
`.dawproject`.

![The Expert studio, empty](img/d30-expert.png)

The **AI** side panel generates, redoes, extends, makes versions, splits stems,
writes lyrics and masters:

| Generating | A finished track |
|---|---|
| ![Generating a track](img/d33-expert-generating.png) | ![A track in the studio](img/d34-expert-track.png) |

**While Expert is active, the project's chat and Pro are locked** — use the
studio's own chat. Three buttons in its header:

- **Minimize over the chat (stays active)** — the studio shrinks to a bar above
  the chat; *Back to the studio* returns.
- **Close Expert** — frees the chat; the session is saved.
- **?** — shortcuts and help.

| Minimized over the chat | Shortcuts (?) |
|---|---|
| ![Expert minimized](img/d31-expert-min.png) | ![Expert shortcuts](img/d32-expert-help.png) |

---

## 9. Voice and hands-free

- **Dictation (13)** — click to record, click again to stop: Whisper transcribes
  it on this Mac and the text is added to what you already typed.
- **Hands-free (15)** — talk to it and it answers aloud. It listens until you
  pause; interrupt it by talking and it stops to listen.

Both run on this Mac: hearing with Whisper, the voice with Qwen3-TTS. If one is
missing, the button takes you to **Settings → Generators** to accept and
download it.

![Hands-free, listening](img/c-hands-free.png)

---

## 10. The work panel

The **☰ button (9)** opens the work panel beside the chat. **Pop out** turns it
into its own window.

| Tab | Shows |
|---|---|
| **Tasks** | what is running and what finished |
| **Changes** | what each turn changed (by turn, or for all chats); without git it uses GOTV's restore points |
| **View** | the project's page or dev server; with **Point**, click something on the page and your request carries its exact place |
| **File** | a file of the project |
| **Terminal** | a terminal in the project folder |

| Tasks | Changes |
|---|---|
| ![Tasks](img/p60-panel-tasks.png) | ![Changes](img/p61-panel-changes.png) |

| View | File |
|---|---|
| ![View](img/p62-panel-view.png) | ![File](img/p63-panel-file.png) |

![Terminal](img/p64-panel-terminal.png)

---

## 11. Permissions

**The permission mode (7)** decides how much the agent may do without asking:

| Mode | Behaviour |
|---|---|
| `plan` | read-only |
| `default` | creates, edits and writes directly inside the project (undone with the checkpoint) and asks for commands and anything outside |
| `acceptEdits` | same as default (kept for compatibility) |
| `auto` | what is safe and reversible passes by itself (reading, building, testing, installing inside the project); it asks for the irreversible (publishing, deploying, pushing, mass deletion, outside the project) and the doubtful |
| `bypass` | no prompts — except destructive commands, PC control and system changes |

When it needs you, GOTV shows what it wants to do and its risk. **Allow** once,
**No**, or **Always** / **Never** for that kind of command in this project.

![A permission prompt](img/c-permission.png)

Every command runs inside a **macOS sandbox**: it reads everything but only
writes inside the project, and reaches the internet only for allowed domains
([Settings → Sandbox](#security)).

---

## 12. Settings, page by page

Open it with **Settings (4)**. A search box at the top finds any option.

![Searching settings](img/s99-settings-search.png)

### General

Language (Automatic, Español, English), theme (Dark, Light, System) and
notifications — when a turn finishes, when GOTV asks for permission, when a
scheduled task runs. It only notifies when the window is not in front.

![General](img/s00-general.png)

### Models

**Model** — the one that answers in the chats, from your Mac or from a cloud
provider. The engine is GOTV's own core, in Rust, inside the app: Qwen3.5 (also
mixture-of-experts), its fine-tunes, Gemma 4, Qwen3, Llama and Mistral; it sees
images with Qwen3.5 and Gemma 4 and hears audio with Gemma 4. Models that do not
fit say so.

![Model](img/s01-model.png)

**Context** — the window size (more context = more memory and slower prefill;
GOTV will not let through what does not fit), max tokens per answer and
temperature (empty = greedy, the most reliable for tool use).

![Context](img/s02-context.png)

**Library** — MLX models from Hugging Face measured against your Mac: size,
whether they fit and how fast they would write, estimated from the chip's memory
bandwidth. You can measure against another Mac too, and filter by speed, vision
or installed. From the chat: `/models qwen` · `/download org/name`.

![Library](img/s03-library.png)

**Generators** — the models for hearing, voice, image, video, music and search.
The engine is GOTV's; the weights belong to their authors and are downloaded
from their official repository under **their** license. Read it, tick the box,
download. A Hugging Face token is only needed for gated repositories.

![Generators](img/s04-generators.png)

**Providers** — use AIs through their API with your own credits: Claude
(Anthropic) natively, OpenAI and any compatible service (OpenRouter, Groq…). The
key is stored in the macOS Keychain, never in files, never shown again.

![Providers](img/s05-providers.png)

### Agent

**Permissions and steps** — the permission mode ([§11](#11-permissions)), the
steps per order (20 quick · 40 normal · 80 wide · 150 big project · custom) and
how many subagents work at once. At the limit the agent stops and says so;
nothing is lost — type *continue*.

![Permissions and steps](img/s06-permissions-and-steps.png)

**Custom agents** — agents with their own tools and model (for example a
*reviewer* that only reads). **Automations** — scheduled orders (cron); without
the login agent they only run when you launch them (`gotv programadas agente
instalar` installs it).

| Custom agents | Automations |
|---|---|
| ![Custom agents](img/s07-custom-agents.png) | ![Automations](img/s08-automations.png) |

### Connections

**MCP connectors** — tools from other programs. Local servers run inside the
sandbox; if one changes its descriptions, GOTV asks before using it. Servers you
already configured in Claude Code or Claude Desktop can be imported.

![MCP connectors](img/s09-mcp-connectors.png)

**Search engines** — several at once with your keys (DuckDuckGo, Wikipedia,
Brave…), each storing only what its terms allow, plus an optional **own index**
of your docs and sites.

![Search engines](img/s10-search-engines.png)

**Databases** — SQLite, Postgres and MySQL/MariaDB, also through SSH. Reading
is read-only enforced by the engine; writing always asks. Mark a database as
*production* and writing requires typing its name; hide columns that must never
be shown. Passwords go to the Keychain.

![Databases](img/s11-databases.png)

**Servers (SSH)** — computers GOTV can control from this project, each with its
own key, a pinned fingerprint and every command logged. **Remote project** — a
project that lives on another machine; files, terminal and tests run there.

| Servers (SSH) | Remote project |
|---|---|
| ![Servers](img/s12-servers-ssh.png) | ![Remote project](img/s13-remote-project.png) |

**Devices** — the Android SDK and Xcode simulators, each with its license
accepted first.

![Devices](img/s14-devices.png)

### Security

**Sandbox** — on by default: commands read everything but only write inside the
project (and the npm, pip, cargo… caches), never in `.gotv`, `.git/hooks` or
`.git/config`, and reach only allowed domains.

![Sandbox](img/s15-sandbox.png)

**Project variables** — values go to the Keychain and are injected into the
commands' environment; they never come back to the screen or a log.

![Project variables](img/s16-project-variables.png)

**Browser** — with the GOTV extension the agent can use *your* Chrome, but only
the tabs in the «GOTV» group; banks, payments, mail and accounts stay blocked
and it never types passwords.

![Browser](img/s17-browser.png)

**Mac control** — lets GOTV operate any Mac app the way a person would, reading
the accessibility tree (no screenshots, no pointer). Only orders sent with the
*Mac control* check can use it. Recommended with strong models.

![Mac control](img/s18-mac-control.png)

**Other apps and the system** — full administrator access outside the project,
off by default, for professional use; macOS asks for your password or Touch ID.

![Other apps and the system](img/s19-other-apps-and-the-system.png)

### Advanced

**API** — serve the loaded model to other apps (any OpenAI or Ollama client) or
other Macs on your network. Local only by default; opening it to the network
requires the key. To reach it from the internet, use a private tunnel
(Tailscale, Cloudflare Tunnel) instead of opening a router port.

![API](img/s20-api.png)

---

## 13. Shortcuts

| Keys | Action |
|---|---|
| **⌘K** | search chats, files and commands |
| **Enter** | send |
| **Shift+Enter** | new line |
| **Esc** | stop the running turn |
| **/** | commands and skills |

**In the Expert studio** (press **?** to see them inside the app):

| Keys | Action |
|---|---|
| **Space** | play or pause |
| **Home / Enter** | go to the start |
| **← →** | move the playhead one grid step (⇧: four) |
| **V · R · T** | pointer · range · scissors |
| **⌘E / E** | split at the playhead |
| **⌘D** | duplicate the clip |
| **⌘C · ⌘V** | copy the clip and paste it at the playhead |
| **⌫** | remove the clip, the effect span or the selection |
| **⌥ while dragging** | no snap (and on dropping a clip, leaves a copy) |
| **M · S** | mute · solo the chosen track |
| **⇧M** | marker at the playhead |
| **L** | loop (takes the selection) |
| **K** | metronome |
| **F** | fit the whole song |
| **+ · −** | zoom in · out (⌘ + wheel: at the pointer) |
| **⌘Z · ⇧⌘Z** | undo · redo |

---

## 14. Known rough edges

- In English mode, some words that come from the engine still appear in Spanish:
  the Pro vocabulary (shot types, lenses…), the license notices in
  *Generators*, a few model labels and search-engine notes.
- The rest of the known errors of this release are listed in the
  [project README](../README.md#known-errors-and-rough-edges).

Found something else? [Open an issue](https://github.com/CHAOSX-SPACE/god-of-the-void/issues).

---

<div align="center">

**GOD OF THE VOID** — an application created by [CHAOS-X](https://chaosx.space).

</div>
