//! Ola EXPERTO · EXP-0 — la SESIÓN del estudio de música de un proyecto y su CANDADO.
//!
//! Pedido del Portador (2026-11-13): un estudio «Experto» dentro del proyecto, que ocupa toda la
//! ventana, se minimiza sobre el chat y, mientras está activo en ese proyecto, BLOQUEA el chat y el
//! panel Pro («para no dañar proyectos de música muy buenos»).
//!
//! Tres reglas:
//! - **No destructiva**: la sesión es una LISTA DE OPERACIONES sobre archivos que nunca se tocan; el
//!   estado (pistas, mezcla, grupos, regiones) se obtiene repitiéndolas. Deshacer y rehacer son mover
//!   la última operación entre dos pilas; nada se pierde.
//! - **Validada**: una operación que no cuadra con el estado (una pista que no existe, una región al
//!   revés, un grupo inexistente) se rechaza y NO entra en la lista.
//! - **Por proyecto**: vive en `<proyecto>/.gotv/experto/sesion.gotv` (formato propio); el candado es su `activa`, que
//!   leen la app, la CLI y el núcleo (`Agent::run` se niega con el candado puesto).
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Donde vive la sesión, relativa a la raíz del proyecto.
/// Formato propio (códec, EXPS); hasta 2026-11-17 era `sesion.json` (se migra sola: el trabajo no se pierde).
pub const ARCHIVO: &str = ".gotv/experto/sesion.gotv";
const VERSION: u32 = 1;

/// De dónde salió una pista.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Origen {
    /// la generó GOTV (ACE-Step)
    Generada,
    /// salió de separar otra pista (HTDemucs): `de` dice cuál
    Separada,
    /// un audio propio de la persona
    Importada,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Pista {
    pub id: u32,
    pub nombre: String,
    /// el WAV, relativo a la raíz del proyecto (nunca se modifica)
    pub archivo: String,
    pub origen: Origen,
    /// la pista madre, si salió de separarla
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub de: Option<u32>,
    /// la clave de la fuente separada (`vocals`, `drums`…), si la tiene
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuente: Option<String>,
    #[serde(default)]
    pub ganancia_db: f32,
    /// −1 (izquierda) … 1 (derecha)
    #[serde(default)]
    pub pan: f32,
    #[serde(default)]
    pub silencio: bool,
    #[serde(default)]
    pub solo: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grupo: Option<u32>,
    /// los efectos de la pista (ver `Insertos`), tras sus clips y sus regiones
    #[serde(default, skip_serializing_if = "Insertos::vacio")]
    pub insertos: Insertos,
    /// el color de la pista en la interfaz (`#rrggbb`); sin él, el del tema
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// X2-1 · Un CLIP: un trozo de un audio colocado en la línea de tiempo de una pista. La pista ES la
/// lista de sus clips: el que va después en la lista tapa al anterior donde se pisan (con sus fundidos).
/// Una pista recién agregada tiene un solo clip con todo su audio desde el segundo 0.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Clip {
    pub id: u32,
    pub pista: u32,
    /// el WAV, relativo a la raíz del proyecto (nunca se modifica)
    pub archivo: String,
    /// dónde empieza en la línea de tiempo (s)
    pub inicio: f64,
    /// desde qué segundo del archivo
    #[serde(default)]
    pub desde: f64,
    /// cuánto dura EN LA LÍNEA DE TIEMPO (s); sin ella, hasta donde llegue el archivo
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duracion: Option<f64>,
    #[serde(default)]
    pub ganancia_db: f32,
    /// fundidos de igual potencia (s)
    #[serde(default)]
    pub fundido_entrada: f32,
    #[serde(default)]
    pub fundido_salida: f32,
    /// transponer el clip (semitonos, −12…12) sin cambiar su duración
    #[serde(default)]
    pub tono: f32,
    /// velocidad sin cambiar el tono (0,25…4; 2 = el doble de rápido, la mitad de largo)
    #[serde(default = "uno")] // juez-idioma: datos (nombre de función)
    pub ritmo: f32,
    /// suena al revés
    #[serde(default)]
    pub invertido: bool,
    #[serde(default)]
    pub silencio: bool,
    /// nombre propio del clip (vacío = el de la pista)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub nombre: String,
}

fn uno() -> f32 {
    1.0
}

impl Clip {
    /// El clip que ocupa una pista recién agregada: todo su archivo desde el segundo 0.
    pub fn entero(id: u32, pista: u32, archivo: &str, duracion: Option<f64>) -> Clip {
        Clip { id, pista, archivo: archivo.into(), inicio: 0.0, desde: 0.0, duracion, ganancia_db: 0.0, fundido_entrada: 0.0, fundido_salida: 0.0, tono: 0.0, ritmo: 1.0, invertido: false, silencio: false, nombre: String::new() }
    }

    /// ¿Es el archivo tal cual (sin mover, recortar ni transformar)? Entonces la pista es su archivo.
    pub fn intacto(&self) -> bool {
        self.inicio == 0.0 && self.desde == 0.0 && self.ganancia_db == 0.0 && self.fundido_entrada == 0.0 && self.fundido_salida == 0.0 && self.tono == 0.0 && self.ritmo == 1.0 && !self.invertido && !self.silencio
    }

    pub fn fin(&self) -> Option<f64> {
        self.duracion.map(|d| self.inicio + d)
    }

    fn validar(&self) -> Result<()> {
        let ok = self.inicio.is_finite()
            && self.inicio >= 0.0
            && self.desde.is_finite()
            && self.desde >= 0.0
            && self.duracion.is_none_or(|d| d.is_finite() && d > 0.0)
            && self.ganancia_db.is_finite()
            && (-60.0..=24.0).contains(&self.ganancia_db)
            && self.fundido_entrada.is_finite()
            && self.fundido_entrada >= 0.0
            && self.fundido_salida.is_finite()
            && self.fundido_salida >= 0.0
            && self.tono.is_finite()
            && (-12.0..=12.0).contains(&self.tono)
            && self.ritmo.is_finite()
            && (0.25..=4.0).contains(&self.ritmo);
        if !ok {
            return Err(gotv_engine::anyhow_tr!(
                "clip: inicio y recorte ≥ 0, duración > 0, ganancia −60…+24 dB, fundidos ≥ 0, tono −12…+12 semitonos y ritmo 0,25…4",
                "clip: start and offset ≥ 0, length > 0, gain −60…+24 dB, fades ≥ 0, pitch −12…+12 semitones and speed 0.25…4"
            ));
        }
        if self.archivo.trim().is_empty() {
            return Err(gotv_engine::anyhow_tr!("el clip necesita su archivo", "the clip needs its file"));
        }
        Ok(())
    }
}

/// Una banda del EQ paramétrico del canal.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Banda {
    pub filtro: Filtro,
    pub frecuencia: f32,
    #[serde(default)]
    pub db: f32,
    #[serde(default = "q")]
    pub q: f32,
    #[serde(default)]
    pub apagada: bool,
}

/// Limitador de pared con anticipación: NINGUNA muestra pasa del techo.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Limitador {
    /// −24 … 0 dBFS
    pub techo_db: f32,
    /// ganancia de entrada (0 … 24 dB): cuánto se empuja contra el techo
    #[serde(default)]
    pub empuje_db: f32,
    pub liberacion_ms: f32,
}

/// Saturación (tangente hiperbólica): armónicos y cuerpo.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Saturacion {
    /// 0 … 36 dB
    pub empuje_db: f32,
    /// 0 (seco) … 1
    pub mezcla: f32,
}

/// De-esser: atenúa los agudos (por encima de `frecuencia`) sólo cuando superan el umbral.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct DeEsser {
    pub frecuencia: f32,
    pub umbral_db: f32,
    /// la atenuación máxima (0 … 24 dB)
    pub reduccion_db: f32,
}

/// Chorus estéreo: un retardo modulado, con los dos canales en cuadratura.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Chorus {
    /// 0,05 … 8 Hz
    pub velocidad_hz: f32,
    /// 0,5 … 20 ms
    pub profundidad_ms: f32,
    pub mezcla: f32,
}

/// El bus MAESTRO: lo último antes de salir (y de exportar la mezcla).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Maestro {
    #[serde(default)]
    pub ganancia_db: f32,
    #[serde(default, skip_serializing_if = "Insertos::vacio")]
    pub insertos: Insertos,
}

/// Lo que se automatiza de una pista.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Parametro {
    /// dB que se SUMAN al fader
    Volumen,
    /// −1 … 1: reemplaza al paneo de la pista
    Pan,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Punto {
    /// segundo
    pub t: f64,
    pub v: f32,
}

/// Una curva de automatización: entre puntos se interpola en línea recta; antes del primero y después
/// del último vale lo que ellos.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Automatizacion {
    pub pista: u32,
    pub parametro: Parametro,
    pub puntos: Vec<Punto>,
}

impl Automatizacion {
    /// El valor de la curva en el segundo `t` (con `puntos` ordenados y no vacíos).
    pub fn valor(&self, t: f64) -> f32 {
        let p = &self.puntos;
        match p.iter().position(|x| x.t > t) {
            Some(0) => p[0].v,
            None => p.last().map(|x| x.v).unwrap_or(0.0),
            Some(i) => {
                let (a, b) = (p[i - 1], p[i]);
                let f = if b.t > a.t { ((t - a.t) / (b.t - a.t)) as f32 } else { 1.0 };
                a.v + (b.v - a.v) * f
            }
        }
    }
}

/// Compresor (detector de pico enlazado en estéreo, rodilla dura, ataque y liberación en dB).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Compresor {
    pub umbral_db: f32,
    /// 1 … 20 (20 ≈ limitador)
    pub razon: f32,
    pub ataque_ms: f32,
    pub liberacion_ms: f32,
    /// ganancia de compensación
    #[serde(default)]
    pub ganancia_db: f32,
}

/// Puerta de ruido: por debajo del umbral, silencio (con rampas).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Puerta {
    pub umbral_db: f32,
    pub ataque_ms: f32,
    pub liberacion_ms: f32,
}

/// Eco estéreo con realimentación.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Delay {
    pub ms: f32,
    /// 0 … 0,95
    pub realimentacion: f32,
    /// 0 (seco) … 1 (sólo el eco)
    pub mezcla: f32,
}

/// Reverberación (Freeverb de Jezar, de dominio público: 8 peines + 4 pasatodos por canal).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Reverb {
    /// 0 … 1 (el tamaño de la sala)
    pub tamano: f32,
    /// 0 … 1 (cuánto se apagan los agudos)
    pub amortiguacion: f32,
    /// 0 (seco) … 1 (sólo la cola)
    pub mezcla: f32,
}

/// Los efectos de un canal (pista, bus o maestro); los que no están, no se aplican. El orden es fijo:
/// puerta → EQ → compresor → de-esser → saturación → chorus → delay → reverb → ancho → limitador.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Insertos {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub puerta: Option<Puerta>,
    /// el EQ paramétrico (hasta 8 bandas)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub eq: Vec<Banda>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compresor: Option<Compresor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deesser: Option<DeEsser>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saturacion: Option<Saturacion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chorus: Option<Chorus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<Delay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverb: Option<Reverb>,
    /// el ancho estéreo (0 = mono, 1 = igual, 2 = el doble de lado)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ancho: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limitador: Option<Limitador>,
}

impl Insertos {
    pub fn vacio(&self) -> bool {
        self == &Insertos::default()
    }

