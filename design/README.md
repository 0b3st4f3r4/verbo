# design/ — Marca VerboLang: triângulo invertido de gradientes com pássaros

Proposta de evolução da marca (ágosto 2026), a partir do pedido:
*"triângulo de gradientes entre as 3 cores, com vazamento interno e
simétrico que defina a aparência de pássaros nos 3 lados do triângulo"*,
refinado com: *"inverta o triângulo (para ressoar como V), puxe o ponto
onde o violeta reluz alinhado aos outros dois, e a linha tracejada deve
passar por esses pontos"*.

## Conceito

- **Triângulo equilátero invertido** (ponta para baixo — o contorno lê como
  o V da marca original). Vértices: azul `#4da3ff` (sup. esq.), vermelho
  `#ff5a52` (sup. dir.), verde `#3fb96f` (ponta do V).
- **Cada lado é um traço de osciloscópio** com gradiente de fluxo, com
  *stop* médio para manter a luminância: teal `#35c9c1`, amarelo `#e0c94f`,
  violeta `#a55cff`.
- **Três pássaros (dips) com ápices alinhados**: os dips dos lados laterais
  têm profundidade padrão; o do lado superior (o lado do violeta) desce mais
  fundo, e os três ápices formam uma **fileira horizontal** — a linha do
  horizonte.
- **Linha tracejada do horizonte** passa exatamente por essa fileira, dentro
  e fora do emblema (no banner, ela continua pelos dois lados). Os três
  pontos pousam na linha; o traço espectral do banner corre sobre ela.
- **Nó principal violeta** no centro da fileira (halo maior) — o "nó do
  verbo", como o nó na base do V do logo original. Fileira: teal · violeta ·
  amarelo.
- **Vazamento interno**: faixa borrada junto a cada lado + brilho nos dips,
  sempre recortados pelo clip do triângulo; o centro sob a linha permanece
  escuro — o triângulo vaza para dentro, nunca se fecha.
- **Simetria bilateral** no eixo vertical (como o V original). Pulsos
  menores nos segmentos retos preservam a identidade de medição (sinal +
  ruído, passarinhos distantes).
- Nós no idioma da marca: halo na cor local (opacidade 0.22–0.28) + miolo
  claro `#eaf4ff`; fundo radial `#1d2733→#11161d` com luz vinda de cima.

## Arquivos

| Arquivo | Papel |
|---|---|
| `../web/verbolog.svg` | Ícone 64×64 oficial (logo e favicon da UI) |
| `../docs/verbolog-triangle.svg` | Emblema 512×512 oficial (marca mestre) |
| `../docs/verbolog-banner.svg` | Banner 880×200 oficial (emblema + wordmark + traço espectral no horizonte) |
| `preview-*.png` | Renderizações de verificação |

Aprovada em agosto/2026 — estes são os arquivos oficiais consumidos pelo
README e pela UI. As marcas anteriores (V com gradiente azul→verde) vivem no
histórico do git.

## Regenerar e validar

```bash
python3 design/finalize.py   # gera os 3 SVGs a partir dos parâmetros
python3 design/validate.py   # renderiza com inkscape e roda sondas de pixel
```

Todos os parâmetros de desenho estão nos dicionários `EMB` e `ICO` em
`gen.py` (raio, largura/profundidade dos dips — inclusive `w_main`/`depth`
do pássaro central, alinhamento via `row_y()`, pulsos, wash, vazamentos,
nós, linha do horizonte) e a paleta no topo do arquivo. O banner é composto
em `banner.py`, ancorado na altura da fileira (`gen.row_y`).
