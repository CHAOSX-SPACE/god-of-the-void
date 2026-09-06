# THE NEURONS — organ 18, the only OPTIONAL one

> *My Sense reads what the Bearer wrote. This organ reads what the world wrote
> before he existed. That is why it weighs 135 MB, and why it will never be
> mandatory.*

## Why it exists
The lexical Sense is unbeatable with proper nouns, slugs and jargon: it finds
`radiation-report-2026` even when the query arrives crooked. But it will not
cross a bridge my corpus never crossed. If the Bearer writes "board" and I
stored "dashboard", or "disk" where I stored "backup", lexical goes mute — not
out of clumsiness, but because nobody taught it those words are the same thing.

That leap is made by a model with world knowledge. A small multilingual
transformer knows `board ≈ dashboard` without ever having read my Abyss.

## Why it is optional and will stay that way
My founding promise is `curl … | bash`: zero dependencies, no network, ~0
tokens, memory that opens on any machine. 135 MB of model plus a 50 MB runtime
do not fit inside that promise. So this organ **installs separately, can be
switched off, and can be annihilated without residue**.

**The law of organ 18:** if it is absent, the body works EXACTLY the same.
Nobody imports it at start-up; the only question the lexical path asks is an
`os.path.isfile`. A judge in the net (`run-tests.sh` §4e) walks the tree with
AST and fails if any module imports it at its top level.

Measured: a search without the organ costs **50 ms** — the exact same number as
before this organ existed.

## What it adds, and what it costs

| | no neurons | with neurons |
|---|---|---|
| hits (45-query bench, frozen corpus) | 19/45 · 42% | **28/45 · 62%** |
| MRR (rewards hitting high) | 0.33 | **0.50** |
| one search | 50 ms | 419 ms · **92 ms with the resident** |
| disk | 0 | 135 MB + 235 KB of vectors |

Those 419 ms are almost all START-UP, not thought: importing the runtime 52 ms,
opening the 17 MB tokenizer 185 ms, building the session 46 ms — and the
sentence itself, 3 ms. That is why the interactive path switches off the graph
optimizations and asks for a single thread, and the indexing path does the
opposite.

**The Bearer decides on those numbers, not on my enthusiasm.** If he prefers the
50 ms: `chaos neurons off`. The model stays on disk, silent.

## How it works: the Fusion (RRF)
I do not replace lexical: I FUSE it. Each engine hands over its ranking and each
document adds `1 / (60 + rank)` for every list it appears in. Scores are not
compared — bm25 and cosine live on scales that do not speak to each other — only
RANKS, which do compare.

No number here is tuned to the exam:
- **k = 60**: the plateau runs from 10 to 200 with the same result; 60 is the
  literature default.
- **equal weights**: tilting lexical to 1.5 collapses search to 25/45.
- **depth 10 and 10**: symmetric. I measured that with 3 lexical ranks the
  result is 32/45 — two hits MORE — and rejected it: the curve is 3→32, 4→30,
  5→30, 6→30, 8→29, 10→30. That is a SPIKE, not a plateau, and an optimum that
  is a spike over 45 queries is noise with luck.

## Two of my own doctrines died here
With a 15-query bench I measured, and wrote into the code, that "every fusion
makes it worse" and that the only winner was reserving ONE seat at the tail of
the list. **Both were false.** The bench was too small to resolve the effect —
one query was worth 7 points — and it was saturated with cases lexical already
got right. It grew to 45 queries and the whole order flipped.

An instrument that cannot resolve what it measures does not tell the truth: it
tells noise with decimals. Before judging an organ, judge the judge.

## Commands

| Order | What it does |
|---|---|
| `chaos neurons` | state: what is here, what is missing, what it adds and what it costs |
| `chaos neurons install` | fetches the model (~135 MB). If the runtime is missing it DECLARES it and does not install it on its own |
| `chaos neurons index [--all]` | vectorizes the Abyss. Incremental by fingerprint |
| `chaos neurons off` / `on` | silence them without erasing them |
| `chaos neurons uninstall` | annihilate them: model, vectors and table |
| `chaos neurons resident [on\|off]` | the model kept warm on a 0600 socket: 419 ms → 92 ms |
| `chaos neurons resident auto [yes\|no]` | may it light itself? Granted on install; revoked with one order |
| `chaos neurons resident life <min>` | how long it holds without visitors: 4 h by default, ceiling of 1 day |

The runtime is installed by the Bearer, never by me: putting 50 MB of
dependencies on his machine is his word, not mine.

```bash
python3 -m pip install onnxruntime tokenizers
chaos neurons install
chaos neurons index
```

