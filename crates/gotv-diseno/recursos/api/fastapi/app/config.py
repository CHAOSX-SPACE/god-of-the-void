"""⟦Configuración: lo que cambia entre entornos entra por variables de entorno y se valida al arrancar.¦Configuration: what changes between environments comes in through environment variables and is validated at startup.⟧"""

import os
from collections.abc import Mapping
from dataclasses import dataclass


@dataclass(frozen=True)
class Config:
    puerto: int
    origenes: list[str]


def leer_config(env: Mapping[str, str] | None = None) -> Config:
    env = os.environ if env is None else env
    crudo = env.get("PUERTO", "8000")
    if not crudo.isdigit() or not 1 <= int(crudo) <= 65535:
        raise ValueError(f"⟦PUERTO inválido¦Invalid PUERTO⟧: {crudo}")
    origenes = [o.strip() for o in env.get("ORIGENES", "http://localhost:5173").split(",") if o.strip()]
    return Config(puerto=int(crudo), origenes=origenes)
