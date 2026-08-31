#!/usr/bin/env bash
# =============================================================================
# vsh — Verbo Shell
# CLI shell avançada para o projeto Verbo (convenções de scripts/ em AGENTS.md)
# Versão: 0.1.1
#
# POR QUÊ EXISTE:
#   O tool `bash` do harness executa cada comando em um shell NOVO — sem cwd,
#   variáveis ou histórico persistidos. O vsh adiciona, sobre o bash comum:
#
#     1. SESSÕES PERSISTENTES — cwd + variáveis exportadas + histórico são
#        salvos a cada comando e recarregados na próxima invocação:
#            vsh run 'cd core && export FOO=1'
#            vsh run 'pwd; echo "$FOO"'     # ⟵ estado mantido entre chamadas
#     2. REDE INTEGRADA — `search`, `fetch` e `net` (fallback funcional
#        enquanto a ferramenta web_search do harness está indisponível).
#     3. AUXILIARES — `json` (via jq), `digest` (sha256), `rt` (proxy do rtk),
#        `history` (histórico da sessão com timestamp).
#     4. GUARDA DE SEGURANÇA — padrões destrutivos (rm -rf /, dd of=/dev/*,
#        mkfs, fork bomb...) exigem confirmação no REPL ou
#        VSH_ALLOW_DANGEROUS=1 no modo run (best-effort).
#     5. REPL INTERATIVO — readline (setas/histórico), cores, banner, help.
#
# USO:
#   vsh                           # REPL interativo (sessão "default")
#   vsh -s NOME                   # REPL em outra sessão
#   vsh run 'COMANDO'             # 1 comando com estado persistido; sai com o exit code dele
#   vsh 'COMANDO'                 # atalho equivalente a `run`
#   vsh -s NOME run 'COMANDO'     # comando em sessão específica
#   vsh sessions                  # lista sessões salvas
#   vsh reset                     # apaga estado da sessão atual
#   vsh doctor                    # verifica dependências
#   vsh help | version
#
# BUILT-INS disponíveis dentro do run/REPL:
#   search [--json] [--n N] [--engine ddg|mojeek] TERMO...   busca web
#       (tenta DuckDuckGo e cai para Mojeek se houver bloqueio/rate-limit)
#   fetch [-o ARQUIVO] URL             baixa URL; metadados HTTP vão p/ stderr
#   net [URL]                          diagnósticos: código, DNS, connect, total
#   json 'CONSULTA_JQ' [ARQUIVO]       consulta JSON (stdin se sem arquivo)
#   digest ARQUIVO                     sha256sum
#   rt COMANDO...                      passa pelo rtk se instalado; senão executa
#   history [N]                        últimos N comandos da sessão
#   cd / export / unset                normais, e ficam PERSISTIDOS
#
# ESTADO (git-ignorado): ./.vsh/sessions/<nome>.env · ./.vsh/history/<nome>.log
# CUSTOMIZAÇÃO: VSH_STATE_DIR, VSH_SESSION, SEARCH_LIMIT (default 8),
#               VSH_ALLOW_DANGEROUS=1 (libera comandos bloqueados pela guarda)
#
# LIMITAÇÕES conhecidas (v0.1.1): linha única por comando (sem continuação
# "\"), heredocs no modo run, guard apenas heurística; no REPL prefira
# `export` a `declare -x` (a linha é avaliada dentro do loop da função).
# =============================================================================

VSH_VERSION="0.1.1"

# -----------------------------------------------------------------------------
# Localização do projeto e diretório de estado
# -----------------------------------------------------------------------------
SCRIPT_SOURCE="${BASH_SOURCE[0]:-$0}"
SCRIPT_DIR="$(cd -- "$(dirname -- "$SCRIPT_SOURCE")" && pwd)"
# git rev-parse falha fora de um repo — fallback: diretório pai do script
ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null)"
[ -n "$ROOT" ] || ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
STATE="${VSH_STATE_DIR:-$ROOT/.vsh}"
SESSIONS_DIR="$STATE/sessions"
HIST_DIR="$STATE/history"
TMP_DIR="$STATE/tmp"
UA="Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120 Safari/537.36"

