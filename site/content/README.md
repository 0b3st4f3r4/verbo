<div class="vbl-hero">
<img src="../../docs/brand/verbolog-triangle.svg" alt="Emblema da VerboLang — triângulo invertido de metal escovado sobre vidro fosco">
<h1>VerboLang</h1>
<p><em>a linguagem onde nenhum dado é inerte</em></p>
<p><span class="vbl-versao">v2027.0.0-alpha.0</span></p>
</div>

A VerboLang é uma linguagem de programação de baixo nível alinhada ao
**Materialismo Computacional**: toda estrutura lógica é uma **forma** com
suporte físico concreto, horizonte de validade e custo energético explícito.
A integridade de um sistema não se mede em selos de conformidade — mede-se em
**Joules, Celsius e ciclos de CPU**, registrados de forma auditável no
**Caderno**.

Este livro é a porta de entrada. Ele é gerado **a partir dos mesmos arquivos
do repositório** — trilha didática, especificação formal, cheat sheets e
documentos de projeto — então o que você lê aqui é exatamente o que governa o
código.

## Por onde começar

<div class="vbl-cards">
<ul>
<li><span class="vbl-num">01</span><a href="guide/visao-geral.md">O que é a VerboLang</a> — formas, horizontes e a física por trás da sintaxe</li>
<li><span class="vbl-num">02</span><a href="guide/instalacao.md">Instalação e primeiro run</a> — do clone ao primeiro Joule no Caderno</li>
<li><span class="vbl-num">03</span><a href="guide/formas.md">As três conjugações</a> — <code>event</code>, <code>equilibrium</code>, <code>nonequilibrium</code></li>
<li><span class="vbl-num">04</span><a href="guide/revisoes.md">Reviews</a> — <code>when</code>, <code>subvert</code>, <code>keep()</code> e atuação</li>
<li><span class="vbl-num">05</span><a href="guide/fxp.md">FXP</a> — sensores e atores por nome simbólico</li>
<li><span class="vbl-num">06</span><a href="guide/caderno.md">O Caderno</a> — a contabilidade termodinâmica à prova de adulteração</li>
<li><span class="vbl-num">07</span><a href="guide/receitas.md">Receitas</a> — padrões prontos e anti-padrões que o AD rejeita</li>
</ul>
</div>

## Uma amostra em 9 linhas

```verbolang
nonequilibrium SpeculativeTrading {
    value: "lucro_arbitragem_alta_frequencia",
    horizon: 7s,
    source_path: "cpu_temp",
    maintenance_deadline: 2s,
    exchange_mode: "extraction"
}

review SpeculativeTrading {
    when cpu_temp > 85°C -> subvert,
                             act(CpuPowerCap, 50)
}
```

Quando a CPU passa de 85 °C, a forma é subvertida: seu valor vira o poético
canônico, ela se dissolve no mesmo tick e o runtime limita a potência da CPU
via FXP — tudo registrado no Caderno. O capítulo
[Instalação e primeiro run](guide/instalacao.md) roda este programa em menos
de um minuto.

## O livro inteiro

- **Trilha** — sete capítulos didáticos, em ordem; é o caminho recomendado.
- **Referência** — a [especificação formal](../../docs/FORMAL.md), o
  [manifesto](../../docs/MANIFESTO.md), os
  [cheat sheets](../../docs/cheatsheet/VBL-CHEATSHEET.md), o schema do FXP
  e o formato binário do Caderno.
- **Projeto** — o [README](../../README.md), o
  [plano de execução](../../docs/PLAN.md), o
  [processo de releases](../../docs/RELEASES.md) e o
  [changelog](../../CHANGELOG.md).

> [!TIP]
> Use <kbd>/</kbd> para buscar em todo o livro. A versão de impressão
> (botão da barra superior) gera o livro inteiro em uma página — boa para
> PDF. O cromo da página fala as 7 línguas da família (pt, en, zh, ru, hi,
> af, ar); o conteúdo canônico dos capítulos é em português.

O código-fonte deste livro é o próprio repositório:
[github.com/0b3st4f3r4/verbo](https://github.com/0b3st4f3r4/verbo) — o link
de lápis em cada página abre a edição do `.md` correspondente. Licença
GPL-3.0: **o comum exige `keep()`**.