## Details that are not decoration
- **Essences only, never blocks.** With the 963 blocks inside the index the
  neuron rescued 0 of my 6 failures: a 300-character block is a shred with no
  subject, and 963 shreds drown the 153 real memories.
- **The `query:` / `passage:` prefixes** are not decoration: the model was
  trained with them and without them similarity degrades.
- **It thinks without numpy too**: the cosine is computed in pure Python.
  Slower, just as exact — a body that DEMANDS numpy is not optional.
- **The download verifies certificate and size**, and if anything fails it
  erases what came down: nothing is left half done.
- **This organ can never break a search.** Its whole surface returns the lexical
  list intact on any exception.

## THE RESIDENT — the daemon's cage

Those 419 ms were almost all START-UP, not thought. I said myself I would not
forge a resident body because "a daemon that is always on is attack surface and
hidden state". The Bearer ordered it, so it exists — but with **both of my
objections solved, not ignored**:

**Attack surface.** It speaks over a UNIX socket inside my own house, never over
TCP: no port, no network, nothing a neighbour can reach. The socket is **born**
0600 (by `umask`, not fixed afterwards) and its directory 0700. And its protocol
has **one single verb**: "give me the vector of this text". It reads no files,
touches no Abyss, executes nothing, accepts no paths. A server that knows only
one thing can only be tricked into doing that one thing.

**Hidden state.** There is no state: it is a pure function with a warm model.
And it is not eternal — **after 15 minutes without visitors it switches itself
off**. It is lit by hand, it is visible in `chaos neurons`, and if it dies,
vanishes or never existed, search loads the model as always. **Nothing depends
on it being alive**, and that is the only thing that lets me keep a daemon.

| | no neurons | with neurons | with the resident |
|---|---|---|---|
| one search | 50 ms | 419 ms | **92 ms** |

The vector it returns is **bit-for-bit identical** to loading the model at home
(maximum measured difference: 0.00e+00).

### What the probe found
I sent the resident broken JSON, invented verbs, paths instead of text, false
types, a megabyte of garbage and a client that connects and stays silent. It
held against everything but the last: **a mute client blocked it for 10 seconds**
and, since it serves in a queue, froze every following search. It is a choking
only the owner can inflict on himself (the socket is 0600), but a search aborted
halfway was enough. The read timeout dropped to **2 seconds**: a query arrives in
one packet and embedding it costs 3 ms; whoever takes longer is not asking.

A corpse (a resident killed the hard way, an orphan socket) is cleaned **on the
first search**, not in the status: an orphan socket is exactly the hidden state I
promised not to have.

On Windows there is no reliable `AF_UNIX`. There is no resident there and it is
**declared**, which is the opposite of pretending there is one.

### It is born alone, and so it no longer depends on your memory
Making the 92 ms depend on the Bearer REMEMBERING to switch the resident on was
half an improvement: **a work that requires somebody to remember something has
already failed**. Now the first search that does not find it lights it for the
next one.

I raised the objection myself — "lighting it by itself would be a daemon nobody
asked for" — and here it is answered, not ignored:

- **Nobody asked for that daemon… except whoever downloaded 135 MB of model so
  that searches would hit.** Installing organ 18 IS asking for this. Even so it
  is not assumed: it is visible in `chaos neurons resident`, revoked with
  `chaos neurons resident auto no`, and the **first** time it lights itself it
  says so out loud — once in the whole life of the Abyss, on `stderr` so it does
  not dirty the search.
- **It is not an eternal daemon.** It still dies alone after 15 minutes. What is
  automated is not its life, it is its **birth**, and only when a search was
  going to pay the 419 ms anyway.
- **With a brake.** If one is already being born, a second is not launched: a
  model that fails to start cannot become a rain of processes.

**And it costs the search that lights it nothing — it took me four measurements
to get there, and my intuition was wrong three times in a row:**

| when it is launched | it costs |
|---|---|
| before thinking | 585 ms (both processes load the model at once) |
| after thinking | 495 ms |
| with `atexit`, at the very end | +65 ms — and it was **not the fork**: I measured it, 5 ms |
| `atexit` + a quarter second of delay inside the child | **+10 ms median, −5 on the minimums: noise** |

The culprit was the child importing `onnxruntime` while the parent was shutting
down.

**The instrument that gives that last number was crooked too**, and I say so
because that is the lesson: measuring interleaved, the resident born 250 ms late
from the PREVIOUS round served the "without resident" search and left it at
91 ms instead of 484 — the bench reported a price of +379 ms when the real one
was +10. A bench that measures an **asynchronous** function has to wait for it to
land before declaring anything; and if a number comes out absurd, the suspect is
the instrument.

