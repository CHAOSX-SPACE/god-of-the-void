// Simulador de las órdenes de F16 SÓLO para ver la interfaz en un navegador (lo carga la página cuando no hay
// Tauri, después de mock.js). Envuelve el invoke del simulador: lo que no es de F16 sigue al de mock.js.
(() => {
  if (!window.__mockEmit) return;
  const emitir = window.__mockEmit;
  const previo = window.__TAURI__.core.invoke;
  let cola = [], nId = 1, nTer = 1;
  const vars = [{ nombre: "API_TOKEN", caja: false }];
  const archivos = { "src/main.rs": "fn main() {\n    println!(\"hola\");\n}\n" };
  const ver = (t) => String([...t].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7));
  // una «shell» de mentira para ver la pestaña: eco y unas pocas órdenes
  const terms = {};
  const vistaDe = (t) => ({ filas: t.lineas.slice(-t.filas).map((l, i, arr) => (i === arr.length - 1 ? [{ t: "$ ", fg: 2, b: true }, { t: t.linea }] : l)), cursor: [Math.min(t.lineas.length, t.filas) - 1, 2 + t.linea.length], cursor_visible: true, cols: 100 });
  const pintar = (id) => emitir({ kind: "terminal", id, vista: vistaDe(terms[id]) });
  const correr = (t, o) => {
    if (o === "ls") return [[{ t: "Cargo.toml  " }, { t: "src", fg: 4, b: true }]];
    if (o === "git status") return [[{ t: "On branch " }, { t: "main", fg: 2 }], [{ t: "nothing to commit, working tree clean" }]];
    if (o.startsWith("echo ")) return [[{ t: o.slice(5) }]];
    return o ? [[{ t: `zsh: command not found: ${o}`, fg: 1 }]] : [];
  };
  window.__TAURI__.core.invoke = async (cmd, a) => {
    switch (cmd) {
      case "mensaje_encolar": {
        const id = nId++;
        cola.push({ id, texto: a.texto, origen: "ventana" });
        emitir({ kind: "cola", pendientes: cola.slice() });
        // entra «entre dos pasos» un rato después
        setTimeout(() => { cola = cola.filter((x) => x.id !== id); emitir({ kind: "cola", pendientes: cola.slice() }); emitir({ kind: "line", text: `  ✉ entró tu mensaje: ${a.texto}` }); }, 2500);
        return id;
      }
      case "mensaje_quitar": { const n = cola.length; cola = cola.filter((x) => x.id !== a.id); emitir({ kind: "cola", pendientes: cola.slice() }); return cola.length !== n; }
      case "agentes_lista": return { agentes: [{ nombre: "revisor", descripcion: "revisa sin tocar", herramientas: ["read_file", "grep", "glob"], modelo: "anthropic:claude-haiku-5-5", archivo: ".gotv/agentes/revisor.md" }, { nombre: "docs", descripcion: "escribe documentación", herramientas: null, modelo: null, archivo: ".claude/agents/docs.md" }], malos: [] };
      case "entorno_lista": return vars.slice();
      case "entorno_guardar": { if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(a.nombre)) throw "nombre de variable inválido"; const i = vars.findIndex((v) => v.nombre === a.nombre); if (i >= 0) vars.splice(i, 1); vars.push({ nombre: a.nombre, caja: a.caja }); return; }
      case "entorno_caja": { const v = vars.find((x) => x.nombre === a.nombre); if (v) v.caja = a.caja; return; }
      case "entorno_quitar": { const i = vars.findIndex((v) => v.nombre === a.nombre); if (i >= 0) vars.splice(i, 1); return; }
      case "archivo_leer": { const t = archivos[a.ruta]; if (t == null) throw `${a.ruta}: No such file or directory`; return { ruta: a.ruta, texto: t, version: ver(t), lenguaje: a.ruta.split(".").pop() }; }
      case "archivo_guardar": { const t = archivos[a.ruta]; if (t != null && a.version && ver(t) !== a.version) throw "CONFLICTO: cambió desde que lo abriste"; archivos[a.ruta] = a.texto; return ver(a.texto); }
      case "terminal_abrir": { const id = nTer++; terms[id] = { lineas: [[{ t: "Last login: hoy en ttys001" }], []], linea: "", filas: a.filas }; setTimeout(() => pintar(id), 50); return id; }
      case "terminal_escribir": {
        const t = terms[a.id];
        if (!t) throw "no hay terminal";
        for (const c of a.datos) {
          if (c === "\r") { const o = t.linea.trim(); t.lineas[t.lineas.length - 1] = [{ t: "$ ", fg: 2, b: true }, { t: t.linea }]; t.lineas.push(...correr(t, o), []); t.linea = ""; }
          else if (c === "\x7f") t.linea = t.linea.slice(0, -1);
          else if (c >= " ") t.linea += c;
        }
        pintar(a.id);
        return;
      }
      case "terminal_tamano": if (terms[a.id]) { terms[a.id].filas = a.filas; pintar(a.id); } return;
      case "terminal_vista": return terms[a.id] ? vistaDe(terms[a.id]) : null;
      case "terminal_cerrar": delete terms[a.id]; return;
      case "remotos_lista": return [{ path: "/Users/demo/.config/gotv/remotos/nodo", conexion: "nodo", raiz: "/srv/app" }];
      case "remoto_crear": if (!String(a.raiz).startsWith("/")) throw "la raíz remota debe ser una ruta absoluta"; return "/Users/demo/.config/gotv/remotos/" + a.nombre;
      case "send":
        if (/^\/revisar/.test(a.task || "")) {
          setTimeout(() => {
            emitir({ kind: "revision", tipo: /seguridad/.test(a.task) ? "seguridad" : "codigo", hallazgos: [
              { id: 1, archivo: "src/calc.rs", linea: 2, severidad: "alta", resumen: "divide entre cero cuando la lista está vacía", como_falla: "promedio(&[]) → pánico", codigo: "    let (s, n) = (v.iter().sum::<i32>(), v.len() as i32); s / n" },
              { id: 2, archivo: "src/api.rs", linea: 41, severidad: "media", resumen: "el error de la base se traga y responde 200", como_falla: "Postgres caído → la API dice «ok» con la lista vacía", codigo: "        Err(_) => Ok(vec![])," },
            ], descartados: ["src/fantasma.rs: no existe en el proyecto"], sin_json: false });
            emitir({ kind: "done", text: "" });
          }, 900);
          return;
        }
        return previo(cmd, a);
      default: return previo(cmd, a);
    }
  };
})();
