---
name: excel
description: Hojas de cálculo (.xlsx) como ARCHIVO con la herramienta `excel` — leer fórmulas y valores, crear libros, editar celdas sin perder formato, recalcular con el Excel del usuario y revisar errores. Úsala siempre que un .xlsx/.csv sea lo que entra o lo que sale, aunque se nombre de pasada («el Excel de descargas»), y para limpiar o reordenar datos de tabla en una hoja. No para documentos de Word, páginas web ni bases de datos.
---

# Excel: crear, editar y revisar

Adaptado de la skill `xlsx` de Claude Code (Anthropic, Apache-2.0): mismas reglas, llevadas a la
herramienta `excel` de GOD OF THE VOID (Rust). Cambios: openpyxl → `excel`; el recálculo lo hace el
EXCEL del usuario (no LibreOffice); fórmulas en español se traducen solas al inglés.

| Tarea | Cómo |
|---|---|
| **Mirar** un libro | `excel` accion «leer» — cada celda con su fórmula Y su valor guardado (`B5 = =SUM(B2:B4) → 20500`) |
| **Crear** uno nuevo | `excel` accion «crear» con `hojas` |
| **Cambiar** uno existente | `excel` accion «editar» con `cambios` (sólo esas celdas; formato y demás fórmulas quedan; respaldo al lado) |
| **Comprobar** | `excel` accion «revisar» (también va sola tras crear/editar; si el libro no tenía valores, calcula una copia) |
| **Mirar** cómo se imprime | `excel` accion «mirar» → `view_image` de cada página |
| Verlo en pantalla, si el usuario lo pide | Control Mac: `desktop` → `["abrir","Microsoft Excel","libro.xlsx"]` |

## Requisitos de toda entrega

- **Fórmulas, nunca resultados escritos a mano.** `"B10": "=SUM(B2:B9)"`, no el total que calculaste tú.
  La hoja debe recalcularse cuando cambian sus datos.
- **Cero errores de fórmula.** No entregues mientras la revisión diga ERROR. Si crees que un error ya
  estaba, demuéstralo leyendo el ORIGINAL (el respaldo): uno que metiste tú se ve igual que uno heredado.
- **Sigue la orden al pie de la letra**: nombres de hoja, encabezados y la fórmula que el usuario dijo.
  Un rediseño que calcula otra cosa es un fallo, por elegante que sea.
- **Documenta los supuestos y los números fijos** donde el lector los vea (una celda al lado de la tabla):
  la fuente, o «dato del usuario».
- **Un libro que CREAS para que alguien lo llene** lleva una leyenda corta de qué celdas editar y una
  fila de ejemplo. Nunca agregues eso a un archivo que te pidieron EDITAR.
- **Editar un archivo existente: respeta sus convenciones.** Lee primero; sus celdas de entrada suelen
  tener otro color o relleno: escribe sólo ahí y no toques sus fórmulas.
- Fuente: Arial (la herramienta la pone sola).

## Fórmulas que sobreviven

- **En INGLÉS y con «,»**: `=SUM(A1:A3)`, `=IF(A1>5,"sí","no")`, `=AVERAGE(…)`, `=VLOOKUP(…)`. El archivo
  guarda los nombres en inglés: `=SUMA` en el archivo da `#¿NOMBRE?` (medido). La herramienta traduce
  las comunes y te dice cuáles; escríbelas ya en inglés.
- Prefiere las de siempre: `SUMIFS INDEX MATCH IFERROR SUMPRODUCT ROUND`.
- Las nuevas (`TEXTJOIN CONCAT IFS SWITCH MAXIFS MINIFS XLOOKUP`) funcionan: la herramienta les pone el
  prefijo que el archivo necesita. Las que DERRAMAN (`FILTER SORT UNIQUE SEQUENCE`) se escriben como matriz
  dinámica: dales espacio libre debajo y a la derecha.
- **Una revisión limpia prueba que las fórmulas EVALÚAN, no que estén BIEN.** Un rango corrido una fila da
  un número limpio y equivocado. Escribe 2–3 fórmulas, mira con «leer» que traen lo esperado, y sólo
  entonces llena la cuadrícula.
- Una hoja con espacios en el nombre va entre comillas simples: `='Supuestos 2026'!B5` (sin ellas:
  `#VALUE!`). Celdas combinadas: escribe sólo la de arriba a la izquierda.

## Recalcular (lo hace la herramienta)

Una librería escribe las fórmulas SIN valor guardado: quien las lea sin recalcular ve vacío. Por eso
«crear» y «editar» abren una copia en el Excel del usuario, recalculan, guardan y revisan, y te devuelven
el informe y la hoja leída. Si no hay Excel en el Mac, la herramienta lo dice: las fórmulas quedan sin
valor hasta que alguien abra el archivo — dilo así al usuario.

Si la revisión nombra errores (`#DIV/0!`, `#REF!`, `#NAME?`, `#VALUE!`, `#N/A`, `#NUM!`, `#NULL!`): corrige
esas celdas con «editar» y la revisión vuelve a correr sola. `#DIV/0!` → protege el denominador
(`=IF(B2=0,0,A2/B2)` o `=IFERROR(A2/B2,0)`).

## Modelos financieros (si el usuario no dice otra cosa)

Colores: azul (`0,0,255`) para datos fijos y palancas · negro para fórmulas · verde (`0,128,0`) para
vínculos a otra hoja · rojo para vínculos a otro archivo · relleno amarillo para supuestos clave.
Números: moneda `$#,##0` con la unidad en el encabezado (`Ingresos (millones)`) · ceros como `-`
(`$#,##0;($#,##0);-`) · negativos entre paréntesis · porcentajes guardados como FRACCIÓN (`0.15` →
`15.0%`) · años como texto (`"2026"`). Cada supuesto en su celda con nombre, y las fórmulas lo
referencian (`=B5*(1+$B$6)`, nunca `=B5*1.05`); fórmulas iguales en todos los periodos.
