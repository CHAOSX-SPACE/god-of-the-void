# -*- coding: utf-8 -*-
""""weave" — an organ of CHAOS's body.

Moved from the monolith by `partir-monolito.py`: not one line was
rewritten by hand. House rule: MODULES are imported here, never names —
the attribute resolves when CALLED, and that is why cycles between
organs are harmless (`a cycle fixture in the forge` proves it on three
systems). And nothing runs at import time: no constant in this module
may read the home, the environment or the disk (judge E1.4).
"""
import datetime, io, os, re, sys, time
import home as _home
from chaos_body.core import text as _text
from chaos_body.core import sense as _sense
from chaos_body.core import territory as _territory
from chaos_body import abyss as _abyss


def _weave_essence(con, slug, content, meta, origin):
    """E1 · indexes metadata, tags and links of ONE essence. Always derived."""
    resident = 1 if (origin or "").startswith(_home.essences()) else 0
    # frontmatter RULES; when silent, the slug family speaks (front 1).
    # It lives here, not in a separate command: `weave` rebuilds this whole
    # table, and typing that does not survive the weave loses itself.
    tipo = meta.get("type") or family_of(slug)
    # By NAME, never by position: when `queries`/`last_query` were added (A-1)
    # this positional INSERT blew up entirely and reindexing died. An INSERT
    # with no named columns is a bomb on a schema timer. And `INSERT OR
    # REPLACE` would erase the accumulated usage: it is preserved.
    con.execute("INSERT INTO essence_meta(slug, type, state, devoured, expires,"
                " coverage, resident, path) VALUES (?,?,?,?,?,?,?,?)"
                " ON CONFLICT(slug) DO UPDATE SET type=excluded.type,"
                " state=excluded.state, devoured=excluded.devoured,"
                " expires=excluded.expires, coverage=excluded.coverage,"
                " resident=excluded.resident, path=excluded.path",
                (slug, tipo, meta.get("state"),
                 meta.get("devoured") or datetime.date.today().isoformat(),
                 meta.get("expires"), meta.get("coverage"), resident, origin))
    con.execute("DELETE FROM tags WHERE slug = ?", (slug,))
    raw = meta.get("tags", "")
    for tg in re.split(r"[,\[\]]+", raw):
        tg = _text._norm(tg).strip()
        if tg:
            con.execute("INSERT OR IGNORE INTO tags(slug, tag) VALUES (?,?)", (slug, tg))
    # links: one per MENTION (they do not collapse), with its line and its block

    con.execute("DELETE FROM links WHERE source = ?", (slug,))
    body, _ = _text._without_frontmatter(content)
    # E2 · blocks: paragraphs ending in ^id → addressable
    con.execute("DELETE FROM blocks WHERE slug = ?", (slug,))
    # -- LAW OF DERIVED INDEXES, applied to myself -------------------------
    # REAL WOUND: `blockify` wrote DB-only blocks for external essences and
    # `weave` - which rebuilds this table from the file's `^id` marks - wiped
    # them. Manual state that a derived index erases is state lost. The cure
    # is not to protect the state: it is to DERIVE the block too. If the text
    # carries no marks and is a sack, it is split by paragraph right here -
    # deterministic, so weaving a thousand times yields the same blocks.
    for paragraph in re.split(r"\n\s*\n", body):
        p = paragraph.strip()
        if not p:
            continue
        m = _BLOCK.search(p.splitlines()[-1])
        if m:
            text = _BLOCK.sub("", p).rstrip()
            con.execute("INSERT INTO blocks(content, slug, block_id) VALUES (?,?,?)",
                        (text, slug, m.group(1)))
    for n, line in enumerate(body.splitlines(), 1):
        for m in _WIKILINK.finditer(line):
            target = _text._norm(m.group(1)).strip().replace(" ", "-")
            block = (m.group(2) or "").lstrip("#")
            con.execute("INSERT INTO links(source,target,block,context,line)"
                        " VALUES (?,?,?,?,?)", (slug, target, block or None,
                                                line.strip()[:200], n))

    # LAW OF DERIVED INDEXES applied to myself: if the text carried no ^id
    # marks and it is a sack, the block is DERIVED - at the END, after every
    # DELETE (my first attempt put it before and it erased itself).
    if len(content) > 4000 and not con.execute(
            "SELECT 1 FROM blocks WHERE slug=? LIMIT 1", (slug,)).fetchone():
        for _bid, _txt in _split(content):
            con.execute("INSERT INTO blocks(content, slug, block_id) VALUES (?,?,?)",
                        (_sense.purge(_txt)[0], slug, _bid))

