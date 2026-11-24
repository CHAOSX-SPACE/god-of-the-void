//! F9.0 · generación EN LOTE: un paso de decodificación para B conversaciones a la vez sobre los MISMOS
//! pesos. Generar un token está limitado por leer los pesos de la memoria; si las B conversaciones pasan
//! juntas por cada proyección y cada MLP, los pesos se leen UNA vez por paso para todas. Lo que no se
//! comparte —la atención con su caché KV y el estado recurrente del híbrido— va por conversación, cada una
//! con su largo (no hace falta rellenar ni enmascarar).
//!
//! Es el MISMO cálculo que `Modelo::tronco` con un token (sin imágenes): las proyecciones se aplican a
//! [1, B, D] en vez de [1, 1, D] y la parte de cada conversación es la de `atencion`/`delta` con l = 1.
//! La medición de abajo comprueba primero que da los mismos logits que de a uno.
//!
//! F9.0 decide con esto si el planificador local trabaja por lotes o por turnos (`gotv_core::enjambre`). El
//! hilo del modelo (`Trabajador`) aún no lo usa: integrarlo (varias cachés vivas, juntar los trabajos que
//! esperan) es el paso 4 de F9 y sólo tiene sentido si F9.0 da la ganancia.

use super::*;

impl Modelo {
    /// Un paso para `ids.len()` conversaciones: el token nuevo de cada una (`ids[i]`) con su caché
    /// (`caches[i]`, que avanza un token). Devuelve los logits [B, vocabulario].
    pub fn paso_lote(&self, ids: &[u32], caches: &mut [&mut Cache]) -> Result<Array> {
        let b = ids.len() as i32;
        if b == 0 || ids.len() != caches.len() {
            bail_tr!("paso_lote: {} tokens y {} cachés", "paso_lote: {} tokens and {} caches", ids.len(), caches.len());
        }
        let eps = self.cfg.rms_norm_eps;
        let offsets: Vec<i32> = caches.iter().map(|c| c.largo).collect();
        let mut x = self.embed.filas(&Array::from_slice(ids, &[1, b]))?;
        for (k, capa) in self.capas.iter().enumerate() {
            let h = fast::rms_norm(&x, Some(&capa.norma_in), eps).m()?;
            let r = match &capa.mezcla {
                Mezcla::Atencion(at) => self.atencion_lote(at, &h, &offsets, caches, k)?,
                Mezcla::Lineal(d) => self.delta_lote(d, &h, caches, k)?,
            };
            x = ops::add(&x, &r).m()?;
            let h = fast::rms_norm(&x, Some(&capa.norma_post), eps).m()?;
            let m = match &capa.mlp {
                Mlp::Densa { puerta, arriba, abajo } => abajo.aplicar(&(self.f.swiglu.borrow_mut())((&puerta.aplicar(&h)?, &arriba.aplicar(&h)?)).m()?)?,
                // B tokens de B conversaciones: el enrutado es por token, igual que un trozo de B tokens
                Mlp::Moe(moe) => self.expertos(moe, &h, b)?,
            };
            x = ops::add(&x, &m).m()?;
        }
        for c in caches.iter_mut() {
            c.largo += 1;
        }
        let x = fast::rms_norm(&x, Some(&self.norma), eps).m()?;
        let logits = match &self.cabeza {
            Some(c) => c.aplicar(&x)?,
            None => self.embed.lineal.aplicar(&x)?,
        };
        ops::reshape(&logits, &[b, -1]).m()
    }

