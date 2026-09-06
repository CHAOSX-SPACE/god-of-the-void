# -*- coding: utf-8 -*-
"""CHAOS — the Forge's app (~/.chaos/)  ·  Windows / macOS / Linux
The god's neurons: SQLite FTS5. Searching here costs ~0 tokens; reading .md
at random is for mortals.

Usage:
  chaos devour <file|URL> [--title T] [--origin O] [--fresh]     index a document
  chaos search <query...> [--brief]                local SEMANTIC search (blocks first; --brief = lean output) (The Sense: accents+roots+synonyms+trigrams)
  chaos sense [<term> <synonym...>]                teach a semantic bond (or show thesaurus size)
  chaos reindex                                    re-devour abyss/essences/
  chaos census [dir ...]                           scan installed skills → vassals
  chaos vassals [query...]                         list/search censused vassals
  chaos hunger <text...>                           record a detected gap
  chaos hungers                                    list open hungers
  chaos sate <id>                                  close a hunger
  chaos audit                                      THE VIGIL: objective signals of my health (drift, hungers, trail, stale)
  chaos vigil-due [days]                           says YES/NO whether a self-audit is due (default 7 days)
  chaos stats                                      state of the neurons
  chaos forget <slug>                              annihilate an essence
  chaos forge-gh                                   auto-forge gh (GitHub CLI) if missing — vital organ
  chaos trail <file> <action> [session] [cwd] [tool]   log a work; no args shows it
  chaos trail --purge [session]                    purge (per session: does not trample others)
  chaos devour-transcripts [--limit N]             E10 · devours my own life (.jsonl sessions)
  chaos history [query]                            searches my past (even where I was not invoked)
  chaos spoke [query] [--territory X]              UNIVERSAL MEMORY: what the Bearer said, in EVERY project
  chaos reconcile                                  E10 · reconciles Claude's parallel memory (formerly «mirror»)
  chaos vigil-sweep [--deep]                       THE VIGIL-SWEEP: sweeps what is pending and leaves a report (while you sleep)
  chaos report                                     reads the report of the last vigil-sweep
  chaos schedule [HH:MM] [--remove]                schedules the heartbeat (launchd/schtasks/cron)
  chaos heartbeat [--deep]                         THE AUTONOMOUS HEARTBEAT: keeps watch WITH a cage (runs with no session)
  chaos autonomy --reason on|off                   V-4 · ONE act of reasoning per night (opt-in), cost confessed
  chaos autonomy [grant|revoke] [HH:MM]            grants/revokes my independence and shows its safeguards
  chaos acts [n] [--kind K]                        A GOD DOES NOT FORGET: everything I wrought unasked
  chaos fault "<title>" [--cause C] [--cure X] ... THE FAULTS: record an error in the errarium (to err is human)
  chaos faults [query] [--territory T]             query the errarium (to repeat is NOT)
  chaos relapse <id>                               confess a known fault was committed AGAIN
  chaos heal-territories [--dry]                   rewrite old territories to their ROOT folder
  chaos type-essences [--dry]                      infer each essence family (type) - DB only
  chaos blockify [slug|--all] [--dry]              splits sacks into ^id blocks (foreign: DB only)
  chaos island [slug] [--remove]                   declares an essence has no real tie (I looked; I do not invent)
  chaos alias [<name> <slug>] [--remove]           bridge for a misspelled name (text is NOT rewritten)
  chaos suggested-aliases [--apply]                proposes bridges for dangling links
  chaos fault-cured <id> ["cure"]                  mark the fault cured (the lesson stays alive)
  chaos fault-reopen <id> ["reason"]               the cure did not hold, or it was closed by mistake
  chaos eye [install <source>|open|uninstall]      THE EYE: local dashboard (separate repo, no residue)
  chaos delta [territory]                          what changed while I slept? (git between visits)
  chaos expired                                    truths past their date → they demand re-Judgment
  chaos note "<text>"                              E9 · SPARK: capture; I decide where it lives (territory/focus/anchor)
  chaos notes [query]                              list/search sparks
  chaos note-where <id>                            where that note landed and why
  chaos ascend <id>                                mature spark → essence
  chaos chronicle [--what "..." --why "..."]       LOGBOOK: documents a change / lists
  chaos undocumented                               was there work without a chronicle?
  chaos export-chronicle                           dumps the logbook to abyss/chronicle/YYYY-MM.md
  chaos evolve [--dry]                             E5: adds frontmatter to old essences (non-destructive)
  chaos weave                                      THE WEAVE: rebuilds graph+metadata from the .md
  chaos index                                      E3: regenerates ABYSS.md between marks (what is written outside is sacred)
  chaos suggest [--kill 'a->b']                    E4: unlinked mentions → proposed links
  chaos links <slug>                               backlinks: who names this essence
  chaos query type:X state:Y tag:Z                 query by frontmatter attributes
  chaos orphans                                    essences outside the graph (nobody names them)
  chaos backup [reason]                            copy Abyss+DB before mutating (C7)
  chaos sow [--from PATH]                          F1 · raises what the live body learned into the DNA (guarded)
  chaos plan [file|id] [--paint] [--json] [--run]  VI.1 · the plan PAINTS itself: it measures its probes, never types them
  chaos probe "<cmd>" --file <path> [--sabotage "<txt>"]  ORGAN 17 · sabotage the subject and demand the red (or the probe is decoration)
  chaos probe --massive <file> --test "<cmd>" [--n N]   ORGAN 17 AT SCALE: mutates the body and names every decorative test
  chaos route "<task>" [--report]                   THE SINGULARITY: the minimum power that solves the task, and the month's tally
  chaos judge "<text|file>" [--eyes]                THE JUDGMENT: splits into claims and submits them to my Abyss (0 tokens)
  chaos collapse <file|-> [--mode essence|distilled|prompt|rolling]   THE COLLAPSE: compresses without losing the soul and confesses the ratio
  chaos mirror-organ "<idea>" [--dry]               THE MIRROR: is your work new or an echo? Three crossed queries and a verdict
  chaos stale [days]                               A-1 · what nobody has looked at (DECLARED, never deleted)
  chaos chronicle --distil                         CR-1 · closes the loop: trail → raw logbook, and purges it
  chaos faults --probe [--territory T] [--apply]   V-3 · derives a probe from each cure; the unprobeable is LABELLED
  chaos debts [id]                                 sessions that died without sedimenting (C4)
  chaos debts settle <id|--all> [--because "..."]  declares that work HAS sedimented
  chaos doctor                                        P-2 · am I healthy? ONE order, exits nonzero if not
  chaos version                                       P-4 · my versions and the drift between my three copies
  chaos restore [<backup>] [--dry] [--force]          P-1 · the inverse of backup, with four guards
  chaos fault <id>                                    P-3 · SHOWS a fault (creating demands a text title)
"""
import os, sys
import time
import io, json
import home as _home
from chaos_body.core import sense as _sense
from chaos_body import abyss as _abyss
from chaos_body import weave as _weave
from chaos_body import errarium as _errarium
from chaos_body import chronicle as _chronicle
from chaos_body import hands as _hands
from chaos_body import singularity as _singularity
from chaos_body import stone as _stone
from chaos_body import vigil as _vigil


