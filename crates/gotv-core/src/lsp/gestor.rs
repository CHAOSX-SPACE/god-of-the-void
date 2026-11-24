//! F5 · ML1 · varios servidores de lenguaje a la vez. Cada archivo va al servidor de su extensión y a la
//! raíz más cercana (`raiz.rs`): un proceso por (servidor, raíz), arrancado al tocar el primer archivo de
//! ese lenguaje. Si cae, se relanza (3 intentos). Los ociosos se apagan a los 15 min, NUNCA el del
//! lenguaje que se está usando. En una app Tauri, el Rust de `src-tauri/` y el TypeScript del frontend
//! tienen cada uno su servidor (antes: uno solo, el primero que encajaba, y el otro lado sin nada).
use super::registro::{self, Def};
use super::{Diag, Lsp, Server};
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

type Clave = (&'static str, PathBuf);

struct Entrada {
    lsp: Lsp,
    ultimo: Instant,
}

pub struct Gestor {
    root: PathBuf,
    defs: Vec<Def>,
    vivos: HashMap<Clave, Entrada>,
    caidas: HashMap<Clave, u32>,
    /// lo que pasó (arranques, caídas, apagados) para que quien tenga el frente lo cuente
    avisos: Vec<String>,
    /// tras cuánto sin uso se apaga un servidor
    pub ocio: Duration,
    /// tope de memoria de TODOS los servidores juntos, en MB (por omisión, 1/8 de la RAM del equipo)
    pub tope_ram_mb: u64,
    /// F16.3: la raíz es el ancla de un proyecto REMOTO (los servidores corren allá)
    remoto: bool,
    /// F16.3: lenguajes que el equipo remoto no tiene (no se vuelve a preguntar en esta sesión)
    sin_remoto: Vec<&'static str>,
}

/// La memoria residente de un proceso, en MB (`ps`; 0 si no se puede medir).
fn rss_mb(pid: u32) -> u64 {
    std::process::Command::new("ps").args(["-o", "rss=", "-p", &pid.to_string()]).output().ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok()).unwrap_or(0) / 1024
}

/// 1/8 de la RAM del equipo, en MB (medido: un rust-analyzer en un proyecto mínimo ya ocupa ~530 MB; los
/// modelos locales necesitan el resto).
fn tope_por_omision() -> u64 {
    std::process::Command::new("sysctl").args(["-n", "hw.memsize"]).output().ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok()).map(|b| b / 8 / 1_048_576).unwrap_or(4_096)
}

impl Gestor {
    pub fn new(root: &Path) -> Gestor {
        Gestor::con(root, registro::defs())
    }

    pub fn con(root: &Path, defs: Vec<Def>) -> Gestor {
        let remoto = crate::lugar::ficha(root).is_some();
        Gestor { root: root.canonicalize().unwrap_or_else(|_| root.to_path_buf()), defs, vivos: HashMap::new(), caidas: HashMap::new(), avisos: Vec::new(), ocio: Duration::from_secs(15 * 60), tope_ram_mb: tope_por_omision(), remoto, sin_remoto: Vec::new() }
    }

    /// ¿Hay servidor instalado para este archivo?
    pub fn handles(&self, p: &Path) -> bool {
        // F16.3: en un proyecto remoto el servidor es el de ALLÁ (se busca al arrancar y, si falta, se dice)
        if self.remoto {
            return registro::para(&self.defs, p).is_some() && !self.sin_remoto.contains(&registro::para(&self.defs, p).map(|d| d.id).unwrap_or(""));
        }
        registro::para(&self.defs, p).is_some_and(|d| registro::comando(d).is_some())
    }

    /// ¿El proyecto tiene algún lenguaje con servidor instalado? (por los marcadores de la raíz o de una
    /// carpeta de primer nivel: `src-tauri/Cargo.toml`, `web/package.json`)
    pub fn hay_servidor(&self) -> bool {
        self.lenguajes_del_proyecto().iter().any(|d| registro::comando(d).is_some())
    }

