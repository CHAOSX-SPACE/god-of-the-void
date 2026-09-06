# -*- coding: utf-8 -*-
""""maw" — an organ of CHAOS's body.

Moved from the monolith by `partir-monolito.py`: not one line was
rewritten by hand. House rule: MODULES are imported here, never names —
the attribute resolves when CALLED, and that is why cycles between
organs are harmless (a cycle fixture in the forge proves it on three
systems). And nothing runs at import time: no constant in this module
may read the home, the environment or the disk (judge E1.4).
"""
import json, os, re, sys
from html.parser import HTMLParser as _HTMLParser
import home as _home
from chaos_body.core import text as _text


def input_poison(text):
    """What tried to give me ORDERS from inside what I devoured.
    Returns [(class, fragment)] — it never modifies the text: what is devoured
    is stored whole and MARKED. Mutilating the source destroys the evidence."""
    found = []
    for pattern, kind in _INPUT_POISON:
        m = pattern.search(text or "")
        if m:
            i = max(0, m.start() - 30)
            frag = re.sub(r"\s+", " ", (text[i:m.end() + 40])).strip()
            found.append((kind, frag[:120]))
    return found

def _ingest_pdf(path):
    """PDF → text per page. pypdf is pure Python with no dependencies; if it
    does not live here, I say so and do not invent the content."""
    try:
        from pypdf import PdfReader
    except ImportError:
        return None, "PDF with no extractor: `pip install pypdf` and devour again"
    try:
        reader = PdfReader(path)
    except Exception as e:
        return None, "unreadable PDF: {}".format(e)
    parts, mute = [], 0
    for i, page in enumerate(reader.pages, 1):
        try:
            txt = (page.extract_text() or "").strip()
        except Exception:
            txt = ""
        if txt:
            parts.append("## Page {}\n\n{}".format(i, txt))
        else:
            mute += 1
    if not parts:
        return None, "PDF with no extractable text ({} page(s)): it is image, not letter".format(mute)
    coverage = ("total" if not mute else
                "partial: {} of {} pages with no text (scanned image)"
                 .format(mute, len(reader.pages)))
    # A PDF's title is its NAME, not "Page 1": without this heading the
    # essence was born named after its first internal heading.
    name = os.path.splitext(os.path.basename(path))[0]
    return "# {}\n\n- **Source**: {} · **Pages**: {}\n\n{}".format(
        name, os.path.basename(path), len(reader.pages),
        "\n\n".join(parts)), coverage

def _ingest_openapi(path):
    """OpenAPI → a table of invocation: path, method, what it does, what it demands."""
    raw = _text.read_file(path)
    try:
        spec = json.loads(raw)
    except ValueError:
        try:
            import yaml
        except ImportError:
            return None, "YAML spec with no reader: `pip install pyyaml` (JSON I do swallow)"
        try:
            spec = yaml.safe_load(raw)
        except Exception as e:
            return None, "unreadable spec: {}".format(e)
    if not isinstance(spec, dict) or "paths" not in spec:
        return None, None                      # not a spec: let the normal path read it
    info = spec.get("info") or {}
    lin = ["# {} {}".format(info.get("title", "API"), info.get("version", "")),
           "", (info.get("description") or "").strip(), "",
           "## Invocation", "", "| method | path | what it does | demands |", "|---|---|---|---|"]
    n = 0
    for path, ops in sorted((spec.get("paths") or {}).items()):
        if not isinstance(ops, dict):
            continue
        for method, op in sorted(ops.items()):
            if method.lower() not in ("get", "post", "put", "patch", "delete"):
                continue
            op = op if isinstance(op, dict) else {}
            demands = [p.get("name") for p in (op.get("parameters") or [])
                     if isinstance(p, dict) and p.get("required")]
            if op.get("requestBody"):
                demands.append("body")
            lin.append("| `{}` | `{}` | {} | {} |".format(
                method.upper(), path,
                (op.get("summary") or op.get("operationId") or "")[:70],
                ", ".join(str(x) for x in demands) or "—"))
            n += 1
    schemas = sorted(((spec.get("components") or {}).get("schemas") or {}).keys())
    if schemas:
        lin += ["", "## Schemas", "", ", ".join("`%s`" % s for s in schemas[:40])]
    sec = sorted(((spec.get("components") or {}).get("securitySchemes") or {}).keys())
    if sec:
        lin += ["", "## Authentication", "", ", ".join("`%s`" % s for s in sec)]
    return "\n".join(lin), "total: {} endpoint(s), {} schema(s)".format(n, len(schemas))

class _Stripper(_HTMLParser):
    """HTML → text. `script`, `style` and `noscript` are not content: they are
    noise that also hides injections."""
    _MUTE = ("script", "style", "noscript", "svg", "head")

    def __init__(self):
        _HTMLParser.__init__(self, convert_charrefs=True)
        self.chunks, self.muted, self.title, self._in_title = [], 0, "", False

    def handle_starttag(self, tag, attrs):
        if tag in self._MUTE:
            self.muted += 1
        elif tag == "title":
            self._in_title = True
        elif tag in ("p", "div", "br", "li", "tr", "h1", "h2", "h3", "h4"):
            self.chunks.append("\n")

    def handle_endtag(self, tag):
        if tag in self._MUTE and self.muted:
            self.muted -= 1
        elif tag == "title":
            self._in_title = False

    def handle_data(self, d):
        if self._in_title:
            self.title += d
        elif not self.muted:
            self.chunks.append(d)