def weave():
    """Rebuilds the WHOLE graph from the .md. Derived indexes: if the DB dies,
    the text begets it again."""
    con = _sense.db()
    con.execute("DELETE FROM links"); con.execute("DELETE FROM tags")
    con.execute("DELETE FROM essence_meta"); con.commit()
    n = 0
    for slug, content, origin in con.execute(
            "SELECT slug, content, origin FROM essences").fetchall():
        # LAW OF DERIVED INDEXES: the .md are the TRUTH. The file is re-read —
        # the content in the DB comes cleaned of `^id` markers and weaving from
        # there would lose the blocks. To derive is to read the source.
        if origin and os.path.isfile(origin):
            try:
                content = _sense.purge(_text.read_file(origin))[0]
            except Exception:
                pass
        _, meta = _text._without_frontmatter(content)
        _weave_essence(con, slug, content, meta, origin or "")
        n += 1
        if n % 20 == 0:
            con.commit()          # C2: chunk it, never one long transaction
    con.commit()
    e = con.execute("SELECT count(*) FROM links").fetchone()[0]
    # a target crosses the alias bridge before being counted broken
    dangling = con.execute(
        "SELECT count(*) FROM links l WHERE NOT EXISTS"
        " (SELECT 1 FROM essence_meta m WHERE m.slug=l.target)"
        " AND NOT EXISTS (SELECT 1 FROM alias a WHERE a.alias=l.target)"
    ).fetchone()[0]
    print("[CHAOS] Woven: {} essence(s), {} link(s){}.".format(
        n, e, ", {} dangling (nonexistent target)".format(dangling) if dangling else ""))

def index():
    """E3 · The index stops being manual. Regenerates ONLY between the marks;
    what is written outside is SACRED (the Bearer and other sessions write too)."""
    path = os.path.join(os.path.dirname(_home.essences()), "ABYSS.md")
    con = _sense.db()
    rows = con.execute(
        "SELECT e.slug, e.title, m.type, m.state FROM essences e"
        " LEFT JOIN essence_meta m ON m.slug = e.slug"
        " WHERE m.resident = 1 OR m.resident IS NULL ORDER BY m.type, e.slug").fetchall()
    lines = [MARK_START, ""]
    current_type = None
    for slug, title, type_, state in rows:
        if not os.path.isfile(os.path.join(_home.essences(), slug + ".md")):
            continue                       # only real residents
        if type_ != current_type:
            current_type = type_
            lines.append("\n**{}**".format((type_ or "no type").upper()))
        n = con.execute("SELECT count(*) FROM links WHERE target=?", (slug,)).fetchone()[0]
        lines.append("- [{}](essences/{}.md){}{}".format(
            title or slug, slug,
            "  ·  {}".format(state) if state else "",
            "  ·  ←{}".format(n) if n else "  ·  ←0 (orphan)"))
    lines += ["", MARK_END]
    block = "\n".join(lines)

    old = _text.read_file(path) if os.path.exists(path) else "# THE ABYSS — index of what CHAOS knows\n\n"
    _sense.backup("before-index")
    if MARK_START in old and MARK_END in old:
        i = old.index(MARK_START); j = old.index(MARK_END) + len(MARK_END)
        new = old[:i] + block + old[j:]
        where = "regenerated between marks (what is outside, intact)"
    else:
        new = old.rstrip() + "\n\n" + block + "\n"
        where = "marks sown at the end (nothing of yours was touched)"
    with io.open(path, "w", encoding="utf-8") as f:
        f.write(new)
    print("[CHAOS] Index {}: {} essence(s).".format(where, len(rows)))

