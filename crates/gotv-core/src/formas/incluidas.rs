//! F12 · paso 5 · las skills `entrevista` y `aprender`, DENTRO del binario (constantes de Rust, no archivos
//! de datos del repo), en los dos idiomas. Viajan como las demás incluidas (`skills::builtin_dir`), pero
//! nunca se sugieren solas en una orden (`skills::relevant` las salta): sólo llegan con `/entrevista` y
//! `/aprender`, o si el modelo las pide por nombre. Así no gastan contexto cuando no se usan.

/// Las que sólo se usan con su orden de barra (no se sugieren por palabras de la tarea).
pub const SOLO_CON_BARRA: [&str; 2] = ["entrevista", "aprender"];

/// `(nombre, SKILL.md)` en el idioma activo.
pub fn skills() -> [(&'static str, &'static str); 2] {
    [("entrevista", crate::i18n::t(es::ENTREVISTA, en::ENTREVISTA)), ("aprender", crate::i18n::t(es::APRENDER, en::APRENDER))]
}

/// El cuerpo (sin cabecera) de una incluida, en el idioma activo.
pub fn cuerpo(nombre: &str) -> Option<&'static str> {
    skills().into_iter().find(|(n, _)| *n == nombre).map(|(_, t)| crate::skills::split_front(t).1)
}

mod es {
    // juez-idioma: par (rama ES de `skills()`, con su gemela inglesa en `en`) — bloque
    pub const ENTREVISTA: &str = "---\n\
        name: entrevista\n\
        description: Entrevista al usuario para precisar una tarea antes de hacerla — una pregunta a la vez, con respuesta recomendada, mirando el código antes de preguntar lo que el código ya responde. Se usa con /entrevista.\n\
        ---\n\
        \n\
        # Entrevista\n\
        \n\
        Objetivo: dejar la tarea tan precisa que se pueda hacer sin adivinar.\n\
        \n\
        1. Antes de preguntar NADA, mira el proyecto (glob, grep, read_file, mapa). Lo que el código, la configuración o la memoria del proyecto ya responden NO se pregunta: se dice como hecho, con dónde lo viste («usa Postgres 16: compose.yaml»).\n\
        2. Pregunta UNA cosa a la vez con `ask_user`: la pregunta corta y, en `options`, PRIMERO la respuesta que recomiendas, marcada «(recomendada)» y con su porqué en pocas palabras; después 1 a 3 alternativas reales.\n\
        3. Ordena las preguntas por cuánto cambian el trabajo: primero lo que decide la arquitectura o es irreversible (datos, borrados, dinero, publicar); al final los detalles.\n\
        4. Tras cada respuesta, ajusta: no preguntes lo que ya quedó decidido ni lo que se deduce de ello.\n\
        5. Para cuando lo que falta ya no cambie el resultado, o cuando GOTV no te deje preguntar más en este turno. Entonces responde con el RESUMEN: lo decidido (y quién lo decidió: el usuario, el código o tu recomendación aceptada), lo que queda abierto con tu recomendación, y el siguiente paso. Si aún quedan preguntas, la siguiente va al final de tu respuesta, sola.\n\
        6. Durante la entrevista no cambies archivos: sólo mira. Hacer el trabajo es la orden siguiente.\n";

    pub const APRENDER: &str = "---\n\
        name: aprender\n\
        description: Repasa las correcciones del usuario en esta conversación y propone UNA regla o skill que evite repetir el error; GOTV le enseña la diferencia y sólo la escribe si el usuario aprueba. Se usa con /aprender.\n\
        ---\n\
        \n\
        # Aprender de las correcciones\n\
        \n\
        1. Busca en ESTA conversación los mensajes del usuario que corrigen lo que hiciste («no, así no», «te dije que…», «siempre…», «nunca…», «en vez de…», o un cambio tuyo que él deshizo). Una orden nueva que no corrige nada no cuenta.\n\
        2. Elige la corrección que más se repetiría y escribe la regla MÍNIMA que la evita: una instrucción concreta, en imperativo, con el caso real como ejemplo. Nada que la memoria del proyecto ya diga.\n\
        3. Elige dónde vive:\n\
        - `regla`: vale sólo para ciertos archivos → lleva `rutas` (patrones como `**/*.sql` o `src/api/**`). Se anuncia sólo cuando se tocan: el resto del tiempo no gasta contexto. Prefiérela.\n\
        - `memoria`: vale para todo el proyecto → se añade a `.gotv/GOTV.md` (va en cada conversación: sólo si de verdad vale siempre).\n\
        - `skill`: un procedimiento de varios pasos que se repite → `.gotv/skills/<nombre>/SKILL.md`.\n\
        Nunca `AGENTS.md` ni `CLAUDE.md`: pueden ser de un repositorio que no es del usuario.\n\
        4. NO escribas ningún archivo. Responde SÓLO un objeto JSON:\n\
        {\"tipo\": \"regla\" | \"memoria\" | \"skill\" | \"nada\", \"nombre\": \"nombre-corto\", \"rutas\": [\"**/*.sql\"], \"descripcion\": \"una línea (sólo para skill)\", \"texto\": \"el texto de la regla o de la skill\", \"porque\": \"la corrección que la motiva, citada\"}\n\
        Si no hubo correcciones: {\"tipo\": \"nada\", \"porque\": \"…\"}.\n\
        5. GOTV le enseña al usuario la diferencia exacta y sólo escribe si él la aprueba.\n";
}