# Cores só quando stdout é um terminal (agentes recebem saída limpa)
if [ -t 1 ]; then
  C_DIM=$'\033[2m'; C_CYN=$'\033[36m'; C_YEL=$'\033[33m'; C_RED=$'\033[31m'
  C_GRN=$'\033[32m'; C_B=$'\033[1m'; C_0=$'\033[0m'
else
  C_DIM=""; C_CYN=""; C_YEL=""; C_RED=""; C_GRN=""; C_B=""; C_0=""
fi

info() { printf '%s[vsh]%s %s\n' "$C_CYN" "$C_0" "$*" >&2; }
warn() { printf '%s[vsh] aviso:%s %s\n' "$C_YEL" "$C_0" "$*" >&2; }
die()  { printf '%s[vsh] erro:%s %s\n' "$C_RED" "$C_0" "$*" >&2; exit 2; }

# -----------------------------------------------------------------------------
# Sessão: carregar / salvar (cwd + variáveis exportadas)
# -----------------------------------------------------------------------------
# (session_load foi removido na v0.1.1: source em função tornava os declare -x
#  locais à função. A carga da sessão acontece no escopo top-level, no fim.)

session_save() {
  mkdir -p "$SESSIONS_DIR" "$HIST_DIR" "$TMP_DIR" 2>/dev/null
  local f="$SESSIONS_DIR/$SESSION.env"
  {
    printf 'cd %q\n' "$PWD"
    # Exportadas, menos ruído de runtime do harness/ciclos de SHLVL
    declare -px 2>/dev/null | grep -vE 'declare -x (SHLVL|OLDPWD|_)=' \
      | grep -v 'declare -x DSH_' || true
  } > "$f.tmp" 2>/dev/null && mv -f "$f.tmp" "$f"
}

history_log() {
  mkdir -p "$HIST_DIR" 2>/dev/null
  printf '%s :: %s\n' "$(date +%Y-%m-%dT%H:%M:%S%z)" "$1" >> "$HIST_DIR/$SESSION.log"
}

cmd_history() { tail -n "${1:-30}" "$HIST_DIR/$SESSION.log" 2>/dev/null | nl -ba; }
# sobrepõe o builtin `history` dentro do vsh para expor o histórico da sessão
history() { cmd_history "$@"; }

cmd_sessions() {
  local f s found=0
  for f in "$SESSIONS_DIR"/*.env; do
    [ -e "$f" ] || continue
    found=1
    s="$(basename "$f" .env)"
    if [ "$s" = "$SESSION" ]; then
      printf '%s* %s%s %s(cwd: %s)%s\n' "$C_GRN" "$C_B" "$s" "$C_DIM" "${PWD/#$HOME/~}" "$C_0"
    else
      printf '  %s\n' "$s"
    fi
  done
  [ "$found" = 1 ] || info "nenhuma sessão salva ainda (o estado surge após o 1º comando)"
}

cmd_reset() {
  rm -f "$SESSIONS_DIR/$SESSION.env" "$HIST_DIR/$SESSION.log"
  info "sessão '$SESSION' resetada"
}

# -----------------------------------------------------------------------------
# Guarda de segurança (best-effort, heurística)
# -----------------------------------------------------------------------------
command_is_dangerous() {
  local c="$1"
  printf '%s' "$c" | grep -qE \
    -e '(^|[;&|[:space:]])rm[[:space:]]+[^;|&]*-[a-zA-Z]*[rf][a-zA-Z]*[[:space:]]+[^;|&]*(-[a-zA-Z]*[rf][a-zA-Z]*[[:space:]]+)*(/|~|\$HOME|"/|\*)([[:space:]]|$)' \
    -e 'mkfs\.' \
    -e 'dd[[:space:]][^;]*of=/dev/' \
    -e ':\(\)[[:space:]]*\{.*\};:' \
    -e 'chmod[[:space:]]+(-R[[:space:]]+)?777[[:space:]]+/([[:space:]]|$)' \
    -e '>[[:space:]]*/dev/(sd|nvme|hd)'
}