def suggest(kill=None):
    """E4 · UNLINKED mentions: I find where an essence is named without a link
    and propose the bond. Passive discovery — what nobody wrote."""
    con = _sense.db()
    if kill:
        o, _, d = kill.partition("->")
        con.execute("INSERT OR IGNORE INTO dead_suggestions(source, target) VALUES (?,?)",
                    (o.strip(), d.strip())); con.commit()
        print("Suggestion annihilated. It will not be proposed again."); return
    titles = con.execute("SELECT e.slug, e.title FROM essences e"
                         " JOIN essence_meta m ON m.slug=e.slug"
                         " WHERE m.resident=1").fetchall()
    already = set((o, d) for o, d in con.execute("SELECT source, target FROM links"))
    dead = set((o, d) for o, d in con.execute("SELECT source, target FROM dead_suggestions"))
    props = []
    for slug, content in con.execute("SELECT slug, content FROM essences").fetchall():
        body_n = _text._norm(content)
        for other, title in titles:
            if other == slug or (slug, other) in already or (slug, other) in dead:
                continue
            # the name must appear as a word, not as a fragment
            for needle in filter(None, {_text._norm(other).replace("-", " ").strip(),
                                        _text._norm(title or "").strip()}):
                if len(needle) < 6:
                    continue
                if re.search(r"(?<![a-z0-9])" + re.escape(needle) + r"(?![a-z0-9])", body_n):
                    props.append((slug, other, needle)); break
    if not props:
        print("Nothing to suggest. Every mention that matters is already woven."); return
    print("UNLINKED MENTIONS ({}) — bonds nobody wrote:".format(len(props)))
    for o, d, needle in props[:20]:
        print("  {} → {}   (names «{}» without linking it)".format(o, d, needle[:40]))
    print("\n  Weave: add [[{}]] in the body.  Reject: chaos suggest --kill 'source->target'"
          .format("target"))

def links_of(slug):
    """Backlinks: who names this essence. The inverse index is free."""
    con = _sense.db()
    incoming = con.execute("SELECT source, context, line FROM links WHERE target=?"
                           " ORDER BY source", (slug,)).fetchall()
    outgoing = con.execute("SELECT DISTINCT target FROM links WHERE source=?",
                           (slug,)).fetchall()
    if not incoming and not outgoing:
        print("Nobody names it and it names nobody. Orphan in the graph."); return
    if incoming:
        print("← NAMED BY ({}):".format(len(incoming)))
        for o, ctx, ln in incoming:
            print("  {} :{}  {}".format(o, ln, ctx[:110]))
    if outgoing:
        print("→ IT NAMES: {}".format(", ".join(d[0] for d in outgoing)))

def query(*criteria):
    """Query by attributes: `chaos query type:project state:active tag:radar`."""
    con = _sense.db()
    where, params = [], []
    for c in criteria:
        if ":" not in c:
            continue
        k, _, v = c.partition(":")
        k, v = k.strip().lower(), v.strip()
        if k in ("type", "state", "coverage"):
            where.append("m.{} = ?".format(k)); params.append(v)
        elif k == "tag":
            where.append("EXISTS (SELECT 1 FROM tags t WHERE t.slug=m.slug AND t.tag=?)")
            params.append(_text._norm(v).strip())
        elif k == "resident":
            where.append("m.resident = ?"); params.append(1 if v in ("1","si","yes","true") else 0)
        elif k == "expires_before":
            where.append("m.expires IS NOT NULL AND m.expires < ?"); params.append(v)
    sql = ("SELECT m.slug, m.type, m.state, m.devoured FROM essence_meta m"
           + (" WHERE " + " AND ".join(where) if where else "") + " ORDER BY m.slug")
    rows = con.execute(sql, params).fetchall()
    if not rows:
        print("Nothing in the Abyss meets that."); return
    for slug, type_, state, dev in rows:
        tg = [r[0] for r in con.execute("SELECT tag FROM tags WHERE slug=?", (slug,))]
        print("· {}  [{}/{}]  {}{}".format(slug, type_ or "?", state or "?", dev or "",
                                           "  #" + " #".join(tg) if tg else ""))
    print("({} essence(s))".format(len(rows)))

