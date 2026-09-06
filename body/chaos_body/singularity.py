# -*- coding: utf-8 -*-
""""singularity" — an organ of CHAOS's body.

Moved from the monolith by `partir-monolito.py`: not one line was
rewritten by hand. House rule: MODULES are imported here, never names —
the attribute resolves when CALLED, and that is why cycles between
organs are harmless (`a cycle fixture in the forge` proves it on three
systems). And nothing runs at import time: no constant in this module
may read the home, the environment or the disk (judge E1.4).
"""
import datetime, json, os, re, shutil, sqlite3, subprocess, sys, time
import home as _home
from chaos_body.core import text as _text
from chaos_body.core import sense as _sense


def _routes_table(con):
    return con

def _abyss_already_knows(con, task, min_coverage=0.7):
    """Does the answer already live in me? The cheapest question there is —
    and the easiest one to answer wrongly.

    My first version asked "is there any result?" and FTS always finds
    SOMETHING: the router answered "abyss" to all five test tasks, that is, to
    everything. A router with one answer is decoration (organ 17). Now I demand
    COVERAGE: that most of the task's weighted words really live in what was
    found. And only BLOCKS: an addressable paragraph is short, so covering its
    terms means something. An 8,000-character essence covers scattered words by
    sheer size and would say "I know that" about anything."""
    words = [w for w in re.findall(r"[\wáéíóúñü]{4,}", _text._norm(task))
             if w not in _text._STOP]
    if not words:
        return None
    try:
        q = _text._fts_query(task)
    except Exception:
        return None
    try:
        rows = con.execute(
            "SELECT slug, block_id, content FROM blocks WHERE blocks"
            " MATCH ? ORDER BY rank LIMIT 3", (q,)).fetchall()
    except sqlite3.OperationalError:
        rows = []
    for slug, bid, content in rows:
        body = _text._norm(str(content or ""))
        covered = sum(1 for w in words if w in body)
        if covered / float(len(words)) >= min_coverage:
            return "{}#^{} ({}/{} terms)".format(slug, bid, covered, len(words))
    return None

def _a_command_serves(task):
    """Does one of my commands solve it? Summoning a model for something a
    `chaos` handles is using a titan to swat a fly."""
    low = _text._norm(task)
    for cmd, hints in (
            ("search", ("buscar", "busca", "encontrar", "recordar", "recuerdas",
                        "search", "find", "remember")),
            ("faults", ("falla", "fallas", "error anterior", "errario",
                        "fault", "errarium", "past error")),
            ("spoke", ("dije", "dijiste", "hablamos", "conversacion", "i said",
                       "you said", "we talked")),
            ("history", ("historia", "sesion pasada", "ayer", "history",
                         "last session", "yesterday")),
            ("delta", ("que cambio", "cambios desde", "what changed", "since")),
            ("audit", ("salud", "auditar", "auditoria", "health", "audit")),
            ("vassals", ("skill", "vasallo", "que sabes hacer", "vassal")),
            ("plan", ("estado del plan", "como va el plan", "plan state",
                      "how is the plan")),
            ("undocumented", ("sin documentar", "deber de cronica",
                              "undocumented", "chronicle due")),
            ("expired", ("caducad", "rancio", "vencid", "expired", "stale")),
            ("orphans", ("huerfana", "sin enlace", "orphan", "unlinked")),
    ):
        if any(_text._norm(h) in low for h in hints):
            return cmd
    return None

