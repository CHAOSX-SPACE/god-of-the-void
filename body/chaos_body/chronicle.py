# -*- coding: utf-8 -*-
""""chronicle" — an organ of CHAOS's body.

Moved from the monolith by `partir-monolito.py`: not one line was
rewritten by hand. House rule: MODULES are imported here, never names —
the attribute resolves when CALLED, and that is why cycles between
organs are harmless (`a cycle fixture in the forge` proves it on three
systems). And nothing runs at import time: no constant in this module
may read the home, the environment or the disk (judge E1.4).
"""
import datetime, io, os, re, sqlite3, sys, time
import home as _home
from chaos_body.core import text as _text
from chaos_body.core import sense as _sense
from chaos_body.core import territory as _territory
from chaos_body import abyss as _abyss
from chaos_body import weave as _weave


def debts_settle(which=None, because=""):
    """Front 6 of the Plan of Perfection. I settled 26 debts with raw SQL
    because this command did not exist: a god who bypasses his own app admits
    the app is incomplete. Settling declares that work HAS sedimented."""
    con = _sense.db()
    if not which:
        print("Name it: chaos debts settle <id|--all> [--because \"...\"]"); sys.exit(1)
    if which == "--all":
        n = con.execute("SELECT COUNT(*) FROM debts WHERE settled=0").fetchone()[0]
        if not n:
            print("No open debts. All work is sedimented."); return 0
        con.execute("UPDATE debts SET settled=1 WHERE settled=0")
        con.commit()
        print("[CHAOS] {} debt(s) settled{}.".format(n, " - " + because if because else ""))
        return n
    row = con.execute("SELECT works, date, settled FROM debts WHERE id=?",
                      (int(which),)).fetchone()
    if not row:
        print("Debt #{} does not exist.".format(which)); sys.exit(1)
    if row[2]:
        print("Debt #{} was already settled.".format(which)); return 0
    con.execute("UPDATE debts SET settled=1 WHERE id=?", (int(which),))
    con.commit()
    print("[CHAOS] Debt #{} settled: {} work(s) from {}{}.".format(
        which, row[0], str(row[1])[:10], " - " + because if because else ""))
    return 1

def note(text, cwd=None):
    """E9 · A spark falls into the Void. I decide where it lives (3 levels)."""
    con = _sense.db()
    territory, focus = _territory._territory_and_focus(cwd)
    # Level 3: semantic anchor — the closest essence (The Sense)
    anchor, sem = None, 0.0
    try:
        rows = con.execute("SELECT slug FROM essences WHERE essences MATCH ?"
                           " ORDER BY rank LIMIT 1", (_text._fts_query(text),)).fetchall()
        if rows:
            anchor = rows[0][0]
            sem = 0.8                     # there was a real semantic match
    except sqlite3.OperationalError:
        pass
    same_terr = 1.0 if (anchor and _text._norm(anchor).find(_text._norm(territory)[:8]) >= 0) else 0.0
    focus_active = 1.0 if focus else 0.0
    confidence = round(0.5 * sem + 0.3 * same_terr + 0.2 * focus_active, 2)
    if confidence < 0.35:                 # Law of the Honest Spark
        anchor = None
    context = "forging {}".format(focus) if focus else "no work in progress"
    con.execute("INSERT INTO notes(text,territory,focus,anchor,confidence,context,date)"
                " VALUES (?,?,?,?,?,?,?)",
                (_sense.purge(text)[0], territory, focus, anchor, confidence, context,
                 datetime.datetime.now().isoformat(timespec="seconds")))
    con.commit()
    nid = con.execute("SELECT last_insert_rowid()").fetchone()[0]
    print("[CHAOS] Spark #{} devoured. Territory: {} · Focus: {} · Anchor: {} (confidence {:.2f})"
          .format(nid, territory, focus or "—", anchor or "no anchor", confidence))

def notes(query=None):
    con = _sense.db()
    if query:
        rows = con.execute("SELECT id, date, text, territory FROM notes"
                           " WHERE state='alive' AND text LIKE ? ORDER BY id",
                           ("%" + query + "%",)).fetchall()
    else:
        rows = con.execute("SELECT id, date, text, territory FROM notes"
                           " WHERE state='alive' ORDER BY id").fetchall()
    if not rows:
        print("No living spark."); return
    for i, f, t, terr in rows:
        print("#{} ({}) [{}] {}".format(i, f[:10], terr, t[:120]))
    print("({} spark(s))".format(len(rows)))

