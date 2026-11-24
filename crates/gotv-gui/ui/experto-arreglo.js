// Ola EXPERTO II · X2-4 — el ARREGLO: la línea de tiempo del estudio.
//
// Regla (compases y tiempo, marcadores, bucle), pistas con su cabecera, y en cada carril sus clips (se mueven,
// se recortan, se parten, llevan fundidos), sus tramos de efecto, su curva de automatización y la selección.
// Cada gesto acaba en UNA operación del núcleo (`EX.op`): la ventana no decide nada del sonido.
//
// Los carriles pueden medir decenas de miles de píxeles: la onda se dibuja en un lienzo del ancho de lo
// VISIBLE, pegado a la izquierda (sticky), y se repinta al desplazar.
(() => {
  const X = EX.X, CAB = EX.CAB, est = EX.est, op = EX.op;
  const A = (EX.arreglo = {});
  const REGLA = 36, FRANJA = 15; // alto de la regla y de la franja de tramos de efecto sobre los clips
  const fs = () => (X.v && X.v.frecuencia) || 48000;
  const ancho = () => Math.ceil(EX.largo() * X.zoom);
  const css = (n, d) => getComputedStyle(document.body).getPropertyValue(n).trim() || d;
  const tDe = (ev, carril) => Math.max(0, (ev.clientX - carril.getBoundingClientRect().left) / X.zoom);
  // arrastrar: `mover(e, dx, dy)` desde que se pasa de 3 px; `soltar(e, movido)`
  // Mientras dura un gesto NO se repinta (`EX.gesto`): un repintado a mitad —p. ej. porque acaba de llegar el
  // audio de una pista— cambiaría los elementos que el gesto está midiendo y moviendo (medido 2026-11-14: una
  // selección que debía acabar en 9 s acabó en 23). Lo pendiente se pinta al soltar.
  function arrastrar(ev, mover, soltar) {
    let movido = false;
    EX.gesto = true;
    const m = (e) => {
      if (!movido && Math.abs(e.clientX - ev.clientX) < 3 && Math.abs(e.clientY - ev.clientY) < 3) return;
      movido = true; document.body.classList.add("ex-arrastrando");
      mover(e, e.clientX - ev.clientX, e.clientY - ev.clientY);
    };
    const s = (e) => {
      document.removeEventListener("mousemove", m); document.removeEventListener("mouseup", s); document.body.classList.remove("ex-arrastrando");
      EX.gesto = false;
      const pendiente = EX.pintarPendiente;
      EX.pintarPendiente = false;
      soltar(e, movido);
      if (pendiente) EX.pintar();
    };
    document.addEventListener("mousemove", m); document.addEventListener("mouseup", s);
  }
  EX.arrastrar = arrastrar;

  // ── montar ──────────────────────────────────────────────────────────────
  A.montar = () => {
    $("exArreglo").innerHTML = `
      <div class="ex-scroll" id="exScroll">
        <div class="ex-regla-fila">
          <div class="ex-esquina" style="width:${CAB}px">
            <button type="button" class="ex-t" id="exNuevaPista" title="${esc(tt("exImportarAyuda"))}">＋ ${esc(tt("exPista"))}</button>
            <button type="button" class="ex-t" id="exMarcar" title="${esc(tt("exMarcadorAyuda"))}">${EX.icono(EX.I.marcador)}</button>
          </div>
          <div class="ex-regla" id="exRegla"><canvas id="exReglaCv" height="${REGLA}"></canvas><div class="ex-bucle-r" id="exBucleR" hidden><i data-z="izq"></i><i data-z="der"></i></div><div id="exMarcas"></div></div>
        </div>
        <div id="exPistas"></div>
        <div class="ex-cabezal" id="exCabezal"><i></i></div>
      </div>
      <div class="ex-vacio" id="exVacio"><div class="ex-vacio-caja"><span class="ex-logo grande">${EX.icono(EX.I.chispa)}</span>
        <h3>${esc(tt("exVacioTitulo"))}</h3><p>${esc(tt("exVacio"))}</p>
        <div class="ex-botones centro"><button type="button" class="ex-bt primario" id="exVacioIA">${EX.icono(EX.I.chispa)}<span>${esc(tt("exGenerarConIA"))}</span></button>
        <button type="button" class="ex-bt" id="exVacioImportar">${EX.icono(EX.I.importar)}<span>${esc(tt("exImportar"))}</span></button></div></div></div>`;
    const sc = $("exScroll");
    sc.addEventListener("scroll", () => A.redibujar());
    // pellizco o ⌘ + rueda: zoom alrededor del puntero
    sc.addEventListener("wheel", (e) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      e.preventDefault();
      EX.ponerZoom(X.zoom * Math.exp(-e.deltaY * 0.01), e.clientX - sc.getBoundingClientRect().left - CAB);
    }, { passive: false });
    $("exNuevaPista").onclick = EX.importar; $("exVacioImportar").onclick = EX.importar;
    $("exVacioIA").onclick = () => EX.lado.abrirIA("generar");
    $("exMarcar").onclick = A.marcadorAqui;
    regla();
  };

  // ── la regla: posición, bucle y marcadores ─────────────────────────────
  function regla() {
    const r = $("exRegla");
    r.addEventListener("mousedown", (ev) => {
      if (ev.button !== 0 || ev.target.closest(".ex-marca-b")) return;
      const zona = ev.target.dataset.z, enBucle = ev.target.closest("#exBucleR");
      const t0 = tDe(ev, r), b0 = X.bucleR ? { ...X.bucleR } : null;
      arrastrar(ev, (e) => {
        const t = EX.iman(tDe(e, r), e);
        if (enBucle && b0) {
          if (zona === "izq") X.bucleR = { desde: Math.min(t, b0.hasta - 0.02), hasta: b0.hasta };
          else if (zona === "der") X.bucleR = { desde: b0.desde, hasta: Math.max(t, b0.desde + 0.02) };
          else { const d = EX.iman(b0.desde + (tDe(e, r) - t0), e); X.bucleR = { desde: d, hasta: d + (b0.hasta - b0.desde) }; }
        } else { const a = EX.iman(t0, e); X.bucleR = { desde: Math.min(a, t), hasta: Math.max(a, t) }; X.bucle = true; }
        pintarBucle();
      }, (e, movido) => {
        if (!movido) { if (!enBucle) EX.irA(EX.iman(t0, e)); return; }
        if (X.bucleR && X.bucleR.hasta - X.bucleR.desde < 0.02) { X.bucleR = null; X.bucle = false; }
        EX.pintar();
        if (X.sonando) { EX.parar(); EX.reproducir(); }
      });
    });
    r.addEventListener("dblclick", (ev) => { if (!ev.target.closest(".ex-marca-b, #exBucleR")) marcador(EX.iman(tDe(ev, r), ev)); });
  }
  const marcadores = (lista) => op({ op: "marcadores", marcadores: lista });
  function marcador(t) {
    const m = est().marcadores, n = m.length + 1;
    marcadores(m.concat({ segundo: t, nombre: `${tt("exMarcador")} ${n}` }));
  }
  A.marcadorAqui = () => marcador(X.pos);
  function pintarBucle() {
    const b = $("exBucleR"); if (!b) return;
    b.hidden = !X.bucleR;
    if (X.bucleR) { b.style.left = X.bucleR.desde * X.zoom + "px"; b.style.width = Math.max(2, (X.bucleR.hasta - X.bucleR.desde) * X.zoom) + "px"; b.classList.toggle("on", X.bucle); }
  }
  function pintarMarcas() {
    const c = $("exMarcas"); if (!c) return;
    c.innerHTML = "";
    est().marcadores.forEach((m, i) => {
      const b = document.createElement("button");
      b.type = "button"; b.className = "ex-marca-b"; b.style.left = m.segundo * X.zoom + "px";
      b.title = `${m.nombre} · ${EX.fmtT(m.segundo)} — ${tt("exMarcadorUso")}`;
      b.innerHTML = `<span>${esc(m.nombre)}</span>`;
      b.addEventListener("mousedown", (ev) => {
        if (ev.button !== 0) return;
        ev.stopPropagation();
        let t = m.segundo;
        arrastrar(ev, (e) => { t = EX.iman(tDe(e, $("exRegla")), e); b.style.left = t * X.zoom + "px"; }, (e, movido) => {
          if (!movido) return EX.irA(m.segundo);
          marcadores(est().marcadores.map((x, k) => (k === i ? { ...x, segundo: t } : x)));
        });
      });
      b.addEventListener("dblclick", async (ev) => {
        ev.stopPropagation();
        const n = await EX.pedirTexto(tt("exMarcadorNombre"), m.nombre);
        if (n !== null && n.trim()) marcadores(est().marcadores.map((x, k) => (k === i ? { ...x, nombre: n.trim() } : x)));
      });
      b.addEventListener("contextmenu", (ev) => { ev.preventDefault(); marcadores(est().marcadores.filter((_, k) => k !== i)); });
      c.appendChild(b);
    });
  }
  // los pasos de la rejilla que caben: cada cuántos compases se rotula y si se ven los pulsos
  function rejilla() {
    const p = EX.pulso(), n = est().compas || 4, pxCompas = p * n * X.zoom;
    const cada = [1, 2, 4, 8, 16, 32, 64, 128].find((k) => k * pxCompas >= 58) || 256;
    return { p, n, compas: p * n, cada, pulsos: p * X.zoom >= 9, lineaCompas: pxCompas >= 5 ? 1 : cada };
  }
  A.pintarRegla = () => {
    const cv = $("exReglaCv"), sc = $("exScroll"); if (!cv || !sc) return;
    const vw = Math.max(50, sc.clientWidth - CAB), dpr = window.devicePixelRatio || 1, x0 = sc.scrollLeft;
    $("exRegla").style.width = ancho() + "px";
    cv.style.width = vw + "px"; cv.width = vw * dpr; cv.height = REGLA * dpr;
    const g = cv.getContext("2d");
    g.setTransform(dpr, 0, 0, dpr, 0, 0); g.clearRect(0, 0, vw, REGLA);
    g.font = "10.5px ui-monospace, SFMono-Regular, Menlo, monospace"; g.textBaseline = "alphabetic";
    const R = rejilla(), [t0, t1] = [x0 / X.zoom, (x0 + vw) / X.zoom];
    // arriba, los compases
    for (let k = Math.max(0, Math.floor(t0 / R.compas)); k * R.compas <= t1; k++) {
      const x = k * R.compas * X.zoom - x0;
      if (k % R.cada === 0) { g.fillStyle = css("--text", "#eee"); g.fillText(String(k + 1), x + 4, 12); g.fillStyle = css("--muted", "#888"); g.fillRect(x, 0, 1, 17); }
      else if (k % R.lineaCompas === 0) { g.fillStyle = css("--line-2", "#444"); g.fillRect(x, 11, 1, 6); }
      if (R.pulsos) { g.fillStyle = css("--line-2", "#444"); for (let b = 1; b < R.n; b++) g.fillRect(x + b * R.p * X.zoom, 13, 1, 4); }
    }
    // abajo, el tiempo
    const paso = [0.1, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300].find((s) => s * X.zoom >= 64) || 600;
    g.fillStyle = css("--faint", "#777");
    for (let s = Math.floor(t0 / paso) * paso; s <= t1; s += paso) {
      const x = s * X.zoom - x0;
      g.fillRect(x, 28, 1, 8);
      const m = Math.floor(s / 60), r = s - m * 60;
      g.fillText(paso < 1 ? `${m}:${r.toFixed(1).padStart(4, "0")}` : `${m}:${String(Math.round(r)).padStart(2, "0")}`, x + 4, 33);
    }
    g.fillStyle = css("--line", "#333"); g.fillRect(0, 18, vw, 1);
    pintarBucle();
  };

  // ── las pistas ──────────────────────────────────────────────────────────
  A.pintar = () => {
    const cont = $("exPistas"); if (!cont) return;
    if (EX.gesto) return void (EX.pintarPendiente = true);
    const e = est(), w = ancho();
    $("exVacio").hidden = e.pistas.length > 0;
    $("exScroll").classList.toggle("sin-pistas", !e.pistas.length);
    cont.innerHTML = "";
    cont.style.setProperty("--ex-alto", X.alto + "px");
    cont.dataset.alto = X.alto <= 56 ? "bajo" : X.alto >= 120 ? "alto" : "medio";
    // primero los buses (con sus pistas), luego las sueltas
    for (const g of e.grupos) {
      const suyas = e.pistas.filter((p) => p.grupo === g.id);
      cont.appendChild(filaGrupo(g, suyas.length, w));
      if (!X.plegados.has(g.id)) for (const p of suyas) cont.appendChild(filaPista(p, w, true));
    }
    for (const p of e.pistas.filter((p) => !p.grupo || !e.grupos.some((g) => g.id === p.grupo))) cont.appendChild(filaPista(p, w, false));
    pintarMarcas();
    A.redibujar();
    A.cabezal();
  };
  const fader = (v, extra) => `<input type="range" class="ex-gan" min="-60" max="12" step="0.5" value="${Math.min(12, v)}" ${extra || ""}>`;
  const paneo = (v, extra) => `<input type="range" class="ex-pan" min="-1" max="1" step="0.05" value="${v}" ${extra || ""}>`;
  const fmtPan = (v) => (Math.abs(v) < 0.025 ? "C" : (v < 0 ? "L" : "R") + Math.round(Math.abs(v) * 100));

  function filaGrupo(g, n, w) {
    const f = document.createElement("div");
    f.className = "ex-fila ex-grupo" + (X.objetivo.tipo === "grupo" && X.objetivo.id === g.id ? " sel" : "");
    f.dataset.grupo = g.id;
    const fx = !EX.vacio(g.insertos);
    f.innerHTML = `<div class="ex-cabecera" style="width:${CAB}px">
        <div class="ex-cab-1"><button type="button" class="ex-plegar" aria-expanded="${!X.plegados.has(g.id)}" title="${esc(tt("exPlegar"))}">${X.plegados.has(g.id) ? "▸" : "▾"}</button>
        <span class="ex-nombre" title="${esc(g.nombre)}"><b class="ex-bus-e">BUS</b> ${esc(g.nombre)} <i>(${n})</i></span>
        ${fx ? `<span class="ex-fx-on" title="${esc(tt("exConEfectos"))}">FX</span>` : ""}
        <button type="button" class="ex-ms${g.silencio ? " on" : ""}" data-a="m" title="${esc(tt("exSilencio"))}">M</button>
        <button type="button" class="ex-ms solo${g.solo ? " on" : ""}" data-a="s" title="${esc(tt("exSolo"))}">S</button>
        <button type="button" class="ex-mas" data-a="menu" aria-haspopup="menu" title="${esc(tt("exMas"))}">⋯</button></div>
        <div class="ex-cab-2">${fader(g.ganancia_db, `title="${esc(tt("exGanancia"))}"`)}<span class="ex-val mono">${EX.fmtDb(g.ganancia_db)}</span>${paneo(g.pan, `title="${esc(tt("exPan"))}"`)}</div>
        <div class="ex-vu" data-vu="g${g.id}"><div></div><i></i></div>
      </div><div class="ex-carril ex-carril-grupo" style="width:${w}px"></div>`;
    f.querySelector(".ex-cabecera").onclick = () => { X.objetivo = { tipo: "grupo", id: g.id }; X.pistaSel = null; X.clipSel = null; EX.pintar(); };
    f.querySelector(".ex-plegar").onclick = (ev) => { ev.stopPropagation(); X.plegados.has(g.id) ? X.plegados.delete(g.id) : X.plegados.add(g.id); A.pintar(); };
    f.querySelectorAll("button[data-a]").forEach((b) => (b.onclick = (ev) => {
      ev.stopPropagation();
      if (b.dataset.a === "m") op({ op: "mezcla_grupo", grupo: g.id, silencio: !g.silencio });
      else if (b.dataset.a === "s") op({ op: "mezcla_grupo", grupo: g.id, solo: !g.solo });
      else EX.menu(b, [[tt("exRenombrar"), async () => { const t = await EX.pedirTexto(tt("exRenombrar"), g.nombre); if (t && t.trim()) op({ op: "renombrar_grupo", grupo: g.id, nombre: t.trim() }); }],
        [tt("exEfectosBus"), () => { X.objetivo = { tipo: "grupo", id: g.id }; EX.dock.abrir("canal"); }], [tt("exQuitarGrupo"), () => op({ op: "quitar_grupo", id: g.id }), "peligro"]]);
    }));
    const gan = f.querySelector(".ex-gan"), pan = f.querySelector(".ex-pan"), val = f.querySelector(".ex-val");
    for (const r of [gan, pan]) r.onclick = (ev) => ev.stopPropagation();
    gan.oninput = () => { EX.vivoGrupo(g.id, Number(gan.value), null); val.textContent = EX.fmtDb(Number(gan.value)); };
    pan.oninput = () => EX.vivoGrupo(g.id, null, Number(pan.value));
    gan.onchange = () => op({ op: "mezcla_grupo", grupo: g.id, ganancia_db: Number(gan.value) });
    pan.onchange = () => op({ op: "mezcla_grupo", grupo: g.id, pan: Number(pan.value) });
    // doble clic: a 0 (se lee en `mousedown`: tras el primer clic la fila se repinta y `dblclick` no llega)
    gan.onmousedown = (ev) => { if (ev.detail >= 2) { ev.preventDefault(); op({ op: "mezcla_grupo", grupo: g.id, ganancia_db: 0 }); } };
    pan.onmousedown = (ev) => { if (ev.detail >= 2) { ev.preventDefault(); op({ op: "mezcla_grupo", grupo: g.id, pan: 0 }); } };
    return f;
  }

  function filaPista(p, w, enGrupo) {
    const f = document.createElement("div"), color = EX.colorDe(p), modo = X.auto[p.id];
    f.className = "ex-fila" + (X.pistaSel === p.id ? " sel" : "") + (enGrupo ? " en-grupo" : "") + (EX.suena(p) ? "" : " muda") + (X.cargando && X.cargando.has(p.id) ? " calculando" : "");
    f.dataset.pista = p.id; f.style.setProperty("--pista", color);
    const origen = p.origen === "separada" ? "✂︎" : p.origen === "generada" ? "✦" : "";
    const fx = !EX.vacio(p.insertos);
    f.innerHTML = `<div class="ex-cabecera" style="width:${CAB}px">
        <div class="ex-cab-1"><button type="button" class="ex-color" data-a="color" title="${esc(tt("exColor"))}"></button><b class="ex-num mono">${p.id}</b>
        <span class="ex-nombre" title="${esc(p.nombre + " — " + tt("exRenombrarAyuda"))}">${origen ? `<i class="ex-origen" title="${esc(tt(p.origen === "separada" ? "exOrigenSeparada" : "exOrigenGenerada"))}">${origen}</i> ` : ""}${esc(p.nombre)}</span>
        ${fx ? `<button type="button" class="ex-fx-on" data-a="fx" title="${esc(tt("exConEfectos"))}">FX</button>` : ""}
        <button type="button" class="ex-ms${p.silencio ? " on" : ""}" data-a="m" title="${esc(tt("exSilencio"))}">M</button>
        <button type="button" class="ex-ms solo${p.solo ? " on" : ""}" data-a="s" title="${esc(tt("exSolo"))}">S</button>
        <button type="button" class="ex-ms auto${modo ? " on" : ""}" data-a="auto" title="${esc(tt("exAutoAyuda"))}">${modo === "pan" ? "PAN" : modo === "volumen" ? "VOL" : "A"}</button>
        <button type="button" class="ex-mas" data-a="menu" aria-haspopup="menu" title="${esc(tt("exMas"))}">⋯</button></div>
        <div class="ex-cab-2">${fader(p.ganancia_db, `title="${esc(tt("exGanancia"))}"`)}<span class="ex-val mono">${EX.fmtDb(p.ganancia_db)}</span>${paneo(p.pan, `title="${esc(tt("exPan"))} · ${fmtPan(p.pan)}"`)}</div>
        <div class="ex-vu" data-vu="p${p.id}"><div></div><i></i></div>
      </div>
      <div class="ex-carril" style="width:${w}px"><canvas class="ex-onda"></canvas></div>`;
    const cab = f.querySelector(".ex-cabecera");
    cab.onclick = (ev) => {
      // doble clic en el nombre: renombrar (se lee aquí: el primer clic repinta la fila y `dblclick` no llega)
      if (ev.detail >= 2 && ev.target.closest(".ex-nombre")) return renombrar(p, f.querySelector(".ex-nombre"));
      X.pistaSel = p.id; X.objetivo = { tipo: "pista", id: p.id }; X.regionSel = null; X.clipSel = null; EX.pintar();
    };
    f.querySelectorAll("button[data-a]").forEach((b) => (b.onclick = (ev) => {
      ev.stopPropagation();
      const a = b.dataset.a;
      if (a === "m") op({ op: "mezcla", pista: p.id, silencio: !p.silencio });
      else if (a === "s") op({ op: "mezcla", pista: p.id, solo: !p.solo });
      else if (a === "auto") { X.auto[p.id] = modo === "volumen" ? "pan" : modo === "pan" ? undefined : "volumen"; A.pintar(); }
      else if (a === "color") paleta(p, b);
      else if (a === "fx") { X.pistaSel = p.id; X.objetivo = { tipo: "pista", id: p.id }; EX.dock.abrir("canal"); EX.pintar(); }
      else menuPista(p, b);
    }));
    const gan = f.querySelector(".ex-gan"), pan = f.querySelector(".ex-pan"), val = f.querySelector(".ex-val");
    for (const r of [gan, pan]) r.onclick = (ev) => ev.stopPropagation();
    gan.oninput = () => { EX.vivoPista(p.id, Number(gan.value), null); val.textContent = EX.fmtDb(Number(gan.value)); };
    pan.oninput = () => { EX.vivoPista(p.id, null, Number(pan.value)); pan.title = `${tt("exPan")} · ${fmtPan(Number(pan.value))}`; };
    gan.onchange = () => op({ op: "mezcla", pista: p.id, ganancia_db: Number(gan.value) });
    pan.onchange = () => op({ op: "mezcla", pista: p.id, pan: Number(pan.value) });
    gan.onmousedown = (ev) => { if (ev.detail >= 2) { ev.preventDefault(); op({ op: "mezcla", pista: p.id, ganancia_db: 0 }); } };
    pan.onmousedown = (ev) => { if (ev.detail >= 2) { ev.preventDefault(); op({ op: "mezcla", pista: p.id, pan: 0 }); } };

    const carril = f.querySelector(".ex-carril");
    for (const r of est().regiones.filter((r) => r.pista === p.id)) carril.appendChild(elRegion(r, p, carril));
    for (const c of EX.clipsDe(p.id)) carril.appendChild(elClip(c, p, carril));
    if (X.sel && X.sel.pista === p.id && X.sel.hasta > X.sel.desde) pintarSeleccion(carril);
    carril.addEventListener("mousedown", (ev) => { if (ev.button === 0) enCarril(ev, carril, p.id); });
    if (modo) carril.appendChild(capaAuto(carril, p, modo, w));
    return f;
  }
  function renombrar(p, span) {
    const i = document.createElement("input");
    i.className = "ex-renombrar"; i.value = p.nombre; i.maxLength = 80;
    span.replaceWith(i); i.focus(); i.select();
    let hecho = false;
    const fin = (ok) => { if (hecho) return; hecho = true; const n = i.value.trim(); if (ok && n && n !== p.nombre) op({ op: "renombrar", pista: p.id, nombre: n }); else A.pintar(); };
    i.onclick = (e) => e.stopPropagation();
    i.onkeydown = (e) => { e.stopPropagation(); if (e.key === "Enter") fin(true); else if (e.key === "Escape") fin(false); };
    i.onblur = () => fin(true);
  }
  function paleta(p, ancla) {
    EX.menu(ancla, [], (m) => {
      const d = document.createElement("div");
      d.className = "ex-paleta";
      d.innerHTML = EX.COLORES.map((c) => `<button type="button" style="background:${c}" data-c="${c}" title="${c}"></button>`).join("") + `<button type="button" class="ex-t" data-c="">${esc(tt("exColorTema"))}</button>`;
      d.onclick = (e) => { const b = e.target.closest("button"); if (!b) return; m.cerrar(); op({ op: "color", pista: p.id, color: b.dataset.c }); };
      m.appendChild(d);
    });
  }
  function menuPista(p, ancla) {
    const e = est(), orden = e.pistas.map((x) => x.id), i = orden.indexOf(p.id);
    const mover = (d) => { const o = orden.slice(); [o[i], o[i + d]] = [o[i + d], o[i]]; op({ op: "ordenar", pistas: o }); };
    const separar = (modelo) => EX.trabajo(`${tt("exSeparando")} · ${p.nombre}`, "experto_separar", { pista: p.id, modelo });
    const items = [
      [tt("exRenombrar"), () => { const s = document.querySelector(`.ex-fila[data-pista="${p.id}"] .ex-nombre`); if (s) renombrar(p, s); }],
      [tt("exRehacerIA"), () => { X.pistaSel = p.id; EX.lado.abrirIA("rehacer"); }, "ia"],
      [tt("exVersionarIA"), () => { X.pistaSel = p.id; EX.lado.abrirIA("versionar"); }, "ia"],
      [tt("exSeparar"), () => separar("4"), "ia"], [tt("exSepararSeis"), () => separar("6"), "ia"], [tt("exSepararFino"), () => separar("ft"), "ia"],
      // EXP-7: partir la batería (afinado de GOTV); sobre una mezcla, primero se saca su batería
      [tt(p.fuente === "drums" ? "exPartirBateria" : "exPartirBateriaMezcla"), () => separar("bateria"), "ia"],
      [tt("exAnalizarPista"), () => { X.pistaSel = p.id; EX.dock.abrir("analisis"); EX.dock.analizar(p.id); }],
      "-",
      ...(i > 0 ? [[tt("exSubir"), () => mover(-1)]] : []), ...(i < orden.length - 1 ? [[tt("exBajar"), () => mover(1)]] : []),
      ...e.grupos.filter((g) => g.id !== p.grupo).map((g) => [`${tt("exAlBus")} «${g.nombre}»`, () => op({ op: "agrupar", pistas: [p.id], grupo: g.id })]),
      [tt("exBusNuevo"), () => A.busNuevo([p.id])],
      ...(p.grupo ? [[tt("exSacarDelBus"), () => op({ op: "agrupar", pistas: [p.id], grupo: null })]] : []),
      ...(EX.curvaDe(p.id, "volumen") || EX.curvaDe(p.id, "pan") ? [[tt("exQuitarCurvas"), async () => { for (const par of ["volumen", "pan"]) if (EX.curvaDe(p.id, par)) await op({ op: "automatizar", pista: p.id, parametro: par, puntos: [] }); }]] : []),
      "-", [tt("exQuitar"), () => op({ op: "quitar_pista", id: p.id }), "peligro"],
    ];
    EX.menu(ancla, items);
  }
  A.busNuevo = async (pistas) => {
    const nombre = await EX.pedirTexto(tt("exAgruparNombre"), "");
    if (!nombre || !nombre.trim()) return;
    const v = await EX.llamar("experto_aplicar", { op: { op: "crear_grupo", grupo: { id: 0, nombre: nombre.trim() } } });
    if (!v) return;
    const g = v.estado.grupos[v.estado.grupos.length - 1];
    if (pistas && pistas.length) await op({ op: "agrupar", pistas, grupo: g.id });
    else await EX.actualizar(v);
  };

  // ── los tramos de efecto (regiones) ────────────────────────────────────
  function etiqueta(r) {
    const e = r.efecto, n = tt("exFiltros").split("|");
    const f = { graves: n[0], agudos: n[1], campana: n[2], paso_alto: n[3], paso_bajo: n[4], muesca: n[5], paso_banda: n[6] };
    if (e.tipo === "ganancia") return `${e.db > 0 ? "+" : ""}${e.db} dB`;
    if (e.tipo === "silencio") return tt("exSilencioCorto");
    if (e.tipo === "eq") return `EQ ${f[e.filtro] || e.filtro} ${e.db > 0 ? "+" : ""}${e.db} dB · ${Math.round(e.frecuencia)} Hz`;
    if (e.tipo === "tono") return `${e.semitonos > 0 ? "+" : ""}${e.semitonos} st`;
    return "✦ " + (e.descripcion || tt("exToma"));
  }
  A.etiquetaRegion = etiqueta;
  function elRegion(r, p, carril) {
    const d = document.createElement("div");
    d.className = "ex-region ex-r-" + r.efecto.tipo + (X.regionSel === r.id ? " sel" : "") + (r.apagada ? " apagada" : "");
    d.style.left = r.desde * X.zoom + "px"; d.style.width = Math.max(4, (r.hasta - r.desde) * X.zoom) + "px";
    d.innerHTML = `<i data-z="izq"></i><span>${esc(etiqueta(r))}</span><i data-z="der"></i>`;
    d.title = `${etiqueta(r)} · ${r.desde.toFixed(2)}–${r.hasta.toFixed(2)} s${r.apagada ? " · " + tt("exApagada") : ""}`;
    d.addEventListener("mousedown", (ev) => {
      if (ev.button !== 0) return;
      ev.stopPropagation();
      const zona = ev.target.dataset.z, t0 = tDe(ev, carril);
      let [a, b] = [r.desde, r.hasta];
      arrastrar(ev, (e) => {
        const t = tDe(e, carril);
        if (zona === "izq") a = Math.min(EX.iman(t, e), r.hasta - 0.02);
        else if (zona === "der") b = Math.max(EX.iman(t, e), r.desde + 0.02);
        else { a = EX.iman(r.desde + t - t0, e); b = a + (r.hasta - r.desde); }
        d.style.left = a * X.zoom + "px"; d.style.width = Math.max(4, (b - a) * X.zoom) + "px";
      }, (e, movido) => {
        if (movido) return op({ op: "region_ajustes", id: r.id, desde: a, hasta: b });
        X.pistaSel = p.id; X.regionSel = r.id; X.clipSel = null; X.sel = { pista: p.id, desde: r.desde, hasta: r.hasta }; X.lado = "inspector"; EX.pintar();
      });
    });
    d.addEventListener("contextmenu", (ev) => {
      ev.preventDefault(); ev.stopPropagation();
      EX.menu({ getBoundingClientRect: () => ({ left: ev.clientX, bottom: ev.clientY, right: ev.clientX }) }, [
        [tt(r.apagada ? "exEncender" : "exApagar"), () => op({ op: "region_ajustes", id: r.id, apagada: !r.apagada })],
        [tt("exQuitarRegion"), () => op({ op: "quitar_region", id: r.id }), "peligro"]]);
    });
    return d;
  }

  // ── los clips ───────────────────────────────────────────────────────────
  // el nombre que se ve en un clip sin nombre propio: el de su pista, o —si su audio vino de otra— el de aquélla
  function nombreDeClip(c, p) {
    if (c.archivo === p.archivo) return p.nombre;
    const otra = est().pistas.find((x) => x.archivo === c.archivo);
    return otra ? otra.nombre : c.archivo.split("/").pop().replace(/\.[^.]+$/, "");
  }
  function elClip(c, p, carril) {
    const d = document.createElement("div"), dur = EX.durClip(c);
    d.className = "ex-clip" + (X.clipSel === c.id ? " sel" : "") + (c.silencio ? " mudo" : "");
    d.dataset.clip = c.id;
    d.style.left = c.inicio * X.zoom + "px"; d.style.width = Math.max(3, dur * X.zoom) + "px";
    const marcas = [c.ganancia_db ? `${c.ganancia_db > 0 ? "+" : ""}${c.ganancia_db} dB` : "", c.tono ? `${c.tono > 0 ? "+" : ""}${c.tono} st` : "", c.ritmo && c.ritmo !== 1 ? `×${c.ritmo}` : "", c.invertido ? "◀" : ""].filter(Boolean).join(" · ");
    d.innerHTML = `<i class="ex-c-borde" data-z="izq"></i><i class="ex-c-borde der" data-z="der"></i>
      <i class="ex-c-fundido" data-z="fe" style="left:${Math.min(dur, c.fundido_entrada || 0) * X.zoom}px" title="${esc(tt("exFundidoEntrada"))}"></i>
      <i class="ex-c-fundido" data-z="fs" style="right:${Math.min(dur, c.fundido_salida || 0) * X.zoom}px" title="${esc(tt("exFundidoSalida"))}"></i>
      <span class="ex-c-nombre">${esc(c.nombre || nombreDeClip(c, p))}${marcas ? ` <em>${esc(marcas)}</em>` : ""}</span>`;
    d.addEventListener("mousedown", (ev) => { if (ev.button === 0) enClip(ev, c, d, carril); });
    d.addEventListener("contextmenu", (ev) => { ev.preventDefault(); ev.stopPropagation(); menuClip(ev, c); });
    return d;
  }
  // la fila (pista) bajo una altura de la pantalla
  function pistaEnY(y) {
    for (const f of document.querySelectorAll("#exPistas .ex-fila[data-pista]")) { const r = f.getBoundingClientRect(); if (y >= r.top && y < r.bottom) return f; }
    return null;
  }
  function enClip(ev, c, el, carril) {
    if (X.herramienta === "rango") return; // sube al carril: arrastrar selecciona por encima del clip
    ev.stopPropagation();
    const t0 = tDe(ev, carril);
    if (X.herramienta === "tijera") return void op({ op: "partir", pista: c.pista, segundo: EX.iman(t0, ev), clip: c.id });
    const zona = ev.target.dataset.z, ini0 = c.inicio, dur0 = EX.durClip(c), desde0 = c.desde || 0, r = c.ritmo || 1, total = EX.durArchivo(c.archivo) || desde0 + dur0 * r;
    let res = null, destino = c.pista;
    const yaSel = X.clipSel === c.id;
    if (!yaSel) { document.querySelectorAll(".ex-clip.sel").forEach((x) => x.classList.remove("sel")); el.classList.add("sel"); }
    arrastrar(ev, (e) => {
      const t = tDe(e, carril);
      if (!zona) {
        const ini = EX.iman(ini0 + (t - t0), e);
        el.style.left = ini * X.zoom + "px"; res = { tipo: "mover", inicio: ini };
        const f = pistaEnY(e.clientY);
        if (f && Number(f.dataset.pista) !== destino) { destino = Number(f.dataset.pista); f.querySelector(".ex-carril").appendChild(el); }
        el.classList.add("viaja");
      } else if (zona === "izq") {
        // hacia la izquierda sólo hasta donde haya archivo; hacia la derecha, hasta dejar 20 ms
        const tope = c.invertido ? (total - (desde0 + dur0 * r)) / r : desde0 / r;
        const d = Math.min(dur0 - 0.02, Math.max(-tope, EX.iman(t, e) - ini0));
        res = { tipo: "recortar", inicio: Math.max(0, ini0 + d), desde: c.invertido ? desde0 : desde0 + d * r, duracion: dur0 - d };
        el.style.left = res.inicio * X.zoom + "px"; el.style.width = res.duracion * X.zoom + "px";
      } else if (zona === "der") {
        const tope = c.invertido ? dur0 + desde0 / r : (total - desde0) / r;
        const dur = Math.min(tope, Math.max(0.02, EX.iman(t, e) - ini0));
        res = { tipo: "recortar", inicio: ini0, desde: c.invertido ? desde0 + (dur0 - dur) * r : desde0, duracion: dur };
        el.style.width = dur * X.zoom + "px";
      } else if (zona === "fe") {
        const v = Math.min(dur0 - (c.fundido_salida || 0), Math.max(0, t - ini0));
        res = { tipo: "fundido", fundido_entrada: v }; ev.target.style.left = v * X.zoom + "px";
      } else {
        const v = Math.min(dur0 - (c.fundido_entrada || 0), Math.max(0, ini0 + dur0 - t));
        res = { tipo: "fundido", fundido_salida: v }; ev.target.style.right = v * X.zoom + "px";
      }
    }, (e, movido) => {
      X.clipSel = c.id; X.pistaSel = c.pista; X.objetivo = { tipo: "pista", id: c.pista }; X.regionSel = null;
      if (!movido || !res) { if (!yaSel) X.lado = X.lado === "ia" ? "ia" : "inspector"; return EX.pintar(); }
      if (res.tipo === "mover") {
        const o = { clip: c.id, inicio: res.inicio, pista: destino !== c.pista ? destino : null };
        // con ⌥ se deja una copia
        return op(Object.assign({ op: e.altKey ? "duplicar_clip" : "mover_clip" }, o)).then((v) => { if (!v) A.pintar(); });
      }
      if (res.tipo === "recortar") return op({ op: "recortar_clip", clip: c.id, inicio: res.inicio, desde: Math.max(0, res.desde), duracion: res.duracion }).then((v) => { if (!v) A.pintar(); });
      const o = { op: "clip_ajustes", clip: c.id };
      if (res.fundido_entrada !== undefined) o.fundido_entrada = Number(res.fundido_entrada.toFixed(4)); else o.fundido_salida = Number(res.fundido_salida.toFixed(4));
      op(o).then((v) => { if (!v) A.pintar(); });
    });
  }
  function menuClip(ev, c) {
    X.clipSel = c.id; X.pistaSel = c.pista;
    const dur = EX.durClip(c), dentro = X.pos > c.inicio && X.pos < c.inicio + dur;
    const aj = (o) => op(Object.assign({ op: "clip_ajustes", clip: c.id }, o));
    EX.menu({ getBoundingClientRect: () => ({ left: ev.clientX, bottom: ev.clientY, right: ev.clientX }) }, [
      [tt("exRehacerIA"), () => { X.sel = { pista: c.pista, desde: c.inicio, hasta: c.inicio + dur }; EX.lado.abrirIA("rehacer"); }, "ia"],
      ...(dentro ? [[`${tt("exPartirCabezal")}  ⌘E`, () => op({ op: "partir", pista: c.pista, segundo: X.pos, clip: c.id })]] : []),
      [`${tt("exDuplicar")}  ⌘D`, () => op({ op: "duplicar_clip", clip: c.id, inicio: c.inicio + dur })],
      [tt(c.silencio ? "exClipSonar" : "exClipSilenciar"), () => aj({ silencio: !c.silencio })],
      [tt(c.invertido ? "exClipDerecho" : "exClipInvertir"), () => aj({ invertido: !c.invertido })],
      [tt("exClipTramo"), () => { X.sel = { pista: c.pista, desde: c.inicio, hasta: c.inicio + dur }; EX.pintar(); }],
      "-", [`${tt("exQuitarClip")}  ⌫`, () => op({ op: "quitar_clip", clip: c.id }), "peligro"]]);
    EX.pintar();
  }
  A.partirEnCabezal = () => {
    const c = X.clipSel && EX.clip(X.clipSel);
    if (c) return op({ op: "partir", pista: c.pista, segundo: X.pos, clip: c.id });
    if (X.pistaSel) return op({ op: "partir", pista: X.pistaSel, segundo: X.pos });
    toast(tt("exElige"), 2500);
  };
  A.duplicarClip = () => { const c = X.clipSel && EX.clip(X.clipSel); if (c) op({ op: "duplicar_clip", clip: c.id, inicio: c.inicio + EX.durClip(c) }); };
  A.pegarClip = () => { const c = X.portapapeles && EX.clip(X.portapapeles); if (c) op({ op: "duplicar_clip", clip: c.id, inicio: EX.iman(X.pos), pista: X.pistaSel && X.pistaSel !== c.pista ? X.pistaSel : null }); };
  A.borrarSeleccion = () => {
    if (X.clipSel && EX.clip(X.clipSel)) return op({ op: "quitar_clip", clip: X.clipSel });
    if (X.regionSel) return op({ op: "quitar_region", id: X.regionSel });
    if (X.sel && X.sel.hasta > X.sel.desde) return op({ op: "quitar_tramo", pista: X.sel.pista, desde: X.sel.desde, hasta: X.sel.hasta, cerrar: false }).then((v) => { if (v) { X.sel = null; EX.pintar(); } });
  };

  // ── la selección de un tramo ───────────────────────────────────────────
  function enCarril(ev, carril, id) {
    const a = EX.iman(tDe(ev, carril), ev);
    arrastrar(ev, (e) => {
      const b = EX.iman(tDe(e, carril), e);
      X.pistaSel = id; X.regionSel = null; X.clipSel = null;
      X.sel = { pista: id, desde: Math.min(a, b), hasta: Math.max(a, b) };
      pintarSeleccion(carril);
    }, (e, movido) => {
      X.pistaSel = id; X.objetivo = { tipo: "pista", id };
      if (!movido || !X.sel || X.sel.hasta - X.sel.desde < 0.01) { X.sel = null; X.clipSel = null; X.regionSel = null; EX.irA(a); }
      EX.pintar();
    });
  }
  function pintarSeleccion(carril) {
    document.querySelectorAll(".ex-sel").forEach((s) => s.parentElement !== carril && s.remove());
    let s = carril.querySelector(".ex-sel");
    if (!s) { s = document.createElement("div"); s.className = "ex-sel"; carril.appendChild(s); }
    s.style.left = X.sel.desde * X.zoom + "px"; s.style.width = Math.max(2, (X.sel.hasta - X.sel.desde) * X.zoom) + "px";
  }

  // ── la automatización ──────────────────────────────────────────────────
  const RANGO = { volumen: [-36, 12], pan: [-1, 1] };
  function capaAuto(carril, p, par, w) {
    const h = X.alto - 1, [min, max] = RANGO[par], NS = "http://www.w3.org/2000/svg";
    const y = (v) => h * (1 - (Math.min(max, Math.max(min, v)) - min) / (max - min)), deY = (py) => min + (1 - Math.min(1, Math.max(0, py / h))) * (max - min);
    const a = EX.curvaDe(p.id, par);
    let pts = a ? a.puntos.map((q) => ({ t: q.t, v: q.v })) : [];
    const svg = document.createElementNS(NS, "svg");
    svg.setAttribute("class", "ex-auto ex-auto-" + par); svg.setAttribute("width", w); svg.setAttribute("height", h);
    const base = par === "volumen" ? 0 : p.pan;
    const dibujar = () => {
      const orden = pts.slice().sort((u, v) => u.t - v.t);
      const linea = orden.length ? `M0,${y(orden[0].v)} ` + orden.map((q) => `L${q.t * X.zoom},${y(q.v)}`).join(" ") + ` L${w},${y(orden[orden.length - 1].v)}` : `M0,${y(base)} L${w},${y(base)}`;
      svg.innerHTML = `<line class="ex-auto-cero" x1="0" x2="${w}" y1="${y(par === "volumen" ? 0 : 0)}" y2="${y(0)}"/><path class="ex-auto-linea${orden.length ? "" : " vacia"}" d="${linea}"/>` +
        pts.map((q, i) => `<circle class="ex-auto-punto" data-i="${i}" cx="${q.t * X.zoom}" cy="${y(q.v)}" r="4.5"><title>${q.t.toFixed(2)} s · ${par === "volumen" ? EX.fmtDb(q.v) + " dB" : fmtPan(q.v)}</title></circle>`).join("") +
        (orden.length ? "" : `<text class="ex-auto-pista" x="${($("exScroll").scrollLeft || 0) + 12}" y="${Math.max(12, y(base) - 6)}">${esc(tt(par === "volumen" ? "exAutoVolumen" : "exAutoPan"))}</text>`);
    };
    const enviar = () => op({ op: "automatizar", pista: p.id, parametro: par, puntos: pts.map((q) => ({ t: Number(q.t.toFixed(4)), v: Number(q.v.toFixed(3)) })) });
    svg.addEventListener("mousedown", (ev) => {
      if (ev.button !== 0) return;
      ev.stopPropagation();
      const r = svg.getBoundingClientRect(), en = (e) => ({ t: EX.iman(Math.max(0, (e.clientX - r.left) / X.zoom), e), v: deY(e.clientY - r.top) });
      let i = ev.target.dataset && ev.target.dataset.i !== undefined ? Number(ev.target.dataset.i) : -1, nuevo = false;
      if (i < 0) { pts.push(en(ev)); i = pts.length - 1; nuevo = true; dibujar(); }
      arrastrar(ev, (e) => { pts[i] = en(e); dibujar(); }, (e, movido) => { if (movido || nuevo) enviar(); });
    });
    svg.addEventListener("dblclick", (ev) => { const i = ev.target.dataset && ev.target.dataset.i; if (i === undefined) return; ev.stopPropagation(); pts.splice(Number(i), 1); enviar(); });
    dibujar();
    return svg;
  }

  // ── dibujar las ondas (sólo lo visible) ────────────────────────────────
  function onda(g, id, xa, xb, x0, y, h, color) {
    const pk = X.picos[id], buf = X.buffers[id];
    if (!pk || !buf) return;
    const medio = y + h / 2, amp = (h / 2) * 0.94, spp = buf.sampleRate / X.zoom;
    g.fillStyle = color;
    if (spp >= pk.paso) {
      const porPx = spp / pk.paso;
      for (let x = Math.max(0, Math.floor(xa)); x < xb; x++) {
        const b0 = Math.floor((x + x0) * porPx), b1 = Math.max(b0 + 1, Math.floor((x + 1 + x0) * porPx));
        if (b0 >= pk.mn.length) break;
        let lo = 0, hi = 0;
        for (let k = b0, f = Math.min(b1, pk.mn.length); k < f; k++) { if (pk.mn[k] < lo) lo = pk.mn[k]; if (pk.mx[k] > hi) hi = pk.mx[k]; }
        g.fillRect(x, medio - hi * amp, 1, Math.max(1, (hi - lo) * amp));
      }
    } else {
      // muy de cerca: las muestras de verdad
      const a = buf.getChannelData(0), c = buf.numberOfChannels > 1 ? buf.getChannelData(1) : a;
      for (let x = Math.max(0, Math.floor(xa)); x < xb; x++) {
        const s0 = Math.floor((x + x0) * spp), s1 = Math.max(s0 + 1, Math.floor((x + 1 + x0) * spp));
        if (s0 >= buf.length) break;
        let lo = 1, hi = -1;
        for (let k = s0, f = Math.min(s1, buf.length); k < f; k++) { const v = (a[k] + c[k]) * 0.5; if (v < lo) lo = v; if (v > hi) hi = v; }
        g.fillRect(x, medio - hi * amp, 1, Math.max(1, (hi - lo) * amp));
      }
    }
  }
  A.redibujar = () => {
    const sc = $("exScroll"); if (!sc || !X.abierta || X.mini) return;
    A.pintarRegla();
    const vw = Math.max(50, sc.clientWidth - CAB), dpr = window.devicePixelRatio || 1, x0 = sc.scrollLeft, h = X.alto - 1;
    const R = rejilla(), [t0, t1] = [x0 / X.zoom, (x0 + vw) / X.zoom];
    const cLinea = css("--line", "#333"), cLinea2 = css("--line-2", "#444"), cMudo = css("--faint", "#666");
    for (const f of document.querySelectorAll("#exPistas .ex-fila")) {
      const cv = f.querySelector("canvas.ex-onda"), id = Number(f.dataset.pista);
      if (!cv) continue;
      const p = EX.pista(id);
      if (cv.width !== Math.round(vw * dpr) || cv.height !== Math.round(h * dpr)) { cv.width = Math.round(vw * dpr); cv.height = Math.round(h * dpr); cv.style.width = vw + "px"; cv.style.height = h + "px"; }
      cv.style.left = CAB + "px";
      const g = cv.getContext("2d");
      g.setTransform(dpr, 0, 0, dpr, 0, 0); g.clearRect(0, 0, vw, h);
      // la rejilla: compases y, si caben, pulsos
      for (let k = Math.max(0, Math.floor(t0 / R.compas)); k * R.compas <= t1; k++) {
        const x = Math.round(k * R.compas * X.zoom - x0);
        if (k % R.lineaCompas === 0) { g.fillStyle = k % R.cada === 0 ? cLinea2 : cLinea; g.fillRect(x, 0, 1, h); }
        if (R.pulsos) { g.fillStyle = cLinea; g.globalAlpha = 0.5; for (let b = 1; b < R.n; b++) g.fillRect(Math.round(x + b * R.p * X.zoom), 0, 1, h); g.globalAlpha = 1; }
      }
      if (!p) continue;
      const color = EX.colorDe(p), apagada = !EX.suena(p);
      for (const c of EX.clipsDe(id)) {
        const xa = c.inicio * X.zoom - x0, dur = EX.durClip(c), xb = xa + dur * X.zoom;
        if (xb < 0 || xa > vw) continue;
        g.globalAlpha = c.silencio ? 0.35 : 1;
        onda(g, id, Math.max(0, xa), Math.min(vw, xb), x0, FRANJA, h - FRANJA - 2, apagada || c.silencio ? cMudo : color);
        g.globalAlpha = 1;
        // los fundidos: la rampa, dibujada
        g.strokeStyle = css("--text", "#fff"); g.globalAlpha = 0.55; g.lineWidth = 1;
        const fe = (c.fundido_entrada || 0) * X.zoom, fsal = (c.fundido_salida || 0) * X.zoom;
        if (fe > 2) { g.beginPath(); g.moveTo(xa, h - 2); g.quadraticCurveTo(xa + fe * 0.4, FRANJA + 2, xa + fe, FRANJA + 2); g.stroke(); }
        if (fsal > 2) { g.beginPath(); g.moveTo(xb, h - 2); g.quadraticCurveTo(xb - fsal * 0.4, FRANJA + 2, xb - fsal, FRANJA + 2); g.stroke(); }
        g.globalAlpha = 1;
      }
    }
    // el rótulo de una curva vacía acompaña al desplazamiento
    document.querySelectorAll(".ex-auto-pista").forEach((t) => t.setAttribute("x", x0 + 12));
  };

  // ── el cabezal y los medidores ─────────────────────────────────────────
  A.cabezal = () => {
    const c = $("exCabezal"), sc = $("exScroll"); if (!c || !sc) return;
    c.style.transform = `translateX(${CAB + X.pos * X.zoom}px)`;
    c.style.height = Math.max(sc.clientHeight, sc.scrollHeight) + "px";
    if (X.sonando) A.seguir(false);
  };
  // que el cabezal no se salga de la vista: al reproducir pasa página; al saltar, se centra
  A.seguir = (salto) => {
    const sc = $("exScroll"); if (!sc) return;
    const x = X.pos * X.zoom, vw = sc.clientWidth - CAB;
    if (x < sc.scrollLeft || x > sc.scrollLeft + vw - 24) sc.scrollLeft = Math.max(0, salto ? x - vw / 3 : x - 40);
  };
  A.medidores = () => {
    for (const el of document.querySelectorAll("#exPistas .ex-vu")) {
      const n = EX.niveles[el.dataset.vu]; if (!n) continue;
      el.firstElementChild.style.width = EX.aMedidor(n.v) + "%";
      el.lastElementChild.style.left = EX.aMedidor(n.max) + "%";
      el.classList.toggle("tope", n.max >= 1);
    }
  };
})();
