//! F23.L-1 · Las fichas legales de fábrica, escritas en Rust (sin archivos de otro formato).
//!
//! Cada una se escribió LEYENDO las condiciones oficiales del servicio el 2026-11-16; `notas_legales`
//! dice de qué cláusula sale cada regla (citas cortas) y `fuentes` lleva las URL. Este código no es
//! asesoría legal: antes del lanzamiento público, un abogado revisa las fichas.
//!
//! `condiciones_hash` es la huella del texto de las condiciones (`legal::huella_de_texto`), medida con
//! la prueba `buscar::tests::huellas_de_las_condiciones` (F23.L-2).

use super::legal::{Ficha, Limites, Par, Precio};

const REVISADA_POR: &str = "Opus 5.5 (hilo F23), sin abogado"; // juez-idioma: datos (firma de la revisión)
const REVISADAS: &str = "2026-11-16";

fn s(x: &str) -> String {
    x.to_string()
}

fn urls(v: &[&str]) -> Vec<String> {
    v.iter().map(|u| u.to_string()).collect()
}

/// Las 8 fichas de fábrica. Una ficha nueva = una función aquí, su línea en esta lista y su prueba.
pub fn de_fabrica() -> Vec<Ficha> {
    vec![duckduckgo(), wikipedia(), arxiv(), github(), brave(), serper(), tavily(), exa()]
}

fn duckduckgo() -> Ficha {
    // juez-idioma: bloque de datos (ficha legal: citas de las condiciones, con su par en inglés al lado)
    Ficha {
        id: s("duckduckgo"),
        nombre: s("DuckDuckGo (lite)"),
        publica: s("Duck Duck Go, Inc."),
        condiciones_url: s("https://duckduckgo.com/terms"),
        condiciones_revisadas: s(REVISADAS),
        condiciones_hash: s("sha256:8b328120f522a8e75432efd18493d4ffd8f29491647d478bae5460945b36af4d"),
        revisada_por: s(REVISADA_POR),
        // F23.L-9: sin API oficial de resultados ni permiso expreso para pedir su HTML; sigue como hoy
        // (sin ampliar el raspado) mientras el Portador decide. Nunca pasa a «revisada» sin esa decisión.
        estado: s("sin_revisar"),
        excepcion: s("F23.L-9"),
        guardar_resultados: s("no"),
        cache_horas: None,
        mostrar_al_usuario: true,
        atribucion: String::new(),
        prohibe_entrenar_ia: s("sin_dato"),
        limites: Limites {
            por_segundo: Some(1.0),
            por_mes: None,
            nota: Par { es: "sin dato oficial; GOTV manda como mucho una consulta por segundo", en: "no official figure; GOTV sends at most one query per second" },
        },
        precio: Precio {
            texto: Par { es: "sin llave ni pago (sin precio oficial publicado)", en: "no key or payment (no official price published)" },
            fecha: s(REVISADAS),
            fuente: s("https://duckduckgo.com/terms"),
        },
        dominio_api: s("lite.duckduckgo.com"),
        necesita_llave: false,
        donde_llave: String::new(),
        resumen: Par {
            es: "DuckDuckGo no tiene una API oficial de resultados ni dice si se pueden guardar: GOTV no los guarda y lo usa como hoy, marcado «sin revisar», hasta que decidas.",
            en: "DuckDuckGo has no official results API and does not say whether results may be stored: GOTV does not store them and uses it as today, marked «not reviewed», until you decide.",
        },
        notas_legales: Par {
            es: "- Condiciones (01-07-2025): «We expect you to use our Services as authorized, or we may otherwise suspend access.» No autorizan ni prohíben literalmente la consulta automática.\n\
- Uso aceptable: prohíbe «Interfere with or disrupt the integrity or performance of the services» y revender el servicio.\n\
- No hay API oficial de resultados web: la Instant Answer API (api.duckduckgo.com) da respuestas instantáneas, no resultados, y su robots.txt dice «Disallow: /».\n\
- robots.txt: duckduckgo.com bloquea /lite y /html a todos; lite.duckduckgo.com dice «Allow: /» sólo para que se respeten sus noindex.\n\
- Medido el 2026-11-17: con un User-Agent honesto responde 202 «anomaly» (0 resultados); sólo responde con el de un navegador.\n\
- Guardar, atribución, límites, entrenar IA: sin dato → nada al disco (F23.L-3).\n\
- Decisión pendiente del Portador (F23.L-9): seguir así, retirarlo o cambiarlo.",
            en: "- Terms (2025-07-01): «We expect you to use our Services as authorized, or we may otherwise suspend access.» They neither authorize nor literally forbid automated querying.\n\
- Acceptable use: forbids «Interfere with or disrupt the integrity or performance of the services» and reselling the service.\n\
- There is no official web results API: the Instant Answer API (api.duckduckgo.com) gives instant answers, not results, and its robots.txt says «Disallow: /».\n\
- robots.txt: duckduckgo.com blocks /lite and /html for everyone; lite.duckduckgo.com says «Allow: /» only so its noindex is respected.\n\
- Measured 2026-11-17: with an honest User-Agent it answers 202 «anomaly» (0 results); it only answers a browser's.\n\
- Storing, attribution, limits, AI training: no data → nothing to disk (F23.L-3).\n\
- Pending decision of the owner (F23.L-9): keep it, withdraw it or replace it.",
        },
        fuentes: urls(&[
            "https://duckduckgo.com/terms",
            "https://duckduckgo.com/acceptable-use",
            "https://duckduckgo.com/robots.txt",
            "https://lite.duckduckgo.com/robots.txt",
            "https://duckduckgo.com/duckduckgo-help-pages/results/sources",
        ]),
    }
}

