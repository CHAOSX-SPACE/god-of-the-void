// F18 · M9 · Configuración → «Dispositivos» (F18.I). Sólo PINTA lo que entrega Rust (`devices_state`) y manda
// los clics a sus órdenes (`devices_*`). Toda la lógica (qué falta, qué licencia, la suma, dónde se instala, qué
// se puede desinstalar) vive en `gotv_core::movil::{instalador, xcode}`. El texto de cada licencia viene del
// catálogo oficial y se muestra ENTERO; «Acepto» acepta UNA.
Object.assign(I18N, {
  sDisp: ["Dispositivos", "Devices"],
  dAndroid: ["Android", "Android"],
  dSdk: ["SDK de Android", "Android SDK"],
  dNoSdk: ["no instalado", "not installed"],
  dDest: ["Se instala en", "Installs into"],
  dPub: ["Publica", "Publisher"],
  dFrom: ["Origen", "Source"],
  dNada: ["No falta nada: el SDK que hay ya trae todo lo necesario.", "Nothing is missing: the existing SDK already has everything needed."],
  dTotal: ["Total a bajar", "Total download"],
  dLic: ["Licencia", "Licence"],
  dReq: ["la exigen", "required by"],
  dAcepto: ["Acepto esta licencia", "I accept this licence"],
  dAceptada: ["aceptada", "accepted"],
  dInstalar: ["Instalar", "Install"],
  dFaltaLic: ["primero acepta su licencia", "accept its licence first"],
  dInstalados: ["Instalado por GOTV", "Installed by GOTV"],
  dDesinstalar: ["Desinstalar", "Uninstall"],
  dAvd: ["Dispositivos virtuales", "Virtual devices"],
  dNinguno: ["ninguno", "none"],
  dXcode: ["Xcode", "Xcode"],
  dNoXcode: ["Xcode no está instalado. Apple lo entrega desde la Mac App Store con tu cuenta de Apple: GOTV nunca la pide ni la toca.", "Xcode is not installed. Apple delivers it from the Mac App Store with your Apple account: GOTV never asks for it or touches it."],
  dTienda: ["Abrir la Mac App Store", "Open the Mac App Store"],
  dPrep: ["Preparación", "Preparation"],
  dPrepOk: ["licencia aceptada y primer arranque hecho", "licence accepted and first launch done"],
  dPrepFalta: ["Falta aceptar el acuerdo de Xcode y los SDK de Apple y hacer su primer arranque (pide la contraseña de administrador de macOS; GOTV no la ve).", "The Xcode and Apple SDKs agreement must be accepted and its first launch done (asks for the macOS administrator password; GOTV never sees it)."],
  dLeer: ["Leer el acuerdo", "Read the agreement"],
  dPreparar: ["Acepto y preparar Xcode", "I accept, prepare Xcode"],
  dSims: ["Simuladores", "Simulators"],
  dBajarIos: ["Bajar el simulador de iOS (de Apple, varios GB)", "Download the iOS simulator (from Apple, several GB)"],
  dIdb: ["idb (Meta, licencia MIT) para leer y tocar el simulador", "idb (Meta, MIT licence) to read and tap the simulator"],
  dIdbInst: ["Instalar idb", "Install idb"],
  dBajando: ["bajando", "downloading"],
});

const fmtMB = (b) => (b >= 1e9 ? (b / 1e9).toFixed(2) + " GB" : Math.max(1, Math.round(b / 1e6)) + " MB");
let DISP_POLL = null;

