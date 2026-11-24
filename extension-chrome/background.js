// GOD OF THE VOID en Chrome (W3). Diseño estudiado de Claude in Chrome (sin copiar código):
// el GRUPO de pestañas «GOTV» es la única frontera; el árbol de accesibilidad da refs; la
// entrada (clic, teclas) va por chrome.debugger (CDP), confiable; un borde morado marca la
// pestaña controlada. El puente es SÓLO local: mensajería nativa → la app de GOTV.
//
// Reglas que el agente no puede saltar (viven AQUÍ, en tu navegador):
// - sólo pestañas del grupo «GOTV»; fuera de él, nada;
// - bancos, pagos, correo y cuentas: bloqueados (ni abrir ni actuar);
// - jamás escribe en un campo de contraseña.

const HOST = "space.chaosx.gotv";
const GROUP_TITLE = "GOTV";
const BLOCKED = [
  /(^|\.)paypal\.com$/, /(^|\.)stripe\.com$/, /(^|\.)mercadopago\./, /(^|\.)nequi\./, /(^|\.)daviplata\./,
  /(^|\.)bancolombia\./, /(^|\.)davivienda\./, /(^|\.)bbva\./, /(^|\.)santander\./, /(^|\.)chase\.com$/,
  /(^|\.)bankofamerica\.com$/, /(^|\.)wellsfargo\.com$/, /bank/i, /banco/i,
  /(^|\.)binance\.com$/, /(^|\.)coinbase\.com$/, /(^|\.)kraken\.com$/,
  /^mail\.google\.com$/, /^accounts\.google\.com$/, /^myaccount\.google\.com$/, /^pay\.google\.com$/,
  /^outlook\.(live|office|office365)\.com$/, /^login\.microsoftonline\.com$/, /(^|\.)icloud\.com$/,
  /^appleid\.apple\.com$/, /(^|\.)proton\.me$/,
];

let port = null;
const refs = {};      // tabId → { eN: backendDOMNodeId }
const attached = {};  // tabId → true

// ── puente ──────────────────────────────────────────────────────────────
function connect() {
  if (port) return;
  try {
    port = chrome.runtime.connectNative(HOST);
  } catch (e) {
    port = null;
    setTimeout(connect, 5000);
    return;
  }
  port.onMessage.addListener(onMessage);
  port.onDisconnect.addListener(() => {
    port = null;
    setTimeout(connect, 5000);
  });
}
chrome.runtime.onStartup.addListener(connect);
chrome.runtime.onInstalled.addListener(connect);
connect();

async function onMessage(m) {
  if (!m || m.id === undefined) return;
  let reply;
  try {
    reply = { id: m.id, ok: true, result: await handle(m) };
  } catch (e) {
    reply = { id: m.id, ok: false, error: String(e && e.message ? e.message : e) };
  }
  if (port) port.postMessage(reply);
}

// ── frontera: grupo «GOTV» y sitios bloqueados ────────────────────────
function hostOf(url) {
  try { return new URL(url).hostname.toLowerCase(); } catch { return ""; }
}
function blocked(url) {
  const h = hostOf(url);
  return h !== "" && BLOCKED.some((r) => r.test(h));
}
async function gotvGroup() {
  const gs = await chrome.tabGroups.query({ title: GROUP_TITLE });
  return gs.length ? gs[0] : null;
}
async function inGroup(tabId) {
  const g = await gotvGroup();
  if (!g) throw new Error("no hay grupo «GOTV»: crea una pestaña con new_tab o arrastra una pestaña a ese grupo");
  const t = await chrome.tabs.get(tabId);
  if (t.groupId !== g.id) throw new Error("esa pestaña NO está en el grupo «GOTV»: GOD OF THE VOID sólo controla ese grupo");
  if (blocked(t.url || "")) throw new Error("sitio bloqueado (banco, pagos, correo o cuentas): GOD OF THE VOID no actúa ahí");
  return t;
}

// ── CDP por chrome.debugger ───────────────────────────────────────────
function cdp(tabId, method, params = {}) {
  return chrome.debugger.sendCommand({ tabId }, method, params);
}
async function attach(tabId) {
  if (attached[tabId]) return;
  await chrome.debugger.attach({ tabId }, "1.3");
  attached[tabId] = true;
  await cdp(tabId, "DOM.enable");
  await cdp(tabId, "Accessibility.enable");
  await mark(tabId);
}
chrome.debugger.onDetach.addListener((src) => { delete attached[src.tabId]; delete refs[src.tabId]; });
chrome.tabs.onRemoved.addListener((id) => { delete attached[id]; delete refs[id]; });

// borde morado: la pestaña que GOD OF THE VOID está controlando
async function mark(tabId) {
  await cdp(tabId, "Runtime.evaluate", {
    expression: `(() => { if (document.getElementById('gotv-borde')) return;
      const d = document.createElement('div'); d.id = 'gotv-borde';
      d.style.cssText = 'position:fixed;inset:0;border:3px solid #8b5cf6;box-shadow:inset 0 0 18px rgba(139,92,246,.45);pointer-events:none;z-index:2147483647;border-radius:4px';
      (document.body || document.documentElement).appendChild(d); })()`,
  }).catch(() => {});
}

const INTERACTIVE = new Set(["button", "link", "textbox", "searchbox", "combobox", "checkbox", "radio", "switch", "slider",
  "spinbutton", "menuitem", "menuitemcheckbox", "menuitemradio", "option", "tab", "listbox"]);