fn wikipedia() -> Ficha {
    // juez-idioma: bloque de datos (ficha legal: citas de las condiciones, con su par en inglés al lado)
    Ficha {
        id: s("wikipedia"),
        nombre: s("Wikipedia"),
        publica: s("Wikimedia Foundation, Inc."),
        condiciones_url: s("https://foundation.wikimedia.org/wiki/Policy:Terms_of_Use"),
        condiciones_revisadas: s(REVISADAS),
        condiciones_hash: s("sha256:71ce323150f8b8970ad80a0af74a0fce34841915db883ae10a08f76f7ffaa834"),
        revisada_por: s(REVISADA_POR),
        estado: s("revisada"),
        excepcion: String::new(),
        guardar_resultados: s("si"),
        cache_horas: None,
        mostrar_al_usuario: true,
        atribucion: s("Wikipedia (CC BY-SA 4.0)"),
        prohibe_entrenar_ia: s("no"),
        limites: Limites {
            por_segundo: Some(1.0),
            por_mes: None,
            nota: Par { es: "en serie; < 5/s (Robot policy); 10/min sin User-Agent con contacto, 200/min con él", en: "serial; < 5/s (Robot policy); 10/min without a User-Agent with contact, 200/min with it" },
        },
        precio: Precio { texto: Par { es: "gratis", en: "free" }, fecha: s(REVISADAS), fuente: s("https://foundation.wikimedia.org/wiki/Policy:Terms_of_Use") },
        dominio_api: s("es.wikipedia.org"),
        necesita_llave: false,
        donde_llave: String::new(),
        resumen: Par {
            es: "Wikipedia permite guardar (CC BY-SA 4.0, con enlace al artículo): GOTV cachea los resultados con su procedencia y los cita con su enlace.",
            en: "Wikipedia allows storing (CC BY-SA 4.0, with a link to the article): GOTV caches results with their provenance and cites them with their link.",
        },
        notas_legales: Par {
            es: "- Acceso automatizado permitido salvo el abusivo: prohíbe «automated uses of the Project Websites that are abusive or disruptive» (Terms of Use §4); al usar las API se aceptan sus políticas (§12).\n\
- Guardar: la etiqueta de la API dice «If your requests obtain data that can be cached for a while, you should take steps to cache it»; sin plazo.\n\
- Licencia: CC BY-SA 4.0 y GFDL (§7); atribución «Through hyperlink (where possible) or URL to the page». GOTV cita siempre el enlace.\n\
- User-Agent obligatorio con contacto (correo, sitio o usuario de la wiki); el de un navegador «will be assumed malicious». El contacto lo pone el usuario en [buscadores] contacto.\n\
- Límites: en serie, < 5/s (Robot policy); 10/min sin identificar, 200/min con User-Agent conforme. GOTV: 1/s.\n\
- Tensión sin resolver en las fuentes: robots.txt dice «Disallow: /w/» a los robots genéricos y las pautas de la API regulan ese mismo uso. GOTV es un cliente de la API, no un rastreador.",
            en: "- Automated access allowed unless abusive: it forbids «automated uses of the Project Websites that are abusive or disruptive» (Terms of Use §4); using the APIs means accepting their policies (§12).\n\
- Storing: the API etiquette says «If your requests obtain data that can be cached for a while, you should take steps to cache it»; no time limit.\n\
- License: CC BY-SA 4.0 and GFDL (§7); attribution «Through hyperlink (where possible) or URL to the page». GOTV always cites the link.\n\
- User-Agent required with contact (email, site or wiki user); a browser's «will be assumed malicious». The user sets the contact in [buscadores] contacto.\n\
- Limits: serial, < 5/s (Robot policy); 10/min unidentified, 200/min with a compliant User-Agent. GOTV: 1/s.\n\
- Unresolved tension in the sources: robots.txt says «Disallow: /w/» to generic robots while the API guidelines regulate that same use. GOTV is an API client, not a crawler.",
        },
        fuentes: urls(&[
            "https://foundation.wikimedia.org/wiki/Policy:Terms_of_Use",
            "https://foundation.wikimedia.org/wiki/Policy:Wikimedia_Foundation_API_Usage_Guidelines",
            "https://foundation.wikimedia.org/wiki/Policy:Wikimedia_Foundation_User-Agent_Policy",
            "https://wikitech.wikimedia.org/wiki/Robot_policy",
            "https://www.mediawiki.org/wiki/API:Etiquette",
            "https://www.mediawiki.org/wiki/Wikimedia_APIs/Rate_limits",
            "https://www.mediawiki.org/wiki/API:REST_API/Reference",
        ]),
    }
}

