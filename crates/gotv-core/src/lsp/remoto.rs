//! F16.3 · paso 5 · servidores de lenguaje para un proyecto REMOTO.
//!
//! **Decisión:** el servidor corre ALLÁ, el que ya tenga ese equipo (nada se instala), y habla LSP con GOTV por
//! la MISMA sesión ssh multiplexada del `Lugar` (su stdio por el `Canal` de F0.3). Allá están los archivos, el
//! compilador y las dependencias: un servidor de aquí no podría resolver nada. GOTV traduce las rutas en cada
//! mensaje: las de esta Mac son las del ANCLA (`~/.config/gotv/remotos/<n>/…`, lo que ven las herramientas),
//! las del servidor son las de la raíz remota. Los textos que GOTV le manda (`didOpen`/`didChange`) se leen
//! por el `Lugar`, no del disco local.
//!
//! El programa se busca con el `PATH` de la shell de inicio de sesión de allá (como `entorno.rs` lo hace
//! aquí: ssh sin terminal no carga el perfil, y `~/.cargo/bin` no estaría) y se lanza por su ruta absoluta con
//! ese `PATH`, SIN shell de inicio de sesión: un perfil que imprime algo no ensucia el canal.
use crate::lugar::{Lugar, Remoto};
use anyhow::{anyhow, bail, Result};
use std::path::{Path, PathBuf};

const A: &str = "__GOTV_LSP_A__";
const B: &str = "__GOTV_LSP_B__";

pub struct Puente {
    /// el ancla en esta Mac, tal como la ven las herramientas y en su forma canónica (`/var` ↔ `/private/var`)
    ancla: PathBuf,
    ancla_canon: PathBuf,
    /// la raíz allá
    pub raiz: PathBuf,
    pub remoto: Remoto,
}

impl Puente {
    /// El puente de un ancla de proyecto remoto (`None` si `root` es un proyecto local).
    pub fn de(root: &Path) -> Option<Puente> {
        let remoto = crate::lugar::remoto_de(root)?;
        let raiz = remoto.raiz().to_path_buf();
        Some(Puente { ancla: root.to_path_buf(), ancla_canon: root.canonicalize().unwrap_or_else(|_| root.to_path_buf()), raiz, remoto })
    }

    fn relativa(&self, p: &Path) -> Option<PathBuf> {
        p.strip_prefix(&self.ancla).or_else(|_| p.strip_prefix(&self.ancla_canon)).ok().map(Path::to_path_buf)
    }

    /// Ruta de aquí (del ancla) → la de allá. Una que no es del ancla se deja (ya era de allá).
    pub fn a_remota(&self, p: &Path) -> PathBuf {
        match self.relativa(p) {
            Some(r) if r.as_os_str().is_empty() => self.raiz.clone(),
            Some(r) => self.raiz.join(r),
            None => p.to_path_buf(),
        }
    }

    /// URI del servidor → ruta de aquí (en el ancla). Una de fuera de la raíz remota se deja como estaba.
    pub fn a_local(&self, uri: &str) -> PathBuf {
        let p = PathBuf::from(uri.trim_start_matches("file://"));
        match p.strip_prefix(&self.raiz) {
            Ok(r) if r.as_os_str().is_empty() => self.ancla_canon.clone(),
            Ok(r) => self.ancla_canon.join(r),
            Err(_) => p,
        }
    }

    /// El texto de un archivo (ruta de aquí), leído ALLÁ.
    pub async fn leer(&self, p: &Path) -> Result<String> {
        let rel = self.relativa(p).ok_or_else(|| anyhow!("{}", crate::tr!("{} no es del proyecto remoto", "{} is not in the remote project", p.display())))?;
        let b = self.remoto.leer(&rel, 0, crate::lugar::LEER_MAX).await?;
        String::from_utf8(b).map_err(|_| anyhow!("{}", crate::tr!("{} es binario (no UTF-8)", "{} is binary (not UTF-8)", p.display())))
    }

