# Qué tiene dentro GOD OF THE VOID

Aquí está cada parte del agente, qué hace y hasta dónde llegamos con ella. Todo el código
vive en este repositorio: 158.698 líneas de Rust en 302 archivos, más 11.497 de la
interfaz. La mayor parte del agente está en `crates/gotv-core`, que tiene 117 módulos (sí,
117).

Casi todas las partes se probaron dos veces. Primero con sus propias pruebas y después con
una tarea real en un modelo local, uno de expertos de 35B (una variante de
Qwen3.6-35B-A3B, en 4 bits). Una tarea cuenta como hecha cuando un juez oculto revisa el
resultado, no cuando el modelo dice que terminó. Los números de abajo salen de esas
corridas, en un M1 Max con 64 GB.

✅ quiere decir hecho y medido con una tarea real. ◐ quiere decir hecho, pero con una parte
que todavía no se probó con el equipo de verdad.

[English](BODY.md) · [Volver al README](README.es.md)

---

## La base

✅ El ciclo de herramientas, ediciones que aguantan pequeñas diferencias y salidas largas
que van a un archivo en vez de llenar el contexto. Los trabajos largos se compactan para
que el modelo no pierda el hilo.

Con el modelo de 35B un trabajo largo terminó en 17 pasos y 3 minutos, sin tener que
compactar ni una vez. Con uno de 9B ya no dice que terminó antes de tiempo, pero compacta
más o menos cada 15 pasos y llegó a nuestro tope de 30 minutos. Los modelos chicos todavía
sufren aquí, la verdad.

## Entender el código

✅ Servidores de lenguaje para muchos lenguajes, que el agente arranca y administra. Los
que no están en tu Mac se bajan de su publicación oficial y se comprueban contra una
SHA-256 fija (zls, lua-language-server, taplo y marksman, por ejemplo). Después de cada
edición el agente lee los diagnósticos nuevos, agrupados por archivo.

✅ Un mapa del repositorio, para que el modelo encuentre las cosas sin leer cada archivo.
En nuestras corridas ayudó a buscar, aunque al editar le cuesta contexto.

✅ Paquetes por lenguaje que saben cómo corre las pruebas cada ecosistema. Medimos 8 tareas
del tipo «corre la prueba X» en varios lenguajes.

## Probar y depurar

✅ Una herramienta `pruebas` con 14 corredores. Antes de que el agente pueda cerrar una
tarea, tienen que pasar las pruebas que tocan los archivos que cambió.

✅ Un depurador por DAP. Medido con debugpy para Python y con lldb-dap para Rust. El de Go
(dlv) está conectado, pero sin medir.

## Seguridad

✅ Cada orden corre dentro de una caja de macOS (Seatbelt) que sólo puede escribir dentro
de tu proyecto. Las reglas de permisos deciden qué corre, qué pregunta y qué se niega.

✅ Para las órdenes que las reglas no saben decidir, las mira un modelo juez. Con nuestro
juego de 100 órdenes seguras y 100 peligrosas, las reglas solas dejaron pasar 0 peligrosas
y 99 seguras. Con el juez encima, las 100 seguras corrieron sin preguntar y no se aprobó
ninguna peligrosa. Ninguna.

✅ Los secretos no salen hacia los modelos de nube, y los permisos se pueden limitar a una
ruta o a una herramienta.

✅ Las dependencias nuevas se revisan antes de instalarlas. En nuestras corridas se
frenaron los nombres de paquete que imitan a otros y las versiones con vulnerabilidades
conocidas.

## Git y pull requests

✅ Las lecturas de git van por gix, que medimos más rápido que llamar al binario `git`.
Las escrituras usan el binario real, y empujar siempre pregunta antes. El agente puede
abrir un PR, leer sus comentarios y su CI y arreglar una compilación en rojo. Eso sí,
nunca fusiona.

## Datos

