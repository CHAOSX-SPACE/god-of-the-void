// F16.1 · Escribir mientras el agente trabaja. La cola vive en Rust (gotv_core::escritorio::cola); esto sólo
// pinta. UN SOLO botón que cambia según el estado (el Portador, 2026-11-20: salían ■ y ↵ a la vez, uno
// encima del otro): libre → ↵ enviar; trabajando con la caja vacía → ■ detener; trabajando con algo
// escrito → ↵ a la cola. Enganches en app.js: setBusy(b) llama a gotvCola.ocupado(b); submit() llama a
// gotvCola.enviar() si está ocupado.
(() => {
  const inv = (c, a) => window.__TAURI__.core.invoke(c, a);
  const $ = (id) => document.getElementById(id);
  const enEspera = new Map(); // id → burbuja

  // el botón de enviar según el estado (`S` es un const de app.js: está en el ámbito global, no en `window`)
  function pintar() {
    const send = $("send"), input = $("in");
    if (!send || typeof S === "undefined" || !S.busy) {
      if (send) { send.title = tt("enviarTip"); send.setAttribute("aria-label", tt("enviarTip")); }
      return;
    }
    const escrito = !!(input && input.value.trim());
    send.textContent = escrito ? "↵" : "■";
    send.classList.toggle("stop", !escrito);
    send.disabled = false;
    send.title = tt(escrito ? "colaEnCola" : "colaDetener");
    send.setAttribute("aria-label", send.title);
  }
  function ocupado() { pintar(); }

  // ↵ con el agente ocupado: a la cola. Devuelve true si se ocupó del clic.
  function enviar() {
    const input = $("in");
    const t =(input && input.value.trim()) || "";
    if (!t) return false;
    input.value = "";
    input.dispatchEvent(new Event("input"));
    if (typeof addUser === "function") addUser(t, false, t, Date.now());
    const burbuja = document.querySelector("#log .msg-u:last-of-type") || [...document.querySelectorAll("#log .msg-u")].pop();
    inv("mensaje_encolar", { texto: t })
      .then((id) => {
        if (!burbuja) return;
        burbuja.classList.add("en-cola");
        const tag = document.createElement("div");
        tag.className = "cola-tag";
        tag.textContent = "⏳ " + tt("colaEnCola");
        const x = document.createElement("button");
        x.className = "cola-x";
        x.type = "button";
        x.textContent = "✕";
        x.title = tt("colaQuitar");
        x.onclick = () => inv("mensaje_quitar", { id }).then((ok) => { if (ok) { burbuja.remove(); enEspera.delete(id); } });
        tag.appendChild(x);
        burbuja.appendChild(tag);
        enEspera.set(id, burbuja);
      })
      .catch((e) => { if (typeof toast === "function") toast("✗ " + e, 5000); });
    return true;
  }

  // lo que ya no está en la cola, entró (o pasó a ser la orden siguiente)
  function cola(m) {
    const quedan = new Set((m.pendientes || []).map((p) => p.id));
    for (const [id, b] of enEspera) {
      if (quedan.has(id)) continue;
      b.classList.remove("en-cola");
      const tag = b.querySelector(".cola-tag");
      if (tag) tag.textContent = m.sigue ? "↪ " + tt("colaSigue") : "✉ " + tt("colaEntro");
      enEspera.delete(id);
    }
  }

  // con el agente ocupado, escribir habilita ↵ (refreshSend de app.js sólo mira cuando está libre)
  const inp = $("in");
  // (`S` es un const de app.js: existe en el ámbito global, pero no en `window`)
  if (inp) inp.addEventListener("input", pintar);
  window.__TAURI__.event.listen("gotv", (ev) => { if (ev.payload && ev.payload.kind === "cola") cola(ev.payload); });
  window.gotvCola = { ocupado, enviar };
})();
