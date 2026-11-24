# __MARCA__ · API

⟦API REST de «__PLURAL__» hecha con GOD OF THE VOID (FastAPI + Pydantic, versiones fijas).¦REST API for «__PLURAL__» made with GOD OF THE VOID (FastAPI + Pydantic, pinned versions).⟧

```bash
uv sync
uv run pytest              # ⟦pruebas¦tests⟧
uv run python -m app       # http://localhost:8000  (⟦documentación en¦docs at⟧ /docs)
```

| ⟦Método¦Method⟧ | ⟦Ruta¦Path⟧ | ⟦Qué hace¦What it does⟧ |
|---|---|---|
| GET | `/salud` | ⟦está viva¦it is alive⟧ |
| GET | `/__RUTA__` | ⟦lista¦list⟧ |
| GET | `/__RUTA__/{id}` | ⟦uno (404 si no existe)¦one (404 if missing)⟧ |
| POST | `/__RUTA__` | ⟦crea (201; 422 con `errores` por campo)¦creates (201; 422 with `errores` per field)⟧ |
| PUT | `/__RUTA__/{id}` | ⟦cambia¦changes⟧ |
| DELETE | `/__RUTA__/{id}` | ⟦borra (204)¦deletes (204)⟧ |

- `app/modelos.py`: ⟦los datos y su validación.¦the data and its validation.⟧ `app/repositorio.py`: ⟦el almacén (en memoria: cámbialo para una base de datos).¦the store (in memory: change it for a database).⟧
- `app/main.py`: ⟦las rutas.¦the routes.⟧ `app/config.py`: ⟦`PUERTO` y `ORIGENES` (ver `.env.example`).¦`PUERTO` and `ORIGENES` (see `.env.example`).⟧
