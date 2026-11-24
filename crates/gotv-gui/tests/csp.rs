//! La CSP de la ventana (`tauri.conf.json`) también manda DENTRO de las vistas: un `<iframe srcdoc>`
//! hereda la política de la página que lo contiene. Medido 2026-11-12: sin `media-src … data:` el
//! reproductor de la música daba «error» (MEDIA_ERR_SRC_NOT_SUPPORTED) aunque el WAV estuviera bien,
//! y sin `font-src … data:` las vistas de página perdían sus fuentes incrustadas en silencio.

#[test]
fn la_ventana_deja_pasar_lo_que_usan_las_vistas() {
    let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    let csp = conf["app"]["security"]["csp"].as_str().unwrap();
    let directiva = |nombre: &str| {
        csp.split(';').map(str::trim).find(|d| d.starts_with(nombre)).unwrap_or_else(|| panic!("{nombre} missing in «{csp}»")).to_string()
    };
    // el reproductor (música y efectos), las fuentes de las páginas y las imágenes: todo va como data:
    for d in ["media-src", "font-src", "img-src"] {
        assert!(directiva(d).split_whitespace().any(|x| x == "data:"), "{d} lacks data: in «{csp}»");
    }
    // M6: el video del chat se pide al verlo (`leer_video`) y se reproduce desde una URL blob:
    assert!(directiva("media-src").split_whitespace().any(|x| x == "blob:"), "media-src lacks blob: in «{csp}»");
    // y la vista previa de la UI dice lo mismo que la ventana (`CSP_VISTA` en app.js)
    let app = include_str!("../ui/app.js");
    assert!(app.contains("media-src data:") && app.contains("font-src data:"));
}

/// F8.7: la pestaña «Vista» enseña el servidor del proyecto en un marco: sólo los de ESTA Mac.
#[test]
fn la_vista_del_servidor_solo_enmarca_esta_mac() {
    let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    let csp = conf["app"]["security"]["csp"].as_str().unwrap();
    let frame = csp.split(';').map(str::trim).find(|d| d.starts_with("frame-src")).expect("frame-src");
    let fuentes: Vec<&str> = frame.split_whitespace().skip(1).collect();
    assert!(fuentes.contains(&"http://localhost:*") && fuentes.contains(&"http://127.0.0.1:*"), "{frame}");
    assert!(fuentes.iter().all(|f| *f == "'self'" || f.contains("://localhost:") || f.contains("://127.0.0.1:")), "nada de fuera: {frame}");
}