def note_where(nid):
    """Where that note landed and why — the three levels + context."""
    r = _sense.db().execute("SELECT text, territory, focus, anchor, confidence, context, date, state"
                     " FROM notes WHERE id=?", (nid,)).fetchone()
    if not r:
        print("That spark does not exist."); return
    txt, terr, focus, anchor, conf, ctx, date, state = r
    print("«{}»".format(txt))
    print("  territory  : {}".format(terr))
    print("  focus      : {}".format(focus or "— (no document under work)"))
    print("  anchor     : {}".format(anchor or "no anchor (it fit nothing; I do not invent)"))
    print("  context    : {}".format(ctx))
    print("  when       : {}  ·  confidence {:.2f}  ·  {}".format(date[:16], conf or 0, state))

def ascend(nid):
    """Mature spark → essence, with the Weave's grammar already in place."""
    con = _sense.db()
    r = con.execute("SELECT text, territory, focus, anchor, date FROM notes"
                    " WHERE id=? AND state='alive'", (nid,)).fetchone()
    if not r:
        print("That spark does not exist or already ascended."); return
    txt, terr, focus, anchor, date = r
    slug = re.sub(r"[^a-z0-9]+", "-", _text._norm(txt)[:40]).strip("-") or "spark-{}".format(nid)
    path = os.path.join(_home.essences(), slug + ".md")
    if os.path.exists(path):
        print("An essence with that name already exists. Rename the spark."); return
    body = ("---\ntype: reference\nstate: active\ndevoured: {}\ncoverage: partial\n---\n\n"
            "# {}\n\n## Essence\n{}\n\n## Hooks\nIt was born as a spark in the territory "
            "**{}**{}.\n{}\n"
            .format(date[:10], txt[:70], txt,
                    terr, ", about `{}`".format(focus) if focus else "",
                    "\n## Links\n[[{}]]\n".format(anchor) if anchor else ""))
    os.makedirs(_home.essences(), exist_ok=True)
    with io.open(path, "w", encoding="utf-8") as f:
        f.write(body)
    con.execute("UPDATE notes SET state='ascended' WHERE id=?", (nid,)); con.commit()
    _abyss.devour(path, silent=True); _weave.weave()
    print("[CHAOS] Spark #{} ascended to essence: {}".format(nid, slug))

def chronicle(what=None, why=None, kind="modification", cwd=None):
    """E9 · The LOGBOOK: documents CHANGES, never conversations."""
    con = _sense.db()
    if not what:
        rows = con.execute("SELECT id, date, territory, kind, what, why"
                           " FROM logbook ORDER BY id DESC LIMIT 20").fetchall()
        if not rows:
            print("Empty logbook. Nothing has been forged (or nothing documented)."); return
        for i, f, terr, kd, q, pq in rows:
            print("#{} ({}) [{}/{}] {}".format(i, f[:16], terr, kd, q))
            if pq:
                print("     why: {}".format(pq))
        return
    territory, _ = _territory._territory_and_focus(cwd)
    here = os.path.realpath(cwd or os.getcwd())
    here_same, all_entries = [], []
    if os.path.exists(_home.trail()):
        with io.open(_home.trail(), encoding="utf-8", errors="replace") as f:
            for l in f:
                d = _trail_line(l)
                if not d:
                    continue
                all_entries.append(os.path.basename(d["path"]))
                # same realpath as the FOCUS: without this it linked 0 files
                if not d["cwd"] or os.path.realpath(d["cwd"]) == here:
                    here_same.append(os.path.basename(d["path"]))
    # Consistency with `undocumented` (which counts the WHOLE trail): if
    # nothing matches this territory but there was work, all of it is linked
    # and declared.
    files = here_same or all_entries
    alien = "" if here_same or not all_entries else " (from other territories — declared)"
    con.execute("INSERT INTO logbook(date,territory,kind,what,why,files)"
                " VALUES (?,?,?,?,?,?)",
                (datetime.datetime.now().isoformat(timespec="seconds"), territory,
                 kind, _sense.purge(what)[0] if what else what,
                 _sense.purge(why)[0] if why else why,
                 ", ".join(sorted(set(files))[:12])))
    con.commit()
    print("[CHAOS] Chronicle recorded. {} file(s) linked{}.".format(len(set(files)), alien))

def _is_entry(line):
    """A work starts with its date. The rest is DEBRIS from the bug that split
    multiline commands (caught in phase 2): 1,016 fragments inflated the
    Chronicle's duty until it became unreadable. They are not work, so they are
    neither counted nor distilled: they are swept, saying how many."""
    return bool(_ENTRY.match(line or ""))

def _trail_works():
    """How many WORKS are in the trail. Gazes do not count: looking creates
    nothing to document, and an inflated duty stops being read (O-1)."""
    n = 0
    try:
        with io.open(_home.trail(), encoding="utf-8", errors="replace") as fh:
            for l in fh:
                if not _is_entry(l):
                    continue                  # debris, not work
                p = l.rstrip("\n").split("\t")
                if len(p) >= 6 and p[3] == "gaze":
                    continue
                n += 1
    except OSError:
        return 0
    return n

