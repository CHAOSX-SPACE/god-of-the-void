//! F5 · ML1 · la tabla declarativa de servidores de lenguaje: qué servidor atiende qué extensiones, con
//! qué comandos (en orden de preferencia, la escalera de L1) y qué archivos marcan su raíz.
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq)]
pub struct Def {
    pub id: &'static str,
    pub lang: &'static str,
    pub exts: &'static [&'static str],
    /// comandos candidatos (programa y argumentos), el primero que esté instalado gana
    pub candidatos: Vec<Vec<String>>,
    /// archivos que marcan la raíz de un proyecto de este lenguaje
    pub raices: &'static [&'static str],
    /// true = la raíz es la MÁS ALTA con marcador (un workspace de Cargo); false = la más cercana
    pub mas_alta: bool,
    pub instalar: &'static str,
    /// F5 ML2: el servidor sólo publica errores de los archivos ABIERTOS (TypeScript; pyright por omisión):
    /// GOTV abre los que usan lo que se va a tocar, para que un error que nazca allí se vea
    pub solo_abiertos: bool,
    /// F5 ML8: programas que deben ESTAR antes de instalar el servidor (su intérprete o runtime: Java,
    /// Elixir, .NET, Zig). GOTV no los instala por la espalda: la fórmula de Homebrew los arrastraría.
    pub requiere: &'static [&'static str],
}

