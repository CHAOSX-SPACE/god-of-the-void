//! Ola EXPERTO II · X2-2 — los efectos del canal que faltaban: EQ paramétrico, limitador, saturación,
//! de-esser, chorus y ancho estéreo. Todo en Rust, sin GPU y determinista (lo que se oye es lo que se
//! exporta). Cada uno tiene su prueba con una señal conocida.
use crate::experto::{Banda, Chorus, DeEsser, Filtro, Limitador, Saturacion};
use crate::experto_dsp::{Biquad, Estereo};

/// El coeficiente de un suavizado exponencial de `ms` milisegundos.
fn coef(ms: f64, fs: u32) -> f64 {
    (-1.0 / (ms.max(0.01) * 1e-3 * fs as f64)).exp()
}

/// EQ paramétrico: las bandas encendidas, en cascada (cada una, un biquad de Web Audio ya juzgado).
pub fn ecualizar(x: &mut Estereo, bandas: &[Banda], fs: u32) {
    for b in bandas.iter().filter(|b| !b.apagada) {
        let f = Biquad::de(b.filtro, b.frecuencia as f64, b.db as f64, b.q as f64, fs as f64);
        for c in x.iter_mut() {
            *c = f.filtrar(c);
        }
    }
}

/// La respuesta en dB del EQ en `frecuencia` (para pintar la curva: la MISMA fórmula que suena).
pub fn respuesta_eq(bandas: &[Banda], frecuencia: f64, fs: u32) -> f64 {
    let w = 2.0 * std::f64::consts::PI * frecuencia / fs as f64;
    let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
    bandas
        .iter()
        .filter(|b| !b.apagada)
        .map(|b| {
            let f = Biquad::de(b.filtro, b.frecuencia as f64, b.db as f64, b.q as f64, fs as f64);
            // H(e^{jw}) = (b0 + b1 e^{-jw} + b2 e^{-2jw}) / (1 + a1 e^{-jw} + a2 e^{-2jw})
            let (nr, ni) = (f.b0 + f.b1 * c1 + f.b2 * c2, -(f.b1 * s1 + f.b2 * s2));
            let (dr, di) = (1.0 + f.a1 * c1 + f.a2 * c2, -(f.a1 * s1 + f.a2 * s2));
            10.0 * ((nr * nr + ni * ni) / (dr * dr + di * di)).max(1e-30).log10()
        })
        .sum()
}

/// Limitador de pared con 5 ms de anticipación. La ganancia que hace falta en cada muestra
/// (`techo / pico`) se adelanta con un mínimo deslizante, se suelta despacio (liberación) y se suaviza
/// con una media de la misma ventana: cada valor promediado es ≤ la ganancia necesaria en esa muestra,
/// así que NINGUNA muestra pasa del techo, y la ganancia nunca salta (sin clics ni distorsión de borde).
pub fn limitar(x: &mut Estereo, l: &Limitador, fs: u32) {
    let n = x[0].len().min(x[1].len());
    if n == 0 {
        return;
    }
    let techo = 10f64.powf(l.techo_db as f64 / 20.0);
    let empuje = 10f64.powf(l.empuje_db as f64 / 20.0);
    let v = ((0.005 * fs as f64) as usize).max(1);
    let necesaria: Vec<f64> = (0..n)
        .map(|i| {
            let p = (x[0][i].abs().max(x[1][i].abs()) as f64) * empuje;
            if p > techo {
                techo / p
            } else {
                1.0
            }
        })
        .collect();
    // mínimo deslizante hacia delante: m[i] = min(necesaria[i ..= i+v]) (cola monótona)
    let mut minimo = vec![1f64; n];
    let mut cola: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    let mut j = 0;
    for (i, m) in minimo.iter_mut().enumerate() {
        while j < n && j <= i + v {
            while cola.back().is_some_and(|&k| necesaria[k] >= necesaria[j]) {
                cola.pop_back();
            }
            cola.push_back(j);
            j += 1;
        }
        while cola.front().is_some_and(|&k| k < i) {
            cola.pop_front();
        }
        *m = cola.front().map(|&k| necesaria[k]).unwrap_or(1.0);
    }
    // liberación: baja al instante, sube despacio
    let c = coef(l.liberacion_ms as f64, fs);
    let mut g = 1f64;
    for m in minimo.iter_mut() {
        g = m.min(c * g + (1.0 - c));
        *m = g;
    }
    // media de caja sobre [i−v, i]
    let mut suma = 0f64;
    for i in 0..n {
        suma += minimo[i];
        if i > v {
            suma -= minimo[i - v - 1];
        }
        let cuantos = (i + 1).min(v + 1) as f64;
        let k = (suma / cuantos).min(necesaria[i]) * empuje;
        for ch in x.iter_mut() {
            ch[i] = ((ch[i] as f64 * k).clamp(-techo, techo)) as f32;
        }
    }
}