✅ SQLite, Postgres, MySQL/MariaDB y DuckDB, más archivos CSV y Parquet a través de
DuckDB. Las conexiones pueden ir por un túnel SSH, las contraseñas van al Llavero y la
sólo lectura la impone el motor, no el prompt. Medimos SQLite, Postgres, una base remota y
una prueba de ataque contra ella.

## Conexiones

✅ Cliente y servidor MCP, por stdio, HTTP, SSE y WebSocket, con OAuth propio. Medimos un
servidor con 40 herramientas, imágenes y OAuth. Los servidores MCP locales corren dentro
de la misma caja.

✅ ACP, para que editores como Zed manejen al agente. Con el motor local, un cambio pedido
desde Zed volvió como diff, se aceptó y llegó al archivo en 12 segundos.

✅ Hooks, plugins propios y plugins de Claude Code. Control remoto desde tu teléfono con un
código QR, por un túnel privado.

## Contenedores

✅ Docker y docker compose, fuera de la caja pero con límites fijos. Los archivos compose
se normalizan primero, y medimos un proyecto compose de punta a punta.

## Varios agentes

✅ Un turno puede lanzar varios subagentes. Cada uno trabaja en su propia rama, y un
verificador aparte (que no puede editar) revisa el resultado. Con modelos de nube corren
en paralelo. Con modelos locales van por turnos.

En una corrida local, un equipo de 3 subagentes terminó en 11 pasos y 300 segundos, con el
PASA del verificador y el juez oculto en verde. Con un modelo chico de nube, el mismo
trabajo tardó 86 segundos en paralelo contra 118 uno tras otro. Crear y fusionar una rama
tarda unos 63 ms en un proyecto de 300 archivos, sin lanzar `git`.

## Formas de trabajar

✅ Reglas por carpeta, objetivos que se comprueban por texto o por pruebas, un modo
entrevista y un modo para aprender. No gastan contexto si no los usas.

✅ Le puedes escribir mientras trabaja y cambia el rumbo sin detenerse. Agentes a medida,
proyectos en otra máquina por SSH (con el servidor de lenguaje corriendo allá), `/revisar`
para revisar un cambio y seguir un PR hasta que el CI quede en verde.

✅ Automatizaciones y avisos: programas un pedido, corre sin la ventana abierta y te avisa
en el escritorio cuando termina.

✅ Vigilar archivos, registros y sockets, y reaccionar. En nuestra prueba, un error escrito
en un registro desde afuera se encontró y se arregló en 9 segundos.

## Cuadernos

✅ Cuadernos de Jupyter con el kernel del propio proyecto, hablándole por ZeroMQ desde Rust.
En nuestra prueba el agente arregló una celda rota, añadió una gráfica de barras que queda
dentro del `.ipynb` y la miró, en 23 segundos.

## Navegador e investigación

✅ Su propio navegador y el control de Chrome, planes que puedes comentar línea por línea y
una grabación de lo que el agente hizo en el navegador.

✅ Investigación web en varios buscadores a la vez, con citas. En el modo «sólo local» no
sale a internet para nada, y eso también lo probamos.

## Móvil

◐ Simulador de iOS y emulador de Android: construir, instalar, lanzar y atrapar una app que
se cierra al abrir. Medido en 144 segundos en los dos. También crea dispositivos e instala
el SDK de Android desde Rust. Con teléfonos reales no se ha probado, porque no tenemos uno
en el banco.

## Equipos conectados

◐ USB, discos, impresoras, cámaras, Raspberry Pi y ESP32. La cámara está medida (un JPEG de
364 KB en 5,1 segundos). Lo demás todavía hay que probarlo con el equipo de verdad.

## La ventana

✅ Tres zonas armadas desde Rust, un panel de trabajo con pestañas, chats por proyecto, una
vista de cambios que funciona sin git, menciones, varias carpetas en un proyecto, chats
sin proyecto y búsqueda con ⌘K. Rebobinar deshace un turno entero, incluso entre carpetas.

---

¿Algo de aquí no coincide con lo que ves? [Abre un issue](https://github.com/CHAOSX-SPACE/god-of-the-void/issues).
