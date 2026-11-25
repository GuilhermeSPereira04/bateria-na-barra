# battery-bar — design

Data: 2026-11-24

## Intenção

Um app Windows pequeno, em Rust, que mostra **na barra de tarefas**, logo à
esquerda da bandeja (perto de Wi-Fi/som/relógio), o ícone e a porcentagem de
bateria de três dispositivos do usuário:

```
 🖱 93%   🎮 64%   📱 81%    ^  📶 🔊   12:41
```

Referência visual: `D:\dev\Wireless-Device-Battery-Monitor\1795560998182_image.png`
(ícone monocromático + porcentagem em negrito, cor do texto da barra, sem
fundo, espaçamento generoso entre itens).

**Por quê:** o usuário quer olhar e saber na hora quanto de bateria têm seus
periféricos, sem abrir driver nem app, com algo leve. O app
"Wireless Device Battery Monitor" (pasta vizinha) faz isso, mas não compila:
depende de um crate privado (`mouse-battery-core`) e não suporta esses
dispositivos.

## Dispositivos (detectados no PC do usuário)

| Dispositivo | Conexão | ID | Como ler |
| --- | --- | --- | --- |
| Mouse Zaopin (receptor se identifica como "Z4D") | Receptor USB 2.4 GHz | `VID 3554 / PID F527`, interface vendor em `MI_01` (várias coleções `COL01..COL07`) | Protocolo **desconhecido**; ver "Zaopin" abaixo |
| Controle PS5 DualSense | Bluetooth | `VID 054C / PID 0CE6` | Relatório HID de entrada |
| Celular Android (modelo ainda não informado) | ADB por Wi-Fi (depuração sem fio) | — | `adb shell dumpsys battery` |

Fora do escopo: teclado Akko (`3151:502D`) e headset ME6S (`0C76:8688`) — com fio.

## Objetivos

- Faixa sempre visível na barra de tarefas com ícone + % de cada dispositivo,
  na ordem: mouse, controle, celular.
- Indicar **carregando** (glifo de raio junto à %) e **bateria baixa**
  (texto em vermelho/laranja abaixo de 20%).
- Leve: meta de < 10 MB de RAM, CPU ~0% em repouso, `.exe` único.
- Ícone na bandeja com menu: *Atualizar agora*, *Iniciar com o Windows*
  (liga/desliga), *Sair*. Tooltip do ícone lista os três dispositivos com
  status, inclusive os offline.

## Não-objetivos

- Tela de configurações, temas, idiomas, notificações, auto-update.
- Outros dispositivos, iPhone, barra de tarefas vertical, barra em
  múltiplos monitores (só o monitor principal).

## Anti-objetivos (falharia mesmo "funcionando")

- **Atrapalhar o uso**: travar/atrasar mouse ou controle, mudar luz, DPI ou
  qualquer configuração do mouse. Para o Zaopin só se enviam comandos de
  **leitura**; nunca comandos de escrita/configuração.
- **Ficar pesado** ou gastar bateria do celular perceptivelmente.
- Mostrar número errado como se fosse certo: se não der para ler, mostrar
  que não leu (ver "Estados").
- Ficar por cima de ícones da barra ou sumir atrás dela.

## Abordagens consideradas

**Exibição** (o usuário escolheu C):
- A. Ícone na bandeja + painel ao clicar — leve, mas exige clique.
- B. Só ícone + menu nativo — o mais leve, visualmente pobre.
- **C. Números fixos na barra de tarefas** — escolhido: é o que a imagem de
  referência mostra; mais trabalhoso por causa do posicionamento.

**Celular** (o usuário escolheu ADB):
- Bluetooth: Windows só reporta bateria de alguns celulares, instável.
- App companheiro no Android mandando por Wi-Fi: confiável, mas exige app
  no celular.
- **ADB por Wi-Fi** — escolhido pelo usuário: nada a instalar no celular,
  usa a depuração sem fio do Android 11+.

## Como funciona

### Janela na barra
- Janela própria, sem borda, sem entrada na barra/Alt-Tab
  (`WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE`),
  desenhada com transparência por pixel (`UpdateLayeredWindow`) para que o
  texto fique nítido sobre a barra transparente/acrílica do Windows 11.
- Posicionada à esquerda da área de notificação, centralizada verticalmente
  na barra. Precisa se reposicionar quando a barra mudar (DPI, resolução,
  ícones da bandeja aparecendo/sumindo, `TaskbarCreated` após o Explorer
  reiniciar) e se manter acima da barra quando ela recebe foco.
  Referência de como fazer: `D:\dev\Wireless-Device-Battery-Monitor\src\taskbar.rs`
  (localiza a bandeja via UI Automation; licença permite estudo/modificação
  para uso pessoal).
- Cliques passam para a barra? Não: a faixa ocupa só o espaço livre à
  esquerda da bandeja; clicar nela abre o mesmo menu do ícone da bandeja.
- Cor do texto segue o tema da barra (`SystemUsesLightTheme` em
  `HKCU\...\Themes\Personalize`): branco no escuro, preto no claro.
- Ícones: fonte *Segoe Fluent Icons* (vem no Windows 11) — mouse, gamepad,
  celular. Texto em *Segoe UI Variable* semibold, tamanho da imagem de
  referência.