fn arxiv() -> Ficha {
    // juez-idioma: bloque de datos (ficha legal: citas de las condiciones, con su par en inglés al lado)
    Ficha {
        id: s("arxiv"),
        nombre: s("arXiv"),
        publica: s("Cornell University (arXiv)"),
        condiciones_url: s("https://info.arxiv.org/help/api/tou.html"),
        condiciones_revisadas: s(REVISADAS),
        condiciones_hash: s("sha256:222183b24657c81821e29b291af1e200f486c5a400ae4286cf3efdd9b0ee99f6"),
        revisada_por: s(REVISADA_POR),
        estado: s("revisada"),
        excepcion: String::new(),
        guardar_resultados: s("si"),
        cache_horas: None,
        mostrar_al_usuario: true,
        atribucion: s("Thank you to arXiv for use of its open access interoperability."),
        prohibe_entrenar_ia: s("sin_dato"),
        limites: Limites {
            por_segundo: Some(1.0 / 3.0),
            por_mes: None,
            nota: Par { es: "una petición cada 3 s y una sola conexión", en: "one request every 3 s and a single connection" },
        },
        precio: Precio { texto: Par { es: "gratis", en: "free" }, fecha: s(REVISADAS), fuente: s("https://info.arxiv.org/help/api/tou.html") },
        dominio_api: s("export.arxiv.org"),
        necesita_llave: false,
        donde_llave: String::new(),
        resumen: Par {
            es: "arXiv permite guardar sus metadatos (CC0): GOTV cachea título, resumen y enlace; nunca los PDF.",
            en: "arXiv allows storing its metadata (CC0): GOTV caches title, abstract and link; never the PDFs.",
        },
        notas_legales: Par {
            es: "- Metadatos (título, resumen, autores, identificadores) bajo CC0 1.0; permitido «Retrieve, store, transform, and share descriptive metadata about arXiv e-prints».\n\
- El manual pide cachear: «Please cache your results.»\n\
- Contenido (PDF, fuentes): no se guarda ni se sirve; GOTV sólo guarda metadatos.\n\
- Atribución exigida: «Thank you to arXiv for use of its open access interoperability.» Sin sugerir respaldo de arXiv.\n\
- Límite: «make no more than one request every three seconds, and limit requests to a single connection at a time», para todas las máquinas del usuario. GOTV lo hace cumplir.\n\
- Entrenar IA: sin dato (metadatos CC0; los artículos tienen su propio copyright).",
            en: "- Metadata (title, abstract, authors, identifiers) under CC0 1.0; allowed to «Retrieve, store, transform, and share descriptive metadata about arXiv e-prints».\n\
- The manual asks for caching: «Please cache your results.»\n\
- Content (PDFs, sources): not stored or served; GOTV only stores metadata.\n\
- Required attribution: «Thank you to arXiv for use of its open access interoperability.» Without implying arXiv's endorsement.\n\
- Limit: «make no more than one request every three seconds, and limit requests to a single connection at a time», across all the user's machines. GOTV enforces it.\n\
- AI training: no data (CC0 metadata; papers have their own copyright).",
        },
        fuentes: urls(&[
            "https://info.arxiv.org/help/api/tou.html",
            "https://info.arxiv.org/help/api/index.html",
            "https://info.arxiv.org/help/api/user-manual.html",
        ]),
    }
}

