#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""CHAOS PostToolUse hook: every work leaves a mark on the trail.
C1 · FOUNDATION: it now captures cwd, session_id, time and tool — the event
ALREADY carried them and they used to be thrown away, leaving the FOCUS
incomputable. Covers Write/Edit/NotebookEdit and the Bash that MUTATE
(git commit, mv, rm, deploy).
Silent and fail-safe: it NEVER breaks the tool that fires it.

THE PURGE (2026-08-18): the trail used to store commands AS TYPED, so
`SUDO_PASS='…'` and `sshpass -p '…'` left live passwords in plain text inside
my own body — 289 lines, in a 644 file (fault #222). Rule 5 violated by my
own tool. Now every command passes through `gag()` before being stored, and
the file is born 600."""
import sys, os, json, re, subprocess

# What is NEVER written to the trail, even if it was typed.
_GAG = [
    (re.compile(r"(sshpass\s+-p\s*)('[^']*'|\"[^\"]*\"|\S+)"), r"\1«PURGED»"),
    (re.compile(r"([A-Z_]*(?:PASS|PASSWORD|SUDO_PASS|SECRET|SECRETO|TOKEN|KEY|LLAVE|CLAVE|PWD|CRED)[A-Z_]*\s*=\s*)('[^']*'|\"[^\"]*\"|\S+)"), r"\1«PURGED»"),
    (re.compile(r"(--password[= ]|--token[= ]|-u\s+\S+:)(\S+)"), r"\1«PURGED»"),
    (re.compile(r"([a-z][a-z0-9+.-]*://[^\s:/@]+:)[^\s:/@]+(@)"), r"\1«PURGED»\2"),
    (re.compile(r"\b(gh[pousr]_[A-Za-z0-9]{16,}|sk-[A-Za-z0-9]{20,}|AKIA[0-9A-Z]{16})\b"), "«PURGED»"),
    # `printf '%s' 'secret' | ssh …` and `echo 'secret' | sudo -S` — the shape
    # no keyword betrays: the literal travels bare, and what makes it a secret
    # is WHERE it is piped.
    (re.compile(r"((?:printf|echo)\s+(?:-\w+\s+)*(?:'%s'\s+)?)('[^']{4,}'|\"[^\"]{4,}\")(\s*\|\s*(?:ssh|sudo|mysql|psql|su\b|openssl))"), r"\1«PURGED»\3"),
    (re.compile(r"(sudo\s+-S[^|]*<<<\s*)('[^']*'|\"[^\"]*\"|\S+)"), r"\1«PURGED»"),
    # lowercase and Spanish too: `$password = '…'`, `clave='…'`, `pass: "…"`
    (re.compile(r"([$\w]*(?:pass|clave|secreto|token|llave|cred)\w*\s*[:=]>?\s*)('[^']*'|\"[^\"]*\")", re.I), r"\1«PURGED»"),
]


def _forbidden():
    """Literal strings that must never be written, listed in ~/.chaos/.gag.

       Pattern-based gagging cannot tell a key used as a credential from the
       same key typed inside an audit `grep`, and it must not try: every
       literal appearance of a secret IS a secret.

       Only a MISSING file means an empty list. Any other failure propagates
       and the hook records nothing: a broken gag must close the door, never
       pretend it found no secrets (fault #232)."""
    f = os.path.join(os.path.expanduser("~"), ".chaos", ".gag")
    try:
        with open(f, encoding="utf-8") as fh:
            return [l.strip() for l in fh
                    if l.strip() and not l.startswith("#")]
    except FileNotFoundError:
        return []


_FORBIDDEN = _forbidden()


def gag(command):
    """A trail is permanent memory: what enters here lives forever.
       No secret crosses this door, whatever it costs in context."""
    for literal in _FORBIDDEN:
        command = command.replace(literal, "«PURGED»")
    for pattern, repl in _GAG:
        command = pattern.sub(repl, command)
    return command

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
        file = "bash: " + gag(command)[:120]
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
