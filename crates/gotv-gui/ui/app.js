// GOD OF THE VOID — la ventana. Habla con el núcleo por comandos (invoke) y eventos.
const T = window.__TAURI__;
const invoke = (c, a) => T.core.invoke(c, a);
const $ = (id) => document.getElementById(id);
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

const S = { busy: false, ready: false, root: "", live: null, work: null, workSteps: 0, askId: null, slash: [], slashSel: 0, recording: false, tasks: [], procs: [], model: "", projects: [] };
const log = $("log"), input = $("in"), send = $("send");

// ── Utilidades ────────────────────────────────────────────────────────────
function toast(msg, ms = 2600) {
  const t = $("toast");
  t.textContent = msg;
  t.classList.add("show");
  clearTimeout(t._h);
  t._h = setTimeout(() => t.classList.remove("show"), ms);
}
function atBottom() { return log.scrollHeight - log.scrollTop - log.clientHeight < 80; }
function stick(fn) { const b = atBottom(); fn(); if (b) log.scrollTop = log.scrollHeight; }
function el(tag, cls, text) { const e = document.createElement(tag); if (cls) e.className = cls; if (text !== undefined) e.textContent = text; return e; }
function fmtK(n) { return n >= 1000 ? (n / 1000).toFixed(1) + "k" : String(n); }
function applyTheme(t) {
  const sys = window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
  document.documentElement.dataset.theme = t === "system" ? sys : t || "dark";
}

// ── Transcripción ─────────────────────────────────────────────────────────
function clearEmpty() { const e = log.querySelector(".empty"); if (e) e.remove(); }
function showEmpty() {
  if (log.children.length) return;
  const e = el("div", "empty");
  // sin logo en el centro (el Portador: «quítalo, no lo reemplaces por nada»); en una charla, el saludo de Chat
  const charla = S.modo === "chat";
  e.innerHTML = `<div><h3>${esc(tt(charla ? "chatVacioTitulo" : "emptyTitle"))}</h3><div>${esc(tt(charla ? "chatVacioCuerpo" : "emptyBody"))}</div></div>`;
  log.appendChild(e);
}

// ── Chat / Code (como Claude Desktop) ─────────────────────────────────────
// El modo NO se guarda aparte: lo dice lo abierto. Una charla (carpeta de `sueltas`, la ruta la da Rust en
// `info`) es «Chat»; cualquier proyecto es «Code».
function esCharla(root) { return !!(root && S.sueltasBase && root.startsWith(S.sueltasBase.replace(/\/+$/, "") + "/")); }
function pintarModo() {
  const side = document.querySelector("aside.side");
  if (side) side.dataset.modo = S.modo;
  for (const b of document.querySelectorAll(".modo-tab")) b.setAttribute("aria-selected", String(b.dataset.modo === S.modo));
}
S.modo = "code";
$("modoChat").onclick = () => {
  if (S.modo === "chat") return;
  if (S.busy) return toast(tt("working"));
  S.modo = "chat"; pintarModo();
  // como en Claude Desktop: Chat abre una charla nueva; las anteriores quedan en la lista
  if (window.gotvChats && gotvChats.nuevaCharla) gotvChats.nuevaCharla();
};
$("modoCode").onclick = () => {
  if (S.modo === "code") return;
  if (S.busy) return toast(tt("working"));
  S.modo = "code"; pintarModo();
  const destino = S.ultimoProyecto || (S.projects[0] && S.projects[0].path);
  if (destino) openProject(destino); else invoke("close_project").catch(() => noProject());
};
function lineClass(t) {
  if (t.startsWith("  │+")) return "add";
  if (t.startsWith("  │-")) return "del";
  if (t.startsWith("  │ @@") || t.startsWith("  ┌")) return "hunk";
  if (t.startsWith("  ↩") || t.startsWith("⏹")) return "undo";
  if (t.includes("[LSP]")) return "lsp";
  if (t.includes("🚫") || t.trim().startsWith("✗") || t.startsWith("❌")) return "bad";
  if (t.includes("⚠")) return "warn";
  if (t.trim().startsWith("✓")) return "ok";
  return "step";
}
// el trabajo del agente (pasos, diffs, avisos) va en un bloque plegable CERRADO: la
// pantalla muestra la conversación; el detalle, sólo si el usuario lo abre. Mientras
// trabaja, la cabecera dice qué está haciendo.
function workBlock() {
  if (!S.work) {
    clearEmpty();
    const d = el("details", "work live");
    d.open = false;
    d.innerHTML = `<summary><span class="ws"></span><span class="wnow"></span><span class="wtok"></span></summary><div class="work-body"></div>`;
    stick(() => log.appendChild(d));
    S.work = d;
    S.workSteps = 0;
  }
  return S.work;
}
function addLine(text) {
  if (text.includes("🗜")) {
    stick(() => log.appendChild(el("div", "divider", "🗜 " + text.replace(/^·\s*🗜\s*/, ""))));
    return;
  }
  const w = workBlock();
  const step = text.match(/^· (?:paso|step) \d+:\s*(\S+)/);
  if (step) { S.workSteps++; w.querySelector(".wnow").textContent = step[1]; }
  w.querySelector(".ws").textContent = trabajoTitulo(w.classList.contains("live"), S.workSteps);
  const n = el("div", "l " + lineClass(text), text);
  stick(() => w.querySelector(".work-body").appendChild(n));
  // «falta …: bájalo en Configuración → Generadores»: un botón que lleva a su fila
  faltaEnAviso(text, n);
}
// «generando imagen / video / música / efecto · …» con el TIEMPO TRANSCURRIDO al lado (pedido del
// Portador): sale del detalle de la tarea del turno y cuenta desde el primer aviso de esa generación
S.gen = null;
const reloj = (s) => {
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), x = String(s % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${x}` : `${m}:${x}`;
};
function genVivo() {
  const t = (S.tasks || []).find((x) => x.state === "running" && /^(generando|generating) /.test(x.detail || ""));
  if (!t) {
    if (S.gen) { S.gen = null; pintarGen(); }
    return;
  }
  const tipo = t.detail.split(" · ")[0];
  if (!S.gen || S.gen.tipo !== tipo) S.gen = { tipo, desde: Date.now() };
  S.gen.texto = t.detail;
  pintarGen();
}
function pintarGen() {
  const w = S.gen && S.busy ? workBlock() : S.work;
  if (!w) return;
  const n = w.querySelector(".wnow");
  n.classList.toggle("gen", !!S.gen);
  if (!S.gen) return;
  n.textContent = `${S.gen.texto} · ⏱ ${reloj(Math.floor((Date.now() - S.gen.desde) / 1000))}`;
}
setInterval(() => { if (S.gen) pintarGen(); }, 1000);

// como Claude Code: mientras trabaja «Trabajando…», al terminar «Trabajó en N pasos ›» (se abre al pulsar)
function trabajoTitulo(vivo, n) {
  if (vivo) return tt("workNow");
  return n === 1 ? tt("workDone1") : tt("workDone").replace("{n}", n);
}
function closeWork() {
  if (S.work) {
    S.work.classList.remove("live");
    S.work.querySelector(".wnow").textContent = "";
    S.work.querySelector(".ws").textContent = trabajoTitulo(false, S.workSteps);
    S.work = null;
  }
}
// ↓ bajar al final: visible sólo si no estás abajo
(() => {
  const b = $("bajar");
  const ver = () => { b.hidden = log.scrollHeight - log.scrollTop - log.clientHeight < 120; };
  log.addEventListener("scroll", ver, { passive: true });
  new MutationObserver(ver).observe(log, { childList: true, subtree: true });
  b.onclick = () => log.scrollTo({ top: log.scrollHeight, behavior: "smooth" });
})();
// tu mensaje + su pie (como Claude Desktop: al pasar el ratón, ✓✓ · hace cuánto · copiar · rebobinar).
// `texto` = lo que se rebobina/copia (lo escrito, sin los nombres de los adjuntos); `cuando` = ms
function addUser(text, mac, texto, cuando) {
  clearEmpty(); closeWork();
  const w = el("div", "msg-u", "");
  const d = el("div", "msg-user", text);
  if (mac) { const b = el("span", "mac-badge", "🖥 " + tt("macCtl")); b.title = tt("macBadgeTip"); d.prepend(b); }
  w._texto = (texto ?? text).trim();
  w._cuando = cuando || null;
  const pie = el("div", "msg-pie", "");
  pie.innerHTML = `<svg class="enviado" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M2.5 12.5l4 4L15 7"/><path d="M10.5 15.5l1 1L20 7"/></svg>` +
    (w._cuando ? `<span class="hace"></span><span class="sep">·</span>` : "") +
    `<button class="copiar" type="button"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round" aria-hidden="true"><rect x="8" y="8" width="12" height="12" rx="2.5"/><path d="M16 8V6.5A2.5 2.5 0 0 0 13.5 4h-7A2.5 2.5 0 0 0 4 6.5v7A2.5 2.5 0 0 0 6.5 16H8"/></svg></button>` +
    `<button class="rebobinar" type="button"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3.5 12a8.5 8.5 0 1 0 2.6-6.1L3.5 8.5"/><path d="M3.5 3.5v5h5"/></svg></button>` +
    `<button class="voz" type="button">${IC_PARLANTE}</button>`;
  const bc = pie.querySelector(".copiar"), br = pie.querySelector(".rebobinar"), bv = pie.querySelector(".voz");
  bc.title = tt("rbCopiar"); bc.setAttribute("aria-label", tt("rbCopiar"));
  br.title = tt("rbRebobinar"); br.setAttribute("aria-label", tt("rbRebobinar"));
  bv.title = tt("readAloud"); bv.setAttribute("aria-label", tt("readAloud"));
  bv.onclick = (e) => { e.stopPropagation(); leer(w, w._texto, bv); };
  bc.onclick = async () => {
    try { await navigator.clipboard.writeText(w._texto); bc.classList.add("ok"); setTimeout(() => bc.classList.remove("ok"), 1400); } catch { toast("✗ " + tt("copyFail")); }
  };
  br.onclick = () => rebobinar(w);
  w.onmouseenter = () => { const h = pie.querySelector(".hace"); if (h && w._cuando) h.textContent = hace(w._cuando); };
  w.append(d, pie);
  stick(() => log.appendChild(w));
}
// «Hace un momento» · «Hace 1 minuto» · «Hace 3 horas» · la fecha
function hace(ms) {
  const s = Math.max(0, (Date.now() - ms) / 1000), m = Math.floor(s / 60), h = Math.floor(m / 60);
  if (s < 45) return tt("haceMomento");
  if (m < 60) return (m <= 1 ? tt("haceMin1") : tt("haceMin").replace("{n}", m));
  if (h < 24) return (h <= 1 ? tt("haceHora1") : tt("haceHora").replace("{n}", h));
  return new Date(ms).toLocaleString(LANG === "en" ? "en-US" : "es-CO", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}
// ↺ REBOBINAR (como Claude Desktop): tu mensaje vuelve al cuadro para editarlo; él y lo de después
// quedan atenuados y NO se borran hasta que envías. Esc o «Deshacer» lo cancelan. Los archivos no se tocan.
function rebobinar(w) {
  if (S.busy) { toast(tt("rbOcupado"), 3000); return; }
  if (S.rb) cancelarRb();
  const nodos = [];
  for (let x = w; x; x = x.nextElementSibling) nodos.push(x);
  const iguales = [...log.querySelectorAll(".msg-u")];
  const n = iguales.slice(0, iguales.indexOf(w)).filter((x) => x._texto === w._texto).length;
  nodos.forEach((x) => x.classList.add("rb-fuera"));
  S.rb = { texto: w._texto, n, nodos, borrador: input.value };
  input.value = w._texto; fit(); refreshSend(); input.focus();
  $("rbAviso").hidden = false;
}
function cancelarRb() {
  if (!S.rb) return;
  S.rb.nodos.forEach((x) => x.classList.remove("rb-fuera"));
  input.value = S.rb.borrador; fit(); refreshSend();
  S.rb = null; $("rbAviso").hidden = true;
}
$("rbDeshacer").onclick = () => { cancelarRb(); input.focus(); };
// las respuestas se pintan en markdown (md.js escapa todo el HTML del modelo primero)
function mdInto(node, text) { node._raw = text; node.innerHTML = mdRender(text); }
function addAnswer(text) { clearEmpty(); const d = el("div", "msg-ans md", ""); mdInto(d, text); stick(() => log.appendChild(d)); vozBtn(d); }
// 🔊 M2: leer en voz alta con la voz de GOTV (núcleo, Rust); otro clic la calla. DOS parlantes de trazo
// fino como copiar y rebobinar: el del pie de TU mensaje lee tu mensaje; el del pie de la RESPUESTA lee
// la respuesta. Uno solo suena a la vez: pulsar otro calla el anterior.
const IC_PARLANTE = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M11 5.5 6.5 9H4v6h2.5l4.5 3.5z"/><path d="M15 9.5a3.5 3.5 0 0 1 0 5"/><path d="M17.8 7a7 7 0 0 1 0 10"/></svg>`;
const IC_PARAR = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round" aria-hidden="true"><rect x="7" y="7" width="10" height="10" rx="2"/></svg>`;
function vozIcono(b, leyendo) {
  b.innerHTML = leyendo ? IC_PARAR : IC_PARLANTE;
  const t = tt(leyendo ? "stopReading" : "readAloud");
  b.title = t; b.setAttribute("aria-label", t);
}
// el pie de la respuesta (al pasar el ratón): 🔈 lee la respuesta
function vozBtn(node) {
  if (!(node._raw || "").trim() || node.querySelector(":scope > .ans-pie")) return;
  const pie = el("div", "ans-pie", "");
  const b = el("button", "voz", "");
  b.type = "button"; vozIcono(b, false);
  b.onclick = (e) => { e.stopPropagation(); leer(node, node._raw, b); };
  pie.appendChild(b);
  node.appendChild(pie);
}
// `nodo` = tu mensaje o la respuesta; `b` = su parlante
async function leer(nodo, texto, b) {
  if (S.vozNode && S.vozNode.nodo === nodo) { await invoke("voz_callar"); vozFin(); return; }
  if (!(texto || "").trim()) return;
  // leer en voz alta pide la voz de GOTV
  if (!(await exigeModelo("hablar"))) return;
  if (S.vozNode) { await invoke("voz_callar").catch(() => {}); vozFin(); }
  const id = (S.vozId = (S.vozId || 0) + 1);
  S.vozNode = { nodo, b, id }; nodo.classList.add("leyendo"); vozIcono(b, true);
  try { await invoke("voz_leer", { texto, id }); } catch (x) { vozFin(); toast("✗ " + x, 9000); faltaEnAviso(String(x), null, true); }
}
function vozFin() {
  const v = S.vozNode; S.vozNode = null; if (!v) return;
  v.nodo.classList.remove("leyendo");
  vozIcono(v.b, false);
}
// en vivo: el texto crudo se acumula y se re-pinta como mucho cada 40 ms
function liveAppend(t) {
  S.live._raw = (S.live._raw || "") + t;
  if (S.live._raf) return;
  const node = S.live;
  // temporizador y no requestAnimationFrame: con la ventana en segundo plano el rAF se
  // CONGELA (medido) y la respuesta no crecía; el temporizador sólo se frena
  node._raf = setTimeout(() => { node._raf = 0; stick(() => (node.innerHTML = mdRender(node._raw))); }, 40);
}
// «copiar» de los bloques de código y enlaces (se abren en el navegador, nunca dentro de la app)
log.addEventListener("click", async (e) => {
  const cp = e.target.closest(".mdcopy");
  if (cp) {
    const code = cp.closest(".mdcode").querySelector("code").textContent;
    try { await navigator.clipboard.writeText(code); cp.textContent = tt("copied"); setTimeout(() => (cp.textContent = tt("copy")), 1400); } catch { toast("✗ " + tt("copyFail")); }
    return;
  }
  const a = e.target.closest("a[data-href]");
  if (a) { e.preventDefault(); invoke("open_link", { url: a.dataset.href }).catch((er) => toast("✗ " + er)); }
});

// ── Estado ────────────────────────────────────────────────────────────────
function setBusy(b) {
  S.busy = b;
  window.gotvDock?.trabajo(b);
  send.textContent = b ? "■" : "↵";
  send.classList.toggle("stop", b);
  send.disabled = !b && !input.value.trim();
  $("hint").textContent = b ? tt("working") : tt("hintIdle");
  $("engDot").className = "dot " + (b ? "busy" : S.ready ? "on" : "");
  engEstado();
  window.gotvCola?.ocupado(b); // F16.1: un solo botón — ■ detener con la caja vacía, ↵ a la cola con algo escrito
}
// F6: el pie lateral dice el ESTADO del motor (el modelo vive arriba, al lado del proyecto)
function engEstado() {
  if (!S.ready) return;
  // en reposo (al abrir la app) no hay modelo cargado: se carga al elegir un proyecto
  $("engTxt").textContent = tt(S.busy ? "engTrabajando" : S.reposo ? "engReposo" : "engListo");
  $("engDot").classList.toggle("on", !S.reposo && !S.busy);
}
function setStatus(s) {
  if (!s) return;
  if (s.model) { S.model = s.model; if (s.model_path) S.modelPath = s.model_path; pintarModelo(); }
  if (s.ready) { S.ready = true; $("engDot").className = "dot " + (S.busy ? "busy" : "on"); engEstado(); }
  if (s.max_steps) S.maxSteps = s.max_steps;
  if (s.mode) { $("mode").value = s.mode; $("mode").disabled = false; }
  if (s.root) setProject(s.root);
  if (s.ctx_budget) setCtx(s.ctx_used || 0, s.ctx_budget);
  $("attachBtn").title = tt(s.vision ? "attachVision" : "attachNoVision");
}
function setCtx(used, budget) {
  const pct = Math.min(100, Math.round((used * 100) / Math.max(1, budget)));
  $("ctxFill").style.width = pct + "%";
  $("ctxLbl").textContent = `${fmtK(used)} / ${fmtK(budget)} · ${pct} %`;
  $("ctx").classList.toggle("warn", pct >= 75);
  $("ctx").title = tt("ctxTip");
}
// ── La ventana de contexto: POR PROYECTO (cada uno puede usar otra IA; pedido del Portador
// 2026-11-13). Las opciones que admite el modelo: `cur` trae su tope (`ctx_top`: lo que GOTV aplica
// con «Máximo», la misma función que decide al arrancar) y si es de la nube. Las usan Configuración
// → Contexto y el menú del medidor de arriba.
// Las opciones salen del TOPE de la IA montada (pedido del Portador 2026-11-13: «el máximo va de
// acuerdo a la IA elegida; hay unas con hasta un millón»): tamaños que se doblan desde 8k (nube: 32k)
// por debajo del tope —8k … 256k, 512k, 1M…—, más el tope exacto. Un preset guardado que no esté en la
// escalera (p. ej. «400000») se conserva como opción si cabe.
const PRESET_TOKENS = { "8k": 8192, "16k": 16384, "32k": 32768, "64k": 65536, "128k": 131072, "256k": 262144, "512k": 524288, "1m": 1048576 };
const presetTokens = (p) => PRESET_TOKENS[String(p || "").trim().toLowerCase()] || (/^\d+$/.test(String(p || "").trim()) ? +p : 0);
const nombrePreset = (n) => Object.keys(PRESET_TOKENS).find((k) => PRESET_TOKENS[k] === n) || String(n);
function escaleraContexto(desde, top, saved) {
  const sizes = [];
  for (let n = desde; top && n < top; n *= 2) sizes.push([nombrePreset(n), n]);
  const s = presetTokens(saved);
  if (s && (!top || s < top) && !sizes.some(([, n]) => n === s)) sizes.push([String(saved).trim(), s]);
  return sizes.sort((a, b) => a[1] - b[1]);
}
function opcionesContexto(cur, saved) {
  const top = cur.ctx_top || 0;
  const conValor = (sizes) => (sizes.some(([v]) => v === String(saved || "").trim()) ? String(saved).trim() : "auto");
  if (cur.cloud) {
    // A3 · nube: «automático» = 200k (cada paso reenvía la ventana: cuida los créditos);
    // se puede subir hasta el máximo del modelo
    const auto = cur.ctx_auto || 200000;
    const sizes = escaleraContexto(32768, top, saved);
    if (top > auto) sizes.push([String(top), top]);
    const sel = conValor(sizes);
    return { sel, items: [["auto", `${tt("ctxCloudAuto")} · ${nf(Math.min(auto, top || auto))} tokens`]].concat(sizes.map(([v, n]) => [v, nf(n) + " tokens" + (n === top ? ` (${tt("ctxMax").toLowerCase()})` : "")])) };
  }
  // un preset guardado por encima del tope (p. ej. «128k» de antes con otra IA) ES el máximo
  const sizes = escaleraContexto(8192, top, saved);
  const sel = conValor(sizes);
  // si la memoria pone el límite, se dice cuánto admite la IA
  const porMem = cur.model_max && top && top < cur.model_max ? ` (${tt("ctxByMem").replace("{a}", nf(cur.model_max))})` : "";
  return { sel, items: [["auto", `${tt("ctxMax")} · ${top ? nf(top) + " tokens" : "—"}${porMem}`]].concat(sizes.map(([v, n]) => [v, nf(n) + " tokens"])) };
}

// clic en el medidor de arriba: las ventanas de ESTE modelo; la elegida es la de ESTE proyecto
async function menuVentana() {
  const viejo = document.querySelector(".ventana-menu");
  if (viejo) { viejo.remove(); return; }
  if (!S.root || S.busy) { toast(tt(S.busy ? "ctxBusy" : "ctxNoProject")); return; }
  const r = $("ctx").getBoundingClientRect();
  const m = document.createElement("div");
  m.className = "ventana-menu";
  m.setAttribute("role", "menu");
  m.style.top = r.bottom + 6 + "px";
  m.style.right = Math.max(8, innerWidth - r.right) + "px";
  m.innerHTML = `<div class="vm-tit">${esc(tt("ctxProject"))}</div><div class="vm-cargando">…</div>`;
  document.body.appendChild(m);
  let o;
  try { o = await invoke("contexto_opciones"); } catch (e) { m.remove(); toast("⚠ " + e); return; }
  if (!m.isConnected) return;
  const { sel, items } = opcionesContexto(o, o.preset || "auto");
  m.innerHTML = `<div class="vm-tit">${esc(tt("ctxProject"))}${o.model ? ` · <span class="mono">${esc(o.model)}</span>` : ""}</div>` +
    items.map(([v, label]) => `<button type="button" role="menuitemradio" data-v="${esc(v)}" aria-checked="${v === sel}">${esc(label)}</button>`).join("") +
    `<div class="vm-nota">${esc(tt("ctxProjectNote"))}</div>`;
  m.onclick = async (e) => {
    const b = e.target.closest("button[data-v]");
    if (!b) return;
    m.remove();
    if (b.dataset.v === sel) return;
    await invoke("apply_settings", { s: { context_preset: b.dataset.v } });
  };
  m.querySelector("button[aria-checked=true]")?.focus();
}
$("ctx").addEventListener("click", menuVentana);
$("ctx").addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); menuVentana(); } });
document.addEventListener("click", (e) => { const m = document.querySelector(".ventana-menu"); if (m && !m.contains(e.target) && !$("ctx").contains(e.target)) m.remove(); });
document.addEventListener("keydown", (e) => { if (e.key === "Escape") document.querySelector(".ventana-menu")?.remove(); });

function setProject(root) {
  S.root = root;
  S.modo = esCharla(root) ? "chat" : "code";
  if (S.modo === "code") S.ultimoProyecto = root;
  pintarModo();
  // el nombre que el usuario le dio en la app; si no, el de la carpeta
  const pr = (S.projects || []).find((p) => p.path === root);
  const name = (pr && pr.name) || root.split("/").filter(Boolean).pop() || root;
  $("projName").textContent = name;
  // la ruta ya no ocupa el encabezado (el espacio es del modelo): queda al pasar el ratón
  $("projName").title = root;
  $("modeloSel").hidden = false;
  document.title = `GOD OF THE VOID — ${name}`;
  renderProjects();
  rapidoEstado();
  if (window.Experto) Experto.proyecto(root);
}

// ── F4/F6 · la memoria rápida se está preparando ─────────────────────────────
// La ventana sólo aparece si hay que LEER mucho con el modelo (≥ 4 096 tokens: minutos en una
// conversación larga); leer del disco tarda segundos y basta el aviso de abajo.
const fmtN = (n) => Number(n || 0).toLocaleString(LANG === "en" ? "en-US" : "es-CO");
function durac(s) {
  if (!isFinite(s) || s <= 0) return "…";
  if (s < 60) return Math.ceil(s) + " s";
  return Math.round(s / 60) + " min";
}
function precarga(m) {
  const modal = $("rapidoModal");
  switch (m.fase) {
    case "inicio":
      S.pre = { t0: Date.now(), primero: null };
      $("hint").textContent = tt("rmHint");
      break;
    case "avance": {
      const p = S.pre || (S.pre = { t0: Date.now(), primero: null });
      if (!p.primero) p.primero = { t: Date.now(), done: m.done };
      if (m.total < 4096) { $("hint").textContent = `${tt("rmHint")} ${Math.floor((m.done * 100) / Math.max(1, m.total))} %`; break; }
      modal.classList.add("show");
      const pct = Math.floor((m.done * 100) / Math.max(1, m.total));
      $("rmFill").style.width = pct + "%";
      $("rmBarra").setAttribute("aria-valuenow", String(pct));
      // ritmo medido desde el primer avance (el arranque no cuenta)
      const seg = (Date.now() - p.primero.t) / 1000, hechos = m.done - p.primero.done;
      const quedan = hechos > 0 && seg > 0.5 ? (m.total - m.done) / (hechos / seg) : NaN;
      $("rmLbl").textContent = tt("rmProgreso").replace("{a}", fmtN(m.done)).replace("{b}", fmtN(m.total)).replace("{t}", durac(quedan));
      break;
    }
    default: {
      modal.classList.remove("show");
      S.pre = null;
      if (m.fase === "error") toast("✗ " + tt("rmError") + ": " + (m.error || ""), 9000);
      else if (m.fase === "cancelada") toast(tt("rmCancelada"), 4000);
      else if (m.fase === "lista" && m.tokens > 0) {
        $("hint").textContent = tt("rmLista").replace("{n}", fmtN(m.tokens)).replace("{s}", (m.seg || 0).toFixed(1));
        setTimeout(() => { if (!S.busy) $("hint").textContent = S.root ? tt("hintIdle") : $("hint").textContent; }, 6000);
      }
      rapidoEstado();
    }
  }
}
$("rmCancelar").onclick = () => invoke("stop_turn").catch(() => {});