/// Saturación: `tanh(k·x) / tanh(k)` con `k = 10^(empuje/20) − 1`. Con empuje 0 no hace nada; al subir,
/// lo flojo gana cuerpo y los picos se redondean sin pasar de la escala completa.
pub fn saturar(x: &mut Estereo, s: &Saturacion) {
    let k = 10f64.powf(s.empuje_db.max(0.0) as f64 / 20.0) - 1.0;
    let m = s.mezcla.clamp(0.0, 1.0) as f64;
    if k < 1e-6 || m == 0.0 {
        return;
    }
    let norma = k.tanh();
    for c in x.iter_mut() {
        for v in c.iter_mut() {
            let seco = *v as f64;
            *v = (seco * (1.0 - m) + (k * seco).tanh() / norma * m) as f32;
        }
    }
}

/// De-esser con estante dinámico. El DETECTOR mide el pico de los agudos (paso alto en `frecuencia`,
/// ataque 1 ms, liberación 60 ms). Lo que pase del umbral cruza la señal hacia su versión con un
/// estante de agudos de −`reduccion_db`: a tope es exactamente ese estante, y por debajo del umbral
/// la señal sale intacta. (Restar la banda alta y volver a sumarla atenuada NO sirve: cerca del cruce
/// las dos mitades están desfasadas y apenas baja; medido, −1,7 dB donde se pedían −9.)
pub fn desesear(x: &mut Estereo, d: &DeEsser, fs: u32) {
    let tope = d.reduccion_db.max(0.0) as f64;
    if tope == 0.0 {
        return;
    }
    let detector = Biquad::de(Filtro::PasoAlto, d.frecuencia as f64, 0.0, std::f64::consts::FRAC_1_SQRT_2, fs as f64);
    let altos: [Vec<f32>; 2] = [detector.filtrar(&x[0]), detector.filtrar(&x[1])];
    let estante = Biquad::de(Filtro::Agudos, d.frecuencia as f64, -tope, std::f64::consts::FRAC_1_SQRT_2, fs as f64);
    let bajado: [Vec<f32>; 2] = [estante.filtrar(&x[0]), estante.filtrar(&x[1])];
    let (at, li) = (coef(1.0, fs), coef(60.0, fs));
    let umbral = d.umbral_db as f64;
    let mut env = 0f64;
    for i in 0..x[0].len().min(x[1].len()) {
        let p = altos[0][i].abs().max(altos[1][i].abs()) as f64;
        let k = if p > env { at } else { li };
        env = k * env + (1.0 - k) * p;
        let nivel = 20.0 * env.max(1e-12).log10();
        let w = ((nivel - umbral) / tope).clamp(0.0, 1.0);
        for c in 0..2 {
            x[c][i] = (x[c][i] as f64 + w * (bajado[c][i] as f64 - x[c][i] as f64)) as f32;
        }
    }
}

