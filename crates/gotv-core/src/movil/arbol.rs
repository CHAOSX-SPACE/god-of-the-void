//! F18 · El árbol de la pantalla de un dispositivo, limpio y con referencias `@d1`, `@d2`… (como el
//! `browser`): sólo lo accionable, una línea por nodo, igual en iOS y en Android.
//!
//! - **iOS** (`accessibility_info` de idb, formato LEGACY: una lista JSON de elementos con `type`, `AXLabel`,
//!   `AXValue`, `AXUniqueId`, `frame` en PUNTOS y `enabled`). Trampa medida en la ficha: con el teclado en
//!   pantalla el árbol pasa de 16 a 46 nodos porque cada tecla es un botón → las teclas van en UNA línea.
//! - **Android** (`uiautomator dump`, XML con `bounds="[x1,y1][x2,y2]"` en PÍXELES). Tres trampas medidas:
//!   el texto vive en hijos NO tocables (se sube al padre tocable), la salida trae pegado al final
//!   `UI hierchary dumped to: /dev/tty` (se corta) y tarda ~3 s (el que llama lo sabe).
use anyhow::{anyhow, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use serde_json::Value;

/// Un elemento de la pantalla. `marco` = [x, y, ancho, alto] en las unidades con que se TOCA ese
/// dispositivo (puntos en iOS, píxeles en Android).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Nodo {
    /// `@d7`: sólo vale para ESTE árbol (se rehace en cada lectura).
    pub r: String,
    /// Tipo normalizado (Button, TextField, SecureTextField, Switch, Cell, Link, Slider, Tappable, Scroll, Text…).
    pub rol: String,
    pub texto: String,
    /// Lo escrito en un campo, o el estado de un interruptor.
    pub valor: String,
    pub id: String,
    pub marco: [i32; 4],
    pub activo: bool,
    pub editable: bool,
}

impl Nodo {
    pub fn centro(&self) -> (i32, i32) {
        (self.marco[0] + self.marco[2] / 2, self.marco[1] + self.marco[3] / 2)
    }

    pub fn linea(&self) -> String {
        let mut s = format!("{} {}", self.r, self.rol);
        if !self.texto.is_empty() {
            s.push_str(&format!(" «{}»", self.texto.chars().take(80).collect::<String>()));
        }
        if !self.valor.is_empty() {
            s.push_str(&format!(" = «{}»", self.valor.chars().take(80).collect::<String>()));
        }
        if !self.id.is_empty() {
            s.push_str(&format!(" #{}", self.id));
        }
        let [x, y, w, h] = self.marco;
        s.push_str(&format!(" [{x},{y} {w}×{h}]"));
        if !self.activo {
            s.push_str(crate::i18n::t(" (inactivo)", " (disabled)"));
        }
        s
    }
}

/// Lo leído, ya limpio.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Arbol {
    pub nodos: Vec<Nodo>,
    /// Teclas del teclado en pantalla agrupadas (0 = no hay teclado visible).
    pub teclas: usize,
    /// Nodos crudos antes de limpiar (para decir cuánto se ahorró).
    pub crudos: usize,
    /// iOS: la app que está al frente (la etiqueta del nodo `Application`); vacío si no se sabe.
    pub app: String,
}

impl Arbol {
    /// Pone las referencias `@d1…` en el orden de lectura.
    fn numerar(mut self) -> Self {
        for (i, n) in self.nodos.iter_mut().enumerate() {
            n.r = format!("@d{}", i + 1);
        }
        self
    }

    pub fn texto(&self) -> String {
        let mut l: Vec<String> = self.nodos.iter().map(Nodo::linea).collect();
        if self.teclas > 0 {
            l.push(crate::tr!("⌨ teclado visible ({} teclas; escribe con `escribir`, no tecla a tecla)", "⌨ keyboard visible ({} keys; type with `escribir`, not key by key)", self.teclas));
        }
        if l.is_empty() {
            return crate::i18n::t("(nada accionable en pantalla; prueba `todo: true` o una `captura`)", "(nothing actionable on screen; try `todo: true` or a `captura`)").into();
        }
        l.join("\n")
    }

