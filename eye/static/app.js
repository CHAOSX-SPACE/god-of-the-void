/* EL OJO — dashboard de CHAOS. Vanilla, cero dependencias.

   LEY DEL REFRESCO SIN PARPADEO (defecto real del Portador: "parpadeos, eso es
   mal visto"): una vista se MONTA una sola vez y luego se PARCHEA. Nunca se
   rewrites innerHTML for one new datum. If the value did not change, the DOM
   is not touched — so no repaint, no jump, no flicker. */
"use strict";

const TOKEN = new URLSearchParams(location.search).get("t") || "";
const api = (r) => fetch(r + (r.includes("?") ? "&" : "?") + "t=" + TOKEN)
  .then(x => { if (!x.ok) throw new Error(x.status); return x.json(); });

/* THE FIST — the only way the Eye writes. POST + a header of its own: no
   <img>, link or foreign form can fire an action. */
const actuar = (accion, arg) => fetch("/api/accion", {
  method: "POST",
  headers: { "X-Ojo-Accion": "1", "Content-Type": "application/json" },
  body: JSON.stringify({ accion, arg }),
}).then(r => r.json());

async function obrar(accion, arg, etiqueta) {
  const r = await actuar(accion, arg);
  if (!r.ok) { alert(t("accion_fallo") + ": " + (r.error || "?")); return false; }
  await refrescar();
  return true;
}

let I18N = {}, IDIOMA = localStorage.getItem("ojo_idioma") || "es";
const t = (k) => I18N[k] || k;
const esc = (s) => String(s ?? "").replace(/[&<>"]/g,
  c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

/* SURGICAL PATCH: writes only if it differs. Zero free repaints. */
const setTxt = (el, v) => { if (el && el.textContent !== String(v)) el.textContent = v; };
const setCls = (el, c, on) => { if (el && el.classList.contains(c) !== !!on) el.classList.toggle(c, !!on); };

/* SSE pulse state: its own variable, NEVER reset by the i18n
   (falla real: recargar idioma dejaba "conectando…" para siempre) */
let PULSO_VIVO = false;
function pintarPulso() {
  setCls(document.querySelector(".punto-vivo"), "muerto", !PULSO_VIVO);
  setTxt(document.getElementById("estado-sse"), PULSO_VIVO ? t("en_vivo") : t("reconectando"));
}

/* ── the void with life (root layer; never behind the text) ── */
(function vacio() {
  const cv = document.getElementById("vacio"), ctx = cv.getContext("2d");
  const quieto = matchMedia("(prefers-reduced-motion: reduce)").matches;
  let W, H, es = [], ang = 0;
  const medir = () => {
    W = cv.width = innerWidth; H = cv.height = innerHeight;
    es = Array.from({ length: 110 }, () => ({
      x: Math.random() * W, y: Math.random() * H, r: Math.random() * 1.3 + .3,
      o: Math.random() * .13 + .05, v: Math.random() * .018 + .004
    }));
  };
  const pintar = () => {
    ctx.clearRect(0, 0, W, H);
    for (const e of es) {
      ctx.globalAlpha = e.o; ctx.fillStyle = "#F2F5FB";
      ctx.beginPath(); ctx.arc(e.x, e.y, e.r, 0, 7); ctx.fill();
      if (!quieto) { e.x += e.v; if (e.x > W + 2) e.x = -2; }
    }
    ctx.globalAlpha = .045; ctx.strokeStyle = "#8A7CF7"; ctx.lineWidth = 1.2;
    ctx.save(); ctx.translate(W * .82, H * .18); ctx.rotate(ang);
    ctx.beginPath(); ctx.ellipse(0, 0, 190, 58, .5, 0, 7); ctx.stroke(); ctx.restore();
    ctx.globalAlpha = 1;
    if (!quieto) { ang += (Math.PI * 2) / 3600; requestAnimationFrame(pintar); }
  };
  addEventListener("resize", () => { medir(); if (quieto) pintar(); });
  medir(); pintar();
})();

/* ── THE DRAWER: detail of anything, without leaving the view ── */
const cajon = {
  abrir(titulo, html) {
    let c = document.getElementById("cajon");
    if (!c) {
      c = document.createElement("aside");
      c.id = "cajon"; c.className = "cajon";
      c.innerHTML = `<div class="cajon-cab"><h2 id="cajon-tit"></h2>
        <button class="btn-fantasma" id="cajon-x" aria-label="cerrar">✕</button></div>
        <div class="cajon-cuerpo" id="cajon-cuerpo"></div>`;
      document.body.appendChild(c);
      c.querySelector("#cajon-x").onclick = () => cajon.cerrar();
      addEventListener("keydown", e => { if (e.key === "Escape") cajon.cerrar(); });
    }
    c.querySelector("#cajon-tit").textContent = titulo;
    c.querySelector("#cajon-cuerpo").innerHTML = html;
    void c.offsetWidth;          /* forced reflow: the transition starts cleanly */
    c.classList.add("abierto");  /* NO requestAnimationFrame: no corre en
                                    background tab and the drawer never opened */
    c.querySelector("#cajon-x").focus();
  },
  cerrar() { document.getElementById("cajon")?.classList.remove("abierto"); },
  cargando(titulo) { this.abrir(titulo, `<p class="vacio-msg">…</p>`); },
};

/* ── vistas ── */
const ICONOS = {
  constelacion: '<svg viewBox="0 0 24 24"><circle cx="5" cy="6" r="2"/><circle cx="19" cy="5" r="2"/><circle cx="12" cy="19" r="2"/><path d="M6.8 7.2 11 17m2-.5 4.7-9.7M7 6.4l10-1"/></svg>',
  errario: '<svg viewBox="0 0 24 24"><path d="M12 3l9 16H3z"/><path d="M12 10v4m0 3v.5"/></svg>',
  territorios: '<svg viewBox="0 0 24 24"><path d="M3 7l6-3 6 3 6-3v13l-6 3-6-3-6 3z"/><path d="M9 4v13m6-10v13"/></svg>',
  grafo: '<svg viewBox="0 0 24 24"><circle cx="6" cy="7" r="2.5"/><circle cx="18" cy="7" r="2.5"/><circle cx="12" cy="18" r="2.5"/><path d="M8.5 7h7M7.3 9.2l3.5 6.8m6-6.8-3.5 6.8"/></svg>',
  tiempo: '<svg viewBox="0 0 24 24"><path d="M12 3v18"/><circle cx="12" cy="7" r="2"/><circle cx="12" cy="14" r="2"/><path d="M14 7h6M14 14h4"/></svg>',
  actos: '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3.5 2"/></svg>',
  buscar: '<svg viewBox="0 0 24 24"><circle cx="10.5" cy="10.5" r="6.5"/><path d="m15.5 15.5 5 5"/></svg>',
  notas: '<svg viewBox="0 0 24 24"><path d="M12 2.5 14 9l6.5 2-6.5 2-2 6.5-2-6.5L3.5 11 10 9z"/></svg>',
  salud: '<svg viewBox="0 0 24 24"><path d="M3 12h4l2.5-6 4 12L16 12h5"/></svg>',
};
const VISTAS = ["constelacion", "errario", "territorios", "notas", "grafo", "tiempo", "actos", "buscar", "salud"];
let vistaActual = "constelacion";

function armarMenu(pulso) {
  const m = document.getElementById("menu");
  if (m.children.length !== VISTAS.length) {
    m.innerHTML = "";
    for (const v of VISTAS) {
      const b = document.createElement("button");
      b.dataset.v = v;
      b.innerHTML = ICONOS[v] + `<span class="etiq"></span><span class="cifra"></span>`;
      b.onclick = () => ir(v);
      m.appendChild(b);
    }
  }
  for (const b of m.children) {
    setTxt(b.querySelector(".etiq"), t(b.dataset.v));
    setCls(b, "activo", b.dataset.v === vistaActual);
    if (b.dataset.v === "errario" && pulso) {
      setTxt(b.querySelector(".cifra"), pulso.fallas_vivas || "");
      setCls(b, "alerta", pulso.fallas_vivas > 0);
    }
  }
}

/* === THE CONSTELLATION === */
const KPIS = [
  ["esencias", "esencias"], ["residentes", "residentes", "orbita"],
  ["enlaces", "enlaces", "", "grafo"], ["rotos", "rotos", "malo", "grafo"],
  ["huerfanas", "huerfanas", "malo"], ["chispas", "chispas"],
  ["hambres", "hambres", "malo"], ["fallas_vivas", "fallas", "malo", "errario"],
  ["reincidencias", "reincidencias", "malo", "errario"],
  ["actos", "actos_kpi", "orbita", "actos"],
];
const vConstelacion = {
  async montar(el) {
    el.innerHTML = `<div class="grilla">${KPIS.map(([k, , cl, v]) => `
      <div class="tarjeta ${v ? "clic" : ""}" ${v ? `data-ir="${v}" role="button" tabindex="0"` : ""}>
        <div class="kpi ${cl || ""}" data-val="${k}">·</div>
        <div class="kpi-nombre" data-nom="${k}"></div></div>`).join("")}
      </div><p class="pie-nota"><span data-nom="ultimo"></span>
      <span class="mono" data-val="ultimo_acto"></span></p>`;
    el.querySelectorAll("[data-ir]").forEach(x => {
      const f = () => ir(x.dataset.ir);
      x.onclick = f;
      x.onkeydown = e => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); f(); } };
    });
    await this.refrescar(el);
  },
  async refrescar(el) {
    const p = await api("/api/pulso");
    armarMenu(p);
    for (const [k, nom, cl] of KPIS) {
      const v = el.querySelector(`[data-val="${k}"]`);
      setTxt(v, k === "fallas_vivas" ? `${p.fallas_vivas} / ${p.fallas_total}` : (p[k] ?? 0));
      setTxt(el.querySelector(`[data-nom="${k}"]`), t(nom));
      if (cl === "malo") { setCls(v, "malo", p[k] > 0); setCls(v, "bueno", !p[k]); }
    }
    setTxt(el.querySelector('[data-nom="ultimo"]'), t("ultimo_latido") + ": ");
    setTxt(el.querySelector('[data-val="ultimo_acto"]'), p.ultimo_acto || "—");
  },
};