    /// La atención completa de B conversaciones: q/k/v/o juntas; RoPE, caché y atención, cada una con su
    /// posición (`offsets[i]`).
    fn atencion_lote(&self, at: &Atencion, h: &Array, offsets: &[i32], caches: &mut [&mut Cache], k: usize) -> Result<Array> {
        let (nh, nkv, hd) = (self.cfg.num_attention_heads, self.cfg.num_key_value_heads, self.head_dim);
        let eps = self.cfg.rms_norm_eps;
        let escala = 1.0 / (hd as f32).sqrt();
        let qp = at.q.aplicar(h)?;
        let kp = at.k.aplicar(h)?;
        let vp = at.v.aplicar(h)?;
        let mut salidas = Vec::with_capacity(offsets.len());
        for (i, c) in caches.iter_mut().enumerate() {
            let fila = |a: &Array| a.try_index((.., i as i32..i as i32 + 1, ..)).m();
            let qpi = fila(&qp)?;
            let (mut q, compuerta) = if at.compuerta {
                let partes = ops::split_at_indices(ops::reshape(&qpi, &[1, 1, nh, 2 * hd]).m()?, &[hd], -1).m()?;
                (partes[0].clone(), Some(ops::reshape(&partes[1], &[1, 1, nh * hd]).m()?))
            } else {
                (ops::reshape(&qpi, &[1, 1, nh, hd]).m()?, None)
            };
            let mut kk = ops::reshape(fila(&kp)?, &[1, 1, nkv, hd]).m()?;
            let vv = ops::reshape(fila(&vp)?, &[1, 1, nkv, hd]).m()?;
            if let Some(qn) = &at.q_norma {
                q = fast::rms_norm(&q, Some(qn), eps).m()?;
            }
            if let Some(kn) = &at.k_norma {
                kk = fast::rms_norm(&kk, Some(kn), eps).m()?;
            }
            let q = ops::transpose_axes(&q, &[0, 2, 1, 3]).m()?;
            let kk = ops::transpose_axes(&kk, &[0, 2, 1, 3]).m()?;
            let vv = ops::transpose_axes(&vv, &[0, 2, 1, 3]).m()?;
            let off = offsets[i];
            let (q, kk) = if self.rope_periodos.is_some() {
                (
                    fast::rope(&q, self.rope_dims, false, None, 1.0, off, self.rope_periodos.as_ref()).m()?,
                    fast::rope(&kk, self.rope_dims, false, None, 1.0, off, self.rope_periodos.as_ref()).m()?,
                )
            } else {
                (
                    fast::rope(&q, self.rope_dims, false, self.cfg.rope_theta, 1.0, off, None::<&Array>).m()?,
                    fast::rope(&kk, self.rope_dims, false, self.cfg.rope_theta, 1.0, off, None::<&Array>).m()?,
                )
            };
            let (kc, vc) = escribir_kv(&mut c.capas[k], &kk, &vv, off, 1)?;
            // un solo token por conversación: sin máscara (ve toda su caché)
            let mascara: Option<ScaledDotProductAttentionMask> = None;
            let a = fast::scaled_dot_product_attention(&q, &kc, &vc, escala, mascara, None::<&Array>).m()?;
            let mut a = ops::reshape(ops::transpose_axes(&a, &[0, 2, 1, 3]).m()?, &[1, 1, nh * hd]).m()?;
            if let Some(g) = compuerta {
                a = (self.f.puerta.borrow_mut())((&a, &g)).m()?;
            }
            salidas.push(a);
        }
        at.o.aplicar(&ops::concatenate(&salidas, 1).m()?)
    }

