//! F8.4 · Varias carpetas por proyecto: la principal (la del proyecto) y carpetas EXTRA con su nombre
//! (`app/` aquí, `api/` en otra parte del disco). Una ruta relativa va a la principal; `@nombre/…`, a la
//! extra de ese nombre.
//!
//! **Dónde se guardan:** FUERA del proyecto, en `~/Library/Application Support/gotv/carpetas/<huella de la
//! raíz>` (códec propio). Nunca en `.gotv/` del proyecto: un repositorio clonado podría traer una lista que
//! le diera escritura a otras carpetas del disco; sólo el Portador las añade, desde la ventana.
//!
//! **Qué cambian:** la caja deja escribir en ellas (zonas `W{i}`, con su `.gotv` y `.git/hooks` protegidos
//! como los del proyecto); los permisos no las tratan como «fuera del proyecto»; y los puntos de
//! restauración las fotografían junto a la principal (sus rutas van como `@nombre/…`), así que un turno que
//! toca las dos se deshace ENTERO o no se deshace. Quitar una carpeta no borra nada de ella.

use crate::programar::codec::{self, Escritor, Lector};
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

const TIPO: &[u8; 4] = b"RAIZ";
const VERSION: u8 = 1;

/// Las carpetas de un proyecto.
#[derive(Debug, Clone, PartialEq)]
pub struct Raices {
    pub principal: PathBuf,
    /// (nombre, carpeta) de cada extra
    pub extra: Vec<(String, PathBuf)>,
}

fn canon(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// Dónde se guarda la lista de un proyecto (`GOTV_CARPETAS` la cambia, para pruebas).
fn archivo(principal: &Path) -> PathBuf {
    use sha2::{Digest, Sha256};
    let base = std::env::var_os("GOTV_CARPETAS")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Library/Application Support/gotv/carpetas"));
    let h = Sha256::digest(canon(principal).to_string_lossy().as_bytes());
    base.join(h.iter().take(12).map(|b| format!("{b:02x}")).collect::<String>())
}

/// Un nombre válido para una extra: letras, cifras, `-`, `_` y `.` (lo que va tras la @).
pub fn nombre_valido(n: &str) -> bool {
    !n.is_empty() && n.len() <= 40 && n.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.')) && n != "." && n != ".."
}

/// Lo leído por última vez de cada proyecto (con la fecha del archivo): `paths::resolve` y la caja lo
/// consultan en cada llamada y no deben ir al disco cada vez.
fn memoria() -> &'static Mutex<HashMap<PathBuf, (Option<SystemTime>, Vec<(String, PathBuf)>)>> {
    static M: OnceLock<Mutex<HashMap<PathBuf, (Option<SystemTime>, Vec<(String, PathBuf)>)>>> = OnceLock::new();
    M.get_or_init(Default::default)
}

/// Las extras de un proyecto (vacío si no tiene). Sin disco si el archivo no cambió.
pub fn extras_de(principal: &Path) -> Vec<(String, PathBuf)> {
    let a = archivo(principal);
    let sello = std::fs::metadata(&a).ok().and_then(|m| m.modified().ok());
    let clave = canon(principal);
    if let Some((s, v)) = memoria().lock().unwrap_or_else(|e| e.into_inner()).get(&clave) {
        if *s == sello {
            return v.clone();
        }
    }
    let v = sello.and_then(|_| std::fs::read(&a).ok()).and_then(|b| leer(&b).ok()).unwrap_or_default();
    memoria().lock().unwrap_or_else(|e| e.into_inner()).insert(clave, (sello, v.clone()));
    v
}

fn leer(b: &[u8]) -> Result<Vec<(String, PathBuf)>> {
    let (mut l, _) = Lector::nuevo(b, TIPO, VERSION)?;
    let n = l.u32()? as usize;
    (0..n).map(|_| Ok((l.texto()?, PathBuf::from(l.texto()?)))).collect()
}

