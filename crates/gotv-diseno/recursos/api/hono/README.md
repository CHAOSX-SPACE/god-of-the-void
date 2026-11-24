# __MARCA__ · API

⟦API REST de «__PLURAL__» hecha con GOD OF THE VOID (Hono + TypeScript, versiones fijas).¦REST API for «__PLURAL__» made with GOD OF THE VOID (Hono + TypeScript, pinned versions).⟧

```bash
npm install
npm test         # ⟦pruebas¦tests⟧
npm run check    # ⟦tipos¦types⟧
npm run dev      # http://localhost:8000
```

| ⟦Método¦Method⟧ | ⟦Ruta¦Path⟧ | ⟦Qué hace¦What it does⟧ |
|---|---|---|
| GET | `/salud` | ⟦está viva¦it is alive⟧ |
| GET | `/__RUTA__` | ⟦lista¦list⟧ |
| GET | `/__RUTA__/:id` | ⟦uno (404 si no existe)¦one (404 if missing)⟧ |
| POST | `/__RUTA__` | ⟦crea (201; 422 con `errores` por campo)¦creates (201; 422 with `errores` per field)⟧ |
| PUT | `/__RUTA__/:id` | ⟦cambia¦changes⟧ |
| DELETE | `/__RUTA__/:id` | ⟦borra (204)¦deletes (204)⟧ |

- `src/datos.ts`: ⟦tipos, validación y almacenamiento (en memoria: cambia `Repositorio` para una base de datos).¦types, validation and storage (in memory: change `Repositorio` for a database).⟧
- `src/app.ts`: ⟦las rutas.¦the routes.⟧ `src/config.ts`: ⟦`PUERTO` y `ORIGENES` (ver `.env.example`).¦`PUERTO` and `ORIGENES` (see `.env.example`).⟧