    /// El nodo de una referencia `@dN`.
    pub fn buscar(&self, r: &str) -> Option<&Nodo> {
        self.nodos.iter().find(|n| n.r == r)
    }

    /// ¿Aparece el texto (sin mayúsculas) en algún nodo?
    pub fn contiene(&self, t: &str) -> bool {
        let t = t.to_lowercase();
        self.nodos.iter().any(|n| n.texto.to_lowercase().contains(&t) || n.valor.to_lowercase().contains(&t))
    }

    /// Los campos de texto (para `escribir` sin destino: vale sólo si hay UNO).
    pub fn campos(&self) -> Vec<&Nodo> {
        self.nodos.iter().filter(|n| n.editable).collect()
    }
}

fn s(v: &Value) -> String {
    v.as_str().map(str::trim).unwrap_or("").to_string()
}

/// Los elementos de idb: lista plana (LEGACY) o anidada (`children`).
fn aplanar<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
    match v {
        Value::Array(a) => a.iter().for_each(|x| aplanar(x, out)),
        Value::Object(o) => {
            out.push(v);
            if let Some(h) = o.get("children") {
                aplanar(h, out);
            }
        }
        _ => {}
    }
}

/// El rol normalizado de un elemento de iOS (por `type` y, si falta, por `role` AX).
fn rol_ios(e: &Value) -> Option<(&'static str, bool)> {
    let t = s(&e["type"]);
    let r = s(&e["role"]);
    Some(match (t.as_str(), r.as_str()) {
        ("Button", _) | (_, "AXButton") | ("PopUpButton", _) | (_, "AXPopUpButton") | ("MenuButton", _) => ("Button", false),
        ("Key", _) | (_, "AXKey") => ("Key", false),
        ("Link", _) | (_, "AXLink") => ("Link", false),
        ("SecureTextField", _) | (_, "AXSecureTextField") => ("SecureTextField", true),
        ("SearchField", _) | (_, "AXSearchField") => ("SearchField", true),
        ("TextField", _) | (_, "AXTextField") | ("TextView", _) | (_, "AXTextArea") => ("TextField", true),
        ("Switch", _) | ("Toggle", _) | (_, "AXSwitch") | ("CheckBox", _) | (_, "AXCheckBox") => ("Switch", false),
        ("Slider", _) | (_, "AXSlider") => ("Slider", false),
        ("Stepper", _) | (_, "AXIncrementor") => ("Stepper", false),
        ("Cell", _) | (_, "AXCell") => ("Cell", false),
        ("Tab", _) | ("TabBarButton", _) | (_, "AXTab") => ("Tab", false),
        ("SegmentedControl", _) | (_, "AXRadioGroup") | ("RadioButton", _) | (_, "AXRadioButton") => ("Segment", false),
        ("MenuItem", _) | (_, "AXMenuItem") => ("MenuItem", false),
        ("Picker", _) | ("PickerWheel", _) | ("ComboBox", _) | (_, "AXComboBox") => ("Picker", false),
        ("StaticText", _) | (_, "AXStaticText") | ("Heading", _) | (_, "AXHeading") => ("Text", false),
        _ => return None,
    })
}

fn marco_ios(e: &Value) -> [i32; 4] {
    let f = &e["frame"];
    let n = |k: &str| f[k].as_f64().unwrap_or(0.0).round() as i32;
    [n("x"), n("y"), n("width"), n("height")]
}

/// ¿Es una tecla del teclado en pantalla? (tipo `Key`, o un botón de una letra —o de las teclas con
/// nombre— en la franja baja de la pantalla).
fn es_tecla(n: &Nodo, alto: i32) -> bool {
    if n.rol == "Key" {
        return true;
    }
    if n.rol != "Button" || alto <= 0 || n.marco[1] < alto * 55 / 100 {
        return false;
    }
    const NOMBRES: &[&str] = &["shift", "delete", "more", "space", "return", "next keyboard", "dictate", "emoji", "globe", "mayús", "borrar", "espacio", "intro", "siguiente teclado", "dictar"]; // juez-idioma: datos (nombres de teclas de iOS en sus dos idiomas)
    let t = n.texto.to_lowercase();
    t.chars().count() <= 1 || NOMBRES.contains(&t.as_str())
}

