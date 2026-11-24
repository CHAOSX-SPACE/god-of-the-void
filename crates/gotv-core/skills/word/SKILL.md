---
name: word
description: Documentos de Word (.docx) como ARCHIVO con la herramienta `word` — crear cartas, informes y documentos con títulos, listas y tablas; leer uno existente; editarlo (reemplazar texto, agregar secciones, borrar párrafos) sin romper su formato; convertirlo a PDF con el Word del usuario. Úsala siempre que se pida un Word, un .docx, «escríbeme en Word» o un documento para enviar, imprimir o firmar. No para hojas de cálculo, presentaciones ni páginas web.
---

# Word: crear, editar y revisar

Adaptado de la skill `docx` de Claude Code (Anthropic, Apache-2.0): mismas reglas, llevadas a la
herramienta `word` de GOD OF THE VOID (Rust). Cambios: docx-js/pandoc/LibreOffice → `word`; el PDF lo hace
el WORD del usuario; las trampas de docx-js ya las resuelve el generador.

Un .docx es un ZIP de XML: GOTV lo escribe y lo edita como archivo. NO abras Word en pantalla para esto
(sólo si el usuario pide VERLO: Control Mac → `["abrir","Microsoft Word","carta.docx"]`).

| Tarea | Cómo |
|---|---|
| **Crear** | `word` accion «crear» con `ficha` (bloques) |
| **Leer** uno existente | `word` accion «leer» — texto con `¶n` (número de cada párrafo), títulos `#`, listas `-`, tablas |
| **Cambiar** uno existente | `word` accion «editar» con `cambios` (respaldo al lado; el resto queda como estaba) |
| **PDF** | `word` accion «pdf» (lo hace el Word del usuario: exacto a como se imprime) |
| **Comprobar** | va solo tras crear/editar; o accion «revisar» |
| **Mirar** cómo QUEDÓ | accion «mirar» → `view_image` de cada página (la QA visual) |

## Crear: la ficha

```json
{"pagina": "carta", "fuente": "Arial", "bloques": [
  {"titulo": "Propuesta de servicios"},
  {"parrafo": "Medellín, 11 de noviembre de 2026", "alineacion": "derecha"},
  {"h1": "Resumen"},
  {"parrafo": "Le presentamos **tres** servicios *nuevos*."},
  {"lista": ["Software a medida", "Consultoría", ["Nube", "Seguridad"]]},
  {"numerada": ["Diagnóstico", "Plan", "Ejecución"]},
  {"tabla": [["Servicio", "Precio"], ["Software", "$10.000.000"]], "anchos": [2, 1]},
  {"linea": true},
  {"salto": true},
  {"parrafo": "Atentamente,\nAna Pérez\nGerente"}
]}
```

- **Página**: `a4` por omisión; `carta` (Colombia, EE. UU.) u `oficio` si el usuario lo pide o el país lo usa.
- **Títulos**: `titulo` una vez arriba; `h1`/`h2`/`h3` para secciones (Word los usa para el índice y el panel
  de navegación; ya llevan fuente, espacio antes/después y «mantener con el siguiente»).
- **Listas**: `lista` (viñetas) y `numerada` (cada una empieza en 1); un ítem que es una lista = segundo
  nivel. NUNCA escribas «•» ni «1.» dentro del texto: la numeración la pone Word.
- **Saltos de línea**: cada línea de un texto es su propio párrafo (así se firma una carta). Para un salto
  de PÁGINA, `{"salto": true}`.
- **Tablas**: la primera fila es el encabezado (negrita, sombreado, se repite en cada página);
  `anchos` son proporciones (`[2,1]` = la primera el doble). No uses una tabla para trazar una línea:
  `{"linea": true}`.
- **Tema (documento con DISEÑO)**: añade `"tema"` a la ficha y el documento sale vestido: títulos con su
  tipografía y el color de marca, filete bajo el título, cabecera de tabla en ese color, filas alternas y
  «página / total» al pie. Formas:
  - `"tema": {"marca": "azul"}` — un color por nombre (`azul`, `verde`, `morado`, `turquesa`, `rojo`,
    `naranja`, `gris`…) o en hex (`"#1e40af"`); opcional `"croma": "sobrio"|"medio"|"vivo"` y
    `"fuentes": "tecnico"|"editorial"|"lujo"|"suizo"|"sobrio"` (editorial y lujo = títulos con serifa);
  - `"tema": "oceano"` — una paleta de las presentaciones (`midnight`, `bosque`, `coral`, `terracota`,
    `oceano`, `carbon`, `teal`, `baya`, `salvia`, `cereza`): úsala si el mismo trabajo lleva un PowerPoint,
    para que vayan a juego.
  El contraste está garantizado (nunca texto ilegible). Sin `tema` el documento sale sobrio, como siempre.
  `"numeros_pagina": true|false` pone o quita el pie. Una **carta** formal NO lleva tema (va por `carta`).