fn c(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Una fila de la tabla (ML4): id y lenguaje, extensiones, candidatos en orden de preferencia, marcadores
/// de raíz (`*.ext` = cualquier entrada con esa extensión, p. ej. `*.xcodeproj`), si manda la raíz más alta,
/// el instalador oficial (vacío = viene con su SDK), lo que el instalador exige y si sólo mira lo abierto.
#[allow(clippy::too_many_arguments)]
fn fila(id: &'static str, exts: &'static [&'static str], cands: &[&[&str]], raices: &'static [&'static str], mas_alta: bool, instalar: &'static str, requiere: &'static [&'static str], solo_abiertos: bool) -> Def {
    Def { id, lang: id, exts, candidatos: cands.iter().map(|x| c(x)).collect(), raices, mas_alta, instalar, solo_abiertos, requiere }
}

/// Los servidores que GOTV sabe usar (ML4 · la tabla del plan, por la escalera de L1). Las filas SIN
/// marcador de raíz (JSON, CSS, YAML, TOML, Bash, Markdown) no dicen «proyecto X sin servidor»: arrancan
/// si están instalados cuando se toca uno de sus archivos. Verificados de verdad en esta Mac (2026-11-15):
/// rust-analyzer, typescript-language-server, sourcekit-lsp y clangd; el resto, por su documentación.
pub fn defs() -> Vec<Def> {
    // F10.4: los de los plugins confiados van primero (sólo los que tienen su programa)
    let mut v = crate::plugins::defs_lsp();
    v.extend(defs_de_gotv());
    v
}

fn defs_de_gotv() -> Vec<Def> {
    vec![
        fila("rust", &["rs"], &[&["rust-analyzer"]], &["Cargo.toml"], true, "rustup component add rust-analyzer", &[], false),
        fila("python", &["py", "pyi"], &[&["basedpyright-langserver", "--stdio"], &["pyright-langserver", "--stdio"], &["pylsp"]], &["pyproject.toml", "setup.py", "requirements.txt"], false, "npm i -g pyright", &[], true),
        fila("typescript", &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"], &[&["typescript-language-server", "--stdio"]], &["tsconfig.json", "package.json"], false, "npm i -g typescript typescript-language-server", &[], true),
        fila("go", &["go"], &[&["gopls"]], &["go.mod"], false, "go install golang.org/x/tools/gopls@latest", &[], false),
        fila("php", &["php"], &[&["intelephense", "--stdio"], &["phpactor", "language-server"]], &["composer.json"], false, "npm i -g intelephense", &[], false),
        // Xcode: vienen con sus herramientas de línea de órdenes
        fila("swift", &["swift"], &[&["sourcekit-lsp"]], &["Package.swift", "*.xcodeproj", "*.xcworkspace"], false, "xcode-select --install", &[], false),
        fila("c", &["c", "h", "cc", "cpp", "cxx", "hpp", "hh", "hxx", "m", "mm"], &[&["clangd"]], &["compile_commands.json", "compile_flags.txt", "CMakeLists.txt", "meson.build"], false, "xcode-select --install", &[], false),
        fila("zig", &["zig", "zon"], &[&["zls"]], &["build.zig"], false, "brew install zls", &["zig"], false),
        fila("dart", &["dart"], &[&["dart", "language-server", "--protocol=lsp"]], &["pubspec.yaml"], false, "", &[], false),
        fila("java", &["java"], &[&["jdtls"]], &["pom.xml", "build.gradle", "settings.gradle"], true, "brew install jdtls", &["java"], false),
        fila("kotlin", &["kt", "kts"], &[&["kotlin-language-server"]], &["settings.gradle.kts", "build.gradle.kts"], true, "brew install kotlin-language-server", &["java"], false),
        fila("csharp", &["cs"], &[&["csharp-ls"]], &["*.sln", "*.csproj", "global.json"], false, "dotnet tool install --global csharp-ls", &[], false),
        fila("ruby", &["rb", "rake", "gemspec"], &[&["ruby-lsp"]], &["Gemfile"], false, "gem install ruby-lsp", &[], false),
        fila("lua", &["lua"], &[&["lua-language-server"]], &[".luarc.json", ".luarc.jsonc"], false, "brew install lua-language-server", &[], false),
        fila("elixir", &["ex", "exs", "heex"], &[&["elixir-ls"]], &["mix.exs"], false, "brew install elixir-ls", &["elixir"], false),
        fila("vue", &["vue"], &[&["vue-language-server", "--stdio"]], &["vue.config.js", "nuxt.config.ts", "nuxt.config.js", "src/App.vue"], false, "npm i -g @vue/language-server", &[], true),
        fila("svelte", &["svelte"], &[&["svelteserver", "--stdio"]], &["svelte.config.js", "svelte.config.ts"], false, "npm i -g svelte-language-server", &[], true),
        fila("astro", &["astro"], &[&["astro-ls", "--stdio"]], &["astro.config.mjs", "astro.config.ts", "astro.config.js"], false, "npm i -g @astrojs/language-server", &[], true),
        // sin marcador de raíz: la del proyecto
        fila("toml", &["toml"], &[&["taplo", "lsp", "stdio"]], &[], false, "brew install taplo", &[], false),
        fila("yaml", &["yaml", "yml"], &[&["yaml-language-server", "--stdio"]], &[], false, "npm i -g yaml-language-server", &[], false),
        fila("bash", &["sh", "bash"], &[&["bash-language-server", "start"]], &[], false, "npm i -g bash-language-server", &[], false),
        fila("markdown", &["md", "markdown"], &[&["marksman", "server"]], &[], false, "brew install marksman", &["dotnet"], false),
        fila("json", &["json", "jsonc"], &[&["vscode-json-language-server", "--stdio"]], &[], false, "npm i -g vscode-langservers-extracted", &[], false),
        fila("css", &["css", "scss", "less"], &[&["vscode-css-language-server", "--stdio"]], &[], false, "npm i -g vscode-langservers-extracted", &[], false),
        fila("html", &["html", "htm"], &[&["vscode-html-language-server", "--stdio"]], &[], false, "npm i -g vscode-langservers-extracted", &[], false),
    ]
}

/// ¿Está el marcador `m` en la carpeta `dir`? (`*.ext`: alguna entrada —archivo o carpeta— con esa extensión)
pub fn marca(dir: &Path, m: &str) -> bool {
    match m.strip_prefix("*.") {
        Some(ext) => std::fs::read_dir(dir).into_iter().flatten().flatten().any(|e| e.path().extension().is_some_and(|x| x == ext)),
        None => dir.join(m).is_file(),
    }
}

/// El `languageId` de LSP de un archivo (el servidor lo usa para elegir cómo leerlo).
pub fn id_de_lenguaje(lang: &str, p: &Path) -> &'static str {
    let e = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
    // F10.4: el de un plugin, como lo declaró (`extensionToLanguage`)
    if let Some(l) = lang.starts_with("plugin:").then(|| crate::plugins::lenguaje_lsp(lang, &e)).flatten() {
        return l;
    }
    match (lang, e.as_str()) {
        ("typescript", "tsx") => "typescriptreact",
        ("typescript", "jsx") => "javascriptreact",
        ("typescript", "js" | "mjs" | "cjs") => "javascript",
        ("typescript", _) => "typescript",
        ("c", "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx") => "cpp",
        ("c", "m") => "objective-c",
        ("c", "mm") => "objective-cpp",
        ("css", "scss") => "scss",
        ("css", "less") => "less",
        ("bash", _) => "shellscript",
        ("json", "jsonc") => "jsonc",
        ("csharp", _) => "csharp",
        _ => defs().into_iter().find(|d| d.id == lang).map(|d| d.lang).unwrap_or("plaintext"),
    }
}

/// La definición que atiende este archivo (por su extensión).
pub fn para<'a>(defs: &'a [Def], p: &Path) -> Option<&'a Def> {
    let e = p.extension()?.to_string_lossy().to_lowercase();
    defs.iter().find(|d| d.exts.contains(&e.as_str()))
}

