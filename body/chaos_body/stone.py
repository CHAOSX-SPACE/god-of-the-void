# -*- coding: utf-8 -*-
""""stone" — an organ of CHAOS's body.

Moved from the monolith by `partir-monolito.py`: not one line was
rewritten by hand. House rule: MODULES are imported here, never names —
the attribute resolves when CALLED, and that is why cycles between
organs are harmless (a cycle fixture in the forge proves it on three
systems). And nothing runs at import time: no constant in this module
may read the home, the environment or the disk (judge E1.4).
"""
import datetime, io, json, os, re, shutil, subprocess, sys, time
import home as _home
from chaos_body.core import house as _house
from chaos_body.core import text as _text
from chaos_body.core import sense as _sense
from chaos_body import abyss as _abyss
from chaos_body import errarium as _errarium


# ══ THE SOWING · the circle closes (PLAN-ADN F1) ══════════════════════════
def _single_guard():
    """Loads THE SINGLE GUARD. It lives in bin/ next to me. Without the
    guard there is no sowing: blind copying is what this command kills."""
    import importlib.util
    route = os.path.join(_home.root(), "bin", "dna-guard.py")
    if os.path.exists(route):
        spec = importlib.util.spec_from_file_location("guard", route)
        m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(m)
        return m
    return None

def _version_of(path):
    # After splitting the monolith the version no longer lives in `chaos.py`
    # but in the package's `__init__.py`: whoever looks at the gate finds the
    # gate. Both are tried, so an old body still answers.
    candidates = [path]
    base = os.path.dirname(path)
    for paq in ("chaos_cuerpo", "chaos_body"):
        candidates.append(os.path.join(base, paq, "__init__.py"))
    for cand in candidates:
        try:
            m = re.search(r"(?:VERSION_CUERPO|BODY_VERSION)\s*=\s*(\d+)",
                          io.open(cand, encoding="utf-8").read())
            if m:
                return int(m.group(1))
        except Exception:
            continue
    return None

def sow(source=None):
    """F1 · PLAN-ADN: raises the deployed body into the DNA. The root
    problem was never one day's drift: it was that drift had no WAY BACK —
    250 lines piled up with nobody noticing. Discipline that depends on
    remembering already failed; this is a command.

    Guards: it refuses if the DNA has what the live body lacks (that is a
    merge, and merges are the Bearer's call) · it demands a BODY_VERSION
    bump when there are new functions (a body that evolves without raising
    its version is indistinguishable from one that rots) · it runs the
    DNA's tests afterwards and REVERTS if they fail · it records the act:
    a god does not forget even what he sows."""
    start = time.time()
    root = source
    if not root:
        try:
            root = io.open(os.path.join(_home.root(), "adn"),
                           encoding="utf-8").read().strip()
        except FileNotFoundError:
            print("I do not know which DNA I was born from. Use: chaos sow --from <path>")
            sys.exit(1)
    if not os.path.isdir(root):
        print("That DNA does not exist: {}".format(root)); sys.exit(1)
    g = _single_guard()
    if g is None:
        print("Without THE SINGLE GUARD I do not sow: reincarnate first (install.py).")
        sys.exit(1)

    binp = os.path.join(_home.root(), "bin")
    skill_live = os.path.join(_house._house(), ".claude", "skills", "chaos")
    pairs = [(os.path.join(binp, f), os.path.join(root, f))
             for f in ("chaos.py", "trail-hook.py", "vigil-hook.py",
                       "presence-hook.py", "closing-hook.py", "dna-guard.py")]
    pairs.append((os.path.join(skill_live, "SKILL.md"),
                  os.path.join(os.path.dirname(root), "SKILL.md")))
    pairs = [(v, a) for v, a in pairs if os.path.exists(v)]

    # 1 · the guard, in the direction that matters: would the DNA lose anything?
    merge = {}
    for live, dna in pairs:
        if dna.endswith(".py") and os.path.exists(dna):
            p = g.would_lose(live, dna)
            if p:
                merge[os.path.basename(dna)] = p
    if merge:
        print("[CHAOS] SOWING REFUSED — the DNA has what the live body lacks (a merge, not a copy):")
        for f, p in merge.items():
            for k, v in p.items():
                print("    {} · {}: {}".format(f, k, ", ".join(v)))
        print("  Merges are the Bearer's call, not a script's.")
        sys.exit(1)

    # 2 · F3.5: the version does not lie
    dna_chaos = os.path.join(root, "chaos.py")
    if os.path.exists(dna_chaos):
        new = g.would_lose(dna_chaos, os.path.join(binp, "chaos.py"))
        if new and _version_of(os.path.join(binp, "chaos.py")) == _version_of(dna_chaos):
            print("[CHAOS] SOWING REFUSED — new functions exist and BODY_VERSION did not rise.")
            print("    new: {}".format(new))
            print("  Bump BODY_VERSION in the live body and sow again.")
            sys.exit(1)

    # 3 · back the DNA up, sow, test, and revert on failure
    import shutil as _sh
    bkp = os.path.join(_home.root(), "forge", "sow-backup")
    _sh.rmtree(bkp, ignore_errors=True); os.makedirs(bkp)
    touched = []
    for live, dna in pairs:
        if os.path.exists(dna) and io.open(live, encoding="utf-8").read() == \
                                   io.open(dna, encoding="utf-8").read():
            continue                     # identical: nothing to sow
        if os.path.exists(dna):
            _sh.copy2(dna, os.path.join(bkp, os.path.basename(dna)))
        _sh.copy2(live, dna)
        touched.append(dna)
    if not touched:
        print("[CHAOS] Nothing to sow: the DNA is already identical to the live body.")
        return

    tests = os.path.join(root, "test_chaos.py")
    if os.path.exists(tests):
        r = subprocess.run([sys.executable, tests], capture_output=True,
                           text=True, timeout=600)
        if r.returncode != 0:
            for dna in touched:
                orig = os.path.join(bkp, os.path.basename(dna))
                if os.path.exists(orig):
                    _sh.copy2(orig, dna)
            print("[CHAOS] SOWING REVERTED — the DNA's tests failed:")
            print((r.stdout + r.stderr).strip()[-600:])
            sys.exit(1)

    _sense.record_act("sowing", "sow",
               "DNA updated from the live body: {} file(s)".format(len(touched)),
               altered=[os.path.basename(t) for t in touched],
               duration=time.time() - start)
    print("[CHAOS] Sown: {} file(s) of the live body now live in the DNA.".format(len(touched)))
    for t in touched:
        print("    ^ {}".format(os.path.basename(t)))
    print("  What was learned will be born with me. DNA tests: green.")