    /// Los lenguajes que el proyecto declara (raíz y carpetas de primer nivel).
    pub fn lenguajes_del_proyecto(&self) -> Vec<&Def> {
        // F8.4 paso 5: también las carpetas EXTRA del proyecto (y sus carpetas de primer nivel)
        let mut bases = vec![self.root.clone()];
        bases.extend(crate::raices::extras_de(&self.root).into_iter().map(|(_, b)| b));
        let mut dirs = bases.clone();
        for b in &bases {
            dirs.extend(std::fs::read_dir(b).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.') || n == "node_modules" || n == "target")));
        }
        self.defs.iter().filter(|d| dirs.iter().any(|x| d.raices.iter().any(|m| registro::marca(x, m)))).collect()
    }

    /// Lo que falta instalar para los lenguajes del proyecto (para decirlo una vez).
    pub fn faltan(&self) -> Vec<String> {
        self.lenguajes_del_proyecto().into_iter().filter(|d| registro::comando(d).is_none()).map(|d| crate::tr!("proyecto {} sin servidor de lenguaje: `toolchain` action=install name={} (`{}`, con permiso del usuario) activa diagnósticos y navegación", "{} project without a language server: `toolchain` action=install name={} (`{}`, with the user’s permission) turns on diagnostics and navigation", d.lang, d.candidatos.first().and_then(|c| c.first()).map(String::as_str).unwrap_or(d.id), d.instalar)).collect()
    }

    /// Lo que pasó desde la última vez (y se vacía).
    pub fn tomar_avisos(&mut self) -> Vec<String> {
        std::mem::take(&mut self.avisos)
    }

    /// Cuántos servidores corren ahora: (servidor, raíz).
    pub fn activos(&self) -> Vec<(String, PathBuf)> {
        let mut v: Vec<(String, PathBuf)> = self.vivos.keys().map(|(i, r)| (i.to_string(), r.clone())).collect();
        v.sort();
        v
    }

    /// El servidor de este archivo: el que ya corre para su (servidor, raíz), o uno nuevo.
    pub async fn para(&mut self, p: &Path) -> Result<&mut Lsp> {
        let d = registro::para(&self.defs, p).cloned().ok_or_else(|| anyhow!("{}", crate::tr!("no hay servidor de lenguaje para {}", "there is no language server for {}", p.display())))?;
        let raiz = super::raiz::raiz_para(p, &d, &self.root);
        let clave: Clave = (d.id, raiz.clone());
        self.apagar_ociosos(&clave);
        // la memoria de un servidor crece con el tiempo: el tope se mira en cada uso, no sólo al arrancar
        self.respetar_tope(&clave);
        // cayó: se quita, y se relanza si aún quedan intentos
        if self.vivos.get(&clave).is_some_and(|e| !e.lsp.vivo()) {
            self.vivos.remove(&clave);
            let n = self.caidas.entry(clave.clone()).or_insert(0);
            *n += 1;
            self.avisos.push(crate::tr!("⚠ el servidor de {} cayó ({n}/3)", "⚠ the {} server crashed ({n}/3)", d.lang));
        }
        if !self.vivos.contains_key(&clave) {
            if self.caidas.get(&clave).is_some_and(|n| *n >= 3) {
                return Err(anyhow!("{}", crate::tr!("el servidor de {} cayó 3 veces: no se relanza en esta sesión (su registro está en .gotv/registros/)", "the {} server crashed 3 times: it is not relaunched this session (its log is in .gotv/registros/)", d.lang)));
            }
            // F16.3: proyecto remoto → el servidor que tenga el equipo de ALLÁ, corriendo allá
            let puente = if self.remoto { super::remoto::Puente::de(&self.root).map(std::sync::Arc::new) } else { None };
            let (cmd, path_remoto) = match &puente {
                Some(b) => match b.buscar(&d.candidatos, d.instalar).await {
                    Ok(x) => (x.0, Some(x.1)),
                    Err(e) => {
                        self.sin_remoto.push(d.id);
                        return Err(e);
                    }
                },
                None => (registro::comando(&d).ok_or_else(|| anyhow!("{}", crate::tr!("proyecto {} sin servidor de lenguaje: `{}`", "{} project without a language server: `{}`", d.lang, d.instalar)))?, None),
            };
            let nombre = std::path::Path::new(&cmd[0]).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let server = Server { lang: d.lang, cmd, exts: d.exts };
            let lsp = match (puente, path_remoto) {
                (Some(b), Some(path)) => Lsp::start_remoto(server, &raiz, b, &path).await?,
                _ => Lsp::start(server, &raiz).await?,
            };
            let nombre = if self.remoto { crate::tr!("{nombre} (en el equipo remoto)", "{nombre} (on the remote machine)") } else { nombre };
            let donde = raiz.strip_prefix(&self.root).ok().filter(|r| !r.as_os_str().is_empty()).map(|r| format!(" ({})", r.display())).unwrap_or_default();
            self.avisos.push(crate::tr!("· LSP: {nombre}{donde}", "· LSP: {nombre}{donde}"));
            self.vivos.insert(clave.clone(), Entrada { lsp, ultimo: Instant::now() });
        }
        let e = self.vivos.get_mut(&clave).unwrap();
        e.ultimo = Instant::now();
        Ok(&mut e.lsp)
    }

    /// Antes de arrancar otro: si los vivos pasan del tope de memoria, se apaga el más ocioso (nunca el que
    /// se va a usar); si no queda a quién, se avisa y se sigue.
    fn respetar_tope(&mut self, excepto: &Clave) {
        loop {
            let total: u64 = self.vivos.values().filter_map(|e| e.lsp.pid()).map(rss_mb).sum();
            if total <= self.tope_ram_mb {
                return;
            }
            let ocioso = self.vivos.iter().filter(|(k, _)| *k != excepto).min_by_key(|(_, e)| e.ultimo).map(|(k, _)| k.clone());
            match ocioso {
                Some(k) => {
                    self.vivos.remove(&k);
                    self.avisos.push(crate::tr!("· LSP de {} apagado: los servidores pasaban de {} MB (tope de memoria)", "· {} LSP shut down: the servers went over {} MB (memory cap)", k.0, self.tope_ram_mb));
                }
                None => {
                    self.avisos.push(crate::tr!("⚠ los servidores de lenguaje ocupan {total} MB (tope {} MB) y no hay otro que apagar", "⚠ the language servers use {total} MB (cap {} MB) and there is no other to shut down", self.tope_ram_mb));
                    return;
                }
            }
        }
    }

    /// Apaga los que llevan más de `ocio` sin uso, menos el que se va a usar.
    fn apagar_ociosos(&mut self, excepto: &Clave) {
        let ocio = self.ocio;
        let viejos: Vec<Clave> = self.vivos.iter().filter(|(k, e)| *k != excepto && e.ultimo.elapsed() > ocio).map(|(k, _)| k.clone()).collect();
        for k in viejos {
            self.vivos.remove(&k);
            self.avisos.push(crate::tr!("· LSP de {} apagado por desuso", "· {} LSP shut down for disuse", k.0));
        }
    }

    /// F5 ML3: los cambios que propone el servidor del archivo (no se aplican aquí).
    pub async fn cambios(&mut self, action: &str, p: &Path, symbol: &str, line: Option<u64>, nuevo: Option<&str>) -> Result<Vec<(PathBuf, Vec<super::acciones::Edicion>)>> {
        self.para(p).await?.cambios(action, p, symbol, line, nuevo).await
    }

    pub async fn diagnostics_full(&mut self, p: &Path, wait: Duration) -> Result<(Vec<Diag>, bool)> {
        self.para(p).await?.diagnostics_full(p, wait).await
    }

    /// F5 ML2 · antes de la PRIMERA edición de un archivo de un lenguaje con servidor: si ninguno corre aún
    /// para él, se arranca y se espera su primer análisis, para que la foto de «antes» exista. Medido
    /// 2026-11-16 (banco `f5-firma`): sin esto, cambiar la firma de una función usada en 3 archivos sólo
    /// decía «calc.rs: sin errores» (el servidor nacía con la edición y no había con qué comparar).
    /// Devuelve si arrancó uno.
    pub async fn calentar(&mut self, p: &Path, wait: Duration) -> bool {
        let Some(d) = registro::para(&self.defs, p) else { return false };
        if registro::comando(d).is_none() {
            return false;
        }
        let clave: Clave = (d.id, super::raiz::raiz_para(p, d, &self.root));
        if self.vivos.get(&clave).is_some_and(|e| e.lsp.vivo()) {
            return false;
        }
        self.diagnostics_full(p, wait).await.is_ok()
    }

    /// Una pregunta sobre un archivo; `workspace` (sin archivo) va a los servidores que ya corren o, si
    /// no hay, al del primer lenguaje del proyecto.
    pub async fn query(&mut self, action: &str, p: &Path, symbol: &str, line: Option<u64>) -> Result<String> {
        if action == "workspace" && registro::para(&self.defs, p).is_none() {
            let mut out = Vec::new();
            let claves: Vec<Clave> = self.vivos.keys().cloned().collect();
            for k in claves {
                if let Some(e) = self.vivos.get_mut(&k) {
                    if let Ok(r) = e.lsp.query(action, p, symbol, line).await {
                        if !r.trim().is_empty() {
                            out.push(r);
                        }
                    }
                }
            }
            if out.is_empty() {
                // nadie corre: arranca el del lenguaje principal con un archivo cualquiera de él
                let principal = self.lenguajes_del_proyecto().into_iter().find(|d| registro::comando(d).is_some()).cloned();
                if let Some(d) = principal {
                    let falso = self.root.join(format!("_.{}", d.exts[0]));
                    return self.para(&falso).await?.query(action, p, symbol, line).await;
                }
            }
            return Ok(out.join("\n"));
        }
        self.para(p).await?.query(action, p, symbol, line).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def_falso(id: &'static str, exts: &'static [&'static str], raices: &'static [&'static str], mas_alta: bool) -> Def {
        Def { id, lang: id, exts, candidatos: vec![super::super::falso::tests::comando_falso()], raices, mas_alta, instalar: "-", solo_abiertos: false, requiere: &[] }
    }

    /// ML1 en una app Tauri de juguete: el Rust de `src-tauri/` y el TypeScript de `web/` tienen cada uno
    /// su servidor (y su raíz); el error de cada lado llega; un servidor caído se relanza; el ocioso se
    /// apaga y el que se usa, no.
    #[tokio::test]
    async fn varios_servidores_en_una_app_tauri() {
        std::env::set_var("GOTV_LSP_FALSO", "1");
        let d = crate::CarpetaPrueba::nueva("gotv-lsp-tauri");
        let r = d.canonicalize().unwrap();
        for (f, t) in [("web/package.json", "{}"), ("web/src/App.ts", "const x = 1;\nERROR aquí\n"), ("src-tauri/Cargo.toml", "[package]"), ("src-tauri/src/main.rs", "fn main() {}\n")] {
            std::fs::create_dir_all(r.join(f).parent().unwrap()).unwrap();
            std::fs::write(r.join(f), t).unwrap();
        }
        let mut g = Gestor::con(&r, vec![def_falso("rust", &["rs"], &["Cargo.toml"], true), def_falso("typescript", &["ts"], &["package.json"], false)]);
        assert!(g.hay_servidor() && g.handles(&r.join("web/src/App.ts")) && !g.handles(&r.join("README.md")));
        let (ts, completo) = g.diagnostics_full(&r.join("web/src/App.ts"), Duration::from_secs(10)).await.unwrap();
        assert!(completo && ts.len() == 1 && ts[0].line == 2, "{ts:?}");
        let (rs, _) = g.diagnostics_full(&r.join("src-tauri/src/main.rs"), Duration::from_secs(10)).await.unwrap();
        assert!(rs.is_empty());
        assert_eq!(g.activos(), vec![("rust".to_string(), r.join("src-tauri")), ("typescript".to_string(), r.join("web"))]);
        let avisos = g.tomar_avisos();
        assert_eq!(avisos.iter().filter(|a| a.contains("LSP:")).count(), 2, "{avisos:?}");
        // el servidor de Rust cae: el siguiente uso lo relanza y lo dice
        std::fs::write(r.join("src-tauri/src/main.rs"), "CAER\n").unwrap();
        let _ = g.diagnostics_full(&r.join("src-tauri/src/main.rs"), Duration::from_secs(3)).await;
        std::fs::write(r.join("src-tauri/src/main.rs"), "fn main() {}\nERROR\n").unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let (rs, _) = g.diagnostics_full(&r.join("src-tauri/src/main.rs"), Duration::from_secs(10)).await.unwrap();
        assert_eq!(rs.len(), 1, "relanzado y diagnosticando");
        assert!(g.tomar_avisos().iter().any(|a| a.contains("cayó") || a.contains("crashed")));
        // tope de memoria: con 0 MB, arrancar otro apaga el ocioso; el que se usa sigue
        g.tope_ram_mb = 0;
        std::fs::write(r.join("web/src/App.ts"), "ok\n").unwrap();
        let _ = g.diagnostics_full(&r.join("web/src/App.ts"), Duration::from_secs(10)).await.unwrap();
        assert_eq!(g.activos().iter().map(|a| a.0.as_str()).collect::<Vec<_>>(), ["typescript"], "el de Rust (ocioso) se apagó por el tope");
        assert!(g.tomar_avisos().iter().any(|a| a.contains("tope") || a.contains("cap")));
        g.tope_ram_mb = u64::MAX;
        let _ = g.diagnostics_full(&r.join("src-tauri/src/main.rs"), Duration::from_secs(10)).await.unwrap();
        // el ocioso (TypeScript) se apaga; el que se usa (Rust), no
        g.ocio = Duration::from_millis(1);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let _ = g.diagnostics_full(&r.join("src-tauri/src/main.rs"), Duration::from_secs(10)).await.unwrap();
        assert_eq!(g.activos().len(), 1);
        assert_eq!(g.activos()[0].0, "rust");
    }

    /// Un servidor que, como rust-analyzer en frío, publica una lista VACÍA mientras carga el proyecto y los
    /// errores de verdad 2,5 s después: GOTV espera a que diga que está quieto (`experimental/serverStatus`).
    /// Sin eso, «1,5 s sin novedades» daba un falso «sin errores» (la prueba de rust-analyzer real se cayó así
    /// con la Mac a carga 123, 2026-11-17).
    #[tokio::test]
    async fn no_cree_la_lista_vacia_de_un_servidor_que_aun_carga() {
        std::env::set_var("GOTV_LSP_FALSO", "1");
        let d = crate::CarpetaPrueba::nueva("gotv-lsp-carga-lenta");
        let r = d.canonicalize().unwrap();
        std::fs::write(r.join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(r.join("main.rs"), "CARGA_LENTA\nERROR aquí\n").unwrap();
        let mut g = Gestor::con(&r, vec![def_falso("rust", &["rs"], &["Cargo.toml"], true)]);
        let (diags, completo) = g.diagnostics_full(&r.join("main.rs"), Duration::from_secs(15)).await.unwrap();
        assert!(completo && diags.len() == 1 && diags[0].line == 2, "falso negativo: {diags:?}");
    }

    /// ML3: renombrar en todo el proyecto (dos archivos), arreglar el error de una línea (el import), y las
    /// preguntas nuevas, en español.
    #[tokio::test]
    async fn preguntas_y_cambios() {
        std::env::set_var("GOTV_LSP_FALSO", "1");
        let d = crate::CarpetaPrueba::nueva("gotv-lsp-ml3");
        let r = d.canonicalize().unwrap();
        std::fs::write(r.join("package.json"), "{}").unwrap();
        std::fs::write(r.join("a.ts"), "function suma(x) { return x; }\nsuma(1);\n").unwrap();
        std::fs::write(r.join("b.ts"), "const y = 2;\nsuma(y);\nERROR falta\n").unwrap();
        let mut g = Gestor::con(&r, vec![def_falso("typescript", &["ts"], &["package.json"], false)]);
        let _ = g.diagnostics_full(&r.join("a.ts"), Duration::from_secs(10)).await.unwrap();
        let _ = g.diagnostics_full(&r.join("b.ts"), Duration::from_secs(10)).await.unwrap();
        // renombrar: el servidor calcula, GOTV aplica en los dos archivos
        let c = g.cambios("renombrar", &r.join("a.ts"), "suma", Some(1), Some("sumar")).await.unwrap();
        let hecho = super::super::acciones::aplicar_en_disco(&r, &c).unwrap();
        assert_eq!(hecho.iter().map(|h| h.1).sum::<usize>(), 3);
        assert_eq!(std::fs::read_to_string(r.join("a.ts")).unwrap(), "function sumar(x) { return x; }\nsumar(1);\n");
        assert!(std::fs::read_to_string(r.join("b.ts")).unwrap().contains("sumar(y);"));
        assert!(g.cambios("renombrar", &r.join("a.ts"), "sumar", Some(1), None).await.is_err(), "sin `nuevo`");
        // arreglar el error de la línea 3 de b.ts: el import
        let _ = g.diagnostics_full(&r.join("b.ts"), Duration::from_secs(10)).await.unwrap();
        let c = g.cambios("arreglar", &r.join("b.ts"), "", Some(3), None).await.unwrap();
        super::super::acciones::aplicar_en_disco(&r, &c).unwrap();
        assert!(std::fs::read_to_string(r.join("b.ts")).unwrap().starts_with("use falso::Algo;\n"));
        assert!(g.cambios("arreglar", &r.join("b.ts"), "", Some(1), None).await.is_err(), "sin errores en esa línea no hay arreglo");
        // preguntas en español
        let _ = g.diagnostics_full(&r.join("a.ts"), Duration::from_secs(10)).await.unwrap();
        let imp = g.query("implementaciones", &r.join("a.ts"), "sumar", Some(1)).await.unwrap();
        assert!(imp.contains("a.ts:1"), "{imp}");
        assert!(g.query("quien_llama", &r.join("a.ts"), "sumar", Some(1)).await.unwrap().contains("llamador"));
        assert!(g.query("a_quien_llama", &r.join("a.ts"), "sumar", Some(1)).await.unwrap().contains("llamado"));
        assert!(g.query("esquema", &r.join("a.ts"), "", None).await.unwrap().contains("falso"));
    }

    /// ML2 con un servidor que sólo mira lo ABIERTO (como TypeScript): renombrar `suma` en a.ts rompe b.ts,
    /// que está cerrado. Sin abrirlo, nadie lo diría; `preparar` lo abre antes de la foto (por las
    /// referencias de lo que se va a tocar) y el error sale como nuevo, sin contar los que ya estaban. Y
    /// `arregla`: resuelto, no había error, sigue, mal escrito.
    #[tokio::test]
    async fn vecinos_abiertos_y_arregla() {
        std::env::set_var("GOTV_LSP_FALSO", "1");
        let d = crate::CarpetaPrueba::nueva("gotv-lsp-vecinos");
        let r = d.canonicalize().unwrap();
        std::fs::write(r.join("package.json"), "{}").unwrap();
        std::fs::write(r.join("a.ts"), "function suma(x) { return x; }\nERROR roto\n").unwrap();
        std::fs::write(r.join("b.ts"), "const y = 1;\nUSA:suma\n").unwrap();
        std::fs::write(r.join("c.ts"), "nada que ver\n").unwrap();
        let mut def = def_falso("typescript", &["ts"], &["package.json"], false);
        def.solo_abiertos = true;
        let mut g = Gestor::con(&r, vec![def]);
        // sin servidor vivo, preparar no arranca uno
        assert_eq!(g.preparar(&r.join("a.ts"), 1, 1).await, 0);
        let _ = g.diagnostics_full(&r.join("a.ts"), Duration::from_secs(10)).await.unwrap();
        // la línea 1 (la función) se va a tocar: b.ts la usa y se abre; c.ts no
        assert_eq!(g.preparar(&r.join("a.ts"), 1, 1).await, 1);
        assert_eq!(g.preparar(&r.join("a.ts"), 1, 1).await, 0, "ya abierto");
        let foto = g.foto().await;
        std::fs::write(r.join("a.ts"), "function sumar(x) { return x; }\nbien\n").unwrap();
        let _ = g.diagnostics_full(&r.join("a.ts"), Duration::from_secs(10)).await.unwrap();
        let inf = g.comparar(&foto, &r.join("a.ts")).await.unwrap();
        assert_eq!((inf.nuevos.len(), inf.resueltos), (1, 1), "{inf:?}");
        assert_eq!((inf.nuevos[0].0.clone(), inf.nuevos[0].1), (r.join("b.ts"), 2));
        let t = g.arreglos(&foto, &["a.ts:2".into(), "a.ts:1".into(), "b.ts".into()]).await.unwrap();
        assert!(t.contains("✓ a.ts:2") && t.contains("a.ts:1 no tenía") && t.contains("? «b.ts»"), "{t}");
        // dice arreglar b.ts:2 pero no lo arregla: sigue
        let foto = g.foto().await;
        std::fs::write(r.join("a.ts"), "function sumar(x) { return x + 0; }\nbien\n").unwrap();
        let _ = g.diagnostics_full(&r.join("a.ts"), Duration::from_secs(10)).await.unwrap();
        let t = g.arreglos(&foto, &["b.ts:2".into()]).await.unwrap();
        assert!(t.contains("✗ b.ts:2 sigue") && t.contains("no existe"), "{t}");
        assert!(g.arreglos(&foto, &[]).await.is_none());
    }

    /// Los mensajes de error que el servidor tiene ahora para el archivo (por su nombre).
    async fn errores_de(g: &Gestor, nombre: &str) -> Vec<String> {
        g.foto().await.por_servidor.values().flat_map(|m| m.iter()).filter(|(u, _)| u.ends_with(nombre)).flat_map(|(_, d)| d.iter().map(|(_, m)| m.clone())).collect()
    }

    /// Espera (máx. 5 s) a que el servidor publique para `nombre` lo que pide `listo`.
    async fn hasta(g: &Gestor, nombre: &str, listo: impl Fn(&[String]) -> bool) -> Vec<String> {
        let fin = Instant::now() + Duration::from_secs(5);
        loop {
            let e = errores_de(g, nombre).await;
            if listo(&e) || Instant::now() > fin {
                return e;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// ML6: lo que cambió POR FUERA (un `bash`) llega al servidor: un archivo nuevo que define lo que
    /// faltaba arregla al que lo usa; uno abierto reescrito en disco se resincroniza; uno borrado se cierra.
    /// Sin servidor vivo no se arranca ninguno; lo que no es de su lenguaje no se avisa.
    #[tokio::test]
    async fn cambios_por_fuera_llegan_al_servidor() {
        std::env::set_var("GOTV_LSP_FALSO", "1");
        let d = crate::CarpetaPrueba::nueva("gotv-lsp-por-fuera");
        let r = d.canonicalize().unwrap();
        std::fs::write(r.join("package.json"), "{}").unwrap();
        std::fs::write(r.join("b.ts"), "const y = 1;\nUSA:nueva\n").unwrap();
        let mut g = Gestor::con(&r, vec![def_falso("typescript", &["ts"], &["package.json"], false)]);
        assert_eq!(g.por_fuera(&[(r.join("a.ts"), 1)]).await, 0, "sin servidor vivo no arranca ninguno");
        let (diag, _) = g.diagnostics_full(&r.join("b.ts"), Duration::from_secs(10)).await.unwrap();
        assert!(diag.iter().any(|x| x.message.contains("nueva")), "{diag:?}");
        // un `bash` crea a.ts con la función: b.ts deja de tener el error sin que nadie lo edite
        std::fs::write(r.join("a.ts"), "function nueva(x) { return x; }\n").unwrap();
        assert_eq!(g.por_fuera(&[(r.join("a.ts"), 1), (r.join("notas.md"), 2)]).await, 1, "sólo lo de su lenguaje");
        let e = hasta(&g, "/b.ts", |e| e.is_empty()).await;
        assert!(e.is_empty(), "{e:?}");
        // b.ts (abierto) reescrito en disco: el servidor ve el texto NUEVO
        std::fs::write(r.join("b.ts"), "ERROR puesto por un formateador\n").unwrap();
        assert_eq!(g.por_fuera(&[(r.join("b.ts"), 2)]).await, 1);
        let e = hasta(&g, "/b.ts", |e| !e.is_empty()).await;
        assert!(e.iter().any(|m| m.contains("formateador")), "{e:?}");
        // borrado: se cierra (y fuera de la raíz no se avisa nada)
        std::fs::remove_file(r.join("b.ts")).unwrap();
        assert_eq!(g.por_fuera(&[(r.join("b.ts"), 3), (PathBuf::from("/tmp/otro.ts"), 2)]).await, 1);
    }

    /// ML2: una edición que rompe OTRO archivo lo dice («Nuevos» en cualquier archivo), sin contar los que ya
    /// estaban; arreglar uno lo cuenta como resuelto; sin base (servidor recién arrancado), no se inventa.
    #[tokio::test]
    async fn errores_nuevos_en_cualquier_archivo() {
        std::env::set_var("GOTV_LSP_FALSO", "1");
        let d = crate::CarpetaPrueba::nueva("gotv-lsp-ml2");
        let r = d.canonicalize().unwrap();
        std::fs::write(r.join("package.json"), "{}").unwrap();
        std::fs::write(r.join("a.ts"), "ERROR viejo\n").unwrap();
        std::fs::write(r.join("b.ts"), "ok\n").unwrap();
        let mut g = Gestor::con(&r, vec![def_falso("typescript", &["ts"], &["package.json"], false)]);
        let vacia = g.foto().await;
        let _ = g.diagnostics_full(&r.join("a.ts"), Duration::from_secs(10)).await.unwrap();
        assert!(g.comparar(&vacia, &r.join("a.ts")).await.is_none(), "sin base no hay nuevos");
        let foto = g.foto().await;
        // editar a.ts: sigue el error viejo y ROMPE b.ts
        std::fs::write(r.join("a.ts"), format!("ERROR viejo\nROMPE:{}\n", r.join("b.ts").display())).unwrap();
        let _ = g.diagnostics_full(&r.join("a.ts"), Duration::from_secs(10)).await.unwrap();
        let inf = g.comparar(&foto, &r.join("a.ts")).await.unwrap();
        assert_eq!((inf.nuevos.len(), inf.previos, inf.resueltos), (1, 1, 0), "{inf:?}");
        assert_eq!(inf.nuevos[0].0, r.join("b.ts"));
        assert!(inf.texto(&r).contains("Nuevos: 1 — b.ts:1"), "{}", inf.texto(&r));
        // arreglar el viejo: resuelto
        let foto = g.foto().await;
        std::fs::write(r.join("a.ts"), "bien\n").unwrap();
        let _ = g.diagnostics_full(&r.join("a.ts"), Duration::from_secs(10)).await.unwrap();
        let inf = g.comparar(&foto, &r.join("a.ts")).await.unwrap();
        assert_eq!((inf.nuevos.len(), inf.resueltos), (0, 1), "{inf:?}");
        assert!(inf.texto(&r).contains("sin errores nuevos"));
    }

    /// ML10: agrupado por archivo (la ruta una vez), 15 errores como máximo y la lista ENTERA a
    /// `.gotv/salidas/lsp/` con su ruta en el informe.
    #[test]
    fn el_informe_agrupa_por_archivo_y_desborda_a_disco() {
        let d = crate::CarpetaPrueba::nueva("gotv-lsp-informe");
        let r = d.canonicalize().unwrap();
        let pocos = Informe { nuevos: vec![(r.join("a.rs"), 2, "uno".into()), (r.join("b.rs"), 4, "dos".into()), (r.join("a.rs"), 9, "tres".into())], previos: 1, resueltos: 0 };
        let t = pocos.texto(&r);
        assert!(t.contains("Nuevos: 3 — a.rs:2 uno; :9 tres · b.rs:4 dos"), "{t}");
        let muchos = Informe { nuevos: (1..=40).map(|i| (r.join(if i % 2 == 0 { "par.rs" } else { "impar.rs" }), i, format!("e{i}"))).collect(), previos: 0, resueltos: 0 };
        let t = muchos.texto(&r);
        assert_eq!(regex::Regex::new(r"\be\d+\b").unwrap().find_iter(&t).count(), 15, "15 como máximo: {t}");
        assert!(t.contains("y 25 más") || t.contains("and 25 more"), "{t}");
        let ruta = t.split(".gotv/salidas/lsp/").nth(1).and_then(|s| s.split(')').next()).map(|f| r.join(".gotv/salidas/lsp").join(f)).unwrap();
        assert_eq!(std::fs::read_to_string(ruta).unwrap().lines().count(), 40, "la lista entera en disco");
    }

    /// F5 «hecho cuando»: Python con `.venv` → 0 errores falsos de import. Con el servidor REAL
    /// (basedpyright o pyright) y un `.venv` de verdad (`uv venv`), sin red: el paquete se pone a mano en
    /// su `site-packages`, como lo deja un `pip install`. El control (mismo código, sin `.venv`) DEBE dar
    /// el import por no resuelto: si no, la prueba no distinguiría nada.
    /// `cargo test --release -p gotv-core -- --ignored --exact lsp::gestor::tests::python_con_venv_sin_imports_falsos`
    #[tokio::test]
    #[ignore]
    async fn python_con_venv_sin_imports_falsos() {
        let codigo = "import json\nimport medidas_gotv\n\nprint(json.dumps(medidas_gotv.VALOR))\n";
        // de CUALQUIER gravedad: un import falso dado como aviso también cuenta
        let de_import = |ds: &[Diag]| ds.iter().filter(|d| d.message.contains("medidas_gotv")).count();
        // con .venv
        let d = crate::CarpetaPrueba::nueva("gotv-lsp-venv");
        let r = d.canonicalize().unwrap();
        let ok = std::process::Command::new("uv").args(["venv", "-q", ".venv"]).current_dir(&r).status().unwrap();
        assert!(ok.success(), "uv venv");
        let sp = std::fs::read_dir(r.join(".venv/lib")).unwrap().flatten().map(|e| e.path().join("site-packages")).find(|p| p.is_dir()).expect("site-packages");
        std::fs::create_dir_all(sp.join("medidas_gotv")).unwrap();
        std::fs::write(sp.join("medidas_gotv/__init__.py"), "VALOR: int = 7\n").unwrap();
        // un paquete con tipos (PEP 561); sin la marca, basedpyright avisa (de verdad) «Stub file not found»
        std::fs::write(sp.join("medidas_gotv/py.typed"), "").unwrap();
        std::fs::write(r.join("pyproject.toml"), "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n").unwrap();
        std::fs::write(r.join("main.py"), codigo).unwrap();
        let mut g = Gestor::new(&r);
        let t = std::time::Instant::now();
        let (ds, completo) = g.diagnostics_full(&r.join("main.py"), Duration::from_secs(60)).await.unwrap();
        eprintln!("con .venv: {} errores de import · {ds:?} · completo={completo} · {:?} · {:?}", de_import(&ds), t.elapsed(), g.activos());
        assert!(completo, "el servidor terminó su análisis");
        assert_eq!(de_import(&ds), 0, "import falso con .venv: {ds:?}");
        // control: sin .venv el mismo import NO se resuelve
        let c = crate::CarpetaPrueba::nueva("gotv-lsp-sin-venv");
        let rc = c.canonicalize().unwrap();
        std::fs::write(rc.join("pyproject.toml"), "[project]\nname = \"demo\"\nversion = \"0.1.0\"\n").unwrap();
        std::fs::write(rc.join("main.py"), codigo).unwrap();
        let mut gc = Gestor::new(&rc);
        let (dc, _) = gc.diagnostics_full(&rc.join("main.py"), Duration::from_secs(60)).await.unwrap();
        eprintln!("sin .venv (control): {} errores de import · {dc:?}", de_import(&dc));
        assert_eq!(de_import(&dc), 1, "el control debe fallar el import: {dc:?}");
    }
}

// ── F5 · ML2 · errores encadenados con antes/después ────────────────────────────────────────────

/// Los ERRORES de cada servidor vivo, por archivo, en un momento: (archivo → [(línea, mensaje)]).
#[derive(Default, Clone, Debug)]
pub struct Foto {
    por_servidor: HashMap<Clave, HashMap<String, Vec<(u64, String)>>>,
}

/// Lo que cambió entre la foto y ahora, en TODOS los archivos de los servidores que ya estaban.
#[derive(Default, Debug, PartialEq)]
pub struct Informe {
    pub nuevos: Vec<(PathBuf, u64, String)>,
    pub previos: usize,
    pub resueltos: usize,
}

fn solo_errores(d: &[serde_json::Value]) -> Vec<(u64, String)> {
    d.iter()
        .filter(|x| x["severity"].as_u64().unwrap_or(1) == 1)
        .map(|x| (x["range"]["start"]["line"].as_u64().unwrap_or(0) + 1, x["message"].as_str().unwrap_or("").lines().next().unwrap_or("").to_string()))
        .collect()
}

impl Gestor {
    pub async fn foto(&self) -> Foto {
        let mut f = Foto::default();
        for (k, e) in &self.vivos {
            let m: HashMap<String, Vec<(u64, String)>> = e.lsp.diags_todos().await.into_iter().map(|(u, d)| (u, solo_errores(&d))).collect();
            f.por_servidor.insert(k.clone(), m);
        }
        f
    }

    /// Compara con la foto. `None` si el servidor del archivo editado no corría al tomarla: sin base, no
    /// hay «nuevos» que decir (serían todos los de siempre).
    pub async fn comparar(&self, antes: &Foto, editado: &Path) -> Option<Informe> {
        let d = registro::para(&self.defs, editado)?;
        let clave: Clave = (d.id, super::raiz::raiz_para(editado, d, &self.root));
        antes.por_servidor.get(&clave)?;
        let mut inf = Informe::default();
        for (k, e) in &self.vivos {
            let Some(previo) = antes.por_servidor.get(k) else { continue };
            let ahora: HashMap<String, Vec<(u64, String)>> = e.lsp.diags_todos().await.into_iter().map(|(u, d)| (u, solo_errores(&d))).collect();
            let uris: std::collections::BTreeSet<&String> = previo.keys().chain(ahora.keys()).collect();
            for u in uris {
                let a = previo.get(u).cloned().unwrap_or_default();
                let b = ahora.get(u).cloned().unwrap_or_default();
                // identidad = el mensaje (las líneas se corren al editar arriba); por multiconjunto
                let mut restantes: Vec<&String> = a.iter().map(|x| &x.1).collect();
                for (linea, msg) in &b {
                    if let Some(i) = restantes.iter().position(|m| *m == msg) {
                        restantes.remove(i);
                        inf.previos += 1;
                    } else {
                        inf.nuevos.push((e.lsp.ruta(u), *linea, msg.clone()));
                    }
                }
                inf.resueltos += restantes.len();
            }
        }
        inf.nuevos.sort();
        Some(inf)
    }

    /// Antes de editar las líneas `desde..=hasta` de `p`: si su servidor YA corre y sólo mira lo abierto,
    /// abre los archivos que usan los símbolos de esa zona y espera a que los revise (como mucho 8 s). Así
    /// la foto que viene después tiene base para ellos. Devuelve cuántos abrió.
    pub async fn preparar(&mut self, p: &Path, desde: u64, hasta: u64) -> usize {
        let Some(d) = registro::para(&self.defs, p).filter(|d| d.solo_abiertos) else { return 0 };
        let clave: Clave = (d.id, super::raiz::raiz_para(p, d, &self.root));
        let Some(e) = self.vivos.get_mut(&clave).filter(|e| e.lsp.vivo()) else { return 0 };
        let abiertos = e.lsp.abrir_vecinos(p, desde, hasta).await;
        if !abiertos.is_empty() {
            e.lsp.esperar_vecinos(&abiertos, Duration::from_secs(8)).await;
        }
        abiertos.len()
    }

    /// F5 ML6 · cambios POR FUERA (lo que tocó un `bash`, `git`, un generador): se avisan a TODOS los
    /// servidores que ya corren (cada uno filtra sus lenguajes y su raíz); no arranca ninguno. Devuelve
    /// cuántos avisos llegaron (un mismo archivo cuenta una vez por servidor que lo atiende).
    pub async fn por_fuera(&mut self, cambios: &[(PathBuf, u8)]) -> usize {
        if cambios.is_empty() {
            return 0;
        }
        let mut n = 0;
        for e in self.vivos.values_mut().filter(|e| e.lsp.vivo()) {
            n += e.lsp.por_fuera(cambios).await.unwrap_or(0);
        }
        n
    }

    /// F15.1: los archivos que usan lo que hay en `p` (todo el archivo, 10 símbolos), si su servidor ya corre
    /// (no arranca uno para esto).
    pub async fn quienes_usan(&mut self, p: &Path) -> Vec<PathBuf> {
        let Some(d) = registro::para(&self.defs, p) else { return vec![] };
        let clave: Clave = (d.id, super::raiz::raiz_para(p, d, &self.root));
        match self.vivos.get_mut(&clave).filter(|e| e.lsp.vivo()) {
            Some(e) => e.lsp.quienes_usan(p, 1, u32::MAX as u64, 10).await,
            None => vec![],
        }
    }

    /// `arregla: ["archivo:línea"]` de una edición: con la foto de ANTES, ¿desapareció el error de esa
    /// línea? (identidad = el mensaje, como en `comparar`: la línea pudo correrse). `None` sin pedidos.
    pub async fn arreglos(&self, antes: &Foto, pedidos: &[String]) -> Option<String> {
        if pedidos.is_empty() {
            return None;
        }
        let raiz = self.root.canonicalize().unwrap_or_else(|_| self.root.clone());
        let mut ahora: HashMap<String, Vec<String>> = HashMap::new();
        for e in self.vivos.values() {
            for (u, d) in e.lsp.diags_todos().await {
                ahora.entry(u).or_default().extend(solo_errores(&d).into_iter().map(|x| x.1));
            }
        }
        let mut partes = Vec::new();
        for pedido in pedidos.iter().take(20) {
            let Some((archivo, linea)) = pedido.rsplit_once(':').and_then(|(a, l)| Some((a, l.trim().parse::<u64>().ok()?))) else {
                partes.push(crate::tr!("? «{pedido}» (se escribe archivo:línea)", "? «{pedido}» (write it as file:line)"));
                continue;
            };
            let ruta = crate::paths::resolve(&raiz, archivo);
            let ruta = ruta.canonicalize().unwrap_or(ruta);
            let u = format!("file://{}", ruta.display());
            let habia: Vec<&String> = antes.por_servidor.values().filter_map(|m| m.get(&u)).flatten().filter(|(l, _)| *l == linea).map(|(_, m)| m).collect();
            if habia.is_empty() {
                partes.push(crate::tr!("· {archivo}:{linea} no tenía error antes del cambio", "· {archivo}:{linea} had no error before the change"));
                continue;
            }
            let mut quedan = ahora.get(&u).cloned().unwrap_or_default();
            let mut siguen = Vec::new();
            for m in habia {
                if let Some(i) = quedan.iter().position(|x| x == m) {
                    quedan.remove(i);
                    siguen.push(m.clone());
                }
            }
            partes.push(if siguen.is_empty() {
                crate::tr!("✓ {archivo}:{linea} resuelto", "✓ {archivo}:{linea} resolved")
            } else {
                crate::tr!("✗ {archivo}:{linea} sigue: {}", "✗ {archivo}:{linea} still there: {}", siguen.join(" · "))
            });
        }
        Some(crate::tr!("[LSP] arregla: {}", "[LSP] fixes: {}", partes.join(" | ")))
    }
}

impl Informe {
    /// `Nuevos: 2 — api/usuarios.rs:14 … · web/src/App.tsx:33 … | Ya existían: 5 | Resueltos: 1` (15 como
    /// mucho a la vista).
    /// ML10 · lo que ve el modelo: agrupado por archivo (la ruta una vez: `a.rs:2 msg; :9 msg · b.rs:4 msg`),
    /// 15 errores como máximo; si hay más, la lista ENTERA va a `.gotv/salidas/lsp/` (F6) y se dice dónde.
    pub fn texto(&self, root: &Path) -> String {
        const MAX: usize = 15;
        let raiz = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let rel = |p: &PathBuf| p.strip_prefix(&raiz).unwrap_or(p).display().to_string();
        // por archivo, en el orden en que aparece cada uno
        let mut grupos: Vec<(String, Vec<(u64, &str)>)> = Vec::new();
        for (p, l, m) in &self.nuevos {
            let r = rel(p);
            match grupos.iter_mut().find(|(g, _)| *g == r) {
                Some((_, v)) => v.push((*l, m.as_str())),
                None => grupos.push((r, vec![(*l, m.as_str())])),
            }
        }
        let mut quedan = MAX;
        let mut lista: Vec<String> = Vec::new();
        for (r, v) in &grupos {
            if quedan == 0 {
                break;
            }
            let partes: Vec<String> = v.iter().take(quedan).enumerate().map(|(i, (l, m))| if i == 0 { format!("{r}:{l} {m}") } else { format!(":{l} {m}") }).collect();
            quedan -= partes.len();
            lista.push(partes.join("; "));
        }
        let mas = self.nuevos.len().saturating_sub(MAX);
        let resto = if mas > 0 {
            let todo: String = self.nuevos.iter().map(|(p, l, m)| format!("{}:{l} {m}\n", rel(p))).collect();
            let dir = crate::salidas::carpeta(root).join("lsp");
            let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
            let ruta = dir.join(format!("errores-{n}.txt"));
            match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&ruta, todo)) {
                Ok(_) => crate::tr!(" (y {mas} más: la lista entera en {})", " (and {mas} more: the whole list in {})", rel(&ruta)),
                Err(_) => crate::tr!(" (y {mas} más)", " (and {mas} more)"),
            }
        } else {
            String::new()
        };
        if self.nuevos.is_empty() {
            crate::tr!("[LSP] sin errores nuevos · ya existían: {} · resueltos: {}", "[LSP] no new errors · already there: {} · resolved: {}", self.previos, self.resueltos)
        } else {
            crate::tr!("[LSP] Nuevos: {} — {}{resto} | Ya existían: {} | Resueltos: {}", "[LSP] New: {} — {}{resto} | Already there: {} | Resolved: {}", self.nuevos.len(), lista.join(" · "), self.previos, self.resueltos)
        }
    }
}