/* ═══ ERRARIO ═══ */
const vErrario = {
  datos: [],
  async montar(el) {
    el.innerHTML = `
      <div class="filtros">
        <input id="f-q" type="search" placeholder="${esc(t("filtrar_fallas"))}" aria-label="${esc(t("filtrar_fallas"))}">
        <select id="f-est"><option value="">${esc(t("todas"))}</option>
          <option value="viva">${esc(t("vivas"))}</option>
          <option value="curada">${esc(t("curadas"))}</option></select>
        <select id="f-ter"><option value="">${esc(t("todo_territorio"))}</option></select>
      </div><div id="f-lista"></div>`;
    el.querySelector("#f-q").addEventListener("input", () => this.pintar(el));
    el.querySelector("#f-est").addEventListener("change", () => this.pintar(el));
    el.querySelector("#f-ter").addEventListener("change", () => this.pintar(el));
    await this.refrescar(el);
  },
  async refrescar(el) {
    this.datos = await api("/api/fallas");
    const sel = el.querySelector("#f-ter"), prev = sel.value;
    const ters = [...new Set(this.datos.map(f => f.territorio || f.territory).filter(Boolean))].sort();
    if (sel.options.length !== ters.length + 1) {
      sel.innerHTML = `<option value="">${esc(t("todo_territorio"))}</option>` +
        ters.map(x => `<option>${esc(x)}</option>`).join("");
      sel.value = prev;
    }
    this.pintar(el);
  },
  pintar(el) {
    const q = el.querySelector("#f-q").value.toLowerCase();
    const est = el.querySelector("#f-est").value, ter = el.querySelector("#f-ter").value;
    const lista = el.querySelector("#f-lista"), vivos = new Set();
    const fs = this.datos.filter(f => {
      const e = f.estado || f.state, tr = f.territorio || f.territory || "";
      const saco = [f.titulo || f.title, f.causa || f.cause, f.cura || f.cure,
        f.leccion || f.lesson, f.sintoma || f.symptom].join(" ").toLowerCase();
      const esViva = e === "viva" || e === "alive";
      return (!q || saco.includes(q)) && (!ter || tr === ter) &&
        (!est || (est === "viva") === esViva);
    });
    const msg = lista.querySelector(".vacio-msg"); if (msg) msg.remove();
    for (const f of fs) {
      const id = "f" + f.id; vivos.add(id);
      let d = lista.querySelector(`[data-id="${id}"]`);
      const viva = (f.estado || f.state) === "viva" || (f.estado || f.state) === "alive";
      const rep = parseInt(f.repeticiones || f.repeats || 0, 10);
      if (!d) {
        d = document.createElement("article");
        d.className = "falla"; d.dataset.id = id; d.tabIndex = 0;
        d.innerHTML = `<div class="falla-cab">
            <span class="chip est"></span><span class="chip reinc" hidden></span>
            <span class="falla-titulo"></span><span class="chip ter"></span>
            <span class="chip mono fec"></span></div><div class="falla-cuerpo"></div>`;
        const abrir = () => d.classList.toggle("abierta");
        d.onclick = abrir;
        d.onkeydown = e => { if (e.key === "Enter") { e.preventDefault(); abrir(); } };
        lista.appendChild(d);
      }
      const c = d.querySelector(".chip.est");
      setTxt(c, viva ? "● " + t("viva") : "✓ " + t("curada"));
      setCls(c, "viva", viva); setCls(c, "curada", !viva);
      const r = d.querySelector(".chip.reinc");
      r.hidden = !rep; if (rep) setTxt(r, `↻ ${t("reincidida")} ×${rep}`);
      /* a living fault is cured FROM HERE - through the CLI, never through SQL */
      let acc = d.querySelector(".falla-acc");
      if (viva && !acc) {
        acc = document.createElement("button");
        acc.className = "btn-fantasma falla-acc";
        acc.onclick = (ev) => { ev.stopPropagation(); obrar("curar_falla", f.id); };
        d.querySelector(".falla-cab").appendChild(acc);
      }
      if (acc) { acc.hidden = !viva; setTxt(acc, "🩹 " + t("curar")); }
      setTxt(d.querySelector(".falla-titulo"), `#${f.id} ${f.titulo || f.title}`);
      setTxt(d.querySelector(".chip.ter"), f.territorio || f.territory || "?");
      setTxt(d.querySelector(".chip.fec"), f.fecha || f.date || "");
      const campos = [["sintoma", f.sintoma || f.symptom], ["causa", f.causa || f.cause],
        ["solucion", f.cura || f.cure, "solucion"], ["leccion", f.leccion || f.lesson, "leccion"]];
      const html = campos.filter(x => x[1]).map(([k, v, cl]) =>
        `<div class="campo ${cl || ""}"><b>${esc(t(k))}</b><span>${esc(v)}</span></div>`).join("");
      const cu = d.querySelector(".falla-cuerpo");
      if (cu.innerHTML !== html) cu.innerHTML = html;
    }
    for (const n of [...lista.children]) if (n.dataset.id && !vivos.has(n.dataset.id)) n.remove();
    if (!fs.length) lista.innerHTML = `<p class="vacio-msg">${esc(t("errario_vacio"))}</p>`;
  },
};

