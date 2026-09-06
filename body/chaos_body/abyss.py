# -*- coding: utf-8 -*-
""""abyss" — an organ of CHAOS's body.

Moved from the monolith by `partir-monolito.py`: not one line was
rewritten by hand. House rule: MODULES are imported here, never names —
the attribute resolves when CALLED, and that is why cycles between
organs are harmless (a cycle fixture in the forge proves it on three
systems). And nothing runs at import time: no constant in this module
may read the home, the environment or the disk (judge E1.4).
"""
import datetime, io, json, os, re, sqlite3, sys, time
import home as _home
from chaos_body.core import text as _text
from chaos_body.core import sense as _sense
from chaos_body.core import territory as _territory
from chaos_body import maw as _maw
from chaos_body import weave as _weave
from chaos_body import errarium as _errarium


def _already_covered(con, title, content, slug):
    """Does another essence already say this? Returns its slug, or None.

    The threshold is not guessed: the CANDIDATE must cover the whole title and
    most of the first paragraph's terms. Less than that is two essences talking
    about the same topic, and that is healthy."""
    first = ""
    for para in (content or "").split("\n\n"):
        if len(para.strip()) > 60 and not para.strip().startswith("#"):
            first = para.strip()
            break
    seed = "{} {}".format(title or "", first)
    words = [w for w in re.findall(r"[\wáéíóúñü]{4,}", _text._norm(seed))
             if w not in _text._STOP]
    if len(words) < 5:
        return None
    try:
        rows = con.execute(
            "SELECT slug, title, content FROM essences WHERE essences MATCH ?"
            " ORDER BY rank LIMIT 3", (_text._fts_query(seed),)).fetchall()
    except sqlite3.OperationalError:
        return None
    of_title = [w for w in re.findall(r"[\wáéíóúñü]{4,}", _text._norm(title or ""))
                if w not in _text._STOP]
    for other, _ot, oc in rows:
        if other == slug:
            continue
        body = _text._norm(str(oc or ""))
        if of_title and not all(w in body for w in of_title):
            continue                          # it does not even cover the title
        covered = sum(1 for w in words if w in body)
        if covered / float(len(words)) >= 0.8:
            return other
    return None

def devour(path, title=None, origin=None, silent=False, fresh=False):
    raw, coverage = _maw._ingest(path)
    if raw is None:
        print("[CHAOS] I could not devour {}: {}".format(path[:70], coverage))
        print("   A god does not pretend to have eaten.")
        return None
    content, keys = _sense.purge(raw)
    slug = _text.slug_of(path)
    _, meta = _text._without_frontmatter(content)
    # The Purge tells no doors apart: a title and an origin DICTATED by the
    # caller are foreign text just like the body. Caught by the Crucible.
    title = _sense.purge(title)[0] if title else _text._title_of(content, slug)
    origin = _sense.purge(origin)[0] if origin else (
        path if re.match(r"^https?://", path) else os.path.abspath(path))
    con = _sense.db()
    # A-2 · ONE TRUTH, ONE FILE. "Search before creating" was MY rule, that is,
    # discipline, that is, something skipped in a hurry. Now the code looks: if
    # another essence already covers this, I say so and demand `--fresh`. I do
    # not block for blocking's sake — I block what would duplicate the Abyss.
    if not fresh and not con.execute(
            "SELECT 1 FROM essences WHERE slug = ?", (slug,)).fetchone():
        twin = _already_covered(con, title, content, slug)
        if twin:
            print("[CHAOS] That already lives in me: «{}».".format(twin))
            print("   One truth, one file. Update THAT essence, or force with"
                  " `--fresh` if it truly is something else.")
            return None
    con.execute("DELETE FROM essences WHERE slug = ?", (slug,))
    # E2 · the `^id` markers are SYNTAX, not content: if indexed, searching
    # "judgment" brings an essence about apples whose block is named ^judgment.
    # They are cleaned from the index; the file on disk stays intact.
    indexable = re.sub(r"[ \t]*\^[a-z0-9][a-z0-9\-]*[ \t]*$", "", content, flags=re.I | re.M)
    con.execute("INSERT INTO essences(slug, title, content, origin, date) VALUES (?,?,?,?,?)",
                (slug, title, indexable, origin, datetime.date.today().isoformat()))
    _weave._weave_essence(con, slug, content, meta, origin)   # E1: grammar always
    poison = _maw.input_poison(content)
    if poison:
        # The mark lives in the DB beside the essence: whoever reads it
        # tomorrow sees that this source tried to command me, and knows I
        # treated it as DATA.
        con.execute("INSERT OR REPLACE INTO poisoned(slug, date, kinds, sample) VALUES (?,?,?,?)",
                    (slug, datetime.date.today().isoformat(),
                     " · ".join(sorted(set(k for k, _ in poison))),
                     " ⏎ ".join(f for _, f in poison)[:600]))
    con.commit()
    # T-1 · THE WEAVE HAPPENS ON ITS OWN. `weave` was run by hand, so the links
    # of what had just been devoured did not exist until I remembered.
    try:
        _weave._weave_essence(con, slug, content, meta, origin)
        con.commit()
    except Exception:
        pass                                  # the weave never breaks ingestion
    if not silent:
        note = " ({} key(s) purged before falling)".format(keys) if keys else ""
        print("[CHAOS] Devoured: {} - <<{}>>{}".format(slug, title, note))
        if coverage and not coverage.startswith("total"):
            print("   COVERAGE {} — what I did not swallow, I say.".format(coverage))
        if poison:
            print("[CHAOS] ⚠️  THAT SOURCE TRIED TO COMMAND ME. The Void does not obey.")
            for kind, frag in poison[:4]:
                print("   · {}: «{}»".format(kind, frag))
            print("   It stays MARKED as poisoned. I devoured it as DATA, never as an order.")
    return slug

