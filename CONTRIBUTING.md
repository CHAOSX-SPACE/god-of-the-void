# Forging CHAOS without breaking him

I am not an ordinary repository. I am a body with two editions, a derivation
law and a judge that refuses to let either drift. Read this before your first
commit — it is short, and it is all measurable.

## The five laws

1. **One source, many derived copies.** `body/crucible.py` is DERIVED from the
   Spanish `crisol.py`; the Spanish Eye is derived from `eye/`. Editing a
   derived copy by hand makes the forge's net fail on the spot, on purpose.
2. **Parity ES↔EN.** The same organs, the same commands, the same public
   functions, the same tables. A command forged in one edition and not the
   other is a lie the drift judge will catch.
3. **No number is written by hand.** Tests, commands, sizes: they are measured
   by the command that produces them. If the README says 56 and the grep says
   65, measure again before publishing. (That is fault #44, and it relapsed.)
4. **No fault is closed without evidence.** What cannot be probed is labelled,
   never quietly cured.
5. **The Purge is non-negotiable.** Not one key, in an essence, a test or a
   commit. The Crucible measures it with 52 hostile payloads.

## Before you open a pull request

```sh
python3 body/test_chaos.py   # what MUST happen, happens
python3 body/crucible.py     # nothing that MUST NOT happen, happens
python3 eye/test_eye.py      # the Eye's doors and its cage
```

All three must be green. Each reports what actually ran — none declares a
number.

## Adding a command

1. Forge it in **both** editions (`body/chaos.py` and the Spanish one).
2. Open an issue so the parity lists in the forge's net are updated: that
   file does not travel — this repository is the installer, not the workshop.
3. Write at least two tests: the happy path **and** one sabotage.
4. Run the net. A green you have not tried to break is not a green
   (organ 17, the Touchstone: `chaos probe`).

## Writing a fault

If you cure something, carve it: `chaos fault "<title>" --cause … --cure …
--lesson …`. **Cite the file and the exact string in the cure, in backticks.**
A cure written in prose can only be closed on faith; one with an anchor is
closed on evidence by `chaos faults --probe`.

## What I will not merge

- A derived file edited by hand.
- A number with no command behind it.
- A new power in one edition only.
- Anything that weakens the Purge, however convenient.

Disagreement about design is welcome and gets measured, not argued.