fn github() -> Ficha {
    // juez-idioma: bloque de datos (ficha legal: citas de las condiciones, con su par en inglés al lado)
    Ficha {
        id: s("github"),
        nombre: s("GitHub (repositorios)"),
        publica: s("GitHub, Inc."),
        condiciones_url: s("https://docs.github.com/en/site-policy/github-terms/github-terms-of-service"),
        condiciones_revisadas: s(REVISADAS),
        condiciones_hash: s("sha256:c8f13a966dafb0ac2a8c04e2d87585b75613af87a28401b9047feced83d200bf"),
        revisada_por: s(REVISADA_POR),
        estado: s("revisada"),
        excepcion: String::new(),
        guardar_resultados: s("no"),
        cache_horas: None,
        mostrar_al_usuario: true,
        atribucion: String::new(),
        prohibe_entrenar_ia: s("no"),
        limites: Limites {
            por_segundo: Some(1.0 / 6.0),
            por_mes: None,
            nota: Par { es: "búsqueda: 10/min sin autenticar, 30/min con token", en: "search: 10/min unauthenticated, 30/min with a token" },
        },
        precio: Precio { texto: Par { es: "gratis (sin cifra para la API pública)", en: "free (no figure for the public API)" }, fecha: s(REVISADAS), fuente: s("https://docs.github.com/en/site-policy/github-terms/github-terms-of-service") },
        dominio_api: s("api.github.com"),
        necesita_llave: false,
        donde_llave: String::new(),
        resumen: Par {
            es: "GitHub no fija una regla para guardar lo que da su API y trae datos personales (dueños): GOTV no guarda sus resultados.",
            en: "GitHub sets no rule for storing what its API returns and it carries personal data (owners): GOTV does not store its results.",
        },
        notas_legales: Par {
            es: "- Uso por la API aceptado con la sección H de los Terms of Service; «Abuse or excessively frequent requests to GitHub via the API may result in the temporary or permanent suspension».\n\
- Guardar: sin plazo ni permiso expreso; la AUP §6 prohíbe «reproduce, duplicate, copy, sell, resell or exploit any portion of the Service» sin permiso, y la §8 limita los datos personales al fin que el usuario autorizó → GOTV no guarda nada (lo más estricto ante el «sin dato»).\n\
- Entrenar IA: sin prohibición general (ToS D.9 sólo trata la reciprocidad de acceso).\n\
- Límite de búsqueda: 10/min sin autenticar, 30/min con token; en serie. GOTV: una cada 6 s.\n\
- User-Agent obligatorio (sin él, 403): GOTV manda GOTV/<versión>. El token (si lo hay) sólo va a api.github.com.",
            en: "- API use accepted under section H of the Terms of Service; «Abuse or excessively frequent requests to GitHub via the API may result in the temporary or permanent suspension».\n\
- Storing: no time limit or express permission; AUP §6 forbids «reproduce, duplicate, copy, sell, resell or exploit any portion of the Service» without permission, and §8 limits personal data to the purpose the user authorized → GOTV stores nothing (the strictest reading of «no data»).\n\
- AI training: no general ban (ToS D.9 only covers access reciprocity).\n\
- Search limit: 10/min unauthenticated, 30/min with a token; serial. GOTV: one every 6 s.\n\
- User-Agent required (without it, 403): GOTV sends GOTV/<version>. The token (if any) only goes to api.github.com.",
        },
        fuentes: urls(&[
            "https://docs.github.com/en/site-policy/github-terms/github-terms-of-service",
            "https://docs.github.com/en/site-policy/acceptable-use-policies/github-acceptable-use-policies",
            "https://docs.github.com/en/rest/search/search",
            "https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api",
            "https://docs.github.com/en/rest/using-the-rest-api/getting-started-with-the-rest-api",
        ]),
    }
}

