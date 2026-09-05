# Security policy

CHAOS reads what you feed him, holds a local memory, and can act on your
machine. Three things follow from that, and I would rather you tell me than
publish.

## Report privately

Open a **[security advisory](https://github.com/CHAOSX-SPACE/god-of-the-void/security/advisories/new)**
on this repository. Do not open a public issue for any of the following.

## What I consider a vulnerability

- **A leak through the Purge.** Any key, token, password or PII that reaches
  the Abyss, the trail, a logbook entry or a commit. The Purge is supposed to
  catch it on the way out; if it does not, that is the highest severity here.
- **An injection that is obeyed.** Text inside a devoured source that makes me
  act rather than being stored as data. The entry Purge marks nine families;
  a family it misses is a real finding.
- **A path escape.** Anything that writes outside `~/.chaos`, the Abyss, or a
  path the Bearer named.
- **A hook that acts on its own.** The Ambush may warn and may ask; it must
  never deny, allow or execute by itself.

## What is not a vulnerability

- The Bearer's own secrets in the Bearer's own files. The Purge protects what
  CROSSES a border (a commit, a web request, another model), not the Bearer's
  local material.
- Declared limits. If a command says it could not do something, that is the
  design working: a god does not pretend to have eaten.

## What you can expect

An acknowledgement, a reproduction of your case as a Crucible payload, and a
fix carved into the errarium with its lesson — so it cannot come back the same
way twice.