// ── F5/F6 · el modelo del PROYECTO, al lado de su nombre ───────────────────
// nombre corto: la carpeta del modelo sin el dueño (mlx-community/…)
const nombreModelo = (m) => (m || "").split("/").filter(Boolean).pop() || m || "…";
function pintarModelo() {
  $("modeloNom").textContent = nombreModelo(S.model);
  $("modeloBtn").title = (S.modelPath || S.model || "") + " · " + tt("tModelo");
}
async function modelosLocales() {
  try { return (await invoke("models_list")).models || []; } catch { return []; }
}
function menuModelo(abrir) {
  const m = $("modeloMenu"), b = $("modeloBtn");
  m.hidden = !abrir;
  b.setAttribute("aria-expanded", String(abrir));
}
$("modeloBtn").onclick = async (e) => {
  e.stopPropagation();
  if (!$("modeloMenu").hidden) return menuModelo(false);
  const m = $("modeloMenu");
  m.innerHTML = `<div class="mm-cargando">${tt("loadingModels")}</div>`;
  menuModelo(true);
  const lista = await modelosLocales();
  m.innerHTML = "";
  for (const x of lista) {
    const b = el("button", "", "");
    b.type = "button"; b.setAttribute("role", "menuitemradio");
    const actual = x.path === S.modelPath || x.name === S.model || x.name === nombreModelo(S.model);
    b.setAttribute("aria-checked", String(actual));
    b.disabled = !x.fits && !actual;
    b.title = x.fits ? x.path : x.why || tt("noFits");
    b.innerHTML = `<span class="mm-nom"></span><span class="mm-gb mono">${x.gb ? x.gb + " GB" : ""}</span>`;
    b.querySelector(".mm-nom").textContent = x.name;
    b.onclick = () => {
      menuModelo(false);
      if (actual) return;
      $("engTxt").textContent = tt("loadingModel");
      invoke("set_model", { path: x.path }).catch((er) => toast("✗ " + er, 6000));
    };
    m.appendChild(b);
  }
  const mas = el("button", "mm-mas", tt("modeloMas"));
  mas.type = "button";
  mas.onclick = () => { menuModelo(false); openSettings("model"); };
  m.appendChild(mas);
  (m.querySelector('[aria-checked="true"]') || m.querySelector("button")).focus();
};
document.addEventListener("click", (e) => { if (!$("modeloSel").contains(e.target)) menuModelo(false); });
$("modeloMenu").onkeydown = (e) => {
  if (e.key === "Escape") { menuModelo(false); $("modeloBtn").focus(); }
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    const bs = [...$("modeloMenu").querySelectorAll("button:not(:disabled)")];
    const i = bs.indexOf(document.activeElement);
    bs[(i + (e.key === "ArrowDown" ? 1 : bs.length - 1)) % bs.length]?.focus();
    e.preventDefault();
  }
};
// ⚡ «Reabrir rápido»: la caché de la conversación de ESTE proyecto en disco (encendida, al reabrir la
// app no relee todo el historial; apagada, no ocupa nada y lo guardado se borra)
async function rapidoEstado(on, topeGb) {
  const b = $("rapidoBtn");
  if (!S.root) { b.hidden = true; $("rapidoTope").hidden = true; return; }
  try {
    const args = {};
    if (on !== undefined) args.on = on;
    if (topeGb !== undefined) args.topeGb = topeGb;
    const r = await invoke("cache_conversacion", args);
    b.hidden = false;
    b.setAttribute("aria-checked", r.on ? "true" : "false");
    b.classList.toggle("on", !!r.on);
    // F3: lo ocupado contra el tope del proyecto («2,4 / 3 GB»); el tope se elige al lado
    const tope = r.tope || 3 * 2 ** 30;
    $("rapidoPeso").textContent = r.on ? " · " + (r.bytes > 0 ? gb(r.bytes) + " / " : "") + Math.round(tope / 2 ** 30) + " GB" : "";
    const sel = $("rapidoTope");
    sel.hidden = !r.on;
    sel.value = String(Math.round(tope / 2 ** 30));
  } catch (e) { if (on !== undefined || topeGb !== undefined) toast("✗ " + e, 6000); }
}
$("rapidoTope").onchange = () => rapidoEstado(undefined, Number($("rapidoTope").value));
function gb(b) { return b >= 1e9 ? (b / 1e9).toLocaleString(LANG === "en" ? "en-US" : "es-CO", { maximumFractionDigits: 1 }) + " GB" : Math.max(1, Math.round(b / 1e6)) + " MB"; }
$("rapidoBtn").onclick = () => rapidoEstado($("rapidoBtn").getAttribute("aria-checked") !== "true");

// ── Proyectos ─────────────────────────────────────────────────────────────
const IC_CARPETA = `<svg class="ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" aria-hidden="true"><path d="M3.5 7.5a2 2 0 0 1 2-2h4l2 2.2h7a2 2 0 0 1 2 2v7.8a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z"/></svg>`;
// «Proyectos ⌄»: se pliega y lo recuerda Rust (`ventana_pref`, formato propio; antes, el localStorage)
(() => {
  const b = $("secProj"), lista = $("projects");
  const pon = (abierto) => { b.setAttribute("aria-expanded", String(abierto)); lista.hidden = !abierto; };
  let abierto = true;
  pon(abierto);
  invoke("ventana_pref", { clave: "proyectos" }).then((v) => { abierto = v !== "0"; pon(abierto); }).catch(() => {});
  b.onclick = () => { abierto = !abierto; pon(abierto); invoke("ventana_pref_poner", { clave: "proyectos", valor: abierto ? "1" : "0" }).catch(() => {}); };
})();
// el logo vivo de la cabecera (nace, 5 min girando, colapsa, 10 s de vacío, y otra vez)
if (window.GotvLogo) GotvLogo.montar($("brand"));
// el icono VIVO del Dock: la X nace al abrir la app y su galaxia gira mientras GOTV trabaja; en
// reposo vuelve el icono fijo del paquete. ~10 cuadros por segundo a 256 px (el Dock no pide más);
// con «reducir movimiento» no se anima. Los cuadros van EN FILA: el «vuelve el fijo» llega el último.
window.gotvDock = (() => {
  const nada = { trabajo() {} };
  if (!window.GotvLogo || !T?.core || matchMedia("(prefers-reduced-motion: reduce)").matches) return nada;
  const caja = document.createElement("div");
  GotvLogo.montar(caja, { solo: "x", icono: 256 });
  const L = caja._logo;
  if (!L) return nada;
  const N = L.tiempos.NACE;
  let t0 = performance.now(), reloj = 0, enVuelo = false, hasta = 0, fila = Promise.resolve();
  const mandar = (png) => (fila = fila.then(() => invoke("dock_icono", { png })).catch((e) => console.error("dock:", e)));
  const parar = () => { clearInterval(reloj); reloj = 0; mandar(null); };
  const paso = () => {
    const ahora = performance.now();
    if (ahora - t0 >= N && !S.busy && ahora >= hasta) return parar();
    if (enVuelo) return; // el anterior aún no llegó: se salta este cuadro, no se amontonan
    L.cuadro(Math.min(N, ahora - t0), 0, ahora);
    enVuelo = true;
    mandar(L.lienzo.toDataURL("image/png").slice("data:image/png;base64,".length)).finally(() => (enVuelo = false));
  };
  const andar = () => { if (!reloj) reloj = setInterval(paso, 100); };
  andar();
  // al terminar el trabajo, la galaxia da aún un respiro de 1,5 s antes de quedarse quieta
  return { trabajo(b) { if (b) andar(); else hasta = performance.now() + 1500; } };
})();
function renderProjects() {
  const box = $("projects");
  box.innerHTML = "";
  for (const p of S.projects) {
    const d = el("div", "proj" + (p.path === S.root ? " active" : ""));
    // una fila limpia: carpeta y nombre (la ruta, en el aviso al pasar el ratón); ✕ sólo al pasar
    d.title = p.path;
    d.setAttribute("role", "button");
    d.setAttribute("aria-label", p.name);
    d.tabIndex = 0;
    d.innerHTML = `${IC_CARPETA}<span class="n">${esc(p.name)}</span><button class="x" type="button" title="${esc(tt("quitarProyecto"))}" aria-label="${esc(tt("quitarProyecto"))}">✕</button>`;
    d.onkeydown = (e) => { if ((e.key === "Enter" || e.key === " ") && e.target === d) { e.preventDefault(); openProject(p.path); } };
    d.onclick = (e) => { if (!e.target.classList.contains("x")) openProject(p.path); };
    // F8.1: cada proyecto se despliega con sus chats (chats.js)
    if (window.gotvChats) gotvChats.decorar(d, p);
    d.querySelector(".x").onclick = async () => {
      if (p.path === S.root && S.busy) return toast(tt("working"));
      await invoke("forget_project", { path: p.path });
      S.projects = S.projects.filter((x) => x.path !== p.path);
      renderProjects();
      // quitar el proyecto ABIERTO lo cierra: se pasa al siguiente o se queda sin proyecto
      if (p.path === S.root) {
        if (S.projects.length) openProject(S.projects[0].path);
        else invoke("close_project");
      }
    };
    box.appendChild(d);
  }
}
async function refreshProjects() { const i = await invoke("info"); S.projects = i.projects || []; if (S.root) setProject(S.root); else renderProjects(); }
async function openProject(path) {
  if (path === S.root) return;
  if (S.busy) return toast(S.busy && tt("working"));
  try {
    await invoke("open_project", { path, trust: false });
    afterOpen(path);
  } catch (e) {
    if (String(e) === "untrusted") return askTrust(path);
    toast("✗ " + e, 5000);
  }
}
function askTrust(path) {
  $("trustPath").textContent = path;
  $("trustModal").classList.add("show");
  $("trustYes").onclick = async () => {
    $("trustModal").classList.remove("show");
    try { await invoke("open_project", { path, trust: true }); afterOpen(path); } catch (e) { toast("✗ " + e, 5000); }
  };
  $("trustNo").onclick = () => $("trustModal").classList.remove("show");
}
function afterOpen(path) { setBusy(true); log.innerHTML = ""; S.work = null; setProject(path); refreshProjects().then(() => aplicarChat(path)); }
function noProject() {
  S.root = null; log.innerHTML = ""; S.work = null;
  $("projName").textContent = tt("noProject"); $("projName").title = "";
  $("modeloSel").hidden = true; $("rapidoTope").hidden = true;
  document.title = "GOD OF THE VOID";
  input.disabled = true; send.disabled = true;
  $("hint").textContent = S.ready ? tt("noProjectHint") : tt("starting");
  const e = el("div", "empty");
  e.innerHTML = `<div><h3>${esc(tt("noProjectTitle"))}</h3><div>${esc(tt(S.reposo && S.projects.length ? "reposoBody" : "noProjectBody"))}</div><div class="actions" style="justify-content:center;margin-top:14px"><button class="btn primary" id="emptyNew">＋ ${esc(tt("newProject"))}</button></div></div>`;
  log.appendChild(e);
  S.modo = "code"; pintarModo();
  $("emptyNew").onclick = () => $("newProj").onclick();
  renderProjects();
}
// el nombre se propone desde la carpeta mientras el usuario no escriba el suyo
const baseName = (p) => p.replace(/\/+$/, "").split("/").pop() || "";
let nameTouched = false;
$("newProj").onclick = () => {
  $("newPath").value = "";
  $("newName").value = ""; $("newName").placeholder = tt("projNameAuto");
  nameTouched = false;
  $("newModal").classList.add("show");
  setTimeout(() => $("newPath").focus(), 50);
  // F5: el modelo del proyecto se elige al crearlo (por omisión, el que está cargado)
  const sel = $("newModel");
  sel.innerHTML = `<option value="">${esc(nombreModelo(S.model))}</option>`;
  modelosLocales().then((lista) => {
    sel.innerHTML = "";
    for (const x of lista.filter((x) => x.fits)) {
      const o = el("option", "", `${x.name}${x.gb ? " · " + x.gb + " GB" : ""}`);
      o.value = x.path;
      if (x.path === S.modelPath || x.name === S.model) o.selected = true;
      sel.appendChild(o);
    }
    if (!sel.options.length) sel.innerHTML = `<option value="">${esc(nombreModelo(S.model))}</option>`;
  });
};
$("newName").oninput = () => (nameTouched = $("newName").value.trim() !== "");
$("newPath").oninput = () => { if (!nameTouched) $("newName").placeholder = baseName($("newPath").value) || tt("projNameAuto"); };
$("newPick").onclick = async () => {
  const p = await invoke("pick_folder");
  if (!p) return;
  $("newPath").value = p;
  if (!nameTouched) $("newName").placeholder = baseName(p);
};
$("newCancel").onclick = () => $("newModal").classList.remove("show");
$("newCreate").onclick = async () => {
  try {
    const p = await invoke("new_project", { path: $("newPath").value, name: $("newName").value, model: $("newModel").value || null });
    $("newModal").classList.remove("show");
    afterOpen(p);
  } catch (e) { toast("✗ " + e, 5000); }
};

// ── Adjuntos y voz ────────────────────────────────────────────────────────
function addChip(c) {
  const icon = { image: "🖼", audio: "🎧", voice: "🎙", doc: "📄" }[c.kind] || "📎";
  const d = el("span", "chip");
  d.innerHTML = `${icon} ${esc(c.name)} <small>${esc(c.detail || "")}</small><button title="quitar">✕</button>`;
  d.querySelector("button").onclick = async () => { await invoke("detach", { id: c.id }); d.remove(); refreshSend(); };
  $("chips").appendChild(d);
  refreshSend();
}
async function attachPaths(paths) {
  for (const p of paths) {
    try { addChip(await invoke("attach_path", { path: p })); } catch (e) { toast("✗ " + e, 5000); }
  }
}
$("attachBtn").onclick = async () => attachPaths(await invoke("pick_files"));
// arrastrar y soltar (archivos de Finder llegan como rutas). Vía de Tauri 2:
// getCurrentWebview().onDragDropEvent; el evento global queda de respaldo.
(() => {
  const box = $("box");
  const wv = T.webview && T.webview.getCurrentWebview && T.webview.getCurrentWebview();
  if (wv && wv.onDragDropEvent) {
    wv.onDragDropEvent((e) => {
      const p = e.payload || {};
      if (p.type === "enter" || p.type === "over") box.classList.add("drag");
      else if (p.type === "drop") { box.classList.remove("drag"); attachPaths(p.paths || []); }
      else box.classList.remove("drag");
    });
  } else {
    T.event.listen("tauri://drag-drop", (e) => { box.classList.remove("drag"); attachPaths(e.payload.paths || []); });
    T.event.listen("tauri://drag-enter", () => box.classList.add("drag"));
    T.event.listen("tauri://drag-leave", () => box.classList.remove("drag"));
  }
})();
// pegar una imagen del portapapeles
input.addEventListener("paste", async (e) => {
  for (const it of e.clipboardData.items) {
    if (it.kind === "file") {
      e.preventDefault();
      const f = it.getAsFile();
      const b64 = await new Promise((r) => { const fr = new FileReader(); fr.onload = () => r(fr.result); fr.readAsDataURL(f); });
      try { addChip(await invoke("attach_bytes", { name: f.name || "pegado.png", b64 })); } catch (err) { toast("✗ " + err, 5000); }
    }
  }
});
// 🎧 M3 · manos libres: GOTV escucha, responde hablando mientras escribe, y calla si le hablas encima
$("manosBtn").onclick = async () => {
  const b = $("manosBtn"), on = !S.manos;
  // sin Whisper (oír) o sin la voz (hablar) no hay manos libres: a su fila en Generadores
  if (on && !(await exigeModelo("manos"))) return;
  b.disabled = true;
  try { await invoke("manos_libres", { on }); if (on) manosEstado("saludando"); else manosEstado("apagado"); }
  catch (e) { toast("✗ " + e, 9000); manosEstado("apagado"); faltaEnAviso(String(e), null, true); }
  finally { b.disabled = false; guardarChat(); }
};
function manosEstado(e) {
  S.manos = e !== "apagado";
  const b = $("manosBtn");
  b.classList.toggle("manos", S.manos);
  b.classList.toggle("oye", e === "oyendo");
  b.setAttribute("aria-pressed", S.manos ? "true" : "false");
  b.title = S.manos ? tt("handsOff") : tt("tHands");
  if (S.manos && !S.busy) $("hint").textContent = tt("hands_" + e) || "";
  else if (!S.manos && !S.busy) $("hint").textContent = tt("hintIdle");
}
$("micBtn").onclick = async () => {
  const b = $("micBtn");
  if (!S.recording) {
    // dictar pide Whisper
    if (!(await exigeModelo("oir"))) return;
    try { await invoke("voice_start"); S.recording = true; b.classList.add("rec"); $("hint").textContent = tt("recording"); } catch (e) { toast("✗ " + e, 5000); faltaEnAviso(String(e), null, true); }
    return;
  }
  S.recording = false; b.classList.remove("rec"); $("hint").textContent = tt("transcribing");
  try {
    const r = await invoke("voice_stop");
    if (r.chip) addChip(r.chip);
    if (r.text) { input.value = (input.value ? input.value + " " : "") + r.text; fit(); }
  } catch (e) { toast("✗ " + e, 6000); }
  setBusy(S.busy);
  input.focus();
};

// ── Envío ─────────────────────────────────────────────────────────────────
function fit() { input.style.height = "auto"; input.style.height = Math.min(input.scrollHeight, 220) + "px"; }
function refreshSend() { if (!S.busy) send.disabled = !(input.value.trim() || $("chips").children.length); }
// `boton`: el clic en ↵/■. Ocupado, lo escrito va a la cola (F16.1); sin nada escrito, el BOTÓN (■) detiene,
// pero Intro con la caja vacía no detiene nada
async function submit(boton) {
  if (S.busy) {
    if (window.gotvCola && gotvCola.enviar()) return;
    if (boton === true) invoke("stop_turn");
    return;
  }
  if (window.Experto && Experto.bloquea()) return toast(tt("expertoCandado"), 6000);
  const t = input.value.trim();
  const chips = [...$("chips").children].map((c) => c.textContent.replace("✕", "").trim());
  if (!t && !chips.length) return;
  // con el panel PRO encendido la orden se genera directo: si falta un modelo, a su fila antes de gastar
  // un minuto escribiendo (2026-11-19: el planificador de 4B sin bajar); lo escrito se queda en la caja
  if (proActivo() && !(await exigePerfil())) return;
  if (S.busy) return;
  const mac = S.mac && S.desktopOk;
  // ↺ un rebobinado pendiente se aplica AHORA: fuera de la pantalla y, en GOTV, la conversación se
  // corta antes de ese mensaje justo antes de correr el nuevo
  let rb = null;
  if (S.rb) { rb = { texto: S.rb.texto, n: S.rb.n }; S.rb.nodos.forEach((x) => x.remove()); S.rb = null; $("rbAviso").hidden = true; }
  addUser(t + (chips.length ? "\n" + chips.map((c) => "📎 " + c).join("\n") : ""), mac, t, Date.now());
  input.value = ""; fit(); $("chips").innerHTML = ""; hideSlash();
  setBusy(true);
  invoke("send", { task: t || (LANG === "en" ? "Look at the attachments." : "Mira los adjuntos."), mac, perfil: S.perfil, rebobinar: rb, pro: proDeOrden() });
}
send.onclick = () => submit(true);

// ── D7: perfil junto al botón de adjuntar (se recuerda POR PROYECTO, con Control Mac y manos libres) ──
S.perfil = "auto";
// el chat de cada proyecto queda como lo dejaste (el estado de la ventana en Rust, `projects[].chat`): se guarda cuando TÚ
// cambias algo y se vuelve a poner al abrir el proyecto o la app
function guardarChat() {
  if (!S.root) return;
  invoke("guardar_chat", { proyecto: S.root, mac: !!S.mac, perfil: S.perfil || null, manos: !!S.manos }).catch(() => {});
}
async function aplicarChat(root) {
  const c = ((S.projects || []).find((p) => p.path === root) || {}).chat || {};
  S.mac = !!c.mac;
  S.perfil = c.perfil || "auto";
  S.pro = proNormal(c.pro);
  drawPerfil();
  paintMac();
  paintPro();
  if (!!c.manos !== !!S.manos) {
    S.aplicandoChat = true;
    try { await invoke("manos_libres", { on: !!c.manos }); manosEstado(c.manos ? "saludando" : "apagado"); }
    catch (e) { toast("✗ " + e, 9000); manosEstado("apagado"); }
    // el «apagado» del núcleo llega después: se deja pasar antes de volver a recordar
    setTimeout(() => { S.aplicandoChat = false; }, 1500);
  }
}
// sólo el perfil elegido a la vista; al pulsarlo se despliega el menú (hacia arriba)
function drawPerfil() {
  for (const b of $("perfiles").querySelectorAll("button")) b.setAttribute("aria-checked", String(b.dataset.p === S.perfil));
  const sel = $("perfiles").querySelector(`[data-p="${S.perfil}"]`) || $("perfiles").querySelector("button");
  const ic = sel.querySelector("svg");
  $("perfilIc").innerHTML = ic ? ic.outerHTML : "";
  const sp = sel.querySelector("span");
  $("perfilNom").textContent = (sp.dataset.i && tt(sp.dataset.i)) || sp.textContent;
}
function menuPerfil(abrir) {
  const m = $("perfiles"), b = $("perfilAct");
  m.hidden = !abrir;
  b.setAttribute("aria-expanded", String(abrir));
  if (abrir) (m.querySelector('[aria-checked="true"]') || m.querySelector("button")).focus();
}
// por data-it: applyI18n la vuelve a traducir cuando el arranque fija el idioma (antes quedaba en español)
$("perfilAct").dataset.it = "pTip";
$("perfilAct").title = tt("pTip");
$("perfilAct").onclick = (e) => { e.stopPropagation(); menuPerfil($("perfiles").hidden); };
$("perfiles").onclick = (e) => {
  const b = e.target.closest("button[data-p]");
  if (!b) return;
  e.stopPropagation();
  S.perfil = b.dataset.p;
  drawPerfil();
  paintPro();
  avisarEstadoChat();
  guardarChat();
  menuPerfil(false);
  $("perfilAct").focus();
  // Imagen, Video o Música sin su modelo en esta Mac: a su fila en Generadores
  exigePerfil();
};
$("perfiles").onkeydown = (e) => {
  const items = [...$("perfiles").querySelectorAll("button")];
  const i = items.indexOf(document.activeElement);
  if (e.key === "Escape") { e.preventDefault(); menuPerfil(false); $("perfilAct").focus(); return; }
  const d = e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : 0;
  if (!d) return;
  e.preventDefault();
  items[(i + d + items.length) % items.length].focus();
};
document.addEventListener("click", (e) => { if (!$("perfiles").hidden && !e.target.closest("#perfilSel")) menuPerfil(false); });
drawPerfil();

// ── Ola PRO · modo profesional por perfil (PLAN.md). Hoy Video (PRO-2); Imagen y Música llegan en
// PRO-1 y PRO-3. El vocabulario (nombre + fragmento en inglés) vive en el núcleo (`pro_opciones`):
// aquí sólo se elige; con el switch encendido los ajustes viajan con la orden y GOTV genera directo.
const PRO_PERFILES = ["video", "imagen", "musica"];
const PRO_VIDEO_DEF = { formato: "16:9", calidad: "720p", segundos: 2, pasos: 50, guia: 5, pulir: true };
const PRO_IMAGEN_DEF = { proporcion: "1:1", resolucion: "1", variantes: 1, pasos: 0, guia: 0, base: false, colores: [], pulir: true };
const PRO_MUSICA_DEF = { idioma: "es", segundos: 30, bpm: 0, tonalidad: "", compas: "", pensar: true, creatividad: 0, fidelidad: 0, shift: 1, variantes: 1, instrumentos: [], fuerza_cover: 1, ruido_cover: 0, fuerza_repintar: 0.5, pulir: true };
function proNormal(p) {
  const o = p && typeof p === "object" ? p : {};
  // los paneles guardados antes de klein base traían 4 pasos fijos: ahora 0 = auto (4, o 50 con el base)
  const img = Object.assign({}, o.imagen || {});
  if (img.pasos === 4 && !img.base) img.pasos = 0;
  return { on: o.on && typeof o.on === "object" ? o.on : {}, video: Object.assign({}, PRO_VIDEO_DEF, o.video || {}), imagen: Object.assign({}, PRO_IMAGEN_DEF, img), musica: Object.assign({}, PRO_MUSICA_DEF, o.musica || {}), min: o.min && typeof o.min === "object" ? o.min : {} };
}
S.pro = proNormal(null);
function proActivo() { return PRO_PERFILES.includes(S.perfil) && !!S.pro.on[S.perfil]; }
// lo que viaja con la orden: sólo el perfil con el switch encendido
function proDeOrden() { return proActivo() ? { [S.perfil]: S.pro[S.perfil] } : null; }
let proGuardando = null;
function guardarPro() {
  if (!S.root) return;
  clearTimeout(proGuardando);
  proGuardando = setTimeout(() => invoke("guardar_pro", { proyecto: S.root, pro: S.pro }).catch(() => {}), 300);
}
function paintPro() {
  const tiene = PRO_PERFILES.includes(S.perfil), on = proActivo();
  $("proBtn").hidden = !tiene;
  $("proBtn").classList.toggle("on", on);
  $("proBtn").setAttribute("aria-checked", String(on));
  // encendido = el panel siempre está (abierto o minimizado en una barra); apagado = no está
  if (!on) $("proPanel").hidden = true;
  else if ($("proPanel").hidden || S.proPanelDe !== S.perfil) abrirPanelPro();
}
$("proBtn").onclick = () => {
  const p = S.perfil;
  S.pro.on[p] = !S.pro.on[p];
  paintPro();
  guardarPro();
  avisarEstadoChat();
  // al encenderlo, lo que ya estaba elegido en el panel también cuenta
  if (S.pro.on[p]) exigePerfil(false);
};

