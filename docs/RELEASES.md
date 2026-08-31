# RELEASES.md — Processo e calendário de releases do VerboLang

> **6 meses de pesquisa e desenvolvimento + 6 anos de suporte + 6 meses de
> descontinuação = 7 anos de ciclo de vida.**

Uma **linha** de release nasce em Janeiro (o stable `vYYYY.0`) e se despede
em Junho do sétimo ano. Os seis meses anteriores (Jul–Dez) são a fase de
pesquisa da linha: alphas, betas e o release candidate — todos carregam o
nome da linha que está por nascer (`vYYYY.0-*`).

## Gramática de versão

| Forma | Significado |
|---|---|
| `vYYYY.N` | minor `N` da linha `YYYY` (a linha é nomeada pelo ano em que o stable `vYYYY.0` sai, em Janeiro) |
| `vYYYY.0-alphaN`, `vYYYY.0-betaN`, `vYYYY.0-rc` | pré-lançamentos da linha que nasce em Janeiro de `YYYY` |
| `vYYYY.final` | a última minor da linha (Janeiro do 6º ano de suporte) |
| `vYYYY.N.Z` | hotfix: patch de defeito/segurança sobre uma minor sob suporte — zero features |
| `vYYYY.N.Z` na despedida | o último patch de cada minor, na descontinuação — a linha inteira (ver abaixo) |
| `vYYYY.final.final` | a última tag da linha: a despedida da minor final, em Junho do 7º ano |

Tags git: sempre anotadas (`git tag -a v… -m "…"`) e precedidas da bateria
`make release-check` + entrada no `CHANGELOG.md`. A fase da release faz parte
do nome — quem lê a tag sabe o que ela promete.

## Fases e calendário

### Alpha — pesquisa, experimentação e definição de escopo

| Release | Janela |
|---|---|
| `vYYYY.0-alpha0` | Julho + Agosto |
| `vYYYY.0-alpha1` | Setembro |
| `vYYYY.0-alpha2` | Setembro |

### Beta & RC — otimizações, refinamentos e correções de bugs

| Release | Janela |
|---|---|
| `vYYYY.0-beta1` | Outubro |
| `vYYYY.0-beta2` | Novembro |
| `vYYYY.0-rc` | Dezembro |

### Stable — 6 anos de suporte

| Anos de suporte | Fase | Releases |
|---|---|---|
| 1º ano | trimestral | `vYYYY.0` (Jan) · `vYYYY.1` (Abr) · `vYYYY.2` (Jul) · `vYYYY.3` (Out) |
| 2º e 3º ano | semestral | `vYYYY.4` (Jan) · `vYYYY.5` (Jul) · `vYYYY.6` (Jan) · `vYYYY.7` (Jul) |
| 4º, 5º e 6º ano | anual | `vYYYY.8` (Jan) · `vYYYY.9` (Jan) · `vYYYY.final` (Jan) |

### Descontinuação — os últimos 6 meses (Dez do 6º ano → Jun do 7º)

A linha para de receber features e caminha para o adeus **inteira**: a série
de despedida percorre TODAS as minors da linha, uma por data, na ordem em que
nasceram — da `.0` até a `.final` (por isso são 11 encontros: a linha tem 11
minors). Cada encontro publica o **último patch** da minor correspondente
(`vYYYY.N.Z`, com `Z` = próximo patch daquela minor); a despedida da minor
final é a tag `vYYYY.final.final`, que encerra a linha:

| Quando | Despedida |
|---|---|
| Dez do 6º ano | último patch de `vYYYY.0` — o primeiro passo volta à cena |
| Janeiro | últimos patches de `vYYYY.1`, `vYYYY.2`, `vYYYY.3` e `vYYYY.4` |
| Abril | últimos patches de `vYYYY.5`, `vYYYY.6` e `vYYYY.7` |
| Maio | últimos patches de `vYYYY.8` e `vYYYY.9` |
| Junho | último patch de `vYYYY.final` — a tag `vYYYY.final.final` encerra a linha |

