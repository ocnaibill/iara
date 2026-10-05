#!/usr/bin/env bash
# Prova 6.3.2 (passos 2-4), hipótese A estendida: dois canais, soma por mix, MASTER depois da soma,
# grupo Não atribuídos (só escuta) e MIC com ramo dedicado + envios. Nós temporários (iara_proof_*).
# O "microfone físico" é simulado por um sink nulo para o resultado ser determinístico.
set -euo pipefail

TMP=$(mktemp -d); PIDS=()
cleanup() { for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null || true; done; rm -rf "$TMP"; }
trap cleanup EXIT

mkfifo "$TMP/cli"; pw-cli < "$TMP/cli" > "$TMP/cli.out" 2>&1 & PIDS+=($!)
exec 9> "$TMP/cli"
mknull() { echo "create-node adapter { factory.name=support.null-audio-sink node.name=iara_proof_$1 media.class=Audio/Sink audio.position=[FL FR] priority.session=0 priority.driver=0 }" >&9; }
# loop NOME ORIGEM DESTINO: captura o monitor de ORIGEM e entrega em DESTINO (soma natural quando vários entram no mesmo sink)
loop() {
  pw-loopback -n "iara_proof_$1" \
    --capture-props="{ node.name=iara_proof_$1_in target.object=iara_proof_$2 stream.capture.sink=true node.passive=true node.dont-fallback=true }" \
    --playback-props="{ node.name=iara_proof_$1_out target.object=iara_proof_$3 node.dont-fallback=true }" &
  PIDS+=($!)
}

for n in gameA gameB unassigned mixP mixT masterP masterT mic micCommon micApps; do mknull $n; done
# canais -> envios
loop A_p gameA mixP;  loop A_t gameA mixT
loop B_p gameB mixP;  loop B_t gameB mixT
loop U_p unassigned mixP                      # Não atribuídos: só escuta
# MASTER depois da soma
loop M_p mixP masterP; loop M_t mixT masterT
# MIC: ganho comum/mute global -> ramos
loop mic_in mic micCommon
loop mic_apps micCommon micApps; loop mic_p micCommon mixP; loop mic_t micCommon mixT
sleep 3

nid() { pw-cli ls Node | awk -v n="iara_proof_$1" '/^\tid /{id=$2} $0 ~ "node.name = \""n"\"" {gsub(",","",id); print id}' | head -1; }
vol()  { pw-cli s "$(nid "$1_out")" Props "{ channelVolumes: [ $2, $2 ] }" >/dev/null; }
mute() { pw-cli s "$(nid "$1_out")" Props "{ mute: $2 }" >/dev/null; }
lin()  { awk -v d="$1" 'BEGIN{printf "%.4f", 10^(d/20)}'; }
sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/f440.wav" synth 8 sine 440 vol 0.5
sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/f880.wav" synth 8 sine 880 vol 0.5

db() { awk -v x="$1" 'BEGIN{ if (x<=1e-7) print "-inf"; else printf "%.2f", 20*log(x)/log(10) }'; }
# run "título" "freq:sink freq:sink" alvo1 alvo2 ...
run() {
  local title="$1" plays="$2"; shift 2
  local rec=() pl=() t
  for t in "$@"; do
    pw-record -P "{ stream.capture.sink=true }" --target "iara_proof_$t" --rate 48000 --channels 2 "$TMP/r_$t.wav" & rec+=($!)
  done
  sleep 1
  for t in $plays; do pw-play --target "iara_proof_${t#*:}" "$TMP/f${t%%:*}.wav" & pl+=($!); done
  sleep 4; kill "${pl[@]}" 2>/dev/null || true; sleep 0.3
  kill -INT "${rec[@]}" 2>/dev/null || true; wait "${rec[@]}" 2>/dev/null || true
  local out="" v
  for t in "$@"; do
    v=$(sox "$TMP/r_$t.wav" -n trim 1 2 stat 2>&1 | awk '/RMS +amplitude/{print $3}')
    out+=" $t=$(db "$v")"
  done
  printf '%-44s%s\n' "$title" "$out"
}
T="masterP masterT micApps"

echo "# dBFS RMS (seno 440/880 Hz, amplitude 0,5; -inf = silêncio)"
run "A sozinho (uma cópia)"              "440:gameA"           masterP masterT
run "A + B (soma, tons distintos)"       "440:gameA 880:gameB" masterP masterT
vol A_p "$(lin -6)";   run "A pessoal -6 dB"          "440:gameA"           masterP masterT; vol A_p 1.0
mute A_t true;         run "A transmissão mute (flag)" "440:gameA"           masterP masterT; mute A_t false
vol M_p "$(lin -6)";   run "MASTER pessoal -6 dB"     "440:gameA 880:gameB" masterP masterT; vol M_p 1.0
vol M_t "$(lin -6)";   run "MASTER transmissão -6 dB" "440:gameA 880:gameB" masterP masterT; vol M_t 1.0
run "Não atribuídos sozinho"             "440:unassigned"      masterP masterT
run "MIC sozinho (3 destinos)"           "440:mic"             masterP masterT micApps
mute mic_t true;       run "MIC envio transmissão mute"       "440:mic" masterP masterT micApps; mute mic_t false
mute mic_p true;       run "MIC envio pessoal mute"           "440:mic" masterP masterT micApps; mute mic_p false
mute mic_in true;      run "MIC mute global"                  "440:mic" masterP masterT micApps; mute mic_in false
vol M_p "$(lin -20)";  run "MASTER pess -20: MIC apps intocado" "440:mic" masterP masterT micApps; vol M_p 1.0
