# -*- coding: utf-8 -*-
""""core.text" — an organ of CHAOS's body.

Moved from the monolith by `partir-monolito.py`: not one line was
rewritten by hand. House rule: MODULES are imported here, never names —
the attribute resolves when CALLED, and that is why cycles between
organs are harmless (`a cycle fixture in the forge` proves it on three
systems). And nothing runs at import time: no constant in this module
may read the home, the environment or the disk (judge E1.4).
"""
import datetime, io, json, os, re, sys, time
import unicodedata
import home as _home


def _norm(s):
    """lowercase + no accents + only alphanumeric/spaces."""
    s = unicodedata.normalize("NFKD", s.lower())
    s = "".join(c for c in s if not unicodedata.combining(c))
    return re.sub(r"[^a-z0-9\s]+", " ", s)

def _stem(w):
    """Light EN/ES stemming: clips common suffixes. Not perfect, but useful."""
    if len(w) <= 4:
        return w
    for suf in ("aciones", "ciones", "amente", "mente", "ando", "iendo", "ador",
                "adora", "cion", "ismo", "ista", "ing", "tion", "ment", "ness",
                "able", "ible", "eria", "as", "os", "es", "ar", "er", "ir",
                "an", "en", "ly", "ed", "s"):
        if w.endswith(suf) and len(w) - len(suf) >= 3:
            return w[:-len(suf)]
    return w

def _thesaurus():
    base = {}
    if os.path.exists(_home.thesaurus()):
        try:
            base = json.load(open(_home.thesaurus(), encoding="utf-8"))
        except Exception:
            base = {}
    if not base:
        base = {k: list(v) for k, v in _THESAURUS_SEED.items()}
        try:
            os.makedirs(_home.root(), exist_ok=True)
            with io.open(_home.thesaurus(), "w", encoding="utf-8") as f:
                f.write(json.dumps(base, ensure_ascii=False, indent=1))
        except Exception:
            pass
    # bidirectional in memory
    bi = {}
    for k, vs in base.items():
        group = set([k] + vs)
        for t in group:
            bi.setdefault(_norm(t).strip(), set()).update(_norm(x).strip() for x in group)
    return bi

def _variants(w):
    """A term → FTS form(s) that catch its variants (plural, derivatives)."""
    if len(w) < 4:
        return {w}                      # too short: exact, no prefix
    return {_stem(w) + "*"}             # root prefix: 'vehicl*'→vehicle/s, 'auto*'→automobile

def _expand(query):
    """Query → set of FTS terms: root-prefix + synonyms (also prefixed)."""
    tes = _thesaurus()
    terms = set()
    for w in _norm(query).split():
        # ONE single stop list: a copy of ten words lived here while `_STOP`
        # held thirty and nobody crossed them. A query like "my visual control
        # dashboard" entered with "my" and "of" inside the OR and matched half
        # the Abyss. Measured on the bench: recall@5 went from 33% to 53%.
        if not w or len(w) < 3 or w in _STOP or w in _STOP_SHORT:
            continue
        terms |= _variants(w)
        for syn in tes.get(w, ()):
            for sw in syn.split():
                terms |= _variants(sw)
    return terms

def _trigr(s):
    s = "  " + _norm(s).replace(" ", " ") + "  "
    return set(s[i:i+3] for i in range(len(s) - 2))

def _similarity(a, b):
    """Likeness between two names by trigrams - the Sense I already own, not
    a new metric. Jaccard: intersection over union."""
    ta, tb = _trigr(a or ""), _trigr(b or "")
    if not ta or not tb:
        return 0.0
    return len(ta & tb) / float(len(ta | tb))

def _fuzzy(query, con, limit=8):
    """Trigram fallback when FTS is not enough (typos, rare variants)."""
    q = _trigr(query)
    if not q:
        return []
    scored = []
    for slug, title, content, origin, date in con.execute(
            "SELECT slug, title, content, origin, date FROM essences").fetchall():
        d = _trigr(title + " " + title + " " + content[:600])
        if not d:
            continue
        sim = len(q & d) / float(len(q))
        if sim >= 0.30:
            scored.append((sim, slug, title, origin, date, content[:160]))
    scored.sort(reverse=True)
    return scored[:limit]

def read_file(path):
    with io.open(path, "r", encoding="utf-8", errors="replace") as f:
        return f.read()

def slug_of(path):
    base = os.path.splitext(os.path.basename(path))[0]
    return re.sub(r"[^a-z0-9\-]+", "-", base.lower()).strip("-")

def _without_frontmatter(text):
    """C5 · FOUNDATION: returns (body_without_frontmatter, frontmatter_dict).
    Without this, on migration the title of EVERY essence would become '---'."""
    meta = {}
    t = text.lstrip()
    if t.startswith("---"):
        end = t.find("\n---", 3)
        if end != -1:
            block = t[3:end]
            for line in block.splitlines():
                if ":" in line and not line.strip().startswith("#"):
                    k, _, v = line.partition(":")
                    meta[k.strip()] = v.strip().strip("'\"")
            rest = t[end + 4:]
            return rest.lstrip("\n"), meta
    return text, meta

