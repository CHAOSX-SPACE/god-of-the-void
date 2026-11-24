// Ola EXPERTO II · X2-5 — el DOCK: mezclador, canal (EQ, dinámica, efectos), análisis e historial.
// También los menús y los diálogos del estudio (exportar, ayuda, pedir un texto).
//
// Todo cambio es una operación del núcleo. La curva del EQ que se pinta la calcula el núcleo
// (`experto_curva_eq`): es la misma fórmula que suena y que se exporta.
(() => {
  const X = EX.X, est = EX.est, op = EX.op;
  const D = (EX.dock = {});
  const css = (n, d) => getComputedStyle(document.body).getPropertyValue(n).trim() || d;

  // ── menús y diálogos ────────────────────────────────────────────────────
  // items: [texto, función, clase?] o "-" (separador); `armar(m)` puede añadir contenido propio
  EX.menu = (ancla, items, armar) => {
    document.querySelectorAll(".ex-menu").forEach((m) => m.remove());
    const m = document.createElement("div");
    m.className = "ex-menu"; m.setAttribute("role", "menu");
    const fuera = (e) => { if (!m.contains(e.target)) m.cerrar(); };
    m.cerrar = () => { m.remove(); document.removeEventListener("mousedown", fuera, true); };
    for (const it of items) {
      if (it === "-") { m.appendChild(document.createElement("hr")); continue; }
      const b = document.createElement("button");
      b.type = "button"; b.setAttribute("role", "menuitem"); b.className = it[2] || "";
      b.innerHTML = (it[2] === "ia" ? EX.icono(EX.I.chispa) : "") + `<span>${esc(it[0])}</span>`;
      b.onclick = () => { m.cerrar(); it[1](); };
      m.appendChild(b);
    }
    if (armar) armar(m);
    document.body.appendChild(m);
    const r = ancla.getBoundingClientRect(), mw = m.offsetWidth, mh = m.offsetHeight;
    m.style.left = Math.max(8, Math.min(window.innerWidth - mw - 8, r.left)) + "px";
    m.style.top = (r.bottom + 4 + mh > window.innerHeight - 8 ? Math.max(8, (r.top ?? r.bottom) - mh - 4) : r.bottom + 4) + "px";
    setTimeout(() => document.addEventListener("mousedown", fuera, true), 0);
    return m;
  };
  function dialogo(titulo, cuerpo, ancho) {
    document.querySelectorAll(".ex-dialogo").forEach((d) => d.remove());
    const d = document.createElement("div");
    d.className = "ex-dialogo";
    d.innerHTML = `<div class="ex-dialogo-caja" role="dialog" aria-modal="true" aria-label="${esc(titulo)}" style="max-width:${ancho || 460}px"><header><h3>${esc(titulo)}</h3><button type="button" class="ex-b" data-cerrar aria-label="${esc(tt("exCerrar"))}">${EX.icono(EX.I.cerrar)}</button></header><div class="ex-dialogo-cuerpo">${cuerpo}</div></div>`;
    d.addEventListener("mousedown", (e) => { if (e.target === d) d.remove(); });
    d.querySelector("[data-cerrar]").onclick = () => d.remove();
    document.body.appendChild(d);
    return d;
  }
  EX.dialogo = dialogo;
  EX.pedirTexto = (titulo, valor) => new Promise((ok) => {
    const d = dialogo(titulo, `<input type="text" class="ex-entrada" id="exPedir" maxlength="120"><div class="ex-botones fin"><button type="button" class="ex-bt" data-no>${esc(tt("exCancelar"))}</button><button type="button" class="ex-bt primario" data-si>${esc(tt("exAceptar"))}</button></div>`, 380);
    const i = d.querySelector("#exPedir");
    let hecho = false;
    const fin = (v) => { if (hecho) return; hecho = true; d.remove(); ok(v); };
    i.value = valor || ""; i.focus(); i.select();
    i.onkeydown = (e) => { e.stopPropagation(); if (e.key === "Enter") fin(i.value); else if (e.key === "Escape") fin(null); };
    d.querySelector("[data-si]").onclick = () => fin(i.value);
    d.querySelector("[data-no]").onclick = () => fin(null);
    d.querySelector("[data-cerrar]").onclick = () => fin(null);
    d.addEventListener("mousedown", (e) => { if (e.target === d) fin(null); });
  });
  EX.dialogoAyuda = () => {
    const filas = tt("exAtajos").split("|").map((f) => { const [k, t] = f.split("="); return `<tr><td><kbd>${esc(k)}</kbd></td><td>${esc(t)}</td></tr>`; }).join("");
    dialogo(tt("exAyuda"), `<p class="ex-nota">${esc(tt("exAyudaIntro"))}</p><table class="ex-atajos">${filas}</table>`, 560);
  };
  EX.dialogoExportar = () => {
    const sel = X.sel && X.sel.hasta > X.sel.desde, buc = !!X.bucleR;
    const d = dialogo(tt("exExportar"), `
      <div class="ex-form">
        <label class="ex-f"><span>${esc(tt("exExpTramo"))}</span><select id="exExpTramo"><option value="todo">${esc(tt("exExpTodo"))}</option>${sel ? `<option value="sel">${esc(tt("exSeleccion"))} (${X.sel.desde.toFixed(2)}–${X.sel.hasta.toFixed(2)} s)</option>` : ""}${buc ? `<option value="bucle">${esc(tt("exBucleCorto"))} (${X.bucleR.desde.toFixed(2)}–${X.bucleR.hasta.toFixed(2)} s)</option>` : ""}</select></label>
        <label class="ex-f"><span>${esc(tt("exExpSonoridad"))}</span><select id="exExpLufs"><option value="">${esc(tt("exExportarTal"))}</option><option value="-14">−14 LUFS · ${esc(tt("exExpStreaming"))}</option><option value="-16">−16 LUFS · ${esc(tt("exExpPodcast"))}</option><option value="-9">−9 LUFS · ${esc(tt("exExpFuerte"))}</option><option value="-23">−23 LUFS · ${esc(tt("exExpBroadcast"))}</option></select></label>
        <label class="ex-f"><span>${esc(tt("exExpBits"))}</span><select id="exExpBits"><option value="24">24 bits</option><option value="16">16 bits · ${esc(tt("exExpTramado"))}</option><option value="32">32 bits · ${esc(tt("exExpFlotante"))}</option></select></label>
        <label class="ex-f"><span>${esc(tt("exExpFormato"))}</span><select id="exExpFormato"><option value="">${esc(tt("exExpSoloWav"))}</option><option value="flac">+ FLAC</option><option value="m4a">+ M4A (AAC 256)</option></select></label>
        <label class="pro-check"><input type="checkbox" id="exExpPistas" checked> ${esc(tt("exExpPistas"))}</label>
        <label class="pro-check"><input type="checkbox" id="exExpDaw" checked> ${esc(tt("exExpDaw"))}</label>
        <p class="ex-nota">${esc(tt("exExportarNota"))}</p>
        <div id="exExpRes"></div>
        <div class="ex-botones fin"><button type="button" class="ex-bt primario" id="exExpIr">${EX.icono(EX.I.exportar)}<span>${esc(tt("exExportar"))}</span></button></div>
      </div>`, 480);
    d.querySelector("#exExpIr").onclick = async () => {
      const b = d.querySelector("#exExpIr"), q = (id) => d.querySelector(id);
      const tramo = q("#exExpTramo").value, r = tramo === "sel" ? X.sel : tramo === "bucle" ? X.bucleR : null;
      const opciones = { bits: Number(q("#exExpBits").value), pistas: q("#exExpPistas").checked, dawproject: q("#exExpDaw").checked, formato: q("#exExpFormato").value };
      if (r) { opciones.desde = r.desde; opciones.hasta = r.hasta; }
      b.disabled = true; q("#exExpRes").innerHTML = `<p class="ex-nota"><span class="ex-giro"></span> ${esc(tt("exExportando"))}</p>`;
      const x = await EX.llamar("experto_exportar", { lufs: q("#exExpLufs").value ? Number(q("#exExpLufs").value) : null, opciones });
      b.disabled = false;
      if (!x) return void (q("#exExpRes").innerHTML = "");
      const e = x.exportado, num = (v, u) => (v === null || v === undefined ? "—" : `${v.toFixed(1)} ${u}`);
      q("#exExpRes").innerHTML = `<div class="ex-resultado"><b>✓ ${esc(tt("exExportadoCorto"))}</b><code>${esc(x.carpeta)}</code>
        <span>${esc(tt("exSonoridad"))}: <b>${num(e.lufs, "LUFS")}</b> · ${esc(tt("exPicoReal"))}: <b>${num(e.pico_real_db, "dBTP")}</b>${e.ganancia_db ? ` · ${e.ganancia_db > 0 ? "+" : ""}${e.ganancia_db.toFixed(1)} dB` : ""}${e.falto_db > 0.05 ? ` · ${esc(tt("exFalto").replace("{n}", e.falto_db.toFixed(1)))}` : ""}</span>
        <span>${(e.pistas || []).length} ${esc(tt("exPistas"))}${e.dawproject ? " · .dawproject" : ""}${e.extra ? " · " + esc(e.extra.split(".").pop().toUpperCase()) : ""}</span>
        <button type="button" class="ex-bt" id="exExpAbrir">${esc(tt("exAbrirCarpeta"))}</button></div>`;
      q("#exExpAbrir").onclick = () => EX.llamar("experto_mostrar", { carpeta: x.carpeta });
    };
  };

  // ── el dock ─────────────────────────────────────────────────────────────
  const PESTANAS = [["mezclador", "exMezclador"], ["canal", "exCanal"], ["analisis", "exAnalisis"], ["historial", "exHistorial"]];
  D.montar = () => {
    $("exDock").innerHTML = `<div class="ex-dock-cab"><div class="ex-pestanas" role="tablist">${PESTANAS.map(([k, t]) => `<button type="button" role="tab" data-p="${k}">${esc(tt(t))}</button>`).join("")}</div>
      <span class="ex-dock-obj" id="exDockObj"></span><button type="button" class="ex-b" id="exDockPlegar" title="${esc(tt("exPlegarDock"))}">${EX.icono(EX.I.plegar)}</button></div><div class="ex-dock-cuerpo" id="exDockCuerpo"></div>`;
    $("exDock").querySelectorAll("[data-p]").forEach((b) => (b.onclick = () => D.abrir(b.dataset.p)));
    $("exDockPlegar").onclick = () => { X.dockPlegado = !X.dockPlegado; EX.recordar("dockPlegado", X.dockPlegado); EX.pintar(); };
  };
  D.abrir = (p) => { X.dock = p; X.dockPlegado = false; EX.recordar("dock", p); EX.recordar("dockPlegado", false); EX.medidas(); EX.arreglo.redibujar(); D.pintar(); };
  D.pintar = () => {
    const c = $("exDockCuerpo"); if (!c) return;
    if (EX.gesto) return void (EX.pintarPendiente = true); // a mitad de un gesto no se repinta (ver `arrastrar`)
    $("exDock").querySelectorAll("[data-p]").forEach((b) => { b.classList.toggle("on", b.dataset.p === X.dock); b.setAttribute("aria-selected", String(b.dataset.p === X.dock)); });
    $("exDockPlegar").classList.toggle("girado", !!X.dockPlegado);
    $("exDockObj").textContent = X.dock === "canal" ? nombreObjetivo() : "";
    c.className = "ex-dock-cuerpo ex-d-" + X.dock;
    if (X.dockPlegado) return void (c.innerHTML = "");
    ({ mezclador: pintarMezclador, canal: pintarCanal, analisis: pintarAnalisis, historial: pintarHistorial })[X.dock](c);
  };
  D.redibujar = () => { if (X.dock === "canal") dibujarEq(); else if (X.dock === "analisis") dibujarEspectro(); };

  // ── el objetivo del canal (una pista, un bus o el maestro) ─────────────
  function objetivo() {
    const o = X.objetivo;
    if (o.tipo === "maestro") return { tipo: "maestro", id: 0, nombre: tt("exMaestro"), insertos: est().maestro.insertos || {} };
    if (o.tipo === "grupo" && EX.grupo(o.id)) return { tipo: "grupo", id: o.id, nombre: "BUS · " + EX.grupo(o.id).nombre, insertos: EX.grupo(o.id).insertos || {} };
    const p = EX.pista(o.tipo === "pista" ? o.id : X.pistaSel) || EX.pista(X.pistaSel) || est().pistas[0];
    return p ? { tipo: "pista", id: p.id, nombre: `${p.id} · ${p.nombre}`, insertos: p.insertos || {} } : null;
  }
  const nombreObjetivo = () => { const o = objetivo(); return o ? o.nombre : ""; };
  const fijarInsertos = (o, ins) => {
    for (const k of Object.keys(ins)) if (ins[k] === null || ins[k] === undefined || (Array.isArray(ins[k]) && !ins[k].length)) delete ins[k];
    return o.tipo === "pista" ? op({ op: "insertos", pista: o.id, insertos: ins }) : o.tipo === "grupo" ? op({ op: "insertos_grupo", grupo: o.id, insertos: ins }) : op({ op: "maestro", insertos: ins });
  };

  // ── el mezclador ────────────────────────────────────────────────────────
  // el recorrido del fader: 0 dB a tres cuartos; por debajo, ley logarítmica hasta −60
  const aDb = (u) => (u >= 0.75 ? ((u - 0.75) / 0.25) * 12 : u <= 0.0237 ? -60 : Math.max(-60, 40 * Math.log10(u / 0.75)));
  const aPos = (db) => (db >= 0 ? 0.75 + (Math.min(12, db) / 12) * 0.25 : db <= -60 ? 0 : 0.75 * Math.pow(10, db / 40));
  const MODULOS = [["puerta", "exPuerta"], ["eq", "EQ"], ["compresor", "exCompresor"], ["deesser", "exDeEsser"], ["saturacion", "exSaturacion"], ["chorus", "exChorus"], ["delay", "exDelay"], ["reverb", "exReverb"], ["ancho", "exAncho"], ["limitador", "exLimitador"]];
  const activos = (ins) => MODULOS.filter(([k]) => (k === "eq" ? (ins.eq || []).length : ins[k] !== undefined && ins[k] !== null));
  function tira(k, nombre, color, o) {
    const fx = activos(o.insertos || {});
    const sel = (k[0] === "p" && X.objetivo.tipo === "pista" && X.objetivo.id === o.id) || (k[0] === "g" && X.objetivo.tipo === "grupo" && X.objetivo.id === o.id) || (k === "m" && X.objetivo.tipo === "maestro");
    const buses = est().grupos;
    return `<div class="ex-tira${sel ? " sel" : ""}${k === "m" ? " maestro" : k[0] === "g" ? " bus" : ""}${o.muda ? " muda" : ""}" data-k="${k}" style="--pista:${color}">
      <div class="ex-tira-fx" title="${esc(tt("exEfectosAyuda"))}">${fx.length ? fx.slice(0, 3).map(([, t]) => `<span>${esc((t === "EQ" ? t : tt(t)).slice(0, 5))}</span>`).join("") + (fx.length > 3 ? `<span>+${fx.length - 3}</span>` : "") : `<span class="vacio">＋ FX</span>`}</div>
      ${k[0] === "p" ? `<select class="ex-tira-bus" title="${esc(tt("exBus"))}" aria-label="${esc(tt("exBus"))} ${esc(nombre)}"><option value="">${esc(tt("exSinBus"))}</option>${buses.map((g) => `<option value="${g.id}"${o.grupo === g.id ? " selected" : ""}>${esc(g.nombre)}</option>`).join("")}<option value="nuevo">${esc(tt("exBusNuevo"))}</option></select>` : `<div class="ex-tira-bus-e">${k === "m" ? esc(tt("exSalida")) : "BUS"}</div>`}
      ${k === "m" ? `<div class="ex-tira-pan-e"></div>` : `<input type="range" class="ex-tira-pan" min="-1" max="1" step="0.05" value="${o.pan}" title="${esc(tt("exPan"))}">`}
      <div class="ex-tira-medio">
        <div class="ex-fader" role="slider" tabindex="0" aria-label="${esc(tt("exGanancia"))} ${esc(nombre)}" aria-valuemin="-60" aria-valuemax="12" aria-valuenow="${o.ganancia_db}" data-db="${o.ganancia_db}"><div class="ex-fader-via"><i style="bottom:${aPos(0) * 100}%"></i></div><div class="ex-fader-pomo" style="bottom:${aPos(o.ganancia_db) * 100}%"></div></div>
        ${k === "m" ? `<div class="ex-medidor-v" data-vu="mL"><div></div><i></i></div><div class="ex-medidor-v" data-vu="mR"><div></div><i></i></div>` : `<div class="ex-medidor-v" data-vu="${k}"><div></div><i></i></div>`}
      </div>
      <div class="ex-tira-db mono">${EX.fmtDb(o.ganancia_db)}</div>
      <div class="ex-tira-ms">${k === "m" ? `<button type="button" class="ex-ms ia" data-a="master" title="${esc(tt("exMasterizarAyuda"))}">✦</button>` : `<button type="button" class="ex-ms${o.silencio ? " on" : ""}" data-a="m" title="${esc(tt("exSilencio"))}">M</button><button type="button" class="ex-ms solo${o.solo ? " on" : ""}" data-a="s" title="${esc(tt("exSolo"))}">S</button>`}</div>
      <div class="ex-tira-nombre" title="${esc(nombre)}">${esc(nombre)}</div></div>`;
  }
  function pintarMezclador(c) {
    const e = est();
    if (!e.pistas.length) return void (c.innerHTML = `<p class="ex-nota centro">${esc(tt("exMezcladorVacio"))}</p>`);
    const orden = [...e.grupos.flatMap((g) => e.pistas.filter((p) => p.grupo === g.id)), ...e.pistas.filter((p) => !p.grupo || !e.grupos.some((g) => g.id === p.grupo))];
    c.innerHTML = `<div class="ex-mezclador">${orden.map((p) => tira("p" + p.id, `${p.id} · ${p.nombre}`, EX.colorDe(p), { ...p, muda: !EX.suena(p) })).join("")}
      ${e.grupos.length ? `<div class="ex-tira-sep"></div>` + e.grupos.map((g) => tira("g" + g.id, g.nombre, css("--info", "#7ea8f2"), g)).join("") : ""}
      <div class="ex-tira-sep"></div>${tira("m", tt("exMaestro"), css("--accent", "#8a7df3"), { id: 0, ganancia_db: e.maestro.ganancia_db || 0, insertos: e.maestro.insertos || {} })}</div>`;
    c.querySelectorAll(".ex-tira").forEach((t) => {
      const k = t.dataset.k, id = Number(k.slice(1)), tipo = k === "m" ? "maestro" : k[0] === "g" ? "grupo" : "pista";
      const elegir = () => { X.objetivo = { tipo, id }; if (tipo === "pista") X.pistaSel = id; };
      const mez = (o) => (tipo === "pista" ? op(Object.assign({ op: "mezcla", pista: id }, o)) : tipo === "grupo" ? op(Object.assign({ op: "mezcla_grupo", grupo: id }, o)) : op({ op: "maestro", ganancia_db: o.ganancia_db }));
      const vivo = (db, pan) => (tipo === "pista" ? EX.vivoPista(id, db, pan) : tipo === "grupo" ? EX.vivoGrupo(id, db, pan) : db !== null && EX.vivoMaestro(db));
      // con `click` y no `mousedown`: repintar al bajar el botón cambiaría el elemento y el clic se perdería
      t.addEventListener("click", (ev) => { if (!ev.target.closest("button, select, input, .ex-fader, .ex-tira-fx, .ex-medidor-v")) { elegir(); EX.pintar(); } });
      t.querySelector(".ex-tira-fx").onclick = () => { elegir(); D.abrir("canal"); EX.pintar(); };
      const f = t.querySelector(".ex-fader"), pomo = f.querySelector(".ex-fader-pomo"), lectura = t.querySelector(".ex-tira-db");
      const poner = (db) => { db = Math.round(Math.min(12, Math.max(-60, db)) * 2) / 2; f.dataset.db = db; pomo.style.bottom = aPos(db) * 100 + "%"; lectura.textContent = EX.fmtDb(db); f.setAttribute("aria-valuenow", db); vivo(db, null); return db; };
      f.addEventListener("mousedown", (ev) => {
        ev.preventDefault();
        // doble clic: a 0 dB (se lee aquí: tras el primer clic el mezclador se repinta y `dblclick` no llega)
        if (ev.detail >= 2) return void mez({ ganancia_db: 0 });
        const r = f.getBoundingClientRect(), u0 = aPos(Number(f.dataset.db)), y0 = ev.clientY;
        // un clic fuera del pomo lo lleva ahí; arrastrar lo mueve (con ⇧, cuatro veces más fino)
        const enPomo = ev.target === pomo;
        if (!enPomo) poner(aDb(1 - (ev.clientY - r.top) / r.height));
        const base = enPomo ? u0 : aPos(Number(f.dataset.db));
        EX.arrastrar(ev, (e) => poner(aDb(Math.min(1, Math.max(0, base + ((y0 - e.clientY) / r.height) * (e.shiftKey ? 0.25 : 1))))), () => mez({ ganancia_db: Number(f.dataset.db) }));
      });
      f.addEventListener("keydown", (e) => { const d = { ArrowUp: 0.5, ArrowDown: -0.5, PageUp: 3, PageDown: -3 }[e.key]; if (!d) return; e.preventDefault(); e.stopPropagation(); mez({ ganancia_db: poner(Number(f.dataset.db) + d) }); });
      const pan = t.querySelector(".ex-tira-pan");
      if (pan) { pan.oninput = () => vivo(null, Number(pan.value)); pan.onchange = () => mez({ pan: Number(pan.value) }); pan.onmousedown = (ev) => { if (ev.detail >= 2) { ev.preventDefault(); mez({ pan: 0 }); } }; }
      const bus = t.querySelector(".ex-tira-bus");
      if (bus) bus.onchange = () => (bus.value === "nuevo" ? EX.arreglo.busNuevo([id]) : op({ op: "agrupar", pistas: [id], grupo: bus.value ? Number(bus.value) : null }));
      t.querySelectorAll("button[data-a]").forEach((b) => (b.onclick = () => {
        const o = tipo === "pista" ? EX.pista(id) : EX.grupo(id);
        if (b.dataset.a === "m") mez({ silencio: !o.silencio });
        else if (b.dataset.a === "s") mez({ solo: !o.solo });
        else EX.lado.abrirIA("masterizar");
      }));
      t.querySelectorAll(".ex-medidor-v").forEach((m) => (m.onclick = () => { const n = EX.niveles[m.dataset.vu]; if (n) n.max = 0; m.classList.remove("tope"); }));
    });
  }
  D.medidores = () => {
    if (X.dockPlegado) return;
    for (const el of document.querySelectorAll("#exDockCuerpo .ex-medidor-v")) {
      const n = EX.niveles[el.dataset.vu]; if (!n) continue;
      el.firstElementChild.style.height = EX.aMedidor(n.v) + "%";
      el.lastElementChild.style.bottom = EX.aMedidor(n.max) + "%";
      if (n.max >= 1) el.classList.add("tope");
    }
    if (X.dock === "canal") dibujarEq(true);
    else if (X.dock === "analisis") dibujarEspectro();
  };

  // ── el canal ────────────────────────────────────────────────────────────
  const DEF = {
    puerta: { umbral_db: -50, ataque_ms: 2, liberacion_ms: 80 },
    compresor: { umbral_db: -18, razon: 3, ataque_ms: 10, liberacion_ms: 120, ganancia_db: 0 },
    deesser: { frecuencia: 6000, umbral_db: -30, reduccion_db: 6 },
    saturacion: { empuje_db: 6, mezcla: 0.5 },
    chorus: { velocidad_hz: 0.8, profundidad_ms: 3, mezcla: 0.3 },
    delay: { ms: 375, realimentacion: 0.3, mezcla: 0.25 },
    reverb: { tamano: 0.6, amortiguacion: 0.5, mezcla: 0.2 },
    limitador: { techo_db: -1, empuje_db: 0, liberacion_ms: 120 },
  };
  // [campo, etiqueta, mín, máx, paso, unidad]
  const CAMPOS = {
    puerta: [["umbral_db", "exUmbral", -90, 0, 1, "dB"], ["ataque_ms", "exAtaque", 0.1, 100, 0.1, "ms"], ["liberacion_ms", "exLiberacion", 5, 2000, 5, "ms"]],
    compresor: [["umbral_db", "exUmbral", -60, 0, 0.5, "dB"], ["razon", "exRazon", 1, 20, 0.1, ":1"], ["ataque_ms", "exAtaque", 0.1, 200, 0.1, "ms"], ["liberacion_ms", "exLiberacion", 5, 2000, 5, "ms"], ["ganancia_db", "exCompensa", -12, 24, 0.5, "dB"]],
    deesser: [["frecuencia", "exFrecuencia", 1000, 16000, 100, "Hz"], ["umbral_db", "exUmbral", -80, 0, 1, "dB"], ["reduccion_db", "exReduccion", 0, 24, 0.5, "dB"]],
    saturacion: [["empuje_db", "exEmpuje", 0, 36, 0.5, "dB"], ["mezcla", "exMezcla", 0, 1, 0.01, ""]],
    chorus: [["velocidad_hz", "exVelocidad", 0.05, 8, 0.05, "Hz"], ["profundidad_ms", "exProfundidad", 0.5, 20, 0.5, "ms"], ["mezcla", "exMezcla", 0, 1, 0.01, ""]],
    delay: [["ms", "exTiempo", 1, 2000, 1, "ms"], ["realimentacion", "exRealim", 0, 0.95, 0.01, ""], ["mezcla", "exMezcla", 0, 1, 0.01, ""]],
    reverb: [["tamano", "exTamano", 0, 1, 0.01, ""], ["amortiguacion", "exAmort", 0, 1, 0.01, ""], ["mezcla", "exMezcla", 0, 1, 0.01, ""]],
    limitador: [["techo_db", "exTecho", -24, 0, 0.1, "dB"], ["empuje_db", "exEmpuje", 0, 24, 0.5, "dB"], ["liberacion_ms", "exLiberacion", 1, 1000, 1, "ms"]],
  };
  const banda = (filtro, frecuencia, db, q) => ({ filtro, frecuencia, db: db || 0, q: q || 0.707 });
  const PRESETS = [
    ["exPreVoz", { eq: [banda("paso_alto", 90), banda("campana", 250, -2.5, 1.2), banda("campana", 3500, 3, 1), banda("agudos", 10000, 2.5)], compresor: { umbral_db: -20, razon: 3, ataque_ms: 8, liberacion_ms: 120, ganancia_db: 3 }, deesser: { frecuencia: 6000, umbral_db: -32, reduccion_db: 6 }, reverb: { tamano: 0.45, amortiguacion: 0.6, mezcla: 0.12 } }],
    ["exPreBateria", { eq: [banda("campana", 70, 3, 1), banda("campana", 400, -3, 1.2), banda("agudos", 8000, 3)], compresor: { umbral_db: -16, razon: 4, ataque_ms: 20, liberacion_ms: 90, ganancia_db: 3 }, saturacion: { empuje_db: 6, mezcla: 0.4 } }],
    ["exPreBajo", { eq: [banda("paso_alto", 35), banda("campana", 90, 2.5, 1), banda("campana", 700, 1.5, 1.2), banda("paso_bajo", 6000)], compresor: { umbral_db: -18, razon: 4, ataque_ms: 15, liberacion_ms: 150, ganancia_db: 2 } }],
    ["exPreGuitarra", { eq: [banda("paso_alto", 100), banda("campana", 300, -2, 1.2), banda("campana", 2500, 2, 1), banda("agudos", 9000, 1.5)], chorus: { velocidad_hz: 0.6, profundidad_ms: 3, mezcla: 0.25 }, ancho: 1.3 }],
    ["exPreAmbiente", { eq: [banda("paso_alto", 120), banda("agudos", 8000, 2)], reverb: { tamano: 0.7, amortiguacion: 0.5, mezcla: 0.25 }, ancho: 1.4 }],
    ["exPreMaestroSuave", { eq: [banda("paso_alto", 25), banda("graves", 60, 1), banda("agudos", 10000, 1.5)], compresor: { umbral_db: -12, razon: 1.8, ataque_ms: 30, liberacion_ms: 250, ganancia_db: 1 }, limitador: { techo_db: -1, empuje_db: 2, liberacion_ms: 120 } }],
    ["exPreMaestroFuerte", { eq: [banda("paso_alto", 25), banda("graves", 60, 1), banda("agudos", 10000, 1.5)], compresor: { umbral_db: -14, razon: 2.5, ataque_ms: 20, liberacion_ms: 200, ganancia_db: 2 }, saturacion: { empuje_db: 3, mezcla: 0.3 }, limitador: { techo_db: -1, empuje_db: 6, liberacion_ms: 100 } }],
  ];
  const memoria = {}; // los ajustes de un efecto apagado, para volver a encenderlo como estaba
  let eqSel = 0;
  // en el MISMO orden que los nombres de `exFiltros` (medido en la app real: la campana salía rotulada «graves»)
  const TIPOS = ["graves", "agudos", "campana", "paso_alto", "paso_bajo", "muesca", "paso_banda"];
  const conGanancia = (f) => f === "campana" || f === "graves" || f === "agudos";
  const fmtHz = (f) => (f >= 1000 ? (f / 1000).toFixed(f >= 10000 ? 1 : 2).replace(/\.?0+$/, "") + " k" : Math.round(f) + "");

  function pintarCanal(c) {
    const o = objetivo();
    if (!o) return void (c.innerHTML = `<p class="ex-nota centro">${esc(tt("exElige"))}</p>`);
    const ins = o.insertos, nombres = tt("exFiltros").split("|"), eq = ins.eq || [];
    if (eqSel >= eq.length) eqSel = Math.max(0, eq.length - 1);
    const campo = (mod, [k, t, min, max, paso, u], v) => `<label class="ex-par"><span>${esc(tt(t))}</span><input type="range" data-m="${mod}" data-c="${k}" min="${min}" max="${max}" step="${paso}" value="${v}"><input type="number" class="mono" data-m="${mod}" data-c="${k}" min="${min}" max="${max}" step="${paso}" value="${v}"><em>${u}</em></label>`;
    const modulo = (mod, titulo, cuerpo, on, extra) => `<section class="ex-mod${on ? " on" : ""}" data-mod="${mod}"><header><label class="ex-enc"><input type="checkbox" data-on="${mod}"${on ? " checked" : ""}><b>${esc(titulo)}</b></label>${extra || ""}</header><div class="ex-mod-c">${cuerpo}</div></section>`;
    const simple = (mod, t, extra) => { const on = ins[mod] !== undefined && ins[mod] !== null, v = on ? ins[mod] : (memoria[o.tipo + o.id] || {})[mod] || DEF[mod]; return modulo(mod, tt(t), CAMPOS[mod].map((f) => campo(mod, f, v[f[0]])).join("") + (extra || ""), on); };
    const b = eq[eqSel];
    const eqCuerpo = `<div class="ex-eq-caja"><canvas id="exEq"></canvas></div>
      <div class="ex-eq-bandas">${eq.map((x, i) => `<button type="button" class="ex-eq-b${i === eqSel ? " on" : ""}${x.apagada ? " apagada" : ""}" data-b="${i}" title="${esc(nombres[TIPOS.indexOf(x.filtro)] || x.filtro)}">${i + 1}</button>`).join("")}${eq.length < 8 ? `<button type="button" class="ex-eq-b mas" data-b="mas" title="${esc(tt("exEqMas"))}">＋</button>` : ""}</div>
      ${b ? `<div class="ex-eq-campos"><label class="ex-par c"><span>${esc(tt("exFiltro"))}</span><select data-eq="filtro">${TIPOS.map((t, i) => `<option value="${t}"${t === b.filtro ? " selected" : ""}>${esc(nombres[i] || t)}</option>`).join("")}</select></label>
        <label class="ex-par c"><span>Hz</span><input type="number" class="mono" data-eq="frecuencia" min="20" max="20000" step="1" value="${Math.round(b.frecuencia)}"></label>
        <label class="ex-par c${conGanancia(b.filtro) ? "" : " apagado"}"><span>dB</span><input type="number" class="mono" data-eq="db" min="-24" max="24" step="0.5" value="${b.db}"></label>
        <label class="ex-par c"><span>Q</span><input type="number" class="mono" data-eq="q" min="0.1" max="18" step="0.1" value="${Number(b.q.toFixed(2))}"></label>
        <button type="button" class="ex-t" data-eq-a="apagar">${esc(tt(b.apagada ? "exEncender" : "exApagar"))}</button><button type="button" class="ex-t peligro" data-eq-a="quitar">${esc(tt("exQuitarBanda"))}</button></div>` : `<p class="ex-nota">${esc(tt("exEqAyuda"))}</p>`}`;
    const ancho = ins.ancho !== undefined && ins.ancho !== null;
    const sincro = `<div class="ex-sincro" title="${esc(tt("exSincroAyuda"))}">${[["1/4", 1], ["1/8", 0.5], ["1/8·", 0.75], ["1/16", 0.25], ["1/4T", 2 / 3]].map(([t, f]) => `<button type="button" class="ex-t" data-sincro="${f}">${t}</button>`).join("")}</div>`;
    c.innerHTML = `<div class="ex-canal">
      <div class="ex-canal-lado"><b class="ex-canal-nombre">${esc(o.nombre)}</b>
        <div class="ex-botones col"><button type="button" class="ex-t${o.tipo === "maestro" ? " on" : ""}" id="exObjMaestro">${esc(tt("exMaestro"))}</button>
        <button type="button" class="ex-t" id="exPresets">${esc(tt("exPresets"))} ▾</button><button type="button" class="ex-t peligro" id="exSinFx"${activos(ins).length ? "" : " disabled"}>${esc(tt("exQuitarEfectos"))}</button></div>
        <p class="ex-nota">${esc(tt("exCadena"))}</p></div>
      ${simple("puerta", "exPuerta")}
      ${modulo("eq", tt("exEqCanal"), eqCuerpo, eq.length > 0, `<span class="ex-nota">${eq.length}/8</span>`)}
      ${simple("compresor", "exCompresor", `<canvas id="exCurvaComp" class="ex-curva"></canvas>`)}
      ${simple("deesser", "exDeEsser")}${simple("saturacion", "exSaturacion")}${simple("chorus", "exChorus")}${simple("delay", "exDelay", sincro)}${simple("reverb", "exReverb")}
      ${modulo("ancho", tt("exAncho"), `<label class="ex-par"><span>${esc(tt("exAnchoEstereo"))}</span><input type="range" data-ancho min="0" max="2" step="0.05" value="${ancho ? ins.ancho : 1}"><input type="number" class="mono" data-ancho min="0" max="2" step="0.05" value="${ancho ? ins.ancho : 1}"><em></em></label><p class="ex-nota">${esc(tt("exAnchoAyuda"))}</p>`, ancho)}
      ${simple("limitador", "exLimitador")}</div>`;
    const copia = () => JSON.parse(JSON.stringify(ins)), clave = o.tipo + o.id;
    const recordar = (mod, v) => ((memoria[clave] ||= {})[mod] = v);
    // encender y apagar
    c.querySelectorAll("input[data-on]").forEach((x) => (x.onchange = () => {
      const mod = x.dataset.on, n = copia();
      if (mod === "eq") { if (x.checked) n.eq = (memoria[clave] || {}).eq || [banda("campana", 1000, 0, 1)]; else { recordar("eq", n.eq); delete n.eq; } }
      else if (mod === "ancho") { if (x.checked) n.ancho = (memoria[clave] || {}).ancho ?? 1.2; else { recordar("ancho", n.ancho); delete n.ancho; } }
      else if (x.checked) n[mod] = (memoria[clave] || {})[mod] || { ...DEF[mod] };
      else { recordar(mod, n[mod]); delete n[mod]; }
      fijarInsertos(o, n);
    }));
    // los parámetros: el deslizador y el número van juntos; se manda al soltar
    c.querySelectorAll("input[data-m]").forEach((x) => {
      const par = () => c.querySelectorAll(`input[data-m="${x.dataset.m}"][data-c="${x.dataset.c}"]`);
      x.oninput = () => { par().forEach((y) => y !== x && (y.value = x.value)); if (x.dataset.m === "compresor") dibujarCompresor(leerModulo("compresor")); };
      x.onchange = () => { const n = copia(), mod = x.dataset.m; n[mod] = leerModulo(mod); fijarInsertos(o, n); };
    });
    const leerModulo = (mod) => { const v = {}; c.querySelectorAll(`input[type=number][data-m="${mod}"]`).forEach((y) => (v[y.dataset.c] = Number(y.value))); return v; };
    c.querySelectorAll("input[data-ancho]").forEach((x) => { x.oninput = () => c.querySelectorAll("input[data-ancho]").forEach((y) => y !== x && (y.value = x.value)); x.onchange = () => { const n = copia(); n.ancho = Number(x.value); fijarInsertos(o, n); }; });
    c.querySelectorAll("button[data-sincro]").forEach((x) => (x.onclick = () => { const n = copia(); n.delay = { ...(n.delay || (memoria[clave] || {}).delay || DEF.delay), ms: Math.round((60000 / (est().bpm || 120)) * Number(x.dataset.sincro)) }; fijarInsertos(o, n); }));
    // el EQ
    const eqFijar = (lista) => { const n = copia(); n.eq = lista; return fijarInsertos(o, n); };
    c.querySelectorAll("button[data-b]").forEach((x) => (x.onclick = () => {
      if (x.dataset.b === "mas") { eqSel = eq.length; return eqFijar(eq.concat(banda("campana", [120, 400, 1000, 2500, 6000, 10000, 200, 4000][eq.length % 8], 0, 1))); }
      eqSel = Number(x.dataset.b); D.pintar();
    }));
    c.querySelectorAll("[data-eq]").forEach((x) => (x.onchange = () => { const l = eq.map((y) => ({ ...y })); l[eqSel][x.dataset.eq] = x.dataset.eq === "filtro" ? x.value : Number(x.value); eqFijar(l); }));
    c.querySelectorAll("[data-eq-a]").forEach((x) => (x.onclick = () => {
      const l = eq.map((y) => ({ ...y }));
      if (x.dataset.eqA === "quitar") l.splice(eqSel, 1); else l[eqSel].apagada = !l[eqSel].apagada;
      eqFijar(l);
    }));
    $("exObjMaestro").onclick = () => { X.objetivo = { tipo: "maestro", id: 0 }; EX.pintar(); };
    $("exSinFx").onclick = () => fijarInsertos(o, {});
    $("exPresets").onclick = (ev) => EX.menu(ev.currentTarget, PRESETS.map(([t, v]) => [tt(t), () => fijarInsertos(o, JSON.parse(JSON.stringify(v)))]));
    eqEventos(o, eq, eqFijar);
    curvaEq = null; pedirCurva(eq);
    dibujarCompresor(ins.compresor || (memoria[clave] || {}).compresor || DEF.compresor);
  }
  // la curva del compresor: lo que entra (horizontal) contra lo que sale (vertical), de −60 a 0 dB
  function dibujarCompresor(v) {
    const cv = $("exCurvaComp"); if (!cv) return;
    const w = 150, h = 96, dpr = window.devicePixelRatio || 1;
    cv.width = w * dpr; cv.height = h * dpr; cv.style.width = w + "px"; cv.style.height = h + "px";
    const g = cv.getContext("2d");
    g.setTransform(dpr, 0, 0, dpr, 0, 0); g.clearRect(0, 0, w, h);
    const x = (db) => ((db + 60) / 60) * w, y = (db) => h - ((Math.min(6, db) + 60) / 66) * h;
    g.strokeStyle = css("--line-2", "#444"); g.lineWidth = 1;
    for (const d of [-48, -36, -24, -12]) { g.beginPath(); g.moveTo(x(d), 0); g.lineTo(x(d), h); g.moveTo(0, y(d)); g.lineTo(w, y(d)); g.stroke(); }
    g.setLineDash([3, 3]); g.beginPath(); g.moveTo(x(-60), y(-60)); g.lineTo(x(0), y(0)); g.stroke(); g.setLineDash([]);
    g.strokeStyle = css("--accent", "#8a7df3"); g.lineWidth = 2; g.beginPath();
    for (let d = -60; d <= 0; d += 1) { const s = (d > v.umbral_db ? v.umbral_db + (d - v.umbral_db) / Math.max(1, v.razon) : d) + (v.ganancia_db || 0); d === -60 ? g.moveTo(x(d), y(s)) : g.lineTo(x(d), y(s)); }
    g.stroke();
    g.fillStyle = css("--warn", "#e3b26b"); g.fillRect(x(v.umbral_db) - 0.5, 0, 1, h);
  }

  // el EQ gráfico: 20 Hz … 20 kHz en logarítmico, ±24 dB
  const F0 = 20, F1 = 20000, DB = 24, NF = 160;
  const frecuencias = Array.from({ length: NF }, (_, i) => F0 * Math.pow(F1 / F0, i / (NF - 1)));
  let curvaEq = null, pidiendoCurva = 0, bandasVivas = null;
  const xDe = (f, w) => (Math.log(f / F0) / Math.log(F1 / F0)) * w, fDe = (x, w) => F0 * Math.pow(F1 / F0, Math.min(1, Math.max(0, x / w)));
  const yDe = (db, h) => h / 2 - (db / DB) * (h / 2), dbDe = (y, h) => Math.max(-DB, Math.min(DB, ((h / 2 - y) / (h / 2)) * DB));
  async function pedirCurva(eq) {
    const n = ++pidiendoCurva;
    try {
      const c = await invoke("experto_curva_eq", { bandas: eq, frecuencias, frecuencia: (X.v && X.v.frecuencia) || 48000 });
      if (n === pidiendoCurva) { curvaEq = c; dibujarEq(); }
    } catch (_) {}
  }
  function eqEventos(o, eq, fijar) {
    const cv = $("exEq"); if (!cv) return;
    const medidas = () => { const r = cv.getBoundingClientRect(); return [r, r.width, r.height]; };
    const cerca = (ev) => { const [r, w, h] = medidas(); let m = -1, dm = 14; eq.forEach((b, i) => { const d = Math.hypot(xDe(b.frecuencia, w) - (ev.clientX - r.left), yDe(conGanancia(b.filtro) ? b.db : 0, h) - (ev.clientY - r.top)); if (d < dm) { dm = d; m = i; } }); return m; };
    cv.onmousedown = (ev) => {
      if (ev.button !== 0) return;
      const i = cerca(ev); if (i < 0) return;
      ev.preventDefault();
      // doble clic sobre una banda: se quita (se lee aquí: el primer clic repinta el canal)
      if (ev.detail >= 2) { const l = eq.map((y) => ({ ...y })); l.splice(i, 1); return void fijar(l); }
      eqSel = i;
      const l = eq.map((y) => ({ ...y })), [r, w, h] = medidas();
      bandasVivas = l;
      // la banda se OYE mientras se arrastra (escucha previa; al soltar renderiza el núcleo)
      const pistaViva = o.tipo === "pista" ? o.id : 0;
      if (pistaViva) EX.eqVivoInicio(pistaViva, eq[i]);
      EX.arrastrar(ev, (e) => {
        l[i].frecuencia = Math.round(fDe(e.clientX - r.left, w));
        if (conGanancia(l[i].filtro)) l[i].db = Math.round(dbDe(e.clientY - r.top, h) * 2) / 2;
        pedirCurva(l);
        if (pistaViva) EX.eqVivoMover(pistaViva, l[i]);
      }, (e, movido) => {
        bandasVivas = null;
        const fin = () => pistaViva && EX.eqVivoFin(pistaViva);
        if (movido) Promise.resolve(fijar(l)).then(fin, fin);
        else { fin(); D.pintar(); }
      });
    };
    cv.ondblclick = (ev) => {
      const i = cerca(ev), [r, w, h] = medidas();
      if (i >= 0 || eq.length >= 8) return;
      eqSel = eq.length;
      fijar(eq.concat(banda("campana", Math.round(fDe(ev.clientX - r.left, w)), Math.round(dbDe(ev.clientY - r.top, h) * 2) / 2, 1)));
    };
    // la rueda sobre una banda cambia su Q
    cv.onwheel = (ev) => { const i = cerca(ev); if (i < 0) return; ev.preventDefault(); const l = eq.map((y) => ({ ...y })); l[i].q = Number(Math.min(18, Math.max(0.1, l[i].q * Math.exp(-ev.deltaY * 0.004))).toFixed(2)); eqSel = i; clearTimeout(cv._t); pedirCurva(l); bandasVivas = l; cv._t = setTimeout(() => { bandasVivas = null; fijar(l); }, 350); };
  }
  let ultimoEq = 0;
  function dibujarEq(vivo) {
    const cv = $("exEq"); if (!cv) return;
    // con la música sonando se repinta ~15 veces por segundo (el espectro); parado, sólo cuando cambia
    const ahora = performance.now();
    if (vivo && (!X.sonando || ahora - ultimoEq < 66)) return;
    ultimoEq = ahora;
    const caja = cv.parentElement, w = caja.clientWidth, h = caja.clientHeight, dpr = window.devicePixelRatio || 1;
    if (!w || !h) return;
    if (cv.width !== Math.round(w * dpr) || cv.height !== Math.round(h * dpr)) { cv.width = Math.round(w * dpr); cv.height = Math.round(h * dpr); }
    const g = cv.getContext("2d");
    g.setTransform(dpr, 0, 0, dpr, 0, 0); g.clearRect(0, 0, w, h);
    g.font = "9.5px ui-monospace, monospace"; g.lineWidth = 1;
    // la rejilla
    for (const f of [20, 30, 40, 50, 60, 70, 80, 90, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1000, 2000, 3000, 4000, 5000, 6000, 7000, 8000, 9000, 10000, 20000]) {
      const mayor = f === 100 || f === 1000 || f === 10000;
      g.fillStyle = mayor ? css("--line-2", "#444") : css("--line", "#333"); g.fillRect(Math.round(xDe(f, w)), 0, 1, h);
      if (mayor || f === 20 || f === 20000) { g.fillStyle = css("--faint", "#777"); g.fillText(fmtHz(f).replace(" ", ""), Math.min(w - 22, xDe(f, w) + 3), h - 3); }
    }
    for (const d of [-18, -12, -6, 0, 6, 12, 18]) { g.fillStyle = d === 0 ? css("--muted", "#888") : css("--line", "#333"); g.fillRect(0, Math.round(yDe(d, h)), w, 1); if (d) { g.fillStyle = css("--faint", "#777"); g.fillText((d > 0 ? "+" : "") + d, 3, yDe(d, h) - 2); } }
    // el espectro de lo que suena (el del canal elegido, o el de la salida)
    const o = objetivo(), an = o && o.tipo === "pista" && X.nodos[o.id] ? X.nodos[o.id].vu : o && o.tipo === "grupo" && X.grupos[o.id] ? X.grupos[o.id].vu : X.espectro;
    if (an && X.sonando) {
      if (an.fftSize < 2048) an.fftSize = 2048;
      const datos = new Float32Array(an.frequencyBinCount), hz = X.ctx.sampleRate / an.fftSize;
      an.getFloatFrequencyData(datos);
      g.beginPath(); g.moveTo(0, h);
      for (let x = 0; x <= w; x += 2) { const k = Math.min(datos.length - 1, Math.max(1, Math.round(fDe(x, w) / hz))); g.lineTo(x, h - Math.min(1, Math.max(0, (datos[k] + 96) / 90)) * h); }
      g.lineTo(w, h); g.closePath(); g.fillStyle = css("--accent-soft", "rgba(138,125,243,.12)"); g.fill();
    }
    // la curva (del núcleo)
    const eq = bandasVivas || (o ? o.insertos.eq || [] : []);
    if (curvaEq) {
      g.beginPath();
      curvaEq.forEach((d, i) => { const x = xDe(frecuencias[i], w), y = yDe(Math.max(-DB * 1.5, Math.min(DB * 1.5, d)), h); i ? g.lineTo(x, y) : g.moveTo(x, y); });
      g.strokeStyle = css("--accent", "#8a7df3"); g.lineWidth = 2; g.stroke();
      g.lineTo(w, yDe(0, h)); g.lineTo(0, yDe(0, h)); g.closePath(); g.globalAlpha = 0.13; g.fillStyle = css("--accent", "#8a7df3"); g.fill(); g.globalAlpha = 1;
    }
    // las bandas
    eq.forEach((b, i) => {
      const x = xDe(b.frecuencia, w), y = yDe(conGanancia(b.filtro) ? b.db : 0, h);
      g.beginPath(); g.arc(x, y, i === eqSel ? 8 : 6.5, 0, Math.PI * 2);
      g.fillStyle = b.apagada ? css("--faint", "#666") : EX.COLORES[i % EX.COLORES.length]; g.globalAlpha = i === eqSel ? 1 : 0.85; g.fill(); g.globalAlpha = 1;
      if (i === eqSel) { g.strokeStyle = css("--text", "#fff"); g.lineWidth = 1.5; g.stroke(); }
      g.fillStyle = "#0b0b0e"; g.font = "bold 9.5px ui-monospace, monospace"; g.textAlign = "center"; g.fillText(String(i + 1), x, y + 3.5); g.textAlign = "left"; g.font = "9.5px ui-monospace, monospace";
    });
  }

  // ── el análisis ─────────────────────────────────────────────────────────
  let analisis = null; // { de, a } | { de, cargando } | { de, error }
  D.analizar = async (pista) => {
    const de = pista ? `${tt("exPista")} ${pista} · ${(EX.pista(pista) || {}).nombre || ""}` : tt("exMezclaFinal");
    analisis = { de, cargando: true };
    if (X.dock === "analisis") D.pintar();
    try { analisis = { de, pista, a: await invoke("experto_analizar", { proyecto: X.root, pista: pista || null }) }; }
    catch (e) { analisis = { de, error: String(e && e.message ? e.message : e) }; }
    if (X.dock === "analisis") D.pintar();
    EX.lado.pintar();
  };
  D.ultimoAnalisis = () => analisis && analisis.a;
  function pintarAnalisis(c) {
    const a = analisis && analisis.a, num = (v, d, u) => (v === null || v === undefined || !isFinite(v) ? "—" : `${v.toFixed(d)}${u ? " " + u : ""}`);
    const fila = (t, v, extra, ayuda) => `<div class="ex-dato"${ayuda ? ` title="${esc(tt(ayuda))}"` : ""}><span>${esc(tt(t))}</span><b class="mono">${v}</b>${extra || ""}</div>`;
    const pct = (v) => `${Math.round(v * 100)} %`;
    c.innerHTML = `<div class="ex-analisis">
      <div class="ex-an-izq"><div class="ex-botones"><button type="button" class="ex-bt" id="exAnMezcla">${esc(tt("exAnalizarMezcla"))}</button><button type="button" class="ex-bt" id="exAnPista"${X.pistaSel ? "" : " disabled"}>${esc(tt("exAnalizarPista"))}</button></div>
        ${!analisis ? `<p class="ex-nota">${esc(tt("exAnalisisAyuda"))}</p>` : analisis.cargando ? `<p class="ex-nota"><span class="ex-giro"></span> ${esc(tt("exAnalizando"))} ${esc(analisis.de)}…</p>` : analisis.error ? `<p class="ex-nota error">✗ ${esc(analisis.error)}</p>` : `<h4>${esc(analisis.de)} <i class="mono">${EX.fmtT(a.segundos).slice(0, -4)}</i></h4>
        <div class="ex-datos">
          ${fila("exTempoD", a.bpm ? num(a.bpm, 1, "bpm") : "—", a.bpm ? `<em>${pct(a.bpm_confianza)}</em><button type="button" class="ex-t" data-usar="bpm">${esc(tt("exUsar"))}</button><button type="button" class="ex-t" data-usar="bpm2">×2</button><button type="button" class="ex-t" data-usar="bpm05">÷2</button>` : "", "exTempoAyuda")}
          ${fila("exTonalidad", a.tonalidad ? esc(a.tonalidad) : "—", a.tonalidad ? `<em>${pct(a.tonalidad_confianza)}</em><button type="button" class="ex-t" data-usar="tono">${esc(tt("exUsar"))}</button>` : "")}
          ${fila("exSonoridadInt", num(a.lufs, 1, "LUFS"), "", "exSonoridadAyuda")}${fila("exSonoridadCorta", num(a.lufs_corto_max, 1, "LUFS"))}${fila("exRango", num(a.rango_lu, 1, "LU"), "", "exRangoAyuda")}
          ${fila("exPicoReal", `<span class="${a.pico_real_db > -1 ? "mal" : ""}">${num(a.pico_real_db, 2, "dBTP")}</span>`, "", "exPicoRealAyuda")}${fila("exPico", num(a.pico_db, 2, "dBFS"))}${fila("exRms", num(a.rms_db, 1, "dB"))}
          ${fila("exCorrelacion", `<span class="${a.correlacion < 0 ? "mal" : ""}">${num(a.correlacion, 2)}</span>`, "", "exCorrelacionAyuda")}
        </div>`}</div>
      <div class="ex-an-der"><div class="ex-an-titulo"><span>${esc(tt("exEspectro"))}</span><span class="ex-corr" title="${esc(tt("exCorrelacionAyuda"))}">−1<i><b id="exCorr"></b></i>+1</span></div><div class="ex-eq-caja"><canvas id="exEspectro"></canvas></div></div></div>`;
    $("exAnMezcla").onclick = () => D.analizar(null);
    $("exAnPista").onclick = () => D.analizar(X.pistaSel);
    c.querySelectorAll("button[data-usar]").forEach((b) => (b.onclick = () => {
      const u = b.dataset.usar, e = est();
      if (u === "tono") op({ op: "tonalidad", tonalidad: a.tonalidad });
      else op({ op: "tempo", bpm: Math.min(400, Math.max(20, a.bpm * (u === "bpm2" ? 2 : u === "bpm05" ? 0.5 : 1))), compas: e.compas || 4 });
    }));
    dibujarEspectro(true);
  }
  let ultimoEsp = 0, tiempoL = null, tiempoR = null;
  function dibujarEspectro(forzar) {
    const cv = $("exEspectro"); if (!cv) return;
    const ahora = performance.now();
    if (!forzar && (!X.sonando || ahora - ultimoEsp < 50)) return;
    ultimoEsp = ahora;
    const caja = cv.parentElement, w = caja.clientWidth, h = caja.clientHeight, dpr = window.devicePixelRatio || 1;
    if (!w || !h) return;
    if (cv.width !== Math.round(w * dpr) || cv.height !== Math.round(h * dpr)) { cv.width = Math.round(w * dpr); cv.height = Math.round(h * dpr); }
    const g = cv.getContext("2d");
    g.setTransform(dpr, 0, 0, dpr, 0, 0); g.clearRect(0, 0, w, h); g.font = "9.5px ui-monospace, monospace";
    for (const f of [50, 100, 200, 500, 1000, 2000, 5000, 10000]) { g.fillStyle = css("--line", "#333"); g.fillRect(Math.round(xDe(f, w)), 0, 1, h); g.fillStyle = css("--faint", "#777"); g.fillText(fmtHz(f).replace(" ", ""), xDe(f, w) + 3, h - 3); }
    for (const d of [-72, -54, -36, -18]) { const y = h - ((d + 96) / 90) * h; g.fillStyle = css("--line", "#333"); g.fillRect(0, Math.round(y), w, 1); g.fillStyle = css("--faint", "#777"); g.fillText(String(d), 3, y - 2); }
    if (!X.espectro || !X.ctx) return;
    const datos = new Float32Array(X.espectro.frequencyBinCount), hz = X.ctx.sampleRate / X.espectro.fftSize;
    X.espectro.getFloatFrequencyData(datos);
    g.beginPath(); g.moveTo(0, h);
    for (let x = 0; x <= w; x += 1.5) {
      // el máximo de los bins que caen en este píxel (arriba hay muchos por píxel)
      const k0 = Math.max(1, Math.floor(fDe(x, w) / hz)), k1 = Math.min(datos.length - 1, Math.max(k0, Math.floor(fDe(x + 1.5, w) / hz)));
      let m = -200; for (let k = k0; k <= k1; k++) if (datos[k] > m) m = datos[k];
      g.lineTo(x, h - Math.min(1, Math.max(0, (m + 96) / 90)) * h);
    }
    g.lineTo(w, h); g.closePath();
    const grad = g.createLinearGradient(0, 0, 0, h);
    grad.addColorStop(0, css("--accent", "#8a7df3")); grad.addColorStop(1, "transparent");
    g.globalAlpha = 0.55; g.fillStyle = grad; g.fill(); g.globalAlpha = 1;
    // la correlación de fase, en vivo
    const n = X.medL.fftSize;
    tiempoL = tiempoL && tiempoL.length === n ? tiempoL : new Float32Array(n); tiempoR = tiempoR && tiempoR.length === n ? tiempoR : new Float32Array(n);
    X.medL.getFloatTimeDomainData(tiempoL); X.medR.getFloatTimeDomainData(tiempoR);
    let lr = 0, ll = 0, rr = 0;
    for (let i = 0; i < n; i++) { lr += tiempoL[i] * tiempoR[i]; ll += tiempoL[i] * tiempoL[i]; rr += tiempoR[i] * tiempoR[i]; }
    const co = ll > 1e-9 && rr > 1e-9 ? lr / Math.sqrt(ll * rr) : 0, el = $("exCorr");
    if (el) { el.style.left = ((co + 1) / 2) * 100 + "%"; el.classList.toggle("mal", co < 0); }
  }

  // ── el historial ────────────────────────────────────────────────────────
  function pintarHistorial(c) {
    const h = (X.v && X.v.historial) || [], r = (X.v && X.v.por_rehacer) || [];
    c.innerHTML = `<div class="ex-historial"><p class="ex-nota">${esc(tt("exHistorialAyuda"))}</p><ol>
      <li><button type="button" data-n="0" class="${h.length ? "" : "aqui"}"><i class="mono">0</i>${esc(tt("exHistorialInicio"))}</button></li>
      ${h.map((t, i) => `<li><button type="button" data-n="${i + 1}" class="${i + 1 === h.length ? "aqui" : ""}"><i class="mono">${i + 1}</i>${esc(t)}</button></li>`).join("")}
      ${r.map((t, i) => `<li><button type="button" data-n="${h.length + i + 1}" class="deshecho"><i class="mono">${h.length + i + 1}</i>${esc(t)}</button></li>`).join("")}</ol></div>`;
    c.querySelectorAll("button[data-n]").forEach((b) => (b.onclick = async () => { const v = await EX.llamar("experto_ir_a", { n: Number(b.dataset.n) }); if (v) EX.actualizar(v); }));
    const aqui = c.querySelector(".aqui");
    if (aqui) aqui.scrollIntoView({ block: "nearest" });
  }
})();