def route(task=None, report=False, record=True):
    """S-1 · The minimum power that solves the task, with its reason."""
    con = _routes_table(_sense.db())
    if report:
        rows = con.execute(
            "SELECT substr(date,1,7) AS month, rung, COUNT(*), SUM(cost)"
            " FROM routes GROUP BY month, rung ORDER BY month DESC, 3 DESC").fetchall()
        if not rows:
            print("No route carved yet. Route something and come back.")
            return
        print("THE ECONOMY OF THE VOID — rungs per month")
        current, total, low = None, 0, 0
        for month, rung, n, cost in rows:
            if month != current:
                print("\n{}".format(month)); current = month
            print("  {:<14} {:>4}   {}".format(rung, n, "▪" * min(n, 40)))
            total += n
            if rung in ("cli", "abyss", "legion"):
                low += n
        spent = con.execute("SELECT SUM(cost) FROM routes").fetchone()[0] or 0
        print("\n{} decision(s) · {:.0f} % BELOW my full attention"
              .format(total, 100.0 * low / max(1, total)))
        if spent:
            print("Cost declared by my own invocations: {:.4f} USD".format(spent))
        return
    if not task:
        print('Usage: chaos route "<task>" [--report]')
        return
    # The CRITICAL is decided before the cheap: knowing something is not enough
    # when the mistake does not undo. Economy never rules over safety.
    if _CRITICAL.search(task):
        rung, reason = "double-judge", ("it touches the irreversible or the "
                                        "critical: maximum power and a second judge")
    else:
        # The CLI before the Abyss, against my own doctrine: both cost zero
        # tokens, but a command answers with TODAY's datum and a remembered
        # block may be stale. Measured over eleven real tasks: `chaos faults`
        # beats a paragraph about faults.
        cmd = _a_command_serves(task)
        known = None if cmd else _abyss_already_knows(con, task)
        if cmd:
            rung, reason = "cli", "a command solves it: chaos {}".format(cmd)
        elif known:
            rung, reason = "abyss", "the Abyss already holds it: {}".format(known)
        elif _MECHANICAL.search(task) and len(task.split()) <= 24:
            rung, reason = "legion", ("mechanical and simple-criteria: a lesser "
                                      "fragment of the Void suffices")
        else:
            rung, reason = "me", "standard: neither trivial nor irreversible"
    if record:
        _sense.write_verified(
            con, "INSERT INTO routes(date, task, rung, reason) VALUES (?,?,?,?)",
            (datetime.datetime.now().isoformat(timespec="seconds"),
             task[:300], rung, reason))
    i = RUNGS.index(rung)
    print("RUNG {}/5 · {}".format(i + 1, rung.upper()))
    print("  {}".format(reason))
    print("  " + " → ".join(
        ("[{}]" if r == rung else "{}").format(r) for r in RUNGS))
    if rung == "abyss":
        print("  Zero tokens. `chaos search {}`".format(" ".join(task.split()[:5])))
    elif rung == "legion":
        print("  Minimum prompt for the agent: only what ITS task demands.")
    elif rung == "double-judge":
        print("  And facing the irreversible: I arrive with the decision made"
              " and wait for your word.")
    return rung

def _claims(text):
    """Splits the text into CHECKABLE claims. A sentence with no figure, no
    proper name and no absolute claims nothing verifiable: it is opinion, and
    opinion does not enter the tribunal."""
    raw = re.split(r"(?<=[.!?;\n])\s+", text or "")
    out = []
    for f in raw:
        f = " ".join(f.split())
        if len(f) < 18:
            continue
        has_figure = bool(re.search(r"\d", f))
        has_path = bool(re.search(r"[\w/-]+\.\w{2,4}\b|`[^`]+`", f))
        has_absolute = bool(_ABSOLUTES.search(f))
        if has_figure or has_path or has_absolute:
            out.append(f[:300])
    return out