async function abrirPanelPro() {
  if (!S.proOpc) {
    try { S.proOpc = await invoke("pro_opciones"); } catch (e) { toast("✗ " + e, 6000); return; }
  }
  // qué modelos de música hay en esta Mac (se mira cada vez que se abre: pudo bajarse uno entretanto)
  if (S.perfil === "musica") {
    try { S.musicaListos = await invoke("musica_listos"); } catch { S.musicaListos = null; }
  }
  if (S.perfil === "video") panelProVideo();
  else if (S.perfil === "imagen") panelProImagen();
  else if (S.perfil === "musica") panelProMusica();
  S.proPanelDe = S.perfil;
  $("proPanel").hidden = false;
}

// la barra de arriba del panel: título, lo que sale (tamaño o tiempo), un resumen (sólo minimizado),
// restablecer y «−» para minimizar / «□» para volver a su tamaño (como una ventana)
const ICO_MIN = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M6 12h12"/></svg>`;
const ICO_MAX = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="6" y="6" width="12" height="12" rx="1.5"/></svg>`;
function proCabecera(titulo, extra = "") {
  return `<div class="pro-cab"><b>${esc(tt(titulo))}</b><span class="pro-est mono" id="proEst"></span><span class="pro-resumen" id="proResumen"></span>${extra}
      <button type="button" class="pro-txt" id="proReset">${esc(tt("proReset"))}</button>
      <button type="button" class="icon-btn pro-tam" id="proTam"></button></div>`;
}
// minimizado = una sola fila; se recuerda por perfil y por proyecto
function proMini() { return !!(S.pro.min || {})[S.perfil]; }
function pintarTamPro() {
  const P = $("proPanel"), mini = proMini(), b = $("proTam");
  P.classList.toggle("mini", mini);
  if (!b) return;
  b.innerHTML = mini ? ICO_MAX : ICO_MIN;
  b.title = tt(mini ? "proMaximizar" : "proMinimizar");
  b.setAttribute("aria-expanded", String(!mini));
  const s = $("proSufijo"), r = $("proResumen");
  if (r) r.textContent = mini && s ? s.textContent.replace(tt("proSufijo"), "").trim() : "";
}

// los campos de un panel (menús, botones segmentados y deslizadores) sobre los ajustes `v`
function proCampos(v) {
  const sel = (campo, lista, etiqueta) =>
    `<label class="pro-campo"><span>${esc(tt(etiqueta))}</span><select data-c="${campo}"><option value="">${esc(tt("proNada"))}</option>${lista
      .map((x) => `<option value="${esc(x.clave)}"${v[campo] === x.clave ? " selected" : ""}>${esc(x.nombre)}</option>`)
      .join("")}</select></label>`;
  const seg = (campo, lista, etiqueta, nombre) =>
    `<div class="pro-campo"><span>${esc(tt(etiqueta))}</span><div class="pro-seg" role="radiogroup">${lista
      .map((x) => `<button type="button" role="radio" data-c="${campo}" data-v="${esc(x.clave)}" aria-checked="${v[campo] === x.clave}" title="${esc(x.nombre)}">${esc(nombre ? x.nombre : x.clave)}</button>`)
      .join("")}</div></div>`;
  const rango = (campo, min, max, paso, etiqueta, unidad) =>
    `<label class="pro-campo"><span>${esc(tt(etiqueta))} <b class="mono" data-val="${campo}">${v[campo]}${unidad}</b></span><input type="range" data-c="${campo}" data-u="${unidad}" min="${min}" max="${max}" step="${paso}" value="${v[campo]}"></label>`;
  return { sel, seg, rango };
}

// conecta los campos del panel con `v`: cada cambio se guarda, viaja al núcleo y se repinta
function proConectar(P, v, pintar, rehacer, defecto) {
  // elegir en el panel algo que no está en esta Mac (un DiT, el planificador de 4B, klein base, un separador)
  // lleva a su fila; lo que falte ya avisado no vuelve a llevar a cada cambio
  let revisar = null;
  const cambio = (campo, valor) => {
    v[campo] = valor; guardarPro(); avisarEstadoChat(); pintar();
    clearTimeout(revisar);
    revisar = setTimeout(() => exigePerfil(false), 400);
  };
  P.querySelectorAll(".pro-seg button").forEach((b) => (b.onclick = () => {
    P.querySelectorAll(`.pro-seg button[data-c="${b.dataset.c}"]`).forEach((x) => x.setAttribute("aria-checked", String(x === b)));
    cambio(b.dataset.c, b.dataset.v);
  }));
  P.querySelectorAll("select[data-c]").forEach((s) => (s.onchange = () => cambio(s.dataset.c, s.value)));
  P.querySelectorAll('input[type="range"]').forEach((r) => (r.oninput = () => {
    P.querySelector(`[data-val="${r.dataset.c}"]`).textContent = r.value + (r.dataset.u || "");
    cambio(r.dataset.c, Number(r.value));
  }));
  P.querySelectorAll('input[type="checkbox"][data-c]').forEach((c) => (c.onchange = () => cambio(c.dataset.c, c.checked)));
  P.querySelectorAll("textarea[data-c], input[type=text][data-c]").forEach((t) => (t.oninput = () => cambio(t.dataset.c, t.value)));
  P.querySelectorAll('input[type="number"][data-c]').forEach((t) => (t.oninput = () => cambio(t.dataset.c, t.value === "" ? null : Math.max(0, Math.floor(Number(t.value))))));
  // OJO (falla medida en la app, 2026-11-13): el clic real cae en el DIBUJO del botón; al cambiar su
  // icono ese dibujo sale del documento y, si el clic sigue subiendo, la fila lo tomaba por «abrir» y
  // deshacía el minimizar en el acto. El clic del botón se queda en el botón.
  const alternar = () => {
    S.pro.min = Object.assign({}, S.pro.min, { [S.perfil]: !proMini() });
    guardarPro();
    pintarTamPro();
  };
  $("proTam").onclick = (e) => { e.stopPropagation(); alternar(); };
  // en la barra minimizada, un clic en el renglón (fuera de los botones) también la abre
  P.querySelector(".pro-cab").onclick = (e) => { if (proMini() && e.target.isConnected && !e.target.closest("button")) alternar(); };
  $("proReset").onclick = () => { S.pro[S.perfil] = Object.assign({}, defecto); guardarPro(); avisarEstadoChat(); rehacer(); };
  return cambio;
}

function panelProVideo() {
  const o = S.proOpc.video, v = S.pro.video, P = $("proPanel");
  const { sel, seg, rango } = proCampos(v);
  P.innerHTML = `
    ${proCabecera("proTitVideo")}
    <div class="pro-cuerpo">
      <div class="pro-grupo">${seg("formato", o.formato, "proFormato")}${seg("calidad", o.calidad, "proCalidad")}
        ${rango("segundos", 1, 5, 1, "proDuracion", " s")}${rango("pasos", 10, 100, 5, "proPasos", "")}${rango("guia", 1, 12, 0.5, "proGuia", "")}
        <p class="pro-aviso" id="proExp" hidden>⚠ ${esc(tt("proExperimental"))}</p></div>
      <div class="pro-grupo"><h4>${esc(tt("proCamara"))}</h4>${sel("plano", o.plano, "proPlano")}${sel("angulo", o.angulo, "proAngulo")}${sel("lente", o.lente, "proLente")}${sel("movimiento", o.movimiento, "proMov")}${sel("velocidad", o.velocidad, "proVel")}</div>
      <div class="pro-grupo"><h4>${esc(tt("proLuzColor"))}</h4>${sel("luz", o.luz, "proLuz")}${sel("hora", o.hora, "proHora")}${sel("tono", o.tono, "proTono")}${sel("estilo", o.estilo, "proEstilo")}</div>
      <div class="pro-grupo pro-ancho">
        <label class="pro-campo"><span>${esc(tt("proNegativo"))}</span><textarea data-c="negativo" rows="2" placeholder="${esc(tt("proNegPh"))}">${esc(v.negativo || "")}</textarea></label>
        <label class="pro-check"><input type="checkbox" data-c="sin_negativo_oficial"${v.sin_negativo_oficial ? " checked" : ""}> ${esc(tt("proSinOficial"))}</label>
        <label class="pro-check"><input type="checkbox" data-c="pulir"${v.pulir !== false ? " checked" : ""}> ${esc(tt("proPulir"))}</label>
        <label class="pro-campo pro-semilla"><span>${esc(tt("proSemilla"))}</span><input type="number" data-c="semilla" min="0" step="1" value="${v.semilla ?? ""}"></label>
        <p class="pro-sufijo" id="proSufijo"></p>
      </div>
    </div>`;
  proConectar(P, v, pintarProVideo, panelProVideo, PRO_VIDEO_DEF);
  pintarProVideo();
}

// PRO-3 · el panel de Música (ACE-Step 1.5): por PESTAÑAS para que cada pantalla quede corta
const PRO_TAREAS = [["extraer", "proTareaExtraer"], ["lego", "proTareaLego"], ["completar", "proTareaCompletar"]];
const PRO_MUSICA_TABS = [["estilo", "proTabEstilo"], ["voz", "proTabVoz"], ["meta", "proTabMeta"], ["motor", "proTabMotor"], ["audio", "proTabAudio"]];
function panelProMusica() {
  const o = S.proOpc.musica, v = S.pro.musica, P = $("proPanel");
  const { sel, seg, rango } = proCampos(v);
  const tab = S.proTabMusica || "estilo";
  // ✓ / ↓ en cada modelo del motor, según lo que hay en ESTA Mac (lo dice el núcleo; sin dato, sin marca)
  const L = S.musicaListos;
  const enMac = (clave, nombre) => (!L || !(clave in L) ? nombre : `${nombre} · ${tt(L[clave] ? "proEnMac" : "proSinBajar")}`);
  const faltaMusica = () => {
    if (!L) return "";
    const falta = [v.modelo, v.planificador].filter((c) => c && c in L && !L[c]);
    return falta.length ? `<p class="pro-sufijo pro-falta" role="alert">${esc(tt("proFaltaMusica"))}</p>` : "";
  };
  const familias = {};
  for (const x of o.instrumentos) (familias[x.clave.split(":")[0]] ||= []).push(x);
  const inst = Object.entries(familias)
    .map(([f, xs]) => `<div class="pro-fam"><span>${esc(tt("proFam_" + f))}</span>${xs.map((x) => `<label class="pro-chip"><input type="checkbox" data-inst="${esc(x.clave)}"${(v.instrumentos || []).includes(x.clave) ? " checked" : ""}>${esc(x.nombre)}</label>`).join("")}</div>`)
    .join("");
  const tonos = [{ clave: "", nombre: tt("proAuto") }, ...o.tonalidad];
  const compases = [["", tt("proAuto")], ["2", "2/4"], ["3", "3/4"], ["4", "4/4"], ["6", "6/8"]].map(([clave, nombre]) => ({ clave, nombre }));
  const cuerpo = {
    estilo: `<div class="pro-grupo">${sel("genero", o.genero, "proGenero")}${sel("epoca", o.epoca, "proEpoca")}${sel("animo", o.animo, "proAnimo")}</div>
      <div class="pro-grupo">${sel("textura", o.textura, "proTextura")}${sel("produccion", o.produccion, "proProduccion")}${sel("ritmo", o.ritmo, "proRitmo")}</div>
      <div class="pro-grupo pro-ancho"><label class="pro-campo"><span>${esc(tt("proEstiloLibre"))}</span><input type="text" data-c="estilo_libre" value="${esc(v.estilo_libre || "")}" placeholder="${esc(tt("proEstiloMusicaPh"))}"></label>
        <div class="pro-campo"><span>${esc(tt("proInstrumentos"))}</span>${inst}</div></div>`,
    voz: `<div class="pro-grupo">${sel("voz", o.voz, "proVoz")}${sel("tecnica", o.tecnica, "proTecnica")}${sel("idioma", o.idioma, "proIdioma")}</div>
      <div class="pro-grupo pro-ancho"><label class="pro-campo"><span>${esc(tt("proLetra"))}</span><textarea data-c="letra" id="proLetra" rows="6" placeholder="${esc(tt("proLetraPh"))}">${esc(v.letra || "")}</textarea></label>
        <div class="pro-etiquetas">${o.etiquetas.map((e) => `<button type="button" class="pro-txt" data-etq="${esc(e)}">${esc(e)}</button>`).join("")}</div>
        <p class="pro-sufijo">${esc(tt("proLetraAyuda"))}</p></div>`,
    meta: `<div class="pro-grupo">${rango("segundos", 10, 600, 5, "proDuracion", " s")}${rango("bpm", 0, 300, 1, "proBpm", "")}<p class="pro-sufijo">${esc(tt("proCeroAuto"))}</p></div>
      <div class="pro-grupo">${sel("tonalidad", tonos, "proTonalidad").replace(`<option value="">${esc(tt("proNada"))}</option>`, "")}${seg("compas", compases, "proCompas", true)}</div>`,
    motor: `<div class="pro-grupo">${sel("modelo", [["sft", "proModSft"], ["xl-turbo", "proModXlTurbo"], ["xl-sft", "proModXlSft"], ["xl-base", "proModXlBase"]].map(([clave, k]) => ({ clave, nombre: enMac(clave, tt(k)) })), "proModeloMusica")}${sel("planificador", [{ clave: "4b", nombre: enMac("4b", tt("proPlan4b")) }], "proPlanificador")}${faltaMusica()}<label class="pro-check"><input type="checkbox" data-c="base"${v.base ? " checked" : ""}> ${esc(tt("proMusicaBase"))}</label><label class="pro-check"><input type="checkbox" data-c="pensar"${v.pensar !== false ? " checked" : ""}> ${esc(tt("proPensar"))}</label>
        ${rango("creatividad", 0, 1.5, 0.05, "proCreatividad", "")}${rango("fidelidad", 0, 5, 0.25, "proFidelidad", "")}<p class="pro-sufijo">${esc(tt("proCeroOficial"))}</p></div>
      <div class="pro-grupo">${seg("shift", [1, 2, 3].map((n) => ({ clave: n, nombre: tt("proShift" + n) })), "proShift", true)}${rango("variantes", 1, 4, 1, "proVariantes", "")}
        <label class="pro-campo pro-semilla"><span>${esc(tt("proSemilla"))}</span><input type="number" data-c="semilla" min="0" step="1" value="${v.semilla ?? ""}"></label>
        <label class="pro-check"><input type="checkbox" data-c="pulir"${v.pulir !== false ? " checked" : ""}> ${esc(tt("proPulirMusica"))}</label></div>
      <div class="pro-grupo pro-ancho"><label class="pro-campo"><span>${esc(tt("proExcluir"))}</span><input type="text" data-c="excluir" value="${esc(v.excluir || "")}" placeholder="${esc(tt("proExcluirPh"))}"></label>
        <p class="pro-sufijo">${esc(tt("proPesosMusica"))}</p>
        <div class="pro-campo"><span>${esc(tt("proSeparar"))}</span><div class="pro-fam">${(o.separables || []).map((x) => `<label class="pro-chip"><input type="checkbox" data-separar="${esc(x.clave)}"${(v.separar || []).includes(x.clave) ? " checked" : ""}>${esc(x.nombre)}</label>`).join("")}</div></div>
        <label class="pro-check"><input type="checkbox" id="proSepFino"${v.separador === "ft" ? " checked" : ""}> ${esc(tt("proSepararFino"))}</label>
        <p class="pro-sufijo">${esc(tt("proSepararAyuda"))}</p></div>`,
    audio: `<div class="pro-grupo pro-ancho"><label class="pro-campo"><span>${esc(tt("proTimbre"))}</span><input type="text" data-c="timbre" value="${esc(v.timbre || "")}" placeholder="${esc(tt("proTimbrePh"))}"></label><p class="pro-sufijo">${esc(tt("proTimbreAyuda"))}</p>
        <label class="pro-campo"><span>${esc(tt("proAudioPropio"))}</span><input type="text" data-c="audio" value="${esc(v.audio || "")}" placeholder="${esc(tt("proAudioPh"))}"></label>
        ${rango("fuerza_cover", 0, 1, 0.05, "proFuerzaCover", "")}${rango("ruido_cover", 0, 0.95, 0.05, "proRuidoCover", "")}
        <label class="pro-campo"><span>${esc(tt("proRepintar"))}</span><input type="text" data-c="repintar" value="${esc(v.repintar || "")}" placeholder="10-18"></label>
        ${rango("fuerza_repintar", 0, 1, 0.05, "proFuerzaRepintar", "")}<p class="pro-sufijo">${esc(tt("proAudioAyuda"))}</p>
        ${sel("tarea", PRO_TAREAS.map(([clave, k]) => ({ clave, nombre: tt(k) })), "proTarea")}
        <div class="pro-campo"><span>${esc(tt("proPistas"))}</span><div class="pro-fam">${o.pistas.map((x) => `<label class="pro-chip"><input type="checkbox" data-pista="${esc(x.clave)}"${(v.pistas || []).includes(x.clave) ? " checked" : ""}>${esc(x.nombre)}</label>`).join("")}</div></div>
        <p class="pro-sufijo">${esc(tt("proTareaAyuda"))}</p></div>`,
  };
  P.innerHTML = `
    ${proCabecera("proTitMusica", `<button type="button" class="btn sm pro-experto" id="proExperto" title="${esc(tt("expertoAbrirTip"))}">🎛️ ${esc(tt("expertoAbrir"))}</button>`)}
    <div class="pro-tabs" role="tablist">${PRO_MUSICA_TABS.map(([k, t]) => `<button type="button" role="tab" data-tab="${k}" aria-selected="${k === tab}">${esc(tt(t))}</button>`).join("")}</div>
    <div class="pro-cuerpo">${cuerpo[tab]}<p class="pro-sufijo pro-ancho" id="proSufijo"></p></div>`;
  const cambio = proConectar(P, v, pintarProMusica, panelProMusica, PRO_MUSICA_DEF);
  P.querySelectorAll(".pro-tabs button").forEach((b) => (b.onclick = (e) => { e.stopPropagation(); S.proTabMusica = b.dataset.tab; panelProMusica(); }));
  const exB = P.querySelector("#proExperto");
  if (exB) exB.onclick = (e) => { e.stopPropagation(); if (window.Experto) Experto.abrir(); };
  P.querySelectorAll("input[data-inst]").forEach((c) => (c.onchange = () => cambio("instrumentos", [...P.querySelectorAll("input[data-inst]:checked")].map((x) => x.dataset.inst))));
  P.querySelectorAll("input[data-pista]").forEach((c) => (c.onchange = () => cambio("pistas", [...P.querySelectorAll("input[data-pista]:checked")].map((x) => x.dataset.pista))));
  P.querySelectorAll("input[data-separar]").forEach((c) => (c.onchange = () => cambio("separar", [...P.querySelectorAll("input[data-separar]:checked")].map((x) => x.dataset.separar))));
  const fino = P.querySelector("#proSepFino");
  if (fino) fino.onchange = () => cambio("separador", fino.checked ? "ft" : "");
  // el shift llega como número
  P.querySelectorAll('.pro-seg button[data-c="shift"]').forEach((b) => b.addEventListener("click", () => { v.shift = Number(b.dataset.v); guardarPro(); avisarEstadoChat(); }));
  P.querySelectorAll("button[data-etq]").forEach((b) => (b.onclick = () => {
    const t = $("proLetra"), e = b.dataset.etq, i = t.selectionStart ?? t.value.length;
    const antes = t.value.slice(0, i), despues = t.value.slice(i);
    t.value = antes + (antes && !antes.endsWith("\n") ? "\n" : "") + e + "\n" + despues;
    cambio("letra", t.value);
    t.focus();
  }));
  pintarTamPro();
  pintarProMusica();
}

function pintarProMusica() {
  const v = S.pro.musica, o = S.proOpc.musica;
  const frag = (campo) => ((o[campo] || []).find((x) => x.clave === v[campo]) || {}).nombre;
  const partes = ["genero", "epoca", "animo", "voz", "tecnica", "textura", "produccion", "ritmo"].map(frag).filter(Boolean);
  const inst = (v.instrumentos || []).map((c) => (o.instrumentos.find((x) => x.clave === c) || {}).nombre).filter(Boolean);
  if (inst.length) partes.push(inst.join(", "));
  if ((v.excluir || "").trim()) partes.push(`✕ ${v.excluir.trim()}`);
  $("proSufijo").textContent = partes.length ? `${tt("proSufijo")} ${partes.join(" · ")}` : tt("proSinSufijo");
  pintarTamPro();
  $("proEst").textContent = `${v.segundos || 30} s${v.bpm ? ` · ${v.bpm} bpm` : ""}${v.tonalidad ? ` · ${v.tonalidad}` : ""} · ${v.variantes || 1}×`;
}

// PRO-1 · el panel de Imagen (FLUX.2 klein: sin negativo, porque el destilado no lo tiene)
function panelProImagen() {
  const o = S.proOpc.imagen, v = S.pro.imagen, P = $("proPanel");
  const { sel, seg, rango } = proCampos(v);
  P.innerHTML = `
    ${proCabecera("proTitImagen")}
    <div class="pro-cuerpo">
      <div class="pro-grupo">${seg("proporcion", o.proporcion, "proProporcion")}${seg("resolucion", o.resolucion, "proResolucion", true)}
        ${rango("variantes", 1, 4, 1, "proVariantes", "")}${rango("pasos", 0, 50, 1, "proPasos", "")}
        <p class="pro-sufijo">${esc(tt("proPasosKlein"))}</p>
        <label class="pro-check"><input type="checkbox" data-c="base"${v.base ? " checked" : ""}> ${esc(tt("proKleinBase"))}</label>
        ${rango("guia", 0, 10, 0.5, "proGuiaCfg", "")}
        <label class="pro-campo"><span>${esc(tt("proNegativoImagen"))}</span><textarea data-c="negativo" rows="2" placeholder="${esc(tt("proNegPh"))}">${esc(v.negativo || "")}</textarea></label></div>
      <div class="pro-grupo"><h4>${esc(tt("proEstiloFoto"))}</h4>${sel("tipo", o.tipo, "proTipo")}${sel("camara", o.camara, "proCamaraFoto")}${sel("lente", o.lente, "proLente")}${sel("apertura", o.apertura, "proApertura")}${sel("pelicula", o.pelicula, "proPelicula")}</div>
      <div class="pro-grupo"><h4>${esc(tt("proEncuadreLuz"))}</h4>${sel("plano", o.plano, "proPlano")}${sel("angulo", o.angulo, "proAngulo")}${sel("composicion", o.composicion, "proComposicion")}${sel("luz", o.luz, "proLuz")}${sel("hora", o.hora, "proHora")}</div>
      <div class="pro-grupo pro-ancho">
        <label class="pro-campo"><span>${esc(tt("proEstiloLibre"))}</span><input type="text" data-c="estilo_libre" value="${esc(v.estilo_libre || "")}" placeholder="${esc(tt("proEstiloLibrePh"))}"></label>
        <label class="pro-campo"><span>${esc(tt("proColores"))}</span><textarea id="proColores" rows="2" placeholder="${esc(tt("proColoresPh"))}">${esc((v.colores || []).join("\n"))}</textarea></label>
        <label class="pro-campo"><span>${esc(tt("proReferencias"))}</span><textarea id="proReferencias" rows="2" placeholder="${esc(tt("proReferenciasPh"))}">${esc((v.referencias || []).join("\n"))}</textarea></label>
        <label class="pro-campo"><span>${esc(tt("proTexto"))}</span><input type="text" data-c="texto" value="${esc(v.texto || "")}" placeholder="${esc(tt("proTextoPh"))}"></label>
        <label class="pro-check"><input type="checkbox" data-c="pulir"${v.pulir !== false ? " checked" : ""}> ${esc(tt("proPulir"))}</label>
        <label class="pro-campo pro-semilla"><span>${esc(tt("proSemilla"))}</span><input type="number" data-c="semilla" min="0" step="1" value="${v.semilla ?? ""}"></label>
        <p class="pro-sufijo" id="proSufijo"></p>
      </div>
    </div>`;
  const cambio = proConectar(P, v, pintarProImagen, panelProImagen, PRO_IMAGEN_DEF);
  $("proColores").oninput = (e) => cambio("colores", e.target.value.split("\n").map((l) => l.trim()).filter(Boolean));
  $("proReferencias").oninput = (e) => cambio("referencias", e.target.value.split("\n").map((l) => l.trim()).filter(Boolean).slice(0, 4));
  pintarProImagen();
}

function pintarProImagen() {
  const v = S.pro.imagen, o = S.proOpc.imagen;
  const frag = (campo) => ((o[campo] || []).find((x) => x.clave === v[campo]) || {}).nombre;
  const partes = ["tipo", "camara", "lente", "apertura", "pelicula", "plano", "angulo", "composicion", "luz", "hora"].map(frag).filter(Boolean);
  if ((v.colores || []).length) partes.push(v.colores.join(" · "));
  if ((v.texto || "").trim()) partes.push(`«${v.texto.trim()}»`);
  $("proSufijo").textContent = partes.length ? `${tt("proSufijo")} ${partes.join(" · ")}` : tt("proSinSufijo");
  pintarTamPro();
  // el tamaño que saldrá (el mismo cálculo que `tamano_imagen` del núcleo)
  const [a, b] = (v.proporcion || "1:1").split(":").map(Number);
  let w = Math.sqrt(Number(v.resolucion || 1) * 1048576 * a / b), h = w * b / a;
  const tope = Math.max(w, h) / 2048;
  if (tope > 1) { w /= tope; h /= tope; }
  const q = (x) => Math.min(2048, Math.max(256, Math.round(x / 16) * 16));
  $("proEst").textContent = `${q(w)}×${q(h)} · ${v.variantes || 1}×`;
}

// lo que se ve sin generar: el tiempo estimado (con lo MEDIDO), si es experimental y lo que se suma
// al prompt (el mismo texto que pondrá el núcleo, en el orden de la fórmula de Wan)
async function pintarProVideo() {
  const v = S.pro.video, o = S.proOpc.video;
  $("proExp").hidden = !(v.formato === "1:1" && v.calidad !== "480p");
  const frag = (campo) => ((o[campo] || []).find((x) => x.clave === v[campo]) || {}).nombre;
  const partes = ["plano", "angulo", "lente", "movimiento", "luz", "hora", "tono", "estilo"].map(frag).filter(Boolean);
  if (v.movimiento && v.movimiento !== "fija" && v.velocidad && v.velocidad !== "normal") partes.splice(partes.indexOf(frag("movimiento")), 1, `${frag("movimiento")} · ${frag("velocidad")}`);
  $("proSufijo").textContent = partes.length ? `${tt("proSufijo")} ${partes.join(" · ")}` : tt("proSinSufijo");
  pintarTamPro();
  try {
    const m = await invoke("pro_video_estimado", { video: v, segundos: v.segundos });
    $("proEst").textContent = tt("proEstimado").replace("{m}", Math.max(1, Math.round(m)));
  } catch { $("proEst").textContent = ""; }
}

// ── D6/D7: vista de una página o isométrica, en un marco AISLADO (sin red, sin acceso a la app)
const CSP_VISTA = `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; media-src data:; style-src 'unsafe-inline'; font-src data:; script-src 'unsafe-inline'">`;
// D6: SEÑALAR. El marco no comparte origen con la app (sandbox sin allow-same-origin): sólo
// hablan por postMessage. Este guion va en la VISTA, nunca en los archivos escritos.
const SENALAR_JS = `<script>(()=>{let on=false,ult=null;const st=document.createElement("style");
st.textContent="[data-gotv-sobre]{outline:2px dashed #e11d48!important;outline-offset:2px!important;cursor:crosshair!important}";
document.head.appendChild(st);
const quita=()=>{if(ult){ult.removeAttribute("data-gotv-sobre");ult=null}};
addEventListener("message",(e)=>{if(e.source!==parent||!e.data||e.data.gotv!=="senalar")return;on=!!e.data.on;if(!on)quita()});
addEventListener("mouseover",(e)=>{if(!on)return;quita();ult=e.target;ult.setAttribute("data-gotv-sobre","")},true);
addEventListener("click",(e)=>{if(!on)return;e.preventDefault();e.stopPropagation();const t=e.target,s=t.closest("[data-gotv]");
parent.postMessage({gotv:"senalado",seccion:s?+s.dataset.gotv:null,tipo:s?s.dataset.gotvTipo:"",etiqueta:t.tagName.toLowerCase(),
texto:(t.innerText||t.getAttribute("alt")||t.getAttribute("aria-label")||"").trim().replace(/\\s+/g," ").slice(0,90)},"*");on=false;quita()},true);})()<\/script>`;
const docVista = (m, html) => m.tipo === "svg"
  ? `<!doctype html><html><head><meta charset="utf-8">${CSP_VISTA}<style>html,body{margin:0;height:100%;background:#fafafa;display:grid;place-items:center}svg{max-width:100%;max-height:100vh}</style></head><body>${html}</body></html>`
  : String(html || "").replace(/<head>/i, `<head>${CSP_VISTA}`).replace(/<\/body>/i, `${SENALAR_JS}</body>`);