    /// El primer candidato que existe ALLÁ (por su ruta absoluta) y el `PATH` de la shell de inicio de sesión de
    /// allá. `Err` dice qué falta, en claro.
    pub async fn buscar(&self, candidatos: &[Vec<String>], instalar: &str) -> Result<(Vec<String>, String)> {
        let nombres: Vec<String> = candidatos.iter().filter_map(|c| c.first()).map(|n| crate::lugar::q(n)).collect();
        if nombres.is_empty() {
            bail!("{}", crate::i18n::t("sin candidatos", "no candidates"));
        }
        // la shell de inicio de sesión del usuario de allá (zsh, bash…), con marcas: lo que imprima su perfil no cuenta
        let dentro = format!("printf '%s\\n' {A}; printf 'PATH=%s\\n' \"$PATH\"; for c in {}; do command -v \"$c\" 2>/dev/null; done; printf '%s\\n' {B}", nombres.join(" "));
        let orden = format!("exec \"${{SHELL:-/bin/sh}}\" -lc {}", crate::lugar::q(&dentro));
        let (_, salida, _) = self.remoto.correr(&orden, 30).await?;
        let cuerpo = salida.split(A).nth(1).and_then(|r| r.split(B).next()).unwrap_or("");
        let path = cuerpo.lines().find_map(|l| l.strip_prefix("PATH=")).unwrap_or("/usr/bin:/bin").to_string();
        let hallados: Vec<&str> = cuerpo.lines().filter(|l| l.starts_with('/')).collect();
        for c in candidatos {
            let Some(n) = c.first() else { continue };
            let base = Path::new(n).file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
            let hallado = if n.starts_with('/') { hallados.iter().find(|h| **h == n) } else { hallados.iter().find(|h| h.ends_with(&format!("/{base}"))) };
            if let Some(abs) = hallado {
                let mut v = c.clone();
                v[0] = abs.to_string();
                return Ok((v, path));
            }
        }
        bail!(
            "{}",
            crate::tr!(
                "el equipo remoto no tiene servidor de lenguaje para esto ({}): instálalo allá (`{instalar}`); GOTV no instala nada en otro equipo",
                "the remote machine has no language server for this ({}): install it there (`{instalar}`); GOTV installs nothing on another machine",
                candidatos.iter().filter_map(|c| c.first().cloned()).collect::<Vec<_>>().join(", ")
            )
        )
    }

