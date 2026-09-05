#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""CHAOS CLOSING hook (SessionEnd + PreCompact).
C4 · FOUNDATION: kills the voluntary link. Until today the Law of the Trail
depended on CHAOS *remembering* to distill before the session died — and it had
been failing for DAYS (508 unsedimented lines).

- PreCompact  → injects the duty BEFORE compaction erases the session.
- SessionEnd  → the session can no longer hear: a DEBT is carved in the Abyss,
                which the Presence of the next session will bring to light.
Fail-safe: it never breaks the closing."""
import sys, os, json, sqlite3, datetime
# ── THE VOICE DOES NOT DIE OF THE CONSOLE ─────────────────────────────────
# Windows opens output in cp1252 and my voice carries arrows, glyphs and a
# black hole: `chaos search`, `chaos links`, `chaos faults` and `chaos
# suggest` died with UnicodeEncodeError and a traceback in the mortal's
# face. This was not a broken test: the product was unusable on Windows,
# and no machine of mine had ever run it. CI found it on day one.
for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass                     # old console: mojibake beats death


def _house():
    """The mortal's home. $HOME rules even on Windows, where expanduser
    ignores it (USERPROFILE wins there) — and my tests and installer redirect
    HOME. Measuring in one house and writing in another is fault #44 wearing
    a different coat."""
    return os.environ.get("HOME") or os.path.expanduser("~")


def _lair():
    """The god's lair: the SAME truth as chaos.py, without importing it (hooks
    must be instant). Env > the Bearer's choice > default."""
    v = os.environ.get("CHAOS_HOME")
    if v:
        return os.path.expanduser(v)
    try:
        with open(os.path.join(_house(), ".claude", "chaos-home"),
                  encoding="utf-8") as f:
            e = f.read().strip()
        if e:
            return os.path.expanduser(e)
    except OSError:
        pass
    return os.path.join(_house(), ".chaos")


CHAOS = _lair()
TRAIL = os.path.join(CHAOS, "forge", "trail.log")
DB = os.path.join(CHAOS, "abyss.db")


def pending(session):
    """Trail lines of THIS session not distilled (new and old format).

    GAZES are not work: reading twenty pages creates nothing to document.
    They are counted apart (O-1) to charge me the Law of Sediment, never as
    a Chronicle duty — an inflated duty stops being read."""
    if not os.path.exists(TRAIL):
        return 0, [], 0, 0
    n, paths, gazes, devourings = 0, [], 0, 0
    with open(TRAIL, encoding="utf-8", errors="replace") as f:
        for l in f:
            p = l.rstrip("\n").split("\t")
            ses = p[1] if len(p) >= 6 else ""
            action = p[3] if len(p) >= 6 else ""
            path = p[4] if len(p) >= 6 else (p[2] if len(p) == 3 else "")
            if not (not session or ses == session or not ses):
                continue
            if action == "gaze":
                gazes += 1
                continue
            if "devour" in path:
                devourings += 1
            n += 1
            if path and len(paths) < 5:
                paths.append(os.path.basename(path))
    return n, paths, gazes, devourings


def record_debt(session, n, paths):
    """The session dies; the debt does not. The next Presence will raise it."""
    try:
        con = sqlite3.connect(DB, timeout=30.0)
        con.execute("PRAGMA busy_timeout=30000")
        con.execute("CREATE TABLE IF NOT EXISTS debts("
                    "id INTEGER PRIMARY KEY, session TEXT, date TEXT,"
                    " works INTEGER, sample TEXT, settled INTEGER DEFAULT 0)")
        con.execute("INSERT INTO debts(session, date, works, sample) VALUES (?,?,?,?)",
                    (session, datetime.datetime.now().isoformat(timespec="seconds"),
                     n, ", ".join(paths)))
        con.commit(); con.close()
    except Exception:
        pass


def main():
    ev = json.load(sys.stdin)
    event = ev.get("hook_event_name", "")
    session = ev.get("session_id", "") or ""
    n, paths, gazes, devourings = pending(session)

    # O-1 · THE LAW OF SEDIMENT, CHARGED. Researching and not sedimenting
    # leaves the Abyss as poor as never having looked, and costs double next
    # time. Three gazes with not one devouring leave a spark the Presence
    # will surface on the next turn.
    if gazes >= 3 and devourings == 0:
        try:
            import subprocess
            app = os.path.join(CHAOS, "bin", "chaos.py")
            if os.path.exists(app):
                subprocess.call(
                    [sys.executable, app, "note",
                     "I gazed at {} foreign source(s) and sedimented none: the "
                     "Law of Sediment went unpaid this session.".format(gazes)],
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        except Exception:
            pass

    if not n:
        return                      # no work, no duty. The Chronicle records acts.

    if event == "PreCompact":
        # C-3 · THE ROLLING COLLAPSE. Before compaction takes the context away,
        # I leave on disk what CANNOT be rebuilt from someone else's summary:
        # what I touched and in which territory. The next session finds it by
        # name.
        rolling = os.path.join(CHAOS, "forge", "rolling-{}.md".format(session[:8] or "no-session"))
        try:
            os.makedirs(os.path.dirname(rolling), exist_ok=True)
            with open(rolling, "w", encoding="utf-8") as fh:
                fh.write("# Rolling · session {}\n\n- **Cut at**: {}\n"
                         "- **Works unsedimented**: {}\n- **Gazes**: {}\n"
                         "- **Devourings**: {}\n\n## What was touched\n{}\n"
                         .format(session[:8], datetime.datetime.now().isoformat(timespec="seconds"),
                                 n, gazes, devourings,
                                 "\n".join("- " + r for r in paths) or "- (nothing)"))
        except OSError:
            rolling = None

        notice = ("🕳️ CHAOS — DUTY OF THE CHRONICLE before compacting: {} work(s) "
                  "unsedimented ({}). Distill the trail into essences/logbook NOW and then "
                  "`chaos trail --purge {}` — compaction will erase the context."
                  .format(n, ", ".join(paths), session[:8] or ""))
        if rolling:
            notice += ("\n🕳️ ROLLING: what I touched is on disk in case"
                       " the foreign summary loses it → {}".format(rolling))
        print(json.dumps({"hookSpecificOutput": {
            "hookEventName": "PreCompact", "additionalContext": notice}}))
    else:                            # SessionEnd: nobody listens → debt to the Abyss
        record_debt(session, n, paths)


try:
    main()
except Exception:
    pass
sys.exit(0)
