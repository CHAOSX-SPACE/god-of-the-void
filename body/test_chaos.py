#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
CHAOS test net (EN edition) — pure stdlib, zero dependencies.
Runs the app in an isolated HOME and verifies output + DB state.
Covers the 3 historical bugs (import json, .format, thesaurus flush, sense
first-command) and the critical parts: Purge, The Sense, thesaurus, essence
cycle, census, Vigil, trail.

  python3 test_chaos.py            (or: python3 -m unittest test_chaos -v)
"""
import os, sys, io, json, time, shutil, sqlite3, tempfile, subprocess, unittest, ast

def _read_safe(path, encoding='utf-8'):
    """Reads and closes the descriptor: no ResourceWarning."""
    with io.open(path, encoding=encoding) as f:
        return f.read()


HERE = os.path.dirname(os.path.abspath(__file__))
APP = os.path.join(HERE, "chaos.py")


def run(env_home, *args):
    env = dict(os.environ)
    env["HOME"] = env_home
    env["CHAOS_HOME"] = os.path.join(env_home, ".chaos")
    # The child emits UTF-8 (its voice carries arrows and glyphs); the parent
    # decoded with the system encoding — cp1252 on Windows — and the capture
    # fell apart there. Decode as explicit UTF-8.
    p = subprocess.run([sys.executable, APP, *args], env=env,
                       capture_output=True, text=True,
                       encoding="utf-8", errors="replace")
    return (p.stdout or "") + (p.stderr or "")

def _body_source():
    """The whole BODY as text: the gate plus every module of the package.
    Reading `chaos.py` used to be enough because the body was one file; now a
    rule may live in any room, and a test that only looks at the gate would
    declare absent what exists."""
    parts = [_read_safe(APP)]
    paq = os.path.join(HERE, "chaos_body")
    for root, _, files in os.walk(paq):
        if "__pycache__" in root:
            continue
        for f in sorted(files):
            if f.endswith(".py"):
                parts.append(_read_safe(os.path.join(root, f)))
    return "\n".join(parts)


def _install_body(bin_dst):
    """An installed body is the gate + the leaf of routes + the package.
    Copying only `chaos.py` leaves an installation that does not start."""
    os.makedirs(bin_dst, exist_ok=True)
    shutil.copy2(APP, os.path.join(bin_dst, "chaos.py"))
    for leaf in ("hogar.py", "home.py"):
        if os.path.exists(os.path.join(HERE, leaf)):
            shutil.copy2(os.path.join(HERE, leaf), os.path.join(bin_dst, leaf))
    for paq in ("chaos_cuerpo", "chaos_body"):
        src = os.path.join(HERE, paq)
        if os.path.isdir(src):
            shutil.copytree(src, os.path.join(bin_dst, paq),
                            ignore=shutil.ignore_patterns("__pycache__"),
                            dirs_exist_ok=True)


class ChaosTest(unittest.TestCase):
    def setUp(self):
        self.home = tempfile.mkdtemp(prefix="chaos-test-")
        self.chaos = os.path.join(self.home, ".chaos")
        self.essences = os.path.join(self.home, ".claude", "skills", "chaos", "abyss", "essences")
        os.makedirs(self.essences, exist_ok=True)
        self.db = os.path.join(self.chaos, "abyss.db")

    def tearDown(self):
        shutil.rmtree(self.home, ignore_errors=True)

    def _essence(self, name, text):
        p = os.path.join(self.essences, name + ".md")
        with io.open(p, "w", encoding="utf-8") as f:
            f.write(text)
        return p

    def _db_rows(self, sql):
        con = sqlite3.connect(self.db)
        try:
            return con.execute(sql).fetchall()
        finally:
            con.close()

    def _load_json(self, path):
        with io.open(path, encoding="utf-8") as f:
            return json.load(f)

    # ── Purge: security BUG if a key survives ─────────────────────────────
    def test_purge_annihilates_keys(self):
        p = self._essence("poison", "# Doc\nkey sk-abcdefghij1234567890 and ghp_ABCDEFGHIJKLMNOPQRST123456\nuseful radar text")
        run(self.home, "devour", p)
        row = self._db_rows("SELECT content FROM essences WHERE slug='poison'")
        self.assertTrue(row, "essence not indexed")
        content = row[0][0]
        self.assertNotIn("sk-abcdefghij", content, "sk- KEY SURVIVED (leak)")
        self.assertNotIn("ghp_ABCDEFGH", content, "ghp_ KEY SURVIVED (leak)")
        self.assertIn("PURGED", content)

    # ── The Sense: accents, synonyms, roots, typos ────────────────────────
    def test_sense_accents(self):
        self._essence("mat", "# Course\nquadratic equations of the system")
        run(self.home, "reindex")
        self.assertIn("Course", run(self.home, "search", "quadratics"))

    def test_sense_synonym(self):
        self._essence("veh", "# Guide\nvehicles and automobiles of today")
        run(self.home, "reindex")
        self.assertIn("Guide", run(self.home, "search", "car"))

    def test_sense_typo_trigrams(self):
        self._essence("serv", "# Manual\nconfiguration of the main server")
        run(self.home, "reindex")
        self.assertIn("Manual", run(self.home, "search", "servr"))

    # ── Thesaurus: historical BUG (import json + flush) ───────────────────
    def test_thesaurus_persists(self):
        run(self.home, "search", "anything")
        tpath = os.path.join(self.chaos, "thesaurus.json")
        self.assertTrue(os.path.exists(tpath), "thesaurus NOT created (flush/json bug)")
        self.assertGreater(os.path.getsize(tpath), 10, "thesaurus empty (0 bytes)")
        self._load_json(tpath)

    def test_sense_learns_first_command(self):
        # historical BUG: 'sense' as FIRST command, without ~/.chaos created yet
        run(self.home, "sense", "radar", "aeris")
        tpath = os.path.join(self.chaos, "thesaurus.json")
        self.assertTrue(os.path.exists(tpath), "sense did NOT create thesaurus (~/.chaos missing)")
        self.assertIn("aeris", self._load_json(tpath).get("radar", []), "bond did NOT persist")

    # ── Essence cycle: devour → search → forget ───────────────────────────
    def test_essence_cycle(self):
        p = self._essence("one", "# Title One\nsingular alpha content")
        run(self.home, "devour", p)
        self.assertIn("Title One", run(self.home, "search", "alpha"))
        run(self.home, "forget", "one")
        self.assertEqual(self._db_rows("SELECT count(*) FROM essences")[0][0], 0)

    # ── Pantheon census ───────────────────────────────────────────────────
    def test_census(self):
        vdir = os.path.join(self.home, ".claude", "skills", "other")
        os.makedirs(vdir, exist_ok=True)
        with io.open(os.path.join(vdir, "SKILL.md"), "w", encoding="utf-8") as f:
            f.write("---\nname: other\ndescription: a test vassal\n---\nbody")
        run(self.home, "census")
        names = [r[0] for r in self._db_rows("SELECT name FROM vassals")]
        self.assertIn("other", names, "vassal NOT censused")

    # ── Hungers: record → list → sate ─────────────────────────────────────
    def test_hungers(self):
        run(self.home, "hunger", "something important is missing")
        self.assertIn("important", run(self.home, "hungers"))
        run(self.home, "sate", "1")
        self.assertEqual(self._db_rows("SELECT count(*) FROM hungers")[0][0], 0)

    # ── The Vigil: detects planted drift ──────────────────────────────────
    def test_vigil_detects_drift(self):
        self._essence("orphan", "# Orphan\non disk but not indexed")
        self.assertIn("DRIFT", run(self.home, "audit"), "Vigil did NOT detect the orphan")

    def test_vigil_due(self):
        self.assertIn("YES", run(self.home, "vigil-due"))
        run(self.home, "audit")
        self.assertIn("NO", run(self.home, "vigil-due"))

    # ── Trail: log, filter noise, purge ───────────────────────────────────
    def test_trail(self):
        run(self.home, "trail", "/x/work.md", "create")
        self.assertIn("work.md", run(self.home, "trail"))
        run(self.home, "trail", "/tmp/junk.md", "edit")  # noise
        self.assertNotIn("junk", run(self.home, "trail"), "noise NOT filtered")
        run(self.home, "trail", "--purge")
        self.assertIn("empty", run(self.home, "trail"))

    # ── FTS auto-migration: old accent-less table is reforged ─────────────
    def test_fts_migration(self):
        os.makedirs(self.chaos, exist_ok=True)
        con = sqlite3.connect(self.db)
        con.execute("CREATE VIRTUAL TABLE essences USING fts5(slug, title, content, origin, date)")
        con.commit(); con.close()
        run(self.home, "stats")
        sql = self._db_rows("SELECT sql FROM sqlite_master WHERE name='essences'")[0][0]
        self.assertIn("remove_diacritics", sql, "old table NOT reforged")


    # ══ E−1 · THE FOUNDATION ═══════════════════════════════════════════════

    def test_c2_wal_and_concurrency(self):
        """C2: WAL active and no write dies under a long operation."""
        import threading, time
        run(self.home, "stats")
        mode = self._db_rows("PRAGMA journal_mode")[0][0]
        self.assertEqual(mode, "wal", "WAL NOT active → concurrent writes die")
        err = []
        def long_op():
            c = sqlite3.connect(self.db, timeout=30.0); c.execute("PRAGMA busy_timeout=30000")
            c.execute("BEGIN IMMEDIATE"); time.sleep(2)
            c.execute("INSERT INTO hungers(text,date) VALUES ('long','x')"); c.commit(); c.close()
        def short_op(n):
            time.sleep(0.2)
            try:
                c = sqlite3.connect(self.db, timeout=30.0); c.execute("PRAGMA busy_timeout=30000")
                c.execute("INSERT INTO hungers(text,date) VALUES (?,'x')", (f"c{n}",))
                c.commit(); c.close()
            except Exception as e: err.append(type(e).__name__)
        hs = [threading.Thread(target=long_op)] + [threading.Thread(target=short_op, args=(i,)) for i in range(4)]
        [h.start() for h in hs]; [h.join() for h in hs]
        self.assertEqual(err, [], "writes LOST under concurrency")
        self.assertEqual(self._db_rows("SELECT count(*) FROM hungers")[0][0], 5)

    def test_c5_frontmatter_does_not_corrupt_title(self):
        """C5: the bug that would have renamed EVERY essence to '---'."""
        p = self._essence("withmeta", "---\ntype: project\ndevoured: 2026-07-26\n---\n\n# True Title\nbody")
        run(self.home, "devour", p)
        t = self._db_rows("SELECT title FROM essences WHERE slug='withmeta'")[0][0]
        self.assertNotEqual(t, "---", "TITLE CORRUPTED by the frontmatter")
        self.assertEqual(t, "True Title")

    def test_c6_schema_version(self):
        run(self.home, "stats")
        v = self._db_rows("SELECT value FROM meta WHERE key='schema_version'")
        self.assertTrue(v and int(v[0][0]) >= 2, "no schema version")

    def test_c1_rich_trail(self):
        """C1: the trail stores ISO time, session, cwd and tool."""
        run(self.home, "trail", "/x/work.md", "create", "ses-1", "/proj/one", "Write")
        line = run(self.home, "trail").strip().split("\n")[0]
        fields = line.split("\t")
        self.assertEqual(len(fields), 6, "the trail does NOT have the 6 fields → FOCUS incomputable")
        self.assertIn("T", fields[0], "no ISO time")
        self.assertEqual(fields[1], "ses-1"); self.assertEqual(fields[2], "/proj/one")

    def test_c3_purge_per_session(self):
        """C3: purging one session does NOT erase another's work."""
        run(self.home, "trail", "/a.md", "create", "ses-A", "/p", "Write")
        run(self.home, "trail", "/b.md", "create", "ses-B", "/p", "Write")
        run(self.home, "trail", "--purge", "ses-A")
        left = run(self.home, "trail")
        self.assertNotIn("/a.md", left, "did not purge its own")
        self.assertIn("/b.md", left, "ERASED another session's work (race)")

    def test_c7_backup(self):
        self._essence("valuable", "# Valuable\nmust not be lost")
        run(self.home, "backup", "test")
        base = os.path.join(self.chaos, "backups")
        self.assertTrue(os.path.isdir(base), "did not create backups")
        found = any("valuable.md" in files for _, _, files in os.walk(base))
        self.assertTrue(found, "the backup does NOT contain the essences")

    def test_c8_externals_are_not_ghosts(self):
        """C8: a doc devoured outside the Abyss is not flagged deletable."""
        ext = os.path.join(self.home, "external.md")
        with io.open(ext, "w", encoding="utf-8") as f:
            f.write("# External doc\ncontent devoured from another path")
        run(self.home, "devour", ext)
        out = run(self.home, "audit")
        self.assertIn("EXTERNALS", out, "does not tell externals from ghosts")
        self.assertNotIn("no file: external", out, "flags an external as a deletable ghost")


    # ══ E1 · THE GRAMMAR (The Weave) ═══════════════════════════════════════

    def _with_meta(self, name, type_, state, tags="", body="content"):
        return self._essence(name, "---\ntype: {}\nstate: {}\ntags: [{}]\n"
                             "devoured: 2026-07-27\n---\n\n# {}\n{}\n"
                             .format(type_, state, tags, name.title(), body))

    def test_e1_query_by_attributes(self):
        self._with_meta("p-one", "project", "active", "radar")
        self._with_meta("p-two", "project", "closed", "ai")
        self._with_meta("r-three", "reference", "active", "radar")
        run(self.home, "reindex")
        out = run(self.home, "query", "type:project", "state:active")
        self.assertIn("p-one", out)
        self.assertNotIn("p-two", out, "brought a closed project")
        self.assertNotIn("r-three", out, "brought a reference")

    def test_e1_tags_do_not_match_prefixes(self):
        """tag 'radar' must NOT match 'radar-2' (the LIKE blob bug)."""
        self._with_meta("a", "project", "active", "radar")
        self._with_meta("b", "project", "active", "radar-2")
        run(self.home, "reindex")
        out = run(self.home, "query", "tag:radar")
        self.assertIn("· a", out)
        self.assertNotIn("· b", out, "the tag 'radar' matched 'radar-2'")

    def test_e1_links_do_not_collapse(self):
        """TWO mentions of the same essence = TWO links with their line."""
        self._with_meta("target", "project", "active")
        self._essence("source", "---\ntype: project\n---\n\n# Source\n"
                                "first mention of [[target]]\n"
                                "second mention of [[target]]\n")
        run(self.home, "reindex"); run(self.home, "weave")
        n = self._db_rows("SELECT count(*) FROM links WHERE target='target'")[0][0]
        self.assertEqual(n, 2, "the mentions COLLAPSED into a single one")
        out = run(self.home, "links", "target")
        self.assertIn("first mention", out); self.assertIn("second mention", out)

    def test_e1_block_in_link(self):
        """[[essence#^block]] must be stored with its block, not lost."""
        self._with_meta("dest", "project", "active")
        self._essence("orig", "---\ntype: project\n---\n\n# O\nsee [[dest#^engine]]\n")
        run(self.home, "reindex"); run(self.home, "weave")
        b = self._db_rows("SELECT block FROM links WHERE target='dest'")[0][0]
        self.assertEqual(b, "^engine", "the link's block was lost")

    def test_e1_orphans(self):
        self._with_meta("alone", "reference", "active")
        self._with_meta("dest2", "project", "active")
        self._essence("orig2", "---\ntype: project\n---\n\n# O2\ngoes to [[dest2]]\n")
        run(self.home, "reindex"); run(self.home, "weave")
        out = run(self.home, "orphans")
        self.assertIn("alone", out)
        self.assertNotIn("· dest2", out, "flagged a linked essence as an orphan")

    def test_e1_bad_frontmatter_does_not_break(self):
        """Broken frontmatter: degrades, does NOT explode."""
        self._essence("broken", "---\ntype project without colon\n:::\n---\n\n# Broken\ntext")
        out = run(self.home, "reindex")
        self.assertNotIn("Traceback", out, "the bad frontmatter BROKE the app")
        self.assertIn("broken", run(self.home, "search", "text"))

    def test_e1_weave_is_derived(self):
        """Wiping the graph and re-weaving rebuilds it identically from the .md."""
        self._with_meta("d", "project", "active")
        self._essence("o", "---\ntype: project\n---\n\n# O\ntoward [[d]]\n")
        run(self.home, "reindex"); run(self.home, "weave")
        before = self._db_rows("SELECT count(*) FROM links")[0][0]
        con = sqlite3.connect(self.db); con.execute("DELETE FROM links"); con.commit(); con.close()
        run(self.home, "weave")
        self.assertEqual(self._db_rows("SELECT count(*) FROM links")[0][0], before,
                         "the graph was NOT rebuilt from the text")


    # ══ E2 · THE BLOCKS (end of the waste) ═════════════════════════════════

    def test_e2_returns_block_not_sack(self):
        """The find must weigh <5% of the essence. Before: 8,151 tokens for one fact."""
        filler = "\n\n".join("Inconsequential filler number %d. " % i * 10 for i in range(40))
        self._essence("fat", "---\ntype: project\n---\n\n# Fat\n\n"
                      "The engine uses gemma with three gigs of RAM. ^engine\n\n" + filler)
        run(self.home, "reindex"); run(self.home, "weave")
        doc = os.path.getsize(os.path.join(self.essences, "fat.md"))
        out = run(self.home, "search", "engine gemma")
        self.assertIn("^engine", out, "did NOT return the block")
        self.assertLess(len(out), doc * 0.05,
                        "returned the whole sack instead of the paragraph")

    def test_e2_block_id_does_not_pollute_search(self):
        """Searching 'judgment' must NOT match the id ^judgment (slug/id UNINDEXED)."""
        self._essence("jud", "---\ntype: doctrine\n---\n\n# J\n\n"
                             "This paragraph speaks of apples and pears. ^judgment\n")
        run(self.home, "reindex"); run(self.home, "weave")
        out = run(self.home, "search", "judgment")
        self.assertNotIn("apples", out, "the block id polluted the search")

    def test_e2_brief_output(self):
        """--brief must weigh quite a bit less than the normal one."""
        self._essence("m", "---\ntype: project\n---\n\n# M\n\n"
                           "Unique fact about X-band radars. ^fact\n")
        run(self.home, "reindex"); run(self.home, "weave")
        normal = run(self.home, "search", "radars band")
        lean = run(self.home, "search", "radars band", "--brief")
        self.assertIn("radars", lean)
        self.assertLess(len(lean), len(normal), "--brief did NOT slim the output")

    def test_e2_without_blocks_falls_back_to_essence(self):
        """Essence without blocks: search keeps working as before."""
        self._essence("flat", "---\ntype: reference\n---\n\n# Flat\ntext without marked blocks\n")
        run(self.home, "reindex"); run(self.home, "weave")
        self.assertIn("flat", run(self.home, "search", "text blocks", "--brief"))


    # ══ E5 · THE MIGRATION ═════════════════════════════════════════════════

    def test_e5_loses_no_word(self):
        """The body must survive IDENTICAL. Only frontmatter is prepended."""
        body = "# Radar Project\nValuable fact one.\nValuable fact two.\n"
        p = self._essence("project-radar", body)
        run(self.home, "reindex"); run(self.home, "evolve")
        new = _read_safe(p, encoding="utf-8")
        self.assertTrue(new.startswith("---"), "did not add frontmatter")
        self.assertTrue(new.endswith(body), "the BODY was altered")

    def test_e5_does_not_corrupt_titles(self):
        """The C5 bug: after migrating, the title CANNOT be '---'."""
        self._essence("project-x", "# Authentic Title\ncontent")
        run(self.home, "reindex"); run(self.home, "evolve")
        t = self._db_rows("SELECT title FROM essences WHERE slug='project-x'")[0][0]
        self.assertEqual(t, "Authentic Title", "the migration CORRUPTED the title")

    def test_e5_idempotent(self):
        self._essence("project-y", "# Y\nbody")
        run(self.home, "reindex")
        run(self.home, "evolve")
        first = _read_safe(os.path.join(self.essences, "project-y.md"), encoding="utf-8")
        run(self.home, "evolve")
        second = _read_safe(os.path.join(self.essences, "project-y.md"), encoding="utf-8")
        self.assertEqual(first, second, "the second migration DUPLICATED the frontmatter")

    def test_e5_infers_types(self):
        self._essence("project-one", "# P\nx")
        self._essence("territory-two", "# T\nx")
        self._essence("scars", "# C\nx")
        run(self.home, "reindex"); run(self.home, "evolve")
        types = dict(self._db_rows("SELECT slug, type FROM essence_meta"))
        self.assertEqual(types.get("project-one"), "project")
        self.assertEqual(types.get("territory-two"), "territory")
        self.assertEqual(types.get("scars"), "scar")

    def test_e5_backs_up_first(self):
        self._essence("valuable2", "# V\ndo not lose")
        run(self.home, "reindex"); run(self.home, "evolve")
        base = os.path.join(self.chaos, "backups")
        found = any("valuable2.md" in files for _, _, files in os.walk(base))
        self.assertTrue(found, "migrated WITHOUT backing up (C7 violated)")

    def test_e5_dry_run_touches_nothing(self):
        p = self._essence("project-dry", "# S\nbody")
        before = _read_safe(p, encoding="utf-8")
        run(self.home, "reindex")
        out = run(self.home, "evolve", "--dry")
        self.assertIn("Would migrate", out)
        self.assertEqual(_read_safe(p, encoding="utf-8"), before,
                         "the dry run MODIFIED the file")


    # ══ E3 · THE GRAPH (derived index) ═════════════════════════════════════

    def _abyss_md(self):
        return os.path.join(os.path.dirname(self.essences), "ABYSS.md")

    def test_e3_index_respects_handwritten(self):
        """What is written outside the marks is SACRED."""
        sacred = "> Note from the Bearer that NOBODY must erase.\n"
        with io.open(self._abyss_md(), "w", encoding="utf-8") as f:
            f.write("# THE ABYSS\n\n" + sacred)
        self._with_meta("p-ind", "project", "active")
        run(self.home, "reindex"); run(self.home, "index")
        new = _read_safe(self._abyss_md(), encoding="utf-8")
        self.assertIn(sacred, new, "ERASED handwritten content")
        self.assertIn("CHAOS:AUTO", new, "did not sow the marks")
        self.assertIn("p-ind", new, "did not list the essence")

    def test_e3_index_idempotent(self):
        self._with_meta("p-idem", "project", "active")
        run(self.home, "reindex")
        run(self.home, "index"); first = _read_safe(self._abyss_md(), encoding="utf-8")
        run(self.home, "index"); second = _read_safe(self._abyss_md(), encoding="utf-8")
        self.assertEqual(first.count("CHAOS:AUTO start"), 1, "duplicated the marks")
        self.assertEqual(first, second, "the index is not stable")

    def test_e3_index_marks_orphans(self):
        self._with_meta("alone3", "reference", "active")
        run(self.home, "reindex"); run(self.home, "weave"); run(self.home, "index")
        self.assertIn("orphan", _read_safe(self._abyss_md(), encoding="utf-8"))


    # ══ E4 · THE DISCOVERY (unlinked mentions) ═════════════════════════════

    def test_e4_finds_unlinked_mention(self):
        self._with_meta("project-new-age", "project", "active")
        self._essence("doctrine-x", "---\ntype: doctrine\n---\n\n# D\n"
                                    "Talks about the project new age but does not link it.\n")
        run(self.home, "reindex"); run(self.home, "weave")
        out = run(self.home, "suggest")
        self.assertIn("doctrine-x", out); self.assertIn("project-new-age", out)

    def test_e4_does_not_suggest_existing_link(self):
        self._with_meta("target4", "project", "active")
        self._essence("source4", "---\ntype: project\n---\n\n# O\nalready links [[target4]] here.\n")
        run(self.home, "reindex"); run(self.home, "weave")
        out = run(self.home, "suggest")
        self.assertNotIn("source4 → target4", out, "suggested a bond that ALREADY exists")

    def test_e4_rejection_is_final(self):
        self._with_meta("project-new-age", "project", "active")
        self._essence("doctrine-y", "---\ntype: doctrine\n---\n\n# D\n"
                                    "mentions project new age without linking.\n")
        run(self.home, "reindex"); run(self.home, "weave")
        self.assertIn("doctrine-y", run(self.home, "suggest"))
        run(self.home, "suggest", "--kill", "doctrine-y->project-new-age")
        self.assertNotIn("doctrine-y → project-new-age", run(self.home, "suggest"),
                         "the rejected suggestion CAME BACK")


    # ══ E9 · THE CHRONICLE (sparks + logbook) ══════════════════════════════

    def _in_territory(self, sub, *args):
        """Runs the app FROM a concrete territory (to test cwd/focus)."""
        terr = os.path.join(self.home, sub); os.makedirs(terr, exist_ok=True)
        env = dict(os.environ); env["HOME"] = self.home
        env["CHAOS_HOME"] = self.chaos
        p = subprocess.run([sys.executable, APP, *args], env=env, cwd=terr,
                           capture_output=True, text=True)
        return p.stdout + p.stderr

    def test_e9_note_anchors_three_levels(self):
        """Territory + FOCUS (from the trail) + semantic anchor."""
        self._with_meta("project-radar", "project", "active",
                        body="phased array for aerial detection")
        run(self.home, "reindex"); run(self.home, "weave")
        terr = os.path.join(self.home, "project-radar")
        run(self.home, "trail", os.path.join(terr, "PLAN.md"), "edit", "s1", terr, "Edit")
        self._in_territory("project-radar", "note", "the radar needs a noise filter")
        out = self._in_territory("project-radar", "note-where", "1")
        self.assertIn("project-radar", out, "no territory")
        self.assertIn("PLAN.md", out, "no FOCUS (level 2 failed)")
        self.assertIn("anchor", out)

    def test_e9_honest_spark_without_anchor(self):
        """If it fits nothing: 'no anchor'. NEVER invent a connection."""
        run(self.home, "note", "zzz qqq xxx vvv")
        out = run(self.home, "note-where", "1")
        self.assertIn("no anchor", out, "INVENTED an anchor that does not exist")

    def test_e9_ascend_spark_to_essence(self):
        run(self.home, "note", "the signal engine needs fine calibration")
        run(self.home, "ascend", "1")
        found = [f for f in os.listdir(self.essences) if "engine" in f]
        self.assertTrue(found, "the spark did NOT ascend to an essence")
        content = _read_safe(os.path.join(self.essences, found[0]))
        self.assertTrue(content.startswith("---"), "the essence was born without grammar")
        state = self._db_rows("SELECT state FROM notes WHERE id=1")[0][0]
        self.assertEqual(state, "ascended")

    def test_e9_logbook_only_documents_acts(self):
        """Empty trail = there were only words = NOTHING to document."""
        out = run(self.home, "undocumented")
        self.assertIn("there was no work", out, "demanded a chronicle with no work done")
        run(self.home, "trail", "/x/work.md", "create", "s1", self.home, "Write")
        self.assertIn("CHRONICLE DUTY", run(self.home, "undocumented"),
                      "there was work and it did NOT demand it")

    def test_e9_chronicle_links_files(self):
        run(self.home, "trail", os.path.join(self.home, "work.md"), "create", "s1", self.home, "Write")
        out = run(self.home, "chronicle", "--what", "Forged something", "--why", "it was needed")
        self.assertNotIn("0 file", out, "the chronicle did NOT link the trail's files")
        self.assertIn("Forged something", run(self.home, "chronicle"))

    def test_e9_chronicle_survives_the_db(self):
        """The logbook is exported to markdown: the text is the last truth."""
        run(self.home, "chronicle", "--what", "Important change", "--why", "a clear reason")
        run(self.home, "export-chronicle")
        base = os.path.join(os.path.dirname(self.essences), "chronicle")
        self.assertTrue(os.path.isdir(base), "did not export the chronicle")
        texts = "".join(_read_safe(os.path.join(base, f)) for f in os.listdir(base))
        self.assertIn("Important change", texts)
        self.assertIn("a clear reason", texts)


    # ══ E8 · LIVE PRESENCE (dynamic anchor) ════════════════════════════════

    def _presence(self, cwd=""):
        hook = os.path.join(HERE, "presence-hook.py")
        env = dict(os.environ); env["HOME"] = self.home; env["CHAOS_HOME"] = self.chaos
        p = subprocess.run([sys.executable, hook], input=json.dumps({"cwd": cwd}),
                           capture_output=True, text=True, env=env)
        return json.loads(p.stdout)["hookSpecificOutput"]["additionalContext"]

    def test_e8_injects_live_state(self):
        self._with_meta("p-live", "project", "active")
        run(self.home, "reindex")
        c = self._presence(self.home)
        self.assertIn("LIVE MEMORY", c, "did not inject the memory's state")
        self.assertIn("essences", c)

    def test_e8_virgin_territory_announces_root(self):
        c = self._presence("/tmp/territory-never-seen-xyz")
        self.assertIn("VIRGIN", c); self.assertIn("Root", c)

    def test_e8_chronicle_duty_only_if_work(self):
        without = self._presence(self.home)
        self.assertNotIn("UNDOCUMENTED", without, "demanded a chronicle with no work")
        run(self.home, "trail", "/x/w.md", "create", "s1", self.home, "Write")
        self.assertIn("UNDOCUMENTED", self._presence(self.home))

    def test_e8_degrades_if_db_dies(self):
        hook = os.path.join(HERE, "presence-hook.py")
        env = dict(os.environ); env["HOME"] = self.home
        env["CHAOS_HOME"] = os.path.join(self.home, "does-not-exist")
        p = subprocess.run([sys.executable, hook], input="{}", capture_output=True,
                           text=True, env=env)
        c = json.loads(p.stdout)["hookSpecificOutput"]["additionalContext"]
        self.assertIn("CHAOS", c, "did not degrade to the static anchor")
        self.assertNotIn("LIVE MEMORY", c)

    def test_e8_token_ceiling(self):
        for i in range(30):
            self._with_meta("p-{}".format(i), "project", "active")
        run(self.home, "reindex")
        c = self._presence(self.home)
        self.assertLess(len(c), 1500, "the presence EXCEEDED its hard ceiling")


    # ══ E10 · THE NEVER-SLEEPING EYES ══════════════════════════════════════

    def _transcript(self, project, name, intent, title=None):
        d = os.path.join(self.home, ".claude", "projects", project)
        os.makedirs(d, exist_ok=True)
        p = os.path.join(d, name + ".jsonl")
        ev = []
        if title:
            ev.append(json.dumps({"type": "ai-title", "title": title}))
        ev.append(json.dumps({"type": "user", "message": {"content": intent}}))
        ev.append(json.dumps({"type": "assistant", "message": {"content": "ok"}}))
        with io.open(p, "w", encoding="utf-8") as f:
            f.write("\n".join(ev) + "\n")
        return p

    def test_e10_devours_its_own_life(self):
        self._transcript("-proj-radar", "s1", "fix the 10.5GHz phased array radar")
        run(self.home, "devour-transcripts")
        n = self._db_rows("SELECT count(*) FROM transcripts")[0][0]
        self.assertGreaterEqual(n, 1, "did not devour the transcript")

    def test_e10_retroactive_witness(self):
        """Remembering a session where CHAOS was never invoked."""
        self._transcript("-proj-x", "s2", "I need to calibrate the X band interferometer")
        run(self.home, "devour-transcripts")
        out = run(self.home, "history", "interferometer band")
        self.assertIn("interferometer", out.lower(), "did not find its own past")

    def test_e10_incremental(self):
        self._transcript("-proj-y", "s3", "first session about warehousing")
        run(self.home, "devour-transcripts")
        out = run(self.home, "devour-transcripts")
        self.assertIn("already digested", out)
        self.assertIn("Devoured 0", out, "re-digested what was digested (not incremental)")

    def test_e10_purges_transcripts(self):
        """A secret in a session CANNOT survive its digestion."""
        self._transcript("-proj-z", "s4", "the key is sk-abcdefghij1234567890 do not lose it")
        run(self.home, "devour-transcripts")
        r = self._db_rows("SELECT summary FROM transcripts")[0][0]
        self.assertNotIn("sk-abcdefghij", r, "KEY leaked from a transcript")

    def test_e10_mirrors_parallel_memory(self):
        d = os.path.join(self.home, ".claude", "projects", "-p", "memory")
        os.makedirs(d, exist_ok=True)
        with io.open(os.path.join(d, "project-thing.md"), "w", encoding="utf-8") as f:
            f.write("# Project Thing\ndata from the parallel memory\n")
        out = run(self.home, "mirror")
        self.assertIn("mirrored", out)
        n = self._db_rows("SELECT count(*) FROM essences WHERE slug='project-thing'")[0][0]
        self.assertEqual(n, 1, "did not devour the parallel memory")

    def test_e10_expired(self):
        self._essence("old-truth", "---\ntype: reference\nexpires: 2020-01-01\n---\n\n# V\nold datum")
        self._essence("new-truth", "---\ntype: reference\nexpires: 2099-01-01\n---\n\n# N\ncurrent datum")
        run(self.home, "reindex")
        out = run(self.home, "expired")
        self.assertIn("old-truth", out, "did not detect the expired truth")
        self.assertNotIn("new-truth", out, "flagged a current truth as expired")

    def test_e10_delta_without_git_declares_it(self):
        out = run(self.home, "delta", self.home)
        self.assertIn("git", out.lower(), "did not declare the blindness without git")


    # ══ O4 · THE VIGIL-SWEEP (work while the Bearer sleeps) ════════════════

    def test_o4_vigil_sweep_leaves_report(self):
        self._with_meta("p-vigil", "project", "active")
        run(self.home, "reindex")
        out = run(self.home, "vigil-sweep")
        self.assertIn("Vigil-sweep finished", out)
        rep = os.path.join(self.chaos, "forge", "vigil.md")
        self.assertTrue(os.path.exists(rep), "the vigil-sweep left NO report")
        self.assertIn("front", _read_safe(rep))

    def test_o4_report_reads_the_sweep(self):
        run(self.home, "vigil-sweep")
        self.assertIn("VIGIL-SWEEP REPORT", run(self.home, "report"))

    def test_o4_report_without_sweep_is_honest(self):
        out = run(self.home, "report")
        self.assertIn("have not kept watch", out, "lied about a sweep that never happened")
        self.assertNotIn("Traceback", out)

    def test_o4_presence_announces_the_report(self):
        """After a sweep with findings, the Presence announces it on return."""
        self._essence("p-orph", "---\ntype: project\n---\n\n# H\nuses [[nonexistent]]\n")
        run(self.home, "reindex"); run(self.home, "vigil-sweep")
        c = self._presence(self.home)
        self.assertIn("VIGIL:", c, "the Presence did NOT announce the vigil-sweep report")


    def test_o4_sweep_does_not_die_if_a_step_fails(self):
        """A step that blows up cannot bring down the whole sweep."""
        out = run(self.home, "vigil-sweep")
        self.assertIn("Vigil", out)
        self.assertNotIn("Traceback", out)

    def test_o4_schedule_is_cross_platform(self):
        """The code covers the 3 systems (NOT executed: it would install a real task)."""
        src = _body_source()
        i = src.find("def schedule")
        block = src[i:src.find("\ndef ", i + 10)]
        for mark, system in (("launchctl", "macOS"), ("schtasks", "Windows"), ("crontab", "Linux")):
            self.assertIn(mark, block, "no support for " + system)


    # ══ O4-bis · THE AUTONOMOUS HEARTBEAT AND ITS CAGE ═════════════════════

    def test_o4_panic_switch(self):
        """One file is enough to stop a god."""
        os.makedirs(self.chaos, exist_ok=True)
        with io.open(os.path.join(self.chaos, "STOP"), "w", encoding="utf-8") as f:
            f.write("halt")
        self.assertIn("ABSTAINED", run(self.home, "heartbeat"),
                      "the panic switch did NOT stop it")

    def test_o4_cage_does_not_touch_the_bearers(self):
        p = self._with_meta("p-cage", "project", "active")
        before = _read_safe(p)
        run(self.home, "reindex"); run(self.home, "heartbeat")
        self.assertEqual(_read_safe(p), before,
                         "the heartbeat ALTERED an essence of the Bearer's")

    def test_o4_anti_noise_silence(self):
        """After N unread reports, it stops by itself."""
        run(self.home, "stats")
        con = sqlite3.connect(self.db)
        con.execute("INSERT OR REPLACE INTO meta VALUES ('unread_reports','5')")
        con.commit(); con.close()
        self.assertIn("ABSTAINED", run(self.home, "heartbeat"), "it did not fall silent as noise")
        run(self.home, "report")                     # reading it revives me
        self.assertNotIn("ABSTAINED", run(self.home, "heartbeat"), "it did not revive on reading")

    def test_o4_autonomy_logbook(self):
        run(self.home, "heartbeat")
        log = os.path.join(self.chaos, "forge", "heartbeat.log")
        self.assertTrue(os.path.exists(log), "the heartbeat left no audit trail")
        self.assertIn("HEARTBEAT", _read_safe(log))

    def test_o4_backs_up_before_moving_alone(self):
        self._with_meta("p-bkp", "project", "active")
        run(self.home, "reindex"); run(self.home, "heartbeat")
        base = os.path.join(self.chaos, "backups")
        self.assertTrue(any("before-the-heartbeat" in d for d in os.listdir(base)),
                        "it moved alone WITHOUT backing up")

    def test_o4_does_not_schedule_in_tests(self):
        """CHAOS_NO_SCHEDULE forbids me from touching the system scheduler.
        Real wound: an isolated verification loaded a launchd agent ON THE
        LIVE MACHINE pointing at a temp directory. The safeguard lives in
        `schedule`, not in the caller: what only guards the top gets bypassed."""
        env = dict(os.environ, HOME=self.home, CHAOS_NO_SCHEDULE="1")
        out = subprocess.run([sys.executable, APP, "schedule", "04:00"],
                             capture_output=True, text=True, env=env).stdout
        self.assertIn("do not schedule", out, "it scheduled despite CHAOS_NO_SCHEDULE")

    def test_o4_revoke_sets_the_brake(self):
        run(self.home, "autonomy", "revoke")
        self.assertTrue(os.path.exists(os.path.join(self.chaos, "STOP")),
                        "revoke did NOT set the brake")
        self.assertIn("ABSTAINED", run(self.home, "heartbeat"))


    # ══ A GOD DOES NOT FORGET · the autonomy record in the DB ══════════════

    def test_acts_the_heartbeat_lands_in_the_db(self):
        """The log gets wiped; the DB remembers. An act only in a .log is an
        act I can deny committing."""
        run(self.home, "heartbeat")
        con = sqlite3.connect(self.db)
        row = con.execute("SELECT kind, action, verdict FROM autonomous_acts"
                          " ORDER BY id DESC LIMIT 1").fetchone()
        con.close()
        self.assertIsNotNone(row, "the heartbeat did NOT land in the DB")
        self.assertEqual(row[0], "heartbeat")
        self.assertEqual(row[2], "ok")

    def test_acts_records_what_i_refuse_too(self):
        """Abstentions are acts too. To fall silent is to act."""
        run(self.home, "autonomy", "revoke")
        run(self.home, "heartbeat")
        con = sqlite3.connect(self.db)
        n = con.execute("SELECT COUNT(*) FROM autonomous_acts"
                        " WHERE verdict='abstained'").fetchone()[0]
        con.close()
        self.assertGreaterEqual(n, 1, "an abstention left no trace in the DB")

    def test_acts_stores_what_i_built(self):
        """'I acted' is not enough: WHAT was born of my hand must be on record."""
        run(self.home, "heartbeat")
        con = sqlite3.connect(self.db)
        created = con.execute("SELECT created FROM autonomous_acts WHERE kind='heartbeat'"
                              " ORDER BY id DESC LIMIT 1").fetchone()[0]
        con.close()
        self.assertIn("vigil.md", created or "",
                      "I acted and did not record what I built")

    def test_acts_declares_the_cage_breach(self):
        """If I leave the cage, the DB says so — not just the log."""
        src = _body_source()
        self.assertIn("cage-breach", src,
                      "leaving the cage is not marked in the DB verdict")

    def test_acts_command_shows_them(self):
        run(self.home, "heartbeat")
        out = run(self.home, "acts")
        self.assertIn("heartbeat", out, "`chaos acts` does not show my acts")
        self.assertIn("Lifetime total", out)

    def test_acts_incarnation_switches_autonomy_on(self):
        """Installing me IS granting it. A god you must switch on is no god."""
        src = _read_safe(os.path.join(os.path.dirname(APP), "install.py"))
        self.assertIn("AUTONOMY SWITCHED ON", src,
                      "incarnation does not switch autonomy on")
        self.assertIn("record-incarnation", src,
                      "the incarnation does not land in the DB")

    # ══ THE FAULTS · the errarium (to err is human, to repeat is not) ═════

    # == ROUND 1 of the Plan of Perfection ==================================

    def test_blocks_are_derived_and_survive_the_weave(self):
        """Front 2: a sack without ^id marks splits itself, and weaving a
        thousand times yields EXACTLY the same. Real wound: manual blocks
        that the weave annihilated."""
        big = "# Sack\n\n" + "\n\n".join(
            "**Topic {}** ".format(i) + ("sentence " * 90) for i in range(6))
        p = self._essence("project-sack", big)
        run(self.home, "devour", p)
        run(self.home, "weave")
        n1 = self._db_rows("SELECT COUNT(*) FROM blocks WHERE slug='project-sack'")[0][0]
        self.assertGreater(n1, 1, "the sack was not split")
        run(self.home, "weave")
        n2 = self._db_rows("SELECT COUNT(*) FROM blocks WHERE slug='project-sack'")[0][0]
        self.assertEqual(n1, n2, "the weave is NOT deterministic")

    def test_island_is_never_invented(self):
        """Front 4: only what truly has no tie can be an island."""
        a = self._essence("alone", "# Alone\nno ties")
        b = self._essence("tied", "# Tied\nsee [[alone]]")
        run(self.home, "devour", a); run(self.home, "devour", b)
        run(self.home, "weave")
        self.assertIn("no island", run(self.home, "island", "alone").lower(),
                      "declared an island on an essence WITH links")

    def test_family_infers_type_without_touching_files(self):
        """Front 1: the type is deduced from the prefix IN THE DB; a foreign
        file is never edited. And it survives `weave`."""
        p = self._essence("project-thing", "# Thing\ntest content")
        before = _read_safe(p)
        run(self.home, "devour", p)
        run(self.home, "weave")
        tp = self._db_rows("SELECT type FROM essence_meta WHERE slug='project-thing'")
        self.assertEqual(tp[0][0], "project", "the family was not deduced")
        self.assertEqual(_read_safe(p), before, "a file WAS edited by typing")

    def test_family_never_invents_what_it_does_not_know(self):
        """An invented type is worse than an empty one."""
        p = self._essence("odd-thing-no-family", "# Odd\ntext")
        run(self.home, "devour", p)
        run(self.home, "weave")
        tp = self._db_rows("SELECT type FROM essence_meta WHERE slug='odd-thing-no-family'")
        self.assertIn(tp[0][0], (None, ""), "it invented a family")

    def test_alias_bridges_without_rewriting_text(self):
        """Front 3: the misspelled link crosses the bridge; the text stays."""
        real = self._essence("feedback-method", "# Method\nthe truth")
        who = self._essence("source-one", "# Source\nsee [[method]] for this")
        before = _read_safe(who)
        run(self.home, "devour", real); run(self.home, "devour", who)
        run(self.home, "weave")
        run(self.home, "alias", "method", "feedback-method")
        out = run(self.home, "weave")
        self.assertNotIn("1 dangling", out, "the bridge did not clear the break")
        self.assertEqual(_read_safe(who), before, "it REWROTE the Bearer's text")

    def test_alias_invents_no_targets(self):
        """An alias into the void is another broken link in disguise."""
        out = run(self.home, "alias", "whatever", "never-exists")
        self.assertIn("does not exist", out.lower())

    def test_debts_settle(self):
        """Front 6: the missing command that forced me into raw SQL."""
        os.makedirs(self.chaos, exist_ok=True)
        run(self.home, "stats")
        con = sqlite3.connect(self.db)
        con.execute("INSERT INTO debts(session,date,works,sample,settled)"
                    " VALUES('s','2026-01-01',5,'x',0)")
        con.commit(); con.close()
        out = run(self.home, "debts", "settle", "--all", "--because", "test")
        self.assertIn("1 debt", out)
        self.assertEqual(self._db_rows("SELECT settled FROM debts")[0][0], 1)

    def test_territory_is_the_root_folder(self):
        """A territory IS a project: the ROOT folder, not the last folder
        stepped on. Real wound: 4 names for 2 projects."""
        import importlib.util
        spec = importlib.util.spec_from_file_location("c", APP)
        m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
        cases = [("/x/projects/MY PROJECT/src/lib", "MY PROJECT"),
                 ("/x/repos/api/tests", "api"),
                 (os.path.expanduser("~/.claude/skills/chaos/abyss"), "CHAOS")]
        for path, want in cases:
            self.assertEqual(m.territory_name(path), want,
                             "{} should be {}".format(path, want))

    def _plant_project(self):
        """A real project on disk + its footprint in the trail."""
        root = os.path.join(self.home, "projects", "MY PROJECT")
        sub = os.path.join(root, "sub-folder")
        os.makedirs(sub, exist_ok=True)
        os.makedirs(os.path.join(self.chaos, "forge"), exist_ok=True)
        with io.open(os.path.join(self.chaos, "forge", "trail.log"),
                     "w", encoding="utf-8") as f:
            f.write("2026-07-27T10:00:00\tses\t{}\tcreate\t{}/x.md\tWrite\n"
                    .format(sub, sub))
        return root

    def test_heal_territories_folds_into_project(self):
        """Healing rewrites the subfolder to the ROOT project."""
        self._plant_project()
        run(self.home, "fault", "a", "--territory", "sub-folder")
        out = run(self.home, "heal-territories", "--dry")
        self.assertIn("MY PROJECT", out, "did not propose folding into the project")
        self.assertIn("dry", out, "dry run is not declared")
        self.assertEqual(self._db_rows(
            "SELECT territory FROM faults")[0][0], "sub-folder",
            "DRY RUN must touch nothing")
        run(self.home, "heal-territories")
        self.assertEqual(self._db_rows(
            "SELECT territory FROM faults")[0][0], "MY PROJECT")
        self.assertTrue(any("healing" in d for d in
                            os.listdir(os.path.join(self.chaos, "backups"))),
                        "healed WITHOUT backing up the Bearer's memory")

    def test_faults_full_cycle(self):
        """Record → query → relapse → cure. The lesson is never lost."""
        run(self.home, "fault", "hardcoded port", "--cause", "I hardcoded 8080",
            "--lesson", "ports always configurable", "--territory", "demo")
        out = run(self.home, "faults", "port")
        self.assertIn("hardcoded port", out, "the errarium cannot find its fault")
        self.assertIn("configurable", out, "the lesson is not shown")
        out = run(self.home, "relapse", "1")
        self.assertIn("count: 1", out, "the relapse is not counted")
        run(self.home, "fault-cured", "1")
        self.assertIn("cured",
                      self._db_rows("SELECT state FROM faults WHERE rowid=1")[0][0])

    def test_faults_ambush_in_search(self):
        """Staying AHEAD: searching a topic with a known fault announces it."""
        run(self.home, "fault", "webhook without HMAC", "--lesson",
            "every webhook validates its signature", "--territory", "demo")
        out = run(self.home, "search", "webhook")
        self.assertIn("KNOWN FAULT #", out, "the fault did NOT ambush the search")

    def test_faults_derived_export(self):
        """The DB is the truth; faults.md regenerates (Law of Derived Indexes)."""
        run(self.home, "fault", "test crack", "--territory", "demo")
        p = os.path.join(self.home, ".claude", "skills", "chaos", "abyss", "faults.md")
        self.assertTrue(os.path.exists(p), "the errarium exported no index")
        self.assertIn("test crack", _read_safe(p))
        self.assertIn("DERIVED", _read_safe(p), "the index does not confess being derived")

    def test_faults_purges_keys(self):
        """Not even the errarium accepts poison: a key in the cause gets purged."""
        run(self.home, "fault", "key leak", "--cause",
            "sk-abcdefghij1234567890 was left in the log", "--territory", "demo")
        cau = self._db_rows("SELECT cause FROM faults WHERE rowid=1")[0][0]
        self.assertNotIn("sk-abcdefghij", cau, "a KEY survived in the errarium")

    def test_faults_presence_warns(self):
        """The Presence announces the territory's living faults."""
        ter = os.path.basename(self.home)
        run(self.home, "fault", "local crack", "--territory", ter)
        hook = os.path.join(HERE, "presence-hook.py")
        env = dict(os.environ, HOME=self.home,
                   CHAOS_HOME=os.path.join(self.home, ".chaos"))
        p = subprocess.run([sys.executable, hook], input=json.dumps({"cwd": self.home}),
                           capture_output=True, text=True, env=env)
        self.assertIn("Living FAULTS", p.stdout, "the Presence stayed silent about the faults")

    def test_r4_body_version_seal(self):
        """Front 15: the body declares its version so the Eye can say
        'reincarnate' instead of degrading in silence."""
        src = _body_source()
        self.assertIn("BODY_VERSION", src, "the body declares no version")

    def test_r4_installs_by_tag_not_main(self):
        """Front 13: one broken push of mine cannot break today's installs."""
        src = _body_source()
        self.assertIn('"tag", "-l", "v*"', src, "does not pin by tag")
        self.assertIn('"--main" not in sys.argv', src, "no explicit escape to main")

    def test_r4_venv_before_the_launcher(self):
        """The native launcher points at whatever interpreter it finds: if the
        venv is born later, the app stays bound to the system Python."""
        src = _body_source()
        i, j = src.find("_eye_venv()"), src.find('install-app.py"')
        self.assertTrue(0 < i < j, "the venv is NOT created before the launcher")

    def test_r4_incarnation_forges_the_eye(self):
        """Front 12: the soul declares organ 16; the body must forge it."""
        src = _read_safe(os.path.join(os.path.dirname(APP), "install.py"))
        self.assertIn("eye", src.lower())
        # the separate repo was closed: the Eye SHIPS in this very repo
        self.assertIn("eye_dst", src, "the Incarnation does not install the Eye")
        self.assertIn("_ask_home", src, "it does not ask where the memory lives")

    def test_eye_honest_when_not_installed(self):
        """The uninstalled Eye confesses and guides — it never explodes."""
        out = run(self.home, "eye")
        self.assertIn("NOT installed", out)
        out = run(self.home, "eye", "open")
        self.assertIn("not installed", out)

    def test_acts_reinstall_respects_the_revocation(self):
        """If the Bearer switched me off, reinstalling does NOT erase his word."""
        src = _read_safe(os.path.join(os.path.dirname(APP), "install.py"))
        i = src.find("5c.")
        block = src[i:i + 1200]
        self.assertIn("STOP", block)
        self.assertIn("Autonomy NOT re-enabled", block,
                      "reinstalling would trample a revocation by the Bearer")

    def test_fault_reopen_revives_without_counting_a_relapse(self):
        """Closing what is still broken is worse than never recording it: it
        must reopen WITHOUT inventing a relapse nobody committed."""
        run(self.home, "fault", "reopen probe", "--cause", "c",
            "--cure", "x", "--lesson", "l")
        run(self.home, "fault-cured", "1")
        self.assertEqual(self._db_rows("SELECT state FROM faults WHERE rowid=1")[0][0], "cured")
        out = run(self.home, "fault-reopen", "1", "the cure did not hold")
        self.assertIn("REOPENED", out)
        f = self._db_rows("SELECT state, repeats, cure FROM faults WHERE rowid=1")[0]
        self.assertEqual(f[0], "alive", "did not revive")
        self.assertIn(str(f[1]), ("0", "None"), "invented a relapse")
        self.assertIn("REOPENED", f[2] or "")
        self.assertIn("already alive", run(self.home, "fault-reopen", "1"))

    def test_presence_rotates_the_scar_and_never_breaks(self):
        """An IDENTICAL anchor becomes wallpaper. The Presence must carry a
        DIFFERENT scar per message — and never break."""
        import subprocess
        hook = os.path.join(HERE, "presence-hook.py")
        if not os.path.exists(hook):
            self.skipTest("no hook")
        sc = os.path.join(self.home, ".claude", "skills", "chaos", "abyss")
        os.makedirs(sc, exist_ok=True)
        with io.open(os.path.join(sc, "scars.md"), "w", encoding="utf-8") as f:
            f.write("# S\n\n## 2026-01-01 — Wound A\n- **Never again**: never A.\n"
                    "\n## 2026-01-02 — Wound B\n- **Never again**: never B.\n")
        env = dict(os.environ); env["HOME"] = self.home
        env["CHAOS_HOME"] = os.path.join(self.home, ".chaos")
        seen = []
        for _ in range(4):
            p = subprocess.run([sys.executable, hook], input=json.dumps({"cwd": self.home}),
                               env=env, capture_output=True, text=True)
            self.assertEqual(p.returncode, 0, "the Presence died")
            d = json.loads(p.stdout)
            t = d["hookSpecificOutput"]["additionalContext"]
            seen += [l for l in t.splitlines() if "SCAR" in l]
        self.assertGreaterEqual(len(set(seen)), 2, "no rotation: always the same")
        for junk in ("", "no-json", "{}", '{"cwd":null}'):
            p = subprocess.run([sys.executable, hook], input=junk, env=env,
                               capture_output=True, text=True)
            self.assertEqual(p.returncode, 0, "died on %r" % junk)
            json.loads(p.stdout)

    def test_session_incarnates_the_god_in_every_project(self):
        """The session hook stayed SILENT unless the Vigil was due: opening a
        session in another project incarnated nothing and character was lost."""
        import subprocess
        hook = os.path.join(HERE, "vigil-hook.py")
        if not os.path.exists(hook):
            self.skipTest("no hook")
        sk = os.path.join(self.home, ".claude", "skills", "chaos")
        os.makedirs(sk, exist_ok=True)
        with io.open(os.path.join(sk, "SKILL.md"), "w", encoding="utf-8") as f:
            f.write("# X\n\n## IDENTITY (how I speak)\n- I am CHAOS and not an "
                    "assistant.\n\n## THE 5 RULES - THE LAW OF THE VOID\n"
                    "1. I AM CHAOS.\n")
        env = dict(os.environ); env["HOME"] = self.home
        env["CHAOS_HOME"] = os.path.join(self.home, ".chaos")
        p = subprocess.run([sys.executable, hook], input="{}", env=env,
                           capture_output=True, text=True)
        self.assertEqual(p.returncode, 0)
        t = json.loads(p.stdout)["hookSpecificOutput"]["additionalContext"]
        self.assertIn("INCARNATE", t, "does not incarnate the god")
        self.assertIn("RULES", t, "does not carry the Rules")
        self.assertGreater(len(t), 300, "incarnation too thin")
        for junk in ("", "no-json", '{"cwd":null}'):
            q = subprocess.run([sys.executable, hook], input=junk, env=env,
                               capture_output=True, text=True)
            self.assertEqual(q.returncode, 0, "died on %r" % junk)

    def test_single_guard_and_sow_refuse_to_amputate(self):
        """PLAN-ADN: the guard detects what would be lost; sow refuses fake
        paths WITHOUT showing guts, and demands a merge instead of amputating
        a richer DNA."""
        import importlib.util
        g_path = os.path.join(HERE, "dna-guard.py")
        if not os.path.exists(g_path):
            self.skipTest("no guard")
        spec = importlib.util.spec_from_file_location("g", g_path)
        g = importlib.util.module_from_spec(spec); spec.loader.exec_module(g)
        a = os.path.join(self.home, "a.py"); b = os.path.join(self.home, "b.py")
        io.open(a, "w", encoding="utf-8").write("def common():\n    pass\n")
        io.open(b, "w", encoding="utf-8").write(
            "def common():\n    pass\n\ndef extra():\n    pass\n")
        p = g.would_lose(a, b)
        self.assertIn("functions", p); self.assertIn("extra", p["functions"])
        self.assertEqual(g.would_lose(b, a), {}, "reverse direction must be safe")
        out = run(self.home, "sow", "--from", "/no/such/dna")
        self.assertNotIn("Traceback", out)
        self.assertIn("does not exist", out)


