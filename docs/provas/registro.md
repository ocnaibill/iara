# Registro técnico das provas (seção 6.3.2)

Resultados medidos, com comando e versões. Só vale o que está medido aqui.

## Ambiente

- 2026-10-05 · PipeWire 1.6.9 (daemon da sessão do usuário, acessado de um toolbox Arch) · GTK 4.22.5.
- Ferramentas: `pw-cli`, `pw-loopback`, `pw-play`, `pw-record`, `sox` (medição de RMS).
- Distribuição/WirePlumber do host não registrados ainda.

## Prova 01 — hipótese A: sink de canal + dois loopbacks do monitor

Script: `tools/provas/01-ramos-loopback.sh`. Cria um sink nulo de canal e dois sinks de destino
(pessoal, transmissão), liga cada destino ao monitor do canal por um `pw-loopback`, toca um
seno de 440 Hz (amplitude 0,5, 48 kHz estéreo) no canal e grava os dois destinos ao mesmo tempo.
O ganho é aplicado por `channelVolumes` no nó de saída de cada loopback.

| Cenário | Pessoal (dBFS RMS) | Transmissão (dBFS RMS) |
| --- | --- | --- |
| Base (ganhos 1,0) | −9,27 | −9,23 |
| Pessoal −6 dB | −15,23 (Δ −5,96) | −9,27 (Δ −0,04) |
| Transmissão −6 dB | −9,27 | −15,28 (Δ −6,05) |
| Transmissão com ganho 0 | −9,23 | −∞ |

Critério do passo 2 (±0,2 dB no ramo alterado; <0,1 dB no outro): **atendido** nas duas direções.
Limpeza: após a execução não restou nenhum nó `iara_proof_*` e os dispositivos padrão não mudaram.

## Prova 02 — dois canais, MASTER após a soma, Não atribuídos e MIC

Script: `tools/provas/02-mixes-master-mic.sh`. Dois canais (A, B) com envio pessoal/transmissão cada,
somados em dois sinks de mix; um loopback de cada mix leva ao MASTER correspondente (ganho depois da soma).
Não atribuídos entra só no mix pessoal. MIC (simulado por sink nulo) passa por ganho/mute global
e se divide em fonte dedicada para aplicativos, envio pessoal e envio de transmissão.
Mute é a flag `mute` do nó (`Props`), não ganho 0. Valores em dBFS RMS.

| Cenário | MASTER pessoal | MASTER transmissão | MIC apps |
| --- | --- | --- | --- |
| A sozinho (uma cópia) | −9,23 | −9,27 | — |
| A + B (440 e 880 Hz) | −6,17 | −6,17 | — |
| A pessoal −6 dB | −15,23 | −9,27 | — |
| A transmissão mute (flag) | −9,27 | −∞ | — |
| MASTER pessoal −6 dB (A+B) | −12,22 | −6,22 | — |
| MASTER transmissão −6 dB (A+B) | −6,22 | −12,22 | — |
| Não atribuídos sozinho | −9,18 | −∞ | — |
| MIC sozinho | −9,08 | −9,08 | −9,08 |
| MIC: mute do envio de transmissão | −9,13 | −∞ | −9,13 |
| MIC: mute do envio pessoal | −∞ | −9,13 | −9,13 |
| MIC: mute global | −∞ | −∞ | −∞ |
| MASTER pessoal −20 dB | −29,13 | −9,13 | −9,18 (intocado) |

Leitura: a soma de dois tons distintos dá +3,0 dB (potência), e uma só fonte mede o mesmo nível que a
base da prova 01, sem acréscimo de +6 dB — **sem cópia duplicada** neste cenário. Mute por flag e
ganho por ramo são independentes; MASTER age só no seu mix; Não atribuídos não chega à transmissão;
o ramo dedicado do MIC não sofre o MASTER nem os mutes dos envios, e o mute global silencia os três.
Limpeza: nenhum nó `iara_proof_*` restou.

## Prova 03 — nós hospedados por um processo Rust (bindings `pipewire` 0.10.1)