def chronicle_distil(session=None, cwd=None):
    """CR-1 · CLOSES THE LOOP. The trail came in and never went out: 1,517
    works and a five-line logbook. Here it is grouped by territory and day and
    left as a RAW entry — raw is worth more than none, and the Vigil polishes
    it later. What is distilled is purged from the trail: otherwise the duty
    grows forever."""
    if not os.path.exists(_home.trail()):
        print("Empty trail: nothing to distil."); return 0
    groups, remain, debris = {}, [], 0
    with io.open(_home.trail(), encoding="utf-8", errors="replace") as f:
        for l in f:
            if not _is_entry(l):
                debris += 1
                continue                      # swept: it never was a work
            p = l.rstrip("\n").split("\t")
            # The OLD format (3 fields) is half my trail and the distiller
            # skipped it entirely: the duty never went down.
            if len(p) == 3:
                p = [p[0], "", "", p[1], p[2], ""]
            elif len(p) == 5:
                p = p + [""]                  # 5-field format: no tool
            if len(p) < 6 or p[3] == "gaze":
                remain.append(l); continue
            if session and p[1] and p[1] != session:
                remain.append(l); continue
            key = (_territory.territory_name(p[2]) or "?", p[0][:10])
            g = groups.setdefault(key, {"n": 0, "files": [], "actions": {}})
            g["n"] += 1
            g["actions"][p[3]] = g["actions"].get(p[3], 0) + 1
            base = os.path.basename(p[4])[:40]
            if base and base not in g["files"] and len(g["files"]) < 8:
                g["files"].append(base)
    if not groups:
        if debris:
            with io.open(_home.trail(), "w", encoding="utf-8") as f:
                f.writelines(remain)
            print("[CHAOS] Nothing to distil; {} debris line(s) swept.".format(debris))
            return 0
        print("Nothing to distil from this session."); return 0
    con = _sense.db()
    n = 0
    for (territory, day), g in sorted(groups.items()):
        dominant = max(g["actions"].items(), key=lambda x: x[1])[0]
        what = "{}: {} work(s), mostly {} - {}".format(
            day, g["n"], dominant, ", ".join(g["files"]))
        if con.execute("SELECT 1 FROM logbook WHERE territory=? AND what=?",
                       (territory, what)).fetchone():
            continue                          # idempotent
        _sense.write_verified(
            con, "INSERT INTO logbook(date, territory, kind, what, why)"
            " VALUES (?,?,?,?,?)",
            (datetime.datetime.now().isoformat(timespec="seconds"), territory,
             "distilled", what,
             "distilled from the trail at closing: raw is worth more than none"))
        n += 1
    with io.open(_home.trail(), "w", encoding="utf-8") as f:
        f.writelines(remain)
    print("[CHAOS] Distilled {} logbook entr(ies); the trail drops to {} line(s)."
          .format(n, len(remain)))
    if debris:
        print("   {} debris line(s) swept (fragments of the multiline bug, now"
              " cured): they never were work.".format(debris))
    return n

def undocumented():
    """Was there work without a chronicle? Empty trail = only words = nothing to document."""
    if not os.path.exists(_home.trail()) or os.path.getsize(_home.trail()) == 0:
        print("Nothing to document: there was no work, only words. The Chronicle records acts.")
        return
    n = _trail_works()
    if not n:
        # The trail may hold GAZES and no work: announcing "0 work(s)" is
        # noise shaped like a duty.
        print("Nothing to document: there was no work, only words. The Chronicle records acts.")
        return
    con = _sense.db()
    last = con.execute("SELECT date FROM logbook ORDER BY id DESC LIMIT 1").fetchone()
    print("CHRONICLE DUTY: {} work(s) in the trail.".format(n))
    print("  last chronicle: {}".format(last[0][:16] if last else "never"))
    print("  → `chaos chronicle --what \"...\" --why \"...\"` and then `chaos trail --purge <session>`")

def export_chronicle():
    """The Chronicle survives the death of the DB: markdown is the last truth."""
    con = _sense.db()
    target = os.path.join(os.path.dirname(_home.essences()), "chronicle")
    os.makedirs(target, exist_ok=True)
    months = {}
    for i, f, terr, kd, q, pq, files in con.execute(
            "SELECT id, date, territory, kind, what, why, files FROM logbook ORDER BY id"):
        months.setdefault(f[:7], []).append((i, f, terr, kd, q, pq, files))
    for month, rows in months.items():
        with io.open(os.path.join(target, month + ".md"), "w", encoding="utf-8") as fh:
            fh.write("# Chronicle — {}\n\n".format(month))
            for i, f, terr, kd, q, pq, files in rows:
                fh.write("## #{} · {} · {}\n- **what**: {}\n- **why**: {}\n"
                         "- **territory**: {} · **kind**: {}\n- **files**: {}\n\n"
                         .format(i, f[:16], q[:60], q, pq or "—", terr, kd, files or "—"))
    print("[CHAOS] Chronicle exported: {} month(s) in {}".format(len(months), target))