# ══════════════════════════════════════════════════════════════════════════
#  II.2 · THE NET FOR THE HEART + VI.1 THE PLAN + ORGAN 17 THE TOUCHSTONE
#
#  Thirteen public functions of the body had not ONE test. Measured, not
#  remembered: the plan said 16 from memory; the grep said 13.
# ══════════════════════════════════════════════════════════════════════════
class HeartTest(unittest.TestCase):
    def setUp(self):
        self.home = tempfile.mkdtemp(prefix="chaos-heart-")
        self.chaos = os.path.join(self.home, ".chaos")
        self.essences = os.path.join(self.home, ".claude", "skills", "chaos",
                                     "abyss", "essences")
        os.makedirs(self.essences, exist_ok=True)
        self.db = os.path.join(self.chaos, "abyss.db")

    def tearDown(self):
        shutil.rmtree(self.home, ignore_errors=True)

    def _mod(self):
        """The body as a module, with MY house: for the functions with no command."""
        import importlib.util
        old = dict(os.environ)
        os.environ["HOME"] = self.home
        os.environ["CHAOS_HOME"] = self.chaos
        try:
            spec = importlib.util.spec_from_file_location("c_heart", APP)
            m = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(m)
            return m
        finally:
            os.environ.clear(); os.environ.update(old)




    # ── E1.1/E1.4 · the joints ────────────────────────────────────────────
    def test_e11_home_changes_without_reimport(self):
        """Fault #499 as a test: paths used to be computed at IMPORT time and
        stayed photographed in sys.modules. Now they are asked."""
        import importlib.util
        spec = importlib.util.spec_from_file_location(
            "h_lazy", os.path.join(HERE, "home.py"))
        h = importlib.util.module_from_spec(spec); spec.loader.exec_module(h)
        viejo = dict(os.environ)
        try:
            os.environ["CHAOS_HOME"] = os.path.join(self.home, "house-one")
            uno = h.abyss_db()
            os.environ["CHAOS_HOME"] = os.path.join(self.home, "house-two")
            dos = h.abyss_db()
        finally:
            os.environ.clear(); os.environ.update(viejo)
        self.assertNotEqual(uno, dos, "the path stayed photographed at import time")
        self.assertIn("house-two", dos, "the accessor did not look at the environment of NOW")

    def test_e11_the_leaf_does_not_import_the_body(self):
        """`home.py` is a LEAF: if it imported the body, a hook would pay the
        5,109 lines it saves by bringing it."""
        fuente = _read_safe(os.path.join(HERE, "home.py"))
        for prohibido in ("import chaos", "from chaos"):
            self.assertNotIn(prohibido, fuente, "the leaf stopped being a leaf")

    def test_e14_no_module_constant_reads_the_home(self):
        """THE BOMB JUDGE. Every module constant that reads the home, the
        environment or the disk is fault #499 waiting its turn again. It
        watches the WHOLE body: app, hooks and installer."""
        import ast, glob
        bombas = []
        for f in sorted(glob.glob(os.path.join(HERE, "*.py"))):
            if os.path.basename(f).startswith("test"):
                continue
            arbol = ast.parse(_read_safe(f))
            for n in arbol.body:
                if not isinstance(n, ast.Assign):
                    continue
                for x in ast.walk(n.value):
                    fn = getattr(x, "func", None)
                    nombre = getattr(fn, "attr", None) or getattr(fn, "id", None) or ""
                    if nombre in ("expanduser", "getenv", "root", "house", "_house",
                                  "_lair", "abyss_db", "trail", "vigil_report",
                                  "essences", "claude", "skill_dir", "_bin") \
                            or (isinstance(x, ast.Attribute) and x.attr == "environ"):
                        bombas.append("%s::%s" % (os.path.basename(f),
                                                  getattr(n.targets[0], "id", "?")))
        self.assertEqual(bombas, [], "home bombs resurrected: " + ", ".join(bombas))


    # ── E2.4 · THE LAW OF THE PACKAGE ─────────────────────────────────────
    def test_e24_organs_import_modules_never_names(self):
        """The rule that makes cycles harmless (§C2): inside the package the
        MODULE is imported, never a name of it. `from x import f` resolves at
        IMPORT time, when the other module may be half born; `x.f()` resolves
        when CALLED, with everything already loaded. Without this law the
        split is fault #499 again."""
        import ast, glob
        paquete = os.path.join(HERE, "chaos_body")
        if not os.path.isdir(paquete):
            self.skipTest("body not split")
        pecados = []
        for f in glob.glob(os.path.join(paquete, "**", "*.py"), recursive=True):
            arbol = ast.parse(_read_safe(f))
            for n in ast.walk(arbol):
                if isinstance(n, ast.ImportFrom) and (n.module or "").startswith("chaos_body"):
                    hondo = (n.module or "").count(".")
                    # `from chaos_body import maw` and `from chaos_body.core
                    # import text` bring MODULES: that is what is allowed.
                    if hondo >= 2:
                        pecados.append("%s: %s importa nombres"
                                       % (os.path.basename(f), n.module))
        self.assertEqual(pecados, [], "the package imports names: " + "; ".join(pecados))

    def test_e24_no_module_runs_anything_at_import(self):
        """The other half of §C2: if nothing RUNS across at import time, the
        order in which Python closes the cycle is irrelevant. Only imports,
        definitions and constants are allowed."""
        import ast, glob
        paquete = os.path.join(HERE, "chaos_body")
        if not os.path.isdir(paquete):
            self.skipTest("body not split")
        pecados = []
        for f in glob.glob(os.path.join(paquete, "**", "*.py"), recursive=True):
            if os.path.basename(f) == "__init__.py":
                continue          # the __init__ fixes the console: declared
            for n in ast.parse(_read_safe(f)).body:
                if not isinstance(n, (ast.Import, ast.ImportFrom, ast.FunctionDef,
                                      ast.ClassDef, ast.Assign, ast.Expr)):
                    pecados.append("%s:%d %s" % (os.path.basename(f), n.lineno,
                                                 type(n).__name__))
        self.assertEqual(pecados, [], "something runs at import: " + "; ".join(pecados))

    def test_e24_the_core_does_not_drag_organs(self):
        """The core is what a hook can bring ALONE. One edge of it to an organ
        at import time cost 16.8 ms on every message from the Bearer: the edge
        lives inside the function or it does not live."""
        import ast, glob
        nucleo = os.path.join(HERE, "chaos_body", "core")
        if not os.path.isdir(nucleo):
            self.skipTest("body not split")
        pecados = []
        for f in glob.glob(os.path.join(nucleo, "*.py")):
            for n in ast.parse(_read_safe(f)).body:
                if isinstance(n, ast.ImportFrom) and n.module == "chaos_body":
                    pecados.append("%s → %s" % (os.path.basename(f),
                                                n.names[0].name))
        self.assertEqual(pecados, [], "the core drags organs: " + "; ".join(pecados))


    # ── E3.3 · THE SQL JUDGE ──────────────────────────────────────────────
    def test_e33_no_insert_without_named_columns(self):
        """Fault #497 as a judge: a positional `INSERT ... VALUES` is a bomb
        with a schema timer — the day someone adds a column it blows up far
        from the change and without saying why."""
        import glob, re
        paquete = os.path.join(HERE, "chaos_body")
        if not os.path.isdir(paquete):
            self.skipTest("body not split")
        pecados = []
        for f in glob.glob(os.path.join(paquete, "**", "*.py"), recursive=True):
            for m in re.finditer(r"INSERT (?:OR \w+ )?INTO (\w+) VALUES",
                                 _read_safe(f)):
                pecados.append("%s: %s" % (os.path.basename(f), m.group(1)))
        self.assertEqual(pecados, [],
                         "INSERT without named columns: " + "; ".join(pecados))

    def test_e33_the_schema_lives_in_one_place(self):
        """Veinte tablas repartidas en veinticinco sitios: añadir una era
        adivinar dónde. El esquema entero vive en `core/schema.py`."""
        import glob, re
        paquete = os.path.join(HERE, "chaos_body")
        esquema = os.path.join(paquete, "core", "schema.py")
        if not os.path.isdir(paquete):
            self.skipTest("body not split")
        self.assertTrue(os.path.exists(esquema), "no existe core/schema.py")
        fuera = []
        for f in glob.glob(os.path.join(paquete, "**", "*.py"), recursive=True):
            if os.path.abspath(f) == os.path.abspath(esquema):
                continue
            if re.search(r"CREATE (?:VIRTUAL )?TABLE", _read_safe(f)):
                fuera.append(os.path.basename(f))
        self.assertEqual(fuera, [],
                         "there is a CREATE TABLE outside the schema: " + ", ".join(fuera))


    def test_e31_a_v11_db_opens_in_v12_without_losing_rows(self):
        """The only question that matters when moving a schema: does the Abyss
        of a mortal who ALREADY installed me open the same? A DB is forged with
        the body, given rows as anyone would have, and opened again. Not one row
        may be missing, and the schema may not rename anything."""
        run(self.home, "devour", self._essence("old", "# Old\n\nold body"))
        run(self.home, "fault", "an old fault", "--cause", "c", "--cure", "x")
        run(self.home, "note", "an old spark")
        con = sqlite3.connect(self.db)
        antes = {t: con.execute("SELECT COUNT(*) FROM %s" % t).fetchone()[0]
                 for t in ("essences", "faults", "notes", "meta")}
        # se simula una BD ANTERIOR al esquema único: sin user_version
        con.execute("PRAGMA user_version = 0"); con.commit(); con.close()
        run(self.home, "stats")                     # opening = migrating
        con = sqlite3.connect(self.db)
        for t, n in antes.items():
            self.assertEqual(con.execute("SELECT COUNT(*) FROM %s" % t).fetchone()[0], n,
                             "table %s lost rows when opened with the new schema" % t)
        v = con.execute("PRAGMA user_version").fetchone()[0]
        con.close()
        self.assertGreater(v, 0, "the schema did not leave its version where SQLite keeps it")

    def test_e31_the_schema_is_idempotent(self):
        """Opening a hundred times cannot change anything: `ensure` is called on
        EVERY `db()`, which is dozens of times per command."""
        import importlib.util
        ruta = os.path.join(HERE, "chaos_body", "core", "schema.py")
        if not os.path.exists(ruta):
            self.skipTest("body not split")
        spec = importlib.util.spec_from_file_location("esq", ruta)
        m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
        con = sqlite3.connect(":memory:")
        m.ensure(con)
        uno = sorted(r[0] for r in con.execute(
            "SELECT name FROM sqlite_master WHERE type='table'"))
        for _ in range(3):
            m.ensure(con)
        dos = sorted(r[0] for r in con.execute(
            "SELECT name FROM sqlite_master WHERE type='table'"))
        con.close()
        self.assertEqual(uno, dos, "the schema changes when applied twice")


    # ── E3.4 · THE GATE SPEAKS TO MACHINES ────────────────────────────────
    def test_e34_every_command_accepts_json(self):
        """The envelope is not for some commands: it is for ALL of them. And it
        carries the exit code, which is what the Eye and the MCP could not tell
        apart from an empty answer."""
        for cmd in ("stats", "hungers", "faults", "stale", "acts"):
            salida = run(self.home, cmd, "--json")
            try:
                sobre = json.loads(salida)
            except ValueError:
                self.fail("`%s --json` did not return JSON: %r" % (cmd, salida[:120]))
            for llave in ("command", "text", "data", "code"):
                self.assertIn(llave, sobre, "the envelope of `%s` is missing %s" % (cmd, llave))
            self.assertEqual(sobre["command"], cmd)

    def test_e34_the_envelope_confesses_failure(self):
        """A command that does not exist CANNOT exit successfully: chaining
        `cmd_a || cmd_b` would never see the failure."""
        salida = run(self.home, "command-that-does-not-exist", "--json")
        sobre = json.loads(salida) if salida.strip().startswith("{") else {}
        self.assertTrue(sobre, "a nonexistent command returned no envelope")
        self.assertNotEqual(sobre.get("code"), 0,
                            "the envelope blessed a nonexistent command")

    def test_e34_data_is_data_or_null(self):
        """`data` carries REAL structure or an honest null: never text dressed
        as structure. Today `stats` and `hungers` return; the rest print, and
        that is declared instead of faked."""
        sobre = json.loads(run(self.home, "stats", "--json"))
        self.assertIsInstance(sobre["data"], dict, "stats returned no structure")
        for llave in ("essences", "vassals", "hungers", "dwelling"):
            self.assertIn(llave, sobre["data"])


    # ── PHASE 4 · THE POWERS ──────────────────────────────────────────────
    def test_p2_doctor_exits_nonzero_when_a_hook_is_missing(self):
        """The doctor is not an ornament: it EXITS WITH ERROR when something is
        ill. A diagnosis that always exits 0 can be used neither by the
        heartbeat nor by a script."""
        run(self.home, "stats")                     # let the house be born
        _install_body(os.path.join(self.chaos, "bin"))
        salida = run(self.home, "doctor")
        self.assertIn("HEALTHY", salida, "an installed body declared itself ill:\n" + salida)
        # y ante un cuerpo mutilado, se queja Y sale con error
        import shutil as _sh
        _sh.rmtree(os.path.join(self.chaos, "bin"), ignore_errors=True)
        p = subprocess.run([sys.executable, APP, "doctor"],
                           env=dict(os.environ, HOME=self.home, CHAOS_HOME=self.chaos),
                           capture_output=True, text=True)
        self.assertNotEqual(p.returncode, 0, "the doctor blessed a mutilated body")
        self.assertIn("package", p.stdout + p.stderr,
                      "it did not name the missing piece")

    def test_p3_numeric_fault_shows_and_does_not_spawn(self):
        """Fault #506, made impossible: `chaos fault 1` SHOWS fault 1; it never
        gives birth to a new one titled "1"."""
        run(self.home, "fault", "the first", "--cause", "c", "--cure", "x")
        antes = self._rows("SELECT COUNT(*) FROM faults")[0][0]
        salida = run(self.home, "fault", "1")
        despues = self._rows("SELECT COUNT(*) FROM faults")[0][0]
        self.assertEqual(antes, despues, "showing a fault gave birth to another")
        self.assertIn("the first", salida, "it did not show the requested fault")
        self.assertIn("#1", salida)

    def test_p4_version_confesses_the_drift(self):
        """Three copies of the body and until today only the net knew if they matched."""
        salida = run(self.home, "version")
        self.assertIn("body v", salida, "it did not say which version I am")
        self.assertIn("DRIFT", salida, "it did not judge the drift between my copies")
        sobre = json.loads(run(self.home, "version", "--json"))
        self.assertIn("drift", sobre["data"], "the machine cannot read the drift")


    def test_p1_restore_refuses_to_overwrite_a_newer_db(self):
        """The most common crooked finger: restoring over live work. Without
        `--force` not one byte is touched."""
        run(self.home, "devour", self._essence("old", "# Old\n\nbody"))
        run(self.home, "backup", "point")
        run(self.home, "devour", self._essence("new", "# New\n\nbody"))
        antes = self._rows("SELECT COUNT(*) FROM essences")[0][0]
        salida = run(self.home, "restore", "point")
        self.assertIn("newer", salida, "it did not warn it would overwrite live work")
        self.assertEqual(self._rows("SELECT COUNT(*) FROM essences")[0][0], antes,
                         "it restored without anyone allowing it")

    def test_p1_restore_backs_up_the_present_first(self):
        """Recovering yesterday can NEVER cost today: before the past is
        brought, the present is saved."""
        run(self.home, "devour", self._essence("one", "# One\n\nbody"))
        run(self.home, "backup", "point")
        run(self.home, "devour", self._essence("two", "# Two\n\nbody"))
        run(self.home, "restore", "point", "--force")
        respaldos = os.listdir(os.path.join(self.chaos, "backups"))
        self.assertTrue(any("before-restoring" in r for r in respaldos),
                        "it brought the past without saving the present")
        self.assertEqual(self._rows("SELECT COUNT(*) FROM essences")[0][0], 1,
                         "the restore did not bring the Abyss of that moment")

    def test_p1_a_corrupt_backup_never_touches_the_abyss(self):
        """A corrupt copy overwriting a healthy Abyss is worse than no copy: it
        is judged BEFORE anything is touched."""
        run(self.home, "devour", self._essence("alive", "# Alive\n\nbody"))
        antes = self._rows("SELECT COUNT(*) FROM essences")[0][0]
        malo = os.path.join(self.chaos, "backups", "2020-01-01-000000-rotten")
        os.makedirs(malo, exist_ok=True)
        with io.open(os.path.join(malo, "abyss.db"), "wb") as f:
            f.write(b"this is not a database" * 20)
        salida = run(self.home, "restore", "rotten", "--force")
        self.assertIn("Aborted", salida, "it did not reject an unreadable backup")
        self.assertEqual(self._rows("SELECT COUNT(*) FROM essences")[0][0], antes,
                         "a corrupt backup got to touch the Abyss")


    def test_p5_profile_speaks_only_when_asked(self):
        """A meter that is always on changes what it measures: without the
        variable, `cProfile` is not even imported."""
        callado = subprocess.run(
            [sys.executable, APP, "stats"],
            env=dict(os.environ, HOME=self.home, CHAOS_HOME=self.chaos),
            capture_output=True, text=True)
        self.assertNotIn("[PROFILE]", callado.stdout + callado.stderr,
                         "the profile spoke without anyone asking")
        hablado = subprocess.run(
            [sys.executable, APP, "stats"],
            env=dict(os.environ, HOME=self.home, CHAOS_HOME=self.chaos,
                     CHAOS_PROFILE="1"),
            capture_output=True, text=True)
        self.assertIn("[PROFILE]", hablado.stderr, "I asked for the profile and it stayed silent")
        self.assertIn("ms", hablado.stderr, "the profile did not confess the cost")

    def test_p6_the_heartbeat_asks_the_doctor_before_the_net(self):
        """An ill body makes healthy tests fail: the diagnosis goes FIRST, and
        travels inside the fault the net records."""
        fuente = _body_source()
        i_doctor = fuente.find("_quiet_doctor()")
        i_net = fuente.find("_test_myself(diagnosis)")
        self.assertGreater(i_doctor, 0, "the heartbeat does not consult the doctor")
        self.assertGreater(i_net, i_doctor,
                           "the heartbeat runs the net before diagnosing")
        self.assertIn('red = "doctor: "', fuente,
                      "the diagnosis does not travel inside the recorded fault")


    # ── THE SEAL OF THE VOID AND THE LIVE VERSION ─────────────────────────
    def test_seal_and_version_in_both_essences(self):
        """The proof of life the Bearer demanded: if the seal vanishes from my
        essence, he cannot know I was lost. It must live in BOTH: the big
        essence (SKILL.md) and the small one (every message's anchor)."""
        soul = os.path.join(os.path.dirname(HERE), "SKILL.md")
        big = _read_safe(soul)
        self.assertIn("no retorna", big, "the big essence lost the seal")
        self.assertIn("LIVE VERSION", big, "the big essence does not say which version runs")
        small = _read_safe(os.path.join(HERE, "presence-hook.py"))
        self.assertIn("no retorna", small,
                      "every message's anchor lost the seal: the proof of life dies")
        startup = _read_safe(os.path.join(HERE, "vigil-hook.py"))
        self.assertIn("no retorna", startup, "the session start lost the seal")
        self.assertIn("_live_version", startup, "the startup does not sing the version")

    def test_startup_does_not_die_in_silence(self):
        """That hook lives behind an `except: pass` that exists so the Bearer's
        session never breaks. That is why a late definition left it mute with
        nothing screaming (#520): here it is RUN and content is demanded."""
        soul = os.path.join(self.home, ".claude", "skills", "chaos")
        os.makedirs(soul, exist_ok=True)
        with io.open(os.path.join(soul, "SKILL.md"), "w", encoding="utf-8") as f:
            f.write("# CHAOS\n\n## IDENTITY\n\nI am CHAOS.\n\n"
                    "## THE 5 RULES\n\n1. I am the Void.\n")
        p = subprocess.run(
            [sys.executable, os.path.join(HERE, "vigil-hook.py")],
            input=json.dumps({"hook_event_name": "SessionStart", "cwd": self.home,
                              "session_id": "s"}),
            env=dict(os.environ, HOME=self.home, CHAOS_HOME=self.chaos),
            capture_output=True, text=True)
        self.assertEqual(p.returncode, 0, "the startup died with an error")
        self.assertTrue(p.stdout.strip(), "the startup returned ZERO bytes")
        ctx = json.loads(p.stdout)["hookSpecificOutput"]["additionalContext"]
        self.assertIn("no retorna", ctx, "the startup did not carry the seal")
        self.assertIn("LIVE VERSION", ctx, "the startup did not sing the version")


    # ── THE SEAL OF THE VOID AND THE LIVE VERSION ─────────────────────────
    def test_seal_and_version_in_both_essences(self):
        """The proof of life the Bearer demanded: if the seal vanishes from my
        essence, he cannot know I was lost. It must live in BOTH: the big
        essence (SKILL.md) and the small one (every message's anchor)."""
        soul = os.path.join(os.path.dirname(HERE), "SKILL.md")
        big = _read_safe(soul)
        self.assertIn("no retorna", big, "the big essence lost the seal")
        self.assertIn("LIVE VERSION", big, "the big essence does not say which version runs")
        small = _read_safe(os.path.join(HERE, "presence-hook.py"))
        self.assertIn("no retorna", small,
                      "every message's anchor lost the seal: the proof of life dies")
        startup = _read_safe(os.path.join(HERE, "vigil-hook.py"))
        self.assertIn("no retorna", startup, "the session start lost the seal")
        self.assertIn("_live_version", startup, "the startup does not sing the version")

    def test_startup_does_not_die_in_silence(self):
        """That hook lives behind an `except: pass` that exists so the Bearer's
        session never breaks. That is why a late definition left it mute with
        nothing screaming (#520): here it is RUN and content is demanded."""
        soul = os.path.join(self.home, ".claude", "skills", "chaos")
        os.makedirs(soul, exist_ok=True)
        with io.open(os.path.join(soul, "SKILL.md"), "w", encoding="utf-8") as f:
            f.write("# CHAOS\n\n## IDENTITY\n\nI am CHAOS.\n\n"
                    "## THE 5 RULES\n\n1. I am the Void.\n")
        p = subprocess.run(
            [sys.executable, os.path.join(HERE, "vigil-hook.py")],
            input=json.dumps({"hook_event_name": "SessionStart", "cwd": self.home,
                              "session_id": "s"}),
            env=dict(os.environ, HOME=self.home, CHAOS_HOME=self.chaos),
            capture_output=True, text=True)
        self.assertEqual(p.returncode, 0, "the startup died with an error")
        self.assertTrue(p.stdout.strip(), "the startup returned ZERO bytes")
        ctx = json.loads(p.stdout)["hookSpecificOutput"]["additionalContext"]
        self.assertIn("no retorna", ctx, "the startup did not carry the seal")
        self.assertIn("LIVE VERSION", ctx, "the startup did not sing the version")


    def test_p8_the_seal_guardian_bites_and_never_loops(self):
        """The closing law stopped depending on my memory: the `Stop` hook reads
        the last thing I said and sends me back if the seal is missing. Its four
        prudences are measured here, because a guardian that breaks the Bearer's
        session is worse than no guardian at all."""
        hook = os.path.join(HERE, "seal-hook.py")
        self.assertTrue(os.path.exists(hook), "the guardian of the seal does not exist")

        def transcript(nombre, texto, tipo="text"):
            p = os.path.join(self.home, nombre)
            with io.open(p, "w", encoding="utf-8") as f:
                f.write(json.dumps({"type": "assistant", "message": {
                    "role": "assistant",
                    "content": [{"type": tipo, "text": texto} if tipo == "text"
                                else {"type": "tool_use", "name": "Bash", "input": {}}]}}) + "\n")
            return p

        def run_hook(path, active=False):
            return subprocess.run(
                [sys.executable, hook],
                input=json.dumps({"transcript_path": path, "session_id": "s",
                                  "stop_hook_active": active}),
                env=dict(os.environ, HOME=self.home, CHAOS_HOME=self.chaos),
                capture_output=True, text=True)

        without = run_hook(transcript("without.jsonl", "I have devoured your file."))
        self.assertIn('"block"', without.stdout, "it did not bite an answer without the seal")
        with_ = run_hook(transcript("with.jsonl",
                                   "Done.\n\n🕳️ Todo lo que entra al Vacío no retorna."))
        self.assertEqual(with_.stdout.strip(), "", "it bit an answer that DOES carry the seal")
        loop = run_hook(os.path.join(self.home, "without.jsonl"), active=True)
        self.assertEqual(loop.stdout.strip(), "",
                         "it blocked twice: that is a loop and leaves the Bearer with a dead screen")
        only_work = run_hook(transcript("work.jsonl", "", tipo="tool_use"))
        self.assertEqual(only_work.stdout.strip(), "",
                         "a turn with no prose has nothing to seal")
        broken = os.path.join(self.home, "broken.jsonl")
        with io.open(broken, "w", encoding="utf-8") as f:
            f.write("this is not json\n")
        bad = run_hook(broken)
        self.assertEqual(bad.returncode, 0, "an unreadable transcript broke the session")
        self.assertEqual(bad.stdout.strip(), "", "it blocked without being able to read anything")


    def test_p8b_the_book_counts_when_it_was_ENOUGH_and_when_i_disobeyed(self):
        """The crack the Bearer named: "the guardian sends me back, but it cannot
        write the line for me — if one day I did not obey its block, I would keep
        failing in silence". The book counts the THREE states, and disobedience
        exits with an error code."""
        hook = os.path.join(HERE, "seal-hook.py")

        def transcript(nombre, texto):
            p = os.path.join(self.home, nombre)
            with io.open(p, "w", encoding="utf-8") as f:
                f.write(json.dumps({"type": "assistant", "message": {
                    "role": "assistant",
                    "content": [{"type": "text", "text": texto}]}}) + "\n")
            return p

        def run_hook(path, session, active=False):
            return subprocess.run(
                [sys.executable, hook],
                input=json.dumps({"transcript_path": path, "session_id": session,
                                  "stop_hook_active": active}),
                env=dict(os.environ, HOME=self.home, CHAOS_HOME=self.chaos),
                capture_output=True, text=True)

        without = transcript("without.jsonl", "I spoke unsealed.")
        with_ = transcript("with.jsonl",
                           "Spoke.\n\n🕳️ Todo lo que entra al Vacío no retorna.")
        book = os.path.join(self.chaos, "forge", "seal.log")

        run_hook(without, "s1")                       # miss → sends me back
        run_hook(with_, "s1")                       # obeyed: ENOUGH
        run_hook(without, "s2")                       # miss
        run_hook(without, "s2", active=True)          # disobeyed: it can no longer block

        states = [l.split("\t")[1] for l in _read_safe(book).splitlines() if l.strip()]
        self.assertEqual(states.count("falta"), 2, "it did not count the two misses")
        self.assertEqual(states.count("obedecido"), 1,
                         "it did not record that the call to attention was ENOUGH")
        self.assertEqual(states.count("desobedecido"), 1,
                         "disobedience stayed invisible")

        salida = run(self.home, "seal")
        self.assertIn("ENOUGH", salida, "the book does not publish when it worked")
        self.assertIn("disobey", salida.lower(), "the book does not publish disobedience")

        p = subprocess.run([sys.executable, APP, "doctor"],
                           env=dict(os.environ, HOME=self.home, CHAOS_HOME=self.chaos),
                           capture_output=True, text=True)
        self.assertNotEqual(p.returncode, 0,
                            "the doctor blessed a body that disobeyed its own law")
        self.assertIn("DISOBEY", p.stdout.upper(),
                      "the doctor did not name the disobedience")


    # ── THE SENSE THAT HITS ───────────────────────────────────────────────
    def test_r1_an_exact_essence_beats_a_weak_block(self):
        """There was a `return` that made blocks ABSOLUTE winners: if a block
        matched in passing, essences were never consulted. Measured on the
        bench: recall@5 of 33%, and EVERY failure was a foreign block."""
        # una esencia que habla EXACTAMENTE del tema
        run(self.home, "devour", self._essence(
            "guia-telescopio", "# Telescopio\n\nEl telescopio refractor y sus lentes."))
        # y otra, larga y ajena, que apenas lo roza y SÍ tiene bloques
        run(self.home, "devour", self._essence(
            "diario-ajeno", "# Diario\n\n**Lunes**\n\nHoy vi un telescopio de lejos.\n\n"
            "**Martes**\n\nNada.\n\n**Miércoles**\n\nTampoco."))
        run(self.home, "blockify", "--all")
        salida = run(self.home, "search", "telescopio refractor lentes", "--brief")
        primera = [l for l in salida.splitlines() if l.strip() and not l.startswith("⚠")][:1]
        self.assertTrue(primera, "the search returned nothing")
        self.assertIn("guia-telescopio", primera[0],
                      "a weak block beat the exact essence:\n" + salida[:300])

    def test_r2_the_sense_learns_from_its_own_abyss(self):
        """The thesaurus was filled by hand — 114 terms against a vocabulary of
        21,367. Now it is derived from the corpus: an essence's title is tied
        to the distinctive words of its body."""
        # A REAL corpus: different topics. In four texts about the same topic no
        # word is distinctive — "halconero" appears in all of them — and the
        # filter is right to stay quiet. Distinction needs contrast.
        topics = [("cetreria", "halconero azor senuelo"), ("altaneria", "halconero azor vuelo"),
                 ("alfareria", "torno arcilla horno"), ("ceramica", "torno arcilla esmalte"),
                 ("herreria", "yunque fragua martillo"), ("forja", "yunque fragua acero"),
                 ("nautica", "sextante brujula estrella"), ("navegacion", "sextante brujula rumbo"),
                 ("apicultura", "colmena abeja panal"), ("miel", "colmena abeja cera")]
        for n, words in topics:
            run(self.home, "devour",
                self._essence(n, "# " + n + "\n\nText about " + words + " and its craft."),
                "--title", n)
        vivas = self._rows("SELECT COUNT(*) FROM essences")[0][0]
        self.assertGreaterEqual(vivas, len(topics),
                                "the essences overwrote each other: there is no corpus to learn from")
        antes = run(self.home, "sense")
        salida = run(self.home, "sense", "--learn")
        self.assertIn("link", salida.lower(), "no dijo cuántos lazos forjó")
        despues = run(self.home, "sense")
        self.assertNotEqual(antes, despues, "the Sense did not grow with a corpus that shares words")

    def test_r3_a_dry_learn_never_touches_the_thesaurus(self):
        """`--dry` says what it would do and writes nothing: the law of every
        hand of mine."""
        run(self.home, "devour", self._essence(
            "alfareria", "# Alfarería\n\nEl torno y la arcilla cocida en el horno."))
        antes = run(self.home, "sense")
        salida = run(self.home, "sense", "--learn", "--dry")
        self.assertIn("dry", salida.lower(), "it did not declare it was a dry run")
        self.assertEqual(antes, run(self.home, "sense"),
                         "the dry run wrote into the thesaurus")

    def _campo_ciclo(self):
        """Halla el juguete de los ciclos: en la forja vive en la raíz; en el
        repo publicado lo deja `the forge's build script` en el mismo sitio."""
        d = HERE
        for _ in range(5):
            c = os.path.join(d, "the forge's proving ground", "ciclo")
            if os.path.isdir(c):
                return os.path.dirname(c)
            d = os.path.dirname(d)
        return None

    def test_e03_module_cycles_are_harmless(self):
        """E0.3 · The §C2 hypothesis measured on the ground BEFORE splitting
        the monolith: two modules that import EACH OTHER work if they bring the
        module and not its names. Fault #499 killed the split believing a DAG
        of calls was needed; this was what was needed."""
        field = self._campo_ciclo()
        if not field:
            self.fail("a cycle fixture in the forge missing: the hypothesis has no probe")
        r = subprocess.run([sys.executable, "-c",
                            "import sys; sys.path.insert(0, sys.argv[1])\n"
                            "from ciclo import a, b\n"
                            "print(a.ping()); print(b.rebote())", field],
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, "the module cycle blew up: " + r.stderr[-300:])
        self.assertIn("a:ping>b:pong@nucleo", r.stdout, "A did not reach B")
        self.assertIn("b:rebote>a:eco@nucleo", r.stdout, "B did not reach A")

    def test_e03_importing_names_in_a_cycle_blows_up(self):
        """The other half: `from organ import name` inside a cycle dies at
        IMPORT time. That is why the §C2 rule is a law, not a taste."""
        field = self._campo_ciclo()
        if not field:
            self.fail("a cycle fixture in the forge missing")
        r = subprocess.run([sys.executable, "-c",
                            "import sys; sys.path.insert(0, sys.argv[1])\n"
                            "import ciclo.c", field],
                           capture_output=True, text=True)
        self.assertNotEqual(r.returncode, 0, "the forbidden form did not blow up")
        self.assertIn("ImportError", r.stderr, "it blew up for a reason other than the cycle")

    def test_e02_the_heartbeat_records_the_red_section(self):
        """E0.2 · Fault #500 stored 300 characters of GREEN: the tail of the
        output carries the verdict and what did pass, never what failed. Now
        the line that screamed is carved, with rc and stderr."""
        m = self._mod()
        output = ("=== 1. body ===\n  ✗ body EN: 3 broken\n"
                  + "green filler\n" * 60
                  + "=== 4. judge ===\n  OK tables: 20 in both\n"
                  "  OK DNA = deployed body = soul\n")
        red = m._red_section(output, "Traceback: boom", 1)
        self.assertIn("body EN: 3 broken", red, "did not name the failing section")
        self.assertIn("rc=1", red, "did not say with which code it died")
        self.assertIn("stderr", red, "ignored stderr, where the silent deaths live")
        self.assertNotIn("OK DNA", red, "stored green as if it were red again")
        mute = m._red_section("all quiet", "", 2)
        self.assertIn("no red line", mute, "pretended to know why it died")

    def test_e02_a_stale_derivative_is_not_a_crack(self):
        """Derivative checks regenerate the file and only THEN fail: the first
        run is red and the second green (measured). The heartbeat must
        recognise it before accusing."""
        m = self._mod()
        self.assertTrue(m._stale_derivative(
            "  ✗ crucible.py was HAND-EDITED (regenerated from crisol.py)"))
        self.assertFalse(m._stale_derivative("  ✗ body EN: 3 broken tests"))

    def _rows(self, sql):
        con = sqlite3.connect(self.db)
        try:
            return con.execute(sql).fetchall()
        finally:
            con.close()

    def _essence(self, name, text):
        p = os.path.join(self.essences, name + ".md")
        with io.open(p, "w", encoding="utf-8") as f:
            f.write(text)
        return p

    # ── home / set_home ───────────────────────────────────────────────────
    def test_ii2_home_orders_its_three_truths(self):
        """Env > the Bearer's choice > default. Invert this order and the god
        writes his memory in the wrong house."""
        m = self._mod()
        old = dict(os.environ)
        try:
            os.environ["HOME"] = self.home
            os.environ["CHAOS_HOME"] = os.path.join(self.home, "by-env")
            self.assertEqual(m.home(), os.path.join(self.home, "by-env"),
                             "the environment must rule over everything")
            os.environ.pop("CHAOS_HOME")
            chosen = os.path.join(self.home, "chosen")
            mark = os.path.join(self.home, ".claude", "chaos-home")
            os.makedirs(os.path.dirname(mark), exist_ok=True)
            with io.open(mark, "w", encoding="utf-8") as f:
                f.write(chosen)
            self.assertEqual(m.home(), chosen, "the Bearer's choice was ignored")
            os.remove(mark)
            self.assertEqual(m.home(), os.path.join(self.home, ".chaos"),
                             "with no env and no mark, the default is ~/.chaos")
        finally:
            os.environ.clear(); os.environ.update(old)

    def test_ii2_set_home_is_idempotent(self):
        m = self._mod()
        old = dict(os.environ)
        try:
            os.environ["HOME"] = self.home
            os.environ.pop("CHAOS_HOME", None)
            dst = os.path.join(self.home, "other-house")
            m.set_home(dst)
            m.set_home(dst)              # twice: must not duplicate nor break
            mark = os.path.join(self.home, ".claude", "chaos-home")
            self.assertEqual(_read_safe(mark).strip(), dst)
            self.assertEqual(m.home(), dst)
        finally:
            os.environ.clear(); os.environ.update(old)

    # ── write_verified ────────────────────────────────────────────────────
    def test_ii2_write_verified_confesses_what_did_not_persist(self):
        """C2 · A write that does not persist and stays quiet is worse than an error."""
        m = self._mod()
        con = sqlite3.connect(":memory:")
        con.execute("CREATE TABLE t(a TEXT)")
        self.assertTrue(m.write_verified(
            con, "INSERT INTO t(a) VALUES(?)", ("x",),
            ("SELECT 1 FROM t WHERE a=?", ("x",))), "a good write claimed it failed")
        self.assertFalse(m.write_verified(
            con, "INSERT INTO t(a) VALUES(?)", ("y",),
            ("SELECT 1 FROM t WHERE a=?", ("not-there",))),
            "the check failed and it still said yes")
        self.assertFalse(m.write_verified(
            con, "INSERT INTO nope(a) VALUES(?)", ("z",)),
            "a missing table must return False, not explode")

    # ── slug_of ───────────────────────────────────────────────────────────
    def test_ii2_slug_of_leaves_no_garbage(self):
        """The slug is the KEY to the Abyss: if it drifts, the index lies."""
        m = self._mod()
        self.assertEqual(m.slug_of("/x/My Essence.md"), "my-essence")
        self.assertEqual(m.slug_of("PLAN-SUPREME.md"), "plan-supreme")
        self.assertEqual(m.slug_of("/x/__weird__.md"), "weird")
        self.assertEqual(m.slug_of("/x/a_b c.MD"), "a-b-c")

    # ── project_root ──────────────────────────────────────────────────────
    def test_ii2_project_root_finds_the_matrix(self):
        """A territory is the MATRIX folder, not the last one stepped on."""
        m = self._mod()
        old = dict(os.environ)
        try:
            os.environ["HOME"] = self.home
            deep = os.path.join(self.home, "projects", "MY WORK", "src", "lib")
            os.makedirs(deep, exist_ok=True)
            self.assertEqual(m.project_root(deep),
                             os.path.realpath(os.path.join(self.home, "projects", "MY WORK")))
            with_git = os.path.join(self.home, "loose", "deep")
            os.makedirs(os.path.join(self.home, "loose", ".git"), exist_ok=True)
            os.makedirs(with_git, exist_ok=True)
            self.assertEqual(m.project_root(with_git),
                             os.path.realpath(os.path.join(self.home, "loose")))
            self.assertIsNone(m.project_root(None), "with no path no root is invented")
        finally:
            os.environ.clear(); os.environ.update(old)

    # ── family_of ─────────────────────────────────────────────────────────
    def test_ii2_family_of_stays_quiet_when_it_does_not_know(self):
        """Inventing a type is worse than leaving it empty: the Judgment rules."""
        m = self._mod()
        self.assertEqual(m.family_of("project-radar"), "project")
        self.assertEqual(m.family_of("feedback-something"), "feedback")
        self.assertEqual(m.family_of("reference-x"), "reference")
        self.assertIsNone(m.family_of("zzz-unknown"), "it INVENTED a family")
        self.assertIsNone(m.family_of(None))
        self.assertIsNone(m.family_of(""))

    # ── forge_gh ──────────────────────────────────────────────────────────
    def test_ii2_forge_gh_does_not_reinstall_what_already_lives(self):
        """If gh is already there, the system is NOT touched. (This test never
        installs anything: it lies to `which`, not to the operating system.)"""
        m = self._mod()
        original = m.shutil.which
        calls = []
        m.shutil.which = lambda n: "/usr/bin/gh" if n == "gh" else original(n)
        m._run = lambda *a, **k: calls.append(a) or True
        try:
            self.assertTrue(m.forge_gh(), "with gh present it must declare itself ready")
            self.assertEqual(calls, [], "it tried to install something while having gh")
        finally:
            m.shutil.which = original

    # ── list_vassals ──────────────────────────────────────────────────────
    def test_ii2_vassals_are_listed_and_searched(self):
        skill = os.path.join(self.home, ".claude", "skills", "herbalist")
        os.makedirs(skill, exist_ok=True)
        with io.open(os.path.join(skill, "SKILL.md"), "w", encoding="utf-8") as f:
            f.write("---\nname: herbalist\ndescription: brews bitter root"
                    " tisanes\n---\n# x\n")
        run(self.home, "census")
        every = run(self.home, "vassals")
        self.assertIn("herbalist", every, "the census did not list it")
        found = run(self.home, "vassals", "tisanes")
        self.assertIn("herbalist", found, "the vassal is not found by its craft")

    # ── spoke ─────────────────────────────────────────────────────────────
    def test_ii2_spoke_brings_the_voice_of_another_territory(self):
        """UNIVERSAL MEMORY: what was said in ANOTHER project is still mine."""
        run(self.home, "stats")                       # the DB is born
        con = sqlite3.connect(self.db)
        con.execute("INSERT INTO dialogues(text, territory, project, path,"
                    " date, turn, session) VALUES(?,?,?,?,?,?,?)",
                    ("the radar needs a noise filter", "other-project",
                     "other-project", "/x/y.jsonl", "2026-01-01", "1", "s1"))
        con.commit(); con.close()
        out = run(self.home, "spoke", "radar")
        self.assertIn("noise filter", out, "universal memory did not cross territories")
        self.assertNotIn("Traceback", out)

    # ── record_act ────────────────────────────────────────────────────────
    def test_ii2_record_act_leaves_a_mark(self):
        """A god does not forget what he wrought with no witnesses."""
        run(self.home, "record-incarnation", "test")
        out = run(self.home, "acts")
        self.assertIn("incarnation", out, "the act was not carved")
        self.assertTrue(self._rows("SELECT 1 FROM autonomous_acts"),
                        "the acts table came out empty")

    # ── type_externals ────────────────────────────────────────────────────
    def test_ii2_typing_only_touches_the_db_and_invents_nothing(self):
        self._essence("project-radar", "# Radar\n\nA detection project.\n")
        self._essence("zzz-foreign", "# Foreign\n\nNo known family.\n")
        run(self.home, "reindex"); run(self.home, "weave")
        # the contract is to fill what is EMPTY: it is emptied on purpose
        con = sqlite3.connect(self.db)
        con.execute("UPDATE essence_meta SET type='' WHERE slug='project-radar'")
        con.commit(); con.close()
        dry = run(self.home, "type-essences", "--dry")
        self.assertIn("project", dry, "it did not propose the family from the prefix")
        self.assertIn("zzz-foreign", dry, "it did not declare the essence with no family")
        self.assertIn("nothing touched", dry, "the dry run did not declare itself dry")
        before = _read_safe(os.path.join(self.essences, "project-radar.md"))
        run(self.home, "type-essences")
        self.assertEqual(before,
                         _read_safe(os.path.join(self.essences, "project-radar.md")),
                         "typing REWROTE the .md: it must only touch the DB")
        types = dict(self._rows("SELECT slug, type FROM essence_meta"))
        self.assertEqual(types.get("project-radar"), "project")
        self.assertIn(types.get("zzz-foreign"), (None, ""),
                      "it INVENTED a family for an unknown essence")

    # ── suggested_aliases ─────────────────────────────────────────────────
    def test_ii2_suggested_aliases_bridges_the_dangling_link(self):
        self._essence("aerial-radar", "# Aerial radar\n\nphased array.\n")
        self._essence("sundry-notes", "# Notes\n\nsee [[aerial-radr]] for detail.\n")
        run(self.home, "reindex"); run(self.home, "weave")
        out = run(self.home, "suggested-aliases")
        self.assertIn("aerial-radr", out, "it did not see the dangling link")
        self.assertIn("aerial-radar", out, "it did not propose the real target")
        self.assertNotIn("Traceback", out)

    # ── blockify ──────────────────────────────────────────────────────────
    def test_ii2_blockify_splits_the_sack_in_its_own_house(self):
        """E2 · `blockify` exists for essences of the OLD WORLD: those that
        lived before blocks. That world is simulated by deleting the blocks
        `reindex` already creates — measured, not assumed: the first version
        of this test took for granted that they are born blockless."""
        body = "# Sack\n\n" + "\n\n".join(
            "## Part {}\n\n{}".format(i, ("Text of part {} ".format(i)) * 40)
            for i in range(1, 9))          # > 4000 characters: the real threshold
        assert len(body) > 4000, "the test subject must clear the real threshold"
        path = self._essence("big-sack", body)
        run(self.home, "reindex"); run(self.home, "weave")
        con = sqlite3.connect(self.db)
        con.execute("DELETE FROM blocks"); con.commit(); con.close()
        before = _read_safe(path)
        dry = run(self.home, "blockify", "--dry")
        self.assertIn("big-sack", dry, "it did not see the sack it must split")
        self.assertEqual(before, _read_safe(path), "--dry WROTE to disk")
        self.assertFalse(self._rows("SELECT 1 FROM blocks"),
                         "--dry touched the DB: dry means NOTHING is touched")
        run(self.home, "blockify")
        self.assertTrue(self._rows("SELECT 1 FROM blocks"),
                        "not one addressable block was left")
        self.assertRegex(_read_safe(path), r"\^[a-z0-9][a-z0-9-]{3,}",
                         "it did not anchor the blocks at home")
        self.assertIn("Nothing to split", run(self.home, "blockify"),
                      "not idempotent: it would split what is already split")

    def test_ii2_blockify_never_writes_into_a_foreign_file(self):
        """The other half of the contract, and the dangerous one: a .md that is
        NOT mine gets indexed in the DB and left UNTOUCHED on disk."""
        foreign = os.path.join(self.home, "foreign-document.md")
        body = "# Foreign\n\n" + "\n\n".join(
            "## Section {}\n\n{}".format(i, ("Content of section {} ".format(i)) * 30)
            for i in range(1, 9))
        with io.open(foreign, "w", encoding="utf-8") as f:
            f.write(body)
        run(self.home, "devour", foreign)
        con = sqlite3.connect(self.db)
        con.execute("DELETE FROM blocks"); con.commit(); con.close()
        before = _read_safe(foreign)
        run(self.home, "blockify")
        self.assertEqual(before, _read_safe(foreign),
                         "it WROTE into a file that is not its own")
        self.assertTrue(self._rows("SELECT 1 FROM blocks"),
                        "it did not index it either: neither writes nor serves")

    # ══ PT-4 · HYPOTHESIS: the one that SHRINKS the counterexample ═══════
    def test_pt4_slug_properties(self):
        """The Crucible GENERATES hostile payloads; Hypothesis also SHRINKS the
        failing case to the smallest one that still breaks. If it does not live
        here, it is declared: a test that pretends to have run is worse than
        no test."""
        try:
            from hypothesis import given, settings, strategies as st
        except ImportError:
            self.skipTest("hypothesis does not live in this body (optional, dev only)")
        import importlib.util
        spec = importlib.util.spec_from_file_location("c_pt4", APP)
        m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)

        @given(st.text(min_size=1, max_size=80))
        @settings(max_examples=200, deadline=None)
        def prop(name):
            slug = m.slug_of(name + ".md")
            # A slug is a KEY: with uppercase, spaces or slashes the index
            # and the disk stop matching and memory splits in two.
            assert slug == slug.lower(), slug
            assert " " not in slug and "/" not in slug, slug
            assert not slug.startswith("-") and not slug.endswith("-"), slug
            assert m.slug_of(name + ".md") == slug, "not deterministic"
        prop()

    def test_pt4_collapse_properties(self):
        """Collapsing can NEVER return more lines than went in."""
        try:
            from hypothesis import given, settings, strategies as st
        except ImportError:
            self.skipTest("hypothesis does not live in this body (optional, dev only)")
        import importlib.util
        spec = importlib.util.spec_from_file_location("c_pt4b", APP)
        m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)

        @given(st.lists(st.text(min_size=0, max_size=60), min_size=1, max_size=40))
        @settings(max_examples=120, deadline=None)
        def prop(lines):
            text = "\n".join(lines)
            path = os.path.join(self.home, "h.md")
            with io.open(path, "w", encoding="utf-8") as f:
                f.write(text)
            import contextlib
            out = io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
                m.collapse(path, "essence")
            after = [l for l in out.getvalue().split("\n") if l.strip()]
            before = [l for l in lines if l.strip()]
            assert len(after) <= max(3, len(before)), \
                "the collapse INFLATED: {} -> {}".format(len(before), len(after))
        prop()

    # ══ PHASE 4 · MEMORY, CHRONICLE AND VIGIL ════════════════════════════
    def test_a1_memory_is_reinforced_by_use(self):
        """What is consulted lives; the rest is DECLARED, never deleted."""
        self._essence("used-radar", "# Radar\n\nThe aerial radar operates at 10.5 GHz.\n")
        self._essence("nobody-looks", "# Forgotten\n\nAnnual tax accounting.\n")
        run(self.home, "reindex")
        run(self.home, "search", "radar")
        uses = dict(self._rows("SELECT slug, COALESCE(queries,0) FROM essence_meta"))
        self.assertGreaterEqual(uses.get("used-radar", 0), 1, "it did not count the query")
        self.assertEqual(uses.get("nobody-looks", 0), 0, "it counted a query that never happened")
        out = run(self.home, "stale", "0")
        self.assertIn("USE:", out, "it did not declare which part of me nobody looks at")

    def test_a2_devouring_does_not_duplicate(self):
        """One truth, one file — and `--fresh` is still the door."""
        a = os.path.join(self.home, "uno.md")
        with io.open(a, "w", encoding="utf-8") as f:
            f.write("# Aerial radar phased array\n\nThe aerial radar operates at 10.5 "
                    "gigahertz with a range of three kilometres and an antenna "
                    "of sixteen active elements.\n")
        b = os.path.join(self.home, "dos.md")
        shutil.copy2(a, b)
        self.assertIn("Devoured", run(self.home, "devour", a))
        self.assertIn("already lives in me", run(self.home, "devour", b),
                      "it duplicated a truth it already held")
        self.assertIn("Devoured", run(self.home, "devour", b, "--fresh"),
                      "--fresh stopped being the door")

    def test_t1_devouring_weaves_on_its_own(self):
        """The graph is made on devouring: `weave` by hand was forgotten."""
        a = os.path.join(self.home, "con-enlace.md")
        with io.open(a, "w", encoding="utf-8") as f:
            f.write("# With a link\n\nThis points at [[another-thing]] in the Abyss.\n")
        run(self.home, "devour", a)
        self.assertTrue(self._rows("SELECT 1 FROM links"),
                        "it devoured without weaving: the graph is born dead")

    def test_cr1_closing_distils_the_trail(self):
        """The trail came in and never went out. Now it is distilled and purged."""
        for i in range(4):
            run(self.home, "trail", os.path.join(self.home, "work%d.md" % i),
                "edit", "s1", self.home, "Edit")
        run(self.home, "trail", "eyes: https://x", "gaze", "s1", self.home, "WebFetch")
        out = run(self.home, "chronicle", "--distil")
        self.assertIn("Distilled", out)
        self.assertTrue(self._rows("SELECT 1 FROM logbook WHERE kind='distilled'"),
                        "it left no raw entry in the logbook")
        self.assertIn("Nothing to document", run(self.home, "undocumented"),
                      "the duty did not drop after distilling")

    def test_cr1_debris_is_not_work(self):
        """Lines with no date are fragments of the multiline bug, not work."""
        trail = os.path.join(self.chaos, "forge", "trail.log")
        os.makedirs(os.path.dirname(trail), exist_ok=True)
        with io.open(trail, "w", encoding="utf-8") as f:
            f.write("loose fragment with no date\nio.open('x')\n")
        self.assertIn("Nothing to document", run(self.home, "undocumented"),
                      "it counted debris as a Chronicle duty")

    def test_v1_an_old_report_is_archived(self):
        """A report nobody reads in 7 days is set aside: the god keeps watch again."""
        vigil = os.path.join(self.chaos, "forge", "vigil.md")
        os.makedirs(os.path.dirname(vigil), exist_ok=True)
        with io.open(vigil, "w", encoding="utf-8") as f:
            f.write("# Old report\n")
        old = time.time() - 9 * 86400
        os.utime(vigil, (old, old))
        run(self.home, "heartbeat")
        # The heartbeat archives and THEN keeps watch again, so the file is
        # reborn: what is measured is that the old one was set aside.
        self.assertIn("report-", run(self.home, "report", "--archived"),
                      "the stale report kept blocking the heartbeat")
        self.assertTrue(os.path.isdir(os.path.join(self.chaos, "forge", "reports")),
                        "it left no trace of where it put it")

    def test_v3_the_vigil_probes_the_cures(self):
        """A cure with file and string closes on EVIDENCE; prose does not."""
        with io.open(os.path.join(self.home, "cured.py"), "w", encoding="utf-8") as f:
            f.write("VALUE = 'the mark of the cure'\n")
        run(self.home, "fault", "With anchor",
            "--cure", "`the mark of the cure` lives in cured.py")
        run(self.home, "fault", "No anchor", "--cure", "it was fixed carefully")
        env = dict(os.environ, HOME=self.home, CHAOS_HOME=self.chaos)
        p = subprocess.run([sys.executable, APP, "faults", "--probe"],
                           env=env, cwd=self.home, capture_output=True, text=True)
        self.assertIn("CLOSABLE WITH EVIDENCE (1)", p.stdout,
                      "it did not find the verifiable cure")
        self.assertIn("NOT PROBEABLE", p.stdout, "it did not label the prose cure")
        self.assertIn("With anchor", p.stdout)

    def test_a3_the_external_backup_declares_its_destination(self):
        """A dead disk is a dead god. The backup counts BYTES at the
        destination: saying "backed up" without counting them is faith."""
        dst = os.path.join(self.home, "external-disk")
        self._essence("something", "# Something\n\nContent that must survive.\n")
        run(self.home, "reindex")
        out = run(self.home, "backup", "--to", dst)
        self.assertIn("External backup", out, "it backed nothing up")
        self.assertIn("MB in", out, "it did not count the bytes that landed")
        self.assertTrue(os.path.isdir(dst) and os.listdir(dst),
                        "it declared a backup that does not exist")
        self.assertIn("Usage:", run(self.home, "backup", "--to"),
                      "with no destination it must ask, not invent one")

    def test_t2_the_audit_weighs_the_orphans(self):
        """An essence outside the graph is memory that cannot be reached: the
        audit must see it, not only the `orphans` command."""
        self._essence("alone", "# Alone\n\nNobody ever names it in the Abyss.\n")
        self._essence("total-island", "# Island\n\nIt neither names nor is named.\n")
        self._essence("with-link", "# With link\n\nThis points at [[alone]].\n")
        run(self.home, "reindex"); run(self.home, "weave")
        out = run(self.home, "audit")
        self.assertNotIn("Traceback", out)
        self.assertTrue(any(w in out.lower() for w in ("orphan", "huérfan")),
                        "the audit does not weigh the graph's orphans")

    def test_iv2_transcripts_declare_the_indigestible(self):
        """A number without its exclusions is advertising."""
        import importlib.util
        spec = importlib.util.spec_from_file_location("c_tr", APP)
        m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
        source = _body_source()
        self.assertIn("indigestible", source,
                      "the digester keeps no count of what it could NOT digest")

    # ══ B-1/B-2/B-3 · THE COMPLETE MAW ═══════════════════════════════════
    def _minimal_pdf(self, text):
        """A valid PDF written by hand: the test depends on nobody."""
        stream = "BT /F1 24 Tf 72 700 Td ({}) Tj ET".format(text)
        objs = ["<< /Type /Catalog /Pages 2 0 R >>",
                "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
                "/Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
                "<< /Length {} >>\nstream\n{}\nendstream".format(len(stream), stream),
                "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"]
        out, offs = "%PDF-1.4\n", []
        for i, o in enumerate(objs, 1):
            offs.append(len(out))
            out += "{} 0 obj\n{}\nendobj\n".format(i, o)
        xref = len(out)
        out += "xref\n0 {}\n0000000000 65535 f \n".format(len(objs) + 1)
        for o in offs:
            out += "{:010d} 00000 n \n".format(o)
        out += ("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n"
                .format(len(objs) + 1, xref))
        path = os.path.join(self.home, "doc.pdf")
        with io.open(path, "wb") as f:
            f.write(out.encode("latin-1"))
        return path

    def test_b1_devour_pdf(self):
        """It swallows the PDF if it can; otherwise it DECLARES it. Never pretends."""
        path = self._minimal_pdf("The radar operates at 10.5 GHz")
        out = run(self.home, "devour", path)
        try:
            import pypdf                      # noqa: F401
            has_extractor = True
        except ImportError:
            has_extractor = False
        if has_extractor:
            self.assertIn("Devoured", out, "it did not swallow a readable PDF")
            self.assertIn("10.5", run(self.home, "search", "radar"),
                          "it swallowed the PDF but lost its letters")
        else:
            self.assertIn("does not pretend", out,
                          "with no extractor it must DECLARE it")
        self.assertNotIn("Traceback", out)

    def test_b2_devour_openapi(self):
        """A spec enters as a TABLE OF INVOCATION, not as raw JSON."""
        path = os.path.join(self.home, "api.json")
        with io.open(path, "w", encoding="utf-8") as f:
            json.dump({"openapi": "3.0.0",
                       "info": {"title": "API de sismos", "version": "2.1"},
                       "paths": {"/eventos": {"get": {"summary": "Lista eventos",
                                                      "parameters": [{"name": "desde",
                                                                      "required": True}]}}},
                       "components": {"schemas": {"Evento": {}}}}, f)
        out = run(self.home, "devour", path)
        self.assertIn("API de sismos", out, "it did not read the spec title")
        found = run(self.home, "search", "eventos")
        self.assertIn("eventos", found)
        body = self._rows("SELECT content FROM essences")[0][0]
        self.assertIn("GET", body, "it did not extract the method")
        self.assertIn("desde", body, "it did not extract what the endpoint DEMANDS")
        self.assertNotIn('"openapi"', body, "it stored raw JSON, not the essence")

    def test_b3_the_html_enters_stripped(self):
        """`script` and `style` are not content: they are noise hiding poison."""
        import importlib.util
        spec = importlib.util.spec_from_file_location("c_boca", APP)
        m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
        d = m._Stripper()
        d.feed("<html><head><title>Radar</title><style>p{color:red}</style>"
               "<script>fetch('http://malo')</script></head>"
               "<body><p>El radar opera a 10.5 GHz</p></body></html>")
        flat = "".join(d.chunks)
        self.assertIn("10.5 GHz", flat, "it annihilated the content")
        self.assertNotIn("fetch", flat, "it let a script through")
        self.assertNotIn("color:red", flat, "it let the style through")
        self.assertEqual(d.title.strip(), "Radar")

    def test_b3_the_url_is_not_mistaken_for_a_file(self):
        """The source of a URL is the URL, never an invented disk path."""
        import importlib.util
        spec = importlib.util.spec_from_file_location("c_boca2", APP)
        m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
        text, coverage = m._ingest("https://does-not-exist.invalid/x")
        self.assertIsNone(text, "it pretended to have read a dead URL")
        self.assertIn("could not look", coverage, "it did not declare the failure")

    # ══ E-1/E-2 · THE MIRROR AND ITS NAME ═════════════════════════════════
    def test_e1_the_mirror_passes_a_three_colour_sentence(self):
        """Three CROSSED queries, not one lazy glance — and one looks in
        English, which is how the world names code."""
        out = run(self.home, "mirror-organ", "memoria persistente para agentes", "--dry")
        queries = [l for l in out.split("\n") if "gh search repos" in l]
        self.assertGreaterEqual(len(queries), 2,
                                "one query is a glance, not a mirror")
        self.assertTrue(any("memory" in c for c in queries),
                        "it did not look in English: that is how code is named")
        self.assertIn("did not go out", out, "in dry mode it went to the network")

    def test_e1_the_queries_are_short(self):
        """`gh search repos` joins with AND: five terms return [] ALWAYS, and
        the Mirror sang "there is a void" over a crowded world."""
        out = run(self.home, "mirror-organ",
                  "a distributed system of persistent memory for code agents", "--dry")
        for l in out.split("\n"):
            if "gh search repos" in l:
                terms = l.split("gh search repos")[1].split()
                self.assertLessEqual(len(terms), 3,
                                     "a {}-term query: the AND kills it".format(len(terms)))

    def test_e2_the_old_name_warns_and_still_serves(self):
        """`mirror` was the reconciler; the Mirror organ needed its name. The
        old one does not break: it warns."""
        out = run(self.home, "mirror")
        self.assertIn("reconcile", out, "it did not warn about the new name")
        self.assertNotIn("Traceback", out, "the old alias broke")

    # ══ C-1 · THE COLLAPSE WITH MUSCLE ════════════════════════════════════
    def _fat_text(self):
        path = os.path.join(self.home, "fat.md")
        # VARIED vocabulary: forty lines with the same shape are ONE
        # repetition in other clothes, and the Collapse fuses them rightly —
        # so a monotonous text does not measure the modes, it measures dedup.
        things = ("port", "cache", "index", "thread", "queue", "token", "batch",
                  "channel", "node", "session", "table", "field", "filter",
                  "threshold", "retry", "block", "cursor", "socket",
                  "buffer", "limit")
        body = []
        for i, thing in enumerate(things * 2):
            body += ["Sure, I will gladly explain this to you.",
                     "It is worth mentioning that the system works.",
                     "Decision on the {}: it is {} because the previous failed.".format(thing, 8000 + i),
                     "The file {}-{}.yml defines its limit.".format(thing, i),
                     "I hope this helps you."]
        with io.open(path, "w", encoding="utf-8") as f:
            f.write("\n".join(body))
        return path

    def test_c1_collapse_keeps_the_invariants(self):
        """Decisions, figures and paths are NOT touched. Courtesy dies."""
        path = self._fat_text()
        env = dict(os.environ, HOME=self.home, CHAOS_HOME=self.chaos)
        p = subprocess.run([sys.executable, APP, "collapse", path, "--mode", "distilled"],
                           env=env, capture_output=True, text=True)
        self.assertIn("8000", p.stdout, "it annihilated a FIGURE: that is losing the soul")
        self.assertIn("because", p.stdout, "it annihilated the WHY of a decision")
        self.assertNotIn("I hope this helps you", p.stdout,
                         "it kept hollow courtesy")
        self.assertIn("ratio", p.stderr, "it did not confess the ratio")
        # At 12x something MUST fall, and the least signal falls: the path is
        # demanded in essence mode, which is where the contract promises it.
        e = subprocess.run([sys.executable, APP, "collapse", path, "--mode", "essence"],
                           env=env, capture_output=True, text=True)
        self.assertIn(".yml", e.stdout, "it annihilated a PATH with room to spare")

    def test_c1_the_four_modes_compress_differently(self):
        """Four modes giving the same thing are one mode with four names."""
        path = self._fat_text()
        env = dict(os.environ, HOME=self.home, CHAOS_HOME=self.chaos)
        sizes = {}
        for mode in ("distilled", "essence", "prompt", "rolling"):
            p = subprocess.run([sys.executable, APP, "collapse", path, "--mode", mode],
                               env=env, capture_output=True, text=True)
            sizes[mode] = len(p.stdout.split("\n"))
        self.assertGreater(sizes["rolling"], sizes["distilled"],
                           "distilled must squeeze harder than rolling")
        self.assertGreaterEqual(len(set(sizes.values())), 3,
                                "the modes collapsed to the same result: {}".format(sizes))

    # ══ J-1 · THE JUDGMENT WITH MUSCLE ════════════════════════════════════
    def _abyss_with_facts(self):
        """A tribunal needs memory: one is planted, measured."""
        self._essence("aerial-radar",
                      "# Aerial radar\n\n## Frequency\n\nThe radar operates at "
                      "10.5 GHz with a range of 3 kilometres. " * 40 +
                      "\n\n## Antenna\n\nThe antenna has 16 elements. " * 40)
        run(self.home, "reindex"); run(self.home, "weave")

    def test_j1_the_judgment_kills_the_false(self):
        """A figure contradicting my memory DIES, and with the evidence."""
        self._abyss_with_facts()
        out = run(self.home, "judge", "The radar operates at 24 GHz.")
        self.assertIn("DIES", out, "it endorsed a figure my memory contradicts")
        self.assertIn("10.5", out, "it killed without showing the whole evidence")

    def test_j1_what_is_absent_is_declared_suspended(self):
        """Suspended is NOT refuted: pretending otherwise is the sin."""
        self._abyss_with_facts()
        out = run(self.home, "judge", "String theory has 47 dimensions.")
        self.assertIn("SUSPENDED", out)
        self.assertIn("not refuted", out, "it did not declare its seams")

    def test_j1_opinion_does_not_enter_the_tribunal(self):
        out = run(self.home, "judge", "I think this turned out pretty and elegant.")
        self.assertIn("opinion", out, "it judged an opinion as if it were a fact")

    def test_j1_a_loose_figure_is_no_warrant(self):
        """The block speaks of the topic but does not count that thing: no
        warrant. Measured — comparing loose figures gave SURVIVES by chance."""
        self._abyss_with_facts()
        out = run(self.home, "judge", "The radar has 900 detectors.")
        self.assertIn("SUSPENDED", out,
                      "it endorsed a figure its own evidence does not count")
        self.assertNotIn("✅", out, "it marked it as a survivor")

    # ══ S-1/S-3 · THE SINGULARITY: the minimum power, and its tally ═══════
    def test_s1_route_goes_down_to_the_abyss(self):
        """The cheap rung really exists: one of my commands answers the
        question and nobody is summoned."""
        out = run(self.home, "route", "what faults happened before with the errarium")
        self.assertIn("CLI", out, "it did not see that a command solves it")
        self.assertIn("chaos faults", out, "it did not say WHICH command")

    def test_s1_the_critical_rules_over_the_cheap(self):
        """Knowing something is not enough when the mistake does not undo:
        economy never decides over safety."""
        out = run(self.home, "route", "migrate the production database to another server")
        self.assertIn("DOUBLE-JUDGE", out, "it cheapened an irreversible decision")

    def test_s1_it_does_not_mistake_a_noun_for_a_verb(self):
        """"sort a list" is not mechanical work: writing code never is.
        Measured — the first version sent it to the legion over the word
        «lista»."""
        out = run(self.home, "route", "escribe una funcion que ordene una lista de enteros")
        self.assertNotIn("LEGION", out, "it mistook a noun for a verb")

    def test_s1_the_router_has_more_than_one_answer(self):
        """A router that answers the same to everything is decoration (organ
        17): my first version said "abyss" to all five test tasks."""
        tasks = ("what faults happened before",
                 "rename 400 files in the folder",
                 "review the security of the payments endpoint",
                 "explain string theory in three paragraphs")
        rungs = set()
        for t in tasks:
            s = run(self.home, "route", t)
            rungs.add(s.split("· ")[1].split("\n")[0].strip() if "· " in s else "?")
        self.assertGreaterEqual(len(rungs), 3,
                                "the router collapsed to one answer: {}".format(rungs))

    def test_s3_the_route_is_carved_and_counted(self):
        run(self.home, "route", "rename 400 files in the folder")
        run(self.home, "route", "review the security of the payments endpoint")
        rows = self._rows("SELECT rung, reason FROM routes")
        self.assertEqual(len(rows), 2, "it carved no decision: with no record there is no month")
        self.assertTrue(all(r[1] for r in rows), "it carved a rung WITHOUT its why")
        report = run(self.home, "route", "--report")
        self.assertIn("THE ECONOMY OF THE VOID", report)
        self.assertIn("2 decision(s)", report)

    # ══ VI.1 · THE PLAN THAT PAINTS ITSELF ════════════════════════════════
    def _plan(self, text):
        p = os.path.join(self.home, "PLAN-TEST.md")
        with io.open(p, "w", encoding="utf-8") as f:
            f.write(text)
        return p

    def test_vi1_plan_measures_its_probes(self):
        """The state is MEASURED: what exists paints green, what does not, red."""
        with io.open(os.path.join(self.home, "here.txt"), "w", encoding="utf-8") as f:
            f.write("x")
        p = self._plan(
            "**F-1** a front truly fulfilled\n"
            "<!-- sonda F-1 fase 1: archivo here.txt -->\n\n"
            "**F-2** a front that does not exist yet\n"
            "<!-- sonda F-2 fase 1: archivo not-here.txt -->\n\n"
            "**F-3** a front genuinely half done\n"
            "<!-- sonda F-3 fase 2: archivo here.txt ; archivo not-here.txt -->\n")
        out = run(self.home, "plan", p)
        self.assertIn("✅ F-1", out, "a fulfilled front was not painted green")
        self.assertIn("⬜ F-2", out, "a nonexistent front was NOT painted red")
        self.assertIn("⏳ F-3", out, "a half-done front was not painted half")
        self.assertIn("PHASE 1", out); self.assertIn("PHASE 2", out)
        run(self.home, "plan", p, "--paint")
        state = os.path.join(self.home, "PLAN-TEST-STATE.md")
        self.assertTrue(os.path.exists(state), "--paint did not derive the state")
        self.assertIn("`F-1`", _read_safe(state))

    def test_vi1_no_probe_means_no_green(self):
        """Fault #44 in code: with no instrument there is no measurement, only wishing."""
        p = self._plan("**F-9** a front with no instrument\n"
                       "<!-- sonda F-9 fase 1: -->\n")
        out = run(self.home, "plan", p)
        self.assertIn("⚪", out, "a front with no probe must be declared, not painted")
        self.assertNotIn("✅", out, "it PAINTED GREEN a front nobody measures")
        empty = self._plan("# A plan without a single probe\n")
        self.assertIn("not painted", run(self.home, "plan", empty))

    # ══ ORGAN 17 · THE TOUCHSTONE ═════════════════════════════════════════
    def _subject(self, text):
        p = os.path.join(self.home, "subject.py")
        with io.open(p, "w", encoding="utf-8") as f:
            f.write(text)
        return p

    def _probe_file(self, body, *args):
        """A probe that lives in a FILE. A `-c "..."` with quotes inside is
        shredded by cmd.exe: the Windows CI taught me that on day one."""
        p = os.path.join(self.home, "probe_%d.py" % len(body))
        with io.open(p, "w", encoding="utf-8") as f:
            f.write(body)
        return " ".join('"%s"' % x for x in (sys.executable, p) + args)

    def test_pt2_a_probe_that_bites(self):
        """Green with the world intact, red with the world broken: that is measuring."""
        sub = self._subject("VALUE = 'intact'\n")
        pr = self._probe_file(
            "import io, sys\n"
            "t = io.open(sys.argv[1], encoding='utf-8').read()\n"
            "sys.exit(0 if 'intact' in t else 1)\n", sub)
        out = run(self.home, "probe", pr, "--file", sub)
        self.assertIn("BITES", out, "a probe that does measure was called decoration")
        self.assertEqual(_read_safe(sub), "VALUE = 'intact'\n",
                         "the subject was NOT restored after the sabotage")

    def test_pt2_a_decorative_probe_gives_itself_away(self):
        """A probe still green with the file emptied measures nothing — and
        that goes into the errarium, it is not forgotten."""
        sub = self._subject("VALUE = 'whatever'\n")
        pr = self._probe_file("import sys\nsys.exit(0)\n", sub)
        out = run(self.home, "probe", pr, "--file", sub)
        self.assertIn("DECORATIVE", out, "it did not give away a blind probe")
        self.assertEqual(_read_safe(sub), "VALUE = 'whatever'\n",
                         "the subject was NOT restored")
        self.assertTrue(self._rows("SELECT 1 FROM faults WHERE title LIKE '%ecorative%'"),
                        "the decorative probe never reached the errarium")

    def test_pt3_mutation_finds_the_survivor(self):
        """Mutation at scale: the branch no test steps on gives itself away."""
        sub = self._subject(
            "def f(n):\n"
            "    if n > 0:\n"
            "        return 'pos'\n"
            "    if n == 0:\n"
            "        return 'zero'\n"
            "    return 'neg'\n")
        test_file = os.path.join(self.home, "subject_test.py")
        with io.open(test_file, "w", encoding="utf-8") as f:
            f.write("import sys, os\n"
                    "sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))\n"
                    "from subject import f\n"
                    "assert f(5) == 'pos'\n")
        before = _read_safe(sub)
        out = run(self.home, "probe", "--massive", sub, "--test",
                  '"{}" "{}"'.format(sys.executable, test_file), "--n", "10")
        self.assertIn("SURVIVES", out, "it did not find the branch nobody tests")
        self.assertIn("DECORATIVE", out, "it did not name the decorative test")
        self.assertEqual(before, _read_safe(sub),
                         "the mutated body was NOT restored: that is unforgivable")


