// F8.1 · Zona izquierda: cada proyecto se despliega con SUS chats (Rust: `chats_lista`, `chat_*`); al pie,
// las charlas sin proyecto (F8.5), la búsqueda ⌘K (F8.6) y Automatizaciones (F2.3). En el chat: bifurcar
// desde un mensaje y citar lo seleccionado (F8.3). Aquí no hay datos ni reglas: sólo se pinta lo que Rust da.
(() => {
  const inv = (c, a) => window.__TAURI__.core.invoke(c, a);
  const abiertos = new Set();
  // qué proyectos quedan desplegados lo guarda Rust (`chats_abiertos`, formato propio), no el navegador
  inv("chats_abiertos").then((v) => {
    let cambio = false;
    for (const p of v || []) if (!abiertos.has(p)) { abiertos.add(p); cambio = true; }
    if (cambio && typeof renderProjects === "function") renderProjects();
  }).catch(() => {});
  const recordar = () => inv("chats_abiertos_guardar", { rutas: [...abiertos] }).catch(() => {});
  const listas = new Map(); // ruta del proyecto → nodo de su lista

  function menu(e, items) {
    e.preventDefault(); e.stopPropagation();
    cerrarMenu();
    const m = el("div", "ctx-menu", "");
    m.setAttribute("role", "menu");
    for (const [texto, hacer, des] of items) {
      const b = el("button", "", texto);
      b.type = "button"; b.setAttribute("role", "menuitem");
      if (des) { b.disabled = true; b.title = des; }
      b.onclick = (ev) => { ev.stopPropagation(); cerrarMenu(); hacer(); };
      m.appendChild(b);
    }
    m.onkeydown = (ev) => {
      const bs = [...m.querySelectorAll("button:not([disabled])")], i = bs.indexOf(document.activeElement);
      if (ev.key === "Escape") { ev.preventDefault(); cerrarMenu(); }
      else if (ev.key === "ArrowDown" || ev.key === "ArrowUp") { ev.preventDefault(); bs[(i + (ev.key === "ArrowDown" ? 1 : -1) + bs.length) % bs.length].focus(); }
    };
    document.body.appendChild(m);
    let x = e.clientX, y = e.clientY;
    if (!x && !y) { const r = e.currentTarget.getBoundingClientRect(); x = r.left + 12; y = r.bottom; }
    const r = m.getBoundingClientRect();
    m.style.left = Math.max(8, Math.min(x, innerWidth - r.width - 8)) + "px";
    m.style.top = Math.max(8, Math.min(y, innerHeight - r.height - 8)) + "px";
    S.ctxMenu = m;
    const f = m.querySelector("button:not([disabled])");
    if (f) f.focus();
  }

  async function exportar(proyecto, id, formato) {
    try { const d = await inv("chat_exportar", { proyecto, id, formato }); if (d) toast("✓ " + tt("chExportado") + " " + d, 5000); }
    catch (x) { toast("✗ " + x, 6000); }
  }

  function renombrar(fila, proyecto, c) {
    const n = fila.querySelector(".n");
    const i = el("input", "ren", "");
    i.value = c.title || ""; i.placeholder = tt("chTituloPh"); i.setAttribute("aria-label", tt("chTituloPh"));
    n.replaceWith(i); i.focus(); i.select();
    let hecho = false;
    const fin = async (guardar) => {
      if (hecho) return; hecho = true;
      if (guardar && i.value.trim() && i.value.trim() !== c.title) {
        try { await inv("chat_renombrar", { proyecto, id: c.id, titulo: i.value }); } catch (x) { toast("✗ " + x, 5000); }
      }
      cargar(proyecto);
    };
    i.onkeydown = (e) => { e.stopPropagation(); if (e.key === "Enter") fin(true); if (e.key === "Escape") fin(false); };
    i.onclick = (e) => e.stopPropagation();
    i.onblur = () => fin(true);
  }

  // la lista de chats de un proyecto (una página; «Ver más» pide la siguiente)
  async function cargar(proyecto, desde = 0) {
    const caja = listas.get(proyecto);
    if (!caja) return;
    let r;
    try { r = await inv("chats_lista", { proyecto, desde }); } catch (x) { caja.innerHTML = `<div class="chat-nota">✗ ${esc(String(x))}</div>`; return; }
    if (desde === 0) caja.innerHTML = "";
    caja.querySelector(".chat-mas")?.remove();
    const activo = proyecto === S.root;
    if (desde === 0 && activo) {
      // F8.4: las carpetas extra del proyecto (las elige el Portador; quitarlas no borra nada)
      const cs = el("div", "carpetas", "");
      caja.appendChild(cs);
      pintarCarpetas(cs);
      const n = el("button", "chat-fila nuevo", "");
      n.type = "button";
      n.innerHTML = `<span class="n">＋ ${esc(tt("chNuevo"))}</span>`;
      n.onclick = () => { if (S.busy) return toast(tt("working")); inv("chat_abrir", { proyecto, id: null }); };
      caja.appendChild(n);
    }
    for (const c of r.chats) {
      const f = el("div", "chat-fila" + (activo && c.id === r.actual ? " on" : ""), "");
      f.setAttribute("role", "button"); f.tabIndex = 0;
      f.title = `${c.title || tt("chSinTitulo")} · ${c.created}${c.parent ? ` · ${tt("chRama")} #${c.parent}` : ""}`;
      // una bifurcación ya lleva «↳ » en su título (lo pone Rust al crearla)
      f.innerHTML = `${c.parent && !String(c.title).startsWith("↳") ? '<span class="rama" aria-hidden="true">↳</span>' : ""}<span class="n">${esc(c.title || tt("chSinTitulo"))}</span><button class="mas" type="button" aria-label="${esc(tt("chMenu"))}" title="${esc(tt("chMenu"))}">⋯</button>`;
      const abrir = () => {
        if (activo && c.id === r.actual) return;
        if (S.busy) return toast(tt("working"));
        inv("chat_abrir", { proyecto, id: c.id }).catch((x) => toast(String(x) === "untrusted" ? tt("trustTitle") : "✗ " + x, 5000));
      };
      f.onclick = (e) => { if (!e.target.closest(".mas")) abrir(); };
      f.onkeydown = (e) => { if ((e.key === "Enter" || e.key === " ") && e.target === f) { e.preventDefault(); abrir(); } };
      const acciones = (e) => menu(e, [
        [tt("chRenombrar"), () => renombrar(f, proyecto, c)],
        [tt("chExportarMd"), () => exportar(proyecto, c.id, "md")],
        [tt("chExportarJson"), () => exportar(proyecto, c.id, "json")],
        [tt("chPapelera"), async () => { try { await inv("chat_papelera", { proyecto, id: c.id }); cargar(proyecto); } catch (x) { toast("✗ " + x, 5000); } }],
      ]);
      f.querySelector(".mas").onclick = acciones;
      f.oncontextmenu = acciones;
      caja.appendChild(f);
    }
    if (desde + r.chats.length < r.total) {
      const m = el("button", "chat-fila chat-mas", "");
      m.type = "button"; m.innerHTML = `<span class="n">${esc(tt("chMas"))} · ${r.total - desde - r.chats.length}</span>`;
      m.onclick = () => cargar(proyecto, desde + r.chats.length);
      caja.appendChild(m);
    }
  }

  // ── F8.4 · carpetas extra del proyecto abierto ────────────────────────────
  async function pintarCarpetas(cs, lista) {
    if (!lista) { try { lista = await inv("carpetas_lista"); } catch { lista = []; } }
    cs.innerHTML = "";
    for (const c of lista) {
      const f = el("div", "carpeta", "");
      f.title = c.ruta;
      f.innerHTML = `<span class="n mono"></span><button type="button" class="x" aria-label="${esc(tt("cpQuitar"))}" title="${esc(tt("cpQuitar"))}">✕</button>`;
      f.querySelector(".n").textContent = "@" + c.nombre;
      f.querySelector(".x").onclick = async (e) => {
        e.stopPropagation();
        if (!(await confirmBox(tt("cpQuitar"), `@${c.nombre} · ${c.ruta}\n\n${tt("cpQuitarQ")}`, tt("cpQuitar")))) return;
        try { pintarCarpetas(cs, await inv("carpetas_quitar", { nombre: c.nombre })); } catch (x) { toast("✗ " + x, 6000); }
      };
      cs.appendChild(f);
    }
    const a = el("button", "chat-fila nuevo", "");
    a.type = "button"; a.title = tt("cpAyuda");
    a.innerHTML = `<span class="n">＋ ${esc(tt("cpAgregar"))}</span>`;
    a.onclick = async () => {
      const ruta = await inv("pick_folder");
      if (!ruta) return;
      const intentar = async (nombre) => {
        try { pintarCarpetas(cs, await inv("carpetas_agregar", { ruta, nombre })); toast("✓ " + tt("cpAgregada")); }
        catch (x) {
          // el nombre choca con otra (dos carpetas «api»): el Portador pone el suyo
          if (nombre == null && /nombre|name/i.test(String(x))) {
            const i = el("input", "ren", ""); i.placeholder = tt("cpNombrePh"); i.setAttribute("aria-label", tt("cpNombrePh"));
            a.replaceWith(i); i.focus();
            i.onkeydown = (e) => { e.stopPropagation(); if (e.key === "Enter" && i.value.trim()) intentar(i.value.trim()); if (e.key === "Escape") pintarCarpetas(cs); };
            toast("✗ " + x, 6000);
          } else toast("✗ " + x, 6000);
        }
      };
      intentar(null);
    };
    cs.appendChild(a);
  }

  window.gotvChats = {
    // engancha la fila de un proyecto (renderProjects de app.js): flecha para desplegar y su lista debajo
    decorar(fila, p) {
      const abierto = p.path === S.root || abiertos.has(p.path);
      const flecha = el("button", "plegar", "");
      flecha.type = "button";
      flecha.setAttribute("aria-expanded", String(abierto));
      flecha.setAttribute("aria-label", tt("chMenu"));
      flecha.innerHTML = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m9 6 6 6-6 6"/></svg>`;
      fila.prepend(flecha);
      const caja = el("div", "chats", "");
      caja.hidden = !abierto;
      listas.set(p.path, caja);
      flecha.onclick = (e) => {
        e.stopPropagation();
        const ab = caja.hidden;
        caja.hidden = !ab; flecha.setAttribute("aria-expanded", String(ab));
        if (ab) { abiertos.add(p.path); cargar(p.path); } else abiertos.delete(p.path);
        recordar();
      };
      // la lista va DEBAJO de la fila (la fila aún no está en la página)
      queueMicrotask(() => { fila.after(caja); if (abierto) cargar(p.path); });
    },
    recargar() {
      if (S.root && listas.has(S.root)) cargar(S.root);
      pintarPie();
      // una charla sin proyecto no se nombra por su carpeta (un sello de fecha): lo dice Rust
      if (S.root) inv("chats_lista", {}).then((r) => { if (r.suelta) { $("projName").textContent = tt("chNuevaCharla"); document.title = "GOD OF THE VOID — " + tt("chNuevaCharla"); } }).catch(() => {});
    },
    papelera,
  };

  // ── las charlas sin proyecto: su lista es la del modo «Chat» (arriba de la barra, como Claude Desktop) ──
  // Antes iban en un recuadro al pie que tapaba la lista de proyectos; el buscador, las automatizaciones
  // y la papelera son ahora filas de la barra (index.html), encima de crear.
  async function pintarPie() {
    let sueltas = [];
    try { sueltas = await inv("sueltas_lista"); } catch {}
    S.sueltasLista = sueltas;
    const sb = $("sueltas");
    if (!sb) return;
    sb.innerHTML = "";
    if (!sueltas.length) sb.innerHTML = `<div class="sueltas-vacio">${esc(tt("chSinCharlas"))}</div>`;
    for (const s of sueltas) {
      const f = el("button", "chat-fila" + (s.ruta === S.root ? " on" : ""), "");
      f.type = "button"; f.title = `${s.titulo} · ${s.creada}`;
      f.innerHTML = `<span class="n">${esc(s.titulo || tt("chSinTitulo"))}</span>`;
      f.onclick = () => { if (S.busy) return toast(tt("working")); inv("suelta_abrir", { ruta: s.ruta }).catch((x) => toast("✗ " + x, 5000)); };
      sb.appendChild(f);
    }
  }
  const nuevaCharla = async () => { if (S.busy) return toast(tt("working")); try { await inv("suelta_nueva"); } catch (x) { toast("✗ " + x, 5000); } };
  $("newCharla").onclick = nuevaCharla;
  $("newCharla").title = tt("chNuevaCharlaTip");
  $("sideBuscar").onclick = () => window.gotvBuscar && gotvBuscar.abrir();
  $("sideAuto").onclick = () => openSettings("automatizaciones");
  $("sidePapelera").onclick = () => papelera();
  gotvChats.nuevaCharla = nuevaCharla;
  gotvChats.charlas = pintarPie;

  // ── la papelera (de los chats del proyecto abierto) ───────────────────────
  async function papelera() {
    if (!S.root) return toast(tt("noProject"));
    let r;
    try { r = await inv("papelera_lista"); } catch (x) { return toast("✗ " + x, 5000); }
    let m = $("papeleraModal");
    if (!m) {
      m = el("div", "modal", ""); m.id = "papeleraModal";
      m.innerHTML = `<div class="card"><button class="close" type="button" aria-label="✕">✕</button><h3></h3><p class="note"></p><div class="papelera"></div><div class="actions"><button class="btn danger" id="papVaciar"></button></div></div>`;
      document.body.appendChild(m);
      m.querySelector(".close").onclick = () => m.classList.remove("show");
      m.addEventListener("mousedown", (e) => { if (e.target === m) m.classList.remove("show"); });
    }
    m.querySelector("h3").textContent = tt("chPapeleraTitulo");
    m.querySelector(".note").textContent = tt("chPapeleraNota");
    const lista = m.querySelector(".papelera");
    lista.innerHTML = r.chats.length ? "" : `<div class="chat-nota">${esc(tt("chVacia"))}</div>`;
    for (const c of r.chats) {
      const f = el("div", "pap-fila", "");
      const cuando = c.deleted_at ? new Date(c.deleted_at * 1000).toLocaleDateString(LANG === "en" ? "en-US" : "es-CO") : "";
      f.innerHTML = `<span class="n">${esc(c.title || tt("chSinTitulo"))}</span><span class="mm">${esc(cuando)}</span><button class="btn" type="button">${esc(tt("chRestaurar"))}</button>`;
      f.querySelector("button").onclick = async () => { try { await inv("chat_restaurar", { id: c.id }); papelera(); gotvChats.recargar(); } catch (x) { toast("✗ " + x, 5000); } };
      lista.appendChild(f);
    }
    const v = $("papVaciar");
    v.textContent = tt("chVaciar");
    v.onclick = async () => { try { const n = await inv("papelera_vaciar"); toast(`✓ ${tt("chVaciados")} ${n}`); papelera(); } catch (x) { toast("✗ " + x, 5000); } };
    m.classList.add("show");
  }

  // ── en el chat: bifurcar desde un mensaje y citar lo seleccionado ─────────
  const seleccionEn = (nodo) => {
    const s = window.getSelection();
    if (!s || s.isCollapsed || !nodo.contains(s.anchorNode)) return "";
    return s.toString();
  };
  async function citar(origen, texto) {
    if (!texto.trim()) return;
    try {
      const c = await inv("citar", { origen, texto });
      input.value = c + input.value;
      fit(); refreshSend(); input.focus();
    } catch (x) { toast("✗ " + x, 5000); }
  }
  window.gotvCitar = citar;
  log.addEventListener("contextmenu", (e) => {
    const m = e.target.closest(".msg-u, .msg-ans");
    if (!m || e.target.closest("a, img, video, audio, .vista, .pista")) return;
    const sel = seleccionEn(m);
    const id = S.chatId;
    const i = m.dataset.i;
    const items = [];
    if (sel) items.push([tt("chCitar"), () => citar(tt(m.classList.contains("msg-u") ? "ctTuMensaje" : "ctRespuesta"), sel)]);
    items.push([tt("chBifurcar"), async () => {
      if (S.busy) return toast(tt("working"));
      try { toast("✓ " + (await inv("chat_bifurcar", { id, hasta: Number(i) })), 7000); } catch (x) { toast("✗ " + x, 6000); }
    }, i == null || id == null ? tt("chSinBifurcar") : null]);
    menu(e, items);
  });

  // ── lo que llega del núcleo ───────────────────────────────────────────────
  window.__TAURI__.event.listen("gotv", ({ payload: m }) => {
    if (m.kind === "done") {
      // tu mensaje y la respuesta ya tienen su lugar en el historial (bifurcar sin reabrir)
      const us = log.querySelectorAll(".msg-u"), as = log.querySelectorAll(".msg-ans");
      if (m.iu != null && us.length && us[us.length - 1].dataset.i == null) us[us.length - 1].dataset.i = m.iu;
      if (m.ia != null && as.length && as[as.length - 1].dataset.i == null) as[as.length - 1].dataset.i = m.ia;
      setTimeout(() => gotvChats.recargar(), 50);
    }
    if (m.kind === "project") setTimeout(() => gotvChats.recargar(), 50);
    if (m.kind === "status" && m.status && "chat" in m.status) S.chatId = m.status.chat;
  });
  // el chat abierto (lo dice el núcleo en su estado)
  const estado = () => inv("info").then((i) => { S.chatId = i.chat ?? null; }).catch(() => {});
  window.__TAURI__.event.listen("gotv", ({ payload: m }) => { if (m.kind === "project" || m.kind === "done") estado(); });
  estado();
  // app.js pudo pintar los proyectos antes de que este archivo cargara: se repintan con sus chats
  renderProjects();
  pintarPie();
})();
