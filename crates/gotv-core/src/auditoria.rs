//! F20.1 · `package` con `registry: "auditar"`: repasa los ARCHIVOS DE BLOQUEO del proyecto (lo que de
//! verdad se instala, versión por versión) y consulta OSV por lotes: las vulnerabilidades conocidas por
//! gravedad, con la versión que las corrige. Sin red dice «no pude comprobarlo», nunca «sin problemas».
//!
//! Lectores (cada uno tolera lo que no entiende: una línea rara se salta, no tumba la auditoría):
//! `Cargo.lock` · `package-lock.json` (v1 y v2/v3) · `pnpm-lock.yaml` (v5, v6 y v9) · `yarn.lock` (1 y 2+) ·
//! `uv.lock` · `poetry.lock` · `requirements.txt` (sólo lo fijado con `==`) · `go.sum` · `composer.lock` ·
//! `Gemfile.lock`.
use std::collections::BTreeSet;
use std::path::Path;

/// (ecosistema de OSV, nombre, versión)
pub type Dependencia = (String, String, String);

fn dep(eco: &str, n: &str, v: &str) -> Option<Dependencia> {
    let (n, v) = (n.trim().trim_matches('"'), v.trim().trim_matches('"').trim_start_matches('v'));
    (!n.is_empty() && v.chars().next().is_some_and(|c| c.is_ascii_digit())).then(|| (eco.to_string(), n.to_string(), v.to_string()))
}

/// `[[package]]` con `name` y `version` (Cargo.lock, uv.lock, poetry.lock). En Cargo, sólo lo que viene
/// de un registro (no lo del propio workspace).
fn toml_paquetes(t: &str, eco: &str, solo_registro: bool) -> Vec<Dependencia> {
    let Ok(v) = toml::from_str::<toml::Value>(t) else { return vec![] };
    v.get("package").and_then(|p| p.as_array()).into_iter().flatten().filter(|p| !solo_registro || p.get("source").and_then(|s| s.as_str()).is_some_and(|s| s.starts_with("registry+") || s.starts_with("sparse+"))).filter_map(|p| dep(eco, p.get("name")?.as_str()?, p.get("version")?.as_str()?)).collect()
}

fn package_lock(t: &str) -> Vec<Dependencia> {
    let Ok(j) = serde_json::from_str::<serde_json::Value>(t) else { return vec![] };
    let mut v = Vec::new();
    if let Some(p) = j["packages"].as_object() {
        for (k, x) in p {
            // «node_modules/a/node_modules/@b/c» → «@b/c»; la raíz («») es el proyecto
            if let (Some(n), Some(ver)) = (k.rsplit("node_modules/").next().filter(|_| k.contains("node_modules/")), x["version"].as_str()) {
                v.extend(dep("npm", n, ver));
            }
        }
    } else if let Some(d) = j["dependencies"].as_object() {
        for (n, x) in d {
            if let Some(ver) = x["version"].as_str() {
                v.extend(dep("npm", n, ver));
            }
        }
    }
    v
}

/// Las claves de `packages:` de pnpm: `/a@1.0.0:` (v6), `a@1.0.0:` (v9), `/a/1.0.0:` (v5), con `(…)` de pares.
fn pnpm_lock(t: &str) -> Vec<Dependencia> {
    let mut v = Vec::new();
    let mut dentro = false;
    for l in t.lines() {
        if !l.starts_with(' ') && !l.is_empty() {
            dentro = l.trim_end() == "packages:";
            continue;
        }
        if !dentro || !l.starts_with("  ") || l.starts_with("   ") {
            continue;
        }
        let k = l.trim().trim_end_matches(':').trim_matches(|c| c == '\'' || c == '"');
        let k = k.split('(').next().unwrap_or(k).trim_start_matches('/');
        // a@1.2.3 / @a/b@1.2.3 / a/1.2.3 (v5)
        let (n, ver) = match k.rfind('@').filter(|i| *i > 0) {
            Some(i) => (&k[..i], &k[i + 1..]),
            None => match k.rsplit_once('/') {
                Some(x) => x,
                None => continue,
            },
        };
        v.extend(dep("npm", n, ver));
    }
    v
}

