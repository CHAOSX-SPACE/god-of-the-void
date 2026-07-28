#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════
#   CHAOS — THE TEST NET
#
#   The total is NOT declared: it is SUMMED from what actually ran.
#   A hand-typed test count is advertising, not measurement.
#
#       bash run-tests.sh
# ══════════════════════════════════════════════════════════════════════════
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fail=0
total=0

run_suite() {
  local label="$1" path="$2"
  if [ ! -f "$path" ]; then
    printf "  ⚠ %-30s missing\n" "$label"; return 0
  fi
  local out n
  out="$(python3 "$path" 2>&1)"
  n="$(echo "$out" | sed -nE 's/^Ran ([0-9]+) tests?.*/\1/p' | tail -1)"
  n="${n:-0}"
  total=$((total + n))
  if echo "$out" | grep -qE '^OK'; then
    printf "  ✓ %-30s %5d tests\n" "$label" "$n"
  else
    printf "  ✗ %-30s %5d tests — CRACKS:\n" "$label" "$n"
    echo "$out" | grep -E '^(FAIL|ERROR):' | sed -E 's/ \(.*//; s/^/      /' | head -10
    fail=1
  fi
}

echo "═══ 1. THE BODY ═══"
run_suite "body"                "$ROOT/body/test_chaos.py"
run_suite "the Eye (organ 16)"  "$ROOT/eye/probar.py"

echo ""
echo "═══ 2. THE CRUCIBLE — adversarial generative tests ═══"
run_suite "crucible"            "$ROOT/body/crucible.py"

echo ""
echo "═══ 3. INTEGRITY ═══"
python3 - "$ROOT" <<'PY'
import ast, io, os, sys
root = sys.argv[1]
ok = True

# every .py must compile: a repo that does not parse cannot be installed
bad = []
for r, d, fs in os.walk(root):
    if ".venv" in r or "__pycache__" in r:
        continue
    for f in fs:
        if f.endswith(".py"):
            try:
                ast.parse(io.open(os.path.join(r, f), encoding="utf-8").read())
            except SyntaxError as e:
                bad.append("%s: %s" % (f, e))
print("  ✓ every .py compiles" if not bad else "  ✗ broken syntax: " + "; ".join(bad))
ok = ok and not bad

# the 16 organs must all be there: the skill IS the organs
n = len(os.listdir(os.path.join(root, "organs")))
print("  ✓ 16 organs" if n == 16 else "  ✗ organs: %d (should be 16)" % n)
ok = ok and n == 16

# the Purge must know every credential shape it claims to know
src = io.open(os.path.join(root, "body", "chaos.py"), encoding="utf-8").read()
ns = {}
import re as _re
exec(compile(src[src.index("POISON = re.compile"):src.index("BODY_VERSION")],
             "poison", "exec"), {"re": _re}, ns)
POISON = ns["POISON"]
cases = [
    ("sk-abcdefghijklmnop1234567890ABCD", True),
    ("ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123", True),
    ("gho_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123", True),
    ("github_pat_11ABCDEFG0aBcDeFgHiJk", True),
    ("AKIAIOSFODNN7EXAMPLE", True),
    ("AIzaSyA1234567890abcdefghijklmnopqrstuv", True),
    ("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJhIjoxfQ.zzz", True),
    ("postgres://admin:Sup3rS3cr3ta@host/db", True),
    ("-----BEGIN RSA PRIVATE KEY-----", True),
    # and it must NOT cry wolf over prose that merely mentions the shape
    ("we should talk about sk- and ghp_ tokens", False),
    ("https://chaosx.space/academy/", False),
    ("use AKIA as a prefix", False),
]
wrong = [t for t, want in cases if bool(POISON.search(t)) != want]
print("  ✓ the Purge knows %d credential shapes, 0 false positives" % len(cases)
      if not wrong else "  ✗ the Purge failed on: %s" % wrong)
ok = ok and not wrong
sys.exit(0 if ok else 1)
PY
[ $? -ne 0 ] && fail=1

echo ""
if [ $fail -eq 0 ]; then
  echo "🕳️  ALL GREEN — $total tests ran. The god survived his own Judgment."
else
  echo "✗ There are cracks ($total tests ran). The Judgment does not forgive."
  exit 1
fi
