// F8.2 · Pestaña «Cambios» del panel de trabajo. Rust da todo (`cambios_turnos`, `cambios_turno`,
// `cambios_git`, `cambios_revertir`, `cambios_comentar`): la lista por turno sale de los puntos de
// restauración (funciona SIN git) y enseña también lo que cambió la terminal. Aquí sólo se pinta y se
// mandan los clics.
(() => {
  const inv = (c, a) => window.__TAURI__.core.invoke(c, a);
  const V = { vista: "turno", todos: false, git: false, base: "main", turno: null, archivos: null, archivo: null, comentarios: [] };
  let nodo = null;

  const estadoTxt = (e) => tt({ nuevo: "cbNuevo", cambiado: "cbCambiado", borrado: "cbBorrado" }[e] || e);
  const cuenta = (c) => `<span class="cb-mas">+${c.mas}</span> <span class="cb-menos">−${c.menos}</span>`;

  function cabecera() {
    const segs = [["turno", "cbPorTurno"]].concat(V.git ? [["sin_confirmar", "cbSinConfirmar"], ["rama", "cbRama"]] : []);
    return `<div class="cb-head"><div class="cb-segs" role="tablist">${segs.map(([v, k]) => `<button type="button" role="tab" data-v="${v}" class="${V.vista === v ? "on" : ""}" aria-selected="${V.vista === v}">${esc(tt(k))}</button>`).join("")}</div>
      ${V.vista === "turno" ? `<label class="cb-todos"><input type="checkbox" id="cbTodos"${V.todos ? " checked" : ""}> ${esc(tt("cbTodos"))}</label>` : ""}
      ${V.vista === "rama" ? `<label class="cb-todos">${esc(tt("cbBase"))} <input type="text" id="cbBase" class="mono" value="${esc(V.base)}" size="10"></label>` : ""}</div>
      ${V.git ? "" : `<div class="cb-nota">${esc(tt("cbSinGit"))}</div>`}
      <div class="cb-coms"></div><div class="cb-cuerpo"></div>`;
  }

  async function pintar() {
    if (!nodo) return;
    nodo.innerHTML = cabecera();
    nodo.querySelectorAll(".cb-segs button").forEach((b) => (b.onclick = () => { V.vista = b.dataset.v; V.turno = null; V.archivos = null; V.archivo = null; pintar(); }));
    const t = nodo.querySelector("#cbTodos");
    if (t) t.onchange = () => { V.todos = t.checked; pintar(); };
    const base = nodo.querySelector("#cbBase");
    if (base) base.onchange = () => { V.base = base.value.trim() || "main"; V.archivos = null; pintar(); };
    pintarComentarios();
    const c = nodo.querySelector(".cb-cuerpo");
    if (V.archivo) return pintarDiff(c);
    if (V.vista === "turno" && V.turno == null) return pintarTurnos(c);
    if (!V.archivos) {
      try {
        V.archivos = V.vista === "turno" ? await inv("cambios_turno", { n: V.turno.n }) : await inv("cambios_git", { vista: V.vista, base: V.base });
      } catch (x) { c.innerHTML = `<div class="cb-nota">✗ ${esc(String(x))}</div>`; return; }
    }
    pintarArchivos(c);
  }

  async function pintarTurnos(c) {
    let r;
    try { r = await inv("cambios_turnos", { todos: V.todos }); } catch (x) { c.innerHTML = `<div class="cb-nota">✗ ${esc(String(x))}</div>`; return; }
    if (r.git !== V.git) { V.git = r.git; return pintar(); }
    if (!r.turnos.length) { c.innerHTML = `<div class="cb-nota">${esc(tt("cbVacio"))}</div>`; return; }
    c.innerHTML = "";
    for (const t of r.turnos) {
      const b = el("button", "cb-turno", "");
      b.type = "button";
      b.innerHTML = `<span class="tt">${esc(t.orden || "· " + tt("cbSinAnotar"))}</span><span class="mm">${esc(t.fecha || "")} · ${t.archivos} ${esc(tt("cbArchivos"))}</span>`;
      b.onclick = () => { V.turno = t; V.archivos = null; pintar(); };
      c.appendChild(b);
    }
  }

  function pintarArchivos(c) {
    c.innerHTML = "";
    if (V.vista === "turno") {
      const v = el("button", "cb-volver", tt("cbVolver"));
      v.type = "button"; v.onclick = () => { V.turno = null; V.archivos = null; pintar(); };
      c.appendChild(v);
      c.appendChild(el("div", "cb-titulo", V.turno.orden || tt("cbSinAnotar")));
    }
    if (!V.archivos.length) { c.appendChild(el("div", "cb-nota", tt("cbSinCambios"))); return; }
    const hayMovido = V.archivos.some((a) => a.estado === "borrado") && V.archivos.some((a) => a.estado === "nuevo");
    for (const a of V.archivos) {
      const f = el("div", "cb-arch", "");
      const marcas = [a.por_comando && tt("cbComando"), a.binario && tt("cbBinario"), a.credencial && tt("cbCredencial"), a.generado && tt("cbGenerado")].filter(Boolean);
      f.innerHTML = `<button type="button" class="cb-abrir"><span class="cb-est ${a.estado}">${esc(estadoTxt(a.estado))}</span><span class="ruta mono">${esc(a.ruta)}</span>${cuenta(a)}</button>${marcas.map((m) => `<span class="cb-marca">${esc(m)}</span>`).join("")}`;
      f.querySelector(".cb-abrir").onclick = () => { V.archivo = a; pintar(); };
      if (V.vista === "turno") {
        const r = el("button", "cb-rev", "↩");
        r.type = "button"; r.title = tt("cbRevertir"); r.setAttribute("aria-label", tt("cbRevertir"));
        r.onclick = () => revertir(a);
        f.appendChild(r);
      }
      c.appendChild(f);
    }
    if (hayMovido && V.vista === "turno") c.appendChild(el("div", "cb-nota", tt("cbRenombrado")));
    if (V.vista === "sin_confirmar") pintarConfirmar(c);
  }

  // F8.2 · Confirmar lo marcado (commit). Las credenciales nunca vienen marcadas; Rust decide y comprueba
  // todo (`cambios_confirmar`); sin push (en pausa hasta producción).
  function pintarConfirmar(c) {
    const filas = c.querySelectorAll(".cb-arch");
    V.archivos.forEach((a, i) => {
      const k = el("input", "cb-marcar", "");
      k.type = "checkbox"; k.checked = !a.credencial; k.disabled = !!a.credencial; k.dataset.ruta = a.ruta;
      k.setAttribute("aria-label", a.ruta);
      filas[i] && filas[i].prepend(k);
    });
    const f = el("div", "cb-confirmar", "");
    f.innerHTML = `<textarea rows="2" class="mono" placeholder="${esc(tt("cbMensajePh"))}" aria-label="${esc(tt("cbMensajePh"))}"></textarea><button type="button" class="btn primary">${esc(tt("cbConfirmar"))}</button>`;
    f.querySelector("button").onclick = async () => {
      const rutas = [...c.querySelectorAll(".cb-marcar:checked")].map((k) => k.dataset.ruta);
      try { toast("✓ " + (await inv("cambios_confirmar", { rutas, mensaje: f.querySelector("textarea").value })), 6000); V.archivos = null; pintar(); }
      catch (x) { toast("✗ " + x, 7000); }
    };
    c.appendChild(f);
  }

  async function revertir(a) {
    if (!(await confirmBox(tt("cbRevertir"), `${a.ruta}\n\n${tt("cbRevertirQ")}`, tt("cbRevertir")))) return;
    try { toast("✓ " + (await inv("cambios_revertir", { n: V.turno.n, ruta: a.ruta })), 6000); }
    catch (x) {
      // cambió DESPUÉS del turno: se pregunta otra vez, en serio
      if (!/DESPU|AFTER/.test(String(x))) return toast("✗ " + x, 6000);
      if (!(await confirmBox(tt("cbRevertir"), `${a.ruta}\n\n${tt("cbForzarQ")}`, tt("cbRevertir")))) return;
      try { toast("✓ " + (await inv("cambios_revertir", { n: V.turno.n, ruta: a.ruta, forzar: true })), 6000); } catch (y) { toast("✗ " + y, 6000); }
    }
  }

  function pintarDiff(c) {
    const a = V.archivo;
    c.innerHTML = "";
    const v = el("button", "cb-volver", tt("cbVolverArch"));
    v.type = "button"; v.onclick = () => { V.archivo = null; pintar(); };
    c.appendChild(v);
    const t = el("div", "cb-titulo mono", "");
    t.innerHTML = `${esc(a.ruta)} · ${cuenta(a)}`;
    c.appendChild(t);
    const avisos = [a.credencial && tt("cbCredencial"), a.binario && tt("cbBinario"), a.generado && tt("cbGenerado"), a.por_comando && tt("cbComando")].filter(Boolean);
    for (const x of avisos) c.appendChild(el("div", "cb-nota", x));
    const tabla = el("div", "cb-diff mono", "");
    tabla.setAttribute("role", "table");
    for (const tr of a.trozos) {
      tabla.appendChild(el("div", "cb-hunk", `@@ −${tr.desde_viejo} +${tr.desde_nuevo} @@`));
      let o = tr.desde_viejo, n = tr.desde_nuevo;
      for (const [s, txt] of tr.lineas) {
        const fila = el("div", "cb-l " + (s === "+" ? "add" : s === "-" ? "del" : "eq"), "");
        const nv = s === "+" ? "" : o, nn = s === "-" ? "" : n;
        fila.innerHTML = `<span class="no">${nv}</span><span class="nn">${nn}</span><span class="sg">${s === " " ? "" : esc(s)}</span><span class="tx"></span>`;
        fila.querySelector(".tx").textContent = txt;
        // se comenta una línea del lado NUEVO (la que el agente verá en el archivo)
        if (s !== "-") {
          const linea = n;
          fila.classList.add("comentable");
          fila.title = tt("cbComentar");
          fila.onclick = (e) => { if (window.getSelection().isCollapsed) comentar(fila, a.ruta, linea); else e.stopPropagation(); };
        }
        if (s !== "+") o++;
        if (s !== "-") n++;
        tabla.appendChild(fila);
      }
    }
    // citar lo seleccionado del diff (F8.3)
    tabla.oncontextmenu = (e) => {
      const sel = window.getSelection();
      if (!sel || sel.isCollapsed || !tabla.contains(sel.anchorNode)) return;
      e.preventDefault();
      window.gotvCitar && gotvCitar(a.ruta, sel.toString());
    };
    if (a.recortado) tabla.appendChild(el("div", "cb-nota", `${tt("cbRecortado")} ${a.recortado}`));
    c.appendChild(tabla);
  }

  function comentar(fila, ruta, linea) {
    if (fila.nextElementSibling && fila.nextElementSibling.classList.contains("cb-com-nuevo")) return;
    const caja = el("div", "cb-com-nuevo", "");
    caja.innerHTML = `<textarea rows="2" placeholder="${esc(tt("cbComentarPh"))}"></textarea><div class="actions"><button class="btn" type="button" data-a="no">${esc(tt("cancel"))}</button><button class="btn primary" type="button" data-a="si">${esc(tt("cbComentar"))}</button></div>`;
    fila.after(caja);
    const ta = caja.querySelector("textarea");
    ta.focus();
    ta.onkeydown = (e) => { e.stopPropagation(); if (e.key === "Escape") caja.remove(); if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) caja.querySelector('[data-a="si"]').click(); };
    caja.querySelector('[data-a="no"]').onclick = () => caja.remove();
    caja.querySelector('[data-a="si"]').onclick = async () => {
      try { V.comentarios = await inv("cambios_comentar", { ruta, linea, texto: ta.value }); caja.remove(); pintarComentarios(); }
      catch (x) { toast("✗ " + x, 5000); }
    };
  }

  function pintarComentarios() {
    const c = nodo && nodo.querySelector(".cb-coms");
    if (!c) return;
    if (!V.comentarios.length) { c.innerHTML = ""; return; }
    c.innerHTML = `<div class="cb-coms-t">${V.comentarios.length} ${esc(tt("cbComentarios"))}</div>`;
    V.comentarios.forEach((k, i) => {
      const f = el("div", "cb-com", "");
      f.innerHTML = `<span class="mono">${esc(k.ruta)} L${k.linea}</span> <span class="tx"></span><button type="button" class="cb-quitar">${esc(tt("cbQuitar"))}</button>`;
      f.querySelector(".tx").textContent = k.texto;
      f.querySelector("button").onclick = async () => { V.comentarios = await inv("cambios_comentarios", { quitar: i }); pintarComentarios(); };
      c.appendChild(f);
    });
  }

  gotvPanel.registrar("cambios", {
    ancho: 480,
    montar(n) { nodo = n; n.classList.add("cambios"); },
    mostrar() { V.archivos = V.archivo ? V.archivos : null; inv("cambios_comentarios", {}).then((c) => { V.comentarios = c; pintar(); }).catch(() => pintar()); },
  });

  window.__TAURI__.event.listen("gotv", ({ payload: m }) => {
    if (m.kind === "comentarios") { V.comentarios = m.lista || []; pintarComentarios(); if (m.enviados) toast(`✓ ${m.enviados} ${tt("cbEnviados")}`); }
    // un turno terminado o un chat distinto: la lista por turno cambió
    if ((m.kind === "done" || m.kind === "project") && nodo && gotvPanel.abierto() && gotvPanel.actual() === "cambios" && !V.archivo) { V.turno = m.kind === "project" ? null : V.turno; V.archivos = null; pintar(); }
  });
})();