/// yarn 1: `"a@^1", a@~1:` + `  version "1.2.3"`; yarn 2+: `"a@npm:^1":` + `  version: 1.2.3`.
fn yarn_lock(t: &str) -> Vec<Dependencia> {
    let mut v = Vec::new();
    let mut nombre: Option<String> = None;
    for l in t.lines() {
        if !l.starts_with(' ') && l.trim_end().ends_with(':') && !l.starts_with('#') {
            let primera = l.trim_end_matches(':').split(", ").next().unwrap_or("").trim_matches('"');
            // el nombre es lo de antes del ÚLTIMO @ que no sea el primero (los paquetes con alcance empiezan por @)
            nombre = primera.get(1..).and_then(|r| r.find('@')).map(|i| primera[..i + 1].to_string()).filter(|n| !n.is_empty() && n != "__metadata");
            continue;
        }
        if let (Some(n), Some(ver)) = (&nombre, l.trim().strip_prefix("version").map(|r| r.trim_start_matches(':').trim())) {
            v.extend(dep("npm", n, ver));
            nombre = None;
        }
    }
    v
}

fn requirements(t: &str) -> Vec<Dependencia> {
    t.lines().filter_map(|l| l.split('#').next()).filter_map(|l| l.split_once("==")).filter_map(|(n, ver)| dep("PyPI", n.split('[').next().unwrap_or(n), ver.split([';', ' ']).next().unwrap_or(ver))).collect()
}

fn go_sum(t: &str) -> Vec<Dependencia> {
    let mut vistos = BTreeSet::new();
    t.lines().filter_map(|l| {
        let mut p = l.split_whitespace();
        let (m, ver) = (p.next()?, p.next()?.trim_end_matches("/go.mod"));
        vistos.insert((m.to_string(), ver.to_string())).then(|| dep("Go", m, ver))?
    }).collect()
}

fn composer_lock(t: &str) -> Vec<Dependencia> {
    let Ok(j) = serde_json::from_str::<serde_json::Value>(t) else { return vec![] };
    ["packages", "packages-dev"].iter().flat_map(|k| j[k].as_array().into_iter().flatten()).filter_map(|p| dep("Packagist", p["name"].as_str()?, p["version"].as_str()?)).collect()
}

/// `GEM` → `specs:` → `    nombre (1.2.3)` (cuatro espacios: las de seis son sus dependencias).
fn gemfile_lock(t: &str) -> Vec<Dependencia> {
    t.lines().filter(|l| l.starts_with("    ") && !l.starts_with("     ")).filter_map(|l| {
        let (n, r) = l.trim().split_once(" (")?;
        dep("RubyGems", n, r.trim_end_matches(')').split('-').next().unwrap_or(""))
    }).collect()
}

