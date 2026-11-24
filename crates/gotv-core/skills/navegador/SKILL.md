---
name: navegador
description: Mecánica web con la herramienta browser de GOD OF THE VOID — probar y diseñar páginas y apps web (servidor local, refs, formularios, iframes, shadow DOM, subir archivos, menús, diálogos, pestañas, vista móvil/oscura, accesibilidad). Úsala al crear, probar o depurar una página web.
---
# Navegador de GOTV — mecánica web

Inspirada en las guías de interacción de browser-harness (MIT, Browser Use) y en la skill
`core` de agent-browser (Apache-2.0, Vercel Labs). Toda orden es la herramienta `browser`
con `args` = lista. Más detalle: `browser` [`skills`,`get`,`core`].

## Ciclo de trabajo (siempre)
1. Servidor: `bash` con background=true (p. ej. `npm run dev`); la URL sale en su salida.
2. `["open","http://localhost:5173"]` → `["snapshot","-i"]` (refs `@e1`, `@e2`…).
3. Actúa por ref → `["snapshot","-i"]` otra vez tras cada cambio. Un ref vale sólo para su snapshot.
4. Comprueba con datos: `["get","text","@e4"]`, `["console"]`, `["errors"]`. Nunca digas que funciona sin mirarlo.

## Esperar (no adivines tiempos)
- `["wait","--text","Listo"]` · `["wait","@e3"]` · `["wait","--load","networkidle"]` · `["wait","--url","**/panel"]`
- Si falta un elemento en el snapshot, casi siempre la página aún carga: espera y vuelve a mirar.

## Formularios
- Texto: `["fill","@e2","valor"]` (reemplaza) · teclas: `["press","Enter"]`, `["press","Tab"]`.
- `<select>` nativo: `["select","@e5","Colombia"]`. Menú hecho a mano: `["click","@e5"]` → snapshot → `["click","@eN"]` de la opción.
- Casillas: `["click","@e7"]` y comprueba con snapshot.
- Contraseñas, pagos y datos personales: NO los escribas; pídeselos al usuario o déjale el paso a él.

## Subir archivos
- `["upload","@e3","./foto.png"]`. Jamás hagas clic en el input de archivo: abre un diálogo del sistema que no se puede manejar.

## Diálogos (alert, confirm, prompt)
- `["dialog","status"]` · `["dialog","accept"]` · `["dialog","accept","texto"]` · `["dialog","dismiss"]`.

## iframes y shadow DOM
- El snapshot ya muestra lo de dentro de los shadow DOM: actúa por ref como siempre.
- iframe: `["frame","#embed"]` (o `"iframe[name='x']"`) → snapshot/acciones dentro → `["frame","main"]` para volver.

## Pestañas y ventanas emergentes
- `["tab","list"]` · `["tab","new","http://localhost:5173/admin"]` · cambiar: `["tab","t2"]`.
- Un enlace que abre pestaña nueva: `["tab","list"]` y cambia a ella antes de hacer snapshot.

## Avisos de cookies y capas que tapan
- Si un clic falla porque algo tapa el elemento, cierra el aviso (su botón por ref), haz snapshot y repite.

## Desplazar y carga perezosa
- `["scroll","down","800"]` y luego snapshot; para listas infinitas, repite hasta que no aparezca nada nuevo.

## Revisión de diseño (para páginas que se ven bien de verdad)
- Tamaños: `["set","viewport","390","844"]` (móvil) · `["set","viewport","768","1024"]` · `["set","viewport","1440","900"]`.
- Tema: `["set","media","dark"]` y `["set","media","light"]`.
- Mirar: `["screenshot","captura.png"]` y, si tu modelo ve, `view_image` de esa captura. Revisa desbordes, textos cortados, contraste y alineación.
- Antes/después: guarda una captura y compara con `["diff","screenshot","--baseline","antes.png"]`.
- Calidad: `["a11y"]` (axe: contraste, etiquetas, roles; corrige los graves) y `["vitals"]` (LCP, CLS, INP).
- Errores: `["console"]` y `["errors"]` deben quedar limpios.

## Al terminar
- `["close"]`. El servidor de desarrollo se para con `process` action=stop si ya no hace falta.