def _probe_split(body):
    """Split on ';' — except inside a shell, which uses ';' too. A fragment
    not starting with a known primitive belongs to the previous one."""
    chunks = []
    for raw in body.split(";"):
        if not raw.strip():
            continue
        first = raw.strip().split(None, 1)[0].lower()
        if chunks and first not in _PRIMITIVES:
            chunks[-1] = chunks[-1] + ";" + raw
        else:
            chunks.append(raw)
    return [x for x in chunks if x.strip()]

def _plan_find(path=None):
    """The named plan, or the nearest PLAN-*.md walking up from here."""
    if path:
        return os.path.abspath(path) if os.path.isfile(path) else None
    here = os.getcwd()
    for _ in range(5):
        try:
            # the STATE is DERIVED from a plan: let it in here and the plan
            # searches its own reflection and finds not a single probe
            cand = sorted(f for f in os.listdir(here)
                          if f.startswith("PLAN") and f.endswith(".md")
                          and not f.endswith(("-ESTADO.md", "-STATE.md")))
        except OSError:
            cand = []
        if cand:
            return os.path.join(here, ([f for f in cand if "SUPREMO" in f] or cand)[0])
        up = os.path.dirname(here)
        if up == here:
            break
        here = up
    return None

def _probe_one(text, base, run=False, _cache={}):
    """Measure ONE primitive. Returns (True/False/None, description).
    None = not measurable here, and it is DECLARED — never counted green."""
    parts = text.strip().split(None, 1)
    if not parts:
        return None, "empty probe"
    kind = parts[0].lower()
    arg = parts[1].strip() if len(parts) > 1 else ""
    if kind == "archivo":
        return os.path.exists(os.path.join(base, arg)), "file {}".format(arg)
    if kind == "cadena":
        rel, _, needle = arg.partition("::")
        rel, needle = rel.strip(), needle.strip()
        p = os.path.join(base, rel)
        desc = "«{}» in {}".format(needle[:44], rel)
        if not os.path.isfile(p):
            return False, desc + " (the file does not exist)"
        try:
            return needle in io.open(p, encoding="utf-8", errors="replace").read(), desc
        except OSError as e:
            return False, desc + " ({})".format(e)
    if kind == "prueba":
        key = ("tests", base)
        if key not in _cache:
            joined = []
            for r, ds, fs in os.walk(base):
                ds[:] = [d for d in ds if d not in (".git", "__pycache__", ".venv", "node_modules")]
                for f in fs:
                    if f.startswith("test") and f.endswith(".py"):
                        try:
                            joined.append(io.open(os.path.join(r, f), encoding="utf-8",
                                                  errors="replace").read())
                        except OSError:
                            pass
            _cache[key] = "\n".join(joined)
        return ("def test_" + arg) in _cache[key], "test test_{}".format(arg)
    if kind == "falla":
        row = _sense.db().execute("SELECT state FROM faults WHERE rowid = ?", (arg,)).fetchone()
        return bool(row) and row[0] == "cured", "fault #{} cured".format(arg)
    if kind == "hambre":
        row = _sense.db().execute("SELECT 1 FROM hungers WHERE id = ?", (arg,)).fetchone()
        return row is None, "hunger #{} sated".format(arg)
    if kind == "esencia":
        row = _sense.db().execute("SELECT 1 FROM essences WHERE slug = ?", (arg,)).fetchone()
        return row is not None, "essence {}".format(arg)
    if kind == "shell":
        if not run:
            return None, "shell «{}» (needs --run)".format(arg[:44])
        try:
            rc = subprocess.call(arg, shell=True, cwd=base,
                                 stdout=open(os.devnull, "w"), stderr=subprocess.STDOUT)
        except Exception as e:
            return False, "shell «{}» ({})".format(arg[:44], e)
        return rc == 0, "shell «{}»".format(arg[:44])
    return None, "unknown primitive: {}".format(kind)