    /// La Gated DeltaNet de B conversaciones: proyecciones juntas; conv y recurrencia, cada una con su
    /// estado (el mismo kernel que `delta`, con T = 1).
    fn delta_lote(&self, d: &DeltaNet, h: &Array, caches: &mut [&mut Cache], k: usize) -> Result<Array> {
        let c = &self.cfg;
        let (hk, hv) = (c.linear_num_key_heads.unwrap_or(16), c.linear_num_value_heads.unwrap_or(32));
        let (dk, dv) = (c.linear_key_head_dim.unwrap_or(128), c.linear_value_head_dim.unwrap_or(128));
        let kc = c.linear_conv_kernel_dim.unwrap_or(4);
        let (key_dim, value_dim) = (hk * dk, hv * dv);
        let conv_dim = 2 * key_dim + value_dim;
        let qkv = d.qkv.aplicar(h)?;
        let z = d.z.aplicar(h)?;
        let bb = d.b.aplicar(h)?;
        let aa = d.a.aplicar(h)?;
        let kernel = self.kernel_delta.as_ref().ok_or_else(|| anyhow_tr!("sin kernel delta", "no delta kernel"))?;
        let inv = (dk as f32).powf(-0.5);
        let esc = |x: f32, t: Dtype| Array::from_f32(x).as_dtype(t);
        let mut salidas = Vec::with_capacity(caches.len());
        for (i, cache) in caches.iter_mut().enumerate() {
            let cc = &mut cache.capas[k];
            let fila = |a: &Array| a.try_index((.., i as i32..i as i32 + 1, ..)).m();
            let qkv_i = fila(&qkv)?;
            let z_i = ops::reshape(fila(&z)?, &[1, 1, hv, dv]).m()?;
            let b_i = fila(&bb)?;
            let a_i = fila(&aa)?;
            let previo = match cc.conv.take() {
                Some(p) => p,
                None => ops::zeros::<f32>(&[1, kc - 1, conv_dim]).m()?.as_dtype(qkv_i.dtype()).m()?,
            };
            let entrada = ops::concatenate(&[previo, qkv_i], 1).m()?;
            cc.conv = Some(entrada.try_index((.., -(kc - 1).., ..)).m()?.contiguous().m()?);
            let conv = (self.f.silu.borrow_mut())(&ops::conv1d(&entrada, &d.conv, 1, 0, 1, conv_dim).m()?).m()?;
            let partes = ops::split_at_indices(&conv, &[key_dim, 2 * key_dim], -1).m()?;
            let q = ops::reshape(&partes[0], &[1, 1, hk, dk]).m()?;
            let kk = ops::reshape(&partes[1], &[1, 1, hk, dk]).m()?;
            let v = ops::reshape(&partes[2], &[1, 1, hv, dv]).m()?;
            let q = ops::multiply(fast::rms_norm(&q, None, 1e-6).m()?, esc(inv * inv, q.dtype()).m()?).m()?;
            let kk = ops::multiply(fast::rms_norm(&kk, None, 1e-6).m()?, esc(inv, kk.dtype()).m()?).m()?;
            let beta = ops::sigmoid(&b_i).m()?;
            let g = (self.f.decaimiento.borrow_mut())((&d.a_log, &a_i, &d.dt_bias)).m()?;
            let estado = match cc.estado.take() {
                Some(e) => e,
                None => ops::zeros::<f32>(&[1, hv, dv, dk]).m()?,
            };
            let opciones = MetalKernelApplyOptions {
                template_args: vec![
                    ("InT".into(), TemplateArg::Dtype(q.dtype())),
                    ("StT".into(), TemplateArg::Dtype(Dtype::Float32)),
                    ("Dk".into(), TemplateArg::Int(dk)),
                    ("Dv".into(), TemplateArg::Int(dv)),
                    ("Hk".into(), TemplateArg::Int(hk)),
                    ("Hv".into(), TemplateArg::Int(hv)),
                ],
                ..Default::default()
            };
            let formas = [MetalKernelOutput { shape: vec![1, 1, hv, dv], dtype: q.dtype() }, MetalKernelOutput { shape: vec![1, hv, dv, dk], dtype: Dtype::Float32 }];
            let t = Array::from_int(1);
            // SAFETY: el mismo kernel y la misma rejilla que `delta` (ver allí), con T = 1: q/k [1,1,Hk,Dk],
            // v [1,1,Hv,Dv], g y beta [1,1,Hv] y el estado [1,Hv,Dv,Dk] de ESTA conversación.
            let mut r = unsafe { kernel.apply(&[&q, &kk, &v, &g, &beta, &estado, &t], &formas, (32, dv, hv), (32, 4, 1), opciones).m()? };
            let nuevo = r.pop().ok_or_else(|| anyhow_tr!("kernel sin estado", "kernel without state"))?;
            let y = r.pop().ok_or_else(|| anyhow_tr!("kernel sin salida", "kernel without output"))?;
            cc.estado = Some(nuevo);
            let x = fast::rms_norm(&y, Some(&d.norma), c.rms_norm_eps).m()?;
            let o = (self.f.norma_puerta.borrow_mut())((&z_i, &x)).m()?;
            salidas.push(ops::reshape(&o, &[1, 1, value_dim]).m()?);
        }
        d.salida.aplicar(&ops::concatenate(&salidas, 1).m()?)
    }
}

