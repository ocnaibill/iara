#!/usr/bin/env bash
# Spec 8.4: ao remover um canal (o que a troca de perfil faz com os canais que o perfil novo não tem), os aplicativos que
# estão nele NÃO podem cair na saída física no meio do caminho: o canal velho só sai depois de os fluxos serem redirecionados.
# Amostra os links a cada ~25 ms durante a remoção e procura qualquer ligação do fluxo de teste com a saída física.
set -uo pipefail
export LC_ALL=C
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$HERE/../.."
(cd "$ROOT" && cargo build -q -p iara-audio --examples) || exit 1
BIN="$ROOT/target/debug/examples/mixer_cli"
PHYS=alsa_output.pci-0000_0b_00.4.analog-stereo
TMP=$(mktemp -d); PROC=""; PIDS=()
cleanup() { for p in "${PIDS[@]:-}" ${PROC:+$PROC}; do kill "$p" 2>/dev/null; done
  for id in $(pw-dump | python3 -c "
import json,sys
for o in json.load(sys.stdin):
    p=o.get('info',{}).get('props',{})
    if p.get('application.name','').startswith('IaraTeste'): print(o['id'])" 2>/dev/null); do pw-metadata -n default -d "$id" target.object >/dev/null 2>&1; done
  rm -rf "$TMP"; }
trap cleanup EXIT
mkfifo "$TMP/in"; "$BIN" < "$TMP/in" > "$TMP/out" 2> "$TMP/err" & PROC=$!
exec 9> "$TMP/in"; cmd() { echo "$1" >&9; sleep "${2:-0.5}"; }
sleep 6
sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/t.wav" synth 60 sine 440 vol 0.02
# EXPLICIT=1: o fluxo declara o canal como destino na própria criação (propriedade do stream), o caso em que o WirePlumber
# joga o fluxo na saída física quando o alvo some (prova 04); sem isso, o fluxo foi movido por metadata (caso do Iara).
if [ "${EXPLICIT:-0}" = 1 ]; then T="--target iara.ch.aux"; else T=""; fi
pw-play -P "{ node.name=IaraTesteX application.name=IaraTesteX application.process.binary=iaratestex media.name=x }" $T "$TMP/t.wav" & PIDS+=($!)
sleep 2
[ "${EXPLICIT:-0}" = 1 ] || cmd "route bin:iaratestex iara.ch.aux" 3
sleep 1
echo "antes: $(pw-link -l | grep -A1 'IaraTesteX:output_FL' | tr -d '\n' | sed 's/  */ /g')"
# amostrador: grava cada observação de link do fluxo de teste
( while true; do
    pw-link -l 2>/dev/null | awk '/^IaraTesteX:output_FL/{getline; print $0}' | sed 's/^ *|-> *//' | sed "s/^/$(date +%s.%N) /"
    sleep 0.025
  done ) > "$TMP/samples" & PIDS+=($!)
sleep 0.5
cmd "remove aux" "${GAP:-0.05}"   # o plano sem o canal aux é aplicado; GAP é a janela até o redirecionamento
cmd "route bin:iaratestex iara.ch.game" 4
kill "${PIDS[-1]}" 2>/dev/null
total=$(wc -l < "$TMP/samples")
phys=$(grep -c "$PHYS" "$TMP/samples" || true)
onaux=$(grep -c 'iara.ch.aux' "$TMP/samples" || true)
ongame=$(grep -c 'iara.ch.game' "$TMP/samples" || true)
echo "amostras: $total | ligado ao canal removido: $onaux | ao canal novo: $ongame | à SAÍDA FÍSICA: $phys"
echo "canal aux ainda existe no grafo: $(pw-cli ls Node | grep -c 'node.name = "iara.ch.aux"') (esperado 0 depois de o fluxo sair)"
[ "$phys" = 0 ] && [ "$ongame" -gt 0 ] && echo "OK: nenhum vazamento para a saída física" || echo "FALHOU"
