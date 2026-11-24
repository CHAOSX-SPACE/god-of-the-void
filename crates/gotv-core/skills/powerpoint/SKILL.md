---
name: powerpoint
description: Presentaciones de PowerPoint (.pptx) como ARCHIVO con la herramienta `powerpoint` — crear decks con diseño (paleta de color, portada y cierre oscuros, gráficos nativos, tablas, cifras grandes, columnas); leer uno existente; editarlo (reemplazar texto, duplicar, borrar o agregar diapositivas) sin romperlo; convertirlo a PDF con el PowerPoint del usuario. Úsala siempre que se pida una presentación, diapositivas, un PowerPoint, un .pptx o un deck. No para documentos de texto ni hojas de cálculo.
---

# PowerPoint: crear, editar y revisar

Adaptado de la skill `pptx` de Claude Code (Anthropic, Apache-2.0): mismas reglas de diseño y de QA,
llevadas a la herramienta `powerpoint` de GOD OF THE VOID (Rust). Cambios: pptxgenjs/python → `powerpoint`;
el PDF lo hace el POWERPOINT del usuario; las reglas de diseño (paletas, contraste, márgenes, sin barras
decorativas) ya las aplica el generador.

Un .pptx es un ZIP de XML: GOTV lo escribe y lo edita como archivo. NO abras PowerPoint en pantalla para
esto (sólo si el usuario pide VERLO: Control Mac → `["abrir","Microsoft PowerPoint","ia.pptx"]`).

| Tarea | Cómo |
|---|---|
| **Crear** | `powerpoint` accion «crear» con `ficha` (tema + diapositivas) |
| **Leer** una existente | `powerpoint` accion «leer» — texto de cada diapositiva, tablas y datos de gráficos |
| **Cambiar** una existente | `powerpoint` accion «editar» con `cambios` (respaldo al lado) |
| **PDF** | `powerpoint` accion «pdf» (lo hace el PowerPoint del usuario) |
| **Comprobar** | va solo tras crear/editar; o accion «revisar» |
| **Mirar** cómo QUEDÓ | accion «mirar» → `view_image` de cada página (la QA visual) |

## Crear: la ficha

```json
{"tema": {"paleta": "teal"}, "diapositivas": [
  {"tipo": "portada", "titulo": "Inteligencia artificial", "subtitulo": "Qué es y cómo usarla"},
  {"tipo": "puntos", "titulo": "Por qué importa", "puntos": ["Automatiza tareas repetitivas", "Ayuda a decidir con datos"]},
  {"tipo": "dos_columnas", "titulo": "Antes y después", "titulo_izquierda": "Antes", "izquierda": ["Manual", "Lento"], "titulo_derecha": "Después", "derecha": ["Automático", "Rápido"]},
  {"tipo": "cifra", "titulo": "Adopción", "cifra": "72 %", "texto": "de las empresas ya usa IA"},
  {"tipo": "tabla", "titulo": "Herramientas", "tabla": [["Uso", "Ejemplo"], ["Texto", "Asistentes"]]},
  {"tipo": "grafico", "titulo": "Inversión anual", "texto": "Crece 40 % al año",
   "grafico": {"tipo": "columnas", "categorias": ["2023", "2024", "2025"], "series": [{"nombre": "Inversión", "valores": [120, 180, 260]}]}},
  {"tipo": "cierre", "titulo": "Gracias", "subtitulo": "Preguntas"}
]}
```

- **Tipos**: `portada` · `seccion` · `puntos` · `dos_columnas` · `cifra` · `tabla` · `grafico` · `cierre`.
- **Gráficos**: `columnas` · `barras` · `lineas` · `torta` · `dona`. Una serie: `"valores": […]`;
  varias: `"series": [{"nombre", "valores"}]`. Quedan NATIVOS (editables en PowerPoint, con su hoja).
- **Paletas** (`tema.paleta`): midnight · bosque · coral · terracota · oceano · carbon · teal · baya ·
  salvia · cereza. Opcional: `fuente_titulo` (Cambria) y `fuente_cuerpo` (Calibri).
- `**negrita**` dentro de un texto.

## Diseño (lo que decides TÚ; lo demás lo hace el generador)

- **Paleta según el TEMA**, no azul por defecto: salud → teal/salvia, naturaleza → bosque, finanzas →
  midnight/carbon, energía/ventas → coral/cereza, cultura → terracota/baya.
- **Sándwich**: portada y cierre oscuros, contenido claro (ya va así).
- **Cada diapositiva con algo visual**: varía los tipos — no repitas `puntos` cinco veces. Un dato
  fuerte → `cifra`; comparar → `dos_columnas`; números en serie → `grafico`; detalle → `tabla`.
- **Pocas palabras**: 3-5 puntos de ≤ 12 palabras. Si no cabe, PARTE en dos diapositivas.
- Títulos SIN punto final. Nada de «Lorem», «TODO», «xxx» ni texto de relleno.
- 1 idea por diapositiva; 5-10 diapositivas para una charla corta.

## Editar

```json
[{"reemplazar": "2025", "por": "2026"},
 {"duplicar_diapositiva": 2},
 {"borrar_diapositiva": 4},
 {"agregar": [{"tipo": "puntos", "titulo": "Próximos pasos", "puntos": ["Piloto", "Medir"]}]}]
```

- Primero **leer** para ver el texto y el número de cada diapositiva.
- `reemplazar` funciona aunque el texto esté partido en fragmentos de formato.
- Los cambios de estructura (duplicar, borrar, agregar) van antes que los de texto; los números
  de diapositiva son los de ANTES de editar.
- Queda un respaldo `.bak` al lado.

## QA (obligatoria)

La primera versión suele traer algún fallo. Tras crear o editar, la herramienta devuelve la revisión y
el texto: **léelo** y busca lo que falta, erratas, orden equivocado o texto de relleno. Si hay `✗`,
corrige y vuelve a revisar. Para entregar en PDF: accion «pdf» y comprueba que el archivo existe.

### QA visual (la de Claude Code: dibujar → MIRAR)

- La revisión también avisa `⚠ … no cabe en su cuadro`: acorta ese texto o pártelo en dos diapositivas.
- Para un deck que se va a presentar o entregar: accion «mirar» y `view_image` de cada diapositiva.
  Busca texto cortado o encima de otra pieza, poco contraste, huecos grandes, algo desalineado.
  Corrige y vuelve a mirar SÓLO las diapositivas que cambiaste.
