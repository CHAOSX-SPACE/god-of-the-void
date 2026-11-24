//! F3 paso 7 · `process` gana `reiniciar: "siempre" | "si-falla"` para procesos de fondo que deben seguir
//! vivos (un servidor de desarrollo, un vigilante de compilación). Un supervisor espera a que el proceso
//! caiga y lo vuelve a lanzar IGUAL (orden, carpeta, caja, terminal), con espera creciente y un tope de
//! reinicios en una ventana: un proceso que cae en bucle no se relanza para siempre. Cada reinicio llega al
//! chat por la cola de vigilancia (como un aviso, sin que nadie pregunte).
//!
//! Parar el proceso a propósito (`process stop`) o cerrar el chat apaga el supervisor ANTES de matar.
use anyhow::{anyhow, bail, Result};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Reinicios permitidos dentro de [`VENTANA`]; uno más y se rinde (y lo dice).
pub const MAX_REINICIOS: usize = 5;
pub const VENTANA: Duration = Duration::from_secs(600);
/// Espera antes del reinicio n: `ESPERA_BASE * n` (1.º medio segundo, 2.º un segundo…).
const ESPERA_BASE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Politica {
    Siempre,
    SiFalla,
}

impl Politica {
    pub fn de(s: &str) -> Option<Politica> {
        match s.trim() {
            "siempre" | "always" => Some(Politica::Siempre),
            "si-falla" | "si_falla" | "on-failure" => Some(Politica::SiFalla),
            _ => None,
        }
    }
}

/// id del proceso supervisado → (dueño, política). Quitar la entrada apaga al supervisor.
fn supervisados() -> &'static Mutex<HashMap<u32, (String, Politica)>> {
    static S: OnceLock<Mutex<HashMap<u32, (String, Politica)>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

fn vigente(id: u32) -> Option<Politica> {
    supervisados().lock().unwrap().get(&id).map(|x| x.1)
}

/// `process {action: "reiniciar", id, reiniciar: "siempre" | "si-falla" | "no"}`.
pub fn poner(dueno: &str, id: u32, politica: &str) -> Result<String> {
    let p = crate::exec::get(dueno, id).ok_or_else(|| anyhow!("{}", crate::tr!("no existe el proceso #{id} (mira action=list)", "process #{id} does not exist (see action=list)")))?;
    if matches!(politica.trim(), "no" | "nunca" | "never") {
        quitar(id);
        return Ok(crate::tr!("proceso #{id}: ya no se reinicia solo", "process #{id}: no longer restarts by itself"));
    }
    let Some(pol) = Politica::de(politica) else {
        bail!("{}", crate::i18n::t("`reiniciar` es «siempre», «si-falla» o «no»", "`reiniciar` is «siempre», «si-falla» or «no»"));
    };
    if !p.vigente() {
        bail!("{}", crate::tr!("el proceso #{id} ya terminó: lánzalo de nuevo y luego ponle `reiniciar`", "process #{id} already ended: launch it again and then set `reiniciar`"));
    }
    let nuevo = supervisados().lock().unwrap().insert(id, (dueno.to_string(), pol)).is_none();
    if nuevo {
        tokio::spawn(supervisar(dueno.to_string(), p));
    }
    Ok(match pol {
        Politica::Siempre => crate::tr!("proceso #{id}: si termina, GOTV lo relanza (hasta {MAX_REINICIOS} veces en 10 min) y te avisa", "process #{id}: if it ends, GOTV relaunches it (up to {MAX_REINICIOS} times in 10 min) and tells you"),
        Politica::SiFalla => crate::tr!("proceso #{id}: si cae con error, GOTV lo relanza (hasta {MAX_REINICIOS} veces en 10 min) y te avisa", "process #{id}: if it crashes, GOTV relaunches it (up to {MAX_REINICIOS} times in 10 min) and tells you"),
    })
}

/// Lo para a propósito: el supervisor se apaga (lo llama `process stop` ANTES de parar).
pub fn quitar(id: u32) {
    supervisados().lock().unwrap().remove(&id);
}

/// Al cerrar el chat (cola de la clave del dueño): ningún supervisor relanza en un chat muerto.
pub fn soltar_chat(cola: &str) {
    supervisados().lock().unwrap().retain(|_, (d, _)| !d.ends_with(cola));
}

