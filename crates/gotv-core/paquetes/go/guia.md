---
name: lenguaje-go
description: Go con módulos — correr UNA prueba, añadir una prueba (tablas), dependencias, depurar y cobertura. Úsala cuando dudes del comando exacto en un proyecto de Go.
---

# Go

## Correr una prueba
- Todas: `go test ./...`.
- Una por nombre (expresión regular): `go test ./... -run '^TestSuma$'`; subprueba: `-run 'TestSuma/negativos'`.
- Un paquete: `go test ./interno/pagos`; ver los logs: `-v`; sin caché: `-count=1`.
- Mejor: la herramienta `pruebas` (lee `go test -json`, también los paquetes que no compilan).

## Añadir una prueba
- `x_test.go` en el mismo paquete: `func TestSuma(t *testing.T) { if got := Suma(1, 2); got != 3 { t.Fatalf("Suma = %d, quería 3", got) } }`.
- Varios casos: una tabla `[]struct{ nombre string; a, b, quiere int }` y `t.Run(c.nombre, …)`.

## Dependencias
- `go get modulo@version` y luego `go mod tidy`; nunca editar go.sum a mano.

## Depurar
- `go vet ./...` encuentra errores que compilan; el primer error de `go build ./...` manda.
- Carreras: `go test -race ./...`.

## Cobertura
- `go test -cover ./...`; detalle: `go test -coverprofile=c.out ./... && go tool cover -func=c.out`.
