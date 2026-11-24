// Simulador del núcleo SÓLO para ver la interfaz en un navegador (fuera de Tauri no
// existe window.__TAURI__). La app real nunca carga este archivo.
(() => {
  const handlers = {};
  const GEN_CAT = [{"repo": "mlx-community/whisper-large-v3-turbo-asr-fp16", "origen": "openai/whisper-large-v3-turbo", "nombre": "Whisper large-v3-turbo", "area": "oir", "licencia": "mit", "url_licencia": "https://opensource.org/license/mit", "clase": "libre", "restringido": false, "fase": "M1", "bytes_repo": 1620000000, "sha": "624c19c9af"}, {"repo": "mistralai/Voxtral-Mini-4B-Realtime-2602", "nombre": "Voxtral Mini 4B Realtime", "area": "oir", "licencia": "apache-2.0", "url_licencia": "https://www.apache.org/licenses/LICENSE-2.0", "clase": "libre", "restringido": false, "fase": "M1", "bytes_repo": 17730000000, "sha": "2769294da9"}, {"repo": "nvidia/parakeet-tdt-0.6b-v3", "nombre": "Parakeet TDT 0.6B v3", "area": "oir", "licencia": "cc-by-4.0", "url_licencia": "https://creativecommons.org/licenses/by/4.0/", "clase": "atribucion", "restringido": false, "fase": "M1", "bytes_repo": 5730000000, "sha": "541d1f99c6"}, {"repo": "hexgrad/Kokoro-82M", "nombre": "Kokoro 82M", "area": "hablar", "licencia": "apache-2.0", "url_licencia": "https://www.apache.org/licenses/LICENSE-2.0", "clase": "libre", "restringido": false, "fase": "M2", "bytes_repo": 360000000, "sha": "f3ff357179"}, {"repo": "Qwen/Qwen3-TTS-12Hz-1.7B-Base", "nombre": "Qwen3-TTS 1.7B", "area": "hablar", "licencia": "apache-2.0", "url_licencia": "https://www.apache.org/licenses/LICENSE-2.0", "clase": "libre", "restringido": false, "fase": "M2", "bytes_repo": 4540000000, "sha": "fd4b254389"}, {"repo": "microsoft/VibeVoice-Realtime-0.5B", "nombre": "VibeVoice Realtime 0.5B", "area": "hablar", "licencia": "mit", "url_licencia": "https://opensource.org/license/mit", "clase": "libre", "restringido": false, "fase": "M2", "bytes_repo": 2040000000, "sha": "6bce5f0604"}, {"repo": "ResembleAI/chatterbox", "nombre": "Chatterbox", "area": "hablar", "licencia": "mit", "url_licencia": "https://opensource.org/license/mit", "clase": "libre", "restringido": false, "fase": "M2", "bytes_repo": 13870000000, "sha": "5bb1f6ee58"}, {"repo": "black-forest-labs/FLUX.1-schnell", "nombre": "FLUX.1 schnell", "area": "imagen", "licencia": "apache-2.0", "url_licencia": "https://www.apache.org/licenses/LICENSE-2.0", "clase": "libre", "restringido": true, "fase": "M4", "bytes_repo": 57850000000, "sha": "741f7c3ce8"}, {"repo": "black-forest-labs/FLUX.2-klein-4B", "nombre": "FLUX.2 klein 4B", "area": "imagen", "licencia": "apache-2.0", "url_licencia": "https://www.apache.org/licenses/LICENSE-2.0", "clase": "libre", "restringido": false, "fase": "M4", "bytes_repo": 23740000000, "sha": "e7b7dc27f9"}, {"repo": "Tongyi-MAI/Z-Image-Turbo", "nombre": "Z-Image Turbo", "area": "imagen", "licencia": "apache-2.0", "url_licencia": "https://www.apache.org/licenses/LICENSE-2.0", "clase": "libre", "restringido": false, "fase": "M4", "bytes_repo": 32900000000, "sha": "f332072aa7"}, {"repo": "black-forest-labs/FLUX.1-dev", "nombre": "FLUX.1 dev", "area": "imagen", "licencia": "flux-1-dev-non-commercial-license", "url_licencia": "https://huggingface.co/black-forest-labs/FLUX.1-dev/blob/main/LICENSE.md", "clase": "no_comercial", "restringido": true, "fase": "M4", "bytes_repo": 57890000000, "sha": "3de623fc3c"}, {"repo": "Wan-AI/Wan2.2-TI2V-5B-Diffusers", "nombre": "Wan 2.2 TI2V 5B", "area": "video", "licencia": "apache-2.0", "url_licencia": "https://www.apache.org/licenses/LICENSE-2.0", "clase": "libre", "restringido": false, "fase": "M6", "bytes_repo": 34200000000, "sha": "b8fff7315c"}, {"repo": "Lightricks/LTX-2.5", "nombre": "LTX 2.5", "area": "video", "licencia": "ltx-2.x-community-license-agreement", "url_licencia": "https://github.com/Lightricks/LTX-2/blob/main/LICENSE-2_x", "clase": "condiciones", "restringido": true, "fase": "M6", "bytes_repo": 200850000000, "sha": "49b093a39c"}, {"repo": "ACE-Step/Ace-Step1.5", "nombre": "ACE-Step 1.5", "area": "musica", "licencia": "mit", "url_licencia": "https://opensource.org/license/mit", "clase": "libre", "restringido": false, "fase": "M7", "bytes_repo": 10090000000, "sha": "19671f406d"}, {"repo": "OpenMOSS-Team/MOSS-SoundEffect-v2.0", "origen": "", "nombre": "MOSS-SoundEffect 2.0", "area": "musica", "licencia": "apache-2.0", "url_licencia": "https://www.apache.org/licenses/LICENSE-2.0", "clase": "libre", "restringido": false, "fase": "M7", "bytes_repo": 11230000000, "sha": "e35df4d82f"}, {"repo": "adefossez/HTDemucs", "nombre": "HTDemucs (4 pistas)", "area": "musica", "licencia": "mit", "url_licencia": "https://github.com/adefossez/demucs/blob/main/LICENSE", "clase": "libre", "restringido": false, "fase": "PRO", "bytes_repo": 84000000, "sha": "cbc8a9b1a8"}, {"repo": "adefossez/HTDemucs-6s", "nombre": "HTDemucs 6 pistas", "area": "musica", "licencia": "mit", "url_licencia": "https://github.com/adefossez/demucs/blob/main/LICENSE", "clase": "libre", "restringido": false, "fase": "PRO", "bytes_repo": 55000000, "sha": "3c5ee475be"}, {"repo": "adefossez/HTDemucs-ft", "nombre": "HTDemucs ft (afinado)", "area": "musica", "licencia": "mit", "url_licencia": "https://github.com/adefossez/demucs/blob/main/LICENSE", "clase": "libre", "restringido": false, "fase": "PRO", "bytes_repo": 336000000, "sha": "d74ac89c3a"}, {"repo": "mlx-community/Qwen3-Embedding-0.6B-4bit-DWQ", "origen": "Qwen/Qwen3-Embedding-0.6B", "nombre": "Qwen3-Embedding 0.6B", "area": "buscar", "licencia": "apache-2.0", "url_licencia": "https://www.apache.org/licenses/LICENSE-2.0", "clase": "libre", "restringido": false, "fase": "M11", "bytes_repo": 350000000, "sha": "6c3ae70858"}];
  const genAcc = {}, genHave = {}, genLeidas = {};
  // F7 · el plan del simulador y sus comentarios
  const PLAN_TXT = "1. Leer src/boton.css y la paleta de la marca\n2. Pintar el botón «Pagar» de rojo\n3. Probarlo con el navegador (captura)\n4. Correr las pruebas";
  const planComs = [];
  let genTok = false;
  let opened = false, everOpened = false;
  const emit = (name, payload) => (handlers[name] || []).forEach((h) => h({ payload }));
  const now = Date.now();
  let tasks = [
    { id: 3, kind: "turn", title: "agrega la validación del login", state: "running", detail: "edit src/auth.rs", t0: now - 14000, t1: null },
    { id: 2, kind: "lsp", title: "rust-analyzer", state: "running", detail: "", t0: now - 60000, t1: null },
    { id: 1, kind: "engine", title: "cargando Qwen3-32B-4bit-DWQ", state: "done", detail: "vivo", t0: now - 95000, t1: now - 80000 },
    { id: 0, kind: "compact", title: "contexto compactado", state: "done", detail: "31.2k → 4.8k", t0: now - 300000, t1: now - 299000 },
  ];
  // C1: procesos del chat (como `procs_view` del núcleo)
  let procs = [
    { id: 2, name: "servidor demo", cmd: "cd demo && npm run dev", tty: false, alive: true, exit: null, secs: 312, urls: ["http://localhost:5173"], screen: "  VITE v8.3.7  ready in 478 ms\n  ➜  Local:   http://localhost:5173/", log: "/p/.gotv/procesos/2.log" },
    { id: 3, name: "cargo test", cmd: "cargo test --workspace", tty: false, alive: false, exit: "exit 0", secs: 41, urls: [], screen: "test result: ok. 186 passed", log: "/p/.gotv/procesos/3.log" },
  ];
  const SIN = location.search.includes("sin");
  const projects = SIN ? [] : [
    { path: "/Users/demo/projects/my-app", name: "my-app", last: 3 },
    { path: "/Users/demo/projects/website", name: "website", last: 2 },
    { path: "/Users/demo/projects/radar", name: "radar", last: 1 },
  ];
  const skills = [
    ["clear", "conversación nueva", "cmd"], ["compact", "resume el contexto ahora", "cmd"], ["context", "uso de la ventana de contexto", "cmd"],
    ["mode", "plan · default · acceptEdits · auto · bypass", "cmd"], ["rewind", "deshace el último paso que cambió archivos", "cmd"], ["help", "ayuda", "cmd"],
    ["tributario", "Asesor experto en derecho tributario colombiano…", "skill"], ["aduanero", "Asesor experto en derecho aduanero y comercio exterior…", "skill"],
    ["anatomy", "Figuras isométricas SVG interactivas…", "skill"], ["kicad", "Analiza proyectos KiCad y esquemáticos…", "skill"], ["chaos", "CHAOS, Dios del Vacío…", "skill"],
  ].map(([name, desc, kind]) => ({ name, desc, kind }));
  // como el estado de la ventana (gui.gotv): el chat de cada proyecto sobrevive a recargar (aquí, en sessionStorage)
  const chatGuardado = () => { try { return JSON.parse(sessionStorage.getItem("mock-chat") || "{}"); } catch { return {}; } };
  for (const p of projects) if (chatGuardado()[p.path]) p.chat = chatGuardado()[p.path];
  const status = { ready: true, model: "mlx-community/Qwen3-32B-4bit-DWQ", mode: "default", root: SIN ? "" : projects[0].path, ctx_used: 12400, ctx_budget: 40960, vision: false, hears: false, desktop_control: location.search.includes("mac") };
  // pruebas de la interfaz: emitir eventos del núcleo a mano (sólo existe en el simulador)
  window.__mockEmit = (payload) => emit("gotv", payload);
  window.__mockEmitir = emit; // el simulador del Experto (mock-experto.js) emite su avance por aquí
  window.__TAURI__ = {
    event: { listen: (name, h) => { (handlers[name] = handlers[name] || []).push(h); return Promise.resolve(() => {}); } },
    core: {
      invoke: async (cmd, a) => {
        switch (cmd) {
          case "pick_files": return ["/Users/demo/musica/cancion.wav"];
          case "info": return { ...status, desktop_control: !!status.desktop_control, ready: !SIN, root: SIN && !opened ? "" : status.root, home: "/Users/demo", lang: "es", theme: "dark", projects, tasks, procs };
          case "close_project": setTimeout(() => emit("gotv", { kind: "noproject" }), 50); return;
          case "pick_folder": return "/Users/demo/projects/notes";
          case "new_project": opened = true; { const p = a.path; projects.unshift({ path: p, name: (a.name || "").trim() || p.split("/").pop(), last: 9 }); status.root = p; const first = !everOpened; everOpened = true;
            // como el worker real: el PRIMER proyecto tras arrancar sin ninguno manda «ready»;
            // uno abierto después de cerrar otro (Msg::Close → Open) manda «done» silencioso
            setTimeout(() => { emit("gotv", { kind: "project", root: p, history: [] }); emit("gotv", first ? { kind: "ready" } : { kind: "done", text: "", silent: true }); }, 50); return p; }
          case "guardar_chat": {
            const p = projects.find((x) => x.path === a.proyecto);
            if (p) {
              p.chat = { mac: a.mac, perfil: a.perfil, manos: a.manos, pro: (p.chat || {}).pro };
              try { const g = chatGuardado(); g[a.proyecto] = p.chat; sessionStorage.setItem("mock-chat", JSON.stringify(g)); } catch {}
            }
            return;
          }
          // Ola PRO: como el núcleo (subconjunto del vocabulario de `gotv_core::pro`) y lo guardado por proyecto
          case "guardar_pro": {
            const p = projects.find((x) => x.path === a.proyecto);
            if (p) p.chat = Object.assign({}, p.chat || {}, { pro: a.pro });
            window.__mockPro = a.pro;
            return;
          }
          case "pro_opciones": {
            const L = (pares) => pares.map(([clave, nombre]) => ({ clave, nombre }));
            return { musica: {
              genero: L([["pop", "pop"], ["synthwave", "synthwave"], ["salsa", "salsa"], ["cinematic orchestral", "orquestal de cine"]]),
              epoca: L([["80s", "años 80"], ["moderno", "moderno"]]),
              animo: L([["nostalgico", "nostálgico"], ["energico", "enérgico"], ["intimo", "íntimo"]]),
              instrumentos: L([["teclas:piano", "piano"], ["teclas:pads", "colchones de sintetizador"], ["cuerdas:acustica", "guitarra acústica"], ["bajo:808", "bajo 808"], ["ritmo:bateria", "batería"], ["ritmo:congas", "congas"], ["orquesta:cuerdas", "sección de cuerdas"], ["viento:saxo", "saxofón"], ["otros:coro", "coro"]]),
              textura: L([["calida", "cálida"], ["contundente", "contundente"]]),
              produccion: L([["estudio", "estudio pulido"], ["lofi", "lo-fi"]]),
              voz: L([["instrumental", "instrumental (sin voz)"], ["femenina", "voz femenina"], ["masculina", "voz masculina"]]),
              tecnica: L([["aireada", "aireada (breathy)"], ["potente", "potente"]]),
              ritmo: L([["lento", "lento"], ["groove", "con groove"]]),
              idioma: L([["es", "español"], ["en", "inglés"]]),
              tonalidad: L([["A minor", "A menor"], ["C major", "C mayor"]]),
              etiquetas: ["[Intro]", "[Verse]", "[Chorus]", "[Bridge]", "[Outro]", "[whispered]"],
              pistas: L([["vocals", "voz principal"], ["drums", "batería"], ["bass", "bajo"], ["guitar", "guitarra"], ["keyboard", "teclados"]]),
              separables: L([["vocals", "voz"], ["drums", "batería"], ["bass", "bajo"], ["other", "resto"], ["guitar", "guitarra"], ["piano", "piano"]]),
            }, imagen: {
              proporcion: ["1:1", "4:3", "3:4", "3:2", "2:3", "16:9", "9:16", "21:9"].map((p) => ({ clave: p, nombre: p })),
              resolucion: L([["0.5", "0,5 MP (borrador rápido)"], ["1", "1 MP (normal)"], ["2", "2 MP (detalle)"], ["4", "4 MP (máximo)"]]),
              tipo: L([["foto", "fotografía"], ["ilustracion", "ilustración digital"], ["3d", "render 3D"], ["acuarela", "acuarela"], ["isometrico", "isométrico"]]),
              camara: L([["hasselblad", "Hasselblad X2D (medio formato)"], ["sony", "Sony A7IV (digital moderna)"], ["digicam", "cámara digital de los 2000"]]),
              lente: L([["35", "35 mm"], ["85", "85 mm (retrato)"], ["macro", "macro"]]),
              apertura: L([["1.4", "f/1.4 (fondo muy desenfocado)"], ["8", "f/8 (todo nítido)"]]),
              pelicula: L([["portra", "Kodak Portra 400"], ["trix", "Kodak Tri-X (blanco y negro)"]]),
              luz: L([["suave", "suave y difusa"], ["contraluz", "contraluz"]]),
              hora: L([["dorada", "hora dorada"], ["noche", "noche"]]),
              plano: L([["general", "plano general"], ["primer", "primer plano"]]),
              angulo: L([["picado", "picado (desde arriba)"], ["contrapicado", "contrapicado (desde abajo)"]]),
              composicion: L([["tercios", "regla de tercios"], ["simetrica", "simétrica"]]),
            }, video: {
              formato: L([["16:9", "16:9 horizontal"], ["9:16", "9:16 vertical"], ["1:1", "1:1 cuadrado"]]),
              calidad: L([["720p", "720p (oficial)"], ["480p", "480p, unas 3 veces más rápido (medido)"]]),
              plano: L([["gran-general", "plano general extremo"], ["general", "plano general"], ["medio", "plano medio"], ["primer", "primer plano"], ["detalle", "plano detalle"]]),
              angulo: L([["ojos", "a la altura de los ojos"], ["picado", "picado (desde arriba)"], ["contrapicado", "contrapicado (desde abajo)"], ["aereo", "aéreo (dron)"]]),
              lente: L([["gran-angular", "gran angular"], ["tele", "teleobjetivo"], ["macro", "macro"], ["bokeh", "poca profundidad (bokeh)"]]),
              movimiento: L([["fija", "cámara fija"], ["acercar", "acercamiento (push-in)"], ["pan-izq", "paneo a la izquierda"], ["seguimiento", "travelling (sigue al sujeto)"], ["orbita", "órbita alrededor"]]),
              velocidad: L([["lenta", "lenta"], ["normal", "normal"], ["rapida", "rápida"]]),
              luz: L([["suave", "suave y difusa"], ["contraluz", "contraluz"], ["borde", "luz de borde (rim)"], ["neon", "neón"]]),
              hora: L([["amanecer", "amanecer"], ["dorada", "hora dorada"], ["noche", "noche"]]),
              tono: L([["calido", "cálido"], ["frio", "frío"], ["teal-naranja", "teal y naranja"], ["bn", "blanco y negro"]]),
              estilo: L([["cine", "cinematográfico"], ["anime", "anime"], ["plastilina", "plastilina (stop-motion)"], ["timelapse", "time-lapse"]]),
            } };
          }
          case "pro_video_estimado": {
            // la misma fórmula que `ProVideo::minutos_estimados`
            const v = a.video || {}, s = Math.min(5, Math.max(1, a.segundos || 2));
            const [w, h] = v.calidad === "480p" ? (v.formato === "9:16" ? [480, 832] : v.formato === "1:1" ? [640, 640] : [832, 480]) : v.formato === "9:16" ? [704, 1280] : v.formato === "1:1" ? [960, 960] : [1280, 704];
            const tok = (w, h, s) => Math.floor(w / 32) * Math.floor(h / 32) * (Math.floor(Math.round(s * 24) / 4) + 1);
            const base = tok(1280, 704, 2), f = Math.pow(tok(w, h, s) / base, 1.24), pasos = v.pasos || 50, g = v.guia || 5;
            return (pasos * (g > 1 ? 54 : 27) * f + 143 * tok(w, h, s) / base) / 60;
          }
          // F13.8: panel «Conectores» (datos de prueba)
          case "mcp_lista": return { proyecto: status.root, avisos: [], importables: [{ nombre: "github", origen: "Claude Code", secretos: 1 }], servidores: (window.__mcp = window.__mcp || [{ nombre: "todo", ambito: "Usuario", tipo: "Stdio", origen: "npx -y @modelcontextprotocol/server-everything", activo: true, caja: true, remoto: false, sesion: false, registro: [] }, { nombre: "remoto", ambito: "Local", tipo: "Http", origen: "https://mcp.ejemplo.com/mcp", activo: true, caja: true, remoto: true, sesion: false, registro: [] }]) };
          case "mcp_probar": return { notas: ["MCP «" + a.nombre + "»: 19 herramienta(s)"], herramientas: ["mcp__todo__echo", "mcp__todo__add"], tokens_si_se_cargan: 3120 };
          case "mcp_agregar": window.__mcp.push({ nombre: a.nombre, ambito: a.ambito === "local" ? "Local" : "Usuario", tipo: /^https?:/.test(a.destino) ? "Http" : "Stdio", origen: a.destino, activo: true, caja: true, remoto: /^https?:/.test(a.destino), sesion: false, registro: [] }); return;
          case "mcp_activar": window.__mcp.find((s) => s.nombre === a.nombre).activo = a.activo; return;
          case "mcp_quitar": window.__mcp = window.__mcp.filter((s) => s.nombre !== a.nombre); return;
          case "mcp_entrar": window.__mcp.find((s) => s.nombre === a.nombre).sesion = true; return;
          case "mcp_salir": window.__mcp.find((s) => s.nombre === a.nombre).sesion = false; return;
          case "mcp_importar": return a.nombres.length;
          // F23.2: panel «Buscadores» (datos de prueba; las fichas reales viven en el núcleo)
          case "buscadores_lista": {
            const B = (window.__bs = window.__bs || [
              { id: "duckduckgo", nombre: "DuckDuckGo (lite)", necesita_llave: false, tiene_llave: true, activo: true, activable: true, problemas: [], aviso: "sin revisar", en_orden: true, en_respaldo: false, tope_mes: null, usado_mes: 41, precio: "sin llave ni pago", precio_fecha: "2026-11-16", que_guarda: "DuckDuckGo no tiene una API oficial de resultados ni dice si se pueden guardar: GOTV no los guarda.", donde_llave: "" },
              { id: "wikipedia", nombre: "Wikipedia", necesita_llave: false, tiene_llave: true, activo: true, activable: true, problemas: [], aviso: null, en_orden: true, en_respaldo: false, tope_mes: null, usado_mes: 12, precio: "gratis", precio_fecha: "2026-11-16", que_guarda: "Wikipedia permite guardar (CC BY-SA 4.0, con enlace al artículo).", donde_llave: "" },
              { id: "brave", nombre: "Brave Search API", necesita_llave: true, tiene_llave: false, activo: false, activable: true, problemas: [], aviso: null, en_orden: false, en_respaldo: true, tope_mes: 1000, usado_mes: 0, precio: "5 USD por 1 000 búsquedas; 5 USD de crédito gratis al mes", precio_fecha: "2026-11-16", que_guarda: "Brave prohíbe guardar sus resultados: GOTV no los guarda.", donde_llave: "https://api-dashboard.search.brave.com/app/keys" },
            ]);
            return { servicios: B, paralelo: true, solo_local: false, contacto: "", ocupa: { cache: 182000, indice: 4200000 }, indice: { paginas: 37, ocupa: 4200000, tope_gb: 5, ritmo_ms: 1000, semillas: ["https://docs.rs/tokio"] } };
          }
          case "buscadores_llave": window.__bs.find((s) => s.id === a.id).tiene_llave = !!a.llave; return;
          case "buscadores_activar": window.__bs.find((s) => s.id === a.id).activo = a.activo; return;
          case "buscadores_tope": window.__bs.find((s) => s.id === a.id).tope_mes = a.tope; return;
          case "buscadores_probar": return { ok: true, detalle: "", resultados: [{ titulo: "Rust Programming Language", url: "https://www.rust-lang.org/" }, { titulo: "Rust (programming language) - Wikipedia", url: "https://en.wikipedia.org/wiki/Rust_(programming_language)" }], usado_mes: 1, tope_mes: 1000 };
          case "buscadores_ficha": return { publica: "Brave Software, Inc.", condiciones_revisadas: "2026-11-16", revisada_por: "Opus 5.5 (hilo F23), sin abogado", prohibe_entrenar_ia: "si", dominio_api: "api.search.brave.com", atribucion: "", notas_legales: "- §3(b)(i): no «store, cache, or create a database of Search Results».", fuentes: ["https://api-dashboard.search.brave.com/terms-of-service"] };
          case "buscadores_revisar": return { igual: true };
          case "buscadores_orden": case "indice_semillas": return;
          case "buscadores_uso": return {};
          case "indice_estado": return { paginas: 37, ocupa: 4200000, semillas: [] };
          case "busquedas_borrar": return { cache: 182000, indice: 4200000 };
          // F1 paso 8: panel «Bases de datos» (datos de prueba)
          case "bases_lista": return { proyecto: status.root, ssh: ["servidor"], bases: (window.__bases = window.__bases || [{ nombre: "app", motor: "sqlite", donde: "datos/app.db", tunel: null, produccion: false, mascara: [], clave: null }, { nombre: "tienda", motor: "postgres", donde: "ana@db.ejemplo.com:5432/tienda", tunel: "servidor", produccion: true, mascara: ["cedula"], clave: null }]), detectadas: [{ nombre: ".env:DATABASE_URL", motor: "postgres", donde: "app@localhost:5432/dev", url: "postgres://app@localhost:5432/dev", secreto: "env:DATABASE_URL" }, { nombre: "datos/app.db", motor: "sqlite", donde: "datos/app.db", url: null, secreto: null }] };
          case "bases_agregar": window.__bases.push({ nombre: a.nombre, motor: a.destino.includes("://") ? a.destino.split("://")[0] : "sqlite", donde: a.destino.replace(/^\w+:\/\//, ""), tunel: a.tunel, produccion: a.produccion, mascara: a.mascara, clave: a.secreto }); return "✓";
          case "bases_probar": return a.nombre === "app" ? "✓ SQLite 3.46.0 · 3 tablas · datos/app.db (abierta en sólo lectura para leer)" : "✓ PostgreSQL 16.14 · usuario ana · base tienda\n⚠ este usuario PUEDE escribir.";
          case "bases_quitar": window.__bases = window.__bases.filter((b) => b.nombre !== a.nombre); return;
          case "bases_secreto": window.__bases.find((b) => b.nombre === a.nombre).clave = "llavero:bd/x/" + a.nombre; return "bd/x/" + a.nombre;
          case "bases_marcar": window.__bases.find((b) => b.nombre === a.nombre).produccion = a.produccion; return;
          case "forget_project": { const i = projects.findIndex((x) => x.path === a.path); if (i >= 0) projects.splice(i, 1); return; }
          case "slash_items": return skills;
          // ⚡ Reabrir rápido (simulado: 1,2 GB al encender)
          case "cache_conversacion": {
            if (a.topeGb) window.__tope = a.topeGb * 2 ** 30;
            // como el núcleo: como mucho 3 proyectos encendidos (`window.__lleno = true` lo simula)
            if (a.on && !window.__rapido && window.__lleno) throw "ya hay 3 proyectos con «Reabrir rápido» (website, radar, notes): apágalo en uno de ellos para encenderlo aquí";
            if (a.on !== undefined) {
              window.__rapido = a.on;
              // F4: al ENCENDER se construye la memoria con su barra (simulada: 20 000 tokens en ~2 s)
              if (a.on) {
                emit("gotv", { kind: "precarga", fase: "inicio" });
                let done = 0;
                clearInterval(window.__pre);
                window.__pre = setInterval(() => {
                  done += 2048;
                  if (window.__preCancel) { clearInterval(window.__pre); window.__preCancel = false; emit("gotv", { kind: "precarga", fase: "cancelada" }); return; }
                  if (done >= 20000) { clearInterval(window.__pre); emit("gotv", { kind: "precarga", fase: "lista", tokens: 20000, del_disco: 0, calculados: 20000, bytes: 6.4e8, seg: 2.1 }); return; }
                  emit("gotv", { kind: "precarga", fase: "avance", done, total: 20000 });
                }, 200);
              }
            }
            return { on: !!window.__rapido, bytes: window.__rapido ? 1.2e9 : 0, tope: window.__tope || 3 * 2 ** 30, topes_gb: [3, 6, 12, 24] };
          }
          // «Cancelar» de la memoria rápida usa stop_turn (como el núcleo: la bandera cancel)
          case "stop_turn": if (window.__pre) window.__preCancel = true; return;
          // F5: los modelos locales del menú del encabezado
          case "models_list": return { model: status.model, model_path: status.model_path, models: [
            { name: "Qwen3-32B-4bit-DWQ", path: "/Users/demo/.lmstudio/models/mlx-community/Qwen3-32B-4bit-DWQ", gb: 18.4, fits: true },
            { name: "Qwen3.5-9B-4bit", path: "/Users/demo/.cache/gotv/models/mlx-community/Qwen3.5-9B-4bit", gb: 6, fits: true },
            { name: "Hermes-4-70B-MLX-4bit", path: "/Users/demo/.lmstudio/models/lmstudio-community/Hermes-4-70B-MLX-4bit", gb: 39.7, fits: false, why: "necesita 44 GB y hay 38 libres" },
          ] };
          case "set_model": setTimeout(() => {
            status.model = a.path.split("/").pop(); status.model_path = a.path;
            emit("gotv", { kind: "status", status: { ...status } });
            emit("gotv", { kind: "done", text: "modelo: " + status.model + " ✅", info: true });
          }, 400); return;
          // M2: leer en voz alta (simulado: «suena» 1,5 s y avisa el fin, como el núcleo)
          case "voz_leer": clearTimeout(window.__voz); window.__vozId = a.id; window.__vozTexto = a.texto; window.__voz = setTimeout(() => emit("gotv", { kind: "voz", fin: true, id: a.id }), 1500); return;
          // como el núcleo: la lectura callada avisa su fin TARDE (con su id)
          case "voz_callar": { clearTimeout(window.__voz); const id = window.__vozId; setTimeout(() => emit("gotv", { kind: "voz", fin: true, id, error: "callada" }), 120); return; }
          // M3: manos libres (simulado: saluda, escucha, «oye» una frase y la manda como mensaje)
          case "manos_libres":
            clearTimeout(window.__manos);
            if (!a.on) { setTimeout(() => emit("gotv", { kind: "manos", estado: "apagado" }), 30); return; }
            window.__manos = setTimeout(() => {
              emit("gotv", { kind: "manos", estado: "escuchando" });
              window.__manos = setTimeout(() => {
                emit("gotv", { kind: "manos", estado: "oyendo" });
                window.__manos = setTimeout(() => { emit("gotv", { kind: "voz_tu", text: "¿Qué hora es en Tokio?" }); emit("gotv", { kind: "manos", estado: "pensando" }); }, 600);
              }, 600);
            }, 600);
            return;
          // Biblioteca: muestra REAL de la API (gotv models search "" --json), re-medida aquí
          // contra la Mac simulada con la misma regla que el núcleo
          case "library_search": {
            const d = await (await fetch("mock-lib.json")).json();
            const presets = ["M2|8", "M2|16", "M4 Pro|48", "M4 Max|128", "M3 Ultra|512"].map((id) => { const [c, g] = id.split("|"); const bw = { M2: 100, "M4 Pro": 273, "M4 Max": 546, "M3 Ultra": 819 }[c]; return { id, label: `${c} · ${g} GB`, device: "Apple " + c, memory_gb: +g, budget: +g * (g >= 36 ? 0.75 : 0.67) * 2 ** 30, bandwidth: bw, exact: true, this_mac: false }; });
            const mac = a.mac && a.mac !== "this" ? presets.find((p) => p.id === a.mac) : d.mac;
            const q = (a.query || "").toLowerCase();
            const sp = (t) => (t >= 50 ? "muy-rapido" : t >= 25 ? "rapido" : t >= 12 ? "medio" : t >= 6 ? "lento" : "muy-lento");
            const items = d.items.filter((m) => !q || m.repo.toLowerCase().includes(q)).filter((m) => !a.org || m.repo.startsWith(a.org + "/")).map((m) => {
              if (mac === d.mac) return m;
              const tok = m.tok_s * (mac.bandwidth / d.mac.bandwidth) * 0.92, lim = mac.budget * 0.95;
              return { ...m, tok_s: tok, speed: sp(tok), fits: m.need <= lim, tight: m.need <= lim && m.need > lim * 0.8 };
            });
            await new Promise((r) => setTimeout(r, 250));
            return { this: d.mac, mac, presets, items, pulls: [] };
          }
          case "library_pull": {
            let done = 0; const total = 4.6e9;
            const tick = setInterval(() => { done = Math.min(total, done + 0.31e9); emit("gotv", { kind: "pull", repo: a.repo, state: done >= total ? "done" : "run", done, total, rate: 88, path: "/m/nuevo" }); if (done >= total) clearInterval(tick); }, 300);
            return `⬇ bajando ${a.repo} (4,6 GB) en segundo plano — el avance va en Tareas`;
          }
          case "library_cancel": emit("gotv", { kind: "pull", repo: a.repo, state: "cancel" }); return true;
          // F18 · M9 · Dispositivos: una Mac SIN SDK de Android y con Xcode sin preparar (las cifras, del catálogo real del 2026-11-17)
          case "devices_state": {
            const D = (window.__disp = window.__disp || { lic: false, inst: [] });
            const base = "https://dl.google.com/android/repository/";
            const ficha = (ruta, nombre, version, url, tam) => ({ ruta, nombre, version, publica: "Google", url: base + url, tamano: tam, destino: "/Users/demo/Library/Application Support/gotv/android-sdk/" + ruta.replaceAll(";", "/"), licencia: "android-sdk-license" });
            const fichas = [
              ficha("platform-tools", "Android SDK Platform-Tools", "37.0.1", "platform-tools_r37.0.1-darwin.zip", 16e6),
              ficha("emulator", "Android Emulator", "37.2.13", "emulator-darwin_aarch64-16740729.zip", 416e6),
              ficha("platforms;android-36", "Android SDK Platform 36", "2.0.0", "platform-36_r02.zip", 66e6),
            ].filter((f) => !D.inst.includes(f.ruta));
            return {
              android: { sdk: null, destino: "/Users/demo/Library/Application Support/gotv/android-sdk", publica: "Google", fichas, total: fichas.reduce((s, f) => s + f.tamano, 0),
                licencias: fichas.length ? [{ id: "android-sdk-license", titulo: "Terms and Conditions", texto: "Terms and Conditions\n\nThis is the Android Software Development Kit License Agreement\n\n1. Introduction\n1.1 The Android Software Development Kit (referred to in the License Agreement as the \"SDK\")…", aceptada: D.lic, paquetes: fichas.map((f) => f.ruta) }] : [],
                instalados: D.inst, dispositivos: [] },
              xcode: { carpeta: "/Applications/Xcode.app/Contents/Developer", version: "Xcode 27.2", simuladores: [], idb: null, tienda: "macappstore://", preparacion: { licencia_aceptada: false, primer_arranque_hecho: false, licencia: "/Applications/Xcode.app/Contents/Resources/en.lproj/License.rtf" }, idb_fijado: "1.7.8", idb_tamano: 30893766, idb_licencia: "https://github.com/meta/idb/blob/main/LICENSE" },
              progreso: [],
            };
          }
          case "devices_accept": (window.__disp = window.__disp || { lic: false, inst: [] }).lic = true; return null;
          case "devices_install": (window.__disp = window.__disp || { lic: false, inst: [] }).inst.push(a.paquete); return null;
          case "devices_uninstall": window.__disp.inst = window.__disp.inst.filter((p) => p !== a.paquete); return null;
          case "devices_xcode_store": case "devices_xcode_license": case "devices_xcode_platform": return null;
          case "devices_xcode_prepare": return "(simulado) Xcode preparado";
          case "devices_idb_install": return "/Users/demo/Library/Application Support/gotv/idb/1.7.8/bin/idb_companion";
          // M0 · generadores: el catálogo REAL (extraído de gotv-core/src/generadores.rs) con las mismas puertas
          case "generators_list": {
            const et = { libre: "libre", atribucion: "libre · pide atribución", condiciones: "⚠ con condiciones", no_comercial: "⚠ no comercial" };
            const an = { oir: "oír", hablar: "hablar", imagen: "imagen", video: "video", musica: "música", buscar: "búsqueda" };
            return { verificado: "2026-11-10", pulls: [], items: GEN_CAT.map((g) => ({ ...g, etiqueta: et[g.clase], area_nombre: an[g.area],
              aviso: g.origen ? `Estos pesos pertenecen a ${g.origen.split("/")[0]} (huggingface.co/${g.origen}); GOTV usa su conversión publicada en huggingface.co/${g.repo}. GOTV no los incluye ni los redistribuye: los baja quien acepta, y su uso es responsabilidad de quien los descarga. Licencia: ${g.licencia} — ${g.url_licencia}` : `Estos pesos pertenecen a ${g.repo.split("/")[0]} (huggingface.co/${g.repo}); GOTV no los incluye ni los redistribuye: los baja quien acepta, desde el repositorio de su autor, y su uso es responsabilidad de quien los descarga. Licencia: ${g.licencia} — ${g.url_licencia}`,
              aceptada: genAcc[g.repo] || null, bytes_local: genHave[g.repo] || 0, token_hf: genTok, leida: !!genLeidas[g.repo] })) };
          }
          // lo que le falta a cada función (como `generadores::falta_para`): lo bajado en el simulador ya no falta
          case "generador_falta": {
            const pide = { oir: ["mlx-community/whisper-large-v3-turbo-asr-fp16"], hablar: ["Qwen/Qwen3-TTS-12Hz-1.7B-Base"], manos: ["mlx-community/whisper-large-v3-turbo-asr-fp16", "Qwen/Qwen3-TTS-12Hz-1.7B-Base"],
              imagen: ["black-forest-labs/FLUX.2-klein-4B"], video: ["Wan-AI/Wan2.2-TI2V-5B-Diffusers"], musica: ["ACE-Step/Ace-Step1.5"] }[a.uso] || [];
            return pide.find((r) => !genHave[r]) || null;
          }
          case "generador_de_aviso": return /Generadores|Generators/.test(a.texto) ? (GEN_CAT.find((g) => a.texto.includes(g.repo)) || {}).repo || null : null;
          case "generator_read": genLeidas[a.repo] = true; return;
          case "generator_accept": if (a.accept && !genLeidas[a.repo]) throw "primero lee la licencia (botón «Leer»): después se desbloquea «Aceptar»"; if (a.accept) genAcc[a.repo] = { licencia: "x", url: "", sha: (GEN_CAT.find((g) => g.repo === a.repo) || {}).sha, fecha: new Date().toISOString(), gotv: "0.0.1" }; else delete genAcc[a.repo]; return !!a.accept;
          case "hf_token": genTok = !!(a.token || "").trim(); return genTok;
          case "generator_pull": {
            const g = GEN_CAT.find((x) => x.repo === a.repo);
            if (!genAcc[a.repo]) throw `falta aceptar la licencia de ${g.nombre} (${g.licencia})`;
            if (g.restringido && !genTok) throw `${g.nombre} es de acceso restringido en Hugging Face: acepta en su página con TU cuenta y guarda tu token. GOTV no acepta por ti.`;
            let done = 0; const total = Math.min(g.bytes_repo, 2.2e9);
            const tick = setInterval(() => { done = Math.min(total, done + 0.27e9); if (done >= total) genHave[a.repo] = total; emit("gotv", { kind: "pull", repo: a.repo, state: done >= total ? "done" : "run", done, total, rate: 61 }); if (done >= total) clearInterval(tick); }, 300);
            return `⬇ bajando ${a.repo} en segundo plano — el avance va en Tareas`;
          }

          case "open_link": return;
          case "answer_choice": status.lastChoice = a.value; return;
          case "chrome_status": return { installed: !!status.crInstalled, connected: false, ext_dir: "/Applications/GOD OF THE VOID.app/Contents/Resources/extension-chrome" };
          case "chrome_install": status.crInstalled = true; return { installed: true, connected: false, ext_dir: "/x" };
          case "reveal_ext": return;
          case "proc_stop": {
            const p = procs.find((x) => x.id === a.id);
            if (p && p.alive) { p.alive = false; p.exit = "terminado por señal"; } else procs = procs.filter((x) => x.id !== a.id);
            setTimeout(() => emit("gotv", { kind: "procs", procs: JSON.parse(JSON.stringify(procs)) }), 80);
            return;
          }
          // F10.5: teléfonos emparejados (simulados)
          case "remoto_tokens": return status.tel || (status.tel = [{ id: "a1b2c3d4", nombre: "iPhone", creado: 1794900000 }]);
          case "remoto_revocar": status.tel = (status.tel || []).filter((t) => t.id !== a.id); return status.tel;
          case "remoto_codigo": return { codigo: "48151623", urls: ["http://192.168.1.20:11435/gotv/movil"], lan: true, minutos: 5, qr: null };
          case "api_status": return status.api || { running: false, enabled: false, lan_wanted: false, port_wanted: 11435, cloud: false, models: ["mlx-community/Qwen3-32B-4bit-DWQ"], hits: 0 };
          case "api_apply": status.api = a.enabled ? { running: true, enabled: true, lan: a.lan, lan_wanted: a.lan, port: a.port, port_wanted: a.port, cloud: a.cloud,
              urls: a.lan ? [`http://192.168.1.20:${a.port}`, `http://127.0.0.1:${a.port}`] : [`http://127.0.0.1:${a.port}`], key: a.lan ? "gotv-3f9a0c1d2e4b5a6978f0e1d2c3b4a5968778695a4b3c2d1e" : null,
              models: ["mlx-community/Qwen3-32B-4bit-DWQ"].concat(a.cloud ? ["anthropic:claude-opus-5-5"] : []), hits: 3 } : { running: false, enabled: false, lan_wanted: a.lan, port_wanted: a.port, cloud: a.cloud, models: [], hits: 3 };
            return status.api;
          case "api_new_key": status.api.key = "gotv-" + Math.random().toString(16).slice(2).padEnd(48, "0"); return status.api;
          case "ventana_pref": return (window.__prefs || {})[a.clave] ?? null;
          case "ventana_pref_poner": (window.__prefs = window.__prefs || {})[a.clave] = a.valor; return null;
          case "settings": return { lang: "auto", theme: "dark", max_tokens: 4096, temperature: null, mode: "default", computer_use: false, get system_changes(){return status.system_changes||false;}, get desktop_control(){return status.desktop_control||false;}, get max_steps(){return status.max_steps||40;}, model: status.model, model_path: "/m/32b", ctx_budget: 40960, context_preset: "128k",
            effort: "high", get sandbox(){return status.sandbox!==false;}, sandbox_available: true, get net_allow(){return status.net_allow||[];}, net_default: ["pypi.org","files.pythonhosted.org","registry.npmjs.org","crates.io","static.crates.io","nodejs.org"], net_learned: ["github.com"], cloud_errors: [{ provider: "openrouter", error: "HTTP 401 (llave inválida o sin permiso): No auth credentials found" }],
            providers: [
              { name: "anthropic", kind: "anthropic", base_url: "", vision: true, has_key: true, builtin: true },
              { name: "openai", kind: "openai", base_url: "https://api.openai.com/v1", vision: true, has_key: false, builtin: true },
              { name: "openrouter", kind: "openai", base_url: "https://openrouter.ai/api/v1", vision: false, has_key: true, builtin: false },
            ],
            presets: [{ name: "openrouter", label: "OpenRouter", base_url: "https://openrouter.ai/api/v1" }, { name: "groq", label: "Groq", base_url: "https://api.groq.com/openai/v1" }],
            models: [
              { name: "anthropic:claude-opus-5-5", label: "Claude Opus 5.5", path: "anthropic:claude-opus-5-5", kind: "nube · anthropic", cloud: true, provider: "anthropic", fits: true, ctx_top: 1000000, model_max: 1000000, max_ctx: 1000000, ctx_auto: 200000, vision: true },
              { name: "anthropic:claude-sonnet-5-5", label: "Claude Sonnet 5.5", path: "anthropic:claude-sonnet-5-5", kind: "nube · anthropic", cloud: true, provider: "anthropic", fits: true, ctx_top: 1000000, model_max: 1000000, max_ctx: 1000000, ctx_auto: 200000, vision: true },
              { name: "mlx-community/Qwen3-32B-4bit-DWQ", path: "/m/32b", kind: "denso", gb: 18.4, fits: true, max_ctx: 40960, model_max: 40960, ctx_top: 40960, vision: false, audio: false },
              { name: "mlx-community/gemma-4-e4b-it-4bit", path: "/m/e4b", kind: "híbrido+visión", gb: 5.2, fits: true, max_ctx: 98304, model_max: 131072, ctx_top: 98304, vision: true, audio: true },
              { name: "propios/vacio-sft", path: "/m/propio", kind: "denso", gb: 0.1, fits: true, max_ctx: 512, model_max: 512, ctx_top: 512, chat_only: true },
              { name: "lmstudio-community/Hermes-4-70B-MLX-4bit", path: "/m/70b", kind: "denso", gb: 39.7, fits: false, why: "necesita 45 GB y ahora sólo se pueden usar 40", max_ctx: 0 },
            ] };
          case "send": status.lastMac = !!a.mac; window.__mockUltimoEnvio = a; setTimeout(() => demoTurn(a.task), 200); return;
          case "open_vista": window.__mockAbierto = a.path; return;
          // F7 · la tarjeta del plan (como gotv_core::artefactos::comentarios)
          case "plan_comentar": { const c = { id: (planComs.length + 1), archivo: a.archivo, linea: a.linea, texto_linea: (PLAN_TXT.split("\n")[a.linea - 1] || "").trim(), texto: a.texto, resuelto: false, entregado: false, version: 1 }; planComs.push(c); return c; }
          case "plan_comentarios": return planComs.filter((c) => c.archivo === a.archivo);
          case "plan_resolver": { const c = planComs.find((c) => c.id === a.id); if (c) c.resuelto = a.resuelto; return; }
          case "guardar_como": window.__mockGuardado = a.path; return "/Users/demo/Downloads/" + String(a.path).split("/").pop();
          case "dock_icono": (window.__mockDock ||= []).push(a.png ? a.png.length : null); return null;
          // M6: un MP4 de verdad (64×48, 12 cuadros, H.264) hecho por `video_mp4.rs`, en binario como en Tauri
          case "leer_video": { window.__mockVideo = a.path; const s = atob("AAAAHGZ0eXBtcDQyAAAAAWlzb21tcDQxbXA0MgAAAAFtZGF0AAAAAAAAAkAAAAA2BgUtR1ZK3FxMQz+U78URPNFDqAEAAAMAAQMAAAMAAQIAACszCwAAAwAAAwAAAwKUDAORKwGAAAAAdCW4IAFPzDYq87/9N8yW4iMtxxX46tBDN+U53bVSxd3gPUudCOHTAF5q/f81kSqX4bldyp3J4+hwO8nCl+Z7OVMTzzia8hHuugfd82IvTbK81eO+2fJveakrH6RH8dlss492xbPKMEAR6XJk1AGyx7G0pJUKAAAAMSHhEEKf3oX9Qdx9NmaJx5ZwPHpjkqEZHqKVNOHzPyMsZWECRF8MywOx1niQze9tLkIAAAAdIaiChCX/yM7FIOvXb/+O1XvW6fAsSMus9fjxqmQAAAAaAajBiFv/yuqAdviza7cU7JZwFo37CNqVO8AAAAAWAajDiFP/hPTOCYBIelVO7YQnLo6tYAAAADEh4yGiI7/VOT6B1/4Dygpwho9A9crL2vqpDeLSGY+AnYLMEvluciwZnesVJ742bbWwAAAAFSGpBoRncypzCKIw610IiPStcXCawAAAABQBqUWIU/+E9M4JgESbaabtScA0gAAAABUBqUeIS/+A8jkI25iK7XC/n4r78IAAAAAyIeUtoiI/0pq6CLRyZImrteFABepwXhnpTCHvI8gJ8p9M6qCXF8zoaagm+bC+d4rv1cAAAAAhIamKhN/+474QwfCm5crKEktXqfiak7FLROSMuEQuPK6YAAAAEgGpyYjveXAaCKI06cUkzRKEgAAAA2Ztb292AAAAbG12aGQAAAAA5xy9quccvaoAAAJYAAABLAABAAABAAAAAAAAAAAAAAAAAQAAAAAAAAAAAAAAAAAAAAEAAAAAAAAAAAAAAAAAAEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACAAAC8nRyYWsAAABcdGtoZAAAAAHnHL2q5xy9qgAAAAEAAAAAAAABLAAAAAAAAAAAAAAAAAAAAAAAAQAAAAAAAAAAAAAAAAAAAAEAAAAAAAAAAAAAAAAAAEAAAAAAQAAAADAAAAAAACRlZHRzAAAAHGVsc3QAAAAAAAAAAQAAASwAAAAyAAEAAAAAAmptZGlhAAAAIG1kaGQAAAAA5xy9quccvaoAAAJYAAABLFXEAAAAAAAxaGRscgAAAAAAAAAAdmlkZQAAAAAAAAAAAAAAAENvcmUgTWVkaWEgVmlkZW8AAAACEW1pbmYAAAAUdm1oZAAAAAEAAAAAAAAAAAAAACRkaW5mAAAAHGRyZWYAAAAAAAAAAQAAAAx1cmwgAAAAAQAAAdFzdGJsAAAAoXN0c2QAAAAAAAAAAQAAAJFhdmMxAAAAAAAAAAEAAAAAAAAAAAAAAAAAAAAAAEAAMABIAAAASAAAAAAAAAABAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAGP//AAAAJ2F2Y0MBZAAL/+EADCdkAAusVlDDeBBhlAEABCjuPLD9+PgAAAAACmZpZWwBAAAAAApjaHJtAAAAAAAYc3R0cwAAAAAAAAABAAAADAAAABkAAABwY3R0cwAAAAAAAAAMAAAAAQAAADIAAAABAAAAfQAAAAEAAAAyAAAAAQAAAAAAAAABAAAAGQAAAAEAAAB9AAAAAQAAADIAAAABAAAAAAAAAAEAAAAZAAAAAQAAAGQAAAABAAAAMgAAAAEAAAAAAAAAFHN0c3MAAAAAAAAAAQAAAAEAAAAYc2R0cAAAAAAgEBAYGBAQGBgQEBgAAAAcc3RzYwAAAAAAAAABAAAAAQAAAAwAAAABAAAARHN0c3oAAAAAAAAAAAAAAAwAAACyAAAANQAAACEAAAAeAAAAGgAAADUAAAAZAAAAGAAAABkAAAA2AAAAJQAAABYAAAAUc3RjbwAAAAAAAAABAAAALA=="); const u = new Uint8Array(s.length); for (let i = 0; i < s.length; i++) u[i] = s.charCodeAt(i); return u.buffer; }
          case "conexiones_lista": { status.cx = status.cx || [{ nombre: "bastion", destino: "deploy@b.ejemplo.com", puerto: 2222, nota: "entrada a la red", solo_lectura: false, llave: true, fijada: true }];
            return { proyecto: "/Users/demo/tienda", conexiones: status.cx, registro: [{ cuando: "2026-11-10T03:12:00Z", conexion: "bastion", salida: 0, orden: "df -h /" }] }; }
          case "conexiones_agregar": { if (!/^[\w-]+$/.test(a.nombre || "")) throw "nombre inválido"; status.cx.push({ nombre: a.nombre, destino: a.destino, salto: a.salto, nota: a.nota, solo_lectura: a.soloLectura, llave: false, fijada: false }); return; }
          case "conexiones_quitar": status.cx = status.cx.filter((c) => c.nombre !== a.nombre); return;
          case "conexiones_llave": { const c = status.cx.find((x) => x.nombre === a.nombre); if (c) c.llave = true; return "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDemoPublica gotv " + a.nombre; }
          case "conexiones_escanear": return { host: "b.ejemplo.com", huellas: ["256 SHA256:jgqThFMbPrsqR9SxP9vZNn4nQVmfbWsCn7Nn9pbw1uE [b.ejemplo.com]:2222 (ED25519)"] };
          case "conexiones_fijar_y_probar": { const c = status.cx.find((x) => x.nombre === a.nombre); if (c) c.fijada = true; return "deploy@web-1 · Linux 6.8.0"; }
          case "vista_variantes": {
            const pag = (d) => `<!doctype html><html><head><meta charset="utf-8"><title>${d}</title><style>body{font:16px system-ui;margin:0}section{padding:32px}h1{font-size:40px}</style></head><body><nav data-gotv="0" data-gotv-tipo="navegacion">Grano Alto</nav><section data-gotv="1" data-gotv-tipo="heroe"><h1>Café de altura · ${d}</h1><p>Tostado en casa</p></section><footer data-gotv="2" data-gotv-tipo="pie">© Grano Alto</footer></body></html>`;
            return ["tecnico", "lujo", "suizo"].map((d, i) => ({ direccion: d, html: pag(d), actual: i === 0 }));
          }
          case "vista_usar": window.__mockUsada = a.direccion; return { html: "", direccion: a.direccion, respetados: 0 };
          case "provider_test": if (a.name === "openai") throw "no hay llave para «openai»"; return 42;
          case "provider_key": return;
          case "set_system_changes": status.system_changes = a.on; return;
          case "contexto_opciones": return { preset: status.ctx_preset || "auto", ctx_budget: status.ctx_budget, model: status.model, cloud: false, ctx_top: 131072, model_max: 262144 };
          case "apply_settings": if (a.s && a.s.context_preset) { const n = { "8k": 8192, "16k": 16384, "32k": 32768, "64k": 65536, "128k": 131072, "256k": 262144 }[a.s.context_preset] || 131072; status.ctx_preset = a.s.context_preset; status.ctx_budget = n; emit("gotv", { kind: "stats", used: status.ctx_used, budget: n }); } if (a.s && a.s.max_steps) status.max_steps = a.s.max_steps; if (a.s && "desktop_control" in a.s) status.desktop_control = a.s.desktop_control; if (a.s && "sandbox" in a.s) status.sandbox = a.s.sandbox; if (a.s && a.s.net_allow) status.net_allow = a.s.net_allow; return;
          case "open_app_management": return;
          case "desktop_status": return { accessibility: { state: "granted" }, screen_recording: { state: "denied" } };
          case "desktop_permissions": return;
          default: return null;
        }
      },
    },
  };
  function demoTurn(task) {
    const lines = [
      "· paso 1: read_file  src/auth.rs", "  ✓ [src/auth.rs · líneas 1–40 de 40]",
      "· paso 2: edit  src/auth.rs", "  ✓ editado src/auth.rs (1 reemplazo(s))",
      "  ┌ src/auth.rs (editado)", "  │ @@ -12,3 +12,4 @@", "  │-    true", "  │+    argon2_verify(pass, &stored)", "  [LSP] src/auth.rs: sin errores",
      "  ↩ paso #4: 1 archivo(s) — `gotv rewind` lo deshace",
    ];
    lines.forEach((t, i) => setTimeout(() => emit("gotv", { kind: "line", text: t }), 120 * i));
    // como el núcleo: tokens del turno (A2) y velocidad en vivo (A1)
    setTimeout(() => emit("gotv", { kind: "usage", usage: { depth: 0, in: 12840, out: 412, cached: 11900, exact: true } }), 600);
    const t0 = tasks.find((x) => x.kind === "turn"); if (t0) t0.tokens = { depth: 0, in: 12840, out: 412, cached: 11900, exact: true };
    setTimeout(() => emit("gotv", { kind: "tasks", tasks }), 650);
    [1500, 2000].forEach((ms, i) => setTimeout(() => emit("gotv", { kind: "rate", tok_s: [11.8, 12.4][i] }), ms));
    const ans = "## Listo\n\n`check_credentials` ahora verifica el hash con **argon2**. Cambios:\n\n- `src/auth.rs`: verificación con *argon2*\n- pruebas nuevas:\n  - [x] hash válido\n  - [ ] hash vacío\n\n| archivo | líneas |\n|---|---:|\n| src/auth.rs | +1 −1 |\n\n```rust\nfn check(p: &str) -> bool {\n    argon2_verify(p, &stored)\n}\n```\n\n> Compila sin errores. Docs: https://docs.rs/argon2";
    // en trozos y EN ORDEN, como el núcleo: el «done» sale después del último trozo
    const parts = ans.match(/[\s\S]{1,6}/g);
    let k = 0;
    setTimeout(function step() {
      if (k < parts.length) { emit("gotv", { kind: "delta", text: parts[k++] }); setTimeout(step, 25); return; }
      emit("gotv", { kind: "stats", used: 15800, budget: 40960 }); emit("gotv", { kind: "done", text: ans });
    }, 1400);
  }
  // F7 · `?plan`: el agente presenta un plan (exit_plan) → tarjeta comentable + la pregunta, como el núcleo
  window.__mockPlan = (version = 1) => {
    const q = "¿Apruebas este plan?\n\n" + PLAN_TXT;
    emit("gotv", { kind: "vista", tipo: "plan", titulo: `Plan · versión ${version}`, sesion: "12", archivo: "plan.md", version, ruta: "/p/.gotv/chats/12/plan.md", texto: PLAN_TXT, comentarios: planComs.slice(), pregunta: q });
    emit("gotv", { kind: "choose", id: 900 + version, question: q, options: ["Sí, ejecútalo", "No, sigue planeando"] });
  };
  if (location.search.includes("plan")) setTimeout(() => window.__mockPlan(), 700);
  if (SIN) setTimeout(() => { emit("gotv", { kind: "status", status: { ...status, root: "", ready: true } }); emit("gotv", { kind: "noproject" }); }, 600);
  else setTimeout(() => emit("gotv", { kind: "ready" }), 300);
})();