guard() {
  local c="$1"
  if command_is_dangerous "$c"; then
    if [ "${VSH_ALLOW_DANGEROUS:-0}" = "1" ]; then
      warn "VSH_ALLOW_DANGEROUS=1 — executando comando potencialmente destrutivo"
      return 0
    fi
    if [ -t 0 ] && [ -t 1 ]; then
      warn "comando potencialmente destrutivo detectado:"
      printf '  %s\n' "$c" >&2
      local ans; printf '%s[vsh] confirmar? digite SIM para executar: %s' "$C_YEL" "$C_0" >&2
      read -r ans
      [ "$ans" = "SIM" ] && return 0
      warn "abortado pelo usuário"
      return 1
    fi
    warn "recusado (padrão destrutivo). Para forçar: VSH_ALLOW_DANGEROUS=1"
    return 1
  fi
  return 0
}

# -----------------------------------------------------------------------------
# Rede: search / fetch / net
# -----------------------------------------------------------------------------
_decode_url() {
  if command -v python3 >/dev/null 2>&1; then
    python3 -c 'import sys,urllib.parse; print(urllib.parse.unquote(sys.argv[1]))' "$1"
  else
    printf '%s' "$1" | sed -e 's/%3A/:/g' -e 's/%2F/\//g' -e 's/%3F/?/g' \
      -e 's/%3D/=/g' -e 's/%26/\&/g' -e 's/%25/%/g' -e 's/%2B/+/g' \
      -e 's/%23/#/g'
  fi
}

_clean_title() {
  printf '%s' "$1" | sed -e 's/&amp;/\&/g' -e 's/&quot;/"/g' -e 's/&#x27;/'"'"'/g' \
    -e 's/&#39;/'"'"'/g' -e 's/&lt;/</g' -e 's/&gt;/>/g'
}

search() {
  local n="${SEARCH_LIMIT:-8}" as_json=0 engine="" engine_list="ddg,mojeek"
  while [ $# -gt 0 ]; do
    case "$1" in
      --json) as_json=1; shift ;;
      --n) n="$2"; shift 2 ;;
      --engine) engine="$2"; engine_list="$2"; shift 2 ;;
      -h|--help) printf 'uso: search [--json] [--n N] [--engine ddg|mojeek] TERMO...\n'; return 0 ;;
      *) break ;;
    esac
  done
  [ $# -gt 0 ] || { printf 'uso: search [--json] [--n N] [--engine ddg|mojeek] TERMO...\n' >&2; return 2; }

  local html urls titles pairs
  pairs="$TMP_DIR/search.pairs"
  mkdir -p "$TMP_DIR" 2>/dev/null
  urls=""; titles=""

  # Motor 1: DuckDuckGo HTML (pode responder 403 sob rate-limit)
  if [ "$engine" != "mojeek" ]; then
    html="$(curl -sS --max-time 20 -G --data-urlencode "q=$*" \
            -H "User-Agent: $UA" "https://html.duckduckgo.com/html/" 2>/dev/null)" || html=""
    if [ -n "$html" ]; then
      urls="$(grep -oE 'class="result__a"[^>]*href="[^"]+"' <<<"$html" \
              | sed -E 's/.*href="([^"]+)".*/\1/' | head -n "$n" \
              | while read -r u; do _decode_url "$u"; done)"
      titles="$(grep -oE '<a[^>]*class="result__a"[^>]*>[^<]+</a>' <<<"$html" \
                | sed -E 's/<[^>]+>//g' | head -n "$n")"
    fi
  fi

  # Motor 2 (fallback): Mojeek — HTML estável, raramente bloqueia
  if { [ -z "$urls" ] || [ -z "$titles" ]; } && [ "$engine" != "ddg" ]; then
    html="$(curl -sS --max-time 20 -G --data-urlencode "q=$*" \
            -H "User-Agent: $UA" "https://www.mojeek.com/search" 2>/dev/null)" || html=""
    if [ -n "$html" ]; then
      urls="$(grep -oE '<a class="title"[^>]*href="[^"]+"' <<<"$html" \
              | sed -E 's/.*href="([^"]+)".*/\1/' | head -n "$n")"
      titles="$(grep -oE '<h2><a class="title"[^>]*>[^<]+</a>' <<<"$html" \
                | sed -E 's/<[^>]*>//g' | head -n "$n")"
    fi
  fi

  if [ -z "$urls" ] || [ -z "$titles" ]; then
    warn "sem resultados para: $* (motores tentados: $engine_list)"; return 1
  fi
  paste <(printf '%s\n' "$urls") <(printf '%s\n' "$titles") > "$pairs" 2>/dev/null

  if [ "$as_json" = 1 ]; then
    if command -v jq >/dev/null 2>&1; then
      jq -Rn 'reduce inputs as $l ([]; . + [($l | split("\t")) | {titulo: .[1], url: .[0]}])' < "$pairs"
    else
      warn "--json requer jq; exibindo texto"; nl -ba < "$pairs"
    fi
  else
    local i=0 title url
    while IFS=$'\t' read -r url title; do
      i=$((i+1))
      printf '%s%2d.%s %s\n%s     %s%s\n' "$C_B" "$i" "$C_0" "$(_clean_title "$title")" "$C_DIM" "$url" "$C_0"
    done < "$pairs"
  fi
}

