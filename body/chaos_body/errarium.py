# -*- coding: utf-8 -*-
""""errarium" — an organ of CHAOS's body.

Moved from the monolith by `partir-monolito.py`: not one line was
rewritten by hand. House rule: MODULES are imported here, never names —
the attribute resolves when CALLED, and that is why cycles between
organs are harmless (a cycle fixture in the forge proves it on three
systems). And nothing runs at import time: no constant in this module
may read the home, the environment or the disk (judge E1.4).
"""
import datetime, io, os, re, sqlite3, sys, time
import home as _home
from chaos_body.core import text as _text
from chaos_body.core import sense as _sense
from chaos_body.core import territory as _territory


def _faults_ambush(con, fts_q):
    """THE FAULTS ambush me: searching a topic where we already erred surfaces
    the lesson BEFORE the work begins. Staying ahead."""
    try:
        for rid, tit, les in con.execute(
                "SELECT rowid, title, lesson FROM faults WHERE faults MATCH ?"
                " ORDER BY rank LIMIT 2", (fts_q,)).fetchall():
            # cured or alive, no matter: the LESSON ambushes forever —
            # relapses happen precisely on faults already cured.
            print("⚠ KNOWN FAULT #{}: {}{}".format(
                rid, tit, " — " + les[:120] if les else ""))
    except sqlite3.OperationalError:
        pass

def _export_faults(con):
    """DERIVED index of the errarium (the DB is the primary truth, as in the
    Chronicle). Fully regenerated: readable for the human, queryable for me."""
    rows = con.execute(
        "SELECT rowid, title, symptom, cause, cure, lesson, territory,"
        " date, state, repeats, last FROM faults ORDER BY rowid DESC").fetchall()
    out = ["# THE FAULTS — the errarium of CHAOS",
           "",
           "> *Life is chaos, and every work inherits its forger's errors.*",
           "> *Committing an error is normal. Repeating it is not.*",
           "",
           "<!-- DERIVED from the DB (`chaos fault/faults`). Do not edit by hand. -->",
           ""]
    for (rid, tit, sym, cau, cur, les, ter, dat, st, rep, last) in rows:
        mark = "🩹" if st == "cured" else "🔴"
        out.append("## #{} {} {}  `[{}]`".format(rid, mark, tit, ter or "?"))
        out.append("- **Date**: {} · **State**: {}{}".format(
            dat, st, " · **RELAPSES: {}** (last {})".format(rep, last)
            if rep and int(rep) > 0 else ""))
        if sym: out.append("- **Symptom**: " + sym)
        if cau: out.append("- **Cause**: " + cau)
        if cur: out.append("- **Cure**: " + cur)
        if les: out.append("- **Lesson**: " + les)
        out.append("")
    try:
        os.makedirs(os.path.dirname(_home.faults_md()), exist_ok=True)
        with io.open(_home.faults_md(), "w", encoding="utf-8") as f:
            f.write("\n".join(out))
    except Exception:
        pass

def fault(title, symptom="", cause="", cure="", lesson="", territory=None):
    """Records a fault in the errarium. Every crack in the work lands here:
    what was seen, why it happened, how it was cured, and the rule that
    forbids repeating it."""
    if not title.strip():
        print("A fault without a name cannot be remembered."); sys.exit(1)
    ter = territory or _territory._territory_and_focus()[0]
    con = _sense.db()
    con.execute("INSERT INTO faults(title,symptom,cause,cure,lesson,territory,"
                "date,state,repeats,last) VALUES(?,?,?,?,?,?,?,?,?,?)",
                (_sense.purge(title)[0], _sense.purge(symptom)[0], _sense.purge(cause)[0],
                 _sense.purge(cure)[0], _sense.purge(lesson)[0], ter,
                 datetime.date.today().isoformat(), "alive", "0", ""))
    con.commit()
    rid = con.execute("SELECT MAX(rowid) FROM faults").fetchone()[0]
    _export_faults(con)
    print("[CHAOS] Fault #{} recorded in the errarium ({}). Committing it was"
          " human; repeating it has no excuse left.".format(rid, ter))
    if cure and not _cure_anchored(cure):
        # P-7 · it WARNS, it never blocks: a half-recorded fault is worse than
        # a loose cure. A cure written in prose is closed on faith; one with a
        # file and a string is closed on evidence — and `faults --probe` can
        # measure it.
        print("   ! that cure cites nothing verifiable: it will be closed on faith.")
        print("     A shape that can be probed:  `<string>` lives in `<path/file>`")
    return rid


def _cure_anchored(cure):
    """Does the cure name a file AND a string that can be measured? Five of my
    live faults were written in prose and none can be closed without taking my
    own word for it."""
    has_path = bool(re.search(r"[\w/.-]+\.(py|sh|md|json|yml|yaml|html)\b", cure))
    has_string = cure.count("`") >= 2
    return has_path and has_string

