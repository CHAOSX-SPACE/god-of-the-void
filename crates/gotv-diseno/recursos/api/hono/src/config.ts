// ⟦Configuración: lo que cambia entre entornos entra por variables de entorno y se valida al arrancar.¦Configuration: what changes between environments comes in through environment variables and is validated at startup.⟧
export type Config = { puerto: number; origenes: string[] };

export function leerConfig(env: Record<string, string | undefined> = process.env): Config {
  const puerto = Number(env.PUERTO ?? 8000);
  if (!Number.isInteger(puerto) || puerto < 1 || puerto > 65535) throw new Error(`⟦PUERTO inválido¦Invalid PUERTO⟧: ${env.PUERTO}`);
  const origenes = (env.ORIGENES ?? "http://localhost:5173").split(",").map((o) => o.trim()).filter(Boolean);
  return { puerto, origenes };
}