fetch() {
  local out="" url=""
  while [ $# -gt 0 ]; do
    case "$1" in
      -o) out="$2"; shift 2 ;;
      -h|--help) printf 'uso: fetch [-o ARQUIVO] URL\n'; return 0 ;;
      *) url="$1"; shift ;;
    esac
  done
  [ -n "$url" ] || { printf 'uso: fetch [-o ARQUIVO] URL\n' >&2; return 2; }
  mkdir -p "$TMP_DIR" 2>/dev/null
  local tmp rc meta
  tmp="$TMP_DIR/fetch.$$"
  meta="$(curl -sSL --max-time 60 -H "User-Agent: $UA" -o "$tmp" \
          -w '%{http_code} | %{time_total}s | %{size_download}B' "$url")"
  rc=$?
  info "HTTP $meta"
  if [ "$rc" -eq 0 ]; then
    if [ -n "$out" ]; then mv -f "$tmp" "$out"; info "salvo em: $out"; else cat "$tmp"; rm -f "$tmp"; fi
  fi
  return "$rc"
}

net() {
  local url="${1:-https://example.com}"
  mkdir -p "$TMP_DIR" 2>/dev/null
  local sink="$TMP_DIR/net.$$"
  curl -sS --max-time 15 -o "$sink" -w 'HTTP %{http_code} | dns %{time_namelookup}s | conectar %{time_connect}s | tls %{time_appconnect}s | total %{time_total}s | %{size_download}B\n' "$url"
  local rc=$?
  rm -f "$sink"
  return "$rc"
}

# -----------------------------------------------------------------------------
# Auxiliares: json / digest / rt
# -----------------------------------------------------------------------------
json() {
  command -v jq >/dev/null 2>&1 || { warn "'json' requer jq (não instalado)"; return 3; }
  local q="${1:-.}" f="${2:--}"
  if [ "$f" = "-" ]; then jq "$q"; else jq "$q" "$f"; fi
}

digest() {
  [ -n "${1:-}" ] || { printf 'uso: digest ARQUIVO\n' >&2; return 2; }
  sha256sum "$1"
}