Código: `tools/pw-probe` (crate descartável, fora do workspace) e `tools/provas/03-hospedagem-rust.sh`.
O processo Rust conecta ao PipeWire, cria sinks nulos com `core.create_object("adapter", …)`, carrega
`libpipewire-module-loopback` **no próprio contexto** (via `pipewire::sys::pw_context_load_module`, unsafe;
não há wrapper seguro), liga os nós pelo registro e aplica `channelVolumes`/`mute` com `Node::set_param(Props)`.

| Cenário | Pessoal | Transmissão |
| --- | --- | --- |
| Base | −9,23 | −9,23 |
| Pessoal −6 dB (Props) | −15,28 | −9,27 |
| Transmissão mute (flag, Props) | −9,27 | −∞ |

- Build: `pipewire`/`pipewire-sys` 0.10.1 compilam contra libpipewire 1.6.9 (precisa de clang/bindgen).
- Hospedagem no contexto do serviço: ao enviar SIGKILL ao processo, **nenhum** nó `iara_probe_*` restou
  (0 nós no grafo). Logo não há órfãos, mas **o áudio dos canais some junto com o serviço**; o comportamento dos
  aplicativos nesse momento (fallback do WirePlumber) ainda não foi observado.

### Achado: o WirePlumber restaura volume/mute dos nós do próprio Iara

`wireplumber` 0.5.18 grava em `~/.local/state/wireplumber/stream-properties` o volume e o mute de streams e sinks
(chave `media.name` ou `node.name`) e os restaura quando o nó reaparece. Numa execução anterior o script
foi encerrado com mute ativo; nas execuções seguintes o ramo de transmissão já nascia mudo (−∞ na base, 4 de 4
repetições) mesmo com os links corretos. Opt-out por nó, confirmado: `state.restore-props=false` e
`state.restore-target=false` nas propriedades do sink e dos streams do loopback; com o estado sujo
(`mute:true`) ainda salvo, a base voltou a −9,23/−9,23 em 2 de 2 execuções.
Consequência para o produto: **todos os nós criados pelo Iara devem declarar esse opt-out**; o estado desejado
é do Iara, não do WirePlumber (spec 6.3: não disputar políticas globais).

## Prova 04 — queda do processo, reinício do PipeWire e hospedagem no daemon

Scripts: `tools/provas/04a-queda-do-processo.sh`, `04b-reinicio-pipewire.sh`, `04c-linger.sh`
(sonda `tools/pw-probe`). Streams de teste: `pw-play` com destino explícito num sink do Iara (S1) e sem destino (S2).
O reinício do PipeWire foi feito de verdade na sessão do usuário (`systemctl --user restart pipewire.service`).

**04a — SIGKILL do processo proprietário (2 execuções, mesmo resultado)**

| Momento | S1 (destino explícito `iara_probe_chan`) | S2 (sem destino) |
| --- | --- | --- |
| Processo vivo | → `iara_probe_chan` | → saída física padrão |
| 3 s após SIGKILL | → **saída física** (fallback) | → saída física |
| Nós recriados (4 s) | continua na saída física | igual |
| `pw-metadata … target.object=iara_probe_chan` | volta ao `iara_probe_chan` em ≤2 s | igual |

Leitura: a queda do serviço faz o fluxo cair na saída física (vaza para os alto-falantes) e **o fluxo não volta
sozinho** quando o nó reaparece; o serviço precisa reencaminhar por metadata ao reiniciar. Metadata funciona para isso.

**04b — reinício do PipeWire (1 execução)**
- Todos os nós do processo sumiram do grafo (0 de 11), WirePlumber voltou ativo, dispositivos padrão preservados
  (mesmos nomes; IDs renumerados) e os aplicativos da sessão (Zen, Cider) reconectaram à saída física.
- O processo Rust **continuou vivo** e ainda acreditava que seus nós existiam (a sonda não ouve o erro do core):
  sem tratamento de desconexão, o serviço ficaria “saudável” e sem áudio. É requisito: ouvir o erro do core,
  invalidar proxies, reconectar e reconstruir sem duplicar (spec 8.8).
- Tempo de indisponibilidade **não medido** (o `printf` do script usou o locale errado e o número é inválido).

