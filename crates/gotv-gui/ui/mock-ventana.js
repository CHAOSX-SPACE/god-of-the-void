// Simulador de las órdenes de F8 (la ventana) SÓLO para verla en un navegador: envuelve el `invoke` de
// mock.js. La app real nunca carga este archivo; lo de verdad está en crates/gotv-gui/src/ventana.rs.
// Casos: `?git` (repositorio: vistas sin confirmar y rama), `?conflicto` (revertir pide forzar), `?vacio`
// (proyecto sin chats ni cambios).
(() => {
  const base = window.__TAURI__.core.invoke;
  const emit = (p) => window.__mockEmit(p);
  const Q = location.search;
  const ahora = Math.floor(Date.now() / 1000);
  const H = (pares) => pares.map(([r, t], i) => ({ role: r, text: t, i: i + 1 }));
  // los chats de cada proyecto (como .gotv/sesiones/)
  const chats = {
    "/Users/demo/projects/my-app": Q.includes("vacio") ? [] : [
      { id: 7, title: "agrega la validación del login", created: "2026-11-17 10:12:00", parent: null, historia: H([["user", "agrega la validación del login"], ["assistant", "Listo: `src/auth.rs` valida el correo y la clave; añadí `nuevo.md` y borré `viejo.txt`. Las pruebas pasan."], ["user", "¿y la canción del pedido?"], ["assistant", "La canción del pedido tiene tres estrofas."]]) },
      { id: 6, title: "revisa el módulo de facturación", created: "2026-11-16 18:40:00", parent: null, historia: H([["user", "revisa el módulo de facturación"], ["assistant", "Revisado: los impuestos se redondean al final."]]) },
      { id: 5, title: "↳ revisa el módulo de facturación", created: "2026-11-16 19:02:00", parent: 6, historia: H([["user", "revisa el módulo de facturación"]]) },
      ...Array.from({ length: 34 }, (_, k) => ({ id: 4 - k, title: `chat de prueba ${k + 1}`, created: "2026-11-01 09:00:00", parent: null, historia: H([["user", `chat de prueba ${k + 1}`]]) })),
    ],
    "/Users/demo/projects/website": [{ id: 3, title: "plan de la academia", created: "2026-11-15 12:00:00", parent: null, historia: H([["user", "plan de la academia"], ["assistant", "Tres módulos: base, práctica y examen."]]) }],
    "/Users/demo/projects/radar": [],
  };
  const papelera = [];
  let actual = Q.includes("vacio") ? null : 7;
  const sueltas = [];
  const comentarios = [];
  const tareas = [
    { id: "a1", nombre: "Resumen diario", cron: "0 9 * * 1-5", proyecto: "/Users/demo/projects/my-app", orden: "resume los commits de ayer", activa: true, ultima: "2026-11-17 09:00", proxima: "2026-11-18 09:00", estado: "ok" },
    { id: "b2", nombre: "Limpiar descargas", cron: "0 18 * * 5", proyecto: "/Users/demo/projects/website", orden: "borra lo viejo de dist/", activa: false, ultima: null, proxima: null, estado: null },
  ];
  let sitios = "abrir permitir docs.rs\nabrir negar *.casino.example\ndescargar negar *";
  const avisos = { turno: true, permiso: true, programadas: false };
  // F8.4 · `pick_folder` del simulador da siempre /Users/demo/projects/notes: la segunda vez choca de nombre
  const carpetas = [{ nombre: "api", ruta: "/Users/demo/projects/api" }];
  const raiz = () => window.__mockRaiz || "/Users/demo/projects/my-app";
  const lista = (p) => (chats[p] = chats[p] || []);
  const sinDatos = ({ historia, ...c }) => c;
  const abrirChat = (id) => {
    const c = lista(raiz()).find((x) => x.id === id);
    actual = c ? c.id : null;
    emit({ kind: "status", status: { chat: actual } });
    setTimeout(() => { emit({ kind: "project", root: raiz(), history: c ? c.historia : [], chat: actual }); emit({ kind: "done", text: "", silent: true }); }, 30);
  };

  // un turno que tocó 3 archivos SIN git (como cambios.rs::por_turno)
  const TURNO2 = [
    { ruta: "src/auth.rs", estado: "cambiado", mas: 3, menos: 1, binario: false, generado: false, credencial: false, por_comando: false, recortado: null,
      trozos: [{ desde_viejo: 10, desde_nuevo: 10, lineas: [[" ", "pub fn entrar(u: &str, c: &str) -> Result<Sesion> {"], ["-", "    let ok = true;"], ["+", "    validar_correo(u)?;"], ["+", "    validar_clave(c)?;"], ["+", "    let ok = verificar(u, c)?;"], [" ", "    Ok(Sesion::nueva(u, ok))"], [" ", "}"]] }] },
    { ruta: "nuevo.md", estado: "nuevo", mas: 2, menos: 0, binario: false, generado: false, credencial: false, por_comando: false, recortado: null, trozos: [{ desde_viejo: 1, desde_nuevo: 1, lineas: [["+", "# Validación"], ["+", "El correo y la clave se validan antes de entrar."]] }] },
    { ruta: "viejo.txt", estado: "borrado", mas: 0, menos: 1, binario: false, generado: false, credencial: false, por_comando: true, recortado: null, trozos: [{ desde_viejo: 1, desde_nuevo: 1, lineas: [["-", "adiós"]] }] },
  ];
  const TURNO1 = [
    { ruta: ".env", estado: "cambiado", mas: 1, menos: 0, binario: false, generado: false, credencial: true, por_comando: false, recortado: null, trozos: [] },
    { ruta: "logo.png", estado: "nuevo", mas: 0, menos: 0, binario: true, generado: false, credencial: false, por_comando: true, recortado: null, trozos: [] },
    { ruta: "datos/largo.csv", estado: "nuevo", mas: 5000, menos: 0, binario: false, generado: false, credencial: false, por_comando: false, recortado: 5000, trozos: [{ desde_viejo: 1, desde_nuevo: 1, lineas: Array.from({ length: 400 }, (_, i) => ["+", `fila ${i + 1};valor ${i * 3}`]) }] },
  ];
  let conflictoVisto = false;
  const ORD = [["nuevo_chat", "oNuevoChat", "chat nuevo en este proyecto new chat in this project"], ["nueva_charla", "oNuevaCharla", "charla sin proyecto chat without a project"], ["nuevo_proyecto", "newProject", "nuevo proyecto new project"], ["ajustes", "settings", "configuracion settings"], ["panel_tareas", "oPanelTareas", "ver las tareas show the tasks"], ["panel_cambios", "oPanelCambios", "ver los cambios show the changes"], ["panel_vista", "oPanelVista", "ver la vista del servidor show the server view"], ["papelera", "oPapelera", "papelera de chats chat trash"]];
  const sinTilde = (s) => String(s).normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();
  const ARCHIVOS = [["src/", "carpeta"], ["src/auth.rs", "archivo"], ["src/main.rs", "archivo"], ["src/util/cadena.rs", "archivo"], ["nuevo.md", "archivo"], [".github/workflows/ci.yml", "archivo"]];

  window.__TAURI__.core.invoke = async (cmd, a = {}) => {
    switch (cmd) {
      case "info": { const i = await base(cmd, a); return { ...i, chat: actual, sueltas: "/Users/demo/Library/Application Support/gotv/sueltas" }; }
      case "ventana_estructura": {
        const h = (id, i18n, fase = "—") => ({ id, i18n, fase });
        return {
          zonas: [h("proyectos", "zonaProyectos"), h("chat", "zonaChat"), h("trabajo", "zonaTrabajo")],
          panel: [h("tareas", "pTareas"), h("cambios", "pCambios"), h("vista", "pVista"), h("archivo", "pArchivo"), h("terminal", "pTerminal")],
          ajustes: [["general", "sGeneral"], ["model", "sModel"], ["library", "sLibrary"], ["gen", "sGen"], ["context", "sContext"], ["prov", "sProv"], ["pasos", "sSteps"], ["perms", "sPerms"], ["seguridad", "sSecurity"], ["conex", "sConex"], ["automatizaciones", "sAutomat"], ["api", "sApi"]].map(([i, k]) => h(i, k)),
          secciones: { conex: [h("ssh", "cxSsh"), h("bases", "sBases"), h("conectores", "cxMcp"), h("buscadores", "sBuscadores"), h("dispositivos", "cxDispositivos"), h("remoto", "cxRemoto")], seguridad: [h("caja", "sgCaja"), h("credenciales", "sgCredenciales"), h("auto", "sgAuto"), h("sitios", "sgSitios"), h("variables", "sgVariables"), h("chrome", "sgChrome"), h("mac", "sgMac"), h("sistema", "sgSistema")], general: [h("aspecto", "gAspecto"), h("avisos", "gAvisos"), h("agentes", "gAgentes")] },
          // la barra de Configuración: copia de `GRUPOS` (ventana.rs) para verla sin la app
          grupos: [
            ["general", "grpGeneral", [["general", "sGeneral", ["general/aspecto", "general/avisos"]]]],
            ["modelos", "grpModelos", [["model", "sModel", ["model"]], ["context", "sContext", ["context"]], ["library", "sLibrary", ["library"]], ["gen", "sGen", ["gen"]], ["prov", "sProv", ["prov"]]]],
            ["agente", "grpAgente", [["perms", "pgConducta", ["perms", "pasos"]], ["general/agentes", "gAgentes", ["general/agentes"]], ["automatizaciones", "sAutomat", ["automatizaciones"]]]],
            ["conex", "sConex", ["conectores", "buscadores", "bases", "ssh", "remoto", "dispositivos"].map((s) => [`conex/${s}`, { conectores: "cxMcp", buscadores: "sBuscadores", bases: "sBases", ssh: "cxSsh", remoto: "cxRemoto", dispositivos: "cxDispositivos" }[s], [`conex/${s}`]])],
            ["seguridad", "sSecurity", [["seguridad", "sgCaja", ["seguridad/caja"]], ["seguridad/variables", "sgVariables", ["seguridad/variables"]], ["seguridad/credenciales", "sgCredenciales", ["seguridad/credenciales"]],
              ["seguridad/navegador", "sgNavegador", ["seguridad/chrome", "seguridad/sitios"]], ["seguridad/mac", "sgMac", ["seguridad/mac"]], ["seguridad/auto", "sgAuto", ["seguridad/auto"]], ["seguridad/sistema", "sgSistema", ["seguridad/sistema"]]]],
            ["avanzado", "grpAvanzado", [["api", "sApi", ["api"]]]],
          ].map(([id, i18n, ps]) => ({ id, i18n, paginas: ps.map(([id, i18n, piezas]) => ({ id, i18n, piezas })) })),
          // y de `FILAS` (los ajustes que encuentra el buscador): página, id, nombre, explicación
          filas: [
            ["general", "idioma", "lang", "langNota"], ["general", "tema", "theme", "themeNota"], ["general", "aviso-turno", "avTurno", "avAyuda"], ["general", "aviso-permiso", "avPermiso", "avAyuda"], ["general", "aviso-programadas", "avProgramadas", "avAyuda"],
            ["model", "modelo", "flModelo", "flModeloNota"], ["context", "contexto", "ctxPreset", "ctxNote"], ["context", "max-tokens", "maxTok", "flMaxTokNota"], ["context", "temperatura", "temp", "tempNote"], ["context", "esfuerzo", "effort", "effortNote"],
            ["library", "lib-mac", "libMeasure", "flLibMacNota"], ["library", "lib-filtros", "flLibFiltros", "flLibFiltrosNota"], ["gen", "hf-token", "genTokTitle", "genTokNone"], ["prov", "prov-nuevo", "provAdd", "flProvNuevoNota"],
            ["perms", "modo", "defMode", "flModoNota"], ["perms", "pasos", "flPasos", "flPasosNota"], ["perms", "subagentes", "stepsParalelo", "stepsParaleloA"],
            ["conex/conectores", "mcp-nuevo", "mcNuevo", "flMcpNuevoNota"], ["conex/buscadores", "bs-paralelo", "bsParalelo", ""], ["conex/buscadores", "bs-local", "bsSoloLocal", ""], ["conex/buscadores", "bs-contacto", "bsContacto", "flBsContactoNota"],
            ["conex/buscadores", "bs-indice", "bsIndice", "bsSemillas"], ["conex/buscadores", "bs-borrar", "flBsBorrar", "flBsBorrarNota"], ["conex/bases", "bd-nueva", "bdNueva", "flBdNuevaNota"], ["conex/ssh", "ssh-nueva", "cxNueva", "flSshNuevaNota"],
            ["conex/remoto", "remoto-crear", "remTitulo", "remNota"], ["conex/dispositivos", "android", "dSdk", "flAndroidNota"], ["conex/dispositivos", "xcode", "dXcode", "flXcodeNota"], ["conex/dispositivos", "simuladores", "dSims", "dBajarIos"], ["conex/dispositivos", "idb", "flIdb", "dIdb"],
            ["seguridad", "caja", "sbTitle", "flCajaNota"], ["seguridad", "caja-dominios", "flCajaDominios", "sbExtra"], ["seguridad/variables", "var-nueva", "flVarNueva", "varNota"], ["seguridad/navegador", "chrome", "flChromeFila", "flChromeNota"], ["seguridad/navegador", "sitios", "sgSitios", "sitAyuda"],
            ["seguridad/mac", "mac-nativo", "dkTitle", "flMacNota"], ["seguridad/mac", "mac-capturas", "computerOld", "computerNote"], ["seguridad/sistema", "sistema", "secSysTitle", "flSistemaNota"],
            ["api", "api-activar", "apiEnable", "apiEnableNote"], ["api", "api-red", "apiLan", "apiLanNote"], ["api", "api-nube", "apiCloud", "apiCloudNote"], ["api", "api-puerto", "apiPort", "flApiPuertoNota"], ["api", "api-llave", "apiKey", "apiKeyNote"], ["api", "telefono", "telTitle", "flTelNota"],
          ].map(([pagina, id, i18n, nota]) => ({ pagina, id, i18n, nota })),
          ordenes: ORD.map(([id, k]) => h(id, k)),
        };
      }
      // F8.1
      case "chats_abiertos": return [...(window.__abiertos || [])];
      case "chats_abiertos_guardar": window.__abiertos = a.rutas; return;
      case "chats_lista": {
        const p = a.proyecto || raiz(), d = a.desde || 0, v = lista(p);
        return { chats: v.slice(d, d + 30).map(sinDatos), total: v.length, actual: p === raiz() ? actual : null, papelera: papelera.length, pagina: 30, suelta: p.includes("/gotv/sueltas/") };
      }
      case "chat_abrir":
        if (a.proyecto && a.proyecto !== raiz()) { window.__mockRaiz = a.proyecto; await base("new_project", { path: a.proyecto, name: "" }).catch(() => {}); }
        if (a.id == null) { const n = Math.max(0, ...Object.values(chats).flat().map((c) => c.id), ...papelera.map((c) => c.id)) + 1; lista(raiz()).unshift({ id: n, title: "", created: "2026-11-17 16:00:00", parent: null, historia: [] }); abrirChat(n); }
        else abrirChat(a.id);
        return;
      case "chat_renombrar": { const c = lista(a.proyecto || raiz()).find((x) => x.id === a.id); if (!a.titulo.trim()) throw "el título no puede quedar vacío"; c.title = a.titulo.trim().replace(/\s+/g, " "); return; }
      case "chat_bifurcar": {
        const c = lista(raiz()).find((x) => x.id === a.id);
        const n = Math.max(...Object.values(chats).flat().map((x) => x.id)) + 1;
        lista(raiz()).unshift({ id: n, title: "↳ " + c.title, created: "2026-11-17 16:05:00", parent: c.id, historia: c.historia.filter((x) => x.i <= a.hasta) });
        window.__mockBifurcado = { de: a.id, hasta: a.hasta, nuevo: n };
        abrirChat(n);
        return `chat #${n} creado desde el mensaje ${a.hasta}: su primera orden relee el historial (la memoria rápida es de cada chat)`;
      }
      case "chat_exportar": window.__mockExportado = a; return `/Users/demo/Downloads/chat-${a.id}.${a.formato}`;
      case "chat_papelera": { const v = lista(a.proyecto || raiz()), i = v.findIndex((x) => x.id === a.id); const [c] = v.splice(i, 1); papelera.unshift({ ...c, deleted_at: ahora }); if (a.id === actual) await window.__TAURI__.core.invoke("chat_abrir", { id: null }); return; }
      case "chat_restaurar": { const i = papelera.findIndex((x) => x.id === a.id); const [c] = papelera.splice(i, 1); lista(raiz()).push(c); return; }
      case "papelera_lista": return { chats: papelera.map(sinDatos), dias: 30 };
      case "papelera_vaciar": return 0;
      // F8.2
      case "cambios_turnos": {
        if (Q.includes("vacio")) return { turnos: [], git: Q.includes("git") };
        const t = [{ n: 2, sesion: 7, desde: 3, hasta: 6, orden: "agrega la validación del login", fecha: "2026-11-17 10:14", archivos: 3, sin_anotar: false }];
        if (a.todos) t.push({ n: 1, sesion: 6, desde: null, hasta: 3, orden: "sube el logo y los datos", fecha: "2026-11-16 18:44", archivos: 3, sin_anotar: false });
        if (Q.includes("encurso")) t.unshift({ n: 1000007, sesion: null, desde: 6, hasta: 7, orden: "", fecha: "", archivos: 1, sin_anotar: true });
        return { turnos: t, git: Q.includes("git") };
      }
      case "cambios_turno": return a.n === 2 || a.n === 1000007 ? TURNO2 : TURNO1;
      case "cambios_confirmar": if (!String(a.mensaje).trim()) throw "escribe el mensaje del commit"; if (!a.rutas.length) throw "marca al menos un archivo"; return `[main 1a2b3c4] ${a.mensaje.trim()}`;
      case "cambios_git": if (!Q.includes("git")) throw "no está dentro de un repositorio git"; return a.vista === "rama" ? TURNO2 : TURNO2.slice(0, 2);
      case "cambios_revertir":
        if (Q.includes("conflicto") && !a.forzar && !conflictoVisto) { conflictoVisto = true; throw `${a.ruta} cambió DESPUÉS del turno ${a.n}: no lo piso (revertir de todos modos lo forzaría)`; }
        window.__mockRevertido = a;
        return `↩ ${a.ruta} volvió a como estaba antes del turno ${a.n} (paso #9: se deshace con «deshacer»)`;
      case "cambios_comentar": if (!a.texto.trim()) throw "el comentario está vacío"; comentarios.push({ ruta: a.ruta, linea: a.linea, texto: a.texto.trim(), como_viaja: `[Comentario en ${a.ruta} L${a.linea}] ${a.texto.trim()}` }); return comentarios.slice();
      case "cambios_comentarios": if (a.quitar != null) comentarios.splice(a.quitar, 1); return comentarios.slice();
      case "send": {
        // como ventana::preparar_envio: los comentarios viajan delante y se vacían
        if (comentarios.length) {
          window.__mockEnviadoConComentarios = comentarios.map((c) => c.como_viaja).join("\n") + "\n\n" + a.task;
          const n = comentarios.length; comentarios.length = 0;
          setTimeout(() => emit({ kind: "comentarios", lista: [], enviados: n }), 20);
        }
        return base(cmd, a);
      }
      // F8.4
      case "carpetas_lista": return carpetas.slice();
      case "carpetas_agregar": {
        const n = (a.nombre || a.ruta.split("/").pop()).trim();
        if (a.ruta.startsWith(raiz())) throw `${a.ruta} ya está dentro del proyecto`;
        if (carpetas.some((c) => c.nombre === n)) throw `ya hay una carpeta «${n}»: dale otro nombre`;
        carpetas.push({ nombre: n, ruta: a.ruta });
        return carpetas.slice();
      }
      case "carpetas_quitar": { const i = carpetas.findIndex((c) => c.nombre === a.nombre); if (i < 0) throw `no hay una carpeta «${a.nombre}»`; carpetas.splice(i, 1); return carpetas.slice(); }
      // F8.3
      case "menciones": {
        const q = sinTilde(a.consulta || "");
        const v = [];
        v.push(...[[".gotv/reglas/sql.md", "sql"]].filter(([r]) => sinTilde(r).includes(q)).map(([r, e]) => ({ clase: "regla", etiqueta: e, ficha: r, detalle: ".gotv/reglas" })));
        v.push(...lista(raiz()).filter((c) => !q || sinTilde(c.title).includes(q)).slice(0, 5).map((c) => ({ clase: "chat", etiqueta: c.title || "#" + c.id, ficha: "chat#" + c.id, detalle: c.created })));
        v.push(...[["docs", "mem://nota", "nota"]].filter((x) => !q || x.some((y) => sinTilde(y).includes(q))).map(([s, u, n]) => ({ clase: "recurso", etiqueta: n, ficha: `mcp:${s}:${u}`, detalle: s })));
        v.push(...ARCHIVOS.filter(([f]) => !q || sinTilde(f).includes(q)).map(([f, c]) => ({ clase: c, etiqueta: f.replace(/\/$/, "").split("/").pop() + (c === "carpeta" ? "/" : ""), ficha: f, detalle: f.includes("/") ? f.replace(/\/?[^/]*\/?$/, "") : "" })));
        return v;
      }
      case "citar": { const c = a.texto.trimEnd().split("\n").map((l) => (l ? "> " + l : ">")).join("\n"); return a.origen ? `De ${a.origen}:\n${c}\n\n` : `${c}\n\n`; }
      // F8.6
      case "buscar_todo": {
        const q = sinTilde(a.consulta || "").trim();
        const ordenes = ORD.filter(([, , t]) => q.split(/\s+/).every((p) => sinTilde(t).includes(p))).map(([id, i18n]) => ({ id, i18n, fase: "" }));
        const hits = [];
        if (q.length >= 2) for (const [p, v] of Object.entries(chats)) for (const c of v) for (const m of c.historia) {
          const t = sinTilde(m.text);
          if (q.split(/\s+/).every((w) => t.includes(w))) { const k = t.indexOf(q.split(/\s+/)[0]); const w = q.split(/\s+/)[0].length; hits.push({ proyecto: p.split("/").pop(), raiz: p, chat: c.id, titulo: c.title, mensaje: m.i, rol: m.role, fragmento: m.text.slice(0, k) + "»" + m.text.slice(k, k + w) + "«" + m.text.slice(k + w, k + w + 60), fecha: c.created }); }
        }
        const archivos = q ? ARCHIVOS.filter(([f]) => sinTilde(f).includes(q)).map(([f, c]) => ({ clase: c, etiqueta: f.split("/").filter(Boolean).pop(), ficha: f, detalle: "" })) : [];
        return { chats: hits.slice(0, 50), archivos, ordenes, proyecto: raiz() };
      }
      // F8.5
      case "suelta_nueva": { const p = `/Users/demo/Library/Application Support/gotv/sueltas/20261117-16${String(sueltas.length).padStart(2, "0")}00`; sueltas.unshift({ ruta: p, titulo: "¿qué hora es en Tokio?", creada: "2026-11-17 16:00:00", mensajes: 2 }); window.__mockRaiz = p; chats[p] = []; actual = null; setTimeout(() => { emit({ kind: "project", root: p, history: [] }); emit({ kind: "done", text: "", silent: true }); }, 30); return p; }
      case "sueltas_lista": return sueltas.slice();
      case "suelta_abrir": window.__mockRaiz = a.ruta; setTimeout(() => emit({ kind: "project", root: a.ruta, history: [] }), 30); return;
      // como el worker de verdad: abrir avisa «proyecto» y luego «listo» (sin esto la vista se quedaba ocupada)
      case "open_project": window.__mockRaiz = a.path; setTimeout(() => { emit({ kind: "project", root: a.path, history: [] }); emit({ kind: "done", text: "", silent: true }); }, 30); return;
      // F8.7
      case "vista_url": { const m = /^https?:\/\/(localhost|127\.0\.0\.1|\[::1\]|0\.0\.0\.0)(:\d+)?(\/|$|\?)/i.exec(String(a.url).trim()); if (!m || /@/.test(a.url.split("/")[2])) throw `«Vista» sólo abre direcciones de esta Mac (localhost): ${a.url}`; return a.url.trim().replace("://0.0.0.0", "://127.0.0.1").replace("://[::1]", "://localhost"); }
      case "vista_pedir_revision": return `Revisa ${a.url} con el navegador (browser): ábrela, mira la consola y dime qué ves y si algo falla.`;
      // Configuración
      case "automatizaciones_lista": return { tareas: tareas.map((t) => ({ ...t })), agente: false };
      case "automatizacion_activar": tareas.find((t) => t.id === a.id).activa = a.activa; return;
      case "automatizacion_quitar": tareas.splice(tareas.findIndex((t) => t.id === a.id), 1); return;
      case "sitios_texto": return sitios;
      case "sitios_guardar": for (const l of a.texto.split("\n").map((x) => x.split("#")[0].trim()).filter(Boolean)) if (!/^(abrir|descargar) (permitir|negar) \S+$/.test(l)) throw `línea que no se entiende: «${l}» (no se guardó nada)`; sitios = a.texto; return;
      case "avisos_estado": return { ...avisos };
      case "avisos_guardar": Object.assign(avisos, a); return;
    }
    return base(cmd, a);
  };
  // `?f8`: se abre con el chat 7 pintado (cada mensaje con su lugar en el historial, para bifurcar)
  if (Q.includes("f8") && actual != null) setTimeout(() => abrirChat(actual), 250);
})();