/* ═══ TERRITORIOS — con ficha abrible ═══ */
const vTerritorios = {
  async montar(el) { el.innerHTML = `<div class="grilla" id="t-grilla"></div>`; await this.refrescar(el); },
  async refrescar(el) {
    const ts = await api("/api/territorios");
    const g = el.querySelector("#t-grilla"), vivos = new Set();
    if (!ts.length) { g.innerHTML = `<p class="vacio-msg">${esc(t("sin_territorios"))}</p>`; return; }
    for (const x of ts) {
      const id = "t" + x.territorio.replace(/\W/g, "_"); vivos.add(id);
      let c = g.querySelector(`[data-id="${id}"]`);
      if (!c) {
        c = document.createElement("div");
        c.className = "tarjeta clic"; c.dataset.id = id; c.tabIndex = 0;
        c.setAttribute("role", "button");
        c.innerHTML = `<div class="t-nom"></div><div class="t-sub"></div>
          <div class="t-abrir"></div>`;
        const f = () => fichaTerritorio(x.territorio);
        c.onclick = f; c.onkeydown = e => { if (e.key === "Enter") f(); };
        g.appendChild(c);
      }
      setTxt(c.querySelector(".t-nom"), x.territorio);
      const partes = [];
      if (x.fallas_vivas) partes.push(`🔴 ${x.fallas_vivas} ${t("fallas_vivas_de")} ${x.fallas || 0}`);
      else if (x.fallas) partes.push(`🩹 ${x.fallas} ${t("fallas_curadas_todas")}`);
      if (x.chispas) partes.push(`✦ ${x.chispas}`);
      if (x.cronica) partes.push(`✎ ${x.cronica}`);
      setTxt(c.querySelector(".t-sub"), partes.join("  ·  ") || "—");
      setTxt(c.querySelector(".t-abrir"), t("abrir_ficha") + " →");
    }
    for (const n of [...g.children]) if (n.dataset.id && !vivos.has(n.dataset.id)) n.remove();
  },
};

const kb = (b) => b > 1048576 ? (b / 1048576).toFixed(1) + " MB" : Math.max(1, Math.round(b / 1024)) + " KB";

async function fichaTerritorio(nombre) {
  cajon.cargando(nombre);
  const d = await api("/api/territorio?n=" + encodeURIComponent(nombre));
  const dirs = d.mapa.filter(m => m.tipo === "dir"), files = d.mapa.filter(m => m.tipo !== "dir");
  cajon.abrir(nombre, `
    ${d.ruta ? `<p class="mono ruta">${esc(d.ruta)}${d.existe ? "" : " — " + esc(t("ruta_ausente"))}</p>` : ""}
    <div class="mini-kpis">
      <span><b>${d.obras}</b> ${esc(t("obras"))}</span>
      <span><b>${d.fallas.length}</b> ${esc(t("fallas_pal"))}</span>
      <span><b>${d.mapa.length}</b> ${esc(t("entradas"))}</span></div>
    ${d.resumen.length ? `<h3>${esc(t("que_es"))}</h3>` + d.resumen.map(r =>
      `<p class="resumen"><b>${esc(r.titulo)}</b><br>${esc(r.texto)}…</p>`).join("") : ""}
    ${dirs.length ? `<h3>${esc(t("carpetas"))}</h3><div class="caja">` + dirs.map(m =>
      `<div class="linea"><b>📁 ${esc(m.nombre)}</b>
        <div class="sub">${esc(m.detalle)}</div></div>`).join("") + `</div>` : ""}
    ${files.length ? `<h3>${esc(t("archivos_prin"))}</h3><div class="caja">` + files.map(m =>
      `<div class="linea"><b>${esc(m.nombre)}</b>
        <span class="num sub" style="float:right">${kb(m.bytes || 0)}</span>
        ${m.detalle ? `<div class="sub">${esc(m.detalle)}</div>` : ""}</div>`).join("") + `</div>` : ""}
    ${d.mas_tocados.length ? `<h3>${esc(t("mas_tocados"))}</h3><div class="caja">` + d.mas_tocados.map(x =>
      `<div class="linea">${esc(x.ruta.split("/").pop())}
        <span class="num" style="float:right;color:var(--orbit)">×${x.veces}</span>
        <div class="sub mono">${esc(x.ruta)}</div></div>`).join("") + `</div>` : ""}
    ${d.fallas.length ? `<h3>${esc(t("errario"))}</h3><div class="caja">` + d.fallas.map(f =>
      `<div class="linea"><b>#${f.id} ${esc(f.titulo || f.title)}</b>
        <div class="sub">${esc(f.cura || f.cure || "")}</div></div>`).join("") + `</div>` : ""}`);
}