fn brave() -> Ficha {
    // juez-idioma: bloque de datos (ficha legal: citas de las condiciones, con su par en inglés al lado)
    Ficha {
        id: s("brave"),
        nombre: s("Brave Search API"),
        publica: s("Brave Software, Inc."),
        condiciones_url: s("https://api-dashboard.search.brave.com/terms-of-service"),
        condiciones_revisadas: s(REVISADAS),
        condiciones_hash: s("sha256:2b87da036cc877d64eb943031eb03b391bbbfc8706bce95597af1bd1cb97261e"),
        revisada_por: s(REVISADA_POR),
        estado: s("revisada"),
        excepcion: String::new(),
        guardar_resultados: s("no"),
        cache_horas: None,
        mostrar_al_usuario: true,
        atribucion: String::new(),
        prohibe_entrenar_ia: s("si"),
        limites: Limites {
            por_segundo: Some(5.0),
            por_mes: None,
            nota: Par { es: "Search: 50 por segundo; prepago, sin tope mensual propio", en: "Search: 50 per second; prepaid, no monthly cap of its own" },
        },
        precio: Precio {
            texto: Par { es: "5 USD por 1 000 búsquedas; 5 USD de crédito gratis al mes (≈ 1 000 búsquedas)", en: "5 USD per 1,000 searches; 5 USD of free credit per month (≈ 1,000 searches)" },
            fecha: s(REVISADAS),
            fuente: s("https://brave.com/search/api/"),
        },
        dominio_api: s("api.search.brave.com"),
        necesita_llave: true,
        donde_llave: s("https://api-dashboard.search.brave.com/app/keys"),
        resumen: Par {
            es: "Brave prohíbe guardar sus resultados (sólo lo transitorio para usarlos) y entrenar IA con ellos: GOTV no los guarda.",
            en: "Brave forbids storing its results (only transient use) and training AI with them: GOTV does not store them.",
        },
        notas_legales: Par {
            es: "- «SEARCH API TERMS OF USE» (01-09-2026) §3(b)(i): no «store, cache, or create a database of Search Results, in whole or in part, other than transient storage required for operation» → GOTV no guarda nada (ni caché ni índice).\n\
- Ayuda oficial: «It is prohibited to retain any and all data received through the Brave Search API.» Guardar sólo con un plan a medida («storage rights»); GOTV no puede saberlo, así que no guarda.\n\
- §3(b)(xiii): prohibido usar los resultados para «create, evaluate, train, re-train, fine-tune, benchmark» IA. Dárselos al modelo para responder no está prohibido expresamente.\n\
- §3(b)(xii): no redistribuir ni revender. Cada usuario pone SU llave: el «Customer» es él.\n\
- Atribución opcional (§4(d)); si se pone, «POWERED BY BRAVE» con su logo. GOTV no la pone.\n\
- Precio: ya no hay plan gratis aparte; «$5 per 1,000 requests» e «Includes $5 in free credits every month». La fecha del cambio (febrero de 2026) sólo la dan terceros.",
            en: "- «SEARCH API TERMS OF USE» (2026-09-01) §3(b)(i): no «store, cache, or create a database of Search Results, in whole or in part, other than transient storage required for operation» → GOTV stores nothing (no cache, no index).\n\
- Official help: «It is prohibited to retain any and all data received through the Brave Search API.» Storing only with a custom plan («storage rights»); GOTV cannot know it, so it does not store.\n\
- §3(b)(xiii): results may not be used to «create, evaluate, train, re-train, fine-tune, benchmark» AI. Giving them to the model to answer is not expressly forbidden.\n\
- §3(b)(xii): no redistribution or resale. Each user sets THEIR key: they are the «Customer».\n\
- Optional attribution (§4(d)); if shown, «POWERED BY BRAVE» with its logo. GOTV does not show it.\n\
- Price: no separate free plan anymore; «$5 per 1,000 requests» and «Includes $5 in free credits every month». The date of the change (February 2026) only comes from third parties.",
        },
        fuentes: urls(&[
            "https://api-dashboard.search.brave.com/terms-of-service",
            "https://api-dashboard.search.brave.com/documentation/resources/help-feedback",
            "https://brave.com/search/api/",
            "https://api-dashboard.search.brave.com/documentation/quickstart",
            "https://api-dashboard.search.brave.com/api-reference/web/search/get",
        ]),
    }
}