rt() {
  if command -v rtk >/dev/null 2>&1; then
    rtk "$@"
  elif [ $# -gt 0 ]; then
    warn "rtk indisponível; executando direto: $*"; "$@"
  else
    warn "rtk indisponível"; return 3
  fi
}

# -----------------------------------------------------------------------------
# NOTA DE ARQUITETURA: a execução do comando e o `source` da sessão vivem no
# ESCOPO TOP-LEVEL do script (final do arquivo), não em funções. Motivo: o
# arquivo de sessão usa `declare -x`, e `declare` executado dentro de uma
# FUNÇÃO cria variáveis LOCAIS — perdidas no `return` (bug v0.1.0).
# -----------------------------------------------------------------------------

# -----------------------------------------------------------------------------
# REPL interativo
# -----------------------------------------------------------------------------
cmd_help() {
  cat <<EOF
${C_B}vsh ${VSH_VERSION}${C_0} — Verbo Shell

${C_B}Uso:${C_0}
  vsh [-s SESSÃO]                REPL interativo
  vsh [-s SESSÃO] run 'CMD'      comando com estado persistido (cwd/env)
  vsh 'CMD'                      atalho para run
  vsh sessions | reset | doctor | help | version

${C_B}Built-ins:${C_0} search [--json] [--n N] TERMO · fetch [-o FILE] URL ·
  net [URL] · json 'JQ' [FILE] · digest FILE · rt CMD · history [N]
  (cd/export/unset funcionam normalmente e ficam persistidos)

${C_B}Estado:${C_0} ${STATE#$ROOT/}/sessions/<sessão>.env · history/<sessão>.log
${C_B}Env:${C_0} VSH_STATE_DIR · VSH_SESSION · SEARCH_LIMIT · VSH_ALLOW_DANGEROUS
EOF
}

cmd_doctor() {
  local ok=1
  check() { # check DESC COMANDO [obrigatório]
    local desc="$1" cmd="$2" req="${3:-1}"
    if command -v "$cmd" >/dev/null 2>&1; then
      printf '  %s ✓ %s%s %s(%s)%s\n' "$C_GRN" "$C_0" "$desc" "$C_DIM" "$cmd" "$C_0"
    else
      if [ "$req" = 1 ]; then printf '  %s ✗ %s%s %s(faltando: %s)%s\n' "$C_RED" "$C_0" "$desc" "$C_DIM" "$cmd" "$C_0"; ok=0
      else printf '  %s ~ %s%s %s(opcional)%s\n' "$C_YEL" "$C_0" "$desc" "$C_DIM" "$C_0"; fi
    fi
  }
  printf '%s[vsh doctor] sessão: %s · estado: %s%s\n' "$C_CYN" "$SESSION" "$STATE" "$C_0"
  check "bash (runtime)"     bash 1
  check "curl (rede)"        curl 1
  check "git (raiz do repo)" git 1
  check "jq (json/search --json)" jq 0
  check "python3 (decode URLs)" python3 0
  check "rtk (filtros de saída)" rtk 0
  mkdir -p "$STATE" 2>/dev/null
  if [ -w "$STATE" ]; then
    printf '  %s ✓ %sestado gravável %s(%s)%s\n' "$C_GRN" "$C_0" "$C_DIM" "$STATE" "$C_0"
  else
    printf '  %s ✗ %sestado NÃO gravável %s(%s — defina VSH_STATE_DIR)%s\n' "$C_RED" "$C_0" "$C_DIM" "$STATE" "$C_0"; ok=0
  fi
  if net https://example.com >/dev/null 2>&1; then
    printf '  %s ✓ %srede alcançável%s\n' "$C_GRN" "$C_0" "$C_0"
  else
    printf '  %s ✗ %srede inacessível (search/fetch/net falharão)%s\n' "$C_RED" "$C_0" "$C_0"
  fi
  [ "$ok" = 1 ] && info "tudo essencial OK" || die "dependências essenciais faltando"
}

repl() {
  printf '%s╭──────────────────────────────────────╮%s\n' "$C_CYN" "$C_0"
  printf '%s│ %svsh %s — Verbo Shell %s(sessão: %s)%s │%s\n' "$C_CYN" "$C_B" "$VSH_VERSION" "$C_DIM" "$SESSION" "$C_0" "$C_0"
  printf '%s╰──────────────────────────────────────╯%s\n' "$C_CYN" "$C_0"
  printf '%sdigite "help" para os comandos; "exit" para sair%s\n\n' "$C_DIM" "$C_0"
  local line rc ps1
  while :; do
    ps1="$(printf '%s\001\033[36m\002vsh:%s\001\033[0m\002 \001\033[33m\002❯\001\033[0m\002 ' "$SESSION" "${PWD/#$HOME/~}")"
    if ! IFS= read -r -e -p "$ps1" line; then printf '\n'; break; fi
    line="${line%$'\r'}"
    [ -z "$line" ] && continue
    case "$line" in
      exit|quit|q) break ;;
      help|h|\?)   cmd_help; continue ;;
      sessions)    cmd_sessions; continue ;;
      reset)       cmd_reset; continue ;;
      doctor)      cmd_doctor; continue ;;
      clear|cls)   clear; continue ;;
      version)     printf 'vsh %s\n' "$VSH_VERSION"; continue ;;
    esac
    if guard "$line"; then
      set +e
      eval "$line"
      rc=$?
      history_log "$line"
      session_save
    fi
  done
  info "sessão '$SESSION' salva em $STATE"
}

