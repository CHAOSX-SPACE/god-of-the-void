// Simulador del estudio Experto para la vista previa en el navegador (sin Tauri). NO es el producto: la
// sesión, la validación y el sonido de verdad son del núcleo en Rust (`gotv_core::experto*`). Aquí hay lo
// justo para ver y probar la interfaz: la lista de operaciones, un audio sintético por archivo y el montaje
// de los clips (posición, recorte, ganancia, fundidos, invertir). Los efectos NO suenan en el simulador.
(() => {
  const T = window.__TAURI__, base = T.core.invoke, FS = 48000;
  const SES = {};
  const sesion = (root) => (SES[root] ||= { activa: false, minimizada: false, frecuencia: FS, ops: [], rehacer: [], durs: {} });
  const hash = (s) => { let h = 2166136261; for (const c of String(s)) h = Math.imul(h ^ c.charCodeAt(0), 16777619); return h >>> 0; };
  const durDe = (s, archivo) => s.durs[archivo] || 20;

  // ── el estado que sale de la lista de operaciones ──
  function estado(s) {
    const e = { pistas: [], grupos: [], regiones: [], clips: [], bpm: 120, compas: 4, marcadores: [], automatizaciones: [], maestro: { ganancia_db: 0, insertos: {} }, tonalidad: "", letra: "", siguiente_clip: 0 };
    const P = (id) => e.pistas.find((x) => x.id === id), G = (id) => e.grupos.find((x) => x.id === id), C = (id) => e.clips.find((x) => x.id === id);
    const mal = (t) => { throw t; };
    const clipNuevo = (c) => { c.id = ++e.siguiente_clip; e.clips.push(c); return c; };
    const fin = (c) => c.inicio + c.duracion;
    const partir = (pista, t, solo) => {
      const cuales = e.clips.map((c, i) => [c, i]).filter(([c]) => c.pista === pista && (!solo || c.id === solo) && t > c.inicio + 0.001 && t < fin(c) - 0.001).map(([, i]) => i);
      for (const i of cuales.reverse()) {
        const izq = e.clips[i], der = { ...izq }, corte = t - izq.inicio, r = izq.ritmo || 1;
        if (izq.invertido) { izq.desde += (izq.duracion - corte) * r; } else der.desde = izq.desde + corte * r;
        der.duracion = izq.duracion - corte; izq.duracion = corte; izq.fundido_salida = 0; der.inicio = t; der.fundido_entrada = 0; der.id = ++e.siguiente_clip;
        e.clips.splice(i + 1, 0, der);
      }
      return cuales.length;
    };
    for (const o of s.ops) {
      switch (o.op) {
        case "agregar_pista":
          e.pistas.push({ ganancia_db: 0, pan: 0, silencio: false, solo: false, insertos: {}, ...o.pista });
          clipNuevo({ pista: o.pista.id, archivo: o.pista.archivo, inicio: 0, desde: 0, duracion: durDe(s, o.pista.archivo), ganancia_db: 0, fundido_entrada: 0, fundido_salida: 0, tono: 0, ritmo: 1, invertido: false, silencio: false, nombre: "" });
          break;
        case "quitar_pista": e.pistas = e.pistas.filter((x) => x.id !== o.id); e.regiones = e.regiones.filter((r) => r.pista !== o.id); e.clips = e.clips.filter((c) => c.pista !== o.id); e.automatizaciones = e.automatizaciones.filter((a) => a.pista !== o.id); break;
        case "renombrar": P(o.pista).nombre = o.nombre; break;
        case "mezcla": for (const k of ["ganancia_db", "pan", "silencio", "solo"]) if (o[k] !== undefined && o[k] !== null) P(o.pista)[k] = o[k]; break;
        case "crear_grupo": e.grupos.push({ ganancia_db: 0, pan: 0, silencio: false, solo: false, insertos: {}, ...o.grupo }); break;
        case "quitar_grupo": e.grupos = e.grupos.filter((x) => x.id !== o.id); e.pistas.forEach((x) => { if (x.grupo === o.id) x.grupo = null; }); break;
        case "agrupar": e.pistas.forEach((x) => { if (o.pistas.includes(x.id)) x.grupo = o.grupo; }); break;
        case "mezcla_grupo": for (const k of ["ganancia_db", "pan", "silencio", "solo"]) if (o[k] !== undefined && o[k] !== null) G(o.grupo)[k] = o[k]; break;
        case "region": if (!(o.region.hasta > o.region.desde)) mal("el tramo va de un segundo a otro MAYOR"); e.regiones.push({ apagada: false, ...o.region }); break;
        case "quitar_region": e.regiones = e.regiones.filter((r) => r.id !== o.id); break;
        case "tempo": e.bpm = o.bpm; e.compas = o.compas; break;
        case "marcador": e.marcadores.push(o.marcador); break;
        case "insertos": P(o.pista).insertos = o.insertos; break;
        case "partir": if (!partir(o.pista, o.segundo, o.clip)) mal("ningún clip de esa pista cruza ese segundo"); break;
        case "quitar_tramo": {
          partir(o.pista, o.desde); partir(o.pista, o.hasta);
          const antes = e.clips.length;
          e.clips = e.clips.filter((c) => !(c.pista === o.pista && c.inicio >= o.desde - 1e-6 && fin(c) <= o.hasta + 1e-6));
          if (e.clips.length === antes) mal("en ese tramo no hay nada que quitar");
          if (o.cerrar) e.clips.forEach((c) => { if (c.pista === o.pista && c.inicio >= o.hasta - 1e-6) c.inicio = Math.max(0, c.inicio - (o.hasta - o.desde)); });
          break;
        }
        case "mover_clip": { const c = C(o.clip); if (!c) mal("ese clip no existe"); if (o.inicio < 0) mal("clip: inicio ≥ 0"); c.inicio = o.inicio; if (o.pista) c.pista = o.pista; e.clips = e.clips.filter((x) => x !== c).concat(c); break; }
        case "recortar_clip": { const c = C(o.clip); if (!(o.duracion > 0) || o.inicio < 0 || o.desde < 0) mal("clip: inicio y recorte ≥ 0, duración > 0"); c.inicio = o.inicio; c.desde = o.desde; c.duracion = o.duracion; break; }
        case "clip_ajustes": {
          const c = C(o.clip);
          if (o.ritmo !== undefined && o.ritmo !== null) { c.duracion = (c.duracion * (c.ritmo || 1)) / o.ritmo; }
          for (const k of ["ganancia_db", "fundido_entrada", "fundido_salida", "tono", "ritmo", "invertido", "silencio", "nombre"]) if (o[k] !== undefined && o[k] !== null) c[k] = o[k];
          break;
        }
        case "duplicar_clip": { const c = C(o.clip); clipNuevo({ ...c, inicio: o.inicio, pista: o.pista || c.pista }); break; }
        case "agregar_clip": clipNuevo({ ...o.clip }); break;
        case "quitar_clip": e.clips = e.clips.filter((c) => c.id !== o.clip); break;
        case "insertos_grupo": G(o.grupo).insertos = o.insertos; break;
        case "maestro": if (o.ganancia_db !== undefined && o.ganancia_db !== null) e.maestro.ganancia_db = o.ganancia_db; if (o.insertos) e.maestro.insertos = o.insertos; break;
        case "automatizar": e.automatizaciones = e.automatizaciones.filter((a) => !(a.pista === o.pista && a.parametro === o.parametro)); if (o.puntos.length) e.automatizaciones.push({ pista: o.pista, parametro: o.parametro, puntos: o.puntos.slice().sort((a, b) => a.t - b.t) }); break;
        case "region_ajustes": { const r = e.regiones.find((x) => x.id === o.id); const d = o.desde ?? r.desde, h = o.hasta ?? r.hasta; if (!(h > d) || d < 0) mal("el tramo va de un segundo a otro MAYOR"); r.desde = d; r.hasta = h; if (o.apagada !== undefined && o.apagada !== null) r.apagada = o.apagada; break; }
        case "marcadores": e.marcadores = o.marcadores.slice().sort((a, b) => a.segundo - b.segundo); break;
        case "ordenar": e.pistas.sort((a, b) => o.pistas.indexOf(a.id) - o.pistas.indexOf(b.id)); break;
        case "color": P(o.pista).color = o.color || null; break;
        case "renombrar_grupo": G(o.grupo).nombre = o.nombre; break;
        case "tonalidad": e.tonalidad = o.tonalidad; break;
        case "letra": e.letra = o.texto; break;
      }
    }
    return e;
  }
  const idNuevo = (s) => 1 + Math.max(0, ...s.ops.concat(s.rehacer).map((o) => (o.pista && o.pista.id) || (o.grupo && o.grupo.id) || (o.region && o.region.id) || 0));
  const numerar = (s, o) => { const n = idNuevo(s); for (const k of ["pista", "grupo", "region"]) if (o[k] && typeof o[k] === "object" && !o[k].id) o[k].id = n; return o; };
  const NOMBRES = { agregar_pista: "Pista nueva", quitar_pista: "Quitar una pista", renombrar: "Renombrar", mezcla: "Mezcla de una pista", crear_grupo: "Bus nuevo", quitar_grupo: "Quitar un bus", agrupar: "Agrupar", mezcla_grupo: "Mezcla de un bus", region: "Tramo de efecto", quitar_region: "Quitar un tramo", tempo: "Tempo", marcador: "Marcador", insertos: "Efectos de una pista", partir: "Partir", quitar_tramo: "Quitar un tramo de audio", mover_clip: "Mover un clip", recortar_clip: "Recortar un clip", clip_ajustes: "Ajustar un clip", duplicar_clip: "Duplicar un clip", agregar_clip: "Clip nuevo", quitar_clip: "Quitar un clip", insertos_grupo: "Efectos de un bus", maestro: "Maestro", automatizar: "Automatización", region_ajustes: "Ajustar un tramo", marcadores: "Marcadores", ordenar: "Ordenar las pistas", color: "Color", renombrar_grupo: "Renombrar un bus", tonalidad: "Tonalidad", letra: "Letra" };
  const etiqueta = (o) => NOMBRES[o.op] + (o.pista && o.pista.nombre ? ": " + o.pista.nombre : o.segundo !== undefined ? ` en ${o.segundo.toFixed(2)} s` : "");
  const vista = (s) => {
    const e = estado(s), archivos = {};
    for (const c of e.clips) archivos[c.archivo] = durDe(s, c.archivo);
    return { activa: s.activa, minimizada: s.minimizada, frecuencia: s.frecuencia, deshacer: s.ops.length, rehacer: s.rehacer.length, historial: s.ops.map(etiqueta), por_rehacer: s.rehacer.slice().reverse().map(etiqueta), archivos, estado: e };
  };
  const aplicar = (s, o) => {
    o = numerar(s, JSON.parse(JSON.stringify(o)));
    s.ops.push(o);
    try { estado(s); } catch (e) { s.ops.pop(); throw e; }
    s.rehacer = [];
  };

  // ── un audio sintético por archivo (distinto según su nombre) y el montaje de los clips ──
  const AUDIO = {};
  function audioDe(s, archivo) {
    if (AUDIO[archivo]) return AUDIO[archivo];
    const h = hash(archivo), n = Math.round(durDe(s, archivo) * FS), l = new Float32Array(n), r = new Float32Array(n);
    const tipo = /drum|bater/i.test(archivo) ? 0 : /bass|bajo/i.test(archivo) ? 1 : /vocal|voz/i.test(archivo) ? 2 : 3 + (h % 3);
    const f0 = [0, 55, 220, 110, 165, 330][tipo] * (1 + ((h >> 4) % 4) * 0.125), pulso = 0.5;
    let ruido = h || 1;
    for (let i = 0; i < n; i++) {
      const t = i / FS, enP = t % pulso, compas = Math.floor(t / (pulso * 4)) % 4;
      let v;
      if (tipo === 0) { ruido = (Math.imul(ruido, 1664525) + 1013904223) >>> 0; const rd = ruido / 2147483648 - 1; v = 0.6 * Math.exp(-enP * 28) * Math.sin(2 * Math.PI * 60 * enP * (1 + 3 * Math.exp(-enP * 40))) + (Math.floor(t / pulso) % 2 ? 0.25 * Math.exp(-enP * 22) * rd : 0) + 0.06 * Math.exp(-((t % (pulso / 2)) * 60)) * rd; }
      else { const f = f0 * [1, 1.335, 1.498, 1.122][compas]; const env = tipo === 2 ? 0.5 + 0.5 * Math.sin(2 * Math.PI * t * 0.4) : Math.exp(-enP * (tipo === 1 ? 3 : 5)); v = 0.3 * env * (Math.sin(2 * Math.PI * f * t) + 0.35 * Math.sin(2 * Math.PI * f * 2 * t) + (tipo >= 3 ? 0.25 * Math.sin(2 * Math.PI * f * 1.5 * t) : 0)); }
      l[i] = v; r[i] = v * (0.85 + 0.15 * Math.sin(t * 0.7 + h));
    }
    return (AUDIO[archivo] = [l, r]);
  }
  function montar(s, e, p) {
    let n = 0;
    const cs = e.clips.filter((c) => c.pista === p.id);
    for (const c of cs) n = Math.max(n, Math.round((c.inicio + c.duracion) * FS));
    const out = [new Float32Array(n), new Float32Array(n)];
    for (const c of cs) {
      if (c.silencio) continue;
      const src = audioDe(s, c.archivo), r = c.ritmo || 1, a = Math.round(c.desde * FS), largo = Math.round(c.duracion * FS), ini = Math.round(c.inicio * FS), g = Math.pow(10, (c.ganancia_db || 0) / 20);
      const fe = Math.min(largo, Math.round((c.fundido_entrada || 0) * FS)), fsal = Math.min(largo - fe, Math.round((c.fundido_salida || 0) * FS)), trozo = Math.round(c.duracion * r * FS);
      for (let ch = 0; ch < 2; ch++) for (let k = 0; k < largo; k++) {
        const j = c.invertido ? a + trozo - 1 - Math.round(k * r) : a + Math.round(k * r), v = (src[ch][j] || 0) * g;
        const w = k < fe ? (k + 0.5) / fe : k >= largo - fsal ? (largo - k - 0.5) / fsal : 1;
        out[ch][ini + k] = w >= 1 ? v : out[ch][ini + k] * Math.cos((w * Math.PI) / 2) + v * Math.sin((w * Math.PI) / 2);
      }
    }
    // los tramos de ganancia y de silencio sí se oyen; los demás efectos, sólo en el núcleo de verdad
    for (const rg of e.regiones.filter((x) => x.pista === p.id && !x.apagada)) {
      const k = rg.efecto.tipo === "silencio" ? 0 : rg.efecto.tipo === "ganancia" ? Math.pow(10, rg.efecto.db / 20) : rg.efecto.tipo === "reemplazo" ? -1 : 1;
      for (let ch = 0; ch < 2; ch++) for (let i = Math.round(rg.desde * FS), f = Math.min(n, Math.round(rg.hasta * FS)); i < f; i++) out[ch][i] = k < 0 ? audioDe(s, "toma-" + rg.id)[ch][i - Math.round(rg.desde * FS)] || 0 : out[ch][i] * k;
    }
    return out;
  }
  function wav([l, r]) {
    const n = l.length, b = new ArrayBuffer(44 + n * 4), v = new DataView(b);
    const txt = (o, t) => [...t].forEach((c, i) => v.setUint8(o + i, c.charCodeAt(0)));
    txt(0, "RIFF"); v.setUint32(4, 36 + n * 4, true); txt(8, "WAVEfmt "); v.setUint32(16, 16, true); v.setUint16(20, 1, true); v.setUint16(22, 2, true);
    v.setUint32(24, FS, true); v.setUint32(28, FS * 4, true); v.setUint16(32, 4, true); v.setUint16(34, 16, true); txt(36, "data"); v.setUint32(40, n * 4, true);
    for (let i = 0; i < n; i++) { v.setInt16(44 + i * 4, Math.max(-1, Math.min(1, l[i])) * 32767, true); v.setInt16(46 + i * 4, Math.max(-1, Math.min(1, r[i])) * 32767, true); }
    return b;
  }
  function mezcla(s) {
    const e = estado(s), partes = e.pistas.map((p) => [p, montar(s, e, p)]), n = Math.max(1, ...partes.map(([, x]) => x[0].length)), out = [new Float32Array(n), new Float32Array(n)];
    const haySolo = e.pistas.some((x) => x.solo) || e.grupos.some((x) => x.solo);
    for (const [p, x] of partes) {
      const g = p.grupo && e.grupos.find((q) => q.id === p.grupo);
      if (p.silencio || (g && g.silencio) || (haySolo && !p.solo && !(g && g.solo))) continue;
      const k = Math.pow(10, (p.ganancia_db + (g ? g.ganancia_db : 0)) / 20);
      for (let i = 0; i < x[0].length; i++) { out[0][i] += x[0][i] * k; out[1][i] += x[1][i] * k; }
    }
    return out;
  }
  // la respuesta de un biquad de Web Audio (las mismas fórmulas RBJ del núcleo), para la curva del EQ
  function respuesta(bandas, f, fs) {
    let total = 0;
    for (const b of bandas.filter((x) => !x.apagada)) {
      const A = Math.pow(10, b.db / 40), w0 = (2 * Math.PI * f0(b.frecuencia, fs)) , s = Math.sin(w0), c = Math.cos(w0), q = b.q || 0.707, aq = s / (2 * q), as = (s / 2) * Math.SQRT2, ra = Math.sqrt(A);
      let k;
      if (b.filtro === "paso_bajo") k = [(1 - c) / 2, 1 - c, (1 - c) / 2, 1 + aq, -2 * c, 1 - aq];
      else if (b.filtro === "paso_alto") k = [(1 + c) / 2, -(1 + c), (1 + c) / 2, 1 + aq, -2 * c, 1 - aq];
      else if (b.filtro === "muesca") k = [1, -2 * c, 1, 1 + aq, -2 * c, 1 - aq];
      else if (b.filtro === "paso_banda") k = [aq, 0, -aq, 1 + aq, -2 * c, 1 - aq];
      else if (b.filtro === "graves") k = [A * (A + 1 - (A - 1) * c + 2 * as * ra), 2 * A * (A - 1 - (A + 1) * c), A * (A + 1 - (A - 1) * c - 2 * as * ra), A + 1 + (A - 1) * c + 2 * as * ra, -2 * (A - 1 + (A + 1) * c), A + 1 + (A - 1) * c - 2 * as * ra];
      else if (b.filtro === "agudos") k = [A * (A + 1 + (A - 1) * c + 2 * as * ra), -2 * A * (A - 1 + (A + 1) * c), A * (A + 1 + (A - 1) * c - 2 * as * ra), A + 1 - (A - 1) * c + 2 * as * ra, 2 * (A - 1 - (A + 1) * c), A + 1 - (A - 1) * c - 2 * as * ra];
      else k = [1 + aq * A, -2 * c, 1 - aq * A, 1 + aq / A, -2 * c, 1 - aq / A];
      const w = (2 * Math.PI * f) / fs, c1 = Math.cos(w), s1 = Math.sin(w), c2 = Math.cos(2 * w), s2 = Math.sin(2 * w);
      const nr = k[0] + k[1] * c1 + k[2] * c2, ni = -(k[1] * s1 + k[2] * s2), dr = k[3] + k[4] * c1 + k[5] * c2, di = -(k[4] * s1 + k[5] * s2);
      total += 10 * Math.log10(Math.max(1e-30, (nr * nr + ni * ni) / (dr * dr + di * di)));
    }
    return total;
    function f0(fr, fs2) { return Math.min(0.5, fr / fs2); }
  }
  const espera = (ms) => new Promise((ok) => setTimeout(ok, ms));
  let cancelado = false;
  async function trabajo(pasos) {
    cancelado = false;
    for (const t of pasos) { window.__mockEmitir("experto-avance", t); await espera(550); if (cancelado) throw "cancelado"; }
  }

  T.core.invoke = async (cmd, a) => {
    if (!cmd.startsWith("experto_")) return base(cmd, a);
    const s = a && a.proyecto ? sesion(a.proyecto) : null, e = s && estado(s);
    switch (cmd) {
      case "experto_sesion": return vista(s);
      case "experto_activar": s.activa = a.activa; s.minimizada = a.activa ? !!a.minimizada : false; return vista(s);
      case "experto_aplicar": aplicar(s, a.op); return vista(s);
      case "experto_deshacer": { const [de, hacia] = a.rehacer ? [s.rehacer, s.ops] : [s.ops, s.rehacer]; if (de.length) hacia.push(de.pop()); return vista(s); }
      case "experto_ir_a": while (s.ops.length > a.n) s.rehacer.push(s.ops.pop()); while (s.ops.length < a.n && s.rehacer.length) s.ops.push(s.rehacer.pop()); return vista(s);
      case "experto_importar": aplicar(s, { op: "agregar_pista", pista: { id: 0, nombre: a.ruta.split("/").pop().replace(/\.[^.]+$/, ""), archivo: a.ruta + "#" + s.ops.length, origen: "importada" } }); return vista(s);
      case "experto_separar": {
        const madre = e.pistas.find((p) => p.id === a.pista), partes = [["vocals", "Voz"], ["drums", "Batería"], ["bass", "Bajo"], ...(a.modelo === "6" ? [["guitar", "Guitarra"], ["piano", "Piano"]] : []), ["other", "Resto"]];
        await trabajo(["cargando HTDemucs", "separando 1 de 3", "separando 2 de 3", "separando 3 de 3"]);
        const d = Math.max(1, ...e.clips.filter((c) => c.pista === madre.id).map((c) => c.inicio + c.duracion));
        for (const [f, n] of partes) { const archivo = `${madre.archivo}-${f}.wav`; s.durs[archivo] = d; aplicar(s, { op: "agregar_pista", pista: { id: 0, nombre: `${madre.nombre} · ${n}`, archivo, origen: "separada", de: madre.id, fuente: f, grupo: madre.grupo || null } }); }
        aplicar(s, { op: "mezcla", pista: madre.id, silencio: true });
        return vista(s);
      }
      case "experto_pista_audio": return wav(montar(s, e, e.pistas.find((p) => p.id === a.pista)));
      case "experto_mezcla_audio": await espera(300); return wav(mezcla(s));
      case "experto_tono": { const p = e.pistas.find((x) => x.id === a.pista), archivo = `${p.archivo}-tono${s.ops.length}.wav`; s.durs[archivo] = a.hasta; aplicar(s, { op: "agregar_pista", pista: { id: 0, nombre: `${p.nombre} · ${a.semitonos > 0 ? "+" : ""}${a.semitonos} st (${a.desde.toFixed(1)}–${a.hasta.toFixed(1)} s)`, archivo, origen: "generada", de: p.de, fuente: p.fuente } }); aplicar(s, { op: "region", region: { id: 0, pista: p.id, desde: a.desde, hasta: a.hasta, efecto: { tipo: "silencio" } } }); return vista(s); }
      case "experto_exportar": { await espera(500); const o = a.opciones || {}; return { carpeta: a.proyecto + "/exportado/2026-11-14T12-00-00", exportado: { mezcla: "x", pistas: o.pistas === false ? [] : Array(e.pistas.length).fill("x"), dawproject: o.dawproject === false ? null : "x.dawproject", extra: o.formato ? "x." + o.formato : null, ganancia_db: a.lufs ? 2.1 : 0, lufs: a.lufs || -17.3, pico_real_db: -1.2, falto_db: 0 } }; }
      case "experto_mostrar": return null;
      case "experto_grabar": s.grabando = Date.now(); return null;
      case "experto_grabar_parar": { const d = Math.max(0.5, (Date.now() - (s.grabando || Date.now())) / 1000); s.grabando = 0; if (a.descartar) return vista(s); const archivo = `grabacion-${s.ops.length}.wav`; s.durs[archivo] = d; aplicar(s, { op: "agregar_pista", pista: { id: 0, nombre: "Grabación " + (e.pistas.filter((p) => p.archivo.startsWith("grabacion-")).length + 1), archivo, origen: "importada" } }); if (a.inicio > 0) { const v = vista(s), p = v.estado.pistas[v.estado.pistas.length - 1], c = v.estado.clips.find((x) => x.pista === p.id); aplicar(s, { op: "mover_clip", clip: c.id, inicio: a.inicio }); } return vista(s); }
      case "experto_cancelar": cancelado = true; return true;
      case "experto_generar": {
        const p = a.pedido;
        await trabajo(["cargando ACE-Step", "planificando (LM)", "generando 4 de 8", "generando 8 de 8", "decodificando el audio"]);
        const archivo = `generada-${s.ops.length}-${p.descripcion}.wav`, total = Math.max(0, ...e.clips.map((c) => c.inicio + c.duracion));
        s.durs[archivo] = p.segundos || total || 30;
        aplicar(s, { op: "agregar_pista", pista: { id: 0, nombre: p.nombre || "✦ " + p.descripcion.slice(0, 28), archivo, origen: "generada" } });
        const v = vista(s);
        return { vista: v, resultado: v.estado.pistas[v.estado.pistas.length - 1].id };
      }
      case "experto_rehacer": {
        await trabajo(["cargando ACE-Step", "repintando el tramo", "separando la parte", "igualando el nivel"]);
        aplicar(s, { op: "region", region: { id: 0, pista: a.pista, desde: a.desde, hasta: a.hasta, efecto: { tipo: "reemplazo", archivo: "toma.wav", descripcion: a.descripcion } } });
        return { vista: vista(s), resultado: null };
      }
      case "experto_extender": {
        await trabajo(["cargando ACE-Step", "generando la continuación", "decodificando el audio"]);
        const total = Math.max(0, ...e.clips.filter((c) => !a.pista || c.pista === a.pista).map((c) => c.inicio + c.duracion)), archivo = `extension-${s.ops.length}.wav`;
        s.durs[archivo] = a.segundos + 0.25;
        let pista = a.pista;
        if (pista) aplicar(s, { op: "agregar_clip", clip: { id: 0, pista, archivo, inicio: total - 0.25, desde: 0, duracion: a.segundos + 0.25, ganancia_db: 0, fundido_entrada: 0.25, fundido_salida: 0, tono: 0, ritmo: 1, invertido: false, silencio: false, nombre: "Extensión" } });
        else { aplicar(s, { op: "agregar_pista", pista: { id: 0, nombre: "Extensión", archivo, origen: "generada" } }); const v = vista(s); pista = v.estado.pistas[v.estado.pistas.length - 1].id; const c = v.estado.clips.find((x) => x.pista === pista); aplicar(s, { op: "mover_clip", clip: c.id, inicio: Math.max(0, total - 0.25) }); aplicar(s, { op: "clip_ajustes", clip: c.id, fundido_entrada: 0.25 }); }
        return { vista: vista(s), resultado: pista };
      }
      case "experto_versionar": {
        const p = e.pistas.find((x) => x.id === a.pista), archivo = `version-${s.ops.length}-${a.descripcion}.wav`;
        await trabajo(["cargando ACE-Step", "generando la versión", "decodificando el audio"]);
        s.durs[archivo] = Math.max(1, ...e.clips.filter((c) => c.pista === p.id).map((c) => c.inicio + c.duracion));
        aplicar(s, { op: "agregar_pista", pista: { id: 0, nombre: `${p.nombre} · ${a.descripcion.slice(0, 24)}`, archivo, origen: "generada", grupo: p.grupo || null } });
        aplicar(s, { op: "mezcla", pista: p.id, silencio: true });
        const v = vista(s);
        return { vista: v, resultado: v.estado.pistas[v.estado.pistas.length - 1].id };
      }
      case "experto_letra": await trabajo(["transcribiendo la letra (1 de 1)"]); return "[verse]\nBajo la luna de papel\ncamino sin mirar atrás\n\n[chorus]\nY vuelvo a empezar";
      case "experto_masterizar": {
        await trabajo(["midiendo la mezcla"]);
        aplicar(s, { op: "maestro", ganancia_db: 0, insertos: { ...e.maestro.insertos, limitador: { techo_db: a.techoDb ?? -1, empuje_db: 6.4, liberacion_ms: 120 } } });
        return { vista: vista(s), resultado: { lufs_antes: -20.4, lufs: a.lufs ?? -14, empuje_db: 6.4, ganancia_db: 0, falto_db: 0 } };
      }
      case "experto_analizar": await espera(400); return { segundos: Math.max(0, ...e.clips.filter((c) => !a.pista || c.pista === a.pista).map((c) => c.inicio + c.duracion)), bpm: 120, bpm_confianza: 0.82, tonalidad: "A minor", tonalidad_confianza: 0.71, lufs: -17.3, lufs_corto_max: -14.9, lufs_momentaneo_max: -12.8, rango_lu: 6.2, pico_db: -2.4, pico_real_db: -1.9, rms_db: -19.6, correlacion: 0.86, continua: 0 };
      case "experto_curva_eq": return a.frecuencias.map((f) => respuesta(a.bandas, f, a.frecuencia || FS));
      case "experto_chat": {
        // el simulador sólo entiende «pista N … del X al Y … más fuerte/suave»; el núcleo de verdad usa el modelo
        const t = a.texto.toLowerCase(), m = t.match(/(\d+(?:[.,]\d+)?)\s*(?:al|a|to|-)\s*(\d+(?:[.,]\d+)?)/), p = t.match(/pista\s*(\d+)|track\s*(\d+)/);
        await espera(500);
        if (!m || !p) return { texto: "", hechas: [], preguntas: ["¿Qué pista y qué tramo?"], errores: [], vista: vista(s) };
        const db = /suave|soft/.test(t) ? -3 : 3, pista = Number(p[1] || p[2]);
        aplicar(s, { op: "region", region: { id: 0, pista, desde: Number(m[1].replace(",", ".")), hasta: Number(m[2].replace(",", ".")), efecto: { tipo: "ganancia", db } } });
        return { texto: "", hechas: [`${pista} · ${m[1]}–${m[2]} s · ${db > 0 ? "+" : ""}${db} dB`], preguntas: [], errores: [], vista: vista(s) };
      }
    }
    throw "comando del Experto sin simular: " + cmd;
  };
})();