// ── M7 · lo GENERADO (música, efecto, imagen): la ubicación y la licencia NO van en la conversación
// (el parlante y los manos libres las leían): viven en el botón ⓘ de su tarjeta.
const IC_PLAY = `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8.5 5.8v12.4a.8.8 0 0 0 1.2.7l10-6.2a.8.8 0 0 0 0-1.4l-10-6.2a.8.8 0 0 0-1.2.7z" fill="currentColor"/></svg>`;
const IC_PAUSA = `<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="7" y="5.5" width="3.6" height="13" rx="1.1" fill="currentColor"/><rect x="13.4" y="5.5" width="3.6" height="13" rx="1.1" fill="currentColor"/></svg>`;
const IC_INFO = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><circle cx="12" cy="12" r="8.5"/><path d="M12 11v5"/><path d="M12 7.8v.1"/></svg>`;
const IC_ABRIR = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 5h5v5"/><path d="M19 5l-8 8"/><path d="M17 14v4a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V8a1 1 0 0 1 1-1h4"/></svg>`;
const mmss = (s) => { s = Math.max(0, Math.floor(s || 0)); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`; };
const abrirArchivo = (p) => invoke("open_vista", { path: String(p || "") }).catch((e) => toast("✗ " + e, 5000));
// [botón ⓘ, panel] de una tarjeta: archivo (clic = copiar la ruta), licencia (ver) y detalle
function infoTarjeta(m) {
  const i = m.info;
  if (!i) return [null, null];
  const b = el("button", "tj-info", "");
  b.type = "button"; b.innerHTML = IC_INFO;
  b.title = tt("infoVer"); b.setAttribute("aria-label", tt("infoVer")); b.setAttribute("aria-expanded", "false");
  const p = el("dl", "tj-det", "");
  p.hidden = true;
  const fila = (k, v, ruta, ver) => {
    if (!v) return;
    p.appendChild(el("dt", "", k));
    const dd = el("dd", "", "");
    const s = el("span", ruta ? "tj-ruta" : "", v);
    if (ruta) {
      s.title = tt("infoCopiar");
      s.onclick = () => navigator.clipboard.writeText(v).then(() => toast("✓ " + tt("infoCopiado"), 2500), () => {});
    }
    dd.appendChild(s);
    if (ver) {
      const vb = el("button", "", tt("infoVerLicencia"));
      vb.type = "button"; vb.onclick = () => abrirArchivo(v);
      dd.appendChild(vb);
    }
    p.appendChild(dd);
  };
  fila(tt("infoArchivo"), String(i.ruta || m.ruta || ""), true, false);
  fila(tt("infoLicencia"), i.ficha ? String(i.ficha) : "", true, true);
  fila(tt("infoDetalle"), String(i.detalle || ""), false, false);
  fila(tt("infoTiempo"), String(i.tiempo || ""), false, false);
  b.onclick = () => { p.hidden = !p.hidden; b.setAttribute("aria-expanded", String(!p.hidden)); b.classList.toggle("on", !p.hidden); };
  return [b, p];
}
// el reproductor PROPIO de la app (no el de WebKit dentro de un marco): ▶ · título y barra · tiempo · ⓘ · ↗
function addPista(m) {
  clearEmpty();
  const w = el("div", "pista-w", "");
  const card = el("div", "pista", "");
  card.innerHTML = `<button class="pista-play" type="button"></button><div class="pista-cuerpo"><div class="pista-tit"></div><div class="pista-barra" role="slider" tabindex="0" aria-valuemin="0"><span class="pista-lleno"></span></div></div><span class="pista-t"></span><button class="tj-abrir" type="button"></button>`;
  const [play, tit, barra, lleno, t, abrir] = [".pista-play", ".pista-tit", ".pista-barra", ".pista-lleno", ".pista-t", ".tj-abrir"].map((s) => card.querySelector(s));
  tit.textContent = m.titulo || ""; tit.title = m.titulo || "";
  barra.setAttribute("aria-label", tt("pistaPos"));
  abrir.innerHTML = IC_ABRIR; abrir.title = tt("pistaAbrir"); abrir.setAttribute("aria-label", tt("pistaAbrir"));
  abrir.onclick = () => abrirArchivo(m.ruta);
  card.oncontextmenu = (e) => menuArchivo(e, m.ruta, play); // clic derecho: abrir o guardar
  const a = new Audio();
  a.preload = "metadata";
  a.src = m.audio;
  const dur = () => (Number.isFinite(a.duration) && a.duration) || m.segundos || 0;
  const pinta = () => {
    const d = dur(), c = Math.min(a.currentTime || 0, d);
    lleno.style.width = d ? (100 * c / d).toFixed(2) + "%" : "0";
    t.textContent = `${mmss(c)} / ${mmss(d)}`;
    barra.setAttribute("aria-valuemax", String(Math.round(d)));
    barra.setAttribute("aria-valuenow", String(Math.round(c)));
    barra.setAttribute("aria-valuetext", `${mmss(c)} / ${mmss(d)}`);
  };
  const icono = () => { const p = !a.paused; play.innerHTML = p ? IC_PAUSA : IC_PLAY; const x = tt(p ? "pistaPausa" : "pistaPlay"); play.title = x; play.setAttribute("aria-label", x); card.classList.toggle("sonando", p); };
  play.onclick = () => {
    if (!a.paused) { a.pause(); return; }
    for (const o of (S.pistas || [])) if (o !== a) o.pause(); // una sola suena a la vez
    a.play().catch((e) => toast("✗ " + e.message, 6000));
  };
  a.onplay = a.onpause = icono;
  a.ontimeupdate = a.onloadedmetadata = pinta;
  a.onended = () => { icono(); pinta(); };
  a.onerror = () => { card.classList.add("err"); t.textContent = "✗"; t.title = `MediaError ${a.error && a.error.code}`; };
  const ir = (x) => { const d = dur(); if (d) { a.currentTime = Math.max(0, Math.min(d, x)); pinta(); } };
  const aRatón = (e) => { const r = barra.getBoundingClientRect(); ir(dur() * (e.clientX - r.left) / r.width); };
  barra.onpointerdown = (e) => { barra.setPointerCapture(e.pointerId); aRatón(e); barra.onpointermove = aRatón; };
  barra.onpointerup = () => { barra.onpointermove = null; };
  barra.onkeydown = (e) => {
    const k = { ArrowRight: 5, ArrowLeft: -5, ArrowUp: 5, ArrowDown: -5 }[e.key];
    if (k) { e.preventDefault(); ir(a.currentTime + k); }
    else if (e.key === "Home") { e.preventDefault(); ir(0); }
    else if (e.key === "End") { e.preventDefault(); ir(dur()); }
    else if (e.key === " ") { e.preventDefault(); play.click(); }
  };
  (S.pistas = S.pistas || []).push(a);
  icono(); pinta();
  const [ib, ip] = infoTarjeta(m);
  if (ib) card.insertBefore(ib, abrir);
  w.appendChild(card);
  if (ip) w.appendChild(ip);
  stick(() => log.appendChild(w));
}
// clic derecho en lo generado: «Abrir» (la app del Mac) y «Guardar como…» (el diálogo nativo)
function cerrarMenu() { if (S.ctxMenu) { S.ctxMenu.remove(); S.ctxMenu = null; } }
function menuArchivo(e, ruta, ancla) {
  e.preventDefault();
  e.stopPropagation();
  cerrarMenu();
  const m = el("div", "ctx-menu", "");
  m.setAttribute("role", "menu");
  const op = (texto, hacer) => {
    const b = el("button", "", texto);
    b.type = "button"; b.setAttribute("role", "menuitem");
    b.onclick = (ev) => { ev.stopPropagation(); cerrarMenu(); hacer(); };
    m.appendChild(b);
  };
  op(tt("ctxAbrir"), () => abrirArchivo(ruta));
  op(tt("ctxGuardar"), async () => {
    try { const d = await invoke("guardar_como", { path: String(ruta || "") }); if (d) toast("✓ " + tt("ctxGuardado") + " " + d, 5000); }
    catch (x) { toast("✗ " + x, 6000); }
  });
  m.onkeydown = (ev) => {
    const items = [...m.querySelectorAll("button")], i = items.indexOf(document.activeElement);
    if (ev.key === "Escape") { ev.preventDefault(); cerrarMenu(); ancla && ancla.focus(); }
    else if (ev.key === "ArrowDown" || ev.key === "ArrowUp") { ev.preventDefault(); items[(i + (ev.key === "ArrowDown" ? 1 : -1) + items.length) % items.length].focus(); }
  };
  document.body.appendChild(m);
  // donde se hizo clic (o junto al elemento si vino del teclado), siempre dentro de la ventana
  let x = e.clientX, y = e.clientY;
  if (!x && !y && ancla) { const r = ancla.getBoundingClientRect(); x = r.left + 12; y = r.top + 12; }
  const r = m.getBoundingClientRect();
  m.style.left = Math.max(8, Math.min(x, innerWidth - r.width - 8)) + "px";
  m.style.top = Math.max(8, Math.min(y, innerHeight - r.height - 8)) + "px";
  S.ctxMenu = m;
  m.querySelector("button").focus();
}
document.addEventListener("click", cerrarMenu);
window.addEventListener("blur", cerrarMenu);
window.addEventListener("resize", cerrarMenu);
log.addEventListener("scroll", cerrarMenu, { passive: true });
// la imagen generada: SOLA, como la muestra cualquier IA (sin marco ni barra); ⓘ abajo, como en las
// canciones; clic derecho: abrir o guardar
function addImagen(m) {
  clearEmpty();
  const f = el("figure", "gen-img", "");
  const img = el("img", "", undefined);
  img.src = m.imagen;
  img.alt = m.titulo || "";
  if (m.ancho && m.alto) { img.width = m.ancho; img.height = m.alto; }
  img.tabIndex = 0;
  img.title = tt("ctxPista");
  img.oncontextmenu = (e) => menuArchivo(e, m.ruta, img);
  f.appendChild(img);
  const [ib, ip] = infoTarjeta(m);
  if (ib) { const pie = el("div", "gen-pie", ""); pie.appendChild(ib); f.appendChild(pie); }
  if (ip) f.appendChild(ip);
  stick(() => log.appendChild(f));
}
// el video generado: como la imagen, SOLO (sin marco), con un ▶ discreto en el centro, una barra
// fina abajo y el tiempo; ⓘ al pie; clic derecho: abrir o guardar. El MP4 NO viaja en la
// conversación: se pide al núcleo (`leer_video`, binario) cuando la tarjeta entra en pantalla.
function addVideo(m) {
  clearEmpty();
  const f = el("figure", "gen-img gen-video", "");
  const marco = el("div", "gv-marco", "");
  if (m.ancho && m.alto) marco.style.aspectRatio = `${m.ancho} / ${m.alto}`;
  if (m.alto > m.ancho) f.classList.add("vertical");
  marco.innerHTML = `<video playsinline loop preload="metadata"></video><button class="gv-play" type="button"></button><div class="gv-barra" role="slider" tabindex="0" aria-valuemin="0"><span class="gv-lleno"></span></div><span class="gv-t"></span>`;
  const [v, play, barra, lleno, t] = ["video", ".gv-play", ".gv-barra", ".gv-lleno", ".gv-t"].map((s) => marco.querySelector(s));
  v.muted = true; // Wan 2.2 no hace sonido
  v.setAttribute("aria-label", m.titulo || "video");
  marco.title = tt("ctxPista");
  barra.setAttribute("aria-label", tt("pistaPos"));
  marco.oncontextmenu = (e) => menuArchivo(e, m.ruta, play);
  const dur = () => (Number.isFinite(v.duration) && v.duration) || m.segundos || 0;
  const pinta = () => {
    const d = dur(), c = Math.min(v.currentTime || 0, d);
    lleno.style.width = d ? (100 * c / d).toFixed(2) + "%" : "0";
    t.textContent = `${mmss(c)} / ${mmss(d)}`;
    barra.setAttribute("aria-valuemax", String(Math.round(d)));
    barra.setAttribute("aria-valuenow", String(Math.round(c)));
  };
  const icono = () => { const p = !v.paused; play.innerHTML = p ? IC_PAUSA : IC_PLAY; const x = tt(p ? "pistaPausa" : "pistaPlay"); play.title = x; play.setAttribute("aria-label", x); f.classList.toggle("sonando", p); };
  // el MP4, una sola vez y sólo cuando hace falta
  let carga = null;
  const cargar = () => (carga = carga || invoke("leer_video", { path: String(m.ruta || "") }).then((b) => {
    v.src = URL.createObjectURL(new Blob([b], { type: /\.webm$/i.test(String(m.ruta)) ? "video/webm" : "video/mp4" }));
  }).catch((e) => { f.classList.add("err"); t.textContent = "✗"; t.title = String(e); throw e; }));
  const alternar = () => {
    if (!v.paused) { v.pause(); return; }
    cargar().then(() => v.play()).catch((e) => toast("✗ " + (e.message || e), 6000));
  };
  play.onclick = (e) => { e.stopPropagation(); alternar(); };
  v.onclick = alternar;
  v.onplay = v.onpause = icono;
  v.ontimeupdate = v.onloadedmetadata = pinta;
  v.onerror = () => { f.classList.add("err"); t.textContent = "✗"; t.title = `MediaError ${v.error && v.error.code}`; };
  const ir = (x) => { const d = dur(); if (d) { v.currentTime = Math.max(0, Math.min(d, x)); pinta(); } };
  const aRatón = (e) => { const r = barra.getBoundingClientRect(); cargar().then(() => ir(dur() * (e.clientX - r.left) / r.width), () => {}); };
  barra.onpointerdown = (e) => { e.stopPropagation(); barra.setPointerCapture(e.pointerId); aRatón(e); barra.onpointermove = aRatón; };
  barra.onpointerup = () => { barra.onpointermove = null; };
  marco.onkeydown = (e) => {
    const k = { ArrowRight: 1, ArrowLeft: -1 }[e.key];
    if (k) { e.preventDefault(); cargar().then(() => ir(v.currentTime + k), () => {}); }
    else if (e.key === " " || e.key === "Enter") { if (e.target === play) return; e.preventDefault(); alternar(); }
  };
  // el primer cuadro aparece al entrar en pantalla (sin reproducir)
  if ("IntersectionObserver" in window) {
    const io = new IntersectionObserver((es) => { if (es.some((x) => x.isIntersecting)) { io.disconnect(); cargar().catch(() => {}); } });
    io.observe(marco);
  }
  icono(); pinta();
  f.appendChild(marco);
  const [ib, ip] = infoTarjeta(m);
  if (ib) { const pie = el("div", "gen-pie", ""); pie.appendChild(ib); f.appendChild(pie); }
  if (ip) f.appendChild(ip);
  stick(() => log.appendChild(f));
}
function addVista(m) {
  if (m.tipo === "plan") return addPlan(m); // F7 · plan-vista.js
  if (m.tipo === "cuaderno") return addCuaderno(m); // F4 · cuaderno-vista.js
  if (m.tipo === "video" && m.ruta) return addVideo(m);
  if (m.tipo === "imagen" && m.imagen) return addImagen(m);
  if (m.tipo === "audio" && m.audio) return addPista(m);
  clearEmpty();
  const card = el("div", "vista", "");
  const doc = docVista(m, m.html);
  card.innerHTML = `<div class="vista-bar"><b></b><span class="vista-tam" role="group"></span><button class="btn vista-senalar" type="button" aria-pressed="false" hidden></button><button class="btn vista-var" type="button" hidden></button><button class="btn vista-abrir" type="button"></button></div><div class="vista-variantes" role="group" hidden></div><div class="vista-marco"><iframe sandbox="allow-scripts" referrerpolicy="no-referrer" loading="lazy"></iframe></div>`;
  card.querySelector("b").textContent = (m.tipo === "svg" ? "◇ " : m.tipo === "audio" ? "♪ " : "▣ ") + (m.titulo || "");
  const f = card.querySelector("iframe");
  f.title = m.titulo || "vista";
  f.srcdoc = doc;
  // M7: la música es un reproductor: marco bajo y sin tamaños de pantalla
  if (m.tipo === "audio") f.style.height = "96px";
  const tam = card.querySelector(".vista-tam");
  if (m.tipo !== "svg" && m.tipo !== "audio") {
    for (const [k, w] of [["vistaMovil", 390], ["vistaTableta", 768], ["vistaEscritorio", 0]]) {
      const b = el("button", "btn", tt(k));
      b.type = "button";
      b.onclick = () => { f.style.width = w ? w + "px" : "100%"; tam.querySelectorAll("button").forEach((x) => x.classList.toggle("on", x === b)); };
      tam.appendChild(b);
    }
    tam.lastChild.classList.add("on");
  }
  const abrir = card.querySelector(".vista-abrir");
  abrir.textContent = tt("vistaAbrir");
  abrir.onclick = () => invoke("open_vista", { path: String(m.ruta || "") }).catch((e) => toast("✗ " + e, 5000));
  // la imagen generada: su archivo y su licencia en ⓘ, no en la conversación
  const [ib, ip] = infoTarjeta(m);
  if (ib) card.querySelector(".vista-bar").insertBefore(ib, abrir);
  if (ip) card.appendChild(ip);
  if (m.tipo !== "svg" && /index\.html$/.test(String(m.ruta || ""))) {
    // D6: señalar un elemento → el pedido sale con su lugar en la ficha
    const sen = card.querySelector(".vista-senalar");
    sen.hidden = false;
    sen.textContent = tt("vistaSenalar");
    sen.title = tt("vistaSenalarTip");
    const pon = (on) => { sen.setAttribute("aria-pressed", String(on)); sen.classList.toggle("on", on); f.contentWindow && f.contentWindow.postMessage({ gotv: "senalar", on }, "*"); };
    sen.onclick = () => pon(sen.getAttribute("aria-pressed") !== "true");
    addEventListener("message", (e) => {
      if (e.source !== f.contentWindow || !e.data || e.data.gotv !== "senalado") return;
      pon(false);
      const d = e.data, carpeta = String(m.ruta).replace(/\/index\.html$/, "");
      const lugar = d.seccion == null ? tt("vistaFueraSeccion") : `${tt("vistaSeccion")} ${d.seccion} (${d.tipo})`;
      const pre = `[${tt("vistaPagina")} ${carpeta} · ${lugar} · <${d.etiqueta}>${d.texto ? ` «${d.texto}»` : ""}] `;
      input.value = pre + input.value.replace(/^\[[^\]]*\] /, "");
      input.dispatchEvent(new Event("input"));
      input.focus();
      input.setSelectionRange(input.value.length, input.value.length);
    });
    // D6: variantes de dirección SIN modelo (sólo se recompila la ficha)
    const vb = card.querySelector(".vista-var"), fila = card.querySelector(".vista-variantes");
    vb.hidden = false;
    vb.textContent = tt("vistaVariantes");
    vb.onclick = async () => {
      if (!fila.hidden) { fila.hidden = true; return; }
      let vs;
      try { vs = await invoke("vista_variantes", { path: String(m.ruta) }); } catch (e) { toast("✗ " + e, 5000); return; }
      fila.innerHTML = "";
      const usar = el("button", "btn vista-usar", tt("vistaUsar"));
      usar.type = "button"; usar.hidden = true;
      let elegida = null;
      for (const v of vs) {
        const b = el("button", "btn" + (v.actual ? " on" : ""), v.direccion + (v.actual ? " ✓" : ""));
        b.type = "button";
        b.setAttribute("aria-pressed", String(!!v.actual));
        b.onclick = () => {
          f.srcdoc = docVista(m, v.html);
          fila.querySelectorAll("button:not(.vista-usar)").forEach((x) => { x.classList.toggle("on", x === b); x.setAttribute("aria-pressed", String(x === b)); });
          elegida = v; usar.hidden = !!v.actual;
        };
        fila.appendChild(b);
      }
      usar.onclick = async () => {
        if (!elegida) return;
        try {
          const r = await invoke("vista_usar", { path: String(m.ruta), direccion: elegida.direccion });
          vs.forEach((x) => { x.actual = x === elegida; });
          fila.querySelectorAll("button:not(.vista-usar)").forEach((x, i) => { x.textContent = vs[i].direccion + (vs[i].actual ? " ✓" : ""); });
          usar.hidden = true;
          toast(`✓ ${tt("vistaEscrita")} «${r.direccion}»` + (r.respetados ? ` · ${tt("vistaRespetados")}` : ""), 4000);
        } catch (e) { toast("✗ " + e, 5000); }
      };
      fila.appendChild(usar);
      fila.hidden = false;
    };
  }
  stick(() => log.appendChild(card));
}

// ── Ola H: conexiones SSH del proyecto abierto (cada proyecto ve sólo las suyas) ──
async function drawConexiones(pane) {
  let D;
  try { D = await invoke("conexiones_lista"); } catch (e) { pane.innerHTML = setError(e); return; }
  // pulido: cada conexión es una fila (nombre, destino y estado a la izquierda; sus acciones, «Quitar» la
  // última, a la derecha); después «Nueva conexión» en filas y, al final, el registro
  const filas = D.conexiones.map((c) => {
    const meta = [esc(c.destino) + (c.puerto && c.puerto !== 22 ? ":" + c.puerto : ""), c.salto ? `${esc(tt("cxSalto"))}: ${esc(c.salto)}` : "", c.nota ? esc(c.nota) : ""].filter(Boolean).join(" · ");
    const estados = [setEstado(tt(c.fijada ? "cxFijada" : "cxSinHuella"), c.fijada ? "ok" : "warn"), c.solo_lectura ? setEstado(tt("cxSoloLectura")) : "", c.llave ? "" : setEstado(tt("cxSinLlave"), "warn")].join("");
    return `<div class="cx" data-n="${esc(c.nombre)}">${setFila("", esc(c.nombre), `${meta}<span class="estados">${estados}</span>`,
      `<button class="btn" data-a="llave">${esc(tt("cxLlave"))}</button><button class="btn" data-a="probar">${esc(tt("cxProbar"))}</button><button class="btn danger" data-a="quitar">${esc(tt("cxQuitar"))}</button>`, "item")}
      <div class="cx-out" hidden></div></div>`;
  }).join("");
  const saltos = D.conexiones.filter((c) => !c.salto).map((c) => `<option value="${esc(c.nombre)}">${esc(c.nombre)}</option>`).join("");
  const log = (D.registro || []).map((r) => `<li><code>${esc(r.cuando || "")}</code> · <b>${esc(r.conexion || "")}</b> · exit ${esc(String(r.salida))} · ${esc(r.orden || "")}</li>`).join("");
  pane.innerHTML = `<h3>${tt("sConex")}</h3><p>${tt("cxAyuda")}</p><p class="mm">${esc(D.proyecto)}</p>
    <div class="cx-lista">${filas || `<p class="mm">${esc(tt("cxNinguna"))}</p>`}</div>
    <div class="cx-form" data-fila="ssh-nueva"><h4>${tt("cxNueva")}</h4>
      <label class="f"><span>${tt("cxNombre")}</span><input id="cxN" placeholder="prod" autocomplete="off" spellcheck="false"></label>
      <label class="f"><span>${tt("cxDestino")}</span><input id="cxD" placeholder="${esc(tt("cxDestinoEj"))}" autocomplete="off" spellcheck="false"></label>
      <label class="f"><span>${tt("cxNota")}</span><input id="cxNo" placeholder="${esc(tt("cxNotaEj"))}"></label>
      <label class="f"><span>${tt("cxSalto")}</span><select id="cxS"><option value="">—</option>${saltos}</select></label>
      ${setFila("", esc(tt("cxSoloLectura")), "", setInterruptor("cxL", false))}
      <div class="actions"><button class="btn primary" id="cxAdd">${tt("cxAgregar")}</button></div>
    </div>
    ${log ? `<h4>${tt("cxRegistro")}</h4><ul class="cx-log">${log}</ul>` : ""}`;
  $("cxAdd").onclick = async () => {
    try {
      await invoke("conexiones_agregar", { nombre: $("cxN").value, destino: $("cxD").value, salto: $("cxS").value || null, nota: $("cxNo").value || null, soloLectura: $("cxL").checked });
      toast(tt("cxAgregada"));
      drawConexiones(pane);
    } catch (e) { setFallo(e); }
  };
  pane.querySelectorAll(".cx").forEach((fila) => {
    const n = fila.dataset.n, out = fila.querySelector(".cx-out");
    const muestra = (html) => { out.hidden = false; out.innerHTML = html; };
    fila.querySelector('[data-a="llave"]').onclick = async () => {
      try {
        const pub = await invoke("conexiones_llave", { nombre: n });
        muestra(`<p>${esc(tt("cxAutoriza"))}</p><pre class="cx-pub">${esc(pub)}</pre><button class="btn" data-copia>${esc(tt("copy"))}</button>`);
        out.querySelector("[data-copia]").onclick = () => navigator.clipboard.writeText(pub).then(() => toast(tt("copiadoAj")));
      } catch (e) { setFallo(e); }
    };
    fila.querySelector('[data-a="probar"]').onclick = async () => {
      muestra(`<p class="mm">${esc(tt("cxEscaneando"))}</p>`);
      try {
        const r = await invoke("conexiones_escanear", { nombre: n });
        muestra(`<p>${esc(tt("cxCompara").replace("{h}", r.host))}</p><pre>${r.huellas.map(esc).join("\n")}</pre><button class="btn primary" data-fijar>${esc(tt("cxFijar"))}</button>`);
        out.querySelector("[data-fijar]").onclick = async () => {
          muestra(`<p class="mm">${esc(tt("cxProbando"))}</p>`);
          try { const ok = await invoke("conexiones_fijar_y_probar", { nombre: n }); muestra(`<p>${setEstado(ok, "ok")}</p>`); setTimeout(() => drawConexiones(pane), 1500); }
          catch (e) { muestra(setError(e)); }
        };
      } catch (e) { muestra(setError(e)); }
    };
    fila.querySelector('[data-a="quitar"]').onclick = async () => {
      if (!confirm(tt("cxQuitarSeguro").replace("{n}", n))) return;
      try { await invoke("conexiones_quitar", { nombre: n }); drawConexiones(pane); } catch (e) { setFallo(e); }
    };
  });
}

// ── F13.8: «Conectores» (servidores MCP) del proyecto abierto ──
async function drawConectores(pane) {
  let D;
  try { D = await invoke("mcp_lista"); } catch (e) { pane.innerHTML = setError(e); return; }
  // pulido: cada servidor es una fila con su interruptor (encendido/apagado) a la derecha; sus estados, con un
  // punto; probar, entrar y quitar debajo. Luego lo que se puede importar y, al final, «Nuevo servidor».
  const filas = D.servidores.map((s) => {
    const estados = [!s.remoto && !s.caja ? setEstado(tt("mcFueraCaja"), "warn") : "", s.sesion ? setEstado(tt("mcSesion"), "ok") : ""].join("");
    return `<div class="cx" data-n="${esc(s.nombre)}">${setFila("", esc(s.nombre), `${esc(s.tipo)} · ${esc(s.ambito)} · <code>${esc(s.origen)}</code>${estados ? `<span class="estados">${estados}</span>` : ""}`,
      `<input type="checkbox" class="interruptor" data-a="activar" aria-label="${esc(tt(s.activo ? "mcApagar" : "mcEncender"))}"${s.activo ? " checked" : ""}>`, "item")}
      <div class="cx-acc"><button class="btn" data-a="probar">${esc(tt("mcProbar"))}</button>
        ${s.remoto ? `<button class="btn" data-a="${s.sesion ? "salir" : "entrar"}">${esc(tt(s.sesion ? "mcSalir" : "mcEntrar"))}</button>` : ""}
        ${s.registro.length ? `<details><summary>${esc(tt("mcRegistro"))}</summary><pre class="cx-pub">${esc(s.registro.join("\n"))}</pre></details>` : ""}
        ${s.ambito === "Proyecto" ? "" : `<button class="btn danger" data-a="quitar">${esc(tt("mcQuitar"))}</button>`}</div>
      <div class="cx-out" hidden></div></div>`;
  }).join("");
  const avisos = (D.avisos || []).map((a) => `<p class="mm">${setEstado(a, "warn")}</p>`).join("");
  const imp = (D.importables || []).map((i, k) => setFila("", `<b>${esc(i.nombre)}</b>`, esc(i.origen) + (i.secretos ? ` · ${esc(tt("mcSecretos").replace("{n}", i.secretos))}` : ""), `<input type="checkbox" class="interruptor" data-imp="${k}">`)).join("");
  pane.innerHTML = `<h3>${tt("sConectores")}</h3><p>${tt("mcAyuda")}</p><p class="mm">${esc(D.proyecto)}</p>${avisos}
    <div class="cx-lista">${filas || `<p class="mm">${esc(tt("mcNinguno"))}</p>`}</div>
    ${imp ? `<h4>${tt("mcImportables")}</h4>${imp}<div class="actions"><button class="btn" id="mcImp">${tt("mcImportar")}</button></div>` : ""}
    <div class="cx-form" data-fila="mcp-nuevo"><h4>${tt("mcNuevo")}</h4>
      <label class="f"><span>${tt("mcNombre")}</span><input id="mcN" placeholder="github" autocomplete="off" spellcheck="false"></label>
      <label class="f"><span>${tt("mcDestino")}</span><input id="mcD" placeholder="npx -y @modelcontextprotocol/server-everything" autocomplete="off" spellcheck="false"></label>
      <label class="f"><span>${tt("mcDominios")}</span><input id="mcDom" placeholder="api.github.com" autocomplete="off" spellcheck="false"></label>
      <label class="f"><span>${tt("mcAmbito")}</span><select id="mcA"><option value="usuario">${tt("mcUsuario")}</option><option value="local">${tt("mcLocal")}</option></select></label>
      <div class="actions"><button class="btn primary" id="mcAdd">${tt("mcAgregar")}</button></div>
    </div>`;
  $("mcAdd").onclick = async () => {
    try {
      const dominios = $("mcDom").value.split(",").map((s) => s.trim()).filter(Boolean);
      await invoke("mcp_agregar", { nombre: $("mcN").value, destino: $("mcD").value, ambito: $("mcA").value, dominios });
      toast(tt("mcAgregado"));
      drawConectores(pane);
    } catch (e) { setFallo(e); }
  };
  const bImp = $("mcImp");
  if (bImp) bImp.onclick = async () => {
    const nombres = [...pane.querySelectorAll("[data-imp]")].filter((c) => c.checked).map((c) => D.importables[+c.dataset.imp].nombre);
    if (!nombres.length) return;
    try { await invoke("mcp_importar", { nombres }); toast(tt("mcImportado")); drawConectores(pane); } catch (e) { setFallo(e); }
  };
  pane.querySelectorAll(".cx").forEach((fila) => {
    const n = fila.dataset.n, out = fila.querySelector(".cx-out"), s = D.servidores.find((x) => x.nombre === n);
    const muestra = (html) => { out.hidden = false; out.innerHTML = html; };
    const on = (a, f) => { const b = fila.querySelector(`[data-a="${a}"]`); if (b) (b.type === "checkbox" ? (b.onchange = f) : (b.onclick = f)); };
    on("probar", async () => {
      muestra(`<p class="mm">${esc(tt("mcProbando"))}</p>`);
      try {
        const r = await invoke("mcp_probar", { nombre: n });
        muestra(`<p>${r.notas.map(esc).join("<br>")}</p><p class="mm">${esc(tt("mcTokens").replace("{n}", r.tokens_si_se_cargan))}</p><pre class="cx-pub">${esc(r.herramientas.join("\n"))}</pre>`);
      } catch (e) { muestra(setError(e)); }
    });
    on("activar", async () => { try { await invoke("mcp_activar", { nombre: n, activo: !s.activo }); drawConectores(pane); } catch (e) { setFallo(e); drawConectores(pane); } });
    on("entrar", async () => { muestra(`<p class="mm">${esc(tt("mcEntrando"))}</p>`); try { await invoke("mcp_entrar", { nombre: n }); drawConectores(pane); } catch (e) { muestra(setError(e)); } });
    on("salir", async () => { await invoke("mcp_salir", { nombre: n }); drawConectores(pane); });
    on("quitar", async () => {
      if (!confirm(tt("mcQuitarSeguro").replace("{n}", n))) return;
      try { await invoke("mcp_quitar", { nombre: n }); drawConectores(pane); } catch (e) { setFallo(e); }
    });
  });
}

// ── F1 paso 8: bases de datos del proyecto abierto (registro .gotv/bases.gotv; claves en el Llavero) ──
async function drawBases(pane) {
  let D;
  try { D = await invoke("bases_lista"); } catch (e) { pane.innerHTML = setError(e); return; }
  const red = (m) => m === "postgres" || m === "mysql";
  // pulido: cada base es una fila (motor, dónde y estados a la izquierda; acciones a la derecha, «Quitar» la
  // última); luego las halladas en el proyecto y, al final, «Nueva base» en filas
  const filas = D.bases.map((b) => {
    const meta = [esc(b.motor), esc(b.donde), b.tunel ? `${esc(tt("bdTunel"))}: ${esc(b.tunel)}` : "", b.mascara.length ? `${esc(tt("bdOcultas"))}: ${esc(b.mascara.join(", "))}` : ""].filter(Boolean).join(" · ");
    const estados = [b.produccion ? setEstado(tt("bdProdMarca"), "bad") : "", red(b.motor) ? (b.clave ? setEstado(tt("bdConClave"), "ok") : setEstado(tt("bdSinClave"), "warn")) : ""].join("");
    return `<div class="cx" data-n="${esc(b.nombre)}">${setFila("", esc(b.nombre), meta + (estados ? `<span class="estados">${estados}</span>` : ""),
      `<button class="btn" data-a="probar">${esc(tt("bdProbar"))}</button>${red(b.motor) ? `<button class="btn" data-a="clave">${esc(tt("bdClave"))}</button>` : ""}
        <button class="btn" data-a="prod">${esc(tt(b.produccion ? "bdDesmarcarProd" : "bdMarcarProd"))}</button><button class="btn danger" data-a="quitar">${esc(tt("bdQuitar"))}</button>`, "item")}
      <div class="cx-out" hidden></div></div>`;
  }).join("");
  const halladas = (D.detectadas || []).filter((x) => !D.bases.some((b) => b.donde === x.donde)).map((x, i) => setFila("", esc(x.motor), esc(x.donde), `<button class="btn" data-reg="${i}">${esc(tt("bdRegistrar"))}</button>`)).join("");
  const libres = (D.detectadas || []).filter((x) => !D.bases.some((b) => b.donde === x.donde));
  const tuneles = (D.ssh || []).map((n) => `<option value="${esc(n)}">${esc(n)}</option>`).join("");
  pane.innerHTML = `<h3>${tt("sBases")}</h3><p>${tt("bdAyuda")}</p><p class="mm">${esc(D.proyecto)}</p>
    <div class="cx-lista">${filas || `<p class="mm">${esc(tt("bdNinguna"))}</p>`}</div>
    ${halladas ? `<h4>${tt("bdDetectadas")}</h4>${halladas}` : ""}
    <div class="cx-form" data-fila="bd-nueva"><h4>${tt("bdNueva")}</h4>
      <label class="f"><span>${tt("bdNombre")}</span><input id="bdN" placeholder="${esc(tt("bdNombreEj"))}" autocomplete="off" spellcheck="false"></label>
      <label class="f"><span>${tt("bdDestino")}</span><input id="bdD" placeholder="${esc(tt("bdDestinoEj"))}" autocomplete="off" spellcheck="false"></label>
      <label class="f"><span>${tt("bdTunel")}</span><select id="bdT"><option value="">—</option>${tuneles}</select></label>
      <label class="f"><span>${tt("bdMascara")}</span><input id="bdM" placeholder="cedula, telefono" autocomplete="off" spellcheck="false"></label>
      ${setFila("", esc(tt("bdProd")), "", setInterruptor("bdP", false))}
      <div class="actions"><button class="btn primary" id="bdAdd">${tt("bdAgregar")}</button></div>
    </div>`;
  pane.querySelectorAll("[data-reg]").forEach((b) => (b.onclick = () => {
    const x = libres[+b.dataset.reg];
    $("bdN").value = x.nombre.replace(/[^A-Za-z0-9_-]+/g, "-").replace(/^-+|-+$/g, "");
    $("bdD").value = x.url || x.donde;
    $("bdD").dataset.secreto = x.secreto || "";
    $("bdN").focus();
  }));
  $("bdAdd").onclick = async () => {
    try {
      const mascara = $("bdM").value.split(",").map((s) => s.trim()).filter(Boolean);
      await invoke("bases_agregar", { nombre: $("bdN").value, destino: $("bdD").value, tunel: $("bdT").value || null, produccion: $("bdP").checked, mascara, secreto: $("bdD").dataset.secreto || null });
      toast(tt("bdAgregada"));
      drawBases(pane);
    } catch (e) { setFallo(e); }
  };
  pane.querySelectorAll(".cx").forEach((fila) => {
    const n = fila.dataset.n, out = fila.querySelector(".cx-out");
    const muestra = (html) => { out.hidden = false; out.innerHTML = html; };
    fila.querySelector('[data-a="probar"]').onclick = async () => {
      muestra(`<p class="mm">${esc(tt("bdProbando"))}</p>`);
      try { muestra(`<pre>${esc(await invoke("bases_probar", { nombre: n }))}</pre>`); } catch (e) { muestra(setError(e)); }
    };
    const clave = fila.querySelector('[data-a="clave"]');
    if (clave) clave.onclick = () => {
      muestra(`<div class="choose-free"><input type="password" autocomplete="off" spellcheck="false" data-v><button class="btn primary" data-g>${esc(tt("bdClaveGuardar"))}</button></div>`);
      const v = out.querySelector("[data-v]");
      v.focus();
      const guarda = async () => {
        if (!v.value) return;
        try { await invoke("bases_secreto", { nombre: n, valor: v.value }); v.value = ""; toast(tt("bdClaveGuardada")); drawBases(pane); } catch (e) { v.value = ""; muestra(setError(e)); }
      };
      out.querySelector("[data-g]").onclick = guarda;
      v.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); guarda(); } });
    };
    fila.querySelector('[data-a="prod"]').onclick = async () => {
      const b = D.bases.find((x) => x.nombre === n);
      try { await invoke("bases_marcar", { nombre: n, produccion: !b.produccion }); drawBases(pane); } catch (e) { setFallo(e); }
    };
    fila.querySelector('[data-a="quitar"]').onclick = async () => {
      if (!confirm(tt("bdQuitarSeguro").replace("{n}", n))) return;
      try { await invoke("bases_quitar", { nombre: n }); drawBases(pane); } catch (e) { setFallo(e); }
    };
  });
}

// ── F23.2: buscadores (fichas legales F23.L; la llave va al Llavero y NUNCA vuelve) ──
const kb = (n) => (n >= 1e9 ? (n / 1e9).toFixed(1) + " GB" : n >= 1e6 ? (n / 1e6).toFixed(1) + " MB" : Math.ceil(n / 1e3) + " KB");
async function drawBuscadores(pane) {
  let D;
  try { D = await invoke("buscadores_lista"); } catch (e) { pane.innerHTML = setError(e); return; }
  // pulido: arriba cómo se busca (filas que se guardan al cambiar); luego cada servicio como una fila con su
  // interruptor y sus estados con punto, y sus acciones debajo; el índice propio; y AL FINAL, borrar.
  const uso = (u, t) => tt("bsUso").replace("{u}", u).replace("{t}", t == null ? "∞" : t);
  const fila = (s) => {
    const estados = [
      s.necesita_llave ? (s.tiene_llave ? setEstado(tt("bsConLlave"), "ok") : setEstado(tt("bsSinLlave"), "warn")) : setEstado(tt("bsSinLlaveNecesaria")),
      s.en_orden ? setEstado(tt("bsEnOrden"), "ok") : s.en_respaldo ? setEstado(tt("bsRespaldo")) : "",
      s.aviso ? setEstado(s.aviso, "warn") : "",
      ...s.problemas.map((p) => setEstado(p, "warn")),
    ].join("");
    return `<div class="cx" data-n="${esc(s.id)}">${setFila("", esc(s.nombre),
      `${esc(s.que_guarda)}<br>${esc(s.precio)} (${esc(s.precio_fecha)}) · ${esc(uso(s.usado_mes, s.tope_mes))}<span class="estados">${estados}</span>`,
      `<input type="checkbox" class="interruptor" data-a="activo" aria-label="${esc(s.nombre)}"${s.activo ? " checked" : ""}${s.activable ? "" : " disabled"}>`, "item")}
      <div class="cx-acc">
        ${s.necesita_llave ? `<input type="password" class="bs-llave" placeholder="${esc(tt("bsLlave"))}" autocomplete="off"><button class="btn" data-a="llave">${esc(tt("save"))}</button>${s.tiene_llave ? `<button class="btn" data-a="quitar">${esc(tt("bsQuitarLlave"))}</button>` : ""}<a href="#" data-a="donde">${esc(tt("bsDonde"))}</a><span class="cx-sep"></span>` : ""}
        <input type="number" min="0" class="bs-tope" value="${s.tope_mes == null ? "" : s.tope_mes}" placeholder="${esc(tt("bsTope"))}"><button class="btn" data-a="tope">${esc(tt("bsPonerTope"))}</button><span class="cx-sep"></span>
        <button class="btn" data-a="probar">${esc(tt("bdProbar"))}</button><button class="btn" data-a="ficha">${esc(tt("bsFicha"))}</button><button class="btn" data-a="revisar">${esc(tt("bsRevisar"))}</button>
      </div><div class="bs-out"></div></div>`;
  };
  const ix = D.indice || { paginas: 0, ocupa: 0, semillas: [] };
  pane.innerHTML = `<h3>${tt("sBuscadores")}</h3>
    <p>${esc(tt("bsIntro"))}</p>
    ${setFila("bs-paralelo", esc(tt("bsParalelo")), "", setInterruptor("bsParalelo", D.paralelo))}
    ${setFila("bs-local", esc(tt("bsSoloLocal")), "", setInterruptor("bsLocal", D.solo_local))}
    ${setFila("bs-contacto", esc(tt("bsContacto")), esc(tt("flBsContactoNota")), `<input id="bsContacto" value="${esc(D.contacto || "")}" placeholder="https://…">`)}
    <h4>${esc(tt("bsServicios"))}</h4>
    ${D.servicios.map(fila).join("")}
    <h4>${esc(tt("bsIndice"))}</h4>
    ${setFila("bs-indice", esc(tt("bsSemillas")), esc(tt("bsIndiceEstado").replace("{p}", ix.paginas).replace("{o}", kb(ix.ocupa))),
      `<textarea id="bsSemillas" rows="3">${esc((ix.semillas || []).join("\n"))}</textarea><button class="btn" id="bsRastrear">${esc(tt("bsRastrear"))}</button>`, "bloque")}
    ${setFila("bs-borrar", esc(tt("flBsBorrar")), esc(tt("flBsBorrarNota").replace("{c}", kb(D.ocupa.cache)).replace("{i}", kb(D.ocupa.indice))), `<button class="btn danger" id="bsBorrar">${esc(tt("flBsBorrarBtn"))}</button>`, "peligro")}`;
  const orden = () => D.servicios.filter((s) => s.en_orden).map((s) => s.id);
  const respaldo = () => D.servicios.filter((s) => s.en_respaldo).map((s) => s.id);
  // cómo se busca: se guarda al cambiar (antes, con un botón «Guardar» aparte)
  const guardarOrden = async () => {
    try { await invoke("buscadores_orden", { orden: orden(), respaldo: respaldo(), paralelo: $("bsParalelo").checked, soloLocal: $("bsLocal").checked, contacto: $("bsContacto").value }); toast(tt("saved")); } catch (e) { setFallo(e); }
  };
  $("bsParalelo").onchange = guardarOrden;
  $("bsLocal").onchange = guardarOrden;
  $("bsContacto").onchange = guardarOrden;
  $("bsRastrear").onclick = async () => {
    try { await invoke("indice_semillas", { semillas: $("bsSemillas").value.split(/\s+/).filter(Boolean) }); toast(tt("bsRastreando"), 4000); } catch (e) { setFallo(e); }
  };
  $("bsBorrar").onclick = async () => {
    if (!confirm(tt("bsBorrarSeguro"))) return;
    try { const r = await invoke("busquedas_borrar"); toast(tt("flBsBorrado").replace("{n}", kb(r.cache + r.indice))); drawBuscadores(pane); } catch (e) { setFallo(e); }
  };
  pane.querySelectorAll(".cx").forEach((el) => {
    const id = el.dataset.n, s = D.servicios.find((x) => x.id === id), out = el.querySelector(".bs-out");
    const a = (k) => el.querySelector(`[data-a="${k}"]`);
    a("activo").onchange = async (e) => {
      try { await invoke("buscadores_activar", { id, activo: e.target.checked }); drawBuscadores(pane); } catch (err) { e.target.checked = !e.target.checked; setFallo(err); }
    };
    if (a("llave")) a("llave").onclick = async () => {
      const i = el.querySelector(".bs-llave");
      try { await invoke("buscadores_llave", { id, llave: i.value }); i.value = ""; toast(tt("bdClaveGuardada")); drawBuscadores(pane); } catch (e) { i.value = ""; setFallo(e); }
    };
    if (a("quitar")) a("quitar").onclick = async () => { try { await invoke("buscadores_llave", { id, llave: null }); drawBuscadores(pane); } catch (e) { setFallo(e); } };
    if (a("donde")) a("donde").onclick = (e) => { e.preventDefault(); invoke("open_link", { url: s.donde_llave }); };
    a("tope").onclick = async () => {
      const v = el.querySelector(".bs-tope").value.trim();
      try { await invoke("buscadores_tope", { id, tope: v === "" ? null : Number(v) }); drawBuscadores(pane); } catch (e) { setFallo(e); }
    };
    a("probar").onclick = async () => {
      out.innerHTML = `<p class="cx-nota">…</p>`;
      try {
        const r = await invoke("buscadores_probar", { id });
        out.innerHTML = `<div class="cx-nota">${r.ok ? setEstado(tt("bsProbadoOk"), "ok") : setEstado(r.detalle, "bad")} · ${esc(uso(r.usado_mes, r.tope_mes))}</div>` + r.resultados.map((x) => `<div class="cx-nota">${esc(x.titulo)} — ${esc(x.url)}</div>`).join("");
      } catch (e) { out.innerHTML = setError(e); }
    };
    a("ficha").onclick = async () => {
      try {
        const f = await invoke("buscadores_ficha", { id });
        out.innerHTML = `<div class="cx-nota"><b>${esc(f.publica)}</b> · ${esc(tt("bsRevisada"))} ${esc(f.condiciones_revisadas)} (${esc(f.revisada_por)})<br>
          ${esc(tt("bsEntrenar"))}: ${esc(f.prohibe_entrenar_ia)} · ${esc(tt("bsDominio"))}: ${esc(f.dominio_api)}${f.atribucion ? " · " + esc(f.atribucion) : ""}</div>
          <pre class="cx-nota" style="white-space:pre-wrap">${esc(f.notas_legales)}</pre>
          <div class="cx-nota">${f.fuentes.map((u) => `<a href="#" data-u="${esc(u)}">${esc(u)}</a>`).join("<br>")}</div>`;
        out.querySelectorAll("a[data-u]").forEach((l) => (l.onclick = (e) => { e.preventDefault(); invoke("open_link", { url: l.dataset.u }); }));
      } catch (e) { out.innerHTML = setError(e); }
    };
    a("revisar").onclick = async () => {
      try { const r = await invoke("buscadores_revisar", { id }); toast(tt(r.igual ? "bsIgual" : "bsCambio"), 6000); drawBuscadores(pane); } catch (e) { setFallo(e); }
    };
  });
}

// ── Control Mac: check redondo de la orden (sólo si Seguridad lo permite) ──
function paintMac() {
  const b = $("macBtn");
  b.classList.toggle("on", !!(S.mac && S.desktopOk));
  b.classList.toggle("locked", !S.desktopOk);
  b.setAttribute("aria-checked", String(!!(S.mac && S.desktopOk)));
  b.title = S.desktopOk ? tt(S.mac ? "macOnTip" : "macOffTip") : tt("macNeedsSecurity");
  avisarEstadoChat();
}
// el check «Control Mac» y el perfil también valen para lo que se DICE en manos libres: el núcleo
// los recibe al cambiar (antes la voz mandaba siempre Control Mac apagado y sin perfil)
function avisarEstadoChat() {
  invoke("chat_estado", { mac: !!(S.mac && S.desktopOk), perfil: S.perfil || null, pro: proDeOrden() }).catch(() => {});
}
$("macBtn").onclick = () => {
  if (!S.desktopOk) { toast(tt("macNeedsSecurity"), 5000); openSettings("seguridad/mac"); return; }
  S.mac = !S.mac; paintMac(); guardarChat();
  if (S.mac) toast("🖥 " + tt("macOnTip"), 3500);
};
// aviso propio (las ventanas del sistema no sirven en la app)
function confirmBox(title, body, yes) {
  return new Promise((res) => {
    $("cfTitle").textContent = title; $("cfBody").textContent = body; $("cfYes").textContent = yes;
    const m = $("confirmModal"); m.classList.add("show");
    const done = (v) => { m.classList.remove("show"); $("cfYes").onclick = $("cfNo").onclick = null; res(v); };
    $("cfYes").onclick = () => done(true); $("cfNo").onclick = () => done(false);
    m.querySelector("[data-close]").onclick = () => done(false);
  });
}
input.addEventListener("input", () => { fit(); refreshSend(); updateSlash(); });

// ── Autocompletado de «/» ─────────────────────────────────────────────────
async function loadSlash() { try { S.slash = await invoke("slash_items"); } catch { S.slash = []; } }
function hideSlash() { $("slash").style.display = "none"; S.slashList = []; }
function updateSlash() {
  const v = input.value;
  const m = /^\/([\w.-]*)$/.exec(v);
  if (!m) return hideSlash();
  const q = m[1].toLowerCase();
  const list = S.slash
    .map((it) => ({ it, score: it.name.toLowerCase().startsWith(q) ? 0 : it.name.toLowerCase().includes(q) ? 1 : it.desc.toLowerCase().includes(q) && q.length > 1 ? 2 : 9 }))
    .filter((x) => x.score < 9)
    .sort((a, b) => a.score - b.score || (a.it.kind === b.it.kind ? a.it.name.localeCompare(b.it.name) : a.it.kind === "cmd" ? -1 : 1))
    .slice(0, 40)
    .map((x) => x.it);
  S.slashList = list;
  S.slashSel = 0;
  const box = $("slash");
  if (!list.length) return hideSlash();
  box.innerHTML = list
    .map((it, i) => {
      const name = q ? esc(it.name).replace(new RegExp("(" + q.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + ")", "i"), "<mark>$1</mark>") : esc(it.name);
      return `<div class="it${i === 0 ? " sel" : ""}" data-i="${i}"><b>/${name}</b><span>${esc(it.desc)}</span><i>${it.kind === "skill" ? "skill" : "cmd"}</i></div>`;
    })
    .join("");
  box.style.display = "block";
  box.querySelectorAll(".it").forEach((d) => (d.onclick = () => pickSlash(+d.dataset.i)));
}
function moveSlash(d) {
  const items = $("slash").querySelectorAll(".it");
  if (!items.length) return;
  items[S.slashSel].classList.remove("sel");
  S.slashSel = (S.slashSel + d + items.length) % items.length;
  items[S.slashSel].classList.add("sel");
  items[S.slashSel].scrollIntoView({ block: "nearest" });
}
function pickSlash(i) {
  const it = S.slashList[i];
  if (!it) return;
  input.value = "/" + it.name + " ";
  hideSlash();
  fit(); refreshSend();
  input.focus();
}
input.addEventListener("keydown", (e) => {
  const open = $("slash").style.display === "block";
  if (open && (e.key === "ArrowDown" || e.key === "ArrowUp")) { e.preventDefault(); moveSlash(e.key === "ArrowDown" ? 1 : -1); return; }
  if (open && (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey))) { e.preventDefault(); pickSlash(S.slashSel); return; }
  if (open && e.key === "Escape") { e.preventDefault(); hideSlash(); return; }
  if (e.key === "Escape" && S.rb) { e.preventDefault(); e.stopPropagation(); cancelarRb(); return; }
  if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); submit(); }
});

// ── Permisos ──────────────────────────────────────────────────────────────
// El permiso: el motor manda la pregunta ya partida (`partes`: riesgo, marca, herramienta, detalle, nota,
// siempre, lote); aquí sólo se pinta. Sin `partes` (un motor viejo), el texto tal cual.
function showAsk(id, desc, partes) {
  S.askId = id;
  const p = partes || { riesgo: "medio", detalle: desc };
  const card = document.querySelector("#askModal .card");
  card.dataset.riesgo = p.riesgo || "medio";
  $("askRiesgo").textContent = tt("permRiesgo_" + (p.riesgo || "medio"));
  const titulo = (h) => (h && I18N["permT_" + h] ? tt("permT_" + h) : null);
  const cuerpo = $("askCuerpo");
  cuerpo.innerHTML = "";
  if (Array.isArray(p.lote)) {
    // varias operaciones: una lista simple (qué hace · sobre qué), sin colores por fila
    $("askTitle").textContent = p.titulo || tt("permLote");
    const ul = document.createElement("ul");
    ul.className = "perm-lista";
    p.lote.forEach((x) => {
      const li = document.createElement("li");
      li.innerHTML = `<span class="h">${esc(titulo(x.herramienta) || x.herramienta || x.marca || "")}</span><code>${esc(x.detalle || x.texto || "")}</code>`;
      ul.appendChild(li);
    });
    cuerpo.appendChild(ul);
    $("askAyuda").textContent = tt("permSiempreLote");
  } else {
    $("askTitle").textContent = titulo(p.herramienta) || p.herramienta || tt("permOtra");
    const pre = document.createElement("pre");
    pre.className = "perm-bloque";
    if (p.herramienta === "bash") { const pr = document.createElement("span"); pr.className = "pr"; pr.textContent = "$"; pre.appendChild(pr); }
    pre.appendChild(document.createTextNode(p.detalle || desc));
    cuerpo.appendChild(pre);
    const ay = $("askAyuda");
    if (p.siempre) { ay.innerHTML = esc(tt("permSiempreProg")).replace("{p}", `<code>${esc(p.siempre)}</code>`); } else { ay.textContent = tt("permSiempreExacta"); }
  }
  $("askMarca").hidden = !p.marca;
  $("askMarca").textContent = p.marca || "";
  $("askNota").hidden = !p.nota;
  $("askNota").textContent = p.nota || "";
  $("askModal").classList.add("show");
  setTimeout(() => { const b = document.querySelector('#askModal [data-a="yes"]'); if (b) b.focus(); }, 30);
}
function answer(a) {
  if (S.askId === null) return;
  invoke("answer", { id: S.askId, ans: a });
  addLine("   ➜ " + tt(a));
  S.askId = null;
  $("askModal").classList.remove("show");
  input.focus();
}
document.querySelectorAll("#askModal [data-a]").forEach((b) => (b.onclick = () => answer(b.dataset.a)));

// C7: el agente pregunta con opciones (ask_user, aprobar un plan)
function showChoose(id, question, options) {
  if (planContesta(id, question, options)) return; // F7: la tarjeta del plan contesta
  S.chooseId = id;
  $("chTitle").textContent = "❔ " + question;
  $("chOpts").innerHTML = (options || []).map((o, i) => `<button class="btn choose-opt" data-i="${i}">${esc(o)}</button>`).join("");
  $("chOpts").querySelectorAll(".choose-opt").forEach((b) => (b.onclick = () => chooseAnswer(options[+b.dataset.i])));
  $("chFree").value = "";
  $("chooseModal").classList.add("show");
  setTimeout(() => { const f = $("chOpts").querySelector("button"); if (f) f.focus(); }, 30);
}
function chooseAnswer(v) {
  if (S.chooseId == null) return;
  invoke("answer_choice", { id: S.chooseId, value: v });
  addLine("   ➜ " + (v || tt("chSkip")));
  S.chooseId = null;
  $("chooseModal").classList.remove("show");
  input.focus();
}
$("chSend").onclick = () => { const v = $("chFree").value.trim(); if (v) chooseAnswer(v); };
$("chFree").addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); $("chSend").onclick(); } });
$("chSkip").onclick = () => chooseAnswer(null);
// F1: la clave de una base, TAPADA: va por answer_choice al Llavero; nunca se escribe en el chat
function showSecreto(id, etiqueta) {
  S.secretoId = id;
  $("seTitle").textContent = "🔑 " + etiqueta;
  $("seValor").value = "";
  $("secretoModal").classList.add("show");
  setTimeout(() => $("seValor").focus(), 30);
}
function secretoAnswer(v) {
  if (S.secretoId == null) return;
  invoke("answer_choice", { id: S.secretoId, value: v });
  addLine("   ➜ " + tt(v ? "seGuardada" : "cancel"));
  S.secretoId = null;
  $("seValor").value = "";
  $("secretoModal").classList.remove("show");
  input.focus();
}
$("seSend").onclick = () => { const v = $("seValor").value; if (v.trim()) secretoAnswer(v); };
$("seValor").addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); $("seSend").onclick(); } });
$("seSkip").onclick = () => secretoAnswer(null);
// cerrar ventanas: ✕, clic fuera de la tarjeta o Esc (la de permiso exige respuesta)
function closeModal(m) { m.classList.remove("show"); input.focus(); }
document.querySelectorAll("[data-close]").forEach((b) => (b.onclick = () => closeModal(b.closest(".modal"))));
document.querySelectorAll(".modal").forEach((m) => {
  if (m.id === "askModal" || m.id === "chooseModal" || m.id === "secretoModal") return;
  m.addEventListener("mousedown", (e) => { if (e.target === m) closeModal(m); });
});
document.addEventListener("keydown", (e) => {
  if (S.askId !== null) {
    const k = { s: "yes", y: "yes", n: "no", a: "always", x: "never", Escape: "no" }[e.key];
    if (k) { e.preventDefault(); answer(k); }
    return;
  }
  if (S.secretoId != null && e.key === "Escape") { e.preventDefault(); secretoAnswer(null); return; }
  if (e.key === "Escape") {
    document.querySelectorAll(".modal.show").forEach((m) => m.classList.remove("show"));
    if (S.busy) invoke("stop_turn");
  }
});

// ── Tareas ────────────────────────────────────────────────────────────────
function renderTasks() { drawTasks($("taskList"), S.tasks, S.procs); }
setInterval(() => { if (S.tasks.some((t) => t.state === "running") || S.procs.some((p) => p.alive)) renderTasks(); }, 1000);
$("tasksBtn").onclick = () => $("tasksPanel").classList.toggle("open");
$("closeTasks").onclick = () => $("tasksPanel").classList.remove("open");
$("popTasks").onclick = async () => { await invoke("tasks_window"); $("tasksPanel").classList.remove("open"); };

// ── Modo ──────────────────────────────────────────────────────────────────
$("mode").onchange = (e) => invoke("apply_settings", { s: { mode: e.target.value } });

// ── Configuración ─────────────────────────────────────────────────────────
let SET = null;
// Pulido (2026-11-18): cada ajuste de cada página es una FILA, como en el rediseño: nombre y explicación a la
// izquierda, el control a la derecha. `id` es el de `FILAS` en ventana.rs (el buscador lleva a ella y la
// resalta); "" para las filas de una lista. `nombre`, `nota` y `control` van ya escapados. `clase`: "item"
// (una fila de lista), "bloque" (el control debajo, a todo lo ancho) o "peligro" (lo que no se deshace).
function setFila(id, nombre, nota, control, clase) {
  return `<div class="fila${clase ? " " + clase : ""}"${id ? ` data-fila="${esc(id)}"` : ""}><div class="fila-txt"><div class="fila-nom">${nombre}</div>${nota ? `<div class="fila-nota">${nota}</div>` : ""}</div>${control ? `<div class="fila-ctl">${control}</div>` : ""}</div>`;
}
// un estado: texto sobrio con un punto pequeño de color (ok · warn · bad; sin tono, gris), como el riesgo del
// permiso. Lo que llega del núcleo con una marca delante (la etiqueta «⚠ con condiciones» de una licencia) la pierde:
// el punto ya la dice
const MARCAS_DELANTE = new RegExp("^[" + String.fromCharCode(0x2713, 0x2714, 0x2717, 0x2715, 0x26a0, 0xfe0f) + "\\s]+", "u");
const sinMarca = (t) => String(t).replace(MARCAS_DELANTE, "");
function setEstado(texto, tono) { return `<span class="estado"${tono ? ` data-tono="${tono}"` : ""}>${esc(sinMarca(texto))}</span>`; }
function setInterruptor(id, on, extra) { return `<input type="checkbox" class="interruptor" id="${id}"${on ? " checked" : ""}${extra || ""}>`; }
function setError(e) { return `<p class="set-error">${esc(String(e))}</p>`; }
function setFallo(e) { toast(String(e), 6000); }
const nf = (n) => Number(n).toLocaleString(LANG === "en" ? "en-US" : "es-CO");
// 40 960 → «41k», 131 072 → «131k», 512 → «512»
const kRound = (n) => (n >= 1e6 ? +(n / 1e6).toFixed(1) + "M" : n >= 1000 ? Math.round(n / 1000) + "k" : String(n));
// A3 · proveedores de IA por API: la llave va al Llavero de macOS y nunca vuelve aquí
function drawProviders(pane) {
  // pulido: cada proveedor es una fila (nombre, tipo, URL y si tiene llave a la izquierda; probar a la derecha)
  // con su llave y lo que se quita debajo; «Añadir proveedor» en filas al final
  const rows = SET.providers.map((p) => {
    const keyless = p.kind === "openai" && p.base_url.startsWith("http://");
    const state = p.has_key ? setEstado(tt("provHasKey"), "ok") : keyless ? setEstado(tt("provNoKeyNeeded")) : setEstado(tt("provNoKey"), "warn");
    return `<div class="prov" data-n="${esc(p.name)}">
      ${setFila("", esc(p.name), `${esc(p.kind === "anthropic" ? "Claude · Messages API" : "OpenAI-compatible")}${p.base_url ? ` · <code>${esc(p.base_url)}</code>` : ""}<span class="estados">${state}</span>`,
        `<span class="pres"></span><button class="btn" data-a="test">${tt("provTest")}</button>`, "item")}
      <div class="cx-acc"><input type="password" class="pkey" placeholder="${esc(tt("provKey"))}" autocomplete="off" spellcheck="false"><button class="btn" data-a="key">${tt("provSaveKey")}</button>${p.has_key ? `<button class="btn" data-a="forget">${tt("provForget")}</button>` : ""}${p.builtin ? "" : `<button class="btn danger" data-a="remove">${tt("provRemove")}</button>`}</div>
    </div>`;
  }).join("");
  const presets = SET.presets.map((x) => `<option value="${esc(x.name)}" data-url="${esc(x.base_url)}">${esc(x.label)}</option>`).join("");
  pane.innerHTML = `<h3>${tt("sProv")}</h3><p>${tt("provHelp")}</p>${rows}
    <div class="cx-form" data-fila="prov-nuevo"><h4>${tt("provAdd")}</h4>
    <label class="f"><span>${tt("provPreset")}</span><select id="pvPreset"><option value="">${tt("provCustom")}</option>${presets}</select></label>
    <label class="f"><span>${tt("provName")}</span><input type="text" id="pvName" placeholder="openrouter"></label>
    <label class="f"><span>${tt("provUrl")}</span><input type="text" id="pvUrl" class="mono" placeholder="https://…/v1"></label>
    ${setFila("", tt("provVision"), "", setInterruptor("pvVision", false))}
    <div class="actions"><button class="btn primary" id="pvAdd">${tt("provAddBtn")}</button></div></div>`;
  $("pvPreset").onchange = (e) => { const o = e.target.selectedOptions[0]; if (o.value) { $("pvName").value = o.value; $("pvUrl").value = o.dataset.url; } };
  const refresh = async () => { SET = await invoke("settings"); drawProviders(pane); };
  pane.querySelectorAll(".prov").forEach((row) => {
    const name = row.dataset.n, res = row.querySelector(".pres");
    row.querySelectorAll("button[data-a]").forEach((b) => (b.onclick = async () => {
      try {
        if (b.dataset.a === "key") {
          const k = row.querySelector(".pkey");
          if (!k.value.trim()) return;
          await invoke("provider_key", { name, key: k.value });
          k.value = ""; // la llave no se queda en la ventana
          toast(tt("provKeySaved"));
          await refresh();
        } else if (b.dataset.a === "test") {
          res.textContent = "…";
          const n = await invoke("provider_test", { name });
          res.innerHTML = setEstado(tt("provTestOk").replace("{n}", n), "ok");
        } else if (b.dataset.a === "forget") {
          await invoke("provider_forget_key", { name }); await refresh();
        } else if (b.dataset.a === "remove") {
          await invoke("provider_remove", { name }); await refresh();
        }
      } catch (e) { res.innerHTML = setEstado(String(e), "bad"); }
    }));
  });
  $("pvAdd").onclick = async () => {
    try {
      await invoke("provider_add", { name: $("pvName").value, kind: "openai", baseUrl: $("pvUrl").value, vision: $("pvVision").checked });
      toast(tt("saved")); await refresh();
    } catch (e) { setFallo(e); }
  };
}
async function openSettings(pane = "general") {
  SET = await invoke("settings");
  // el buscador de la barra empieza vacío cada vez
  const q = $("setBuscar");
  if (q && q.value) { q.value = ""; q.oninput && q.oninput(); }
  $("settingsModal").classList.add("show");
  showPane(pane);
}
$("openSettings").onclick = () => openSettings();
function showPane(p) {
  // F8.0 + diseño: grupos, páginas y piezas los reparte panel.js (lo que Rust decide en `GRUPOS`); también
  // ensancha la ventana para la Biblioteca. Sin estructura todavía, lo de antes (paneBase)
  if (window.gotvAjustes && gotvAjustes.pintar(p, $("setPane"))) return;
  paneBase(p, $("setPane"));
}
function paneBase(p, pane) {
  if (p === "library") return drawLibrary(pane);
  if (p === "gen") return drawGen(pane);
  if (p === "api") return drawApi(pane);
  if (p === "conex") return drawConexiones(pane);
  if (p === "bases") return drawBases(pane);
  if (p === "buscadores") return drawBuscadores(pane);
  if (p === "conectores") return drawConectores(pane);
  const opt = (v, cur, label) => `<option value="${esc(v)}"${String(v) === String(cur) ? " selected" : ""}>${esc(label)}</option>`;
  if (p === "general") {
    // cada ajuste es una fila (nombre y explicación a la izquierda, control a la derecha) y se guarda al cambiar
    pane.innerHTML = `<h3>${tt("sGeneral")}</h3>
      <label class="f" data-fila="idioma"><span>${tt("lang")}<small>${tt("langNota")}</small></span><select id="sLang">${opt("auto", SET.lang, tt("auto"))}${opt("es", SET.lang, "Español")}${opt("en", SET.lang, "English")}</select></label>
      <label class="f" data-fila="tema"><span>${tt("theme")}<small>${tt("themeNota")}</small></span><select id="sTheme">${opt("dark", SET.theme, tt("dark"))}${opt("light", SET.theme, tt("light"))}${opt("system", SET.theme, tt("system"))}</select></label>`;
    const guardar = async () => {
      const lang = $("sLang").value, theme = $("sTheme").value;
      await invoke("apply_settings", { s: { lang, theme } });
      applyTheme(theme);
      const cambia = (lang === "auto" ? LANG : lang) !== LANG;
      LANG = lang === "auto" ? LANG : lang;
      applyI18n(); input.placeholder = tt("placeholder"); setBusy(S.busy);
      toast(tt("saved"));
      // con otro idioma, la página entera (barra incluida) se vuelve a pintar en él
      if (cambia) showPane("general");
    };
    $("sLang").onchange = guardar;
    $("sTheme").onchange = guardar;
  } else if (p === "model") {
    // A3: los de la nube van en la misma lista; con muchos (OpenRouter trae cientos), buscador
    const many = SET.models.length > 12;
    const errs = (SET.cloud_errors || []).map((e) => `<div class="note set-error">${esc(e.provider)}: ${esc(e.error)}</div>`).join("");
    // pulido: arriba la fila del modelo en uso (lo que se busca); debajo, la lista para cambiarlo
    const enUso = SET.models.find((m) => (m.path && m.path === SET.model_path) || m.name === SET.model || m.name === SET.model_path);
    pane.innerHTML = `<h3>${tt("sModel")}</h3><p>${tt("modelHelp")}</p>
      ${setFila("modelo", tt("flModelo"), tt("flModeloNota"), enUso ? `<span class="fila-valor">${esc(enUso.name)}</span>` : setEstado(tt("flSinModelo"), "warn"))}
      <p class="mm"><b>${tt("motor")}:</b> ${tt("motorAyuda")}</p>
      ${many ? `<input type="search" id="mFilter" class="mfilter" placeholder="${esc(tt("filterModels"))}">` : ""}${errs}<div class="models" id="mList"></div>`;
    const list = $("mList");
    const draw = (q) => { list.innerHTML = ""; for (const m of SET.models.filter((m) => !q || `${m.name} ${m.label || ""} ${m.kind}`.toLowerCase().includes(q))) drawModel(m); };
    if (many) $("mFilter").oninput = (e) => draw(e.target.value.trim().toLowerCase());
    const drawModel = (m) => {
      // marca el modelo EN USO: por ruta (lo fiable, local y nube) o por nombre de respaldo
      const cur = (m.path && m.path === SET.model_path) || m.name === SET.model || m.name === SET.model_path;
      const d = el("div", "model" + (cur ? " cur" : "") + (m.fits ? "" : " no"));
      // la ventana de contexto va como un cuadrito más, redondeada a «k» (40 960 → 41k);
      // el número exacto y quién pone el límite, al pasar el ratón
      const ctxTip = m.ctx_top ? `${tt("ctxWin")}: ${nf(m.ctx_top)} tokens` + (m.model_max && m.ctx_top < m.model_max ? ` · ${tt("ctxByMem").replace("{a}", nf(m.model_max))}` : "") : "";
      // dos copias del mismo modelo en carpetas distintas: el origen las distingue
      const twin = !m.cloud && m.source && SET.models.filter((x) => x.name === m.name).length > 1;
      const badges = [[twin ? m.source : "", m.path], [m.kind], [m.cloud ? "" : m.gb + " GB"], [m.fits && m.ctx_top ? kRound(m.ctx_top) : "", ctxTip], [m.vision ? tt("bVision") : "", tt("seesTip")], [m.audio ? tt("bAudio") : "", tt("hearsTip")], [m.chat_only ? "chat" : ""]]
        .filter(([b]) => b)
        .map(([b, tip]) => `<span class="badge"${tip ? ` title="${esc(tip)}"` : ""}>${esc(b)}</span>`)
        .join("");
      d.innerHTML = `<div class="mn">${esc(m.name)}${cur ? `<span class="m-uso">${tt("mEnUso")}</span>` : ""}</div><div class="badges">${badges}</div>${m.fits ? "" : `<div class="mm">${esc(tt("noFit") + " · " + (m.why || ""))}</div>`}`;
      d.title = m.label && m.label !== m.name ? `${m.label} · ${m.path}` : m.path;
      d.onclick = () => {
        if (cur) return;
        $("settingsModal").classList.remove("show");
        setBusy(true); S.ready = false;
        $("engTxt").textContent = tt("loadingModel");
        invoke("set_model", { path: m.path });
      };
      list.appendChild(d);
    };
    draw("");
  } else if (p === "prov") {
    drawProviders(pane);
  } else if (p === "context") {
    // el TOPE es el mismo número que la pantalla Modelo muestra para el modelo cargado
    // (ctx_top: lo que GOTV aplica con «Máximo»); sólo se ofrecen tamaños por debajo
    const cur = SET.models.find((m) => m.path === SET.model_path) || SET.models.find((m) => m.name === SET.model) || {};
    const top = cur.ctx_top || 0;
    // la ventana del PROYECTO abierto: las mismas opciones que el menú del medidor de arriba
    const { sel, items } = opcionesContexto(cur, SET.context_preset || "auto");
    const options = items.map(([v, label]) => opt(v, sel, label));
    const efforts = ["low", "medium", "high", "xhigh", "max"];
    const isClaude = cur.cloud && cur.provider === "anthropic";
    // pulido: cada ajuste, una fila con su explicación debajo del nombre; lo frecuente (la ventana) arriba
    const ctxNota = (cur.cloud ? tt("ctxCloudNote") + " " : top ? tt("ctxTopNote").replace("{m}", esc(cur.name || "")).replace("{t}", nf(top)) + (cur.model_max && top < cur.model_max ? " " + tt("ctxByMem").replace("{a}", nf(cur.model_max)) + "." : "") + " " : "") + tt("ctxNote");
    pane.innerHTML = `<h3>${tt("sContext")}</h3>
      <label class="f" data-fila="contexto"><span>${tt("ctxPreset")} · ${tt("ctxNow")} ${nf(SET.ctx_budget || 0)}<small>${ctxNota}</small></span><select id="sCtx">${options.join("")}</select></label>
      ${isClaude ? `<label class="f" data-fila="esfuerzo"><span>${tt("effort")}<small>${tt("effortNote")}</small></span><select id="sEffort">${efforts.map((e) => opt(e, SET.effort || "high", e)).join("")}</select></label>` : ""}
      <label class="f" data-fila="max-tokens"><span>${tt("maxTok")}<small>${tt("flMaxTokNota")}</small></span><input type="number" id="sMax" min="256" max="32768" step="256" value="${SET.max_tokens}"></label>
      ${cur.cloud ? "" : `<label class="f" data-fila="temperatura"><span>${tt("temp")}<small>${tt("tempNote")}</small></span><input type="number" id="sTemp" min="0" max="2" step="0.1" placeholder="—" value="${SET.temperature ?? ""}"></label>`}
      <div class="actions"><button class="btn primary" id="sSave">${tt("save")}</button></div>`;
    $("sSave").onclick = async () => {
      const s = { context_preset: $("sCtx").value, max_tokens: +$("sMax").value };
      if ($("sTemp") && $("sTemp").value !== "") s.temperature = +$("sTemp").value;
      if ($("sEffort")) s.effort = $("sEffort").value;
      await invoke("apply_settings", { s });
      toast(tt("saved"));
      SET = await invoke("settings");
    };
  } else if (p === "perms") {
    const modes = ["plan", "default", "acceptEdits", "auto", "bypass"];
    pane.innerHTML = `<h3>${tt("sPerms")}</h3>
      <label class="f" data-fila="modo"><span>${tt("defMode")}<small>${tt("flModoNota")}</small></span><select id="sMode">${modes.map((m) => opt(m, SET.mode, m)).join("")}</select></label>
      <div class="note ancha">${tt("permsModeNote")}</div>`;
    $("sMode").onchange = async () => {
      await invoke("apply_settings", { s: { mode: $("sMode").value } });
      $("mode").value = $("sMode").value;
      toast(tt("saved"));
    };
  } else if (p === "seguridad") {
    drawSecurity(pane);
  } else if (p === "pasos") {
    drawSteps(pane);
  }
}

// ── Servicio de API (A4) ─────────────────────────────────────────────────────
// GOTV como Ollama: los modelos de la app por una API OpenAI/Ollama, en esta Mac o
// en la red (con llave).
let API = null;
async function drawApi(pane, fresh = true) {
  if (fresh || !API) { try { API = await invoke("api_status"); } catch (e) { pane.innerHTML = setError(e); return; } }
  const a = API, on = a.running;
  const url0 = (a.urls && a.urls[0]) || `http://127.0.0.1:${a.port_wanted}`;
  const copyBtn = (v) => `<button class="mdcopy" data-copy="${esc(v)}">${tt("copy")}</button>`;
  const urls = (a.urls || []).map((u) => `<div class="api-url"><code>${esc(u)}/v1</code><span>OpenAI</span>${copyBtn(u + "/v1")}</div><div class="api-url"><code>${esc(u)}</code><span>Ollama</span>${copyBtn(u)}</div>`).join("");
  const auth = a.key ? ` \\\n  -H "Authorization: Bearer $GOTV_KEY"` : "";
  const curl = `curl ${url0}/v1/chat/completions${auth} \\\n  -H "content-type: application/json" \\\n  -d '{"model":"default","messages":[{"role":"user","content":"hola"}]}'`;
  // pulido: el estado (texto con un punto, sin ●/○), los interruptores y el puerto en filas; lo que sólo
  // existe encendido debajo; el teléfono; y AL FINAL lo que no se deshace (cambiar la llave)
  const estado = setEstado(tt(on ? "apiOn" : "apiOff") + (on ? ` · ${a.lan ? tt("apiInLan") : tt("apiOnlyHere")} · :${a.port}` : ""), on ? "ok" : "");
  pane.innerHTML = `<div class="api">
    <h3>${tt("apiTitle")}</h3><p class="lib-intro">${tt("apiIntro")}</p>
    ${setFila("", tt("flEstado"), esc(tt("apiHits").replace("{n}", a.hits || 0)), estado)}
    ${setFila("api-activar", tt("apiEnable"), tt("apiEnableNote"), setInterruptor("apiOn", a.enabled))}
    ${setFila("api-red", tt("apiLan"), tt("apiLanNote"), setInterruptor("apiLan", a.lan_wanted))}
    ${setFila("api-nube", tt("apiCloud"), tt("apiCloudNote"), setInterruptor("apiCloud", a.cloud))}
    ${setFila("api-puerto", tt("apiPort"), tt("flApiPuertoNota"), `<span class="pres" id="apiMsg"></span><input type="number" id="apiPort" class="corto" min="1024" max="65535" value="${a.port_wanted}"><button class="btn primary" id="apiApply">${tt("apiApply")}</button>`)}
    ${on ? `<h4>${tt("apiAddr")}</h4><div class="api-urls">${urls}</div><div class="note">${tt("apiCompat")}</div>
      ${a.key ? setFila("api-llave", tt("apiKey"), esc(tt("apiKeyNote")), `<code id="apiKeyTxt" data-k="${esc(a.key)}">${"•".repeat(24)}</code><button class="btn sm" id="apiKeyShow">${tt("apiShow")}</button>${copyBtn(a.key)}`) : ""}
      <h4>${tt("apiModels")}</h4><div class="api-models">${(a.models || []).length ? a.models.map((m) => `<span class="badge">${esc(m)}</span>`).join("") : `<span class="note">${tt("apiNoModel")}</span>`}</div>
      <h4>${tt("apiTry")}</h4><div class="msg-ans md">${mdRender("```bash\n" + curl + "\n```")}</div>
      ${setFila("telefono", tt("telTitle"), esc(tt("telIntro")), a.lan ? `<button class="btn" id="telPair">${tt("telPair")}</button>` : "")}
      ${a.lan ? `<div id="telCode"></div>` : `<div class="note">${esc(tt("telLocal"))}</div>`}
      <h4>${tt("telPaired")}</h4><div id="telList">…</div>` : ""}
    <h4>${tt("apiNet")}</h4><div class="note" style="margin-top:0">${esc(tt("apiNetNote"))}</div>
    ${on && a.key ? setFila("", tt("apiNewKey"), esc(tt("flApiNuevaNota")), `<button class="btn danger" id="apiKeyNew">${tt("apiNewKey")}</button>`, "peligro") : ""}
  </div>`;
  // F10.5: emparejar y revocar teléfonos (la lógica vive en Rust: `gotv_core::remoto`)
  const telList = async (lista) => {
    const box = $("telList"); if (!box) return;
    try { lista = lista || await invoke("remoto_tokens"); } catch (e) { box.innerHTML = setError(e); return; }
    box.innerHTML = lista.length ? lista.map((t) => setFila("", `<code>${esc(t.nombre)}</code>`, esc(new Date(t.creado * 1000).toLocaleDateString(LANG === "en" ? "en-US" : "es-CO")), `<button class="btn sm danger" data-rev="${esc(t.id)}">${tt("telRevoke")}</button>`, "item")).join("") : `<p class="note">${esc(tt("telNone"))}</p>`;
    box.querySelectorAll("[data-rev]").forEach((b) => (b.onclick = async () => { try { telList(await invoke("remoto_revocar", { id: b.dataset.rev })); toast(tt("telRevoked")); } catch (e) { setFallo(e); } }));
  };
  telList();
  if ($("telPair")) $("telPair").onclick = async () => {
    try {
      const c = await invoke("remoto_codigo");
      // el SVG del QR lo genera GOTV (Rust): sólo rectángulos, sin texto del usuario
      $("telCode").innerHTML = `<div class="note">${esc(tt("telCode").replace("{m}", c.minutos))}</div>${c.qr ? `<div style="width:180px;margin:8px 0">${c.qr}</div><div class="note">${esc(tt("telQr"))}</div>` : ""}${c.urls.map((u) => `<div class="api-url"><code>${esc(u)}</code>${copyBtn(u)}</div>`).join("")}<div class="api-key"><code style="font-size:22px;letter-spacing:4px">${esc(c.codigo)}</code></div>`;
      $("telCode").querySelectorAll("[data-copy]").forEach((b) => (b.onclick = async () => { try { await navigator.clipboard.writeText(b.dataset.copy); b.textContent = tt("copiadoAj"); } catch {} }));
    } catch (e) { $("telCode").innerHTML = setError(e); }
  };
  $("apiApply").onclick = async () => {
    const port = Math.round(+$("apiPort").value);
    $("apiMsg").textContent = "";
    try {
      API = await invoke("api_apply", { enabled: $("apiOn").checked, lan: $("apiLan").checked, port, cloud: $("apiCloud").checked });
      drawApi(pane, false); toast(tt("apiApplied"));
    } catch (e) { $("apiMsg").innerHTML = setEstado(String(e), "bad"); }
  };
  pane.querySelectorAll("[data-copy]").forEach((b) => (b.onclick = async () => { try { await navigator.clipboard.writeText(b.dataset.copy); b.textContent = tt("copiadoAj"); setTimeout(() => (b.textContent = tt("copy")), 1400); } catch { toast(tt("copyFail")); } }));
  pane.querySelectorAll(".mdcopy:not([data-copy])").forEach((b) => (b.onclick = async () => { try { await navigator.clipboard.writeText(b.closest(".mdcode").querySelector("code").textContent); b.textContent = tt("copiadoAj"); } catch {} }));
  if ($("apiKeyShow")) $("apiKeyShow").onclick = () => { const k = $("apiKeyTxt"), shown = k.textContent === k.dataset.k; k.textContent = shown ? "•".repeat(24) : k.dataset.k; $("apiKeyShow").textContent = tt(shown ? "apiShow" : "apiHide"); };
  if ($("apiKeyNew")) $("apiKeyNew").onclick = async () => { if (!(await confirmBox(tt("apiNewKey"), tt("apiNewKeyQ"), tt("apiNewKey")))) return; API = await invoke("api_new_key"); drawApi(pane, false); };
}

