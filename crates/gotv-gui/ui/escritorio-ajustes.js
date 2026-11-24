// F16.2 · F16.3 · F16.4 en Configuración: agentes a medida (lista), proyecto remoto (crear) y variables del
// proyecto (nombres; el valor va al Llavero y no vuelve). Se registran con gotvAjustes.registrar (F8);
// también gotvEscritorioAjustes.{variables,remoto,agentes}(cont) para montarlas en otro sitio.
// Pulido (2026-11-18): las tres se pintan con las FILAS del rediseño (`setFila`/`setEstado` de app.js).
(() => {
  const inv = (c, a) => window.__TAURI__.core.invoke(c, a);
  const aviso = (t) => (typeof toast === "function" ? toast(String(t), 5000) : console.log(t));
  // la página de demostración (escritorio.html) no carga app.js: lo mínimo para pintar igual allí
  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
  const setFila = window.setFila || ((id, nombre, nota, control, clase) => `<div class="fila${clase ? " " + clase : ""}"${id ? ` data-fila="${esc(id)}"` : ""}><div class="fila-txt"><div class="fila-nom">${nombre}</div>${nota ? `<div class="fila-nota">${nota}</div>` : ""}</div>${control ? `<div class="fila-ctl">${control}</div>` : ""}</div>`);
  const setEstado = window.setEstado || ((texto, tono) => `<span class="estado"${tono ? ` data-tono="${tono}"` : ""}>${esc(texto)}</span>`);

  // cada variable, una fila: su nombre a la izquierda; «también en la caja» y «Quitar» (lo último) a la
  // derecha. Debajo, «Nueva variable» en filas.
  async function variables(cont) {
    cont.innerHTML = `<h3 class="esc-tit">${esc(tt("varTitulo"))}</h3><p class="esc-nota">${esc(tt("varNota"))}</p><div class="var-lista"></div>
      <div class="cx-form" data-fila="var-nueva"><h4>${esc(tt("flVarNueva"))}</h4>
        <label class="f"><span>${esc(tt("flVarNombre"))}</span><input class="var-nombre mono" autocapitalize="characters" autocomplete="off" spellcheck="false" placeholder="${esc(tt("varNombre"))}"></label>
        <label class="f"><span>${esc(tt("flVarValor"))}</span><input class="var-valor" type="password" autocomplete="off" placeholder="${esc(tt("varValor"))}"></label>
        ${setFila("", esc(tt("varCaja")), "", `<input type="checkbox" class="interruptor var-caja-nueva">`)}
        <div class="actions"><button class="btn primary var-guardar" type="button">${esc(tt("varGuardar"))}</button></div></div>`;
    const lista = cont.querySelector(".var-lista");
    const n = cont.querySelector(".var-nombre"), v = cont.querySelector(".var-valor"), c = cont.querySelector(".var-caja-nueva");
    cont.querySelector(".var-guardar").onclick = async () => {
      try { await inv("entorno_guardar", { nombre: n.value.trim(), valor: v.value, caja: c.checked }); n.value = ""; v.value = ""; c.checked = false; pintar(); }
      catch (e) { v.value = ""; aviso(e); }
    };
    async function pintar() {
      const vs = await inv("entorno_lista").catch(() => []);
      lista.innerHTML = vs.length ? vs.map((x, i) => setFila("", `<code>${esc(x.nombre)}</code>`, "",
        `<label class="switch-mini"><input type="checkbox" data-caja="${i}"${x.caja ? " checked" : ""}>${esc(tt("varCaja"))}</label><button class="btn sm danger" data-quita="${i}" type="button">${esc(tt("varQuitar"))}</button>`, "item")).join("")
        : `<p class="note">${esc(tt("varNinguna"))}</p>`;
      lista.querySelectorAll("[data-caja]").forEach((k) => (k.onchange = () => inv("entorno_caja", { nombre: vs[+k.dataset.caja].nombre, caja: k.checked }).catch(aviso)));
      lista.querySelectorAll("[data-quita]").forEach((b) => (b.onclick = () => inv("entorno_quitar", { nombre: vs[+b.dataset.quita].nombre }).then(pintar).catch(aviso)));
    }
    pintar();
  }

  // crear un proyecto remoto: cuatro filas (nombre, destino, carpeta, llave) y «Crear y abrir»
  function remoto(cont) {
    const campo = (k, nom) => `<label class="f"><span>${esc(tt(nom))}</span><input class="rem-campo" data-k="${k}" placeholder="${esc(tt(k))}" autocomplete="off" spellcheck="false"></label>`;
    cont.innerHTML = `<h3 class="esc-tit">${esc(tt("remTitulo"))}</h3><p class="esc-nota">${esc(tt("remNota"))}</p>
      <div class="cx-form" data-fila="remoto-crear">${campo("remNombre", "flRemNombre")}${campo("remDestino", "flRemDestino")}${campo("remRaiz", "flRemRaiz")}${campo("remLlave", "flRemLlave")}
      <div class="actions"><button class="btn primary" type="button">${esc(tt("remCrear"))}</button></div></div>`;
    cont.querySelector(".actions .btn").onclick = async () => {
      const [nombre, destino, raiz, llave] = ["remNombre", "remDestino", "remRaiz", "remLlave"].map((k) => cont.querySelector(`[data-k="${k}"]`).value.trim());
      try {
        const ancla = await inv("remoto_crear", { nombre, destino, raiz, llave: llave || null });
        await inv("open_project", { path: ancla }).catch(() => {});
      } catch (e) { aviso(e); }
    };
  }

  // cada agente a medida, una fila: nombre y descripción a la izquierda; herramientas y modelo, en texto, a la
  // derecha (antes con 🔧 y 🧠); los que no se pudieron leer, con un punto
  async function agentes(cont) {
    const r = await inv("agentes_lista").catch(() => ({ agentes: [], malos: [] }));
    cont.innerHTML = `<h3 class="esc-tit">${esc(tt("agTitulo"))}</h3>` +
      (r.agentes.length ? r.agentes.map((a) => setFila("", esc(a.nombre), esc(a.descripcion || ""),
        `<span class="fila-valor">${esc(tt("flAgHerr"))}: ${esc((a.herramientas || [tt("agTodas")]).join(", "))} · ${esc(tt("flAgModelo"))}: ${esc(a.modelo || tt("agDelChat"))}</span>`, "item")).join("")
        : `<p class="esc-nota">${esc(tt("agNinguno"))}</p>`) +
      r.malos.map((m) => `<p>${setEstado(m, "warn")}</p>`).join("");
  }

  // las secciones viven en la estructura de Rust (ventana.rs): Conexiones → Proyecto remoto, Seguridad →
  // Variables del proyecto, Agente → Agentes a medida. El `gotvAjustes` de F8 las pinta (no se pisa).
  if (window.gotvAjustes && gotvAjustes.registrar) {
    gotvAjustes.registrar("conex/remoto", remoto);
    gotvAjustes.registrar("seguridad/variables", variables);
    gotvAjustes.registrar("general/agentes", agentes);
  }
  window.gotvEscritorioAjustes = { variables, remoto, agentes };
})();
