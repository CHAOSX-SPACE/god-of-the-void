//! F14.3 · Secretos fuera del contexto de NUBE. Con un motor de nube, lo que devuelven las herramientas
//! viaja al proveedor: una llave leída de un `.env` o impresa por un comando saldría de la Mac. Aquí se
//! reconoce (formas de llave conocidas y los valores del `.env` del proyecto) y se cambia por
//! `«secreto:NOMBRE»` ANTES de mandarlo. Con un motor local no se toca nada (no sale de la Mac).
//! El historial de la sesión guarda el texto real; sólo la petición al proveedor va enmascarada.
use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

/// El aviso de una línea se da una vez por proceso.
pub static AVISADO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Las formas de llave reconocibles por sí solas: (nombre, patrón).
static FORMAS: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    [
        // bloque PEM completo (también una llave SSH OpenSSH)
        ("pem", r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z0-9 ]*PRIVATE KEY-----"),
        ("anthropic", r"\bsk-ant-[A-Za-z0-9_\-]{20,}"),
        ("openai", r"\bsk-(?:proj-)?[A-Za-z0-9_\-]{20,}"),
        ("github", r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{30,}\b|\bgithub_pat_[A-Za-z0-9_]{40,}"),
        ("aws", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
        ("google", r"\bAIza[0-9A-Za-z_\-]{35}\b"),
        ("slack", r"\bxox[abprs]-[A-Za-z0-9\-]{10,}"),
        ("huggingface", r"\bhf_[A-Za-z0-9]{30,}\b"),
        ("stripe", r"\b(?:sk|rk)_(?:live|test)_[A-Za-z0-9]{20,}"),
        ("jwt", r"\beyJ[A-Za-z0-9_\-]{8,}\.eyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}"),
    ]
    .into_iter()
    .map(|(n, p)| (n, Regex::new(p).unwrap()))
    .collect()
});

/// Cambia cada secreto del texto por `«secreto:NOMBRE»`. `conocidos` son (nombre, valor) del `.env`;
/// se reemplazan primero los más largos (un valor puede contener a otro). Devuelve el texto y cuántos
/// cambió.
pub fn enmascarar(texto: &str, conocidos: &[(String, String)]) -> (String, usize) {
    let mut t = texto.to_string();
    let mut n = 0;
    let mut orden: Vec<&(String, String)> = conocidos.iter().filter(|(_, v)| v.chars().count() >= 8).collect();
    orden.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
    for (nombre, valor) in orden {
        let c = t.matches(valor.as_str()).count();
        if c > 0 {
            n += c;
            t = t.replace(valor.as_str(), &format!("«secreto:{nombre}»"));
        }
    }
    for (nombre, re) in FORMAS.iter() {
        let c = re.find_iter(&t).count();
        if c > 0 {
            n += c;
            t = re.replace_all(&t, format!("«secreto:{nombre}»").as_str()).into_owned();
        }
    }
    (t, n)
}

