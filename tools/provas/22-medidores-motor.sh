#!/usr/bin/env bash
# Medidores no motor real (em PipeWire privado): um seno de amplitude 0,5 (−6,02 dBFS) tocado em cada barramento deve aparecer, e só nele, com o
# nível certo; sliders refletem ganho/mute; desligar remove todos os taps; ligar não altera o áudio nem a saída padrão.
set -uo pipefail
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$HERE/../.."
(cd "$ROOT" && cargo build -q -p iara-audio --examples) || exit 1
BIN="$ROOT/target/debug/examples/mixer_cli"
source "$HERE/lib_privado.sh"
TMP=$(mktemp -d); PROC=""
cleanup() { [ -n "$PROC" ] && kill "$PROC" 2>/dev/null; rm -rf "$TMP"; privado_parar; }
trap cleanup EXIT
privado_subir   # grafo PRIVADO: não toca na sessão de áudio de quem estiver usando o computador
FAIL=0; ck() { if [ "$2" = ok ]; then echo "  ok   $1"; else echo "  FALHA $1 ($3)"; FAIL=$((FAIL+1)); fi; }
meters() { pw-cli ls Node | grep -c 'node.name = "iara\.meter\.' || true; }

mkfifo "$TMP/in"; "$BIN" < "$TMP/in" > "$TMP/out" 2> "$TMP/err" & PROC=$!
exec 9> "$TMP/in"; cmd() { echo "$1" >&9; sleep "${2:-1.5}"; }
sleep 6
sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/t.wav" synth 12 sine 440 vol 0.5

echo "# sem interface pedindo medidores: nenhum tap no grafo"
ck "zero taps antes de ligar" "$([ "$(meters)" = 0 ] && echo ok)" "$(meters)"
cmd "meters on" 2
N=$(meters); ck "oito taps depois de ligar" "$([ "$N" = 8 ] && echo ok)" "$N"
cmd "levels" 0.5; : > "$TMP/skip"
lv() { # extrai o nível de uma linha do bloco 'levels' do último comando
  awk -v k="$1" '$0 ~ k {print $(NF-1)}' "$TMP/out" | tail -1
}
mark() { wc -l < "$TMP/out"; }
near() { awk -v a="$1" -v b="$2" -v t="$3" 'BEGIN{ if (a=="-inf") a=-200; if (b=="-inf") b=-200; d=a-b; if (d<0) d=-d; exit !(d<=t) }'; }

play() { # alvo secs
  pw-play --target "$1" "$TMP/t.wav" & PP=$!; sleep "$2"
}
stop() { kill "$PP" 2>/dev/null; wait "$PP" 2>/dev/null; }

echo "# seno 0,5 em iara.ch.game (envio escuta 0 dB, transmissão 0 dB)"
echo "levels" >&9; sleep 0.3   # descarta o que acumulou
play iara.ch.game 2; echo "levels" >&9; sleep 0.6; stop
EV=$(awk '/^eventos=/{sub("eventos=","");print}' "$TMP/out" | tail -1)
ck "cerca de 20 eventos por segundo (≥ 30 em ~2,6 s)" "$([ "${EV:-0}" -ge 30 ] && echo ok)" "$EV"
B=$(lv 'barramento iara.ch.game ')
ck "barramento game ≈ −6,02 dBFS" "$(near "$B" -6.02 0.5 && echo ok)" "$B"
ck "barramento chat em silêncio" "$(near "$(lv 'barramento iara.ch.chat ')" -200 1 && echo ok)" "$(lv 'barramento iara.ch.chat ')"
S=$(lv 'slider Channel { id: "game", transmission: false }')
ck "slider game/escuta ≈ −6,02" "$(near "$S" -6.02 0.5 && echo ok)" "$S"

echo "# escuta −12 dB: só o slider da escuta cai, o barramento e a transmissão não"
cmd "gain game personal -12"
echo "levels" >&9; sleep 0.3
play iara.ch.game 2; echo "levels" >&9; sleep 0.6; stop
S=$(lv 'slider Channel { id: "game", transmission: false }'); T=$(lv 'slider Channel { id: "game", transmission: true }'); B=$(lv 'barramento iara.ch.game ')
ck "slider escuta ≈ −18,04" "$(near "$S" -18.04 0.6 && echo ok)" "$S"
ck "slider transmissão ≈ −6,02" "$(near "$T" -6.02 0.5 && echo ok)" "$T"
ck "barramento intacto ≈ −6,02" "$(near "$B" -6.02 0.5 && echo ok)" "$B"
cmd "gain game personal 0"

echo "# mute da escuta: slider vai a −inf, a transmissão segue"
cmd "mute game personal true"
echo "levels" >&9; sleep 0.3
play iara.ch.game 2; echo "levels" >&9; sleep 0.6; stop
S=$(lv 'slider Channel { id: "game", transmission: false }'); T=$(lv 'slider Channel { id: "game", transmission: true }')
ck "slider mutado em −inf" "$(near "$S" -200 1 && echo ok)" "$S"
ck "transmissão segue ≈ −6,02" "$(near "$T" -6.02 0.5 && echo ok)" "$T"
cmd "mute game personal false"

echo "# MASTER pessoal recebe o canal e o medidor dele acompanha o ganho do MASTER"
cmd "master-gain personal -6"
echo "levels" >&9; sleep 0.3
play iara.ch.game 2; echo "levels" >&9; sleep 0.6; stop
M=$(lv 'barramento iara.master.personal ')
ck "MASTER pessoal ≈ −12,04" "$(near "$M" -12.04 0.7 && echo ok)" "$M"
cmd "master-gain personal 0"

echo "# desligar remove todos os taps"
cmd "meters off" 2
ck "zero taps depois de desligar" "$([ "$(meters)" = 0 ] && echo ok)" "$(meters)"

echo "# religar e encerrar o motor sem deixar nada"
cmd "meters on" 2
echo quit >&9; wait "$PROC" 2>/dev/null; PROC=""; sleep 1
ck "nada de iara.* após encerrar" "$([ "$(pw-cli ls Node | grep -c 'node.name = "iara\.')" = 0 ] && echo ok)" "$(pw-cli ls Node | grep -c 'node.name = "iara\.')"
[ -s "$TMP/err" ] && { echo "stderr do motor:"; cat "$TMP/err"; }
echo "falhas: $FAIL"; exit $FAIL
