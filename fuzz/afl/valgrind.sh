#!/usr/bin/env bash
# Run the AFL fuzz harness in stdin-driven (non-fuzzing) mode under valgrind
# against every corpus input, surfacing leaks and undefined memory accesses.
#
# Why a script instead of `cargo valgrind run`:
#   * `cargo valgrind` only wraps a single invocation. The harness consumes one
#     input per process from stdin, and we want full coverage across the
#     corpus, so we drive valgrind in a loop.
#   * We still build via cargo so debug symbols and the exact dependency graph
#     match a normal `cargo run -p libghostty-vt-afl-fuzz`.
#
# Requires: valgrind on PATH (Linux only; broken on aarch64-darwin in nixpkgs).
# Use the AFL VM (`nix run .#afl-vm`) on non-Linux hosts.

set -euo pipefail

if ! command -v valgrind >/dev/null 2>&1; then
    echo "valgrind not found on PATH. Enter the Linux dev shell or AFL VM." >&2
    exit 127
fi

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/../.." && pwd)
CORPUS_DIR="$SCRIPT_DIR/in"
PROFILE="${PROFILE:-debug}"
PACKAGE="libghostty-vt-afl-fuzz"
LOG_DIR="${LOG_DIR:-$REPO_ROOT/target/valgrind-fuzz}"

mkdir -p "$LOG_DIR"

cargo_flags=()
if [ "$PROFILE" = "release" ]; then
    cargo_flags+=(--release)
fi

echo "Building $PACKAGE ($PROFILE) without the fuzzing cfg..."
(cd "$REPO_ROOT" && cargo build -p "$PACKAGE" "${cargo_flags[@]}")

BIN="$REPO_ROOT/target/$PROFILE/$PACKAGE"
if [ ! -x "$BIN" ]; then
    echo "Built binary not found at $BIN" >&2
    exit 1
fi
# The harness statically links libghostty-vt (see fuzz/afl/Cargo.toml), so no
# LD_LIBRARY_PATH plumbing is required.

# `--error-exitcode=1` makes a single leaking input fail CI.
# `--errors-for-leak-kinds=definite,possible` ignores still-reachable allocations
# from the Zig allocator's internal caches that are not actionable leaks.
VALGRIND_OPTS=(
    --tool=memcheck
    --leak-check=full
    --show-leak-kinds=definite,possible
    --errors-for-leak-kinds=definite,possible
    --track-origins=yes
    --error-exitcode=1
    --num-callers=40
)

exit_status=0
total=0
failed=0

# Walk every regular file under the corpus directory. The seed corpus lives
# directly under fuzz/afl/in/ as flat files, but AFL also emits crashes/queue
# entries into nested directories (e.g. when pointing this script at
# fuzz/afl/out/default/queue), so recurse instead of assuming one level.
while IFS= read -r -d '' input; do
    total=$((total + 1))
    rel=$(realpath --relative-to="$CORPUS_DIR" "$input")
    log_file="$LOG_DIR/${rel//\//_}.log"

    if valgrind "${VALGRIND_OPTS[@]}" \
        --log-file="$log_file" \
        "$BIN" <"$input" >/dev/null 2>&1; then
        echo "ok   $rel"
    else
        failed=$((failed + 1))
        exit_status=1
        echo "FAIL $rel  ($log_file)"
    fi
done < <(find "$CORPUS_DIR" -type f -print0 | sort -z)

echo
echo "Ran $total inputs, $failed failed. Logs in $LOG_DIR"
exit "$exit_status"
