import { describe, expect, it } from "vitest";
import { crearApp } from "./app";
import { leerConfig } from "./config";

const VALIDO = __VALIDO__;
const ROTO = __ROTO__;

const nueva = () => crearApp(leerConfig({}));
const pedir = (app: ReturnType<typeof nueva>, ruta: string, metodo = "GET", cuerpo?: unknown) =>
  app.request(ruta, { method: metodo, headers: { "content-type": "application/json" }, body: cuerpo === undefined ? undefined : JSON.stringify(cuerpo) });

describe("__RUTA__", () => {
  it("⟦crea, lista, lee, cambia y borra¦creates, lists, reads, changes and deletes⟧", async () => {
    const app = nueva();
    expect((await pedir(app, "/salud")).status).toBe(200);
    const creado = await pedir(app, "/__RUTA__", "POST", VALIDO);
    expect(creado.status).toBe(201);
    expect(await creado.json()).toMatchObject({ id: 1, ...VALIDO });
    expect(await (await pedir(app, "/__RUTA__")).json()).toHaveLength(1);
    expect((await pedir(app, "/__RUTA__/1")).status).toBe(200);
    const cambiado = await pedir(app, "/__RUTA__/1", "PUT", __CAMBIADO__);
    expect(cambiado.status).toBe(200);
    expect(await cambiado.json()).toMatchObject(__CAMBIADO__);
    expect((await pedir(app, "/__RUTA__/1", "DELETE")).status).toBe(204);
    expect((await pedir(app, "/__RUTA__/1")).status).toBe(404);
    expect((await pedir(app, "/__RUTA__/1", "DELETE")).status).toBe(404);
  });

  it("⟦rechaza lo inválido y dice qué campo¦rejects what is invalid and names the field⟧", async () => {
    const app = nueva();
    const r = await pedir(app, "/__RUTA__", "POST", ROTO);
    expect(r.status).toBe(422);
    const { errores } = (await r.json()) as { errores: Record<string, string> };
    expect(Object.keys(errores)).toEqual(["__CAMPO_ROTO__"]);
    expect((await pedir(app, "/__RUTA__", "POST", "⟦no es un objeto¦not an object⟧")).status).toBe(422);
    expect((await pedir(app, "/__RUTA__/1", "PUT", VALIDO)).status).toBe(404);
    expect(await (await pedir(app, "/__RUTA__")).json()).toEqual([]);
  });
});

it("⟦la configuración se valida al arrancar¦the configuration is validated at startup⟧", () => {
  expect(leerConfig({ PUERTO: "3000", ORIGENES: "https://a.com, https://b.com" })).toEqual({ puerto: 3000, origenes: ["https://a.com", "https://b.com"] });
  expect(() => leerConfig({ PUERTO: "abc" })).toThrow();
});