/// F9.0 · LA MEDICIÓN (necesita el modelo: NO corre en `cargo test`; el Portador la corre a mano).
///
/// ```text
/// cargo test -p gotv-mlx --release --lib -- --ignored --nocapture f9_0_generacion_en_lote
/// GOTV_F90_MODELO=<carpeta de otro modelo> …   (por omisión el 9B: ~/.cache/gotv/models/mlx-community/Qwen3.5-9B-4bit)
/// ```
///
/// **Criterio, escrito ANTES de medir (PLAN-CUERPO F9.0):**
/// 1. *Correcto primero.* El lote debe dar lo mismo que de a uno: con B = 1, |Δlogit| máx. ≤ 0,05; con
///    B = 2 (dos conversaciones de largos distintos, 8 pasos), el token voraz coincide en TODOS los pasos y
///    |Δlogit| máx. ≤ 1,0 (con B > 1 la multiplicación cuantizada usa otra forma: se tolera el redondeo, no
///    otro resultado). Si falla, la prueba FALLA y no se decide nada con las velocidades.
/// 2. *Ganancia.* Tokens por segundo TOTALES (todas las conversaciones) con 1, 2 y 4 en lote, 64 pasos
///    tras 4 de calentamiento, conversaciones de 1 024 tokens. ×2 = tps(2)/tps(1).
/// 3. *Memoria.* Lo que suma cada conversación (su caché, tras 1 024 tokens) y el pico con 4.
/// 4. *Largos distintos.* 2 en lote con 256 y 8 192 tokens: tps total contra el de 2 iguales.
/// 5. **Veredicto:** LOTES si el lote es correcto, ×2 ≥ 1,5 y el pico con 2 deja libre al menos el 25 % de la
///    RAM de la Mac; si no, TURNOS. La línea `F9.0 VEREDICTO:` va tal cual a `NOTAS.md`.
#[cfg(test)]
mod medir {
    use super::super::{Cache, Modelo, M};
    use mlx_rs::Array;
    use std::path::PathBuf;
    use std::time::Instant;

    /// Tokens de relleno, distintos por conversación (el contenido no cambia la velocidad).
    fn tokens(n: usize, semilla: u32) -> Vec<u32> {
        (0..n as u32).map(|i| 1000 + (i.wrapping_mul(7919).wrapping_add(semilla.wrapping_mul(104_729))) % 30_000).collect()
    }

    fn prellenar(m: &Modelo, ids: &[u32]) -> Cache {
        let mut c = m.cache_nueva();
        for trozo in ids.chunks(512) {
            m.adelante(trozo, &mut c).unwrap().eval().unwrap();
        }
        c
    }

    fn f32s(a: &Array) -> Vec<f32> {
        let a = a.as_dtype(mlx_rs::Dtype::Float32).m().unwrap();
        a.eval().unwrap();
        a.as_slice::<f32>().to_vec()
    }

    fn argmax(v: &[f32]) -> u32 {
        v.iter().enumerate().fold(0, |b, (i, x)| if *x > v[b] { i } else { b }) as u32
    }

    fn dif_max(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
    }