/// El árbol de iOS desde el JSON de idb. `todo` = también los textos fijos.
pub fn de_ios(json: &str, todo: bool) -> Result<Arbol> {
    let v: Value = serde_json::from_str(json).map_err(|e| anyhow!("{}", crate::tr!("idb devolvió un árbol que no es JSON: {e}", "idb returned a tree that is not JSON: {e}")))?;
    let mut planos = Vec::new();
    aplanar(&v, &mut planos);
    let alto = planos
        .iter()
        .find(|e| s(&e["type"]) == "Application" || s(&e["role"]) == "AXApplication")
        .map(|e| marco_ios(e)[3])
        .unwrap_or_else(|| planos.iter().map(|e| { let m = marco_ios(e); m[1] + m[3] }).max().unwrap_or(0));
    let mut nodos = Vec::new();
    for e in &planos {
        let Some((rol, editable)) = rol_ios(e) else { continue };
        let marco = marco_ios(e);
        if marco[2] <= 0 || marco[3] <= 0 {
            continue;
        }
        let etiqueta = [s(&e["AXLabel"]), s(&e["title"]), s(&e["placeholder"])].into_iter().find(|t| !t.is_empty()).unwrap_or_default();
        if rol == "Text" && (!todo || etiqueta.is_empty()) {
            continue;
        }
        nodos.push(Nodo {
            r: String::new(),
            rol: rol.into(),
            texto: etiqueta,
            valor: match &e["AXValue"] {
                Value::String(t) => t.trim().to_string(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => String::new(),
            },
            id: s(&e["AXUniqueId"]),
            marco,
            activo: e["enabled"].as_bool().unwrap_or(true),
            editable,
        });
    }
    let teclas = nodos.iter().filter(|n| es_tecla(n, alto)).count();
    if teclas >= 10 {
        nodos.retain(|n| !es_tecla(n, alto));
    } else {
        nodos.iter_mut().filter(|n| n.rol == "Key").for_each(|n| n.rol = "Button".into());
    }
    let app = planos.iter().find(|e| s(&e["type"]) == "Application" || s(&e["role"]) == "AXApplication").map(|e| s(&e["AXLabel"])).unwrap_or_default();
    Ok(Arbol { crudos: planos.len(), teclas: if teclas >= 10 { teclas } else { 0 }, nodos, app }.numerar())
}

/// Corta lo que `uiautomator dump /dev/tty` pega antes y después del XML (el aviso
/// `UI hierchary dumped to: /dev/tty`, con su errata original).
pub fn recortar_volcado(salida: &str) -> Option<&str> {
    let ini = salida.find("<?xml").or_else(|| salida.find("<hierarchy"))?;
    let fin = salida.rfind("</hierarchy>")? + "</hierarchy>".len();
    (fin > ini).then(|| &salida[ini..fin])
}

fn atributo(e: &BytesStart, clave: &str) -> String {
    e.attributes()
        .flatten()
        .find(|a| a.key.local_name().as_ref() == clave)
        .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok().map(|v| v.trim().to_string()))
        .unwrap_or_default()
}

/// `[x1,y1][x2,y2]` → [x, y, ancho, alto].
fn limites(b: &str) -> [i32; 4] {
    let n: Vec<i32> = b.split(|c: char| !(c.is_ascii_digit() || c == '-')).filter(|t| !t.is_empty()).filter_map(|t| t.parse().ok()).collect();
    match n[..] {
        [x1, y1, x2, y2] => [x1, y1, x2 - x1, y2 - y1],
        _ => [0; 4],
    }
}

struct Crudo {
    padre: Option<usize>,
    clase: String,
    texto: String,
    desc: String,
    pista: String,
    id: String,
    marco: [i32; 4],
    tocable: bool,
    marcable: bool,
    marcado: bool,
    desliza: bool,
    activo: bool,
    secreto: bool,
}