## Exemplo completo — a linha `v2027.0`

| Tag | Quando | Fase |
|---|---|---|
| `v2027.0-alpha0` | Jul + Ago 2026 | alpha — pesquisa e escopo |
| `v2027.0-alpha1` | Set 2026 | alpha |
| `v2027.0-alpha2` | Set 2026 | alpha |
| `v2027.0-beta1` | Out 2026 | beta — refinamento |
| `v2027.0-beta2` | Nov 2026 | beta |
| `v2027.0-rc` | Dez 2026 | release candidate |
| `v2027.0` | Jan 2027 | **stable** — 1º ano, trimestral |
| `v2027.1` | Abr 2027 | 1º ano |
| `v2027.2` | Jul 2027 | 1º ano |
| `v2027.3` | Out 2027 | 1º ano |
| `v2027.4` | Jan 2028 | 2º ano, semestral |
| `v2027.5` | Jul 2028 | 2º ano |
| `v2027.6` | Jan 2029 | 3º ano, semestral |
| `v2027.7` | Jul 2029 | 3º ano |
| `v2027.8` | Jan 2030 | 4º ano, anual |
| `v2027.9` | Jan 2031 | 5º ano, anual |
| `v2027.final` | Jan 2032 | 6º ano, anual — última minor |
| `v2027.0.1` | Dez 2032 | descontinuação — despedida da minor `.0` |
| `v2027.1.1`–`v2027.4.1` | Jan 2033 | descontinuação — despedida das minors `.1`–`.4` |
| `v2027.5.1`–`v2027.7.1` | Abr 2033 | descontinuação — despedida das minors `.5`–`.7` |
| `v2027.8.1`–`v2027.9.1` | Mai 2033 | descontinuação — despedida das minors `.8`–`.9` |
| `v2027.final.final` | Jun 2033 | **fim da linha** — 7 anos de ciclo |

(no exemplo, `Z = .1` por assumir que nenhuma minor recebeu hotfix antes;
cada despedida usa o próximo patch real daquela minor)

## Política de hotfixes

- `vYYYY.N.Z` corrige defeito ou vulnerabilidade sobre a minor `vYYYY.N`
  enquanto ela estiver sob suporte; **nunca** muda comportamento ou adiciona
  feature — quem quiser o novo, espera a próxima minor do calendário.
- O patch incrementa sempre a partir da minor; hotfix de hotfix é só `Z+1`.
- Na fase de descontinuação, **todas** as minors da linha recebem um último
  patch (a despedida percorre a linha inteira, do `.0` ao `.final`, no
  calendário da seção anterior). Depois da tag `vYYYY.final.final`, nada mais
  é publicado — o calendário já avisou por 7 anos.

## Cortando uma release (checklist)

1. `make release-check` — bateria completa: `check` (shell + JS da UI),
   testes unitários do web (`node --test`) e `smoke` (endpoints com o
   servidor no ar). Para minors estáveis, rodar também a suíte do núcleo
   (`make rust-check`).
2. `CHANGELOG.md` — mover o "Não lançado" para a nova tag com data.
3. `git tag -a vYYYY.N -m "vYYYY.N — <fase>: o que esta release promete"`.
4. `git push origin vYYYY.N` — o CI (`.github/workflows/release.yml`) repete
   a portaria (check + TDD + BDD + clippy/testes/E2E do núcleo), compila o
   `vbl` em modo release, empacota binário + dashboard + documentos +
   exemplos + scripts, carimba SHA-256 e anexa à release da tag
   (`-alpha/-beta/-rc` saem como pré-lançamento).
5. Anunciar apontando a entrada do changelog — a fase da tag fala por si.

## Onde estamos

Desenvolvimento **pré-alpha** (versões internas `v0.x` de conveniência, sem
compromisso de calendário). A primeira linha oficial é a **`v2027.0`**: a
base atual corresponde à janela `alpha0`/`alpha1` (pesquisa, experimentação
e definição de escopo) — Jul–Set do ano anterior ao stable.
