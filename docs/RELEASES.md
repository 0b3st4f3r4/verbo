# RELEASES.md — Processo e calendário de releases do VerboLang

> **6 meses de pesquisa e desenvolvimento + 6 anos de suporte + 6 meses de
> descontinuação = 7 anos de ciclo de vida.**

Uma **linha** de release nasce em Janeiro (o stable `vYYYY.0.0`) e se despede
em Junho do sétimo ano. Os seis meses anteriores (Jul–Dez) são a fase de
pesquisa da linha: alphas, betas e o release candidate — todos carregam o
nome da linha que está por nascer (`vYYYY.0.0-*`).

## Gramática de versão (cargo/SemVer)

A tag git é sempre `v` + a **versão cargo exata** — `MAJOR.MINOR.PATCH`, os
três componentes numéricos que o `[workspace.package]` do `core/` carrega e
que o gate do `.github/workflows/publish.yml` compara com a tag. O nome da
linha é o `MAJOR`: a linha `2027.0.0` é nomeada pelo ano em que o stable sai,
em Janeiro. A fase de um pré-lançamento vive no identificador SemVer
(`alpha.N`, `beta.N`, `rc`) e a precedência é a do calendário:
`alpha.0 < alpha.1 < … < beta.1 < … < rc <` o stable.

| Tag | Significado |
|---|---|
| `vYYYY.N.0` | minor `N` da linha `YYYY` |
| `vYYYY.0.0-alpha.N`, `vYYYY.0.0-beta.N`, `vYYYY.0.0-rc` | pré-lançamentos da linha que nasce em Janeiro de `YYYY` |
| `vYYYY.10.0` — a `.final` | a última minor da linha, a 11ª (Janeiro do 6º ano de suporte); apelido narrativo `.final`, número `10` na tag |
| `vYYYY.N.Z` | hotfix: patch de defeito/segurança sobre uma minor sob suporte — zero features |
| `vYYYY.N.Z` na despedida | o último patch de cada minor, na descontinuação — a linha inteira (ver abaixo); o último de todos é a despedida da minor `10` |

Tags git: sempre anotadas (`git tag -a v… -m "…"`) e precedidas da bateria
`make release-check` + entrada no `CHANGELOG.md`. A fase da release faz parte
do nome — quem lê a tag sabe o que ela promete.

## Fases e calendário

### Alpha — pesquisa, experimentação e definição de escopo

| Release | Janela |
|---|---|
| `vYYYY.0.0-alpha.0` | Julho + Agosto |
| `vYYYY.0.0-alpha.1` | Setembro |
| `vYYYY.0.0-alpha.2` | Setembro |

### Beta & RC — otimizações, refinamentos e correções de bugs

| Release | Janela |
|---|---|
| `vYYYY.0.0-beta.1` | Outubro |
| `vYYYY.0.0-beta.2` | Novembro |
| `vYYYY.0.0-rc` | Dezembro |

### Stable — 6 anos de suporte

| Anos de suporte | Fase | Releases |
|---|---|---|
| 1º ano | trimestral | `vYYYY.0.0` (Jan) · `vYYYY.1.0` (Abr) · `vYYYY.2.0` (Jul) · `vYYYY.3.0` (Out) |
| 2º e 3º ano | semestral | `vYYYY.4.0` (Jan) · `vYYYY.5.0` (Jul) · `vYYYY.6.0` (Jan) · `vYYYY.7.0` (Jul) |
| 4º, 5º e 6º ano | anual | `vYYYY.8.0` (Jan) · `vYYYY.9.0` (Jan) · `vYYYY.10.0` (Jan) — a `.final` |

### Descontinuação — os últimos 6 meses (Dez do 6º ano → Jun do 7º)

A linha para de receber features e caminha para o adeus **inteira**: a série
de despedida percorre TODAS as minors da linha, uma por data, na ordem em que
nasceram — da minor `0` até a `10` (a `.final`; por isso são 11 encontros: a
linha tem 11 minors). Cada encontro publica o **último patch** da minor
correspondente (`vYYYY.N.Z`, com `Z` = próximo patch daquela minor); a
despedida da minor final é a última tag da linha, que a encerra:

