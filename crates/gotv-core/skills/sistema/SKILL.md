---
name: sistema
disparadores: batería, bateria, volumen, silencia, silencio, brillo, modo oscuro, modo claro, bajo consumo, apaga la pantalla, espacio libre, espacio en disco, memoria ram, ram, procesador, cpu, wifi, ip, macos, encendida, prendida, uptime, battery, volume, brightness, dark mode, processor
description: La Mac por comando — batería, volumen, silencio, brillo, modo oscuro, modo de bajo consumo, apagar o bloquear la pantalla, espacio en disco, memoria, procesador, CPU, qué proceso consume más, red, IP, wifi, versión de macOS, cuánto lleva encendida. Úsala cuando pidan mirar o cambiar algo del SISTEMA o del equipo (también por voz). Todo con `bash`, sin abrir Ajustes del Sistema ni hacer clics.
---

# El sistema por comando (medido en esta Mac, 2026-11-11)

Para el SISTEMA, comandos: nunca abras Ajustes del Sistema ni Monitor de Actividad a clics (no
responden a tiempo). Dos caminos: lo que dice `bash` va por `bash`; lo de AppleScript (volumen,
brillo, modo oscuro) va por la herramienta `desktop` con `["applescript", "…"]` — NUNCA `osascript`
desde `bash` (la caja no le llega). `desktop` necesita Control Mac encendido; si está apagado, dilo. Mirar no pide permiso; cambiar algo de administrador va con `sudo` (macOS le
pide al usuario contraseña o Touch ID). Responde con el dato, corto: por voz se escucha entero.

## Mirar (una orden, sin permiso)

| Pregunta | Orden | Cómo leerla |
|---|---|---|
| batería | `pmset -g batt` | «100%; charged» → «al 100 %, cargada» |
| espacio en disco | `df -h /System/Volumes/Data` | columna Avail = libre (NO uses `df -h /`: es el volumen del sistema) |
| memoria y CPU ahora | `top -l 1 -n 0 \| grep -E 'PhysMem\|CPU usage'` | usado/libre y % ocioso |
| qué consume más CPU | `ps -Ao pcpu,comm -r \| head -6` | el primero es el que más gasta |
| qué consume más memoria | `ps -Ao pmem,comm -m \| head -6` | |
| procesador | `sysctl -n machdep.cpu.brand_string` | «Apple M1 Max» |
| el equipo entero | `system_profiler SPHardwareDataType` | chip, núcleos, memoria, serie |
| versión de macOS | `sw_vers -productVersion` | |
| IP local | `ipconfig getifaddr en0` | |
| volumen | `desktop` `["applescript", "output volume of (get volume settings)"]` | 0–100 |
| modo oscuro | `desktop` `["applescript", "tell application \"System Events\" to tell appearance preferences to get dark mode"]` | true/false |
| modo de energía | `pmset -g \| grep -E 'powermode\|lowpowermode'` | `powermode` 0 auto · 1 bajo · 2 alto (MacBook Pro); `lowpowermode` 0/1 en otras |
| cuánto lleva encendida | `uptime` | |

**El nombre del wifi**: macOS lo oculta (`<redacted>`) por privacidad de ubicación en TODAS las
órdenes. Dilo así; no lo inventes. Sí puedes decir si hay conexión (`ipconfig getifaddr en0`).

## Cambiar

| Pedido | Orden |
|---|---|
| volumen a N | `desktop` `["applescript", "set volume output volume N"]` |
| silenciar / quitar silencio | `desktop` `["applescript", "set volume with output muted"]` / `"set volume without output muted"` |
| subir / bajar brillo un paso | `desktop` `["applescript", "tell application \"System Events\" to key code 144"]` (sube) / `145` (baja) |
| modo oscuro / claro | `desktop` `["applescript", "tell application \"System Events\" to tell appearance preferences to set dark mode to true"]` (`false` = claro) |
| apagar la pantalla ya | `pmset displaysleepnow` |
| bajo consumo | `sudo pmset -a powermode 1` DE UNA VEZ (macOS pide contraseña o Touch ID: esa es la aprobación; no preguntes antes). Si la Mac no tiene `powermode` (mira `pmset -g`), `sudo pmset -a lowpowermode 1`. Automático: `powermode 0` · alto rendimiento: `powermode 2` |
| abrir / cerrar una app | `open -a "Safari"` / herramienta `desktop` |

- «En modo bajo consumo», «modo oscuro», «volumen 30» SIN verbo (la voz a veces lo corta) = PONLA así.
- Para varios pasos de brillo, repite la orden (cada una es un paso de la tecla).
- Tras cambiar algo, LÉELO de nuevo y di el valor real («volumen en 30»).
- Apagar, reiniciar o borrar el sistema NO: que lo haga el usuario.