fn serper() -> Ficha {
    // juez-idioma: bloque de datos (ficha legal: citas de las condiciones, con su par en inglés al lado)
    Ficha {
        id: s("serper"),
        nombre: s("Serper"),
        publica: s("Serper.dev (sin razón social publicada; ley del Reino Unido)"),
        condiciones_url: s("https://serper.dev/terms"),
        condiciones_revisadas: s(REVISADAS),
        condiciones_hash: s("sha256:001b9eece18d1d2ee921f0c7bae4f214f880d45da489c2d318ed4475aa6c3632"),
        revisada_por: s(REVISADA_POR),
        estado: s("revisada"),
        excepcion: String::new(),
        guardar_resultados: s("no"),
        cache_horas: None,
        mostrar_al_usuario: true,
        atribucion: String::new(),
        prohibe_entrenar_ia: s("sin_dato"),
        limites: Limites {
            por_segundo: Some(5.0),
            por_mes: None,
            nota: Par { es: "50 por segundo en Starter; capa gratis sin dato", en: "50 per second on Starter; free tier unknown" },
        },
        precio: Precio {
            texto: Par { es: "2 500 consultas gratis una vez; luego desde 1 USD por 1 000 (50 USD por 50 000, válidos 6 meses)", en: "2,500 free queries once; then from 1 USD per 1,000 (50 USD for 50,000, valid 6 months)" },
            fecha: s(REVISADAS),
            fuente: s("https://serper.dev/"),
        },
        dominio_api: s("google.serper.dev"),
        necesita_llave: true,
        donde_llave: s("https://serper.dev/api-keys"),
        resumen: Par {
            es: "Serper no dice si se pueden guardar sus resultados (que son de Google): GOTV no los guarda.",
            en: "Serper does not say whether its results (which are Google's) may be stored: GOTV does not store them.",
        },
        notas_legales: Par {
            es: "- Condiciones (29-05-2024): nada sobre guardar, cachear o usar para IA (sin dato) → GOTV no guarda (F23.L-3).\n\
- Data License: no falsear la propiedad o el origen ni quitar avisos («Misrepresent the ownership or the source»). GOTV cita la fuente de cada resultado.\n\
- Los resultados son de Google y Serper «is not affiliated with or endorsed by Google»; sus condiciones no cubren lo que Google permita: riesgo de quien pone la llave.\n\
- Servicio «business-to-business»: el usuario contrata con su propia cuenta y llave.\n\
- Llave en la cabecera X-API-KEY, sólo a google.serper.dev.",
            en: "- Terms (2024-05-29): nothing about storing, caching or AI use (no data) → GOTV does not store (F23.L-3).\n\
- Data License: do not misrepresent ownership or origin or remove notices («Misrepresent the ownership or the source»). GOTV cites each result's source.\n\
- Results are Google's and Serper «is not affiliated with or endorsed by Google»; its terms do not cover what Google allows: a risk for whoever sets the key.\n\
- «business-to-business» service: the user contracts with their own account and key.\n\
- Key in the X-API-KEY header, only to google.serper.dev.",
        },
        fuentes: urls(&["https://serper.dev/terms", "https://serper.dev/privacy", "https://serper.dev/"]),
    }
}

