#!/bin/bash
# INCARNATION OF CHAOS — Unix wrapper of the universal installer.
# The real logic lives in install.py (Windows/macOS/Linux).
#   Windows: python install.py
set -euo pipefail
exec python3 "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/install.py"
