// F8.6 · ⌘K: buscar en los chats de TODOS los proyectos, en los archivos del abierto y en las órdenes de la
// ventana. Rust busca (`buscar_todo`, en el texto visible de los chats, sin tildes ni mayúsculas); aquí sólo
// se pinta y se ejecuta lo elegido.
(() => {
  const inv = (c, a) => window.__TAURI__.core.invoke(c, a);
  const m = el("div", "modal buscar", "");
  m.id = "buscarModal";
  m.setAttribute("role", "dialog"); m.setAttribute("aria-modal", "true");
  m.innerHTML = `<div class="card bk"><input type="search" id="bkQ" autocomplete="off" spellcheck="false"><div class="bk-res" id="bkRes" role="listbox"></div></div>`;
  document.body.appendChild(m);
  const q = m.querySelector("#bkQ"), res = m.querySelector("#bkRes");
  const B = { items: [], sel: 0, pedido: 0 };
  // a dónde ir tras abrir un chat: el mensaje encontrado
  let irA = null;

  const ORDEN = {
    nuevo_chat: () => inv("chat_abrir", { proyecto: null, id: null }),
    nueva_charla: () => inv("suelta_nueva"),
    nuevo_proyecto: () => $("newProj").onclick(),
    ajustes: () => openSettings(),
    panel_tareas: () => gotvPanel.abrir("tareas"),
    panel_cambios: () => gotvPanel.abrir("cambios"),
    panel_vista: () => gotvPanel.abrir("vista"),
    papelera: () => gotvChats.papelera(),
  };

  function cerrar() { m.classList.remove("show"); input.focus(); }
  function pintar(r) {
    B.items = [];
    res.innerHTML = "";
    const grupo = (titulo, lista, fila) => {
      if (!lista.length) return;
      res.appendChild(el("div", "bk-g", titulo));
      for (const x of lista) {
        const i = B.items.length;
        const d = el("div", "bk-it", "");
        d.setAttribute("role", "option");
        fila(d, x);
        d.onmousedown = (e) => { e.preventDefault(); B.sel = i; elegir(); };
        res.appendChild(d);
        B.items.push({ d, x });
      }
    };
    grupo(tt("bkOrdenes"), r.ordenes.map((o) => ({ tipo: "orden", ...o })), (d, o) => { d.textContent = tt(o.i18n); });
    grupo(tt("bkChats"), r.chats.map((c) => ({ tipo: "chat", ...c })), (d, c) => {
      d.innerHTML = `<div class="bk-t"><b></b><span class="mm"></span></div><div class="bk-f"></div>`;
      d.querySelector("b").textContent = c.titulo || tt("chSinTitulo");
      d.querySelector(".mm").textContent = `${c.proyecto} · ${c.fecha}`;
      // el fragmento llega con lo encontrado entre » «: se resalta sin meter HTML ajeno
      const f = d.querySelector(".bk-f");
      f.textContent = (c.rol === "user" ? tt("bkTu") : "GOTV") + ": ";
      for (const [k, parte] of String(c.fragmento).split(/[»«]/).entries()) { const s = el(k % 2 ? "mark" : "span", "", parte); f.appendChild(s); }
    });
    grupo(tt("bkArchivos"), r.archivos.map((a) => ({ tipo: "archivo", ...a })), (d, a) => {
      d.innerHTML = `<b></b> <span class="mm mono"></span>`;
      d.querySelector("b").textContent = a.etiqueta;
      d.querySelector(".mm").textContent = a.ficha;
    });
    if (!B.items.length) res.appendChild(el("div", "bk-nada", tt("bkNada")));
    B.sel = 0;
    marcar();
  }
  function marcar() { B.items.forEach((it, i) => { it.d.classList.toggle("on", i === B.sel); it.d.setAttribute("aria-selected", String(i === B.sel)); }); const on = B.items[B.sel]; if (on) on.d.scrollIntoView({ block: "nearest" }); }
  async function buscar() {
    const n = ++B.pedido;
    let r;
    try { r = await inv("buscar_todo", { consulta: q.value }); } catch (x) { res.textContent = "✗ " + x; return; }
    if (n === B.pedido) pintar(r);
  }
  async function elegir() {
    const it = B.items[B.sel];
    if (!it) return;
    const x = it.x;
    cerrar();
    try {
      if (x.tipo === "orden") await (ORDEN[x.id] || (() => {}))();
      else if (x.tipo === "archivo") { input.value = (input.value ? input.value.replace(/\s*$/, " ") : "") + "@" + x.ficha + " "; fit(); refreshSend(); input.focus(); }
      else if (x.tipo === "chat") {
        if (S.busy) return toast(tt("working"));
        irA = x.mensaje;
        if (x.raiz === S.root && x.chat === S.chatId) return saltar();
        await inv("chat_abrir", { proyecto: x.raiz, id: x.chat });
      }
    } catch (e) { toast("✗ " + e, 5000); }
  }
  // al abrir el chat, se va al mensaje encontrado y se resalta un momento
  function saltar() {
    if (irA == null) return;
    const n = log.querySelector(`[data-i="${irA}"]`);
    irA = null;
    if (!n) return;
    n.scrollIntoView({ block: "center" });
    n.classList.add("bk-flash");
    setTimeout(() => n.classList.remove("bk-flash"), 1600);
  }
  window.__TAURI__.event.listen("gotv", ({ payload: e }) => { if (e.kind === "project") setTimeout(saltar, 60); });

  let espera = 0;
  q.oninput = () => { clearTimeout(espera); espera = setTimeout(buscar, 120); };
  q.onkeydown = (e) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") { e.preventDefault(); if (B.items.length) { B.sel = (B.sel + (e.key === "ArrowDown" ? 1 : -1) + B.items.length) % B.items.length; marcar(); } }
    else if (e.key === "Enter") { e.preventDefault(); elegir(); }
    else if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); cerrar(); }
  };
  m.addEventListener("mousedown", (e) => { if (e.target === m) cerrar(); });

  window.gotvBuscar = {
    abrir() { q.placeholder = tt("bkPh"); q.setAttribute("aria-label", tt("bkPh")); q.value = ""; m.classList.add("show"); q.focus(); buscar(); },
  };
  document.addEventListener("keydown", (e) => {
    if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "k") { e.preventDefault(); m.classList.contains("show") ? cerrar() : gotvBuscar.abrir(); }
  });
})();
