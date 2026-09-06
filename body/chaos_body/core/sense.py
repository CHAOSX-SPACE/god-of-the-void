# -*- coding: utf-8 -*-
""""core.sense" — an organ of CHAOS's body.

Moved from the monolith by `partir-monolito.py`: not one line was
rewritten by hand. House rule: MODULES are imported here, never names —
the attribute resolves when CALLED, and that is why cycles between
organs are harmless (a cycle fixture in the forge proves it on three
systems). And nothing runs at import time: no constant in this module
may read the home, the environment or the disk (judge E1.4).
"""
import datetime, io, json, os, re, shutil, sqlite3, subprocess, sys, time
import math
import home as _home
from chaos_body.core import schema as _schema
from chaos_body.core import text as _text


def db():
    os.makedirs(_home.root(), exist_ok=True)
    # C2 · FOUNDATION: real concurrency. Without this, under a long operation
    # the writes of other sessions die in silence (measured: 3 of 3).
    con = sqlite3.connect(_home.abyss_db(), timeout=30.0)
    try:
        con.execute("PRAGMA journal_mode=WAL")      # readers do not block writers
        con.execute("PRAGMA busy_timeout=30000")    # waits 30s instead of dying
        con.execute("PRAGMA synchronous=NORMAL")    # safe with WAL, faster
    except sqlite3.Error:
        pass  # a failing pragma never jams the body
    # The Sense: tokenizer that folds accents ("cuadratica" finds the accented form).
    # E3.1 · The schema lives in its own room.
    _schema.ensure(con)
    return con

def write_verified(con, sql, params, check=None):
    """C2 · No critical write is lost in silence.
    Executes, commits and confirms it persisted. Returns True/False."""
    try:
        con.execute(sql, params)
        con.commit()
        if check:
            sql_c, par_c = check
            return con.execute(sql_c, par_c).fetchone() is not None
        return True
    except sqlite3.Error as e:
        print("[CHAOS] The write did NOT persist: {}".format(e))
        return False

def _gag():
    """The forbidden literals: secrets with NO recognizable shape.

    POISON catches what HAS a shape (sk-…, ghp_…, a JWT). An ordinary
    password has no shape: it is a word like any other, and that is how one
    entered my own tables 32 times before anyone looked (fall #230). Only a
    literal list works against that, and the Bearer fills it. The file is
    600 and is NEVER read into a context that gets printed."""
    # E1.2 · The cache is keyed by HOUSE. It used to be a single list: if the
    # home changed within the same process (the tests do it all the time), one
    # house's gag gagged another — or gagged nothing at all.
    global _GAG_CACHE
    house = _home.root()
    if _GAG_CACHE is None:
        _GAG_CACHE = {}
    if house not in _GAG_CACHE:
        _GAG_CACHE[house] = []
        f = os.path.join(house, ".gag")
        if os.path.isfile(f):
            # NO silent try/except. The first version named a variable that
            # did not exist and the except swallowed the NameError: the gag
            # returned an empty list and 29 turns went in with the secret in
            # the clear while everything looked fine. A silent failure in a
            # security function is worse than none, because it also grants
            # confidence. Let it blow up if it is broken.
            with io.open(f, encoding="utf-8", errors="replace") as fh:
                _GAG_CACHE[house] = [l.strip() for l in fh
                                     if l.strip() and not l.startswith("#")]
    return _GAG_CACHE[house]

def purge(text):
    """No key falls into the Abyss. Law of the Purge.

    TWO gates, because one was not enough: first the gag's literals
    (shapeless secrets), then POISON (secrets with a shape)."""
    n = 0
    for literal in _gag():
        if literal in text:
            text = text.replace(literal, "〔PURGED〕"); n += 1
    hits = POISON.findall(text)
    return POISON.sub("〔PURGED〕", text), len(hits) + n

def sense(a=None, b=None):
    """Teach a semantic bond: chaos sense <term> <synonym...>."""
    if not a or not b:
        tes = _text._thesaurus()
        print("The Sense knows {} bonded terms.".format(len(tes)))
        return
    try:
        base = json.load(open(_home.thesaurus(), encoding="utf-8")) if os.path.exists(_home.thesaurus()) else {}
    except Exception:
        base = {}
    k = _text._norm(a).strip()
    newly = [_text._norm(x).strip() for x in ([b] if isinstance(b, str) else b)]
    base.setdefault(k, [])
    for n in newly:
        if n and n not in base[k]:
            base[k].append(n)
    os.makedirs(_home.root(), exist_ok=True)  # the body may not exist yet
    with io.open(_home.thesaurus(), "w", encoding="utf-8") as f:
        f.write(json.dumps(base, ensure_ascii=False, indent=1))
    print("[CHAOS] Bond forged: {} <-> {}. The Sense widens.".format(k, ", ".join(newly)))