- Negrita y cursiva sólo en el texto: `**así**` y `*así*`.

## Una CARTA: la ficha `carta` (formato colombiano GTC 185)

Una carta NO se arma con bloques sueltos (salía sin fecha, sin destinatario y en dos hojas). La ficha
`carta` aplica la GTC 185 de ICONTEC y la deja en UNA hoja si cabe:

```json
{"carta": {
  "ciudad": "Medellín",
  "membrete": {"nombre": "<NOMBRE DE LA EMPRESA>", "linea": "<NIT · dirección · teléfono, SÓLO si el usuario los dio>"},
  "destinatario": {"tratamiento": "Señora", "nombre": "Laura Restrepo Mejía", "cargo": "Gerente General",
                   "organizacion": "Hotel Montaña", "direccion": "Calle 10 32-15", "ciudad": "Medellín, Antioquia"},
  "asunto": "Propuesta de suministro",
  "texto": ["Primer párrafo…", "Segundo párrafo…", "Cierre…"],
  "remitente": {"nombre": "Andrés Gómez Ruiz", "cargo": "Gerente Comercial"},
  "anexos": "1 (portafolio, cuatro folios)"
}}
```

- **NUNCA inventes datos** (NIT, dirección, teléfono, nombres de personas): una carta real con un NIT
  falso es peor que sin NIT. Si no los dieron, `membrete` sólo con el nombre de la empresa (u omítelo)
  y el remitente con el nombre que haya.
- La fecha de HOY va sola («Medellín, 11 de noviembre de 2026»); `fecha` sólo si piden otra.
- Sin nombre de la persona: `"destinatario": "Hotel Montaña"` → «Señores / Hotel Montaña /
  Respetados señores:» (carta general a la organización).
- El nombre del destinatario y del remitente salen en MAYÚSCULA FIJA; nada en negrilla; el asunto sin
  subrayar (es la norma). `"subrayar_destinatario": true` si el usuario lo pide.
- `saludo` y `despedida` tienen su valor por omisión («Respetada señora:», «Atentamente,»).
- `estilo`: «bloque_extremo» (todo a la izquierda, por omisión) · «bloque» (fecha y firma desde el
  centro) · «semibloque» (bloque + sangría).
- Texto: 3-4 párrafos en primera persona del plural, tratando de «usted»; NUNCA `salto`.
- Luego `pdf` si la piden en PDF, y `mirar` para ver que quedó en una hoja.

## Editar un documento existente

1. **Lee primero** (`leer`): copia el texto EXACTO que vas a reemplazar o junto al que vas a agregar.
2. Cambios, en orden:
   - `{"reemplazar": "texto exacto", "por": "texto nuevo"}` (cambia TODAS las veces; `"todas": false` sólo la primera). Funciona
     aunque Word haya partido la frase internamente: GOTV une los fragmentos antes (como `merge_runs` de
     Claude Code) y el texto nuevo toma el formato de donde empezaba el viejo.
   - `{"agregar": [bloques], "despues_de": "texto de un párrafo"}` — sin `despues_de`, al final.
   - `{"borrar": "texto de un párrafo"}` — borra los párrafos que lo contienen.
3. **Respeta el documento**: no cambies lo que no te pidieron; usa su mismo tono y su estructura.

## Verificar antes de entregar

- La revisión va sola (paquete válido, ids, estilos). Si dice ✗, corrige y vuelve a revisar.
- Si el usuario va a imprimir, enviar o firmar, haz también el **pdf**: si Word no pudiera abrir el
  archivo, la conversión fallaría (es la prueba de que abre).
- Al final, di qué archivo quedó y dónde (y el respaldo si editaste).
- Para un documento con diseño (tablas, varias columnas, portada): accion «mirar» y `view_image` de
  cada página; corrige lo que se vea cortado, encimado o desalineado, y vuelve a mirar esa página.
