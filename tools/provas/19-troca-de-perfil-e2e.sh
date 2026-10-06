#!/usr/bin/env bash
# Troca de perfil de ponta a ponta (spec 8.4): serviço + D-Bus + PipeWire reais, dois perfis com canais diferentes, fluxos de teste
# em canais que só existem num dos perfis. Amostra os links a cada ~25 ms durante as trocas e exige: nenhuma ligação com a saída
# física (vazamento), o perfil anterior gravado, a ativação persistida e os erros sem efeito. Captura da saída padrão desligada
# (não toca no padrão do sistema). Só os fluxos de teste IaraTeste* são movidos.
set -uo pipefail
export LC_ALL=C
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$HERE/../.."
(cd "$ROOT" && cargo build -q -p iara-service) || exit 1
PHYS=alsa_output.pci-0000_0b_00.4.analog-stereo
TMP=$(mktemp -d); SVC=""; PIDS=()
cleanup() { for p in "${PIDS[@]:-}" ${SVC:+$SVC}; do kill "$p" 2>/dev/null; done
  for id in $(pw-dump | python3 -c "
import json,sys
for o in json.load(sys.stdin):
    p=o.get('info',{}).get('props',{})
    if p.get('application.name','').startswith('IaraTeste'): print(o['id'])" 2>/dev/null); do pw-metadata -n default -d "$id" target.object >/dev/null 2>&1; done
  rm -rf "$TMP"; }
trap cleanup EXIT
export XDG_CONFIG_HOME="$TMP/config" XDG_STATE_HOME="$TMP/state"
mkdir -p "$XDG_CONFIG_HOME/iara"
printf 'schema_version = 1\nautostart = true\nshare_output_device = false\nshare_microphone_device = false\ncapture_default_output = false\n' > "$XDG_CONFIG_HOME/iara/config.toml"
N="dev.iara.MixerPF$$"; export IARA_BUS_NAME="$N"; O=/dev/iara/Mixer; I=dev.iara.Mixer1
bc() { busctl --user call -- "$N" "$O" "$I" "$@" 2>&1; }
profiles() { busctl --user --json=short call -- "$N" "$O" "$I" GetState 2>/dev/null | python3 -c "
import json,re,sys
d=json.load(sys.stdin)['data']
m=re.search(r'^id = \"([^\"]*)\"', d[1], re.M)
print('ativo=%s lista=%s' % (m.group(1), [(p[0], p[1]) for p in d[8]]))"; }
nodes_iara() { pw-cli ls Node | grep -o 'node.name = "iara\.ch\.[a-z0-9-]*"' | cut -d'"' -f2 | sort | tr '\n' ' '; }

"$ROOT/target/debug/iara-service" > "$TMP/svc.log" 2>&1 & SVC=$!
for i in $(seq 1 100); do busctl --user status "$N" >/dev/null 2>&1 && break; sleep 0.1; done
sleep 1.5
sox -n -r 48000 -c 2 -b 32 -e floating-point "$TMP/t.wav" synth 180 sine 440 vol 0.02
play() { pw-play -P "{ node.name=IaraTeste$1 application.name=IaraTeste$1 application.process.binary=iarateste$1 media.name=f$1 }" "$TMP/t.wav" & PIDS+=($!); }
play a; play b; sleep 2

echo "1. perfil 'padrão': A → aux, B → game (regras salvas)"
bc AssignApp ssss "" iaratestea "" aux >/dev/null; bc AssignApp ssss "" iaratesteb "" game >/dev/null
sleep 3
echo "   canais no grafo: $(nodes_iara)"; profiles

echo "2. 'Copia' do perfil; nela o canal aux é removido e A vai para game"
bc DuplicateProfile ss default "Copia" | sed 's/^/   duplicar: /'
bc SwitchProfile s copia | sed 's/^/   trocar: /'; sleep 2
bc RemoveChannel ss aux game >/dev/null; sleep 4
echo "   canais agora: $(nodes_iara)"; profiles
bc SwitchProfile s default >/dev/null; sleep 4
echo "   de volta ao padrão: canais $(nodes_iara)"

echo "3. amostrando os links de A e B durante 4 trocas seguidas (default ⇄ copia)"
( while true; do
    pw-link -l 2>/dev/null | awk '/^IaraTeste[ab]:output_FL/{n=$1; getline; print n, $0}' | sed "s/^/$(date +%s.%N) /"
    sleep 0.025
  done ) > "$TMP/samples" & SAMPLER=$!; PIDS+=($SAMPLER)
sleep 0.5
mark() { echo "$(date +%s.%N) MARCA troca-para-$1" >> "$TMP/marks"; }
for i in 1 2; do mark copia; bc SwitchProfile s copia >/dev/null; sleep 3; mark default; bc SwitchProfile s default >/dev/null; sleep 3; done
kill $SAMPLER 2>/dev/null
total=$(wc -l < "$TMP/samples"); phys=$(grep -c "$PHYS" "$TMP/samples" || true)
echo "   amostras: $total | em canais do Iara: $(grep -c 'iara.ch' "$TMP/samples" || true) | NA SAÍDA FÍSICA: $phys"
if [ "$phys" = 0 ] && [ "$total" -gt 100 ]; then
  echo "   OK: nenhum fluxo caiu na saída física durante as trocas"
else
  echo "   FALHOU"
  python3 - "$TMP/samples" "$TMP/marks" "$PHYS" <<'PY'
import sys
marks = [(float(l.split()[0]), l.split()[2]) for l in open(sys.argv[2]) if "MARCA" in l]
leaks = [(float(l.split()[0]), l.split()[1]) for l in open(sys.argv[1]) if sys.argv[3] in l]
for t, who in leaks:
    m = min(marks, key=lambda m: abs(m[0] - t))
    print("   vazamento de %s a %+.3f s da marca %s" % (who, t - m[0], m[1]))
PY
fi
echo "   canais ao fim (ativo = default): $(nodes_iara)"

echo "4. erros não mudam nada"
bc SwitchProfile s nao-existe | sed 's|^|   trocar para inexistente: |'
bc DeleteProfile s default | sed 's/^/   excluir o ativo: /'
profiles
echo "5. excluir 'copia' (inativo): vai para a lixeira"
bc DeleteProfile s copia | sed 's/^/   /'; sleep 1; profiles
echo "   lixeira: $(ls "$TMP/state/iara/trash" 2>/dev/null | tr '\n' ' ')"
echo "6. perfil ativo persistido: $(grep active_profile "$TMP/config/iara/config.toml")"