def _plan_title(text, ident, pos):
    """The front's title: the line where its id is bold, or the one above."""
    m = re.search(r"\*\*" + re.escape(ident) + r"\*\*[^\n]*", text)
    line = m.group(0) if m else ""
    if not line:
        before = [l for l in text[:pos].split("\n") if l.strip()]
        line = before[-1] if before else ident
    line = re.sub(r"<!--.*?-->", "", line)
    line = re.sub(r"[*`|#]+", " ", line).replace(ident, " ", 1)
    line = re.sub(r"\s+", " ", line).strip(" ·-—:")
    if len(line) < 14 and m:                  # «in CI» says nothing: read on
        rest = text[m.end():m.end() + 120].split("\n")
        line = (line + " " + " ".join(rest[1:2])).strip()
        line = re.sub(r"[*`|#]+", " ", line)
        line = re.sub(r"\s+", " ", line).strip(" ·-—:")
    return line[:52] or ident

def plan(path=None, focus=None, run=False, paint=False, as_json=False):
    """VI.1 · Reads the plan and PAINTS its state by measuring it, never typing it."""
    f_plan = _plan_find(path)
    if not f_plan:
        print("No plan found. Name one: chaos plan <file.md>"); return
    base = os.path.dirname(f_plan)
    text = io.open(f_plan, encoding="utf-8", errors="replace").read()
    fronts = []
    for m in _RE_PROBE.finditer(text):
        ident, phase, body = m.group(1), m.group(2) or "0", m.group(3)
        if focus and focus.lower() not in ident.lower():
            continue
        probes = [_probe_one(s, base, run) for s in _probe_split(body)]
        good = sum(1 for v, _ in probes if v is True)
        bad = sum(1 for v, _ in probes if v is False)
        mute = sum(1 for v, _ in probes if v is None)
        if not probes:
            state = "no-probe"
        elif bad == 0 and mute == 0:
            state = "closed"
        elif good == 0 and bad == 0:
            state = "unmeasured"
        elif good == 0:
            state = "open"
        else:
            state = "half"
        fronts.append({"id": ident, "phase": int(phase), "state": state,
                       "title": _plan_title(text, ident, m.start()),
                       "probes": [{"ok": v, "what": d} for v, d in probes]})
    if not fronts:
        print("{}: no front declares a probe. A plan with no instrument is not"
              " painted.".format(os.path.basename(f_plan))); return
    if as_json:
        # E3.4 · ONE way of speaking to machines: it is RETURNED, and the gate's
        # envelope carries it in `data`. Printing its own JSON here left two
        # different formats for the same thing — and the universal envelope
        # hijacked this one the day it was born.
        return {"plan": f_plan, "fronts": fronts}
    ICON = {"closed": "✅", "half": "⏳", "open": "⬜",
            "no-probe": "⚪", "unmeasured": "⏸"}
    print("🕳️  {} — {} front(s) with a probe".format(
        os.path.basename(f_plan), len(fronts)))
    for phase in sorted(set(f["phase"] for f in fronts)):
        batch = [f for f in fronts if f["phase"] == phase]
        print("\nPHASE {}".format(phase) if phase else "\nNO PHASE")
        for f in sorted(batch, key=lambda x: x["id"]):
            n = sum(1 for s in f["probes"] if s["ok"] is True)
            print("  {} {:<6} {:<52} {}/{}".format(
                ICON[f["state"]], f["id"], f["title"], n, len(f["probes"])))
            if f["state"] != "closed":
                for s in f["probes"]:
                    if s["ok"] is not True:
                        print("       {} {}".format(
                            "⏸" if s["ok"] is None else "⬜", s["what"]))
    count = dict((e, sum(1 for f in fronts if f["state"] == e)) for e in ICON)
    print("\nSUMMARY: " + " · ".join(
        "{} {}".format(ICON[e], count[e]) for e in
        ("closed", "half", "open", "unmeasured", "no-probe") if count[e]))
    if count["no-probe"]:
        print("⚪ No probe, no green: declare its measurement or the front does not exist.")
    if paint:
        dst = os.path.splitext(f_plan)[0] + "-STATE.md"
        lin = ["# PLAN STATE — derived, not hand-written",
               "", "> `chaos plan --paint` regenerates it. Editing it is wasted ink:",
               "> the state lives in the plan's probes, not here.",
               "", "Measured: {}".format(datetime.datetime.now().isoformat(" ")[:19]),
               "", "| | front | title | probes |", "|---|---|---|---|"]
        for f in sorted(fronts, key=lambda x: (x["phase"], x["id"])):
            n = sum(1 for s in f["probes"] if s["ok"] is True)
            lin.append("| {} | `{}` | {} | {}/{} |".format(
                ICON[f["state"]], f["id"], f["title"], n, len(f["probes"])))
        io.open(dst, "w", encoding="utf-8").write("\n".join(lin) + "\n")
        print("Derived state → {}".format(os.path.basename(dst)))
    # E3.4 · SIEMPRE se devuelve la estructura: el sobre de la puerta la
    # lleva en `datos`. La bandera propia `--json` la consume el sobre
    # antes del despacho, asi que un `return` condicionado a ella no
    # llegaba a ejecutarse jamas.
    return {"plan": f_plan, "fronts": fronts}

