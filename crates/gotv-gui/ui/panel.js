// F8.0 · El armazón de la ventana. Rust decide la estructura (`ventana_estructura`: tres zonas, las pestañas
// del panel de trabajo y las de Configuración con sus secciones); aquí sólo se PINTA y se reparten los huecos.
//
// Cómo se monta una pestaña o sección nueva (F16, F18 y quien venga), sin tocar este archivo:
//   gotvPanel.registrar("archivo", { montar(nodo) {…}, mostrar(nodo) {…}, ancho: 520 });
//   gotvAjustes.registrar("conex/dispositivos", (nodo) => {…});   // sección de una pestaña
//   gotvAjustes.registrar("automatizaciones", (nodo) => {…});      // pestaña entera
// El id debe existir en la estructura de Rust (`crates/gotv-gui/src/ventana.rs`). Una pestaña o sección sin
// quien la pinte no se enseña.
(() => {
  const inv = (c, a) => window.__TAURI__.core.invoke(c, a);
  const $ = (id) => document.getElementById(id);
  let E = null;
  const pintores = {}, montados = {};
  const ajustes = {};
  let actual = "tareas";
  const panel = $("tasksPanel"), barra = $("panelTabs"), cuerpo = $("panelBody");

  // ── el panel de trabajo (zona derecha) ────────────────────────────────────
  function visibles() { return (E ? E.panel : []).filter((p) => pintores[p.id]); }
  function pintarBarra() {
    if (!barra) return;
    barra.innerHTML = "";
    for (const p of visibles()) {
      const b = document.createElement("button");
      b.type = "button"; b.className = "ptab" + (p.id === actual ? " on" : "");
      b.setAttribute("role", "tab"); b.setAttribute("aria-selected", String(p.id === actual));
      b.dataset.tab = p.id; b.textContent = tt(p.i18n);
      b.onclick = () => mostrar(p.id);
      barra.appendChild(b);
    }
    const pop = $("popTasks");
    if (pop) pop.hidden = actual !== "tareas";
  }
  function nodoDe(id) {
    let n = cuerpo.querySelector(`.panel-pane[data-pane="${id}"]`);
    if (!n) { n = document.createElement("div"); n.className = "panel-pane"; n.dataset.pane = id; cuerpo.appendChild(n); }
    return n;
  }
  function mostrar(id) {
    if (!pintores[id]) return;
    actual = id;
    const n = nodoDe(id);
    if (!montados[id]) { montados[id] = true; try { pintores[id].montar && pintores[id].montar(n); } catch (e) { console.error(id, e); } }
    cuerpo.querySelectorAll(".panel-pane").forEach((x) => (x.hidden = x !== n));
    panel.style.setProperty("--panel-w", (pintores[id].ancho || 320) + "px");
    try { pintores[id].mostrar && pintores[id].mostrar(n); } catch (e) { console.error(id, e); }
    pintarBarra();
  }
  window.gotvPanel = {
    registrar(id, p) { pintores[id] = p || {}; if (E) pintarBarra(); },
    abrir(id) { panel.classList.add("open"); mostrar(id || actual); },
    cerrar() { panel.classList.remove("open"); },
    abierto: () => panel.classList.contains("open"),
    actual: () => actual,
    nodo: (id) => nodoDe(id),
  };
  // Tareas: el de siempre (tasks-view.js lo dibuja en #taskList)
  gotvPanel.registrar("tareas", { montar(n) { const l = $("taskList"); if (l && l.parentElement !== n) n.appendChild(l); }, ancho: 300 });
  // el botón de la cabecera abre el panel en la pestaña que estaba
  const tb = $("tasksBtn");
  if (tb) tb.onclick = () => (gotvPanel.abierto() ? gotvPanel.cerrar() : gotvPanel.abrir());

  // ── Configuración: grupos → páginas → piezas (como los ajustes de Claude Desktop) ──────────────────────
  // Rust decide la barra (`GRUPOS` en ventana.rs): un título pequeño por grupo y, debajo, sus páginas. Cada
  // página pinta sus piezas una debajo de otra; cada pieza es la clave de siempre («model», «conex/ssh»…), con
  // su pintor registrado. Aquí sólo se pinta: la barra, el buscador y la página elegida.
  // lo de antes se registra con su pintor de siempre (funciones de app.js)
  const base = {
    library: (n) => drawLibrary(n), gen: (n) => drawGen(n), api: (n) => drawApi(n),
    model: (n) => paneBase("model", n), context: (n) => paneBase("context", n), prov: (n) => paneBase("prov", n),
    perms: (n) => paneBase("perms", n), pasos: (n) => paneBase("pasos", n),
    "conex/ssh": (n) => drawConexiones(n), "conex/bases": (n) => drawBases(n), "conex/conectores": (n) => drawConectores(n), "conex/buscadores": (n) => drawBuscadores(n),
    "general/aspecto": (n) => paneBase("general", n),
    // pulido: la «Caja de arena y acceso» de antes, partida en sus páginas (Seguridad)
    "seguridad/caja": (n) => drawSecurity(n, "caja"), "seguridad/chrome": (n) => drawSecurity(n, "chrome"),
    "seguridad/mac": (n) => drawSecurity(n, "mac"), "seguridad/sistema": (n) => drawSecurity(n, "sistema"),
  };
  for (const [k, f] of Object.entries(base)) ajustes[k] = f;
  // el nombre de una pieza (para el subtítulo cuando una página tiene varias)
  function nombreDe(clave) {
    const [p, s] = clave.split("/");
    const h = s ? ((E.secciones[p] || []).find((x) => x.id === s)) : E.ajustes.find((a) => a.id === p);
    return h ? tt(h.i18n) : clave;
  }
  const piezasVivas = (pg) => pg.piezas.filter((k) => ajustes[k]);
  const grupos = () => (E && E.grupos || []).map((g) => ({ ...g, paginas: g.paginas.filter((pg) => piezasVivas(pg).length) })).filter((g) => g.paginas.length);
  // «model», «seguridad», «conex/dispositivos»…: la página con ese id, o la primera que tenga esa pieza
  function paginaDe(clave) {
    const todas = grupos().flatMap((g) => g.paginas);
    return todas.find((pg) => pg.id === clave) || todas.find((pg) => pg.piezas.includes(clave)) || todas.find((pg) => pg.piezas.some((k) => k.startsWith(clave + "/"))) || todas[0];
  }
  const sinTildes = (s) => String(s).normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();
  // el texto que se lee de una clave (sin etiquetas ni marcadores {n}), para buscar en él
  const leido = (k) => (k ? tt(k).replace(/<[^>]*>/g, " ").replace(/\{[a-z]+\}/g, " ") : "");
  const casa = (texto, palabras) => { const t = sinTildes(texto); return palabras.every((p) => t.includes(p)); };
  let filtro = "", elegida = null;
  // ── el buscador: con algo escrito, la barra lista lo que casa. Primero los AJUSTES cuyo nombre casa (cada
  // fila de `FILAS` en ventana.rs, con su página debajo), luego las páginas, y al final los ajustes que casan
  // sólo por su explicación. Elegir un ajuste abre su página y lleva a la fila, resaltándola un momento.
  function resultados(q) {
    const palabras = q.split(/\s+/).filter(Boolean);
    const vivas = new Map();
    for (const g of grupos()) for (const pg of g.paginas) vivas.set(pg.id, { g, pg });
    const filas = (E.filas || []).filter((f) => vivas.has(f.pagina));
    const donde = (id) => { const { g, pg } = vivas.get(id); return tt(g.i18n) === tt(pg.i18n) ? tt(pg.i18n) : `${tt(g.i18n)} › ${tt(pg.i18n)}`; };
    const porNombre = filas.filter((f) => casa(leido(f.i18n), palabras));
    const porNota = filas.filter((f) => !porNombre.includes(f) && casa(leido(f.nota), palabras));
    const paginas = [...vivas.values()].filter(({ g, pg }) => casa([tt(g.i18n), tt(pg.i18n), ...piezasVivas(pg).map(nombreDe)].join(" "), palabras));
    const fila = (f) => ({ nombre: leido(f.i18n).trim(), donde: donde(f.pagina), pagina: f.pagina, fila: f.id });
    return [...porNombre.map(fila), ...paginas.map(({ pg }) => ({ nombre: tt(pg.i18n), donde: tt(vivas.get(pg.id).g.i18n), pagina: pg.id, fila: null })), ...porNota.map(fila)].slice(0, 40);
  }
  function pintarResultados(nav, q) {
    for (const r of resultados(q)) {
      const b = document.createElement("button");
      b.type = "button"; b.className = "set-res" + ((r.fila ? r.fila === elegida : !elegida && aLaVista && aLaVista.pg.id === r.pagina) ? " on" : "");
      b.dataset.p = r.pagina; if (r.fila) b.dataset.fila = r.fila;
      const n = document.createElement("span"); n.className = "r-nom"; n.textContent = r.nombre;
      const d = document.createElement("span"); d.className = "r-donde"; d.textContent = r.fila ? r.donde : tt("setPagina");
      b.append(n, d);
      b.onclick = () => irA(r.pagina, r.fila);
      nav.appendChild(b);
    }
  }
  // abre la página y, si se eligió un ajuste, espera a que su pintor (que suele ir a Rust) lo ponga y lleva a él
  function irA(pagina, fila) {
    elegida = fila || null;
    showPane(pagina);
    if (!fila || !aLaVista) return;
    const pane = aLaVista.pane, t0 = performance.now();
    const paso = () => {
      const f = pane.querySelector(`[data-fila="${CSS.escape(fila)}"]`);
      if (f) {
        f.scrollIntoView({ block: "center", behavior: "smooth" });
        f.classList.remove("fila-luz"); void f.offsetWidth; f.classList.add("fila-luz");
        setTimeout(() => f.classList.remove("fila-luz"), 2200);
        return;
      }
      // una fila que esta Mac no enseña (p. ej. el esfuerzo sin un modelo de Claude): basta con la página
      if (performance.now() - t0 < 4000 && pane.isConnected) requestAnimationFrame(paso);
    };
    requestAnimationFrame(paso);
  }
  function pintarNav(sel) {
    const nav = $("setNav");
    if (!nav || !E) return;
    nav.innerHTML = "";
    const q = sinTildes(filtro.trim());
    nav.classList.toggle("buscando", !!q);
    if (q) pintarResultados(nav, q);
    else for (const g of grupos()) {
      const ps = g.paginas;
      if (!ps.length) continue;
      // el título del grupo, salvo si sólo repetiría el nombre de su única página («General» → «General»)
      if (!(g.paginas.length === 1 && tt(g.paginas[0].i18n) === tt(g.i18n))) {
        const t = document.createElement("div");
        t.className = "set-grupo"; t.textContent = tt(g.i18n);
        nav.appendChild(t);
      }
      for (const pg of ps) {
        const b = document.createElement("button");
        b.type = "button"; b.dataset.p = pg.id; b.textContent = tt(pg.i18n);
        if (pg.id === sel) { b.className = "on"; b.setAttribute("aria-current", "page"); }
        b.onclick = () => irA(pg.id, null);
        nav.appendChild(b);
      }
    }
    if (!nav.children.length) { const v = document.createElement("div"); v.className = "set-nada"; v.textContent = tt("setBuscarNada"); nav.appendChild(v); }
  }
  // el buscador de la barra: Intro abre el primer resultado; ↓ y ↑ recorren la lista; Esc la vacía
  const caja = $("setBuscar");
  if (caja) {
    caja.oninput = () => { filtro = caja.value; elegida = null; pintarNav(aLaVista && aLaVista.pg && aLaVista.pg.id); };
    caja.onkeydown = (e) => {
      if (e.key === "Enter") { const b = $("setNav").querySelector("button"); if (b) b.click(); }
      if (e.key === "ArrowDown") { const b = $("setNav").querySelector("button"); if (b) { e.preventDefault(); b.focus(); } }
      if (e.key === "Escape" && caja.value) { e.stopPropagation(); caja.value = ""; caja.oninput(); }
    };
    const nav = $("setNav");
    if (nav) nav.addEventListener("keydown", (e) => {
      if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
      const bs = [...nav.querySelectorAll("button")], i = bs.indexOf(document.activeElement);
      if (i < 0) return;
      e.preventDefault();
      if (e.key === "ArrowUp" && i === 0) caja.focus();
      else (bs[i + (e.key === "ArrowDown" ? 1 : -1)] || bs[i]).focus();
    });
  }
  // la página de Configuración que está a la vista (para repintarla si una pieza llega tarde)
  let aLaVista = null;
  window.gotvAjustes = {
    // F16: una pieza que se registra TARDE (carga lenta) con su página ya abierta, la repinta: si no, esa
    // pieza no salía hasta cerrar y abrir Configuración
    registrar(clave, pintar) {
      ajustes[clave] = pintar;
      const modal = $("settingsModal");
      if (aLaVista && aLaVista.pane.isConnected && modal && modal.classList.contains("show")) {
        const pg = paginaDe(aLaVista.clave);
        if (pg && pg.piezas.includes(clave)) gotvAjustes.pintar(aLaVista.clave, aLaVista.pane);
        else pintarNav(aLaVista.pg && aLaVista.pg.id);
      }
    },
    // pinta en `pane` la página que corresponde a `clave`; false = la estructura aún no llegó
    pintar(clave, pane) {
      if (!E) return false;
      const pg = paginaDe(clave);
      if (!pg) return false;
      aLaVista = { clave: pg.id, pg, pane };
      // la barra se repinta cada vez: así sigue el idioma si se cambió en General
      pintarNav(pg.id);
      const card = pane.closest(".card.settings");
      if (card) card.classList.toggle("wide", pg.piezas.includes("library") || pg.piezas.includes("gen"));
      const piezas = piezasVivas(pg);
      pane.innerHTML = `<header class="set-cab"><h2>${esc(tt(pg.i18n))}</h2></header>`;
      pane.scrollTop = 0;
      for (const k of piezas) {
        if (piezas.length > 1) { const h = document.createElement("h3"); h.className = "set-sub"; h.textContent = nombreDe(k); pane.appendChild(h); }
        const n = document.createElement("section");
        n.className = "set-pieza"; n.dataset.k = k;
        pane.appendChild(n);
        try { Promise.resolve(ajustes[k](n)).catch((e) => console.error(k, e)); } catch (e) { console.error(k, e); }
      }
      return true;
    },
    // abre Configuración en una página o pieza concreta («conex/dispositivos»)
    abrir(clave) { openSettings(clave); },
  };

  inv("ventana_estructura").then((e) => {
    E = e;
    pintarBarra(); pintarNav();
    if (!pintores[actual]) actual = "tareas";
    mostrar(actual);
  }).catch((e) => console.error("ventana_estructura", e));
})();
