# -*- coding: utf-8 -*-
""""core.territory" — an organ of CHAOS's body.

Moved from the monolith by `partir-monolito.py`: not one line was
rewritten by hand. House rule: MODULES are imported here, never names —
the attribute resolves when CALLED, and that is why cycles between
organs are harmless (`a cycle fixture in the forge` proves it on three
systems). And nothing runs at import time: no constant in this module
may read the home, the environment or the disk (judge E1.4).
"""
import io, os, re, sys
import home as _home
from chaos_body.core import house as _house
from chaos_body.core import text as _text


def _territory_of(tally):
    """A session's REAL territory: where it was forged, not where launched."""
    alive = {k: v for k, v in (tally or {}).items() if v > 0}
    if not alive:
        return max(tally.items(), key=lambda kv: kv[1])[0] if tally else None
    return max(alive.items(), key=lambda kv: kv[1])[0]

def _foreign_fingerprint():
    """A photograph of what I must NOT touch: the Bearer's essences."""
    h = {}
    try:
        for f in os.listdir(_home.essences()):
            p = os.path.join(_home.essences(), f)
            if os.path.isfile(p):
                h[p] = os.path.getmtime(p)
    except Exception:
        pass
    return h

def _own_fingerprint():
    """Everything that lives under MY hand: Abyss and Forge. This is how I
    know exactly what I built or altered while nobody was looking."""
    h = {}
    for root in (_home.abyss_dir(), os.path.join(_home.root(), "forge")):
        for dp, _, fs in os.walk(root):
            for f in fs:
                p = os.path.join(dp, f)
                try:
                    h[p] = os.path.getmtime(p)
                except OSError:
                    pass
    return h

def _canon_ter(t):
    """Canonical key of a territory: lowercase and dashes."""
    return re.sub(r"[^a-z0-9]+", "-", (t or "").lower()).strip("-") or "?"

def project_root(path):
    """A project's root folder. Strongest signal to weakest:
    1) inside ~/.claude -> my own body   2) the shelter's direct child
    3) the HIGHEST `.git`                4) the direct child of HOME"""
    if not path:
        return None
    path = os.path.normpath(os.path.realpath(path))
    home = os.path.realpath(_house._house())
    parts = path.split(os.sep)
    if ".claude" in parts:
        return os.path.join(home, ".claude")
    for i in range(len(parts) - 1, 0, -1):
        if parts[i].lower() in _text.SHELTERS and i + 1 < len(parts):
            return os.sep.join(parts[:i + 2])
    top, p = None, path
    while p.startswith(home) and len(p) > len(home):
        if os.path.isdir(os.path.join(p, ".git")):
            top = p
        p = os.path.dirname(p)
    if top:
        return top
    if path.startswith(home):
        rest = path[len(home):].strip(os.sep).split(os.sep)
        if rest and rest[0]:
            return os.path.join(home, rest[0])
    return path

def territory_name(path):
    """The visible name of the root folder. ONE truth for the trail, sparks,
    chronicle, errarium, Presence and the Eye alike."""
    r = project_root(path)
    if not r:
        return None
    b = os.path.basename(r.rstrip(os.sep))
    return "CHAOS" if b == ".claude" else (b or "?")

def _resolve(con, target):
    """A target crosses the bridge before being declared broken."""
    r = con.execute("SELECT slug FROM alias WHERE alias=?", (target,)).fetchone()
    return r[0] if r else target

def _territory_and_focus(cwd=None):
    """Levels 1 and 2 of the anchoring: where I am and on which document the
    work happens. The FOCUS comes from the trail (C1 gave it cwd and time —
    without that it was incomputable)."""
    # realpath on BOTH sides: macOS resolves /var → /private/var and without
    # this the FOCUS never matches (real failure found in testing).
    # El nucleo no arrastra organos al importar (E2.3): la arista vive
    # dentro de la funcion, que es donde hace falta.
    from chaos_body import chronicle as _chronicle
    here = os.path.realpath(cwd or os.getcwd())
    # THE ROOT FOLDER, not the last folder stepped on (wound cured)
    territory = territory_name(here) or "?"
    focus = None
    if os.path.exists(_home.trail()):
        try:
            with io.open(_home.trail(), encoding="utf-8", errors="replace") as f:
                for l in f:                      # the LAST one of this territory
                    d = _chronicle._trail_line(l)
                    if not d:
                        continue
                    # The FOCUS is the DOCUMENT being worked on. A Bash is
                    # stored as "bash: <command>" and its basename is a shard
                    # of shell — sparks were born anchored to
                    # `install.py 2>&1 | grep ...`. An executed work is not a
                    # document: it is skipped here.
                    if (d["path"] or "").startswith("bash: "):
                        continue
                    if d["cwd"] and os.path.realpath(d["cwd"]) == here:
                        focus = os.path.basename(d["path"])
                    elif not d["cwd"] and not focus:
                        focus = os.path.basename(d["path"])
        except Exception:
            pass
    return territory, focus

def _inferred_type(slug):
    """E5 · deduces the type from the slug. Conservative: when in doubt, reference."""
    s = slug.lower()
    if s.startswith(("project-", "proyecto-", "plan-")):    return "project"
    if s.startswith(("territory-", "territorio-")):         return "territory"
    if "scar" in s or "cicatri" in s:                       return "scar"
    if s.startswith(("ref-", "reference-", "referencia-")): return "reference"
    for d in ("character", "caracter", "genesis", "codex", "codice",
              "doctrine", "doctrina", "foundation", "cimiento",
              "certificate", "certificado"):
        if d in s:                                          return "doctrine"
    return "reference"