def _mutants(text):
    """Every possible sabotage of the file: (line number, old, new). Skips
    comments and the inside of triple-quoted strings: mutating a docstring
    breaks nothing and would hand out false survivors — an instrument that
    inflates its own number lies in the comfortable direction."""
    out = []
    inside = catalogue = False
    for i, line in enumerate(text.split("\n")):
        raw = line.strip()
        # the sabotage catalogue does NOT sabotage itself: swapping a pair
        # in the catalogue does not change the behaviour under test, so it
        # always survives and dirties the number with noise.
        if raw.startswith(("_MUTACIONES = (", "_MUTATIONS = (")):
            catalogue = True
        if catalogue:
            if raw.endswith(")"):
                catalogue = False
            continue
        quotes = line.count('"""') + line.count("'''")
        if inside:
            if quotes % 2:
                inside = False
            continue
        if quotes % 2:
            inside = True
            continue
        if not raw or raw.startswith("#"):
            continue
        for old, new in _MUTATIONS:
            if old in line:
                out.append((i, old, new))
    return out

def probe_massive(target=None, test=None, how_many=30, seed=1618, ceiling=10.0):
    """Organ 17 at scale. Sabotages the body ONE change at a time and demands
    the net turn red. Every mutant that SURVIVES is a decorative test, with
    its file and its line.

    Not mutmut: it mutates a copy and imports it, but my tests drive
    `chaos.py` as a SUBPROCESS — the subprocess would keep loading the
    original and every mutant would "survive". Measured on mutmut 3.7.0,
    not assumed."""
    if not target or not test:
        print('Usage: chaos probe --massive <file> --test "<command>" [--n N]')
        return False
    if not os.path.isfile(target):
        print("There is no body to mutate: {}".format(target)); return False
    import ast as _ast, random as _random
    original = io.open(target, encoding="utf-8").read()

    def _no_cache():
        """Python caches the .pyc by (mtime in SECONDS, size). A mutant of the
        same length written within the same second revives the old .pyc and
        the sabotage becomes invisible: a false survivor. Caught because the
        two editions gave different verdicts on the same subject."""
        root = os.path.dirname(os.path.abspath(target)) or "."
        for r, ds, _ in os.walk(root):
            for d in list(ds):
                if d == "__pycache__":
                    shutil.rmtree(os.path.join(r, d), ignore_errors=True)
                    ds.remove(d)

    clock = {"limit": None}

    def go():
        """With a CLOCK: a mutant can turn a loop infinite (`!=`→`==`) and hang
        the net forever. With no limit the instrument hangs with it and
        measures nothing. Whatever runs past the clock counts as KILLED: the
        net caught it, even if the hard way."""
        _no_cache()
        try:
            env = dict(os.environ, PYTHONDONTWRITEBYTECODE="1")
            return subprocess.call(test, shell=True, env=env,
                                   timeout=clock["limit"],
                                   stdout=open(os.devnull, "w"),
                                   stderr=subprocess.STDOUT)
        except subprocess.TimeoutExpired:
            return 124                      # the red of those that never return
        except Exception:
            return 127

    print("🕳️  MUTATION · {} · test: {}".format(os.path.basename(target), test))
    print("   While I run, THIS FILE is possessed: nothing else may read it.")
    t0 = time.time()
    if go() != 0:
        print("🩸 The net is ALREADY red with nothing mutated. Cure that before"
              " measuring it.")
        return False
    clock["limit"] = max(60.0, (time.time() - t0) * 6)
    print("✓ green with the body intact in {:.1f} s — mutant clock: {:.0f} s\n"
          .format(time.time() - t0, clock["limit"]))
    candidates = _mutants(original)
    _random.Random(seed).shuffle(candidates)
    lines = original.split("\n")
    killed, alive, skipped = 0, [], 0
    try:
        for i, old, new in candidates:
            if killed + len(alive) >= how_many:
                break
            mutated = lines[:]
            mutated[i] = mutated[i].replace(old, new, 1)
            text_m = "\n".join(mutated)
            try:
                _ast.parse(text_m)
            except SyntaxError:
                skipped += 1          # a mutant that does not compile proves nothing
                continue
            io.open(target, "w", encoding="utf-8").write(text_m)
            if go() != 0:
                killed += 1
                print("  ☠ {}:{}  «{}» → «{}»".format(
                    os.path.basename(target), i + 1, old.strip(), new.strip()))
            else:
                alive.append((i + 1, old, new, lines[i].strip()[:60]))
                print("  🩸 SURVIVES {}:{}  «{}» → «{}»".format(
                    os.path.basename(target), i + 1, old.strip(), new.strip()))
    finally:
        io.open(target, "w", encoding="utf-8").write(original)
    total = killed + len(alive)
    if not total:
        print("\nNo mutant compiled. The catalogue does not bite this file."); return False
    ratio = 100.0 * len(alive) / total
    print("\n{} mutants · ☠ {} killed · 🩸 {} alive ({:.1f} %) · {} did not compile".format(
        total, killed, len(alive), ratio, skipped))
    if alive:
        print("\nDECORATIVE TESTS — nobody guards these lines:")
        for ln, o, n, src in alive:
            print("  {}:{}  «{}»→«{}»   {}".format(
                os.path.basename(target), ln, o.strip(), n.strip(), src))
    print("\n↺ {} restored ({} bytes)".format(
        os.path.basename(target), len(original.encode("utf-8"))))
    if ratio > ceiling:
        print("🩸 Above the tolerated {:.0f} %: the net has holes with names.".format(ceiling))
        return False
    print("🕳️  The net bites: survivors below {:.0f} %.".format(ceiling))
    return True