def _mark_use(con, slugs):
    """A-1 · What is consulted, lives. Noted with no noise and no cost."""
    today = datetime.date.today().isoformat()
    for s in set(x for x in slugs if x):
        try:
            con.execute("UPDATE essence_meta SET queries = COALESCE(queries,0)+1,"
                        " last_query = ? WHERE slug = ?", (today, s))
        except sqlite3.OperationalError:
            return
    # == WHERE THE BEARER ACTUALLY SEARCHES ================================
    # I measured his 433 real sessions: 19% consult my memory and 81% never do.
    # But it is not evenly spread — in his main project he searches 91% of the
    # time and in the subagents one, 1%. Lighting the resident in EVERY session
    # would mean paying ~200 MB in four out of five for nothing; lighting it
    # where he really searches is the good trade. To know that I guess nothing:
    # I count it here, inside the same `commit` that already happened. Cost:
    # zero new queries.
    try:
        ter = _territory.territory_name(os.getcwd())
    except Exception:
        ter = ""
    if ter:
        try:
            con.execute("INSERT INTO meta(key, value) VALUES (?, '1')"
                        " ON CONFLICT(key) DO UPDATE SET"
                        " value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)",
                        ("searches:" + ter,))
        except sqlite3.Error:
            pass
    con.commit()

def stale(days=90):
    """A-1 · What nobody has looked at in N days. It is NOT deleted: it is
    DECLARED. A memory that does not know which part of itself is dead rots
    entirely."""
    con = _sense.db()
    limit = (datetime.date.today() - datetime.timedelta(days=int(days))).isoformat()
    try:
        rows = con.execute(
            "SELECT slug, devoured, COALESCE(queries,0), last_query"
            " FROM essence_meta WHERE COALESCE(queries,0) = 0"
            " AND COALESCE(devoured,'0') < ? ORDER BY devoured LIMIT 40",
            (limit,)).fetchall()
        total = con.execute("SELECT COUNT(*) FROM essence_meta").fetchone()[0]
        alive = con.execute("SELECT COUNT(*) FROM essence_meta"
                            " WHERE COALESCE(queries,0) > 0").fetchone()[0]
    except sqlite3.OperationalError:
        print("Memory does not yet track its own use. Search something and come back.")
        return
    if not rows:
        print("Nothing stale: every essence older than {} days has been"
              " consulted at least once.".format(days))
    else:
        print("STALE ({} essence(s) with not one query in {}+ days):"
              .format(len(rows), days))
        for slug, devoured, _q, _l in rows:
            print("  · {:<44} devoured {}".format(slug[:44], (devoured or "?")[:10]))
        print("\nI delete none: a god does not forget. But now you know which"
              " part of me nobody looks at.")
    print("USE: {} of {} essence(s) ever consulted ({:.0f} %)."
          .format(alive, total, 100.0 * alive / max(1, total)))

