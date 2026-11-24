// Ola EXPERTO II — el estudio de música del proyecto (armazón, transporte y motor de escucha).
//
// Ocupa toda la ventana de GOTV; minimizado es una barra flotante sobre el chat. Mientras esté ACTIVO en un
// proyecto (aunque esté minimizado) el chat y el Pro de ese proyecto quedan bloqueados: el núcleo también se
// niega (`Agent::run`). La sesión (pistas, clips, buses, maestro, regiones, curvas) vive en el núcleo
// (`gotv_core::experto`): aquí sólo se pinta y se mandan operaciones que allí se validan.
//
// Las partes: `experto.js` (esto), `experto-arreglo.js` (la línea de tiempo), `experto-dock.js` (mezclador,
// canal, análisis, historial) y `experto-lado.js` (IA e inspector). Todas cuelgan de `EX`.
//
// ESCUCHA. El núcleo renderiza cada pista (clips + regiones + efectos) y la ventana le pone en vivo el fader,
// el paneo y las curvas (GainNode y StereoPannerNode: el mismo cálculo que `experto_dsp`). Si un bus o el
// maestro tienen efectos, lo que suena es la MEZCLA FINAL renderizada por el núcleo (`experto_mezcla_audio`):
// lo que se oye es lo que se exporta.
const EX = (window.EX = {});
(() => {
  const X = (EX.X = {
    root: "", v: null, abierta: false, mini: false,
    ctx: null, buffers: {}, picos: {}, firmas: {}, nodos: {}, grupos: {}, maestro: null, medL: null, medR: null, espectro: null,
    mezcla: { buffer: null, firma: "", pidiendo: "", ganancia: 0 }, fuentes: [], sonando: false, pos: 0, t0: 0, tic: 0,
    bucle: false, bucleR: null, metronomo: false, metroProx: 0,
    zoom: 60, alto: 84, sel: null, pistaSel: null, clipSel: null, regionSel: null, objetivo: { tipo: "pista", id: 0 },
    herramienta: "puntero", iman: "pulso", plegados: new Set(), auto: {}, dock: "mezclador", lado: "ia",
    dockAlto: 300, ladoAncho: 380, dockPlegado: false, portapapeles: null, trabajo: null, picosMax: {},
  });
  EX.CAB = 248; // el ancho de la cabecera de una pista
  EX.COLORES = ["#8a7df3", "#5fc48c", "#e3b26b", "#ef7682", "#7ea8f2", "#d77ad3", "#4fc3c7", "#f08a5d", "#a3b86c", "#9aa0b4"];
  // Las preferencias del estudio las recuerda Rust (`ventana_pref`, formato propio PREF; antes, localStorage
  // con JSON). Cada valor viaja como texto y se lee según el tipo de su valor por defecto.
  const leerPref = (txt, d) => (txt == null ? d : typeof d === "number" ? (Number.isFinite(+txt) ? +txt : d) : typeof d === "boolean" ? txt === "true" : txt);
  EX.recordar = (k, v) => { invoke("ventana_pref_poner", { clave: "ex." + k, valor: String(v) }).catch(() => {}); };
  X.latencia = null; // sin preferencia: la que mide el contexto de audio
  EX.prefsListas = Promise.all(
    ["zoom", "alto", "iman", "dock", "lado", "dockAlto", "ladoAncho", "dockPlegado", "metronomo", "latencia"].map((k) =>
      invoke("ventana_pref", { clave: "ex." + k }).then((t) => { X[k] = k === "latencia" ? (t == null ? null : leerPref(t, 0)) : leerPref(t, X[k]); }).catch(() => {}),
    ),
  ).then(() => { if (EX.pintar) EX.pintar(); });

  // ── utilidades ──────────────────────────────────────────────────────────
  const VACIO = { pistas: [], grupos: [], regiones: [], clips: [], bpm: 120, compas: 4, marcadores: [], automatizaciones: [], maestro: { ganancia_db: 0, insertos: {} }, tonalidad: "", letra: "" };
  const est = (EX.est = () => Object.assign({}, VACIO, (X.v && X.v.estado) || {}));
  EX.pista = (id) => est().pistas.find((p) => p.id === id);
  EX.clip = (id) => est().clips.find((c) => c.id === id);
  EX.grupo = (id) => est().grupos.find((g) => g.id === id);
  EX.clipsDe = (id) => est().clips.filter((c) => c.pista === id);
  EX.colorDe = (p) => p.color || EX.COLORES[(p.id - 1) % EX.COLORES.length];
  const dbLin = (EX.dbLin = (db) => Math.pow(10, db / 20));
  EX.linDb = (v) => 20 * Math.log10(Math.max(v, 1e-6));
  EX.fmtT = (s) => { s = Math.max(0, s); const m = Math.floor(s / 60); return `${m}:${(s - m * 60).toFixed(3).padStart(6, "0")}`; };
  EX.fmtDb = (db) => (db <= -59.5 ? "−∞" : (db > 0 ? "+" : db < 0 ? "−" : "") + Math.abs(db).toFixed(1));
  EX.pulso = () => 60 / (est().bpm || 120);
  EX.fmtCompas = (t) => { const e = est(), k = Math.floor(t / EX.pulso() + 1e-9), n = e.compas || 4; return `${Math.floor(k / n) + 1}.${(k % n) + 1}`; };
  // lo que dura un clip en la línea de tiempo (el núcleo la da; sin ella, lo que quede de su archivo)
  EX.durClip = (c) => (c.duracion != null ? c.duracion : Math.max(0, ((X.v && X.v.archivos && X.v.archivos[c.archivo]) || 0) - (c.desde || 0)) / (c.ritmo || 1));
  EX.durArchivo = (a) => (X.v && X.v.archivos && X.v.archivos[a]) || 0;
  EX.duracion = () => {
    let s = 0;
    for (const c of est().clips) s = Math.max(s, c.inicio + EX.durClip(c));
    for (const b of Object.values(X.buffers)) s = Math.max(s, b.duration);
    return s;
  };
  EX.largo = () => Math.max(EX.duracion() + 30, 60); // lo que se puede recorrer: 30 s de aire tras el final
  // el imán: el paso de la rejilla en segundos (0 = libre)
  EX.paso = () => {
    const p = EX.pulso(), n = est().compas || 4;
    return { compas: p * n, pulso: p, medio: p / 2, cuarto: p / 4, segundo: 1, decima: 0.1, libre: 0 }[X.iman] ?? p;
  };
  EX.iman = (t, ev) => { const g = EX.paso(); return Math.max(0, !g || (ev && ev.altKey) ? t : Math.round(t / g) * g); };
  EX.vacio = (i) => !i || !Object.keys(i).some((k) => (Array.isArray(i[k]) ? i[k].length : i[k] !== null && i[k] !== undefined));
  EX.icono = (d, extra) => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"${extra || ""}>${d}</svg>`;
  const I = (EX.I = {
    inicio: '<path d="M6 5v14M19 5l-9 7 9 7z"/>', play: '<path d="M7 4l13 8-13 8z" fill="currentColor"/>', pausa: '<path d="M7 4v16M17 4v16" stroke-width="3"/>',
    parar: '<rect x="6" y="6" width="12" height="12" rx="1.5" fill="currentColor"/>', bucle: '<path d="M17 2l4 4-4 4"/><path d="M3 11V9a3 3 0 0 1 3-3h15"/><path d="M7 22l-4-4 4-4"/><path d="M21 13v2a3 3 0 0 1-3 3H3"/>',
    metro: '<path d="M8 21h8l-3-18h-2z"/><path d="M12 14l6-8"/>', puntero: '<path d="M5 3l14 8-6 2-2 6z"/>', rango: '<path d="M4 6v12M20 6v12M8 12h8"/>',
    tijera: '<circle cx="6" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M20 4L8.1 15.9M14.5 14.5L20 20M8.1 8.1L12 12"/>',
    deshacer: '<path d="M9 14L4 9l5-5"/><path d="M4 9h10a6 6 0 0 1 0 12h-3"/>', rehacer: '<path d="M15 14l5-5-5-5"/><path d="M20 9H10a6 6 0 0 0 0 12h3"/>',
    mas: '<path d="M12 5v14M5 12h14"/>', menos: '<path d="M5 12h14"/>', ajustar: '<path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5"/>',
    min: '<path d="M5 19h14"/>', cerrar: '<path d="M6 6l12 12M18 6L6 18"/>', ayuda: '<circle cx="12" cy="12" r="9"/><path d="M9.5 9a2.5 2.5 0 1 1 3.5 2.3c-.7.4-1 1-1 1.7M12 17h.01"/>',
    importar: '<path d="M12 3v12M7 10l5 5 5-5M4 21h16"/>', exportar: '<path d="M12 15V3M7 8l5-5 5 5M4 21h16"/>', chispa: '<path d="M12 3l1.9 5.6L19.5 10l-5.6 1.9L12 17.5l-1.9-5.6L4.5 10l5.6-1.4z"/><path d="M19 16l.7 2 2 .7-2 .7-.7 2-.7-2-2-.7 2-.7z"/>',
    plegar: '<path d="M6 9l6 6 6-6"/>', grabar: '<circle cx="12" cy="12" r="6" fill="currentColor"/>', marcador: '<path d="M6 21V4h11l-2.5 4L17 12H6"/>',
  });

  // ── el núcleo ───────────────────────────────────────────────────────────
  const llamar = (EX.llamar = async (cmd, args) => {
    try { return await invoke(cmd, Object.assign({ proyecto: X.root }, args || {})); }
    catch (e) {
      const t = String(e && e.message ? e.message : e);
      toast("✗ " + t, 7000);
      // falta un modelo (separar, generar, extender…): a su fila en Configuración → Generadores
      if (typeof faltaEnAviso === "function") faltaEnAviso(t, null, true);
      return null;
    }
  });
  const op = (EX.op = async (o) => {
    const v = await llamar("experto_aplicar", { op: o });
    if (v) await actualizar(v);
    return v;
  });
  EX.deshacer = async (rehacer) => { const v = await llamar("experto_deshacer", { rehacer }); if (v) await actualizar(v); };
  // un trabajo de IA: uno a la vez, con su avance en el pie y su botón de cancelar
  EX.trabajo = async (titulo, cmd, args) => {
    if (X.trabajo) { toast(tt("exOcupado"), 4000); return null; }
    X.trabajo = { titulo, texto: "" };
    pintarPie();
    EX.lado && EX.lado.pintar();
    const r = await llamar(cmd, args);
    X.trabajo = null;
    pintarPie();
    if (r && r.vista) await actualizar(r.vista);
    else if (r && r.estado) await actualizar(r);
    EX.lado && EX.lado.pintar();
    return r;
  };

  // lo que define el audio renderizado de una pista (si cambia, se vuelve a pedir)
  const firma = (p) => JSON.stringify([EX.clipsDe(p.id), est().regiones.filter((r) => r.pista === p.id), p.insertos || {}]);
  // ¿hay efectos que la ventana no puede hacer en vivo? (un bus con efectos y pistas, o el maestro)
  const exacto = (EX.exacto = () => {
    const e = est();
    return !EX.vacio(e.maestro.insertos) || e.grupos.some((g) => !EX.vacio(g.insertos) && e.pistas.some((p) => p.grupo === g.id));
  });
  // lo que define la mezcla final SALVO la ganancia del maestro (va en vivo: es lo último y es lineal)
  const firmaMezcla = () => {
    const e = est();
    return JSON.stringify([e.pistas.map((p) => [firma(p), p.ganancia_db, p.pan, p.silencio, p.solo, p.grupo]), e.grupos, e.automatizaciones, e.maestro.insertos]);
  };

  async function actualizar(v) {
    X.v = v;
    const e = est();
    const vivas = new Set(e.pistas.map((p) => p.id));
    for (const id of Object.keys(X.buffers)) if (!vivas.has(Number(id))) { delete X.buffers[id]; delete X.firmas[id]; delete X.picos[id]; }
    if (X.pistaSel && !vivas.has(X.pistaSel)) X.pistaSel = null;
    if (X.clipSel && !EX.clip(X.clipSel)) X.clipSel = null;
    if (X.regionSel && !e.regiones.some((r) => r.id === X.regionSel)) X.regionSel = null;
    const cambian = e.pistas.filter((p) => X.firmas[p.id] !== firma(p));
    // las pistas cuyo audio hay que volver a pedir se marcan «calculando» (un clip transpuesto tarda segundos)
    X.cargando = new Set(cambian.map((p) => p.id));
    pintar();
    if (cambian.length) {
      const seguia = X.sonando;
      await Promise.all(cambian.map((p) => cargarPista(p).finally(() => X.cargando.delete(p.id))));
      pintar();
      if (seguia && X.sonando) { parar(); reproducir(); }
    }
    aplicarMezcla();
    pedirMezcla();
    if (X.sonando) programarCurvas();
  }
  EX.actualizar = actualizar;

  function calcularPicos(b) {
    const PASO = 256, n = Math.ceil(b.length / PASO), mn = new Float32Array(n), mx = new Float32Array(n);
    const a = b.getChannelData(0), c = b.numberOfChannels > 1 ? b.getChannelData(1) : a;
    for (let k = 0; k < n; k++) {
      let lo = 0, hi = 0;
      for (let i = k * PASO, f = Math.min(b.length, i + PASO); i < f; i++) { const v = (a[i] + c[i]) * 0.5; if (v < lo) lo = v; else if (v > hi) hi = v; }
      mn[k] = lo; mx[k] = hi;
    }
    return { mn, mx, paso: PASO };
  }
  async function cargarPista(p) {
    asegurarCtx();
    const f = firma(p);
    try {
      const b = await invoke("experto_pista_audio", { proyecto: X.root, pista: p.id });
      const buf = await X.ctx.decodeAudioData(b instanceof ArrayBuffer ? b.slice(0) : new Uint8Array(b).buffer);
      X.buffers[p.id] = buf; X.picos[p.id] = calcularPicos(buf); X.firmas[p.id] = f;
      EX.eqVivoFin(p.id); // el audio nuevo ya trae el EQ: la escucha previa sobra
    } catch (e) { toast(`✗ ${p.nombre}: ${e.message || e}`, 7000); X.firmas[p.id] = f; }
  }
  // la mezcla final del núcleo (sólo cuando hace falta), sin amontonar pedidos
  let esperaMezcla = 0;
  function pedirMezcla() {
    clearTimeout(esperaMezcla);
    if (!exacto()) { X.mezcla.buffer = null; X.mezcla.firma = ""; pintarEstadoEscucha(); return; }
    const f = firmaMezcla();
    if (f === X.mezcla.firma || f === X.mezcla.pidiendo) return;
    pintarEstadoEscucha(true);
    esperaMezcla = setTimeout(async () => {
      X.mezcla.pidiendo = f;
      asegurarCtx();
      try {
        const ganancia = est().maestro.ganancia_db || 0;
        const b = await invoke("experto_mezcla_audio", { proyecto: X.root });
        const buf = await X.ctx.decodeAudioData(b instanceof ArrayBuffer ? b.slice(0) : new Uint8Array(b).buffer);
        if (X.mezcla.pidiendo !== f) return; // llegó tarde: ya se pidió otra
        X.mezcla = { buffer: buf, firma: f, pidiendo: "", ganancia };
        aplicarMezcla();
        if (X.sonando) { parar(); reproducir(); }
      } catch (e) { toast("✗ " + (e.message || e), 7000); X.mezcla.pidiendo = ""; }
      pintarEstadoEscucha();
    }, 220);
  }
  function pintarEstadoEscucha(render) {
    const el = $("exEscucha"); if (!el) return;
    const ex = exacto(), pend = render || (ex && X.mezcla.firma !== firmaMezcla());
    el.hidden = !ex;
    el.textContent = pend ? tt("exRenderizando") : tt("exEscuchaExacta");
    el.classList.toggle("pend", !!pend);
    el.title = tt("exEscuchaAyuda");
  }

  // ── el armazón ──────────────────────────────────────────────────────────
  function montar() {
    if ($("experto")) return;
    const d = document.createElement("div");
    d.id = "experto"; d.className = "ex"; d.hidden = true;
    d.setAttribute("role", "dialog"); d.setAttribute("aria-label", tt("expertoTitulo"));
    const b = (id, ic, t, cls) => `<button type="button" class="ex-b${cls ? " " + cls : ""}" id="${id}" title="${esc(tt(t))}" aria-label="${esc(tt(t))}">${EX.icono(I[ic])}</button>`;
    const imanes = [["compas", "exImanCompas"], ["pulso", "exImanPulso"], ["medio", "exImanMedio"], ["cuarto", "exImanCuarto"], ["segundo", "exImanSegundo"], ["decima", "exImanDecima"], ["libre", "exImanLibre"]];
    const tonos = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"].flatMap((n) => [`${n} major`, `${n} minor`]);
    d.innerHTML = `
      <header class="ex-barra">
        <div class="ex-marca"><span class="ex-logo">${EX.icono(I.chispa)}</span><b>${esc(tt("expertoTitulo"))}</b><span class="ex-proy" id="exProy"></span></div>
        <div class="ex-grupo-b ex-transporte">
          ${b("exInicio", "inicio", "exInicio")}${b("exPlay", "play", "exPlay", "ex-play")}${b("exGrabar", "grabar", "exGrabar", "ex-rec")}${b("exBucle", "bucle", "exBucle")}${b("exMetro", "metro", "exMetronomo")}
        </div>
        <div class="ex-reloj" title="${esc(tt("exRelojAyuda"))}"><span class="mono ex-pos" id="exPos">0:00.000</span><span class="mono ex-compas" id="exCompasPos">1.1</span></div>
        <div class="ex-grupo-b ex-tempo">
          <label class="ex-campo" title="${esc(tt("exTempo"))}"><span>BPM</span><input type="number" id="exBpm" min="20" max="400" step="1" class="mono"></label>
          <button type="button" class="ex-t" id="exBpmMitad" title="${esc(tt("exTempoMitad"))}">÷2</button><button type="button" class="ex-t" id="exBpmDoble" title="${esc(tt("exTempoDoble"))}">×2</button><button type="button" class="ex-t" id="exTap" title="${esc(tt("exTap"))}">TAP</button>
          <label class="ex-campo" title="${esc(tt("exCompasT"))}"><select id="exCompas">${[2, 3, 4, 5, 6, 7, 12].map((n) => `<option value="${n}">${n}/4</option>`).join("")}</select></label>
          <label class="ex-campo" title="${esc(tt("exTonalidad"))}"><select id="exTono"><option value="">${esc(tt("exSinTonalidad"))}</option>${tonos.map((t) => `<option>${t}</option>`).join("")}</select></label>
        </div>
        <div class="ex-grupo-b ex-herr" role="radiogroup" aria-label="${esc(tt("exHerramientas"))}">
          ${b("exHPuntero", "puntero", "exHPuntero")}${b("exHRango", "rango", "exHRango")}${b("exHTijera", "tijera", "exHTijera")}
        </div>
        <div class="ex-grupo-b">
          <label class="ex-campo" title="${esc(tt("exIman"))}"><select id="exIman" aria-label="${esc(tt("exImanCorto"))}">${imanes.map(([k, t]) => `<option value="${k}">${esc(tt(t))}</option>`).join("")}</select></label>
          ${b("exZoomMenos", "menos", "exZoomMenos")}<input type="range" id="exZoom" min="0" max="100" step="1" title="${esc(tt("exZoom"))}" aria-label="${esc(tt("exZoom"))}">${b("exZoomMas", "mas", "exZoomMas")}${b("exAjustar", "ajustar", "exAjustar")}
          <button type="button" class="ex-t" id="exAlto" title="${esc(tt("exAlto"))}">↕</button>
        </div>
        <span class="ex-escucha" id="exEscucha" hidden></span>
        <div class="ex-grupo-b ex-fin">
          ${b("exDeshacer", "deshacer", "exDeshacer")}${b("exRehacer", "rehacer", "exRehacer")}
          ${b("exImportar", "importar", "exImportarAyuda")}
          <button type="button" class="ex-bt primario" id="exExportar">${EX.icono(I.exportar)}<span>${esc(tt("exExportar"))}</span></button>
          ${b("exAyuda", "ayuda", "exAyuda")}${b("exMin", "min", "exMinimizar")}${b("exCerrar", "cerrar", "exCerrar")}
        </div>
      </header>
      <div class="ex-cuerpo" id="exCuerpo">
        <div class="ex-izq" id="exIzq">
          <div class="ex-arreglo" id="exArreglo"></div>
          <div class="ex-partidor h" id="exPartDock" title="${esc(tt("exArrastra"))}"></div>
          <div class="ex-dock" id="exDock"></div>
        </div>
        <div class="ex-partidor v" id="exPartLado" title="${esc(tt("exArrastra"))}"></div>
        <aside class="ex-lado" id="exLado"></aside>
      </div>
      <footer class="ex-pie" id="exPie"></footer>`;
    document.body.appendChild(d);
    const m = document.createElement("div");
    m.id = "exMini"; m.className = "ex-mini"; m.hidden = true;
    m.innerHTML = `<span class="ex-logo">${EX.icono(I.chispa)}</span><b>${esc(tt("expertoTitulo"))}</b><span class="ex-proy" id="exMiniProy"></span>
      <button type="button" class="ex-b" id="exMiniPlay" title="${esc(tt("exPlay"))}">${EX.icono(I.play)}</button><span class="mono" id="exMiniPos">0:00.000</span>
      <button type="button" class="ex-b" id="exMiniMax" title="${esc(tt("exRestaurar"))}">${EX.icono(I.ajustar)}</button>
      <button type="button" class="ex-b" id="exMiniCerrar" title="${esc(tt("exCerrar"))}">${EX.icono(I.cerrar)}</button>`;
    ($("log") || document.body).parentElement.appendChild(m);

    const clic = (id, f) => $(id).addEventListener("click", (e) => { e.stopPropagation(); f(e); });
    clic("exPlay", alternar); clic("exMiniPlay", alternar);
    clic("exInicio", () => irA(X.bucle && X.bucleR ? X.bucleR.desde : 0));
    clic("exBucle", alternarBucle);
    clic("exGrabar", grabar);
    clic("exMetro", () => { X.metronomo = !X.metronomo; EX.recordar("metronomo", X.metronomo); X.metroProx = 0; pintarBarra(); });
    clic("exDeshacer", () => EX.deshacer(false)); clic("exRehacer", () => EX.deshacer(true));
    clic("exImportar", importar); clic("exExportar", () => EX.dialogoExportar());
    clic("exAyuda", () => EX.dialogoAyuda());
    clic("exMin", () => minimizar(true)); clic("exMiniMax", () => minimizar(false));
    clic("exCerrar", cerrar); clic("exMiniCerrar", cerrar);
    for (const [id, h] of [["exHPuntero", "puntero"], ["exHRango", "rango"], ["exHTijera", "tijera"]]) clic(id, () => herramienta(h));
    const tempo = (bpm) => op({ op: "tempo", bpm: Math.min(400, Math.max(20, bpm)), compas: Number($("exCompas").value) });
    $("exBpm").addEventListener("change", () => tempo(Number($("exBpm").value)));
    $("exCompas").addEventListener("change", () => tempo(Number($("exBpm").value)));
    clic("exBpmMitad", () => tempo(est().bpm / 2)); clic("exBpmDoble", () => tempo(est().bpm * 2));
    let toques = [];
    clic("exTap", () => {
      const t = performance.now();
      toques = toques.filter((x) => t - x < 3000).concat(t);
      if (toques.length >= 4) { const d = (toques[toques.length - 1] - toques[0]) / (toques.length - 1); tempo(Math.round(60000 / d)); }
      else toast(tt("exTapSigue").replace("{n}", 4 - toques.length), 1200);
    });
    $("exTono").addEventListener("change", () => op({ op: "tonalidad", tonalidad: $("exTono").value }));
    $("exIman").addEventListener("change", () => { X.iman = $("exIman").value; EX.recordar("iman", X.iman); EX.arreglo.pintar(); });
    // el zoom va de «todo cabe» a 600 px por segundo, en escala logarítmica
    const zMin = 2, zMax = 600, aZoom = (v) => zMin * Math.pow(zMax / zMin, v / 100), deZoom = (z) => (100 * Math.log(z / zMin)) / Math.log(zMax / zMin);
    EX.ponerZoom = (z, ancla) => {
      const sc = $("exScroll"), antes = X.zoom;
      z = Math.min(zMax, Math.max(zMin, z));
      const ax = ancla === undefined ? (sc ? (sc.clientWidth - EX.CAB) / 2 : 0) : ancla, t = sc ? (sc.scrollLeft + ax) / antes : 0;
      X.zoom = z; EX.recordar("zoom", z);
      $("exZoom").value = deZoom(z);
      EX.arreglo.pintar();
      if (sc) sc.scrollLeft = Math.max(0, t * z - ax);
    };
    $("exZoom").addEventListener("input", () => EX.ponerZoom(aZoom(Number($("exZoom").value))));
    clic("exZoomMenos", () => EX.ponerZoom(X.zoom / 1.4)); clic("exZoomMas", () => EX.ponerZoom(X.zoom * 1.4));
    clic("exAjustar", () => { const sc = $("exScroll"); EX.ponerZoom((sc.clientWidth - EX.CAB - 24) / Math.max(4, EX.duracion()), 0); sc.scrollLeft = 0; });
    clic("exAlto", () => { X.alto = X.alto === 56 ? 84 : X.alto === 84 ? 124 : 56; EX.recordar("alto", X.alto); EX.arreglo.pintar(); });
    partidor($("exPartDock"), "y", () => X.dockAlto, (v) => { X.dockAlto = Math.min(window.innerHeight - 220, Math.max(140, v)); EX.recordar("dockAlto", X.dockAlto); medidas(); EX.arreglo.pintar(); EX.dock.redibujar(); });
    partidor($("exPartLado"), "x", () => X.ladoAncho, (v) => { X.ladoAncho = Math.min(620, Math.max(300, v)); EX.recordar("ladoAncho", X.ladoAncho); medidas(); EX.arreglo.pintar(); });
    window.addEventListener("resize", () => X.abierta && !X.mini && pintar());
    document.addEventListener("keydown", teclas, true);
    if (window.__TAURI__ && window.__TAURI__.event) window.__TAURI__.event.listen("experto-avance", (e) => { if (X.trabajo) { X.trabajo.texto = String(e.payload || ""); pintarPie(); } });
    EX.arreglo.montar(); EX.dock.montar(); EX.lado.montar();
    medidas();
  }
  // un partidor arrastrable entre dos paneles (el valor crece hacia arriba o hacia la izquierda)
  function partidor(el, eje, leer, poner) {
    el.addEventListener("mousedown", (ev) => {
      ev.preventDefault();
      const p0 = eje === "y" ? ev.clientY : ev.clientX, v0 = leer();
      const mover = (e) => poner(v0 + p0 - (eje === "y" ? e.clientY : e.clientX));
      const soltar = () => { document.removeEventListener("mousemove", mover); document.removeEventListener("mouseup", soltar); document.body.classList.remove("ex-arrastrando"); };
      document.body.classList.add("ex-arrastrando");
      document.addEventListener("mousemove", mover); document.addEventListener("mouseup", soltar);
    });
  }
  function medidas() {
    const c = $("exCuerpo"); if (!c) return;
    c.style.setProperty("--ex-lado", X.ladoAncho + "px");
    c.style.setProperty("--ex-dock", (X.dockPlegado ? 34 : X.dockAlto) + "px");
    $("exIzq").classList.toggle("dock-plegado", !!X.dockPlegado);
  }
  EX.medidas = medidas;
  function herramienta(h) { X.herramienta = h; pintarBarra(); $("exArreglo").dataset.herr = h; }
  EX.herramienta = herramienta;

  function teclas(e) {
    if (!X.abierta || X.mini) return;
    if (document.querySelector(".ex-dialogo")) { if (e.key === "Escape") { e.preventDefault(); document.querySelectorAll(".ex-dialogo").forEach((d) => d.remove()); } return; }
    if (e.key === "Escape") document.querySelectorAll(".ex-menu").forEach((m) => m.cerrar && m.cerrar());
    const t = e.target, mod = e.metaKey || e.ctrlKey, k = e.key.toLowerCase();
    const escribiendo = t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable);
    if (mod && k === "z") { e.preventDefault(); EX.deshacer(e.shiftKey); return; }
    if (escribiendo) { if (e.key === "Escape") t.blur(); return; }
    const A = EX.arreglo;
    if (e.code === "Space") { e.preventDefault(); alternar(); }
    else if (mod && k === "e") { e.preventDefault(); A.partirEnCabezal(); }
    else if (mod && k === "d") { e.preventDefault(); A.duplicarClip(); }
    else if (mod && k === "c") { if (X.clipSel) { e.preventDefault(); X.portapapeles = X.clipSel; toast(tt("exCopiado"), 1200); } }
    else if (mod && k === "v") { e.preventDefault(); A.pegarClip(); }
    else if (mod) return;
    else if (k === "v") herramienta("puntero");
    else if (k === "r") herramienta("rango");
    else if (k === "t") herramienta("tijera");
    else if (k === "e") A.partirEnCabezal();
    else if (k === "l") alternarBucle();
    else if (k === "k") $("exMetro").click();
    else if (k === "f") $("exAjustar").click();
    else if (k === "m" && e.shiftKey) A.marcadorAqui();
    else if (k === "m" && X.pistaSel) { const p = EX.pista(X.pistaSel); if (p) op({ op: "mezcla", pista: p.id, silencio: !p.silencio }); }
    else if (k === "s" && X.pistaSel) { const p = EX.pista(X.pistaSel); if (p) op({ op: "mezcla", pista: p.id, solo: !p.solo }); }
    else if (e.key === "Backspace" || e.key === "Delete") { e.preventDefault(); A.borrarSeleccion(); }
    else if (e.key === "Home" || e.key === "Enter") { e.preventDefault(); irA(0); }
    else if (e.key === "End") irA(EX.duracion());
    else if (e.key === "ArrowLeft" || e.key === "ArrowRight") { e.preventDefault(); const g = EX.paso() || 0.1; irA(Math.max(0, X.pos + (e.key === "ArrowLeft" ? -g : g) * (e.shiftKey ? 4 : 1))); }
    else if (e.key === "+" || e.key === "=") EX.ponerZoom(X.zoom * 1.4);
    else if (e.key === "-") EX.ponerZoom(X.zoom / 1.4);
    else if (e.key === "?") EX.dialogoAyuda();
    else if (e.key === "Escape") { X.sel = null; X.regionSel = null; X.clipSel = null; pintar(); }
  }

  // ── abrir, minimizar, cerrar y el candado ──────────────────────────────
  async function proyecto(root) {
    if (X.root && X.root !== root) { parar(); X.buffers = {}; X.firmas = {}; X.picos = {}; X.sel = null; X.pistaSel = null; X.clipSel = null; X.mezcla = { buffer: null, firma: "", pidiendo: "", ganancia: 0 }; }
    X.root = root;
    montar();
    const v = root ? await llamar("experto_sesion") : null;
    X.v = v;
    X.abierta = !!(v && v.activa);
    X.mini = !!(v && v.minimizada);
    mostrar();
    if (X.abierta) await actualizar(v);
  }
  async function abrir() {
    if (!X.root && S.root) await proyecto(S.root);
    if (!X.root) return toast(tt("ctxNoProject"));
    montar();
    const v = await llamar("experto_activar", { activa: true, minimizada: false });
    if (!v) return;
    X.abierta = true; X.mini = false;
    mostrar();
    await actualizar(v);
  }
  async function minimizar(si) {
    const v = await llamar("experto_activar", { activa: true, minimizada: si });
    if (!v) return;
    X.mini = si;
    mostrar();
    if (!si) pintar();
  }
  async function cerrar() {
    parar();
    const v = await llamar("experto_activar", { activa: false });
    if (!v) return;
    X.abierta = false; X.mini = false;
    mostrar();
  }
  function mostrar() {
    const d = $("experto"), m = $("exMini");
    if (!d) return;
    const nombre = ((S.projects || []).find((p) => p.path === X.root) || {}).name || (X.root || "").split("/").filter(Boolean).pop() || "";
    $("exProy").textContent = nombre; $("exMiniProy").textContent = nombre;
    d.hidden = !(X.abierta && !X.mini);
    m.hidden = !(X.abierta && X.mini);
    document.body.classList.toggle("experto-activo", X.abierta);
    // el candado: el chat y el Pro de ESTE proyecto
    const input = $("in");
    if (input) {
      if (X.abierta) { input.dataset.phAntes = input.dataset.phAntes || input.placeholder; input.placeholder = tt("expertoPh"); }
      else if (input.dataset.phAntes !== undefined) { input.placeholder = input.dataset.phAntes; delete input.dataset.phAntes; }
      input.disabled = X.abierta || !S.ready;
    }
    const pb = $("proBtn");
    if (pb) pb.disabled = X.abierta;
    if (X.abierta && $("proPanel")) $("proPanel").hidden = true;
    // al liberar, el panel Pro vuelve como estaba (encendido = abierto o en su barra); medido en la app real
    else if (!X.abierta && typeof paintPro === "function") paintPro();
    if (!d.hidden) pintar();
  }
  const bloquea = () => X.abierta;

  // ── pintar ──────────────────────────────────────────────────────────────
  function pintarBarra() {
    const e = est();
    if (document.activeElement !== $("exBpm")) $("exBpm").value = Math.round((e.bpm || 120) * 10) / 10;
    $("exCompas").value = String(e.compas || 4);
    if (![...$("exTono").options].some((o) => o.value === e.tonalidad) && e.tonalidad) $("exTono").add(new Option(e.tonalidad, e.tonalidad));
    $("exTono").value = e.tonalidad || "";
    $("exIman").value = X.iman;
    $("exDeshacer").disabled = !(X.v && X.v.deshacer);
    $("exRehacer").disabled = !(X.v && X.v.rehacer);
    $("exBucle").classList.toggle("on", X.bucle); $("exBucle").setAttribute("aria-pressed", String(X.bucle));
    $("exMetro").classList.toggle("on", X.metronomo); $("exMetro").setAttribute("aria-pressed", String(X.metronomo));
    $("exGrabar").classList.toggle("grabando", !!X.grabando); $("exGrabar").setAttribute("aria-pressed", String(!!X.grabando)); $("exGrabar").title = tt(X.grabando ? "exGrabarParar" : "exGrabar");
    for (const [id, h] of [["exHPuntero", "puntero"], ["exHRango", "rango"], ["exHTijera", "tijera"]]) { $(id).classList.toggle("on", X.herramienta === h); $(id).setAttribute("aria-checked", String(X.herramienta === h)); $(id).setAttribute("role", "radio"); }
    $("exArreglo").dataset.herr = X.herramienta;
    pintarEstadoEscucha();
  }
  function pintarPie() {
    const p = $("exPie"); if (!p) return;
    const e = est(), t = X.trabajo;
    const sel = X.sel && X.sel.hasta > X.sel.desde ? `${tt("exSeleccion")}: ${X.sel.desde.toFixed(2)}–${X.sel.hasta.toFixed(2)} s (${(X.sel.hasta - X.sel.desde).toFixed(2)} s)` : "";
    const pista = X.pistaSel && EX.pista(X.pistaSel) ? `${tt("exPista")} ${X.pistaSel} · ${EX.pista(X.pistaSel).nombre}` : "";
    const g = X.grabando;
    p.innerHTML = g
      ? `<span class="ex-trabajo"><i class="ex-rec-luz"></i><b class="rec">${esc(tt("exGrabando"))}</b><span class="ex-trabajo-t mono">${EX.fmtT((performance.now() - g.t) / 1000).slice(0, -2)}</span><button type="button" class="ex-t" id="exGrabarFin">${esc(tt("exGrabarParar"))}</button><button type="button" class="ex-t peligro" id="exGrabarNo">${esc(tt("exDescartar"))}</button></span>`
      : t
      ? `<span class="ex-trabajo"><span class="ex-giro"></span><b>${esc(t.titulo)}</b><span class="ex-trabajo-t">${esc(t.texto || tt("exTrabajando"))}</span><button type="button" class="ex-t" id="exCancelar">${esc(tt("exCancelar"))}</button></span>`
      : `<span>${esc(pista)}</span><span>${esc(sel)}</span>`;
    p.innerHTML += `<span class="ex-pie-fin mono">${e.pistas.length} ${esc(tt("exPistas"))} · ${EX.fmtT(EX.duracion()).slice(0, -4)} · ${((X.v && X.v.frecuencia) || 48000) / 1000} kHz</span>`;
    const c = $("exCancelar");
    if (c) c.onclick = () => { invoke("experto_cancelar").catch(() => {}); c.disabled = true; c.textContent = tt("exCancelando"); };
    if ($("exGrabarFin")) { $("exGrabarFin").onclick = () => pararGrabacion(false); $("exGrabarNo").onclick = () => pararGrabacion(true); }
  }
  EX.pintarPie = pintarPie;
  function pintar() {
    if (!X.abierta || X.mini || !$("experto")) return;
    medidas();
    pintarBarra();
    EX.arreglo.pintar();
    EX.dock.pintar();
    EX.lado.pintar();
    pintarPie();
    pintarCabezal();
  }
  EX.pintar = pintar;
  function pintarCabezal() {
    if (!$("exPos")) return;
    $("exPos").textContent = EX.fmtT(X.pos); $("exMiniPos").textContent = EX.fmtT(X.pos);
    $("exCompasPos").textContent = EX.fmtCompas(X.pos);
    const ic = EX.icono(X.sonando ? I.pausa : I.play);
    if ($("exPlay").dataset.s !== String(X.sonando)) { $("exPlay").innerHTML = ic; $("exMiniPlay").innerHTML = ic; $("exPlay").dataset.s = String(X.sonando); $("exPlay").classList.toggle("on", X.sonando); $("exPlay").title = tt(X.sonando ? "exPausa" : "exPlay"); }
    EX.arreglo.cabezal();
  }
  EX.pintarCabezal = pintarCabezal;

  async function importar() {
    let rutas = [];
    try { rutas = await invoke("pick_files"); } catch (e) { return toast("✗ " + e); }
    for (const ruta of rutas || []) {
      const v = await llamar("experto_importar", { ruta });
      if (v) await actualizar(v);
    }
  }
  EX.importar = importar;

  // ── el motor de audio ───────────────────────────────────────────────────
  function asegurarCtx() {
    if (X.ctx) return;
    X.ctx = new (window.AudioContext || window.webkitAudioContext)({ sampleRate: (X.v && X.v.frecuencia) || 48000 });
    X.maestro = X.ctx.createGain();
    const partir = X.ctx.createChannelSplitter(2);
    X.medL = X.ctx.createAnalyser(); X.medR = X.ctx.createAnalyser(); X.espectro = X.ctx.createAnalyser();
    X.medL.fftSize = X.medR.fftSize = 1024; X.espectro.fftSize = 4096; X.espectro.smoothingTimeConstant = 0.75;
    X.maestro.connect(X.ctx.destination); X.maestro.connect(partir); X.maestro.connect(X.espectro);
    partir.connect(X.medL, 0); partir.connect(X.medR, 1);
    X.clic = X.ctx.createGain(); X.clic.gain.value = 0.5; X.clic.connect(X.ctx.destination);
    // los medidores cuelgan de un nodo a cero unido a la salida: un nodo suelto no se procesa
    X.mudo = X.ctx.createGain(); X.mudo.gain.value = 0; X.mudo.connect(X.ctx.destination);
  }
  EX.asegurarCtx = asegurarCtx;
  // ¿suena? (como `experto_dsp::suena`)
  function suena(p) {
    const e = est(), g = p.grupo && e.grupos.find((x) => x.id === p.grupo);
    if (p.silencio || (g && g.silencio)) return false;
    const haySolo = e.pistas.some((x) => x.solo) || e.grupos.some((x) => x.solo);
    return !haySolo || p.solo || !!(g && g.solo);
  }
  EX.suena = suena;
  const curvaDe = (id, par) => est().automatizaciones.find((a) => a.pista === id && a.parametro === par && a.puntos.length);
  EX.curvaDe = curvaDe;
  EX.valorCurva = (a, t) => {
    const p = a.puntos, i = p.findIndex((x) => x.t > t);
    if (i === 0) return p[0].v;
    if (i < 0) return p[p.length - 1].v;
    const [u, w] = [p[i - 1], p[i]];
    return u.v + (w.v - u.v) * (w.t > u.t ? (t - u.t) / (w.t - u.t) : 1);
  };
  function aplicarMezcla() {
    if (!X.ctx) return;
    const e = est(), ex = exacto() && X.mezcla.buffer;
    for (const g of e.grupos) {
      const n = X.grupos[g.id] || (X.grupos[g.id] = { gan: X.ctx.createGain(), pan: X.ctx.createStereoPanner(), vu: X.ctx.createAnalyser() });
      if (!n.unido) { n.gan.connect(n.pan); n.pan.connect(n.vu); n.vu.fftSize = 512; n.vu.connect(X.mudo); n.unido = true; }
      const sale = !ex;
      if (n.sale !== sale) { try { n.pan.disconnect(X.maestro); } catch (_) {} if (sale) n.pan.connect(X.maestro); n.sale = sale; }
      n.gan.gain.value = dbLin(g.ganancia_db); n.pan.pan.value = g.pan;
    }
    for (const p of e.pistas) {
      const n = X.nodos[p.id] || (X.nodos[p.id] = { gan: X.ctx.createGain(), curva: X.ctx.createGain(), pan: X.ctx.createStereoPanner(), vu: X.ctx.createAnalyser() });
      if (!n.unido) { n.gan.connect(n.curva); n.curva.connect(n.pan); n.pan.connect(n.vu); n.vu.fftSize = 512; n.vu.connect(X.mudo); n.unido = true; }
      // en escucha exacta la pista sólo alimenta su medidor: lo que suena es la mezcla del núcleo
      const destino = ex ? null : p.grupo && X.grupos[p.grupo] ? X.grupos[p.grupo].gan : X.maestro;
      if (n.destino !== destino) {
        if (n.destino) { try { n.pan.disconnect(n.destino); } catch (_) {} }
        if (destino) n.pan.connect(destino);
        n.destino = destino;
      }
      n.gan.gain.value = suena(p) ? dbLin(p.ganancia_db) : 0;
      if (!curvaDe(p.id, "pan")) { n.pan.pan.cancelScheduledValues(0); n.pan.pan.value = p.pan; }
      if (!curvaDe(p.id, "volumen")) { n.curva.gain.cancelScheduledValues(0); n.curva.gain.value = 1; }
    }
    // la ganancia del maestro va en vivo: en escucha exacta, sólo lo que cambió desde el render
    X.maestro.gain.value = dbLin((e.maestro.ganancia_db || 0) - (ex ? X.mezcla.ganancia : 0));
  }
  EX.aplicarMezcla = aplicarMezcla;
  // las curvas de volumen (rectas en dB = rampas exponenciales) y de paneo, desde la posición actual
  function programarCurvas() {
    if (!X.ctx) return;
    const ahora = X.ctx.currentTime, pos = Math.max(0, ahora - X.t0);
    for (const p of est().pistas) {
      const n = X.nodos[p.id]; if (!n) continue;
      for (const [par, param, conv] of [["volumen", n.curva.gain, dbLin], ["pan", n.pan.pan, (v) => v]]) {
        const a = curvaDe(p.id, par);
        param.cancelScheduledValues(0);
        if (!a) continue;
        param.setValueAtTime(conv(EX.valorCurva(a, pos)), ahora);
        for (const q of a.puntos) if (q.t > pos) { par === "volumen" ? param.exponentialRampToValueAtTime(Math.max(1e-4, conv(q.v)), X.t0 + q.t) : param.linearRampToValueAtTime(conv(q.v), X.t0 + q.t); }
      }
    }
  }
  EX.vivoPista = (id, db, pan) => { const n = X.nodos[id], p = EX.pista(id); if (!n || !p) return; if (db !== null) n.gan.gain.value = suena(p) ? dbLin(db) : 0; if (pan !== null && !curvaDe(id, "pan")) n.pan.pan.value = pan; };
  EX.vivoGrupo = (id, db, pan) => { const n = X.grupos[id]; if (!n) return; if (db !== null) n.gan.gain.value = dbLin(db); if (pan !== null) n.pan.pan.value = pan; };
  EX.vivoMaestro = (db) => { if (X.maestro) X.maestro.gain.value = dbLin(db - (exacto() && X.mezcla.buffer ? X.mezcla.ganancia : 0)); };

  // ── el EQ, AUDIBLE mientras se arrastra una banda (X2-8) ───────────────
  // El audio de la pista ya trae su EQ «horneado» por el núcleo. Para oír el cambio sin esperar al render,
  // mientras dura el arrastre se intercalan dos filtros de Web Audio: el INVERSO de la banda vieja (una
  // campana o un estante con la ganancia cambiada de signo lo es exactamente: mismas fórmulas RBJ) y la banda
  // nueva. Es una escucha previa: va detrás de los demás efectos del canal y no delante, así que con un
  // compresor puesto difiere un poco; al soltar, el núcleo renderiza y suena lo exacto. Los filtros sin
  // ganancia (paso alto y bajo, muesca, paso banda) no tienen inverso: sólo se oyen en vivo si son nuevos.
  const TIPO_WA = { campana: "peaking", graves: "lowshelf", agudos: "highshelf", paso_alto: "highpass", paso_bajo: "lowpass", muesca: "notch", paso_banda: "bandpass" };
  const conInverso = (b) => b && (b.filtro === "campana" || b.filtro === "graves" || b.filtro === "agudos");
  function ponerBanda(f, b, signo) {
    const t = X.ctx.currentTime;
    f.type = TIPO_WA[b.filtro] || "peaking";
    f.frequency.setTargetAtTime(b.frecuencia, t, 0.01);
    // Web Audio quiere la Q en dB para el paso alto y el paso bajo, como hace el núcleo
    f.Q.setTargetAtTime(b.filtro === "paso_alto" || b.filtro === "paso_bajo" ? 20 * Math.log10(Math.max(0.1, b.q)) : b.q, t, 0.01);
    f.gain.setTargetAtTime((b.db || 0) * signo, t, 0.01);
  }
  EX.eqVivoInicio = (id, vieja) => {
    const n = X.nodos[id];
    if (!n || !X.ctx || n.eqVivo || (exacto() && X.mezcla.buffer)) return;
    // una banda vieja sin inverso (o apagada, que no suena) no se puede deshacer en vivo
    if (vieja && !vieja.apagada && !conInverso(vieja)) return;
    const inv = vieja && !vieja.apagada ? X.ctx.createBiquadFilter() : null, nueva = X.ctx.createBiquadFilter();
    if (inv) ponerBanda(inv, vieja, -1);
    ponerBanda(nueva, vieja && !vieja.apagada ? vieja : { filtro: "campana", frecuencia: 1000, db: 0, q: 1 }, 1);
    try { n.gan.disconnect(n.curva); } catch (_) {}
    n.gan.connect(inv || nueva); if (inv) inv.connect(nueva); nueva.connect(n.curva);
    n.eqVivo = { inv, nueva };
  };
  EX.eqVivoMover = (id, b) => { const n = X.nodos[id]; if (n && n.eqVivo && b && !b.apagada) ponerBanda(n.eqVivo.nueva, b, 1); };
  EX.eqVivoFin = (id) => {
    const n = X.nodos[id]; if (!n || !n.eqVivo) return;
    try { n.gan.disconnect(); } catch (_) {}
    try { n.eqVivo.nueva.disconnect(); if (n.eqVivo.inv) n.eqVivo.inv.disconnect(); } catch (_) {}
    n.gan.connect(n.curva);
    n.eqVivo = null;
  };

  const hayCurvas = () => est().automatizaciones.some((a) => a.puntos.length);
  function reproducir() {
    asegurarCtx();
    if (X.ctx.state === "suspended") X.ctx.resume();
    aplicarMezcla();
    const bucle = X.bucle && X.bucleR && X.bucleR.hasta > X.bucleR.desde ? X.bucleR : null;
    if (bucle && (X.pos < bucle.desde || X.pos >= bucle.hasta)) X.pos = bucle.desde;
    if (!bucle && X.pos >= EX.duracion()) X.pos = 0;
    const t = X.ctx.currentTime + 0.06;
    // con curvas, el bucle se hace volviendo a empezar (las rampas no dan la vuelta solas)
    const nativo = bucle && !hayCurvas();
    X.fuentes = [];
    const lanzar = (buf, destino) => {
      if (!buf || X.pos >= buf.duration) return;
      const s = X.ctx.createBufferSource();
      s.buffer = buf;
      if (nativo && bucle.desde < buf.duration) { s.loop = true; s.loopStart = bucle.desde; s.loopEnd = Math.min(bucle.hasta, buf.duration); }
      s.connect(destino);
      s.start(t, X.pos);
      X.fuentes.push(s);
    };
    for (const p of est().pistas) lanzar(X.buffers[p.id], X.nodos[p.id].gan);
    if (exacto() && X.mezcla.buffer) lanzar(X.mezcla.buffer, X.maestro);
    X.t0 = t - X.pos; X.sonando = true; X.vuelta = bucle ? { ...bucle, nativo } : null; X.metroProx = 0;
    programarCurvas();
    clearInterval(X.tic);
    // un temporizador y no requestAnimationFrame: éste se detiene con la ventana oculta o detrás de otra
    X.tic = setInterval(paso, 33);
    pintarCabezal();
  }
  function paso() {
    if (!X.sonando) return;
    let pos = X.ctx.currentTime - X.t0;
    const b = X.vuelta;
    if (b && b.nativo && pos >= b.hasta) pos = b.desde + ((pos - b.desde) % (b.hasta - b.desde));
    else if (b && !b.nativo && pos >= b.hasta) { parar(); X.pos = b.desde; reproducir(); return; }
    X.pos = Math.max(0, pos);
    if (!b && X.pos >= EX.duracion() + 0.05) { parar(); X.pos = 0; pintarCabezal(); return; }
    if (X.metronomo) tocarMetronomo();
    pintarCabezal(); medir();
  }
  // el metrónomo: programa los clics del próximo cuarto de segundo (el primero del compás, más agudo)
  function tocarMetronomo() {
    const p = EX.pulso(), n = est().compas || 4, ahora = X.ctx.currentTime;
    if (X.vuelta && X.vuelta.nativo) return; // en bucle nativo la posición da la vuelta: sin clic
    let k = Math.max(X.metroProx, Math.ceil((ahora - X.t0) / p - 1e-6));
    for (; X.t0 + k * p < ahora + 0.25; k++) {
      const t = X.t0 + k * p;
      if (t < ahora) continue;
      const o = X.ctx.createOscillator(), g = X.ctx.createGain();
      o.frequency.value = k % n === 0 ? 1760 : 1175;
      g.gain.setValueAtTime(0.0001, t); g.gain.exponentialRampToValueAtTime(k % n === 0 ? 0.9 : 0.55, t + 0.002); g.gain.exponentialRampToValueAtTime(0.0001, t + 0.05);
      o.connect(g); g.connect(X.clic); o.start(t); o.stop(t + 0.06);
    }
    X.metroProx = k;
  }
  function parar() {
    for (const s of X.fuentes) { try { s.stop(); } catch (_) {} }
    X.fuentes = [];
    if (X.sonando && X.ctx) { const pos = X.ctx.currentTime - X.t0, b = X.vuelta; X.pos = Math.max(0, b && b.nativo && pos >= b.hasta ? b.desde + ((pos - b.desde) % (b.hasta - b.desde)) : pos); }
    X.sonando = false;
    clearInterval(X.tic);
    if (X.ctx) for (const n of Object.values(X.nodos)) { n.curva.gain.cancelScheduledValues(0); n.pan.pan.cancelScheduledValues(0); }
    if ($("exPlay")) { pintarCabezal(); medir(true); }
  }
  // ── grabar del micrófono a una pista nueva (X2-7) ──────────────────────
  // El micrófono lo abre el núcleo (nativo). Al empezar suena la sesión (para tocar encima) y al parar la
  // toma queda como pista, con su clip donde estaba el cabezal MENOS la latencia: lo que se oye sale
  // `outputLatency` tarde, así que lo tocado «a tiempo» llega esa demora después (más ~5 ms de entrada).
  // `gotv.ex.latencia` (ms) la ajusta quien mida su equipo.
  async function grabar() {
    if (X.grabando) return pararGrabacion(false);
    if (X.trabajo) return toast(tt("exOcupado"), 3000);
    asegurarCtx();
    // (el comando no devuelve nada: `llamar` no distingue eso de un fallo)
    try { await invoke("experto_grabar", { proyecto: X.root }); } catch (e) { return toast("✗ " + (e && e.message ? e.message : e), 7000); }
    X.grabando = { inicio: X.pos, t: performance.now(), tic: setInterval(pintarPie, 250) };
    if (!X.sonando) { reproducir(); X.grabando.inicio = X.pos; }
    pintarBarra(); pintarPie();
  }
  async function pararGrabacion(descartar) {
    const g = X.grabando; if (!g) return;
    clearInterval(g.tic); X.grabando = null;
    if (X.sonando) parar();
    const latencia = (X.latencia ?? Math.round(((X.ctx && (X.ctx.outputLatency || 0) + (X.ctx.baseLatency || 0)) || 0) * 1000 + 5)) / 1000;
    const v = await llamar("experto_grabar_parar", { inicio: Math.max(0, g.inicio - latencia), descartar: !!descartar });
    pintarBarra(); pintarPie();
    if (v) { await actualizar(v); if (!descartar) { const p = est().pistas[est().pistas.length - 1]; if (p) { X.pistaSel = p.id; pintar(); } } }
  }
  EX.grabar = grabar;

  function alternar() { if (X.grabando) return pararGrabacion(false); X.sonando ? parar() : reproducir(); }
  function irA(t) { const s = X.sonando; if (s) parar(); X.pos = Math.max(0, t); if (s) reproducir(); else pintarCabezal(); EX.arreglo.seguir(true); }
  function alternarBucle() {
    // con una selección, el bucle la toma; sin nada, se apaga o se enciende sobre el último tramo
    if (!X.bucle && X.sel && X.sel.hasta > X.sel.desde) X.bucleR = { desde: X.sel.desde, hasta: X.sel.hasta };
    if (!X.bucle && !X.bucleR) { const p = EX.pulso() * (est().compas || 4); X.bucleR = { desde: Math.floor(X.pos / p) * p, hasta: Math.floor(X.pos / p) * p + 4 * p }; }
    X.bucle = !X.bucle;
    pintarBarra(); EX.arreglo.pintarRegla();
    if (X.sonando) { parar(); reproducir(); }
  }
  Object.assign(EX, { reproducir, parar, alternar, irA, alternarBucle });

  // los medidores: pico de cada pista y bus (con retención) y del maestro por canal
  const tmp = {};
  function pico(an) { const a = tmp[an.fftSize] || (tmp[an.fftSize] = new Float32Array(an.fftSize)); an.getFloatTimeDomainData(a); let m = 0; for (let i = 0; i < a.length; i++) { const v = a[i] < 0 ? -a[i] : a[i]; if (v > m) m = v; } return m; }
  EX.niveles = {}; // id → { v, max, t }
  function nivel(clave, an, apagado) {
    const n = EX.niveles[clave] || (EX.niveles[clave] = { v: 0, max: 0, t: 0 });
    const p = apagado || !an ? 0 : pico(an), ahora = performance.now();
    n.v = p >= n.v ? p : n.v * 0.82; // sube al instante y cae suave
    // la retención del pico: 1,6 s arriba y luego baja
    if (p >= n.max) { n.max = p; n.t = ahora; } else if (ahora - n.t > 1600) n.max = Math.max(p, n.max * 0.9);
    return n;
  }
  function medir(apagado) {
    for (const p of est().pistas) nivel("p" + p.id, X.nodos[p.id] && X.nodos[p.id].vu, apagado);
    for (const g of est().grupos) nivel("g" + g.id, X.grupos[g.id] && X.grupos[g.id].vu, apagado);
    nivel("mL", X.medL, apagado); nivel("mR", X.medR, apagado);
    EX.arreglo.medidores(); EX.dock.medidores();
  }
  EX.medir = medir;
  // de amplitud a porcentaje del medidor (−60 dB … +6 dB)
  EX.aMedidor = (v) => Math.min(100, Math.max(0, ((EX.linDb(v) + 60) / 66) * 100));

  window.Experto = { abrir, proyecto, bloquea, cerrar, _X: X };
})();