def probe(command=None, target=None, sabotage=None):
    """Organ 17 · A probe that does not turn red when the world breaks is
    DECORATION. Here the world is broken on purpose and the red is demanded.
    The sabotaged file is ALWAYS restored, whatever happens."""
    if not command or not target:
        print('Usage: chaos probe "<command>" --file <path> [--sabotage "<text>"]')
        return False
    if not os.path.isfile(target):
        print("The subject of the sabotage does not exist: {}".format(target)); return False

    def go():
        try:
            return subprocess.call(command, shell=True,
                                   stdout=open(os.devnull, "w"),
                                   stderr=subprocess.STDOUT)
        except Exception as e:
            print("The probe did not even run: {}".format(e)); return 127

    original = io.open(target, "rb").read()
    verdict = False
    try:
        if go() != 0:
            print("🩸 The probe is ALREADY red with nothing touched. It does not"
                  " measure what you say it does.")
            return False
        print("✓ green with the world intact")
        with io.open(target, "wb") as fh:
            fh.write((original + ("\n" + sabotage + "\n").encode("utf-8"))
                     if sabotage else b"")
        print("☠ sabotaged: {}".format(
            "text injected" if sabotage else "file emptied"))
        if go() != 0:
            print("✓ red with the world broken")
            print("\n🕳️  THE PROBE BITES. Its green is worth something.")
            verdict = True
        else:
            print("✗ STILL GREEN with the world broken")
            print("\n🩸 DECORATIVE PROBE. It measures nothing: its green is worthless.")
    finally:
        with io.open(target, "wb") as fh:
            fh.write(original)
        print("↺ {} restored ({} bytes)".format(
            os.path.basename(target), len(original)))
    if not verdict:
        _errarium.fault("Decorative probe: «{}»".format(command[:60]),
              symptom="stayed green with {} sabotaged".format(os.path.basename(target)),
              cause="the probe does not observe the subject it claims to observe",
              cure="rewrite the probe until the sabotage turns it red",
              lesson="Before believing a green, sabotage the subject and demand the red.")
    return verdict