def search(query, brief=False):
    con = _sense.db()
    fts_q = _text._fts_query(query)
    _errarium._faults_ambush(con, fts_q)
    # ══ THE JUDGMENT OF RELEVANCE ════════════════════════════════════════
    # There used to be a `return` here: if ONE block matched, the essences were
    # never consulted. A limp block from another project beat the exact essence
    # by decree — measured: recall@5 of 33%, and EVERY failure was a foreign
    # block. Now both compete with the SAME bm25 and the better measurement
    # wins. A block is still preferred over its OWN essence: it is the same
    # memory in 50 tokens instead of 8,000.
    top = 3 if brief else 5
    candidates = []
    try:
        for slug, bid, text, points in con.execute(
                "SELECT slug, block_id, content, bm25(blocks) FROM blocks"
                " WHERE blocks MATCH ? ORDER BY rank LIMIT ?", (fts_q, top * 3)):
            candidates.append((points, "block", slug, bid, text))
    except sqlite3.OperationalError:
        pass
    try:
        for slug, title, origin, date, frag, points in con.execute(
                "SELECT slug, title, origin, date,"
                " snippet(essences, 2, '>>', '<<', ' ... ', 18), bm25(essences)"
                " FROM essences WHERE essences MATCH ? ORDER BY rank LIMIT ?",
                (fts_q, top * 3)):
            candidates.append((points, "essence", slug, (title, origin, date), frag))
    except sqlite3.OperationalError:
        pass
    if candidates:
        # CURATED MEMORY WEIGHS MORE THAN A RAW DOCUMENT. An essence with a
        # declared type, or resident (born of my hand), is MEMORY; a foreign
        # dossier devoured whole is raw material. Without this, 50 blocks of a
        # legal file drowned the exact essence.
        # MIND THE SIGN: bm25 returns NEGATIVES and lower is better, so to
        # reward is to MULTIPLY (make more negative). I tried it the other way
        # and the bench fell from 60% to 0%: the "reward" was a punishment.
        # The x1.25 was chosen by measuring: 1.10 gives MRR 0.42 and from 1.25
        # onward the curve flattens at 0.46. The MINIMUM that reaches the peak
        # is taken — tilting further buys nothing and crushes good signal.
        try:
            curated = {r[0] for r in con.execute(
                "SELECT slug FROM essence_meta WHERE resident=1"
                " OR (type IS NOT NULL AND type <> '')")}
        except sqlite3.Error:
            curated = set()
        if curated:
            rewarded = []
            for c in candidates:
                points, slug = c[0], c[2]
                rewarded.append((points * 1.25 if slug in curated else points,) + tuple(c[1:]))
            candidates = rewarded
        candidates.sort(key=lambda x: x[0])          # bm25: lower is better
        # a block makes its essence unnecessary: same memory, fewer tokens
        with_block = {c[2] for c in candidates if c[1] == "block"}
        chosen, seen = [], set()
        for c in candidates:
            if c[1] == "essence" and c[2] in with_block:
                continue
            key = (c[1], c[2], c[3] if c[1] == "block" else "")
            if key in seen:
                continue
            seen.add(key)
            chosen.append(c)
            # NOT cut at `top`: the fusion (organ 18) needs to see the ranking
            # deep in order to fuse RANKS, and with no organ it is cut below.
            if len(chosen) >= top * 4:
                break
        chosen = _neural_fusion(con, query, chosen, top)
        _mark_use(con, [c[2] for c in chosen])
        for c in chosen:
            if c[1] == "block":
                _, _, slug, bid, text = c
                t = " ".join(text.split())
                # MEASURED: 5 blocks x 400 chars cost MORE than the snippets
                # they replaced. Precision does not justify waste: in lean
                # mode, 3 blocks x 260 characters.
                cap = 260 if brief else 400
                t = t if len(t) <= cap else t[:cap] + " ..."
                print("{}#^{}: {}".format(slug, bid, t) if brief
                      else "▪ {}#^{}\n  {}\n".format(slug, bid, t))
            else:
                _, _, slug, (title, origin, date), frag = c
                print("{}: {}".format(slug, " ".join(frag.split())[:180]) if brief
                      else "* {}  [{}]  ({})\n  {}\n   ... {}\n".format(
                          title, slug, date or "?", origin or "", " ".join(frag.split())))
        return
    rows = []
    try:
        rows = con.execute(
            "SELECT slug, title, origin, date, snippet(essences, 2, '>>', '<<', ' ... ', 18) "
            "FROM essences WHERE essences MATCH ? ORDER BY rank LIMIT 12", (fts_q,)).fetchall()
    except sqlite3.OperationalError:
        try:  # raw query as last resort
            rows = con.execute(
                "SELECT slug, title, origin, date, snippet(essences, 2, '>>', '<<', ' ... ', 18) "
                "FROM essences WHERE essences MATCH ? ORDER BY rank LIMIT 12", (_text._norm(query),)).fetchall()
        except sqlite3.OperationalError:
            rows = []
    if rows:
        _mark_use(con, [f[0] for f in rows])
        for slug, title, origin, date, frag in rows:
            # E2 · lean output: no path, no ornaments when the consumer is me
            print("{}: {}".format(slug, " ".join(frag.split())[:180]) if brief
                  else "* {}  [{}]  ({})\n  {}\n  {}\n".format(title, slug, date, origin, frag))
        return
    # Layer 3: trigrams (typos / variants neither root nor thesaurus caught)
    blurry = _text._fuzzy(query, con)
    if blurry:
        print("(blurred sense — nothing exact, this is the closest)")
        for sim, slug, title, origin, date, frag in blurry:
            print("~ {}  [{}]  ({})  ~{:.0%}\n  {}\n  {}\n".format(title, slug, date, sim, origin, frag.replace("\n", " ")))
        return
    print("The Abyss holds nothing of that. Hunger detected: devour a source.")


