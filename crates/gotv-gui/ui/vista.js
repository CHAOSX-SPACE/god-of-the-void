// F8.7 · Pestaña «Vista» del panel de trabajo: el servidor del proyecto, aquí mismo. Rust valida que la
// dirección sea de ESTA Mac (`vista_url`) y redacta el pedido de revisión (`vista_pedir_revision`); la CSP
// de la ventana sólo deja enmarcar localhost. Otras piezas (plan de F7, dispositivo de F18) pueden poner su
// contenido con `gotvVista.poner(nodo, titulo)`.
(() => {
  const inv = (c, a) => window.__TAURI__.core.invoke(c, a);
  let nodo = null, url = "";

  function vacio() {
    if (!nodo) return;
    url = "";
    nodo.innerHTML = `<div class="vs-vacio">${esc(tt("vsVacio"))}</div>`;
  }

  function pintar() {
    if (!nodo) return;
    // sin dirección no hay marco (un src vacío cargaría la propia ventana dentro)
    if (!url) return vacio();
    nodo.innerHTML = `<div class="vs-barra"><span class="vs-url mono" title=""></span>
      <button type="button" class="icon-btn" data-a="recargar" title="${esc(tt("vsRecargar"))}" aria-label="${esc(tt("vsRecargar"))}">↻</button>
      <button type="button" class="icon-btn" data-a="fuera" title="${esc(tt("vsAbrirFuera"))}" aria-label="${esc(tt("vsAbrirFuera"))}">↗</button>
      <button type="button" class="btn vs-rev" data-a="revisar">${esc(tt("vsRevisar"))}</button>
      <button type="button" class="icon-btn" data-a="cerrar" title="${esc(tt("vsCerrar"))}" aria-label="${esc(tt("vsCerrar"))}">✕</button></div>
      <iframe class="vs-marco" referrerpolicy="no-referrer"></iframe>`;
    nodo.querySelector(".vs-url").textContent = url;
    nodo.querySelector(".vs-url").title = url;
    const marco = nodo.querySelector("iframe");
    marco.src = url;
    nodo.querySelector('[data-a="recargar"]').onclick = () => { marco.src = "about:blank"; setTimeout(() => (marco.src = url), 30); };
    nodo.querySelector('[data-a="fuera"]').onclick = () => inv("open_link", { url });
    nodo.querySelector('[data-a="cerrar"]').onclick = vacio;
    nodo.querySelector('[data-a="revisar"]').onclick = async () => {
      try {
        const t = await inv("vista_pedir_revision", { url });
        input.value = input.value.trim() ? `${input.value.trim()}\n\n${t}` : t;
        fit(); refreshSend(); input.focus();
      } catch (x) { toast("✗ " + x, 5000); }
    };
  }

  window.gotvVista = {
    async abrir(u) {
      let ok;
      try { ok = await inv("vista_url", { url: u }); } catch (x) { return toast("✗ " + x, 6000); }
      // primero se monta la pestaña (montar deja el aviso de vacío) y DESPUÉS se pone la dirección
      gotvPanel.abrir("vista");
      url = ok;
      pintar();
    },
    poner(contenido, titulo) {
      gotvPanel.abrir("vista");
      if (!nodo) return;
      url = "";
      nodo.innerHTML = "";
      if (titulo) nodo.appendChild(el("div", "vs-barra", titulo));
      nodo.appendChild(contenido);
    },
  };

  gotvPanel.registrar("vista", { ancho: 560, montar(n) { nodo = n; n.classList.add("vista-pane"); if (!url) vacio(); } });

  // «Ver aquí» junto a cada dirección local de un proceso (Tareas)
  const lista = document.getElementById("taskList");
  if (lista) lista.addEventListener("click", (e) => {
    const b = e.target.closest(".pver");
    if (b) { e.stopPropagation(); gotvVista.abrir(b.dataset.url); }
  });
})();
