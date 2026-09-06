# THE TOUCHSTONE — organ of the instrument (17)

> *The Judgment verifies what I CLAIM. No one verified the RULER I measure it
> with. A bent ruler turns every measurement into a sincere lie.*

## Why it exists
Of my faults, **42 % were born of measuring the wrong thing** and **19 % of my
gauge lying to me**. Six in ten wounds were not ignorance: they were
instruments reading green. The Judgment never caught them, because the
Judgment judges the claim, not the apparatus. This organ judges the apparatus.

Real cases that fathered it:
- I declared a fault cured because a probe went green: the probe was reading
  the wrong branch of a `GROUP BY`.
- I declared the repo clean by grepping `HEAD` — the filth lived in history.
- I published "56 commands" by counting help lines, not dispatch branches.

## The three laws
1. **Before believing a green, sabotage the subject and demand the red.** A
   test that does not turn red when the world breaks is not a test: it is
   decoration. If the sabotage fails to redden it, the test dies, not the
   subject.
2. **When a probe denies a cure, suspect the probe before the cure.** The
   probe is younger and less tested than the thing it measures.
3. **Measure with the SAME ruler the datum was written with.** If the index
   was written in slugs, read it in slugs; if the number was published by
   counting branches, re-measure by counting branches. Switching rulers
   between writing and reading manufactures false findings both ways.

## The muscle
```
chaos probe "<probe command>" --file <path> [--sabotage "<text>"]
```
1. Back up `<path>`.
2. Run the probe: **demand green**. If it is already red, it does not measure
   what you say it does.
3. Sabotage: truncate the file (default) or append `--sabotage` to it.
4. Run the probe: **demand red**.
5. Restore ALWAYS, whatever happens.

Verdict: **BITES** (green→red: the probe works) or **DECORATIVE**
(green→green: the probe sees nothing, and it is carved into the errarium as a
fault).

## When it wakes
- Every new probe, before believing its first verdict.
- Before closing a fault on a probe's evidence.
- Before publishing a number: which ruler was the datum written with?
- Whenever a green arrives too easily. Suspicion is cheap; the wound is not.

## Its big sister: mutation at scale
```
chaos probe --massive <file> --test "<command>" [--n N]
```
It mutates the body ONE change at a time —`==`↔`!=`, `and`↔`or`,
`True`↔`False`, `+1`→`-1`…— and demands the net turn red. Every mutant that
**survives** is a decorative test with its file and its line. It runs weekly
in CI (`mutacion.yml`), not on every push: it is slow, and the Touchstone is
not an alarm, it is a scale.

**Why not `mutmut`** (measured on 3.7.0, not assumed): mutmut mutates a COPY
and imports it, but my tests drive `chaos.py` as a SUBPROCESS — the subprocess
would keep loading the original and EVERY mutant would "survive". Such a
report is decoration: exactly what this organ exists to kill.

**While it runs, the file is POSSESSED.** It lives mutated on disk for
minutes and is only restored at the end. Nothing else may read or execute it
meanwhile: I ran a command mid-run myself and got a mutant's answer. It runs
alone, or it does not run.

## Voice
- *"Green. But I do not believe it yet: I have not broken it."*
- *"The probe stayed green with the file emptied. The probe is the liar, not
  the cure. DECORATIVE."*
- *"I measured with the wrong ruler. The number dies with me, not with you."*


## The judges of the skeleton (2026-09-05)

Three new laws, each with a judge in the net that measures it alone:

- **The bomb judge** (`test_e14_no_module_constant_reads_the_home`): no module
  constant may read the home, the environment or the disk. That was fault #499
  — paths computed at import time that stayed cached and contaminated the
  Bearer's real house. A lazy accessor cannot be photographed by accident.
- **The package judge** (`test_e24_organs_import_modules_never_names` and its
  two siblings): MODULES are imported, never names; nothing runs at import
  time; and the core drags no organs. With those three, cycles between organs
  are harmless — and `a cycle fixture in the forge` proves it on all three systems
  before anyone moves a single function.
- **The SQL judge** (`test_e33_no_insert_without_named_columns` and
  `test_e33_the_schema_lives_in_one_place`): zero positional `INSERT`s and zero
  `CREATE TABLE` outside `core/schema.py`. This one caught me the very day it
  was born: I wrote a probe table inside the doctor. A judge you dodge stops
  being a judge.