/* === THE GRAPH - a hierarchy, not an anthill ===============================

   The Bearer called it a mess and he was right: a force cloud is noise with
   aesthetics. Now there is declared ORDER:

        CHAOS  ->  clusters (by type)  ->  essences

   DETERMINISTIC layout (same data = same shape, always): clusters spread on a
   ring around the god, and inside each cluster the essences are ordered by
   degree in concentric rings. No physics, no tremor, no chance. Wheel to
   zoom, drag the void to move the world.
*/
const vGrafo = {
  g: null, N: [], A: [], cumulos: [], cv: null, ctx: null, W: 0, H: 0,
  zoom: 1, panX: 0, panY: 0, arrastrando: null, moviendoMundo: null,
  expandidos: new Set(),
  pulsado: null, ro: null, foco: null,

  async montar(el) {
    el.innerHTML = `
      <div class="filtros">
        <button id="g-modo" class="btn-fantasma"></button>
        <button id="g-centrar" class="btn-fantasma"></button>
        <button id="g-limpiar" class="btn-fantasma" hidden></button>
        <span class="zoom-par"><button id="g-menos" class="btn-fantasma" aria-label="zoom -">−</button
        ><button id="g-mas" class="btn-fantasma" aria-label="zoom +">+</button></span>
        <span class="chip" id="g-n"></span><span class="chip" id="g-e"></span>
        <span class="chip" id="g-r"></span>
        <span class="sub" id="g-ayuda"></span></div>
      <canvas id="g-lienzo"></canvas><div id="g-lista" hidden></div>`;
    this.cv = el.querySelector("#g-lienzo");
    this.ctx = this.cv.getContext("2d");

    el.querySelector("#g-modo").onclick = (e) => {
      const l = el.querySelector("#g-lista"), oculto = l.hidden;
      l.hidden = !oculto; this.cv.hidden = oculto;
      setTxt(e.target, oculto ? t("ver_grafo") : t("ver_lista"));
      if (!oculto) this.medirYTrazar();
    };
    el.querySelector("#g-centrar").onclick = () => { this.encuadrar(); this.pintar(); };
    /* clear: clusters fold again and the world reframes -
       el grafo queda exactamente como nace. Solo aparece si hay algo abierto:
       a button that does nothing visible is noise on the bar. */
    el.querySelector("#g-limpiar").onclick = () => {
      this.expandidos.clear(); this.encuadrar(); this.pintar(); this.botones();
    };
    el.querySelector("#g-mas").onclick = () => this.escalar(1.3);
    el.querySelector("#g-menos").onclick = () => this.escalar(1 / 1.3);
    this.cv.tabIndex = 0;                    /* the graph is keyboard-driven too */
    this.cv.onkeydown = (ev) => {
      const paso = 60;
      if (ev.key === "+" || ev.key === "=") this.escalar(1.3);
      else if (ev.key === "-") this.escalar(1 / 1.3);
      else if (ev.key === "0") { this.encuadrar(); this.pintar(); }
      else if (ev.key === "ArrowLeft") this.panX += paso;
      else if (ev.key === "ArrowRight") this.panX -= paso;
      else if (ev.key === "ArrowUp") this.panY += paso;
      else if (ev.key === "ArrowDown") this.panY -= paso;
      else return;
      ev.preventDefault(); this.pintar();
    };

    this.cv.onpointerdown = e => this.tomar(e);
    this.cv.onpointermove = e => this.mover(e);
    this.cv.onpointerup = this.cv.onpointercancel = () => this.soltar();
    this.cv.onwheel = e => this.rueda(e);
    this.cv.ondblclick = () => { this.encuadrar(); this.pintar(); };

    /* ResizeObserver: the canvas is born 0 px tall and the drawing came out
       BLANK until a button was pressed (a real defect). Now it paints itself
       the moment the browser gives it a size - and on resize too. */
    if (this.ro) this.ro.disconnect();
    this.ro = new ResizeObserver(() => this.medirYTrazar());
    this.ro.observe(this.cv);

    await this.refrescar(el);
    this.medirYTrazar();
  },

  async refrescar(el) {
    const g = await api("/api/grafo");
    const rotas = g.aristas.filter(a => a.roto).length;
    setTxt(el.querySelector("#g-n"), `${g.nodos.length} ${t("nodos")}`);
    setTxt(el.querySelector("#g-e"), `${g.aristas.length} ${t("enlaces")}`);
    const r = el.querySelector("#g-r");
    setTxt(r, `${rotas ? "⚠ " : "✓ "}${rotas} ${t("rotos")}`);
    setCls(r, "viva", rotas > 0); setCls(r, "curada", !rotas);
    setTxt(el.querySelector("#g-modo"), el.querySelector("#g-lista").hidden ? t("ver_lista") : t("ver_grafo"));
    setTxt(el.querySelector("#g-centrar"), t("centrar"));
    this.botones();
    setTxt(el.querySelector("#g-ayuda"), t("grafo_ayuda"));

    const cambio = !this.g || this.g.nodos.length !== g.nodos.length ||
      this.g.aristas.length !== g.aristas.length;
    this.g = g;

    const lista = el.querySelector("#g-lista");
    const porCumulo = {};
    for (const n of g.nodos) (porCumulo[n.tipo || "?"] ||= []).push(n);
    const html = Object.entries(porCumulo).sort((a, b) => b[1].length - a[1].length)
      .map(([tp, ns]) => `<h3>${esc(tp)} · ${ns.length}</h3><div class="caja">` +
        ns.sort((a, b) => b.grado - a.grado).map(n => {
          const rot = g.aristas.filter(a => a.o === n.id && a.roto);
          return `<div class="linea"><b>${esc(n.id)}</b>
            <span class="num sub"> ${n.grado} ${esc(t("vinculos"))}</span>
            ${rot.length ? `<span class="chip viva">⚠ ${rot.length} ${esc(t("rotos"))}: ${esc(rot.map(a => a.d).join(", "))}</span>` : ""}</div>`;
        }).join("") + `</div>`).join("");
    if (lista.innerHTML !== html) lista.innerHTML = html;
    if (cambio) this.trazar();
  },

  /* -- the order: CHAOS -> clusters -> essences -- */
  trazar() {
    const g = this.g;
    if (!g || !this.W) return;
    const grupos = {};
    for (const n of g.nodos) (grupos[n.tipo || "?"] ||= []).push(n);
    const claves = Object.keys(grupos).sort((a, b) => grupos[b].length - grupos[a].length);

    const idx = {};
    this.N = [];
    this.cumulos = [];
    /* the god at the centre: everything hangs from him */
    const CX = 0, CY = 0;

    /* PACKING: first I measure how much room each cluster takes, then compute
       the MINIMUM ring that fits them without touching. A fixed radius used to
       todo quedaba disperso y diminuto (encuadre a 0.29). */
    const medida = {};
    for (const clave of claves) {
      const n = grupos[clave].length;
      let i = 0, anillo = 0, rr = 0;
      while (i < n) {
        const cupo = anillo === 0 ? 1 : Math.max(5, Math.floor(anillo * 6.2));
        rr = anillo === 0 ? 0 : 34 + anillo * 31;
        i += Math.min(cupo, n - i); anillo++;
      }
      medida[clave] = rr + 34;
    }
    /* the circumference must hold the sum of diameters, with slack */
    const perim = claves.reduce((s2, c) => s2 + medida[c] * 2, 0) * 1.22;
    const R = Math.max(150, perim / (Math.PI * 2));

    /* angle proportional to size: the fat ones get more arc */
    let acum = 0;
    const angulos = {};
    for (const c of claves) {
      const porc = (medida[c] * 2 * 1.22) / perim;
      angulos[c] = (acum + porc / 2) * Math.PI * 2 - Math.PI / 2;
      acum += porc;
    }

    claves.forEach((clave) => {
      const miembros = grupos[clave].slice().sort((a, b) => b.grado - a.grado);
      const ang = angulos[clave];
      const cx = CX + Math.cos(ang) * R, cy = CY + Math.sin(ang) * R;
      /* concentric rings inside the cluster: the best connected at the core */
      let i = 0, anillo = 0, radioC = 0;
      while (i < miembros.length) {
        const cupo = anillo === 0 ? 1 : Math.max(5, Math.floor(anillo * 6.2));
        const rr = anillo === 0 ? 0 : 34 + anillo * 31;
        const n_este = Math.min(cupo, miembros.length - i);
        for (let k = 0; k < n_este; k++) {
          const a2 = (k / n_este) * Math.PI * 2 + ang + anillo * .45;
          const n = miembros[i + k];
          idx[n.id] = this.N.length;
          this.N.push({ ...n, cumulo: clave,
            x: cx + Math.cos(a2) * rr, y: cy + Math.sin(a2) * rr });
        }
        radioC = rr;
        i += n_este; anillo++;
      }
      this.cumulos.push({ clave, x: cx, y: cy, r: radioC + 34, n: miembros.length });
    });

    this.idx = idx;
    this.A = g.aristas.filter(a => idx[a.o] !== undefined && idx[a.d] !== undefined)
      .map(a => ({ o: idx[a.o], d: idx[a.d], roto: a.roto,
                   mismo: this.N[idx[a.o]].cumulo === this.N[idx[a.d]].cumulo }));
    this.encuadrar();
  },

  botones() {
    const b = document.getElementById("g-limpiar");
    if (!b) return;
    b.hidden = this.expandidos.size === 0;
    setTxt(b, "✕ " + t("limpiar_grafo") + (this.expandidos.size > 1
      ? " (" + this.expandidos.size + ")" : ""));
  },

  encuadrar() {
    if (!this.N.length || !this.W) return;
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (const c of this.cumulos) {
      x0 = Math.min(x0, c.x - c.r); y0 = Math.min(y0, c.y - c.r);
      x1 = Math.max(x1, c.x + c.r); y1 = Math.max(y1, c.y + c.r);
    }
    const m = 40;
    this.zoom = Math.min((this.W - m * 2) / (x1 - x0 || 1),
                         (this.H - m * 2) / (y1 - y0 || 1), 1.6);
    this.panX = this.W / 2 - ((x0 + x1) / 2) * this.zoom;
    this.panY = this.H / 2 - ((y0 + y1) / 2) * this.zoom;
  },

  medirYTrazar() {
    if (!this.cv) return;
    const w = this.cv.clientWidth, h = this.cv.clientHeight;
    if (!w || !h) return;
    const dpr = devicePixelRatio || 1;
    const cambio = w !== this.W || h !== this.H;
    this.W = w; this.H = h;
    this.cv.width = w * dpr; this.cv.height = h * dpr;
    if (!this.N.length) this.trazar();
    else if (cambio) this.encuadrar();
    this.pintar();
  },

  aMundo(e) {
    const r = this.cv.getBoundingClientRect();
    return { x: (e.clientX - r.left - this.panX) / this.zoom,
             y: (e.clientY - r.top - this.panY) / this.zoom };
  },

  pintar() {
    const ctx = this.ctx; if (!ctx || !this.W) return;
    const dpr = devicePixelRatio || 1;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, this.W, this.H);
    ctx.save();
    ctx.translate(this.panX, this.panY); ctx.scale(this.zoom, this.zoom);

    /* SEMANTIC ZOOM: from afar a cluster is ONE body with its
       conteo; al acercarte se abre en sus esencias. Con 200+ nodos la vista
       overview would be an anthill again if everything were always drawn. */
    const abierto = (c) => this.zoom >= .85 || this.expandidos.has(c.clave);

    /* cluster halos: molecules look like molecules */
    for (const c of this.cumulos) {
      const malo = c.clave === "roto";
      ctx.beginPath(); ctx.arc(c.x, c.y, c.r, 0, 7);
      ctx.fillStyle = malo ? "rgba(240,82,63,.06)" : "rgba(138,124,247,.045)";
      ctx.fill();
      ctx.setLineDash(malo ? [6 / this.zoom, 5 / this.zoom] : []);
      ctx.strokeStyle = malo ? "rgba(240,82,63,.45)" : "rgba(138,124,247,.16)";
      ctx.lineWidth = 1 / this.zoom;
      ctx.stroke();
      ctx.setLineDash([]);
      /* spoke from god to cluster: the hierarchy made visible */
      ctx.beginPath(); ctx.moveTo(0, 0); ctx.lineTo(c.x, c.y);
      ctx.strokeStyle = "rgba(138,124,247,.10)"; ctx.stroke();
    }

    /* links: those outside the cluster fainter; the broken ones dashed */
    const visible = (n) => abierto(this.cumulos.find(c => c.clave === n.cumulo) || {});
    for (const a of this.A) {
      const o = this.N[a.o], d = this.N[a.d];
      if (!visible(o) && !visible(d)) continue;   /* plegado: ni se dibuja */
      ctx.beginPath();
      ctx.setLineDash(a.roto ? [5 / this.zoom, 4 / this.zoom] : []);
      ctx.strokeStyle = a.roto ? "rgba(240,82,63,.85)"
        : a.mismo ? "rgba(103,232,249,.30)" : "rgba(138,124,247,.16)";
      ctx.lineWidth = (a.roto ? 1.5 : 1) / this.zoom;
      ctx.moveTo(o.x, o.y); ctx.lineTo(d.x, d.y); ctx.stroke();
    }
    ctx.setLineDash([]);

    /* nodos */
    for (const n of this.N) {
      if (!visible(n)) continue;                  /* the cluster speaks for it */
      const r = 4 + Math.min(10, n.grado * .8);
      if (n === this.foco) {
        ctx.beginPath(); ctx.arc(n.x, n.y, r + 6 / this.zoom, 0, 7);
        ctx.fillStyle = "rgba(103,232,249,.18)"; ctx.fill();
      }
      ctx.beginPath(); ctx.arc(n.x, n.y, r, 0, 7);
      ctx.fillStyle = n.tipo === "roto" ? "#F0523F"
        : (n.residente ? "#67E8F9" : "#8A7CF7");
      ctx.fill();
      if (n.tipo === "roto") {
        ctx.fillStyle = "#F0523F";
        ctx.font = `${11 / this.zoom}px system-ui`;
        ctx.fillText("⚠", n.x + r + 1, n.y - r);
      }
      /* ETIQUETAS POR NIVEL (el Portador: "las etiquetas se pisan").
         Overview = clusters only. Zoom in and the names appear: hubs first,
         then all of them. And always the one you point at. */
      const verNombre = n === this.foco ||
        (this.zoom >= 2.0) ||
        (this.zoom >= 1.15 && n.grado >= 3) ||
        (this.zoom >= 0.95 && n.grado >= 7);
      if (verNombre) {
        ctx.fillStyle = n === this.foco ? "#F2F5FB" : "#8C97B5";
        ctx.font = `${11 / this.zoom}px system-ui`;
        ctx.fillText(n.id.slice(0, 26), n.x + r + 4 / this.zoom, n.y + 3.5 / this.zoom);
      }
    }

    /* the god and his cluster names, above everything */
    for (const c of this.cumulos) {
      if (!abierto(c)) {                          /* plegado: un solo cuerpo */
        ctx.beginPath(); ctx.arc(c.x, c.y, Math.min(c.r * .42, 46), 0, 7);
        ctx.fillStyle = c.clave === "roto" ? "rgba(240,82,63,.30)" : "rgba(138,124,247,.28)";
        ctx.fill();
        ctx.strokeStyle = c.clave === "roto" ? "#F0523F" : "#8A7CF7";
        ctx.lineWidth = 1.5 / this.zoom; ctx.stroke();
        ctx.fillStyle = "#F2F5FB";
        ctx.font = `700 ${18 / this.zoom}px system-ui`;
        ctx.textAlign = "center"; ctx.textBaseline = "middle";
        ctx.fillText(String(c.n), c.x, c.y);
        ctx.textBaseline = "alphabetic";
      }
      const txt = `${c.clave} · ${c.n}`;
      ctx.font = `600 ${13 / this.zoom}px system-ui`;
      ctx.textAlign = "center";
      const w = ctx.measureText(txt).width, h = 17 / this.zoom;
      const ty = c.y - c.r - 8 / this.zoom;
      ctx.fillStyle = "rgba(15,20,32,.86)";        /* placa: el nombre se lee */
      ctx.fillRect(c.x - w / 2 - 6 / this.zoom, ty - h * .78, w + 12 / this.zoom, h);
      ctx.fillStyle = c.clave === "roto" ? "#F0523F" : "rgba(242,245,251,.88)";
      ctx.fillText(txt, c.x, ty);
    }
    ctx.beginPath(); ctx.arc(0, 0, 26, 0, 7);
    ctx.fillStyle = "#151B2B"; ctx.fill();
    ctx.strokeStyle = "#8A7CF7"; ctx.lineWidth = 2 / this.zoom; ctx.stroke();
    ctx.fillStyle = "#F2F5FB";
    ctx.font = `700 ${13 / this.zoom}px system-ui`;
    ctx.textAlign = "center"; ctx.textBaseline = "middle";
    ctx.fillText("CHAOS", 0, 0);
    ctx.textAlign = "start"; ctx.textBaseline = "alphabetic";
    ctx.restore();
  },

  cerca(e) {
    const p = this.aMundo(e);
    let mejor = null, dm = (22 / this.zoom) ** 2;
    const abre = (n) => this.zoom >= .85 || this.expandidos.has(n.cumulo);
    for (const n of this.N) {
      if (!abre(n)) continue;
      const d = (n.x - p.x) ** 2 + (n.y - p.y) ** 2;
      if (d < dm) { dm = d; mejor = n; }
    }
    return mejor;
  },

  /* FALLA MEDIDA: en un trackpad de Mac, rozarlo dispara `wheel`. Con la
     rueda atada al zoom, 25 microeventos llevaban el grafo de 0.53 a 0.15 y
     "desaparecia" al pasar el cursor (4485 -> 583 pixeles pintados).
     Gesto correcto de macOS: dos dedos = MOVER, pellizco (ctrlKey) = zoom.
     Y siempre quedan los botones +/-, que ningun gesto puede robar. */
  rueda(e) {
    e.preventDefault();
    if (e.ctrlKey || e.metaKey) {                    /* pellizco = zoom */
      const r = this.cv.getBoundingClientRect();
      this.escalar(Math.pow(1.0022, -e.deltaY), e.clientX - r.left, e.clientY - r.top);
      return;
    }
    this.panX -= e.deltaX;                           /* dos dedos = mover */
    this.panY -= e.deltaY;
    this.pintar();
  },

  escalar(k, mx, my) {
    if (mx === undefined) { mx = this.W / 2; my = this.H / 2; }
    const antes = { x: (mx - this.panX) / this.zoom, y: (my - this.panY) / this.zoom };
    this.zoom = Math.max(.2, Math.min(6, this.zoom * k));
    this.panX = mx - antes.x * this.zoom;            /* el zoom respeta el punto */
    this.panY = my - antes.y * this.zoom;
    this.pintar();
  },

  tomar(e) {
    /* click a folded cluster = open it (semantic zoom, by hand) */
    const p0 = this.aMundo(e);
    for (const c of this.cumulos) {
      const dentro = Math.hypot(c.x - p0.x, c.y - p0.y) < Math.min(c.r * .42, 46);
      if (dentro && this.zoom < .85 && !this.expandidos.has(c.clave)) {
        this.expandidos.add(c.clave); this.pintar(); this.botones(); return;
      }
    }
    const n = this.cerca(e);
    if (n) {
      this.arrastrando = n;
      this.pulsado = { t: Date.now(), n };
    } else {
      const r = this.cv.getBoundingClientRect();
      this.moviendoMundo = { x: e.clientX - r.left - this.panX,
                             y: e.clientY - r.top - this.panY };
      this.cv.style.cursor = "grabbing";
    }
    try { this.cv.setPointerCapture(e.pointerId); } catch (_) { }
  },

  mover(e) {
    if (this.arrastrando) {
      const p = this.aMundo(e);
      this.arrastrando.x = p.x; this.arrastrando.y = p.y;
      this.pintar(); return;
    }
    if (this.moviendoMundo) {
      const r = this.cv.getBoundingClientRect();
      this.panX = e.clientX - r.left - this.moviendoMundo.x;
      this.panY = e.clientY - r.top - this.moviendoMundo.y;
      this.pintar(); return;
    }
    const n = this.cerca(e);
    if (n !== this.foco) { this.foco = n; this.pintar(); }
    this.cv.style.cursor = n ? "pointer" : "grab";
  },

  soltar() {
    this.cv.style.cursor = "grab";
    this.moviendoMundo = null;
    const p = this.pulsado;
    this.arrastrando = null; this.pulsado = null;
    if (p && Date.now() - p.t < 260) this.ficha(p.n);
  },

  ficha(n) {
    const sal = this.g.aristas.filter(a => a.o === n.id);
    const ent = this.g.aristas.filter(a => a.d === n.id);
    cajon.abrir(n.id, `
      <div class="mini-kpis"><span><b>${n.grado}</b> ${esc(t("vinculos"))}</span>
        <span><b>${sal.length}</b> ${esc(t("salientes"))}</span>
        <span><b>${ent.length}</b> ${esc(t("entrantes"))}</span>
        <span class="chip">${esc(n.tipo)}</span>
        <span class="chip ${n.residente ? "curada" : ""}">${n.residente ? esc(t("residente_si")) : esc(t("residente_no"))}</span></div>
      ${sal.length ? `<h3>${esc(t("apunta_a"))}</h3><div class="caja">` + sal.map(a =>
        `<div class="linea ${a.roto ? "veredicto-mal" : ""}">${a.roto ? "⚠ " : "→ "}${esc(a.d)}
          ${a.roto ? `<span class="chip viva">${esc(t("roto"))}</span>` : ""}</div>`).join("") + "</div>" : ""}
      ${ent.length ? `<h3>${esc(t("lo_nombran"))}</h3><div class="caja">` + ent.map(a =>
        `<div class="linea">← ${esc(a.o)}</div>`).join("") + "</div>" : ""}`);
  },
};

