#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""CHAOS PostToolUse hook: every work leaves a mark on the trail.
C1 · FOUNDATION: it now captures cwd, session_id, time and tool — the event
ALREADY carried them and they used to be thrown away, leaving the FOCUS
incomputable. Covers Write/Edit/NotebookEdit and the Bash that MUTATE
(git commit, mv, rm, deploy).
Silent and fail-safe: it NEVER breaks the tool that fires it."""
import sys, os, json, re, subprocess

# Bash that mutates the world (what used to be invisible to me)
_MUTATES = re.compile(
    r"\b(git\s+(commit|push|merge|rebase|reset|checkout|rm|mv)|"
    r"mv|rm|cp|chmod|chown|mkdir|touch|tee|dd|"
    r"npm\s+(i|install|publish)|pip\s+install|brew\s+install|"
    r"docker\s+(build|run|compose)|make|deploy|rsync|scp|"
    r"sed\s+-i|>>?\s*\S)")


def main():
    ev = json.load(sys.stdin)
    ti = ev.get("tool_input", {}) or {}
    tool = ev.get("tool_name", "")
    session = ev.get("session_id", "") or ""
    cwd = ev.get("cwd", "") or ""

    if tool in ("Write", "Edit", "NotebookEdit", "MultiEdit"):
        file = ti.get("file_path") or ti.get("path") or ti.get("notebook_path")
        action = "create" if tool == "Write" else "edit"
    elif tool == "Bash":
        command = (ti.get("command") or "")
        if not _MUTATES.search(command):
            return                      # read-only Bash: not a work
        file = "bash: " + command[:120]
        action = "run"
    else:
        return

    if not file:
        return
    app = os.path.join(os.path.expanduser("~"), ".chaos", "bin", "chaos.py")
    subprocess.call([sys.executable, app, "trail", file, action, session, cwd, tool],
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


try:
    main()
except Exception:
    pass  # a lock never jams the door
sys.exit(0)
