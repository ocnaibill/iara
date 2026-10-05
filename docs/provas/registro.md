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

### O que isto NÃO prova

- Passos 5–8: aplicativos reais como clientes de captura, ausência/reconexão da entrada física, queda do processo/PipeWire e fallback, mudança de aplicativos por metadata, CPU/RAM/latência/xruns.
- Microfone físico real (a prova 02 usa um sink nulo como fonte simulada); latência acumulada dos estágios encadeados (até 3 loopbacks em série no caminho do MIC) não foi medida.
- Os nós foram hospedados por processos `pw-loopback`/`pw-cli`, não por um serviço Rust; a decisão de hospedagem segue aberta.
- A diferença de ~0,2 dB entre o RMS esperado (−9,03) e o medido (−9,25) na base não foi investigada.