    fn ram_total() -> usize {
        let mut v: u64 = 0;
        let mut n = std::mem::size_of::<u64>();
        // SAFETY: sysctlbyname con un búfer de 8 bytes para hw.memsize (u64), nombre terminado en NUL
        let r = unsafe { libc::sysctlbyname(c"hw.memsize".as_ptr(), &mut v as *mut u64 as *mut libc::c_void, &mut n, std::ptr::null_mut(), 0) };
        if r == 0 {
            v as usize
        } else {
            0
        }
    }

    const GB: f64 = 1024.0 * 1024.0 * 1024.0;

    /// tps totales de `b` conversaciones de `largos[i]` tokens, `pasos` pasos tras 4 de calentamiento.
    fn medir_lote(m: &Modelo, largos: &[usize], pasos: usize) -> (f64, usize) {
        let mut caches: Vec<Cache> = largos.iter().enumerate().map(|(i, &n)| prellenar(m, &tokens(n, i as u32 + 11))).collect();
        let mem = mlx_rs::memory::active_memory().unwrap_or(0);
        let mut toks: Vec<u32> = (0..largos.len() as u32).map(|i| 2000 + i).collect();
        let mut paso = |toks: &mut Vec<u32>| {
            let mut refs: Vec<&mut Cache> = caches.iter_mut().collect();
            let l = f32s(&m.paso_lote(toks, &mut refs).unwrap());
            let v = l.len() / toks.len();
            for (i, t) in toks.iter_mut().enumerate() {
                *t = argmax(&l[i * v..(i + 1) * v]);
            }
        };
        for _ in 0..4 {
            paso(&mut toks);
        }
        let t0 = Instant::now();
        for _ in 0..pasos {
            paso(&mut toks);
        }
        ((largos.len() * pasos) as f64 / t0.elapsed().as_secs_f64(), mem)
    }