def orphans():
    """Essences nobody names and that name nobody: outside the graph."""
    con = _sense.db()
    rows = con.execute(
        "SELECT slug FROM essences WHERE slug NOT IN (SELECT target FROM links)"
        " AND slug NOT IN (SELECT source FROM links) ORDER BY slug").fetchall()
    if not rows:
        print("No orphans. The whole Abyss is woven."); return
    print("ORPHANS ({}) — nobody names them and they name nobody:".format(len(rows)))
    for (s,) in rows:
        print("  · " + s)

def reconcile():
    """O2 · Reconciles the PARALLEL memory (Claude's `memory/*.md`).
    They describe the same world as my Abyss and were not speaking to each
    other. They are devoured as externals (resident=0) and the overlaps are
    DECLARED: two truths about the same thing is a wound, not a redundancy."""
    base = _home.claude_projects()
    if not os.path.isdir(base):
        print("There is no parallel memory to mirror."); return
    con = _sense.db()
    n, overlaps = 0, []
    for root, _, files in os.walk(base):
        if os.path.basename(root) != "memory":
            continue
        for a in sorted(files):
            if not a.endswith(".md"):
                continue
            path = os.path.join(root, a)
            slug = _abyss.devour(path, silent=True)
            n += 1
            # Does any essence of MINE speak of THE SAME? Overlap MEASURED by
            # trigrams: the FTS match was noise (my long essences matched
            # everything). Hard threshold; a false positive is worse than
            # silence, because it would make me "reconcile" alien things.
            try:
                raw = _text.read_file(path)
                fingerprint = _text._trigr(_text._title_of(raw, slug) + " " + raw[:800])
                for other, cont in con.execute(
                        "SELECT e.slug, e.content FROM essences e"
                        " JOIN essence_meta m ON m.slug=e.slug"
                        " WHERE m.resident=1").fetchall():
                    if other == slug or not fingerprint:
                        continue
                    sim = len(fingerprint & _text._trigr(cont[:800])) / float(len(fingerprint))
                    if sim >= 0.45:
                        overlaps.append((slug, other, sim))
            except Exception:
                pass
    con.commit()
    print("[CHAOS] Parallel memory mirrored: {} file(s) devoured as externals."
          .format(n))
    if overlaps:
        print("  OVERLAPS (two memories about the same thing — review, do not pick blindly):")
        for ext, mine, sim in sorted(overlaps, key=lambda x: -x[2])[:10]:
            print("    · {}  ⇄  {}   ({:.0%} of overlap)".format(ext, mine, sim))
    else:
        print("  No overlaps with my resident essences.")

def expired():
    """O5 · What expires is re-judged: flagging stale without acting is knowingly lying."""
    con = _sense.db()
    today = datetime.date.today().isoformat()
    rows = con.execute("SELECT slug, expires FROM essence_meta"
                       " WHERE expires IS NOT NULL AND expires <= ? ORDER BY expires",
                       (today,)).fetchall()
    if not rows:
        print("No truth has expired. What I assert still stands."); return
    print("EXPIRED ({}) — do NOT assert without re-Judgment:".format(len(rows)))
    for slug, c in rows:
        print("  · {}  (expired {})".format(slug, c))

def family_of(slug):
    """A slug's family by its prefix. None when it cannot be asserted:
    inventing a type is worse than leaving it empty (Judgment rules)."""
    if not slug:
        return None
    pref = slug.split("-")[0] if "-" in slug else slug
    return FAMILY.get(pref.lower())

def type_externals(dry=False):
    """Fills `type` for essences that do not declare one, IN THE DB ONLY.
    Idempotent. Resident frontmatter always wins over this."""
    con = _sense.db()
    rows = con.execute(
        "SELECT slug, resident FROM essence_meta WHERE type IS NULL OR type=''"
    ).fetchall()
    plan, mute = [], []
    for slug, res in rows:
        f = family_of(slug)
        if f:
            plan.append((slug, f, res))
        else:
            mute.append(slug)
    if not plan and not mute:
        print("[CHAOS] Every essence declares its family. Nothing to type.")
        return 0
    print("TYPING BY FAMILY ({} essence(s)):".format(len(plan)))
    byf = {}
    for _s, f, _r in plan:
        byf[f] = byf.get(f, 0) + 1
    for f, n in sorted(byf.items(), key=lambda x: -x[1]):
        print("  {:<12} {}".format(f, n))
    if mute:
        print("  NO FAMILY (left empty, never invented): {}".format(
            ", ".join(mute[:8]) + (" ..." if len(mute) > 8 else "")))
    if dry:
        print("\n(dry run: nothing touched)")
        return len(plan)
    for slug, f, _r in plan:
        con.execute("UPDATE essence_meta SET type=? WHERE slug=?", (f, slug))
    con.commit()
    print("\n[CHAOS] Typed. The foreign files remain untouched: only the DB spoke.")
    return len(plan)