- Sem framework de interface (sem egui/GPUI/etc.); crates `windows`/
  `windows-sys`. Direct2D/DirectWrite ou GDI+ fica a critério do builder,
  desde que o texto saia suavizado sobre fundo transparente.

### Estados por dispositivo
- **Lido**: ícone + `NN%` (+ raio se carregando).
- **Offline / não encontrado**: continua mostrando o último valor lido por até
  10 min (desconexão conta como falha de leitura — evita o celular piscar
  quando o ADB cai no modo de espera). Passados 10 min sem leitura boa, o
  item some da faixa e aparece no tooltip como "desconectado". Se todos
  sumirem, só o ícone da bandeja permanece. *(decisão minha)*
- **Conectado mas leitura falhou**: ícone + `--%` esmaecido.
- Conectado mas sem nenhuma leitura boa ainda (ex.: logo ao iniciar): `--%`.

### Leitura
- Cada leitor roda numa thread de fundo e manda resultados para a janela;
  nenhuma leitura bloqueia a interface.
- **DualSense (BT)**: a cada 60 s, abrir o dispositivo HID (crate `hidapi`),
  ler relatórios de entrada até obter o relatório completo `0x31`, extrair
  nível e estado de carga, fechar. Detalhes do formato (offset do byte de
  bateria/carga, escala 0–10 → %) ficam para o builder, a partir de
  documentação pública (ex.: driver `hid-playstation` do Linux).
  **Somente leitura passiva** (decisão do usuário, que usa DS4Windows às
  vezes): o app **nunca** pede o feature report `0x05` nem envia nada ao
  controle, porque isso o deixa no modo `0x31` e pode quebrar DS4Windows,
  emuladores e jogos fora da Steam. Se o controle estiver mandando só o
  relatório simplificado `0x01` (nem Steam nem DS4Windows o mudaram), o item
  mostra `--%`. Se o controle estiver oculto pelo HidHide do DS4Windows e não
  der para abri-lo, trata como "conectado mas leitura falhou" (`--%`).
  Abrir o dispositivo deve ser em modo compartilhado, sem acesso exclusivo.
- **Celular (ADB)**: a cada 3 min, `adb -s <serial> shell dumpsys battery`;
  usar `level` e `status` (2 = carregando, 5 = cheio). Se não houver
  dispositivo em `adb devices`, tentar reconectar via `adb mdns services` /
  `adb connect`, já que a porta da depuração sem fio muda quando o celular
  reinicia. `adb.exe` procurado ao lado do `.exe` e depois no `PATH`.
  Processos do adb iniciados sem janela de console. O pareamento inicial
  (`adb pair`) é feito uma vez, manualmente, com instruções no README.
- **Zaopin (3554:F527)**: *Atualização 2026-11-24: protocolo obtido do driver oficial (`HIDUsb.dll`), ver `docs/zaopin-protocol.md`; o app envia exatamente o pedido de bateria do software oficial.* Plano original: protocolo desconhecido. **Caminho seguro**
  (decisão do usuário) — nenhum comando "chutado" de outras marcas é enviado
  ao receptor. Etapas, numa fase separada, depois do app funcionando:
  1. Ferramenta só de listagem (`examples/probe_zaopin.rs`): lista as
     coleções HID do receptor (usage page, usage, tamanhos de relatório).
     Não envia nada.
  2. Com o usuário presente: instalar o software oficial da Zaopin (se ainda
     não tiver) e USBPcap + Wireshark; capturar o tráfego enquanto o software
     mostra a bateria; comparar com o nível real para achar o pedido e o
     byte de bateria/carga.
  3. O app passa a enviar **exatamente** o pedido capturado, nada mais.
  Até lá o mouse aparece como `--%`. O app é entregue e útil com controle +
  celular antes disso. Intervalo de leitura do mouse: 60 s.

### Inicialização
- *Iniciar com o Windows* via `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
- Uma instância só (mutex nomeado).

## Restrições
- Windows 11, monitor principal, barra horizontal embaixo.
- Rust estável, toolchain `x86_64-pc-windows-gnu` instalada em `D:\rust`;
  `hidapi` com backend `windows-native` (Rust puro, sem compilador C). Projeto novo em `D:\dev\battery-bar`, independente do repositório
  Wireless-Device-Battery-Monitor.
- Já instalados (2026-11-24): Rust em `D:\rust` e `adb` em
  `D:\android\platform-tools` (no PATH do usuário).
- A meta de < 10 MB de RAM vale para o nosso `.exe`; o servidor do `adb`
  fica rodando em segundo plano e não entra na conta *(decisão minha)*.
- No celular: Android 11+, depuração sem fio ligada e pareada uma vez.

## Decisões tomadas por mim (Claude), não pelo usuário
- Nome/pasta do projeto, intervalos (60 s / 3 min), limite de 20% para "baixo",
  esconder dispositivo offline, ordem dos itens, menu da bandeja, ADB sem
  janela, não suportar multi-monitor.

## Deixado para o builder
- Organização dos módulos, Direct2D vs GDI+, detalhes de parsing,
  estratégia de manter a janela acima da barra, testes.