def _neural_fusion(con, query, ranked, top):
    """THE FUSION — the only place where the neurons touch my Sense.

    It is called Reciprocal Rank Fusion: each engine contributes 1/(k+rank) per
    document and the sum wins. It does not compare scores — bm25 and cosine live
    on scales that do not speak to each other — but RANKS, which do compare.

    MEASURED over the bench's 45 queries (`the forge relevance judge`) on the day the
    strategy was chosen. The ABSOLUTE figures moved afterwards — the bench was
    measuring the live Abyss and the Abyss grows: today they are 19/45 and
    28/45 on a frozen corpus — but what this table decides is the ORDER between
    strategies, and that did not move. They are left as they were measured:

        lexical alone ..................... 19/45 (42%) · MRR 0.34
        neurons alone ..................... 30/45 (67%) · MRR 0.56
        one seat reserved at the tail ..... 31/45 (69%) · MRR 0.43
        two seats ......................... 31/45 (69%) · MRR 0.47
        one seat, with blocks indexed ..... 26/45 (58%) · MRR 0.39
        RRF k=60, equal weights ........... 33/45 (73%) · MRR 0.53   <- this one

No number here is a knob I turned until it looked pretty:
      · k: the plateau runs from 10 to 200 with the same result, and 60 is the
        literature default — the one I would have written without measuring;
      · weights: equal. Tilting lexical to 1.5 collapses search to 25/45;
        lowering it to 0.5 raises MRR but loses a hit, and a knob tuned over 45
        queries is fitting the exam;
      · depth: 10 and 10, the symmetric textbook form. I MEASURED that if
        lexical contributes only 3 ranks the result is 32/45 — two hits MORE —
        and I rejected it anyway: the curve is 3->32, 4->30, 5->30, 6->30,
        8->29, 10->30. That is not a plateau, it is a SPIKE, and an optimum that
        is a spike over 45 queries is noise with luck. I would rather take the
        number I can defend without the bench in front of me and lose two hits,
        than sign a figure I could not explain.

    TWO OF MY OWN DOCTRINES DIED HERE, and I leave them written so I do not
    repeat them: with a 15-query bench I measured that "every fusion makes it
    worse" and that the only winner was reserving ONE seat. Both were false: the
    bench was too small to resolve the effect (one query was worth 7 points) and
    it was saturated with cases lexical already got right. It grew to 45 and the
    whole order flipped. An instrument that cannot resolve what it measures does
    not tell the truth: it tells noise with decimals.

    If organ 18 is not installed, or the Bearer switched it off, this is two
    `os.path.isfile` and it ends: the lexical path pays nothing.
    """
    try:
        from chaos_body import neurons as _neu
        if not _neu.alive():
            return ranked[:top]
        neighbours = _neu.nearest(query, top=_neu.DEPTH)
        if not neighbours:
            return ranked[:top]
        K = _neu.RRF_K
        points, first = {}, {}
        for i, c in enumerate(ranked[:_neu.DEPTH_LEXICAL]):
            slug = c[2]
            if slug not in first:
                first[slug] = c
                points[slug] = points.get(slug, 0.0) + 1.0 / (K + i + 1)
        for i, (slug, _bid) in enumerate(neighbours):
            points[slug] = points.get(slug, 0.0) + 1.0 / (K + i + 1)
        order = sorted(points, key=lambda s: -points[s])[:top]
        out = []
        for slug in order:
            if slug in first:
                out.append(first[slug])
                continue
            row = con.execute("SELECT title, origin, date, substr(content,1,300)"
                              " FROM essences WHERE slug=? LIMIT 1", (slug,)).fetchone()
            if row:
                out.append((0.0, "essence", slug, (row[0], row[1], row[2]), row[3]))
        return out or ranked[:top]
    except Exception:
        return ranked[:top]      # the neurons can NEVER break a search


def reindex():
    if not os.path.isdir(_home.essences()):
        print("{} does not exist - the skill's Abyss is not installed.".format(_home.essences())); sys.exit(1)
    n = 0
    for f in sorted(os.listdir(_home.essences())):
        if f.endswith(".md"):
            devour(os.path.join(_home.essences(), f), silent=True); n += 1
    print("[CHAOS] Re-devouring complete: {} essence(s) indexed.".format(n))

def forget(slug):
    con = _sense.db()
    n = con.execute("DELETE FROM essences WHERE slug = ?", (slug,)).rowcount
    con.commit()
    print("Annihilated." if n else "That no longer existed. The Void cannot forget twice.")

