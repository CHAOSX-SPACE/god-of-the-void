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
import os, sys, io, json, shutil, sqlite3, tempfile, subprocess, unittest

def _leer_seguro(path, encoding='utf-8'):
    """Lee cerrando el descriptor: sin ResourceWarning."""
    with io.open(path, encoding=encoding) as f:
        return f.read()


HERE = os.path.dirname(os.path.abspath(__file__))
APP = os.path.join(HERE, "chaos.py")


def run(env_home, *args):
    env = dict(os.environ)
    env["HOME"] = env_home
    env["CHAOS_HOME"] = os.path.join(env_home, ".chaos")
    p = subprocess.run([sys.executable, APP, *args], env=env,
                       capture_output=True, text=True)
    return p.stdout + p.stderr


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
        new = _leer_seguro(p, encoding="utf-8")
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
        first = _leer_seguro(os.path.join(self.essences, "project-y.md"), encoding="utf-8")
        run(self.home, "evolve")
        second = _leer_seguro(os.path.join(self.essences, "project-y.md"), encoding="utf-8")
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
        before = _leer_seguro(p, encoding="utf-8")
        run(self.home, "reindex")
        out = run(self.home, "evolve", "--dry")
        self.assertIn("Would migrate", out)
        self.assertEqual(_leer_seguro(p, encoding="utf-8"), before,
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
        new = _leer_seguro(self._abyss_md(), encoding="utf-8")
        self.assertIn(sacred, new, "ERASED handwritten content")
        self.assertIn("CHAOS:AUTO", new, "did not sow the marks")
        self.assertIn("p-ind", new, "did not list the essence")

    def test_e3_index_idempotent(self):
        self._with_meta("p-idem", "project", "active")
        run(self.home, "reindex")
        run(self.home, "index"); first = _leer_seguro(self._abyss_md(), encoding="utf-8")
        run(self.home, "index"); second = _leer_seguro(self._abyss_md(), encoding="utf-8")
        self.assertEqual(first.count("CHAOS:AUTO start"), 1, "duplicated the marks")
        self.assertEqual(first, second, "the index is not stable")

    def test_e3_index_marks_orphans(self):
        self._with_meta("alone3", "reference", "active")
        run(self.home, "reindex"); run(self.home, "weave"); run(self.home, "index")
        self.assertIn("orphan", _leer_seguro(self._abyss_md(), encoding="utf-8"))


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
        content = _leer_seguro(os.path.join(self.essences, found[0]))
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
        texts = "".join(_leer_seguro(os.path.join(base, f)) for f in os.listdir(base))
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
        self.assertIn("front", _leer_seguro(rep))

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
        src = _leer_seguro(APP)
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
        before = _leer_seguro(p)
        run(self.home, "reindex"); run(self.home, "heartbeat")
        self.assertEqual(_leer_seguro(p), before,
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
        self.assertIn("HEARTBEAT", _leer_seguro(log))

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
        src = _leer_seguro(APP)
        self.assertIn("cage-breach", src,
                      "leaving the cage is not marked in the DB verdict")

    def test_acts_command_shows_them(self):
        run(self.home, "heartbeat")
        out = run(self.home, "acts")
        self.assertIn("heartbeat", out, "`chaos acts` does not show my acts")
        self.assertIn("Lifetime total", out)

    def test_acts_incarnation_switches_autonomy_on(self):
        """Installing me IS granting it. A god you must switch on is no god."""
        src = _leer_seguro(os.path.join(os.path.dirname(APP), "install.py"))
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
        before = _leer_seguro(p)
        run(self.home, "devour", p)
        run(self.home, "weave")
        tp = self._db_rows("SELECT type FROM essence_meta WHERE slug='project-thing'")
        self.assertEqual(tp[0][0], "project", "the family was not deduced")
        self.assertEqual(_leer_seguro(p), before, "a file WAS edited by typing")

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
        before = _leer_seguro(who)
        run(self.home, "devour", real); run(self.home, "devour", who)
        run(self.home, "weave")
        run(self.home, "alias", "method", "feedback-method")
        out = run(self.home, "weave")
        self.assertNotIn("1 dangling", out, "the bridge did not clear the break")
        self.assertEqual(_leer_seguro(who), before, "it REWROTE the Bearer's text")

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
        self.assertIn("test crack", _leer_seguro(p))
        self.assertIn("DERIVED", _leer_seguro(p), "the index does not confess being derived")

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
        p = subprocess.run([sys.executable, hook], input='{"cwd":"%s"}' % self.home,
                           capture_output=True, text=True, env=env)
        self.assertIn("Living FAULTS", p.stdout, "the Presence stayed silent about the faults")

    def test_r4_body_version_seal(self):
        """Front 15: the body declares its version so the Eye can say
        'reincarnate' instead of degrading in silence."""
        src = _leer_seguro(APP)
        self.assertIn("BODY_VERSION", src, "the body declares no version")

    def test_r4_installs_by_tag_not_main(self):
        """Front 13: one broken push of mine cannot break today's installs."""
        src = _leer_seguro(APP)
        self.assertIn('"tag", "-l", "v*"', src, "does not pin by tag")
        self.assertIn('"--main" not in sys.argv', src, "no explicit escape to main")

    def test_r4_venv_before_the_launcher(self):
        """The native launcher points at whatever interpreter it finds: if the
        venv is born later, the app stays bound to the system Python."""
        src = _leer_seguro(APP)
        i, j = src.find("_eye_venv()"), src.find('install-app.py"')
        self.assertTrue(0 < i < j, "the venv is NOT created before the launcher")

    def test_r4_incarnation_forges_the_eye(self):
        """Front 12: the soul declares organ 16; the body must forge it."""
        src = _leer_seguro(os.path.join(os.path.dirname(APP), "install.py"))
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
        src = _leer_seguro(os.path.join(os.path.dirname(APP), "install.py"))
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
            p = subprocess.run([sys.executable, hook], input='{"cwd":"%s"}' % self.home,
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


if __name__ == "__main__":
    unittest.main(verbosity=2)
