//! RAG del código (F6 + S4) — búsqueda sobre el repo sin volcar todo al contexto.
//! - BM25: índice invertido PROPIO en Rust puro (antes, SQLite FTS5): el mismo tokenizador (`unicode61`:
//!   letras y números, minúscula, sin tilde; el guion bajo separa) y la misma fórmula (k1 = 1,2, b = 0,75,
//!   idf con piso), así que el orden de los resultados es el de siempre (una prueba lo compara con FTS5);
//! - S4: vectores (embeddings MLX, Qwen3-Embedding-0.6B) junto al índice, y búsqueda
//!   HÍBRIDA por RRF: lo semántico encuentra «¿dónde se valida el login?» →
//!   `check_credentials` sin una palabra en común; BM25 sigue clavando los nombres
//!   exactos. Sin vectores, todo funciona con BM25 como antes.
//!
//! En disco, `.gotv/indice/` con el formato propio de [`crate::programar::codec`]: `indice.bin` (rutas, trozos y
//! términos con sus apariciones; se carga una vez y queda en memoria hasta que cambie), `cuerpos.bin` (el texto
//! de los trozos, que se lee por posición sólo para lo que se muestra) y `vectores.bin`.

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

const EXCLUDE_DIRS: &[&str] = &[
    ".git",
    ".gotv",
    "target",
    "node_modules",
    ".venv",
    "__pycache__",
    "dist",
    "build",
];
const EXTS: &[&str] = &[
    "rs", "py", "js", "ts", "tsx", "jsx", "go", "c", "h", "cpp", "hpp", "cc", "java", "rb", "php",
    "md", "toml", "json", "yaml", "yml", "sh", "html", "css", "sql", "txt",
];
const MAX_FILE: u64 = 262_144; // 256 KiB

pub struct Hit {
    pub path: String,
    pub start: i64,
    pub end: i64,
    pub snippet: String,
}

/// Repo del embedder (S4): `GOTV_EMBED_REPO` o Qwen3-Embedding-0.6B (0,3 GB; medido: 3/3
/// preguntas en español → código en inglés sin palabras en común).
pub fn embed_repo() -> String {
    std::env::var("GOTV_EMBED_REPO")
        .ok()
        .filter(|r| crate::hub::valid_repo(r))
        .unwrap_or_else(|| "mlx-community/Qwen3-Embedding-0.6B-4bit-DWQ".into())
}

/// Carpeta del embedder si se bajó. El del catálogo exige su licencia ACEPTADA (Configuración →
/// Generadores, área «búsqueda»); uno propio por `GOTV_EMBED_REPO` queda a cargo de quien lo elige.
pub fn embedder_dir() -> Option<PathBuf> {
    let repo = embed_repo();
    if crate::generadores::ficha(&repo).is_some() && crate::generadores::aceptacion(&repo).is_none() {
        return None;
    }
    let d = crate::hub::models_dir().join(&repo);
    d.join("config.json").exists().then_some(d)
}

/// Qué hacer si falta el embebedor.
pub fn falta_embedder() -> String {
    crate::tr!(
        "falta el embebedor de la búsqueda ({}): acéptalo y bájalo en Configuración → Generadores (o `gotv setup rag`)",
        "the search embedder is missing ({}): accept and download it in Settings → Generators (or `gotv setup rag`)",
        embed_repo()
    )
}

/// M11: el embebedor NATIVO (Rust + MLX, sin Python) — `(carpeta del modelo, textos) → vectores`.
/// Lo registran el CLI y la GUI al arrancar (`gotv-core` no depende del núcleo MLX).
pub type Incrustador = fn(&Path, &[String]) -> Result<Vec<Vec<f32>>>;
static NATIVO: std::sync::OnceLock<Incrustador> = std::sync::OnceLock::new();

pub fn registrar_incrustador(f: Incrustador) {
    let _ = NATIVO.set(f);
}

/// El embebedor de la búsqueda semántica, SÓLO en el núcleo (M11; sin Python desde 2026-11-12).
pub struct Embedder {
    dir: PathBuf,
    f: Incrustador,
}

impl Embedder {
    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let (dir, f, t) = (self.dir.clone(), self.f, texts.to_vec());
        tokio::task::spawn_blocking(move || f(&dir, &t)).await.map_err(|e| anyhow!("embebedor: {e}"))?
    }

    /// Nada que apagar: el núcleo lo tiene en su hilo y lo suelta solo tras 5 min sin uso.
    pub async fn stop(&mut self) {}
}

/// El embebedor del núcleo (licencia aceptada y bajado).
pub async fn start_embedder() -> Result<Embedder> {
    let f = NATIVO.get().ok_or_else(|| anyhow!("{}", crate::i18n::t("este binario no trae el embebedor del núcleo", "this binary has no core embedder")))?;
    let dir = embedder_dir().ok_or_else(|| anyhow!("{}", falta_embedder()))?;
    Ok(Embedder { dir, f: *f })
}