def debts(settle=None):
    """C4 · debts of sessions that died without distilling their trail."""
    con = _sense.db()
    con.commit()
    if settle:
        con.execute("UPDATE debts SET settled=1 WHERE id=?", (settle,))
        con.commit(); print("Debt settled."); return
    rows = con.execute("SELECT id, date, works, sample FROM debts "
                       "WHERE settled=0 ORDER BY id").fetchall()
    if not rows:
        print("No debts. Everything forged was sedimented."); return
    for i, f, o, m in rows:
        print("#{} ({}) {} work(s) unsedimented: {}".format(i, f[:16], o, m))

def _is_noise(path, cwd=None):
    """What regenerates itself is always noise. What lives in a staging
    folder is noise UNLESS the work is happening inside it."""
    if _NOISE.search(path):
        return True
    if _EPHEMERAL.search(path):
        here = os.path.realpath(cwd) if cwd else None
        return not (here and os.path.realpath(path).startswith(here))
    return False

def _trail_line(l):
    """Reads a trail line in the new format (6 fields) or the old one (3)."""
    p = l.rstrip("\n").split("\t")
    if len(p) >= 6:
        return {"ts": p[0], "session": p[1], "cwd": p[2], "action": p[3],
                "path": p[4], "tool": p[5]}
    if len(p) == 3:  # old format: it is read, it is not broken
        return {"ts": p[0], "session": "", "cwd": "", "action": p[1],
                "path": p[2], "tool": ""}
    return None

def _flat(x):
    """No trail field may carry a newline or a tab: the format is ONE work per
    line, six fields. A single heredoc broke the whole thing."""
    return re.sub(r"[\t\r\n]+", " ", str(x or "")).strip()

def trail(file=None, action=None, session=None, cwd=None, tool=None):
    """C1 · FOUNDATION: the trail stores iso8601, session, cwd and tool.
    Without cwd+time the FOCUS of the notes is incomputable."""
    if file == "--purge":
        # C3 · purge per SESSION: session A no longer erases B's undocumented work.
        if not os.path.exists(_home.trail()):
            print("Trail already empty."); return
        target = action  # `chaos trail --purge <session>`
        if not target:
            os.remove(_home.trail())
            print("Trail purged ENTIRELY. (Use `--purge <session>` so other sessions are not trampled.)")
            return
        remaining = []
        removed = 0
        with io.open(_home.trail(), encoding="utf-8", errors="replace") as f:
            for l in f:
                d = _trail_line(l)
                if d and d["session"] == target:
                    removed += 1
                else:
                    remaining.append(l)
        with io.open(_home.trail(), "w", encoding="utf-8") as f:
            f.writelines(remaining)
        print("Trail purged: {} line(s) of session {}. {} from other sessions intact."
              .format(removed, target[:8], len(remaining)))
        return
    if not file:  # show the pending diary
        if os.path.exists(_home.trail()):
            with io.open(_home.trail(), "r", encoding="utf-8", errors="replace") as f:
                print(f.read().rstrip() or "(empty trail)")
        else:
            print("(empty trail — nothing forged yet)")
        return
    if _is_noise(file, cwd):  # the Abyss does not log noise
        return
    os.makedirs(os.path.dirname(_home.trail()), exist_ok=True)
    with io.open(_home.trail(), "a", encoding="utf-8") as f:
        f.write("{}\t{}\t{}\t{}\t{}\t{}\n".format(
            datetime.datetime.now().isoformat(timespec="seconds"),
            _flat(session), _flat(cwd), _flat(action) or "edit",
            _flat(file), _flat(tool)))

_ENTRY = re.compile(r"^\d{4}-\d\d-\d\dT")

# Noise is what REGENERATES itself.
_NOISE = re.compile(r"(scratchpad|\.log$|\.pyc$|node_modules|\.git/)")

# A STAGING folder is different: garbage nearly always, and THE WORLD
# when the work happens inside it. I had "/tmp/" as plain noise, and on
# Linux — containers, CI, staging folders — that killed the WHOLE trail
# in silence, and with it the Chronicle, the FOCUS of sparks and
# "undocumented". On macOS temporary things hang off /var/folders, so no
# test ever saw it until CI ran in three worlds.
_EPHEMERAL = re.compile(r"(^|/)(tmp|temp)/")