/// ¿`p` cae dentro de la principal o de alguna extra? (sustituye a `paths::is_inside` donde la pregunta es
/// «¿es del proyecto?»). Sigue symlinks como `is_inside`.
pub fn dentro(principal: &Path, p: &Path) -> bool {
    crate::paths::is_inside(principal, p) || extras_de(principal).iter().any(|(_, r)| crate::paths::is_inside(r, p))
}

/// `@nombre/resto` → la ruta en esa extra (normalizada; si el resto escapa con `..`, la ruta queda FUERA y
/// los permisos la tratan como tal). `None` si no es una ficha `@` o no hay extra con ese nombre.
pub fn resolver_extra(principal: &Path, pedida: &str) -> Option<PathBuf> {
    let r = pedida.trim().strip_prefix('@')?;
    let (nombre, resto) = r.split_once('/').unwrap_or((r, ""));
    let (_, base) = extras_de(principal).into_iter().find(|(n, _)| n == nombre)?;
    Some(crate::paths::normalize(&base.join(resto)))
}

/// Lo que el modelo sabe de las carpetas extra (en el prompt de sistema; vacío sin extras).
pub fn bloque(principal: &Path) -> String {
    let ex = extras_de(principal);
    if ex.is_empty() {
        return String::new();
    }
    let lista: Vec<String> = ex.iter().map(|(n, r)| format!("- @{n}/ → {}", r.display())).collect();
    crate::tr!(
        "\n\n# Carpetas del proyecto\nAdemás de la principal, este proyecto trabaja en estas carpetas (puedes leer y editar en ellas como en el proyecto). Para nombrar un archivo suyo usa `@nombre/ruta` (p. ej. `@{}/src/main.rs`):\n{}\n",
        "\n\n# Project folders\nBesides the main one, this project works in these folders (you can read and edit in them as in the project). To name one of their files use `@name/path` (e.g. `@{}/src/main.rs`):\n{}\n",
        ex[0].0,
        lista.join("\n")
    )
}

impl Raices {
    /// Las carpetas de un proyecto, como están guardadas.
    pub fn de_proyecto(principal: &Path) -> Raices {
        Raices { principal: principal.to_path_buf(), extra: extras_de(principal) }
    }

    /// Una ruta relativa va a la principal; `@nombre/…` a la carpeta extra de ese nombre.
    pub fn resolver(&self, pedida: &str) -> PathBuf {
        if let Some(r) = pedida.trim().strip_prefix('@') {
            let (nombre, resto) = r.split_once('/').unwrap_or((r, ""));
            if let Some((_, base)) = self.extra.iter().find(|(n, _)| n == nombre) {
                return crate::paths::normalize(&base.join(resto));
            }
        }
        crate::paths::resolve(&self.principal, pedida)
    }

    pub fn contiene(&self, p: &Path) -> bool {
        self.de(p).is_some()
    }

    /// A qué carpeta pertenece `p` (la extra más específica gana; la principal si ninguna).
    pub fn de(&self, p: &Path) -> Option<&Path> {
        if let Some((_, r)) = self.extra.iter().filter(|(_, r)| crate::paths::is_inside(r, p)).max_by_key(|(_, r)| r.as_os_str().len()) {
            return Some(r);
        }
        crate::paths::is_inside(&self.principal, p).then_some(self.principal.as_path())
    }

