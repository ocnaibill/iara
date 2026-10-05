#!/usr/bin/env bash
# Prova 6.3.2 passo 6 (parcial): nó criado no daemon com object.linger sobrevive à queda do cliente?
set -uo pipefail
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"
(cd "$HERE/../pw-probe" && cargo build -q)
BIN="$HERE/../pw-probe/target/debug/iara-pw-probe"
TMP=$(mktemp -d); PROBE=""
cleanup() { [ -n "$PROBE" ] && kill "$PROBE" 2>/dev/null; rm -rf "$TMP"; }
trap cleanup EXIT
mkfifo "$TMP/in"; "$BIN" < "$TMP/in" > "$TMP/out" 2> "$TMP/err" & PROBE=$!
exec 9> "$TMP/in"
echo "null sem_linger" >&9; echo "null com_linger linger" >&9; sleep 2
echo "[vivo]   $(pw-cli ls Node | grep -o 'iara_probe_[a-z_]*' | sort -u | tr '\n' ' ')"
kill -9 "$PROBE"; wait "$PROBE" 2>/dev/null; PROBE=""; sleep 2
echo "[SIGKILL] $(pw-cli ls Node | grep -o 'iara_probe_[a-z_]*' | sort -u | tr '\n' ' ')"