/* === TIME - a real timeline, expandable and clickable === */
const CLASE_ICO = { obra: "✎", falla: "⚠", acto: "◉", chispa: "✦" };
const vTiempo = {
  async montar(el) { el.innerHTML = `<div id="lt"></div>`; await this.refrescar(el); },
  async refrescar(el) {
    const [d, tm] = await Promise.all([api("/api/linea"), api("/api/tiempo")]);
    const cont = el.querySelector("#lt");
    const abiertos = new Set([...cont.querySelectorAll(".dia.abierto")].map(x => x.dataset.d));
    const html = `
      <div class="grilla" style="margin-bottom:18px">
        <div class="tarjeta"><div class="kpi orbita">${tm.horas_inferidas}h</div>
          <div class="kpi-nombre">${esc(t("horas_inferidas"))}</div></div>
        <div class="tarjeta"><div class="kpi">${tm.total_obras}</div>
          <div class="kpi-nombre">${esc(t("obras"))}</div></div>
        <div class="tarjeta"><div class="kpi">${tm.sesiones.length}</div>
          <div class="kpi-nombre">${esc(t("sesiones_obra"))}</div></div>
      </div>
      <p class="pie-nota">ⓘ ${esc(t("aviso_inferido"))}</p>
      <div class="linea-temporal">${d.dias.map((dia, i) => `
        <section class="dia ${i === 0 || abiertos.has(dia.dia) ? "abierto" : ""}" data-d="${dia.dia}">
          <button class="dia-cab" aria-expanded="${i === 0}">
            <span class="dia-punto"></span>
            <span class="dia-fecha mono">${dia.dia}</span>
            <span class="dia-conteo">${Object.entries(dia.conteo).filter(([, n]) => n)
              .map(([c, n]) => `<span class="chip c-${c}">${CLASE_ICO[c]} ${n}</span>`).join("")}</span>
            <span class="dia-chevron">▾</span></button>
          <div class="dia-cuerpo">${dia.eventos.map((e, j) => `
            <div class="suceso c-${e.clase}" data-j="${i}-${j}" tabindex="0" role="button">
              <span class="suceso-hora mono">${esc((e.ts || "").slice(11, 16))}</span>
              <span class="suceso-ico">${CLASE_ICO[e.clase]}</span>
              <span class="suceso-tit">${esc(e.titulo || "—")}</span>
              ${e.sub ? `<span class="chip">${esc(e.sub)}</span>` : ""}
              ${e.ter ? `<span class="chip ter">${esc(e.ter)}</span>` : ""}
            </div>`).join("")}</div>
        </section>`).join("")}</div>`;
    if (cont.innerHTML === html) return;           /* nothing changed -> do not touch */
    cont.innerHTML = html;
    cont.querySelectorAll(".dia-cab").forEach(b => b.onclick = () => {
      const s = b.closest(".dia");
      s.classList.toggle("abierto");
      b.setAttribute("aria-expanded", s.classList.contains("abierto"));
    });
    cont.querySelectorAll(".suceso").forEach(s => {
      const [i, j] = s.dataset.j.split("-").map(Number);
      const e = d.dias[i].eventos[j];
      const abrir = () => cajon.abrir(e.titulo || "—", `
        <div class="mini-kpis"><span class="mono">${esc(e.ts)}</span>
          <span class="chip c-${e.clase}">${CLASE_ICO[e.clase]} ${esc(t(e.clase))}</span>
          ${e.ter ? `<span class="chip">${esc(e.ter)}</span>` : ""}
          ${e.sub ? `<span class="chip">${esc(e.sub)}</span>` : ""}</div>
        <pre class="consola">${esc(e.detalle || "—")}</pre>`);
      s.onclick = abrir;
      s.onkeydown = ev => { if (ev.key === "Enter") abrir(); };
    });
  },
};