    fn validar(&self) -> Result<()> {
        let rango = |v: f32, a: f32, b: f32| v.is_finite() && (a..=b).contains(&v);
        if let Some(c) = self.compresor {
            if !(rango(c.umbral_db, -60.0, 0.0) && rango(c.razon, 1.0, 20.0) && rango(c.ataque_ms, 0.1, 500.0) && rango(c.liberacion_ms, 5.0, 3000.0) && rango(c.ganancia_db, -12.0, 24.0)) {
                return Err(gotv_engine::anyhow_tr!(
                    "compresor: umbral −60…0 dB, razón 1…20, ataque 0,1…500 ms, liberación 5…3000 ms, compensación −12…+24 dB",
                    "compressor: threshold −60…0 dB, ratio 1…20, attack 0.1…500 ms, release 5…3000 ms, makeup −12…+24 dB"
                ));
            }
        }
        if let Some(p) = self.puerta {
            if !(rango(p.umbral_db, -90.0, 0.0) && rango(p.ataque_ms, 0.1, 500.0) && rango(p.liberacion_ms, 5.0, 3000.0)) {
                return Err(gotv_engine::anyhow_tr!("puerta: umbral −90…0 dB, ataque 0,1…500 ms, liberación 5…3000 ms", "gate: threshold −90…0 dB, attack 0.1…500 ms, release 5…3000 ms"));
            }
        }
        if let Some(d) = self.delay {
            if !(rango(d.ms, 1.0, 4000.0) && rango(d.realimentacion, 0.0, 0.95) && rango(d.mezcla, 0.0, 1.0)) {
                return Err(gotv_engine::anyhow_tr!("delay: 1…4000 ms, realimentación 0…0,95, mezcla 0…1", "delay: 1…4000 ms, feedback 0…0.95, mix 0…1"));
            }
        }
        if let Some(r) = self.reverb {
            if !(rango(r.tamano, 0.0, 1.0) && rango(r.amortiguacion, 0.0, 1.0) && rango(r.mezcla, 0.0, 1.0)) {
                return Err(gotv_engine::anyhow_tr!("reverb: tamaño, amortiguación y mezcla van de 0 a 1", "reverb: size, damping and mix go from 0 to 1"));
            }
        }
        if self.eq.len() > 8 || self.eq.iter().any(|b| !(rango(b.frecuencia, 20.0, 20_000.0) && rango(b.db, -24.0, 24.0) && rango(b.q, 0.1, 18.0))) {
            return Err(gotv_engine::anyhow_tr!("EQ: hasta 8 bandas, de 20 Hz a 20 kHz, ±24 dB y Q de 0,1 a 18", "EQ: up to 8 bands, 20 Hz to 20 kHz, ±24 dB and Q from 0.1 to 18"));
        }
        if let Some(l) = self.limitador {
            if !(rango(l.techo_db, -24.0, 0.0) && rango(l.empuje_db, 0.0, 24.0) && rango(l.liberacion_ms, 1.0, 2000.0)) {
                return Err(gotv_engine::anyhow_tr!("limitador: techo −24…0 dB, empuje 0…24 dB, liberación 1…2000 ms", "limiter: ceiling −24…0 dB, drive 0…24 dB, release 1…2000 ms"));
            }
        }
        if let Some(s) = self.saturacion {
            if !(rango(s.empuje_db, 0.0, 36.0) && rango(s.mezcla, 0.0, 1.0)) {
                return Err(gotv_engine::anyhow_tr!("saturación: empuje 0…36 dB y mezcla 0…1", "saturation: drive 0…36 dB and mix 0…1"));
            }
        }
        if let Some(d) = self.deesser {
            if !(rango(d.frecuencia, 1000.0, 16_000.0) && rango(d.umbral_db, -80.0, 0.0) && rango(d.reduccion_db, 0.0, 24.0)) {
                return Err(gotv_engine::anyhow_tr!("de-esser: 1…16 kHz, umbral −80…0 dB, reducción 0…24 dB", "de-esser: 1…16 kHz, threshold −80…0 dB, reduction 0…24 dB"));
            }
        }
        if let Some(c) = self.chorus {
            if !(rango(c.velocidad_hz, 0.05, 8.0) && rango(c.profundidad_ms, 0.5, 20.0) && rango(c.mezcla, 0.0, 1.0)) {
                return Err(gotv_engine::anyhow_tr!("chorus: 0,05…8 Hz, 0,5…20 ms y mezcla 0…1", "chorus: 0.05…8 Hz, 0.5…20 ms and mix 0…1"));
            }
        }
        if self.ancho.is_some_and(|a| !rango(a, 0.0, 2.0)) {
            return Err(gotv_engine::anyhow_tr!("el ancho estéreo va de 0 (mono) a 2", "stereo width goes from 0 (mono) to 2"));
        }
        Ok(())
    }
}

/// Un grupo (BUS): «Batería» = bombo + caja + platos. Sus pistas se suman, pasan por los insertos del
/// bus y luego por su fader y su paneo.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Grupo {
    pub id: u32,
    pub nombre: String,
    #[serde(default)]
    pub ganancia_db: f32,
    #[serde(default)]
    pub pan: f32,
    #[serde(default)]
    pub silencio: bool,
    #[serde(default)]
    pub solo: bool,
    #[serde(default, skip_serializing_if = "Insertos::vacio")]
    pub insertos: Insertos,
}

/// Los filtros de la EQ (las fórmulas RBJ, las mismas de Web Audio: EXP-2).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Filtro {
    Graves,
    Agudos,
    Campana,
    PasoAlto,
    PasoBajo,
    /// quita una frecuencia (`notch`)
    Muesca,
    /// deja sólo una banda (`bandpass`)
    PasoBanda,
}