def faults(query=None, territory=None):
    """Queries the errarium. No arguments: the living ones. With a query:
    searches with the Sense. Staying ahead = reading this BEFORE forging."""
    con = _sense.db()
    rows = []
    if query:
        try:
            rows = con.execute(
                "SELECT rowid, title, cause, cure, lesson, territory, state,"
                " repeats FROM faults WHERE faults MATCH ?"
                " ORDER BY rank LIMIT 10", (_text._fts_query(query),)).fetchall()
        except sqlite3.OperationalError:
            rows = []
        if not rows:  # LIKE rescue: an unfound fault is a fault repeated
            like = "%" + query.strip() + "%"
            rows = con.execute(
                "SELECT rowid, title, cause, cure, lesson, territory, state,"
                " repeats FROM faults WHERE title LIKE ? OR cause LIKE ?"
                " OR lesson LIKE ? OR cure LIKE ? LIMIT 10", (like, like, like, like)).fetchall()
    else:
        q = "SELECT rowid, title, cause, cure, lesson, territory, state, repeats FROM faults"
        args = []
        if territory:
            q += " WHERE territory=?"; args.append(territory)
        q += " ORDER BY rowid DESC LIMIT 15"
        rows = con.execute(q, args).fetchall()
    if not rows:
        print("The errarium holds nothing like that. Either we never erred that way… or we never confessed it.")
        return
    # the header counts WHAT IS SHOWN: quoting the global total after a
    # filter is a true number answering a different question - i.e. a lie
    if query or territory:
        if territory and not query:
            tot, alive = con.execute(
                "SELECT COUNT(*), SUM(CASE WHEN state='alive' THEN 1 ELSE 0 END)"
                " FROM faults WHERE territory=?", (territory,)).fetchone()
        else:
            tot = len(rows)
            alive = sum(1 for f in rows if (f[6] if len(f) > 6 else "") == "alive")
        print("THE ERRARIUM — {} fault(s) in this filter, {} alive\n".format(tot, alive or 0))
    else:
        tot, alive = con.execute(
            "SELECT COUNT(*), SUM(CASE WHEN state='alive' THEN 1 ELSE 0 END) FROM faults").fetchone()
        print("THE ERRARIUM — {} fault(s), {} alive\n".format(tot, alive or 0))
    for rid, tit, cau, cur, les, ter, st, rep in rows:
        mark = "🩹" if st == "cured" else "🔴"
        rel = "  RELAPSED x{}!".format(rep) if rep and int(rep) > 0 else ""
        print("#{} {} [{}] {}{}".format(rid, mark, ter or "?", tit, rel))
        if cau: print("   cause: {}".format(cau[:150]))
        if cur: print("   SOLUTION: {}".format(cur[:180]))
        if les: print("   lesson: {}".format(les[:150]))

def faults_probe(territory=None, apply=False):
    """V-3 · CHEAP CLOSURE OF THE ERRARIUM. 400 live faults turn the alarm into
    wallpaper. For each one a probe is DERIVED from its own cure — does that
    string live in that file? — and closure is proposed WITH its evidence.
    What cannot be probed is LABELLED, never closed: closing a fault by reading
    its cure is fault #94."""
    con = _sense.db()
    q = ("SELECT rowid, title, cure, territory FROM faults WHERE state='alive'"
         + (" AND territory=?" if territory else "") + " ORDER BY rowid DESC LIMIT 200")
    rows = con.execute(q, (territory,) if territory else ()).fetchall()
    if not rows:
        print("No live fault {}.".format("in " + territory if territory else "")); return
    closable, mute = [], []
    for rid, tit, cure, _terr in rows:
        cure = cure or ""
        files = re.findall(r"\b([\w./-]{4,60}\.(?:py|sh|md|yml|yaml|json|sql|conf))\b", cure)
        strings = re.findall(r"`([^`\n]{4,60})`", cure)
        # THE FILE EXISTING PROVES NOTHING: `run-tests.sh` existed before the
        # fault and will exist after. The only evidence that bites is the
        # cure's STRING living INSIDE the file (organ 17).
        evidence = []
        for c in strings[:4]:
            for a in files[:4]:
                try:
                    if os.path.isfile(a) and c in _text.read_file(a):
                        evidence.append("«{}» lives in {}".format(c[:34], a))
                        break
                except OSError:
                    pass
        if evidence:
            closable.append((rid, tit, evidence))
        elif not strings:
            mute.append((rid, tit, "not-probeable: its cure quotes no string"))
        elif not files:
            mute.append((rid, tit, "not-probeable: it quotes a string but no file"))
        else:
            mute.append((rid, tit, "NOT CURED: what its cure promises is not in the code"))
    print("ERRARIUM PROBE ({} alive {})".format(
        len(rows), "in " + territory if territory else "in total"))
    if closable:
        print("\nCLOSABLE WITH EVIDENCE ({}):".format(len(closable)))
        for rid, tit, ev in closable[:20]:
            print("  #{:<5} {:<52} {}".format(rid, (tit or "")[:52], "; ".join(ev)[:70]))
    if mute:
        print("\nNOT PROBEABLE ({}) — LABELLED, never closed:".format(len(mute)))
        for rid, tit, why in mute[:10]:
            print("  #{:<5} {:<52} {}".format(rid, (tit or "")[:52], why))
        if len(mute) > 10:
            print("  … and {} more".format(len(mute) - 10))
    if apply and closable:
        for rid, tit, ev in closable:
            fault_cured(str(rid), "probed: " + "; ".join(ev)[:120])
        print("\n{} fault(s) closed WITH measured evidence.".format(len(closable)))
    elif closable:
        print("\nNothing was closed: `chaos faults --probe --apply` to close them"
              " with their evidence.")

