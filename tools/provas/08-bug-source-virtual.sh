#!/usr/bin/env bash
# Reprodução do defeito do PipeWire 1.6.9: pw-loopback com media.class=Audio/Source/Virtual encerra com SIGSEGV.
# Serve de sentinela: quando imprimir "OK", o defeito foi corrigido e dá para reavaliar o uso de Audio/Source/Virtual.
set -uo pipefail
echo "# $(pw-cli info 0 2>/dev/null | grep -E '[[:space:]]version:' | head -1 | tr -d '\t')"
check() { # nome classe
  pw-loopback -n "$1" --capture-props="{ node.name=$1_cap media.class=Audio/Sink }" \
    --playback-props="{ node.name=$1 media.class=$2 }" >/dev/null 2>&1 & local L=$!
  sleep 3
  if kill -0 $L 2>/dev/null; then echo "OK    $2"; else wait $L; echo "CRASH $2 (código $?)"; fi
  kill $L 2>/dev/null; wait $L 2>/dev/null
}
check iara_bug_src Audio/Source
check iara_bug_virtual Audio/Source/Virtual