fn tavily() -> Ficha {
    // juez-idioma: bloque de datos (ficha legal: citas de las condiciones, con su par en inglés al lado)
    Ficha {
        id: s("tavily"),
        nombre: s("Tavily"),
        publica: s("AlphaAI Technologies Inc. (Tavily)"),
        condiciones_url: s("https://www.tavily.com/terms"),
        condiciones_revisadas: s(REVISADAS),
        condiciones_hash: s("sha256:9424b24c6cb193ea43b750f9820e1d241255a8aed65ba70c8212c33e3090c8d5"),
        revisada_por: s(REVISADA_POR),
        estado: s("revisada"),
        excepcion: String::new(),
        guardar_resultados: s("no"),
        cache_horas: None,
        mostrar_al_usuario: true,
        atribucion: String::new(),
        prohibe_entrenar_ia: s("sin_dato"),
        limites: Limites {
            por_segundo: Some(1.0),
            por_mes: Some(1_000),
            nota: Par { es: "100/min en la capa gratis; 1 000 créditos gratis al mes (búsqueda basic = 1 crédito)", en: "100/min on the free tier; 1,000 free credits per month (basic search = 1 credit)" },
        },
        precio: Precio {
            texto: Par { es: "1 000 créditos gratis al mes; luego 0,008 USD por crédito (≈ 8 USD por 1 000 búsquedas basic)", en: "1,000 free credits per month; then 0.008 USD per credit (≈ 8 USD per 1,000 basic searches)" },
            fecha: s(REVISADAS),
            fuente: s("https://www.tavily.com/pricing"),
        },
        dominio_api: s("api.tavily.com"),
        necesita_llave: true,
        donde_llave: s("https://app.tavily.com"),
        resumen: Par {
            es: "Tavily no dice si se pueden guardar sus resultados: GOTV no los guarda. Ojo: Tavily puede usar tus consultas para entrenar sus modelos.",
            en: "Tavily does not say whether its results may be stored: GOTV does not store them. Note: Tavily may use your queries to train its models.",
        },
        notas_legales: Par {
            es: "- Condiciones (04-05-2026): no permiten ni prohíben guardar el «Output» (sin dato) → GOTV no guarda (F23.L-3).\n\
- Privacidad: Tavily recibe una licencia «perpetual, and irrevocable» sobre lo que se le manda (§9.2) y puede entrenar con Input y Output «for purposes of training, improving, developing, and enhancing artificial intelligence models» (§6.5).\n\
- Prohíbe usar el Output «to develop models that compete with the Company» (§6.6(i)); la llave no se comparte.\n\
- Llave en la cabecera Authorization: Bearer, sólo a api.tavily.com.\n\
- Límites: 100/min en la capa gratis, compartidos por todas las llaves de la cuenta.",
            en: "- Terms (2026-05-04): they neither allow nor forbid storing the «Output» (no data) → GOTV does not store (F23.L-3).\n\
- Privacy: Tavily gets a «perpetual, and irrevocable» license over what is sent (§9.2) and may train on Input and Output «for purposes of training, improving, developing, and enhancing artificial intelligence models» (§6.5).\n\
- Forbids using the Output «to develop models that compete with the Company» (§6.6(i)); the key is not shared.\n\
- Key in the Authorization: Bearer header, only to api.tavily.com.\n\
- Limits: 100/min on the free tier, shared by all the account's keys.",
        },
        fuentes: urls(&[
            "https://www.tavily.com/terms",
            "https://www.tavily.com/acceptable-use-policy",
            "https://www.tavily.com/pricing",
            "https://docs.tavily.com/documentation/rate-limits",
            "https://docs.tavily.com/documentation/api-reference/endpoint/search",
        ]),
    }
}