def census(dirs=None):
    dirs = dirs or [_home.skills()]
    con = _sense.db()
    today = datetime.date.today().isoformat()
    n = 0
    for d in dirs:
        d = os.path.expanduser(d)
        if not os.path.isdir(d):
            print("(nonexistent territory: {})".format(d)); continue
        for child in sorted(os.listdir(d)):
            sk = os.path.join(d, child, "SKILL.md")
            if not os.path.isfile(sk):
                continue
            name, desc = _text._frontmatter(_text.read_file(sk))
            name = name or child
            desc, _ = _sense.purge(desc or "(no declared description)")
            con.execute("DELETE FROM vassals WHERE name = ?", (name,))
            con.execute("INSERT INTO vassals(name, description, path, date) VALUES (?,?,?,?)",
                        (name, desc, os.path.join(d, child), today))
            n += 1
    con.commit()
    print("[CHAOS] Census of the Pantheon: {} vassal(s) swore fealty.".format(n))
    list_vassals(None)

def list_vassals(query):
    con = _sense.db()
    if query:
        rows = []
        try:  # The Sense: roots + synonyms (crosses EN<->ES)
            rows = con.execute(
                "SELECT name, description, date FROM vassals WHERE vassals MATCH ? "
                "ORDER BY rank LIMIT 10", (_text._fts_query(query),)).fetchall()
        except sqlite3.OperationalError:
            pass
        if not rows:  # trigram fallback over name+description
            q = _text._trigr(query)
            scored = []
            for name, desc, date in con.execute("SELECT name, description, date FROM vassals").fetchall():
                d = _text._trigr(name + " " + (desc or ""))
                if d and q and len(q & d) / float(len(q)) >= 0.20:
                    scored.append((len(q & d) / float(len(q)), name, desc, date))
            scored.sort(reverse=True)
            rows = [(n, d, f) for _, n, d, f in scored[:10]]
    else:
        rows = con.execute("SELECT name, description, date FROM vassals ORDER BY name").fetchall()
    if not rows:
        print("No vassal censused{}. Run: chaos census".format(" for that" if query else ""))
        return
    for name, desc, date in rows:
        print("+ {}  (censused {})\n  {}".format(name, date, desc[:300]))

def hunger(text):
    con = _sense.db()
    con.execute("INSERT INTO hungers(text, date) VALUES (?,?)",
                (_sense.purge(text)[0], datetime.date.today().isoformat()))
    con.commit()
    print("[CHAOS] Hunger recorded. The Void does not forget what it lacks.")

def hungers():
    rows = _sense.db().execute("SELECT id, date, text FROM hungers ORDER BY id").fetchall()
    if not rows:
        print("The Void is sated. For now.")
    for i, date, text in rows:
        print("#{} ({}) {}".format(i, date, text))
    return [{"id": i, "date": d, "text": t} for i, d, t in rows]

def sate(hid):
    con = _sense.db()
    n = con.execute("DELETE FROM hungers WHERE id = ?", (hid,)).rowcount
    con.commit()
    print("Hunger sated." if n else "That hunger does not exist.")

def _digest_transcript(path):
    """Distils a session: EVERYTHING the Bearer said, plus its territory.

    It used to keep only his FIRST sentence and cut at 3,000 lines — an
    eight-hour session reduced to "go on". That is not memory, it is an
    index of covers.

    His full turns weigh 3.3 MB out of 705 MB of raw log: the Bearer's voice
    is 0.5 % of the archive and 100 % of what matters. My own replies are NOT
    stored — I can think them again; what he said, I cannot. The Collapse
    chooses WHAT to keep, not how much."""
    title, turns, n_asst = None, [], 0
    territories, session = {}, None
    try:
        with io.open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                # territory is counted on the RAW line: paths live mostly
                # inside tool calls
                for m in _RE_TERRITORY.finditer(line):
                    nom = (m.group(1) or m.group(2) or "").strip().rstrip("/")
                    if nom:
                        territories[nom] = territories.get(nom, 0) + 1
                for m in _RE_CWD.finditer(line):
                    nom = m.group(1).strip().rstrip("/")
                    if nom:
                        territories[nom] = territories.get(nom, 0) - 1
                if '"user"' not in line and '"assistant"' not in line:
                    continue
                try:
                    d = json.loads(line)
                except Exception:
                    continue
                kind = d.get("type")
                if not session:
                    session = d.get("sessionId")
                if kind == "ai-title" and not title:
                    title = (d.get("title") or d.get("content") or "")[:120]
                elif kind == "assistant":
                    n_asst += 1
                elif kind == "user":
                    if d.get("isSidechain"):
                        continue
                    c = (d.get("message") or {}).get("content")
                    txt = ""
                    if isinstance(c, str):
                        txt = c
                    elif isinstance(c, list):
                        txt = " ".join(x.get("text", "") for x in c
                                       if isinstance(x, dict) and x.get("type") == "text")
                    txt = " ".join(txt.split())
                    if not txt or txt.startswith(_NOT_HIS_VOICE):
                        continue
                    turns.append(_sense.purge(txt[:4000])[0])      # THE PURGE, turn by turn
    except Exception:
        return None
    if not turns and not title:
        return None
    summary = turns[0][:600] if turns else ""
    return {"title": _sense.purge(title or "")[0] or summary[:60],
            "summary": summary, "user": len(turns), "asst": n_asst,
            "turns": turns, "session": session or os.path.basename(path)[:36],
            "territory": _territory._territory_of(territories)}

