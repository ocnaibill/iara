#!/usr/bin/env bash
# Regressão da restauração da saída padrão: instala, derruba o serviço (kill -9) e restaura com --restore-default, N vezes.
# O defeito original era intermitente (a mensagem de metadata ainda estava no buffer quando o processo saía).
# ATENÇÃO: troca a saída padrão do sistema a cada rodada (seus apps passam pelo mixer e voltam). O `trap` devolve o original.
set -uo pipefail
export LC_ALL=C
N_RUNS="${1:-6}"
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$HERE/../.."
(cd "$ROOT" && cargo build -q -p iara-service) || exit 1
BIN="$ROOT/target/debug/iara-service"
cfg() { pw-metadata -n default 0 default.configured.audio.sink 2>/dev/null | grep -o 'name":"[^"]*' | head -1 | cut -d'"' -f3; }
ORIG=$(cfg); [ -n "$ORIG" ] || { echo "sem saída padrão configurada"; exit 1; }
TMP=$(mktemp -d); SVC=""
cleanup() { [ -n "$SVC" ] && kill "$SVC" 2>/dev/null; sleep 0.3
  [ "$(cfg)" = "iara.unassigned" ] && pw-metadata -n default 0 default.configured.audio.sink "{\"name\":\"$ORIG\"}" Spa:String:JSON >/dev/null 2>&1
  sleep 0.5; echo "# padrão ao sair: $(cfg) (original: $ORIG)"; rm -rf "$TMP"; }
trap cleanup EXIT
export XDG_CONFIG_HOME="$TMP/config" XDG_STATE_HOME="$TMP/state" IARA_BUS_NAME="dev.iara.MixerRR$$"
ok=0
for i in $(seq 1 "$N_RUNS"); do
  "$BIN" > "$TMP/svc.log" 2>&1 & SVC=$!
  for _ in $(seq 1 80); do [ "$(cfg)" = "iara.unassigned" ] && break; sleep 0.25; done
  inst=$(cfg)
  sleep 0.7; kill -9 "$SVC"; wait "$SVC" 2>/dev/null; SVC=""; sleep 0.4
  "$BIN" --restore-default >/dev/null 2>&1
  sleep 0.6; after=$(cfg)
  if [ "$inst" = "iara.unassigned" ] && [ "$after" = "$ORIG" ]; then ok=$((ok+1)); r=OK; else r="FALHOU (instalou=$inst, depois=$after)"; fi
  echo "rodada $i: $r"
done
echo "resultado: $ok de $N_RUNS"
[ "$ok" = "$N_RUNS" ]
