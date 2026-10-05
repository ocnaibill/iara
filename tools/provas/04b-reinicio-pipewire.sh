#!/usr/bin/env bash
# Prova 6.3.2 passo 6 (parcial): reinício real do PipeWire da sessão com nós hospedados por processo Rust.
# ATENÇÃO: interrompe o áudio de todos os aplicativos por alguns segundos.
set -uo pipefail
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"
(cd "$HERE/../pw-probe" && cargo build -q)
BIN="$HERE/../pw-probe/target/debug/iara-pw-probe"
TMP=$(mktemp -d); PROBE=""
cleanup() { [ -n "$PROBE" ] && kill "$PROBE" 2>/dev/null; rm -rf "$TMP"; }
trap cleanup EXIT
defaults() { wpctl status 2>/dev/null | grep -E '^\s+[│ ]\s+\*' | sed 's/\[vol.*//' | tr -s ' '; }
nodes() { pw-cli ls Node 2>/dev/null | grep -c 'iara_probe_' || true; }

mkfifo "$TMP/in"; "$BIN" < "$TMP/in" > "$TMP/out" 2> "$TMP/err" & PROBE=$!
exec 9> "$TMP/in"; cmd() { echo "$*" >&9; }
cmd "null chan"; cmd "null pers"; cmd "null tx"; cmd "loop lp chan pers"; cmd "loop lt chan tx"
sleep 3
echo "[antes] nós iara_probe=$(nodes) processo_vivo=$(kill -0 $PROBE 2>/dev/null && echo sim || echo não)"
echo "[antes] padrões:"; defaults | sed 's/^/   /'

T0=$(date +%s.%N)
systemctl --user restart pipewire.service
for i in $(seq 1 100); do pw-cli info 0 >/dev/null 2>&1 && break; sleep 0.1; done
T1=$(date +%s.%N)
printf '[reinício] PipeWire respondeu de novo após %.1f s\n' "$(echo "$T1 - $T0" | bc)"
sleep 6
echo "[depois, 6 s] wireplumber=$(systemctl --user is-active wireplumber) nós iara_probe=$(nodes) processo_vivo=$(kill -0 $PROBE 2>/dev/null && echo sim || echo não)"
echo "[depois] erros do processo:"; sed 's/^/   /' "$TMP/err" | head -5
echo "[depois] padrões:"; defaults | sed 's/^/   /'
# o processo antigo tenta ser reutilizado? envia um comando
cmd "list"; sleep 1; echo "[depois] resposta a 'list': $(cat "$TMP/out" | tail -1 | cut -c1-120)"
