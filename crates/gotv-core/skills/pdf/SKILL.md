---
name: pdf
description: PDF como ARCHIVO con la herramienta `pdf` — CREAR un PDF con diseño desde una ficha de bloques con tema (informes, propuestas), ver sus datos (páginas, tamaño, formulario), leer su texto por página, convertir páginas a imagen para MIRARLAS (escaneos, diseño, revisar cómo quedó un documento), extraer páginas (dividir), unir varios PDF y girar páginas. Úsala siempre que se pida leer, resumir, unir, separar, girar o revisar un .pdf. Para HACER un PDF desde un Word, Excel o PowerPoint usa esa herramienta con accion «pdf».
---

# PDF: leer, mirar, extraer, unir, girar

Adaptado de la skill `pdf` de Claude Code (Anthropic, Apache-2.0): mismas tareas, llevadas a la
herramienta `pdf` de GOD OF THE VOID (Rust: pdf-extract + lopdf, imágenes con `sips` de macOS).
Cambios: pypdf/pdfplumber/qpdf → `pdf`; el OCR es MIRAR la página (si tu modelo ve).

| Tarea | Cómo |
|---|---|
| ¿Qué es este PDF? | `pdf` accion «info» — páginas, tamaño (A4/carta), título, autor, si tiene formulario |
| Leer / resumir | `pdf` accion «leer» (todas) o con `paginas` «3-5» |
| Escaneo o página sin texto | accion «imagenes» con `paginas` y luego `view_image` de cada PNG |
| Revisar cómo QUEDÓ un documento | accion «imagenes» del PDF y mirarlas (QA visual) |
| Dividir / sacar páginas | accion «extraer» con `paginas` «1-3» y `destino` «parte1.pdf» |
| Unir | accion «unir» con `archivos` [«a.pdf», «b.pdf»] (en ese orden) y `destino` |
| Girar | accion «rotar» con `paginas`, `grados` (90, 180, 270, -90) y `destino` |
| Crear un PDF con diseño | accion «crear» con `ficha` (bloques + `tema`) y `destino` — ver abajo |
| PDF de un Word/PowerPoint | esa herramienta con su accion «pdf» |

## Crear un PDF de diseño

`pdf` accion «crear» con `ficha` y `destino` hace un PDF maquetado directamente (sin Word): la ficha es la
MISMA de la herramienta `word` (`titulo`, `h1`/`h2`/`h3`, `parrafo`, `lista`, `numerada`, `tabla`, `linea`,
`salto`; `**negrita**` y `*cursiva*`), con `"pagina": "carta"|"a4"|"oficio"` y un `"tema"`:

```json
{"pagina": "carta", "tema": {"marca": "verde", "fuentes": "editorial"}, "bloques": [
  {"titulo": "Informe anual"},
  {"h1": "Resumen"}, {"parrafo": "Crecimos **un 12 %**."},
  {"tabla": [["Línea", "Total"], ["Tienda", "$10"]], "anchos": [3, 1]}
]}
```

- El PDF lleva sus tipografías DENTRO (se ve igual en cualquier equipo), la cabecera de las tablas se repite
  en cada página y el pie dice «página / total» (`"numeros_pagina": false` lo quita).
- `tema`: igual que en `word` (`{"marca": "azul"}`, un hex, o una paleta de las presentaciones como
  `"oceano"`). Sin `tema` sale sobrio, en gris.
- Después, MÍRALO: accion «imagenes» del PDF nuevo y `view_image`.
- Si el usuario pidió un Word (para editarlo), hazlo con `word` y conviértelo con su accion «pdf».

## Reglas

- **Nunca se sobrescribe**: lo nuevo va a un `destino` que no exista. Si ya existe, elige otro nombre.
- `paginas`: «1-3,5», «2-» (de la 2 al final), vacío = todas. Las páginas se numeran desde 1.
- «leer» corta a los ~40 000 caracteres: si dice «cortado», pide las páginas siguientes.
- Si «leer» avisa «sin texto», es un escaneo o una imagen: conviértela con «imagenes» y MÍRALA.
  Hasta 10 páginas por llamada; mira una imagen por llamada de `view_image`.
- Para resumir un PDF largo: «info» primero (cuántas páginas), luego «leer» por tramos.
- Al «extraer» en otro orden que el original, las páginas quedan en su orden original: para
  reordenar, extrae por partes y únelas en el orden que quieras.

## Lo que todavía no hace

Rellenar formularios, marcas de agua, contraseñas y extraer tablas como celdas. Si te lo piden,
dilo claro y ofrece lo cercano: leer el formulario y responder con los valores, o pasar la tabla
leída a Excel con la herramienta `excel`.