/// Vectoriza todo el índice con el embedder dado. Devuelve cuántos trozos.
pub async fn index_vectors(rag: &Rag, e: &Embedder) -> Result<usize> {
    let chunks = rag.chunk_texts()?;
    let texts: Vec<String> = chunks.iter().map(|(_, t)| t.clone()).collect();
    let vecs = e.embed(&texts).await?;
    let pairs: Vec<(i64, Vec<f32>)> = chunks.into_iter().map(|(id, _)| id).zip(vecs).collect();
    rag.store_vectors(&pairs)?;
    Ok(pairs.len())
}

/// Qwen3-Embedding rinde más si la CONSULTA lleva la instrucción (los documentos, no).
pub fn query_text(q: &str) -> String {
    format!("Instruct: Given a question about a codebase, retrieve the code that answers it\nQuery: {q}")
}

fn normalized(v: &[f32]) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    v.iter().map(|x| x / n).collect()
}

// ── El índice, en Rust puro (antes: SQLite FTS5) ───────────────────────────────────────────────────

/// Dónde vive el índice del proyecto.
fn carpeta_indice(root: &Path) -> PathBuf {
    root.join(".gotv/indice")
}

const TIPO_INDICE: &[u8; 4] = b"RAGI";
const TIPO_CUERPOS: &[u8; 4] = b"RAGC";
const TIPO_VECTORES: &[u8; 4] = b"RAGV";
const VERSION_INDICE: u8 = 1;
/// BM25 con las constantes de SQLite FTS5 (las de siempre: el orden de los resultados no cambia).
const K1: f64 = 1.2;
const B: f64 = 0.75;
/// Tokens del fragmento (como `snippet(…, 12)` de FTS5).
const FRAGMENTO: usize = 12;

fn sin_indice() -> anyhow::Error {
    anyhow!("{}", crate::i18n::t("no hay índice: corre `gotv index` primero", "no index: run `gotv index` first"))
}

/// Quita la tilde a una letra ya en minúscula (el `remove_diacritics` del tokenizador `unicode61` de FTS5
/// para lo latino: español, portugués, francés, alemán…).
fn sin_tilde(c: char) -> char {
    match c {
        'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' | 'ā' | 'ą' => 'a',
        'é' | 'è' | 'ë' | 'ê' | 'ē' | 'ę' | 'ě' => 'e',
        'í' | 'ì' | 'ï' | 'î' | 'ī' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' | 'õ' | 'ō' | 'ø' => 'o',
        'ú' | 'ù' | 'ü' | 'û' | 'ū' | 'ů' => 'u',
        'ñ' | 'ń' | 'ň' => 'n',
        'ç' | 'ć' | 'č' => 'c',
        'ý' | 'ÿ' => 'y',
        'š' | 'ś' => 's',
        'ž' | 'ź' | 'ż' => 'z',
        'ř' => 'r',
        'ł' => 'l',
        'ď' => 'd',
        'ť' => 't',
        'ğ' => 'g',
        otro => otro,
    }
}

/// Los tokens de un texto (como `unicode61`): tramos de letras y números, en minúscula y sin tilde; el
/// guion bajo y lo demás separan. Con su posición (bytes) para el fragmento.
fn tokens(texto: &str) -> Vec<(String, usize, usize)> {
    let mut v = Vec::new();
    let mut actual = String::new();
    let mut desde = 0;
    for (i, c) in texto.char_indices() {
        if c.is_alphanumeric() {
            if actual.is_empty() {
                desde = i;
            }
            for l in c.to_lowercase() {
                actual.push(sin_tilde(l));
            }
        } else if !actual.is_empty() {
            v.push((std::mem::take(&mut actual), desde, i));
        }
    }
    if !actual.is_empty() {
        v.push((actual, desde, texto.len()));
    }
    v
}

/// Los términos de una consulta en texto libre: sus palabras de 2 o más caracteres, unidas con «o» (lo que
/// hacía la consulta de FTS5: una palabra rara —comillas, `AND (`, `*`— nunca rompe nada).
fn terminos_de(q: &str) -> Vec<String> {
    q.split(|c: char| !c.is_alphanumeric()).filter(|t| t.chars().count() >= 2).flat_map(|t| tokens(t).into_iter().map(|x| x.0)).collect()
}

#[derive(Debug, Clone)]
struct Trozo {
    ruta: u32,
    inicio: u32,
    fin: u32,
    /// tokens de la fila (ruta + cuerpo), para BM25
    largo: u32,
    /// dónde está su cuerpo en `cuerpos.bin`
    desde: u64,
    bytes: u32,
}

/// El índice en memoria (lo de `cuerpos.bin` se lee por posición, sólo para lo que se muestra).
struct Indice {
    rutas: Vec<String>,
    trozos: Vec<Trozo>,
    /// término → [(trozo, veces)]
    terminos: std::collections::HashMap<String, Vec<(u32, u32)>>,
    promedio: f64,
}

