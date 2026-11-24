// F8.3 · Menciones con @ en la caja de escribir. Rust busca (`menciones`: archivos sin lo ignorado, chats,
// reglas y recursos MCP); aquí sólo se pinta la lista y se inserta la ficha. Lo que trae cada mención al
// mensaje (un chat, un recurso) lo añade Rust al mandarlo.
(() => {
  const inv = (c, a) => window.__TAURI__.core.invoke(c, a);
  const caja = el("div", "slash menc", "");
  caja.id = "menc";
  caja.setAttribute("role", "listbox");
  caja.style.display = "none";
  const comp = document.querySelector(".composer");
  comp.prepend(caja);
  const M = { items: [], sel: 0, desde: -1, pedido: 0, abierta: false };
  const ICONO = { archivo: "📄", carpeta: "📁", chat: "💬", regla: "📏", recurso: "🔌" };
  const CLASE = { archivo: "mnArchivo", carpeta: "mnCarpeta", chat: "mnChat", regla: "mnRegla", recurso: "mnRecurso" };

  function cerrar() { caja.style.display = "none"; M.abierta = false; M.items = []; }
  // la @ que se está escribiendo justo antes del cursor
  function token() {
    const t = input.value.slice(0, input.selectionStart);
    const m = /(^|\s)@([^\s@]*)$/.exec(t);
    return m ? { desde: t.length - m[2].length - 1, q: m[2] } : null;
  }
  function pintar() {
    caja.innerHTML = "";
    if (!M.items.length) { caja.innerHTML = `<div class="it vacio">${esc(tt("mnVacio"))}</div>`; return; }
    M.items.forEach((m, i) => {
      const d = el("div", "it" + (i === M.sel ? " on" : ""), "");
      d.setAttribute("role", "option"); d.setAttribute("aria-selected", String(i === M.sel));
      d.innerHTML = `<span class="mi" aria-hidden="true">${ICONO[m.clase] || "•"}</span><b></b><span class="d"></span><i>${esc(tt(CLASE[m.clase] || m.clase))}</i>`;
      d.querySelector("b").textContent = m.etiqueta;
      d.querySelector(".d").textContent = m.detalle;
      d.onmousedown = (e) => { e.preventDefault(); elegir(i); };
      caja.appendChild(d);
    });
    const on = caja.querySelector(".on");
    if (on) on.scrollIntoView({ block: "nearest" });
  }
  async function buscar() {
    const t = token();
    if (!t || !S.root) return cerrar();
    M.desde = t.desde;
    const n = ++M.pedido;
    let r = [];
    try { r = await inv("menciones", { consulta: t.q }); } catch { r = []; }
    if (n !== M.pedido) return; // llegó una más nueva
    M.items = r; M.sel = 0; M.abierta = true;
    caja.style.display = "block";
    pintar();
  }
  function elegir(i) {
    const m = M.items[i];
    if (!m) return cerrar();
    const fin = input.selectionStart;
    const antes = input.value.slice(0, M.desde), despues = input.value.slice(fin);
    // una carpeta queda abierta (sin espacio) para seguir eligiendo lo de dentro
    const carpeta = m.clase === "carpeta";
    const ficha = "@" + m.ficha + (carpeta ? "" : " ");
    input.value = antes + ficha + despues;
    const pos = antes.length + ficha.length;
    input.setSelectionRange(pos, pos);
    cerrar(); fit(); refreshSend(); input.focus();
    if (carpeta) buscar();
  }
  let espera = 0;
  input.addEventListener("input", () => { clearTimeout(espera); espera = setTimeout(buscar, 90); });
  input.addEventListener("blur", () => setTimeout(cerrar, 120));
  // antes que el Enter de enviar y las flechas de «/»: en fase de captura, en la caja que lo contiene
  document.getElementById("box").addEventListener("keydown", (e) => {
    if (!M.abierta || e.target !== input) return;
    const k = e.key;
    if (k === "ArrowDown" || k === "ArrowUp") { e.preventDefault(); e.stopImmediatePropagation(); if (M.items.length) { M.sel = (M.sel + (k === "ArrowDown" ? 1 : -1) + M.items.length) % M.items.length; pintar(); } }
    else if ((k === "Enter" || k === "Tab") && M.items.length) { e.preventDefault(); e.stopImmediatePropagation(); elegir(M.sel); }
    else if (k === "Escape") { e.preventDefault(); e.stopImmediatePropagation(); cerrar(); }
  }, true);
})();