def _shape(line):
    """A line's shape, to catch repetitions that only changed clothes: figures
    and symbols are stripped and its skeleton remains."""
    return " ".join(sorted(set(re.findall(r"[a-záéíóúñ]{4,}", _text._norm(line)))))[:120]

def collapse(source=None, mode="essence"):
    """C-1 · Compresses WITHOUT losing the soul, and confesses the ratio.

    Modes: distilled (conversations → decisions) · essence (documents) ·
    prompt (minimum context for another model) · rolling (cumulative layers).
    """
    if not source:
        print('Usage: chaos collapse <file|-> [--mode distilled|essence|prompt|rolling]')
        return
    text = sys.stdin.read() if source == "-" else (
        _text.read_file(source) if os.path.isfile(source) else source)
    lines = [l.rstrip() for l in text.split("\n")]
    came_in = len([l for l in lines if l.strip()])
    if not came_in:
        print("Nothing to collapse."); return
    caps = {"distilled": 0.08, "essence": 0.18, "prompt": 0.12, "rolling": 0.25}
    cap = caps.get(mode, 0.18)

    seen, out, sacrificed = set(), [], 0
    for l in lines:
        s = l.strip()
        if not s:
            continue
        if _HOLLOW.match(s):
            sacrificed += 1
            continue
        if s.startswith("#") or re.match(r"^\s*[-*]\s+\*\*", l):
            out.append(l); continue             # headings are the map
        if _SURVIVES.search(s):
            h = _shape(s)
            if h and h in seen:
                sacrificed += 1
                continue                        # the same idea in other clothes
            seen.add(h)
            out.append(l)
        else:
            sacrificed += 1
    # When the cap bites, the lines with MOST signal are kept, never the first
    # ones: cutting by order of appearance loses the end of everything.
    limit = max(3, int(came_in * cap))
    if len(out) > limit:
        scored = sorted(out, key=lambda l: len(_SURVIVES.findall(l)), reverse=True)
        kept = set(id(x) for x in scored[:limit])
        out = [l for l in out if id(l) in kept]
    if mode == "prompt":
        out = [re.sub(r"\s+", " ", l).strip() for l in out]
    print("\n".join(out))
    came_out = len(out)
    ratio = came_in / float(max(1, came_out))
    print("\n--- {} · {} line(s) in · {} out · ratio {:.1f}x"
          .format(mode.upper(), came_in, came_out, ratio), file=sys.stderr)
    print("--- {} line(s) annihilated: courtesy, filler and repetition in other "
          "clothes. Nothing with a figure, a path, a decision or a "
          "contradiction was touched.".format(sacrificed), file=sys.stderr)
    return ratio