/* ═══ MIS ACTOS — clicables, ficha completa ═══ */
const vActos = {
  async montar(el) { el.innerHTML = `<div class="caja" id="a-lista"></div>`; await this.refrescar(el); },
  async refrescar(el) {
    const as = await api("/api/actos");
    const c = el.querySelector("#a-lista"), vivos = new Set();
    if (!as.length) { c.innerHTML = `<p class="vacio-msg">${esc(t("sin_actos"))}</p>`; return; }
    for (const a of as) {
      const id = "a" + a.id; vivos.add(id);
      let d = c.querySelector(`[data-id="${id}"]`);
      if (!d) {
        d = document.createElement("div");
        d.className = "linea clic"; d.dataset.id = id; d.tabIndex = 0;
        d.setAttribute("role", "button");
        d.innerHTML = `<span class="mono f"></span> · <b class="tp"></b> ·
          <span class="num du"></span> · <span class="ver"></span>
          <div class="sub de"></div>`;
        const abrir = () => {
          const nac = (a.creados || a.created || "").split("\n").filter(Boolean);
          const alt = (a.alterados || a.altered || "").split("\n").filter(Boolean);
          const aj = (a.archivos || a.files || "").split("\n").filter(Boolean);
          const ver = a.veredicto || a.verdict || "ok";
          cajon.abrir(`${a.tipo || a.kind} / ${a.accion || a.action}`, `
            <div class="mini-kpis"><span class="mono">${esc(a.fecha || a.date)}</span>
              <span><b>${(+(a.duracion || a.duration) || 0).toFixed(1)}s</b></span>
              <span><b>${a.hallazgos ?? a.findings ?? 0}</b> ${esc(t("frentes"))}</span>
              <span class="chip ${ver === "ok" ? "curada" : "viva"}">${esc(ver)}</span></div>
            <p class="sub">${esc(a.detalle || a.detail || "")}</p>
            <p class="sub mono">${esc(t("maquina"))}: ${esc(a.maquina || a.machine || "?")}</p>
            ${aj.length ? `<h3 class="peligro">${esc(t("ajenos_tocados"))}</h3><div class="caja">` +
              aj.map(x => `<div class="linea veredicto-mal">⚠ ${esc(x)}</div>`).join("") + "</div>" : ""}
            ${nac.length ? `<h3>${esc(t("nacio_de_mi"))} (${nac.length})</h3><div class="caja">` +
              nac.map(x => `<div class="linea">${esc(x.split("/").pop())}<div class="sub mono">${esc(x)}</div></div>`).join("") + "</div>" : ""}
            ${alt.length ? `<h3>${esc(t("altere"))} (${alt.length})</h3><div class="caja">` +
              alt.map(x => `<div class="linea">${esc(x.split("/").pop())}<div class="sub mono">${esc(x)}</div></div>`).join("") + "</div>" : ""}`);
        };
        d.onclick = abrir; d.onkeydown = e => { if (e.key === "Enter") abrir(); };
        c.appendChild(d);
      }
      const ver = a.veredicto || a.verdict || "ok";
      setTxt(d.querySelector(".f"), a.fecha || a.date);
      setTxt(d.querySelector(".tp"), `${a.tipo || a.kind}/${a.accion || a.action}`);
      setTxt(d.querySelector(".du"), (+(a.duracion || a.duration) || 0).toFixed(1) + "s");
      const v = d.querySelector(".ver"); setTxt(v, ver);
      setCls(v, "veredicto-ok", ver === "ok"); setCls(v, "veredicto-mal", ver !== "ok");
      setTxt(d.querySelector(".de"), a.detalle || a.detail || "");
    }
    for (const n of [...c.children]) if (n.dataset.id && !vivos.has(n.dataset.id)) n.remove();
  },
};

