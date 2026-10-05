#!/usr/bin/env bash
# Serviço real + D-Bus real + PipeWire real, com um cliente independente (busctl): prova o contrato de fora do nosso código.
# Diretórios XDG isolados e nome de barramento de teste; cria nós temporários `iara.*` e toca um tom (amplitude 0,5) neles.
set -uo pipefail
export LC_ALL=C
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$HERE/../.."
(cd "$ROOT" && cargo build -q -p iara-service) || exit 1
TMP=$(mktemp -d); SVC=""
cleanup() { [ -n "$SVC" ] && kill "$SVC" 2>/dev/null; rm -rf "$TMP"; }
trap cleanup EXIT
export XDG_CONFIG_HOME="$TMP/config" XDG_STATE_HOME="$TMP/state"
N="dev.iara.MixerE2E$$"; export IARA_BUS_NAME="$N"; O=/dev/iara/Mixer; I=dev.iara.Mixer1
# `--` evita que o busctl leia -6 e -inf como opções
bc() { busctl --user call -- "$N" "$O" "$I" "$@"; }
count() { pw-cli ls Node 2>/dev/null | grep -c 'node.name = "iara\.' || true; }
sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/t.wav" synth 8 sine 440 vol 0.5
db() { awk -v x="$1" 'BEGIN{ if (x<=1e-7) print "-inf"; else printf "%.2f", 20*log(x)/log(10) }'; }
level() { # origem alvo(s) -> dBFS RMS do primeiro alvo
  pw-record -P "{ stream.capture.sink=true }" --target "$2" --rate 48000 --channels 2 "$TMP/r.wav" & local r=$!
  sleep 1; pw-play --target "$1" "$TMP/t.wav" & local p=$!; sleep 3.5; kill $p 2>/dev/null; sleep 0.3
  kill -INT $r 2>/dev/null; wait $r 2>/dev/null
  db "$(sox "$TMP/r.wav" -n trim 1 2 stat 2>&1 | awk '/RMS +amplitude/{print $3}')"; }

"$ROOT/target/debug/iara-service" > "$TMP/svc.log" 2>&1 & SVC=$!
for i in $(seq 1 100); do busctl --user status "$N" >/dev/null 2>&1 && break; sleep 0.1; done
sleep 1
echo "1. interface publicada (busctl introspect): $(busctl --user introspect "$N" "$O" "$I" 2>&1 | grep -c ' method ') métodos, $(busctl --user introspect "$N" "$O" "$I" 2>&1 | grep -c ' signal ') sinal"
echo "2. GetState: $(bc GetState | cut -c1-60)…  (nós iara.* no grafo: $(count))"
echo "   GAME → MASTER pessoal, base: $(level iara.ch.game iara.master.personal) dBFS"
echo "3. SetChannelGain game personal -6 → $(bc SetChannelGain ssd game personal -6)"
sleep 0.5; echo "   GAME → MASTER pessoal, depois: $(level iara.ch.game iara.master.personal) dBFS (esperado ≈ 6 dB abaixo)"
echo "4. SetChannelGain com -inf (silêncio) → $(bc SetChannelGain ssd game personal -inf)"
sleep 0.5; echo "   GAME → MASTER pessoal: $(level iara.ch.game iara.master.personal) dBFS (esperado -inf)"
echo "5. comandos inválidos:"
bc SetChannelGain ssd game ouvido 0 2>&1 | sed 's/^/   /'
bc SetChatMixPosition d 7.5 2>&1 | sed 's/^/   /'
bc AddChannel ss '../x' 'X' 2>&1 | sed 's/^/   /'
echo "6. canal novo: $(bc AddChannel ss musica Musica) → nós iara.* após 1 s: $(sleep 1; count) (esperado 42 + 1 sink + 2 streams do ramo pessoal = 45)"
echo "7. segunda instância com o mesmo nome:"
"$ROOT/target/debug/iara-service" 2>&1 | head -2 | sed 's/^/   /'; echo "   código: ${PIPESTATUS[0]}"
echo "8. SIGTERM com autosave pendente"
bc SetMicGlobalMute b true >/dev/null
kill -TERM "$SVC"; wait "$SVC" 2>/dev/null; echo "   saída: $?"; SVC=""
sleep 1
echo "   nós iara.* após encerrar: $(count)"
echo "   perfil em disco: mute global = $(grep -A1 '^\[microphone\]' "$TMP/config/iara/profiles/default.toml" | grep global_mute); canal musica presente: $(grep -c 'id = "musica"' "$TMP/config/iara/profiles/default.toml"); ganho game: $(grep -A8 'id = "game"' "$TMP/config/iara/profiles/default.toml" | grep -c 'silence = true')"
echo "   revisões no histórico: $(ls "$TMP/state/iara/history/default" 2>/dev/null | wc -l)"