**04c — `object.linger` (nó criado no daemon via `create_object("adapter")`)**
- Sem linger: o nó some junto com o processo. Com `object.linger=true`: **sobrevive ao SIGKILL** (removido depois
  à mão com `pw-cli destroy`). Logo buses/sinks podem ficar no daemon, mas viram **órfãos** se ninguém os limpar:
  o serviço precisa marcar seus objetos (prefixo/propriedade própria) e varrê-los na inicialização.
- Os ramos com ganho (loopbacks) vivem no processo cliente e não têm equivalente com linger nesta prova. Hospedar tudo no
  daemon por `pipewire.conf.d` **não foi testado** (exigiria reinícios extras e não permite canais dinâmicos sem recarregar).

## Prova 05 — mover aplicativos e ausência de dispositivo físico

Scripts: `tools/provas/05a-mover-aplicativos.sh`, `05b-ausencia-dispositivo.sh`, `lib_links.py` (sonda `tools/pw-probe`, comando `lpx`).

**05a — mover por metadata (streams `pw-play` de teste)**

| Caso | Resultado |
| --- | --- |
| A (sem destino): `pw-metadata … target.object=<nome do sink>` | moveu para o sink do Iara |
| B com `node.dont-move=true` | **não moveu**; a propriedade é visível no nó antes da tentativa (detectável) |
| C com destino explícito na saída física (`--target`) | moveu (metadata vence o destino explícito do stream) |
| Mudança externa em A com `pw-metadata` | A voltou à saída física; um observador independente (`pw-metadata -m`) viu `update id:<A> key:'target.object'` com o nome |
| Mudança externa com `pactl move-sink-input` (protocolo Pulse) | A moveu; o observador viu `target.node=<id do nó>` e `target.object=<serial numérico>` (formato diferente do nosso, que é o nome) |

Leitura: movimentos externos são observáveis como eventos de metadata cujo sujeito é o id do stream; escritas próprias
são reconhecíveis por quem as fez. Não se mediu a política de aplicativos que ignoram metadata sem `dont-move`.
Não explicado: na segunda execução o stream A já nasceu ligado ao sink do Iara, sem pedido. Hipótese não confirmada:
restauração de destino do WirePlumber ou metadata remanescente; o arquivo de estado não contém entrada para A.
Da leitura de `state-stream.lua` (não medido): o WirePlumber grava o destino ao ver `target.object` mudar quando ele
resolve o nó pelo **serial numérico** (caso do `pactl`/seletores do desktop), e a escrita por nome (a nossa) não o resolve.

**05b — ausência e retorno do fifine AM8 Pro (perfil de placa desligado e religado por software)**

Quatro loopbacks (`nofb` = com `node.dont-fallback`; `fb` = sem) do microfone físico para um sink do Iara e de um sink do Iara para
a saída física do fifine. Resultado após 4 s de ausência e 6 s após o retorno (1 execução):

| Variante | Ausente | Após o retorno |
| --- | --- | --- |
| Entrada `nofb` | nó do loopback **sumiu** do grafo (sem fallback) | **não voltou** (continua ausente) |
| Entrada `fb` | **caiu para outro microfone** (webcam C922) | voltou ao fifine sozinha |
| Saída `nofb` | nó do loopback sumiu (sem fallback para as caixas) | **não voltou** |
| Saída `fb` | **caiu para as caixas (ALC887)** | voltou ao fifine sozinha |

- Fonte padrão do sistema: mudou para a C922 durante a ausência e voltou para o fifine sozinha.
- O módulo loopback encerra o par de streams quando o destino/origem some com `dont-fallback`; o processo cliente seguiu vivo.
- Consequência: nem o loopback com fallback (troca indevida de dispositivo, proibida pelas specs 8.5/8.6) nem o sem fallback (some e não volta)
  atendem sozinhos. O serviço precisa ser dono do ciclo: manter os nós virtuais estáveis (silêncio sem origem), observar o registro
  e **recriar** o par de streams quando o dispositivo preferido voltar, com a geração da escolha (8.5).