impl Indice {
    fn bytes(&self) -> Vec<u8> {
        use crate::programar::codec::Escritor;
        let mut e = Escritor::nuevo(TIPO_INDICE, VERSION_INDICE);
        e.textos(&self.rutas);
        e.u32(self.trozos.len() as u32);
        for t in &self.trozos {
            e.u32(t.ruta);
            e.u32(t.inicio);
            e.u32(t.fin);
            e.u32(t.largo);
            e.i64(t.desde as i64);
            e.u32(t.bytes);
        }
        // en orden: el mismo índice da el mismo archivo
        let mut claves: Vec<&String> = self.terminos.keys().collect();
        claves.sort();
        e.u32(claves.len() as u32);
        for k in claves {
            e.texto(k);
            let p = &self.terminos[k];
            e.u32(p.len() as u32);
            for (t, n) in p {
                e.u32(*t);
                e.u32(*n);
            }
        }
        e.bytes()
    }

    fn de_bytes(b: &[u8]) -> Result<Indice> {
        let (mut l, _) = crate::programar::codec::Lector::nuevo(b, TIPO_INDICE, VERSION_INDICE)?;
        let rutas = l.textos()?;
        let n = l.u32()? as usize;
        let mut trozos = Vec::with_capacity(n);
        let mut suma = 0f64;
        for _ in 0..n {
            let t = Trozo { ruta: l.u32()?, inicio: l.u32()?, fin: l.u32()?, largo: l.u32()?, desde: l.i64()? as u64, bytes: l.u32()? };
            suma += t.largo as f64;
            trozos.push(t);
        }
        let m = l.u32()? as usize;
        let mut terminos = std::collections::HashMap::with_capacity(m);
        for _ in 0..m {
            let k = l.texto()?;
            let np = l.u32()? as usize;
            let mut p = Vec::with_capacity(np);
            for _ in 0..np {
                p.push((l.u32()?, l.u32()?));
            }
            terminos.insert(k, p);
        }
        Ok(Indice { rutas, promedio: if n == 0 { 0.0 } else { suma / n as f64 }, trozos, terminos })
    }

    /// BM25 de FTS5: por cada término de la consulta (repetidos cuentan), idf · f·(k1+1) / (f + k1·(1−b+b·|D|/avgdl)),
    /// con idf = ln((N−n+½)/(n+½)) y un piso de 1e-6. Los `tope` mejores, de mayor a menor (empate: el trozo antes).
    fn bm25(&self, terminos: &[String], tope: usize) -> Vec<(u32, f64)> {
        let n_total = self.trozos.len() as f64;
        let mut puntos: std::collections::HashMap<u32, f64> = std::collections::HashMap::new();
        for t in terminos {
            let Some(p) = self.terminos.get(t) else { continue };
            let n = p.len() as f64;
            let idf = ((n_total - n + 0.5) / (n + 0.5)).ln();
            let idf = if idf <= 0.0 { 1e-6 } else { idf };
            for (id, f) in p {
                let dl = self.trozos[*id as usize].largo as f64;
                let f = *f as f64;
                *puntos.entry(*id).or_default() += idf * (f * (K1 + 1.0)) / (f + K1 * (1.0 - B + B * dl / self.promedio.max(1e-9)));
            }
        }
        let mut v: Vec<(u32, f64)> = puntos.into_iter().collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        v.truncate(tope);
        v
    }
}

/// El índice cargado, por proyecto (se vuelve a leer sólo si cambió en disco).
fn cargado(dir: &Path) -> Result<std::sync::Arc<Indice>> {
    type Cache = std::collections::HashMap<PathBuf, (std::time::SystemTime, u64, std::sync::Arc<Indice>)>;
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Cache>> = std::sync::OnceLock::new();
    let ruta = dir.join("indice.bin");
    let m = std::fs::metadata(&ruta).map_err(|_| sin_indice())?;
    let (fecha, largo) = (m.modified()?, m.len());
    let cache = CACHE.get_or_init(Default::default);
    if let Some((f, l, i)) = cache.lock().unwrap().get(&ruta) {
        if *f == fecha && *l == largo {
            return Ok(i.clone());
        }
    }
    let i = std::sync::Arc::new(Indice::de_bytes(&std::fs::read(&ruta)?)?);
    cache.lock().unwrap().insert(ruta, (fecha, largo, i.clone()));
    Ok(i)
}

/// El cuerpo de un trozo, leído por su posición en `cuerpos.bin`.
fn cuerpo(dir: &Path, t: &Trozo) -> Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(dir.join("cuerpos.bin")).map_err(|_| sin_indice())?;
    f.seek(SeekFrom::Start(t.desde))?;
    let mut b = vec![0u8; t.bytes as usize];
    f.read_exact(&mut b)?;
    Ok(String::from_utf8(b)?)
}

