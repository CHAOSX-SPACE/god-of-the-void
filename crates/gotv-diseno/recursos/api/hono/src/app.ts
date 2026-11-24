import { Hono } from "hono";
import { cors } from "hono/cors";
import type { Config } from "./config";
import { Repositorio, validar } from "./datos";

const NO_EXISTE = { error: "⟦No existe.¦Not found.⟧" };

/** ⟦La app SIN servidor: las pruebas la llaman directamente con `app.request`.¦The app WITHOUT a server: the tests call it directly with `app.request`.⟧ */
export function crearApp(config: Config, repo = new Repositorio()) {
  const app = new Hono();
  app.use("*", cors({ origin: config.origenes }));
  app.get("/salud", (c) => c.json({ estado: "ok" }));
  app.get("/__RUTA__", (c) => c.json(repo.listar()));
  app.get("/__RUTA__/:id", (c) => {
    const r = repo.obtener(Number(c.req.param("id")));
    return r ? c.json(r) : c.json(NO_EXISTE, 404);
  });
  app.post("/__RUTA__", async (c) => {
    const v = validar(await c.req.json().catch(() => null));
    return v.ok ? c.json(repo.crear(v.valor), 201) : c.json({ errores: v.errores }, 422);
  });
  app.put("/__RUTA__/:id", async (c) => {
    const v = validar(await c.req.json().catch(() => null));
    if (!v.ok) return c.json({ errores: v.errores }, 422);
    const r = repo.cambiar(Number(c.req.param("id")), v.valor);
    return r ? c.json(r) : c.json(NO_EXISTE, 404);
  });
  app.delete("/__RUTA__/:id", (c) => (repo.borrar(Number(c.req.param("id"))) ? c.body(null, 204) : c.json(NO_EXISTE, 404)));
  return app;
}
