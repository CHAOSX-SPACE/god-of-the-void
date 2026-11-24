---
name: lenguaje-python
description: Python con uv, poetry, pipenv o .venv — correr UNA prueba con pytest o unittest, añadir una prueba, dependencias, depurar y cobertura. Úsala cuando dudes del comando exacto en un proyecto de Python.
---

# Python

El ENTORNO del proyecto, nunca el Python del sistema: `uv.lock` → `uv run …`; `poetry.lock` → `poetry run …`; `Pipfile` → `pipenv run …`; `.venv/` → `.venv/bin/python -m …`. Si el usuario dice CÓMO (un `.venv` con pip, un archivo concreto), eso manda: `python3 -m venv .venv` y `.venv/bin/python -m pip install …`. Si no, proyecto nuevo aquí: `uv init --bare` (sólo `pyproject.toml`, sin subcarpetas ni paquete de ejemplo).

## Correr una prueba
- Todas: `uv run pytest` (o el prefijo del gestor).
- Una por nombre: `uv run pytest -k "nombre"`; exacta: `uv run pytest ruta/test_x.py::TestClase::test_caso`.
- Parar en el primer fallo y ver los prints: `uv run pytest -x -s`.
- unittest: `uv run python -m unittest paquete.test_modulo.Clase.test_caso`.
- Mejor: la herramienta `pruebas` (corre y lee el resultado con `archivo:línea`).

## Añadir una prueba
- pytest: `tests/test_x.py` con funciones `def test_caso(): assert f(2) == 4`; datos con `@pytest.mark.parametrize`.
- Un error esperado: `with pytest.raises(ValueError): f(-1)`.

## Dependencias
- uv: `uv add paquete` (desarrollo: `uv add --dev pytest`); poetry: `poetry add`; pip en .venv: `.venv/bin/python -m pip install paquete` y anótalo en requirements.txt.
- Un import que falla: comprueba que corres con el intérprete del proyecto, no con `python3` suelto.

## Depurar
- La ÚLTIMA línea de la traza dice el error; la línea de TU código más abajo en la traza, dónde.
- `breakpoint()` sólo si el usuario va a estar delante; si no, `print()` y quitarlo al final.

## Cobertura
- `uv run pytest --cov=paquete --cov-report=term-missing` (pide `pytest-cov`).
