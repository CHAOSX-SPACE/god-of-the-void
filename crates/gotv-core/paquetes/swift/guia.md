---
name: lenguaje-swift
description: Swift con SwiftPM o Xcode — correr UNA prueba (XCTest o Swift Testing), añadir una prueba, dependencias, depurar y cobertura. Úsala cuando dudes del comando exacto en un proyecto de Swift.
---

# Swift

## Correr una prueba
- Paquete (Package.swift): `swift test`; una: `swift test --filter Modulo.Clase/testCaso` (acepta parte del nombre).
- App de Xcode: primero `xcodebuild -list` (esquemas); luego `xcodebuild test -scheme Esquema -destination 'platform=macOS'` (iOS: `-destination 'platform=iOS Simulator,name=iPhone 16'`); una: `-only-testing:DestinoTests/Clase/testCaso`.
- Mejor: la herramienta `pruebas` (lee el resultado de XCTest y de Swift Testing).

## Añadir una prueba
- XCTest: `Tests/ModuloTests/XTests.swift` con `final class XTests: XCTestCase { func testCaso() { XCTAssertEqual(f(2), 4) } }`.
- Swift Testing (Xcode 16+): `import Testing; @Test func caso() { #expect(f(2) == 4) }`.

## Dependencias
- SwiftPM: en Package.swift, `.package(url: "https://…", from: "1.2.0")` y el producto en el target; luego `swift package resolve`.
- Xcode: los paquetes se añaden desde el proyecto; `xcodebuild -resolvePackageDependencies`.

## Depurar
- El primer error de `swift build` manda; `swift build -v` dice qué compila.
- Un fallo en ejecución: `swift test --filter caso` con prints, y quítalos al final.

## Cobertura
- `swift test --enable-code-coverage` y luego `xcrun llvm-cov report` sobre el `.profdata` que indica `swift test --show-codecov-path`.
