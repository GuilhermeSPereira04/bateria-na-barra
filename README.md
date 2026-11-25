# battery-bar

Mostra na barra de tarefas do Windows 11, à esquerda da bandeja, a bateria de:

- 🖱 **Mouse Zaopin** (receptor 2.4 GHz `3554:F527`, ou o mouse no cabo USB `3554:F526`)
- 🎮 **Controle PS5 (DualSense)** por Bluetooth
- 📱 **Celular Android** via ADB por Wi-Fi

```
 🖱 25%   🎮 95%   📱 81%⚡    ^  📶 🔊   12:41
```

- Raio ⚡ = carregando. Texto vermelho = abaixo de 20%. `--%` esmaecido = conectado, mas sem leitura.
- Dispositivo desconectado continua com o último valor por 10 min e depois some.
- A faixa some em jogos/vídeos em tela cheia e acompanha a barra quando ela está em ocultar automaticamente.
- Passe o mouse no ícone da bandeja para ver os três em texto. Clique (no ícone ou na faixa) para o menu:
  **Atualizar agora**, **Iniciar com o Windows**, **Sair**.
- Leve: ~4 MB de memória, CPU ~0% em repouso. Leituras: mouse e controle a cada 60 s, celular a cada 3 min.

## Painel do topo

Um painel de vidro que desce do topo da tela, no monitor principal.

- **Abrir:** deixe o mouse na borda de cima da tela por 0,5 s, ou use **Abrir painel** no menu da bandeja.
  `battery-bar.exe --painel abrir` abre o painel na instância que está rodando (útil pra atalho); `--painel fechar` fecha.
  Fecha com `Esc`, clicando fora ou quando perde o foco, e sozinho ~0,5 s depois que o mouse sai dele
  (isso fica pausado enquanto há texto na busca, durante o speed test ou com o ⚙ ou a lista de áudio aberta).
- **Busca de apps:** é só digitar com o painel aberto; `Enter` abre o resultado.
- **Atalhos rápidos:** Wi-Fi, Bluetooth e áudio. Clique liga/desliga (no áudio, muta); clique com o botão
  direito abre a página correspondente das Configurações do Windows. No Wi-Fi e no Bluetooth a seta faz o
  mesmo; no áudio a seta abre uma lista suspensa logo abaixo do cartão: uma barra de volume (clique ou arraste)
  e as saídas de áudio, com ✓ na atual; clicar numa saída troca o som pra ela. `Esc` ou um clique fora fecham só a
  lista. A roda do mouse sobre o cartão ou a lista também muda o volume.
- **Bateria** dos dispositivos e **player de mídia** (o que estiver tocando).
- **Abas:** *Recursos* (CPU, GPU e memória), *Calendário* e *Speed test* (ping, download e upload).
- **⚙ Cores:** cor de destaque (presets, a cor de destaque do Windows), tom do vidro (escuro, claro ou destaque) e uma
  barra de **Transparência** (0–100%, padrão 45%) que age na hora enquanto você arrasta. O vidro é acrylic do Windows.

A escolha fica em `%APPDATA%\battery-bar\config`, um arquivo de texto com `chave=valor` por linha
(`#` começa um comentário):

```
accent=preset:0      # preset:0..7, windows ou #RRGGBB
tint=dark            # dark, light ou accent
transparency=45      # 0 a 100 (mais alto = mais transparente)
```

Valores inválidos são ignorados e ficam os padrões. Escolher uma cor no ⚙ regrava o arquivo inteiro:
comentários escritos à mão se perdem. 
Consumo medido (memória privada, a coluna "Memória" do Gerenciador de Tarefas, com `tools/medir-memoria.ps1`):
fechado ~2,8 MB, aberto ~17,7 MB e fechado de novo ~1,7 MB depois de 10 ciclos de abrir e fechar.

## Instalar

1. Compile (veja abaixo) ou use o `.exe` já gerado em `target\release\battery-bar.exe`.
2. Copie o `battery-bar.exe` para uma pasta fixa, por exemplo `D:\apps\battery-bar\`.
3. Execute. Para ligar junto com o Windows, use o menu **Iniciar com o Windows**
   (se mover o `.exe` depois, desligue e ligue de novo essa opção).

Opcional: coloque `adb.exe` (e as DLLs `AdbWinApi.dll`, `AdbWinUsbApi.dll`) na mesma pasta do
`battery-bar.exe`; se não estiver lá, ele usa o `adb` do PATH.

## Parear o celular (uma vez só)

Precisa de Android 11+, celular e PC na mesma rede Wi-Fi.

1. No celular: **Configurações → Sobre o telefone →** toque 7 vezes em **Número da versão**
   para liberar as *Opções do desenvolvedor*.
2. **Opções do desenvolvedor → Depuração por Wi-Fi** → ative → toque em
   **Parear dispositivo com código de pareamento**. Aparecem um **IP:porta** e um **código de 6 dígitos**.
3. No PC (PowerShell):
   ```powershell
   adb pair 192.168.0.10:41234   # o IP:porta da tela de pareamento
   # digite o código de 6 dígitos
   ```
4. Pronto. O battery-bar encontra o celular sozinho (mDNS), inclusive quando a porta muda
   depois de reiniciar o celular. Use **Atualizar agora** para não esperar os 3 min.

Se o celular sumir depois de um tempo, confira se a **Depuração por Wi-Fi** continua ativa
(alguns celulares desligam ao trocar de rede).

## Limitações

- **Controle PS5:** o app só *escuta* o controle, nunca envia nada (para não atrapalhar o
  DS4Windows). A bateria só aparece quando a **Steam** ou o **DS4Windows** está aberto e já
  colocou o controle no modo de relatório completo. Fora isso aparece `--%`. Se o DS4Windows
  esconder o controle com o HidHide, também aparece `--%`.
- **Mouse Zaopin:** o app envia ao receptor só o pedido de bateria que o software oficial
  da Zaopin usa (tirado da `HIDUsb.dll` do driver; detalhes em `src/readers/zaopin.rs`).
  O número segue a mesma suavização do app oficial: descarregando só desce, carregando só sobe e
  fica até 10% abaixo do que o mouse informa (o cabo infla a leitura). Com o mouse dormindo,
  o último valor continua por 10 min e depois vira `--%`.
  Ferramentas: `cargo run --example zaopin_battery` (uma leitura, mostra a resposta crua) e
  `cargo run --example probe_zaopin` (lista as interfaces, não envia nada).
- **Controle PS5:** para a bateria aparecer com a Steam, ative em Steam → Configurações →
  Controle → "Steam Input para controles PlayStation".
- Só o monitor principal e barra de tarefas horizontal embaixo.

## Compilar

Toolchain usada: Rust `x86_64-pc-windows-gnu` + MinGW-w64 (WinLibs), tudo no disco D:.

| O quê | Onde |
| --- | --- |
| Rust (rustup/cargo) | `D:\rust` (`RUSTUP_HOME=D:\rust\rustup`, `CARGO_HOME=D:\rust\cargo`) |
| MinGW-w64 (o `dlltool` do Rust precisa do `as.exe` daqui) | `D:\mingw64\bin` |
| `dlltool` do Rust | `D:\rust\rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained` |
| Android platform-tools (`adb`) | `D:\android\platform-tools` |

Todas essas pastas estão no PATH do usuário. Em um terminal novo:

```powershell
cargo build --release   # gera target\release\battery-bar.exe
cargo test              # testes de unidade
cargo test -- --ignored --nocapture   # verificações ao vivo (controle, adb, barra de tarefas)
```
