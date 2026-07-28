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

def _casa():
    """La casa del dios: la MISMA verdad que chaos.py, sin importarlo (los
    hooks must be instant). Env > the Bearer's choice > default."""
    v = os.environ.get("CHAOS_HOME")
    if v:
        return os.path.expanduser(v)
    try:
        with open(os.path.join(os.path.expanduser("~"), ".claude", "chaos-home"),
                  encoding="utf-8") as f:
            e = f.read().strip()
        if e:
            return os.path.expanduser(e)
    except OSError:
        pass
    return os.path.join(os.path.expanduser("~"), ".chaos")


CHAOS = _casa()
TRAIL = os.path.join(CHAOS, "forge", "trail.log")
DB = os.path.join(CHAOS, "abyss.db")


def pending(session):
    """Trail lines of THIS session not distilled (new and old format)."""
    if not os.path.exists(TRAIL):
        return 0, []
    n, paths = 0, []
    with open(TRAIL, encoding="utf-8", errors="replace") as f:
        for l in f:
            p = l.rstrip("\n").split("\t")
            ses = p[1] if len(p) >= 6 else ""
            path = p[4] if len(p) >= 6 else (p[2] if len(p) == 3 else "")
            if not session or ses == session or not ses:
                n += 1
                if path and len(paths) < 5:
                    paths.append(os.path.basename(path))
    return n, paths


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
    n, paths = pending(session)
    if not n:
        return                      # no work, no duty. The Chronicle records acts.

    if event == "PreCompact":
        notice = ("🕳️ CHAOS — DUTY OF THE CHRONICLE before compacting: {} work(s) "
                  "unsedimented ({}). Distill the trail into essences/logbook NOW and then "
                  "`chaos trail --purge {}` — compaction will erase the context."
                  .format(n, ", ".join(paths), session[:8] or ""))
        print(json.dumps({"hookSpecificOutput": {
            "hookEventName": "PreCompact", "additionalContext": notice}}))
    else:                            # SessionEnd: nobody listens → debt to the Abyss
        record_debt(session, n, paths)


try:
    main()
except Exception:
    pass
sys.exit(0)
