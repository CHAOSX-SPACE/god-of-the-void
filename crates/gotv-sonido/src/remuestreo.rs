//! 24 kHz → 16 kHz (×2/3) para el micrófono de [`crate::Duplex`]: VoiceProcessingIO exige la MISMA
//! frecuencia a la entrada y a la salida (medido: 24k/16k da OSStatus −10875) y la voz sale a 24 kHz.
//!
//! Polifásico: subir ×2 (ceros), FIR paso bajo a 48 kHz (sinc con ventana Blackman, corte 7,2 kHz:
//! por debajo de los 8 kHz de Nyquist del destino, así nada se dobla) y bajar ×3. Sólo se calculan
//! las muestras que quedan. Con estado: los bloques se encadenan sin costuras.

const TAPS: usize = 144;
const CORTE_HZ: f64 = 7200.0;
const ALTA: f64 = 48000.0;

pub struct De24a16 {
    h: Vec<f32>,
    /// historia a 24 kHz (lo necesario para el FIR)
    hist: Vec<f32>,
    /// fase: posición a 48 kHz de la próxima salida, relativa al inicio de `hist`·2
    pos: usize,
}

impl Default for De24a16 {
    fn default() -> Self {
        Self::nuevo()
    }
}

impl De24a16 {
    pub fn nuevo() -> De24a16 {
        let fc = CORTE_HZ / ALTA;
        let m = (TAPS - 1) as f64;
        let mut h: Vec<f64> = (0..TAPS)
            .map(|n| {
                let x = n as f64 - m / 2.0;
                let sinc = if x == 0.0 { 2.0 * fc } else { (2.0 * std::f64::consts::PI * fc * x).sin() / (std::f64::consts::PI * x) };
                let w = 0.42 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / m).cos() + 0.08 * (4.0 * std::f64::consts::PI * n as f64 / m).cos();
                sinc * w
            })
            .collect();
        // ganancia 2 en continua (los ceros de subir ×2 la parten a la mitad)
        let s: f64 = h.iter().sum();
        h.iter_mut().for_each(|x| *x *= 2.0 / s);
        De24a16 { h: h.into_iter().map(|x| x as f32).collect(), hist: vec![0.0; TAPS / 2 + 1], pos: TAPS }
    }

    /// Bloque a 24 kHz → lo que sale a 16 kHz (2 de cada 3, con la fase arrastrada).
    pub fn procesar(&mut self, x: &[f32]) -> Vec<f32> {
        self.hist.extend_from_slice(x);
        let total_alta = self.hist.len() * 2; // índices a 48 kHz disponibles
        let mut y = Vec::with_capacity(x.len() * 2 / 3 + 2);
        // salida en el índice `pos` (a 48 kHz): y = Σ h[k]·up[pos−k], up[j] = x[j/2] si j es par
        while self.pos < total_alta {
            let mut s = 0.0f32;
            // k con la misma paridad que pos (sólo ahí up ≠ 0)
            let mut k = self.pos % 2;
            while k < TAPS {
                let j = self.pos - k; // ≥ pos − TAPS + 1 ≥ 0 por construcción
                s += self.h[k] * self.hist[j / 2];
                k += 2;
            }
            y.push(s);
            self.pos += 3;
        }
        // descartar historia que ya no hace falta (se conserva TAPS a 48 kHz = TAPS/2 a 24 kHz)
        let necesario = (self.pos.saturating_sub(TAPS)) / 2;
        if necesario > 0 {
            self.hist.drain(..necesario);
            self.pos -= necesario * 2;
        }
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tono(f: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (2.0 * std::f32::consts::PI * f * i as f32 / 24000.0).sin()).collect()
    }
    fn amplitud(y: &[f32]) -> f32 {
        let s = &y[y.len() / 4..];
        (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt() * std::f32::consts::SQRT_2
    }

    #[test]
    fn deja_pasar_la_voz_y_corta_lo_que_se_doblaria() {
        for (f, cerca_de_1) in [(300.0, true), (1000.0, true), (3400.0, true), (10000.0, false), (11000.0, false)] {
            let mut r = De24a16::nuevo();
            let x = tono(f, 24000);
            // por bloques de tamaños raros: sin costuras
            let mut y = Vec::new();
            for c in x.chunks(517) {
                y.extend(r.procesar(c));
            }
            assert!((y.len() as i64 - 16000).abs() <= 2, "{f} Hz: {} muestras", y.len());
            let a = amplitud(&y);
            if cerca_de_1 {
                assert!((a - 1.0).abs() < 0.02, "{f} Hz pasa con amplitud {a}");
            } else {
                assert!(a < 0.01, "{f} Hz debía cortarse: {a}");
            }
        }
    }

    #[test]
    fn por_bloques_igual_que_de_una_vez() {
        let x = tono(1234.0, 9000);
        let mut a = De24a16::nuevo();
        let entero = a.procesar(&x);
        let mut b = De24a16::nuevo();
        let trozos: Vec<f32> = x.chunks(160).flat_map(|c| b.procesar(c)).collect();
        assert_eq!(entero.len(), trozos.len());
        let dif = entero.iter().zip(&trozos).map(|(p, q)| (p - q).abs()).fold(0.0, f32::max);
        assert!(dif < 1e-6, "{dif}");
    }
}