async function snapshot(tabId, interactiveOnly) {
  await attach(tabId);
  const { nodes } = await cdp(tabId, "Accessibility.getFullAXTree");
  const byId = new Map(nodes.map((n) => [n.nodeId, n]));
  const map = {};
  let n = 0;
  const lines = [];
  const walk = (node, depth) => {
    if (!node) return;
    const role = node.role && node.role.value;
    const name = node.name && node.name.value;
    const shown = !node.ignored && role && role !== "none" && role !== "generic" && role !== "InlineTextBox";
    const inter = INTERACTIVE.has(role);
    let d = depth;
    if (shown && (!interactiveOnly || inter) && (name || inter)) {
      let line = `${"  ".repeat(depth)}- ${role}${name ? ` "${String(name).slice(0, 120)}"` : ""}`;
      if (inter && node.backendDOMNodeId) {
        n += 1;
        map[`e${n}`] = node.backendDOMNodeId;
        line += ` [ref=e${n}]`;
      }
      const v = node.value && node.value.value;
      if (v !== undefined && v !== "") line += `: ${String(v).slice(0, 80)}`;
      lines.push(line);
      d = depth + 1;
    }
    for (const c of node.childIds || []) walk(byId.get(c), d);
  };
  walk(nodes[0], 0);
  refs[tabId] = map;
  return lines.join("\n").slice(0, 24000);
}

function backend(tabId, ref) {
  const id = (refs[tabId] || {})[String(ref).replace(/^@/, "")];
  if (!id) throw new Error(`ref ${ref} no existe: haz snapshot primero (un ref sólo vale para su snapshot)`);
  return id;
}

async function center(tabId, backendNodeId) {
  await cdp(tabId, "DOM.scrollIntoViewIfNeeded", { backendNodeId }).catch(() => {});
  const { model } = await cdp(tabId, "DOM.getBoxModel", { backendNodeId });
  const q = model.content;
  return { x: (q[0] + q[2] + q[4] + q[6]) / 4, y: (q[1] + q[3] + q[5] + q[7]) / 4 };
}

async function isPassword(tabId, backendNodeId) {
  const { node } = await cdp(tabId, "DOM.describeNode", { backendNodeId });
  const attrs = node.attributes || [];
  for (let i = 0; i < attrs.length; i += 2) {
    if (attrs[i] === "type" && String(attrs[i + 1]).toLowerCase() === "password") return true;
    if (attrs[i] === "autocomplete" && /password|cc-/.test(String(attrs[i + 1]).toLowerCase())) return true;
  }
  return false;
}

// ── órdenes ───────────────────────────────────────────────────────────
async function handle(m) {
  switch (m.cmd) {
    case "ping":
      return "pong";
    case "tabs": {
      const g = await gotvGroup();
      if (!g) return [];
      const ts = await chrome.tabs.query({ groupId: g.id });
      return ts.map((t) => ({ tab: t.id, title: t.title, url: t.url, active: t.active, blocked: blocked(t.url || "") }));
    }
    case "new_tab": {
      if (m.url && blocked(m.url)) throw new Error("sitio bloqueado (banco, pagos, correo o cuentas)");
      const t = await chrome.tabs.create({ url: m.url || "about:blank", active: false });
      let g = await gotvGroup();
      if (g) await chrome.tabs.group({ tabIds: t.id, groupId: g.id });
      else {
        const gid = await chrome.tabs.group({ tabIds: t.id });
        await chrome.tabGroups.update(gid, { title: GROUP_TITLE, color: "purple" });
      }
      return { tab: t.id };
    }
    case "navigate": {
      await inGroup(m.tab);
      if (blocked(m.url)) throw new Error("sitio bloqueado (banco, pagos, correo o cuentas)");
      await chrome.tabs.update(m.tab, { url: m.url });
      delete refs[m.tab];
      return "ok";
    }
    case "snapshot":
      await inGroup(m.tab);
      return await snapshot(m.tab, m.interactive !== false);
    case "text": {
      await inGroup(m.tab);
      await attach(m.tab);
      const r = await cdp(m.tab, "Runtime.evaluate", { expression: "document.body ? document.body.innerText : ''", returnByValue: true });
      return String(r.result.value || "").slice(0, 24000);
    }
    case "click": {
      await inGroup(m.tab);
      await attach(m.tab);
      const p = await center(m.tab, backend(m.tab, m.ref));
      for (const type of ["mouseMoved", "mousePressed", "mouseReleased"]) {
        await cdp(m.tab, "Input.dispatchMouseEvent", { type, x: p.x, y: p.y, button: "left", clickCount: type === "mouseMoved" ? 0 : 1 });
      }
      return "ok";
    }
    case "type": {
      await inGroup(m.tab);
      await attach(m.tab);
      const id = backend(m.tab, m.ref);
      if (await isPassword(m.tab, id)) throw new Error("es un campo de contraseña o de tarjeta: GOD OF THE VOID jamás escribe ahí (hazlo tú)");
      await cdp(m.tab, "DOM.focus", { backendNodeId: id });
      await cdp(m.tab, "Input.insertText", { text: String(m.text || "") });
      return "ok";
    }
    case "press": {
      await inGroup(m.tab);
      await attach(m.tab);
      const key = String(m.key || "Enter");
      const codes = { Enter: 13, Tab: 9, Escape: 27, Backspace: 8 };
      for (const type of ["keyDown", "keyUp"]) {
        await cdp(m.tab, "Input.dispatchKeyEvent", { type, key, windowsVirtualKeyCode: codes[key] || 0, text: key === "Enter" && type === "keyDown" ? "\r" : undefined });
      }
      return "ok";
    }
    case "screenshot": {
      await inGroup(m.tab);
      await attach(m.tab);
      const r = await cdp(m.tab, "Page.captureScreenshot", { format: "png" });
      return { png_base64: r.data };
    }
    default:
      throw new Error(`orden desconocida: ${m.cmd}`);
  }
}
