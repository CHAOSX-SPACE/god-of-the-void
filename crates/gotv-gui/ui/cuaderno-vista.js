// F4 · La tarjeta del NOTEBOOK: sus celdas con sus salidas (texto, imágenes, errores con su traza) y la celda
// que tocó el agente resaltada. Sólo pinta: la tarjeta la arma el núcleo (gotv_core::cuaderno::vista) desde el
// archivo tal como quedó. Una tarjeta por notebook: la nueva reemplaza a la anterior del mismo archivo.
// El markdown pasa por mdRender (escapa TODO el HTML antes de dar formato); el texto, por textContent.

function addCuaderno(m) {
  clearEmpty();
  const ruta = String(m.ruta || "");
  const card = el("div", "vista cuaderno-card", "");
  card.dataset.ruta = ruta;
  const bar = el("div", "vista-bar", "");
  const titulo = el("b", "", "📓 " + (m.titulo || "notebook"));
  const info = el("span", "plan-estado", `${m.total || 0} ${tt("cuadernoCeldas")}${m.lenguaje ? " · " + m.lenguaje : ""}`);
  const abrir = el("button", "btn", tt("vistaAbrir"));
  abrir.type = "button";
  abrir.onclick = () => invoke("open_vista", { path: ruta }).catch((e) => toast("✗ " + e, 5000));
  bar.append(titulo, info, abrir);
  const lista = el("div", "cuaderno-celdas", "");
  let marcada = null;
  for (const c of Array.isArray(m.celdas) ? m.celdas : []) {
    const codigo = c.tipo === "code";
    const d = el("div", "cuaderno-celda" + (codigo ? " codigo" : " texto") + (m.marcada === c.n ? " marcada" : ""), "");
    d.append(el("span", "cuaderno-n", codigo ? `[${c.cuenta == null ? " " : c.cuenta}]` : String(c.n)));
    const cuerpo = el("div", "cuaderno-cuerpo", "");
    if (c.tipo === "markdown" && typeof window.mdRender === "function") {
      const md = el("div", "cuaderno-md md", "");
      md.innerHTML = window.mdRender(String(c.fuente || ""));
      cuerpo.append(md);
    } else {
      cuerpo.append(el("pre", "cuaderno-fuente", String(c.fuente || "")));
    }
    for (const s of Array.isArray(c.salidas) ? c.salidas : []) {
      if (s.imagen && /^data:image\/(png|jpeg);base64,/.test(s.imagen)) {
        const img = el("img", "cuaderno-img", "");
        img.alt = tt("cuadernoImagen");
        img.loading = "lazy";
        img.src = s.imagen;
        cuerpo.append(img);
      } else if (s.error) {
        const e = el("div", "cuaderno-error", "");
        e.append(el("b", "", "✗ " + s.error));
        if (s.traza) e.append(el("pre", "", s.traza));
        cuerpo.append(e);
      } else if (s.texto) {
        cuerpo.append(el("pre", "cuaderno-salida" + (s.flujo === "stderr" ? " stderr" : "") + (s.resultado ? " resultado" : ""), s.texto));
      }
    }
    d.append(cuerpo);
    if (m.marcada === c.n) marcada = d;
    lista.append(d);
  }
  card.append(bar, lista);
  const prev = [...log.querySelectorAll(".cuaderno-card")].find((x) => x.dataset.ruta === ruta);
  stick(() => {
    if (prev) prev.remove();
    log.appendChild(card);
  });
  // la celda tocada a la vista dentro de su tarjeta (sin mover la conversación)
  if (marcada) lista.scrollTop = Math.max(0, marcada.offsetTop - lista.offsetTop - 24);
}
