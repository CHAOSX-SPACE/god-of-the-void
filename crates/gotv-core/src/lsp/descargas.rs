//! F5 · ML8 · la descarga OFICIAL de un servidor de lenguaje, con su suma SHA-256 FIJA, para una Mac sin
//! Homebrew (o cuando la fórmula arrastraría un runtime que la descarga no necesita: marksman con brew
//! pide .NET; su binario oficial es autocontenido). Va a `~/.cache/gotv/lsp/<id>/<versión>/`. Nunca un
//! intérprete: sólo binarios nativos de su proyecto oficial.
//!
//! Medido 2026-11-16 en esta Mac (Apple Silicon): las cuatro bajadas de GitHub y su SHA-256 coincide con
//! la huella que GitHub publica del archivo (zls, lua-language-server, marksman); taplo 0.10.0 es anterior
//! a esas huellas: la suya se midió de la descarga oficial por HTTPS. Sólo arm64 (el foco es esta Mac).
use anyhow::{bail, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Formato {
    TarXz,
    TarGz,
    /// un solo archivo comprimido con gzip
    Gz,
    /// el binario tal cual
    Binario,
}

#[derive(Debug)]
pub struct Descarga {
    /// el id del lenguaje en el registro (`lua`, `toml`…)
    pub id: &'static str,
    /// el programa del candidato del registro al que reemplaza
    pub programa: &'static str,
    pub version: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub formato: Formato,
    /// dónde queda el ejecutable dentro de la carpeta instalada
    pub ruta: &'static str,
    /// lo que debe estar YA (zls sin zig no sirve); a diferencia de la fórmula de brew, nada más
    pub requiere: &'static [&'static str],
}

pub const DESCARGAS: &[Descarga] = &[
    Descarga { id: "zig", programa: "zls", version: "0.17.0", url: "https://github.com/zigtools/zls/releases/download/0.17.0/zls-aarch64-macos.tar.xz", sha256: "2e451358b4f7e2d4b1fccb65db177dbb321c5a98e95f67e09aa72d6ecfb39d20", formato: Formato::TarXz, ruta: "zls", requiere: &["zig"] },
    Descarga { id: "lua", programa: "lua-language-server", version: "3.19.2", url: "https://github.com/LuaLS/lua-language-server/releases/download/3.19.2/lua-language-server-3.19.2-darwin-arm64.tar.gz", sha256: "79149584320a23bd5fcd5bc31f47a1975acd8a4560a0e89036bb1016fa61687f", formato: Formato::TarGz, ruta: "bin/lua-language-server", requiere: &[] },
    Descarga { id: "toml", programa: "taplo", version: "0.10.0", url: "https://github.com/tamasfe/taplo/releases/download/0.10.0/taplo-darwin-aarch64.gz", sha256: "713734314c3e71894b9e77513c5349835eefbd52908445a0d73b0c7dc469347d", formato: Formato::Gz, ruta: "taplo", requiere: &[] },
    Descarga { id: "markdown", programa: "marksman", version: "2026-02-08", url: "https://github.com/artempyanykh/marksman/releases/download/2026-02-08/marksman-macos", sha256: "6a801c17b5ac0dba69787c5282b3b3bd416e66c96253fae098d311c6bbd1833b", formato: Formato::Binario, ruta: "marksman", requiere: &[] },
];

/// La descarga de ese lenguaje para ESTA máquina (sólo Apple Silicon).
pub fn para(id: &str) -> Option<&'static Descarga> {
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return None;
    }
    DESCARGAS.iter().find(|d| d.id == id)
}

pub fn base() -> PathBuf {
    crate::paths::home().join(".cache/gotv/lsp")
}

fn carpeta(base: &Path, d: &Descarga) -> PathBuf {
    base.join(d.id).join(d.version)
}

/// Si ese programa ya se bajó, su ruta absoluta.
pub fn instalado(programa: &str) -> Option<PathBuf> {
    instalado_en(&base(), programa)
}

fn instalado_en(base: &Path, programa: &str) -> Option<PathBuf> {
    DESCARGAS.iter().filter(|d| d.programa == programa).map(|d| carpeta(base, d).join(d.ruta)).find(|p| p.is_file())
}

/// ¿Estos bytes son los que dice la tabla?
pub fn verificar(bytes: &[u8], sha256: &str) -> bool {
    let h = Sha256::digest(bytes);
    h.iter().map(|b| format!("{b:02x}")).collect::<String>() == sha256
}

/// Deja el contenido en `destino` según su formato (tar de macOS para los .tar; gzip para el .gz).
fn extraer(archivo: &Path, formato: Formato, destino: &Path, programa: &str) -> Result<()> {
    std::fs::create_dir_all(destino)?;
    let ok = match formato {
        Formato::TarXz | Formato::TarGz => std::process::Command::new("/usr/bin/tar").arg("-xf").arg(archivo).arg("-C").arg(destino).status()?.success(),
        Formato::Gz => {
            let salida = std::fs::File::create(destino.join(programa))?;
            std::process::Command::new("/usr/bin/gzip").arg("-dc").arg(archivo).stdout(salida).status()?.success()
        }
        Formato::Binario => std::fs::copy(archivo, destino.join(programa)).is_ok(),
    };
    if !ok {
        bail!("{}", crate::tr!("no pude extraer {}", "could not extract {}", archivo.display()));
    }
    Ok(())
}

/// Baja, comprueba la suma y deja el ejecutable listo. Si la suma no casa, NO se instala nada.
pub async fn instalar(d: &Descarga) -> Result<PathBuf> {
    instalar_en(&base(), d).await
}