    /// Añade una carpeta extra. El nombre lo fija el Portador; sin él, el de la carpeta (si choca con otro,
    /// hay que darlo). No vale: una que no existe, la raíz del disco o la casa entera, una carpeta de
    /// credenciales o que las contenga, una DENTRO de la principal (no cuenta dos veces) ni una que contenga
    /// a la principal. Devuelve el nombre.
    pub fn agregar(&mut self, ruta: &Path, nombre: Option<&str>) -> Result<String> {
        let r = ruta.canonicalize().with_context(|| crate::tr!("{} no existe", "{} does not exist", ruta.display()))?;
        if !r.is_dir() {
            bail!("{}", crate::tr!("{} no es una carpeta", "{} is not a folder", r.display()));
        }
        let casa = canon(&crate::paths::home());
        if r == Path::new("/") || r == casa || !r.starts_with("/") {
            bail!("{}", crate::tr!("{} es demasiado amplia: elige una carpeta de trabajo concreta", "{} is too broad: choose a specific work folder", r.display()));
        }
        if crate::sandbox::es_credencial(&r).is_some() || crate::sandbox::CREDENCIALES.iter().any(|c| casa.join(c).starts_with(&r)) {
            bail!("{}", crate::tr!("{} tiene credenciales (llaves, contraseñas): no se añade", "{} holds credentials (keys, passwords): it is not added", r.display()));
        }
        let p = canon(&self.principal);
        if r.starts_with(&p) {
            bail!("{}", crate::tr!("{} ya está dentro del proyecto", "{} is already inside the project", r.display()));
        }
        if p.starts_with(&r) {
            bail!("{}", crate::tr!("{} contiene al proyecto: añade una carpeta de al lado, no la de encima", "{} contains the project: add a sibling folder, not the one above", r.display()));
        }
        if self.extra.iter().any(|(_, x)| canon(x) == r) {
            bail!("{}", crate::tr!("{} ya es una carpeta del proyecto", "{} is already a folder of the project", r.display()));
        }
        let n = match nombre.map(str::trim).filter(|n| !n.is_empty()) {
            Some(n) => n.to_string(),
            None => r.file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default(),
        };
        if !nombre_valido(&n) {
            bail!("{}", crate::tr!("«{n}» no vale como nombre (letras, cifras, - _ .)", "«{n}» is not a valid name (letters, digits, - _ .)"));
        }
        if self.extra.iter().any(|(x, _)| *x == n) {
            bail!("{}", crate::tr!("ya hay una carpeta «{n}»: dale otro nombre", "there is already a folder «{n}»: give it another name"));
        }
        self.extra.push((n.clone(), r));
        Ok(n)
    }

    /// Quita una extra de la lista. No borra NADA de la carpeta.
    pub fn quitar(&mut self, nombre: &str) -> Result<()> {
        let antes = self.extra.len();
        self.extra.retain(|(n, _)| n != nombre);
        if self.extra.len() == antes {
            bail!("{}", crate::tr!("no hay una carpeta «{nombre}»", "there is no folder «{nombre}»"));
        }
        Ok(())
    }

