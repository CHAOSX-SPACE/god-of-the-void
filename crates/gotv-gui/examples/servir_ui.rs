//! Servidor de archivos MÍNIMO para ver la interfaz en un navegador con el simulador (`ui/mock.js`).
//! Sólo para desarrollo, en Rust puro y sin dependencias (el Portador: nada de Python). Escucha sólo
//! en 127.0.0.1 y no sale de la carpeta que sirve.
//!
//! `cargo run --release -p gotv-gui --example servir_ui -- 8799 crates/gotv-gui/ui`

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Component, Path, PathBuf};

fn tipo(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let puerto: u16 = args.get(1).and_then(|p| p.parse().ok()).unwrap_or(8799);
    let raiz = PathBuf::from(args.get(2).map(String::as_str).unwrap_or("crates/gotv-gui/ui")).canonicalize()?;
    let l = TcpListener::bind(("127.0.0.1", puerto))?;
    eprintln!("sirviendo {} en http://127.0.0.1:{puerto}", raiz.display());
    // un hilo por conexión y tope de lectura: un navegador abre sockets de pre-conexión que no mandan
    // nada, y con un solo hilo sin tope el servidor quedaba bloqueado para siempre (medido 2026-11-15)
    for c in l.incoming().flatten() {
        let raiz = raiz.clone();
        std::thread::spawn(move || atender(c, &raiz));
    }
    Ok(())
}

fn atender(mut c: std::net::TcpStream, raiz: &Path) {
    {
        let _ = c.set_read_timeout(Some(std::time::Duration::from_secs(5)));
        let mut linea = String::new();
        if BufReader::new(&c).read_line(&mut linea).is_err() {
            return;
        }
        let ruta = linea.split_whitespace().nth(1).unwrap_or("/").split(['?', '#']).next().unwrap_or("/").to_string();
        let rel = Path::new(ruta.trim_start_matches('/'));
        // nunca fuera de la carpeta: nada de «..» ni rutas absolutas
        let sano = rel.components().all(|x| matches!(x, Component::Normal(_)));
        let mut p = raiz.join(rel);
        if p.is_dir() {
            p = p.join("index.html");
        }
        let (estado, cuerpo, ct) = match (sano, std::fs::read(&p)) {
            (true, Ok(b)) => ("200 OK", b, tipo(&p)),
            _ => ("404 Not Found", b"no encontrado".to_vec(), "text/plain"),
        };
        let _ = write!(c, "HTTP/1.1 {estado}\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", cuerpo.len());
        let _ = c.write_all(&cuerpo);
    }
}
