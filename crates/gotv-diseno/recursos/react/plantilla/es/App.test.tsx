import { fireEvent, render, screen, within } from "@testing-library/react";
import { expect, it } from "vitest";
import App from "./App";

it("muestra los clientes y busca", () => {
  render(<App />);
  expect(screen.getByRole("heading", { level: 1 })).toBeTruthy();
  const tabla = screen.getByRole("table");
  expect(within(tabla).getByText("Laura Méndez")).toBeTruthy();
  fireEvent.change(screen.getByRole("searchbox"), { target: { value: "zzz" } });
  expect(screen.getByText("Sin resultados")).toBeTruthy();
});

it("no crea un cliente inválido y explica por qué", () => {
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: /nuevo cliente/i }));
  fireEvent.click(screen.getByRole("button", { name: /guardar/i }));
  expect(screen.getAllByRole("alert").length).toBeGreaterThan(0);
});

it("las gráficas dejan leer sus números sin depender del color", () => {
  render(<App />);
  const barras = screen.getByRole("figure", { name: "Compras por ciudad" });
  expect(within(barras).getByText("Bucaramanga")).toBeTruthy();
  expect(within(barras).getByText("21")).toBeTruthy();
  const lineas = screen.getByRole("figure", { name: "Compras por mes" });
  expect(screen.getAllByRole("table")).toHaveLength(1);
  fireEvent.click(within(lineas).getByRole("button", { name: /ver como tabla/i }));
  expect(within(within(lineas).getByRole("table")).getAllByRole("row")).toHaveLength(7);
  fireEvent.keyDown(within(lineas).getByRole("group"), { key: "End" });
  expect(within(lineas).getByRole("status").textContent).toContain("32");
});
