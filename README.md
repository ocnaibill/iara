# Iara

Mixer de áudio para Linux, em Rust e GTK4. Licença MIT.

Este workspace é um esqueleto inicial: não cria dispositivos, não move fluxos e não altera áudio ou preferências do sistema. O backend PipeWire e o IPC ainda não estão implementados. Veja [especificacao.md](docs/especificacao.md), especialmente a seção 6.3.

## Organização

- `iara-core`: domínio compartilhado, ganhos, envios e fórmula do ChatMix; sem GTK/PipeWire.
- `iara-service`: ponto de entrada do serviço e contrato provisório do backend.
- `iara-ui`: ponto de entrada GTK4, habilitado por feature para separar dependências nativas.

O contrato de backend não comprova a viabilidade da topologia. O serviço retorna falha explícita enquanto não houver backend implementado.

## Verificação

Requer Rust/Cargo e acesso ao registro de dependências. A interface também exige bibliotecas de desenvolvimento GTK4, pkg-config e ambiente gráfico. A versão gtk-rs 0.10 é uma seleção inicial, ainda sujeita à resolução e compilação.

```sh
cargo fmt --all --check
cargo test -p iara-core
cargo check --workspace
cargo check -p iara-ui --features gtk-ui
cargo run -p iara-ui --features gtk-ui
```

Ainda não há `Cargo.lock`: gere-o e versioná-lo após a primeira resolução bem-sucedida. O ambiente de criação não possuía Cargo/Rust nem GTK4/PipeWire disponíveis; compilação e testes Rust não foram executados. Manifestos e empacotamento foram verificados com Python.

## Próximo bloqueio técnico

Execute as provas da seção 6.3 numa sessão Linux com PipeWire/WirePlumber. Meça os dois ramos capturados, não apenas sliders ou metadata. Só então escolha loopback, filter-chain ou processamento próprio e implemente o backend.