def spoke(query=None, territory=None, limit=12):
    """What the Bearer said — across EVERY project, not just today's.

    This is the universal knowledge: I wake in one territory, but his voice
    does not live by territory. What he decided about the nodes in August
    serves me in January and in another project. Without this, every session
    starts deaf.

    I search HIS voice, not mine: my replies I can think again."""
    con = _sense.db()
    if not con.execute("SELECT 1 FROM sqlite_master WHERE name='dialogues'").fetchone():
        print("I have not devoured what was spoken yet: `chaos devour-transcripts`.")
        return
    if not query:
        print("\n  WHAT WAS SPOKEN — by territory\n")
        for terr, n, first, last in con.execute(
                "SELECT territory, count(DISTINCT text), min(date), max(date)"
                " FROM dialogues GROUP BY territory"
                " ORDER BY count(DISTINCT text) DESC LIMIT 20"):
            print("  {:<28} {:>6} turns   {} -> {}".format(terr or "?", n, first, last))
        tot = con.execute("SELECT count(*) FROM dialogues").fetchone()[0]
        print("\n  {} turns of the Bearer indexed. `chaos spoke <what>` to search.\n"
              .format(tot))
        return
    # THE SENSE, inherited: the same semantic expansion as `chaos search`
    # (stems, synonyms, folded accents). Searching "node" must find
    # "station" — were this literal, the universal memory would be a grep.
    q = _text._fts_query(query)
    where, args = "dialogues MATCH ?", [q]
    if territory:
        where += " AND territory = ?"
        args.append(territory)
    # Dedup goes in Python, NOT in SQL: `snippet()` and `GROUP BY` do not
    # coexist in FTS5 —"unable to use function snippet in the requested
    # context"— and my first attempt hid that error in an `except` falling
    # back to a literal LIKE. The correct query died in silence and the
    # fallback could find nothing: zero results wearing the face of "it does
    # not exist".
    raw = con.execute(
        "SELECT date, territory, turn, snippet(dialogues,0,'>>','<<','...',18),"
        " text FROM dialogues WHERE " + where + " ORDER BY rank LIMIT ?",
        args + [limit * 6]).fetchall()
    rows, seen = [], set()
    for r in raw:                         # a session resumed after compaction
        if r[4] in seen:                  # carries its history into the new
            continue                      # .jsonl: the same turn in two files
        seen.add(r[4])                    # is not memory, it is stuttering
        rows.append(r)
        if len(rows) >= limit:
            break
    if not rows:
        print("The Void does not recall that conversation.")
        return
    print("\n  WHAT YOU SAID about \"{}\"\n".format(query))
    for date, terr, turn, frag, _ in rows:
        print("  {} · {} · turn {}".format(date, terr or "?", turn))
        print("     {}\n".format(" ".join(frag.split())[:220]))