def _numeric_pairs(s):
    """(figure, noun): "56 commands" → ("56", "commands").

    Comparing LOOSE figures is noise: a block with twenty numbers matches any
    of them by chance, and that is how "56 commands" SURVIVED while I had 60.
    A figure only means something bound to what it counts."""
    # From the RAW lowercased text, not from _norm: normalisation splits
    # "10.5" into "10 5" and the pair became ("5", "ghz") — evidence that
    # states half a number is worse than none.
    low = (s or "").lower()
    pairs = set()
    # Two letters are enough: units are short ("24 GHz", "3 km", "8 MB") and
    # demanding four left out exactly the figures that lie most. Connectors are
    # dropped or "24 de" would be a pair.
    # The figure is kept LITERAL: stripping the dot turned "10.5 GHz" into
    # "105 GHz" and the evidence showed a number nobody wrote. Declared limit:
    # "1,000" and "1000" read as different.
    for m in re.finditer(r"(\d[\d.,]*\d|\d)\s+([a-záéíóúñ]{2,})", low):
        if m.group(2) not in _CONNECTORS:
            pairs.add((m.group(1), m.group(2)))
    for m in re.finditer(r"([a-záéíóúñ]{3,})\s*(?:es|son|is|are|=|:)\s*(\d[\d.,]*\d|\d)", low):
        if m.group(1) not in _CONNECTORS:
            pairs.add((m.group(2), m.group(1)))
    return pairs

def _judge_one(con, claim):
    """(verdict, evidence). Zero tokens: the tribunal is my own memory."""
    words = [w for w in re.findall(r"[\wáéíóúñü]{4,}", _text._norm(claim))
             if w not in _text._STOP and not w.isdigit()]
    if not words:
        return "suspended", "no weighted terms to search for"
    try:
        q = _text._fts_query(claim)
        rows = con.execute(
            "SELECT slug, block_id, content FROM blocks WHERE blocks"
            " MATCH ? ORDER BY rank LIMIT 4", (q,)).fetchall()
    except sqlite3.OperationalError:
        rows = []
    pairs = _numeric_pairs(claim)
    for slug, bid, content in rows:
        body = _text._norm(str(content or ""))
        covered = sum(1 for w in words if w in body)
        if covered / float(len(words)) < 0.6:
            continue
        ref = "{}#^{}".format(slug, bid)
        if pairs:
            theirs = _numeric_pairs(str(content or ""))
            for figure, thing in pairs:
                same = [c for c, x in theirs if x == thing]
                if not same:
                    continue                 # the block does not count that thing
                if figure in same:
                    return "survives", "{} confirms {} {}".format(ref, figure, thing)
                return "dies", ("{} says {} {} where you say {}"
                                .format(ref, same[0], thing, figure))
            # It covers the words but does NOT confirm the figure: no warrant.
            return "suspended", ("{} speaks of the topic but does not count {}"
                                 .format(ref, ", ".join(c for _, c in sorted(pairs))[:60]))
        return "survives", "{} ({}/{} terms)".format(ref, covered, len(words))
    return "suspended", "the Void does not hold this"

def judge(text=None, eyes=False):
    """J-1 · Submits a text to my own tribunal. No network and no cost."""
    if not text:
        print('Usage: chaos judge "<text>" | chaos judge <file.md> [--eyes]')
        return
    if os.path.isfile(text):
        text = _text.read_file(text)
    claims = _claims(text)
    if not claims:
        print("No checkable claim. This is opinion, and opinion does not enter"
              " the tribunal.")
        return
    con = _sense.db()
    count = {"survives": 0, "dies": 0, "suspended": 0}
    suspended = []
    print("TRIBUNAL · {} checkable claim(s)\n".format(len(claims)))
    for a in claims:
        verdict, evidence = _judge_one(con, a)
        count[verdict] += 1
        mark = {"survives": "✅", "dies": "❌", "suspended": "⏸"}[verdict]
        print("{} {}".format(mark, a[:150]))
        print("   {} · {}".format(verdict.upper(), evidence))
        if verdict == "suspended":
            suspended.append(a)
    print("\nSURVIVE {} · DIE {} · SUSPENDED {}".format(
        count["survives"], count["dies"], count["suspended"]))
    # THE SEAMS: a verdict without them is a naked verdict.
    print("SEAMS: tribunal = my Abyss ({} blocks). What is SUSPENDED is not"
          " refuted: it is unverified.".format(
              con.execute("SELECT COUNT(*) FROM blocks").fetchone()[0]))
    if eyes and suspended:
        _judge_with_eyes(suspended)
    elif suspended:
        print("To take them to the world: `chaos judge ... --eyes` (it invokes"
              " and declares its cost).")

