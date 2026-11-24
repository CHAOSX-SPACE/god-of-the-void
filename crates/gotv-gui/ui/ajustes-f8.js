// F8.0 · Lo que Configuración gana al reagrupar: la pestaña «Automatizaciones» (F2.3), la sección «Sitios
// del navegador» de Seguridad (F7) y «Notificaciones» de General (F2.1). Los datos y las reglas, en Rust
// (`automatizaciones_*`, `sitios_*`, `avisos_*`); aquí sólo se pintan y se mandan los clics.
// Pulido (2026-11-18): cada cosa es una FILA del rediseño (`setFila`/`setEstado` de app.js), sin ✓/✗.
(() => {
  const inv = (c, a) => window.__TAURI__.core.invoke(c, a);

  // cada tarea programada, una fila: nombre, horario, orden y su última vuelta a la izquierda; el interruptor
  // (activa) a la derecha y «Quitar» debajo, lo último
  async function automatizaciones(pane) {
    pane.innerHTML = `<h3>${esc(tt("sAutomat"))}</h3><div class="auto-lista"></div>`;
    const caja = pane.querySelector(".auto-lista");
    let e;
    try { e = await inv("automatizaciones_lista"); } catch (x) { caja.innerHTML = setError(x); return; }
    const v = e.tareas;
    caja.insertAdjacentHTML("beforebegin", `<p>${setEstado(tt(e.agente ? "atAgenteSi" : "atAgenteNo"), e.agente ? "ok" : "warn")}</p>`);
    if (!v.length) { caja.innerHTML = `<p class="note">${esc(tt("atVacio"))}</p>`; return; }
    for (const t of v) {
      const f = el("div", "auto-fila", "");
      const meta = [t.proyecto, t.proxima && `${tt("atProxima")} ${t.proxima}`, t.ultima && `${tt("atUltima")} ${t.ultima}${t.estado ? " · " + t.estado : ""}`].filter(Boolean).join(" · ");
      f.innerHTML = setFila("", `${esc(t.nombre || t.id)} <code class="mono">${esc(t.cron)}</code>`, `${esc(t.orden)}<br>${esc(meta)}`,
        `<input type="checkbox" class="interruptor" aria-label="${esc(tt("atActiva"))}"${t.activa ? " checked" : ""}>`, "item") +
        `<div class="cx-acc"><button type="button" class="btn danger">${esc(tt("atQuitar"))}</button></div>`;
      f.querySelector("input").onchange = async (ev) => { try { await inv("automatizacion_activar", { id: t.id, activa: ev.target.checked }); automatizaciones(pane); } catch (x) { setFallo(x); } };
      f.querySelector(".danger").onclick = async () => {
        if (!(await confirmBox(tt("atQuitar"), `${t.nombre || t.id}\n\n${tt("atQuitarQ")}`, tt("atQuitar")))) return;
        try { await inv("automatizacion_quitar", { id: t.id }); automatizaciones(pane); } catch (x) { setFallo(x); }
      };
      caja.appendChild(f);
    }
  }

  // las reglas de sitios: una fila «bloque» (la explicación arriba, el texto y «Guardar» debajo)
  async function sitios(pane) {
    let t = "";
    try { t = await inv("sitios_texto"); } catch (x) { pane.innerHTML = setError(x); return; }
    pane.innerHTML = setFila("sitios", esc(tt("flSitiosNom")), esc(tt("sitAyuda")),
      `<textarea id="sitTxt" rows="8" spellcheck="false" class="mono" placeholder="abrir permitir docs.rs&#10;descargar negar *"></textarea><button class="btn primary" type="button" id="sitSave">${esc(tt("save"))}</button>`, "bloque");
    pane.querySelector("#sitTxt").value = t;
    pane.querySelector("#sitSave").onclick = async () => {
      try { await inv("sitios_guardar", { texto: pane.querySelector("#sitTxt").value }); toast(tt("saved")); } catch (x) { toast(String(x), 7000); }
    };
  }

  // los avisos: tres filas con su interruptor; se guardan al cambiar
  async function avisos(pane) {
    let a;
    try { a = await inv("avisos_estado"); } catch (x) { pane.innerHTML = setError(x); return; }
    const sw = (k) => `<input type="checkbox" class="interruptor" data-k="${k}"${a[k] ? " checked" : ""}>`;
    pane.innerHTML = setFila("aviso-turno", esc(tt("avTurno")), "", sw("turno")) + setFila("aviso-permiso", esc(tt("avPermiso")), "", sw("permiso")) +
      setFila("aviso-programadas", esc(tt("avProgramadas")), "", sw("programadas")) + `<p class="note">${esc(tt("avAyuda"))}</p>`;
    pane.querySelectorAll("input[data-k]").forEach((i) => (i.onchange = async () => {
      const v = {}; pane.querySelectorAll("input[data-k]").forEach((x) => (v[x.dataset.k] = x.checked));
      try { await inv("avisos_guardar", v); toast(tt("saved")); } catch (x) { setFallo(x); }
    }));
  }

  gotvAjustes.registrar("automatizaciones", automatizaciones);
  gotvAjustes.registrar("seguridad/sitios", sitios);
  gotvAjustes.registrar("general/avisos", avisos);
})();
