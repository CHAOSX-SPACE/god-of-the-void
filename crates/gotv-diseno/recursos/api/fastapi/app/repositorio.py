"""⟦Guarda en memoria. Para una base de datos cambia SÓLO esta clase (la misma interfaz).¦Stores in memory. For a database change ONLY this class (the same interface).⟧"""

from .modelos import Entrada, Registro


class Repositorio:
    def __init__(self) -> None:
        self._filas: dict[int, Registro] = {}
        self._siguiente = 1

    def listar(self) -> list[Registro]:
        return list(self._filas.values())

    def obtener(self, id: int) -> Registro | None:
        return self._filas.get(id)

    def crear(self, entrada: Entrada) -> Registro:
        r = Registro(id=self._siguiente, **entrada.model_dump())
        self._siguiente += 1
        self._filas[r.id] = r
        return r

    def cambiar(self, id: int, entrada: Entrada) -> Registro | None:
        if id not in self._filas:
            return None
        r = Registro(id=id, **entrada.model_dump())
        self._filas[id] = r
        return r

    def borrar(self, id: int) -> bool:
        return self._filas.pop(id, None) is not None
