// Ola EXPERTO II · X2-6 — el LADO: la IA del estudio y el inspector.
//
// La IA manda: cada acción (generar una pista, rehacer un tramo, extender, versionar, separar, letra,
// masterizar) toma el contexto de la sesión —la pista y el tramo elegidos, el tempo, la tonalidad, la letra— y
// lo que devuelve entra como pistas, clips o tomas normales: se mezclan, se comparan y se deshacen. Debajo,
// el chat: órdenes en lenguaje natural que el modelo local convierte en operaciones validadas por el núcleo.
(() => {
  const X = EX.X, est = EX.est, op = EX.op;
  const L = (EX.lado = {});
  X.ia = { abierta: "generar", f: {}, resultado: {}, chat: [] };
  const IDIOMAS = [["", "exIdiomaAuto"], ["es", "Español"], ["en", "English"], ["pt", "Português"], ["fr", "Français"], ["it", "Italiano"], ["de", "Deutsch"], ["ja", "日本語"], ["ko", "한국어"], ["zh", "中文"]];
  // el formulario de cada acción se guarda al escribir: repintar no borra lo tecleado
  const F = (accion) => (X.ia.f[accion] ||= {});
  const val = (accion, k, d) => (F(accion)[k] !== undefined ? F(accion)[k] : d);

  L.montar = () => {
    $("exLado").innerHTML = `<div class="ex-lado-cab"><div class="ex-pestanas" role="tablist">
        <button type="button" role="tab" data-l="ia">${EX.icono(EX.I.chispa)}<span>${esc(tt("exIA"))}</span></button><button type="button" role="tab" data-l="inspector"><span>${esc(tt("exInspector"))}</span></button></div></div>
      <div class="ex-lado-cuerpo" id="exLadoCuerpo"></div>`;
    $("exLado").querySelectorAll("[data-l]").forEach((b) => (b.onclick = () => { X.lado = b.dataset.l; EX.recordar("lado", X.lado); L.pintar(true); }));
  };
  L.abrirIA = (accion) => { X.lado = "ia"; X.ia.abierta = accion; EX.recordar("lado", "ia"); EX.pintar(); L.pintar(true); const t = document.querySelector(`.ex-accion[data-a="${accion}"] textarea, .ex-accion[data-a="${accion}"] button.primario`); if (t) t.focus(); };
  L.pintar = (forzar) => {
    const c = $("exLadoCuerpo"); if (!c) return;
    $("exLado").querySelectorAll("[data-l]").forEach((b) => { b.classList.toggle("on", b.dataset.l === X.lado); b.setAttribute("aria-selected", String(b.dataset.l === X.lado)); });
    const escribiendo = c.contains(document.activeElement) && /^(TEXTAREA|INPUT|SELECT)$/.test(document.activeElement.tagName);
    if (X.lado === "ia") {
      // mientras se escribe no se rehace el panel: sólo el contexto y los botones
      if (!forzar && escribiendo && c.querySelector(".ex-ia")) return refrescarIA(c);
      pintarIA(c);
    } else if (!forzar && escribiendo && c.querySelector(".ex-insp")) return;
    else pintarInspector(c);
  };

  // ── la IA ───────────────────────────────────────────────────────────────
  const pistaSel = () => (X.pistaSel && EX.pista(X.pistaSel)) || null;
  const tramo = () => (X.sel && X.sel.hasta > X.sel.desde && EX.pista(X.sel.pista) ? X.sel : null);
  function contexto() {
    const e = est(), p = pistaSel(), s = tramo();
    const partes = [p ? `<b style="color:${EX.colorDe(p)}">${p.id} · ${esc(p.nombre)}</b>` : `<i>${esc(tt("exSinPista"))}</i>`];
    if (s) partes.push(`<span class="mono">${s.desde.toFixed(2)}–${s.hasta.toFixed(2)} s</span>`);
    partes.push(`<span class="mono">${Math.round(e.bpm)} bpm · ${e.compas}/4${e.tonalidad ? " · " + esc(e.tonalidad) : ""}</span>`);
    return partes.join("<i class='ex-pt'>·</i>");
  }
  const ACCIONES = [["generar", "exIaGenerar", "exIaGenerarD"], ["rehacer", "exIaRehacer", "exIaRehacerD"], ["extender", "exIaExtender", "exIaExtenderD"], ["versionar", "exIaVersionar", "exIaVersionarD"], ["separar", "exIaSeparar", "exIaSepararD"], ["letra", "exIaLetra", "exIaLetraD"], ["masterizar", "exIaMasterizar", "exIaMasterizarD"]];
  const area = (a, k, ph, filas, d) => `<textarea data-f="${k}" rows="${filas || 2}" placeholder="${esc(tt(ph))}">${esc(val(a, k, d || ""))}</textarea>`;
  const idioma = (a) => `<label class="ex-f"><span>${esc(tt("exIdioma"))}</span><select data-f="idioma">${IDIOMAS.map(([v, t]) => `<option value="${v}"${val(a, "idioma", "") === v ? " selected" : ""}>${esc(v ? t : tt(t))}</option>`).join("")}</select></label>`;
  const letra = (a) => `<details class="ex-plegable"${val(a, "letra", est().letra) ? " open" : ""}><summary>${esc(tt("exLetraOpcional"))}</summary>${area(a, "letra", "exLetraPh", 4, a === "generar" ? "" : est().letra)}${idioma(a)}</details>`;
  const ideas = (a, lista) => `<div class="ex-ideas">${tt(lista).split("|").map((t) => `<button type="button" class="ex-idea" data-idea="${esc(t)}">${esc(t)}</button>`).join("")}</div>`;
  const ir = (a, t, puede, motivo) => `<button type="button" class="ex-bt primario ancho" data-ir="${a}"${puede && !X.trabajo ? "" : " disabled"}>${EX.icono(EX.I.chispa)}<span>${esc(tt(t))}</span></button>${puede ? "" : `<p class="ex-nota aviso" data-motivo>${esc(tt(motivo))}</p>`}`;
  function cuerpoAccion(a) {
    const e = est(), p = pistaSel(), s = tramo(), r = X.ia.resultado[a];
    const res = r ? `<p class="ex-nota ${r.mal ? "error" : "bien"}">${esc(r.texto)}</p>` : "";
    if (a === "generar") return `${area(a, "descripcion", "exDescripcionPh", 3)}${ideas(a, "exIdeasGenerar")}${letra(a)}
      <div class="ex-f2"><label class="ex-f"><span>${esc(tt("exSegundos"))}</span><input type="number" data-f="segundos" min="5" max="600" step="1" placeholder="${Math.round(EX.duracion()) || 30}" value="${val(a, "segundos", "")}"></label>
      <label class="ex-f"><span>${esc(tt("exNombrePista"))}</span><input type="text" data-f="nombre" maxlength="60" value="${esc(val(a, "nombre", ""))}"></label></div>
      <label class="pro-check"><input type="checkbox" data-f="con_tempo"${val(a, "con_tempo", true) ? " checked" : ""}> ${esc(tt("exConTempo").replace("{n}", Math.round(e.bpm)))}</label>
      <label class="pro-check"><input type="checkbox" data-f="con_tonalidad"${val(a, "con_tonalidad", !!e.tonalidad) ? " checked" : ""}${e.tonalidad ? "" : " disabled"}> ${esc(e.tonalidad ? tt("exConTonalidad").replace("{t}", e.tonalidad) : tt("exSinTonalidadAviso"))}</label>
      ${ir(a, "exIaGenerarIr", true)}${res}`;
    if (a === "rehacer") return `<p class="ex-destino">${s ? `${esc(tt("exPista"))} <b>${s.pista}</b> · <span class="mono">${s.desde.toFixed(2)}–${s.hasta.toFixed(2)} s</span>` : esc(tt("exRehacerElige"))}</p>
      ${area(a, "descripcion", "exRehacerPh", 3)}${ideas(a, "exIdeasRehacer")}${letra(a)}${ir(a, "exIaRehacerIr", !!s, "exRehacerElige")}${res}`;
    if (a === "extender") return `<label class="ex-f"><span>${esc(tt("exQueExtender"))}</span><select data-f="destino"><option value="">${esc(tt("exMezclaFinal"))}</option>${p ? `<option value="${p.id}"${String(val(a, "destino", "")) === String(p.id) ? " selected" : ""}>${esc(tt("exPista"))} ${p.id} · ${esc(p.nombre)}</option>` : ""}</select></label>
      <label class="ex-f"><span>${esc(tt("exSegundosMas"))}</span><input type="number" data-f="segundos" min="2" max="240" step="1" value="${val(a, "segundos", 30)}"></label>
      ${area(a, "descripcion", "exExtenderPh", 3)}${letra(a)}${ir(a, "exIaExtenderIr", e.pistas.length > 0, "exSinPistasAviso")}${res}`;
    if (a === "versionar") return `<p class="ex-destino">${p ? `${esc(tt("exPista"))} <b>${p.id}</b> · ${esc(p.nombre)}` : esc(tt("exElige"))}</p>
      ${area(a, "descripcion", "exVersionarPh", 3)}${ideas(a, "exIdeasVersionar")}
      <label class="ex-f"><span>${esc(tt("exFidelidad"))} <b class="mono" data-lee="fuerza">${Math.round(val(a, "fuerza", 0.6) * 100)} %</b></span><input type="range" data-f="fuerza" min="0.1" max="1" step="0.05" value="${val(a, "fuerza", 0.6)}"></label>
      <p class="ex-nota">${esc(tt("exFidelidadAyuda"))}</p>${letra(a)}${ir(a, "exIaVersionarIr", !!p, "exElige")}${res}`;
    if (a === "separar") return `<p class="ex-destino">${p ? `${esc(tt("exPista"))} <b>${p.id}</b> · ${esc(p.nombre)}` : esc(tt("exElige"))}</p>
      <div class="ex-radios">${[["4", "exSep4"], ["6", "exSep6"], ["ft", "exSepFt"], ["bateria", "exSepBateria"]].map(([v, t]) => `<label><input type="radio" name="exSepModelo" data-f="modelo" value="${v}"${val(a, "modelo", "4") === v ? " checked" : ""}> ${esc(tt(t))}</label>`).join("")}</div>
      ${ir(a, "exIaSepararIr", !!p, "exElige")}${res}`;
    if (a === "letra") return `<label class="ex-f"><span>${esc(tt("exLetraDe"))}</span><select data-f="destino"><option value="">${esc(tt("exMezclaFinal"))}</option>${p ? `<option value="${p.id}"${String(val(a, "destino", p && p.fuente === "vocals" ? String(p.id) : "")) === String(p.id) ? " selected" : ""}>${esc(tt("exPista"))} ${p.id} · ${esc(p.nombre)}</option>` : ""}</select></label>
      ${ir(a, "exIaLetraIr", e.pistas.length > 0, "exSinPistasAviso")}
      ${area(a, "texto", "exLetraSesionPh", 7, e.letra)}
      <div class="ex-botones fin"><button type="button" class="ex-bt" data-guardar-letra>${esc(tt("exGuardarLetra"))}</button></div><p class="ex-nota">${esc(tt("exLetraAyuda"))}</p>${res}`;
    return `<div class="ex-f2"><label class="ex-f"><span>${esc(tt("exObjetivo"))}</span><select data-f="lufs">${[["-14", "exExpStreaming"], ["-16", "exExpPodcast"], ["-9", "exExpFuerte"], ["-23", "exExpBroadcast"]].map(([v, t]) => `<option value="${v}"${String(val(a, "lufs", "-14")) === v ? " selected" : ""}>${v.replace("-", "−")} LUFS · ${esc(tt(t))}</option>`).join("")}</select></label>
      <label class="ex-f"><span>${esc(tt("exTecho"))}</span><input type="number" data-f="techo" min="-6" max="0" step="0.1" value="${val(a, "techo", -1)}"></label></div>
      <p class="ex-nota">${esc(tt("exMasterizarAyuda"))}</p>${ir(a, "exIaMasterizarIr", e.pistas.length > 0, "exSinPistasAviso")}${res}`;
  }
  function pintarIA(c) {
    // las siete acciones SIEMPRE a la vista (una rejilla de botones) y debajo el formulario de la elegida:
    // como acordeón, con una abierta las demás quedaban bajo el pliegue (medido en una ventana de 768 px)
    if (!ACCIONES.some(([a]) => a === X.ia.abierta)) X.ia.abierta = "generar";
    const [, titulo, descripcion] = ACCIONES.find(([a]) => a === X.ia.abierta);
    c.innerHTML = `<div class="ex-ia"><div class="ex-ia-ctx" id="exIaCtx">${contexto()}</div>
      <div class="ex-ia-menu" role="tablist" aria-label="${esc(tt("exIA"))}">${ACCIONES.map(([a, t, d], i) => `<button type="button" role="tab" class="ex-accion-cab${X.ia.abierta === a ? " on" : ""}" data-a="${a}" aria-selected="${X.ia.abierta === a}" aria-label="${esc(tt(t))}" title="${esc(tt(t) + ": " + tt(d))}">${esc(tt("exIaCortos").split("|")[i] || tt(t))}</button>`).join("")}</div>
      <div class="ex-ia-acciones"><section class="ex-accion abierta" data-a="${X.ia.abierta}"><p class="ex-accion-d"><b>${esc(tt(titulo))}.</b> ${esc(tt(descripcion))}</p><div class="ex-accion-c">${cuerpoAccion(X.ia.abierta)}</div></section></div>
      <div class="ex-chat"><div class="ex-chat-log" id="exChatLog"></div>
        <div class="ex-chat-in"><textarea id="exChatIn" rows="2" placeholder="${esc(tt("exChatPh"))}">${esc(X.ia.borrador || "")}</textarea><button type="button" class="ex-bt primario" id="exChatEnviar">${esc(tt("exChatEnviar"))}</button></div></div></div>`;
    c.querySelectorAll(".ex-accion-cab").forEach((b) => (b.onclick = () => { X.ia.abierta = b.dataset.a; L.pintar(true); }));
    const a = X.ia.abierta, caja = c.querySelector(".ex-accion.abierta");
    if (caja) {
      caja.querySelectorAll("[data-f]").forEach((x) => {
        const guardar = () => { if (x.type === "radio" && !x.checked) return; F(a)[x.dataset.f] = x.type === "checkbox" ? x.checked : x.value; const l = caja.querySelector(`[data-lee="${x.dataset.f}"]`); if (l) l.textContent = Math.round(Number(x.value) * 100) + " %"; };
        x.addEventListener("input", guardar); x.addEventListener("change", guardar);
        if (x.tagName === "TEXTAREA") x.addEventListener("keydown", (e) => { if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); const b = caja.querySelector("[data-ir]"); if (b && !b.disabled) b.click(); } });
      });
      caja.querySelectorAll(".ex-idea").forEach((b) => (b.onclick = () => { const t = caja.querySelector('textarea[data-f="descripcion"]'); t.value = t.value.trim() ? `${t.value.trim()}, ${b.dataset.idea}` : b.dataset.idea; F(a).descripcion = t.value; t.focus(); }));
      const irB = caja.querySelector("[data-ir]");
      if (irB) irB.onclick = () => ejecutar(a);
      const gl = caja.querySelector("[data-guardar-letra]");
      if (gl) gl.onclick = async () => { const t = caja.querySelector('textarea[data-f="texto"]').value; delete F("letra").texto; if (await op({ op: "letra", texto: t })) { X.ia.resultado.letra = { texto: tt("exLetraGuardada") }; L.pintar(true); } };
    }
    pintarChat();
    $("exChatEnviar").onclick = (e) => { e.stopPropagation(); enviarChat(); };
    $("exChatIn").oninput = () => (X.ia.borrador = $("exChatIn").value);
    $("exChatIn").onkeydown = (e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); enviarChat(); } };
  }
  // sin rehacer el panel: el contexto, y si el botón de la acción abierta se puede pulsar
  function refrescarIA(c) {
    const ctx = $("exIaCtx"); if (ctx) ctx.innerHTML = contexto();
    const b = c.querySelector(".ex-accion.abierta [data-ir]");
    if (b && X.trabajo) b.disabled = true;
  }
  const num = (a, k, d) => { const v = Number(val(a, k, d)); return isFinite(v) ? v : d; };
  async function ejecutar(a) {
    const f = F(a), e = est(), p = pistaSel(), s = tramo(), desc = (f.descripcion || "").trim();
    const fin = (texto, mal) => { X.ia.resultado[a] = { texto, mal }; L.pintar(true); };
    const falta = () => { toast(tt("exFaltaDescripcion"), 3500); const t = document.querySelector(`.ex-accion[data-a="${a}"] textarea`); if (t) t.focus(); };
    delete X.ia.resultado[a];
    if (a === "generar") {
      if (!desc) return falta();
      const r = await EX.trabajo(tt("exIaGenerar"), "experto_generar", { pedido: { descripcion: desc, letra: (f.letra || "").trim(), idioma: f.idioma || "", segundos: Number(f.segundos) || 0, con_tempo: val(a, "con_tempo", true), con_tonalidad: !!e.tonalidad && val(a, "con_tonalidad", true), nombre: (f.nombre || "").trim(), inicio: 0 } });
      if (r) { X.pistaSel = r.resultado; fin(tt("exHechoPista").replace("{n}", r.resultado)); EX.pintar(); }
    } else if (a === "rehacer") {
      if (!s) return;
      if (!desc) return falta();
      const r = await EX.trabajo(tt("exIaRehacer"), "experto_rehacer", { pista: s.pista, desde: s.desde, hasta: s.hasta, descripcion: desc, letra: (val(a, "letra", e.letra) || "").trim(), idioma: f.idioma || "" });
      if (r) fin(tt("exHechoToma"));
    } else if (a === "extender") {
      if (!desc) return falta();
      const r = await EX.trabajo(tt("exIaExtender"), "experto_extender", { pista: f.destino ? Number(f.destino) : null, segundos: num(a, "segundos", 30), descripcion: desc, letra: (val(a, "letra", e.letra) || "").trim(), idioma: f.idioma || "" });
      if (r) fin(tt("exHechoExtension").replace("{n}", num(a, "segundos", 30)));
    } else if (a === "versionar") {
      if (!p) return;
      if (!desc) return falta();
      const r = await EX.trabajo(tt("exIaVersionar"), "experto_versionar", { pista: p.id, descripcion: desc, fuerza: num(a, "fuerza", 0.6), letra: (val(a, "letra", e.letra) || "").trim(), idioma: f.idioma || "" });
      if (r) { X.pistaSel = r.resultado; fin(tt("exHechoVersion").replace("{n}", r.resultado)); EX.pintar(); }
    } else if (a === "separar") {
      if (!p) return;
      const r = await EX.trabajo(`${tt("exIaSeparar")} · ${p.nombre}`, "experto_separar", { pista: p.id, modelo: val(a, "modelo", "4") });
      if (r) fin(tt("exHechoSeparar"));
    } else if (a === "letra") {
      const r = await EX.trabajo(tt("exIaLetra"), "experto_letra", { pista: f.destino === undefined ? (p && p.fuente === "vocals" ? p.id : null) : f.destino ? Number(f.destino) : null });
      if (typeof r === "string") { F("letra").texto = r; fin(tt("exHechoLetra")); }
    } else {
      const r = await EX.trabajo(tt("exIaMasterizar"), "experto_masterizar", { lufs: num(a, "lufs", -14), techoDb: num(a, "techo", -1) });
      if (r && r.resultado) { const m = r.resultado; fin(tt("exHechoMaster").replace("{a}", m.lufs_antes.toFixed(1)).replace("{b}", m.lufs.toFixed(1)).replace("{e}", m.empuje_db ? `+${m.empuje_db.toFixed(1)} dB` : `${m.ganancia_db.toFixed(1)} dB`) + (m.falto_db > 0.1 ? " · " + tt("exFalto").replace("{n}", m.falto_db.toFixed(1)) : "")); }
    }
  }

  // ── el chat del estudio (EXP-5) ────────────────────────────────────────
  function pintarChat() {
    const log = $("exChatLog"); if (!log) return;
    if (!X.ia.chat.length) {
      log.innerHTML = `<p class="ex-nota">${esc(tt("exChatAyuda"))}</p><div class="ex-ideas">${tt("exChatIdeas").split("|").map((t) => `<button type="button" class="ex-idea" data-chat="${esc(t)}">${esc(t)}</button>`).join("")}</div>`;
      log.querySelectorAll("[data-chat]").forEach((b) => (b.onclick = () => { $("exChatIn").value = b.dataset.chat; X.ia.borrador = b.dataset.chat; $("exChatIn").focus(); }));
      return;
    }
    log.innerHTML = X.ia.chat.map((m) => `<p class="ex-msg ${m.c}">${esc(m.t)}</p>`).join("");
    log.scrollTop = log.scrollHeight;
  }
  async function enviarChat() {
    const t = $("exChatIn"), texto = t.value.trim();
    if (!texto || X.ia.pensando) return;
    t.value = ""; X.ia.borrador = ""; X.ia.pensando = true; $("exChatEnviar").disabled = true;
    X.ia.chat.push({ c: "yo", t: texto }, { c: "ia espera", t: "…" });
    pintarChat();
    const r = await EX.llamar("experto_chat", { texto });
    X.ia.chat.pop();
    X.ia.pensando = false;
    if (r) {
      for (const h of r.hechas || []) X.ia.chat.push({ c: "hecho", t: "✓ " + h });
      for (const q of r.preguntas || []) X.ia.chat.push({ c: "pregunta", t: "? " + q });
      for (const e of r.errores || []) X.ia.chat.push({ c: "error", t: "✗ " + e });
      if (r.texto) X.ia.chat.push({ c: "ia", t: r.texto });
      if (!(r.hechas || []).length && !(r.preguntas || []).length && !(r.errores || []).length && !r.texto) X.ia.chat.push({ c: "ia", t: tt("exChatNada") });
      if (r.vista) await EX.actualizar(r.vista);
    }
    if ($("exChatEnviar")) $("exChatEnviar").disabled = false;
    pintarChat();
    // el foco vuelve al chat: se sigue pidiendo sin tocar el ratón
    if ($("exChatIn")) $("exChatIn").focus();
  }

  // ── el inspector ────────────────────────────────────────────────────────
  const campo = (t, attrs, u) => `<label class="ex-f"><span>${esc(tt(t))}${u ? ` <em>${u}</em>` : ""}</span><input ${attrs}></label>`;
  function pintarInspector(c) {
    const e = est(), clip = X.clipSel && EX.clip(X.clipSel), reg = X.regionSel && e.regiones.find((r) => r.id === X.regionSel), p = pistaSel(), s = tramo();
    const nombres = tt("exFiltros").split("|"), claves = ["graves", "agudos", "campana", "paso_alto", "paso_bajo", "muesca", "paso_banda"];
    let h = `<div class="ex-insp">`;
    if (clip) {
      const dur = EX.durClip(clip);
      h += `<section class="ex-sec" data-s="clip"><h4>${esc(tt("exClip"))} <i class="mono">#${clip.id}</i></h4>
        ${campo("exNombre", `type="text" data-c="nombre" maxlength="60" placeholder="${esc((EX.pista(clip.pista) || {}).nombre || "")}" value="${esc(clip.nombre || "")}"`)}
        <div class="ex-f2">${campo("exInicio", `type="number" data-c="inicio" min="0" step="0.01" value="${clip.inicio.toFixed(3)}"`, "s")}${campo("exDuracion", `type="number" data-c="duracion" min="0.02" step="0.01" value="${dur.toFixed(3)}"`, "s")}</div>
        <div class="ex-f2">${campo("exGanancia", `type="number" data-c="ganancia_db" min="-60" max="24" step="0.5" value="${clip.ganancia_db || 0}"`, "dB")}${campo("exDesdeArchivo", `type="number" data-c="desde" min="0" step="0.01" value="${(clip.desde || 0).toFixed(3)}"`, "s")}</div>
        <div class="ex-f2">${campo("exFundidoEntrada", `type="number" data-c="fundido_entrada" min="0" max="${dur.toFixed(2)}" step="0.01" value="${Number((clip.fundido_entrada || 0).toFixed(3))}"`, "s")}${campo("exFundidoSalida", `type="number" data-c="fundido_salida" min="0" max="${dur.toFixed(2)}" step="0.01" value="${Number((clip.fundido_salida || 0).toFixed(3))}"`, "s")}</div>
        <div class="ex-f2">${campo("exTonoClip", `type="number" data-c="tono" min="-12" max="12" step="1" value="${clip.tono || 0}"`, "st")}${campo("exRitmo", `type="number" data-c="ritmo" min="0.25" max="4" step="0.05" value="${clip.ritmo || 1}"`, "×")}</div>
        <label class="pro-check"><input type="checkbox" data-c="invertido"${clip.invertido ? " checked" : ""}> ${esc(tt("exClipInvertir"))}</label>
        <label class="pro-check"><input type="checkbox" data-c="silencio"${clip.silencio ? " checked" : ""}> ${esc(tt("exClipSilenciar"))}</label>
        <div class="ex-botones"><button type="button" class="ex-t" data-clip="partir">${esc(tt("exPartirCabezal"))}</button><button type="button" class="ex-t" data-clip="duplicar">${esc(tt("exDuplicar"))}</button><button type="button" class="ex-t ia" data-clip="rehacer">${EX.icono(EX.I.chispa)} ${esc(tt("exRehacerIA"))}</button><button type="button" class="ex-t peligro" data-clip="quitar">${esc(tt("exQuitarClip"))}</button></div></section>`;
    }
    if (reg) {
      h += `<section class="ex-sec" data-s="region"><h4>${esc(tt("exTramoEfecto"))}</h4><p class="ex-destino">${esc(EX.arreglo.etiquetaRegion(reg))}</p>
        <div class="ex-f2">${campo("exDesde", `type="number" data-r="desde" min="0" step="0.01" value="${reg.desde.toFixed(3)}"`, "s")}${campo("exHasta", `type="number" data-r="hasta" min="0" step="0.01" value="${reg.hasta.toFixed(3)}"`, "s")}</div>
        <div class="ex-botones"><button type="button" class="ex-t${reg.apagada ? "" : " on"}" data-reg="apagar" title="${esc(tt("exApagarAyuda"))}">${esc(tt(reg.apagada ? "exApagada" : "exEncendida"))}</button><button type="button" class="ex-t peligro" data-reg="quitar">${esc(tt("exQuitarRegion"))}</button></div></section>`;
    }
    if (s && !reg) {
      h += `<section class="ex-sec" data-s="sel"><h4>${esc(tt("exSeleccion"))} <i class="mono">${(s.hasta - s.desde).toFixed(2)} s</i></h4>
        <div class="ex-f2">${campo("exDesde", `type="number" id="exSelDesde" step="0.01" min="0" value="${s.desde.toFixed(2)}"`, "s")}${campo("exHasta", `type="number" id="exSelHasta" step="0.01" min="0" value="${s.hasta.toFixed(2)}"`, "s")}</div>
        <div class="ex-botones"><button type="button" class="ex-t ia" data-t="ia">${EX.icono(EX.I.chispa)} ${esc(tt("exRehacerIA"))}</button><button type="button" class="ex-t" data-t="mas">＋3 dB</button><button type="button" class="ex-t" data-t="menos">−3 dB</button><button type="button" class="ex-t" data-t="sil">${esc(tt("exSilenciarTramo"))}</button><button type="button" class="ex-t" data-t="bucle">${esc(tt("exBucleAqui"))}</button></div>
        <div class="ex-botones"><button type="button" class="ex-t" data-t="quitar">${esc(tt("exQuitarTramo"))}</button><button type="button" class="ex-t" data-t="cerrar">${esc(tt("exQuitarYCerrar"))}</button></div>
        <h5>${esc(tt("exEq"))}</h5>
        <div class="ex-f2"><label class="ex-f"><span>${esc(tt("exFiltro"))}</span><select id="exEqF">${claves.map((k, i) => `<option value="${k}"${k === "agudos" ? " selected" : ""}>${esc(nombres[i] || k)}</option>`).join("")}</select></label>${campo("exFrecuencia", `type="number" id="exEqHz" min="20" max="20000" value="4000"`, "Hz")}</div>
        <div class="ex-f2">${campo("exGanancia", `type="number" id="exEqDb" min="-24" max="24" step="0.5" value="4"`, "dB")}${campo("exQ", `type="number" id="exEqQ" min="0.1" max="18" step="0.1" value="0.7"`)}</div>
        <div class="ex-botones"><button type="button" class="ex-t" data-t="eq">${esc(tt("exAplicarEq"))}</button></div>
        <h5>${esc(tt("exTono"))}</h5>
        <div class="ex-f2">${campo("exSemitonos", `type="number" id="exTonoSt" min="-12" max="12" step="1" value="2"`, "st")}<div class="ex-botones bajo"><button type="button" class="ex-t" data-t="tono">${esc(tt("exTonoPista"))}</button></div></div></section>`;
    }
    if (p) {
      const rs = e.regiones.filter((r) => r.pista === p.id), curvas = ["volumen", "pan"].filter((k) => EX.curvaDe(p.id, k));
      h += `<section class="ex-sec" data-s="pista"><h4><i class="ex-punto" style="background:${EX.colorDe(p)}"></i>${esc(tt("exPista"))} ${p.id} · ${esc(p.nombre)}</h4>
        <p class="ex-nota">${esc(tt(p.origen === "separada" ? "exOrigenSeparada" : p.origen === "generada" ? "exOrigenGenerada" : "exOrigenImportada"))}${p.grupo && EX.grupo(p.grupo) ? ` · BUS «${esc(EX.grupo(p.grupo).nombre)}»` : ""}${curvas.length ? ` · ${esc(tt("exCurvas"))}: ${curvas.map((k) => tt(k === "pan" ? "exPan" : "exVolumen")).join(", ")}` : ""}</p>
        <h5>${esc(tt("exRegiones"))} <i class="mono">${rs.length}</i></h5>
        ${rs.length ? `<ul class="ex-lista">${rs.map((r) => `<li class="${X.regionSel === r.id ? "sel" : ""}${r.apagada ? " apagada" : ""}"><button type="button" class="ex-lista-n" data-ver="${r.id}">${esc(EX.arreglo.etiquetaRegion(r))} <i class="mono">${r.desde.toFixed(1)}–${r.hasta.toFixed(1)} s</i></button><button type="button" class="ex-t${r.apagada ? "" : " on"}" data-ab="${r.id}" title="${esc(tt("exApagarAyuda"))}">${r.apagada ? "B" : "A"}</button><button type="button" class="lnk" data-quitar="${r.id}">✕</button></li>`).join("")}</ul>` : `<p class="ex-nota">${esc(tt("exSinRegiones"))}</p>`}</section>`;
    }
    h += `<section class="ex-sec" data-s="sesion"><h4>${esc(tt("exSesion"))}</h4>
      <h5>${esc(tt("exMarcadores"))} <i class="mono">${e.marcadores.length}</i></h5>
      ${e.marcadores.length ? `<ul class="ex-lista">${e.marcadores.map((m, i) => `<li><button type="button" class="ex-lista-n" data-ir="${m.segundo}">${esc(m.nombre)} <i class="mono">${EX.fmtT(m.segundo).slice(0, -1)}</i></button><button type="button" class="lnk" data-qm="${i}">✕</button></li>`).join("")}</ul>` : `<p class="ex-nota">${esc(tt("exSinMarcadores"))}</p>`}
      <h5>${esc(tt("exLetraSesion"))}</h5><p class="ex-nota">${e.letra ? esc(e.letra.slice(0, 140)) + (e.letra.length > 140 ? "…" : "") : esc(tt("exSinLetra"))}</p>
      <div class="ex-botones"><button type="button" class="ex-t ia" data-letra>${EX.icono(EX.I.chispa)} ${esc(tt("exIaLetra"))}</button></div></section></div>`;
    c.innerHTML = h;

    // el clip
    c.querySelectorAll("[data-c]").forEach((x) => (x.onchange = () => {
      const k = x.dataset.c, v = x.type === "checkbox" ? x.checked : x.type === "number" ? Number(x.value) : x.value;
      if (k === "inicio") op({ op: "mover_clip", clip: clip.id, inicio: Math.max(0, v) });
      else if (k === "duracion" || k === "desde") op({ op: "recortar_clip", clip: clip.id, inicio: clip.inicio, desde: k === "desde" ? Math.max(0, v) : clip.desde || 0, duracion: k === "duracion" ? Math.max(0.02, v) : EX.durClip(clip) });
      else op({ op: "clip_ajustes", clip: clip.id, [k]: v });
    }));
    c.querySelectorAll("button[data-clip]").forEach((b) => (b.onclick = () => {
      const a = b.dataset.clip, dur = EX.durClip(clip);
      if (a === "partir") EX.arreglo.partirEnCabezal();
      else if (a === "duplicar") EX.arreglo.duplicarClip();
      else if (a === "rehacer") { X.sel = { pista: clip.pista, desde: clip.inicio, hasta: clip.inicio + dur }; L.abrirIA("rehacer"); }
      else op({ op: "quitar_clip", clip: clip.id });
    }));
    // el tramo de efecto
    c.querySelectorAll("[data-r]").forEach((x) => (x.onchange = () => op({ op: "region_ajustes", id: reg.id, [x.dataset.r]: Number(x.value) })));
    c.querySelectorAll("button[data-reg]").forEach((b) => (b.onclick = () => (b.dataset.reg === "quitar" ? op({ op: "quitar_region", id: reg.id }) : op({ op: "region_ajustes", id: reg.id, apagada: !reg.apagada }))));
    // la selección
    const selActual = () => ({ pista: s.pista, desde: Number($("exSelDesde").value), hasta: Number($("exSelHasta").value) });
    for (const id of ["exSelDesde", "exSelHasta"]) if ($(id)) $(id).onchange = () => { const n = selActual(); if (n.hasta > n.desde) { X.sel = n; EX.arreglo.pintar(); EX.pintarPie(); } };
    const region = (efecto) => { const n = selActual(); return op({ op: "region", region: { id: 0, pista: n.pista, desde: n.desde, hasta: n.hasta, efecto } }); };
    c.querySelectorAll("button[data-t]").forEach((b) => (b.onclick = () => {
      const a = b.dataset.t, n = selActual();
      if (a === "ia") { X.sel = n; L.abrirIA("rehacer"); }
      else if (a === "mas") region({ tipo: "ganancia", db: 3 });
      else if (a === "menos") region({ tipo: "ganancia", db: -3 });
      else if (a === "sil") region({ tipo: "silencio" });
      else if (a === "bucle") { X.bucleR = { desde: n.desde, hasta: n.hasta }; X.bucle = true; EX.pintar(); if (X.sonando) { EX.parar(); EX.reproducir(); } }
      else if (a === "quitar" || a === "cerrar") op({ op: "quitar_tramo", pista: n.pista, desde: n.desde, hasta: n.hasta, cerrar: a === "cerrar" }).then((v) => { if (v) { X.sel = null; EX.pintar(); } });
      else if (a === "tono") EX.llamar("experto_tono", { pista: n.pista, desde: n.desde, hasta: n.hasta, semitonos: Number($("exTonoSt").value) }).then((v) => v && EX.actualizar(v));
      else region({ tipo: "eq", filtro: $("exEqF").value, frecuencia: Number($("exEqHz").value), db: Number($("exEqDb").value), q: Number($("exEqQ").value) });
    }));
    // la pista y la sesión
    c.querySelectorAll("button[data-ver]").forEach((b) => (b.onclick = () => { const r = e.regiones.find((x) => x.id === Number(b.dataset.ver)); if (!r) return; X.regionSel = r.id; X.sel = { pista: r.pista, desde: r.desde, hasta: r.hasta }; EX.irA(r.desde); EX.pintar(); L.pintar(true); }));
    c.querySelectorAll("button[data-ab]").forEach((b) => (b.onclick = () => { const r = e.regiones.find((x) => x.id === Number(b.dataset.ab)); if (r) op({ op: "region_ajustes", id: r.id, apagada: !r.apagada }); }));
    c.querySelectorAll("button[data-quitar]").forEach((b) => (b.onclick = () => op({ op: "quitar_region", id: Number(b.dataset.quitar) })));
    c.querySelectorAll("button[data-ir]").forEach((b) => (b.onclick = () => EX.irA(Number(b.dataset.ir))));
    c.querySelectorAll("button[data-qm]").forEach((b) => (b.onclick = () => op({ op: "marcadores", marcadores: e.marcadores.filter((_, i) => i !== Number(b.dataset.qm)) })));
    const bl = c.querySelector("button[data-letra]");
    if (bl) bl.onclick = () => L.abrirIA("letra");
  }

  // `app.js` abre el proyecto ANTES de que carguen estos archivos: se sincroniza aquí, cuando ya están
  // todas las partes del estudio (y el candado vale desde el arranque)
  if (typeof S !== "undefined" && S.root) Experto.proyecto(S.root);
})();