/// Lo que se sabe de qué programas están (ver [`existe`]; [`olvidar`] lo vacía tras instalar).
static CACHE: Mutex<Option<HashMap<String, bool>>> = Mutex::new(None);

/// ¿Está este programa de verdad? (en el PATH del usuario y responde a `--version`: el atajo de rustup
/// existe aunque el componente no). Una ruta absoluta ejecutable vale sin preguntarle. Se recuerda por
/// proceso: preguntar cuesta ~100 ms.
fn existe(bin: &str) -> bool {
    if let Some(v) = CACHE.lock().unwrap().as_ref().and_then(|m| m.get(bin)) {
        return *v;
    }
    let v = if Path::new(bin).is_absolute() {
        Path::new(bin).is_file()
    } else {
        match crate::entorno::donde(bin) {
            None => false,
            // sólo a un ATAJO se le pregunta: `--version`, o `--help` si no lo tiene (sourcekit-lsp sale con 64
            // ante `--version`, medido); el atajo sin la herramienta detrás falla con las dos. A un programa
            // de verdad no: los de pyright (basedpyright-langserver, pyright-langserver) salen con 1 ante
            // las dos porque sólo aceptan `--stdio` (medido 2026-11-16: Python nunca tenía servidor)
            Some(ruta) if es_atajo(&ruta) => {
                let responde = |a: &str| crate::entorno::comando(bin).arg(a).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
                responde("--version") || responde("--help")
            }
            Some(_) => true,
        }
    };
    CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(bin.to_string(), v);
    v
}

/// ¿Es un atajo que puede estar sin la herramienta detrás? Los de rustup (`rust-analyzer` → `rustup`, existe
/// aunque el componente no) y los de las herramientas de Xcode en `/usr/bin` (`clangd`, `sourcekit-lsp`: el
/// mismo binario que llama a `xcrun`, y sin Xcode no hay nada).
fn es_atajo(ruta: &Path) -> bool {
    let real = std::fs::canonicalize(ruta).unwrap_or_else(|_| ruta.to_path_buf());
    real.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("rustup")) || real.starts_with("/usr/bin")
}

/// El comando del primer candidato instalado.
pub fn comando(d: &Def) -> Option<Vec<String>> {
    d.candidatos.iter().find(|c| c.first().is_some_and(|b| existe(b))).cloned().or_else(|| {
        // ML8: el que bajó GOTV (descarga oficial con su suma) a ~/.cache/gotv/lsp
        d.candidatos.iter().find_map(|c| {
            let exe = super::descargas::instalado(c.first()?)?;
            let mut v = c.clone();
            v[0] = exe.to_string_lossy().into_owned();
            Some(v)
        })
    })
}

/// Olvida lo que se sabía de qué está instalado (tras instalar un servidor: el siguiente arranque lo ve).
pub fn olvidar() {
    if let Ok(mut c) = CACHE.lock() {
        *c = None;
    }
}

/// F5 ML8 · el servidor por su nombre: el id del lenguaje (`rust`, `go`) o el programa de un candidato
/// (`rust-analyzer`, `gopls`, `pyright-langserver`, o `pyright` por su paquete).
pub fn por_nombre(nombre: &str) -> Option<Def> {
    let n = nombre.trim().to_lowercase();
    defs().into_iter().find(|d| {
        d.id == n
            || d.candidatos.iter().any(|c| c.first().is_some_and(|b| *b == n || b.strip_suffix("-langserver") == Some(n.as_str())))
            || d.instalar.split_whitespace().last() == Some(n.as_str())
    })
}