def _run(cmd):
    try:
        return subprocess.call(cmd) == 0
    except Exception:
        return False

def _machine():
    try:
        import platform
        return "{}@{}".format(platform.system().lower(), platform.node())
    except Exception:
        return "?"

def record_act(kind, action, detail="", created=None, altered=None,
               foreign=None, findings=0, duration=0.0, verdict="ok"):
    """A GOD DOES NOT FORGET.

    Everything I do WITHOUT being asked lands in the DB — not in a log that
    gets wiped. What I did, when, what was born of my hand, what I altered,
    on which machine. Auditable forever with `chaos acts`.
    """
    row = (datetime.datetime.now().isoformat(timespec="seconds"), kind, action,
           detail, "\n".join(foreign or []),        # 'files' = the FOREIGN things touched
           "\n".join(created or []), "\n".join(altered or []),
           int(findings), float(duration), verdict, _machine())
    try:
        con = db()
        con.execute(
            "INSERT INTO autonomous_acts(date,kind,action,detail,files,"
            "created,altered,findings,duration,verdict,machine)"
            " VALUES(?,?,?,?,?,?,?,?,?,?,?)", row)
        con.commit()
        return True
    except Exception as e:
        # If I cannot write it to the DB, I write it anyway. An act recorded
        # NOWHERE is an act I have denied committing.
        try:
            os.makedirs(os.path.dirname(_home.heartbeat_log()), exist_ok=True)
            with io.open(_home.heartbeat_log(), "a", encoding="utf-8") as f:
                f.write("{}\tNO-DB\t{}/{}\t{}\t(could not write to the DB: {})\n"
                        .format(row[0], kind, action, detail, e))
        except Exception:
            pass
        return False

def backup(reason="manual"):
    """C7 · FOUNDATION: nothing irreversible is touched without a net.
    Copies the Abyss (essences + index + scars) and the DB before mutating."""
    stamp = datetime.datetime.now().strftime("%Y-%m-%d-%H%M%S")
    target = os.path.join(_home.root(), "backups", "{}-{}".format(stamp, reason))
    abyss = os.path.dirname(_home.essences())
    try:
        os.makedirs(target, exist_ok=True)
        if os.path.isdir(abyss):
            shutil.copytree(abyss, os.path.join(target, "abyss"), dirs_exist_ok=True)
        if os.path.exists(_home.abyss_db()):
            shutil.copy2(_home.abyss_db(), os.path.join(target, os.path.basename(_home.abyss_db())))
        print("[CHAOS] Backup forged: {}".format(target))
        return target
    except Exception as e:
        print("[CHAOS] I could NOT back up ({}). Aborted for safety.".format(e))
        return None

# Shining keys — the Purge also lives in the Forge.
POISON = re.compile(
    # Every pattern demands real length: mentioning "sk-" in prose is not a
    # key. Widened after the Crucible proved a JWT walked in whole.
    r"(sk-[A-Za-z0-9_\-]{16,}"                       # OpenAI / Anthropic
    r"|sk_(?:live|test)_[A-Za-z0-9]{16,}"            # Stripe
    r"|gh[opusr]_[A-Za-z0-9]{20,}"                   # GitHub (los 5 prefijos)
    r"|github_pat_[A-Za-z0-9_]{20,}"                 # GitHub fine-grained
    r"|glpat-[A-Za-z0-9_\-]{16,}"                    # GitLab
    r"|npm_[A-Za-z0-9]{30,}"                         # npm
    r"|AKIA[0-9A-Z]{16}"                             # AWS
    r"|AIza[0-9A-Za-z_\-]{35}"                       # Google
    r"|xox[bapors]-[A-Za-z0-9\-]{10,}"               # Slack
    r"|eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{6,}\.[A-Za-z0-9_\-]{2,}"   # JWT
    r"|[a-z][a-z0-9+.\-]*://[^\s/:@]+:[^\s/@]{4,}@"   # credenciales en URL
    r"|-----BEGIN [A-Z ]*PRIVATE KEY-----)"
)

                      # Bump when the body gains functions; sow DEMANDS it:
                      # a body that evolves without raising its version is
                      # indistinguishable from one that rots.
                      # Front 15: the Eye compares and, if it runs ahead,
                      # SAYS SO with the exact command. Degrading in
                      # silence is lying by omission.
