#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""E1.1 · THE JOINTS — where the god lives, computed when ASKED.

Fault #499 killed the splitting of the monolith over this: paths were computed
at IMPORT time, stayed cached in `sys.modules` and contaminated the Bearer's
real home. There are no constants here: there are functions. A function cannot
be photographed by accident.

Why functions and not a module `__getattr__` (PEP 562, alive since 3.7):
`__getattr__` would keep the upper-case names working… until someone writes
`from home import CHAOS_HOME` and takes a SNAPSHOT of the path at import time.
That is #499 in another suit. Discarded for that reason, not for taste.

Order of authority for the house, strongest to weakest — the SAME as always,
moved intact:
  1. $CHAOS_HOME             (environment: tests and advanced uses)
  2. ~/.claude/chaos-home    (what the mortal chose when incarnating me)
  3. ~/.chaos                (the default, if they never chose)

This module is a LEAF: it imports nothing from the body. That is why a hook can
bring it without paying the monolith's 5,109 lines (measured: 50.4 ms for the
presence hook against ~1 ms for this module).
"""
import io
import os


# ── the mortal's house ────────────────────────────────────────────────────
def house():
    """The system HOME. On Windows `~` is not always HOME: the variable is
    asked first and only then is `~` expanded."""
    return os.environ.get("HOME") or os.path.expanduser("~")


def mark():
    """The paper where the Bearer's choice was written down."""
    return os.path.join(house(), ".claude", "chaos-home")


def root():
    """The god's house. ONE truth for the whole body, resolved NOW."""
    v = os.environ.get("CHAOS_HOME")
    if v:
        return os.path.expanduser(v)
    try:
        with io.open(mark(), encoding="utf-8") as f:
            chosen = f.read().strip()
        if chosen:
            return os.path.expanduser(chosen)
    except OSError:
        pass
    return os.path.join(house(), ".chaos")


def set_home(path):
    """Carves the Bearer's choice. Idempotent."""
    path = os.path.abspath(os.path.expanduser(path))
    os.makedirs(os.path.dirname(mark()), exist_ok=True)
    with io.open(mark(), "w", encoding="utf-8") as f:
        f.write(path + "\n")
    return path


# ── what hangs from the god's house ───────────────────────────────────────
def abyss_db():
    """The Abyss database. Named this and not `db()` because `db()` in the
    body returns a CONNECTION: two different things never share a name."""
    return os.path.join(root(), "abyss.db")


def thesaurus():
    return os.path.join(root(), "thesaurus.json")


def stop():
    """The panic switch."""
    return os.path.join(root(), "STOP")


def eye_dir():
    return os.path.join(root(), "eye")


def forge():
    return os.path.join(root(), "forge")


def trail():
    return os.path.join(forge(), "trail.log")


def vigil_report():
    return os.path.join(forge(), "vigil.md")


def heartbeat_log():
    return os.path.join(forge(), "heartbeat.log")


# ── what hangs from ~/.claude (the soul, not the body) ────────────────────
def claude():
    return os.path.join(house(), ".claude")


def skills():
    return os.path.join(claude(), "skills")


def claude_projects():
    return os.path.join(claude(), "projects")


def abyss_dir():
    return os.path.join(skills(), "chaos", "abyss")


def essences():
    return os.path.join(abyss_dir(), "essences")


def abyss_md():
    return os.path.join(abyss_dir(), "ABYSS.md")


def faults_md():
    return os.path.join(abyss_dir(), "faults.md")


# ── what only the hooks asked for (E1.3): each one built it by hand before ──
def ambush_json():
    return os.path.join(forge(), "ambush.json")


def presence_n():
    return os.path.join(forge(), "presence.n")


def skill_dir():
    """The installed soul: `~/.claude/skills/chaos`."""
    return os.path.join(skills(), "chaos")