async function drawDispositivos(pane) {
  clearTimeout(DISP_POLL);
  let D;
  try { D = await invoke("devices_state"); } catch (e) { pane.innerHTML = setError(e); return; }
  if (!document.body.contains(pane) || !$("settingsModal").classList.contains("show")) return;
  const A = D.android, X = D.xcode, prog = D.progreso || [];
  const enCurso = (p) => prog.find((x) => x.paquete === p);
  const aceptada = (id) => !!(A && A.licencias.find((l) => l.id === id && l.aceptada));
  const bajando = (p) => (p.error ? setEstado(p.error, "bad") : setEstado(`${tt("dBajando")} ${p.total ? Math.round((100 * p.bajado) / p.total) : 0} %`));
  // pulido: todo en filas (qué es y su estado a la izquierda; el botón a la derecha). Primero el SDK y lo que
  // falta (licencias antes que paquetes: sin aceptar no se instala), los dispositivos virtuales y, al final
  // de Android, lo que se puede desinstalar. Luego Xcode, su preparación, los simuladores e idb.
  let h = `<h4>${tt("dAndroid")}</h4>`;
  if (!A) h += setError(D.android_error || "");
  else {
    h += setFila("android", tt("dSdk"), `${tt("dDest")} <code>${esc(A.destino)}</code> · ${tt("dPub")}: ${esc(A.publica)}`, A.sdk ? `<code>${esc(A.sdk)}</code>` : setEstado(tt("dNoSdk"), "warn"));
    if (!A.fichas.length) h += `<p class="note">${tt("dNada")}</p>`;
    else {
      h += setFila("", tt("dTotal"), "", `<span class="fila-valor">${fmtMB(A.total)}</span>`);
      for (const l of A.licencias) {
        h += setFila("", `${tt("dLic")}: ${esc(l.titulo)}`, `${esc(l.id)} · ${tt("dReq")} ${esc(l.paquetes.join(", "))}<details><summary>${tt("dLeer")}</summary><pre class="lic">${esc(l.texto)}</pre></details>`,
          l.aceptada ? setEstado(tt("dAceptada"), "ok") : `<button class="btn" data-acepta="${esc(l.id)}">${tt("dAcepto")}</button>`, "item");
      }
      for (const f of A.fichas) {
        const p = enCurso(f.ruta);
        h += setFila("", `${esc(f.nombre)} ${esc(f.version)} · ${fmtMB(f.tamano)}`, `${tt("dFrom")}: <code>${esc(f.url)}</code><br>${tt("dDest")}: <code>${esc(f.destino)}</code>`,
          p && !p.error ? bajando(p) : aceptada(f.licencia) ? `${p ? bajando(p) : ""}<button class="btn primary" data-instala="${esc(f.ruta)}">${tt("dInstalar")}</button>` : setEstado(tt("dFaltaLic")), "item");
      }
    }
    h += setFila("", tt("dAvd"), A.dispositivos.length ? A.dispositivos.map(esc).join(", ") : tt("dNinguno"), "");
    if (A.instalados.length) h += `<h4>${tt("dInstalados")}</h4>` + A.instalados.map((p) => setFila("", `<code>${esc(p)}</code>`, "", `<button class="btn danger" data-quita="${esc(p)}">${tt("dDesinstalar")}</button>`, "item")).join("");
  }
  h += `<h4>${tt("dXcode")}</h4>`;
  if (!X.version) h += setFila("xcode", tt("dXcode"), tt("dNoXcode"), `<button class="btn primary" id="dTienda">${tt("dTienda")}</button>`);
  else {
    h += setFila("xcode", tt("dXcode"), `<code>${esc(X.carpeta || "")}</code>`, `<span class="fila-valor">${esc(X.version)}</span>`);
    const P = X.preparacion;
    if (P && P.licencia_aceptada && P.primer_arranque_hecho) h += setFila("", tt("dPrep"), "", setEstado(tt("dPrepOk"), "ok"));
    else h += setFila("", tt("dPrep"), tt("dPrepFalta"), `<button class="btn" id="dLeerX">${tt("dLeer")}</button><button class="btn primary" id="dPrepX">${tt("dPreparar")}</button>`);
    const ios = enCurso("ios-platform");
    h += setFila("simuladores", tt("dSims"), X.simuladores.length ? X.simuladores.map(esc).join(", ") : tt("dNinguno"), ios ? (ios.error ? setEstado(ios.error, "bad") : setEstado(tt("dBajando") + "…")) : `<button class="btn" id="dIos">${tt("dBajarIos")}</button>`);
    h += setFila("idb", tt("flIdb"), `${tt("dIdb")} · <a href="#" id="dIdbLic">MIT</a>`, X.idb ? setEstado(X.idb, "ok") : `<button class="btn" id="dIdb">${tt("dIdbInst")} ${esc(X.idb_fijado)} (${fmtMB(X.idb_tamano)})</button>`);
  }
  pane.innerHTML = h;
  const act = (sel, f) => pane.querySelectorAll(sel).forEach((b) => (b.onclick = async () => { b.disabled = true; try { await f(b); } catch (e) { toast(String(e), 7000); } drawDispositivos(pane); }));
  act("[data-acepta]", (b) => invoke("devices_accept", { id: b.dataset.acepta }));
  act("[data-instala]", (b) => invoke("devices_install", { paquete: b.dataset.instala }));
  act("[data-quita]", (b) => invoke("devices_uninstall", { paquete: b.dataset.quita }));
  act("#dTienda", () => invoke("devices_xcode_store"));
  act("#dLeerX", () => invoke("devices_xcode_license"));
  act("#dPrepX", async () => toast(await invoke("devices_xcode_prepare"), 6000));
  act("#dIos", () => invoke("devices_xcode_platform"));
  act("#dIdb", async () => toast(String(await invoke("devices_idb_install")), 6000));
  const lic = $("dIdbLic");
  if (lic) lic.onclick = (e) => { e.preventDefault(); invoke("open_link", { url: X.idb_licencia }); };
  if (prog.some((p) => !p.error)) DISP_POLL = setTimeout(() => drawDispositivos(pane), 1500);
}

// F8.0: Dispositivos es una sección de Configuración → Conexiones (el hueco que F8 declaró para F18)
if (window.gotvAjustes) gotvAjustes.registrar("conex/dispositivos", drawDispositivos);