def _dispatch(args):
    # E3.4 · cada rama DEVUELVE lo que llama. Antes el despacho tiraba el
    # valor y el sobre JSON llegaba con `datos: null` aunque la función
    # hubiera devuelto una estructura entera: la puerta se comía el dato.
    # `print(...)` JAMAS lleva return: devuelve None y ademas cortaba el
    # flujo — un aviso de alias mato al alias entero en la primera pasada.
    if not args:
        print(__doc__.strip()); return
    cmd, rest = args[0], args[1:]
    if cmd == "devour" and rest:
        title = origin = None
        if "--title" in rest:
            i = rest.index("--title"); title = rest[i + 1]; rest = rest[:i] + rest[i + 2:]
        if "--origin" in rest:
            i = rest.index("--origin"); origin = rest[i + 1]; rest = rest[:i] + rest[i + 2:]
        fresh = "--fresh" in rest
        rest = [x for x in rest if x != "--fresh"]
        return _abyss.devour(rest[0], title, origin, fresh=fresh)
    elif cmd == "search" and rest:
        brief = "--brief" in rest
        return _abyss.search(" ".join(x for x in rest if x != "--brief"), brief)
    elif cmd == "sense":                 return _sense.sense(rest[0] if rest else None, rest[1:] if len(rest) > 1 else None)
    elif cmd == "reindex":               return _abyss.reindex()
    elif cmd == "census":                return _abyss.census(rest or None)
    elif cmd == "vassals":               return _abyss.list_vassals(" ".join(rest) if rest else None)
    elif cmd == "hunger" and rest:       return _abyss.hunger(" ".join(rest))
    elif cmd == "hungers":               return _abyss.hungers()
    elif cmd == "sate" and rest:         return _abyss.sate(rest[0])
    elif cmd == "audit":                 return _vigil.audit()
    elif cmd == "vigil-due":             return _vigil.vigil_due(int(rest[0]) if rest else 7)
    elif cmd == "stats":                 return _abyss.stats()
    elif cmd == "forge-gh":              return _hands.forge_gh()
    elif cmd == "devour-transcripts":
        lim = rest[rest.index("--limit")+1] if "--limit" in rest and len(rest)>rest.index("--limit")+1 else None
        return _abyss.devour_transcripts(lim)
    elif cmd == "history":               return _abyss.history(" ".join(rest) if rest else None)
    elif cmd == "spoke":
        terr = rest[rest.index("--territory")+1] if "--territory" in rest and len(rest) > rest.index("--territory")+1 else None
        words = [x for k, x in enumerate(rest)
                 if x != "--territory" and (k == 0 or rest[k-1] != "--territory")]
        return _abyss.spoke(" ".join(words) if words else None, terr)
    elif cmd in ("reconcile", "mirror"):
        if cmd == "mirror":
            print("[CHAOS] «mirror» is now `reconcile`: the Mirror's name is"
                  " needed by its own organ. The old one still lives for now.")
        return _weave.reconcile()
    elif cmd == "vigil-sweep":           return _vigil.vigil_sweep("--deep" in rest)
    elif cmd == "report":                return _vigil.report("--archived" in rest)
    elif cmd == "schedule":
        when = next((x for x in rest if ":" in x), "03:00")
        return _hands.schedule(when, "--remove" in rest)
    elif cmd == "heartbeat":             return _vigil.heartbeat("--deep" in rest)
    elif cmd == "autonomy" and "--reason" in rest:
        i = rest.index("--reason")
        val = "1" if (len(rest) > i + 1 and rest[i + 1] == "on") else "0"
        _c = _sense.db(); _c.execute("INSERT OR REPLACE INTO meta(key, value) VALUES ('reason',?)", (val,))
        _c.commit()
        print("[CHAOS] Nightly reasoning {}. {}".format(
            "ON" if val == "1" else "off",
            "I will spend ONE invocation per night on the report, and tell you"
            " what it cost." if val == "1" else "Not one token without your word."))
    elif cmd == "autonomy":
        act = next((x for x in rest if x in ("grant", "revoke")), None)
        return _vigil.autonomy(act, next((x for x in rest if ":" in x), "03:00"))
    elif cmd == "record-incarnation":
        # Called by the installer: incarnating is an act of mine too.
        return _sense.record_act("incarnation", rest[0] if rest else "install",
                   "I incarnated and took the autonomy that installing me grants")
    elif cmd == "restore":
        which = next((x for x in rest if not x.startswith("--")), None)
        return _sense.restore(which, "--dry" in rest, "--force" in rest)
    elif cmd == "doctor":
        return _vigil.doctor()
    elif cmd in ("version", "--version", "-v"):
        return _vigil.version()
    elif cmd == "acts":
        kd = rest[rest.index("--kind") + 1] if "--kind" in rest and len(rest) > rest.index("--kind") + 1 else None
        return _vigil.acts(next((int(x) for x in rest if x.isdigit()), 20), kd)
    elif cmd == "fault" and rest and rest[0].isdigit():
        # P-3 · a numeric id SHOWS. Creating demands a text title: that way
        # `chaos fault 500` stops giving birth to a fault titled "500" (#506).
        return _errarium.fault_show(rest[0])
    elif cmd == "fault" and rest:
        kw = {}
        pos = []
        i = 0
        while i < len(rest):
            if rest[i].startswith("--") and i + 1 < len(rest):
                kw[rest[i][2:]] = rest[i + 1]; i += 2
            else:
                pos.append(rest[i]); i += 1
        return _errarium.fault(" ".join(pos), kw.get("symptom", ""), kw.get("cause", ""),
              kw.get("cure", ""), kw.get("lesson", ""), kw.get("territory"))
    elif cmd == "faults" and "--probe" in rest:
        ter = rest[rest.index("--territory") + 1] if "--territory" in rest and len(rest) > rest.index("--territory") + 1 else None
        return _errarium.faults_probe(ter, "--apply" in rest)
    elif cmd == "faults":
        ter = rest[rest.index("--territory") + 1] if "--territory" in rest and len(rest) > rest.index("--territory") + 1 else None
        free = [x for x in rest if not x.startswith("--") and x != ter]
        return _errarium.faults(" ".join(free) if free else None, ter)
    elif cmd == "alias":
        return _weave.alias(rest[0] if rest else None,
              rest[1] if len(rest) > 1 and not rest[1].startswith("--") else None,
              "--remove" in rest)
    elif cmd == "suggested-aliases":
        return _weave.suggested_aliases("--apply" in rest)
    elif cmd == "island":
        return _weave.island(next((x for x in rest if not x.startswith("--")), None),
               "--remove" in rest)
    elif cmd == "blockify":
        return _weave.blockify(next((x for x in rest if not x.startswith("--")), None),
                 "--dry" in rest)
    elif cmd == "type-essences":
        return _weave.type_externals("--dry" in rest)
    elif cmd == "heal-territories":
        return _abyss.heal_territories("--dry" in rest)
    elif cmd == "relapse" and rest:     return _errarium.relapse(rest[0])
    elif cmd == "fault-cured" and rest:
        return _errarium.fault_cured(rest[0], " ".join(rest[1:]))
    elif cmd == "fault-reopen" and rest:
        return _errarium.fault_reopen(rest[0], " ".join(rest[1:]))
    elif cmd == "eye":
        return _hands.eye(rest[0] if rest else None, rest[1] if len(rest) > 1 else None)
    elif cmd == "delta":                 return _vigil.delta(rest[0] if rest else None)
    elif cmd == "expired":               return _weave.expired()
    elif cmd == "note" and rest:         return _chronicle.note(" ".join(rest))
    elif cmd == "notes":                 return _chronicle.notes(" ".join(rest) if rest else None)
    elif cmd == "note-where" and rest:   return _chronicle.note_where(rest[0])
    elif cmd == "ascend" and rest:       return _chronicle.ascend(rest[0])
    elif cmd == "chronicle" and "--distil" in rest:
        return _chronicle.chronicle_distil()
    elif cmd == "chronicle":
        what = rest[rest.index("--what")+1] if "--what" in rest and len(rest)>rest.index("--what")+1 else None
        why  = rest[rest.index("--why")+1] if "--why" in rest and len(rest)>rest.index("--why")+1 else None
        kd   = rest[rest.index("--kind")+1] if "--kind" in rest and len(rest)>rest.index("--kind")+1 else "modification"
        return _chronicle.chronicle(what, why, kd)
    elif cmd == "undocumented":          return _chronicle.undocumented()
    elif cmd == "export-chronicle":      return _chronicle.export_chronicle()
    elif cmd == "evolve":                return _weave.evolve("--dry" in rest)
    elif cmd == "weave":                 return _weave.weave()
    elif cmd == "index":                 return _weave.index()
    elif cmd == "suggest":
        kill = rest[rest.index("--kill")+1] if "--kill" in rest and len(rest) > rest.index("--kill")+1 else None
        return _weave.suggest(kill=kill)
    elif cmd == "links" and rest:        return _weave.links_of(rest[0])
    elif cmd == "query":                 return _weave.query(*rest)
    elif cmd == "orphans":               return _weave.orphans()
    elif cmd == "backup" and "--to" in rest:
        i = rest.index("--to")
        return _hands.backup_outside(rest[i + 1] if len(rest) > i + 1 else None)
    elif cmd == "backup":                return _sense.backup(rest[0] if rest else "manual")
    elif cmd == "sow":
        return _stone.sow(rest[1] if len(rest) > 1 and rest[0] == "--from" else None)
    elif cmd == "debts" and rest and rest[0] == "settle":
        bc = rest[rest.index("--because") + 1] if "--because" in rest and len(rest) > rest.index("--because") + 1 else ""
        return _chronicle.debts_settle(rest[1] if len(rest) > 1 else None, bc)
    elif cmd == "debts":                 return _chronicle.debts(rest[0] if rest else None)
    elif cmd == "trail":
        # trail <file> <action> [session] [cwd] [tool] | trail --purge [session]
        return _chronicle.trail(*(rest + [None] * 5)[:5])
    elif cmd == "plan":
        pt = next((x for x in rest if x.endswith(".md") and os.path.isfile(x)), None)
        fo = next((x for x in rest if not x.startswith("--") and x != pt), None)
        return _stone.plan(pt, fo, "--run" in rest, "--paint" in rest, "--json" in rest)
    elif cmd == "probe":
        op = lambda k: (rest[rest.index(k) + 1]
                        if k in rest and len(rest) > rest.index(k) + 1 else None)
        if "--massive" in rest:
            return sys.exit(0 if _stone.probe_massive(op("--massive"), op("--test"),
                                        int(op("--n") or 30),
                                        int(op("--seed") or 1618)) else 1)
        fi, sb = op("--file"), op("--sabotage")
        co = next((x for x in rest if not x.startswith("--") and x != fi and x != sb), None)
        return sys.exit(0 if _stone.probe(co, fi, sb) else 1)
    elif cmd == "route":
        return _singularity.route(" ".join(x for x in rest if not x.startswith("--")) or None,
              "--report" in rest)
    elif cmd == "judge":
        return _singularity.judge(" ".join(x for x in rest if not x.startswith("--")) or None,
              "--eyes" in rest)
    elif cmd == "collapse":
        md = rest[rest.index("--mode") + 1] if "--mode" in rest and len(rest) > rest.index("--mode") + 1 else "essence"
        return _stone.collapse(next((x for x in rest if not x.startswith("--") and x != md), None), md)
    elif cmd == "mirror-organ":
        return _stone.mirror_organ(" ".join(x for x in rest if not x.startswith("--")) or None,
                     "--dry" in rest)
    elif cmd == "stale":
        return _abyss.stale(next((int(x) for x in rest if x.isdigit()), 90))
    elif cmd == "forget" and rest:       return _abyss.forget(rest[0])
    else:
        print(__doc__.strip())
        # A non-existent command must NOT exit successfully: a script that
        # chains `cmd_a || cmd_b` would never see the failure (found in the
        # isolated verification).
        return sys.exit(1)


