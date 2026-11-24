"""⟦Arranque: `uv run python -m app`.¦Startup: `uv run python -m app`.⟧"""

import uvicorn

from .config import leer_config

if __name__ == "__main__":
    # ⟦sólo esta máquina; para abrirlo a la red pon host="0.0.0.0" a sabiendas¦this machine only; to open it to the network set host="0.0.0.0" knowingly⟧
    uvicorn.run("app.main:app", host="127.0.0.1", port=leer_config().puerto)