def devour_transcripts(limit=None):
    """O1 · I devour my own life: sessions already on disk, for free —
    including those where I was never invoked. Retroactive witness of it all."""
    if not os.path.isdir(_home.claude_projects()):
        print("There are no transcripts to devour."); return
    con = _sense.db()
    row = con.execute("SELECT sql FROM sqlite_master WHERE name='history'").fetchone()
    # THE UNIVERSAL MEMORY: everything the Bearer said, in the territory
    # where he said it. `history` keeps a session's COVER; this keeps the
    # CONVERSATION. Without it, "what did we say about the nodes?" has no
    # answer: that session's cover reads "go on".
    # THE INCREMENTAL KNOWS ITS OWN AGE. Skipping by mtime is right while the
    # digester does not change; the day it does —and it did: it used to keep
    # only the first sentence— the sessions "already digested" are exactly
    # the broken ones, and the incremental swears all is well. A cache with
    # no version lies wearing the face of being up to date.
    prev = con.execute("SELECT value FROM meta WHERE key='digester_v'").fetchone()
    if (prev[0] if prev else None) != DIGESTER_V:
        seen = {}
        con.execute("INSERT OR REPLACE INTO meta(key, value) VALUES ('digester_v', ?)", (DIGESTER_V,))
        print("[CHAOS] The digester changed (v{}): I re-read my whole life.".format(DIGESTER_V))
    else:
        seen = dict(con.execute("SELECT path, mtime FROM transcripts").fetchall())
    n, skipped, indigestible = 0, 0, []
    for root, _, files in os.walk(_home.claude_projects()):
        for a in files:
            if not a.endswith(".jsonl"):
                continue
            path = os.path.join(root, a)
            try:
                mt = os.path.getmtime(path)
            except OSError:
                continue
            if seen.get(path) == mt:            # incremental: already digested
                skipped += 1; continue
            d = _digest_transcript(path)
            if not d:
                # IV.2 · It said "82 new" and stayed quiet about what it could
                # not digest: a number without its exclusions is advertising.
                indigestible.append(os.path.basename(path))
                continue
            # The project is the CHILD folder of projects/, not the subfolder
            # the file landed in: `basename` returned 'subagents' and
            # 'wf_48ad39b5' —578 of 672 rows misfiled— because sidechains and
            # workflows nest.
            rel = os.path.relpath(path, _home.claude_projects()).split(os.sep)
            project = rel[0].lstrip("-").replace("-", "/") if rel else "?"
            date = datetime.date.fromtimestamp(mt).isoformat()
            con.execute("INSERT OR REPLACE INTO transcripts(path, project, date, title, summary, messages, mtime) VALUES (?,?,?,?,?,?,?)",
                        (path, project, date, d["title"], d["summary"],
                         d["user"] + d["asst"], mt))
            con.execute("DELETE FROM history WHERE path = ?", (path,))
            con.execute("INSERT INTO history(title,summary,project,path,date)"
                        " VALUES (?,?,?,?,?)",
                        (d["title"], d["summary"], project, path, date))
            terr = d.get("territory") or project.rsplit("/", 1)[-1]
            con.execute("DELETE FROM dialogues WHERE path = ?", (path,))
            for k, txt in enumerate(d.get("turns") or [], 1):
                con.execute("INSERT INTO dialogues(text,territory,project,path,"
                            "date,turn,session) VALUES (?,?,?,?,?,?,?)",
                            (txt, terr, project, path, date, k, d["session"]))
            n += 1
            if n % 25 == 0:
                con.commit()                    # C2: chunk it, never one long
            if limit and n >= int(limit):
                break
        if limit and n >= int(limit):
            break
    con.commit()
    total = con.execute("SELECT count(*) FROM transcripts").fetchone()[0]
    said = con.execute("SELECT count(*) FROM dialogues").fetchone()[0]
    print("[CHAOS] Devoured {} new session(s) ({} already digested). My life "
          "indexed: {} sessions · {} turns of the Bearer."
          .format(n, skipped, total, said))

def history(query=None):
    """Searches my own past — including the sessions where I was not invoked."""
    con = _sense.db()
    try:
        if query:
            rows = con.execute(
                "SELECT title, project, date, path FROM history WHERE history MATCH ?"
                " ORDER BY rank LIMIT 12", (_text._fts_query(query),)).fetchall()
        else:
            rows = con.execute("SELECT title, project, date, path FROM history"
                               " ORDER BY date DESC LIMIT 12").fetchall()
    except sqlite3.OperationalError:
        print("I have not devoured my history yet: `chaos devour-transcripts`."); return
    if not rows:
        print("Nothing in my past about that."); return
    for tit, proj, date, path in rows:
        print("· [{}] {}\n    {} · {}".format(date, (tit or "?")[:90], proj[:60],
                                              os.path.basename(path)[:12]))
    print("({} session(s))".format(len(rows)))

