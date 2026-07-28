#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
EL OJO — servidor del dashboard de CHAOS · stdlib puro, cero dependencias.

Leyes del nervio:
  · 127.0.0.1 y puerto alto aleatorio. JAMÁS 0.0.0.0.
  · Token aleatorio por arranque: sin él, nadie lee.
  · Toda LÓGICA vive en chaos.py (~/.chaos/bin/): aquí no se duplica una
    sola regla — lecturas simples por SQL vía chaos.db(), lo demás por CLI.
  · CSP estricta: sin CDN, sin fuentes remotas, sin fetch externo.
  · Muere con la ventana: cerrar = servidor abajo, token quemado.
"""
import os, sys, json, io, re, secrets, sqlite3, threading, subprocess
import importlib.util
import webbrowser
from http.server import HTTPServer, BaseHTTPRequestHandler
from urllib.parse import urlparse, parse_qs

AQUI = os.path.dirname(os.path.abspath(__file__))
def _casa():
    """La casa del dios: la MISMA verdad que chaos.py, sin importarlo (los
    hooks deben ser instantáneos). Env > elección del Portador > defecto."""
    v = os.environ.get("CHAOS_HOME")
    if v:
        return os.path.expanduser(v)
    try:
        with open(os.path.join(os.path.expanduser("~"), ".claude", "chaos-home"),
                  encoding="utf-8") as f:
            e = f.read().strip()
        if e:
            return os.path.expanduser(e)
    except OSError:
        pass
    return os.path.join(os.path.expanduser("~"), ".chaos")


CHAOS_HOME = _casa()
CHAOS_APP = os.path.join(CHAOS_HOME, "bin", "chaos.py")
TOKEN = secrets.token_urlsafe(18)

# ── chaos.py ES el backend: se importa, no se reimplementa ────────────────
spec = importlib.util.spec_from_file_location("chaos", CHAOS_APP)
chaos = importlib.util.module_from_spec(spec)
spec.loader.exec_module(chaos)

# Edición: si la BD inglesa existe y la española no, hablamos con esa.
ES = os.path.exists(os.path.join(CHAOS_HOME, "abismo.db")) or \
     not os.path.exists(os.path.join(CHAOS_HOME, "abyss.db"))
T = {  # nombres de tablas/columnas por edición — el único mapa permitido
    "fallas": "fallas" if ES else "faults",
    "f_cols": ("titulo,sintoma,causa,cura,leccion,territorio,fecha,estado,repeticiones,ultima"
               if ES else "title,symptom,cause,cure,lesson,territory,date,state,repeats,last"),
    "viva": "viva" if ES else "alive",
    "actos": "actos_autonomos" if ES else "autonomous_acts",
    "a_cols": ("fecha,tipo,accion,detalle,creados,alterados,hallazgos,duracion,veredicto,maquina"
               if ES else "date,kind,action,detail,created,altered,findings,duration,verdict,machine"),
    "notas": "notas" if ES else "notes",
    "n_ter": "territorio" if ES else "territory",
    "cmd_buscar": "buscar" if ES else "search",
    "cmd_auditar": "auditar" if ES else "audit",
}


def _cli(*args):
    """Lo que tiene lógica se pide al CLI: cero duplicación, una sola verdad."""
    try:
        p = subprocess.run([sys.executable, CHAOS_APP, *args],
                           capture_output=True, text=True, timeout=30)
        return p.stdout
    except Exception as e:
        return "[error] {}".format(e)


def _q(sql, args=()):
    con = chaos.db()
    try:
        return [list(r) for r in con.execute(sql, args).fetchall()]
    except sqlite3.Error as e:
        return {"error": str(e)}
    finally:
        con.close()


def _uno(sql, args=(), defecto=0):
    r = _q(sql, args)
    return r[0][0] if isinstance(r, list) and r and r[0] else defecto


# ── endpoints ─────────────────────────────────────────────────────────────
OJO_EXIGE = 5      # versión de cuerpo que este Ojo necesita para todo


def _desajuste_cuerpo():
    """Frente 15. Si el cuerpo va por detrás, no se calla: se dice con el
    comando exacto. Un puente getattr que degrada en silencio es un fallo
    que el Portador descubre por accidente."""
    v = getattr(chaos, "VERSION_CUERPO", None) or getattr(chaos, "BODY_VERSION", None)
    if v is None:
        return {"cuerpo": 0, "exige": OJO_EXIGE,
                "aviso": "el cuerpo no declara versión — reencárnalo: python3 install.py"}
    if v < OJO_EXIGE:
        return {"cuerpo": v, "exige": OJO_EXIGE,
                "aviso": "el cuerpo es v{} y el Ojo espera v{} — reencarna: "
                         "python3 ~/.claude/skills/chaos/cuerpo/install.py".format(v, OJO_EXIGE)}
    return None


def api_pulso():
    esencias = _uno("SELECT COUNT(*) FROM esencias") if ES else _uno("SELECT COUNT(*) FROM essences")
    meta_t = "meta_esencia" if ES else "essence_meta"
    res_col = "residente" if ES else "resident"
    enl_t = "enlaces" if ES else "links"
    dst = "destino" if ES else "target"
    org = "origen" if ES else "source"
    return {
        "esencias": esencias,
        "residentes": _uno("SELECT COUNT(*) FROM {} WHERE {}=1".format(meta_t, res_col)),
        "enlaces": _uno("SELECT COUNT(*) FROM {}".format(enl_t)),
        "rotos": _uno("SELECT COUNT(*) FROM {e} WHERE NOT EXISTS("
                      "SELECT 1 FROM {m} m WHERE m.slug={e}.{d})".format(e=enl_t, m=meta_t, d=dst)),
        "huerfanas": _uno("SELECT COUNT(*) FROM {m} m WHERE NOT EXISTS(SELECT 1 FROM {e} "
                          "WHERE {e}.{d}=m.slug OR {e}.{o}=m.slug)".format(m=meta_t, e=enl_t, d=dst, o=org)),
        "chispas": _uno("SELECT COUNT(*) FROM {}".format(T["notas"])),
        "hambres": _uno("SELECT COUNT(*) FROM hambres" if ES else "SELECT COUNT(*) FROM hungers"),
        "fallas_total": _uno("SELECT COUNT(*) FROM {}".format(T["fallas"])),
        "fallas_vivas": _uno("SELECT COUNT(*) FROM {} WHERE {}=?".format(
            T["fallas"], "estado" if ES else "state"), (T["viva"],)),
        "reincidencias": _uno("SELECT COALESCE(SUM(CAST({} AS INT)),0) FROM {}".format(
            "repeticiones" if ES else "repeats", T["fallas"])),
        "actos": _uno("SELECT COUNT(*) FROM {}".format(T["actos"])),
        "ultimo_acto": _uno("SELECT {} FROM {} ORDER BY id DESC LIMIT 1".format(
            "fecha" if ES else "date", T["actos"]), defecto=None),
        "cuerpo": _desajuste_cuerpo(),
    }


def api_fallas():
    filas = _q("SELECT rowid,{} FROM {} ORDER BY "
               "CAST({} AS INT) DESC, rowid DESC".format(
                   T["f_cols"], T["fallas"], "repeticiones" if ES else "repeats"))
    claves = ["id"] + T["f_cols"].split(",")
    return [dict(zip(claves, f)) for f in filas] if isinstance(filas, list) else filas


def api_actos(n=50):
    filas = _q("SELECT id,{} FROM {} ORDER BY id DESC LIMIT ?".format(
        T["a_cols"], T["actos"]), (n,))
    claves = ["id"] + T["a_cols"].split(",")
    return [dict(zip(claves, f)) for f in filas] if isinstance(filas, list) else filas


# ── UNA SOLA VERDAD: la raíz de proyecto vive en chaos.py ─────────────────
# Falla propia cazada en la auto-vigilia: el Ojo REIMPLEMENTABA
# raiz_proyecto/COBIJOS, violando mi ley fundacional ("chaos.py ES el
# backend, cero duplicación"). Si la regla evoluciona en la conciencia, la
# vitrina divergiría en silencio. El puente admite ambas ediciones.
RAIZ_PROYECTO = getattr(chaos, "raiz_proyecto", None) or getattr(chaos, "project_root", None)
NOMBRE_TERRITORIO = getattr(chaos, "nombre_territorio", None) or getattr(chaos, "territory_name", None)
COBIJOS = getattr(chaos, "COBIJOS", None) or getattr(chaos, "SHELTERS", set())
if RAIZ_PROYECTO is None:      # cuerpo viejo sin la función: se declara, no se inventa
    def RAIZ_PROYECTO(r): return r
    def NOMBRE_TERRITORIO(r): return os.path.basename((r or "").rstrip(os.sep)) or None


def raiz_proyecto(ruta):
    return RAIZ_PROYECTO(ruta)


def nombre_territorio(ruta):
    return NOMBRE_TERRITORIO(ruta)


def _canon(t):
    """Clave canonica de un territorio. Sin esto el mismo proyecto aparecia
    DOS veces: el rastro lo llama "DIOS DEL VACIO" (nombre de carpeta) y las
    fallas "dios-del-vacio" (slug). Un territorio, una ficha."""
    return re.sub(r"[^a-z0-9]+", "-", (t or "").lower()).strip("-") or "?"


def _mejor_nombre(a, b):
    """Entre dos formas del mismo territorio gana la legible por humanos:
    la que tiene espacios o mayusculas (nombre de carpeta) sobre el slug."""
    if not a:
        return b
    if not b:
        return a
    pa = (" " in a) + any(c.isupper() for c in a)
    pb = (" " in b) + any(c.isupper() for c in b)
    return a if pa >= pb else b


def _mapa_raices():
    """Raices de proyecto: {clave_canonica: (nombre, ruta)} + indice de las
    subcarpetas de cada una, para plegar etiquetas sueltas.

    Se descubren por DOS vias, y la segunda importa: no basta con lo que el
    rastro visito. Los proyectos hermanos del mismo cobijo (~/…/proyectos/)
    tambien cuentan, porque una falla puede nombrar un proyecto en el que
    todavia no se ha trabajado desde aqui.
    """
    raices, sub, cobijos = {}, {}, set()
    for f in _leer_rastro():
        cwd = f["cwd"]
        if not cwd:
            continue
        r = raiz_proyecto(cwd)
        n = nombre_territorio(cwd)
        if not n:
            continue
        raices[_canon(n)] = (n, r)
        if r and os.path.basename(os.path.dirname(r)).lower() in COBIJOS:
            cobijos.add(os.path.dirname(r))
        if r and cwd.startswith(r):
            for seg in cwd[len(r):].strip(os.sep).split(os.sep):
                if seg:
                    sub.setdefault(_canon(seg), _canon(n))

    # los hermanos del cobijo: el disco es la verdad, no solo lo pisado
    for c in cobijos:
        try:
            for e in os.scandir(c):
                if e.is_dir() and not e.name.startswith("."):
                    raices.setdefault(_canon(e.name), (e.name, e.path))
        except OSError:
            pass

    # y las carpetas REALES de cada proyecto (2 niveles), no solo las visitadas
    for k, (n, r) in list(raices.items()):
        if not r or not os.path.isdir(r):
            continue
        try:
            for e in os.scandir(r):
                if e.is_dir() and not e.name.startswith("."):
                    sub.setdefault(_canon(e.name), k)
        except OSError:
            pass
    return raices, sub


def _plegar(nombre, raices, sub):
    """Una etiqueta suelta cae en su proyecto por, en este orden:
      1. coincidir con la raiz          (dios-del-vacio -> DIOS DEL VACIO)
      2. ser una carpeta de dentro      (chaos-ojo      -> DIOS DEL VACIO)
      3. ser una EXTENSION del nombre   (chaosx-web     -> chaosx)
    El caso 3 lo enseño el Portador: yo mismo etiqueto fallas con nombres que
    no son carpetas (chaosx-web), y quedaban sueltas como si fueran proyectos
    aparte. Gana la raiz MAS LARGA: radar-1k no cae dentro de radar.
    Si no pertenece a nada conocido, se respeta como territorio propio."""
    k = _canon(nombre)
    if k in raices:
        return k, raices[k][0]
    if k in sub and sub[k] in raices:
        return sub[k], raices[sub[k]][0]
    mejor = None
    for rk in raices:
        if k.startswith(rk + "-") and (mejor is None or len(rk) > len(mejor)):
            mejor = rk
    if mejor:
        return mejor, raices[mejor][0]
    return k, nombre


def api_territorios():
    """Un PROYECTO = una tarjeta. Todo lo de dentro se pliega hacia su raiz."""
    ter_f = "territorio" if ES else "territory"
    est = "estado" if ES else "state"
    raices, sub = _mapa_raices()
    out = {}

    def toca(nombre, campo, valor, ruta=None):
        if not nombre:
            return
        k, visible = _plegar(nombre, raices, sub)
        d = out.setdefault(k, {"territorio": visible, "ruta": None, "fallas": 0,
                               "fallas_vivas": 0, "chispas": 0, "cronica": 0,
                               "obras": 0, "alias": set()})
        d["territorio"] = visible
        d["alias"].add(nombre)
        if ruta and not d["ruta"]:
            d["ruta"] = ruta
        d[campo] = d.get(campo, 0) + (valor or 0)

    for t, viv in _q("SELECT {c}, SUM(CASE WHEN {e}=? THEN 1 ELSE 0 END) FROM {f}"
                     " GROUP BY {c}".format(c=ter_f, e=est, f=T["fallas"]), (T["viva"],)) or []:
        toca(t, "fallas_vivas", viv)
    for t, n in _q("SELECT {c}, COUNT(*) FROM {f} GROUP BY {c}".format(
            c=ter_f, f=T["fallas"])) or []:
        toca(t, "fallas", n)
    for t, n in _q("SELECT {c}, COUNT(*) FROM {n} GROUP BY {c}".format(
            c=T["n_ter"], n=T["notas"])) or []:
        toca(t, "chispas", n)
    bit = "bitacora" if ES else "logbook"
    for t, n in _q("SELECT {c}, COUNT(*) FROM {b} GROUP BY {c}".format(
            c="territorio" if ES else "territory", b=bit)) or []:
        toca(t, "cronica", n)

    obras = {}
    for f in _leer_rastro():
        n = nombre_territorio(f["cwd"])
        if n:
            obras.setdefault(n, [0, raiz_proyecto(f["cwd"])])[0] += 1
    for n, (c, r) in obras.items():
        toca(n, "obras", c, r)

    res = []
    for k, d in out.items():
        d = dict(d)
        d["alias"] = sorted(d.pop("alias"))
        d["clave"] = k
        res.append(d)
    # los proyectos con mas vida primero; lo sano abajo
    res.sort(key=lambda x: (-x["fallas_vivas"], -x["obras"], x["territorio"].lower()))
    return res


def api_buscar(q):
    """La búsqueda ES la del dios: se delega al CLI. Cero lógica duplicada."""
    if not q or len(q) > 200:
        return {"lineas": []}
    return {"lineas": _cli(T["cmd_buscar"], q, "--breve").splitlines()[:30]}


# ══ EL DIAGNÓSTICO · la salud REAL de la memoria ══════════════════════════
# El Portador: "solo dice un monton de cosas que no es nada intuitivo".
# Tenia razon: volcar la salida de `auditar` no es diagnosticar. Aqui cada
# dimension declara SU FORMULA, entrega un puntaje 0-100 y — lo que importa —
# la LISTA EXACTA de lo que le falta para llegar a 100. El porcentaje que no
# se cubre siempre se puede desglosar: si no puedo nombrar el problema, no
# tengo derecho a puntuarlo.
import datetime as _dt


def _dias(iso):
    """Dias desde una fecha ISO. None si no se puede leer."""
    if not iso:
        return None
    try:
        return (_dt.datetime.now() - _dt.datetime.fromisoformat(str(iso)[:19])).days
    except Exception:
        return None


def _t(es_txt, en_txt):
    """El motor de Salud TIENE edición: que hable la suya. Falla real hallada
    probando el Ojo contra un cuerpo inglés: dimensiones, fórmulas y detalles
    salían en español. Una interfaz bilingüe con el motor monolingüe miente a
    medias."""
    return es_txt if ES else en_txt


def _dim(clave, titulo, peso, puntaje, formula, problemas, detalle=""):
    return {"clave": clave, "titulo": titulo, "peso": peso,
            "puntaje": max(0, min(100, round(puntaje))), "formula": formula,
            "detalle": detalle, "problemas": problemas[:200],
            "n_problemas": len(problemas)}


def _d_tejido():
    """Un grafo con enlaces rotos y esencias sueltas es memoria fracturada."""
    meta = "meta_esencia" if ES else "essence_meta"
    enl = "enlaces" if ES else "links"
    org, dst = ("origen", "destino") if ES else ("source", "target")
    slugs = set(r[0] for r in (_q("SELECT slug FROM {}".format(meta)) or []))
    # EL PUENTE DE LOS ALIAS: `tejer` decía 3 rotos y la Salud 12 — dos
    # verdades sobre el mismo dato. Un destino con alias NO cuelga.
    puentes = dict(_q("SELECT alias, slug FROM alias") or [])
    aristas = _q("SELECT {},{} FROM {}".format(org, dst, enl)) or []
    rotos = [(o, d) for o, d in aristas if puentes.get(d, d) not in slugs]
    tocados = set()
    for o, d in aristas:
        tocados.add(o); tocados.add(puentes.get(d, d))
    # una ISLA declarada no es una herida: es un saber que miré y no ata con
    # nada. Castigarla sería premiar el vínculo inventado (Ley del Hilo Mínimo)
    est = "estado" if ES else "state"
    islas = set(r[0] for r in (_q("SELECT slug FROM {} WHERE {}='{}'".format(
        meta, est, "isla" if ES else "island")) or []))
    huerfanas = sorted(slugs - tocados - islas)
    tot_a = len(aristas) or 1
    tot_e = len(slugs) or 1
    p = 100 - (60.0 * len(rotos) / tot_a + 40.0 * len(huerfanas) / tot_e)
    probs = [{"titulo": "{} → {}".format(o, d), "clase": "roto",
              "detalle": "el destino no existe en el Abismo"} for o, d in rotos]
    probs += [{"titulo": s, "clase": "huerfana",
               "detalle": "ninguna esencia la nombra ni ella nombra a nadie"}
              for s in huerfanas]
    return _dim("tejido", "Tejido", 18, p,
                _t("100 − (60×rotos/enlaces + 40×huérfanas/esencias)",
                   "100 − (60×broken/links + 40×orphans/essences)"), probs,
                _t("{} enlaces · {} rotos · {} huérfanas de {} esencias{}",
                   "{} links · {} broken · {} orphans of {} essences{}").format(
                    len(aristas), len(rotos), len(huerfanas), len(slugs),
                    " · {} isla(s) declaradas".format(len(islas)) if islas else ""))


def _d_estructura():
    """Sin tipo declarado no hay consulta posible; sin bloques, cada respuesta
    arrastra el documento entero."""
    meta = "meta_esencia" if ES else "essence_meta"
    tipo = "tipo" if ES else "type"
    ese = "esencias" if ES else "essences"
    cont = "contenido" if ES else "content"
    filas = _q("SELECT slug,{} FROM {}".format(tipo, meta)) or []
    sin_tipo = [s for s, t in filas if not t]
    grandes = _q("SELECT slug, length({}) FROM {} WHERE length({})>4000"
                 .format(cont, ese, cont)) or []
    con_bloque = set(r[0] for r in (_q("SELECT DISTINCT slug FROM bloques" if ES
                                       else "SELECT DISTINCT slug FROM blocks") or []))
    gordas_mudas = [(s, n) for s, n in grandes if s not in con_bloque]
    tot = len(filas) or 1
    p_tipo = 100.0 * (tot - len(sin_tipo)) / tot
    p_blq = 100.0 if not grandes else 100.0 * (len(grandes) - len(gordas_mudas)) / len(grandes)
    probs = [{"titulo": s, "clase": "sin-tipo",
              "detalle": "sin `tipo` en el frontmatter: no se puede consultar por familia"}
             for s in sin_tipo]
    probs += [{"titulo": s, "clase": "sin-bloques",
               "detalle": "{} caracteres sin un solo bloque ^id: cada respuesta arrastra el saco".format(n)}
              for s, n in gordas_mudas]
    return _dim("estructura", "Estructura", 14, .6 * p_tipo + .4 * p_blq,
                _t("60% × (con tipo) + 40% × (esencias grandes con bloques ^id)",
                   "60% × (typed) + 40% × (large essences with ^id blocks)"), probs,
                _t("{} sin tipo · {} grandes sin bloques",
                   "{} untyped · {} large without blocks").format(len(sin_tipo), len(gordas_mudas)))


def _d_frescura():
    """Una verdad vieja que nadie re-verificó es una mentira esperando turno."""
    meta = "meta_esencia" if ES else "essence_meta"
    dev = "devorado" if ES else "devoured"
    cad = "caduca" if ES else "expires"
    filas = _q("SELECT slug,{},{} FROM {}".format(dev, cad, meta)) or []
    hoy = _dt.date.today().isoformat()
    rancias = [(s, _dias(d)) for s, d, c in filas if _dias(d) and _dias(d) > 120]
    caducadas = [(s, c) for s, d, c in filas if c and str(c) < hoy]
    tot = len(filas) or 1
    p = 100 - (55.0 * len(rancias) / tot + 45.0 * len(caducadas) / tot)
    probs = [{"titulo": s, "clase": "rancia",
              "detalle": "{} días sin re-verificar".format(d)} for s, d in rancias]
    probs += [{"titulo": s, "clase": "caducada",
               "detalle": "venció el {} — exige re-Juicio".format(c)} for s, c in caducadas]
    return _dim("frescura", "Frescura", 12, p,
                _t("100 − (55×rancias>120d + 45×caducadas) / esencias",
                   "100 − (55×stale>120d + 45×expired) / essences"), probs,
                _t("{} rancias · {} caducadas", "{} stale · {} expired").format(len(rancias), len(caducadas)))


def _d_sincronia():
    """El disco es la verdad. Si la BD y el disco discrepan, miento con datos."""
    ese = "esencias" if ES else "essences"
    org = "origen" if ES else "source"
    meta = "meta_esencia" if ES else "essence_meta"
    res = "residente" if ES else "resident"
    dir_ese = os.path.join(chaos.CLAUDE_DIR, "skills", "chaos",
                           "abismo" if ES else "abyss", "esencias" if ES else "essences")
    en_disco = set()
    try:
        for f in os.listdir(dir_ese):
            if f.endswith(".md"):
                en_disco.add(f[:-3])
    except OSError:
        pass
    indexadas = set(r[0] for r in (_q("SELECT slug FROM {}".format(ese)) or []))
    residentes = set(r[0] for r in (_q("SELECT slug FROM {} WHERE {}=1".format(meta, res)) or []))
    sin_indexar = sorted(en_disco - indexadas)
    fantasmas = sorted((residentes & indexadas) - en_disco)
    tot = len(en_disco | indexadas) or 1
    p = 100 - 100.0 * (len(sin_indexar) + len(fantasmas)) / tot
    probs = [{"titulo": s, "clase": "sin-indexar",
              "detalle": "vive en disco pero NO está en las neuronas"} for s in sin_indexar]
    probs += [{"titulo": s, "clase": "fantasma",
               "detalle": "indexada como residente pero su archivo no existe"} for s in fantasmas]
    return _dim("sincronia", "Sincronía", 16, p,
                _t("100 − (sin indexar + fantasmas) / total en disco∪BD",
                   "100 − (unindexed + ghosts) / total on disk∪DB"), probs,
                _t("{} en disco · {} indexadas · {} desalineadas",
                   "{} on disk · {} indexed · {} misaligned").format(
                    len(en_disco), len(indexadas), len(sin_indexar) + len(fantasmas)))


def _d_errario():
    """Una falla viva es una herida abierta; una reincidencia, una negligencia."""
    f_t = T["fallas"]
    cols = T["f_cols"].split(",")
    filas = _q("SELECT rowid,{} FROM {}".format(T["f_cols"], f_t)) or []
    probs = []
    vivas = reinc = mudas = 0
    for f in filas:
        d = dict(zip(["id"] + cols, f))
        est = d.get("estado") or d.get("state")
        rep = int(d.get("repeticiones") or d.get("repeats") or 0)
        lec = d.get("leccion") or d.get("lesson")
        cur = d.get("cura") or d.get("cure")
        if est in ("viva", "alive"):
            vivas += 1
            probs.append({"titulo": "#{} {}".format(d["id"], d.get("titulo") or d.get("title")),
                          "clase": "viva", "detalle": "sin curar — {}".format(cur or "sin solución escrita")})
        if rep:
            reinc += 1
            probs.append({"titulo": "#{} {}".format(d["id"], d.get("titulo") or d.get("title")),
                          "clase": "reincidida", "detalle": "cometida {} vez(ces) MÁS".format(rep)})
        if not lec:
            mudas += 1
            probs.append({"titulo": "#{} {}".format(d["id"], d.get("titulo") or d.get("title")),
                          "clase": "sin-leccion", "detalle": "sin lección: no puede emboscar a nadie"})
    tot = len(filas) or 1
    p = 100 - (45.0 * vivas / tot + 40.0 * reinc / tot + 15.0 * mudas / tot) * 100 / 100 * 1.0
    p = 100 - (45.0 * vivas + 40.0 * reinc + 15.0 * mudas) / tot
    return _dim("errario", "Errario", 12, p,
                _t("100 − (45×vivas + 40×reincididas + 15×sin lección) / fallas",
                   "100 − (45×alive + 40×relapsed + 15×lessonless) / faults"), probs,
                _t("{} fallas · {} vivas · {} reincididas · {} sin lección",
                   "{} faults · {} alive · {} relapsed · {} lessonless").format(
                    len(filas), vivas, reinc, mudas))


def _d_deuda():
    """Obra sin sedimentar es memoria que se pierde al cerrar la sesión."""
    probs = []
    n_rastro = 0
    try:
        with io.open(RASTRO, encoding="utf-8", errors="replace") as f:
            n_rastro = sum(1 for l in f if _ISO.match(l))
    except OSError:
        pass
    if n_rastro:
        probs.append({"titulo": "{} obra(s) sin documentar".format(n_rastro),
                      "clase": "rastro", "detalle": "el rastro espera destilarse en crónica y esencias"})
    deu = _q("SELECT id,fecha,obras FROM deudas WHERE saldada=0" if ES
             else "SELECT id,date,works FROM debts WHERE settled=0") or []
    for i, fe, ob in deu:
        probs.append({"titulo": "deuda #{} · {} obra(s)".format(i, ob), "clase": "deuda",
                      "detalle": "sesión del {} murió sin sedimentar".format(str(fe)[:10])})
    ham = _q("SELECT id,texto,fecha FROM hambres" if ES
             else "SELECT id,text,date FROM hungers") or []
    for i, tx, fe in ham:
        probs.append({"titulo": "hambre #{}".format(i), "clase": "hambre", "detalle": tx})
    sin_leer = _uno("SELECT valor FROM meta WHERE clave='partes_sin_leer'" if ES
                    else "SELECT value FROM meta WHERE key='unread_reports'", defecto=0)
    try:
        sin_leer = int(sin_leer or 0)
    except Exception:
        sin_leer = 0
    if sin_leer:
        probs.append({"titulo": "{} parte(s) de vela sin leer".format(sin_leer),
                      "clase": "parte", "detalle": "si nadie me lee, dejo de velar"})
    castigo = min(60, n_rastro / 8.0) + 10.0 * len(deu) + 6.0 * len(ham) + 5.0 * sin_leer
    return _dim("deuda", "Deuda", 14, 100 - castigo,
                _t("100 − (obras/8 hasta 60 + 10×deudas + 6×hambres + 5×partes sin leer)",
        "100 − (works/8 up to 60 + 10×debts + 6×hungers + 5×unread reports)"),
                probs, _t("{} obras · {} deudas · {} hambres",
                          "{} works · {} debts · {} hungers").format(n_rastro, len(deu), len(ham)))


def _d_autonomia():
    """Lo que obro sin testigos debe ser auditable y limpio."""
    a_t = T["actos"]
    cols = T["a_cols"].split(",")
    filas = _q("SELECT id,{} FROM {} ORDER BY id DESC LIMIT 100".format(T["a_cols"], a_t)) or []
    probs, sucios = [], 0
    for f in filas:
        d = dict(zip(["id"] + cols, f))
        ver = d.get("veredicto") or d.get("verdict") or "ok"
        if ver != "ok":
            sucios += 1
            probs.append({"titulo": "acto #{} · {}".format(d["id"], d.get("tipo") or d.get("kind")),
                          "clase": "veredicto", "detalle": ver})
    frenado = os.path.exists(os.path.join(CHAOS_HOME, "PARAR" if ES else "STOP"))
    if frenado:
        probs.append({"titulo": "autonomía frenada", "clase": "freno",
                      "detalle": "existe el archivo de freno: no velo mientras esté"})
    ultimo = None
    if filas:
        d0 = dict(zip(["id"] + cols, filas[0]))
        ultimo = _dias(d0.get("fecha") or d0.get("date"))
    viejo = ultimo is not None and ultimo > 3
    if viejo:
        probs.append({"titulo": "último acto hace {} días".format(ultimo),
                      "clase": "dormido", "detalle": "el latido no se ha disparado"})
    tot = len(filas) or 1
    p = 100 - (70.0 * sucios / tot) - (20 if frenado else 0) - (10 if viejo else 0)
    return _dim("autonomia", "Autonomía", 8, p,
                _t("100 − 70×sucios/actos − 20 si frenada − 10 si dormida >3d",
                   "100 − 70×dirty/acts − 20 if braked − 10 if asleep >3d"), probs,
                _t("{} actos · {} con veredicto sucio",
                   "{} acts · {} with a dirty verdict").format(len(filas), sucios))


def _d_resguardo():
    """Sin respaldo reciente, una máquina muerta es una memoria muerta."""
    probs = []
    base = os.path.join(CHAOS_HOME, "respaldos" if ES else "backups")
    dias = None
    try:
        ds = sorted(os.listdir(base))
        if ds:
            dias = _dias(ds[-1][:10] + "T00:00:00")
    except OSError:
        pass
    if dias is None:
        probs.append({"titulo": "sin respaldos", "clase": "resguardo",
                      "detalle": "jamás se ha respaldado el Abismo"})
        p = 0
    else:
        p = 100 - min(100, dias * 12)
        if dias > 2:
            probs.append({"titulo": "último respaldo hace {} días".format(dias),
                          "clase": "resguardo", "detalle": "un respaldo viejo cubre poco"})
    # una llave REAL trae 16+ caracteres tras el prefijo; hablar de "sk-" en
    # una frase no es una fuga. Falso positivo cazado: 2 esencias que solo
    # MENCIONAN el patron. Diagnosticar mal cuesta la confianza del veredicto.
    ese_t = "esencias" if ES else "essences"
    col_c = "contenido" if ES else "content"
    reales = []
    for slug, cont in (_q("SELECT slug,{} FROM {}".format(col_c, ese_t)) or []):
        if re.search(r"sk-[A-Za-z0-9_\-]{16,}|ghp_[A-Za-z0-9]{20,}", cont or ""):
            reales.append(slug)
    if reales:
        probs.append({"titulo": "{} esencia(s) con patrón de llave".format(len(reales)),
                      "clase": "fuga", "detalle": ", ".join(reales[:6])})
        p -= 40
    return _dim("resguardo", "Resguardo", 6, p,
                _t("100 − 12×días desde el último respaldo − 40 si hay patrón de llave",
        "100 − 12×days since last backup − 40 if a key pattern is found"),
                probs, _t("último respaldo: {}", "last backup: {}").format(
                    _t("hace {} día(s)", "{} day(s) ago").format(dias) if dias is not None else _t("nunca", "never")))


def _guardar_salud(d):
    """Frente 7: cada lectura de salud deja su huella (una por día, la última
    manda). Sin historia, un porcentaje es una foto sin antes ni después."""
    try:
        import datetime as dt
        con = chaos.db()
        con.execute("INSERT OR REPLACE INTO {} VALUES (?,?,?)".format(
            "salud_historia" if ES else "health_history"),
            (dt.date.today().isoformat(), d["global"],
             json.dumps({x["clave"]: x["puntaje"] for x in d["dimensiones"]})))
        con.commit(); con.close()
    except Exception:
        pass          # la historia jamás rompe el diagnóstico


def api_salud_historia(dias=30):
    filas = _q("SELECT fecha, global, dimensiones FROM {} ORDER BY fecha DESC LIMIT ?".format(
        "salud_historia" if ES else "health_history"), (int(dias),)) or []
    return [{"fecha": f, "global": g, "dimensiones": json.loads(dd or "{}")}
            for f, g, dd in reversed(filas)]


def api_salud():
    """SALUD REAL: 8 dimensiones, cada una con su fórmula y su desglose.
    El global es la media PONDERADA — y todo lo que resta se puede nombrar."""
    dims = []
    for f in (_d_tejido, _d_estructura, _d_frescura, _d_sincronia,
              _d_errario, _d_deuda, _d_autonomia, _d_resguardo):
        try:
            dims.append(f())
        except Exception as e:
            dims.append(_dim(f.__name__, f.__name__, 0, 0, "—",
                             [{"titulo": "no se pudo medir", "clase": "error",
                               "detalle": str(e)}], "error"))
    peso = sum(d["peso"] for d in dims) or 1
    global_ = sum(d["puntaje"] * d["peso"] for d in dims) / peso
    res = {"global": round(global_, 1),
            "veredicto": ("sano" if global_ >= 85 else
                          "vigilar" if global_ >= 65 else
                          "herido" if global_ >= 40 else "podrido"),
            "dimensiones": sorted(dims, key=lambda d: d["puntaje"]),
            "total_problemas": sum(d["n_problemas"] for d in dims)}
    _guardar_salud(res)
    res["historia"] = api_salud_historia()
    return res


def api_notas():
    """Todas las chispas con su anclaje de tres niveles y su confianza."""
    n_t = T["notas"]
    cols = ("id,texto,territorio,foco,ancla,confianza,contexto,fecha,estado" if ES
            else "id,text,territory,focus,anchor,confidence,context,date,state")
    filas = _q("SELECT {} FROM {} ORDER BY id DESC".format(cols, n_t)) or []
    claves = cols.split(",")
    notas = [dict(zip(claves, f)) for f in filas]
    for n in notas:
        n["territorio"] = n.get("territorio") or n.get("territory") or ""
    return notas


def api_grafo():
    """El grafo del Tejido. Los enlaces ROTOS se marcan — Obsidian los pinta
    igual que los sanos; yo señalo la grieta."""
    meta = "meta_esencia" if ES else "essence_meta"
    enl = "enlaces" if ES else "links"
    org, dst = ("origen", "destino") if ES else ("source", "target")
    res = "residente" if ES else "resident"
    tipo = "tipo" if ES else "type"
    # CUMULO: el tipo declarado manda; si falta (59 de 76 esencias no lo
    # tienen), se deduce del PREFIJO del slug, que es dato real y no invento.
    # Un cumulo llamado "?" con el 78% de los nodos no es una jerarquia.
    FAMILIA = {"project": "proyecto", "proyecto": "proyecto",
               "feedback": "feedback", "reference": "referencia",
               "referencia": "referencia", "territorio": "territorio",
               "cicatrices": "cicatriz", "cicatriz": "cicatriz"}
    nodos = {}
    for slug, tp, rs in _q("SELECT slug,{},{} FROM {}".format(tipo, res, meta)) or []:
        if not tp:
            pref = slug.split("-")[0] if "-" in slug else slug
            tp = FAMILIA.get(pref, "doctrina")   # lo suelto es doctrina propia
        nodos[slug] = {"id": slug, "tipo": tp, "residente": rs or 0, "grado": 0}
    puentes = dict(_q("SELECT alias, slug FROM alias") or [])
    aristas = []
    for o, d in _q("SELECT {},{} FROM {}".format(org, dst, enl)) or []:
        d = puentes.get(d, d)      # el nombre mal escrito cruza su puente
        roto = d not in nodos
        aristas.append({"o": o, "d": d, "roto": roto})
        if o in nodos: nodos[o]["grado"] += 1
        if d in nodos: nodos[d]["grado"] += 1
    # los destinos rotos existen como fantasmas: verlos es el punto
    for a in aristas:
        if a["roto"] and a["d"] not in nodos:
            nodos[a["d"]] = {"id": a["d"], "tipo": "roto", "residente": 0, "grado": 0}
    return {"nodos": list(nodos.values()), "aristas": aristas}


# ── EL TIEMPO · actividad INFERIDA, jamás cronometrada ────────────────────
RASTRO = os.path.join(CHAOS_HOME, "forja" if ES else "forge",
                      "rastro.log" if ES else "trail.log")
CORTE_SESION = 30 * 60          # hueco > 30 min = otra sesión de obra
_ISO = re.compile(r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})\t")


def _leer_rastro():
    """Un comando Bash multilínea parte el registro: solo cuenta la línea que
    ARRANCA con un timestamp ISO. (Falla que evité: partir por \n a ciegas.)"""
    filas = []
    try:
        with io.open(RASTRO, encoding="utf-8", errors="replace") as f:
            for l in f:
                m = _ISO.match(l)
                if not m:
                    continue
                p = l.rstrip("\n").split("\t")
                filas.append({"ts": p[0], "cwd": p[2] if len(p) > 2 else "",
                              "accion": p[3] if len(p) > 3 else "",
                              "ruta": p[4] if len(p) > 4 else ""})
    except OSError:
        pass
    return filas


def api_tiempo():
    import datetime as dt
    filas = _leer_rastro()
    if not filas:
        return {"sesiones": [], "mapa": [], "total_obras": 0,
                "horas_inferidas": 0, "por_territorio": []}
    marcas = []
    for f in filas:
        try:
            marcas.append((dt.datetime.fromisoformat(f["ts"]), f))
        except ValueError:
            continue
    marcas.sort(key=lambda x: x[0])
    # agrupar en sesiones de obra por hueco > 30 min
    sesiones, actual = [], [marcas[0]]
    for prev, cur in zip(marcas, marcas[1:]):
        if (cur[0] - prev[0]).total_seconds() > CORTE_SESION:
            sesiones.append(actual); actual = []
        actual.append(cur)
    sesiones.append(actual)
    out, total = [], 0.0
    for s in sesiones:
        dur = (s[-1][0] - s[0][0]).total_seconds()
        # una sesión de una sola obra no dura 0: se le acredita el corte mínimo
        dur = max(dur, 60.0)
        total += dur
        ters = {}
        for _, f in s:
            t = os.path.basename((f["cwd"] or "").rstrip("/")) or "?"
            ters[t] = ters.get(t, 0) + 1
        out.append({"inicio": s[0][0].isoformat(timespec="minutes"),
                    "fin": s[-1][0].isoformat(timespec="minutes"),
                    "minutos": round(dur / 60), "obras": len(s),
                    "territorio": max(ters, key=ters.get)})
    # mapa de calor: día × hora
    mapa = {}
    for d, _ in marcas:
        mapa["{}|{}".format(d.date().isoformat(), d.hour)] = \
            mapa.get("{}|{}".format(d.date().isoformat(), d.hour), 0) + 1
    por_ter = {}
    for s in out:
        por_ter[s["territorio"]] = por_ter.get(s["territorio"], 0) + s["minutos"]
    return {
        "sesiones": list(reversed(out))[:40],
        "mapa": [{"dia": k.split("|")[0], "hora": int(k.split("|")[1]), "n": v}
                 for k, v in sorted(mapa.items())],
        "total_obras": len(marcas),
        "horas_inferidas": round(total / 3600, 1),
        "por_territorio": sorted(({"territorio": k, "minutos": v}
                                  for k, v in por_ter.items()),
                                 key=lambda x: -x["minutos"])[:8],
    }


# ── TERRITORIO: el mapa real del proyecto ───────────────────────
IGNORA = {".git", "node_modules", "__pycache__", ".venv", "venv", ".DS_Store",
          ".idea", ".vscode", "dist", "build", ".next", ".cache"}
ROLES = {
    "README.md": "puerta de entrada del proyecto",
    "package.json": "dependencias y scripts de Node",
    "requirements.txt": "dependencias de Python",
    "Dockerfile": "receta del contenedor",
    "Makefile": "tareas de construcción",
    ".gitignore": "qué NO entra al repo",
    "LICENSE": "términos de uso",
}
EXT = {".py": "código Python", ".js": "código JavaScript", ".ts": "código TypeScript",
       ".md": "documento", ".json": "datos/config", ".sh": "guión de shell",
       ".css": "estilos", ".html": "página", ".sql": "esquema/consulta",
       ".yml": "config", ".yaml": "config", ".toml": "config", ".png": "imagen",
       ".jpg": "imagen", ".svg": "vector", ".txt": "texto", ".log": "bitácora"}


def _que_hace(ruta, nombre):
    """Qué hace un archivo — leído de ÉL, no inventado: docstring, primer
    encabezado, o el rol conocido. Si no se sabe, se dice que no se sabe."""
    if nombre in ROLES:
        return ROLES[nombre]
    ext = os.path.splitext(nombre)[1].lower()
    try:
        if ext in (".py", ".js", ".css", ".sh", ".md", ".html"):
            with io.open(ruta, encoding="utf-8", errors="replace") as f:
                cab = f.read(1400)
            if ext == ".md":
                for l in cab.splitlines():
                    if l.startswith("#"):
                        return l.lstrip("# ").strip()[:110]
                    if l.strip() and not l.startswith("---"):
                        return l.strip()[:110]
            else:
                m = re.search(r'"""(.+?)"""', cab, re.S) or \
                    re.search(r"'''(.+?)'''", cab, re.S)
                if m:
                    return " ".join(m.group(1).split())[:110]
                for l in cab.splitlines():
                    st = l.strip()
                    if st.startswith(("#!", "# -*-")):
                        continue
                    if st.startswith(("#", "//", "/*", "*")):
                        limpio = st.lstrip("#/*  ").strip()
                        if len(limpio) > 12:
                            return limpio[:110]
                    elif st:
                        break
    except OSError:
        pass
    return EXT.get(ext, "")


def _mapa_carpeta(base):
    """Árbol de UN nivel + conteo real de lo que hay dentro de cada carpeta."""
    out = []
    try:
        entradas = sorted(os.scandir(base), key=lambda e: (not e.is_dir(), e.name.lower()))
    except OSError:
        return out
    for e in entradas:
        if e.name in IGNORA or e.name.startswith("._"):
            continue
        try:
            st = e.stat()
        except OSError:
            continue
        if e.is_dir():
            n_arch = n_dir = 0
            try:
                for sub in os.scandir(e.path):
                    if sub.name in IGNORA:
                        continue
                    n_dir += sub.is_dir(); n_arch += sub.is_file()
            except OSError:
                pass
            out.append({"nombre": e.name, "tipo": "dir", "hijos": n_arch + n_dir,
                        "detalle": "{} archivo(s) · {} carpeta(s)".format(n_arch, n_dir),
                        "mtime": int(st.st_mtime)})
        else:
            out.append({"nombre": e.name, "tipo": "archivo", "bytes": st.st_size,
                        "detalle": _que_hace(e.path, e.name), "mtime": int(st.st_mtime)})
    return out


def api_territorio(nombre):
    """Ficha completa de un territorio: qué es, su mapa, sus fallas, su obra."""
    filas = _leer_rastro()
    raices, sub = _mapa_raices()
    clave, visible = _plegar(nombre, raices, sub)
    ruta = raices.get(clave, (None, None))[1]
    nombre = visible
    ter_f = "territorio" if ES else "territory"
    resumen = []
    try:
        con = chaos.db()
        pat = "%" + nombre.lower().replace(" ", "-") + "%"
        sql = ("SELECT slug,titulo,contenido FROM esencias WHERE lower(slug) LIKE ?"
               " OR lower(origen) LIKE ? LIMIT 4") if ES else \
              ("SELECT slug,title,content FROM essences WHERE lower(slug) LIKE ?"
               " OR lower(source) LIKE ? LIMIT 4")
        for slug, titulo, cont in con.execute(sql, (pat, "%" + nombre.lower() + "%")).fetchall():
            frag = " ".join((cont or "").split())
            i = frag.find("## Esencia")
            frag = frag[i + 10:] if i >= 0 else frag
            resumen.append({"slug": slug, "titulo": titulo, "texto": frag[:320]})
        con.close()
    except Exception:
        pass
    todas = _q("SELECT rowid,{} FROM {}".format(T["f_cols"], T["fallas"])) or []
    claves = ["id"] + T["f_cols"].split(",")
    i_ter = claves.index(ter_f)
    fallas = [f for f in todas if _plegar(f[i_ter], raices, sub)[0] == clave]
    fallas.sort(key=lambda f: -f[0])
    obras = [f for f in filas
             if _canon(nombre_territorio(f["cwd"]) or "") == clave]
    archivos = {}
    for o in obras:
        if o["accion"] in ("crear", "editar", "create", "edit") and o["ruta"]:
            archivos[o["ruta"]] = archivos.get(o["ruta"], 0) + 1
    return {
        "nombre": nombre, "ruta": ruta, "existe": bool(ruta and os.path.isdir(ruta)),
        "resumen": resumen,
        "mapa": _mapa_carpeta(ruta) if ruta and os.path.isdir(ruta) else [],
        "fallas": [dict(zip(claves, f)) for f in fallas],
        "obras": len(obras),
        "mas_tocados": sorted(({"ruta": k, "veces": v} for k, v in archivos.items()),
                              key=lambda x: -x["veces"])[:12],
        "ultimas": [{"ts": o["ts"], "accion": o["accion"],
                     "ruta": os.path.basename(o["ruta"] or "")} for o in obras[-15:]][::-1],
    }


def api_linea(dias=14):
    """LÍNEA TEMPORAL: todo suceso fundido en orden — obras, fallas, actos,
    chispas. Cada uno con su detalle para expandir."""
    ev = []
    for f in _leer_rastro():
        ev.append({"ts": f["ts"], "clase": "obra", "titulo": os.path.basename(f["ruta"] or "?"),
                   "sub": f["accion"], "ter": os.path.basename((f["cwd"] or "").rstrip("/")),
                   "detalle": f["ruta"]})
    fcol = T["f_cols"].split(",")
    for f in _q("SELECT rowid,{} FROM {}".format(T["f_cols"], T["fallas"])) or []:
        d = dict(zip(["id"] + fcol, f))
        ev.append({"ts": (d.get("fecha") or d.get("date") or "") + "T12:00:00",
                   "clase": "falla", "titulo": d.get("titulo") or d.get("title"),
                   "sub": d.get("estado") or d.get("state"),
                   "ter": d.get("territorio") or d.get("territory"),
                   "detalle": "SOLUCIÓN: " + (d.get("cura") or d.get("cure") or "—"),
                   "id": d["id"]})
    acol = T["a_cols"].split(",")
    for a in _q("SELECT id,{} FROM {}".format(T["a_cols"], T["actos"])) or []:
        d = dict(zip(["id"] + acol, a))
        ev.append({"ts": d.get("fecha") or d.get("date"), "clase": "acto",
                   "titulo": "{}/{}".format(d.get("tipo") or d.get("kind"),
                                            d.get("accion") or d.get("action")),
                   "sub": d.get("veredicto") or d.get("verdict"), "ter": "",
                   "detalle": d.get("detalle") or d.get("detail") or ""})
    ncol = ("texto,territorio,foco,fecha" if ES else "text,territory,focus,date")
    for n in _q("SELECT rowid,{} FROM {}".format(ncol, T["notas"])) or []:
        ev.append({"ts": n[4], "clase": "chispa", "titulo": (n[1] or "")[:90],
                   "sub": n[3] or "", "ter": n[2] or "", "detalle": n[1] or ""})
    ev = [e for e in ev if e["ts"]]
    ev.sort(key=lambda x: x["ts"], reverse=True)
    dias_out, orden = {}, []
    for e in ev[:600]:
        d = e["ts"][:10]
        if d not in dias_out:
            if len(orden) >= dias:
                break
            dias_out[d] = []; orden.append(d)
        dias_out[d].append(e)
    return {"dias": [{"dia": d, "eventos": dias_out[d][:120],
                      "conteo": {c: sum(1 for x in dias_out[d] if x["clase"] == c)
                                 for c in ("obra", "falla", "acto", "chispa")}}
                     for d in orden]}


# ══ FRENTE 5 · EL OJO ACTÚA ═══════════════════════════════════════════════
# Hasta aquí el Ojo era solo lente. Ahora escribe — pero JAMÁS con SQL propio:
# cada acción ejecuta EL MISMO comando que usaría el Portador en su terminal.
# Las validaciones del cuerpo son las validaciones del Ojo; si mañana cambia
# una regla en chaos.py, el Ojo obedece sin enterarse.
#
# Y una puerta que escribe se cierra distinto: solo POST, solo con la cabecera
# X-Ojo-Accion. Una etiqueta <img src="...">, un enlace o un formulario ajeno
# NO pueden ponerla — un GET jamás muta nada.
ACCIONES = {
    # nombre → (comando ES, comando EN, cuántos argumentos exige)
    "curar_falla":   ("falla-curada", "fault-cured", 1),
    "reincidir":     ("reincidir", "relapse", 1),
    "saciar":        ("saciar", "sate", 1),
    "saldar_deuda":  ("deudas", "debts", 1),      # se expande abajo
    "matar_sugerencia": ("sugerir", "suggest", 1),
    "declarar_isla": ("isla", "island", 1),
    "frenar":        ("autonomia", "autonomy", 0),
    "reanudar":      ("autonomia", "autonomy", 0),
}


def ejecutar_accion(nombre, arg=None):
    """Traduce una acción del Ojo al comando exacto del cuerpo. Nada más."""
    if nombre not in ACCIONES:
        return {"ok": False, "error": "acción desconocida"}
    cmd_es, cmd_en, n_args = ACCIONES[nombre]
    cmd = cmd_es if ES else cmd_en
    if n_args and not arg:
        return {"ok": False, "error": "falta el argumento"}
    if not re.match(r"^[\w\-\. ]{1,120}$", str(arg or "x")):
        return {"ok": False, "error": "argumento inaceptable"}
    if nombre == "saldar_deuda":
        argv = [cmd, "saldar" if ES else "settle", str(arg)]
    elif nombre == "matar_sugerencia":
        argv = [cmd, "--matar" if ES else "--kill", str(arg)]
    elif nombre == "frenar":
        argv = [cmd, "revocar" if ES else "revoke"]
    elif nombre == "reanudar":
        argv = [cmd, "conceder" if ES else "grant"]
    else:
        argv = [cmd, str(arg)]
    salida = _cli(*argv)
    return {"ok": True, "comando": " ".join(["chaos"] + argv),
            "salida": salida.strip().splitlines()[-3:]}


# ── tiempo real: SSE sobre data_version (barato, no polling bruto) ────────
# FALLA CAZADA EN LA PRUEBA DE FUEGO: data_version solo cambia DENTRO de la
# misma conexión cuando OTRA escribe. Abrir una conexión por sondeo daba un
# valor eternamente igual — un pulso que no late. La conexión del vigía es
# UNA y persiste todo el stream.
def _abrir_vigia():
    try:
        return chaos.db()
    except Exception:
        return None


def _data_version(vigia):
    try:
        return vigia.execute("PRAGMA data_version").fetchone()[0]
    except Exception:
        return -1


class Ojo(BaseHTTPRequestHandler):
    def log_message(self, *a):  # silencio: el nervio no narra
        pass

    def _token_ok(self):
        """FRENTE 9: el token vale por query O por cookie. La primera visita
        lo canjea y la URL queda limpia — una llave en el historial del
        navegador es una llave regalada (Regla 5)."""
        qs = parse_qs(urlparse(self.path).query)
        if qs.get("t", [""])[0] == TOKEN:
            return True
        galleta = self.headers.get("Cookie") or ""
        for par in galleta.split(";"):
            k, _, v = par.strip().partition("=")
            if k == "ojo" and v == TOKEN:
                return True
        return False

    def _trae_token_en_url(self):
        return parse_qs(urlparse(self.path).query).get("t", [""])[0] == TOKEN

    def _json(self, data, code=200):
        cuerpo = json.dumps(data, ensure_ascii=False).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(cuerpo)))
        self._csp()
        self.end_headers()
        self.wfile.write(cuerpo)

    def _csp(self):
        self.send_header("Content-Security-Policy",
                         "default-src 'self'; img-src 'self' data:; "
                         "style-src 'self' 'unsafe-inline'; script-src 'self'; "
                         "connect-src 'self'")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Referrer-Policy", "no-referrer")

    def do_POST(self):
        """La única puerta que escribe. Exige POST + X-Ojo-Accion: ni un <img>
        ni un formulario ajeno pueden dispararla (un GET jamás muta nada)."""
        if not self._token_ok():
            self._json({"error": "sin token"}, 403); return
        if self.headers.get("X-Ojo-Accion") != "1":
            self._json({"error": "falta la cabecera de acción"}, 400); return
        if urlparse(self.path).path != "/api/accion":
            self._json({"error": "esa puerta no escribe"}, 404); return
        try:
            n = int(self.headers.get("Content-Length") or 0)
            cuerpo = json.loads(self.rfile.read(n) or b"{}")
        except Exception:
            self._json({"error": "cuerpo ilegible"}, 400); return
        self._json(ejecutar_accion(cuerpo.get("accion"), cuerpo.get("arg")))

    def do_GET(self):
        ruta = urlparse(self.path).path
        # FALLA CAZADA EN VIVO: <script src> y <link href> NO llevan token →
        # la puerta les negaba el paso y el rostro nacía muerto (403 en
        # app.js/style.css). Los estáticos e i18n son CÓDIGO, no datos del
        # Portador: pasan libres. El token guarda la página y TODA la API.
        publico = ruta.startswith("/static/") or ruta == "/i18n"
        if not publico and not self._token_ok():
            self._json({"error": "sin token — el Vacío no abre la puerta"}, 403)
            return
        if ruta == "/" or ruta == "/index.html":
            if self._trae_token_en_url():
                # canje: cookie HttpOnly + SameSite=Strict, y la URL se limpia
                self.send_response(302)
                self.send_header("Set-Cookie",
                                 "ojo={}; Path=/; HttpOnly; SameSite=Strict".format(TOKEN))
                self.send_header("Location", "/")
                self.end_headers()
                return
            self._archivo(os.path.join(AQUI, "static", "index.html"), "text/html")
        elif ruta.startswith("/static/"):
            base = os.path.realpath(os.path.join(AQUI, "static"))
            p = os.path.realpath(os.path.join(AQUI, ruta.lstrip("/")))
            if not p.startswith(base + os.sep):          # jaula de rutas
                self._json({"error": "fuera de la jaula"}, 403); return
            tipo = {"css": "text/css", "js": "application/javascript",
                    "png": "image/png", "json": "application/json",
                    "svg": "image/svg+xml"}.get(p.rsplit(".", 1)[-1], "application/octet-stream")
            self._archivo(p, tipo)
        elif ruta == "/i18n":
            idioma = parse_qs(urlparse(self.path).query).get("l", ["es" if ES else "en"])[0]
            idioma = "es" if idioma == "es" else "en"
            self._archivo(os.path.join(AQUI, "i18n", idioma + ".json"), "application/json")
        elif ruta == "/api/pulso":        self._json(api_pulso())
        elif ruta == "/api/fallas":       self._json(api_fallas())
        elif ruta == "/api/actos":        self._json(api_actos())
        elif ruta == "/api/territorios":  self._json(api_territorios())
        elif ruta == "/api/buscar":
            q = parse_qs(urlparse(self.path).query).get("q", [""])[0]
            self._json(api_buscar(q))
        elif ruta == "/api/salud":        self._json(api_salud())
        elif ruta == "/api/grafo":        self._json(api_grafo())
        elif ruta == "/api/territorio":
            n = parse_qs(urlparse(self.path).query).get("n", [""])[0]
            self._json(api_territorio(n) if n else {"error": "sin nombre"})
        elif ruta == "/api/linea":        self._json(api_linea())
        elif ruta == "/api/notas":        self._json(api_notas())
        elif ruta == "/api/tiempo":       self._json(api_tiempo())
        elif ruta == "/api/eventos":      self._sse()
        else:
            self._json({"error": "el Vacío no contiene esa ruta"}, 404)

    def _archivo(self, p, tipo):
        try:
            with io.open(p, "rb") as f:
                cuerpo = f.read()
        except OSError:
            self._json({"error": "no existe"}, 404); return
        self.send_response(200)
        self.send_header("Content-Type", tipo + ("; charset=utf-8" if tipo.startswith("text") or "json" in tipo or "javascript" in tipo else ""))
        self.send_header("Content-Length", str(len(cuerpo)))
        self._csp()
        self.end_headers()
        self.wfile.write(cuerpo)

    def _sse(self):
        """El pulso: solo emite si la BD cambió (data_version) — cada 1s."""
        import time
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.end_headers()
        vigia = _abrir_vigia()
        v = _data_version(vigia)
        try:
            self.wfile.write(b"event: hola\ndata: {}\n\n"); self.wfile.flush()
            while True:
                time.sleep(1)
                nv = _data_version(vigia)
                if nv != v:
                    v = nv
                    self.wfile.write(b"event: cambio\ndata: {}\n\n")
                    self.wfile.flush()
                else:                                   # keepalive cada 25s
                    if int(time.time()) % 25 == 0:
                        self.wfile.write(b": latido\n\n"); self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError, OSError):
            pass
        finally:
            try:
                vigia and vigia.close()
            except Exception:
                pass


class Servidor(HTTPServer):
    daemon_threads = True
    # hilos por petición: el SSE no puede bloquear al resto
    def process_request(self, request, client_address):
        t = threading.Thread(target=self._hilo, args=(request, client_address), daemon=True)
        t.start()

    def _hilo(self, request, client_address):
        try:
            self.finish_request(request, client_address)
        except Exception:
            pass
        finally:
            self.shutdown_request(request)


def main():
    srv = Servidor(("127.0.0.1", 0), Ojo)          # puerto alto aleatorio
    puerto = srv.server_address[1]
    url = "http://127.0.0.1:{}/?t={}".format(puerto, TOKEN)
    # flush: si el Portador redirige la salida, la URL DEBE salir igual
    print("[OJO] {}".format(url), flush=True)
    print("[OJO] token por arranque; cerrar esta terminal apaga el Ojo.", flush=True)
    if "--sin-navegador" not in sys.argv:
        try:
            webbrowser.open(url)
        except Exception:
            pass
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
