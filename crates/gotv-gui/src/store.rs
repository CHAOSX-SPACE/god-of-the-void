//! Lo que la GUI recuerda entre sesiones (`~/.config/gotv/gui.gotv`, formato propio): proyectos
//! recientes, carpetas en las que el usuario CONFÍA y el tema. Va aparte de
//! `config.toml` porque ese archivo rechaza llaves desconocidas (deny_unknown_fields).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Project {
    pub path: String,
    pub name: String,
    /// segundos unix de la última apertura
    pub last: u64,
    /// F5 de la memoria rápida: el modelo de ESTE proyecto (ruta o `proveedor:modelo`). Al abrirlo se
    /// carga éste; `None` = el último que elegiste.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Cómo dejaste el chat de ESTE proyecto (Control Mac, perfil, manos libres): al reabrir la app o
    /// volver al proyecto queda igual (pedido del Portador 2026-11-11: antes Control Mac se apagaba).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<Chat>,
    /// La ventana de contexto de ESTE proyecto (un preset: «auto» = el máximo que cabe, «32k», «64k»…
    /// o un número): cada proyecto puede usar otra IA y otra ventana (pedido del Portador 2026-11-13).
    /// `None` = la de la configuración.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contexto: Option<String>,
}

/// Los controles del chat que se recuerdan por proyecto.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Chat {
    #[serde(default)]
    pub mac: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perfil: Option<String>,
    #[serde(default)]
    pub manos: bool,
    /// Ola PRO: los paneles del modo profesional (`{"on": {"video": true}, "video": {…}}`), tal como
    /// los dejó la interfaz; el núcleo sólo lee lo que viaja con cada orden
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pro: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Store {
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub trusted: Vec<String>,
    #[serde(default = "default_theme")]
    pub theme: String,
    /// El modelo que el usuario ELIGIÓ en la app (Configuración → Modelo): al reabrir,
    /// manda sobre `prefer`. Sólo lo escribe una elección explícita, nunca un arranque
    /// automático (medido 2026-11-09: el arranque con `prefer` pisaba la elección).
    #[serde(default)]
    pub model: Option<String>,
}

fn default_theme() -> String {
    "dark".into()
}

impl Default for Store {
    fn default() -> Self {
        Store { projects: vec![], trusted: vec![], theme: default_theme(), model: None }
    }
}