/// El instalador oficial de un servidor y el programa que necesita (`rustup`, `npm`, `go`, `brew`). GOTV
/// nunca instala un intérprete para poner un servidor: si falta el instalador, lo dice.
pub fn instalador(d: &Def) -> (&'static str, &'static str) {
    (d.instalar, d.instalar.split_whitespace().next().unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sólo a un atajo (rustup, /usr/bin de Xcode) se le exige responder a `--version`/`--help`; un
    /// servidor que sólo acepta `--stdio` (los de pyright) cuenta con estar en el PATH. Antes, Python
    /// nunca tenía servidor (medido 2026-11-16).
    #[test]
    fn solo_a_los_atajos_se_les_pregunta() {
        let d = crate::CarpetaPrueba::nueva("gotv-atajos");
        let r = d.canonicalize().unwrap();
        std::fs::write(r.join("rustup"), "").unwrap();
        std::os::unix::fs::symlink(r.join("rustup"), r.join("rust-analyzer")).unwrap();
        std::fs::write(r.join("basedpyright-langserver"), "").unwrap();
        assert!(es_atajo(&r.join("rust-analyzer")), "enlace a rustup");
        assert!(es_atajo(Path::new("/usr/bin/clangd")), "atajo de xcrun");
        assert!(!es_atajo(&r.join("basedpyright-langserver")), "programa de verdad");
    }

    /// ML8: el servidor se encuentra por lenguaje, por programa o por paquete, y su instalador oficial
    /// dice qué programa necesita (para no instalar nunca un intérprete por la espalda).
    #[test]
    fn el_servidor_se_encuentra_por_nombre_y_trae_su_instalador() {
        for (n, id) in [("rust", "rust"), ("rust-analyzer", "rust"), ("gopls", "go"), ("pyright", "python"), ("Pyright-LangServer", "python"), ("typescript-language-server", "typescript"), ("intelephense", "php")] {
            assert_eq!(por_nombre(n).map(|d| d.id), Some(id), "{n}");
        }
        assert!(por_nombre("node").is_none() && por_nombre("python3").is_none(), "un intérprete no es un servidor");
        let d = por_nombre("go").unwrap();
        assert_eq!(instalador(&d), ("go install golang.org/x/tools/gopls@latest", "go"));
        assert_eq!(instalador(&por_nombre("rust").unwrap()).1, "rustup");
        // ML8: el runtime que la fórmula arrastraría se exige antes; lo que viene con su SDK no se instala
        assert_eq!(por_nombre("jdtls").unwrap().requiere, &["java"]);
        assert_eq!(instalador(&por_nombre("dart").unwrap()).0, "");
    }

    /// ML4: la tabla cubre los lenguajes del plan sin extensiones repetidas; los marcadores `*.ext`
    /// encuentran carpetas como `App.xcodeproj`; el `languageId` sale de la extensión.
    #[test]
    fn la_tabla_de_lenguajes_del_plan() {
        let ds = defs();
        for id in ["rust", "python", "typescript", "go", "php", "swift", "c", "zig", "dart", "java", "kotlin", "csharp", "ruby", "lua", "elixir", "vue", "svelte", "astro", "toml", "yaml", "bash", "markdown", "json", "css", "html"] {
            assert!(ds.iter().any(|d| d.id == id), "falta {id}");
        }
        let mut vistas = std::collections::BTreeSet::new();
        for d in &ds {
            for e in d.exts {
                assert!(vistas.insert(*e), "la extensión «{e}» la atienden dos servidores");
            }
            assert!(!d.candidatos.is_empty() && d.candidatos.iter().all(|c| !c.is_empty()), "{}", d.id);
        }
        assert_eq!(para(&ds, Path::new("a/b.swift")).map(|d| d.id), Some("swift"));
        assert_eq!(para(&ds, Path::new("x.MM")).map(|d| d.id), Some("c"));
        let d = crate::CarpetaPrueba::nueva("gotv-lsp-marca");
        std::fs::create_dir_all(d.join("App.xcodeproj")).unwrap();
        assert!(marca(&d, "*.xcodeproj") && !marca(&d, "*.sln") && !marca(&d, "Package.swift"));
        for (lang, f, id) in [("typescript", "a.tsx", "typescriptreact"), ("typescript", "a.js", "javascript"), ("typescript", "a.ts", "typescript"), ("c", "a.cpp", "cpp"), ("c", "a.m", "objective-c"), ("c", "a.c", "c"), ("css", "a.scss", "scss"), ("bash", "a.sh", "shellscript"), ("swift", "a.swift", "swift")] {
            assert_eq!(id_de_lenguaje(lang, Path::new(f)), id, "{f}");
        }
    }
}