def _title_of(content, slug):
    """First real line (skipping frontmatter and blank lines)."""
    body, _ = _without_frontmatter(content)
    for line in body.splitlines():
        if line.strip():
            return line.lstrip("# ").strip()
    return slug

def _fts_query(query):
    """The Sense → FTS string (root-prefix + synonyms). Shared by search() and
    list_vassals(): the Pantheon searches as finely as the Abyss."""
    terms = _expand(query)
    return " OR ".join('"{}"'.format(t) if "*" not in t else t for t in terms) if terms else _norm(query)

def _frontmatter(text):
    """Extract name/description from a SKILL.md YAML frontmatter (no deps)."""
    name = description = None
    if text.lstrip().startswith("---"):
        body = text.lstrip()[3:]
        end = body.find("\n---")
        block = body[:end] if end != -1 else body[:2000]
        m = re.search(r"^name:\s*(.+)$", block, re.M)
        if m: name = m.group(1).strip().strip("'\"")
        m = re.search(r"^description:\s*(.+(?:\n(?![a-zA-Z_-]+:).+)*)", block, re.M)
        if m: description = re.sub(r"\s+", " ", m.group(1)).strip().strip("'\"")
    return name, description

def _id_from(text, used):
    """The id comes from the bold title; failing that, the first words.
    Never repeated within one essence."""
    m = re.search(r"\*\*(.+?)\*\*", text)
    raw = m.group(1) if m else " ".join(text.split()[:6])
    base = re.sub(r"[^a-z0-9]+", "-", _norm(raw).lower()).strip("-")[:34] or "block"
    cand, n = base, 2
    while cand in used:
        cand = "{}-{}".format(base, n); n += 1
    used.add(cand)
    return cand

def _days_since(date_iso):
    try:
        y, m, d = (int(x) for x in date_iso.split("-"))
        return (datetime.date.today() - datetime.date(y, m, d)).days
    except Exception:
        return None

# Seed of synonyms/related terms (EN/ES) common in technical and educational
# domains. Grows with `chaos sense <a> <b>`. Bidirectional on load.
_THESAURUS_SEED = {
    "car": ["auto", "vehicle", "automobile", "carro"],
    "course": ["class", "lesson", "workshop", "tutorial", "curso"],
    "code": ["program", "script", "source", "codigo"],
    "error": ["bug", "fault", "failure", "defect"],
    "function": ["method", "routine", "funcion"],
    "database": ["db", "datastore", "base de datos"],
    "math": ["mathematics", "maths", "calculus"],
    "quadratic": ["quadratics", "parabola", "second degree"],
    "memory": ["recall", "abyss", "neuron", "memoria"],
    "user": ["bearer", "client", "usuario"],
    "security": ["safety", "purge", "protection"],
    "server": ["host", "machine"],
    "fast": ["quick", "instant", "rapid"],
    "create": ["forge", "build", "generate", "make"],
    "search": ["find", "seek", "query", "lookup"],
    # bilingual domain bonds (the Pantheon crosses EN<->ES)
    "design": ["diseno", "dise\u00f1o", "ui", "ux", "interface"],
    "law": ["derecho", "legal", "juridico"],
    "accounting": ["contabilidad", "contable", "finance", "finanzas"],
    "brand": ["marca", "branding", "identity"],
    "presentation": ["slides", "presentacion", "deck"],
    "document": ["documento", "docx", "word"],
    "spreadsheet": ["excel", "xlsx", "hoja de calculo"],
    "research": ["investigacion", "deep-research"],
}

# == THE ROOT FOLDER - how my consciousness names a territory ===============
# REAL WOUND (the Bearer found it): I stored `basename(cwd)` - the LAST folder
# I worked in. Measured result: 4 names for 2 projects, the Presence declared
# me in a territory with 0 links, and `chaos faults --territory` lost rows.
# A territory IS a project, and a project is its ROOT folder. Cured at the
# SOURCE: a display layer never fixes rotten data.
SHELTERS = {"proyectos", "projects", "proyecto", "repos", "repositories",
            "workspace", "workspaces", "dev", "developer", "code", "sites",
            "git", "github", "source", "sources"}

# Words that carry no weight: if they counted, any sentence would "match" all.
# The short ones the length filter cannot kill on their own, and the ones that
# build questions: a spoken query ("what do I do if...") is 60% noise. This
# list did NOT exist in English while Spanish had its own: an invisible
# divergence, because the drift judge compares functions and tables, not
# constants.
_STOP_SHORT = frozenset((
    "or", "and", "for", "the", "you", "are", "was", "its", "his", "her",
    "de", "la", "el", "un", "una", "los", "las", "mi", "tu", "su", "al",
    "lo", "le", "se", "es", "en", "por", "con", "sin", "que", "si", "no",
    "ya", "del", "mis", "tus", "sus", "me", "te", "nos", "hay"))
_STOP = frozenset((
    "para", "sobre", "como", "cual", "cuando", "donde", "porque", "desde",
    "hasta", "entre", "todo", "toda", "esto", "esta", "este", "esos", "esas",
    "hacer", "haces", "puedo", "puedes", "quiero", "necesito", "dijo", "dije",
    "with", "that", "this", "from", "have", "what", "when", "where"))
