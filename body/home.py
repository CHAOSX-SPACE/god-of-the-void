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


# ── la llave de la guadaña ────────────────────────────────────────────────
def sweep_key():
    """The mark that authorises the automatic sweep: `<home>/vivos.barrer`."""
    return os.path.join(root(), "vivos.barrer")


def sweep_authorized():
    """Whether THE LIVING may reap by itself when a session closes.

    The law used to live inline inside the closing hook, where nobody could
    judge it: that hook calls `main()` at module level and `main()` reads stdin,
    so importing it hangs. A law that cannot be measured is a promise.

    It stays HERE, in the leaf, so the hook keeps its cheap import: dragging the
    monolith in just to answer a yes/no would break organ 18's own law.
    """
    return os.path.isfile(sweep_key())


# ── el aniquilador ────────────────────────────────────────────────────────
def annihilate(path):
    """Erase a tree WHOLE, read-only files included.

    `shutil.rmtree` cannot delete a read-only file on Windows, and with
    `ignore_errors=True` it fails IN SILENCE, leaving debris while the caller
    reports a clean sweep. Git marks its `pack/*` read-only and pip leaves such
    files inside a venv, so this is not an edge case: it is every uninstall.
    Measured on Windows 11 — a pack survived a wipe that declared itself total.

    Returns True if nothing is left. It does not raise: a god that cannot
    delete says so, it does not break the Bearer's session over it.
    """
    import shutil as _sh
    import stat as _st

    def _insistir(func, ruta, _exc):
        try:
            os.chmod(ruta, _st.S_IWRITE | _st.S_IREAD)
            func(ruta)
        except Exception:
            pass

    if not os.path.exists(path):
        return True
    try:
        try:                                  # 3.12+ renamed the parameter
            _sh.rmtree(path, onexc=lambda f, p, e: _insistir(f, p, e))
        except TypeError:
            _sh.rmtree(path, onerror=_insistir)
    except Exception:
        pass
    if not os.path.exists(path):
        return True
    # It survived. On Windows that is almost never a permission: it is a file
    # LOADED by a living process, and no chmod on earth moves it. Measured: the
    # MCP's venv resisted because the host had the server open. Saying "it could
    # not be deleted" leaves the mortal with nowhere to go — so whoever is
    # holding it gets NAMED.
    for quien in holders(path):
        print("      held by pid %s (%s)" % (quien[0], quien[1]))
    return False


def holders(path):
    """Who is holding a tree open. Empty when it cannot be known — never a guess.

    Only the honest, cheap question: which live processes RUN from inside that
    tree. It does not enumerate open handles (that needs tools not everyone
    has), so it is a floor, not a ceiling — and it is declared as such."""
    import subprocess as _sp
    fuera = []
    raiz = os.path.abspath(path).rstrip("\\/").lower()
    try:
        if os.name == "nt":
            ps = ("Get-CimInstance Win32_Process | ForEach-Object { "
                  "'{0}|{1}|{2}' -f $_.ProcessId, $_.Name, $_.ExecutablePath }")
            r = _sp.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", ps],
                        capture_output=True, timeout=40)
            for l in (r.stdout or b"").decode("utf-8", "replace").splitlines():
                p = l.split("|", 2)
                if len(p) == 3 and p[2] and p[2].replace("/", "\\").lower().startswith(raiz):
                    fuera.append((p[0], p[1]))
        else:
            r = _sp.run(["ps", "-eo", "pid=,comm="], capture_output=True,
                        text=True, timeout=20)
            for l in (r.stdout or "").splitlines():
                p = l.split(None, 1)
                if len(p) == 2 and p[1].lower().startswith(raiz):
                    fuera.append((p[0], os.path.basename(p[1])))
    except Exception:
        pass
    return fuera