async fn supervisar(dueno: String, mut p: std::sync::Arc<crate::exec::Proc>) {
    let mut reinicios: Vec<Instant> = Vec::new();
    loop {
        // espera a que caiga (o a que lo suelten)
        while p.vigente() {
            if vigente(p.id).is_none() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let Some(pol) = vigente(p.id) else { return };
        let como = p.exit_label().unwrap_or_default();
        if pol == Politica::SiFalla && p.exit_code() == Some(0) {
            quitar(p.id);
            super::avisar_chat(&dueno, &crate::tr!("[process #{}] terminó bien ({como}): con «si-falla» no se relanza", "[process #{}] ended fine ({como}): with «si-falla» it is not relaunched", p.id));
            return;
        }
        let ahora = Instant::now();
        reinicios.retain(|t| ahora.duration_since(*t) < VENTANA);
        if reinicios.len() >= MAX_REINICIOS {
            quitar(p.id);
            super::avisar_chat(&dueno, &crate::tr!(
                "[process #{}] cayó otra vez ({como}) tras {MAX_REINICIOS} reinicios en 10 min: GOTV deja de relanzarlo. Mira su salida con `process` y arregla la causa",
                "[process #{}] crashed again ({como}) after {MAX_REINICIOS} restarts in 10 min: GOTV stops relaunching it. Look at its output with `process` and fix the cause",
                p.id
            ));
            return;
        }
        reinicios.push(ahora);
        tokio::time::sleep(ESPERA_BASE * reinicios.len() as u32).await;
        // ¿lo soltaron mientras esperaba?
        if vigente(p.id).is_none() {
            return;
        }
        match crate::exec::spawn(&dueno, &p.cmd, &p.cwd, p.tty, Some(&p.name), !p.sandboxed) {
            Ok(nuevo) => {
                quitar(p.id);
                supervisados().lock().unwrap().insert(nuevo.id, (dueno.clone(), pol));
                super::avisar_chat(&dueno, &crate::tr!(
                    "[process #{} → #{}] «{}» cayó ({como}) y GOTV lo relanzó ({}/{MAX_REINICIOS})",
                    "[process #{} → #{}] «{}» went down ({como}) and GOTV relaunched it ({}/{MAX_REINICIOS})",
                    p.id,
                    nuevo.id,
                    p.name,
                    reinicios.len()
                ));
                p = nuevo;
            }
            Err(e) => {
                quitar(p.id);
                super::avisar_chat(&dueno, &crate::tr!("[process #{}] cayó ({como}) y no pude relanzarlo: {e}", "[process #{}] went down ({como}) and I could not relaunch it: {e}", p.id));
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{dueno, esperar_evento, falso};
    use super::*;

    #[tokio::test]
    async fn un_proceso_con_reiniciar_que_cae_dos_veces() {
        let raiz = crate::CarpetaPrueba::nueva("gotv-vigilar-reiniciar");
        let d = dueno("reiniciar");
        // cae (exit 1) las dos primeras veces; a la tercera se queda vivo
        let cmd = falso(&format!("d|300;c|{}|2|1;e|vivo;d|30000", raiz.join("cuenta").display()));
        let p = crate::exec::spawn(&d, &cmd, &raiz, false, Some("servidor"), true).unwrap();
        assert!(poner(&d, p.id, "si-falla").unwrap().contains(&format!("#{}", p.id)));
        let mut avisos = String::new();
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(10) && avisos.matches("→ #").count() < 2 {
            if let Some(t) = esperar_evento(&d, Duration::from_secs(1)).await {
                avisos.push_str(&t);
            }
        }
        assert_eq!(avisos.matches("→ #").count(), 2, "{avisos}");
        assert!(avisos.contains("(1/5)") && avisos.contains("(2/5)") && avisos.contains("exit 1"), "{avisos}");
        // el tercero tarda en arrancar y contar (≈0,3 s + el arranque): se espera a que cuente
        let t1 = Instant::now();
        while t1.elapsed() < Duration::from_secs(5) && std::fs::read_to_string(raiz.join("cuenta")).unwrap_or_default().trim() != "3" {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(std::fs::read_to_string(raiz.join("cuenta")).unwrap().trim(), "3");
        let vivos: Vec<_> = crate::exec::list(&d).into_iter().filter(|p| p.vigente()).collect();
        assert_eq!(vivos.len(), 1, "queda uno, el tercero, vivo");
        let ultimo = vivos[0].clone();
        assert_eq!(vigente(ultimo.id), Some(Politica::SiFalla), "la política pasa al relanzado");
        // pararlo a propósito: NO se relanza
        quitar(ultimo.id);
        ultimo.stop().await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(crate::exec::list(&d).iter().all(|p| !p.vigente()), "parado a propósito no vuelve");
        crate::exec::kill_owner(&d);
        super::super::soltar_chat(&d);
    }

    #[tokio::test]
    async fn si_falla_no_relanza_lo_que_termina_bien_y_el_tope_se_rinde() {
        let raiz = crate::CarpetaPrueba::nueva("gotv-vigilar-reiniciar-2");
        let d = dueno("reiniciar-bien");
        let p = crate::exec::spawn(&d, &falso("d|400;e|listo"), &raiz, false, None, true).unwrap();
        poner(&d, p.id, "si-falla").unwrap();
        let t = esperar_evento(&d, Duration::from_secs(5)).await.unwrap();
        assert!(t.contains("terminó bien") || t.contains("ended fine"), "{t}");
        assert!(poner(&d, p.id, "a veces").is_err());
        // «siempre» con algo que cae al instante: se rinde tras el tope (la ventana real es de 10 min)
        let p = crate::exec::spawn(&d, &falso("d|200;x|3"), &raiz, false, None, true).unwrap();
        poner(&d, p.id, "siempre").unwrap();
        let mut todo = String::new();
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(20) && !(todo.contains("deja de") || todo.contains("stops relaunching")) {
            if let Some(t) = esperar_evento(&d, Duration::from_secs(1)).await {
                todo.push_str(&t);
            }
        }
        assert_eq!(todo.matches("→ #").count(), MAX_REINICIOS, "{todo}");
        assert!(todo.contains("deja de") || todo.contains("stops relaunching"), "{todo}");
        crate::exec::kill_owner(&d);
        super::super::soltar_chat(&d);
    }
}