/// Los valores del `.env` del proyecto (y sus variantes `.env.local`, `.env.development`…): `CLAVE=valor`,
/// con o sin comillas y con `export`. Sólo valores de 8 caracteres o más (los cortos —`true`, `3000`—
/// darían falsos positivos por todas partes).
pub fn del_proyecto(raiz: &Path) -> Vec<(String, String)> {
    let mut v = Vec::new();
    let Ok(dir) = std::fs::read_dir(raiz) else { return v };
    let mut archivos: Vec<_> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n == ".env" || (n.starts_with(".env.") && !n.ends_with(".example") && !n.ends_with(".sample") && !n.ends_with(".template"))))
        .collect();
    archivos.sort();
    for a in archivos {
        let Ok(texto) = std::fs::read_to_string(&a) else { continue };
        for l in texto.lines() {
            let l = l.trim();
            if l.starts_with('#') {
                continue;
            }
            let l = l.strip_prefix("export ").unwrap_or(l);
            let Some((k, val)) = l.split_once('=') else { continue };
            let val = val.trim().trim_matches(|c| c == '"' || c == '\'');
            if val.chars().count() >= 8 && !k.trim().is_empty() && !v.iter().any(|(_, x): &(String, String)| x == val) {
                v.push((k.trim().to_string(), val.to_string()));
            }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cada forma de llave se reconoce y su valor desaparece.
    #[test]
    fn cada_forma_de_llave() {
        for (texto, nombre) in [
            ("OPENAI=sk-proj-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789", "openai"),
            ("token: sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWx", "anthropic"),
            ("ghp_0123456789abcdefghijABCDEFGHIJ012345", "github"),
            ("github_pat_11ABCDEFG0123456789_abcdefghijklmnopqrstuvwxyzABCDEF", "github"),
            ("aws_access_key_id = AKIAIOSFODNN7EXAMPLE", "aws"),
            ("key=AIzaSyA-0123456789abcdefghijklmnopqrstu", "google"),
            ("xoxb-123456789012-abcdefghijkl", "slack"),
            ("HF_TOKEN=hf_abcdefghijklmnopqrstuvwxyzABCDEFG", "huggingface"),
            // de mentira, armada en dos trozos: escrita de corrido, el escáner de secretos de GitHub la toma por real
            (concat!("sk_", "live_0123456789abcdefghijklmn"), "stripe"),
            ("Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U", "jwt"),
            ("-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\nAAAA\n-----END OPENSSH PRIVATE KEY-----", "pem"),
        ] {
            let (t, n) = enmascarar(texto, &[]);
            assert!(n >= 1 && t.contains(&format!("«secreto:{nombre}»")), "{texto} → {t}");
        }
        let (t, _) = enmascarar("-----BEGIN RSA PRIVATE KEY-----\nMIIEow\n-----END RSA PRIVATE KEY-----", &[]);
        assert!(!t.contains("MIIEow"), "{t}");
    }

    /// Los valores del `.env` (8+ caracteres) se cambian por su NOMBRE; los cortos y los comentarios no.
    #[test]
    fn valores_del_env() {
        let d = crate::CarpetaPrueba::nueva("gotv-secretos-env");
        std::fs::write(d.join(".env"), "# comentario = no-es-secreto-12345\nDB_PASSWORD=\"correcaballo-grapa-77\"\nexport SESSION_KEY=x9!z8@y7#w6$\nPORT=3000\nDEBUG=true\n").unwrap();
        std::fs::write(d.join(".env.example"), "DB_PASSWORD=pon-aqui-tu-clave\n").unwrap();
        let c = del_proyecto(&d);
        assert_eq!(c.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["DB_PASSWORD", "SESSION_KEY"], "{c:?}");
        let (t, n) = enmascarar("DB_PASSWORD=\"correcaballo-grapa-77\"\nSESSION_KEY=x9!z8@y7#w6$\nPORT=3000", &c);
        assert_eq!(n, 2);
        assert!(t.contains("«secreto:DB_PASSWORD»") && t.contains("«secreto:SESSION_KEY»") && t.contains("PORT=3000"), "{t}");
        assert!(!t.contains("correcaballo") && !t.contains("x9!z8"), "{t}");
    }

    /// Falsos positivos conocidos: un hash de git, un UUID, una ruta o un sha256 no son secretos.
    #[test]
    fn lo_que_no_es_secreto_queda_igual() {
        for texto in [
            "commit 3c3fb89d1e2f4a5b6c7d8e9f0a1b2c3d4e5f6a7b",
            "id 123e4567-e89b-12d3-a456-426614174000",
            "sha256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "src/skeleton/sk-components.ts",
            "el token expira en 8 horas",
        ] {
            assert_eq!(enmascarar(texto, &[]), (texto.to_string(), 0), "{texto}");
        }
    }
}