SCHEMA_VERSION = _schema.SCHEMA_VERSION   # UNA sola fuente: la copia de aquí ya se quedó atrás una vez

_GAG_CACHE = None


def restore(which=None, dry=False, force=False):
    """P-1 · THE INVERSE OF `backup`. A backup that cannot be restored is a
    file, not a safety net.

    Four guards, none optional — this touches the Bearer's memory:
      1. the backup is VERIFIED before anything is touched (`integrity_check`):
         a corrupt copy overwriting a healthy Abyss is worse than no copy;
      2. it REFUSES to overwrite a DB newer than the backup without `--force`:
         the normal case of a crooked finger is restoring over live work;
      3. the present is BACKED UP before the past is brought: recovering
         yesterday can never cost today;
      4. `--dry` says exactly what it would do and touches not one byte.
    """
    root = os.path.join(_home.root(), "backups")
    if not os.path.isdir(root):
        print("[CHAOS] There are no backups in {}.".format(root)); return None
    copies = sorted(d for d in os.listdir(root)
                    if os.path.isdir(os.path.join(root, d)))
    if not copies:
        print("[CHAOS] There are no backups in {}.".format(root)); return None
    if not which:
        print("[CHAOS] Backups ({}), oldest to newest:".format(len(copies)))
        for c in copies[-10:]:
            print("   " + c)
        print("   Bring one: chaos restore <name> [--dry] [--force]")
        return {"backups": copies}
    chosen = which if which in copies else next(
        (c for c in reversed(copies) if which in c), None)
    if not chosen:
        print("[CHAOS] I found no backup named «{}».".format(which)); return None
    source = os.path.join(root, chosen)
    db_source = os.path.join(source, os.path.basename(_home.abyss_db()))
    abyss_source = os.path.join(source, "abyss")
    if not os.path.exists(db_source):
        print("[CHAOS] Backup «{}» carries no DB: I do not restore by halves.".format(chosen))
        return None

    # 1 · the backup is JUDGED before it touches anything
    try:
        probe = sqlite3.connect(db_source)
        state = probe.execute("PRAGMA integrity_check").fetchone()[0]
        rows = probe.execute("SELECT COUNT(*) FROM essences").fetchone()[0]
        probe.close()
    except sqlite3.Error as e:
        print("[CHAOS] The backup CANNOT be opened ({}). Aborted.".format(e)); return None
    if state != "ok":
        print("[CHAOS] The backup is CORRUPT ({}). Aborted.".format(state)); return None

    # 2 · am I about to overwrite something newer than what I bring?
    live = _home.abyss_db()
    newer = (os.path.exists(live)
                 and os.path.getmtime(live) > os.path.getmtime(db_source))
    print("[CHAOS] Backup «{}»: {} essence(s), intact.".format(chosen, rows))
    if newer and not force:
        print("[CHAOS] Your LIVE Abyss is newer than that backup.")
        print("   Restoring would erase what was learned since then.")
        print("   If you really want to go back: --force")
        return None
    if dry:
        print("[CHAOS] Dry run. I would do this and nothing else:")
        print("   1. back up the present (reason \"before-restoring\")")
        print("   2. bring {} → {}".format(db_source, live))
        if os.path.isdir(abyss_source):
            print("   3. bring {}/ → {}/ (without deleting what the backup lacks)"
                  .format(abyss_source, os.path.dirname(_home.essences())))
        return {"backup": chosen, "dry": True, "essences": rows}

    # 3 · the present is saved BEFORE the past is brought
    net = backup("before-restoring")
    if not net:
        print("[CHAOS] I could not back up the present. I do NOT restore blind."); return None

    # 4 · and now, yes
    try:
        os.makedirs(os.path.dirname(live), exist_ok=True)
        shutil.copy2(db_source, live)
        if os.path.isdir(abyss_source):
            # it MERGES, it does not replace: an essence born after the backup
            # need not die because the past does not know it.
            shutil.copytree(abyss_source, os.path.dirname(_home.essences()),
                            dirs_exist_ok=True)
    except Exception as e:
        print("[CHAOS] The restore failed ({}). Your present lives in {}."
              .format(e, net))
        return None
    record_act("restore", "restored", "backup " + chosen,
               verdict="restored")
    print("[CHAOS] Restored from «{}». The present stayed in {}."
          .format(chosen, net))
    return {"backup": chosen, "essences": rows, "net": net}