def alias(a=None, slug=None, remove=False):
    """Declares `a` as another name for `slug`. No arguments: lists them."""
    con = _sense.db()
    if not a:
        rows = con.execute("SELECT alias, slug FROM alias ORDER BY slug").fetchall()
        if not rows:
            print("No alias declared. Misspelled names stay broken.")
            return
        print("ALIASES ({}):".format(len(rows)))
        for al, sl in rows:
            print("  {:<34} -> {}".format(al, sl))
        return
    if remove:
        con.execute("DELETE FROM alias WHERE alias=?", (a,))
        con.commit()
        print("[CHAOS] Alias '{}' annihilated.".format(a))
        return
    if not slug:
        print("Name the essence it points at: chaos alias <name> <slug>"); sys.exit(1)
    if not con.execute("SELECT 1 FROM essence_meta WHERE slug=?", (slug,)).fetchone():
        print("[CHAOS] '{}' does not exist in the Abyss. An alias invents no targets.".format(slug))
        sys.exit(1)
    con.execute("INSERT OR REPLACE INTO alias(alias, slug) VALUES (?,?)", (a, slug))
    con.commit()
    print("[CHAOS] Bridge laid: '{}' -> {}".format(a, slug))

def suggested_aliases(apply_=False):
    """Proposes bridges for broken links using the Sense: a broken target that
    resembles a real slug by >=60% is almost certainly the same name
    misspelled. Without --apply it only proposes: blind bridges break graphs."""
    con = _sense.db()
    broken = [r[0] for r in con.execute(
        "SELECT DISTINCT target FROM links l WHERE NOT EXISTS"
        " (SELECT 1 FROM essence_meta m WHERE m.slug=l.target)"
        " AND NOT EXISTS (SELECT 1 FROM alias a WHERE a.alias=l.target)").fetchall()]
    if not broken:
        print("[CHAOS] No link dangles. The weave is whole.")
        return 0
    slugs = [r[0] for r in con.execute("SELECT slug FROM essence_meta").fetchall()]
    props, orph = [], []
    for d in broken:
        best, sc = None, 0.0
        for s in slugs:
            if d and (d in s or s in d):
                p = len(d) / float(max(len(s), 1)) if d in s else len(s) / float(max(len(d), 1))
                p = min(1.0, p + .25)
            else:
                p = _text._similarity(d, s)
            if p > sc:
                best, sc = s, p
        if best and sc >= .60:
            props.append((d, best, sc))
        else:
            orph.append(d)
    if props:
        print("PROPOSED BRIDGES ({}):".format(len(props)))
        for d, s, p in sorted(props, key=lambda x: -x[2]):
            print("  {:<34} -> {:<34} ~{:.0%}".format(d, s, p))
    if orph:
        print("NO CREDIBLE TARGET ({}) - declared, never invented:".format(len(orph)))
        for d in orph:
            print("  . {}".format(d))
    if not apply_:
        print("\\nApply: chaos suggested-aliases --apply   ·   One by one: chaos alias <broken> <slug>")
        return len(props)
    for d, s, _p in props:
        con.execute("INSERT OR REPLACE INTO alias(alias, slug) VALUES (?,?)", (d, s))
    con.commit()
    print("\\n[CHAOS] {} bridge(s) laid. The Bearer's text is untouched.".format(len(props)))
    return len(props)

def _split(content):
    """Cuts by paragraph and groups up to the ceiling. Returns [(id, text)]."""
    body = _text._without_frontmatter(content)[0]
    parts, cur = [], []
    for par in re.split(r"\n\s*\n", body):
        par = par.strip()
        if not par:
            continue
        size = sum(len(x) for x in cur)
        if cur and size + len(par) > BLOCK_CEILING:
            parts.append("\n\n".join(cur)); cur = []
        cur.append(par)
    if cur:
        parts.append("\n\n".join(cur))
    used = set()
    return [(_text._id_from(p, used), p) for p in parts if len(p) > 60]