    /// El `ssh` que lanza el servidor allá, en la raíz, con el `PATH` de allá y por su ruta absoluta.
    pub fn lanzar(&self, cmd: &[String], path: &str) -> Result<tokio::process::Command> {
        let partes: Vec<String> = cmd.iter().map(|x| crate::lugar::q(x)).collect();
        self.remoto.comando(&format!("PATH={} exec {}", crate::lugar::q(path), partes.join(" ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traduce_las_rutas_en_los_dos_sentidos() {
        let d = crate::CarpetaPrueba::nueva("gotv-lsp-puente");
        crate::lugar::guardar_ficha(&d, &crate::lugar::Ficha { conexion: "nodo".into(), raiz: "/srv/App".into() }).unwrap();
        let p = Puente::de(&d).unwrap();
        assert_eq!(p.a_remota(&d.join("src/main.rs")), Path::new("/srv/App/src/main.rs"));
        assert_eq!(p.a_remota(&d.canonicalize().unwrap().join("src/main.rs")), Path::new("/srv/App/src/main.rs"));
        assert_eq!(p.a_remota(&d), Path::new("/srv/App"));
        assert_eq!(p.a_local("file:///srv/App/src/lib.rs"), d.canonicalize().unwrap().join("src/lib.rs"));
        assert_eq!(p.a_local("file:///usr/lib/rustlib/core.rs"), Path::new("/usr/lib/rustlib/core.rs"), "lo de fuera no se toca");
        // las mayúsculas de allá se respetan (sin canonicalize local)
        assert_eq!(p.a_local("file:///srv/app/x.rs"), Path::new("/srv/app/x.rs"));
        assert!(Puente::de(&crate::CarpetaPrueba::nueva("gotv-lsp-puente-local")).is_none());
    }

    /// Ficha F16.3 paso 5, contra el `sshd` desechable: el servidor de lenguaje corre ALLÁ (el falso de F5, como
    /// proceso, buscado con el `PATH` de allá), recibe el texto leído ALLÁ (el ancla no tiene el archivo),
    /// publica con rutas de allá y GOTV las ve como rutas del ancla: el error del archivo editado y el que nace en
    /// OTRO archivo (`comparar`, lo que el agente recibe tras editar). Un lenguaje sin servidor allá lo dice.
    #[tokio::test]
    async fn el_servidor_de_lenguaje_corre_alla_y_las_rutas_se_traducen() {
        use crate::lsp::gestor::Gestor;
        use crate::lsp::registro::Def;
        use std::time::Duration;
        let Some(s) = crate::lugar::pruebas::servidor("lsp") else { return };
        let raiz = s.dir.join("app");
        std::fs::create_dir_all(raiz.join("src")).unwrap();
        std::fs::write(raiz.join("src/a.rs"), "fn main() {}\nERROR aquí\n").unwrap();
        std::fs::write(raiz.join("src/b.rs"), "fn b() {}\n").unwrap();
        let ancla = crate::CarpetaPrueba::nueva("gotv-lsp-remoto-ancla");
        crate::lugar::pruebas::ancla_con_conexion(&s, &ancla, &raiz);
        let ancla = ancla.canonicalize().unwrap();
        // el servidor falso, lanzado ALLÁ por `env` (la variable que lo enciende viaja en la orden)
        let mut falso = vec!["env".to_string(), "GOTV_LSP_FALSO=1".to_string()];
        falso.extend(crate::lsp::falso::tests::comando_falso());
        let rust = Def { id: "rust", lang: "rust", exts: &["rs"], candidatos: vec![falso], raices: &["Cargo.toml"], mas_alta: true, instalar: "-", solo_abiertos: false, requiere: &[] };
        let ts = Def { id: "typescript", lang: "typescript", exts: &["ts"], candidatos: vec![vec!["gotv-servidor-que-no-existe".into()]], raices: &["package.json"], mas_alta: false, instalar: "npm i -g x", solo_abiertos: false, requiere: &[] };
        let mut g = Gestor::con(&ancla, vec![rust, ts]);
        let a = ancla.join("src/a.rs");
        assert!(!a.exists(), "el ancla no tiene los archivos: viven allá");
        assert!(g.handles(&a));
        let (d, completo) = g.diagnostics_full(&a, Duration::from_secs(15)).await.unwrap();
        assert!(completo && d.len() == 1 && d[0].line == 2, "{d:?}");
        let avisos = g.tomar_avisos();
        assert!(avisos.iter().any(|x| x.contains("remot")), "{avisos:?}");
        // el error nace en OTRO archivo: la ruta de allá vuelve como la del ancla
        let antes = g.foto().await;
        std::fs::write(raiz.join("src/a.rs"), format!("fn main() {{}}\nROMPE:{}\n", raiz.join("src/b.rs").display())).unwrap();
        g.diagnostics_full(&a, Duration::from_secs(15)).await.unwrap();
        let inf = g.comparar(&antes, &a).await.expect("hay base");
        assert!(inf.nuevos.iter().any(|(p, _, _)| *p == ancla.join("src/b.rs")), "{:?}", inf.nuevos);
        assert_eq!(inf.resueltos, 1, "el ERROR de a.rs se fue");
        // un lenguaje sin servidor ALLÁ: se dice, y no se vuelve a intentar
        let e = g.diagnostics_full(&ancla.join("web/x.ts"), Duration::from_secs(5)).await.unwrap_err().to_string();
        assert!(e.contains("gotv-servidor-que-no-existe") && (e.contains("remoto") || e.contains("remote")), "{e}");
        assert!(!g.handles(&ancla.join("web/x.ts")));
        assert!(!ancla.join("src").exists(), "nada se escribió en el ancla");
    }
}
