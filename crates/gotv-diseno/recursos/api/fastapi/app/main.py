"""⟦Las rutas de «__PLURAL__».¦The routes for «__PLURAL__».⟧"""

from fastapi import FastAPI, HTTPException, Request, Response
from fastapi.exceptions import RequestValidationError
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import JSONResponse

from .config import Config, leer_config
from .modelos import MENSAJES, Entrada, Registro
from .repositorio import Repositorio

NO_EXISTE = "⟦No existe.¦Not found.⟧"


def crear_app(config: Config | None = None, repo: Repositorio | None = None) -> FastAPI:
    """⟦La app con su configuración y su almacén: las pruebas le pasan los suyos.¦The app with its configuration and its store: the tests pass their own.⟧"""
    config = config or leer_config()
    repo = repo or Repositorio()
    app = FastAPI(title="__MARCA__")
    app.add_middleware(CORSMiddleware, allow_origins=config.origenes, allow_methods=["*"], allow_headers=["*"])

    @app.exception_handler(RequestValidationError)
    async def errores_por_campo(_: Request, exc: RequestValidationError) -> JSONResponse:
        # ⟦un error por campo, con su nombre: lo que la interfaz necesita para pintarlo junto a él¦one error per field, with its name: what the interface needs to show it next to it⟧
        errores = {str(e["loc"][-1]): MENSAJES.get(str(e["loc"][-1]), e["msg"]) for e in exc.errors()}
        return JSONResponse(status_code=422, content={"errores": errores})

    @app.exception_handler(HTTPException)
    async def error_simple(_: Request, exc: HTTPException) -> JSONResponse:
        return JSONResponse(status_code=exc.status_code, content={"error": exc.detail})

    @app.get("/salud")
    def salud() -> dict[str, str]:
        return {"estado": "ok"}

    @app.get("/__RUTA__")
    def listar() -> list[Registro]:
        return repo.listar()

    @app.get("/__RUTA__/{id}")
    def obtener(id: int) -> Registro:
        r = repo.obtener(id)
        if r is None:
            raise HTTPException(404, NO_EXISTE)
        return r

    @app.post("/__RUTA__", status_code=201)
    def crear(entrada: Entrada) -> Registro:
        return repo.crear(entrada)

    @app.put("/__RUTA__/{id}")
    def cambiar(id: int, entrada: Entrada) -> Registro:
        r = repo.cambiar(id, entrada)
        if r is None:
            raise HTTPException(404, NO_EXISTE)
        return r

    @app.delete("/__RUTA__/{id}", status_code=204)
    def borrar(id: int) -> Response:
        if not repo.borrar(id):
            raise HTTPException(404, NO_EXISTE)
        return Response(status_code=204)

    return app


app = crear_app()
