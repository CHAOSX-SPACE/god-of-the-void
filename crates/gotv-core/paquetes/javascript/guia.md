---
name: lenguaje-node
description: JavaScript y TypeScript con Node (npm, pnpm, yarn, bun) — correr UNA prueba con vitest, jest, mocha o node:test, añadir una prueba, dependencias, depurar y cobertura. Úsala cuando dudes del comando exacto en un proyecto de Node.
---

# JavaScript / TypeScript (Node)

El GESTOR lo dice el archivo de bloqueo (`pnpm-lock.yaml` → pnpm, `bun.lockb`/`bun.lock` → bun, `yarn.lock` → yarn, `package-lock.json` → npm) o `packageManager` en package.json; no mezcles gestores. Los scripts de package.json mandan.

## Correr una prueba
- Todas: el script `test` (`npm test`, `pnpm test`, `yarn test`, `bun run test` —`bun test` es OTRO corredor—).
- Una por nombre: vitest `npx vitest run -t "nombre"`; jest `npx jest -t "nombre"`; mocha `npx mocha -g "nombre"`; node:test `node --test --test-name-pattern="nombre"`. (Con pnpm: `pnpm exec …`; con bun: `bunx …`.)
- Un archivo: vitest `npx vitest run ruta/x.test.ts`; jest `npx jest ruta/x.test.ts`.
- vitest sin `run` se queda VIGILANDO: en la terminal de GOTV usa siempre `vitest run`.
- Mejor: la herramienta `pruebas` (corre y lee el resultado con `archivo:línea`).

## Añadir una prueba
- vitest/jest: `x.test.ts` junto al código: `import { describe, it, expect } from "vitest"; it("suma", () => expect(suma(1, 2)).toBe(3));` (jest no necesita el import).
- node:test: `import test from "node:test"; import assert from "node:assert/strict";`.

## Dependencias
- Añadir con el gestor del proyecto: `pnpm add x` / `npm install x` / `yarn add x` / `bun add x` (desarrollo: `-D`).
- Un paquete que «no existe»: compruébalo con la herramienta `package` antes de instalar parecidos.
- TypeScript: `npx tsc --noEmit` revisa los tipos sin compilar.

## Depurar
- El primer error de `tsc` es el que manda; una prueba suelta con `--reporter=verbose` (vitest) dice qué esperaba.
- `node --inspect-brk` sólo si el usuario va a conectar un depurador; si no, `console.log` y quitarlo al final.

## Cobertura
- vitest: `npx vitest run --coverage` (pide `@vitest/coverage-v8`); jest: `npx jest --coverage`.