/// El fragmento (como `snippet(…, '»', '«', '…', 12)` de FTS5): la ventana de 12 tokens con más términos
/// de la consulta, cada uno entre » «, y «…» donde se corta.
fn fragmento(cuerpo: &str, buscados: &std::collections::HashSet<&str>) -> String {
    let toks = tokens(cuerpo);
    if toks.is_empty() {
        return String::new();
    }
    let casa: Vec<bool> = toks.iter().map(|t| buscados.contains(t.0.as_str())).collect();
    let ancho = FRAGMENTO.min(toks.len());
    let (mut mejor, mut mejor_n) = (0usize, (0usize, 0usize));
    for s in 0..=toks.len() - ancho {
        let distintos: std::collections::HashSet<&str> = (s..s + ancho).filter(|i| casa[*i]).map(|i| toks[i].0.as_str()).collect();
        let n = (distintos.len(), (s..s + ancho).filter(|i| casa[*i]).count());
        if n > mejor_n {
            (mejor, mejor_n) = (s, n);
        }
    }
    let fin = mejor + ancho;
    let mut out = String::new();
    if mejor > 0 {
        out.push('…');
    }
    let mut pos = toks[mejor].1;
    for (i, (_, a, b)) in toks.iter().enumerate().take(fin).skip(mejor) {
        out.push_str(&cuerpo[pos..*a]);
        if casa[i] {
            out.push('»');
            out.push_str(&cuerpo[*a..*b]);
            out.push('«');
        } else {
            out.push_str(&cuerpo[*a..*b]);
        }
        pos = *b;
    }
    if fin < toks.len() {
        out.push('…');
    }
    out
}

/// Los vectores guardados: (trozo, vector normalizado).
fn leer_vectores(dir: &Path) -> Result<Vec<(i64, Vec<f32>)>> {
    let Ok(b) = std::fs::read(dir.join("vectores.bin")) else { return Ok(vec![]) };
    let (mut l, _) = crate::programar::codec::Lector::nuevo(&b, TIPO_VECTORES, VERSION_INDICE)?;
    let (n, dim) = (l.u32()? as usize, l.u32()? as usize);
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        let id = l.i64()?;
        let mut x = Vec::with_capacity(dim);
        for _ in 0..dim {
            x.push(f32::from_bits(l.u32()?));
        }
        v.push((id, x));
    }
    Ok(v)
}

fn guardar_vectores(dir: &Path, v: &[(i64, Vec<f32>)]) -> Result<()> {
    let dim = v.first().map(|x| x.1.len()).unwrap_or(0);
    let mut e = crate::programar::codec::Escritor::nuevo(TIPO_VECTORES, VERSION_INDICE);
    e.u32(v.len() as u32);
    e.u32(dim as u32);
    for (id, x) in v {
        if x.len() != dim {
            anyhow::bail!("{}", crate::tr!("vectores de largo distinto ({} y {dim})", "vectors of different length ({} and {dim})", x.len()));
        }
        e.i64(*id);
        for f in x {
            e.u32(f.to_bits());
        }
    }
    crate::programar::codec::guardar(&dir.join("vectores.bin"), &e.bytes())
}

pub struct Rag {
    dir: PathBuf,
}

impl Rag {
    pub fn new(root: &Path) -> Self {
        Self { dir: carpeta_indice(root) }
    }