/// Lo que se le hace a un tramo `[desde, hasta]` de una pista.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum Efecto {
    /// más fuerte o más suave, con fundidos de `fundido` segundos en los bordes
    Ganancia { db: f32, #[serde(default = "fundido")] fundido: f32 },
    Silencio { #[serde(default = "fundido")] fundido: f32 },
    /// «más agudo», «más grave»: una banda de EQ sólo en ese tramo
    Eq { filtro: Filtro, frecuencia: f32, #[serde(default)] db: f32, #[serde(default = "q")] q: f32, #[serde(default = "fundido")] fundido: f32 },
    /// el tramo se cambia por OTRO audio (lo generativo, EXP-4): «otro tipo de batería»
    Reemplazo { archivo: String, #[serde(default)] descripcion: String, #[serde(default = "fundido")] fundido: f32 },
    /// «más agudo» o «más grave» de TONO: transponer el tramo `semitonos` (−12…12) sin cambiar su duración
    Tono { semitonos: f32, #[serde(default = "fundido")] fundido: f32 },
}

fn fundido() -> f32 {
    0.03
}

fn q() -> f32 {
    0.707
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Region {
    pub id: u32,
    pub pista: u32,
    pub desde: f64,
    pub hasta: f64,
    pub efecto: Efecto,
    /// apagada: sigue en la sesión pero no suena (para comparar A/B una toma o un efecto)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub apagada: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Marcador {
    pub segundo: f64,
    pub nombre: String,
}

/// Una operación de la lista. Las que crean algo traen su `id` ya asignado (`Sesion::nuevo_id`), para
/// que repetirlas dé siempre lo mismo.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    AgregarPista { pista: Pista },
    QuitarPista { id: u32 },
    Renombrar { pista: u32, nombre: String },
    /// la mezcla de una pista (lo que no viene, no cambia)
    Mezcla {
        pista: u32,
        #[serde(default)]
        ganancia_db: Option<f32>,
        #[serde(default)]
        pan: Option<f32>,
        #[serde(default)]
        silencio: Option<bool>,
        #[serde(default)]
        solo: Option<bool>,
    },
    CrearGrupo { grupo: Grupo },
    QuitarGrupo { id: u32 },
    /// mete esas pistas en el grupo (o las saca, con `grupo` vacío)
    Agrupar { pistas: Vec<u32>, #[serde(default)] grupo: Option<u32> },
    MezclaGrupo {
        grupo: u32,
        #[serde(default)]
        ganancia_db: Option<f32>,
        #[serde(default)]
        pan: Option<f32>,
        #[serde(default)]
        silencio: Option<bool>,
        #[serde(default)]
        solo: Option<bool>,
    },
    Region { region: Region },
    QuitarRegion { id: u32 },
    Tempo { bpm: f32, #[serde(default = "cuatro")] compas: u8 },
    Marcador { marcador: Marcador },
    /// los efectos de una pista (los que vienen reemplazan a los que había)
    Insertos { pista: u32, insertos: Insertos },
    // ── X2-1 · clips ──
    /// parte en `segundo` los clips de la pista que lo crucen (o sólo `clip`)
    Partir { pista: u32, segundo: f64, #[serde(default)] clip: Option<u32> },
    /// quita lo que haya entre `desde` y `hasta` en la pista (cortando los clips en los bordes); con
    /// `cerrar`, lo de después se corre a la izquierda y no queda hueco
    QuitarTramo { pista: u32, desde: f64, hasta: f64, #[serde(default)] cerrar: bool },
    /// mueve un clip (también a otra pista)
    MoverClip { clip: u32, inicio: f64, #[serde(default)] pista: Option<u32> },
    /// recorta: dónde empieza, desde qué segundo del archivo y cuánto dura
    RecortarClip { clip: u32, inicio: f64, desde: f64, duracion: f64 },
    /// lo que no viene, no cambia
    ClipAjustes {
        clip: u32,
        #[serde(default)]
        ganancia_db: Option<f32>,
        #[serde(default)]
        fundido_entrada: Option<f32>,
        #[serde(default)]
        fundido_salida: Option<f32>,
        #[serde(default)]
        tono: Option<f32>,
        #[serde(default)]
        ritmo: Option<f32>,
        #[serde(default)]
        invertido: Option<bool>,
        #[serde(default)]
        silencio: Option<bool>,
        #[serde(default)]
        nombre: Option<String>,
    },
    /// una copia del clip en `inicio` (también en otra pista)
    DuplicarClip { clip: u32, inicio: f64, #[serde(default)] pista: Option<u32> },
    /// un clip nuevo (pegar, una toma de la IA); su `id` lo pone el núcleo
    AgregarClip { clip: Clip },
    QuitarClip { clip: u32 },
    // ── X2-2 · canal, buses, maestro ──
    InsertosGrupo { grupo: u32, insertos: Insertos },
    Maestro { #[serde(default)] ganancia_db: Option<f32>, #[serde(default)] insertos: Option<Insertos> },
    /// la curva de un parámetro de la pista (sin puntos = se quita)
    Automatizar { pista: u32, parametro: Parametro, puntos: Vec<Punto> },
    /// mover, redimensionar o apagar una región
    RegionAjustes { id: u32, #[serde(default)] desde: Option<f64>, #[serde(default)] hasta: Option<f64>, #[serde(default)] apagada: Option<bool> },
    /// TODOS los marcadores (reemplaza la lista: agregar, mover, renombrar o quitar)
    Marcadores { marcadores: Vec<Marcador> },
    /// el orden de las pistas (una permutación de sus números)
    Ordenar { pistas: Vec<u32> },
    /// el color de una pista (`#rrggbb`; vacío = el del tema)
    Color { pista: u32, color: String },
    RenombrarGrupo { grupo: u32, nombre: String },
    /// la tonalidad de la sesión («A minor»…; vacía = sin fijar)
    Tonalidad { tonalidad: String },
    /// la letra de la canción (la usa la IA al rehacer una voz)
    Letra { texto: String },
}

fn cuatro() -> u8 {
    4
}

/// Lo que resulta de repetir la lista: lo que pinta la interfaz y lo que renderiza el DSP.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Estado {
    pub pistas: Vec<Pista>,
    pub grupos: Vec<Grupo>,
    pub regiones: Vec<Region>,
    pub bpm: f32,
    pub compas: u8,
    pub marcadores: Vec<Marcador>,
    /// X2-1: los clips de todas las pistas, en orden de capa (el posterior tapa al anterior)
    #[serde(default)]
    pub clips: Vec<Clip>,
    #[serde(default)]
    pub maestro: Maestro,
    #[serde(default)]
    pub automatizaciones: Vec<Automatizacion>,
    #[serde(default)]
    pub tonalidad: String,
    #[serde(default)]
    pub letra: String,
    /// el número del próximo clip (los clips tienen su propia cuenta)
    #[serde(default)]
    pub siguiente_clip: u32,
}

impl Estado {
    /// Los clips de una pista, en orden de capa.
    pub fn clips_de(&self, pista: u32) -> impl Iterator<Item = &Clip> {
        self.clips.iter().filter(move |c| c.pista == pista)
    }

    /// ¿La pista es su archivo tal cual? (un solo clip intacto que es su propio archivo)
    pub fn pista_intacta(&self, p: &Pista) -> bool {
        let mut cs = self.clips_de(p.id);
        matches!((cs.next(), cs.next()), (Some(c), None) if c.intacto() && c.archivo == p.archivo)
    }

    pub fn automatizacion(&self, pista: u32, parametro: Parametro) -> Option<&Automatizacion> {
        self.automatizaciones.iter().find(|a| a.pista == pista && a.parametro == parametro && !a.puntos.is_empty())
    }

    fn clip_nuevo(&mut self, mut c: Clip) -> u32 {
        self.siguiente_clip += 1;
        c.id = self.siguiente_clip;
        self.clips.push(c);
        self.siguiente_clip
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Sesion {
    pub version: u32,
    /// el CANDADO: con la sesión activa, el chat y el Pro de ESE proyecto quedan bloqueados
    #[serde(default)]
    pub activa: bool,
    /// minimizada sobre el chat (sigue activa)
    #[serde(default)]
    pub minimizada: bool,
    /// la frecuencia de trabajo (la de ACE-Step)
    pub frecuencia: u32,
    #[serde(default)]
    pub ops: Vec<Op>,
    #[serde(default)]
    pub rehacer: Vec<Op>,
    /// la raíz del proyecto (la pone `cargar`): con ella, cada pista nueva sabe cuánto dura su archivo
    #[serde(skip)]
    pub raiz: Option<PathBuf>,
}

impl Default for Sesion {
    fn default() -> Self {
        Sesion { version: VERSION, activa: false, minimizada: false, frecuencia: 48_000, ops: Vec::new(), rehacer: Vec::new(), raiz: None }
    }
}


/// Parte en `t` los clips de la pista que lo crucen (o sólo `solo`): cada uno queda en dos, sin fundido
/// en el corte. Devuelve cuántos partió.
fn partir_clips(e: &mut Estado, pista: u32, t: f64, solo: Option<u32>) -> Result<usize> {
    // un clip se parte si el corte cae DENTRO, a más de 1 ms de sus bordes
    const MIN: f64 = 0.001;
    let cuales: Vec<usize> = e
        .clips
        .iter()
        .enumerate()
        .filter(|(_, c)| c.pista == pista && solo.is_none_or(|id| id == c.id) && t.is_finite() && t > c.inicio + MIN && c.fin().is_none_or(|f| t < f - MIN))
        .map(|(i, _)| i)
        .collect();
    if cuales.iter().any(|&i| e.clips[i].invertido && e.clips[i].duracion.is_none()) {
        return Err(gotv_engine::anyhow_tr!("un clip invertido necesita su duración para partirse", "a reversed clip needs its length to be split"));
    }
    // de atrás hacia delante: insertar la mitad derecha no mueve los índices que faltan
    for &i in cuales.iter().rev() {
        let corte = t - e.clips[i].inicio;
        let mut der = e.clips[i].clone();
        let izq = &mut e.clips[i];
        let ritmo = izq.ritmo as f64;
        if izq.invertido {
            // al revés, el principio del clip es el FINAL de su trozo de archivo
            let d = izq.duracion.unwrap_or(corte);
            izq.desde += (d - corte) * ritmo;
            der.duracion = Some(d - corte);
        } else {
            der.desde += corte * ritmo;
            der.duracion = izq.duracion.map(|d| d - corte);
        }
        izq.duracion = Some(corte);
        izq.fundido_salida = 0.0;
        der.inicio = t;
        der.fundido_entrada = 0.0;
        e.siguiente_clip += 1;
        der.id = e.siguiente_clip;
        e.clips.insert(i + 1, der);
    }
    Ok(cuales.len())
}

/// Aplica UNA operación sobre el estado, validándola.
fn aplicar_a(e: &mut Estado, op: &Op, dur: &dyn Fn(&str) -> Option<f64>) -> Result<()> {
    let existe_pista = |e: &Estado, id: u32| e.pistas.iter().any(|p| p.id == id);
    let existe_grupo = |e: &Estado, id: u32| e.grupos.iter().any(|g| g.id == id);
    let ganancia = |g: f32| -> Result<f32> {
        if g.is_finite() && (-60.0..=24.0).contains(&g) {
            Ok(g)
        } else {
            Err(gotv_engine::anyhow_tr!("la ganancia va de −60 a +24 dB", "gain goes from −60 to +24 dB"))
        }
    };
    let pan = |p: f32| -> Result<f32> {
        if p.is_finite() && (-1.0..=1.0).contains(&p) {
            Ok(p)
        } else {
            Err(gotv_engine::anyhow_tr!("el paneo va de −1 (izquierda) a 1 (derecha)", "pan goes from −1 (left) to 1 (right)"))
        }
    };
    match op {
        Op::AgregarPista { pista } => {
            if existe_pista(e, pista.id) {
                return Err(gotv_engine::anyhow_tr!("ya hay una pista con ese número", "there is already a track with that number"));
            }
            if pista.archivo.trim().is_empty() || pista.nombre.trim().is_empty() {
                return Err(gotv_engine::anyhow_tr!("la pista necesita nombre y archivo", "the track needs a name and a file"));
            }
            if pista.de.is_some_and(|d| !existe_pista(e, d)) || pista.grupo.is_some_and(|g| !existe_grupo(e, g)) {
                return Err(gotv_engine::anyhow_tr!("la pista madre o el grupo no existen", "the parent track or the group does not exist"));
            }
            ganancia(pista.ganancia_db)?;
            pan(pista.pan)?;
            e.pistas.push(pista.clone());
            // la pista nace con UN clip: todo su archivo desde el segundo 0
            let d = dur(&pista.archivo);
            e.clip_nuevo(Clip::entero(0, pista.id, &pista.archivo, d));
        }
        Op::QuitarPista { id } => {
            if !existe_pista(e, *id) {
                return Err(gotv_engine::anyhow_tr!("esa pista no existe", "that track does not exist"));
            }
            e.pistas.retain(|p| p.id != *id);
            e.regiones.retain(|r| r.pista != *id);
            e.clips.retain(|c| c.pista != *id);
            e.automatizaciones.retain(|a| a.pista != *id);
        }
        Op::Renombrar { pista, nombre } => {
            if nombre.trim().is_empty() {
                return Err(gotv_engine::anyhow_tr!("el nombre está vacío", "the name is empty"));
            }
            e.pistas.iter_mut().find(|p| p.id == *pista).ok_or_else(|| gotv_engine::anyhow_tr!("esa pista no existe", "that track does not exist"))?.nombre = nombre.trim().to_string();
        }
        Op::Mezcla { pista, ganancia_db, pan: pn, silencio, solo } => {
            let g = ganancia_db.map(ganancia).transpose()?;
            let pn = pn.map(pan).transpose()?;
            let p = e.pistas.iter_mut().find(|p| p.id == *pista).ok_or_else(|| gotv_engine::anyhow_tr!("esa pista no existe", "that track does not exist"))?;
            if let Some(g) = g {
                p.ganancia_db = g;
            }
            if let Some(v) = pn {
                p.pan = v;
            }
            if let Some(s) = silencio {
                p.silencio = *s;
            }
            if let Some(s) = solo {
                p.solo = *s;
            }
        }
        Op::CrearGrupo { grupo } => {
            if existe_grupo(e, grupo.id) {
                return Err(gotv_engine::anyhow_tr!("ya hay un grupo con ese número", "there is already a group with that number"));
            }
            if grupo.nombre.trim().is_empty() {
                return Err(gotv_engine::anyhow_tr!("el grupo necesita nombre", "the group needs a name"));
            }
            ganancia(grupo.ganancia_db)?;
            pan(grupo.pan)?;
            e.grupos.push(grupo.clone());
        }
        Op::QuitarGrupo { id } => {
            if !existe_grupo(e, *id) {
                return Err(gotv_engine::anyhow_tr!("ese grupo no existe", "that group does not exist"));
            }
            e.grupos.retain(|g| g.id != *id);
            // sus pistas quedan sueltas, no se borran
            for p in e.pistas.iter_mut().filter(|p| p.grupo == Some(*id)) {
                p.grupo = None;
            }
        }
        Op::Agrupar { pistas, grupo } => {
            if pistas.is_empty() || pistas.iter().any(|p| !existe_pista(e, *p)) {
                return Err(gotv_engine::anyhow_tr!("alguna de esas pistas no existe", "one of those tracks does not exist"));
            }
            if grupo.is_some_and(|g| !existe_grupo(e, g)) {
                return Err(gotv_engine::anyhow_tr!("ese grupo no existe", "that group does not exist"));
            }
            for p in e.pistas.iter_mut().filter(|p| pistas.contains(&p.id)) {
                p.grupo = *grupo;
            }
        }
        Op::MezclaGrupo { grupo, ganancia_db, pan: pn, silencio, solo } => {
            let g = ganancia_db.map(ganancia).transpose()?;
            let pn = pn.map(pan).transpose()?;
            let gr = e.grupos.iter_mut().find(|x| x.id == *grupo).ok_or_else(|| gotv_engine::anyhow_tr!("ese grupo no existe", "that group does not exist"))?;
            if let Some(g) = g {
                gr.ganancia_db = g;
            }
            if let Some(v) = pn {
                gr.pan = v;
            }
            if let Some(s) = silencio {
                gr.silencio = *s;
            }
            if let Some(s) = solo {
                gr.solo = *s;
            }
        }
        Op::Region { region } => {
            if !existe_pista(e, region.pista) {
                return Err(gotv_engine::anyhow_tr!("esa pista no existe", "that track does not exist"));
            }
            if e.regiones.iter().any(|r| r.id == region.id) {
                return Err(gotv_engine::anyhow_tr!("ya hay una región con ese número", "there is already a region with that number"));
            }
            if !(region.desde.is_finite() && region.hasta.is_finite() && region.desde >= 0.0 && region.hasta > region.desde) {
                return Err(gotv_engine::anyhow_tr!("el tramo va de un segundo a otro MAYOR (p. ej. del 5 al 10)", "the span goes from one second to a LATER one (e.g. 5 to 10)"));
            }
            match &region.efecto {
                Efecto::Ganancia { db, .. } => {
                    ganancia(*db)?;
                }
                Efecto::Eq { frecuencia, db, q, .. } => {
                    if !(20.0..=20_000.0).contains(frecuencia) || !(-24.0..=24.0).contains(db) || !(0.1..=18.0).contains(q) {
                        return Err(gotv_engine::anyhow_tr!("la EQ va de 20 Hz a 20 kHz, ±24 dB y Q de 0,1 a 18", "the EQ goes from 20 Hz to 20 kHz, ±24 dB and Q from 0.1 to 18"));
                    }
                }
                Efecto::Tono { semitonos, .. } if !(semitonos.is_finite() && (-12.0..=12.0).contains(semitonos) && *semitonos != 0.0) => {
                    return Err(gotv_engine::anyhow_tr!("transponer va de −12 a +12 semitonos (y distinto de 0)", "transposing goes from −12 to +12 semitones (and not 0)"));
                }
                Efecto::Reemplazo { archivo, .. } if archivo.trim().is_empty() => {
                    return Err(gotv_engine::anyhow_tr!("el reemplazo necesita su archivo", "the replacement needs its file"));
                }
                _ => {}
            }
            e.regiones.push(region.clone());
        }
        Op::QuitarRegion { id } => {
            if !e.regiones.iter().any(|r| r.id == *id) {
                return Err(gotv_engine::anyhow_tr!("esa región no existe", "that region does not exist"));
            }
            e.regiones.retain(|r| r.id != *id);
        }
        Op::Tempo { bpm, compas } => {
            if !(20.0..=400.0).contains(bpm) || !(1..=16).contains(compas) {
                return Err(gotv_engine::anyhow_tr!("el tempo va de 20 a 400 bpm y el compás de 1 a 16 tiempos", "tempo goes from 20 to 400 bpm and the meter from 1 to 16 beats"));
            }
            e.bpm = *bpm;
            e.compas = *compas;
        }
        Op::Insertos { pista, insertos } => {
            insertos.validar()?;
            e.pistas.iter_mut().find(|p| p.id == *pista).ok_or_else(|| gotv_engine::anyhow_tr!("esa pista no existe", "that track does not exist"))?.insertos = insertos.clone();
        }
        Op::Marcador { marcador } => {
            if !(marcador.segundo.is_finite() && marcador.segundo >= 0.0) {
                return Err(gotv_engine::anyhow_tr!("el marcador va en un segundo ≥ 0", "the marker goes at a second ≥ 0"));
            }
            e.marcadores.push(marcador.clone());
            e.marcadores.sort_by(|a, b| a.segundo.total_cmp(&b.segundo));
        }
        Op::Partir { pista, segundo, clip } => {
            if partir_clips(e, *pista, *segundo, *clip)? == 0 {
                return Err(gotv_engine::anyhow_tr!("ningún clip de esa pista cruza ese segundo", "no clip on that track crosses that second"));
            }
        }
        Op::QuitarTramo { pista, desde, hasta, cerrar } => {
            if !existe_pista(e, *pista) {
                return Err(gotv_engine::anyhow_tr!("esa pista no existe", "that track does not exist"));
            }
            if !(desde.is_finite() && hasta.is_finite() && *desde >= 0.0 && hasta > desde) {
                return Err(gotv_engine::anyhow_tr!("el tramo va de un segundo a otro MAYOR (p. ej. del 5 al 10)", "the span goes from one second to a LATER one (e.g. 5 to 10)"));
            }
            // se corta en los dos bordes y se quita lo que queda dentro
            partir_clips(e, *pista, *desde, None)?;
            partir_clips(e, *pista, *hasta, None)?;
            const EPS: f64 = 1e-6;
            let antes = e.clips.len();
            e.clips.retain(|c| !(c.pista == *pista && c.inicio >= *desde - EPS && c.fin().is_some_and(|f| f <= *hasta + EPS)));
            if e.clips.len() == antes {
                return Err(gotv_engine::anyhow_tr!("en ese tramo no hay nada que quitar", "there is nothing to remove in that span"));
            }
            if *cerrar {
                // lo que venía después se corre a la izquierda y tapa el hueco
                for c in e.clips.iter_mut().filter(|c| c.pista == *pista && c.inicio >= *hasta - EPS) {
                    c.inicio = (c.inicio - (hasta - desde)).max(0.0);
                }
            }
        }
        Op::MoverClip { clip, inicio, pista } => {
            if pista.is_some_and(|p| !existe_pista(e, p)) {
                return Err(gotv_engine::anyhow_tr!("esa pista no existe", "that track does not exist"));
            }
            let i = e.clips.iter().position(|c| c.id == *clip).ok_or_else(|| gotv_engine::anyhow_tr!("ese clip no existe", "that clip does not exist"))?;
            let mut c = e.clips[i].clone();
            c.inicio = *inicio;
            if let Some(p) = pista {
                c.pista = *p;
            }
            c.validar()?;
            // el clip que se mueve queda encima
            e.clips.remove(i);
            e.clips.push(c);
        }
        Op::RecortarClip { clip, inicio, desde, duracion } => {
            let c = e.clips.iter_mut().find(|c| c.id == *clip).ok_or_else(|| gotv_engine::anyhow_tr!("ese clip no existe", "that clip does not exist"))?;
            let nuevo = Clip { inicio: *inicio, desde: *desde, duracion: Some(*duracion), ..c.clone() };
            nuevo.validar()?;
            *c = nuevo;
        }
        Op::ClipAjustes { clip, ganancia_db, fundido_entrada, fundido_salida, tono, ritmo, invertido, silencio, nombre } => {
            let c = e.clips.iter_mut().find(|c| c.id == *clip).ok_or_else(|| gotv_engine::anyhow_tr!("ese clip no existe", "that clip does not exist"))?;
            let mut n = c.clone();
            if let Some(v) = ganancia_db {
                n.ganancia_db = *v;
            }
            if let Some(v) = fundido_entrada {
                n.fundido_entrada = *v;
            }
            if let Some(v) = fundido_salida {
                n.fundido_salida = *v;
            }
            if let Some(v) = tono {
                n.tono = *v;
            }
            if let Some(v) = ritmo {
                // la duración en la línea de tiempo sigue al ritmo: el mismo trozo de archivo, más rápido o más lento
                if let (Some(d), true) = (n.duracion, *v > 0.0) {
                    n.duracion = Some(d * n.ritmo as f64 / *v as f64);
                }
                n.ritmo = *v;
            }
            if let Some(v) = invertido {
                if *v && n.duracion.is_none() {
                    return Err(gotv_engine::anyhow_tr!("para invertir un clip hace falta su duración", "reversing a clip needs its length"));
                }
                n.invertido = *v;
            }
            if let Some(v) = silencio {
                n.silencio = *v;
            }
            if let Some(v) = nombre {
                n.nombre = v.trim().to_string();
            }
            n.validar()?;
            *c = n;
        }
        Op::DuplicarClip { clip, inicio, pista } => {
            if pista.is_some_and(|p| !existe_pista(e, p)) {
                return Err(gotv_engine::anyhow_tr!("esa pista no existe", "that track does not exist"));
            }
            let mut c = e.clips.iter().find(|c| c.id == *clip).cloned().ok_or_else(|| gotv_engine::anyhow_tr!("ese clip no existe", "that clip does not exist"))?;
            c.inicio = *inicio;
            if let Some(p) = pista {
                c.pista = *p;
            }
            c.validar()?;
            e.clip_nuevo(c);
        }
        Op::AgregarClip { clip } => {
            if !existe_pista(e, clip.pista) {
                return Err(gotv_engine::anyhow_tr!("esa pista no existe", "that track does not exist"));
            }
            clip.validar()?;
            e.clip_nuevo(clip.clone());
        }
        Op::QuitarClip { clip } => {
            if !e.clips.iter().any(|c| c.id == *clip) {
                return Err(gotv_engine::anyhow_tr!("ese clip no existe", "that clip does not exist"));
            }
            e.clips.retain(|c| c.id != *clip);
        }
        Op::InsertosGrupo { grupo, insertos } => {
            insertos.validar()?;
            e.grupos.iter_mut().find(|g| g.id == *grupo).ok_or_else(|| gotv_engine::anyhow_tr!("ese grupo no existe", "that group does not exist"))?.insertos = insertos.clone();
        }
        Op::Maestro { ganancia_db, insertos } => {
            let g = ganancia_db.map(ganancia).transpose()?;
            if let Some(i) = insertos {
                i.validar()?;
                e.maestro.insertos = i.clone();
            }
            if let Some(g) = g {
                e.maestro.ganancia_db = g;
            }
        }
        Op::Automatizar { pista, parametro, puntos } => {
            if !existe_pista(e, *pista) {
                return Err(gotv_engine::anyhow_tr!("esa pista no existe", "that track does not exist"));
            }
            let (min, max) = match parametro {
                Parametro::Volumen => (-60.0, 24.0),
                Parametro::Pan => (-1.0, 1.0),
            };
            if puntos.len() > 4000 || puntos.iter().any(|p| !(p.t.is_finite() && p.t >= 0.0 && p.v.is_finite() && (min..=max).contains(&p.v))) {
                return Err(gotv_engine::anyhow_tr!(
                    "automatización: puntos en segundos ≥ 0; el volumen va de −60 a +24 dB y el paneo de −1 a 1",
                    "automation: points at seconds ≥ 0; volume goes from −60 to +24 dB and pan from −1 to 1"
                ));
            }
            let mut puntos = puntos.clone();
            puntos.sort_by(|a, b| a.t.total_cmp(&b.t));
            e.automatizaciones.retain(|a| !(a.pista == *pista && a.parametro == *parametro));
            if !puntos.is_empty() {
                e.automatizaciones.push(Automatizacion { pista: *pista, parametro: *parametro, puntos });
            }
        }
        Op::RegionAjustes { id, desde, hasta, apagada } => {
            let r = e.regiones.iter_mut().find(|r| r.id == *id).ok_or_else(|| gotv_engine::anyhow_tr!("esa región no existe", "that region does not exist"))?;
            let (d, h) = (desde.unwrap_or(r.desde), hasta.unwrap_or(r.hasta));
            if !(d.is_finite() && h.is_finite() && d >= 0.0 && h > d) {
                return Err(gotv_engine::anyhow_tr!("el tramo va de un segundo a otro MAYOR (p. ej. del 5 al 10)", "the span goes from one second to a LATER one (e.g. 5 to 10)"));
            }
            r.desde = d;
            r.hasta = h;
            if let Some(a) = apagada {
                r.apagada = *a;
            }
        }
        Op::Marcadores { marcadores } => {
            if marcadores.len() > 500 || marcadores.iter().any(|m| !(m.segundo.is_finite() && m.segundo >= 0.0)) {
                return Err(gotv_engine::anyhow_tr!("el marcador va en un segundo ≥ 0", "the marker goes at a second ≥ 0"));
            }
            e.marcadores = marcadores.clone();
            e.marcadores.sort_by(|a, b| a.segundo.total_cmp(&b.segundo));
        }
        Op::Ordenar { pistas } => {
            let mut a: Vec<u32> = pistas.clone();
            let mut b: Vec<u32> = e.pistas.iter().map(|p| p.id).collect();
            a.sort_unstable();
            b.sort_unstable();
            if a != b {
                return Err(gotv_engine::anyhow_tr!("el orden debe nombrar cada pista una sola vez", "the order must name every track exactly once"));
            }
            e.pistas.sort_by_key(|p| pistas.iter().position(|id| *id == p.id));
        }
        Op::Color { pista, color } => {
            let c = color.trim();
            if !(c.is_empty() || (c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|x| x.is_ascii_hexdigit()))) {
                return Err(gotv_engine::anyhow_tr!("el color va como #rrggbb", "the color goes as #rrggbb"));
            }
            e.pistas.iter_mut().find(|p| p.id == *pista).ok_or_else(|| gotv_engine::anyhow_tr!("esa pista no existe", "that track does not exist"))?.color = Some(c.to_lowercase()).filter(|c| !c.is_empty());
        }
        Op::RenombrarGrupo { grupo, nombre } => {
            if nombre.trim().is_empty() {
                return Err(gotv_engine::anyhow_tr!("el nombre está vacío", "the name is empty"));
            }
            e.grupos.iter_mut().find(|g| g.id == *grupo).ok_or_else(|| gotv_engine::anyhow_tr!("ese grupo no existe", "that group does not exist"))?.nombre = nombre.trim().to_string();
        }
        Op::Tonalidad { tonalidad } => {
            if tonalidad.chars().count() > 24 {
                return Err(gotv_engine::anyhow_tr!("la tonalidad va como «A minor» o «C major»", "the key goes as \"A minor\" or \"C major\""));
            }
            e.tonalidad = tonalidad.trim().to_string();
        }
        Op::Letra { texto } => {
            if texto.len() > 40_000 {
                return Err(gotv_engine::anyhow_tr!("la letra es demasiado larga", "the lyrics are too long"));
            }
            e.letra = texto.trim().to_string();
        }
    }
    Ok(())
}

impl Sesion {
    /// Cuánto dura un archivo de la sesión (sin raíz, no se sabe: el clip llega «hasta donde haya audio»).
    fn dur(&self) -> impl Fn(&str) -> Option<f64> + '_ {
        move |archivo: &str| self.raiz.as_ref().and_then(|r| crate::experto_dsp::duracion_wav(&r.join(archivo)))
    }

    /// El estado que resulta de la lista (tempo por omisión: 120 bpm en 4).
    pub fn estado(&self) -> Estado {
        let dur = self.dur();
        let mut e = Estado { bpm: 120.0, compas: 4, ..Default::default() };
        for op in &self.ops {
            // la lista guardada ya se validó al entrar; si un archivo se editó a mano, lo que no cuadra se salta
            let _ = aplicar_a(&mut e, op, &dur);
        }
        e
    }

    /// Un número nuevo para una pista, grupo o región (mayor que todos los que hubo, también los deshechos).
    pub fn nuevo_id(&self) -> u32 {
        let id = |op: &Op| match op {
            Op::AgregarPista { pista } => pista.id,
            Op::CrearGrupo { grupo } => grupo.id,
            Op::Region { region } => region.id,
            _ => 0,
        };
        self.ops.iter().chain(&self.rehacer).map(id).max().unwrap_or(0) + 1
    }

    /// Lo que crea algo y llega con `id` 0 (de la interfaz o del chat de Experto) recibe un número nuevo.
    pub fn numerar(&self, mut op: Op) -> Op {
        let n = self.nuevo_id();
        match &mut op {
            Op::AgregarPista { pista } if pista.id == 0 => pista.id = n,
            Op::CrearGrupo { grupo } if grupo.id == 0 => grupo.id = n,
            Op::Region { region } if region.id == 0 => region.id = n,
            _ => {}
        }
        op
    }

    /// Lo que ve la interfaz: el estado y cuánto se puede deshacer y rehacer.
    pub fn vista(&self) -> serde_json::Value {
        let estado = self.estado();
        // cuánto dura cada archivo que usa un clip: la ventana lo necesita para recortar sin pasarse
        let dur = self.dur();
        let archivos: std::collections::BTreeMap<&str, f64> = estado.clips.iter().filter_map(|c| Some((c.archivo.as_str(), dur(&c.archivo)?))).collect();
        serde_json::json!({
            "archivos": archivos,
            "activa": self.activa,
            "minimizada": self.minimizada,
            "frecuencia": self.frecuencia,
            "deshacer": self.ops.len(),
            "rehacer": self.rehacer.len(),
            // el historial: lo hecho (en orden) y lo deshecho (lo próximo a rehacer, primero)
            "historial": self.ops.iter().map(etiqueta_op).collect::<Vec<_>>(),
            "por_rehacer": self.rehacer.iter().rev().map(etiqueta_op).collect::<Vec<_>>(),
            "estado": estado,
        })
    }

    /// Aplica una operación (validada contra el estado actual) y vacía «rehacer».
    pub fn aplicar(&mut self, op: Op) -> Result<Estado> {
        let e = {
            let dur = self.dur();
            let mut e = self.estado();
            aplicar_a(&mut e, &op, &dur)?;
            e
        };

        self.ops.push(op);
        self.rehacer.clear();
        Ok(e)
    }

    pub fn deshacer(&mut self) -> bool {
        match self.ops.pop() {
            Some(op) => {
                self.rehacer.push(op);
                true
            }
            None => false,
        }
    }

    pub fn rehacer(&mut self) -> bool {
        match self.rehacer.pop() {
            Some(op) => {
                self.ops.push(op);
                true
            }
            None => false,
        }
    }

    /// Deja la sesión con exactamente `n` operaciones hechas (deshaciendo o rehaciendo): así se recorre
    /// el historial.
    pub fn ir_a(&mut self, n: usize) {
        while self.ops.len() > n && self.deshacer() {}
        while self.ops.len() < n && self.rehacer() {}
    }
}

pub fn ruta(root: &Path) -> PathBuf {
    root.join(ARCHIVO)
}

/// La sesión del proyecto (vacía e inactiva si no hay).
pub fn cargar(root: &Path) -> Sesion {
    let r = ruta(root);
    let mut s: Sesion = crate::programar::codec::leer_serde_migrado(&r, &r.with_extension("json"), b"EXPS").unwrap_or_default();
    s.raiz = Some(root.to_path_buf());
    s
}

/// Guarda de forma atómica (archivo temporal y renombrar): un corte a mitad no deja la sesión rota.
pub fn guardar(root: &Path, s: &Sesion) -> Result<()> {
    crate::programar::codec::guardar_serde(&ruta(root), b"EXPS", s)
}

/// ¿Está el Experto activo en este proyecto? (el candado del chat y del Pro)
pub fn bloqueado(root: &Path) -> bool {
    cargar(root).activa
}

/// Abre o cierra el Experto del proyecto (cerrarlo NO borra la sesión: se retoma tal cual).
pub fn activar(root: &Path, activa: bool) -> Result<Sesion> {
    let mut s = cargar(root);
    s.activa = activa;
    if !activa {
        s.minimizada = false;
    }
    guardar(root, &s)?;
    Ok(s)
}

/// La ruta relativa al proyecto (con `/`), o error si queda fuera.
pub fn relativa(root: &Path, p: &Path) -> Result<String> {
    let raiz = std::fs::canonicalize(root)?;
    let abs = std::fs::canonicalize(p).map_err(|e| gotv_engine::anyhow_tr!("{}: {e}", "{}: {e}", p.display()))?;
    let rel = abs.strip_prefix(&raiz).map_err(|_| gotv_engine::anyhow_tr!("{} está fuera del proyecto", "{} is outside the project", p.display()))?;
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

/// Trae un audio a la sesión. Si ya es un WAV estéreo o mono a la frecuencia de la sesión y está DENTRO
/// del proyecto, se usa tal cual (no se copia); si no, `afconvert` (macOS) lo deja en
/// `.gotv/experto/audio/` como WAV estéreo en coma flotante a la frecuencia de la sesión. Devuelve la
/// ruta relativa al proyecto.
pub fn importar(root: &Path, s: &Sesion, ruta: &Path) -> Result<String> {
    let ruta = if ruta.is_absolute() { ruta.to_path_buf() } else { root.join(ruta) };
    if !ruta.is_file() {
        anyhow::bail!("{}", crate::tr!("no existe el audio {}", "the audio {} does not exist", ruta.display()));
    }
    if let (Ok(rel), Ok((c, f))) = (relativa(root, &ruta), crate::experto_dsp::leer_wav(&ruta)) {
        if f == s.frecuencia && (1..=2).contains(&c.len()) {
            return Ok(rel);
        }
    }
    let base = ruta.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "audio".into());
    let dir = root.join(".gotv/experto/audio");
    std::fs::create_dir_all(&dir)?;
    let mut destino = dir.join(format!("{base}.wav"));
    let mut k = 1;
    while destino.exists() {
        k += 1;
        destino = dir.join(format!("{base}-{k}.wav"));
    }
    let salida = std::process::Command::new("afconvert")
        .args(["-f", "WAVE", "-d", &format!("LEF32@{}", s.frecuencia), "-c", "2"])
        .arg(&ruta)
        .arg(&destino)
        .output()
        .map_err(|e| gotv_engine::anyhow_tr!("afconvert: {e}", "afconvert: {e}"))?;
    if !salida.status.success() || !destino.is_file() {
        anyhow::bail!("{}", crate::tr!("no se pudo convertir {}: {}", "could not convert {}: {}", ruta.display(), String::from_utf8_lossy(&salida.stderr).trim()));
    }
    relativa(root, &destino)
}

/// La operación que agrega un audio (importado o generado) como pista nueva.
pub fn op_importar(root: &Path, s: &Sesion, ruta: &Path, nombre: Option<&str>, origen: Origen) -> Result<Op> {
    let archivo = importar(root, s, ruta)?;
    let nombre = nombre.map(str::trim).filter(|n| !n.is_empty()).map(String::from).unwrap_or_else(|| ruta.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "audio".into()));
    Ok(s.numerar(Op::AgregarPista { pista: Pista { color: None, id: 0, nombre, archivo, origen, de: None, fuente: None, ganancia_db: 0.0, pan: 0.0, silencio: false, solo: false, grupo: None, insertos: Default::default() } }))
}

/// Los nombres de las fuentes de HTDemucs, para las pistas hijas.
pub fn nombre_fuente(f: &str) -> &'static str {
    match f {
        "vocals" => crate::i18n::t("Voz", "Vocals"),
        "drums" => crate::i18n::t("Batería", "Drums"),
        "bass" => crate::i18n::t("Bajo", "Bass"),
        "guitar" => crate::i18n::t("Guitarra", "Guitar"),
        "piano" => crate::i18n::t("Piano", "Piano"),
        "kick" => crate::i18n::t("Bombo", "Kick"),
        "snare" => crate::i18n::t("Caja", "Snare"),
        "cymbals" => crate::i18n::t("Platos", "Cymbals"),
        "toms" => crate::i18n::t("Toms", "Toms"),
        _ => crate::i18n::t("Resto", "Other"),
    }
}

/// Divide una pista con HTDemucs (`modelo` «4», «6», «ft» o «bateria»): agrega una pista hija por fuente
/// (origen «separada», con su madre en `de`) y SILENCIA la madre, para que la mezcla siga sonando igual.
/// Con «bateria» (EXP-7) la pista se parte en bombo, caja, platos y toms; si NO es ya una batería (su
/// fuente no es «drums»), primero se saca su batería. Devuelve los números de las pistas nuevas.
pub fn separar(root: &Path, s: &mut Sesion, pista: u32, modelo: &str, progreso: &(dyn Fn(String) + Sync), cancelar: &std::sync::atomic::AtomicBool) -> Result<Vec<u32>> {
    let e = s.estado();
    let madre = e.pistas.iter().find(|p| p.id == pista).ok_or_else(|| gotv_engine::anyhow_tr!("esa pista no existe", "that track does not exist"))?.clone();
    // lo que se separa es la pista COMO ESTÁ (sus clips y sus regiones); intacta, su propio archivo
    let editada = crate::experto_dsp::pista_editada(root, &e, &madre, s.frecuencia)?;
    let (fuentes, _) = if modelo == "bateria" {
        crate::generadores::separar_bateria(&editada, madre.fuente.as_deref() == Some("drums"), progreso, cancelar)?
    } else {
        crate::generadores::separar_audio(&editada, modelo, progreso, cancelar)?
    };
    let mut nuevas = Vec::new();
    for (f, ruta) in fuentes {
        let archivo = relativa(root, &ruta)?;
        let op = s.numerar(Op::AgregarPista {
            pista: Pista { color: None, id: 0, nombre: format!("{} · {}", madre.nombre, nombre_fuente(&f)), archivo, origen: Origen::Separada, de: Some(madre.id), fuente: Some(f), ganancia_db: 0.0, pan: 0.0, silencio: false, solo: false, grupo: madre.grupo, insertos: Default::default() },
        });
        if let Op::AgregarPista { pista } = &op {
            nuevas.push(pista.id);
        }
        s.aplicar(op)?;
    }
    s.aplicar(Op::Mezcla { pista: madre.id, ganancia_db: None, pan: None, silencio: Some(true), solo: None })?;
    Ok(nuevas)
}

/// Pedido del Portador (2026-11-14): lo transpuesto va en SU PROPIA pista («así poderlos sacar en una mejor
/// voz»). Transpone `[desde, hasta]` de `pista` a una pista nueva (sólo ese tramo, con fundidos), hermana de
/// la pista (misma madre y parte, mismo fader, paneo, grupo y efectos), y SILENCIA ese tramo en la original
/// con el fundido complementario: la mezcla suena igual que transponer en el sitio, pero la parte nueva se
/// puede oír sola, ecualizar, borrar o rehacer con IA. Devuelve el número de la pista nueva.
pub fn tono_a_pista(root: &Path, s: &mut Sesion, pista: u32, desde: f64, hasta: f64, semitonos: f32, fundido: f32) -> Result<u32> {
    use crate::experto_dsp as dsp;
    let e = s.estado();
    let p = e.pistas.iter().find(|p| p.id == pista).cloned().ok_or_else(|| gotv_engine::anyhow_tr!("esa pista no existe", "that track does not exist"))?;
    if !(semitonos.is_finite() && (-12.0..=12.0).contains(&semitonos) && semitonos != 0.0) || !(desde >= 0.0 && hasta > desde) {
        return Err(gotv_engine::anyhow_tr!("transponer va de −12 a +12 semitonos sobre un tramo creciente", "transposing goes from −12 to +12 semitones over an increasing span"));
    }
    let fs = s.frecuencia;
    // la pista con sus regiones (sin sus efectos: la nueva los hereda y se aplican una sola vez)
    let x = dsp::pista_con_regiones(&e, &p, fs, root)?;
    // la misma operación que la región «tono» en el sitio, pero sólo con la parte transpuesta (lo demás, cero)
    let mut solo_tramo: dsp::Estereo = [vec![0f32; x[0].len()], vec![0f32; x[1].len()]];
    let region = Region { apagada: false, id: 0, pista, desde, hasta, efecto: Efecto::Tono { semitonos, fundido } };
    let mut transpuesto = x.clone();
    dsp::aplicar_region(&mut transpuesto, &region, fs, root)?;
    let mut silenciado = x.clone();
    dsp::aplicar_region(&mut silenciado, &Region { efecto: Efecto::Silencio { fundido }, ..region.clone() }, fs, root)?;
    for c in 0..2 {
        for n in 0..x[c].len() {
            // en el sitio = (1 − w)·x + w·y; silenciar deja (1 − w)·x: la pista nueva es la diferencia, w·y
            solo_tramo[c][n] = transpuesto[c][n] - silenciado[c][n];
        }
    }
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let archivo = format!(".gotv/experto/audio/tono-{}-{ms}.wav", p.id);
    dsp::escribir_wav(&root.join(&archivo), &solo_tramo, fs, false)?;
    let signo = if semitonos > 0.0 { "+" } else { "" };
    let nueva = Pista {
        id: 0,
        nombre: format!("{} · {signo}{semitonos} st ({desde:.1}–{hasta:.1} s)", p.nombre),
        archivo,
        origen: Origen::Generada,
        de: p.de,
        fuente: p.fuente.clone(),
        ganancia_db: p.ganancia_db,
        pan: p.pan,
        silencio: false,
        solo: false,
        grupo: p.grupo,
        insertos: p.insertos.clone(),
        color: p.color.clone(),
    };
    let op = s.numerar(Op::AgregarPista { pista: nueva });
    let id = match &op {
        Op::AgregarPista { pista } => pista.id,
        _ => 0,
    };
    s.aplicar(op)?;
    let op = s.numerar(Op::Region { region: Region { apagada: false, id: 0, pista, desde, hasta, efecto: Efecto::Silencio { fundido } } });
    s.aplicar(op)?;
    Ok(id)
}

/// Ola EXPERTO · EXP-4: rehacer `[desde, hasta]` de una pista con OTRO sonido (`descripcion`, en inglés), y si
/// se da, con su LETRA (para que una voz nueva cante las mismas palabras y no balbucee).
/// - La ventana: la canción ENTERA si dura hasta 2 min (así la letra queda alineada); si no, ±8 s.
/// - Si la pista es una PARTE separada (batería, voz…), el contexto es la mezcla de las partes (la madre está
///   silenciada) y se REPINTA el tramo; HTDemucs saca de lo repintado sólo esa parte. (Medido 2026-11-14: el
///   `lego` del DiT base apenas generaba la parte pedida.) Si es una mezcla completa, se repinta y ya.
/// - El tramo nuevo se iguala en nivel a lo que reemplaza (±12 dB) y entra como región «reemplazo» con
///   80 ms de fundido: se deshace como cualquier otra. Los temporales se borran al terminar.
#[allow(clippy::too_many_arguments)]
pub fn rehacer_tramo(root: &Path, s: &mut Sesion, pista: u32, desde: f64, hasta: f64, descripcion: &str, letra: &str, idioma: &str, progreso: &(dyn Fn(String) + Sync), cancelar: &std::sync::atomic::AtomicBool) -> Result<()> {
    use crate::experto_dsp as dsp;
    let e = s.estado();
    let fs = s.frecuencia;
    let p = e.pistas.iter().find(|p| p.id == pista).cloned().ok_or_else(|| gotv_engine::anyhow_tr!("esa pista no existe", "that track does not exist"))?;
    let total = (dsp::pista_con_regiones(&e, &p, fs, root)?[0].len() as f64 / fs as f64).max(hasta);
    let (v0, v1) = if total <= 120.0 { (0.0, total) } else { ((desde - 8.0).max(0.0), (hasta + 8.0).min(total)) };
    let (i0, i1) = ((v0 * fs as f64) as usize, (v1 * fs as f64) as usize);
    let recorte = |x: &dsp::Estereo| -> dsp::Estereo { std::array::from_fn(|c| (i0..i1).map(|n| x[c].get(n).copied().unwrap_or(0.0)).collect()) };
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let tmp = root.join(".gotv/experto/tmp");
    std::fs::create_dir_all(&tmp)?;
    let contexto = tmp.join(format!("contexto-{ms}.wav")); // juez-idioma: datos (nombre de archivo)
    let salida = tmp.join(format!("nuevo-{ms}.wav"));
    // una VOZ sin letra: Whisper la saca de la voz ORIGINAL separada (la hermana real, no una transpuesta),
    // sólo de la ventana (≤ 120 s, el tope de Whisper), para que la voz nueva cante las mismas palabras
    let mut letra = letra.trim().to_string();
    if letra.is_empty() && p.fuente.as_deref() == Some("vocals") {
        let original = e.pistas.iter().find(|h| h.fuente.as_deref() == Some("vocals") && h.origen == Origen::Separada && h.de == p.de).unwrap_or(&p);
        if let Ok(x) = dsp::pista_con_regiones(&e, original, fs, root) {
            let voz = tmp.join(format!("voz-{ms}.wav")); // juez-idioma: datos (nombre de archivo)
            if dsp::escribir_wav(&voz, &recorte(&x), fs, false).is_ok() {
                progreso(crate::i18n::t("transcribiendo la letra de la voz (Whisper)", "transcribing the vocal lyrics (Whisper)").to_string());
                let r = crate::audio::transcribe(&voz);
                if let Err(e) = &r {
                    // sin letra, la voz nueva puede balbucear: se dice por qué (p. ej. Whisper sin aceptar)
                    progreso(crate::tr!("sin letra: {e}", "no lyrics: {e}"));
                }
                if let Ok(t) = r {
                    letra = t.trim().to_string();
                    progreso(crate::tr!("letra: «{}…»", "lyrics: «{}…»", letra.chars().take(60).collect::<String>()));
                }
            }
        }
    }
    let mut pedido = crate::generadores::PedidoMusica {
        descripcion: descripcion.to_string(),
        letra,
        idioma: idioma.trim().to_string(),
        segundos: (v1 - v0) as f32,
        bpm: Some(e.bpm.round() as u32).filter(|b| (30..=300).contains(b)),
        semilla: ms % 1_000_000_007,
        fuente: Some(contexto.clone()),
        fuerza_cover: 1.0,
        ..Default::default()
    };
    let parte = p.fuente.clone();
    let mezcla = if parte.is_some() {
        let partes: Vec<dsp::Estereo> = e.pistas.iter().filter(|h| h.de.is_some() && h.de == p.de).map(|h| dsp::render_pista(&e, h, fs, root).map(|x| recorte(&x))).collect::<Result<_>>()?;
        dsp::sumar(&partes)
    } else {
        recorte(&dsp::render_pista(&e, &p, fs, root)?)
    };
    dsp::escribir_wav(&contexto, &mezcla, fs, false)?;
    pedido.repintar = Some(((desde - v0) as f32, (hasta - v0) as f32));
    let limpiar = || {
        let marca = ms.to_string();
        for x in std::fs::read_dir(&tmp).into_iter().flatten().flatten() {
            if x.file_name().to_string_lossy().contains(&marca) {
                let r = x.path();
                let _ = if r.is_dir() { std::fs::remove_dir_all(&r) } else { std::fs::remove_file(&r) };
            }
        }
    };
    let generar = || -> Result<std::path::PathBuf> {
        crate::generadores::generar_musica(&pedido, &salida, progreso, cancelar)?;
        match &parte {
            // lo repintado trae todas las partes: HTDemucs se queda sólo con la pedida
            Some(f) => {
                let (pistas, _) = crate::generadores::separar_audio(&salida, &crate::experto_chat::modelo_para(f), progreso, cancelar)?;
                pistas.into_iter().find(|(k, _)| k == f).map(|(_, r)| r).ok_or_else(|| gotv_engine::anyhow_tr!("el separador no sacó «{f}»", "the separator did not extract «{f}»"))
            }
            None => Ok(salida.clone()),
        }
    };
    let nuevo = match generar() {
        Ok(n) => n,
        Err(e) => {
            limpiar();
            return Err(e);
        }
    };
    // el tramo, alineado para que empiece JUSTO en `desde` (así lo lee la región)
    let x = dsp::leer_estereo(&nuevo, fs)?;
    let (a, b) = (((desde - v0) * fs as f64) as usize, ((hasta - v0) * fs as f64).ceil() as usize);
    let mut tramo: dsp::Estereo = std::array::from_fn(|c| (a..b).map(|n| x[c].get(n).copied().unwrap_or(0.0)).collect());
    // al NIVEL de lo que reemplaza (medido 2026-11-14: la batería de escobillas salió 12 dB más baja y dejaba
    // un hueco): RMS igualada a la del tramo original ya editado, con tope de ±12 dB
    let rms = |x: &[f32]| (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt();
    let antes = dsp::render_pista(&e, &p, fs, root)?;
    let (ra, rb) = (((desde * fs as f64) as usize).min(antes[0].len()), ((hasta * fs as f64) as usize).min(antes[0].len()));
    let (viejo, nuevo_rms) = (rms(&antes[0][ra..rb]) + rms(&antes[1][ra..rb]), rms(&tramo[0]) + rms(&tramo[1]));
    if viejo > 1e-6 && nuevo_rms > 1e-6 {
        let g = (viejo / nuevo_rms).clamp(0.25, 4.0);
        for c in tramo.iter_mut() {
            c.iter_mut().for_each(|v| *v = (*v as f64 * g) as f32);
        }
    }
    let archivo = format!(".gotv/experto/audio/reemplazo-{ms}.wav");
    dsp::escribir_wav(&root.join(&archivo), &tramo, fs, false)?;
    let op = s.numerar(Op::Region { region: Region { apagada: false, id: 0, pista, desde, hasta, efecto: Efecto::Reemplazo { archivo, descripcion: descripcion.to_string(), fundido: 0.08 } } });
    s.aplicar(op)?;
    limpiar();
    Ok(())
}

/// El nombre corto de una operación, para el historial de la interfaz.
pub fn etiqueta_op(op: &Op) -> String {
    use crate::i18n::t;
    match op {
        Op::AgregarPista { pista } => crate::tr!("Pista nueva: {}", "New track: {}", pista.nombre),
        Op::QuitarPista { id } => crate::tr!("Quitar la pista {id}", "Remove track {id}"),
        Op::Renombrar { nombre, .. } => crate::tr!("Renombrar a «{nombre}»", "Rename to \"{nombre}\""),
        Op::Mezcla { pista, ganancia_db, pan, silencio, solo } => {
            let que = match (ganancia_db, pan, silencio, solo) {
                (Some(g), ..) => crate::tr!("volumen {g:+.1} dB", "volume {g:+.1} dB"),
                (_, Some(p), ..) => crate::tr!("paneo {p:+.2}", "pan {p:+.2}"),
                (_, _, Some(s), _) => t(if *s { "silenciar" } else { "quitar el silencio" }, if *s { "mute" } else { "unmute" }).to_string(),
                (_, _, _, Some(s)) => t(if *s { "solo" } else { "quitar el solo" }, if *s { "solo" } else { "unsolo" }).to_string(),
                _ => t("mezcla", "mix").to_string(),
            };
            crate::tr!("Pista {pista}: {que}", "Track {pista}: {que}")
        }
        Op::CrearGrupo { grupo } => crate::tr!("Bus nuevo: {}", "New bus: {}", grupo.nombre),
        Op::QuitarGrupo { .. } => t("Quitar un bus", "Remove a bus").to_string(),
        Op::Agrupar { pistas, grupo } => match grupo {
            Some(_) => crate::tr!("Agrupar {} pistas", "Group {} tracks", pistas.len()),
            None => crate::tr!("Sacar {} pistas de su bus", "Ungroup {} tracks", pistas.len()),
        },
        Op::MezclaGrupo { .. } => t("Mezcla de un bus", "Bus mix").to_string(),
        Op::Region { region } => {
            let que = match &region.efecto {
                Efecto::Ganancia { db, .. } => format!("{db:+.1} dB"),
                Efecto::Silencio { .. } => t("silencio", "silence").to_string(),
                Efecto::Eq { .. } => "EQ".to_string(),
                Efecto::Reemplazo { descripcion, .. } => crate::tr!("IA: {descripcion}", "AI: {descripcion}"),
                Efecto::Tono { semitonos, .. } => crate::tr!("{semitonos:+} semitonos", "{semitonos:+} semitones"),
            };
            crate::tr!("Tramo {:.1}–{:.1} s: {que}", "Span {:.1}–{:.1} s: {que}", region.desde, region.hasta)
        }
        Op::QuitarRegion { .. } => t("Quitar un tramo", "Remove a span").to_string(),
        Op::Tempo { bpm, compas } => crate::tr!("Tempo {bpm:.0} bpm, compás de {compas}", "Tempo {bpm:.0} bpm, {compas} beats per bar"),
        Op::Marcador { marcador } => crate::tr!("Marcador «{}»", "Marker \"{}\"", marcador.nombre),
        Op::Insertos { pista, .. } => crate::tr!("Efectos de la pista {pista}", "Track {pista} effects"),
        Op::Partir { segundo, .. } => crate::tr!("Partir en {segundo:.2} s", "Split at {segundo:.2} s"),
        Op::QuitarTramo { desde, hasta, .. } => crate::tr!("Quitar {desde:.2}–{hasta:.2} s", "Remove {desde:.2}–{hasta:.2} s"),
        Op::MoverClip { inicio, .. } => crate::tr!("Mover un clip a {inicio:.2} s", "Move a clip to {inicio:.2} s"),
        Op::RecortarClip { .. } => t("Recortar un clip", "Trim a clip").to_string(),
        Op::ClipAjustes { .. } => t("Ajustar un clip", "Adjust a clip").to_string(),
        Op::DuplicarClip { .. } => t("Duplicar un clip", "Duplicate a clip").to_string(),
        Op::AgregarClip { .. } => t("Clip nuevo", "New clip").to_string(),
        Op::QuitarClip { .. } => t("Quitar un clip", "Remove a clip").to_string(),
        Op::InsertosGrupo { .. } => t("Efectos de un bus", "Bus effects").to_string(),
        Op::Maestro { .. } => t("Maestro", "Master").to_string(),
        Op::Automatizar { parametro, puntos, .. } => {
            let que = match parametro {
                Parametro::Volumen => t("volumen", "volume"),
                Parametro::Pan => t("paneo", "pan"),
            };
            crate::tr!("Automatización de {que} ({} puntos)", "{que} automation ({} points)", puntos.len())
        }
        Op::RegionAjustes { apagada, .. } => match apagada {
            Some(true) => t("Apagar un tramo", "Bypass a span").to_string(),
            Some(false) => t("Encender un tramo", "Enable a span").to_string(),
            None => t("Mover un tramo", "Move a span").to_string(),
        },
        Op::Marcadores { marcadores } => crate::tr!("Marcadores ({})", "Markers ({})", marcadores.len()),
        Op::Ordenar { .. } => t("Ordenar las pistas", "Reorder tracks").to_string(),
        Op::Color { pista, .. } => crate::tr!("Color de la pista {pista}", "Track {pista} color"),
        Op::RenombrarGrupo { nombre, .. } => crate::tr!("Renombrar el bus a «{nombre}»", "Rename the bus to \"{nombre}\""),
        Op::Tonalidad { tonalidad } => crate::tr!("Tonalidad: {tonalidad}", "Key: {tonalidad}"),
        Op::Letra { .. } => t("Letra de la canción", "Song lyrics").to_string(),
    }
}

/// El aviso con el que el chat se niega mientras el Experto está activo.
pub fn aviso_candado() -> &'static str {
    crate::i18n::t(
        "🎛️ El estudio Experto está activo en este proyecto: el chat y el modo Pro quedan bloqueados para no tocar la sesión de música. Ciérralo (o úsalo desde su propio chat) para volver.",
        "🎛️ The Expert studio is active in this project: the chat and Pro mode are locked so the music session is not touched. Close it (or use its own chat) to come back.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pista(id: u32, nombre: &str) -> Pista {
        Pista { color: None, id, nombre: nombre.into(), archivo: format!("generado/{nombre}.wav"), origen: Origen::Separada, de: None, fuente: Some(nombre.into()), ganancia_db: 0.0, pan: 0.0, silencio: false, solo: false, grupo: None, insertos: Default::default() }
    }

    /// Cada edición se aplica y se DESHACE igual: el estado vuelve exactamente a como estaba, y
    /// rehacer lo devuelve a como quedó.
    #[test]
    fn cada_edicion_se_aplica_y_se_deshace_igual() {
        let mut s = Sesion::default();
        let mut estados = vec![s.estado()];
        let mut ops = vec![
            Op::AgregarPista { pista: pista(1, "drums") },
            Op::AgregarPista { pista: pista(2, "bass") },
            Op::AgregarPista { pista: pista(3, "vocals") },
            Op::CrearGrupo { grupo: Grupo { insertos: Default::default(), id: 4, nombre: "Ritmo".into(), ganancia_db: 0.0, pan: 0.0, silencio: false, solo: false } },
            Op::Agrupar { pistas: vec![1, 2], grupo: Some(4) },
            Op::Mezcla { pista: 3, ganancia_db: Some(3.0), pan: Some(-0.25), silencio: None, solo: None },
            Op::MezclaGrupo { grupo: 4, ganancia_db: Some(-2.0), pan: None, silencio: None, solo: Some(true) },
            Op::Region { region: Region { apagada: false, id: 5, pista: 1, desde: 5.0, hasta: 10.0, efecto: Efecto::Ganancia { db: 4.0, fundido: 0.03 } } },
            Op::Region { region: Region { apagada: false, id: 6, pista: 1, desde: 5.0, hasta: 10.0, efecto: Efecto::Eq { filtro: Filtro::Agudos, frecuencia: 6000.0, db: 5.0, q: 0.707, fundido: 0.03 } } },
            Op::Tempo { bpm: 92.0, compas: 4 },
            Op::Marcador { marcador: Marcador { segundo: 12.5, nombre: "estribillo".into() } },
            Op::Renombrar { pista: 3, nombre: "Voz".into() },
        ];
        for op in ops.drain(..) {
            s.aplicar(op).unwrap();
            estados.push(s.estado());
        }
        let e = s.estado();
        assert_eq!(e.pistas.iter().filter(|p| p.grupo == Some(4)).count(), 2);
        assert_eq!(e.pistas[2].nombre, "Voz");
        assert_eq!(e.regiones.len(), 2);
        assert_eq!((e.bpm, e.compas), (92.0, 4));
        // deshacer todo, paso a paso, vuelve por los mismos estados
        for k in (0..estados.len() - 1).rev() {
            assert!(s.deshacer());
            assert_eq!(s.estado(), estados[k], "deshacer el paso {}", k + 1);
        }
        assert!(!s.deshacer());
        // y rehacer, también
        for (k, esperado) in estados.iter().enumerate().skip(1) {
            assert!(s.rehacer());
            assert_eq!(&s.estado(), esperado, "rehacer el paso {k}");
        }
        // una operación nueva vacía «rehacer»
        s.deshacer();
        s.aplicar(Op::QuitarRegion { id: 5 }).unwrap();
        assert!(!s.rehacer());
        assert_eq!(s.nuevo_id(), 7);
    }

    /// Lo que no cuadra se rechaza y NO entra en la lista.
    #[test]
    fn lo_invalido_no_entra() {
        let mut s = Sesion::default();
        s.aplicar(Op::AgregarPista { pista: pista(1, "drums") }).unwrap();
        let malas = [
            Op::AgregarPista { pista: pista(1, "otra") },
            Op::Mezcla { pista: 9, ganancia_db: Some(1.0), pan: None, silencio: None, solo: None },
            Op::Mezcla { pista: 1, ganancia_db: Some(40.0), pan: None, silencio: None, solo: None },
            Op::Mezcla { pista: 1, ganancia_db: None, pan: Some(2.0), silencio: None, solo: None },
            Op::Agrupar { pistas: vec![1], grupo: Some(3) },
            Op::Region { region: Region { apagada: false, id: 2, pista: 1, desde: 10.0, hasta: 5.0, efecto: Efecto::Silencio { fundido: 0.03 } } },
            Op::Region { region: Region { apagada: false, id: 2, pista: 1, desde: 0.0, hasta: 5.0, efecto: Efecto::Eq { filtro: Filtro::Campana, frecuencia: 5.0, db: 0.0, q: 1.0, fundido: 0.03 } } },
            Op::Region { region: Region { apagada: false, id: 2, pista: 1, desde: 0.0, hasta: 5.0, efecto: Efecto::Reemplazo { archivo: " ".into(), descripcion: String::new(), fundido: 0.03 } } },
            Op::Tempo { bpm: 5.0, compas: 4 },
            Op::Insertos { pista: 1, insertos: Insertos { compresor: Some(Compresor { umbral_db: -20.0, razon: 50.0, ataque_ms: 5.0, liberacion_ms: 100.0, ganancia_db: 0.0 }), ..Default::default() } },
            Op::Insertos { pista: 7, insertos: Insertos::default() },
        ];
        for op in malas {
            assert!(s.aplicar(op.clone()).is_err(), "debió rechazarse: {op:?}");
        }
        assert_eq!(s.ops.len(), 1);
        // quitar una pista se lleva sus regiones; quitar un grupo deja sus pistas sueltas
        s.aplicar(Op::CrearGrupo { grupo: Grupo { insertos: Default::default(), id: 2, nombre: "G".into(), ganancia_db: 0.0, pan: 0.0, silencio: false, solo: false } }).unwrap();
        s.aplicar(Op::Agrupar { pistas: vec![1], grupo: Some(2) }).unwrap();
        s.aplicar(Op::Region { region: Region { apagada: false, id: 3, pista: 1, desde: 1.0, hasta: 2.0, efecto: Efecto::Silencio { fundido: 0.0 } } }).unwrap();
        s.aplicar(Op::QuitarGrupo { id: 2 }).unwrap();
        assert_eq!(s.estado().pistas[0].grupo, None);
        s.aplicar(Op::QuitarPista { id: 1 }).unwrap();
        assert!(s.estado().regiones.is_empty());
    }

    /// El JSON de la sesión es legible y estable (lo escribe la interfaz y el chat de Experto).
    #[test]
    fn el_json_va_y_vuelve() {
        let op: Op = serde_json::from_str(r#"{"op":"region","region":{"id":1,"pista":1,"desde":5,"hasta":10,"efecto":{"tipo":"ganancia","db":4}}}"#).unwrap();
        assert_eq!(op, Op::Region { region: Region { apagada: false, id: 1, pista: 1, desde: 5.0, hasta: 10.0, efecto: Efecto::Ganancia { db: 4.0, fundido: 0.03 } } });
        let mut s = Sesion::default();
        s.aplicar(Op::AgregarPista { pista: pista(1, "drums") }).unwrap();
        let t = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Sesion>(&t).unwrap(), s);
    }

    /// Importar: un WAV a 48 kHz dentro del proyecto se usa tal cual; uno mono de 16 bits a 44,1 kHz
    /// pasa por afconvert y queda estéreo, en coma flotante y a 48 kHz en `.gotv/experto/audio/`.
    #[test]
    fn importar_convierte_lo_que_hace_falta() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-experto-importar");
        std::fs::create_dir_all(d.join("musica")).unwrap();
        let s = Sesion::default();
        let x: crate::experto_dsp::Estereo = [vec![0.1; 4800], vec![-0.1; 4800]];
        crate::experto_dsp::escribir_wav(&d.join("musica/ya.wav"), &x, 48_000, false).unwrap();
        assert_eq!(importar(&d, &s, Path::new("musica/ya.wav")).unwrap(), "musica/ya.wav");
        // mono, 16 bits, 44,1 kHz, medio segundo
        let n = 22_050u32;
        let mut w = Vec::new();
        w.extend(b"RIFF");
        w.extend((36 + n * 2).to_le_bytes());
        w.extend(b"WAVEfmt ");
        w.extend(16u32.to_le_bytes());
        w.extend([1u8, 0, 1, 0]);
        w.extend(44_100u32.to_le_bytes());
        w.extend((44_100u32 * 2).to_le_bytes());
        w.extend([2u8, 0, 16, 0]);
        w.extend(b"data");
        w.extend((n * 2).to_le_bytes());
        for k in 0..n {
            w.extend((((k as f32 * 0.05).sin() * 8000.0) as i16).to_le_bytes());
        }
        std::fs::write(d.join("musica/viejo.wav"), w).unwrap();
        let rel = importar(&d, &s, Path::new("musica/viejo.wav")).unwrap();
        assert_eq!(rel, ".gotv/experto/audio/viejo.wav");
        let (c, f) = crate::experto_dsp::leer_wav(&d.join(&rel)).unwrap();
        assert_eq!((c.len(), f), (2, 48_000));
        assert!((c[0].len() as i64 - 24_000).abs() < 50, "medio segundo a 48 kHz: {}", c[0].len());
        assert!(importar(&d, &s, Path::new("no-existe.wav")).is_err());
    }

    /// Tono a pista nueva: la mezcla es IDÉNTICA a transponer en el sitio, la pista nueva es hermana (misma
    /// madre, parte, fader y paneo) y sólo suena en su tramo; la original queda en silencio ahí.
    #[test]
    fn el_tono_va_a_su_propia_pista_y_la_mezcla_no_cambia() {
        use crate::experto_dsp as dsp;
        let d = crate::CarpetaPrueba::sin_crear("gotv-experto-tono");
        std::fs::create_dir_all(&*d).unwrap();
        let fs = 48_000u32;
        let voz: Vec<f32> = (0..3 * fs as usize).map(|n| 0.3 * (2.0 * std::f32::consts::PI * 220.0 * n as f32 / fs as f32).sin()).collect();
        dsp::escribir_wav(&d.join("voz.wav"), &[voz.clone(), voz], fs, false).unwrap();
        let base = |s: &mut Sesion| {
            s.aplicar(Op::AgregarPista { pista: Pista { color: None, id: 1, nombre: "Voz".into(), archivo: "voz.wav".into(), origen: Origen::Separada, de: Some(7), fuente: Some("vocals".into()), ganancia_db: -2.0, pan: -0.3, silencio: false, solo: false, grupo: None, insertos: Default::default() } }).unwrap_or_else(|_| {
                // la madre 7 no existe en esta sesión mínima: sin madre
                s.aplicar(Op::AgregarPista { pista: Pista { color: None, id: 1, nombre: "Voz".into(), archivo: "voz.wav".into(), origen: Origen::Separada, de: None, fuente: Some("vocals".into()), ganancia_db: -2.0, pan: -0.3, silencio: false, solo: false, grupo: None, insertos: Default::default() } }).unwrap()
            });
        };
        let (mut a, mut b) = (Sesion::default(), Sesion::default());
        base(&mut a);
        base(&mut b);
        a.aplicar(Op::Region { region: Region { apagada: false, id: 2, pista: 1, desde: 1.0, hasta: 2.0, efecto: Efecto::Tono { semitonos: 2.0, fundido: 0.03 } } }).unwrap();
        let nueva = tono_a_pista(&d, &mut b, 1, 1.0, 2.0, 2.0, 0.03).unwrap();
        let eb = b.estado();
        let p = eb.pistas.iter().find(|p| p.id == nueva).unwrap();
        assert!(p.nombre.contains("+2 st") && p.fuente.as_deref() == Some("vocals") && p.ganancia_db == -2.0 && p.pan == -0.3);
        let (ma, _) = dsp::mezclar(&a.estado(), fs, &d).unwrap();
        let (mb, finales) = dsp::mezclar(&eb, fs, &d).unwrap();
        let err = (0..2).flat_map(|c| ma[c].iter().zip(&mb[c]).map(|(x, y)| (x - y).abs())).fold(0f32, f32::max);
        assert!(err < 1e-5, "la mezcla debe ser la misma: {err}");
        // la pista nueva sólo suena en su tramo
        let fuera = finales[1][0][..(0.9 * fs as f64) as usize].iter().chain(&finales[1][0][(2.1 * fs as f64) as usize..]).fold(0f32, |m, v| m.max(v.abs()));
        assert!(fuera == 0.0, "fuera del tramo la pista nueva calla: {fuera}");
        // se deshace en dos pasos (la pista y el silencio) y vuelve a como estaba
        b.deshacer();
        b.deshacer();
        assert_eq!(b.estado().pistas.len(), 1);
    }

    /// Herramienta (a mano): en un proyecto REAL (`GOTV_EXPERTO_PROYECTO`), pasa cada región de tono «en el
    /// sitio» a su propia pista.
    #[test]
    #[ignore]
    fn pasar_tonos_a_pistas_en_un_proyecto_real() {
        let Some(root) = std::env::var_os("GOTV_EXPERTO_PROYECTO").map(std::path::PathBuf::from) else { return };
        let mut s = cargar(&root);
        let tonos: Vec<Region> = s.estado().regiones.into_iter().filter(|r| matches!(r.efecto, Efecto::Tono { .. })).collect();
        for r in tonos {
            let Efecto::Tono { semitonos, fundido } = r.efecto else { continue };
            s.aplicar(Op::QuitarRegion { id: r.id }).unwrap();
            let id = tono_a_pista(&root, &mut s, r.pista, r.desde, r.hasta, semitonos, fundido).unwrap();
            eprintln!("región {} → pista {id}", r.id);
        }
        guardar(&root, &s).unwrap();
    }

    /// El candado es por proyecto, sobrevive en disco y cerrarlo no borra la sesión.
    #[test]
    fn el_candado_es_por_proyecto() {
        let a = crate::CarpetaPrueba::sin_crear("gotv-experto-a");
        let b = crate::CarpetaPrueba::sin_crear("gotv-experto-b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        assert!(!bloqueado(&a) && !bloqueado(&b));
        let mut s = activar(&a, true).unwrap();
        s.aplicar(Op::AgregarPista { pista: pista(1, "drums") }).unwrap();
        guardar(&a, &s).unwrap();
        assert!(bloqueado(&a) && !bloqueado(&b));
        activar(&a, false).unwrap();
        assert!(!bloqueado(&a));
        assert_eq!(cargar(&a).estado().pistas.len(), 1, "cerrar no borra la sesión");
    }
}