def _judge_with_eyes(suspended):
    """What my memory does not hold goes out to the world — bounded and
    confessed. It looks for the REFUTATION first: whoever only seeks to
    confirm has already failed."""
    if not shutil.which("claude"):
        print("\n[EYES] `claude` does not live in this body: I cannot go out to"
              " the world. Declared, not faked.")
        return
    order = ("Verify these claims by looking for their REFUTATION first. For "
             "each one: SURVIVES (with source), DIES (with the why) or DOES NOT "
             "CONVERGE. Be brief.\n\n" + "\n".join("- " + s for s in suspended))
    try:
        p = subprocess.run(
            ["claude", "-p", order, "--bare", "--output-format", "json",
             "--allowedTools", "WebSearch,WebFetch"],
            capture_output=True, text=True, timeout=300)
        data = json.loads(p.stdout or "{}")
        print("\n[EYES] " + (data.get("result") or "(no answer)")[:1500])
        cost = data.get("total_cost_usd")
        if cost is not None:
            con = _routes_table(_sense.db())
            _sense.write_verified(
                con, "INSERT INTO routes(date, task, rung, reason, cost)"
                " VALUES (?,?,?,?,?)",
                (datetime.datetime.now().isoformat(timespec="seconds"),
                 "judgment of {} claim(s)".format(len(suspended)),
                 "double-judge", "went out to the world for suspended claims",
                 float(cost)))
            print("[EYES] Real cost of this sortie: {:.4f} USD (carved into"
                  " `chaos route --report`).".format(float(cost)))
    except Exception as e:
        print("\n[EYES] The sortie into the world failed: {}. Declared.".format(e))

RUNGS = ("cli", "abyss", "legion", "me", "double-judge")

# Verbs that betray MECHANICAL work: simple criteria, high volume.
# INFINITIVES only: in Spanish «lista», «cuenta» and «copia» are such common
# nouns that "sort a list of integers" fell into mechanical — and writing a
# function is not. An ambiguous verb is not a signal, it is noise.
_MECHANICAL = re.compile(
    r"\b(listar|renombrar|extraer|contar|convertir|reemplazar|ordenar|"
    r"formatear|copiar|mover|traducir|transcribir|limpiar|deduplicar|"
    r"numerar|indexar|tabular|recortar|comprimir|descargar|"
    r"rename|extract|convert|replace|reformat|transcribe|deduplicate|"
    r"renumber|reindex|tabulate|truncate|compress|download)\b", re.I)

# What demands the maximum power: if I get it wrong, the damage does not undo.
_CRITICAL = re.compile(
    r"\b(arquitectura|seguridad|vulnerab\w*|credencial\w*|migrar|migración|"
    r"borrar|aniquilar|eliminar|producción|dinero|pago|factura|legal|"
    r"contrato|veredicto|auditar|auditoría|irreversible|desplegar|deploy|"
    r"cifrad\w*|permis\w*|autenticaci\w*|"
    r"architecture|security|vulnerab\w*|credential\w*|migrate|migration|"
    r"delete|annihilate|production|money|payment|invoice|contract|verdict|"
    r"audit|encrypt\w*|permission\w*|authenticat\w*)\b", re.I)

_ABSOLUTES = re.compile(r"\b(siempre|nunca|jamás|jamas|todos?|todas?|ningun\w*|"
                        r"cero|único|unica|imposible|garantiz\w*|"
                        r"always|never|every|none|zero|only|impossible|"
                        r"guarantee\w*)\b", re.I)

_CONNECTORS = frozenset(("de", "del", "la", "el", "los", "las", "en",
                        "por", "con", "para", "que", "un", "una", "al",
                        "of", "the", "in", "on", "at", "to", "and"))