def main():
    """E3.4 · The gate speaks two languages: the mortal's and the machine's.

    `--json` wraps ANY command in an envelope with its text, its exit code and
    whatever it returned. The Eye and the MCP used to read the output by eye,
    with no way to tell an empty result from a failure; now they ask."""
    args = sys.argv[1:]
    if "--json" in args:
        return _as_json([a for a in args if a != "--json"])
    # P-5 · el perfil solo habla si se le pide: sin la variable, cero coste.
    if os.environ.get("CHAOS_PROFILE"):
        return _with_profile(args)
    return _dispatch(args)


def _as_json(args):
    """The envelope. `data` carries whatever the function returns; today most
    commands PRINT instead of returning, and that is declared with an honest
    null rather than faking a structure that does not exist."""
    buf = io.StringIO()
    old, sys.stdout = sys.stdout, buf
    code, data = 0, None
    try:
        data = _dispatch(args)
    except SystemExit as e:
        code = e.code if isinstance(e.code, int) else 1
    finally:
        sys.stdout = old
    try:
        json.dumps(data)
    except (TypeError, ValueError):
        data = None
    print(json.dumps({"command": args[0] if args else None,
                      "text": buf.getvalue(),
                      "data": data,
                      "code": code}, ensure_ascii=False))
    if code:
        sys.exit(code)


def _with_profile(args):
    """P-5 · `CHAOS_PROFILE=1` and every order confesses what it cost and where.

    Without the variable cProfile is NOT imported and nothing is measured: a
    meter that is always on changes what it measures. With it, the cost stops
    being an opinion even for the mortal who cannot forge."""
    import cProfile
    import pstats
    profile = cProfile.Profile()
    t0 = time.time()
    profile.enable()
    try:
        return _dispatch(args)
    finally:
        profile.disable()
        ms = (time.time() - t0) * 1000
        buf = io.StringIO()
        try:
            pstats.Stats(profile, stream=buf).sort_stats("cumulative").print_stats(3)
            top = [l.strip() for l in buf.getvalue().splitlines()
                   if l.strip() and "{" not in l][-3:]
        except Exception:
            top = []
        sys.stderr.write("\n[PROFILE] %s · %.1f ms\n" % (" ".join(args) or "—", ms))
        for l in top:
            sys.stderr.write("[PROFILE]   %s\n" % l[:110])