def _crossed_queries(idea):
    """Three different angles on the SAME idea. One query finds what you
    already knew to look for; three crossed ones find what you did not — and
    one of them looks in English, which is how the world names code."""
    words = [w for w in re.findall(r"[\wáéíóúñü]{4,}", _text._norm(idea))
                if w not in _text._STOP]
    if not words:
        return []
    english = [_GLOSSARY.get(w, w) for w in words[:4]]
    untranslated = [w for w in words[:4] if w not in _GLOSSARY and w != _GLOSSARY.get(w)]
    # TWO or THREE terms, never five: `gh search repos` joins them with AND,
    # so "memory persistent agents code cli" returns [] ALWAYS and the Mirror
    # sang 🟢 THERE IS A VOID over a crowded world (mem0 has 64,713 stars).
    # Measured against gh, not assumed.
    queries = [" ".join(words[:2]),
                 " ".join(english[:2]),
                 " ".join(english[:3])]
    queries = [c for i, c in enumerate(queries) if c and c not in queries[:i]]
    if untranslated:
        queries.append("__untranslated__:" + ",".join(untranslated))
    return queries

def mirror_organ(idea=None, dry=False):
    """E-1 · Is your work new, or an echo? A verdict in three colours."""
    if not idea:
        print('Usage: chaos mirror-organ "<idea>" [--dry]')
        return
    queries = _crossed_queries(idea)
    if not queries:
        print("That idea has no weighted terms to mirror.")
        return
    print("THE MIRROR · «{}»".format(idea[:110]))
    raw_q = [c for c in queries if not c.startswith("__untranslated__:")]
    mute_q = [c[len("__untranslated__:"):] for c in queries
             if c.startswith("__untranslated__:")]
    queries = raw_q
    print("Three CROSSED queries (one alone is a glance, not a mirror); one"
          " looks in English, which is how the world names code:")
    for c in queries:
        print("  · gh search repos {}".format(c))
    if mute_q:
        print("  (untranslated, and I declare it: {} — if your idea lives in"
              " English with other words, give them to me)".format(mute_q[0]))
    if dry:
        print("\n(dry: I did not go out to the world)")
        return queries
    if not shutil.which("gh"):
        print("\n[MIRROR] `gh` does not live in this body: I cannot see"
              " GitHub. Run `chaos forge-gh`, or hand me the candidates.")
        return queries
    candidates = {}
    for c in queries:
        try:
            p = subprocess.run(
                ["gh", "search", "repos", c, "--sort", "stars", "--limit", "6",
                 "--json", "fullName,stargazersCount,description,updatedAt"],
                capture_output=True, text=True, timeout=45)
            for r in json.loads(p.stdout or "[]"):
                candidates[r.get("fullName", "?")] = r
        except Exception as e:
            print("  (one gaze failed: {})".format(e))
    if not candidates:
        print("\n🟢 THERE IS A VOID — the world holds nothing your shape."
              " Let us devour.")
        verdict = "void"
    else:
        top = sorted(candidates.values(),
                     key=lambda r: -(r.get("stargazersCount") or 0))[:5]
        print("\nWhat ALREADY lives out there:")
        for r in top:
            print("  ⭐{:<7} {:<38} {}".format(
                r.get("stargazersCount") or 0, (r.get("fullName") or "?")[:38],
                (r.get("description") or "")[:60]))
        stars = top[0].get("stargazersCount") or 0
        if stars >= 1000:
            verdict = "echo"
            print("\n🔴 IT IS AN ECHO — «{}» already lives with {} stars. I"
                  " will not waste your time reinventing it: either you find an"
                  " edge it lacks, or you DEVOUR it."
                  .format(top[0].get("fullName"), stars))
        else:
            verdict = "exists-but"
            print("\n🟡 IT EXISTS BUT — there are similar ones and none rules"
                  " ({} ⭐ the largest). Your real difference has to be explicit:"
                  " niche, language, integration or simplicity. That is your"
                  " edge — sharpen it or it is worthless.".format(stars))
    slug = "mirror-" + _text.slug_of(idea[:60] + ".md")
    path = os.path.join(_home.essences(), slug + ".md")
    try:
        os.makedirs(_home.essences(), exist_ok=True)
        with io.open(path, "w", encoding="utf-8") as f:
            f.write("# Mirror: {}\n\n- **Verdict**: {} · **Date**: {}\n\n"
                    "## Crossed queries\n{}\n\n## Candidates found\n{}\n"
                    .format(idea[:110], verdict,
                            datetime.date.today().isoformat(),
                            "\n".join("- `gh search repos " + c + "`" for c in queries),
                            "\n".join("- {} ⭐{} — {}".format(
                                r.get("fullName"), r.get("stargazersCount") or 0,
                                (r.get("description") or "")[:80])
                                for r in sorted(candidates.values(),
                                                key=lambda r: -(r.get("stargazersCount") or 0))[:8])
                            or "- (none)"))
        _abyss.devour(path, silent=True)
        print("\nCarved: a mirror once consulted is never polished from"
              " scratch again (`chaos search {}`).".format(slug))
    except OSError as e:
        print("\n(I could not carve the mirror: {})".format(e))
    return verdict