// ── Biblioteca de modelos ────────────────────────────────────────────────────
// El catálogo MLX de Hugging Face medido contra esta Mac (o una simulada): peso,
// si cabe, velocidad estimada con su etiqueta, filtros y descargas con progreso.
const LIB = { q: "", org: "", sort: "downloads", mac: "this", speed: "", fits: true, vision: false, installed: false, data: null, seq: 0, pulls: {} };
const SPEEDS = ["muy-rapido", "rapido", "medio", "lento", "muy-lento"];
const gbf = (b) => (b / 1e9).toLocaleString(LANG === "en" ? "en-US" : "es-CO", { maximumFractionDigits: b >= 1e10 ? 0 : 1 });
const numf = (n, d = 1) => n.toLocaleString(LANG === "en" ? "en-US" : "es-CO", { maximumFractionDigits: d });
const kdl = (n) => (n >= 1e6 ? numf(n / 1e6) + "M" : n >= 1e3 ? numf(n / 1e3, 0) + "k" : String(n));
function libModelPath(repo) {
  const r = repo.toLowerCase();
  const m = (SET.models || []).find((x) => (x.name || "").toLowerCase() === r);
  return m ? m.path : "";
}
function drawLibrary(pane) {
  const sel = (id, opts, cur) => `<select id="${id}">${opts.map(([v, l]) => `<option value="${esc(v)}"${v === cur ? " selected" : ""}>${esc(l)}</option>`).join("")}</select>`;
  pane.innerHTML = `<div class="lib">
    <div class="lib-head">
      <div><h3>${tt("libTitle")}</h3><p class="lib-intro">${tt("libIntro")}</p></div>
    </div>
    <div id="libMacBox" data-fila="lib-mac" class="fila"><div class="fila-txt"><div class="fila-nom">${tt("libMeasure")} · <span id="libMacName">—</span></div><div class="fila-nota"><span id="libMacBw"></span><br>${tt("flLibMacNota")}</div></div>
      <div class="fila-ctl"><select id="libMac" aria-label="${esc(tt("libMeasure"))}"><option value="this">${tt("libThisGroup")}</option></select></div></div>
    <div class="lib-sim" id="libSim" hidden>${tt("libSimNote")}</div>
    <div data-fila="lib-filtros">
    <div class="lib-bar">
      <div class="lib-search"><input type="search" id="libQ" placeholder="${esc(tt("libSearch"))}" value="${esc(LIB.q)}" autocomplete="off" spellcheck="false"></div>
      ${sel("libOrg", [["", tt("libOrgAll")], ["mlx-community", "mlx-community"], ["lmstudio-community", "lmstudio-community"]], LIB.org)}
      ${sel("libSort", [["downloads", tt("libSortDl")], ["trending", tt("libSortTrend")], ["recent", tt("libSortRecent")], ["likes", tt("libSortLikes")]], LIB.sort)}
    </div>
    <div class="lib-filters">
      <div class="seg" id="libSpeed" role="group" aria-label="${esc(tt("libSpeed"))}">
        <button data-s="" class="${LIB.speed ? "" : "on"}">${tt("spAll")}<i></i></button>
        ${SPEEDS.map((k) => `<button data-s="${k}" class="sp-${k}${LIB.speed === k ? " on" : ""}" title="${esc(tt("spTip_" + k))}"><span class="dot"></span>${tt("sp_" + k)}<i></i></button>`).join("")}
      </div>
      <div class="lib-toggles">
        <label class="switch-mini"><input type="checkbox" id="libFits"${LIB.fits ? " checked" : ""}>${tt("libOnlyFits")}</label>
        <label class="switch-mini"><input type="checkbox" id="libVision"${LIB.vision ? " checked" : ""}>${tt("libOnlyVision")}</label>
        <label class="switch-mini"><input type="checkbox" id="libInst"${LIB.installed ? " checked" : ""}>${tt("libOnlyInstalled")}</label>
      </div>
    </div>
    </div>
    <div class="lib-meta"><span id="libCount"></span><span class="lib-chat">${tt("libChat")}</span></div>
    <div class="lib-list" id="libList"><div class="lib-wait">${tt("libLoading")}</div></div>
  </div>`;
  let timer = 0;
  $("libQ").oninput = (e) => { LIB.q = e.target.value; clearTimeout(timer); timer = setTimeout(libFetch, 380); };
  $("libOrg").onchange = (e) => { LIB.org = e.target.value; libFetch(); };
  $("libSort").onchange = (e) => { LIB.sort = e.target.value; libFetch(); };
  $("libMac").onchange = (e) => { LIB.mac = e.target.value; libFetch(); };
  $("libSpeed").onclick = (e) => { const b = e.target.closest("button"); if (!b) return; LIB.speed = b.dataset.s; libPaint(); };
  $("libFits").onchange = (e) => { LIB.fits = e.target.checked; libPaint(); };
  $("libVision").onchange = (e) => { LIB.vision = e.target.checked; libPaint(); };
  $("libInst").onchange = (e) => { LIB.installed = e.target.checked; libPaint(); };
  $("libList").onclick = libAction;
  if (LIB.data) libPaint();
  libFetch();
}
async function libFetch() {
  const seq = ++LIB.seq;
  const list = $("libList");
  if (!list) return;
  if (!LIB.data) list.innerHTML = `<div class="lib-wait">${tt("libLoading")}</div>`;
  list.classList.add("busy");
  try {
    const d = await invoke("library_search", { query: LIB.q, org: LIB.org, sort: LIB.sort, mac: LIB.mac });
    if (seq !== LIB.seq) return; // llegó tarde: ya se pidió otra búsqueda
    LIB.data = d;
    for (const p of d.pulls || []) LIB.pulls[p.repo] = LIB.pulls[p.repo] || { done: p.done, total: p.total, rate: 0 };
    libPaintMac();
    libPaint();
  } catch (e) {
    if (seq === LIB.seq && $("libList")) $("libList").innerHTML = `<div class="lib-wait err">${esc(String(e))}</div>`;
  } finally {
    if ($("libList")) $("libList").classList.remove("busy");
  }
}
function libPaintMac() {
  const d = LIB.data, sel = $("libMac");
  if (!d || !sel) return;
  if (sel.options.length < 2) {
    sel.innerHTML = `<optgroup label="${esc(tt("libThisGroup"))}"><option value="this">${esc(d.this.label)}</option></optgroup>` +
      `<optgroup label="${esc(tt("libOtherGroup"))}">${d.presets.map((p) => `<option value="${esc(p.id)}">${esc(p.label)}</option>`).join("")}</optgroup>`;
  }
  sel.value = LIB.mac;
  $("libMacName").textContent = d.mac.label;
  $("libMacBw").textContent = tt("libBw").replace("{bw}", numf(d.mac.bandwidth, 0)).replace("{gpu}", numf(d.mac.budget / 2 ** 30, 0)) + (d.mac.exact ? "" : " · " + tt("libBwEst"));
  $("libSim").hidden = d.mac.this_mac;
  $("libMacBox").classList.toggle("sim", !d.mac.this_mac);
}
function libVisible() {
  const items = (LIB.data && LIB.data.items) || [];
  const base = items.filter((m) => (!LIB.fits || m.fits) && (!LIB.vision || m.vision) && (!LIB.installed || m.installed));
  return { base, shown: base.filter((m) => !LIB.speed || m.speed === LIB.speed), all: items };
}
function libPaint() {
  const list = $("libList");
  if (!list || !LIB.data) return;
  const { base, shown, all } = libVisible();
  // contador por velocidad sobre lo que dejan pasar los demás filtros
  document.querySelectorAll("#libSpeed button").forEach((b) => {
    const k = b.dataset.s;
    b.classList.toggle("on", k === LIB.speed);
    b.querySelector("i").textContent = k ? base.filter((m) => m.speed === k).length : base.length;
  });
  $("libCount").textContent = tt(LIB.data.mac.this_mac ? "libCount" : "libCountSim").replace("{n}", all.length).replace("{f}", all.filter((m) => m.fits).length);
  if (!shown.length) { list.innerHTML = `<div class="lib-wait">${tt("libEmpty")}</div>`; return; }
  list.innerHTML = shown.map(libRow).join("");
}
function libRow(m) {
  const [org, name] = m.repo.split("/");
  const size = m.active
    ? `<span class="badge moe" title="${esc(tt("libMoeTip"))}">${numf(m.params, 0)}B · ${numf(m.active)}B ${tt("libActive")}</span>`
    : `<span class="badge">${numf(m.params)}B</span>`;
  const badges = [size, m.bits ? `<span class="badge">${numf(m.bits, 0)}-bit</span>` : "", m.vision ? `<span class="badge">${tt("bVision")}</span>` : "",
    `<span class="badge dim" title="Hugging Face">${esc(tt("flDescargas").replace("{n}", kdl(m.downloads)))}</span>`, m.updated ? `<span class="badge dim">${esc(m.updated)}</span>` : ""].join("");
  const gpu = LIB.data.mac.budget;
  const needTip = tt("libNeed").replace("{a}", numf(m.need / 2 ** 30)).replace("{b}", numf(gpu / 2 ** 30, 0));
  // pulido: si cabe, en texto con un punto (sin ✓/◐/✕)
  const fit = `<span class="fit" title="${esc(needTip)}">${!m.fits ? setEstado(tt("libNoFit"), "bad") : m.tight ? setEstado(tt("libTight"), "warn") : setEstado(tt("libFits"), "ok")}</span>`;
  const speed = `<span class="speed sp-${m.speed}" title="${esc(tt("spTip_" + m.speed))}"><span class="dot"></span>${tt("sp_" + m.speed)}</span><span class="tps" title="${esc(tt("libEstNote"))}">~${numf(m.tok_s, 0)} tok/s</span>`;
  return `<div class="lib-row${m.fits ? "" : " nofit"}" data-repo="${esc(m.repo)}">
    <div class="lib-name"><div class="nm"><span class="org">${esc(org)}/</span>${esc(name)}</div><div class="badges">${badges}</div></div>
    <div class="lib-num"><b>${gbf(m.bytes)} GB</b>${fit}</div>
    <div class="lib-spd">${speed}</div>
    <div class="lib-act">${libActHtml(m)}</div>
  </div>`;
}
function libActHtml(m) {
  const p = LIB.pulls[m.repo];
  if (p) {
    const pct = p.total ? Math.min(100, Math.floor((p.done * 100) / p.total)) : 0;
    return `<div class="pbar"><div style="width:${pct}%"></div></div><div class="pinfo"><span>${pct} %${p.rate ? ` · ${numf(p.rate, 0)} MB/s` : ""}</span><button class="lnk" data-cancel="${esc(m.repo)}">${tt("cancel")}</button></div>`;
  }
  if (m.installed) {
    const path = libModelPath(m.repo);
    const inUse = path && path === SET.model_path;
    return inUse ? setEstado(tt("libInUse"), "ok") : `${setEstado(tt("libInstalled"))}${path && LIB.data.mac.this_mac ? `<button class="btn sm" data-use="${esc(path)}">${tt("libUse")}</button>` : ""}`;
  }
  if (!LIB.data.mac.this_mac) return "";
  return `<button class="btn sm primary" data-pull="${esc(m.repo)}"${m.fits ? "" : " disabled"}>${tt("libDownload")}</button>`;
}
async function libAction(e) {
  const b = e.target.closest("button");
  if (!b) return;
  if (b.dataset.pull) {
    b.disabled = true;
    try { toast(await invoke("library_pull", { repo: b.dataset.pull }), 4500); } catch (er) { setFallo(er); b.disabled = false; }
  } else if (b.dataset.cancel) {
    await invoke("library_cancel", { repo: b.dataset.cancel });
  } else if (b.dataset.use) {
    $("settingsModal").classList.remove("show");
    setBusy(true); S.ready = false;
    $("engTxt").textContent = tt("loadingModel");
    invoke("set_model", { path: b.dataset.use });
  }
}
function libUpdateRow(repo) {
  const row = document.querySelector(`.lib-row[data-repo="${CSS.escape(repo)}"] .lib-act`);
  const m = LIB.data && LIB.data.items.find((x) => x.repo === repo);
  if (row && m) row.innerHTML = libActHtml(m);
}
async function onPull(m) {
  if (GEN.data && GEN.data.items.some((x) => x.repo === m.repo)) return genOnPull(m);
  if (m.state === "run") { LIB.pulls[m.repo] = { done: m.done, total: m.total, rate: m.rate }; libUpdateRow(m.repo); return; }
  delete LIB.pulls[m.repo];
  if (m.state === "done") {
    toast(tt("libDone").replace("{repo}", m.repo), 6000);
    try { SET = await invoke("settings"); } catch {}
    const it = LIB.data && LIB.data.items.find((x) => x.repo === m.repo);
    if (it) it.installed = true;
  } else if (m.state === "error") toast(m.repo + ": " + m.error, 8000);
  else if (m.state === "cancel") toast(tt("libCanceled"), 4000);
  libUpdateRow(m.repo);
}