/// Formato propio (códec, GUIS); hasta 2026-11-17 era `gui.json` (se migra solo la primera vez, y el
/// viejo queda como `gui.json.migrado`).
pub fn path() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/gotv/gui.gotv")
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Store {
    pub fn load() -> Store {
        let p = path();
        gotv_core::programar::codec::leer_serde_migrado(&p, &p.with_extension("json"), b"GUIS").unwrap_or_default()
    }

    pub fn save(&self) {
        let _ = gotv_core::programar::codec::guardar_serde(&path(), b"GUIS", self);
    }

    pub fn is_trusted(&self, p: &Path) -> bool {
        let s = p.to_string_lossy();
        self.trusted.iter().any(|t| t == &s)
    }

    pub fn trust(&mut self, p: &Path) {
        let s = p.to_string_lossy().to_string();
        if !self.trusted.contains(&s) {
            self.trusted.push(s);
        }
    }

    /// Lo pone primero en recientes (máx. 30). Conserva el nombre que el usuario le
    /// dio; si no tiene, usa el de la carpeta.
    pub fn touch(&mut self, p: &Path) {
        self.touch_named(p, None);
    }

    /// Como `touch`, fijando el nombre que se ve en la app (la carpeta no cambia).
    pub fn touch_named(&mut self, p: &Path, name: Option<&str>) {
        // una charla sin proyecto NUNCA es un proyecto reciente (medido por el Portador 2026-11-18: cambiar el
        // modelo dentro de una charla la metía en la lista como «20261118-…»)
        if gotv_core::sueltas::es_suelta(p) {
            return;
        }
        let s = p.to_string_lossy().to_string();
        let previo = self.projects.iter().find(|x| x.path == s).cloned();
        self.projects.retain(|x| x.path != s);
        let name = name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(String::from)
            .or(previo.as_ref().map(|x| x.name.clone()))
            .unwrap_or_else(|| p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| s.clone()));
        // reabrir conserva el modelo, el chat y el contexto del proyecto
        let (model, chat, contexto) = previo.map(|x| (x.model, x.chat, x.contexto)).unwrap_or_default();
        self.projects.insert(0, Project { path: s, name, last: now(), model, chat, contexto });
        self.projects.truncate(30);
    }

    /// Cómo quedó el chat de un proyecto (por omisión: todo apagado, perfil automático).
    pub fn project_chat(&self, p: &Path) -> Chat {
        let s = p.to_string_lossy();
        self.projects.iter().find(|x| x.path == s).and_then(|x| x.chat.clone()).unwrap_or_default()
    }

    /// Guarda cómo quedó el chat de un proyecto (sólo de uno ya conocido: no inventa recientes).
    pub fn set_project_chat(&mut self, p: &Path, chat: Chat) {
        let s = p.to_string_lossy();
        if let Some(x) = self.projects.iter_mut().find(|x| x.path == s) {
            x.chat = Some(Chat { perfil: chat.perfil.filter(|x| !x.trim().is_empty()), ..chat });
        }
    }

    /// El modelo de un proyecto, si se le eligió uno.
    pub fn project_model(&self, p: &Path) -> Option<String> {
        let s = p.to_string_lossy();
        self.projects.iter().find(|x| x.path == s).and_then(|x| x.model.clone()).filter(|m| !m.trim().is_empty())
    }

    /// Fija el modelo de un proyecto (y lo deja como «el último que elegiste» para los nuevos).
    pub fn set_project_model(&mut self, p: &Path, m: &str) {
        let s = p.to_string_lossy().to_string();
        if !self.projects.iter().any(|x| x.path == s) {
            self.touch(p);
        }
        if let Some(x) = self.projects.iter_mut().find(|x| x.path == s) {
            x.model = Some(m.to_string());
        }
        self.model = Some(m.to_string());
    }

    /// La ventana de contexto de un proyecto (un preset), si se le eligió una.
    pub fn project_context(&self, p: &Path) -> Option<String> {
        let s = p.to_string_lossy();
        self.projects.iter().find(|x| x.path == s).and_then(|x| x.contexto.clone()).filter(|c| !c.trim().is_empty())
    }

    /// Fija la ventana de contexto de un proyecto.
    pub fn set_project_context(&mut self, p: &Path, preset: &str) {
        let s = p.to_string_lossy().to_string();
        if !self.projects.iter().any(|x| x.path == s) {
            self.touch(p);
        }
        if let Some(x) = self.projects.iter_mut().find(|x| x.path == s) {
            x.contexto = Some(preset.trim().to_string()).filter(|c| !c.is_empty());
        }
    }

    /// Recientes que todavía existen.
    pub fn live_projects(&self) -> Vec<Project> {
        // las charlas que ya se hubieran colado en la lista no se enseñan (no se borra nada)
        self.projects.iter().filter(|p| Path::new(&p.path).is_dir() && !gotv_core::sueltas::es_suelta(Path::new(&p.path))).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Una charla sin proyecto no entra en los recientes, ni al abrirla ni al cambiarle el modelo o la ventana
    /// (pasaba: «20261118-…» en la lista); el modelo elegido sigue valiendo como preferencia general.
    #[test]
    fn una_charla_no_es_un_proyecto_reciente() {
        let charla = gotv_core::sueltas::base().join("20261118-000000-prueba-store");
        let mut st = Store::default();
        st.touch(&charla);
        st.set_project_model(&charla, "/m/huihui");
        st.set_project_context(&charla, "64k");
        assert!(st.projects.is_empty(), "{:?}", st.projects.iter().map(|p| &p.path).collect::<Vec<_>>());
        assert_eq!(st.model.as_deref(), Some("/m/huihui"), "la elección queda como preferencia");
        // un proyecto de verdad sí entra
        st.touch(Path::new("/x/web"));
        assert_eq!(st.projects.len(), 1);
    }

    /// El `gui.json` de antes (proyectos, confianza, tema, modelo, chat por proyecto) se migra al formato
    /// propio sin perder nada; la segunda lectura ya es de lo propio.
    #[test]
    fn el_gui_json_viejo_se_migra_entero() {
        use gotv_core::programar::codec;
        let d = std::env::temp_dir().join(format!("gotv-gui-migrar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let (propio, viejo) = (d.join("gui.gotv"), d.join("gui.json"));
        std::fs::write(
            &viejo,
            r#"{"projects":[{"path":"/p/web","name":"Web","last":7,"model":"/m/32B","chat":{"mac":true,"manos":false},"contexto":"64k"}],"trusted":["/p/web"],"theme":"light","model":"/m/9B"}"#,
        )
        .unwrap();
        let st: Store = codec::leer_serde_migrado(&propio, &viejo, b"GUIS").unwrap();
        assert_eq!(st.projects[0].model.as_deref(), Some("/m/32B"));
        assert!(st.projects[0].chat.as_ref().is_some_and(|c| c.mac) && st.projects[0].contexto.as_deref() == Some("64k"));
        assert!(st.is_trusted(Path::new("/p/web")) && st.theme == "light" && st.model.as_deref() == Some("/m/9B"));
        assert!(propio.is_file() && d.join("gui.json.migrado").is_file() && !viejo.exists());
        let otra: Store = codec::leer_serde_migrado(&propio, &viejo, b"GUIS").unwrap();
        assert_eq!(otra.projects[0].name, "Web");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// El nombre es SÓLO de la app: la ruta queda tal cual y el nombre sobrevive a reabrir.
    #[test]
    fn el_nombre_es_de_la_app_y_la_ruta_no_cambia() {
        let mut st = Store::default();
        let p = Path::new("/Users/x/Documents/proyectos/web");
        st.touch_named(p, Some("  Tienda  "));
        assert_eq!((st.projects[0].path.as_str(), st.projects[0].name.as_str()), ("/Users/x/Documents/proyectos/web", "Tienda"));
        st.touch(p); // reabrir no le quita el nombre
        assert_eq!(st.projects[0].name, "Tienda");
        st.touch_named(Path::new("/a/sin-nombre"), Some("   "));
        assert_eq!(st.projects[0].name, "sin-nombre", "vacío = el de la carpeta");
        assert_eq!(st.projects.len(), 2);
    }

    /// F5: cada proyecto recuerda su modelo; reabrirlo (o renombrarlo) no lo pierde; un gui.json viejo
    /// (sin el campo) se lee igual.
    #[test]
    fn cada_proyecto_recuerda_su_modelo() {
        let mut st: Store = serde_json::from_str(r#"{"projects":[{"path":"/a/web","name":"Web","last":1}],"model":"/m/9B"}"#).unwrap();
        let web = Path::new("/a/web");
        assert_eq!(st.project_model(web), None, "un gui.json viejo no trae modelo por proyecto");
        st.set_project_model(web, "/m/32B");
        st.set_project_model(Path::new("/a/api"), "/m/0.6B");
        assert_eq!(st.project_model(web).as_deref(), Some("/m/32B"));
        assert_eq!(st.model.as_deref(), Some("/m/0.6B"), "el último elegido es el de los nuevos");
        st.touch(web);
        st.touch_named(web, Some("Tienda"));
        assert_eq!(st.project_model(web).as_deref(), Some("/m/32B"), "reabrir y renombrar no lo pierden");
        let vuelta: Store = serde_json::from_str(&serde_json::to_string(&st).unwrap()).unwrap();
        assert_eq!(vuelta.project_model(Path::new("/a/api")).as_deref(), Some("/m/0.6B"));
    }

    /// Cada proyecto recuerda su ventana de contexto (puede usar otra IA): sobrevive a reabrir, a
    /// renombrar y a guardar/leer; un gui.json viejo no trae ninguna; uno no cuenta en otro.
    #[test]
    fn cada_proyecto_recuerda_su_contexto() {
        let mut st: Store = serde_json::from_str(r#"{"projects":[{"path":"/a/web","name":"Web","last":1}]}"#).unwrap();
        let (web, api) = (Path::new("/a/web"), Path::new("/a/api"));
        assert_eq!(st.project_context(web), None, "un gui.json viejo no trae contexto por proyecto");
        st.set_project_context(web, "64k");
        st.set_project_context(api, " auto ");
        st.touch(web);
        st.touch_named(web, Some("Tienda"));
        assert_eq!(st.project_context(web).as_deref(), Some("64k"), "reabrir y renombrar no lo pierden");
        assert_eq!(st.project_context(api).as_deref(), Some("auto"));
        let vuelta: Store = serde_json::from_str(&serde_json::to_string(&st).unwrap()).unwrap();
        assert_eq!(vuelta.project_context(web).as_deref(), Some("64k"));
        st.set_project_context(api, "  ");
        assert_eq!(st.project_context(api), None, "vacío = el de la configuración");
    }

    /// Cada proyecto recuerda su chat (Control Mac, perfil, manos libres): sobrevive a reabrir, a
    /// renombrar y a guardar/leer el archivo; un gui.json viejo da todo apagado; uno no cuenta en otro.
    #[test]
    fn cada_proyecto_recuerda_su_chat() {
        let mut st: Store = serde_json::from_str(r#"{"projects":[{"path":"/a/php","name":"Php","last":1},{"path":"/a/web","name":"Web","last":2}]}"#).unwrap();
        let (php, web) = (Path::new("/a/php"), Path::new("/a/web"));
        assert_eq!(st.project_chat(php), Chat::default(), "un gui.json viejo: todo apagado");
        st.set_project_chat(php, Chat { mac: true, perfil: Some("diseno".into()), manos: true, pro: None });
        st.set_project_chat(web, Chat { mac: false, perfil: Some("  ".into()), manos: false, pro: None });
        st.touch(php);
        st.touch_named(php, Some("Php nuevo"));
        let vuelta: Store = serde_json::from_str(&serde_json::to_string(&st).unwrap()).unwrap();
        assert_eq!(vuelta.project_chat(php), Chat { mac: true, perfil: Some("diseno".into()), manos: true, pro: None });
        assert_eq!(vuelta.project_chat(web), Chat::default(), "el de un proyecto no pasa al otro (y un perfil vacío es «sin perfil»)");
        st.set_project_chat(Path::new("/no/abierto"), Chat { mac: true, ..Default::default() });
        assert_eq!(st.projects.len(), 2, "no inventa recientes");
        // Ola PRO: los paneles del modo profesional viajan con el chat del proyecto y vuelven igual
        let pro = serde_json::json!({"on": {"video": true}, "video": {"formato": "9:16", "movimiento": "orbita"}});
        st.set_project_chat(web, Chat { pro: Some(pro.clone()), ..st.project_chat(web) });
        let vuelta: Store = serde_json::from_str(&serde_json::to_string(&st).unwrap()).unwrap();
        assert_eq!(vuelta.project_chat(web).pro, Some(pro));
        assert_eq!(vuelta.project_chat(php).pro, None, "el de un proyecto no pasa al otro");
    }
}