/* === SEARCH === */
const vBuscar = {
  async montar(el) {
    el.innerHTML = `<div class="filtros"><input id="b-q" type="search"
      placeholder="${esc(t("buscar_ph"))}" aria-label="${esc(t("buscar_ph"))}"></div>
      <pre class="consola" id="b-out">${esc(t("buscar_hint"))}</pre>`;
    const i = el.querySelector("#b-q"), o = el.querySelector("#b-out");
    let tm;
    i.oninput = () => {
      clearTimeout(tm);
      tm = setTimeout(async () => {
        if (!i.value.trim()) { setTxt(o, t("buscar_hint")); return; }
        const r = await api("/api/buscar?q=" + encodeURIComponent(i.value));
        setTxt(o, r.lineas.join("\n") || t("sin_resultados"));
      }, 260);
    };
    i.focus();
  },
  async refrescar() { /* no auto-refresh: it would get in the way of typing */ },
};

/* === HEALTH - the diagnosis, not the dump ================================
   This view used to spit the raw output of `audit`: a pile of lines with no
   hierarchy. Now each dimension declares ITS FORMULA, contributes its weight
   global, y TODO lo que resta para 100 se puede desglosar por nombre. Un
   porcentaje que no se puede explicar es un adorno. */
const COLOR_SALUD = (p) => p >= 85 ? "var(--ok)" : p >= 65 ? "var(--orbit)"
  : p >= 40 ? "#FFB054" : "var(--danger)";

const vSalud = {
  datos: null,
  async montar(el) {
    el.innerHTML = `<div id="sa-cab" class="salud-cab"></div>
      <h3 class="sec-tit" id="sa-tit"></h3>
      <div id="sa-dims" class="salud-dims"></div>`;
    await this.refrescar(el);
  },
  async refrescar(el) {
    const d = await api("/api/salud");
    this.datos = d;
    const C = 2 * Math.PI * 54;
    const cab = el.querySelector("#sa-cab");
    const html = `
      <div class="anillo">
        <svg viewBox="0 0 128 128" width="150" height="150" role="img"
             aria-label="${esc(t("salud_global"))}: ${d.global}%">
          <circle cx="64" cy="64" r="54" fill="none" stroke="var(--card)" stroke-width="13"/>
          <circle cx="64" cy="64" r="54" fill="none" stroke="${COLOR_SALUD(d.global)}"
            stroke-width="13" stroke-linecap="round" transform="rotate(-90 64 64)"
            stroke-dasharray="${(C * d.global / 100).toFixed(1)} ${C.toFixed(1)}"/>
          <text x="64" y="60" text-anchor="middle" fill="var(--text)"
            font-size="26" font-weight="700">${d.global}</text>
          <text x="64" y="80" text-anchor="middle" fill="var(--muted)" font-size="11">%</text>
        </svg>
      </div>
      <div class="salud-resumen">
        <div class="salud-veredicto" style="color:${COLOR_SALUD(d.global)}">
          ${esc(t("ver_" + d.veredicto))}</div>
        <p class="sub">${esc(t("salud_expl"))}</p>
        <p class="sub"><b>${d.total_problemas}</b> ${esc(t("problemas_nombrables"))}</p>
      </div>`;
    if (cab.innerHTML !== html) cab.innerHTML = html;
    setTxt(el.querySelector("#sa-tit"), t("por_dimension"));

    const cont = el.querySelector("#sa-dims"), vivos = new Set();
    for (const x of d.dimensiones) {
      vivos.add(x.clave);
      let f = cont.querySelector(`[data-d="${x.clave}"]`);
      if (!f) {
        f = document.createElement("div");
        f.className = "dim"; f.dataset.d = x.clave; f.tabIndex = 0;
        f.setAttribute("role", "button");
        f.innerHTML = `<div class="dim-cab"><b class="dim-tit"></b>
            <span class="chip peso"></span><span class="dim-pct num"></span></div>
          <div class="barra"><i></i></div>
          <div class="dim-det sub"></div>
          <div class="dim-formula mono"></div>`;
        const abrir = () => this.desglose(x.clave);
        f.onclick = abrir;
        f.onkeydown = e => { if (e.key === "Enter") abrir(); };
        cont.appendChild(f);
      }
      // el motor entrega la CLAVE; el idioma lo pone el rostro (frente 11:
      // against an English body the dimensions came out in Spanish)
      setTxt(f.querySelector(".dim-tit"), t("dim_" + x.clave));
      setTxt(f.querySelector(".peso"), t("peso") + " " + x.peso);
      const pct = f.querySelector(".dim-pct");
      setTxt(pct, x.puntaje + "%");
      pct.style.color = COLOR_SALUD(x.puntaje);
      const b = f.querySelector(".barra i");
      b.style.width = x.puntaje + "%";
      b.style.background = COLOR_SALUD(x.puntaje);
      setTxt(f.querySelector(".dim-det"),
        x.detalle + (x.n_problemas ? `  ·  ${x.n_problemas} ${t("por_resolver")} →` : "  ·  ✓"));
      setTxt(f.querySelector(".dim-formula"), "ƒ  " + x.formula);
      setCls(f, "sana", x.n_problemas === 0);
    }
    for (const n of [...cont.children]) if (!vivos.has(n.dataset.d)) n.remove();
  },
  desglose(clave) {
    const x = this.datos.dimensiones.find(d => d.clave === clave);
    if (!x) return;
    const porClase = {};
    for (const p of x.problemas) (porClase[p.clase] ||= []).push(p);
    cajon.abrir(`${t("dim_" + x.clave)} — ${x.puntaje}%`, `
      <div class="mini-kpis">
        <span><b>${x.puntaje}%</b> ${esc(t("puntaje"))}</span>
        <span><b>${x.peso}</b> ${esc(t("peso"))}</span>
        <span><b>${x.n_problemas}</b> ${esc(t("por_resolver"))}</span></div>
      <p class="dim-formula mono">ƒ  ${esc(x.formula)}</p>
      <p class="sub">${esc(x.detalle)}</p>
      ${x.n_problemas === 0 ? `<p class="vacio-msg">✓ ${esc(t("dim_sana"))}</p>` :
        Object.entries(porClase).map(([cl, ps]) =>
          `<h3>${esc(t("cl_" + cl))} · ${ps.length}</h3><div class="caja">` +
          ps.map(p => {
            /* what is fixable brings its own button: a diagnosis that cannot be
               puede accionar es una queja con formato */
            const m = /#?(\d+)/.exec(p.titulo || "");
            const acc = cl === "hambre" ? ["saciar", m && m[1], "🍽 " + t("saciar")]
              : cl === "deuda" ? ["saldar_deuda", m && m[1], "✓ " + t("saldar")]
              : cl === "viva" ? ["curar_falla", m && m[1], "🩹 " + t("curar")]
              : cl === "huerfana" ? ["declarar_isla", p.titulo, "🏝 " + t("declarar_isla")]
              : null;
            return `<div class="linea"><b>${esc(p.titulo)}</b>
              ${acc && acc[1] ? `<button class="btn-fantasma acc-dim" data-a="${acc[0]}"
                 data-g="${esc(acc[1])}">${esc(acc[2])}</button>` : ""}
              <div class="sub">${esc(p.detalle)}</div></div>`;
          }).join("") + `</div>`).join("")}`);
    document.querySelectorAll("#cajon-cuerpo .acc-dim").forEach(b2 => {
      b2.onclick = async () => {
        b2.disabled = true;
        if (await obrar(b2.dataset.a, b2.dataset.g)) { this.desglose(clave); }
        else b2.disabled = false;
      };
    });
  },
};

