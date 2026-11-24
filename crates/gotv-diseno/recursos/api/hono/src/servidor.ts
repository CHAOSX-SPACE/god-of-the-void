import { serve } from "@hono/node-server";
import { crearApp } from "./app";
import { leerConfig } from "./config";

const config = leerConfig();
// ⟦sólo esta máquina; para abrirlo a la red pon hostname: "0.0.0.0" a sabiendas¦this machine only; to open it to the network set hostname: "0.0.0.0" knowingly⟧
serve({ fetch: crearApp(config).fetch, port: config.puerto, hostname: "127.0.0.1" }, (i) => console.log(`http://localhost:${i.port}`));
