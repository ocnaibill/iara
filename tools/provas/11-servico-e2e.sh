#!/usr/bin/env bash
# Serviço real de ponta a ponta com diretórios XDG isolados (não toca em ~/.config/iara):
# inicia, cria o perfil padrão, reinicia o PipeWire da sessão (interrompe o áudio por alguns segundos), mede a
# indisponibilidade, confere a reconexão sem duplicar nós e encerra com SIGTERM (nós removidos, perfil gravado).
set -uo pipefail
export LC_ALL=C
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$HERE/../.."
(cd "$ROOT" && cargo build -q -p iara-service) || exit 1
TMP=$(mktemp -d); SVC=""
cleanup() { [ -n "$SVC" ] && kill "$SVC" 2>/dev/null; rm -rf "$TMP"; }
trap cleanup EXIT
export XDG_CONFIG_HOME="$TMP/config" XDG_STATE_HOME="$TMP/state"
count() { pw-cli ls Node 2>/dev/null | grep -c 'node.name = "iara\.' || true; }
now() { date +%s.%N; }
wait_for() { # descrição, comando de teste, timeout em s -> imprime segundos até o sucesso (ou TIMEOUT)
  local t0 t; t0=$(now)
  while true; do
    if eval "$2" >/dev/null 2>&1; then awk -v a="$t0" -v b="$(now)" 'BEGIN{printf "%.1f", b-a}'; return 0; fi
    t=$(awk -v a="$t0" -v b="$(now)" 'BEGIN{print b-a}'); awk -v t="$t" -v m="$3" 'BEGIN{exit !(t>m)}' && { echo TIMEOUT; return 1; }
    sleep 0.1
  done
}

"$ROOT/target/debug/iara-service" > "$TMP/svc.log" 2>&1 & SVC=$!
echo "1. início: $(wait_for 'nós' '[ "$(count)" -ge 42 ]' 15) s até 42 nós; nós=$(count); perfil: $(ls "$TMP/config/iara/profiles" 2>&1 | tr '\n' ' ')"

echo "2. reinício do PipeWire (áudio interrompido por alguns segundos); amostragem a cada 100 ms"
: > "$TMP/timeline"
( while true; do
    echo "$(now) $(count) $(pw-cli ls Node 2>/dev/null | grep -c 'node.name = "alsa_output.pci-0000_0b_00.4')" >> "$TMP/timeline"; sleep 0.1
  done ) & SAMPLER=$!
sleep 1; T0=$(now)
systemctl --user restart pipewire.service
T1=$(now)
sleep 12; kill $SAMPLER 2>/dev/null
python3 - "$TMP/timeline" "$T0" "$T1" <<'PY'
import sys
t0, t1 = float(sys.argv[2]), float(sys.argv[3])
rows = [tuple(map(float, l.split())) for l in open(sys.argv[1]) if len(l.split()) == 3]
def span(pred):
    bad = [r[0] for r in rows if r[0] >= t0 and pred(r)]
    return (bad[0] - t0, bad[-1] - t0) if bad else None
nodes_down = span(lambda r: r[1] < 42)
sink_down = span(lambda r: r[2] < 1)
print(f"   comando restart durou {t1 - t0:.1f} s")
print(f"   saída física ausente de {sink_down[0]:.1f} s a {sink_down[1]:.1f} s após o início do restart" if sink_down else "   saída física nunca amostrada ausente")
print(f"   nós do Iara abaixo de 42 de {nodes_down[0]:.1f} s a {nodes_down[1]:.1f} s (voltaram a 42 em {nodes_down[1] + 0.1:.1f} s)" if nodes_down else "   nós do Iara nunca amostrados abaixo de 42")
print(f"   máximo de nós amostrado: {int(max(r[1] for r in rows if r[0] >= t0))} (sem duplicar se 42); último valor: {int(rows[-1][1])}")
PY
echo "   serviço vivo: $(kill -0 $SVC 2>/dev/null && echo sim || echo não)"

echo "3. SIGTERM"
kill -TERM "$SVC"; wait "$SVC" 2>/dev/null; echo "   código de saída: $?"; SVC=""
sleep 1; echo "   nós após encerrar: $(count)"
echo "--- log do serviço"; sed 's/^/   /' "$TMP/svc.log"
