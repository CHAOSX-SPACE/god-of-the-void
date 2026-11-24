import { describe, expect, it } from "vitest";
import { borrar, crear, porCiudad, resumen, SEMILLA, validar } from "./datos";

describe("validar", () => {
  it("acepta un borrador correcto", () => {
    expect(validar({ nombre: "Eva", correo: "eva@x.co", ciudad: "Pasto" }, SEMILLA)).toEqual({});
  });
  it("nombra cada campo malo", () => {
    const e = validar({ nombre: "E", correo: "no-es-correo", ciudad: " " }, SEMILLA);
    expect(Object.keys(e).sort()).toEqual(["ciudad", "correo", "nombre"]);
  });
  it("no deja repetir un correo (sin importar mayúsculas)", () => {
    expect(validar({ nombre: "Otra", correo: "LAURA@lumen.co", ciudad: "Cali" }, SEMILLA).correo).toMatch(/Ya hay/);
  });
});

describe("crear y borrar", () => {
  it("crea con el siguiente id y no muta la lista", () => {
    const antes = [...SEMILLA];
    const nueva = crear({ nombre: " Eva ", correo: "EVA@x.co", ciudad: "Pasto" }, SEMILLA);
    expect(SEMILLA).toEqual(antes);
    expect(nueva.at(-1)).toMatchObject({ id: 6, nombre: "Eva", correo: "eva@x.co", estado: "pendiente" });
  });
  it("borra por id", () => {
    expect(borrar(1, SEMILLA).some((c) => c.id === 1)).toBe(false);
  });
});

it("resume", () => {
  expect(resumen(SEMILLA)).toMatchObject({ total: 5, activos: 3, compras: 43 });
  expect(resumen([]).promedio).toBe(0);
});

it("suma las compras por ciudad, de mayor a menor", () => {
  const dos = [...SEMILLA, { ...SEMILLA[0], id: 9, compras: 30 }];
  expect(porCiudad(dos)[0]).toEqual({ etiqueta: SEMILLA[0].ciudad, valor: 42 });
  expect(porCiudad(dos).reduce((s, d) => s + d.valor, 0)).toBe(resumen(dos).compras);
  expect(porCiudad([])).toEqual([]);
});