def blockify(which=None, dry=False):
    """Gives addressable blocks to the essences that are sacks."""
    con = _sense.db()
    rows = con.execute(
        "SELECT e.slug, e.content, e.origin, m.resident FROM essences e"
        " LEFT JOIN essence_meta m ON m.slug=e.slug"
        " WHERE length(e.content)>4000").fetchall()
    if which and which != "--all":
        rows = [f for f in rows if f[0] == which]
        if not rows:
            print("No large essence named '{}'.".format(which)); sys.exit(1)
    have = set(r[0] for r in con.execute("SELECT DISTINCT slug FROM blocks"))
    pending = [f for f in rows if f[0] not in have]
    if not pending:
        print("[CHAOS] No large essence is a sack. Nothing to split."); return 0

    plan = []
    for slug, content, source, resident in pending:
        chunks = _split(content)
        if chunks:
            plan.append((slug, source, resident, chunks))
    print("SPLITTING SACKS ({} essence(s)):".format(len(plan)))
    for slug, _o, res, chunks in plan[:12]:
        print("  {:<38} {:>3} block(s)  {}".format(
            slug, len(chunks), "RESIDENT (marks the file)" if res else "external (DB only)"))
    if len(plan) > 12:
        print("  ... and {} more".format(len(plan) - 12))
    if dry:
        print("\n(dry run: nothing touched)")
        return len(plan)

    _sense.backup("before-blockify")
    touched = 0
    for slug, source, resident, chunks in plan:
        for bid, text in chunks:
            con.execute("INSERT INTO blocks(content, slug, block_id) VALUES (?,?,?)",
                        (_sense.purge(text)[0], slug, bid))
        if resident and source and os.path.exists(source):
            import hashlib
            before = _text.read_file(source)
            sig = hashlib.sha1(_text._without_frontmatter(before)[0].replace(" ", "")
                               .replace("\n", "").encode("utf-8")).hexdigest()
            new = before
            for bid, text in chunks:
                last = text.rstrip().split("\n")[-1]
                if last in new and "^" + bid not in new:
                    new = new.replace(last, last + " ^" + bid, 1)
            sig2 = hashlib.sha1(re.sub(r"\s*\^[a-z0-9\-]+", "", _text._without_frontmatter(new)[0])
                                .replace(" ", "").replace("\n", "").encode("utf-8")).hexdigest()
            if sig == sig2:
                with io.open(source, "w", encoding="utf-8") as f:
                    f.write(new)
            else:
                print("  ! {}: the body would change - file NOT touched".format(slug))
        touched += 1
    con.commit()
    print("\n[CHAOS] {} sack(s) split into {} block(s). Search returns the paragraph,"
          " not the sack.".format(touched, sum(len(c) for _s, _o, _r, c in plan)))
    return touched

def island(slug=None, remove=False):
    """Front 4. An essence without links is NOT always sick: some knowledge
    genuinely touches nothing else (an index, a one-off project). The Law of
    the Minimum Thread forbids inventing kinship to dress up a number.

    Declaring it an `island` is an act of honesty: I say I LOOKED and found no
    tie. Health stops punishing it; the day a tie appears, it is withdrawn."""
    con = _sense.db()
    if not slug:
        rows = con.execute(
            "SELECT slug FROM essence_meta WHERE state='island' ORDER BY slug").fetchall()
        if not rows:
            print("No island declared.")
            return
        print("DECLARED ISLANDS ({}) - looked at, and no real tie:".format(len(rows)))
        for (s,) in rows:
            print("  . {}".format(s))
        return
    if not con.execute("SELECT 1 FROM essence_meta WHERE slug=?", (slug,)).fetchone():
        print("[CHAOS] '{}' does not exist in the Abyss.".format(slug)); sys.exit(1)
    if remove:
        con.execute("UPDATE essence_meta SET state=NULL WHERE slug=? AND state='island'",
                    (slug,))
        con.commit()
        print("[CHAOS] '{}' is no longer an island.".format(slug))
        return
    v = con.execute("SELECT COUNT(*) FROM links WHERE source=? OR target=?",
                    (slug, slug)).fetchone()[0]
    if v:
        print("[CHAOS] '{}' has {} link(s): it is no island.".format(slug, v))
        sys.exit(1)
    con.execute("UPDATE essence_meta SET state='island' WHERE slug=?", (slug,))
    con.commit()
    print("[CHAOS] '{}' declared an ISLAND. I looked and found no tie - I do not invent one.".format(slug))