    /// (Re)construye el índice del árbol. Devuelve nº de trozos indexados.
    pub fn build(&self, root: &Path) -> Result<usize> {
        std::fs::create_dir_all(&self.dir)?;
        // el índice de ANTES (SQLite) era una caché de GOTV: se quita por su ruta exacta
        let viejo = root.join(".gotv/index.db");
        if viejo.is_file() {
            let _ = std::fs::remove_file(&viejo);
        }
        // F8.4 paso 5: la principal y cada carpeta EXTRA del proyecto (sus rutas, como `@nombre/…`, que es como
        // las resuelven read_file y edit)
        let mut files: Vec<(PathBuf, String, PathBuf)> = vec![];
        let mut principal = vec![];
        collect(root, &mut principal);
        files.extend(principal.into_iter().map(|f| (root.to_path_buf(), String::new(), f)));
        for (nombre, base) in crate::raices::extras_de(root) {
            let mut v = vec![];
            collect(&base, &mut v);
            files.extend(v.into_iter().map(|f| (base.clone(), format!("@{nombre}/"), f)));
        }
        files.sort_by(|a, b| (&a.1, &a.2).cmp(&(&b.1, &b.2)));
        // trocear y contar, archivo por archivo, en varios hilos; unir en el orden de los archivos (mismos ids)
        let hilos = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8);
        let mut por_archivo: Vec<Option<(String, Vec<TrozoCrudo>)>> = (0..files.len()).map(|_| None).collect();
        std::thread::scope(|s| {
            for (parte, salida) in files.chunks(files.len().div_ceil(hilos).max(1)).zip(por_archivo.chunks_mut(files.len().div_ceil(hilos).max(1))) {
                s.spawn(move || {
                    for ((base, prefijo, f), o) in parte.iter().zip(salida.iter_mut()) {
                        *o = trocear(base, f).map(|(rel, t)| (format!("{prefijo}{rel}"), t));
                    }
                });
            }
        });
        let mut cuerpos = crate::programar::codec::Escritor::nuevo(TIPO_CUERPOS, VERSION_INDICE);
        let mut ix = Indice { rutas: vec![], trozos: vec![], terminos: Default::default(), promedio: 0.0 };
        for (rel, trozos) in por_archivo.into_iter().flatten() {
            let ruta = ix.rutas.len() as u32;
            ix.rutas.push(rel);
            for t in trozos {
                let id = ix.trozos.len() as u32;
                for (termino, n) in t.cuenta {
                    match ix.terminos.get_mut(&termino) {
                        Some(p) => p.push((id, n)),
                        None => {
                            ix.terminos.insert(termino, vec![(id, n)]);
                        }
                    }
                }
                ix.trozos.push(Trozo { ruta, inicio: t.inicio, fin: t.fin, largo: t.largo, desde: cuerpos.largo() as u64, bytes: t.cuerpo.len() as u32 });
                cuerpos.crudo(t.cuerpo.as_bytes());
            }
        }
        // los vectores eran de los trozos de antes: se quitan (como cuando se rehacía la base)
        let vectores = self.dir.join("vectores.bin");
        if vectores.is_file() {
            std::fs::remove_file(&vectores)?;
        }
        crate::programar::codec::guardar(&self.dir.join("cuerpos.bin"), &cuerpos.bytes())?;
        crate::programar::codec::guardar(&self.dir.join("indice.bin"), &ix.bytes())?;
        Ok(ix.trozos.len())
    }

    /// Trozos a vectorizar: (id, «ruta\ncontenido» recortado).
    pub fn chunk_texts(&self) -> Result<Vec<(i64, String)>> {
        let ix = cargado(&self.dir)?;
        let todo = std::fs::read(self.dir.join("cuerpos.bin")).map_err(|_| sin_indice())?;
        Ok(ix
            .trozos
            .iter()
            .enumerate()
            .map(|(id, t)| {
                let body = String::from_utf8_lossy(&todo[t.desde as usize..(t.desde + t.bytes as u64) as usize]);
                (id as i64, format!("{}\n{}", ix.rutas[t.ruta as usize], body.chars().take(2000).collect::<String>()))
            })
            .collect())
    }

    /// Guarda los vectores (normalizados) de cada trozo (reemplaza los del mismo trozo).
    pub fn store_vectors(&self, vecs: &[(i64, Vec<f32>)]) -> Result<()> {
        let mut todos: std::collections::BTreeMap<i64, Vec<f32>> = leer_vectores(&self.dir)?.into_iter().collect();
        for (id, v) in vecs {
            todos.insert(*id, normalized(v));
        }
        guardar_vectores(&self.dir, &todos.into_iter().collect::<Vec<_>>())
    }

    /// ¿El índice tiene vectores? (sólo mira la cabecera: no lee los vectores)
    pub fn has_vectors(&self) -> bool {
        use std::io::Read;
        let mut cab = [0u8; 13];
        std::fs::File::open(self.dir.join("vectores.bin")).and_then(|mut f| f.read_exact(&mut cab)).is_ok()
            && crate::programar::codec::version_de(&cab, TIPO_VECTORES).is_some()
            && u32::from_le_bytes([cab[9], cab[10], cab[11], cab[12]]) > 0
    }

    /// Búsqueda HÍBRIDA: BM25 (top 50) + coseno (top 50) fusionados por RRF (k = 60).
    /// Sin `qvec` es BM25 puro.
    pub fn search_hybrid(&self, query: &str, qvec: Option<&[f32]>, k: usize) -> Result<Vec<Hit>> {
        let ix = cargado(&self.dir)?;
        let terminos = terminos_de(query);
        let mut score: std::collections::HashMap<i64, f64> = std::collections::HashMap::new();
        let mut por_bm25 = std::collections::HashSet::new();
        for (rank, (id, _)) in ix.bm25(&terminos, 50).into_iter().enumerate() {
            *score.entry(id as i64).or_default() += 1.0 / (60.0 + rank as f64 + 1.0);
            por_bm25.insert(id as i64);
        }
        if let Some(qv) = qvec {
            let qv = normalized(qv);
            let mut sims: Vec<(i64, f32)> = leer_vectores(&self.dir)?.into_iter().map(|(id, v)| (id, v.iter().zip(&qv).map(|(a, b)| a * b).sum::<f32>())).collect();
            sims.sort_by(|a, b| b.1.total_cmp(&a.1));
            for (rank, (id, _)) in sims.into_iter().take(50).enumerate() {
                *score.entry(id).or_default() += 1.0 / (60.0 + rank as f64 + 1.0);
            }
        }
        let mut ranked: Vec<(i64, f64)> = score.into_iter().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let buscados: std::collections::HashSet<&str> = terminos.iter().map(String::as_str).collect();
        let mut out = Vec::new();
        for (id, _) in ranked.into_iter().take(k) {
            let Some(t) = ix.trozos.get(id as usize) else { continue };
            let body = cuerpo(&self.dir, t)?;
            let snippet = if por_bm25.contains(&id) {
                fragmento(&body, &buscados)
            } else {
                // sólo lo encontró el vector: las primeras líneas con contenido
                body.lines().filter(|l| !l.trim().is_empty()).take(3).collect::<Vec<_>>().join(" ⏎ ").chars().take(180).collect()
            };
            out.push(Hit { path: ix.rutas[t.ruta as usize].clone(), start: t.inicio as i64, end: t.fin as i64, snippet });
        }
        Ok(out)
    }

    /// Busca y devuelve los mejores trozos (ruta:línea + fragmento).
    pub fn search(&self, query: &str, k: usize) -> Result<Vec<Hit>> {
        let ix = cargado(&self.dir)?;
        let terminos = terminos_de(query);
        if terminos.is_empty() {
            return Ok(vec![]);
        }
        let buscados: std::collections::HashSet<&str> = terminos.iter().map(String::as_str).collect();
        ix.bm25(&terminos, k)
            .into_iter()
            .map(|(id, _)| {
                let t = &ix.trozos[id as usize];
                Ok(Hit { path: ix.rutas[t.ruta as usize].clone(), start: t.inicio as i64, end: t.fin as i64, snippet: fragmento(&cuerpo(&self.dir, t)?, &buscados) })
            })
            .collect()
    }
}

