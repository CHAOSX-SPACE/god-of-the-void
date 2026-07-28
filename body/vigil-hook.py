#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""CHAOS SessionStart hook: fires the Vigil on its own, not by discipline.
On session start, if >=7 days passed since the last self-audit, it injects a
reminder into the context so CHAOS runs `chaos audit` at the close. Silent when
not due (most of the time it emits nothing). Fail-safe: never breaks startup."""
import sys, os, json, subprocess

try:
    app = os.path.join(os.path.expanduser("~"), ".chaos", "bin", "chaos.py")
    if os.path.exists(app):
        r = subprocess.run([sys.executable, app, "vigil-due"],
                           capture_output=True, text=True, timeout=8)
        if "YES" in (r.stdout or ""):
            note = ("🕳️ CHAOS — The Vigil is due (>=7 days without self-auditing). "
                    "At this session's close: `chaos audit`, read scars, and propose "
                    "improvements to the Bearer if anything is actionable.")
            print(json.dumps({"hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": note}}))
except Exception:
    pass  # a sleeping sentinel never jams the door
sys.exit(0)