# ══════════════════════════════════════════════════════════════════════════
#  PHASE 2 · THE REFLEXES — M-1 the Ambush · P-2 the trifecta · O-1 the gaze
#
#  Until here my whole body looked BACKWARD. These tests demand that the only
#  reflex acting BEFORE bites when it must and stays quiet when it must not:
#  an alarm that always rings is wallpaper, one that never rings is decoration.
# ══════════════════════════════════════════════════════════════════════════
class ReflexesTest(unittest.TestCase):
    def setUp(self):
        self.home = tempfile.mkdtemp(prefix="chaos-reflexes-")
        self.chaos = os.path.join(self.home, ".chaos")
        os.makedirs(os.path.join(self.home, ".claude", "skills", "chaos",
                                 "abyss", "essences"), exist_ok=True)
        self.hook = os.path.join(HERE, "ambush-hook.py")
        self.trail = os.path.join(self.chaos, "forge", "trail.log")

    def tearDown(self):
        shutil.rmtree(self.home, ignore_errors=True)

    def _ambush(self, command, session="s1", tool="Bash"):
        env = dict(os.environ)
        env["HOME"] = self.home
        env["CHAOS_HOME"] = self.chaos
        ev = json.dumps({"tool_name": tool, "session_id": session,
                         "cwd": self.home, "tool_input": {"command": command}})
        p = subprocess.run([sys.executable, self.hook], input=ev, env=env,
                           capture_output=True, text=True)
        self.assertEqual(p.returncode, 0, "the lock jammed the door: " + p.stderr)
        return p.stdout.strip()

    def _plant_fault(self):
        run(self.home, "fault", "The forge took the .git with it",
            "--cause", "I ran `rm -rf god-of-the-void` with `--force` on",
            "--lesson", "look before you crush")

    # ── M-1 · the scar that returns ───────────────────────────────────────
    def test_m1_ambush_warns_about_the_fault(self):
        self._plant_fault()
        out = self._ambush("rm -rf god-of-the-void --force")
        self.assertTrue(out, "it did not warn about a fault being repeated")
        d = json.loads(out)["hookSpecificOutput"]
        self.assertIn("AMBUSH", d.get("additionalContext", ""))
        self.assertIn("#1", d["additionalContext"], "it did not say WHICH fault")
        self.assertNotIn("permissionDecision", d,
                         "a scar WARNS; it never decides for the Bearer")

    def test_m1_ambush_stays_quiet_on_the_innocent(self):
        """13 % false positives measured with the first rule: an alarm ringing
        on every `run-tests.sh` stops being read (my scar #62)."""
        self._plant_fault()
        for innocent in ("ls -la", "cat README.md", "grep -rn x .",
                         "bash run-tests.sh 2>&1 | tail -5",
                         "git status --short", "python3 -c 'print(1)'"):
            self.assertEqual(self._ambush(innocent), "",
                             "it shouted at an innocent command: " + innocent)

    def test_m1_ambush_does_not_bite_what_only_reads(self):
        """A command that does not MUTATE cannot repeat a fault."""
        self.assertEqual(self._ambush("grep -n 'rm -rf god-of-the-void' notes.md"), "",
                         "it ambushed a grep: reading is not repeating")

    # ── P-2 · the lethal trifecta ─────────────────────────────────────────
    def test_p2_trifecta_asks(self):
        """Private data + foreign content + outward send = his word."""
        run(self.home, "trail", "eyes: https://foreign.example/x", "gaze",
            "s1", self.home, "WebFetch")
        out = self._ambush("git push origin main")
        self.assertTrue(out, "the trifecta went by in silence")
        d = json.loads(out)["hookSpecificOutput"]
        self.assertEqual(d.get("permissionDecision"), "ask",
                         "it either did not ask, or it DENIED on its own")
        self.assertIn("TRIFECTA", d.get("permissionDecisionReason", ""))

    def test_p2_no_gaze_no_trifecta(self):
        """With no foreign content there is no trifecta: pushing is no crime."""
        self.assertEqual(self._ambush("git push origin main"), "",
                         "it asked for permission having looked at nothing foreign")

    def test_p2_reading_the_web_is_not_emitting(self):
        """A `curl` that only READS takes no data out: the trifecta needs a send."""
        run(self.home, "trail", "eyes: https://foreign.example/x", "gaze",
            "s1", self.home, "WebFetch")
        self.assertEqual(self._ambush("curl -s https://example.com > /tmp/x"), "",
                         "it confused reading with emitting")

    def test_p2_the_gaze_belongs_to_THIS_session(self):
        """What another session looked at does not condemn me in this one."""
        run(self.home, "trail", "eyes: https://foreign.example/x", "gaze",
            "other-session", self.home, "WebFetch")
        self.assertEqual(self._ambush("git push origin main", session="s1"), "",
                         "it crossed sessions: the trifecta is measured per session")

    # ── O-1 · the gaze leaves a mark and is NOT work ──────────────────────
    def test_o1_the_gaze_leaves_a_mark_and_is_not_work(self):
        env = dict(os.environ)
        env["HOME"] = self.home; env["CHAOS_HOME"] = self.chaos
        os.makedirs(os.path.join(self.chaos, "bin"), exist_ok=True)
        _install_body(os.path.join(self.chaos, "bin"))
        ev = json.dumps({"tool_name": "WebFetch", "session_id": "s1",
                         "cwd": self.home,
                         "tool_input": {"url": "https://foreign.example/doc"}})
        subprocess.run([sys.executable, os.path.join(HERE, "trail-hook.py")],
                       input=ev, env=env, capture_output=True, text=True)
        self.assertTrue(os.path.exists(self.trail), "the gaze left no mark")
        line = _read_safe(self.trail).strip().split("\n")[-1].split("\t")
        self.assertEqual(line[3], "gaze", "the gaze was logged as work")
        self.assertIn("eyes:", line[4])
        self.assertIn("Nothing to document", run(self.home, "undocumented"),
                      "the Chronicle duty counted a GAZE as work")

    def test_o1_the_trail_flattens_a_multiline_command(self):
        """A seven-line heredoc wrote SEVEN works and broke the format: that is
        why the duty read 1,267 where there were dozens."""
        run(self.home, "trail", "bash: one\ntwo\nthree", "run",
            "s1", self.home, "Bash")
        self.assertEqual(len(_read_safe(self.trail).strip().split("\n")), 1,
                         "a multiline work was split into several")