/// Un trozo recién cortado, con sus términos ya contados (lo hace cada hilo de `build`).
struct TrozoCrudo {
    inicio: u32,
    fin: u32,
    largo: u32,
    cuerpo: String,
    cuenta: Vec<(String, u32)>,
}

/// Cuenta los tokens de `texto` en `cuenta` sin pedir memoria por cada repetido (el token se arma en un búfer
/// y sólo se copia la primera vez que aparece). Devuelve cuántos hubo.
fn contar(texto: &str, cuenta: &mut std::collections::HashMap<String, u32>, buf: &mut String) -> u32 {
    let mut n = 0;
    let mut cerrar = |buf: &mut String, n: &mut u32| {
        if !buf.is_empty() {
            match cuenta.get_mut(buf.as_str()) {
                Some(c) => *c += 1,
                None => {
                    cuenta.insert(buf.clone(), 1);
                }
            }
            *n += 1;
            buf.clear();
        }
    };
    for c in texto.chars() {
        if c.is_alphanumeric() {
            if c.is_ascii() {
                buf.push(c.to_ascii_lowercase());
            } else {
                for l in c.to_lowercase() {
                    buf.push(sin_tilde(l));
                }
            }
        } else {
            cerrar(buf, &mut n);
        }
    }
    cerrar(buf, &mut n);
    n
}

/// Los trozos de un archivo (50 líneas, avanzando 40), con sus términos contados (ruta + cuerpo).
fn trocear(root: &Path, f: &Path) -> Option<(String, Vec<TrozoCrudo>)> {
    let text = std::fs::read_to_string(f).ok()?;
    let rel = f.strip_prefix(root).unwrap_or(f).to_string_lossy().to_string();
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return None;
    }
    let (win, step) = (50usize, 40usize);
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut cuenta: std::collections::HashMap<String, u32> = Default::default();
    let mut i = 0usize;
    loop {
        let end = (i + win).min(lines.len());
        let cuerpo = lines[i..end].join("\n");
        cuenta.clear();
        let largo = contar(&rel, &mut cuenta, &mut buf) + contar(&cuerpo, &mut cuenta, &mut buf);
        out.push(TrozoCrudo { inicio: (i + 1) as u32, fin: end as u32, largo, cuerpo, cuenta: cuenta.drain().collect() });
        if end >= lines.len() {
            break;
        }
        i += step;
    }
    Some((rel, out))
}