// ── Generadores (M0) ─────────────────────────────────────────────────────────
// Voz, imagen, video y música: el motor es de GOTV; los pesos, de su autor, con su licencia.
// Sin la casilla no se descarga; los restringidos piden el token de Hugging Face (al Llavero).
const GEN = { data: null, area: "", pulls: {} };
const GEN_AREAS = ["oir", "hablar", "imagen", "video", "musica", "buscar"];
async function drawGen(pane) {
  pane.innerHTML = `<div class="lib gen">
    <div class="lib-head"><div><h3>${tt("genTitle")}</h3><p class="lib-intro">${tt("genIntro")}</p></div></div>
    <div class="gen-token" id="genTok"></div>
    <div class="lib-filters"><div class="seg" id="genArea" role="group" aria-label="${esc(tt("genArea"))}">
      <button data-a="" class="on">${tt("spAll")}<i></i></button>
      ${GEN_AREAS.map((a) => `<button data-a="${a}">${tt("genA_" + a)}<i></i></button>`).join("")}
    </div></div>
    <div class="lib-list" id="genList"><div class="lib-wait">${tt("libLoading")}</div></div>
    <p class="gen-foot">${tt("genFoot")}</p>
  </div>`;
  $("genArea").onclick = (e) => { const b = e.target.closest("button"); if (!b) return; GEN.area = b.dataset.a; genPaint(); };
  $("genList").onclick = genAction;
  $("genList").onchange = genAccept;
  await genFetch();
}
async function genFetch() {
  try {
    GEN.data = await invoke("generators_list");
    for (const p of GEN.data.pulls || []) if (GEN.data.items.some((x) => x.repo === p.repo)) GEN.pulls[p.repo] = GEN.pulls[p.repo] || { done: p.done, total: p.total, rate: 0 };
    genPaint();
  } catch (e) {
    if ($("genList")) $("genList").innerHTML = `<div class="lib-wait err">${esc(String(e))}</div>`;
  }
}
function genPaint() {
  const list = $("genList");
  if (!list || !GEN.data) return;
  const items = GEN.data.items;
  document.querySelectorAll("#genArea button").forEach((b) => {
    b.classList.toggle("on", b.dataset.a === GEN.area);
    b.querySelector("i").textContent = b.dataset.a ? items.filter((g) => g.area === b.dataset.a).length : items.length;
  });
  const tok = items.length && items[0].token_hf;
  // pulido: el token es una fila (nombre y estado a la izquierda; la caja y sus botones a la derecha)
  $("genTok").innerHTML = setFila("hf-token", tt("genTokTitle"), (tok ? setEstado(tt("genTokSaved"), "ok") : esc(tt("genTokNone"))),
    `<input type="password" id="genTokIn" placeholder="hf_…" autocomplete="off" spellcheck="false">
    <button class="btn sm primary" data-tok="1">${tt("save")}</button>${tok ? `<button class="btn sm" data-tokdel="1">${tt("genTokDel")}</button>` : ""}`);
  $("genTok").onclick = genAction;
  const shown = items.filter((g) => !GEN.area || g.area === GEN.area);
  list.innerHTML = shown.map(genCard).join("");
  // llegaste aquí porque una función pedía este modelo: su fila, a la vista y resaltada
  if (GEN.foco) {
    const c = list.querySelector(`.gen-card[data-repo="${CSS.escape(GEN.foco)}"]`);
    GEN.foco = null;
    if (c) {
      c.scrollIntoView({ block: "center", behavior: "smooth" });
      c.classList.remove("fila-luz"); void c.offsetWidth; c.classList.add("fila-luz", "gen-falta");
      setTimeout(() => c.classList.remove("fila-luz"), 2200);
    }
  }
}
// ── Lo que falta, a su fila. Si una función pide un modelo que no está en esta Mac (lo decide el núcleo:
// `generador_falta`), Configuración → Generadores se abre en él, subrayado, listo para leer, aceptar y bajar.
async function irAGenerador(repo) {
  GEN.foco = repo; GEN.area = "";
  await openSettings("gen");
}
// true = está todo; si falta algo, avisa y lleva a su fila (`siempre` = false: sólo si no es el mismo de antes)
async function exigeModelo(uso, pro, siempre = true) {
  let repo = null;
  try { repo = await invoke("generador_falta", { uso, pro: pro || null }); } catch { return true; }
  if (!repo) { S.faltaVisto = null; return true; }
  if (siempre || repo !== S.faltaVisto) {
    S.faltaVisto = repo;
    toast(tt("genFaltaIr").replace("{repo}", repo), 8000);
    await irAGenerador(repo);
  }
  return false;
}
// el perfil Imagen, Video o Música pide su modelo, y lo elegido en su panel PRO encendido (klein base, el DiT,
// el planificador, el separador…) los suyos
function exigePerfil(siempre = true) {
  const p = S.perfil;
  if (!PRO_PERFILES.includes(p)) return Promise.resolve(true);
  return exigeModelo(p, proActivo() ? S.pro[p] : {}, siempre);
}
// un error que manda a Generadores: si nombra un modelo, se va a su fila (`ir`) o se ofrece un botón en `nodo`
async function faltaEnAviso(texto, nodo, ir) {
  if (!/Generadores|Generators/.test(texto || "")) return;
  let repo = null;
  try { repo = await invoke("generador_de_aviso", { texto }); } catch { return; }
  if (!repo) return;
  if (ir) return irAGenerador(repo);
  if (!nodo || nodo.querySelector(".gen-ir")) return;
  const b = el("button", "btn sm gen-ir", tt("genIrBajar"));
  b.type = "button";
  b.onclick = (e) => { e.stopPropagation(); irAGenerador(repo); };
  nodo.appendChild(b);
}
function genCard(g) {
  const [org, name] = g.repo.split("/");
  // pulido: la licencia y el acceso restringido, en texto con un punto (sin colores de fondo ni 🔒)
  const tono = { libre: "ok", atribucion: "ok", condiciones: "warn", no_comercial: "warn", sin_licencia: "warn" }[g.clase] || "";
  const acc = g.aceptada;
  const badges = [`<span class="badge">${esc(g.area_nombre)}</span>`, setEstado(g.etiqueta, tono),
    g.restringido ? `<span title="${esc(tt("genGatedTip"))}">${setEstado(tt("genGated"), "warn")}</span>` : "",
    `<span class="badge dim" title="${esc(tt("genPhaseTip"))}">${tt("genEngine")} ${esc(g.fase)}</span>`].join("");
  return `<div class="gen-card${acc ? " acc" : ""}" data-repo="${esc(g.repo)}">
    <div class="gen-top"><div class="lib-name"><div class="nm">${esc(g.nombre)} <span class="org">${esc(org)}/${esc(name)}</span></div><div class="badges">${badges}</div></div>
      <div class="lib-num"><b>${gbf(g.bytes_repo)} GB</b><span class="gen-sub">${tt("genRepoSize")}</span></div></div>
    <div class="gen-lic"><span>${tt("genLicense")}: <b>${esc(g.licencia)}</b></span><button class="gen-link${g.leida ? "" : " gen-leer"}" data-leer="${esc(g.repo)}">${tt("genReadLicense")}</button>${g.leida ? `<span class="gen-sub">${tt("genLeida")}</span>` : ""}
      <span class="gen-sub">${tt("genUnmeasured")}</span></div>
    <p class="gen-aviso">${esc(g.aviso)}</p>
    ${(g.afinados || []).length ? `<p class="gen-sub gen-afinado" title="${esc(tt("genAfinados"))}">${esc(tt("genAfinados"))}: ${g.afinados.map(esc).join(" · ")}</p>` : ""}
    <div class="gen-act"><label class="tg gen-ok${acc || g.leida ? "" : " bloq"}" title="${acc || g.leida ? "" : esc(tt("genLeerPrimero"))}"><input type="checkbox" data-accept="${esc(g.repo)}"${acc ? " checked" : ""}${acc || g.leida ? "" : " disabled"}>${tt("genAccept")}</label>${acc || g.leida ? "" : `<span class="gen-sub gen-pide">${tt("genLeerPrimero")}</span>`}
      <span class="gen-when">${acc ? tt(acc.sha ? "genAcceptedOn" : "genAcceptedOnSinPublicar").replace("{f}", esc(acc.fecha.slice(0, 10))).replace("{c}", esc(acc.sha)) : ""}</span>
      <div class="gen-dl">${genActHtml(g)}</div></div>
  </div>`;
}
function genActHtml(g) {
  const p = GEN.pulls[g.repo];
  if (p) {
    const pct = p.total ? Math.min(100, Math.floor((p.done * 100) / p.total)) : 0;
    return `<div class="pbar"><div style="width:${pct}%"></div></div><div class="pinfo"><span>${pct} %${p.rate ? ` · ${numf(p.rate, 0)} MB/s` : ""}</span><button class="lnk" data-cancel="${esc(g.repo)}">${tt("cancel")}</button></div>`;
  }
  if (g.bytes_local > 0) return setEstado(tt("genHave").replace("{gb}", gbf(g.bytes_local)), "ok");
  const falta = !g.aceptada ? tt(g.leida ? "genNeedAccept" : "genLeerPrimero") : g.restringido && !g.token_hf ? tt("genNeedToken") : "";
  return `<button class="btn sm primary" data-gpull="${esc(g.repo)}"${falta ? ` disabled title="${esc(falta)}"` : ""}>${tt("libDownload")}</button>${falta && (g.aceptada || g.leida) ? `<span class="gen-sub">${esc(falta)}</span>` : ""}`;
}
async function genAccept(e) {
  const c = e.target.closest("input[data-accept]");
  if (!c) return;
  try { await invoke("generator_accept", { repo: c.dataset.accept, accept: c.checked }); } catch (er) { setFallo(er); }
  await genFetch();
}
async function genAction(e) {
  const b = e.target.closest("button");
  if (!b) return;
  if (b.dataset.link) return invoke("open_link", { url: b.dataset.link }).catch(setFallo);
  if (b.dataset.leer) {
    try { await invoke("generator_read", { repo: b.dataset.leer }); } catch (er) { setFallo(er); }
    return genFetch();
  }
  if (b.dataset.gpull) {
    b.disabled = true;
    try { toast(await invoke("generator_pull", { repo: b.dataset.gpull }), 4500); } catch (er) { toast(String(er), 9000); b.disabled = false; }
  } else if (b.dataset.cancel) {
    await invoke("library_cancel", { repo: b.dataset.cancel });
  } else if (b.dataset.tok || b.dataset.tokdel) {
    const v = b.dataset.tokdel ? "" : $("genTokIn").value;
    if (b.dataset.tok && !v.trim()) return;
    try { const hay = await invoke("hf_token", { token: v }); toast(tt(hay ? "genTokSaved" : "genTokDeleted")); } catch (er) { setFallo(er); }
    await genFetch();
  }
}
function genOnPull(m) {
  const row = () => document.querySelector(`.gen-card[data-repo="${CSS.escape(m.repo)}"] .gen-dl`);
  const g = GEN.data.items.find((x) => x.repo === m.repo);
  if (m.state === "run") {
    GEN.pulls[m.repo] = { done: m.done, total: m.total, rate: m.rate };
    if (row() && g) row().innerHTML = genActHtml(g);
    return;
  }
  delete GEN.pulls[m.repo];
  if (m.state === "done") { toast(tt("genDone").replace("{repo}", m.repo), 6000); genFetch(); return; }
  if (m.state === "error") toast(m.repo + ": " + m.error, 9000);
  else if (m.state === "cancel") toast(tt("libCanceled"), 4000);
  if (row() && g) row().innerHTML = genActHtml(g);
}

