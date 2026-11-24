import { describe, expect, it } from "vitest";
import { borrar, crear, porCiudad, resumen, SEMILLA, validar } from "./datos";

describe("validar", () => {
  it("accepts a correct draft", () => {
    expect(validar({ nombre: "Eva", correo: "eva@x.co", ciudad: "Reno" }, SEMILLA)).toEqual({});
  });
  it("names every bad field", () => {
    const e = validar({ nombre: "E", correo: "not-an-email", ciudad: " " }, SEMILLA);
    expect(Object.keys(e).sort()).toEqual(["ciudad", "correo", "nombre"]);
  });
  it("does not allow a repeated email (case-insensitive)", () => {
    expect(validar({ nombre: "Other", correo: "LAURA@lumen.co", ciudad: "Austin" }, SEMILLA).correo).toMatch(/already/);
  });
});

describe("create and delete", () => {
  it("creates with the next id and does not mutate the list", () => {
    const antes = [...SEMILLA];
    const nueva = crear({ nombre: " Eva ", correo: "EVA@x.co", ciudad: "Reno" }, SEMILLA);
    expect(SEMILLA).toEqual(antes);
    expect(nueva.at(-1)).toMatchObject({ id: 6, nombre: "Eva", correo: "eva@x.co", estado: "pending" });
  });
  it("deletes by id", () => {
    expect(borrar(1, SEMILLA).some((c) => c.id === 1)).toBe(false);
  });
});

it("summarizes", () => {
  expect(resumen(SEMILLA)).toMatchObject({ total: 5, activos: 3, compras: 43 });
  expect(resumen([]).promedio).toBe(0);
});

it("adds up purchases by city, largest first", () => {
  const dos = [...SEMILLA, { ...SEMILLA[0], id: 9, compras: 30 }];
  expect(porCiudad(dos)[0]).toEqual({ etiqueta: SEMILLA[0].ciudad, valor: 42 });
  expect(porCiudad(dos).reduce((s, d) => s + d.valor, 0)).toBe(resumen(dos).compras);
  expect(porCiudad([])).toEqual([]);
});
