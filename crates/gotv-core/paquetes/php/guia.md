---
name: lenguaje-php
description: PHP con Composer — correr UNA prueba con PHPUnit o Pest, añadir una prueba, dependencias, depurar y cobertura (también Laravel). Úsala cuando dudes del comando exacto en un proyecto de PHP.
---

# PHP (Composer)

## Correr una prueba
- Todas: `vendor/bin/phpunit` (Pest: `vendor/bin/pest`; Laravel: `php artisan test`).
- Una por nombre: `vendor/bin/phpunit --filter nombre` (Pest igual; Laravel `php artisan test --filter nombre`).
- Un archivo: `vendor/bin/phpunit tests/Unit/XTest.php`.
- Sintaxis de un archivo: `php -l archivo.php`.

## Añadir una prueba
- PHPUnit: `tests/Unit/XTest.php` con `final class XTest extends TestCase { public function test_caso(): void { $this->assertSame(4, f(2)); } }`.
- Pest: `it('suma', fn () => expect(suma(1, 2))->toBe(3));`.

## Dependencias
- `composer require paquete/nombre` (desarrollo: `--dev`); nunca editar vendor/.
- Clases nuevas que no se encuentran: `composer dump-autoload`.

## Depurar
- Activa los errores en desarrollo (`display_errors=1`); el mensaje dice archivo y línea («on line N»).
- Laravel: `storage/logs/laravel.log` guarda la traza completa.

## Cobertura
- `XDEBUG_MODE=coverage vendor/bin/phpunit --coverage-text` (pide Xdebug o PCOV).