// Administrador de pasos: cuántas vueltas puede dar el agente por cada orden.
function drawSteps(pane) {
  const presets = [[20, "stepsQuick"], [40, "stepsNormal"], [80, "stepsWide"], [150, "stepsBig"]];
  const cur = SET.max_steps || 40;
  const isPreset = presets.some(([n]) => n === cur);
  // pulido: dos filas que se guardan al cambiar, como el modo de permisos de arriba (antes: tarjetas de
  // opción y un botón «Guardar»); las preguntas frecuentes, debajo y en texto secundario
  const opciones = presets.map(([n, k]) => `<option value="${n}"${n === cur ? " selected" : ""}>${n} · ${esc(tt(k))}</option>`).join("") + `<option value="custom"${isPreset ? "" : " selected"}>${esc(tt("stepsCustom"))}</option>`;
  pane.innerHTML = `<h3>${tt("sSteps")}</h3>
    ${setFila("pasos", tt("flPasos"), tt("flPasosNota"), `<span class="pres" id="stpMsg"></span><input type="number" id="stpN" class="corto" min="5" max="500" step="5" value="${cur}"${isPreset ? " hidden" : ""}><select id="stpSel">${opciones}</select>`)}
    ${setFila("subagentes", tt("stepsParalelo"), tt("stepsParaleloA"), `<input type="number" id="stpSub" class="corto" min="1" max="16" step="1" value="${SET.subagentes || 4}">`)}
    <div class="faq"><p>${tt("stepsWhat")}</p>
    <h4>${tt("stepsLimitQ")}</h4><p>${tt("stepsLimitA")}</p>
    <h4>${tt("stepsWhyQ")}</h4><p>${tt("stepsWhyA")}</p>
    <h4>${tt("stepsSubQ")}</h4><p>${tt("stepsSubA")}</p></div>`;
  const msg = (k) => ($("stpMsg").innerHTML = k ? setEstado(tt(k), "bad") : "");
  const guardar = async (s) => {
    msg("");
    await invoke("apply_settings", { s });
    SET = await invoke("settings"); S.maxSteps = SET.max_steps;
    toast(tt("stepsSaved").replace("{n}", SET.max_steps));
  };
  $("stpSel").onchange = (e) => {
    const custom = e.target.value === "custom";
    $("stpN").hidden = !custom;
    if (custom) { $("stpN").focus(); return; }
    $("stpN").value = e.target.value;
    guardar({ max_steps: +e.target.value });
  };
  $("stpN").onchange = () => {
    const n = Math.round(+$("stpN").value);
    if (!(n >= 5 && n <= 500)) return msg("stepsRange");
    guardar({ max_steps: n });
  };
  $("stpSub").onchange = () => {
    const sub = Math.round(+$("stpSub").value);
    if (!(sub >= 1 && sub <= 16)) return msg("stepsParaleloRango");
    guardar({ subagentes: sub });
  };
}