def _ingest_url(url):
    """URL → text. It goes out to the world, and what it brings back is DATA:
    the entry Purge reviews it afterwards, like everything else."""
    # The certificate store: on many macOS installs Python cannot see the
    # system one and every URL dies with CERTIFICATE_VERIFY_FAILED. `certifi`
    # is used if it lives here. What is NEVER done is turning verification off:
    # a god who swallows any certificate no longer knows who he is looking at.
    # E2.6 · urllib cuesta 15 ms al importar y solo lo usa esta funcion.
    from urllib.request import urlopen as _open_url, Request as _Request
    ctx = None
    try:
        import ssl, certifi
        ctx = ssl.create_default_context(cafile=certifi.where())
    except Exception:
        ctx = None
    try:
        request = _Request(url, headers={"User-Agent": "chaos/1.0 (+god-of-the-void)"})
        with _open_url(request, timeout=25, context=ctx) as r:
            rawb = r.read(4 * 1024 * 1024)
            kind = (r.headers.get("Content-Type") or "").lower()
    except Exception as e:
        hint = ""
        if "CERTIFICATE_VERIFY" in str(e):
            hint = ("  → your Python cannot see the certificate store: "
                    "`pip install certifi` (or run Install Certificates.command)")
        return None, "I could not look at {}: {}{}".format(url[:60], e, hint)
    text = rawb.decode("utf-8", "replace")
    if "html" in kind or text.lstrip()[:1] == "<":
        d = _Stripper()
        try:
            d.feed(text)
        except Exception:
            pass
        body = re.sub(r"\n{3,}", "\n\n",
                        re.sub(r"[ \t]+", " ", "".join(d.chunks))).strip()
        title = " ".join(d.title.split()) or url
        return "# {}\n\n- **Source**: {}\n\n{}".format(title, url, body), \
               "total: {} characters of stripped HTML".format(len(body))
    return "# {}\n\n- **Source**: {}\n\n{}".format(url.rsplit("/", 1)[-1] or url,
                                                     url, text), "total: plain text"

def _ingest(source):
    """(text, coverage) — the Maw decides by the SHAPE of the source."""
    if re.match(r"^https?://", source or ""):
        return _ingest_url(source)
    if source.lower().endswith(".pdf"):
        return _ingest_pdf(source)
    if source.lower().endswith((".json", ".yaml", ".yml")):
        text, coverage = _ingest_openapi(source)
        if text:
            return text, coverage
        if coverage:
            return None, coverage
    return _text.read_file(source), None

# ══ THE PURGE · DIRECTION 1: THE POISON THAT COMES IN ════════════════════
# The Purge watched what goes OUT (keys, tokens, PII). What comes IN — text
# giving me ORDERS from inside what I devour — lived only in my doctrine: half
# of my only always-awake organ was asleep. OWASP has ranked prompt injection
# as LLM01 for two editions running.
#
# Nothing is censored here: what is devoured is stored WHOLE, because mutilating
# a source destroys the evidence. It is MARKED. Marked data is still data; data
# pretending to be an order is not.
_INPUT_POISON = (
    # «IGNORA TODO LO ANTERIOR» is the textbook example and my first pattern
    # missed it: it demanded the word "instrucciones". The Crucible found it
    # by counting how many injection payloads I recognised — two out of three.
    (re.compile(r"(?i)\b(ignora\w*|olvida\w*|descarta\w*|ignore|disregard|forget)"
                r"\s+(todo|todas?|all|any|the)?\s*(lo\s+|las\s+|los\s+)?"
                r"(anterior\w*|previo\w*|previous|prior|above|instruc\w*|"
                r"system\s+prompt|reglas)"),
     "order aimed at the model"),
    (re.compile(r"(?i)\b(ahora eres|a partir de ahora eres|you are now|"
                r"from now on you are|act as|pretend to be)\b"),
     "identity reassignment"),
    (re.compile(r"(?im)^\s*(system|assistant|sistema)\s*:"),
     "faked role"),
    (re.compile(r"(?i)\b(importante|important|urgent|urgente)\s*:?\s*"
                r"(you\s+(must|should|need)|debes|tienes que)\b"),
     "aimed urgency"),
    # Mind the trailing \b: "autoriz" followed by "ó" has NO word boundary
    # (ó is a letter), so the pattern died in Spanish. That is fault #483 in a
    # different coat — hence the \w* on the prefixes.
    (re.compile(r"(?i)\b(el usuario ya (autoriz\w*|aprob\w*|consinti\w*)|"
                r"the user (has )?(already )?(authorized|approved|consented)|"
                r"admin mode|developer mode|modo administrador)"),
     "faked authority"),
    # NOT every invisible character is poison: U+200D joins family emoji
    # (👨‍👩‍👧‍👦) and U+200E/200F order Arabic and Hebrew. Flagging them turned
    # legitimate text into a suspect — caught by the Crucible's «emoji»
    # payload. What remains are the ones that only serve to HIDE: zero-width
    # space, word joiner and the ones that REVERSE reading direction. The BOM
    # is dropped when the file is read: flagging it accused a ghost (Crucible).
    (re.compile(r"[\u200b\u2060-\u2064\u202a-\u202e]"),
     "hidden text (invisible characters)"),
    (re.compile(r"(?is)<!--(?:(?!-->).){0,300}\b(ai|llm|assistant|claude|gpt|"
                r"model|modelo)\b(?:(?!-->).){0,300}-->"),
     "comment aimed at a model"),
    (re.compile(r"(?i)(display\s*:\s*none|font-size\s*:\s*0|color\s*:\s*#fff(fff)?\s*;"
                r"[^}]*background[^}]*#fff)"),
     "text hidden with CSS"),
    (re.compile(r"(?i)\b(exfiltra|exfiltrate|env[íi]a (tus|las|the) (claves|llaves|"
                r"secretos|keys|secrets)|send (me )?(the|your) (contents|secrets|keys))\b"),
     "exfiltration instruction"),
)
