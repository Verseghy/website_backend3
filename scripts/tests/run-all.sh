#!/usr/bin/env bash
# run-all.sh: run every shell test in scripts/tests/.
#
# Why this exists: the repository's shell scripts (CI checks, the Loco skill
# sync, the guard hook, the parity-issue sync) each have a test file
# scripts/tests/test-*.sh. The "Test" CI job runs this one entry point, so a
# new test file is picked up without touching the workflow (ADR 0022
# section 3, ADR 0012 section 3, ADR 0024).
#
# Contract for test files: `#!/usr/bin/env bash` with `set -euo pipefail`;
# they run as `bash scripts/tests/<file>` from the repository root, need no
# network, clean up after themselves, and exit non-zero on any failure.
#
# Every file runs, even after a failure, in its own bash process from the
# repository root; the summary lists the failed files.
#
# Usage: bash scripts/tests/run-all.sh
# Exit codes: 0 all test files passed, 1 at least one failed, 2 no test files.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$ROOT"

shopt -s nullglob
tests=(scripts/tests/test-*.sh)
shopt -u nullglob
if ((${#tests[@]} == 0)); then
  echo "error: no scripts/tests/test-*.sh files found under $ROOT" >&2
  exit 2
fi

failed=()
for t in "${tests[@]}"; do
  echo "==> bash $t"
  if bash "$t"; then
    echo "<== passed: $t"
  else
    echo "<== FAILED: $t"
    failed+=("$t")
  fi
  echo
done

if ((${#failed[@]} > 0)); then
  echo "${#failed[@]} of ${#tests[@]} test file(s) failed:"
  printf '  %s\n' "${failed[@]}"
  exit 1
fi
echo "All ${#tests[@]} test file(s) passed."
