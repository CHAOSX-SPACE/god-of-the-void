// Lógica del negocio SIN interfaz: fácil de probar y de cambiar.
export type Estado = "activo" | "pendiente" | "inactivo";
export type Cliente = { id: number; nombre: string; correo: string; ciudad: string; estado: Estado; compras: number };
export type Borrador = { nombre: string; correo: string; ciudad: string };
export type Errores = Partial<Record<keyof Borrador, string>>;

const CORREO = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/** Valida en el borde: devuelve los errores por campo (vacío = válido). */
export function validar(b: Borrador, existentes: Cliente[]): Errores {
  const e: Errores = {};
  if (b.nombre.trim().length < 2) e.nombre = "Escribe el nombre (al menos 2 letras).";
  if (!CORREO.test(b.correo.trim())) e.correo = "Escribe un correo válido, como ana@empresa.com.";
  else if (existentes.some((c) => c.correo.toLowerCase() === b.correo.trim().toLowerCase())) e.correo = "Ya hay un cliente con ese correo.";
  if (!b.ciudad.trim()) e.ciudad = "Escribe la ciudad.";
  return e;
}

/** Crea el cliente con el siguiente id libre (no muta la lista). */
export function crear(b: Borrador, lista: Cliente[]): Cliente[] {
  const id = lista.reduce((m, c) => Math.max(m, c.id), 0) + 1;
  return [...lista, { id, nombre: b.nombre.trim(), correo: b.correo.trim().toLowerCase(), ciudad: b.ciudad.trim(), estado: "pendiente", compras: 0 }];
}

export function borrar(id: number, lista: Cliente[]): Cliente[] {
  return lista.filter((c) => c.id !== id);
}

export function resumen(lista: Cliente[]) {
  const activos = lista.filter((c) => c.estado === "activo").length;
  const compras = lista.reduce((s, c) => s + c.compras, 0);
  return { total: lista.length, activos, compras, promedio: lista.length ? compras / lista.length : 0 };
}

/** Compras sumadas por ciudad, de mayor a menor (lo que pinta la gráfica de barras). */
export function porCiudad(lista: Cliente[]): { etiqueta: string; valor: number }[] {
  const m = new Map<string, number>();
  for (const c of lista) m.set(c.ciudad, (m.get(c.ciudad) ?? 0) + c.compras);
  return [...m].map(([etiqueta, valor]) => ({ etiqueta, valor })).sort((a, b) => b.valor - a.valor);
}

/** Compras de los últimos seis meses (de ejemplo: cámbialo por tus datos). */
export const HISTORIA = {
  meses: ["jun", "jul", "ago", "sep", "oct", "nov"],
  nuevos: [4, 6, 5, 9, 8, 11],
  recurrentes: [10, 12, 15, 14, 19, 32],
};

export const SEMILLA: Cliente[] = [
  { id: 1, nombre: "Laura Méndez", correo: "laura@lumen.co", ciudad: "Medellín", estado: "activo", compras: 12 },
  { id: 2, nombre: "Diego Rojas", correo: "diego@kora.co", ciudad: "Bogotá", estado: "activo", compras: 7 },
  { id: 3, nombre: "Ana Torres", correo: "ana@orbita.co", ciudad: "Cali", estado: "pendiente", compras: 0 },
  { id: 4, nombre: "Camilo Ruiz", correo: "camilo@velox.co", ciudad: "Barranquilla", estado: "inactivo", compras: 3 },
  { id: 5, nombre: "Sofía Gómez", correo: "sofia@sierra.co", ciudad: "Bucaramanga", estado: "activo", compras: 21 },
];