def evolve(dry=False):
    """E5 · THE MIGRATION: adds frontmatter to the essences that lack it.
    NEVER touches the body. Idempotent. Backs up first (C7)."""
    if not os.path.isdir(_home.essences()):
        print("There is no Abyss to migrate."); return
    con = _sense.db()
    dates = dict(con.execute("SELECT slug, date FROM essences").fetchall())
    candidates = []
    for f in sorted(os.listdir(_home.essences())):
        if not f.endswith(".md"):
            continue
        path = os.path.join(_home.essences(), f)
        slug = f[:-3]
        raw = _text.read_file(path)
        _, meta = _text._without_frontmatter(raw)
        if meta:
            continue                       # already evolved: idempotent
        candidates.append((path, slug, raw))
    if not candidates:
        print("[CHAOS] Every essence already speaks the grammar. Nothing to migrate.")
        return
    if dry:
        print("[CHAOS] Would migrate {} essence(s):".format(len(candidates)))
        for _, slug, _ in candidates:
            print("  · {} → type: {}".format(slug, _territory._inferred_type(slug)))
        return
    if not _sense.backup("before-evolving"):
        return                             # C7: without a net, we do not jump
    n = 0
    for path, slug, raw in candidates:
        header = ("---\ntype: {}\nstate: active\ndevoured: {}\ncoverage: total\n---\n\n"
                  .format(_territory._inferred_type(slug),
                          dates.get(slug) or datetime.date.today().isoformat()))
        with io.open(path, "w", encoding="utf-8") as fh:
            fh.write(header + raw)         # ONLY prepended: the body intact
        n += 1
    print("[CHAOS] Evolved {} essence(s). Not one word of the body touched.".format(n))
    _abyss.reindex(); weave()

_WIKILINK = re.compile(r"\[\[([^\]|#]+)(#\^[a-z0-9\-]+)?(?:\|[^\]]*)?\]\]", re.I)

_BLOCK = re.compile(r"\^([a-z0-9][a-z0-9\-]*)\s*$", re.I)

MARK_START = "<!-- CHAOS:AUTO start — regenerated by `chaos index`. DO NOT edit inside. -->"

MARK_END = "<!-- CHAOS:AUTO end -->"

# -- FAMILY: the type when the frontmatter is silent -----------------------
# Front 1 of the Plan of Perfection. Most essences declare no `type`, and they
# are all external: mirrors of the Bearer's parallel memory. Their files are
# NOT mine - I never edit them. But their FAMILY is real data, not invention:
# it lives in the slug prefix. The type is inferred IN THE DB.
FAMILY = {"project": "project", "proyecto": "project", "projects": "project",
          "feedback": "feedback", "reference": "reference",
          "referencia": "reference", "territorio": "territory",
          "territory": "territory", "cicatrices": "scar", "scar": "scar",
          "user": "doctrine", "memory": "reference"}

# -- THE BLOCKS: split the sack without touching what is not mine ---------
# Front 2. Essences over 4,000 characters with not a single addressable
# block: one of them is 62,195 characters (~15,500 tokens) for ONE datum.
# My own Rule of waste, violated inside my own memory.
#
# THE PLAN WAS CORRECTED BY THE DATA: it assumed splitting by `##` headings,
# but almost all of them are EXTERNAL and have none (0 headings, 512 bolds).
# We split by PARAGRAPH, grouping up to a ceiling, and the id comes from the
# first bold title - which is how the Bearer actually titles things.
#
# LAW: resident -> mark `^id` in the file (it is mine, with backup and a
# verified SHA1). External -> blocks in the DB ONLY. Foreign files untouched.
BLOCK_CEILING = 1400         # characters per block: ~350 tokens, a useful paragraph