- Efeito na sessão do usuário durante o teste: o stream de captura do Zen terminou na C922 depois da ausência do fifine. A causa **não está
  estabelecida**: pode ter sido o fallback do WirePlumber, o navegador reagindo à remoção do dispositivo, ou o próprio usuário trocando o microfone
  no Google Meet na mesma hora (o usuário relatou essa possibilidade). O que está medido é só o comportamento dos loopbacks de teste e da fonte padrão.
  Por precaução o Zen foi movido ao fifine por metadata (uma chave `target.object` do Zen permanece no daemon até o próximo reinício do PipeWire);
  isso pode ter sobrescrito uma escolha deliberada do usuário. Uma limpeza minha apagou essa chave uma vez por engano e ela foi regravada.
  O perfil do fifine foi restaurado ao original.

## Prova 06 — latência, CPU, RAM e xruns (passo 8)

Scripts: `tools/provas/06a-latencia.sh`, `06b-recursos.sh`. PipeWire 1.6.9, `clock.rate=48000`, `clock.quantum=1024`
(21,3 ms por ciclo), grafo guiado pelo driver ALC887 da sessão do usuário, que seguia com Zen e Cider ativos (ruído de base).

**06a — latência de um clique entre sinks de uma cadeia de loopbacks hospedados no processo Rust (5 repetições por caminho)**

| Caminho | Atraso (ms) |
| --- | --- |
| Controle positivo: loopback com `-d 0,05` s | 50,0 / 50,0 / 50,0 / 50,0 / 50,0 |
| canal → mix (1 loopback) | 0,0 em todas |
| canal → MASTER (2 em série) | 0,0 em todas |
| MIC → fonte para aplicativos (2 em série) | 0,0 em todas |
| MIC → MASTER de transmissão (3 em série) | 0,0 em todas |

Método: dois taps simétricos copiam a origem para FL e o fim do caminho para FR de um sink estéreo; o atraso é a diferença entre
os picos (resolução de 1 amostra ≈ 0,02 ms). O controle de 50 ms foi lido corretamente, então a ferramenta enxerga atrasos.
Leitura: neste grafo (um único driver, nós ligados) os estágios de loopback não adicionam atraso mensurável entre sinks.
Isto **não** inclui a latência de saída do dispositivo físico, nem o comportamento com outro quantum (aplicativos que pedem quantum
menor/maior) ou com Bluetooth.

**06b — recursos com a topologia completa da prova 02 (11 loopbacks no processo Rust, 54 nós no grafo)**

| Estado | CPU pipewire | CPU wireplumber | CPU sonda | RSS pipewire | RSS sonda |
| --- | --- | --- | --- | --- | --- |
| Base (sem topologia, 15 s) | 1,2 % | 0,0 % | — | 31,3 MiB | — |
| Topologia ociosa (20 s) | 2,0 % | 0,1 % | 1,0 % | 115,6 MiB | 24,2 MiB |
| 3 tons ativos (20 s) | 1,6 % | 0,0 % | 0,6 % | 125,5 MiB | 24,4 MiB |

- CPU em % de um núcleo, de janelas de 20 s (contadores de `/proc`); xruns (`pw-top`, coluna ERR cumulativa): **0** nos 32 nós do Iara e
  no driver principal, nessas janelas curtas.
- RSS do `pipewire` sobe ~85–95 MiB com a topologia (≈1,7 MiB por nó, a investigar se os buffers são dimensionáveis) e **volta a 31,5 MiB**
  depois da remoção (pico 123,5 MiB): sem vazamento observado.
- Sem metas numéricas definidas; estes são os números de partida para a spec 10.

### O que isto NÃO prova

- Medições longas (horas), CPU com medidores de nível ativos e com efeitos; o tempo de indisponibilidade após reinício do PipeWire; clientes de captura reais (OBS/Discord) lendo a fonte virtual; saída sem hot-plug físico real (o perfil de placa foi desligado por software); Bluetooth.
- Microfone físico real (a prova 02 usa um sink nulo como fonte simulada).
- Provas 01–02 usaram `pw-loopback`/`pw-cli`; a 03 usa um processo Rust, mas só no contexto cliente. Hospedagem no daemon (módulo em `pipewire.conf.d`) não foi testada e a decisão de hospedagem segue aberta.
- A diferença de ~0,2 dB entre o RMS esperado (−9,03) e o medido (−9,25) na base não foi investigada.