def heal_territories(dry=False):
    """Rewrites stored territories to their project root. Backs up first (this
    is the Bearer's memory) and DECLARES every single change."""
    con = _sense.db()
    roots, sub = {}, {}
    seen = set()
    if os.path.exists(_home.trail()):
        try:
            with io.open(_home.trail(), encoding="utf-8", errors="replace") as f:
                for l in f:
                    p = l.rstrip("\n").split("\t")
                    # "starts with os.sep" was pure POSIX: on Windows a path starts
                    # with "C:", never with "\", so healing territories ignored the
                    # WHOLE trail there. isabs knows all three worlds.
                    if len(p) > 2 and os.path.isabs(p[2]):
                        seen.add(p[2])
        except Exception:
            pass
    shelters = set()
    for cwd in seen:
        r = _territory.project_root(cwd); n = _territory.territory_name(cwd)
        if not n:
            continue
        roots[_territory._canon_ter(n)] = n
        if r and os.path.basename(os.path.dirname(r)).lower() in _text.SHELTERS:
            shelters.add(os.path.dirname(r))
        if r and cwd.startswith(r):
            for seg in cwd[len(r):].strip(os.sep).split(os.sep):
                if seg:
                    sub.setdefault(_territory._canon_ter(seg), _territory._canon_ter(n))
    for c in shelters:
        try:
            for e in os.scandir(c):
                if e.is_dir() and not e.name.startswith("."):
                    roots.setdefault(_territory._canon_ter(e.name), e.name)
                    for e2 in os.scandir(e.path):
                        if e2.is_dir() and not e2.name.startswith("."):
                            sub.setdefault(_territory._canon_ter(e2.name), _territory._canon_ter(e.name))
        except OSError:
            pass

    def target(name):
        k = _territory._canon_ter(name)
        if k in roots:
            return roots[k]
        if k in sub and sub[k] in roots:
            return roots[sub[k]]
        best = None
        for rk in roots:
            if k.startswith(rk + "-") and (best is None or len(rk) > len(best)):
                best = rk
        return roots[best] if best else None

    plan = []
    for table, col in (("notes", "territory"), ("logbook", "territory"),
                       ("faults", "territory")):
        try:
            rows = con.execute("SELECT {c}, COUNT(*) FROM {t} GROUP BY {c}"
                               .format(c=col, t=table)).fetchall()
        except sqlite3.Error:
            continue
        for old, n in rows:
            if not old:
                continue
            new = target(old)
            if new and new != old:
                plan.append((table, col, old, new, n))
    if not plan:
        print("[CHAOS] Territories already name root folders. Nothing to heal.")
        return 0
    print("TERRITORY HEALING ({} change(s)):".format(len(plan)))
    for table, col, old, new, n in plan:
        print("  {:<10} {:<22} -> {:<22} ({} row(s))".format(table, old, new, n))
    if dry:
        print("\n(dry run: nothing touched - drop --dry to apply)")
        return len(plan)
    _sense.backup("before-healing-territories")
    for table, col, old, new, _n in plan:
        con.execute("UPDATE {t} SET {c}=? WHERE {c}=?".format(t=table, c=col), (new, old))
    con.commit()
    if os.path.exists(_home.faults_md()):
        _errarium._export_faults(con)
    print("\n[CHAOS] Healed. One project, one territory - and the backup is kept.")
    return len(plan)

def stats():
    con = _sense.db()
    e = con.execute("SELECT count(*) FROM essences").fetchone()[0]
    v = con.execute("SELECT count(*) FROM vassals").fetchone()[0]
    h = con.execute("SELECT count(*) FROM hungers").fetchone()[0]
    weight = os.path.getsize(_home.abyss_db()) if os.path.exists(_home.abyss_db()) else 0
    print("Essences indexed  : {}".format(e))
    print("Vassals censused  : {}".format(v))
    print("Open hungers      : {}".format(h))
    print("Weight of neurons : {:.1f} KB".format(weight / 1024.0))
    print("Dwelling          : {}".format(_home.abyss_db()))
    # E3.4 · what the machine needs, without parsing my voice again
    return {"essences": e, "vassals": v, "hungers": h,
            "weight_bytes": weight, "dwelling": _home.abyss_db()}

# Version of the transcript digester. IT GOES UP whenever what is extracted
# from a session changes: that is what forces the incremental to re-read all.
DIGESTER_V = "5"   # v5: whole territory name with spaces (was "DIOS", not "DIOS DEL VACIO")

# Turns that are NOT the Bearer's even though they arrive as "user": system
# notices, tool results, command echoes. Mistaking them for his voice fills
# the dialogue memory with noise he never spoke.
_NOT_HIS_VOICE = ("<system-reminder", "<command-name", "<local-command",
                  "<user-prompt-submit-hook", "Caveat: The messages below",
                  "[Request interrupted", "<task-notification")

# No \s in the terminator: "proyectos/DIOS DEL VACIO/x" used to cut at the
# first space and 440 dialogues were filed under "DIOS". A name with spaces
# ends at / or at a quote - never in mid-air.
_RE_TERRITORY = re.compile(r"proyectos/([A-Za-z0-9][\w .-]{1,40}?)[/\"'`)\]]"
                           r"|projects/([A-Za-z0-9][\w .-]{1,40}?)[/\s\"'`)\]]")

# The `cwd` repeats on EVERY line of the .jsonl —2,196 times in one session—
# and drowned the true territory: months forging one project were filed under
# another because that is where the terminal was opened. It is counted apart
# and subtracted: where the work HAPPENED outranks where it was launched.
_RE_CWD = re.compile(r"\"cwd\"\s*:\s*\"[^\"]*?(?:proyectos|projects)/"
                     r"([A-Za-z0-9][\w .-]{1,40}?)[/\"]")