fn rol_android(c: &Crudo) -> Option<(&'static str, bool)> {
    let k = c.clase.rsplit('.').next().unwrap_or("");
    if k.contains("EditText") || k.contains("AutoCompleteTextView") || k == "TextInputEditText" {
        return Some((if c.secreto { "SecureTextField" } else { "TextField" }, true));
    }
    if k.contains("Switch") || k.contains("ToggleButton") {
        return Some(("Switch", false));
    }
    if k.contains("CheckBox") {
        return Some(("CheckBox", false));
    }
    if k.contains("RadioButton") {
        return Some(("Segment", false));
    }
    if k.contains("SeekBar") || k.contains("Slider") {
        return Some(("Slider", false));
    }
    if c.tocable || c.marcable {
        return Some((if k.contains("Button") { "Button" } else { "Tappable" }, false));
    }
    if c.desliza {
        return Some(("Scroll", false));
    }
    None
}

/// El árbol de Android desde la salida de `uiautomator dump` (con o sin el aviso pegado).
pub fn de_android(salida: &str, todo: bool) -> Result<Arbol> {
    let xml = recortar_volcado(salida).ok_or_else(|| anyhow!("{}", crate::i18n::t("uiautomator no devolvió un árbol (¿la pantalla está bloqueada o la app es segura?)", "uiautomator returned no tree (is the screen locked or the app secure?)")))?;
    let mut r = Reader::from_str(xml);
    let mut crudos: Vec<Crudo> = Vec::new();
    let mut pila: Vec<usize> = Vec::new();
    loop {
        let ev = r.read_event().map_err(|e| anyhow!("uiautomator XML: {e}"))?;
        let (e, vacio) = match &ev {
            Event::Start(e) => (e, false),
            Event::Empty(e) => (e, true),
            Event::End(e) => {
                if e.local_name().as_ref() == "node" {
                    pila.pop();
                }
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        if e.local_name().as_ref() != "node" {
            continue;
        }
        let si = |k: &str| atributo(e, k) == "true";
        crudos.push(Crudo {
            padre: pila.last().copied(),
            clase: atributo(e, "class"),
            texto: atributo(e, "text"),
            desc: atributo(e, "content-desc"),
            pista: atributo(e, "hint"),
            id: atributo(e, "resource-id").rsplit(":id/").next().unwrap_or("").to_string(),
            marco: limites(&atributo(e, "bounds")),
            tocable: si("clickable") || si("long-clickable"),
            marcable: si("checkable"),
            marcado: si("checked"),
            desliza: si("scrollable"),
            activo: atributo(e, "enabled") != "false",
            secreto: si("password"),
        });
        if !vacio {
            pila.push(crudos.len() - 1);
        }
    }
    let roles: Vec<Option<(&'static str, bool)>> = crudos.iter().map(rol_android).collect();
    // el texto de los hijos NO accionables sube a su ancestro accionable más cercano (salvo a un Scroll,
    // que contiene media pantalla)
    let mut subido: Vec<Vec<String>> = vec![Vec::new(); crudos.len()];
    let mut sueltos: Vec<usize> = Vec::new();
    for (i, c) in crudos.iter().enumerate() {
        let propio = if c.texto.is_empty() { &c.desc } else { &c.texto };
        if roles[i].is_some() || propio.is_empty() {
            continue;
        }
        let mut p = c.padre;
        let mut dueno = None;
        while let Some(j) = p {
            if matches!(roles[j], Some((r, _)) if r != "Scroll") {
                dueno = Some(j);
                break;
            }
            p = crudos[j].padre;
        }
        match dueno {
            Some(j) => subido[j].push(propio.clone()),
            None => sueltos.push(i),
        }
    }
    let mut nodos = Vec::new();
    for (i, c) in crudos.iter().enumerate() {
        let (rol, editable) = match roles[i] {
            Some(x) => x,
            None if todo && sueltos.contains(&i) => ("Text", false),
            None => continue,
        };
        if c.marco[2] <= 0 || c.marco[3] <= 0 {
            continue;
        }
        let (texto, valor) = if editable {
            ([c.pista.clone(), c.desc.clone()].into_iter().find(|t| !t.is_empty()).unwrap_or_default(), c.texto.clone())
        } else {
            let propio = if c.texto.is_empty() { c.desc.clone() } else { c.texto.clone() };
            let t = if propio.is_empty() { subido[i].join(" · ") } else { propio };
            (t, if c.marcable { c.marcado.to_string() } else { String::new() })
        };
        nodos.push(Nodo { r: String::new(), rol: rol.into(), texto, valor, id: c.id.clone(), marco: c.marco, activo: c.activo, editable });
    }
    Ok(Arbol { crudos: crudos.len(), teclas: 0, nodos, app: String::new() }.numerar())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Un árbol de idb con la forma de su formato LEGACY (lista plana; marcos en puntos) de la app Ajustes
    /// en un iPhone de 402×874 puntos. NO es un volcado real: idb no está instalado en esta Mac (ver la
    /// entrega); la forma sigue a `accessibility_info` de idb.
    pub fn ajustes_ios(con_teclado: bool) -> String {
        let el = |tipo: &str, etiqueta: &str, valor: Option<&str>, id: Option<&str>, x: f64, y: f64, w: f64, h: f64| {
            serde_json::json!({"AXFrame": format!("{{{{{x}, {y}}}, {{{w}, {h}}}}}"), "AXUniqueId": id, "frame": {"x": x, "y": y, "width": w, "height": h},
                "role_description": tipo.to_lowercase(), "AXLabel": etiqueta, "content_required": false, "type": tipo, "title": null, "help": null,
                "custom_actions": [], "AXValue": valor, "enabled": true, "role": format!("AX{tipo}"), "subrole": null})
        };
        let mut v = vec![
            el("Application", "Ajustes", None, None, 0.0, 0.0, 402.0, 874.0),
            el("Heading", "Ajustes", None, None, 16.0, 100.0, 200.0, 41.0),
            el("SearchField", "Buscar", Some(if con_teclado { "ñandú" } else { "" }), Some("search"), 16.0, 150.0, 370.0, 36.0),
            el("Button", "Cuenta de Apple", None, Some("APPLE_ACCOUNT"), 16.0, 200.0, 370.0, 66.0),
            el("Cell", "General", None, Some("com.apple.settings.general"), 16.0, 290.0, 370.0, 52.0),
            el("Cell", "Accesibilidad", None, None, 16.0, 342.0, 370.0, 52.0),
            el("Switch", "Modo Avión", Some("0"), Some("airplane"), 16.0, 394.0, 370.0, 52.0),
            el("StaticText", "Wi‑Fi", None, None, 60.0, 460.0, 100.0, 22.0),
            el("Other", "", None, None, 0.0, 0.0, 402.0, 874.0),
        ];
        if con_teclado {
            for (i, k) in "qwertyuiopasdfghjklñzxcvbnm".chars().enumerate() {
                v.push(el("Button", &k.to_string(), None, None, 3.0 + (i % 10) as f64 * 39.0, 600.0 + (i / 10) as f64 * 54.0, 36.0, 46.0));
            }
            v.push(el("Button", "shift", None, None, 3.0, 708.0, 42.0, 46.0));
            v.push(el("Button", "delete", None, None, 357.0, 708.0, 42.0, 46.0));
            v.push(el("Button", "space", None, None, 100.0, 762.0, 200.0, 46.0));
        }
        serde_json::Value::Array(v).to_string()
    }

    #[test]
    fn ios_lo_accionable_con_referencias_y_el_teclado_en_una_linea() {
        let a = de_ios(&ajustes_ios(false), false).unwrap();
        let t = a.texto();
        assert_eq!(a.nodos.len(), 5, "{t}");
        assert!(t.starts_with("@d1 SearchField «Buscar» #search [16,150 370×36]"), "{t}");
        assert!(t.contains("@d4 Cell «Accesibilidad» [16,342 370×52]") && t.contains("@d5 Switch «Modo Avión» = «0» #airplane"), "{t}");
        assert!(!t.contains("Wi‑Fi") && !t.contains("teclado"), "sin `todo`, los textos fijos no salen: {t}");
        assert_eq!(a.buscar("@d3").unwrap().centro(), (201, 316));
        assert_eq!(a.campos().len(), 1);
        // con `todo`: los textos fijos con su rol Text
        assert!(de_ios(&ajustes_ios(false), true).unwrap().texto().contains("Text «Wi‑Fi»"));
        // con teclado: 30 teclas → UNA línea; el campo conserva lo escrito
        let k = de_ios(&ajustes_ios(true), false).unwrap();
        assert_eq!((k.crudos, k.nodos.len(), k.teclas), (39, 5, 30), "{}", k.texto());
        assert!(k.texto().contains("= «ñandú»") && k.texto().lines().last().unwrap().contains("30"), "{}", k.texto());
        assert!(k.contiene("ÑANDÚ") && !k.contiene("zz"));
    }

    #[test]
    fn ios_anidado_y_sin_aplicacion_tambien_se_lee() {
        let j = r#"{"type":"Window","frame":{"x":0,"y":0,"width":400,"height":800},"children":[{"type":"Button","AXLabel":"Hola","frame":{"x":10,"y":10,"width":100,"height":40},"enabled":false,"children":[]}]}"#;
        let a = de_ios(j, false).unwrap();
        assert_eq!(a.texto(), format!("@d1 Button «Hola» [10,10 100×40]{}", crate::i18n::t(" (inactivo)", " (disabled)")));
        assert!(de_ios("no es json", false).is_err());
    }

    /// Un volcado de `uiautomator dump /dev/tty` con la forma del real (Android 36): el texto en hijos no
    /// tocables, un campo con pista, un interruptor, y el aviso pegado al final con su errata.
    pub const VOLCADO_ANDROID: &str = concat!(
        "<?xml version='1.0' encoding='UTF-8' standalone='yes' ?><hierarchy rotation=\"0\">",
        "<node index=\"0\" text=\"\" resource-id=\"\" class=\"android.widget.FrameLayout\" package=\"com.ejemplo\" content-desc=\"\" checkable=\"false\" checked=\"false\" clickable=\"false\" enabled=\"true\" focusable=\"false\" focused=\"false\" scrollable=\"false\" long-clickable=\"false\" password=\"false\" selected=\"false\" bounds=\"[0,0][1080,2400]\">",
        "<node index=\"0\" text=\"Notas\" resource-id=\"com.ejemplo:id/titulo\" class=\"android.widget.TextView\" package=\"com.ejemplo\" content-desc=\"\" checkable=\"false\" checked=\"false\" clickable=\"false\" enabled=\"true\" focusable=\"false\" focused=\"false\" scrollable=\"false\" long-clickable=\"false\" password=\"false\" selected=\"false\" bounds=\"[42,150][300,220]\" />",
        "<node index=\"1\" text=\"\" resource-id=\"com.ejemplo:id/buscar\" class=\"android.widget.EditText\" package=\"com.ejemplo\" content-desc=\"\" checkable=\"false\" checked=\"false\" clickable=\"true\" enabled=\"true\" focusable=\"true\" focused=\"false\" scrollable=\"false\" long-clickable=\"true\" password=\"false\" selected=\"false\" hint=\"Buscar notas\" bounds=\"[42,250][1038,376]\" />",
        "<node index=\"2\" text=\"\" resource-id=\"com.ejemplo:id/lista\" class=\"androidx.recyclerview.widget.RecyclerView\" package=\"com.ejemplo\" content-desc=\"\" checkable=\"false\" checked=\"false\" clickable=\"false\" enabled=\"true\" focusable=\"true\" focused=\"false\" scrollable=\"true\" long-clickable=\"false\" password=\"false\" selected=\"false\" bounds=\"[0,400][1080,2000]\">",
        "<node index=\"0\" text=\"\" resource-id=\"\" class=\"android.widget.LinearLayout\" package=\"com.ejemplo\" content-desc=\"\" checkable=\"false\" checked=\"false\" clickable=\"true\" enabled=\"true\" focusable=\"true\" focused=\"false\" scrollable=\"false\" long-clickable=\"false\" password=\"false\" selected=\"false\" bounds=\"[0,400][1080,560]\">",
        "<node index=\"0\" text=\"Compras de mañana\" resource-id=\"com.ejemplo:id/nombre\" class=\"android.widget.TextView\" package=\"com.ejemplo\" content-desc=\"\" checkable=\"false\" checked=\"false\" clickable=\"false\" enabled=\"true\" focusable=\"false\" focused=\"false\" scrollable=\"false\" long-clickable=\"false\" password=\"false\" selected=\"false\" bounds=\"[42,420][800,480]\" />",
        "<node index=\"1\" text=\"hace 2 h\" resource-id=\"com.ejemplo:id/fecha\" class=\"android.widget.TextView\" package=\"com.ejemplo\" content-desc=\"\" checkable=\"false\" checked=\"false\" clickable=\"false\" enabled=\"true\" focusable=\"false\" focused=\"false\" scrollable=\"false\" long-clickable=\"false\" password=\"false\" selected=\"false\" bounds=\"[42,490][300,540]\" />",
        "</node>",
        "<node index=\"1\" text=\"\" resource-id=\"com.ejemplo:id/fijar\" class=\"android.widget.Switch\" package=\"com.ejemplo\" content-desc=\"Fijar arriba\" checkable=\"true\" checked=\"true\" clickable=\"true\" enabled=\"true\" focusable=\"true\" focused=\"false\" scrollable=\"false\" long-clickable=\"false\" password=\"false\" selected=\"false\" bounds=\"[900,600][1038,680]\" />",
        "</node>",
        "<node index=\"3\" text=\"\" resource-id=\"com.ejemplo:id/nueva\" class=\"android.widget.ImageButton\" package=\"com.ejemplo\" content-desc=\"Nueva nota\" checkable=\"false\" checked=\"false\" clickable=\"true\" enabled=\"false\" focusable=\"true\" focused=\"false\" scrollable=\"false\" long-clickable=\"false\" password=\"false\" selected=\"false\" bounds=\"[880,2100][1038,2258]\" />",
        "</node></hierarchy>UI hierchary dumped to: /dev/tty\n",
    );

    #[test]
    fn android_las_tres_trampas() {
        assert!(recortar_volcado(VOLCADO_ANDROID).unwrap().ends_with("</hierarchy>"));
        let a = de_android(VOLCADO_ANDROID, false).unwrap();
        let t = a.texto();
        assert_eq!(a.crudos, 9);
        let l: Vec<&str> = t.lines().collect();
        assert_eq!(l.len(), 5, "{t}");
        assert_eq!(l[0], "@d1 TextField «Buscar notas» #buscar [42,250 996×126]");
        assert_eq!(l[1], "@d2 Scroll #lista [0,400 1080×1600]");
        // el texto de los hijos subió al padre tocable; el Scroll no se lo queda
        assert_eq!(l[2], "@d3 Tappable «Compras de mañana · hace 2 h» [0,400 1080×160]");
        assert_eq!(l[3], "@d4 Switch «Fijar arriba» = «true» #fijar [900,600 138×80]");
        assert!(l[4].starts_with("@d5 Button «Nueva nota» #nueva [880,2100 158×158]") && l[4].ends_with(crate::i18n::t(" (inactivo)", " (disabled)")));
        assert!(!t.contains("Notas»"), "un texto suelto no sale sin `todo`");
        assert!(de_android(VOLCADO_ANDROID, true).unwrap().texto().contains("Text «Notas» #titulo"));
        assert_eq!(a.campos().len(), 1);
        assert_eq!(a.buscar("@d1").unwrap().centro(), (540, 313));
        assert!(de_android("ERROR: could not get idle state.", false).is_err());
    }
}