/// Chorus: un retardo de 7 ms que oscila ± `profundidad_ms / 2`; el derecho va un cuarto de ciclo
/// por delante (ensancha). Sale seco·(1 − mezcla) + modulado·mezcla.
pub fn corear(x: &mut Estereo, c: &Chorus, fs: u32) {
    let m = c.mezcla.clamp(0.0, 1.0) as f64;
    if m == 0.0 {
        return;
    }
    let fsf = fs as f64;
    let (base, prof) = (0.007 * fsf, c.profundidad_ms as f64 * 1e-3 * fsf / 2.0);
    let w = 2.0 * std::f64::consts::PI * c.velocidad_hz as f64 / fsf;
    for (canal, ch) in x.iter_mut().enumerate() {
        let seco = ch.clone();
        let fase = canal as f64 * std::f64::consts::FRAC_PI_2;
        for (i, v) in ch.iter_mut().enumerate() {
            let retardo = (base + prof * (1.0 + (w * i as f64 + fase).sin())).max(1.0);
            let t = i as f64 - retardo;
            let mojado = if t < 0.0 {
                0.0
            } else {
                let (k, f) = (t.floor() as usize, t - t.floor());
                seco[k] as f64 * (1.0 - f) + seco.get(k + 1).copied().unwrap_or(0.0) as f64 * f
            };
            *v = (seco[i] as f64 * (1.0 - m) + mojado * m) as f32;
        }
    }
}

