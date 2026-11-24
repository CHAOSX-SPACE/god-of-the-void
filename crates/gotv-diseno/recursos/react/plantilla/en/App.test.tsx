import { fireEvent, render, screen, within } from "@testing-library/react";
import { expect, it } from "vitest";
import App from "./App";

it("shows the customers and searches", () => {
  render(<App />);
  expect(screen.getByRole("heading", { level: 1 })).toBeTruthy();
  const tabla = screen.getByRole("table");
  expect(within(tabla).getByText("Laura Mendez")).toBeTruthy();
  fireEvent.change(screen.getByRole("searchbox"), { target: { value: "zzz" } });
  expect(screen.getByText("No results")).toBeTruthy();
});

it("does not create an invalid customer and explains why", () => {
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: /new customer/i }));
  fireEvent.click(screen.getByRole("button", { name: /save/i }));
  expect(screen.getAllByRole("alert").length).toBeGreaterThan(0);
});

it("the charts let you read their numbers without relying on color", () => {
  render(<App />);
  const barras = screen.getByRole("figure", { name: "Purchases by city" });
  expect(within(barras).getByText("Chicago")).toBeTruthy();
  expect(within(barras).getByText("21")).toBeTruthy();
  const lineas = screen.getByRole("figure", { name: "Purchases by month" });
  expect(screen.getAllByRole("table")).toHaveLength(1);
  fireEvent.click(within(lineas).getByRole("button", { name: /view as table/i }));
  expect(within(within(lineas).getByRole("table")).getAllByRole("row")).toHaveLength(7);
  fireEvent.keyDown(within(lineas).getByRole("group"), { key: "End" });
  expect(within(lineas).getByRole("status").textContent).toContain("32");
});
