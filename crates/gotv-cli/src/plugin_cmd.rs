//! F10.4 · `gotv plugin lista | instalar <carpeta> [--si] [--reemplazar] | confiar <nombre> | quitar <nombre>`.
//! Instalar ENSEÑA todo lo que el plugin trae (los programas que correrán en esta Mac) y pide confianza a una
//! persona en la terminal; sin terminal hace falta `--si` (quien lo escribe es quien confía). Sin confianza el
//! plugin queda instalado pero no carga nada.
use anyhow::{bail, Result};
use gotv_core::plugins::{self, Estado};
use gotv_core::tr;
use std::io::{BufRead, IsTerminal, Write};

/// Pregunta en la terminal; sin terminal, no (no hay a quién preguntar).
fn confirmar(pregunta: &str) -> bool {
    if !std::io::stdin().is_terminal() {
        return false;
    }
    print!("{pregunta} [s/N] ");
    std::io::stdout().flush().ok();
    let mut l = String::new();
    std::io::stdin().lock().read_line(&mut l).ok();
    matches!(l.trim().to_lowercase().as_str(), "s" | "si" | "sí" | "y" | "yes") // juez-idioma: datos (respuestas que se aceptan)
}

pub fn correr(args: &[String]) -> Result<()> {
    let base = plugins::dir();
    let si = args.iter().any(|a| a == "--si" || a == "--yes");
    let reemplazar = args.iter().any(|a| a == "--reemplazar" || a == "--replace");
    let resto: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    match resto.first().map(|s| s.as_str()) {
        None | Some("lista" | "list") => {
            let v = plugins::instalados_en(&base);
            if v.is_empty() {
                println!("{}", tr!("sin plugins en {}", "no plugins in {}", base.display()));
            }
            for (p, c, e) in v {
                let estado = match e {
                    Estado::Activo => gotv_core::i18n::t("✓ activo", "✓ active").to_string(),
                    Estado::SinConfianza => tr!("· sin confianza (no carga): `gotv plugin confiar {}`", "· not trusted (loads nothing): `gotv plugin confiar {}`", p.nombre),
                    Estado::Cambiado => tr!("⚠ cambió desde que confiaste (no carga): revisa y `gotv plugin confiar {}`", "⚠ changed since you trusted it (loads nothing): review it and `gotv plugin confiar {}`", p.nombre),
                    Estado::Roto(por) => tr!("✗ no se puede leer: {por}", "✗ cannot be read: {por}"),
                };
                for l in plugins::resumen(&p, &c) {
                    println!("{l}");
                }
                println!("  {estado}\n");
            }
            Ok(())
        }
        Some("instalar" | "install") => {
            let Some(origen) = resto.get(1) else { bail!("{}", tr!("uso: gotv plugin instalar <carpeta> [--si] [--reemplazar]", "usage: gotv plugin instalar <folder> [--si] [--reemplazar]")) };
            let origen = std::path::Path::new(origen.as_str()).canonicalize()?;
            let (p, c) = plugins::leer(&origen, &base)?;
            println!("{}", gotv_core::i18n::t("Este plugin trae (lo que dice «correrá» son programas que se ejecutarán en esta Mac):", "This plugin brings (what says «will run» are programs that will execute on this Mac):"));
            for l in plugins::resumen(&p, &c) {
                println!("{l}");
            }
            let confia = si || confirmar(gotv_core::i18n::t("¿Confías en este contenido exacto?", "Do you trust this exact content?"));
            let p = plugins::instalar_en(&base, &origen, confia, reemplazar)?;
            if confia {
                println!("{}", tr!("✓ «{}» instalado y confiado en {}", "✓ «{}» installed and trusted in {}", p.nombre, p.dir.display()));
            } else {
                println!("{}", tr!("· «{}» instalado SIN confianza: no carga nada hasta `gotv plugin confiar {}`", "· «{}» installed WITHOUT trust: it loads nothing until `gotv plugin confiar {}`", p.nombre, p.nombre));
            }
            Ok(())
        }
        Some("confiar" | "trust") => {
            let Some(n) = resto.get(1) else { bail!("{}", tr!("uso: gotv plugin confiar <nombre>", "usage: gotv plugin confiar <name>")) };
            let (p, c) = plugins::leer(&base.join(n.as_str()), &base)?;
            for l in plugins::resumen(&p, &c) {
                println!("{l}");
            }
            if !(si || confirmar(gotv_core::i18n::t("¿Confías en este contenido exacto?", "Do you trust this exact content?"))) {
                bail!("{}", gotv_core::i18n::t("sin confianza: no cambia nada", "not trusted: nothing changes"));
            }
            plugins::confiar_en(&base, n)?;
            println!("{}", tr!("✓ «{n}» confiado: carga en la próxima orden", "✓ «{n}» trusted: it loads with the next order"));
            Ok(())
        }
        Some("quitar" | "remove" | "uninstall") => {
            let Some(n) = resto.get(1) else { bail!("{}", tr!("uso: gotv plugin quitar <nombre>", "usage: gotv plugin quitar <name>")) };
            plugins::quitar_en(&base, n)?;
            println!("{}", tr!("✓ «{n}» quitado (su carpeta y su confianza)", "✓ «{n}» removed (its folder and its trust)"));
            Ok(())
        }
        Some(otro) => bail!("{}", tr!("«{otro}»: gotv plugin lista | instalar <carpeta> | confiar <nombre> | quitar <nombre>", "«{otro}»: gotv plugin lista | instalar <folder> | confiar <name> | quitar <name>")),
    }
}