/* ═══ NOTAS — las chispas, con su anclaje de tres niveles ══════════════════ */
const vNotas = {
  datos: [],
  async montar(el) {
    el.innerHTML = `
      <div class="filtros">
        <input id="n-q" type="search" placeholder="${esc(t("filtrar_notas"))}" aria-label="${esc(t("filtrar_notas"))}">
        <select id="n-ter"><option value="">${esc(t("todo_territorio"))}</option></select>
        <select id="n-fec">
          <option value="">${esc(t("toda_fecha"))}</option>
          <option value="7">${esc(t("ult_7"))}</option>
          <option value="30">${esc(t("ult_30"))}</option>
          <option value="90">${esc(t("ult_90"))}</option></select>
        <select id="n-ord">
          <option value="nuevas">${esc(t("mas_nuevas"))}</option>
          <option value="viejas">${esc(t("mas_viejas"))}</option>
          <option value="conf">${esc(t("por_confianza"))}</option></select>
      </div>
      <p class="pie-nota" id="n-cuenta"></p>
      <div id="n-lista"></div>`;
    ["#n-q", "#n-ter", "#n-fec", "#n-ord"].forEach(sel => {
      const e = el.querySelector(sel);
      e.addEventListener(sel === "#n-q" ? "input" : "change", () => this.pintar(el));
    });
    await this.refrescar(el);
  },
  async refrescar(el) {
    this.datos = await api("/api/notas");
    const sel = el.querySelector("#n-ter"), prev = sel.value;
    const ters = [...new Set(this.datos.map(n => n.territorio).filter(Boolean))].sort();
    if (sel.options.length !== ters.length + 1) {
      sel.innerHTML = `<option value="">${esc(t("todo_territorio"))}</option>` +
        ters.map(x => `<option>${esc(x)}</option>`).join("");
      sel.value = prev;
    }
    this.pintar(el);
  },
  pintar(el) {
    const q = el.querySelector("#n-q").value.toLowerCase();
    const ter = el.querySelector("#n-ter").value;
    const dias = parseInt(el.querySelector("#n-fec").value || "0", 10);
    const ord = el.querySelector("#n-ord").value;
    const corte = dias ? Date.now() - dias * 864e5 : 0;
    let ns = this.datos.filter(n => {
      const saco = [n.texto || n.text, n.contexto || n.context, n.foco || n.focus,
        n.ancla || n.anchor, n.territorio].join(" ").toLowerCase();
      const ts = Date.parse(n.fecha || n.date || "") || 0;
      return (!q || saco.includes(q)) && (!ter || n.territorio === ter) &&
        (!corte || ts >= corte);
    });
    ns.sort((a, b) => ord === "conf"
      ? (b.confianza || b.confidence || 0) - (a.confianza || a.confidence || 0)
      : ord === "viejas" ? (a.id - b.id) : (b.id - a.id));
    setTxt(el.querySelector("#n-cuenta"),
      `${ns.length} ${t("de")} ${this.datos.length} ${t("chispas")}`);
    const lista = el.querySelector("#n-lista");
    if (!ns.length) { lista.innerHTML = `<p class="vacio-msg">${esc(t("sin_chispas"))}</p>`; return; }
    const html = ns.map(n => {
      const conf = +(n.confianza ?? n.confidence ?? 0);
      const col = conf >= .7 ? "var(--ok)" : conf >= .4 ? "var(--orbit)" : "var(--danger)";
      return `<article class="nota" tabindex="0" data-id="${n.id}">
        <div class="nota-txt">${esc(n.texto || n.text)}</div>
        <div class="nota-anclaje">
          <span class="ancl n1" title="${esc(t("nivel1"))}">📍 ${esc(n.territorio || "—")}</span>
          <span class="ancl n2" title="${esc(t("nivel2"))}">📄 ${esc(n.foco || n.focus || "—")}</span>
          <span class="ancl n3" title="${esc(t("nivel3"))}">🔗 ${esc(n.ancla || n.anchor || t("sin_ancla"))}</span>
          <span class="ancl conf" style="color:${col}" title="${esc(t("confianza_t"))}">
            ${(conf * 100).toFixed(0)}%</span>
          <span class="ancl mono">${esc(String(n.fecha || n.date || "").slice(0, 16).replace("T", " "))}</span>
        </div>
        ${(n.contexto || n.context) ? `<div class="nota-ctx sub">« ${esc(n.contexto || n.context)} »</div>` : ""}
      </article>`;
    }).join("");
    if (lista.innerHTML !== html) lista.innerHTML = html;
  },
};

const RENDER = {
  constelacion: vConstelacion, errario: vErrario, territorios: vTerritorios,
  notas: vNotas, grafo: vGrafo, tiempo: vTiempo, actos: vActos,
  buscar: vBuscar, salud: vSalud
};

function ir(v) { if (v !== vistaActual) { vistaActual = v; montar(); } }

async function montar() {
  setTxt(document.getElementById("titulo-vista"), t(vistaActual));
  armarMenu();
  const el = document.getElementById("vista");
  el.dataset.v = vistaActual;
  try { await RENDER[vistaActual].montar(el); }
  catch (e) { el.innerHTML = `<p class="vacio-msg">✗ ${esc(e.message)}</p>`; }
}

/* SURGICAL refresh: patches the live view, never rebuilds it */
let refrescando = false;
async function refrescar() {
  if (refrescando) return;
  refrescando = true;
  const el = document.getElementById("vista");
  try { if (el.dataset.v === vistaActual) await RENDER[vistaActual].refrescar(el); }
  catch (e) { /* un refresco fallido nunca rompe la pantalla */ }
  finally { refrescando = false; }
}

/* -- SSE: batches 500 ms and patches. Never reloads the page. -- */
function pulso() {
  const es = new EventSource("/api/eventos?t=" + TOKEN);
  let tm;
  es.addEventListener("hola", () => { PULSO_VIVO = true; pintarPulso(); });
  es.addEventListener("cambio", () => {
    PULSO_VIVO = true; pintarPulso();
    clearTimeout(tm); tm = setTimeout(refrescar, 500);
  });
  es.onerror = () => { PULSO_VIVO = false; pintarPulso(); };
}

async function cargarIdioma() {
  I18N = await api("/i18n?l=" + IDIOMA);
  document.documentElement.lang = IDIOMA;
  setTxt(document.getElementById("idioma"), IDIOMA === "es" ? "EN" : "ES");
  document.querySelectorAll("[data-i18n]").forEach(x => setTxt(x, t(x.dataset.i18n)));
  pintarPulso();          /* the pulse recovers ITS state, not the generic text */
}

document.getElementById("idioma").onclick = async () => {
  IDIOMA = IDIOMA === "es" ? "en" : "es";
  localStorage.setItem("ojo_idioma", IDIOMA);
  await cargarIdioma(); await montar();
};

(async function init() {
  await cargarIdioma();
  await montar();
  pulso();
})();