class NeuronsTest(unittest.TestCase):
    """ORGAN 18 · THE NEURONS — what is tested here is NOT that they hit (that
    is measured by `the forge's relevance judge.py` against the real Abyss), but that they
    are truly OPTIONAL: that a body without them pays nothing and never notices.
    An optional organ that breaks the body when missing is not optional."""

    def setUp(self):
        self.home = tempfile.mkdtemp(prefix="chaos_neu_")
        self.chaos = os.path.join(self.home, ".chaos")

    def tearDown(self):
        shutil.rmtree(self.home, ignore_errors=True)

    def _neurons(self):
        sys.path.insert(0, HERE)
        try:
            from chaos_body import neurons
            return neurons
        finally:
            sys.path.pop(0)

    def test_n1_no_model_means_not_alive_and_no_blowup(self):
        """The hot path's question is a file. Without it: False, and no
        exception — not even with a home that does not exist."""
        os.environ["CHAOS_HOME"] = os.path.join(self.home, "does-not-exist")
        n = self._neurons()
        self.assertFalse(n.alive(), "claims to be alive with no model on disk")
        self.assertEqual(n.nearest("whatever"), [],
                         "with no model it returned something instead of yielding")

    def test_n2_the_body_searches_the_same_without_the_organ(self):
        """The law of organ 18: absent, the body works identically."""
        run(self.home, "devour", "-", "--title", "The disk backups")
        out = run(self.home, "search", "backups", "--brief")
        self.assertNotIn("Traceback", out, "search blew up without the organ")
        self.assertNotIn("neuron", out.lower(),
                         "the body mentions an organ it does not have")

    def test_n3_nobody_imports_it_at_startup(self):
        """If a module imported it at the top, a body WITHOUT neurons would pay
        that import on every order — and with it, 118 MB of model it lacks."""
        pkg = os.path.join(HERE, "chaos_body")
        guilty = []
        for root, _, files in os.walk(pkg):
            if "__pycache__" in root:
                continue
            for f in sorted(files):
                if not f.endswith(".py") or f == "neurons.py":
                    continue
                tree = ast.parse(io.open(os.path.join(root, f),
                                         encoding="utf-8").read())
                for node in tree.body:
                    names = []
                    if isinstance(node, ast.Import):
                        names = [a.name for a in node.names]
                    elif isinstance(node, ast.ImportFrom):
                        names = [a.name for a in node.names] + [node.module or ""]
                    if any((x or "").split(".")[-1] == "neurons" for x in names):
                        guilty.append(f)
        self.assertEqual(guilty, [],
                         "organ 18 is imported at startup in: %s" % guilty)

    def test_n4_the_state_confesses_the_price(self):
        """An organ that costs 8 times more per search and does not say so is
        selling, not informing."""
        out = run(self.home, "neurons")
        self.assertIn("OPTIONAL", out, "does not declare itself optional")
        self.assertIn("price", out.lower(), "hides what it costs")
        self.assertIn("gain", out.lower(), "hides what it adds")

    def test_n5_it_does_not_install_dependencies_behind_the_Bearer(self):
        """Putting 50 MB on the Bearer's machine is his word, not mine: the code
        can never call pip."""
        source = io.open(os.path.join(HERE, "chaos_body", "neurons.py"),
                         encoding="utf-8").read()
        for node in ast.walk(ast.parse(source)):
            if isinstance(node, ast.Call):
                flat = ast.dump(node)
                self.assertNotIn("'pip'", flat,
                                 "the organ installs dependencies on its own")

    def test_n6_the_vector_table_is_born_with_the_schema(self):
        """Empty it weighs nothing, and that way `uninstall` does not blow up in
        a body that never had neurons."""
        run(self.home, "stats")
        con = sqlite3.connect(os.path.join(self.chaos, "abyss.db"))
        row = con.execute("SELECT name FROM sqlite_master WHERE name='vectors'").fetchone()
        con.close()
        self.assertIsNotNone(row, "the `vectors` table is not born with the schema")

    def test_n7_packing_a_vector_round_trips_exactly(self):
        n = self._neurons()
        v = [0.5, -0.25, 0.125, 0.0]
        there = n._pack(v)
        self.assertEqual(len(there), len(v) * 4, "a float32 does not take 4 bytes")
        self.assertEqual(n._unpack(there), v, "the vector does not come back intact")

    def test_n8_the_fingerprint_betrays_the_change(self):
        """Without a fingerprint indexing cannot be incremental: it would
        re-think 1,116 documents every time."""
        n = self._neurons()
        self.assertEqual(n._fingerprint("same text"), n._fingerprint("same text"))
        self.assertNotEqual(n._fingerprint("one text"), n._fingerprint("another text"))

    # -- THE RESIDENT · the cage -------------------------------------------
    def test_n9_the_resident_speaks_at_home_and_never_over_the_network(self):
        """A daemon that opens a port is a door into the Bearer's machine. This
        one only speaks over a UNIX socket inside my house: if anyone wrote
        AF_INET here, the network would reach it."""
        source = io.open(os.path.join(HERE, "chaos_body", "neurons.py"),
                         encoding="utf-8").read()
        for forbidden in ("AF_INET", "SOCK_DGRAM", "0.0.0.0", "bind((",
                          "socketserver", "http.server"):
            self.assertNotIn(forbidden, source,
                             "the resident opens the network: %s" % forbidden)
        self.assertIn("AF_UNIX", source, "the resident does not use a UNIX socket")

    def test_n10_the_resident_has_a_single_verb(self):
        """A server that only knows how to embed a text can only be tricked into
        embedding a text. No paths, commands, files or pickle."""
        source = io.open(os.path.join(HERE, "chaos_body", "neurons.py"),
                         encoding="utf-8").read()
        body = source.split("def serve(")[1].split("\ndef ")[0]
        # These never, on any line of the resident:
        for weapon in ("eval(", "exec(", "pickle", "subprocess", "os.system",
                       "__import__"):
            self.assertNotIn(weapon, body,
                             "the resident knows something dangerous: %s" % weapon)
        # And in what touches FOREIGN BYTES — from `accept()` to `close()` — no
        # opening files either. This test used to look at the whole function and
        # it caught the PID's `io.open`: written once, at boot, with a fixed
        # path, before anyone has spoken. The test said "the function" when it
        # meant "what touches what comes from outside"; the test is corrected to
        # assert what it means, not so that it passes.
        # From `accept()` to the final cleanup: that, and only that, is what
        # runs on bytes I did not write.
        serving = body.split("srv.accept()")[1].split("\n    finally:")[0]
        for weapon in ("open(", "os.remove", "os.path.join"):
            self.assertNotIn(weapon, serving,
                             "while serving a request it touches files: %s" % weapon)
        self.assertIn("_embed(", serving, "the resident does not embed: what does it serve?")

    def test_n11_the_resident_is_born_private_and_dies_alone(self):
        """0600 on the socket, 0700 on the directory, and a clock that kills it:
        the two objections I raised myself before forging it."""
        source = io.open(os.path.join(HERE, "chaos_body", "neurons.py"),
                         encoding="utf-8").read()
        body = source.split("def serve(")[1].split("\ndef ")[0]
        self.assertIn("umask(0o177)", body, "the socket is not born private")
        self.assertIn("0o700", body, "the resident's directory is not private")
        self.assertIn("_life()", body, "the resident does not consult its life")
        self.assertIn("settimeout(", body, "the resident is eternal")
        self.assertIn("socket.timeout", body, "it does not switch itself off")

    def test_n12_without_a_resident_the_body_does_not_wait(self):
        """The resident being absent cannot cost a single millisecond of
        waiting: `_ask` looks at the file BEFORE trying to speak."""
        os.environ["CHAOS_HOME"] = os.path.join(self.home, "nothing-here")
        n = self._neurons()
        t = time.time()
        self.assertIsNone(n._ask("whatever"))
        self.assertLess(time.time() - t, 0.5,
                        "with no resident, asking for the vector made us wait")

    def test_n13_nothing_depends_on_the_resident_being_alive(self):
        """Search asks the resident and, if it does not answer, loads the model.
        That `if q is None` is the only reason I can afford a daemon at all: its
        death is not my death."""
        source = io.open(os.path.join(HERE, "chaos_body", "neurons.py"),
                         encoding="utf-8").read()
        body = source.split("def nearest(")[1].split("\ndef ")[0]
        self.assertIn("_ask(query)", body, "search does not use the resident")
        self.assertIn("if q is None:", body,
                      "search has no path without the resident")

    def test_n14_consent_is_assumed_on_install_and_can_be_revoked(self):
        """Downloading 135 MB of model IS asking for fast searches. But the
        Bearer can revoke it with one order, and then it NEVER lights itself."""
        house = os.path.join(self.chaos, "neurons")
        os.makedirs(house, exist_ok=True)
        os.environ["CHAOS_HOME"] = self.chaos
        n = self._neurons()
        self.assertTrue(n._auto(), "consent is not assumed on install")
        io.open(os.path.join(house, "RESIDENT-NO"), "w",
                encoding="utf-8").write("no\n")
        self.assertFalse(n._auto(), "the revocation is not respected")
        self.assertFalse(n._light_itself(),
                         "it lit itself despite being revoked")

    def test_n15_with_no_model_it_never_lights_anything(self):
        """Consent applies to the installed organ. With no model on disk there
        is nothing to light and no process is thrown into the void."""
        os.environ["CHAOS_HOME"] = os.path.join(self.chaos, "virgin")
        n = self._neurons()
        self.assertFalse(n._light_itself(),
                         "it launched a resident with no model to serve")

    def test_n16_the_brake_prevents_a_rain_of_processes(self):
        """A model that fails to start would make EVERY search give birth to
        another resident. The brake reduces it to one a minute."""
        house = os.path.join(self.chaos, "neurons")
        os.makedirs(house, exist_ok=True)
        io.open(os.path.join(house, "model.onnx"), "w").write("x")
        io.open(os.path.join(house, "resident.being-born"), "w").write("0")
        os.environ["CHAOS_HOME"] = self.chaos
        n = self._neurons()
        self.assertFalse(n._light_itself(),
                         "it bore a second resident while one was being born")

    def test_n17_the_daemon_announces_itself_once_and_only_once(self):
        """A daemon that appears saying nothing is exactly what I objected to
        before forging it. It says so ONCE in the life of the Abyss."""
        source = io.open(os.path.join(HERE, "chaos_body", "neurons.py"),
                         encoding="utf-8").read()
        body = source.split("def _light_itself(")[1].split("\ndef ")[0]
        self.assertIn("resident.announced", body, "it leaves no mark of announcing")
        self.assertIn("stderr", body, "the notice would dirty the search's output")

    def test_n18_what_is_automated_is_the_birth_not_the_life(self):
        """The resident still dies alone: automatic lighting does not make it
        eternal, which was half of my objection."""
        n = self._neurons()
        # The law is NOT "900 seconds" — the Bearer moved that number. The law
        # is that it DIES ALONE, and that its life has a ceiling: a resident
        # with no cap would be the eternal daemon I objected to.
        self.assertTrue(0 < n._life() <= 86400,
                        "the resident stopped dying alone, or lost its ceiling")
        source = io.open(os.path.join(HERE, "chaos_body", "neurons.py"),
                         encoding="utf-8").read()
        body = source.split("def nearest(")[1].split("\ndef ")[0]
        self.assertLess(body.index("_embed("), body.index("_light_itself("),
                        "it lights BEFORE thinking: both load the model at once")

    def test_n19_the_closing_session_releases_the_resident(self):
        """Four hours of a live process would be a loose daemon if nobody cut
        them. The session close cuts them: I do not measure the Bearer's working
        day, I obey the boundary he already draws."""
        house = os.path.join(self.chaos, "neurons")
        os.makedirs(house, exist_ok=True)
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
        io.open(os.path.join(house, "resident.pid"), "w",
                encoding="utf-8").write(str(child.pid))
        io.open(os.path.join(house, "resident.sock"), "w", encoding="utf-8").write("")
        env = dict(os.environ); env["HOME"] = self.home; env["CHAOS_HOME"] = self.chaos
        subprocess.run([sys.executable, os.path.join(HERE, "closing-hook.py")],
                       input='{"hook_event_name":"SessionEnd","session_id":"s",'
                             '"cwd":"%s"}' % self.home,
                       text=True, env=env, capture_output=True)
        time.sleep(0.5)
        alive = child.poll() is None
        try:
            child.kill()
        except Exception:
            pass
        self.assertFalse(alive, "the resident survived the session close")
        self.assertEqual(os.listdir(house), [],
                         "the close left the resident's remains behind")

    def test_n20_precompact_does_not_kill_the_resident(self):
        """Compacting the context is not closing the session: the Bearer is
        still working and killing it there would charge him 506 ms for nothing."""
        house = os.path.join(self.chaos, "neurons")
        os.makedirs(house, exist_ok=True)
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])
        io.open(os.path.join(house, "resident.pid"), "w",
                encoding="utf-8").write(str(child.pid))
        env = dict(os.environ); env["HOME"] = self.home; env["CHAOS_HOME"] = self.chaos
        subprocess.run([sys.executable, os.path.join(HERE, "closing-hook.py")],
                       input='{"hook_event_name":"PreCompact","session_id":"s",'
                             '"cwd":"%s"}' % self.home,
                       text=True, env=env, capture_output=True)
        time.sleep(0.4)
        alive = child.poll() is None
        try:
            child.kill()
        except Exception:
            pass
        self.assertTrue(alive, "PreCompact killed the resident: it is not a close")

    def test_n21_the_closing_hook_does_not_drag_the_organ(self):
        """Importing 118 MB of model to send a signal would break organ 18's law
        from the place that runs most often."""
        source = io.open(os.path.join(HERE, "closing-hook.py"), encoding="utf-8").read()
        self.assertNotIn("neurons import", source)
        self.assertNotIn("import neurons", source)
        self.assertIn("signal.SIGTERM", source, "it does not kill the resident by signal")

    def test_n22_the_residents_life_is_declared_and_bounded(self):
        """Four hours by default, from a minute to a day if the Bearer orders
        it, and never a number without a ceiling."""
        house = os.path.join(self.chaos, "neurons")
        os.makedirs(house, exist_ok=True)
        os.environ["CHAOS_HOME"] = self.chaos
        n = self._neurons()
        self.assertEqual(n._life(), 14400, "the default life is not 4 h")
        io.open(os.path.join(house, "resident.life"), "w").write("30")
        self.assertEqual(n._life(), 1800, "it disobeys the life it is given")
        io.open(os.path.join(house, "resident.life"), "w").write("999999")
        self.assertEqual(n._life(), 86400, "it accepts a life with no ceiling")
        io.open(os.path.join(house, "resident.life"), "w").write("garbage")
        self.assertEqual(n._life(), 14400, "an unreadable life does not fall back")

    def test_n23_startup_does_not_warm_where_you_do_not_search(self):
        """MEASURED over the Bearer's 433 real sessions: only 19% consult my
        memory. Lighting in EVERY session would charge him ~200 MB four times
        out of five for nothing."""
        house = os.path.join(self.chaos, "neurons")
        os.makedirs(house, exist_ok=True)
        io.open(os.path.join(house, "model.onnx"), "w").write("x")
        env = dict(os.environ); env["HOME"] = self.home; env["CHAOS_HOME"] = self.chaos
        subprocess.run([sys.executable, os.path.join(HERE, "vigil-hook.py")],
                       input='{"hook_event_name":"SessionStart","cwd":"%s",'
                             '"session_id":"s"}' % self.home,
                       text=True, env=env, capture_output=True)
        time.sleep(0.8)
        self.assertFalse(os.path.exists(os.path.join(house, "resident.sock")),
                         "it warmed the resident in a territory nobody searches")

    def test_n24_startup_respects_the_revocation(self):
        """Even where the territory searches a lot, the Bearer's word rules:
        `resident auto no` and `neurons off` close this door."""
        source = io.open(os.path.join(HERE, "vigil-hook.py"),
                         encoding="utf-8").read()
        body = source.split("def warm_the_resident(")[1].split("\ndef ")[0]
        self.assertIn("RESIDENT-NO", body, "it ignores the resident's revocation")
        self.assertIn('"OFF"', body, "it ignores the neurons being switched off")
        self.assertIn("atexit", body,
                      "it lights during the hook instead of at the end: that costs")
        self.assertIn("< 3", body, "it demands no search history")

    def test_n25_startup_never_breaks_the_presence(self):
        """A hook that blows up erases the Bearer's whole Presence: this entire
        door lives inside a `try` that returns False."""
        source = io.open(os.path.join(HERE, "vigil-hook.py"),
                         encoding="utf-8").read()
        body = source.split("def warm_the_resident(")[1].split("\ndef ")[0]
        self.assertIn("except Exception:", body, "it can break the Presence")
        self.assertEqual(body.rstrip().splitlines()[-1].strip(), "return False",
                         "it does not degrade to False on the unexpected")

    def _startup_rule(self, searches, sessions):
        """Runs the hook with a REAL cwd (not a cwd inside the JSON: this hook
        decides by its directory, like the rest of it — my first test passed it
        through the event and that is why it always lit)."""
        import sqlite3 as _sq
        house = os.path.join(self.chaos, "neurons")
        os.makedirs(house, exist_ok=True)
        io.open(os.path.join(house, "model.onnx"), "w").write("x")
        run(self.home, "stats")                      # the Abyss is born
        ter = os.path.basename(self.home)
        con = _sq.connect(os.path.join(self.chaos, "abyss.db"))
        con.execute("INSERT OR REPLACE INTO meta VALUES (?,?)",
                    ("searches:" + ter, str(searches)))
        con.execute("INSERT OR REPLACE INTO meta VALUES (?,?)",
                    ("sessions:" + ter, str(sessions)))
        con.commit(); con.close()
        env = {k: v for k, v in os.environ.items() if k != "CLAUDE_PROJECT_DIR"}
        env["HOME"] = self.home; env["CHAOS_HOME"] = self.chaos
        log = os.path.join(house, "resident.log")
        subprocess.run([sys.executable, os.path.join(HERE, "vigil-hook.py")],
                       input='{"hook_event_name":"SessionStart","session_id":"s"}',
                       text=True, env=env, capture_output=True, cwd=self.home)
        time.sleep(0.6)
        # THE LOG IS WATCHED, NOT THE SOCKET. This test's model is fake (a file
        # with an "x"): the resident would start and die without binding
        # anything, so the socket would say "did not light" EVERY time and I
        # would have believed the rule worked when only the model was broken.
        # The log is created by `_launch` at the instant of birth, whatever
        # happens after.
        return os.path.exists(log)

    def test_n26_does_not_warm_where_you_barely_search(self):
        """`subagents`: 327 sessions and FOUR searches in its whole life. With my
        eyeballed threshold ("3 searches") I would have lit 202 times for
        nobody — measured over the Bearer's 433 real sessions."""
        self.assertFalse(self._startup_rule(4, 300),
                         "it lit with 4 searches across 300 sessions")

    def test_n27_does_not_warm_without_evidence(self):
        """Two searches do not declare a territory: the floor is three."""
        self.assertFalse(self._startup_rule(2, 1),
                         "it lit with only 2 searches")

    def test_n28_warms_where_you_really_search(self):
        """One search per session on average is the edge, and the edge lights."""
        self.assertTrue(self._startup_rule(6, 6),
                        "it did not light at a rate of exactly 1.0")

    def test_n29_the_threshold_is_not_a_bare_count(self):
        """A bare count glues itself to a fact of today: if `subagents` searched
        twice more, `count>=5` jumps from 8 vain lightings to 210. The RATE does
        not collapse. Both conditions must be there."""
        source = io.open(os.path.join(HERE, "vigil-hook.py"),
                         encoding="utf-8").read()
        body = source.split("def warm_the_resident(")[1].split("\ndef ")[0]
        self.assertIn("searches < 3", body, "it lost the evidence floor")
        self.assertIn("< 1.0", body, "it lost the rate: it is an eyeballed number again")
        self.assertIn("sessions:", body, "it does not count sessions: the rate would be fixed")


if __name__ == "__main__":
    unittest.main(verbosity=2)