/// Ancho estéreo en medio/lado: el lado (I − D) se multiplica por `ancho` (0 = mono, 1 = igual).
pub fn ensanchar(x: &mut Estereo, ancho: f32) {
    let a = ancho.clamp(0.0, 2.0) as f64;
    let [l, r] = x;
    for (i, d) in l.iter_mut().zip(r.iter_mut()) {
        let (medio, lado) = ((*i as f64 + *d as f64) / 2.0, (*i as f64 - *d as f64) / 2.0 * a);
        *i = (medio + lado) as f32;
        *d = (medio - lado) as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: u32 = 48_000;

    fn seno(f: f64, a: f32, s: f64) -> Vec<f32> {
        (0..(s * FS as f64) as usize).map(|n| a * (2.0 * std::f64::consts::PI * f * n as f64 / FS as f64).sin() as f32).collect()
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
    }

    fn pico(x: &Estereo) -> f32 {
        x.iter().flat_map(|c| c.iter()).fold(0f32, |m, v| m.max(v.abs()))
    }

    /// Ruido determinista en −1…1 (congruencial).
    fn ruido(n: usize, semilla: u64) -> Vec<f32> {
        let mut s = semilla;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                ((s >> 33) as f64 / (1u64 << 31) as f64 - 1.0) as f32
            })
            .collect()
    }

    #[test]
    fn el_limitador_nunca_pasa_su_techo_y_no_toca_lo_flojo() {
        // ruido con golpes de hasta 4× la escala y un techo de −1 dB
        let mut x: Estereo = [ruido(FS as usize, 1), ruido(FS as usize, 2)];
        for c in x.iter_mut() {
            for (i, v) in c.iter_mut().enumerate() {
                *v *= if i % 5000 < 40 { 4.0 } else { 0.6 };
            }
        }
        let l = Limitador { techo_db: -1.0, empuje_db: 0.0, liberacion_ms: 80.0 };
        limitar(&mut x, &l, FS);
        let techo = 10f32.powf(-1.0 / 20.0);
        assert!(pico(&x) <= techo + 1e-6, "pico {} > techo {techo}", pico(&x));
        // con empuje, lo mismo: el techo manda
        let mut y: Estereo = [ruido(FS as usize, 3), ruido(FS as usize, 4)];
        limitar(&mut y, &Limitador { techo_db: -6.0, empuje_db: 18.0, liberacion_ms: 30.0 }, FS);
        assert!(pico(&y) <= 10f32.powf(-6.0 / 20.0) + 1e-6);
        // lo que no llega al techo sale idéntico
        let flojo: Estereo = [seno(440.0, 0.3, 0.5), seno(440.0, 0.3, 0.5)];
        let mut z = flojo.clone();
        limitar(&mut z, &l, FS);
        assert_eq!(z, flojo);
        // y la ganancia no salta: entre muestras vecinas cambia menos de 1 % (sin clics)
        let mut golpe: Estereo = [vec![0.5; 9600], vec![0.5; 9600]];
        for c in golpe.iter_mut() {
            c[4800] = 3.0;
        }
        let antes = golpe.clone();
        limitar(&mut golpe, &l, FS);
        let g: Vec<f64> = (0..9600).map(|i| golpe[0][i] as f64 / antes[0][i] as f64).collect();
        let salto = g.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f64::max);
        assert!(salto < 0.01, "salto de ganancia {salto}");
    }

    #[test]
    fn la_saturacion_sin_empuje_no_hace_nada_y_con_empuje_anade_armonicos() {
        let x: Estereo = [seno(1000.0, 0.8, 0.5), seno(1000.0, 0.8, 0.5)];
        let mut y = x.clone();
        saturar(&mut y, &Saturacion { empuje_db: 0.0, mezcla: 1.0 });
        assert_eq!(y, x);
        let mut z = x.clone();
        saturar(&mut z, &Saturacion { empuje_db: 18.0, mezcla: 1.0 });
        assert!(pico(&z) <= 1.0);
        assert!(rms(&z[0]) > rms(&x[0]) * 1.1, "con empuje gana cuerpo");
        // el tercer armónico (3 kHz) aparece: correlación con un seno de 3 kHz
        let arm = |s: &[f32]| {
            let (mut a, mut b) = (0f64, 0f64);
            for (n, v) in s.iter().enumerate() {
                let w = 2.0 * std::f64::consts::PI * 3000.0 * n as f64 / FS as f64;
                a += *v as f64 * w.sin();
                b += *v as f64 * w.cos();
            }
            a.hypot(b) / s.len() as f64
        };
        assert!(arm(&z[0]) > 100.0 * arm(&x[0]).max(1e-9), "{} vs {}", arm(&z[0]), arm(&x[0]));
        // la mezcla a 0 lo deja seco
        let mut w = x.clone();
        saturar(&mut w, &Saturacion { empuje_db: 18.0, mezcla: 0.0 });
        assert_eq!(w, x);
    }

    #[test]
    fn el_deesser_baja_la_ese_y_deja_el_cuerpo() {
        let db = |a: &[f32], b: &[f32]| 20.0 * (rms(&a[a.len() / 2..]) / rms(&b[b.len() / 2..])).log10();
        let d = DeEsser { frecuencia: 4000.0, umbral_db: -30.0, reduccion_db: 9.0 };
        let ese: Estereo = [seno(12000.0, 0.5, 0.5), seno(12000.0, 0.5, 0.5)];
        let mut y = ese.clone();
        desesear(&mut y, &d, FS);
        let baja = db(&y[0], &ese[0]);
        assert!((-9.3..=-8.0).contains(&baja), "la ese bajó {baja} dB");
        let cuerpo: Estereo = [seno(200.0, 0.5, 0.5), seno(200.0, 0.5, 0.5)];
        let mut z = cuerpo.clone();
        desesear(&mut z, &d, FS);
        assert!(db(&z[0], &cuerpo[0]).abs() < 0.05, "el cuerpo cambió {} dB", db(&z[0], &cuerpo[0]));
        // por debajo del umbral, nada
        let floja: Estereo = [seno(12000.0, 0.01, 0.5), seno(12000.0, 0.01, 0.5)];
        let mut w = floja.clone();
        desesear(&mut w, &d, FS);
        assert!(db(&w[0], &floja[0]).abs() < 0.05);
    }

    #[test]
    fn el_chorus_ensancha_y_a_cero_no_hace_nada() {
        let x: Estereo = [seno(440.0, 0.5, 1.0), seno(440.0, 0.5, 1.0)];
        let mut y = x.clone();
        corear(&mut y, &Chorus { velocidad_hz: 0.8, profundidad_ms: 4.0, mezcla: 0.0 }, FS);
        assert_eq!(y, x);
        let mut z = x.clone();
        corear(&mut z, &Chorus { velocidad_hz: 0.8, profundidad_ms: 4.0, mezcla: 0.5 }, FS);
        assert!(z[0] != x[0]);
        let dif: f64 = z[0].iter().zip(&z[1]).map(|(a, b)| (*a as f64 - *b as f64).abs()).sum::<f64>() / z[0].len() as f64;
        assert!(dif > 0.01, "los dos canales deben diferir: {dif}");
        assert!(pico(&z) <= 0.55);
    }

    #[test]
    fn el_ancho_va_de_mono_al_doble() {
        let x: Estereo = [seno(440.0, 0.5, 0.1), seno(660.0, 0.3, 0.1)];
        let mut igual = x.clone();
        ensanchar(&mut igual, 1.0);
        for c in 0..2 {
            for (a, b) in igual[c].iter().zip(&x[c]) {
                assert!((a - b).abs() < 1e-6);
            }
        }
        let mut mono = x.clone();
        ensanchar(&mut mono, 0.0);
        assert_eq!(mono[0], mono[1]);
        // una señal mono no cambia por mucho que se ensanche
        let m: Estereo = [seno(440.0, 0.5, 0.1), seno(440.0, 0.5, 0.1)];
        let mut ancha = m.clone();
        ensanchar(&mut ancha, 2.0);
        assert_eq!(ancha, m);
    }

    #[test]
    fn el_eq_por_bandas_es_la_cascada_y_su_curva_es_lo_que_suena() {
        let bandas = [
            Banda { filtro: Filtro::PasoAlto, frecuencia: 80.0, db: 0.0, q: 0.707, apagada: false },
            Banda { filtro: Filtro::Campana, frecuencia: 1000.0, db: 6.0, q: 1.4, apagada: false },
            Banda { filtro: Filtro::Agudos, frecuencia: 8000.0, db: -4.0, q: 0.707, apagada: false },
            Banda { filtro: Filtro::Muesca, frecuencia: 3000.0, db: 0.0, q: 8.0, apagada: true },
        ];
        // la curva pintada contra lo medido con senos, en varias frecuencias
        for f in [40.0, 100.0, 500.0, 1000.0, 2000.0, 3000.0, 6000.0, 12000.0] {
            let x: Estereo = [seno(f, 0.25, 1.0), seno(f, 0.25, 1.0)];
            let mut y = x.clone();
            ecualizar(&mut y, &bandas, FS);
            let medido = 20.0 * (rms(&y[0][FS as usize / 2..]) / rms(&x[0][FS as usize / 2..])).log10();
            let curva = respuesta_eq(&bandas, f, FS);
            assert!((medido - curva).abs() < 0.02, "{f} Hz: medido {medido} dB, curva {curva} dB");
        }
        assert!((respuesta_eq(&bandas, 1000.0, FS) - 6.0).abs() < 0.3);
        // la muesca (encendida) quita su frecuencia y el paso banda deja la suya
        let muesca = [Banda { filtro: Filtro::Muesca, frecuencia: 3000.0, db: 0.0, q: 8.0, apagada: false }];
        assert!(respuesta_eq(&muesca, 3000.0, FS) < -60.0);
        assert!(respuesta_eq(&muesca, 300.0, FS).abs() < 0.1);
        let banda = [Banda { filtro: Filtro::PasoBanda, frecuencia: 1000.0, db: 0.0, q: 2.0, apagada: false }];
        assert!(respuesta_eq(&banda, 1000.0, FS).abs() < 0.01);
        assert!(respuesta_eq(&banda, 100.0, FS) < -12.0);
    }
}
