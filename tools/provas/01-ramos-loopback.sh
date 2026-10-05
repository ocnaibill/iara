#!/usr/bin/env bash
# Prova 6.3.2 (passos 1-3), hipótese A: sink de canal + dois loopbacks do monitor.
# Cria nós virtuais temporários (prefixo iara_proof_), mede ganho por ramo e remove tudo ao sair.
# Requer: pw-cli, pw-loopback, pw-play, pw-record, sox, PipeWire em execução.
set -euo pipefail

TMP=$(mktemp -d)
PIDS=()
cleanup() { for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null || true; done; rm -rf "$TMP"; }
trap cleanup EXIT

echo "# versões"; pw-cli info 0 | grep -E '[[:space:]](version|name):' | head -3; echo
# nó nulo capturável por monitor; vive enquanto o processo pw-cli viver
mkfifo "$TMP/cli"; pw-cli < "$TMP/cli" > "$TMP/cli.out" 2>&1 & PIDS+=($!)
exec 9> "$TMP/cli"
mknull() { echo "create-node adapter { factory.name=support.null-audio-sink node.name=$1 media.class=Audio/Sink audio.position=[FL FR] priority.session=0 priority.driver=0 }" >&9; }
mknull iara_proof_chan; mknull iara_proof_pers; mknull iara_proof_tx

loop() { # $1 nome, $2 destino
  pw-loopback -n "$1" \
    --capture-props="{ node.name=$1_in target.object=iara_proof_chan stream.capture.sink=true node.passive=true node.dont-fallback=true }" \
    --playback-props="{ node.name=$1_out target.object=$2 node.dont-fallback=true }" &
  PIDS+=($!)
}
loop iara_proof_lp iara_proof_pers; loop iara_proof_lt iara_proof_tx
sleep 2

nid() { pw-cli ls Node | awk -v n="$1" '/^\tid /{id=$2} $0 ~ "node.name = \""n"\"" {gsub(",","",id); print id}' | head -1; }
setvol() { pw-cli s "$(nid "$1")" Props "{ channelVolumes: [ $2, $2 ] }" >/dev/null; }
sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/tone.wav" synth 8 sine 440 vol 0.5

measure() { # imprime "pers tx" (amplitude RMS) com gravações simultâneas
  pw-record -P "{ stream.capture.sink=true }" --target iara_proof_pers --rate 48000 --channels 2 "$TMP/p.wav" & local r1=$!
  pw-record -P "{ stream.capture.sink=true }" --target iara_proof_tx   --rate 48000 --channels 2 "$TMP/t.wav" & local r2=$!
  sleep 1; pw-play --target iara_proof_chan "$TMP/tone.wav" & local pp=$!
  sleep 4; kill $pp 2>/dev/null || true; sleep 0.3
  kill -INT $r1 $r2 2>/dev/null || true; wait $r1 $r2 2>/dev/null || true
  local a b
  a=$(sox "$TMP/p.wav" -n trim 1 2 stat 2>&1 | awk '/RMS +amplitude/{print $3}')
  b=$(sox "$TMP/t.wav" -n trim 1 2 stat 2>&1 | awk '/RMS +amplitude/{print $3}')
  echo "$a $b"
}
db() { awk -v x="$1" 'BEGIN{ if (x<=0) print "-inf"; else printf "%.2f", 20*log(x)/log(10) }'; }
report() { read -r a b <<<"$(measure)"; printf '%-34s pers=%s dB  tx=%s dB\n' "$1" "$(db "$a")" "$(db "$b")"; }

echo "# nós criados"; pw-cli ls Node | grep -c 'iara_proof_' || true
report "base (ganhos 1.0)"
setvol iara_proof_lp_out 0.501; report "pers -6 dB"
setvol iara_proof_lp_out 1.0; setvol iara_proof_lt_out 0.501; report "tx -6 dB (invertido)"
setvol iara_proof_lt_out 0.0; report "tx mudo (ganho 0)"
setvol iara_proof_lt_out 1.0
