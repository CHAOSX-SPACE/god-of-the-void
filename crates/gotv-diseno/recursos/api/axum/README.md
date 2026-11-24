# __MARCA__ · API

⟦API REST de «__PLURAL__» hecha con GOD OF THE VOID (Axum + Tokio, versiones fijas).¦REST API for «__PLURAL__» made with GOD OF THE VOID (Axum + Tokio, pinned versions).⟧

```bash
cargo test       # ⟦pruebas¦tests⟧
cargo run        # http://localhost:8000
```

| ⟦Método¦Method⟧ | ⟦Ruta¦Path⟧ | ⟦Qué hace¦What it does⟧ |
|---|---|---|
| GET | `/salud` | ⟦está viva¦it is alive⟧ |
| GET | `/__RUTA__` | ⟦lista¦list⟧ |
| GET | `/__RUTA__/{id}` | ⟦uno (404 si no existe)¦one (404 if missing)⟧ |
| POST | `/__RUTA__` | ⟦crea (201; 422 con `errores` por campo)¦creates (201; 422 with `errores` per field)⟧ |
| PUT | `/__RUTA__/{id}` | ⟦cambia¦changes⟧ |
| DELETE | `/__RUTA__/{id}` | ⟦borra (204)¦deletes (204)⟧ |

- `src/lib.rs`: ⟦configuración, datos con su validación, almacén (en memoria: cambia `Repositorio` para una base de datos) y rutas.¦configuration, data with its validation, store (in memory: change `Repositorio` for a database) and routes.⟧
- `src/main.rs`: ⟦el arranque.¦startup.⟧ `PUERTO` ⟦y¦and⟧ `ORIGENES`: `.env.example`.