// Seguridad. Pulido (2026-11-18): la «Caja de arena y acceso» de antes se parte en cuatro piezas, cada una en
// su página (ventana.rs → `GRUPOS`): la caja de arena (encendida de serie), Chrome con tus sesiones (en la
// página «Navegador», con los sitios), el control del Mac y, la última, modificar otras apps y el sistema (pide
// tu contraseña de administrador). Sin `parte` (lo de antes, sin estructura) pinta las cuatro seguidas.
const SEG_PARTES = ["caja", "chrome", "mac", "sistema"];
function drawSecurity(pane, parte) {
  if (!parte) {
    pane.innerHTML = `<h3>${tt("sSecurity")}</h3>`;
    for (const p of SEG_PARTES) { const n = document.createElement("section"); pane.appendChild(n); drawSecurity(n, p); }
    return;
  }
  const repinta = async () => { SET = await invoke("settings"); drawSecurity(pane, parte); };
  const sinDosPuntos = (t) => t.replace(/:\s*$/, "");
  if (parte === "caja") {
    const sb = SET.sandbox !== false;
    const learned = (SET.net_learned || []).map((h) => `<code>${esc(h)}</code>`).join(" ") || esc(tt("sbNone"));
    const estado = SET.sandbox_available === false ? setEstado(tt("sbMissing"), "bad") : setEstado(tt(sb ? "sbOn" : "sbOff"), sb ? "ok" : "bad");
    pane.innerHTML = `<p class="seg-intro">${tt("sbWhat")}</p>
      ${setFila("caja", tt("sbTitle"), `${tt("flCajaNota")}<span class="estados">${estado}</span>`, setInterruptor("sSb", sb))}
      ${setFila("", esc(sinDosPuntos(tt("sbDefault"))), (SET.net_default || []).map((h) => `<code>${esc(h)}</code>`).join(" "))}
      ${setFila("", esc(sinDosPuntos(tt("sbLearned"))), learned)}
      ${setFila("caja-dominios", tt("flCajaDominios"), esc(sinDosPuntos(tt("sbExtra"))),
        `<textarea id="sbExtra" rows="3" spellcheck="false" placeholder="github.com&#10;*.mi-empresa.com">${esc((SET.net_allow || []).join("\n"))}</textarea><button class="btn" id="sbSave">${tt("save")}</button>`, "bloque")}`;
    // C2: apagar la caja deja a los comandos con acceso a todo el Mac: se confirma
    $("sSb").onchange = async (e) => {
      const on = e.target.checked;
      if (!on && !(await confirmBox(tt("sbTitle"), tt("sbConfirmOff"), tt("sbTurnOff")))) { e.target.checked = true; return; }
      await invoke("apply_settings", { s: { sandbox: on } });
      await repinta(); toast(tt(on ? "sbOn" : "sbOff"));
    };
    $("sbSave").onclick = async () => {
      const hosts = $("sbExtra").value.split(/[\s,]+/).map((h) => h.trim()).filter(Boolean);
      await invoke("apply_settings", { s: { net_allow: hosts } });
      await repinta(); toast(tt("saved"));
    };
  } else if (parte === "chrome") {
    // W3: Chrome con tus sesiones (sólo el grupo «GOTV»)
    pane.innerHTML = `<p class="seg-intro">${tt("crWhat")}</p>
      ${setFila("chrome", tt("flChromeFila"), `${tt("flChromeNota")}<span class="estados" id="crState">${setEstado("…")}</span>`,
        `<button class="btn" id="crReveal">${tt("crReveal")}</button><button class="btn primary" id="crInstall">${tt("crInstall")}</button>`)}
      <ol class="seg-pasos">${tt("crSteps")}</ol>`;
    const crPaint = (st) => {
      $("crState").innerHTML = setEstado(`${tt("crBridge")}: ${tt(st.installed ? "crYes" : "crNo")}`, st.installed ? "ok" : "warn") +
        setEstado(`${tt("crExt")}: ${tt(st.connected ? "crConnected" : "crNotConnected")}`, st.connected ? "ok" : "warn");
    };
    invoke("chrome_status").then(crPaint).catch((e) => ($("crState").innerHTML = setEstado(String(e), "bad")));
    $("crInstall").onclick = async () => { try { crPaint(await invoke("chrome_install")); toast(tt("crInstalled")); } catch (e) { setFallo(e); } };
    $("crReveal").onclick = () => invoke("reveal_ext");
  } else if (parte === "mac") {
    const dk = SET.desktop_control;
    pane.innerHTML = `<p class="seg-intro">${tt("dkWhat")}</p>
      ${setFila("mac-nativo", tt("dkTitle"), tt("flMacNota"), setInterruptor("sDk", dk))}
      ${dk ? setFila("", tt("flMacPermisos"), `<span class="estados" id="dkPerm">${setEstado(tt("dkChecking"))}</span>`,
        `<button class="btn" id="dkGuide">${tt("dkGuide")}</button><button class="btn" id="dkAx">${tt("dkAx")}</button><button class="btn" id="dkScreen">${tt("dkScreen")}</button>`) : ""}
      ${setFila("mac-capturas", tt("computerOld"), `${tt("computer")}. ${tt("computerNote")}`, setInterruptor("sPc", SET.computer_use))}`;
    $("sPc").onchange = async (e) => { await invoke("apply_settings", { s: { computer_use: e.target.checked } }); toast(tt("saved")); };
    $("sDk").onchange = async (e) => {
      const on = e.target.checked;
      if (on && !(await confirmBox(tt("dkTitle"), tt("dkConfirm"), tt("dkEnable")))) { e.target.checked = false; return; }
      await invoke("apply_settings", { s: { desktop_control: on } });
      SET = await invoke("settings"); S.desktopOk = !!SET.desktop_control; if (!S.desktopOk) S.mac = false; paintMac();
      drawSecurity(pane, parte); toast(tt(on ? "dkOn" : "dkOff"));
    };
    if (dk) {
      invoke("desktop_status").then((p) => {
        const st = (nombre, x) => (x && x.state === "granted" ? setEstado(`${tt(nombre)}: ${tt("dkGranted")}`, "ok") : setEstado(`${tt(nombre)}: ${tt("dkMissing")}`, "warn"));
        $("dkPerm").innerHTML = st("dkAxName", p.accessibility) + st("dkScreenName", p.screen_recording);
      }).catch((e) => { $("dkPerm").innerHTML = setEstado(String(e), "bad"); });
      $("dkGuide").onclick = () => invoke("desktop_permissions", { pane: "guide" });
      $("dkAx").onclick = () => invoke("desktop_permissions", { pane: "ax" });
      $("dkScreen").onclick = () => invoke("desktop_permissions", { pane: "screen" });
    }
  } else if (parte === "sistema") {
    const sys = SET.system_changes;
    pane.innerHTML = `<p class="seg-intro">${tt("secSysWarn")}</p>
      ${setFila("sistema", tt("secSysTitle"), sys ? `${tt("flSistemaNota")}<span class="estados">${setEstado(tt("secSysOn"), "bad")}</span>` : tt("flSistemaNota"),
        sys ? `<button class="btn" id="secOff">${tt("secDisable")}</button>` : `<span class="pres" id="secMsg"></span><button class="btn danger" id="secOn">${tt("flSistemaActivar")}</button>`, "peligro")}
      ${sys ? setFila("", tt("flGestionApps"), tt("secGrant"), `<button class="btn" id="secGrant">${tt("flAbrirAjustesMac")}</button>`) : ""}`;
    if ($("secOn")) $("secOn").onclick = async () => {
      $("secMsg").textContent = tt("secAuthing");
      try {
        await invoke("set_system_changes", { on: true }); // macOS pide Touch ID/contraseña
        await repinta(); toast(tt("secSysOn"));
      } catch (e) { $("secMsg").innerHTML = setEstado(String(e), "bad"); }
    };
    if ($("secOff")) $("secOff").onclick = async () => { await invoke("set_system_changes", { on: false }); await repinta(); toast(tt("saved")); };
    if ($("secGrant")) $("secGrant").onclick = () => invoke("open_app_management");
  }
}

// ── Eventos del núcleo ────────────────────────────────────────────────────
T.event.listen("gotv", ({ payload: m }) => {
  switch (m.kind) {
    case "line": S.live = null; addLine(m.text); break;
    // F12 · /objetivo: la ronda siguiente la manda GOTV; la ventana sigue ocupada (el stop la para)
    case "objetivo": S.live = null; addLine(m.text); setBusy(true); break;
    case "delta":
      if (!S.live) { if (!m.text.trim()) break; closeWork(); clearEmpty(); S.live = el("div", "msg-ans md live", ""); stick(() => log.appendChild(S.live)); }
      liveAppend(m.text);
      break;
    case "prefill": $("hint").textContent = `${tt("working")} ${Math.floor((m.done * 100) / Math.max(1, m.total))} %`; break;
    case "precarga": precarga(m); break;
    // A1: generando, con la velocidad real del motor (cada 0,5 s)
    case "rate": if (S.busy && m.tok_s > 0) $("hint").textContent = `${tt("generating")} · ≈ ${m.tok_s.toLocaleString(LANG === "en" ? "en-US" : "es-CO", { maximumFractionDigits: 1 })} tok/s`; break;
    // A2: tokens del turno en la cabecera del bloque de trabajo
    case "usage": if (S.work) { const u = m.usage; S.work.querySelector(".wtok").textContent = `↑ ${taskTok(u.in)} · ↓ ${taskTok(u.out)} tok`; S.work.querySelector(".wtok").title = `${tt("tokIn")}: ${u.in}${u.cached ? ` (${tt("tokCached")}: ${u.cached})` : ""} · ${tt("tokOut")}: ${u.out}`; } break;
    case "stats": if (m.budget) setCtx(m.used, m.budget); break;
    case "status": setStatus(m.status); break;
    case "compacted": break;
    case "ask": showAsk(m.id, m.desc, m.partes); break;
    // F10.5: el permiso lo contestó el teléfono: el diálogo se cierra (su botón ya no vale)
    case "ask_cerrado":
      if (S.askId === m.id) { S.askId = null; $("askModal").classList.remove("show"); addLine("   ➜ " + (m.aprobado ? "✓ " : "✗ ") + tt("telAnswered")); }
      break;
    case "choose": showChoose(m.id, m.question, m.options); break;
    case "secreto": showSecreto(m.id, m.etiqueta); break;
    case "tasks": S.tasks = m.tasks; renderTasks(); genVivo(); break;
    case "procs": S.procs = procStamp(m.procs); renderTasks(); break;
    case "pull": onPull(m); break;
    case "noproject": noProject(); $("hint").textContent = tt("noProjectHint"); break;
    case "reposo":
      S.reposo = true; S.ready = true; noProject(); engEstado();
      $("hint").textContent = tt(S.projects.length ? "reposoHint" : "noProjectHint");
      break;
    case "project":
      S.reposo = false; engEstado();
      log.innerHTML = ""; S.work = null; S.live = null;
      // las imágenes generadas vuelven como su tarjeta (la misma vista que en vivo)
      for (const h of m.history || []) {
        // lo de arriba es lo que la compactación sacó de la vista del modelo (se guarda aparte para ti)
        if (h.role === "compactado") { const d = el("div", "compactado"); d.textContent = tt("compactadoSep"); log.appendChild(d); continue; }
        if (h.role === "vista") addVista(h);
        else (h.role === "user" ? addUser : addAnswer)(h.text);
        // F8.1: su lugar en el historial (bifurcar desde aquí)
        if (h.i != null && log.lastElementChild) log.lastElementChild.dataset.i = h.i;
      }
      setProject(m.root); showEmpty(); loadSlash();
      // abrir un proyecto DESBLOQUEA el chat, venga por donde venga (medido: tras
      // cerrar uno y crear otro llega «done», no «ready», y el chat quedaba bloqueado)
      if (m.root) { input.disabled = false; refreshSend(); }
      break;
    case "done":
      closeWork();
      if (S.live) {
        const node = S.live;
        if (node._raf) { clearTimeout(node._raf); node._raf = 0; }
        node.classList.remove("live");
        // el texto final del núcleo manda si continúa lo que llegó en vivo (viene entero y
        // sin restos de llamadas); si no (p. ej. interrumpido), se queda lo que se vio
        const raw = node._raw || "", fin = m.text || "";
        // `limpia`: el núcleo quitó la ubicación/licencia de lo generado (va en la tarjeta ⓘ):
        // manda su versión aunque sea más corta que lo que se vio en vivo
        mdInto(node, (m.limpia && fin) || !raw.trim() || (fin.length >= raw.length && fin.startsWith(raw.trim().slice(0, 40))) ? fin || raw : raw);
        vozBtn(node);
        // la respuesta explica que falta un modelo: el botón a su fila, debajo
        faltaEnAviso(node._raw || fin, node);
        S.live = null;
      }
      else if (m.text && !m.silent) (m.info ? (t) => stick(() => log.appendChild(el("div", "msg-info", t))) : addAnswer)(m.text);
      setBusy(false); S.ready = true; setStatus({}); input.focus();
      rapidoEstado();
      break;
    case "error": {
      closeWork(); S.live = null;
      const n = el("div", "msg-err", "✗ " + m.text);
      stick(() => log.appendChild(n));
      faltaEnAviso(m.text, n);
      setBusy(false);
      break;
    }
    case "vista": addVista(m); break;
    // M3: manos libres — su estado, y tu frase dicha entra al chat como tu mensaje
    case "manos": {
      // se apagó sola (por voz o por un error) y no porque cambiaste de proyecto: también se recuerda
      const sola = m.estado === "apagado" && S.manos && !S.aplicandoChat;
      manosEstado(m.estado);
      if (sola) guardarChat();
      if (m.error) { toast("✗ " + m.error, 6000); faltaEnAviso(m.error, null, true); }
      break;
    }
    // (con el distintivo de Control Mac si está encendido: la orden dicha lo lleva igual)
    case "voz_tu": addUser(m.text, !!(S.mac && S.desktopOk), m.text, Date.now()); setBusy(true); break;
    // M2: terminó (o se calló) la lectura en voz alta
    case "voz": if (m.id == null || !S.vozNode || S.vozNode.id === m.id) vozFin(); if (m.error && !/callada/.test(m.error)) { toast("✗ " + m.error, 9000); faltaEnAviso(m.error, null, true); } break;
    case "ready": S.ready = true; if (S.root) { input.disabled = false; setBusy(false); input.focus(); } break;
  }
});

// ── Arranque ──────────────────────────────────────────────────────────────
(async () => {
  const i = await invoke("info");
  S.home = i.home || "";
  S.sueltasBase = i.sueltas || "";
  S.desktopOk = !!i.desktop_control; S.mac = false;
  LANG = i.lang || "es";
  document.documentElement.lang = LANG;
  applyTheme(i.theme);
  applyI18n();
  input.placeholder = tt("placeholder");
  $("hint").textContent = tt("starting");
  paintMac();
  if (!i.ready) $("engTxt").textContent = tt("starting");
  S.projects = i.projects || [];
  S.tasks = i.tasks || [];
  S.procs = procStamp(i.procs);
  renderTasks();
  if (!i.root) {
    // en reposo: la app abrió sin proyecto y sin modelo (se carga al elegir uno)
    if (i.reposo) S.reposo = true;
    noProject();
    if (i.ready) { setStatus(i); engEstado(); }
    if (S.reposo) $("hint").textContent = tt(S.projects.length ? "reposoHint" : "noProjectHint");
    return;
  }
  setProject(i.root);
  aplicarChat(i.root);
  if (i.ready) { S.ready = true; setStatus(i); input.disabled = false; setBusy(false); }
  showEmpty();
  loadSlash();
})();