| Quando | Despedida |
|---|---|
| Dez do 6º ano | último patch de `vYYYY.0.0` — o primeiro passo volta à cena |
| Janeiro | últimos patches de `vYYYY.1.0`, `vYYYY.2.0`, `vYYYY.3.0` e `vYYYY.4.0` |
| Abril | últimos patches de `vYYYY.5.0`, `vYYYY.6.0` e `vYYYY.7.0` |
| Maio | últimos patches de `vYYYY.8.0` e `vYYYY.9.0` |
| Junho | último patch de `vYYYY.10.0` — a última tag da linha encerra a `.final` |

## Exemplo completo — a linha `v2027.0`

| Tag | Quando | Fase |
|---|---|---|
| `v2027.0.0-alpha.0` | Jul + Ago 2026 | alpha — pesquisa e escopo |
| `v2027.0.0-alpha.1` | Set 2026 | alpha |
| `v2027.0.0-alpha.2` | Set 2026 | alpha |
| `v2027.0.0-beta.1` | Out 2026 | beta — refinamento |
| `v2027.0.0-beta.2` | Nov 2026 | beta |
| `v2027.0.0-rc` | Dez 2026 | release candidate |
| `v2027.0.0` | Jan 2027 | **stable** — 1º ano, trimestral |
| `v2027.1.0` | Abr 2027 | 1º ano |
| `v2027.2.0` | Jul 2027 | 1º ano |
| `v2027.3.0` | Out 2027 | 1º ano |
| `v2027.4.0` | Jan 2028 | 2º ano, semestral |
| `v2027.5.0` | Jul 2028 | 2º ano |
| `v2027.6.0` | Jan 2029 | 3º ano, semestral |
| `v2027.7.0` | Jul 2029 | 3º ano |
| `v2027.8.0` | Jan 2030 | 4º ano, anual |
| `v2027.9.0` | Jan 2031 | 5º ano, anual |
| `v2027.10.0` | Jan 2032 | 6º ano, anual — última minor (`.final`) |
| `v2027.0.1` | Dez 2032 | descontinuação — despedida da minor `0` |
| `v2027.1.1`–`v2027.4.1` | Jan 2033 | descontinuação — despedida das minors `1`–`4` |
| `v2027.5.1`–`v2027.7.1` | Abr 2033 | descontinuação — despedida das minors `5`–`7` |
| `v2027.8.1`–`v2027.9.1` | Mai 2033 | descontinuação — despedida das minors `8`–`9` |
| `v2027.10.1` | Jun 2033 | **fim da linha** — a despedida da `.final` fecha 7 anos de ciclo |

(no exemplo, `Z = .1` por assumir que nenhuma minor recebeu hotfix antes;
cada despedida usa o próximo patch real daquela minor)

## Política de hotfixes

- `vYYYY.N.Z` corrige defeito ou vulnerabilidade sobre a minor `vYYYY.N.0`
  enquanto ela estiver sob suporte; **nunca** muda comportamento ou adiciona
  feature — quem quiser o novo, espera a próxima minor do calendário.
- O patch incrementa sempre a partir da minor; hotfix de hotfix é só `Z+1`.
- Na fase de descontinuação, **todas** as minors da linha recebem um último
  patch (a despedida percorre a linha inteira, da minor `0` à `10`, no
  calendário da seção anterior). Depois da despedida da minor `10` — a
  última tag da linha —, nada mais é publicado — o calendário já avisou por
  7 anos.

## Cortando uma release (checklist)

1. `make release-check` — bateria completa: `check` (shell + JS da UI),
   testes unitários do web (`node --test`), portaria do livro do site
   (`make site-check`) e `smoke` (endpoints com o servidor no ar). Para
   minors estáveis, rodar também a suíte do núcleo (`make rust-check`).
2. `CHANGELOG.md` — mover o "Não lançado" para a nova tag com data.
3. `git tag -a vYYYY.N.0 -m "vYYYY.N.0 — <fase>: o que esta release promete"`
   (pré-lançamento carrega o identificador: `v2027.0.0-alpha.1`).
