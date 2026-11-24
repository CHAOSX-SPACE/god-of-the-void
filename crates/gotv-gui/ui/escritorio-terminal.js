// F16.7 · Pestaña «Terminal»: la terminal del USUARIO (fuera de la caja). Rust emula la pantalla
// (gotv_core::escritorio::pantalla) y manda VISTAS (filas de tramos con estilo + cursor); esto sólo las pinta
// y manda las teclas tal cual. Montaje: gotvTerminal.montar(contenedor).
(() => {
  const inv = (c, a) => window.__TAURI__.core.invoke(c, a);
  const h = (tag, cls, txt) => { const e = document.createElement(tag); if (cls) e.className = cls; if (txt != null) e.textContent = txt; return e; };
  // los 16 colores de xterm (el 256 y el rgb llegan resueltos: índice o «#rrggbb»)
  const P16 = ["#1e1e1e", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#d0d0d0", "#5c6370", "#ff7b86", "#b5e890", "#ffd88a", "#7ec4ff", "#de9bff", "#7ad9e6", "#ffffff"];
  const color = (c) => {
    if (c == null) return null;
    if (typeof c === "string") return c;
    if (c < 16) return P16[c];
    if (c < 232) { const n = c - 16, v = (x) => (x ? 55 + x * 40 : 0); return `rgb(${v(Math.floor(n / 36))},${v(Math.floor(n / 6) % 6)},${v(n % 6)})`; }
    const g = 8 + (c - 232) * 10; return `rgb(${g},${g},${g})`;
  };
  let ui = null, id = null;

  function pintar(v) {
    if (!ui) return;
    const frag = document.createDocumentFragment();
    v.filas.forEach((fila, i) => {
      const linea = h("div", "ter-linea");
      let col = 0;
      for (const t of fila) {
        const s = h("span", null, t.t);
        let fg = color(t.fg), bg = color(t.bg);
        if (t.inv) { fg = fg || "var(--ter-bg)"; bg = bg || "var(--ter-fg)"; }
        if (fg) s.style.color = fg;
        if (bg) s.style.background = bg;
        if (t.b) s.style.fontWeight = "700";
        if (t.d) s.style.opacity = "0.6";
        if (t.i) s.style.fontStyle = "italic";
        if (t.u) s.style.textDecoration = "underline";
        linea.appendChild(s);
        col += [...t.t].length;
      }
      if (v.cursor_visible && i === v.cursor[0]) {
        const falta = v.cursor[1] - col;
        if (falta > 0) linea.appendChild(h("span", null, " ".repeat(falta)));
        const c = h("span", "ter-cursor", " ");
        if (falta < 0) { /* el cursor cae dentro del texto: se marca con un borde */ c.classList.add("dentro"); }
        linea.appendChild(c);
      }
      frag.appendChild(linea);
    });
    ui.pantalla.replaceChildren(frag);
  }

  function medida() {
    const r = ui.pantalla.getBoundingClientRect();
    const cw = 7.8, ch = 17; // ancho/alto de una celda en la fuente mono de la pestaña
    return { cols: Math.max(20, Math.floor((r.width - 12) / cw)), filas: Math.max(5, Math.floor((r.height - 8) / ch)) };
  }

  // teclas → secuencias de terminal
  const TECLAS = { Enter: "\r", Backspace: "\x7f", Tab: "\t", Escape: "\x1b", ArrowUp: "\x1b[A", ArrowDown: "\x1b[B", ArrowRight: "\x1b[C", ArrowLeft: "\x1b[D", Home: "\x1b[H", End: "\x1b[F", Delete: "\x1b[3~", PageUp: "\x1b[5~", PageDown: "\x1b[6~" };
  function tecla(e) {
    if (id == null) return;
    let d = null;
    if (e.ctrlKey && e.key.length === 1) d = String.fromCharCode(e.key.toUpperCase().charCodeAt(0) - 64);
    else if (e.metaKey) return; // ⌘C/⌘V los maneja el sistema
    else if (TECLAS[e.key]) d = TECLAS[e.key];
    else if (e.key.length === 1) d = e.key;
    if (d == null) return;
    e.preventDefault();
    inv("terminal_escribir", { id, datos: d }).catch(() => {});
  }

  async function abrir() {
    if (!ui || id != null) return;
    const m = medida();
    try {
      id = await inv("terminal_abrir", { cols: m.cols, filas: m.filas });
      ui.pantalla.focus();
    } catch (e) {
      ui.pantalla.textContent = "✗ " + e;
    }
  }

  function cerrar() {
    if (id == null) return;
    inv("terminal_cerrar", { id }).catch(() => {});
    id = null;
  }

  function montar(cont) {
    cont.innerHTML = "";
    const barra = h("div", "ter-barra");
    const bA = h("button", "btn sm", tt("terAbrir"));
    bA.onclick = abrir;
    const bC = h("button", "btn sm", tt("terCerrar"));
    bC.onclick = cerrar;
    barra.append(bA, bC, h("span", "ter-nota", tt("terNota")));
    const pantalla = h("div", "ter-pantalla");
    pantalla.tabIndex = 0;
    pantalla.onkeydown = tecla;
    pantalla.onpaste = (e) => { if (id != null) { e.preventDefault(); inv("terminal_escribir", { id, datos: e.clipboardData.getData("text") }); } };
    cont.append(barra, pantalla);
    ui = { pantalla };
    new ResizeObserver(() => { if (id != null) { const m = medida(); inv("terminal_tamano", { id, cols: m.cols, filas: m.filas }).catch(() => {}); } }).observe(pantalla);
  }

  window.__TAURI__.event.listen("gotv", (ev) => {
    const m = ev.payload;
    if (!m || m.kind !== "terminal" || m.id !== id) return;
    if (m.vista && m.vista.fin) { ui.pantalla.appendChild(h("div", "ter-fin", tt("terFin"))); id = null; return; }
    if (m.vista) pintar(m.vista);
  });
  window.gotvTerminal = { montar, abrir, cerrar };
  // F8: la pestaña «Terminal» del panel de trabajo (se abre sola la primera vez que se muestra)
  if (window.gotvPanel) gotvPanel.registrar("terminal", { montar, mostrar() { if (id == null) abrir(); ui && ui.pantalla.focus(); }, ancho: 640 });
})();