/// Recorre SIN seguir symlinks (`file_type()` no los sigue): un enlace a otra
/// carpeta no mete código ajeno en el índice ni arma bucles.
fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if EXCLUDE_DIRS.contains(&name.as_str()) {
                continue;
            }
            collect(&p, out);
        } else if ft.is_file() {
            let ext_ok = p
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| EXTS.contains(&x))
                .unwrap_or(false);
            let size_ok = e.metadata().map(|m| m.len() <= MAX_FILE).unwrap_or(false);
            if ext_ok && size_ok {
                out.push(p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(tag: &str) -> crate::CarpetaPrueba {
        let d = crate::CarpetaPrueba::sin_crear(&format!("gotv-rag-{tag}"));
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(
            d.join("src/stock.php"),
            "<?php\nfunction validar_stock($item) {\n  return $item->cantidad >= 0;\n}\n",
        )
        .unwrap();
        std::fs::write(
            d.join("src/otro.rs"),
            "fn saludo() { println!(\"hola\"); }\n",
        )
        .unwrap();
        d
    }

    #[test]
    fn encuentra_por_bm25_con_ruta_y_linea() {
        let d = tree("bm25");
        let rag = Rag::new(&d);
        assert!(rag.build(&d).unwrap() >= 2);
        let hits = rag.search("¿dónde se valida el stock?", 5).unwrap();
        assert_eq!(hits[0].path, "src/stock.php");
        assert_eq!(hits[0].start, 1);
        assert!(hits[0].snippet.contains("»stock«"), "{}", hits[0].snippet);
        // sin índice, el mismo aviso de siempre
        let e = Rag::new(&tree("sin-indice")).search("x", 3).err().unwrap().to_string();
        assert!(e.contains("gotv index"), "{e}");
    }

    #[test]
    fn hibrida_rrf_con_vectores_fabricados() {
        let d = tree("hib");
        std::fs::write(
            d.join("src/auth.rs"),
            "fn check_credentials(u: &str, p: &str) -> bool { true }\n",
        )
        .unwrap();
        let rag = Rag::new(&d);
        rag.build(&d).unwrap();
        assert!(!rag.has_vectors());
        // vectores a mano: auth.rs apunta al eje 0; el resto, al eje 1
        let vecs: Vec<(i64, Vec<f32>)> = rag
            .chunk_texts()
            .unwrap()
            .into_iter()
            .map(|(id, t)| {
                (
                    id,
                    if t.starts_with("src/auth.rs") {
                        vec![1.0, 0.0]
                    } else {
                        vec![0.0, 1.0]
                    },
                )
            })
            .collect();
        rag.store_vectors(&vecs).unwrap();
        assert!(rag.has_vectors());
        // BM25 no ve nada (ninguna palabra en común) …
        assert!(rag
            .search_hybrid("¿dónde se valida el login?", None, 5)
            .unwrap()
            .is_empty());
        // … el vector sí
        let h = rag
            .search_hybrid("¿dónde se valida el login?", Some(&[0.9, 0.1]), 5)
            .unwrap();
        assert_eq!(h[0].path, "src/auth.rs");
        assert!(
            h[0].snippet.contains("check_credentials"),
            "fragmento del trozo: {}",
            h[0].snippet
        );
        // y lo que encuentran AMBOS sube primero: «stock» por BM25 y por vector
        let h = rag
            .search_hybrid("validar stock", Some(&[0.0, 1.0]), 5)
            .unwrap();
        assert_eq!(h[0].path, "src/stock.php");
        // rehacer el índice quita los vectores (eran de los trozos de antes)
        rag.build(&d).unwrap();
        assert!(!rag.has_vectors());
    }

    #[test]
    fn no_sigue_symlinks() {
        let d = tree("link");
        // lo ajeno vive FUERA del proyecto, en su propia carpeta de prueba (antes era un
        // hermano `.ajeno` que nadie borraba)
        let ajeno = crate::CarpetaPrueba::nueva("gotv-rag-ajeno");
        std::fs::write(ajeno.join("secreto.rs"), "fn ajeno_unico_xyz() {}\n").unwrap();
        std::os::unix::fs::symlink(&ajeno, d.join("enlace")).unwrap();
        let rag = Rag::new(&d);
        rag.build(&d).unwrap();
        assert!(rag.search("ajeno_unico_xyz", 5).unwrap().is_empty());
    }

    #[test]
    fn consultas_raras_no_rompen_nada() {
        let d = tree("raras");
        let rag = Rag::new(&d);
        rag.build(&d).unwrap();
        for q in ["\"", "a AND (", "*", "NEAR(", "", "x:y", "OR OR", "ñ", "🕳️"] {
            assert!(rag.search(q, 3).is_ok(), "rompió con {q:?}");
            assert!(rag.search_hybrid(q, None, 3).is_ok(), "rompió con {q:?}");
        }
    }

    #[test]
    fn tokens_como_unicode61() {
        let t: Vec<String> = tokens("Validación_del STOCK: ñandú-2 Öl").into_iter().map(|x| x.0).collect();
        assert_eq!(t, ["validacion", "del", "stock", "nandu", "2", "ol"]);
        assert_eq!(terminos_de("¿Dónde se valida? a «x»"), ["donde", "se", "valida"]);
    }

    /// El juez: SQLite FTS5 (sólo en esta prueba) sobre los MISMOS trozos. Cada fila que devuelve FTS5 tiene el
    /// mismo puntaje BM25 en el índice propio, y salen en el mismo orden.
    #[test]
    fn el_puntaje_es_el_mismo_que_el_de_fts5() {
        use rusqlite::{params, Connection};
        let d = tree("juez");
        std::fs::write(d.join("src/permisos.rs"), "// ¿Dónde se valida el permiso? Aquí: validar_permiso\nfn validar_permiso(cmd: &str) -> bool {\n    // la validación del permiso de bash\n    !cmd.contains(\"rm -rf\")\n}\n").unwrap();
        std::fs::write(d.join("src/notas.md"), "# Notas\n\nEl permiso se pide una vez. Permiso, permiso, permiso.\nLa compactación del contexto.\n").unwrap();
        let largo: String = (0..120).map(|i| format!("línea {i} de relleno con stock y más texto\n")).collect();
        std::fs::write(d.join("src/largo.txt"), largo).unwrap();
        let rag = Rag::new(&d);
        let n = rag.build(&d).unwrap();
        let ix = cargado(&rag.dir).unwrap();
        // los mismos trozos en FTS5, en el mismo orden (rowid = id + 1)
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE VIRTUAL TABLE chunks USING fts5(path, startln UNINDEXED, endln UNINDEXED, body);").unwrap();
        for t in &ix.trozos {
            c.execute("INSERT INTO chunks(path,startln,endln,body) VALUES (?1,?2,?3,?4)", params![ix.rutas[t.ruta as usize], t.inicio, t.fin, cuerpo(&rag.dir, t).unwrap()]).unwrap();
        }
        assert_eq!(c.query_row("SELECT count(*) FROM chunks", [], |r| r.get::<_, i64>(0)).unwrap(), n as i64);
        for q in ["¿dónde se valida el permiso de bash?", "permiso", "stock", "validación del contexto", "VALIDAR_PERMISO", "Permiso permiso", "relleno 7"] {
            let fts: String = q.split(|c: char| !c.is_alphanumeric()).filter(|t| t.chars().count() >= 2).map(|t| format!("\"{}\"", t.to_lowercase())).collect::<Vec<_>>().join(" OR ");
            let mut st = c.prepare("SELECT rowid, bm25(chunks) FROM chunks WHERE chunks MATCH ?1 ORDER BY bm25(chunks), rowid LIMIT 50").unwrap();
            let de_fts: Vec<(u32, f64)> = st.query_map(params![fts], |r| Ok(((r.get::<_, i64>(0)? - 1) as u32, -r.get::<_, f64>(1)?))).unwrap().map(|x| x.unwrap()).collect();
            let propio = ix.bm25(&terminos_de(q), 50);
            assert_eq!(propio.len(), de_fts.len(), "«{q}»: {propio:?} vs {de_fts:?}");
            for ((a, pa), (b, pb)) in propio.iter().zip(&de_fts) {
                assert!((pa - pb).abs() < 1e-9, "«{q}»: puntaje {pa} vs FTS5 {pb}");
                // mismo puntaje → el orden sólo puede diferir en empates exactos
                assert!(a == b || (pa - pb).abs() < 1e-12, "«{q}»: orden {propio:?} vs {de_fts:?}");
            }
        }
    }

    /// El índice se carga UNA vez y se vuelve a leer sólo si cambió; el índice viejo de SQLite se quita.
    #[test]
    fn la_cache_se_renueva_al_rehacer_y_el_indice_viejo_se_quita() {
        let d = tree("cache");
        std::fs::create_dir_all(d.join(".gotv")).unwrap();
        std::fs::write(d.join(".gotv/index.db"), b"SQLite format 3\0").unwrap();
        let rag = Rag::new(&d);
        rag.build(&d).unwrap();
        assert!(!d.join(".gotv/index.db").exists(), "el de antes, por su ruta exacta");
        assert!(rag.search("nuevo_unico", 3).unwrap().is_empty());
        std::fs::write(d.join("src/nuevo.rs"), "fn nuevo_unico() {}\n").unwrap();
        rag.build(&d).unwrap();
        assert_eq!(rag.search("nuevo_unico", 3).unwrap()[0].path, "src/nuevo.rs", "rehecho, no se sirve el de la caché");
        // y en disco es el formato propio
        assert!(std::fs::read(d.join(".gotv/indice/indice.bin")).unwrap().starts_with(b"GOTVRAGI"));
    }

    fn copiar(de: &Path, a: &Path) {
        std::fs::create_dir_all(a).unwrap();
        for e in std::fs::read_dir(de).unwrap().flatten() {
            let (p, q) = (e.path(), a.join(e.file_name()));
            let ft = e.file_type().unwrap();
            if ft.is_dir() {
                copiar(&p, &q);
            } else if ft.is_file() {
                std::fs::copy(&p, &q).unwrap();
            }
        }
    }

    /// BANCO, a mano (sin modelos): indexa una COPIA de `crates/` del repo y mide construir y buscar; imprime los
    /// 3 primeros de cada consulta para comparar con el motor de antes (SQLite: 3 241 trozos en 201 ms y 1,72 ms
    /// por consulta, medido el 2026-11-17 con este mismo banco).
    /// `cargo test -q --release -p gotv-core --lib -- --ignored --nocapture banco_del_indice`
    #[test]
    #[ignore]
    fn banco_del_indice() {
        let fuente = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let d = crate::CarpetaPrueba::nueva("gotv-rag-banco");
        copiar(&fuente, &d.join("crates"));
        let rag = Rag::new(&d);
        let t = std::time::Instant::now();
        let n = rag.build(&d).unwrap();
        println!("construir: {n} trozos en {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
        let consultas = ["dónde se valida el permiso de bash", "vigilar archivos con FSEvents", "compactar el contexto", "LaunchAgent plist", "check credentials", "cargar el modelo mlx", "tool_search catálogo de familias", "notificaciones del sistema"];
        let t = std::time::Instant::now();
        let _ = rag.search("primera", 3).unwrap();
        println!("primera consulta (carga el índice): {:.2} ms", t.elapsed().as_secs_f64() * 1e3);
        let t = std::time::Instant::now();
        let mut tops = vec![];
        for _ in 0..10 {
            tops.clear();
            for q in consultas {
                tops.push(rag.search(q, 3).unwrap().iter().map(|h| format!("{}:{}", h.path, h.start)).collect::<Vec<_>>());
            }
        }
        println!("buscar: {:.2} ms por consulta (80 consultas)", t.elapsed().as_secs_f64() * 1e3 / 80.0);
        for (q, top) in consultas.iter().zip(&tops) {
            println!("«{q}» → {top:?}");
        }
    }
}