/// Las dependencias que fijan los archivos de bloqueo de `root` (sin repetir).
pub fn dependencias(root: &Path) -> (Vec<Dependencia>, Vec<&'static str>) {
    let lee = |f: &str| std::fs::read_to_string(root.join(f)).ok();
    let mut todas: Vec<Dependencia> = Vec::new();
    let mut leidos = Vec::new();
    let lectores: &[(&'static str, fn(&str) -> Vec<Dependencia>)] = &[
        ("Cargo.lock", |t| toml_paquetes(t, "crates.io", true)),
        ("package-lock.json", package_lock),
        ("pnpm-lock.yaml", pnpm_lock),
        ("yarn.lock", yarn_lock),
        ("uv.lock", |t| toml_paquetes(t, "PyPI", false)),
        ("poetry.lock", |t| toml_paquetes(t, "PyPI", false)),
        ("requirements.txt", requirements),
        ("go.sum", go_sum),
        ("composer.lock", composer_lock),
        ("Gemfile.lock", gemfile_lock),
    ];
    for (f, leer) in lectores {
        if let Some(t) = lee(f) {
            leidos.push(*f);
            for d in leer(&t) {
                if !todas.contains(&d) {
                    todas.push(d);
                }
            }
        }
    }
    (todas, leidos)
}

fn orden_gravedad(g: &str) -> u8 {
    match g {
        "CRITICAL" => 0,
        "HIGH" => 1,
        "MODERATE" | "MEDIUM" => 2,
        "LOW" => 3,
        _ => 4,
    }
}

/// La auditoría para el modelo.
pub async fn auditar(root: &Path) -> anyhow::Result<String> {
    let (deps, leidos) = dependencias(root);
    if leidos.is_empty() {
        return Ok(crate::i18n::t("No hay archivos de bloqueo en el proyecto (Cargo.lock, package-lock.json, pnpm-lock.yaml, yarn.lock, uv.lock, poetry.lock, requirements.txt, go.sum, composer.lock, Gemfile.lock): nada que auditar.", "There are no lock files in the project (Cargo.lock, package-lock.json, pnpm-lock.yaml, yarn.lock, uv.lock, poetry.lock, requirements.txt, go.sum, composer.lock, Gemfile.lock): nothing to audit.").into());
    }
    let c = reqwest::Client::builder().user_agent("GOD-OF-THE-VOID (local agent; audit)").timeout(std::time::Duration::from_secs(20)).build()?;
    let Some(ids) = crate::paquetes::osv_lote(&c, &deps).await else {
        return Ok(crate::tr!("{} dependencias en {}: NO pude comprobarlas (OSV sin respuesta). No las des por seguras.", "{} dependencies in {}: could NOT check them (OSV did not answer). Do not assume they are safe.", deps.len(), leidos.join(", ")));
    };
    let mut halladas: Vec<(crate::paquetes::Vuln, &Dependencia)> = Vec::new();
    let mut detalles = 0;
    for (d, ids) in deps.iter().zip(ids) {
        for id in ids {
            // el detalle cuesta una petición por vulnerabilidad: hasta 40 con detalle, el resto con su id
            let v = if detalles < 40 {
                detalles += 1;
                crate::paquetes::osv_detalle(&c, &id).await
            } else {
                None
            };
            halladas.push((v.unwrap_or(crate::paquetes::Vuln { id, resumen: String::new(), gravedad: "?".into(), arreglo: None }), d));
        }
    }
    let cabeza = crate::tr!("{} dependencias en {}", "{} dependencies in {}", deps.len(), leidos.join(", "));
    if halladas.is_empty() {
        return Ok(crate::tr!("{cabeza}: ninguna vulnerabilidad conocida en OSV.", "{cabeza}: no known vulnerability in OSV."));
    }
    halladas.sort_by(|a, b| orden_gravedad(&a.0.gravedad).cmp(&orden_gravedad(&b.0.gravedad)).then(a.1 .1.cmp(&b.1 .1)));
    let mut s = crate::tr!("{cabeza}: {} vulnerabilidad(es) conocidas, la más grave primero:", "{cabeza}: {} known vulnerabilit(ies), the most severe first:", halladas.len());
    for (v, (eco, n, ver)) in halladas.iter().take(30) {
        let arreglo = v.arreglo.as_deref().map(|a| crate::tr!(" → sube a {a}", " → upgrade to {a}")).unwrap_or_else(|| crate::i18n::t(" → sin versión que la corrija", " → no fixed version").into());
        s.push_str(&format!("\n- [{}] {n} {ver} ({eco}) · {}{arreglo}{}", v.gravedad, v.id, if v.resumen.is_empty() { String::new() } else { format!(" — {}", v.resumen) }));
    }
    if halladas.len() > 30 {
        s.push_str(&crate::tr!("\n… y {} más", "\n… and {} more", halladas.len() - 30));
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lee_cada_archivo_de_bloqueo() {
        let n = |v: Vec<Dependencia>| v.into_iter().map(|(e, n, v)| format!("{e}:{n}@{v}")).collect::<Vec<_>>();
        assert_eq!(n(toml_paquetes("[[package]]\nname = \"mio\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.200\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n", "crates.io", true)), ["crates.io:serde@1.0.200"]);
        assert_eq!(n(package_lock(r#"{"lockfileVersion":3,"packages":{"":{"name":"app"},"node_modules/lodash":{"version":"4.17.20"},"node_modules/a/node_modules/@b/c":{"version":"1.0.0"}}}"#)), ["npm:@b/c@1.0.0", "npm:lodash@4.17.20"]);
        assert_eq!(n(package_lock(r#"{"lockfileVersion":1,"dependencies":{"minimist":{"version":"1.2.0"}}}"#)), ["npm:minimist@1.2.0"]);
        assert_eq!(n(pnpm_lock("lockfileVersion: '9.0'\n\nimporters:\n  .:\n    dependencies: {}\n\npackages:\n\n  '@types/node@20.1.0':\n    resolution: {integrity: x}\n\n  vite@5.0.0(@types/node@20.1.0):\n    resolution: {integrity: y}\n")), ["npm:@types/node@20.1.0", "npm:vite@5.0.0"]);
        assert_eq!(n(pnpm_lock("packages:\n  /axios@0.21.0:\n    dev: false\n  /left-pad/1.3.0:\n    dev: true\n")), ["npm:axios@0.21.0", "npm:left-pad@1.3.0"]);
        assert_eq!(n(yarn_lock("# yarn lockfile v1\n\n\"@babel/core@^7.0.0\", \"@babel/core@^7.1.0\":\n  version \"7.20.0\"\n  resolved \"x\"\n\nlodash@^4.17.0:\n  version \"4.17.15\"\n")), ["npm:@babel/core@7.20.0", "npm:lodash@4.17.15"]);
        assert_eq!(n(yarn_lock("__metadata:\n  version: 8\n\n\"react@npm:^18.0.0\":\n  version: 18.2.0\n  resolution: \"react@npm:18.2.0\"\n")), ["npm:react@18.2.0"]);
        assert_eq!(n(requirements("requests==2.19.0  # viejo\nflask>=2\nDjango[argon2]==3.2.0 ; python_version > '3'\n")), ["PyPI:requests@2.19.0", "PyPI:Django@3.2.0"]);
        assert_eq!(n(go_sum("golang.org/x/text v0.3.5 h1:a=\ngolang.org/x/text v0.3.5/go.mod h1:b=\n")), ["Go:golang.org/x/text@0.3.5"]);
        assert_eq!(n(composer_lock(r#"{"packages":[{"name":"guzzlehttp/guzzle","version":"7.4.0"}],"packages-dev":[{"name":"phpunit/phpunit","version":"v9.5.0"}]}"#)), ["Packagist:guzzlehttp/guzzle@7.4.0", "Packagist:phpunit/phpunit@9.5.0"]);
        assert_eq!(n(gemfile_lock("GEM\n  remote: https://rubygems.org/\n  specs:\n    rack (2.2.3)\n      rack-test (>= 0)\n    nokogiri (1.13.0-arm64-darwin)\n\nPLATFORMS\n")), ["RubyGems:rack@2.2.3", "RubyGems:nokogiri@1.13.0"]);
    }

    /// OSV contra un servidor falso: lote, detalle con gravedad y arreglo, el orden por gravedad; y sin
    /// respuesta, «no pude comprobarlo».
    #[tokio::test]
    async fn auditar_contra_un_osv_falso() {
        use axum::routing::{get, post};
        let _g = crate::paquetes::CANDADO_RED.lock().await;
        let app = axum::Router::new()
            .route("/v1/querybatch", post(|axum::Json(q): axum::Json<serde_json::Value>| async move {
                let r: Vec<serde_json::Value> = q["queries"].as_array().unwrap().iter().map(|x| match x["package"]["name"].as_str().unwrap() {
                    "lodash" => serde_json::json!({"vulns": [{"id": "GHSA-lodash"}]}),
                    "minimist" => serde_json::json!({"vulns": [{"id": "GHSA-minimist"}]}),
                    _ => serde_json::json!({}),
                }).collect();
                axum::Json(serde_json::json!({"results": r}))
            }))
            .route("/v1/vulns/{id}", get(|axum::extract::Path(id): axum::extract::Path<String>| async move {
                let (g, f) = if id == "GHSA-lodash" { ("HIGH", "4.17.21") } else { ("CRITICAL", "1.2.6") };
                axum::Json(serde_json::json!({"id": id, "summary": format!("fallo de {id}"), "database_specific": {"severity": g}, "affected": [{"ranges": [{"events": [{"introduced": "0"}, {"fixed": f}]}]}]}))
            }));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dir = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        let d = crate::CarpetaPrueba::nueva("gotv-auditoria");
        std::fs::write(d.join("package-lock.json"), r#"{"lockfileVersion":3,"packages":{"":{},"node_modules/lodash":{"version":"4.17.20"},"node_modules/minimist":{"version":"1.2.0"},"node_modules/ok":{"version":"1.0.0"}}}"#).unwrap();
        std::env::set_var("GOTV_OSV", format!("http://{dir}"));
        let t = auditar(&d).await.unwrap();
        assert!(t.contains("3 dependencias") || t.contains("3 dependencies"), "{t}");
        let (i_crit, i_alta) = (t.find("[CRITICAL] minimist").unwrap(), t.find("[HIGH] lodash").unwrap());
        assert!(i_crit < i_alta, "la más grave primero: {t}");
        assert!(t.contains("1.2.6") && t.contains("4.17.21"), "{t}");
        // sin servidor: «no pude comprobarlo»
        std::env::set_var("GOTV_OSV", "http://127.0.0.1:9");
        let t = auditar(&d).await.unwrap();
        assert!(t.contains("NO pude") || t.contains("could NOT"), "{t}");
        std::env::remove_var("GOTV_OSV");
        let vacio = crate::CarpetaPrueba::nueva("gotv-auditoria-vacia");
        assert!(auditar(&vacio).await.unwrap().contains("nada que auditar") || auditar(&vacio).await.unwrap().contains("nothing to audit"));
    }
}