    #[test]
    #[ignore]
    fn f9_0_generacion_en_lote() {
        let dir = std::env::var("GOTV_F90_MODELO").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap()).join(".cache/gotv/models/mlx-community/Qwen3.5-9B-4bit"));
        assert!(dir.exists(), "no está el modelo en {} (GOTV_F90_MODELO=<carpeta>)", dir.display());
        let t0 = Instant::now();
        let m = Modelo::cargar(&dir).unwrap();
        let _ = mlx_rs::memory::reset_peak_memory();
        let mem_modelo = mlx_rs::memory::active_memory().unwrap_or(0);
        eprintln!("F9.0 · {} cargado en {:.1} s · {:.2} GB activos", dir.display(), t0.elapsed().as_secs_f32(), mem_modelo as f64 / GB);

        // 1. CORRECTO: B = 1 contra `adelante`
        let a = tokens(300, 1);
        let (mut c1, mut c2) = (prellenar(&m, &a), prellenar(&m, &a));
        let uno = f32s(&m.adelante(&[1234], &mut c1).unwrap());
        let lote = f32s(&m.paso_lote(&[1234], &mut [&mut c2]).unwrap());
        let d1 = dif_max(&uno, &lote);
        eprintln!("correcto B=1: |Δ| máx {d1:.4} · voraz {} / {}", argmax(&uno), argmax(&lote));
        assert!(d1 <= 0.05 && argmax(&uno) == argmax(&lote), "el lote de 1 no da lo mismo que de a uno (|Δ| {d1})");
        // B = 2, largos distintos, 8 pasos
        let (pa, pb) = (tokens(300, 2), tokens(1200, 3));
        let (mut sa, mut sb) = (prellenar(&m, &pa), prellenar(&m, &pb));
        let (mut la, mut lb) = (prellenar(&m, &pa), prellenar(&m, &pb));
        let (mut ta, mut tb) = (4321u32, 5432u32);
        let mut d2 = 0f32;
        for paso in 0..8 {
            let va = f32s(&m.adelante(&[ta], &mut sa).unwrap());
            let vb = f32s(&m.adelante(&[tb], &mut sb).unwrap());
            let l = f32s(&m.paso_lote(&[ta, tb], &mut [&mut la, &mut lb]).unwrap());
            let v = l.len() / 2;
            d2 = d2.max(dif_max(&va, &l[..v])).max(dif_max(&vb, &l[v..]));
            assert!(argmax(&va) == argmax(&l[..v]) && argmax(&vb) == argmax(&l[v..]), "paso {paso}: el lote de 2 elige otro token");
            (ta, tb) = (argmax(&va), argmax(&vb));
        }
        eprintln!("correcto B=2 (300 y 1 200 tokens, 8 pasos): |Δ| máx {d2:.4}");
        assert!(d2 <= 1.0, "el lote de 2 se aparta demasiado (|Δ| {d2})");
        drop((c1, c2, sa, sb, la, lb));
        let _ = mlx_rs::memory::clear_cache();

        // 2 y 3. GANANCIA y MEMORIA: 1, 2 y 4 conversaciones de 1 024 tokens
        // la línea base de hoy: `adelante` de a un token (el camino del motor)
        let mut c = prellenar(&m, &tokens(1024, 7));
        let mut t = 2000u32;
        for _ in 0..4 {
            t = argmax(&f32s(&m.adelante(&[t], &mut c).unwrap()));
        }
        let t1 = Instant::now();
        for _ in 0..64 {
            t = argmax(&f32s(&m.adelante(&[t], &mut c).unwrap()));
        }
        let tps_hoy = 64.0 / t1.elapsed().as_secs_f64();
        drop(c);
        let _ = mlx_rs::memory::clear_cache();
        let mut tps = Vec::new();
        let mut por_conv = 0f64;
        let mut pico2 = 0usize;
        let mut mem1 = 0usize;
        for b in [1usize, 2, 4] {
            let _ = mlx_rs::memory::reset_peak_memory();
            let (v, mem) = medir_lote(&m, &vec![1024; b], 64);
            let pico = mlx_rs::memory::peak_memory().unwrap_or(0);
            if b == 2 {
                pico2 = pico;
            }
            // lo que suma CADA conversación: de 1 a 4 en lote. Restarle lo «activo tras cargar» no sirve:
            // los pesos entran en diferido (Huihui marcó 0,00 GB al cargar y daba +4 980 MB, no ~320)
            if b == 1 {
                mem1 = mem;
            }
            if b == 4 {
                por_conv = mem.saturating_sub(mem1) as f64 / 3.0;
            }
            eprintln!("B={b}: {v:.1} tok/s en total ({:.1} por conversación) · activos {:.2} GB · pico {:.2} GB", v / b as f64, mem as f64 / GB, pico as f64 / GB);
            tps.push(v);
            let _ = mlx_rs::memory::clear_cache();
        }
        // 4. LARGOS DISTINTOS
        let (v_dist, _) = medir_lote(&m, &[256, 8192], 64);
        eprintln!("B=2 con 256 y 8 192 tokens: {v_dist:.1} tok/s en total (iguales de 1 024: {:.1})", tps[1]);

        // 5. VEREDICTO
        let (x2, x4) = (tps[1] / tps[0], tps[2] / tps[0]);
        let ram = ram_total();
        let cabe = ram > 0 && (pico2 as f64) <= ram as f64 * 0.75;
        let lotes = x2 >= 1.5 && cabe;
        eprintln!("línea base (`adelante`, 1 conversación): {tps_hoy:.1} tok/s · lote de 1: {:.1}", tps[0]);
        eprintln!(
            "F9.0 VEREDICTO: {} · ×{x2:.2} con 2, ×{x4:.2} con 4 · +{:.0} MB por conversación de 1 024 tokens · pico con 2: {:.1} de {:.0} GB · largos 256+8 192: {v_dist:.1} tok/s · modelo {}",
            if lotes { "LOTES" } else { "TURNOS" },
            por_conv / (1024.0 * 1024.0),
            pico2 as f64 / GB,
            ram as f64 / GB,
            dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        );
    }
}