    /// Guarda la lista (fuera del proyecto).
    pub fn guardar(&self) -> Result<()> {
        let a = archivo(&self.principal);
        if let Some(d) = a.parent() {
            std::fs::create_dir_all(d)?;
        }
        let mut e = Escritor::nuevo(TIPO, VERSION);
        e.u32(self.extra.len() as u32);
        for (n, r) in &self.extra {
            e.texto(n);
            e.texto(&r.to_string_lossy());
        }
        codec::guardar(&a, &e.bytes())?;
        memoria().lock().unwrap_or_else(|e| e.into_inner()).remove(&canon(&self.principal));
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Las pruebas que tocan la lista guardada usan su propia carpeta (una sola vez por proceso).
    pub(crate) fn aislar() {
        static UNA: OnceLock<crate::CarpetaPrueba> = OnceLock::new();
        UNA.get_or_init(|| {
            let d = crate::CarpetaPrueba::sin_crear("gotv-carpetas-guardadas");
            // SAFETY: se pone UNA vez por proceso y siempre con el mismo valor
            unsafe { std::env::set_var("GOTV_CARPETAS", &*d) };
            d
        });
    }

    /// Un proyecto `app` con su carpeta hermana `api` (fuera del proyecto).
    pub(crate) fn dos(tag: &str) -> (crate::CarpetaPrueba, PathBuf, PathBuf) {
        aislar();
        let d = crate::CarpetaPrueba::sin_crear(&format!("gotv-raices-{tag}"));
        let (app, api) = (d.join("app"), d.join("api"));
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::create_dir_all(api.join("src")).unwrap();
        let (app, api) = (app.canonicalize().unwrap(), api.canonicalize().unwrap());
        (d, app, api)
    }

    /// F8.4 paso 5: índice, servidor de lenguaje y mapa también para la carpeta EXTRA (rutas `@api/…`).
    #[test]
    fn indice_lsp_y_mapa_por_carpeta_extra() {
        let (_d, app, api) = dos("paso5");
        std::fs::write(app.join("package.json"), "{\"name\":\"app\"}").unwrap();
        std::fs::write(app.join("src/index.js"), "export function pintar() {}\n").unwrap();
        std::fs::write(api.join("Cargo.toml"), "[package]\nname = \"api\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
        std::fs::write(api.join("src/lib.rs"), "pub fn api_suma_pedidos(a: i32, b: i32) -> i32 {\n    a + b\n}\n").unwrap();
        // el mapa ordena por lo que USA el resto del código: que alguien la llame
        std::fs::write(api.join("src/main.rs"), "fn main() {\n    println!(\"{}\", api::api_suma_pedidos(1, 2) + api::api_suma_pedidos(3, 4));\n}\n").unwrap();
        let mut r = Raices::de_proyecto(&app);
        r.agregar(&api, Some("api")).unwrap();
        r.guardar().unwrap();
        // el índice: lo de la extra, con su ruta `@api/…`
        let rag = crate::rag::Rag::new(&app);
        rag.build(&app).unwrap();
        let h = rag.search("api_suma_pedidos", 5).unwrap();
        assert!(h.iter().any(|x| x.path == "@api/src/lib.rs"), "{:?}", h.iter().map(|x| &x.path).collect::<Vec<_>>());
        // el servidor de lenguaje de un archivo de la extra arranca EN la extra (su Cargo.toml)
        let rust = crate::lsp::registro::defs().into_iter().find(|d| d.id == "rust").unwrap();
        assert_eq!(crate::lsp::raiz::raiz_para(&api.join("src/lib.rs"), &rust, &app), api);
        assert!(crate::lsp::gestor::Gestor::new(&app).lenguajes_del_proyecto().iter().any(|d| d.id == "rust"), "el Cargo.toml de la extra cuenta");
        // el mapa de `@api`
        let m = crate::mapa::herramienta(&app, &serde_json::json!({"carpeta": "@api"}));
        assert!(m.contains("api_suma_pedidos") && m.contains("@api/"), "{m}");
        assert!(crate::mapa::herramienta(&app, &serde_json::json!({"carpeta": "@nada"})).contains("@api"), "dice las que hay");
        r.quitar("api").unwrap();
        r.guardar().unwrap();
    }

    #[test]
    fn con_una_sola_carpeta_todo_es_como_antes() {
        let (_d, app, _) = dos("sola");
        let r = Raices::de_proyecto(&app);
        assert!(r.extra.is_empty());
        assert_eq!(r.resolver("src/x.rs"), crate::paths::resolve(&app, "src/x.rs"));
        assert_eq!(r.resolver("@api/x"), crate::paths::resolve(&app, "@api/x"), "sin extras, una @ es un nombre más");
        assert!(r.contiene(&app.join("src/x.rs")) && !r.contiene(Path::new("/etc/hosts")));
    }

    /// `resolver("@api/src/x.rs")` cae en la carpeta `api`; una que escapa con `..` queda fuera; el nombre
    /// lo fija el Portador; se guarda fuera del proyecto y sobrevive.
    #[test]
    fn la_arroba_va_a_su_carpeta_y_el_escape_queda_fuera() {
        let (d, app, api) = dos("arroba");
        let mut r = Raices::de_proyecto(&app);
        assert_eq!(r.agregar(&api, None).unwrap(), "api");
        r.guardar().unwrap();
        assert!(!app.join(".gotv").exists(), "nada se guarda dentro del proyecto");
        let r = Raices::de_proyecto(&app);
        assert_eq!(r.resolver("@api/src/x.rs"), api.join("src/x.rs"));
        assert_eq!(r.de(&api.join("src/x.rs")), Some(api.as_path()));
        assert_eq!(r.de(&app.join("src/x.rs")), Some(app.as_path()));
        let fuera = r.resolver("@api/../../secreto.txt");
        assert!(!r.contiene(&fuera) && !dentro(&app, &fuera), "{} escapa: fuera", fuera.display());
        assert!(dentro(&app, &api.join("src/x.rs")));
        // y lo mismo por la vía que usan las herramientas
        assert_eq!(crate::paths::resolve(&app, "@api/src/x.rs"), api.join("src/x.rs"));
        assert_eq!(resolver_extra(&app, "@nada/x"), None);
        drop(d);
    }

    /// Un turno que toca las DOS carpetas se deshace entero o no se deshace; el panel de cambios lo enseña
    /// con `@api/…` y revierte un archivo de la extra.
    #[test]
    fn un_turno_en_las_dos_carpetas_se_deshace_entero_o_nada() {
        let (_d, app, api) = dos("deshacer");
        std::fs::write(app.join("src/a.rs"), "app antes\n").unwrap();
        std::fs::write(api.join("src/b.rs"), "api antes\n").unwrap();
        let mut r = Raices::de_proyecto(&app);
        r.agregar(&api, None).unwrap();
        r.guardar().unwrap();
        let cp = crate::checkpoint::Checkpoint::new(&app);
        cp.begin().unwrap();
        let desde = cp.ultimo_id();
        let p = cp.begin().unwrap();
        std::fs::write(app.join("src/a.rs"), "app después\n").unwrap();
        std::fs::write(api.join("src/b.rs"), "api después\n").unwrap();
        std::fs::write(api.join("src/nuevo.rs"), "nuevo\n").unwrap();
        let c = cp.commit(p, "bash · paso 1").unwrap().unwrap();
        let mut rutas: Vec<&str> = c.changes.iter().map(|x| x.path.as_str()).collect();
        rutas.sort();
        assert_eq!(rutas, ["@api/src/b.rs", "@api/src/nuevo.rs", "src/a.rs"]);
        // el panel (sin git) enseña las dos carpetas
        let n = crate::cambios::anotar_turno(&app, None, desde, cp.ultimo_id().unwrap(), "toca las dos").unwrap();
        assert_eq!(crate::cambios::por_turno(&cp, n).unwrap().len(), 3);
        // editado después en la extra: no se deshace NADA (tampoco lo de la principal)
        std::fs::write(api.join("src/b.rs"), "api a mano\n").unwrap();
        assert!(cp.rewind(false).is_err());
        assert_eq!(std::fs::read_to_string(app.join("src/a.rs")).unwrap(), "app después\n", "ni la principal");
        // devuelto lo de a mano, se deshace entero: vuelven las dos
        std::fs::write(api.join("src/b.rs"), "api después\n").unwrap();
        cp.rewind(false).unwrap();
        assert_eq!(std::fs::read_to_string(app.join("src/a.rs")).unwrap(), "app antes\n");
        assert_eq!(std::fs::read_to_string(api.join("src/b.rs")).unwrap(), "api antes\n");
        assert!(!api.join("src/nuevo.rs").exists());
        // un archivo de la extra se revierte solo (y se le encuentra por su ruta absoluta)
        let p = cp.begin_scoped(&[api.join("src/b.rs")]).unwrap();
        std::fs::write(api.join("src/b.rs"), "otra vez\n").unwrap();
        cp.commit(p, "edit · paso 1").unwrap().unwrap();
        let n2 = crate::cambios::anotar_turno(&app, None, cp.ultimo_id().and_then(|x| x.checked_sub(1)), cp.ultimo_id().unwrap(), "edita b").unwrap();
        crate::cambios::revertir_archivo(&cp, n2, &api.join("src/b.rs"), false).unwrap();
        assert_eq!(std::fs::read_to_string(api.join("src/b.rs")).unwrap(), "api antes\n");
        // quitada la extra, su paso no se deshace a medias
        let mut r = Raices::de_proyecto(&app);
        r.quitar("api").unwrap();
        r.guardar().unwrap();
        assert!(cp.rewind(false).is_err());
        assert!(api.join("src/b.rs").is_file(), "quitar no borra nada");
    }

    /// Permisos y caja: en la extra se escribe como en el proyecto; en una tercera carpeta, no.
    #[test]
    fn permisos_y_caja_dejan_escribir_en_la_extra_y_en_ninguna_mas() {
        use crate::permissions::{Decision, Mode, Permissions};
        let (d, app, api) = dos("permisos");
        let otra = d.join("otra");
        std::fs::create_dir_all(&otra).unwrap();
        let mut r = Raices::de_proyecto(&app);
        r.agregar(&api, None).unwrap();
        r.guardar().unwrap();
        let p = Permissions::new(Mode::AcceptEdits, &app, vec![], vec![], vec![]);
        let w = |x: &str| p.check("write_file", &serde_json::json!({"path": x}));
        assert!(matches!(w("src/a.rs"), Decision::Allow));
        assert!(matches!(w("@api/src/b.rs"), Decision::Allow), "la extra es del proyecto");
        assert!(matches!(w(&api.join("src/b.rs").to_string_lossy()), Decision::Allow), "también por su ruta absoluta");
        assert!(!matches!(w(&otra.join("x.rs").to_string_lossy()), Decision::Allow), "una tercera carpeta, no");
        assert!(!matches!(w("@api/../otra/x.rs"), Decision::Allow), "escapar de la extra con .. no vale");
        // la caja: la extra entra como zona de escritura y su .gotv, protegido
        let spec = crate::sandbox::Spec { root: app.clone(), hosts: vec![], all_net: false, sockets: vec![], credenciales: vec![], entorno: vec![] };
        let c = crate::sandbox::command(&spec, "true");
        let args: Vec<String> = c.as_std().get_args().map(|a| a.to_string_lossy().to_string()).collect();
        assert!(args.iter().any(|a| a.starts_with("-DW") && a.ends_with(&*api.to_string_lossy())), "{args:?}");
        assert!(args.iter().any(|a| a.starts_with("-DPROT") && a.ends_with(&*api.join(".gotv").to_string_lossy())));
        assert!(!args.iter().any(|a| a.ends_with(&*otra.to_string_lossy())));
    }

    #[test]
    fn lo_que_no_se_puede_anadir() {
        let (d, app, api) = dos("no");
        let mut r = Raices::de_proyecto(&app);
        assert!(r.agregar(&app.join("src"), None).is_err(), "dentro de la principal no cuenta dos veces");
        assert!(r.agregar(app.parent().unwrap(), None).is_err(), "la de encima contiene al proyecto");
        assert!(r.agregar(Path::new("/"), None).is_err());
        assert!(r.agregar(&crate::paths::home(), None).is_err(), "la casa entera, no");
        assert!(r.agregar(&d.join("no-existe"), None).is_err());
        assert!(r.agregar(&api, Some("mal nombre")).is_err());
        r.agregar(&api, None).unwrap();
        assert!(r.agregar(&api, Some("otra")).is_err(), "la misma carpeta dos veces, no");
        // dos carpetas con el mismo nombre final: la segunda necesita el suyo
        let otra = d.join("b/api");
        std::fs::create_dir_all(&otra).unwrap();
        assert!(r.agregar(&otra, None).is_err());
        assert_eq!(r.agregar(&otra, Some("api2")).unwrap(), "api2");
        // quitar no borra nada
        r.quitar("api").unwrap();
        assert!(api.join("src").is_dir());
        assert!(r.quitar("api").is_err());
    }
}
