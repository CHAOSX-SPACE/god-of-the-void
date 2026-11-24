// F7 · La tarjeta del PLAN: el plan que presentó el agente (`exit_plan`), por líneas, comentable línea a
// línea; «Aprobar» y «Pedir cambios» contestan la pregunta del agente desde la tarjeta (sin el cuadro
// modal). Los comentarios los guarda el núcleo (`plan_comentar` → .gotv/chats/<sesión>/comentarios.gotv, formato propio de GOTV) y
// llegan al modelo marcados como del usuario. Sólo pinta: la lógica vive en Rust (gotv_core::artefactos).
// Va en el chat (zona central) hasta que exista la pestaña «Vista» del panel de trabajo (F8.0).

// la tarjeta que espera respuesta (la pregunta exacta que mandará el agente)
let PLAN_PENDIENTE = null;

function addPlan(m) {
  clearEmpty();
  // una tarjeta nueva deja a las anteriores sólo de lectura
  if (PLAN_PENDIENTE) PLAN_PENDIENTE.cerrar();
  const card = el("div", "vista plan-card", "");
  card.innerHTML = `<div class="vista-bar"><b></b><span class="plan-estado"></span><button class="btn plan-abrir" type="button"></button></div><ol class="plan-lineas"></ol><div class="plan-huerfanos" hidden></div><div class="plan-acciones" hidden><button class="btn primary plan-si" type="button"></button><button class="btn plan-no" type="button"></button></div>`;
  card.querySelector("b").textContent = "☰ " + (m.titulo || "plan");
  const abrir = card.querySelector(".plan-abrir");
  abrir.textContent = tt("vistaAbrir");
  abrir.onclick = () => invoke("open_vista", { path: String(m.ruta || "") }).catch((e) => toast("✗ " + e, 5000));
  const ol = card.querySelector(".plan-lineas"), huerf = card.querySelector(".plan-huerfanos"), estado = card.querySelector(".plan-estado");
  const lineas = String(m.texto || "").split("\n");
  let coms = Array.isArray(m.comentarios) ? m.comentarios : [];
  let abierta = true;

  const burbuja = (c) => {
    const d = el("div", "plan-com" + (c.resuelto ? " resuelto" : ""), "");
    const t = el("span", "plan-com-texto", ""); t.textContent = c.texto;
    const b = el("button", "btn plan-com-ok", c.resuelto ? "↺" : "✓");
    b.type = "button";
    b.title = tt(c.resuelto ? "planReabrir" : "planResolver");
    b.setAttribute("aria-label", b.title);
    b.onclick = () => invoke("plan_resolver", { sesion: String(m.sesion), id: c.id, resuelto: !c.resuelto })
      .then(() => { c.resuelto = !c.resuelto; pinta(); }).catch((e) => toast("✗ " + e, 5000));
    d.append(t, b);
    return d;
  };

  const editor = (li, n) => {
    if (li.querySelector(".plan-editor")) return;
    const f = el("div", "plan-editor", "");
    const ta = el("textarea", "", ""); ta.rows = 2; ta.placeholder = tt("planComentarPh");
    const g = el("button", "btn primary", tt("planGuardar")); g.type = "button";
    const x = el("button", "btn", tt("cancel")); x.type = "button";
    const guardar = () => {
      const texto = ta.value.trim();
      if (!texto) return;
      invoke("plan_comentar", { sesion: String(m.sesion), archivo: String(m.archivo || "plan.md"), linea: n, texto })
        .then((c) => { coms.push(c); pinta(); }).catch((e) => toast("✗ " + e, 5000));
    };
    g.onclick = guardar;
    x.onclick = () => f.remove();
    ta.addEventListener("keydown", (e) => { if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); guardar(); } if (e.key === "Escape") f.remove(); });
    f.append(ta, g, x);
    li.appendChild(f);
    ta.focus();
  };

  function pinta() {
    ol.innerHTML = "";
    lineas.forEach((txt, i) => {
      const n = i + 1;
      const li = el("li", "plan-linea", "");
      li.appendChild(el("span", "plan-n", String(n)));
      const t = el("span", "plan-texto", ""); t.textContent = txt || " ";
      const b = el("button", "plan-comentar", "+"); b.type = "button";
      b.title = `${tt("planComentar")} ${n}`; b.setAttribute("aria-label", b.title);
      b.onclick = () => editor(li, n);
      li.append(t, b);
      for (const c of coms.filter((c) => c.linea === n)) li.appendChild(burbuja(c));
      ol.appendChild(li);
    });
    // trampa 1: un comentario cuya línea ya no está se muestra como huérfano, nunca se pierde
    const hs = coms.filter((c) => c.linea == null);
    huerf.hidden = !hs.length;
    huerf.innerHTML = "";
    if (hs.length) {
      huerf.appendChild(el("div", "plan-huerf-tit", tt("planHuerfanos")));
      for (const c of hs) { const d = burbuja(c); d.prepend(el("q", "", c.texto_linea || "")); huerf.appendChild(d); }
    }
    const pend = coms.filter((c) => !c.resuelto).length;
    estado.textContent = pend ? `${pend} ${tt("planPendientes")}` : "";
  }
  pinta();

  const acc = card.querySelector(".plan-acciones"), si = card.querySelector(".plan-si"), no = card.querySelector(".plan-no");
  si.textContent = tt("planAprobar");
  no.textContent = tt("planCambios");
  const yo = {
    pregunta: m.pregunta || null,
    // llegó la pregunta del agente: la tarjeta la contesta
    activar(id, opciones) {
      if (!abierta) return false;
      acc.hidden = false;
      const responder = (v, etiqueta) => {
        invoke("answer_choice", { id, value: v });
        acc.hidden = true;
        estado.textContent = etiqueta;
        abierta = false;
        if (PLAN_PENDIENTE === yo) PLAN_PENDIENTE = null;
        input.focus();
      };
      si.onclick = () => responder(opciones[0], "✓ " + tt("planAprobado"));
      no.onclick = () => responder(opciones[1], "↻ " + tt("planCambiosPedidos"));
      setTimeout(() => si.focus(), 30);
      return true;
    },
    cerrar() { abierta = false; acc.hidden = true; },
  };
  if (m.pregunta) PLAN_PENDIENTE = yo;
  stick(() => log.appendChild(card));
}

// showChoose: si la pregunta es la del plan que está en pantalla, contesta la tarjeta (no el modal)
function planContesta(id, question, options) {
  return !!(PLAN_PENDIENTE && PLAN_PENDIENTE.pregunta === question && (options || []).length >= 2 && PLAN_PENDIENTE.activar(id, options));
}
