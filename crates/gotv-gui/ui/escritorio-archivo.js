// F16.7 · Pestaña «Archivo»: ver y editar un archivo del proyecto. Rust decide todo (dentro del proyecto,
// versión para no pisar lo que cambió el agente, escritura atómica); esto sólo pinta un área de texto.
// Montaje: gotvArchivo.montar(contenedor) · abrir una ruta desde fuera: gotvArchivo.abrir("src/main.rs").
(() => {
  const inv = (c, a) => window.__TAURI__.core.invoke(c, a);
  const h = (tag, cls, txt) => { const e = document.createElement(tag); if (cls) e.className = cls; if (txt != null) e.textContent = txt; return e; };
  let ui = null;
  let actual = null; // {ruta, version, texto}

  function estado(t, mal) {
    ui.estado.textContent = t;
    ui.estado.classList.toggle("mal", !!mal);
  }

  async function abrir(ruta) {
    if (!ui) return;
    ui.ruta.value = ruta;
    try {
      const a = await inv("archivo_leer", { ruta });
      actual = { ruta: a.ruta, version: a.version, texto: a.texto };
      ui.area.value = a.texto;
      ui.area.dataset.lenguaje = a.lenguaje;
      estado(tt("arcSinCambios"));
    } catch (e) {
      actual = null;
      ui.area.value = "";
      estado("✗ " + e, true);
    }
  }

  async function guardar() {
    if (!ui) return;
    const ruta = (actual && actual.ruta) || ui.ruta.value.trim();
    if (!ruta) return;
    try {
      const v = await inv("archivo_guardar", { ruta, texto: ui.area.value, version: actual ? actual.version : null });
      actual = { ruta, version: v, texto: ui.area.value };
      estado(tt("arcGuardado"));
    } catch (e) {
      estado("✗ " + e, true);
    }
  }

  function montar(cont) {
    cont.innerHTML = "";
    const barra = h("div", "arc-barra");
    const ruta = h("input", "arc-ruta");
    ruta.placeholder = tt("arcRuta");
    ruta.onkeydown = (e) => { if (e.key === "Enter") abrir(ruta.value.trim()); };
    const bA = h("button", "btn sm", tt("arcAbrir"));
    bA.onclick = () => abrir(ruta.value.trim());
    const bG = h("button", "btn sm primary", tt("arcGuardar"));
    bG.onclick = guardar;
    const est = h("span", "arc-estado", "");
    barra.append(ruta, bA, bG, est);
    const area = h("textarea", "arc-area");
    area.spellcheck = false;
    area.oninput = () => estado(actual && area.value === actual.texto ? tt("arcSinCambios") : "● " + tt("arcCambiado"));
    area.onkeydown = (e) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "s") { e.preventDefault(); guardar(); }
      if (e.key === "Tab") { e.preventDefault(); area.setRangeText("    ", area.selectionStart, area.selectionEnd, "end"); area.oninput(); }
    };
    cont.append(barra, area);
    ui = { ruta, area, estado: est };
  }

  window.gotvArchivo = { montar, abrir, guardar };
  // F8: la pestaña «Archivo» del panel de trabajo
  if (window.gotvPanel) gotvPanel.registrar("archivo", { montar, ancho: 560 });
})();
