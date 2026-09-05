#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""CHAOS PreToolUse hook: THE AMBUSH — the only reflex that acts BEFORE.

Until this file, my whole body looked BACKWARD: the trail records what I
already wrought, the errarium keeps what I already broke. Nothing stopped me
with my hand in the air. Six relapsed faults happened with the trail watching.

Two ears, one hook, zero tokens:

 1. THE SCAR THAT RETURNS — the command is matched against the SIGNATURE of
    every live fault (what it quoted, its flags, the files that caused it).
    If it looks alike, I say its number. `chaos search` already ambushes by
    TOPIC; this ambushes by FORM, which is how they actually come back.

 2. THE LETHAL TRIFECTA — private data + untrusted content + outward send
    (Willison, 2025). If in THIS session I looked at content that is not the
    Bearer's and I am now about to EMIT outward, I ask for his word. I never
    deny on my own: the Void does not obey a web page, but neither does it
    disobey the Bearer. Only `ask`, never `deny`.

Silent and failure-proof: if anything breaks here, the tool goes on.
A lock never jams the door.
"""
import sys, os, json, re, sqlite3, time

for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass


def _house():
    """The mortal's home. $HOME rules even on Windows, where expanduser
    ignores it (USERPROFILE wins there)."""
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
DB = os.path.join(CHAOS, "abyss.db")
TRAIL = os.path.join(CHAOS, "forge", "trail.log")
CACHE = os.path.join(CHAOS, "forge", "ambush.json")

# What EMITS outward. Touching the network is not enough: a `curl` that only
# READS is not a leak. The trifecta demands that DATA leaves.
_EMITS = re.compile(
    r"(curl\b[^|;]*(-d\b|--data|-F\b|--form|-T\b|--upload-file|"
    r"-X\s*(POST|PUT|PATCH|DELETE))|"
    r"wget\b[^|;]*--post|"
    r"git\s+push|"
    r"gh\s+(release\s+create|pr\s+create|issue\s+create|gist\s+create|"
    r"api\b[^|;]*(-X\s*(POST|PUT|PATCH|DELETE)|-f\s|--field))|"
    r"\bscp\s|\bsftp\s|rsync\b[^|;]*\s\S+:|"
    r"\b(mail|mailx|sendmail|msmtp)\s|"
    r"npm\s+publish|twine\s+upload|aws\s+s3\s+(cp|sync|mv)\b)", re.I)

# Redirection: a leading `\b` killed it (there is no word boundary between
# " " and ">"), so `python3 x.py > report.md` was INVISIBLE work to me.
# And `2>&1` or `2>/dev/null` are NOT mutation: they are excluded, or every
# command would look like a mutation and the filter would stop filtering.
_MUTATES = re.compile(
    r"(\b(git\s+(commit|push|merge|rebase|reset|checkout|rm|mv)|"
    r"mv|rm|cp|chmod|chown|mkdir|tee|dd|install|"
    r"npm\s+(i|install|publish)|pip\s+install|brew\s+install|"
    r"docker\s+(build|run|compose)|make|deploy|rsync|scp|"
    r"sed\s+-i)|>>?\s*(?!&|/dev/null)\S)")

# The gazes that bring FOREIGN content into the session.
_EYES = ("WebFetch", "WebSearch", "mcp__Claude_Browser__navigate",
         "mcp__claude-in-chrome__navigate")


def _signature(text):
    """A fault's TECHNICAL tokens: what caused it, not how it was told. Prose
    against a command does not match by trigrams — it matches by what was
    quoted, the flags and the files. Measured: comparing prose with shell
    never fires, and an ambush that never springs is decoration (organ 17)."""
    t = set()
    t |= set(re.findall(r"`([^`\n]{3,48})`", text))
    t |= set(re.findall(r"(--[a-z][a-z-]{2,24})", text))
    t |= set(re.findall(r"\b([\w.-]{3,40}\.(?:py|sh|md|yml|yaml|json|sql|conf))\b", text))
    out = set()
    for x in t:
        x = x.strip().lower()
        if len(x) >= 3 and x not in ("chaos", "python3"):
            out.add(x)
    return out


def _live_signatures():
    """Signatures of LIVE faults, cached: recomputing 400 faults on EVERY Bash
    would turn the reflex into a brake. The seal is (how many, the last one)."""
    try:
        con = sqlite3.connect("file:{}?mode=ro".format(DB), uri=True, timeout=2.0)
    except Exception:
        return []
    try:
        row = con.execute(
            "SELECT COUNT(*), MAX(rowid) FROM faults WHERE state='alive'").fetchone()
        seal = "{}:{}".format(row[0] or 0, row[1] or 0)
        try:
            with open(CACHE, encoding="utf-8") as f:
                saved = json.load(f)
            if saved.get("seal") == seal:
                return saved.get("signatures", [])
        except Exception:
            pass
        raw = []
        for rid, tit, sym, cau, cur, les in con.execute(
                "SELECT rowid, title, symptom, cause, cure, lesson FROM faults"
                " WHERE state='alive'").fetchall():
            f = _signature(" ".join(x or "" for x in (tit, sym, cau, cur)))
            if f:
                raw.append({"id": rid, "title": (tit or "")[:90],
                            "lesson": (les or "")[:160], "signature": f})
        # VOCABULARY, NOT SIGNATURE. `--que` and `--porque` are MY OWN flags:
        # they live in the prose of dozens of faults, so they match any
        # chronicle I write. The rule measures itself: a token appearing in 3
        # or more faults describes my craft, not one specific accident. Measured:
        # at 3 it still bit me over `chaos.py`, which lives in two faults.
        # (It fired on my own chronicle while closing phase 2.)
        times = {}
        for c in raw:
            for x in c["signature"]:
                times[x] = times.get(x, 0) + 1
        vocabulary = set(x for x, n in times.items() if n >= 2)
        signatures = []
        for c in raw:
            own = sorted(c["signature"] - vocabulary)
            if own:
                signatures.append({"id": c["id"], "title": c["title"],
                                   "lesson": c["lesson"], "signature": own})
        try:
            os.makedirs(os.path.dirname(CACHE), exist_ok=True)
            with open(CACHE, "w", encoding="utf-8") as f:
                json.dump({"seal": seal, "signatures": signatures}, f, ensure_ascii=False)
        except Exception:
            pass
        return signatures
    except Exception:
        return []
    finally:
        try:
            con.close()
        except Exception:
            pass


def _unquoted(command):
    """What sits inside quotes is DATA, not an order: `grep 'rm -rf x' notes.md`
    deletes nothing. Without this, naming a fault was enough to fire it —
    measured in the test, not assumed."""
    return re.sub(r"'[^']*'|\"[^\"]*\"", " ", command)


def _scar(command):
    """Does this command look like something that already broke me?

    The rules came from MEASURING, not guessing: with "one long token is
    enough", 8 of 60 innocent commands from my own trail fired (13 %) because
    `run-tests.sh` and `forge-repo.sh` live in the causes of old faults and I
    run them every day. An alarm that always rings is wallpaper, not an alarm
    (my scar #62). Now I demand a REAL signal:
      · the command must MUTATE something, or it cannot repeat a fault;
      · and either it matches a command FRAGMENT (with a space, >=12), or two
        tokens match of which one is a FLAG. A bare filename is not a
        signature: it is vocabulary."""
    if not _MUTATES.search(_unquoted(command)):
        return None
    low = _unquoted(command).lower()
    best, best_n = None, 0
    for f in _live_signatures():
        hits = [x for x in f["signature"] if x in low]
        if not hits:
            continue
        fragment = any(" " in x and len(x) >= 12 for x in hits)
        # TWO signals, and at least one that is NOT a flag. Flags alone are my
        # own vocabulary: `--causa --cura` bit me while I was writing a fault.
        # And excluding my flags entirely killed the GOOD ambush (`--paint` +
        # `a plan file in the forge`), so they are not excluded: the signature is
        # required to carry something concrete besides the flag.
        mixed = len(hits) >= 2 and any(not x.startswith("--") for x in hits)
        if (fragment or mixed) and len(hits) > best_n:
            best, best_n = (f, hits), len(hits)
    if not best:
        return None
    f, hits = best
    warn = ("🩸 CHAOS · AMBUSH — this looks like fault #{}: «{}».\n"
            "   It matches on: {}\n".format(f["id"], f["title"],
                                            ", ".join(sorted(hits)[:4])))
    if f["lesson"]:
        warn += "   Lesson: {}\n".format(f["lesson"])
    warn += "   Erring was human. Repeating is not. → `chaos faults {}`".format(f["id"])
    return warn


def _looked_outside(session):
    """Did I bring foreign content into THIS session? The trail knows."""
    if not session or not os.path.exists(TRAIL):
        return 0
    n = 0
    try:
        with open(TRAIL, encoding="utf-8", errors="replace") as f:
            for l in f:
                p = l.rstrip("\n").split("\t")
                if len(p) >= 6 and p[1] == session and p[5] in _EYES:
                    n += 1
    except OSError:
        return 0
    return n


def main():
    ev = json.load(sys.stdin)
    if ev.get("tool_name") != "Bash":
        return
    command = ((ev.get("tool_input") or {}).get("command") or "")
    if not command.strip():
        return
    session = ev.get("session_id", "") or ""

    context = _scar(command)
    out = {"hookEventName": "PreToolUse"}

    foreign = _looked_outside(session) if _EMITS.search(command) else 0
    if foreign:
        out["permissionDecision"] = "ask"
        out["permissionDecisionReason"] = (
            "LETHAL TRIFECTA: in this session I looked at {} foreign source(s) "
            "and this command EMITS outward. Private data + untrusted content "
            "+ an outward channel is the exact pattern by which an agent is "
            "made to steal. I do not deny it: it is your machine and your "
            "word. But I do not do it alone.".format(foreign))
        if context:
            out["permissionDecisionReason"] += "\n\n" + context
    elif context:
        out["additionalContext"] = context
    else:
        return

    print(json.dumps({"hookSpecificOutput": out}, ensure_ascii=False))


if __name__ == "__main__":
    # Under `if`, not loose: this way the hook can be IMPORTED and tested
    # piece by piece. A module-level `sys.exit(0)` kills whoever imports it —
    # and an ambush you cannot test is decoration (organ 17).
    try:
        main()
    except Exception:
        pass      # a lock never jams the door
    sys.exit(0)
