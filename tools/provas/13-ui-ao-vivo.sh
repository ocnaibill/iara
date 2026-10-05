#!/usr/bin/env bash
# Janela GTK + serviço real + D-Bus real + PipeWire real: captura a janela depois de uma mudança feita por FORA (busctl)
# para provar que a interface acompanha o serviço. Diretórios XDG isolados e nome de barramento de teste.
# A janela do Iara aparece por alguns segundos na tela do desenvolvedor. Saída: PNGs em $1 (padrão: ./capturas).
set -uo pipefail
export LC_ALL=C
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$HERE/../.."
OUT="${1:-$ROOT/capturas}"; mkdir -p "$OUT"
(cd "$ROOT" && cargo build -q -p iara-service -p iara-ui --features gtk-ui) || exit 1
TMP=$(mktemp -d); SVC=""; UI=""
cleanup() { [ -n "$UI" ] && kill "$UI" 2>/dev/null; [ -n "$SVC" ] && kill "$SVC" 2>/dev/null; rm -rf "$TMP"; }
trap cleanup EXIT
export XDG_CONFIG_HOME="$TMP/config" XDG_STATE_HOME="$TMP/state"
N="dev.iara.MixerUI$$"; export IARA_BUS_NAME="$N"; O=/dev/iara/Mixer; I=dev.iara.Mixer1
bc() { busctl --user call -- "$N" "$O" "$I" "$@" >/dev/null; }

# 1) sem serviço: a janela tem de dizer isso em texto
IARA_UI_SHOT_DELAY_MS=1500 "$ROOT/target/debug/iara-ui" --screenshot "$OUT/ui-sem-servico.png" 2>&1 | grep -E "captura|falha"

# 2) com serviço; muda coisas por fora enquanto a janela está aberta
"$ROOT/target/debug/iara-service" > "$TMP/svc.log" 2>&1 & SVC=$!
for i in $(seq 1 100); do busctl --user status "$N" >/dev/null 2>&1 && break; sleep 0.1; done
IARA_UI_SHOT_DELAY_MS=5000 "$ROOT/target/debug/iara-ui" --screenshot "$OUT/ui-ao-vivo.png" 2>&1 | grep -E "captura|falha" & UI=$!
sleep 1.8
bc SetChannelGain ssd game personal -9
bc SetChannelGain ssd chat transmission -12
bc SetChannelMute ssb media transmission true
bc SetChannelEnabled ssb aux transmission true
bc SetMasterGain sd personal -6
bc SetMicInputGain d -18
bc SetChatMixPosition d 0.35
bc AddChannel ss musica "Música"
bc SetMicGlobalMute b true
wait $UI; UI=""
echo "log do serviço:"; sed 's/^/   /' "$TMP/svc.log"
