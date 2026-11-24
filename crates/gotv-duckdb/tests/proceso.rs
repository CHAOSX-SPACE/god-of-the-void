//! DuckDB por su proceso aparte, como lo usa la app: el cliente de `gotv-datos` lanza el binario real.
#[cfg(test)]
mod proceso {
    use gotv_datos::duckdb::Duck;
    use gotv_datos::{Limites, Valor};
    use std::path::Path;

    fn bin() -> &'static Path {
        Path::new(env!("CARGO_BIN_EXE_gotv-duckdb"))
    }

    fn proyecto() -> gotv_engine::prueba::CarpetaPrueba {
        let d = gotv_engine::prueba::CarpetaPrueba::nueva("gotv-duckdb-proceso");
        std::fs::create_dir_all(d.join("datos")).unwrap();
        let mut csv = String::from("id,ciudad,monto\n");
        for i in 1..=300 {
            csv.push_str(&format!("{i},{},{}.5\n", ["Cali", "Pasto", "Neiva"][i % 3], i));
        }
        std::fs::write(d.join("datos/v.csv"), csv).unwrap();
        d
    }

    /// CSV con SQL por el proceso, con los topes de memoria e hilos puestos y la cerca intacta.
    #[test]
    fn consulta_por_el_proceso_con_cerca_y_topes() {
        let d = proyecto();
        let mut k = Duck::abrir_con(bin(), &d, None, false).unwrap();
        assert!(k.version.starts_with('v'), "{}", k.version);
        let r = k.consultar("SELECT ciudad, count(*) AS n, sum(monto) AS t FROM 'datos/v.csv' GROUP BY ciudad ORDER BY ciudad", &Limites::default()).unwrap();
        assert_eq!(r.filas.len(), 3);
        assert_eq!(r.filas[0][0], Valor::Texto("Cali".into()));
        assert!(matches!(r.filas[0][2], Valor::Real(_)), "{:?}", r.filas[0]);
        assert!(k.columnas_de_archivo("datos/v.csv").unwrap().contains("(300 filas): id BIGINT"));
        // los topes del proceso: hilos = la mitad de los núcleos; memoria lejos del 80 % por omisión
        let hilos = std::thread::available_parallelism().map(|n| n.get() / 2).unwrap_or(2).max(1);
        let r = k.consultar("SELECT current_setting('threads') AS h, current_setting('memory_limit') AS m", &Limites::default()).unwrap();
        assert_eq!(r.filas[0][0].to_string(), hilos.to_string(), "{:?}", r.filas);
        // la cerca: nada fuera del proyecto
        let e = k.consultar("SELECT * FROM read_text('/etc/hosts')", &Limites::default());
        assert!(e.is_err(), "{:?}", e.map(|r| r.filas));
        // un error no mata la conexión: la siguiente consulta sigue
        assert!(k.consultar("SELECT count(*) FROM 'datos/v.csv'", &Limites::default()).is_ok());
    }

    /// Una escritura en dos tiempos: lo que haría, la decisión de la app, y COMMIT o ROLLBACK.
    #[test]
    fn escritura_en_dos_tiempos() {
        let d = proyecto();
        let f = d.join("datos/a.duckdb");
        {
            let mut k = Duck::abrir_con(bin(), &d, Some(&f), true).unwrap();
            assert!(k.ejecutar("CREATE TABLE v AS SELECT * FROM 'datos/v.csv'", |_| true).unwrap().confirmada);
            let mut vio = 0;
            let e = k.ejecutar("DELETE FROM v WHERE id > 290", |e| {
                vio = e.filas;
                false
            }).unwrap();
            assert_eq!((vio, e.filas, e.confirmada), (10, 10, false));
            assert!(k.ejecutar("DELETE FROM v WHERE id > 290", |_| true).unwrap().confirmada);
            assert!(k.ejecutar("SELECT 1", |_| true).unwrap_err().to_string().contains("lectura"));
        }
        let mut k = Duck::abrir_con(bin(), &d, Some(&f), false).unwrap();
        assert!(k.esquema(None).unwrap().contains("table v (290 filas)"), "{}", k.esquema(None).unwrap());
        assert!(k.ejecutar("DELETE FROM v", |_| true).is_err(), "abierta en sólo lectura");
    }

    /// Soltar la conexión cierra el proceso (la memoria vuelve) y un binario que falta da un error claro.
    #[test]
    fn el_proceso_muere_al_soltar_y_sin_binario_hay_error() {
        let d = proyecto();
        let k = Duck::abrir_con(bin(), &d, None, false).unwrap();
        let pid = k.pid() as i32;
        // SAFETY: kill con señal 0 sólo pregunta si el proceso existe
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "vivo mientras dura la conexión");
        drop(k);
        assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "soltar la conexión cierra el proceso");
        let e = Duck::abrir_con(Path::new("/no/existe/gotv-duckdb"), &d, None, false).err().unwrap().to_string();
        assert!(e.contains("/no/existe/gotv-duckdb"), "{e}");
    }

}