4. `git push origin vYYYY.N.0` — o CI (`.github/workflows/release.yml`) repete
   a portaria (check + TDD + BDD + clippy/testes/E2E do núcleo), compila o
   `vbl` em modo release, empacota binário + dashboard + documentos + livro
   didático compilado (site/) + exemplos + scripts, carimba SHA-256 e anexa
   à release da tag (`-alpha/-beta/-rc` saem como pré-lançamento). Em
   paralelo, o `.github/workflows/publish.yml` publica os módulos Rust no
   crates.io (seção abaixo) e o `.github/workflows/site.yml` republica o
   livro em verbolang.org/docs — a tag `vX.Y.Z` deve confabular com a
   `version` do `[workspace.package]` (`core/Cargo.toml`), única fonte de
   versão.
5. Acompanhar os dois workflows até o verde; o publish é idempotente — se
   falhar no meio da fila, re-execute o job (os crates já publicados são
   pulados).
6. Anunciar apontando a entrada do changelog — a fase da tag fala por si.

## crates.io — publicação dos módulos Rust

O núcleo (`core/`) publica quatro crates: `vbl-lang`, `vbl-runtime`,
`vbl-fxp` e `vbl-cli`. A versão vive **só** em `[workspace.package]`
(`core/Cargo.toml`) — todos os crates saem em lockstep; dependências internas
carregam `version` + `path` no `[workspace.dependencies]` (o `path` manda no
workspace, o `version` entra no manifesto publicado).

**Estreia pública em `2027.0.0-alpha.0`** — o primeiro pré-lançamento da
linha, já na gramática `vYYYY.0.0-alpha.N` do calendário (as versões
internas `0.y.z` anteriores nunca foram lançadas).
Duas consequências: as tags da fase alpha são `v2027.0.0-alpha.N` (o gate do
workflow compara a tag, sem o `v`, com a versão do workspace); e o cargo
**não** resolve pré-release por padrão — quem depender dos crates na fase
alpha precisa fixar `= "2027.0.0-alpha.N"` (o req `^2027.0.0-alpha.0` das deps
internas casa os pré-lançamentos seguintes do mesmo trio).

**Ordem de dependência** (o workflow respeita, esperando a indexação de cada
versão antes do dependente): `vbl-lang → vbl-runtime → vbl-fxp → vbl-cli`.

**Gatilhos** do `.github/workflows/publish.yml`:

- push de tag `v*` — portaria (clippy + testes + `cargo package`)
  e publicação real;
- manual (`workflow_dispatch`) — `dry_run: true` por padrão: roda tudo, não
  publica. Use para ensaiar a publicação antes da tag (o ensaio usa
  `cargo publish --workspace --dry-run`: por crate isolado, o cargo
  resolveria os irmãos no registry, que ainda não os tem).

**Requisitos (uma vez):**

1. Token de API em <https://crates.io/settings/tokens> (escopo
   `publish-new` + `publish-update`) registrado como secret
   `CARGO_REGISTRY_TOKEN` em <https://github.com/0b3st4f3r4/verbo/settings/secrets/actions>.
2. (Opcional, recomendado) No environment `crates-io`
   (settings/environments), adicione *required reviewers* — todo publish
   passa a exigir aprovação humana.

**Verificação local** (equivalente ao que o CI roda): `make rust-package`
(`cargo package --workspace --locked --allow-dirty` — o `--allow-dirty` é só
porque a árvore local pode estar suja; no CI o checkout é limpo e o comando
roda estrito). O `cargo package` nunca publica — empacota e compila cada
pacote (verify build) exatamente como o registry fará.

**Regras de ouro:**

- Tag nova para versão nova — tags não se reutilizam; o publish pula crate
  cuja versão já existe no registry.
- `rust-version = "1.87"` é o piso declarado (verificado pelo
  `clippy::incompatible_msrv` — API mais nova que o campo quebra o clippy);
  se uma feature nova exigir toolchain mais novo, suba o campo no
  `[workspace.package]` e registre no CHANGELOG.
- Pré-lançamentos (calendário `v2027.0.0-alpha.N` etc.) são versões internas
  — só publique crates em fases alpha/beta com a consciência de que
  **versão publicada é versão para sempre**.

## Onde estamos

Linha **`v2027.0`** (major `2027` no cargo), fase **alpha.0** — a tag
`v2027.0.0-alpha.0` (Jul + Ago 2026: pesquisa, experimentação e definição de
escopo) foi cortada em 31/08/2026. A próxima janela é a `alpha.1`
(Setembro). O site de documentação didática (verbolang.org/docs) acompanha a
linha: o workflow `site.yml` republica o livro a cada push relevante e a
cada tag.