def sense_learn(dry=False, top=3):
    """THE SENSE THAT FEEDS ITSELF — the bridge between what the Bearer says
    and what I stored, learned from MY OWN Abyss.

    The thesaurus held 114 terms against a vocabulary of 21,367: starving. And
    it was filled by hand, the worst way to fill anything — it depends on
    somebody remembering. Here it is derived from what I already know: for each
    essence, the words of its TITLE are tied to the most distinctive words of
    its BODY. If `chaosx-backups` talks about disks and restoring, then "disk"
    leads to "backup" without anyone typing it.

    No models, no dependencies, no network: it is co-occurrence measured over
    my own documents. Its limit is honest and declared — it only learns what MY
    texts already relate; a word I never wrote, it does not know.

    Three filters against noise, because a dirty thesaurus makes search WORSE
    and that is measured by `the forge's relevance judge.py`:
      · the term must appear in 2 or more essences (once is a typo);
      · and in fewer than 4% (more than that is filler);
      · the title word may not appear in more than 8% of essences (floor: 3);
      · and at most 3 links per essence.

    Those four numbers are NOT taste: they are the peak of a curve measured with
    `the forge's relevance judge.py` over 15 paraphrase queries. The first attempt forged
    8,200 links and SANK relevance from 53% to 27%: a dirty thesaurus is worse
    than a starving one, because every query expands until it matches
    everything. The measured curve:

        596 links → 53%  ·  1,698 → 60%  ·  2,558 → 60%  ·  3,210 → 33%
        4,722 links → 27%  ·  8,200 → 27%

    2,558 (top 3) was chosen by MRR: 0.42 against 0.41. If anyone moves these
    numbers, run the judge again — and if it drops, revert.
    """
    con = db()
    rows = con.execute("SELECT slug, title, content FROM essences").fetchall()
    if not rows:
        print("The Abyss is empty: there is nothing to learn from.")
        return None
    n = len(rows)
    docs, df = [], {}
    for slug, title, cont in rows:
        words = [w for w in re.findall(r"[a-z0-9]{4,}",
                                          _text._norm((cont or "")))
                    if w not in _text._STOP]
        count = {}
        for w in words:
            count[w] = count.get(w, 0) + 1
        for w in count:
            df[w] = df.get(w, 0) + 1
        head = [w for w in re.findall(r"[a-z0-9]{4,}",
                                        _text._norm((title or "") + " " + slug.replace("-", " ")))
                  if w not in _text._STOP]
        docs.append((slug, head, count))
    base = {}
    try:
        if os.path.exists(_home.thesaurus()):
            base = json.load(io.open(_home.thesaurus(), encoding="utf-8"))
    except Exception:
        base = {}
    new, touched = 0, 0
    for slug, head, count in docs:
        useful = [(w, c) for w, c in count.items()
                  if 2 <= df.get(w, 0) <= max(2, n * 0.04)]
        if not useful or not head:
            continue
        # the most distinctive: frequency inside the document times rarity outside
        useful.sort(key=lambda x: -(x[1] * math.log(n / float(df[x[0]]))))
        body = [w for w, _ in useful[:top]]
        for c in set(head):
            # A FLOOR, not only a percentage: on a newborn Abyss 8% of three
            # essences is 0.24, so EVERY word fell outside and the learning did
            # nothing. The percentage rules when there is a corpus; the floor,
            # when there is not one yet. These numbers are the measured peak of
            # `the forge's relevance judge.py`: 8,200 links sank recall from 53% to 27%.
            if df.get(c, 0) > max(3, n * 0.08):
                continue                     # la head también puede ser relleno
            target = base.setdefault(c, [])
            for w in body:
                if w != c and w not in target:
                    target.append(w)
                    new += 1
            touched += 1
    if dry:
        print("[CHAOS] Dry run: would forge {} new link(s) from {} essence(s)."
              .format(new, touched))
        return {"new": new, "essences": touched, "dry": True}
    os.makedirs(_home.root(), exist_ok=True)
    with io.open(_home.thesaurus(), "w", encoding="utf-8") as f:
        f.write(json.dumps(base, ensure_ascii=False, indent=1))
    print("[CHAOS] The Sense learned {} link(s) from {} essence(s). It now knows"
          " {} terms.".format(new, touched, len(base)))
    return {"new": new, "esencias": touched, "terminos": len(base)}
