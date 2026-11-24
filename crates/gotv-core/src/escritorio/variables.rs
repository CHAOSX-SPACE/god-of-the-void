//! F16.4 · Variables guardadas por proyecto (tokens de API, URLs con clave…).
//!
//! - **Los NOMBRES** (y si pasan a la caja) se guardan con el códec propio en `~/.config/gotv/variables.bin`:
//!   no son secretos. **Los VALORES** sólo en el Llavero de macOS (servicio `space.chaosx.gotv.variables`,
//!   cuenta `<raíz>␟<NOMBRE>`): jamás en disco, en un registro ni de vuelta a la ventana.
//! - Se **inyectan en el entorno del hijo** de `bash` (`exec::spawn`), nunca en la línea de órdenes: `ps`
//!   no las enseña en el comando (trampa 3 de la ficha).
//! - **Caja (F14.1):** por omisión una variable sólo llega a los comandos que corren FUERA de la caja (que ya
//!   pidieron permiso). Dentro de la caja sólo si el Portador lo marcó para esa variable (`caja`).
//! - Si un comando imprime el valor, la salida lo TAPA (`‹variable NOMBRE›`) antes de tocar el registro de
//!   `.gotv/procesos/`, la pantalla o lo que lee el modelo ([`Tapador`]), aunque llegue partido en dos
//!   lecturas.
use crate::programar::codec::{self, Escritor, Lector};
use anyhow::{anyhow, bail, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const TIPO: &[u8; 4] = b"VARS";
const SERVICIO: &str = "space.chaosx.gotv.variables";

/// Nombres que no se dejan pisar: rompen la caja, la shell o el cargador.
const PROHIBIDAS: &[&str] = &["PATH", "HOME", "USER", "LOGNAME", "SHELL", "TMPDIR", "PWD", "OLDPWD", "TERM", "IFS", "ENV", "BASH_ENV", "PROMPT_COMMAND", "SSH_AUTH_SOCK", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY", "http_proxy", "https_proxy", "all_proxy", "no_proxy"];

#[derive(Debug, Clone, PartialEq)]
pub struct Variable {
    pub nombre: String,
    /// ¿pasa también a los comandos dentro de la caja?
    pub caja: bool,
}

/// Dónde viven los valores. El Llavero en la app; en memoria en las pruebas.
pub trait Boveda: Send + Sync {
    fn leer(&self, cuenta: &str) -> Option<String>;
    fn guardar(&self, cuenta: &str, valor: &str) -> Result<()>;
    fn borrar(&self, cuenta: &str);
}

pub struct Llavero;

impl Boveda for Llavero {
    fn leer(&self, cuenta: &str) -> Option<String> {
        security_framework::passwords::get_generic_password(SERVICIO, cuenta).ok().and_then(|b| String::from_utf8(b).ok())
    }
    fn guardar(&self, cuenta: &str, valor: &str) -> Result<()> {
        security_framework::passwords::set_generic_password(SERVICIO, cuenta, valor.as_bytes()).map_err(|e| anyhow!("{}: {e}", crate::i18n::t("el Llavero no guardó la variable", "the Keychain did not save the variable")))
    }
    fn borrar(&self, cuenta: &str) {
        let _ = security_framework::passwords::delete_generic_password(SERVICIO, cuenta);
    }
}

#[derive(Default)]
pub struct EnMemoria(Mutex<HashMap<String, String>>);

impl Boveda for EnMemoria {
    fn leer(&self, cuenta: &str) -> Option<String> {
        self.0.lock().unwrap().get(cuenta).cloned()
    }
    fn guardar(&self, cuenta: &str, valor: &str) -> Result<()> {
        self.0.lock().unwrap().insert(cuenta.into(), valor.into());
        Ok(())
    }
    fn borrar(&self, cuenta: &str) {
        self.0.lock().unwrap().remove(cuenta);
    }
}

/// Los nombres de todos los proyectos (archivo) + los valores (bóveda), con una caché de valores en memoria.
pub struct Almacen {
    ruta: PathBuf,
    boveda: Box<dyn Boveda>,
    cache: Mutex<HashMap<String, String>>,
}

fn cuenta(root: &Path, nombre: &str) -> String {
    format!("{}\u{1f}{nombre}", root.display())
}

pub fn nombre_valido(n: &str) -> Result<()> {
    let ok = !n.is_empty() && n.len() <= 128 && n.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !ok {
        bail!("{}", crate::tr!("nombre de variable inválido: «{n}» (letras, números y _; no empieza por número)", "invalid variable name: «{n}» (letters, digits and _; not starting with a digit)"));
    }
    if PROHIBIDAS.contains(&n) || n.starts_with("DYLD_") || n.starts_with("LD_") || n.starts_with("GOTV_") {
        bail!("{}", crate::tr!("«{n}» no se puede guardar: la usa el sistema o GOTV", "«{n}» cannot be saved: the system or GOTV uses it"));
    }
    Ok(())
}

impl Almacen {
    pub fn nuevo(ruta: PathBuf, boveda: Box<dyn Boveda>) -> Self {
        Almacen { ruta, boveda, cache: Default::default() }
    }

    fn todas(&self) -> Vec<(String, Variable)> {
        let Ok(b) = std::fs::read(&self.ruta) else { return vec![] };
        let leer = || -> Result<Vec<(String, Variable)>> {
            let (mut l, _) = Lector::nuevo(&b, TIPO, 1)?;
            let n = l.u32()?;
            let mut v = Vec::new();
            for _ in 0..n {
                let root = l.texto()?;
                let nombre = l.texto()?;
                let caja = l.bool()?;
                v.push((root, Variable { nombre, caja }));
            }
            Ok(v)
        };
        leer().unwrap_or_default()
    }

    fn escribir(&self, v: &[(String, Variable)]) -> Result<()> {
        let mut e = Escritor::nuevo(TIPO, 1);
        e.u32(v.len() as u32);
        for (root, x) in v {
            e.texto(root);
            e.texto(&x.nombre);
            e.bool(x.caja);
        }
        if let Some(d) = self.ruta.parent() {
            std::fs::create_dir_all(d)?;
        }
        codec::guardar(&self.ruta, &e.bytes())
    }

    /// Las variables de un proyecto (sólo nombres y caja: lo que la ventana puede ver).
    pub fn lista(&self, root: &Path) -> Vec<Variable> {
        let r = root.display().to_string();
        self.todas().into_iter().filter(|(x, _)| *x == r).map(|(_, v)| v).collect()
    }

    /// Guarda (o cambia) una variable. El valor va a la bóveda; el nombre, al archivo.
    pub fn guardar(&self, root: &Path, nombre: &str, valor: &str, caja: bool) -> Result<()> {
        nombre_valido(nombre)?;
        if valor.is_empty() || valor.contains('\0') {
            bail!("{}", crate::i18n::t("el valor está vacío o no es texto", "the value is empty or not text"));
        }
        let _candado = codec::Candado::esperar(&self.ruta.with_extension("lock"))?;
        self.boveda.guardar(&cuenta(root, nombre), valor)?;
        let r = root.display().to_string();
        let mut v = self.todas();
        v.retain(|(x, y)| !(*x == r && y.nombre == nombre));
        v.push((r, Variable { nombre: nombre.into(), caja }));
        self.escribir(&v)?;
        self.cache.lock().unwrap().insert(cuenta(root, nombre), valor.into());
        Ok(())
    }

    /// Cambia sólo si pasa a la caja (sin tocar el valor).
    pub fn marcar_caja(&self, root: &Path, nombre: &str, caja: bool) -> Result<()> {
        let _candado = codec::Candado::esperar(&self.ruta.with_extension("lock"))?;
        let r = root.display().to_string();
        let mut v = self.todas();
        let x = v.iter_mut().find(|(x, y)| *x == r && y.nombre == nombre).ok_or_else(|| anyhow!("{}", crate::tr!("no hay una variable «{nombre}»", "there is no variable «{nombre}»")))?;
        x.1.caja = caja;
        self.escribir(&v)
    }

    pub fn quitar(&self, root: &Path, nombre: &str) -> Result<()> {
        let _candado = codec::Candado::esperar(&self.ruta.with_extension("lock"))?;
        let r = root.display().to_string();
        let mut v = self.todas();
        v.retain(|(x, y)| !(*x == r && y.nombre == nombre));
        self.escribir(&v)?;
        self.boveda.borrar(&cuenta(root, nombre));
        self.cache.lock().unwrap().remove(&cuenta(root, nombre));
        Ok(())
    }

    fn valor(&self, root: &str, nombre: &str) -> Option<String> {
        let c = format!("{root}\u{1f}{nombre}");
        if let Some(v) = self.cache.lock().unwrap().get(&c) {
            return Some(v.clone());
        }
        let v = self.boveda.leer(&c)?;
        self.cache.lock().unwrap().insert(c, v.clone());
        Some(v)
    }

    /// Lo que se inyecta a un hijo que corre en `cwd`: las del proyecto que contiene `cwd` (la raíz guardada
    /// más larga que lo contiene), y dentro de la caja sólo las marcadas.
    pub fn para_hijo(&self, cwd: &Path, en_caja: bool) -> Vec<(String, String)> {
        let todas = self.todas();
        let Some(root) = todas.iter().map(|(r, _)| r).filter(|r| cwd.starts_with(Path::new(r.as_str()))).max_by_key(|r| r.len()).cloned() else { return vec![] };
        todas
            .into_iter()
            .filter(|(r, v)| *r == root && (!en_caja || v.caja))
            .filter_map(|(r, v)| self.valor(&r, &v.nombre).map(|x| (v.nombre, x)))
            .collect()
    }
}

fn global() -> &'static Mutex<Option<std::sync::Arc<Almacen>>> {
    static G: OnceLock<Mutex<Option<std::sync::Arc<Almacen>>>> = OnceLock::new();
    G.get_or_init(Default::default)
}

/// El almacén de la app: `~/.config/gotv/variables.bin` + el Llavero.
pub fn almacen() -> std::sync::Arc<Almacen> {
    let mut g = global().lock().unwrap();
    g.get_or_insert_with(|| {
        let ruta = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/gotv/variables.bin");
        std::sync::Arc::new(Almacen::nuevo(ruta, Box::new(Llavero)))
    })
    .clone()
}

/// Para las pruebas: otro archivo y una bóveda en memoria (jamás el Llavero real del usuario).
#[cfg(test)]
pub fn usar_para_pruebas(a: std::sync::Arc<Almacen>) {
    *global().lock().unwrap() = Some(a);
}

/// Enganche de `exec::spawn`.
pub fn para_hijo(cwd: &Path, en_caja: bool) -> Vec<(String, String)> {
    // en las pruebas de otros módulos, sin almacén puesto, no se toca el Llavero ni ~/.config
    #[cfg(test)]
    if global().lock().unwrap().is_none() {
        return vec![];
    }
    almacen().para_hijo(cwd, en_caja)
}

/// Tapa en la salida de un proceso los valores que se le inyectaron, aunque uno llegue partido entre dos
/// lecturas: retiene sólo el final de una lectura que EMPIEZA un valor, hasta ver cómo sigue.
pub struct Tapador {
    valores: Vec<(Vec<u8>, Vec<u8>)>,
    retenido: [Vec<u8>; 2],
}

impl Tapador {
    /// `None` si no hay nada que tapar (valores de menos de 4 bytes no se tapan: taparían texto común).
    pub fn de(vars: &[(String, String)]) -> Option<Tapador> {
        let mut valores: Vec<(Vec<u8>, Vec<u8>)> = vars.iter().filter(|(_, v)| v.len() >= 4).map(|(n, v)| (v.as_bytes().to_vec(), format!("‹variable {n}›").into_bytes())).collect();
        // el más largo primero: un valor que contiene a otro se tapa entero
        valores.sort_by_key(|(v, _)| std::cmp::Reverse(v.len()));
        (!valores.is_empty()).then(|| Tapador { valores, retenido: [vec![], vec![]] })
    }

    fn reemplazar(&self, mut b: Vec<u8>) -> Vec<u8> {
        for (v, t) in &self.valores {
            let mut out = Vec::with_capacity(b.len());
            let mut i = 0;
            while i < b.len() {
                if b[i..].starts_with(v) {
                    out.extend_from_slice(t);
                    i += v.len();
                } else {
                    out.push(b[i]);
                    i += 1;
                }
            }
            b = out;
        }
        b
    }

    /// Lo que puede salir de esta lectura (`err` = stderr, que lleva su propio retenido).
    pub fn pasar(&mut self, datos: &[u8], err: bool) -> Vec<u8> {
        let k = err as usize;
        let mut b = std::mem::take(&mut self.retenido[k]);
        b.extend_from_slice(datos);
        let b = self.reemplazar(b);
        // ¿el final es el principio de algún valor? se retiene esa cola
        let mut cola = 0;
        for (v, _) in &self.valores {
            for n in (1..v.len()).rev() {
                if n <= b.len() && b.ends_with(&v[..n]) {
                    cola = cola.max(n);
                    break;
                }
            }
        }
        let corte = b.len() - cola;
        self.retenido[k] = b[corte..].to_vec();
        b[..corte].to_vec()
    }

    /// Al terminar el proceso: lo retenido sale (no era un valor entero).
    pub fn fin(&mut self) -> [Vec<u8>; 2] {
        [std::mem::take(&mut self.retenido[0]), std::mem::take(&mut self.retenido[1])]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn almacen_de_prueba(tag: &str) -> (Arc<Almacen>, crate::CarpetaPrueba) {
        let d = crate::CarpetaPrueba::nueva(&format!("gotv-vars-{tag}"));
        (Arc::new(Almacen::nuevo(d.join("variables.bin"), Box::new(EnMemoria::default()))), d)
    }

    #[test]
    fn guarda_nombres_en_el_archivo_y_valores_solo_en_la_boveda() {
        let (a, d) = almacen_de_prueba("guardar");
        let root = d.join("proyecto");
        a.guardar(&root, "API_TOKEN", "tok-secreto-1234", false).unwrap();
        a.guardar(&root, "DB_URL", "postgres://u:clave@h/db", true).unwrap();
        assert!(a.guardar(&root, "PATH", "/x", false).is_err());
        assert!(a.guardar(&root, "1MAL", "x", false).is_err());
        assert!(a.guardar(&root, "DYLD_INSERT_LIBRARIES", "x", false).is_err());
        let crudo = std::fs::read(d.join("variables.bin")).unwrap();
        let crudo = String::from_utf8_lossy(&crudo);
        assert!(crudo.contains("API_TOKEN") && !crudo.contains("tok-secreto-1234") && !crudo.contains("clave@h"), "el archivo sólo lleva nombres");
        assert_eq!(a.lista(&root), vec![Variable { nombre: "API_TOKEN".into(), caja: false }, Variable { nombre: "DB_URL".into(), caja: true }]);
        // fuera de la caja, las dos; dentro, sólo la marcada; en otro proyecto, ninguna
        let fuera = a.para_hijo(&root.join("src"), false);
        assert_eq!(fuera.len(), 2);
        assert_eq!(a.para_hijo(&root, true), vec![("DB_URL".to_string(), "postgres://u:clave@h/db".to_string())]);
        assert!(a.para_hijo(&d.join("otro"), false).is_empty());
        a.marcar_caja(&root, "API_TOKEN", true).unwrap();
        assert_eq!(a.para_hijo(&root, true).len(), 2);
        a.quitar(&root, "API_TOKEN").unwrap();
        assert_eq!(a.lista(&root).len(), 1);
        assert!(a.boveda.leer(&cuenta(&root, "API_TOKEN")).is_none(), "quitar también la borra de la bóveda");
    }

    /// Ficha F16.4: una variable guardada llega al hijo (por su entorno) y no aparece en ningún registro: ni en
    /// la salida que lee el modelo, ni en `.gotv/procesos/N.log`, ni en la línea de órdenes que enseña `ps`.
    #[tokio::test]
    async fn una_variable_guardada_no_aparece_en_ningun_registro() {
        let (a, d) = almacen_de_prueba("exec");
        usar_para_pruebas(a.clone());
        let root = d.join("proyecto");
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        const VALOR: &str = "valor-muy-secreto-9876";
        a.guardar(&root, "SECRETO_F16", VALOR, false).unwrap();
        let dueno = format!("{}#f16-vars", root.display());
        let out = crate::exec::run(&dueno, "echo inicio; printenv SECRETO_F16; echo \"largo=${#SECRETO_F16}\"; echo fin", &root, 20, false, false, None, true).await.unwrap();
        assert!(out.contains("‹variable SECRETO_F16›"), "llegó al hijo y se tapó: {out}");
        assert!(out.contains(&format!("largo={}", VALOR.len())), "el hijo tiene el valor entero: {out}");
        assert!(!out.contains(VALOR), "{out}");
        // un proceso vivo: su orden en `ps` no lleva el valor
        let vivo = crate::exec::run(&dueno, "sleep 30", &root, 5, true, false, Some("dormir"), true).await.unwrap();
        assert!(!vivo.contains(VALOR));
        let ps = std::process::Command::new("ps").args(["-axww", "-o", "command="]).output().unwrap();
        let ps = String::from_utf8_lossy(&ps.stdout);
        assert!(ps.contains("sleep 30"), "el proceso vive");
        assert!(!ps.contains(VALOR), "ps no enseña el valor en la orden");
        crate::exec::kill_owner(&dueno);
        for e in std::fs::read_dir(root.join(".gotv/procesos")).unwrap().flatten() {
            let t = std::fs::read_to_string(e.path()).unwrap_or_default();
            assert!(!t.contains(VALOR), "registro {} limpio", e.path().display());
        }
        // otro proyecto no la recibe
        let otro = d.join("otro");
        std::fs::create_dir_all(&otro).unwrap();
        let out = crate::exec::run(&format!("{}#f16-vars2", otro.display()), "echo \"[${SECRETO_F16:-nada}]\"", &otro, 20, false, false, None, true).await.unwrap();
        assert!(out.contains("[nada]"), "{out}");
    }

    #[test]
    fn el_tapador_tapa_aunque_el_valor_llegue_partido() {
        let mut t = Tapador::de(&[("TOKEN".into(), "abcdef123456".into()), ("CORTO".into(), "ab".into())]).unwrap();
        let mut salida = Vec::new();
        salida.extend(t.pasar(b"hola abcdef", false));
        salida.extend(t.pasar(b"123456 adios ab", false));
        let [r, _] = t.fin();
        salida.extend(r);
        let s = String::from_utf8(salida).unwrap();
        assert_eq!(s, "hola ‹variable TOKEN› adios ab");
        // un principio que al final NO era el valor sale tal cual
        let mut t = Tapador::de(&[("TOKEN".into(), "abcdef123456".into())]).unwrap();
        let mut s = t.pasar(b"x abc", false);
        s.extend(t.pasar(b"zz", false));
        assert_eq!(String::from_utf8(s).unwrap(), "x abczz");
        assert!(Tapador::de(&[("X".into(), "abc".into())]).is_none());
    }
}
