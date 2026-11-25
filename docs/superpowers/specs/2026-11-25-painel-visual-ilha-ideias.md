# Painel — ideias de visual e "ilha" (rascunho, ainda não aprovado pra código)

Data: 2026-11-25. Branch `feat/painel-topo`.

## Visual (referência: D:\download\Liquid-Glass-Dark.webp)
Estilo liquid glass escuro (iOS 26) sem a lente distorcendo o fundo:
- pílulas e círculos, cartões com cantos de 20–24 px
- borda de luz: contorno fino, claro em cima e sumindo embaixo
- camadas grafite translúcidas, sombra suave
- cor só no que está ativo (azul por padrão?)
- abas como controle segmentado em pílula, busca em pílula
- barras grossas com bolinha branca
- player em cartão (capa maior, barra de progresso, botões grandes)
- menu de áudio com ícone por linha
Pendente: decidir se a cor de ativo padrão vira azul.

## Ilha (decidido pelo usuário)
- Com o painel fechado: uma pílula preta pequena (~120×28 px, cantos totalmente redondos)
  no centro do topo do monitor principal, **sempre visível**, **sem conteúdo**.
- Some em tela cheia (mesma regra do painel).
- Abre igual a hoje: segurar o mouse no topo por 0,5 s. Clicar na pílula não muda nada.
- **Animação:** ao abrir, a pílula estica e desce até virar o painel. Ao fechar, encolhe de
  volta pra pílula. Uns 200 ms com easing, e só gasta CPU durante a animação.
- Risco a testar: o desfoque acrylic do Windows pode atrasar quando a janela muda de tamanho
  a cada quadro. Se ficar feio, animar o recorte (SetWindowRgn) em vez do tamanho, ou
  desligar o desfoque só durante a animação.
