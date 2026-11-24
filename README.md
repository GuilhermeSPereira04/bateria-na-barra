# battery-bar

Mostra na barra de tarefas do Windows 11, à esquerda da bandeja, a bateria de:

- 🖱 **Mouse Zaopin** (receptor 2.4 GHz `3554:F527`) — por enquanto mostra `--%` (veja *Limitações*)
- 🎮 **Controle PS5 (DualSense)** por Bluetooth
- 📱 **Celular Android** via ADB por Wi-Fi

```
 🖱 --%   🎮 64%⚡   📱 81%    ^  📶 🔊   12:41
```

- Raio ⚡ = carregando. Texto vermelho = abaixo de 20%. `--%` esmaecido = conectado, mas sem leitura.
- Dispositivo desconectado continua com o último valor por 10 min e depois some.
- A faixa some em jogos/vídeos em tela cheia e acompanha a barra quando ela está em ocultar automaticamente.
- Passe o mouse no ícone da bandeja para ver os três em texto. Clique (no ícone ou na faixa) para o menu:
  **Atualizar agora**, **Iniciar com o Windows**, **Sair**.
- Leve: ~4 MB de memória, CPU ~0% em repouso. Leituras: mouse e controle a cada 60 s, celular a cada 3 min.

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
- **Mouse Zaopin:** o protocolo de bateria ainda não é conhecido. O plano é capturar o que o
  software oficial da Zaopin envia ao receptor (USBPcap + Wireshark) e repetir só esse pedido.
  Até lá, `--%`. A listagem das interfaces do receptor está em `docs/zaopin-probe.txt`
  (`cargo run --example probe_zaopin`, que não envia nada ao receptor).
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