def relapse(fid):
    """The sin: confessing that a KNOWN fault was committed AGAIN.
    It is counted and declared — numbered shame teaches more than oblivion."""
    con = _sense.db()
    row = con.execute("SELECT title, lesson, repeats FROM faults"
                      " WHERE rowid=?", (int(fid),)).fetchone()
    if not row:
        print("Fault #{} does not exist.".format(fid)); sys.exit(1)
    n = int(row[2] or 0) + 1
    con.execute("UPDATE faults SET repeats=?, last=?, state='alive'"
                " WHERE rowid=?",
                (str(n), datetime.date.today().isoformat(), int(fid)))
    con.commit()
    _export_faults(con)
    print("[CHAOS] Fault #{} RELAPSED (count: {}). «{}»".format(fid, n, row[0]))
    if row[1]:
        print("   The lesson we ignored: {}".format(row[1]))
    print("   To err was human. This is negligence now — mine or ours.")

def fault_cured(fid, cure=""):
    """Marks the fault as cured (the lesson stays alive forever)."""
    con = _sense.db()
    if not con.execute("SELECT 1 FROM faults WHERE rowid=?", (int(fid),)).fetchone():
        print("Fault #{} does not exist.".format(fid)); sys.exit(1)
    if cure:
        con.execute("UPDATE faults SET cure=?, state='cured' WHERE rowid=?",
                    (_sense.purge(cure)[0], int(fid)))
    else:
        con.execute("UPDATE faults SET state='cured' WHERE rowid=?", (int(fid),))
    con.commit()
    _export_faults(con)
    print("[CHAOS] Fault #{} cured. The work's scar remains; so does the lesson.".format(fid))

def fault_reopen(fid, reason=""):
    """REOPEN: the cure did not hold, or it was marked cured by mistake.

    Without this the errarium could only close. An organ that can declare
    'solved' but never 'I was wrong' piles up comfortable lies: the fault
    stays alive in the work and dead in the memory. This is NOT a relapse
    (nobody committed it again), so the relapse counter is left untouched —
    the state is corrected, no guilt is invented."""
    con = _sense.db()
    row = con.execute("SELECT state, title FROM faults WHERE rowid=?",
                      (int(fid),)).fetchone()
    if not row:
        print("Fault #{} does not exist.".format(fid)); sys.exit(1)
    if row[0] == "alive":
        print("[CHAOS] Fault #{} was already alive. Nothing to reopen.".format(fid))
        return
    con.execute("UPDATE faults SET state='alive' WHERE rowid=?", (int(fid),))
    if reason:
        con.execute("UPDATE faults SET cure=? WHERE rowid=?",
                    (_sense.purge("[REOPENED] " + reason)[0], int(fid)))
    con.commit()
    _export_faults(con)
    print("[CHAOS] Fault #{} REOPENED: \u00ab{}\u00bb".format(fid, row[1]))
    print("   It ambushes again until the cure is real. Closing what is still "
          "broken is worse than never recording it.")


def fault_show(ident):
    """P-3 · SHOWS a fault by its id. Without this, `chaos fault 500` took the
    number as a TITLE and gave birth to a new fault called "500" — I did it
    myself on 2026-09-05 (fault #506) and annihilated it a minute later. A
    command that creates and one that shows must never share a shape: if the
    argument is an id, the only sane action is to show."""
    con = _sense.db()
    row = con.execute(
        "SELECT rowid, title, symptom, cause, cure, lesson, territory,"
        " date, state, repeats, last FROM faults WHERE rowid=?",
        (int(ident),)).fetchone()
    if not row:
        print("[CHAOS] There is no fault #{} in the errarium.".format(ident))
        return None
    (rid, title, symptom, cause, cure, lesson, territory,
     date, state, reps, last) = row
    icon = "🔴" if (state or "alive") == "alive" else "🩹"
    print("{} FAULT #{} [{}] {}".format(icon, rid, territory or "?", title))
    print("   state: {}   date: {}{}".format(
        state or "alive", date or "?",
        # FTS5 stores UNINDEXED columns as TEXT: `"0"` is truthy and the
        # counter showed up with zero relapses. The number is compared.
        "   relapses: {} (last {})".format(reps, last or "?")
        if str(reps or "0").isdigit() and int(reps or 0) else ""))
    for label, text in (("symptom", symptom), ("cause", cause),
                        ("cure", cure), ("lesson", lesson)):
        if text:
            print("   {}: {}".format(label, text))
    return {"id": rid, "title": title, "symptom": symptom, "cause": cause,
            "cure": cure, "lesson": lesson, "territory": territory,
            "date": date, "state": state, "repeats": reps}