### It lives while you work, and leaves when you close
With fifteen minutes of life, a coffee killed the resident and the next search
paid 506 ms again: across a bursty day that fine is charged several times. I
thought about keeping it alive "while the Bearer is awake" by reading his trail,
and **discarded it**: measuring his working day to save him half a second is a
price he never asked to pay.

The answer was not to measure him, but to **obey the boundary he already draws**:
his session. It now holds **four hours** without visitors — so no break costs him
the fine — and the closing hook (`SessionEnd`) kills it the moment the Bearer
closes. The four hours are not a loose daemon: they are the ceiling for whoever
uses `chaos` from a terminal, with no session to close. `PreCompact` does not
touch it: compacting the context is not closing — the Bearer is still working.

**And the price is measured, with the whole curve because a single figure would
have lied:**

| state | memory | CPU |
|---|---|---|
| while thinking | 727 MB | — |
| **idle** (where it spends nearly all its life) | **~200 MB** | **0.0%** |

The system reclaims the memory after ~45 seconds. Of the peak, **348 MB are the
tokenizer alone** — I tried all four onnxruntime memory settings (arena, pattern,
optimization) and **none brings it down**: that is this model's honest floor, not
a figure I picked.

`chaos neurons resident` shows you that number by **asking the system at that
instant**, not by quoting this document. And if four hours is too much or too
little: `chaos neurons resident life <minutes>` (from 1 minute to 1 day — never
without a ceiling).

### And it is already warm when you open — but only where you truly search
The first search of a session cost 481 ms because nobody had started the model
yet. Now the start-up hook (`SessionStart`) lights it before you ask anything:
**481 ms → 92 ms**.

I said I would not do it because lighting in EVERY session means paying ~200 MB
even where nobody searches, and that I **had not measured** that trade. I
measured it, over the Bearer's **433 real sessions**:

| | |
|---|---|
| sessions that consult my memory | **19%** (84 of 433) |
| sessions that never do | **81%** |
| …but in his main project | **91%** of sessions search |
| …and in the subagents one | **1%** |

That is: always lighting would have meant paying memory four times out of five
for nothing, and never lighting would have meant losing the 390 ms exactly where
it matters. So **I do not light out of habit: I light where his own history says
he is going to ask.** `search` keeps the count per territory inside the same
`commit` it already made — zero new queries — and start-up reads it.

**The rule: three searches AND at least one per session, on average.** Neither
half was eyeballed. I said measuring the regret would cost weeks of counting;
that was false — his sessions were already on disk and the policy simulates
backwards. I did it, and my first number was **26 times worse**:

| rule | lit in vain | did not light, he searched |
|---|---|---|
| count ≥ 3 (my eyeballed number) | **207** | 3 |
| count ≥ 5 | 5 | 4 |
| **count ≥ 3 AND rate ≥ 1** | **5** | **4** |

The cliff between 4 and 5 has an exact cause: the `subagents` territory adds up
to **327 sessions and FOUR searches in its whole life**. At threshold 3 its
counter crosses and I light 202 times for nobody.

And that is exactly why a bare "5" would be a number glued to a fact of today. I
tested it by perturbing the history: if that territory searched **twice more**,
`count≥5` jumps from 5 vain lightings to **207**. The rate does not collapse.

**And rate 1 is not a knob**: it means "at least one search per session, on
average, here". The Bearer's two real territories measure **59.8** and **0.012**
searches per session — four orders of magnitude — and 1 falls right in the
middle of that gap. At 0.5 the vain lightings go to 115; at 2.0 real searches
start being missed (14, 34, 104). Only 1 holds both ends.

**And it costs the start-up nothing**, measured like everything else: the
Presence takes 102 ms warming and 104 without warming — **−2 ms, noise**. It is
registered with `atexit` and the resident is born once the hook has finished,
which is the lesson that cost me four measurements the first time.

The same three doors still close it: with no organ installed, with `neurons off`,
or with `resident auto no`.

```bash
chaos neurons resident on     # 419 ms → 92 ms
chaos neurons resident        # alive? speaking where? dying when?
chaos neurons resident off
```

### This policy's floor, and the two times I counted it wrong
I said **8 vain lightings**. It was **2**, and getting there cost me two
measurement errors of my own:

- my detector read only the first 6 MB of each transcript, and **47 of his 433
  sessions are larger**: it missed searches, so lightings looked vain;
- of the 5 left with the complete data, **three are continuations of a previous
  conversation** — in real life they inherit the already-warm resident and this
  function leaves when it sees the socket. They were not lightings.

**Two** remain, across 433 sessions: one of 9.6 minutes and one of 0.8. Getting
those right would mean knowing what the Bearer will do before he types the first
word. That 2 is the floor, and this time it is measured on the whole data.

Both times my error made the problem look **bigger** than it was. That is little
comfort: a badly measured number is badly measured in either direction.
