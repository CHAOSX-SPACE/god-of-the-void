// Business logic WITHOUT UI: easy to test and to change.
export type Estado = "active" | "pending" | "inactive";
export type Cliente = { id: number; nombre: string; correo: string; ciudad: string; estado: Estado; compras: number };
export type Borrador = { nombre: string; correo: string; ciudad: string };
export type Errores = Partial<Record<keyof Borrador, string>>;

const CORREO = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/** Validates at the edge: returns the errors per field (empty = valid). */
export function validar(b: Borrador, existentes: Cliente[]): Errores {
  const e: Errores = {};
  if (b.nombre.trim().length < 2) e.nombre = "Type the name (at least 2 letters).";
  if (!CORREO.test(b.correo.trim())) e.correo = "Type a valid email, like ana@company.com.";
  else if (existentes.some((c) => c.correo.toLowerCase() === b.correo.trim().toLowerCase())) e.correo = "There is already a customer with that email.";
  if (!b.ciudad.trim()) e.ciudad = "Type the city.";
  return e;
}

/** Creates the customer with the next free id (does not mutate the list). */
export function crear(b: Borrador, lista: Cliente[]): Cliente[] {
  const id = lista.reduce((m, c) => Math.max(m, c.id), 0) + 1;
  return [...lista, { id, nombre: b.nombre.trim(), correo: b.correo.trim().toLowerCase(), ciudad: b.ciudad.trim(), estado: "pending", compras: 0 }];
}

export function borrar(id: number, lista: Cliente[]): Cliente[] {
  return lista.filter((c) => c.id !== id);
}

export function resumen(lista: Cliente[]) {
  const activos = lista.filter((c) => c.estado === "active").length;
  const compras = lista.reduce((s, c) => s + c.compras, 0);
  return { total: lista.length, activos, compras, promedio: lista.length ? compras / lista.length : 0 };
}

/** Purchases added up by city, largest first (what the bar chart draws). */
export function porCiudad(lista: Cliente[]): { etiqueta: string; valor: number }[] {
  const m = new Map<string, number>();
  for (const c of lista) m.set(c.ciudad, (m.get(c.ciudad) ?? 0) + c.compras);
  return [...m].map(([etiqueta, valor]) => ({ etiqueta, valor })).sort((a, b) => b.valor - a.valor);
}

/** Purchases of the last six months (sample data: replace it with yours). */
export const HISTORIA = {
  meses: ["Jun", "Jul", "Aug", "Sep", "Oct", "Nov"],
  nuevos: [4, 6, 5, 9, 8, 11],
  recurrentes: [10, 12, 15, 14, 19, 32],
};

export const SEMILLA: Cliente[] = [
  { id: 1, nombre: "Laura Mendez", correo: "laura@lumen.co", ciudad: "Austin", estado: "active", compras: 12 },
  { id: 2, nombre: "Diego Rojas", correo: "diego@kora.co", ciudad: "Denver", estado: "active", compras: 7 },
  { id: 3, nombre: "Ana Torres", correo: "ana@orbita.co", ciudad: "Seattle", estado: "pending", compras: 0 },
  { id: 4, nombre: "Camilo Ruiz", correo: "camilo@velox.co", ciudad: "Boston", estado: "inactive", compras: 3 },
  { id: 5, nombre: "Sofia Gomez", correo: "sofia@sierra.co", ciudad: "Chicago", estado: "active", compras: 21 },
];