pub async fn instalar_en(base: &Path, d: &Descarga) -> Result<PathBuf> {
    let destino = carpeta(base, d);
    let exe = destino.join(d.ruta);
    if exe.is_file() {
        return Ok(exe);
    }
    let c = reqwest::Client::builder().user_agent("gotv").timeout(std::time::Duration::from_secs(300)).build()?;
    let r = c.get(d.url).send().await?;
    if !r.status().is_success() {
        bail!("{}", crate::tr!("la descarga de {} respondió {}", "the {} download answered {}", d.programa, r.status()));
    }
    let bytes = r.bytes().await?;
    if !verificar(&bytes, d.sha256) {
        bail!("{}", crate::tr!(
            "la suma SHA-256 de {} {} NO casa con la fijada: no instalé nada (el archivo pudo cambiar o estar alterado)",
            "the SHA-256 of {} {} does NOT match the pinned one: nothing installed (the file may have changed or been tampered with)",
            d.programa,
            d.version
        ));
    }
    // se arma aparte y se mueve al final: una carpeta a medias nunca parece instalada
    let tmp = base.join(format!(".{}-{}-{}", d.id, d.version, std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    let archivo = tmp.join("descarga");
    std::fs::write(&archivo, &bytes)?;
    let armado = tmp.join("listo");
    let r = extraer(&archivo, d.formato, &armado, d.ruta);
    if let Err(e) = r {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    let bin = armado.join(d.ruta);
    if !bin.is_file() {
        let _ = std::fs::remove_dir_all(&tmp);
        bail!("{}", crate::tr!("la descarga de {} no trae {}", "the {} download has no {}", d.programa, d.ruta));
    }
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))?;
    if let Some(p) = destino.parent() {
        std::fs::create_dir_all(p)?;
    }
    let _ = std::fs::remove_dir_all(&destino);
    std::fs::rename(&armado, &destino)?;
    let _ = std::fs::remove_dir_all(&tmp);
    super::registro::olvidar();
    Ok(exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La tabla es coherente: HTTPS de GitHub, la versión en la dirección, una suma de 64 hex y un
    /// candidato del registro de ese lenguaje.
    #[test]
    fn la_tabla_es_coherente() {
        for d in DESCARGAS {
            assert!(d.url.starts_with("https://github.com/") && d.url.contains(d.version), "{d:?}");
            assert!(d.sha256.len() == 64 && d.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{d:?}");
            let def = super::super::registro::defs().into_iter().find(|x| x.id == d.id).unwrap_or_else(|| panic!("{} no está en el registro", d.id));
            assert!(def.candidatos.iter().any(|c| c[0] == d.programa), "{d:?}");
        }
    }

    #[test]
    fn la_suma_se_comprueba() {
        assert!(verificar(b"", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"));
        assert!(!verificar(b"x", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"));
    }

    /// Los cuatro formatos se extraen (con el tar y el gzip de macOS) y el ejecutable queda donde dice.
    #[test]
    fn cada_formato_se_extrae() {
        let d = crate::CarpetaPrueba::nueva("gotv-descargas");
        std::fs::create_dir_all(d.join("src/bin")).unwrap();
        std::fs::write(d.join("src/bin/srv"), "#!/bin/sh\necho hola\n").unwrap();
        std::fs::write(d.join("srv"), "#!/bin/sh\necho hola\n").unwrap();
        assert!(std::process::Command::new("/usr/bin/tar").args(["-czf", "a.tar.gz", "-C", "src", "bin"]).current_dir(&*d).status().unwrap().success());
        assert!(std::process::Command::new("/usr/bin/tar").args(["-cJf", "a.tar.xz", "-C", "src", "bin"]).current_dir(&*d).status().unwrap().success());
        assert!(std::process::Command::new("/bin/sh").args(["-c", "gzip -c srv > a.gz"]).current_dir(&*d).status().unwrap().success());
        for (archivo, formato, ruta) in [("a.tar.gz", Formato::TarGz, "bin/srv"), ("a.tar.xz", Formato::TarXz, "bin/srv"), ("a.gz", Formato::Gz, "srv"), ("srv", Formato::Binario, "srv")] {
            let destino = d.join(format!("fuera-{archivo}"));
            extraer(&d.join(archivo), formato, &destino, ruta).unwrap();
            assert_eq!(std::fs::read_to_string(destino.join(ruta)).unwrap(), "#!/bin/sh\necho hola\n", "{archivo}");
        }
    }

    /// DE VERDAD: baja los cuatro servidores oficiales a una carpeta temporal, comprueba su suma y que
    /// cada uno ARRANCA en esta Mac. ~52 MB de red.
    /// `cargo test --release -p gotv-core -- --ignored --exact lsp::descargas::tests::las_cuatro_descargas_de_verdad --nocapture`
    #[tokio::test]
    #[ignore]
    async fn las_cuatro_descargas_de_verdad() {
        let d = crate::CarpetaPrueba::nueva("gotv-descargas-reales");
        for x in DESCARGAS {
            let t = std::time::Instant::now();
            let exe = instalar_en(&d, x).await.unwrap_or_else(|e| panic!("{}: {e}", x.programa));
            assert_eq!(instalado_en(&d, x.programa).as_deref(), Some(exe.as_path()));
            let o = std::process::Command::new(&exe).arg("--version").output().unwrap();
            let v = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
            eprintln!("{} {} · {:?} · --version: {}", x.programa, x.version, t.elapsed(), v.lines().next().unwrap_or("").trim());
            assert!(o.status.success(), "{} no arranca: {v}", x.programa);
        }
        // una segunda vez no baja nada
        let t = std::time::Instant::now();
        instalar_en(&d, &DESCARGAS[2]).await.unwrap();
        assert!(t.elapsed() < std::time::Duration::from_millis(50));
    }
}
