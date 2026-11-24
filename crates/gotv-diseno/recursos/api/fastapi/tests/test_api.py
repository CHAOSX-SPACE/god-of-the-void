import json

import pytest
from fastapi.testclient import TestClient

from app.config import Config, leer_config
from app.main import crear_app

VALIDO = json.loads(r"""__VALIDO__""")
ROTO = json.loads(r"""__ROTO__""")
CAMBIADO = json.loads(r"""__CAMBIADO__""")


@pytest.fixture
def cliente() -> TestClient:
    return TestClient(crear_app(Config(puerto=8000, origenes=["http://localhost:5173"])))


def test_crea_lista_lee_cambia_y_borra(cliente: TestClient) -> None:
    assert cliente.get("/salud").status_code == 200
    creado = cliente.post("/__RUTA__", json=VALIDO)
    assert creado.status_code == 201
    assert creado.json() == {**VALIDO, "id": 1}
    assert len(cliente.get("/__RUTA__").json()) == 1
    assert cliente.get("/__RUTA__/1").status_code == 200
    cambiado = cliente.put("/__RUTA__/1", json=CAMBIADO)
    assert cambiado.status_code == 200
    assert cambiado.json() == {**CAMBIADO, "id": 1}
    assert cliente.delete("/__RUTA__/1").status_code == 204
    assert cliente.get("/__RUTA__/1").status_code == 404
    assert cliente.delete("/__RUTA__/1").status_code == 404


def test_rechaza_lo_invalido_y_dice_que_campo(cliente: TestClient) -> None:
    r = cliente.post("/__RUTA__", json=ROTO)
    assert r.status_code == 422
    assert list(r.json()["errores"]) == ["__CAMPO_ROTO__"]
    assert cliente.post("/__RUTA__", json="x").status_code == 422
    assert cliente.put("/__RUTA__/1", json=VALIDO).status_code == 404
    assert cliente.get("/__RUTA__/1").json() == {"error": "⟦No existe.¦Not found.⟧"}
    assert cliente.get("/__RUTA__").json() == []


def test_la_configuracion_se_valida_al_arrancar() -> None:
    assert leer_config({"PUERTO": "3000", "ORIGENES": "https://a.com, https://b.com"}) == Config(3000, ["https://a.com", "https://b.com"])
    with pytest.raises(ValueError):
        leer_config({"PUERTO": "abc"})
