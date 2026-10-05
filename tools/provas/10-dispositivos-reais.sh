#!/usr/bin/env bash
# Verificação rápida com os dispositivos reais do desenvolvedor (fone na saída analógica da placa-mãe, microfone USB):
# só confere as ligações no grafo e toca 2 s de um tom bem baixo no fone. Não desliga nenhum perfil de placa.
set -uo pipefail
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$HERE/../.."
OUT="${1:-alsa_output.pci-0000_0b_00.4.analog-stereo}"
IN="${2:-alsa_input.usb-MV-SILICON_fifine_AM8_Pro_20190808-00.mono-fallback}"
(cd "$ROOT" && cargo build -q -p iara-audio --examples) || exit 1
TMP=$(mktemp -d); PROC=""
cleanup() { [ -n "$PROC" ] && kill "$PROC" 2>/dev/null; rm -rf "$TMP"; }
trap cleanup EXIT
mkfifo "$TMP/in"; "$ROOT/target/debug/examples/mixer_cli" < "$TMP/in" > "$TMP/out" 2>&1 & PROC=$!
exec 9> "$TMP/in"; sleep 6
echo "output $OUT" >&9; sleep 4; echo "mic-device $IN" >&9; sleep 4
tail -1 "$TMP/out"
python3 "$HERE/lib_links.py" iara.dev.output.out iara.dev.input.in "$OUT" "$IN"
sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/t.wav" synth 2 sine 440 vol 0.02
pw-play --target iara.ch.game "$TMP/t.wav"; sleep 0.5
echo quit >&9; wait "$PROC" 2>/dev/null; PROC=""; sleep 1
echo "nós iara.* após encerrar: $(pw-cli ls Node | grep -c 'node.name = "iara\.')"
