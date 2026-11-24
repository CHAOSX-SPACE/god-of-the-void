//! F9 · el coordinador como skill INCLUIDA (`enjambre`): cómo repartir una tarea grande entre varios
//! subagentes para que no choquen, con el tablero y el verificador. Vive en el binario (constantes de Rust
//! en los dos idiomas, como las de F12), viaja con las demás incluidas (`skills::builtin_dir`) y se sugiere
//! sola cuando la orden habla de repartir el trabajo («en paralelo», «varios agentes», «equipo»…).

/// `(nombre, SKILL.md)` en el idioma activo.
pub fn skills() -> [(&'static str, &'static str); 1] {
    [("enjambre", crate::i18n::t(es::ENJAMBRE, en::ENJAMBRE))]
}

mod es {
    // juez-idioma: par (rama ES de `skills()`, con su gemela inglesa en `en`) — bloque
    pub const ENJAMBRE: &str = "---\n\
        name: enjambre\n\
        description: Reparte una tarea grande entre varios subagentes que trabajan a la vez (en paralelo, varios agentes, un equipo), cada uno en su rama, con un tablero común y un verificador que corre las pruebas. Para trabajos que se parten en piezas que no tocan los mismos archivos.\n\
        ---\n\
        \n\
        # Enjambre: varios subagentes a la vez\n\
        \n\
        1. Mira el proyecto lo justo para partir el trabajo en piezas que NO toquen los mismos archivos: un subagente por archivo, por módulo o por paquete. Si dos piezas necesitan el mismo archivo, júntalas en una sola o hazlas tú antes.\n\
        2. Lo común que todos necesitan (una función compartida, un tipo, una dependencia) hazlo TÚ primero y anótalo en el tablero: `enjambre` con accion «anotar» («ya existe validar() en util.js; úsenla»).\n\
        3. Lanza las piezas EN EL MISMO TURNO: varias llamadas a `task`, una por pieza, cada una con todo lo que necesita saber (qué archivos SÍ puede tocar, cuáles NO, cómo comprobar su parte). GOTV las corre a la vez —en la nube en paralelo; con el modelo local por turnos o por lotes—, cada una en su propia rama, y al terminar trae al árbol lo que no choca y te dice qué chocó.\n\
        4. Cada subagente puede leer y escribir el tablero (`enjambre` «tablero» / «anotar»): lo que descubre y otro necesita va ahí, corto.\n\
        5. Si algo chocó, resuélvelo tú con la rama que GOTV dejó (`git diff HEAD...<rama>`), o vuelve a lanzar esa pieza.\n\
        6. Al final, el VERIFICADOR: `enjambre` con accion «verificar» y el criterio (qué pruebas correr y qué debe cumplirse). Corre en otra conversación —y en otro modelo si lo pides con «modelo»— y no edita: sólo comprueba y dice qué falla. Si falla, arregla y verifica otra vez.\n\
        7. Responde con lo que hizo cada pieza, lo que se fusionó, lo que chocó y el veredicto del verificador.\n";
}

mod en {
    // juez-idioma: datos (rama EN de `skills()`; `enjambre`, `task`, «anotar», «tablero», «verificar», «modelo» son nombres fijos de herramientas y acciones) — bloque
    pub const ENJAMBRE: &str = "---\n\
        name: enjambre\n\
        description: Splits a big task among several subagents working at the same time (in parallel, several agents, a team), each on its own branch, with a shared board and a verifier that runs the tests. For work that splits into pieces that do not touch the same files.\n\
        ---\n\
        \n\
        # Swarm: several subagents at once\n\
        \n\
        1. Look at the project just enough to split the work into pieces that do NOT touch the same files: one subagent per file, per module or per package. If two pieces need the same file, merge them into one or do it yourself first.\n\
        2. What everyone needs (a shared function, a type, a dependency) do it YOURSELF first and note it on the board: `enjambre` with accion «anotar» («validar() already exists in util.js; use it»).\n\
        3. Launch the pieces IN THE SAME TURN: several `task` calls, one per piece, each with everything it needs to know (which files it MAY touch, which NOT, how to check its part). GOTV runs them at once —in parallel on the cloud; in turns or batches with the local model—, each on its own branch, and when they finish it brings into the tree what does not conflict and tells you what conflicted.\n\
        4. Each subagent can read and write the board (`enjambre` «tablero» / «anotar»): what it finds and another needs goes there, short.\n\
        5. If something conflicted, resolve it yourself with the branch GOTV left (`git diff HEAD...<branch>`), or launch that piece again.\n\
        6. At the end, the VERIFIER: `enjambre` with accion «verificar» and the criterion (which tests to run and what must hold). It runs in another conversation —and on another model if you ask with «modelo»— and does not edit: it only checks and says what fails. If it fails, fix and verify again.\n\
        7. Answer with what each piece did, what was merged, what conflicted and the verifier's verdict.\n";
}

#[cfg(test)]
mod tests {
    #[test]
    fn la_skill_se_reconoce_con_su_cabecera() {
        for (nombre, texto) in super::skills() {
            let (cabecera, cuerpo) = crate::skills::split_front(texto);
            assert!(cabecera.contains(&format!("name: {nombre}")), "{cabecera}");
            assert!(cuerpo.contains("task") && cuerpo.contains("verificar"), "{cuerpo}");
        }
    }
}
