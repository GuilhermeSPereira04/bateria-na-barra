# Painel do topo — ajustes após o primeiro teste

Data: 2026-11-25
Base: `docs/superpowers/specs/2026-11-25-painel-topo-design.md` (continua valendo no que não
mudar aqui). Branch `feat/painel-topo`.

## Por quê

O usuário testou o painel e ele funciona, mas:
- o vidro ficou opaco e esverdeado, longe do "vidro fosco transparente" escolhido no modelo;
- quer controlar o volume e trocar a saída de áudio sem a parte de baixo do painel virar
  outra "página";
- 1,5 s segurando no topo é tempo demais;
- (achado nosso) com o painel aberto o processo mostrou **70 MB**. A meta era ≤ ~20 MB.

## Mudanças

### 1. Vidro com tom e transparência controlados (abordagem escolhida)
- **Hoje:** usa o backdrop padrão do sistema (`DWMSBT_TRANSIENTWINDOW`), que tem opacidade
  fixa. Por cima, o D2D pinta um retângulo de tom; na config do usuário esse tom está em
  `accent` (verde a 15%).
- **Novo:** desfoque via `SetWindowCompositionAttribute` com
  `ACCENT_ENABLE_ACRYLICBLURBEHIND`. A cor e a opacidade vão no `GradientColor` (ABGR), o
  Windows continua fazendo o desfoque (custo ~0) e o backdrop do DWM fica em `DWMSBT_NONE`.
  O retângulo de tom desenhado pelo D2D sai, porque a cor agora vem do próprio acrylic.
  Cantos arredondados continuam (`DWMWCP_ROUND`).
- **⚙ ganha uma barra "Transparência"** (0–100%). A opacidade do tom é
  `1 − transparência`. Os tons continuam: escuro `#20242C`, claro `#F3F3F3` e cor de
  destaque. **Padrão: escuro com 45% de transparência**, igual ao modelo A
  (`rgba(32,36,44,.55)`). Fica salvo na config como `transparency=45`.
- A mudança aparece na hora, enquanto arrasta.
- Se a função não existir ou falhar, volta pro backdrop do sistema como hoje. Nunca pode
  travar.
- Os cartões continuam brancos a 7% por cima do vidro.

### 2. Áudio: barra de volume e lista suspensa
- **A setinha do cartão de áudio abre uma lista suspensa** logo abaixo do cartão, desenhada
  por cima do resto do painel (não é uma janela separada). A parte de baixo (abas) não muda.

```
[Wi‑Fi][BT][🔊 74% ⌄]   🖱 100%
        ╭───────────────────────╮
        │ 🔊 ━━━━━━━━●───  74%   │  ← barra: clicar ou arrastar
        │ ✓ Alto-falantes        │  ← saída atual marcada
        │   Fone USB             │
        ╰───────────────────────╯
```

- A barra muda o volume ao clicar ou arrastar, ao vivo. A roda do mouse também funciona em
  cima dela.
- Clicar numa saída troca a saída e fecha a lista. A saída atual aparece marcada com ✓.
- A lista fecha com um clique fora dela (dentro do painel), com Esc (o primeiro Esc fecha só
  a lista) ou na setinha de novo.
- Enquanto ela está aberta, a regra "mouse fora fecha o painel" fica pausada, como hoje.
- O resto do cartão de áudio fica igual: clique = mudo, roda = volume, clique direito =
  Configurações > Som.
- Bluetooth: **sem mudança**, a pedido do usuário.

### 3. Abrir mais rápido
- O tempo segurando o mouse no topo passa de 1,5 s pra **0,5 s** (`HOLD_TO_OPEN`). O resto do
  gatilho fica igual: não abre com botão apertado nem em tela cheia, e pra reabrir precisa
  sair da borda.
- *Risco aceito:* com 0,5 s fica mais fácil abrir sem querer ao parar o mouse nas abas do
  navegador.

### 4. Memória com o painel aberto
- Meta da spec original: aberto ≤ ~20 MB, fechado ≤ ~8 MB (coluna "Memória").
- Suspeita principal: o render target do painel é de **hardware**
  (`CreateHwndRenderTarget` padrão), o que carrega o D3D e o driver da GPU (dezenas de MB).
  A faixa usa render target de **software** e fica em ~4 MB.
- Investigar medindo antes de mudar. Provável correção: render target de software (ou WARP)
  pro painel. Desenhar um painel pequeno a cada hover é barato em CPU.
- Também conferir o que fica carregado depois de fechar o painel.
- **Medição:** desta vez eu rodo o programa pra medir. Fecho a sua instância, abro a nova
  versão, meço fechado, aberto em cada aba e fechado depois de 10 ciclos, e deixo a nova
  versão rodando no fim. Os números vão pro README.

## Fora do escopo
Bluetooth integrado, Wallpaper Engine, "liquid glass" com distorção (lente).

## Deixado pro construtor
Medidas exatas da lista suspensa e da barra (seguindo o estilo dos cartões), passo do
arrasto, como desenhar o ✓, e qual tipo de render target de software usar, desde que a
meta de memória seja cumprida.