# -----------------------------------------------------------------------------
# Fluxo principal — TOP-LEVEL de propósito (ver NOTA DE ARQUITETURA acima)
# -----------------------------------------------------------------------------
SESSION="${VSH_SESSION:-default}"
MODE=""
LINE=""

parse_args() {
  local opt sub
  while getopts ":s:h" opt "$@"; do
    case "$opt" in
      s) SESSION="$OPTARG" ;;
      h) MODE="help"; return 0 ;;
      \?) die "opção inválida: -$OPTARG (veja: vsh help)" ;;
    esac
  done
  shift $((OPTIND - 1))
  printf '%s' "$SESSION" | grep -qE '^[A-Za-z0-9_.-]+$' \
    || die "nome de sessão inválido: '$SESSION' (use [A-Za-z0-9_.-])"
  sub="${1:-}"
  case "$sub" in
    "")       MODE="repl" ;;
    run)      shift; LINE="$*" ;;
    sessions) MODE="sessions" ;;
    reset)    MODE="reset" ;;
    doctor)   MODE="doctor" ;;
    help)     MODE="help" ;;
    version)  MODE="version" ;;
    *)        MODE="run"; LINE="$*" ;;  # atalho: vsh 'COMANDO'
  esac
}

dispatch_simple() {
  case "$1" in
    help)     cmd_help ;;
    version)  printf 'vsh %s\n' "$VSH_VERSION" ;;
    sessions) cmd_sessions ;;
    reset)    cmd_reset ;;
    doctor)   cmd_doctor ;;
  esac
}

parse_args "$@"

case "$MODE" in
  help|version|sessions|reset|doctor)
    dispatch_simple "$MODE"
    ;;
  *)
    # Carrega a sessão no escopo GLOBAL — em função, os `declare -x` do
    # arquivo virariam variáveis locais e se perderiam no return (bug v0.1.0)
    if [ -f "$SESSIONS_DIR/$SESSION.env" ]; then
      # shellcheck disable=SC1090
      source "$SESSIONS_DIR/$SESSION.env" >/dev/null 2>&1 || true
    fi
    if [ "$MODE" = "repl" ]; then
      repl
    else
      [ -n "$LINE" ] || die "uso: vsh run 'COMANDO'"
      rc=0
      if guard "$LINE"; then
        set +e
        eval "$LINE"
        rc=$?
        history_log "$LINE"
        session_save
        info "sessão '$SESSION' | exit=$rc | cwd=${PWD/#$HOME/~}"
        exit "$rc"
      fi
      exit 1
    fi
    ;;
esac