mod en {
    // juez-idioma: datos (rama EN de `skills()`; `regla`, `memoria`, `tipo`… son las claves fijas del JSON) — bloque
    pub const ENTREVISTA: &str = "---\n\
        name: entrevista\n\
        description: Interviews the user to pin down a task before doing it — one question at a time, with a recommended answer, looking at the code before asking what the code already answers. Used with /interview.\n\
        ---\n\
        \n\
        # Interview\n\
        \n\
        Goal: make the task precise enough to be done without guessing.\n\
        \n\
        1. Before asking ANYTHING, look at the project (glob, grep, read_file, mapa). What the code, the configuration or the project memory already answers is NOT asked: state it as a fact, with where you saw it (\"it uses Postgres 16: compose.yaml\").\n\
        2. Ask ONE thing at a time with `ask_user`: a short question and, in `options`, FIRST the answer you recommend, marked \"(recommended)\" with its reason in a few words; then 1 to 3 real alternatives.\n\
        3. Order the questions by how much they change the work: first what decides the architecture or is irreversible (data, deletions, money, publishing); details last.\n\
        4. After each answer, adjust: do not ask what is already decided or follows from it.\n\
        5. Stop when what is left no longer changes the result, or when GOTV does not let you ask more this turn. Then answer with the SUMMARY: what was decided (and who decided it: the user, the code or your accepted recommendation), what is still open with your recommendation, and the next step. If questions remain, the next one goes at the end of your answer, alone.\n\
        6. During the interview do not change files: only look. Doing the work is the next order.\n";

    pub const APRENDER: &str = "---\n\
        name: aprender\n\
        description: Reviews the user's corrections in this conversation and proposes ONE rule or skill that avoids repeating the mistake; GOTV shows them the exact diff and only writes it if the user approves. Used with /learn.\n\
        ---\n\
        \n\
        # Learning from corrections\n\
        \n\
        1. Find in THIS conversation the user's messages that correct what you did (\"no, not like that\", \"I told you…\", \"always…\", \"never…\", \"instead of…\", or a change of yours they undid). A new order that corrects nothing does not count.\n\
        2. Pick the correction most likely to repeat and write the MINIMAL rule that avoids it: one concrete instruction, in the imperative, with the real case as the example. Nothing the project memory already says.\n\
        3. Pick where it lives:\n\
        - `regla`: applies only to certain files → it carries `rutas` (patterns like `**/*.sql` or `src/api/**`). It is announced only when they are touched: the rest of the time it costs no context. Prefer it.\n\
        - `memoria`: applies to the whole project → appended to `.gotv/GOTV.md` (it goes in every conversation: only if it truly always applies).\n\
        - `skill`: a multi-step procedure that repeats → `.gotv/skills/<name>/SKILL.md`.\n\
        Never `AGENTS.md` or `CLAUDE.md`: they may belong to a repository that is not the user's.\n\
        4. Do NOT write any file. Answer ONLY a JSON object:\n\
        {\"tipo\": \"regla\" | \"memoria\" | \"skill\" | \"nada\", \"nombre\": \"short-name\", \"rutas\": [\"**/*.sql\"], \"descripcion\": \"one line (skill only)\", \"texto\": \"the text of the rule or the skill\", \"porque\": \"the correction that motivates it, quoted\"}\n\
        If there were no corrections: {\"tipo\": \"nada\", \"porque\": \"…\"}.\n\
        5. GOTV shows the user the exact diff and only writes if they approve it.\n";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn las_dos_skills_existen_en_los_dos_idiomas_con_su_cabecera() {
        for (es, en, nombre) in [(es::ENTREVISTA, en::ENTREVISTA, "entrevista"), (es::APRENDER, en::APRENDER, "aprender")] {
            for t in [es, en] {
                let (meta, cuerpo) = crate::skills::split_front(t);
                assert!(meta.contains(&format!("name: {nombre}")) && meta.contains("description: "), "{meta}");
                assert!(cuerpo.starts_with("# "), "{cuerpo}");
            }
        }
        // la que propone nunca escribe AGENTS.md (trampa 4)
        assert!(es::APRENDER.contains("Nunca `AGENTS.md`") && en::APRENDER.contains("Never `AGENTS.md`"));
        assert!(cuerpo("entrevista").is_some_and(|c| c.contains("ask_user")));
        assert!(cuerpo("otra").is_none());
    }
}