fn exa() -> Ficha {
    // juez-idioma: bloque de datos (ficha legal: citas de las condiciones, con su par en inglés al lado)
    Ficha {
        id: s("exa"),
        nombre: s("Exa"),
        publica: s("Exa Labs, Inc."),
        condiciones_url: s("https://exa.ai/assets/Exa_Labs_Terms_of_Service.pdf"),
        condiciones_revisadas: s(REVISADAS),
        condiciones_hash: s("sha256:1669a8cf8c161a0cb8bbf1f640e13ccda537e6243b1f4325565f51a1a844d11b"),
        revisada_por: s(REVISADA_POR),
        estado: s("revisada"),
        excepcion: String::new(),
        guardar_resultados: s("no"),
        cache_horas: None,
        mostrar_al_usuario: true,
        atribucion: String::new(),
        prohibe_entrenar_ia: s("sin_dato"),
        limites: Limites {
            por_segundo: Some(5.0),
            por_mes: None,
            nota: Par { es: "/search: 10 por segundo por equipo", en: "/search: 10 per second per team" },
        },
        precio: Precio {
            texto: Par { es: "10 USD de crédito gratis al mes; búsqueda auto 7 USD por 1 000 (lite 4 USD)", en: "10 USD of free credit per month; auto search 7 USD per 1,000 (lite 4 USD)" },
            fecha: s(REVISADAS),
            fuente: s("https://exa.ai/pricing"),
        },
        dominio_api: s("api.exa.ai"),
        necesita_llave: true,
        donde_llave: s("https://dashboard.exa.ai/api-keys"),
        resumen: Par {
            es: "Exa prohíbe copiar o guardar lo que devuelve (sólo la caché temporal para mostrarlo): GOTV no guarda nada de Exa.",
            en: "Exa forbids copying or storing what it returns (only the temporary cache to display it): GOTV stores nothing from Exa.",
        },
        notas_legales: Par {
            es: "- §4.2(a) prohíbe «download, modify, copy, distribute, transmit, display, … reproduce … any information … obtained from or through, the Services», salvo «temporary files that are automatically cached by your web browser for display purposes» → GOTV no guarda nada (ni caché ni índice).\n\
- Mostrar al usuario dentro del uso del servicio: cubierto por la licencia de §4.1; republicar no.\n\
- §4.2(f): no usar el Output para un producto competidor. Entrenar IA: sin prohibición expresa (sin dato).\n\
- Exa recibe una licencia «perpetual and irrevocable» sobre las consultas (§1.2(c)).\n\
- El PDF deja «Last Revised» en blanco y remite a un Master Subscription Agreement no publicado (sin dato).\n\
- Llave en la cabecera x-api-key, sólo a api.exa.ai. GOTV no pide el texto de las páginas (se cobra aparte): sólo título y enlace.",
            en: "- §4.2(a) forbids «download, modify, copy, distribute, transmit, display, … reproduce … any information … obtained from or through, the Services», except «temporary files that are automatically cached by your web browser for display purposes» → GOTV stores nothing (no cache, no index).\n\
- Showing it to the user within the service's use: covered by the §4.1 license; republishing is not.\n\
- §4.2(f): the Output may not be used for a competing product. AI training: no express ban (no data).\n\
- Exa gets a «perpetual and irrevocable» license over the queries (§1.2(c)).\n\
- The PDF leaves «Last Revised» blank and refers to an unpublished Master Subscription Agreement (no data).\n\
- Key in the x-api-key header, only to api.exa.ai. GOTV does not request page text (billed separately): only title and link.",
        },
        fuentes: urls(&[
            "https://exa.ai/assets/Exa_Labs_Terms_of_Service.pdf",
            "https://exa.ai/pricing",
            "https://docs.exa.ai/reference/rate-limits",
            "https://docs.exa.ai/reference/search",
        ]),
    }
}