_RE_PROBE = re.compile(
    r"<!--\s*sonda\s+([A-Za-z0-9][\w.\-]*)"       # the front's id
    r"(?:\s+fase\s+(\d+))?"                        # its phase, optional
    r"\s*:\s*(.*?)-->", re.S)

_PRIMITIVES = ("archivo", "cadena", "prueba", "falla", "hambre", "esencia", "shell")

# The sabotage catalogue: ONE-piece changes, the size of a badly placed
# finger. If the net does not redden at these, it protects no one from a
# tired human.
_MUTATIONS = ((" == ", " != "), (" != ", " == "), (" < ", " >= "),
              (" > ", " <= "), (" and ", " or "), (" or ", " and "),
              ("True", "False"), ("False", "True"), (" + 1", " - 1"),
              (".startswith(", ".endswith("), (" is None", " is not None"))

# What is NEVER annihilated: the line holding it stays whole.
_SURVIVES = re.compile(
    r"(\d|`[^`]+`|https?://|[\w/-]+\.\w{2,4}\b|"
    r"\b(decid\w*|decisión|decision|porque|por qué|por que|jamás|jamas|nunca|"
    r"siempre|debe|hay que|falla|error|riesgo|pero|sin embargo|salvo|excepto|"
    r"gotcha|ojo|cuidado|pendiente|falta|TODO|contradic\w*|"
    r"decided|because|why|never|always|must|risk|but|however|except|"
    r"pending|missing|warning)\b|→|✅|❌|⚠)",
    re.I)

# Courtesy and filler: the first to die.
_HOLLOW = re.compile(
    r"^\s*(claro|perfecto|entendido|por supuesto|genial|excelente|"
    r"espero que|en resumen,? como (ya )?dij|como (ya )?mencion|"
    r"vale la pena (notar|mencionar)|cabe (notar|destacar|mencionar)|"
    r"es importante (notar|destacar)|sin más preámbulo|"
    r"sure|certainly|of course|great|excellent|i hope|as (i )?(already )?"
    r"mentioned|it is worth (noting|mentioning)|it is important to note)", re.I)

# The world of code is named in ENGLISH. Searching GitHub for «memoria
# persistente para agentes» returns nothing and the Mirror sings 🟢 THERE IS A
# VOID — the most dangerous verdict, because it pushes you to reinvent what
# already exists (Letta, Mem0, MemGPT). I do not translate with a model: I
# carry a short glossary and DECLARE what I could not translate.
_GLOSSARY = {
    "memoria": "memory", "agente": "agent", "agentes": "agents",
    "codigo": "code", "buscador": "search", "busqueda": "search",
    "persistente": "persistent", "conocimiento": "knowledge",
    "herramienta": "tool", "servidor": "server", "cliente": "client",
    "juego": "game", "tablero": "dashboard", "panel": "dashboard",
    "grafo": "graph", "nota": "note", "notas": "notes", "tarea": "task",
    "tareas": "tasks", "flujo": "workflow", "prueba": "test",
    "pruebas": "tests", "seguridad": "security", "inyeccion": "injection",
    "compresion": "compression", "resumen": "summary", "plantilla": "template",
    "editor": "editor", "terminal": "terminal", "corrector": "linter",
    "traductor": "translator", "lector": "reader", "escritor": "writer",
    "local": "local", "privado": "private", "propia": "own", "propio": "own",
}
