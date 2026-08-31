# -*- coding: utf-8 -*-
"""vlcheck — validador de SUPERFÍCIE de programas `.vl` (Etapa 1).

Papel (PLAN §7): verificador sintático "mini-validador dedicado" para o
banco de 20 prompts — o protótipo Python não parseia texto e o parser real
só existe na Etapa 2. Este módulo NÃO substitui o parser da Etapa 2: valida
estrutura léxica/sintática declarativa e as cláusulas de erro textuais da
FORMAL §3 (value/horizon obrigatórios e ordenados, review órfã/duplicada,
keep de forma inexistente, atributos por conjugação, durações, strings).

Limites assumidos (documentados): sem tabela de símbolos de expressões,
sem validação de registros FXP (isso é papel do `contract.py` no IR) e sem
semântica de execução.
"""

from __future__ import annotations

import re
from typing import NamedTuple

CONJUGACOES = {"event", "equilibrium", "nonequilibrium"}
ATRIBUTOS = {"value", "horizon", "source_path", "maintenance_deadline",
             "exchange_mode", "cost_bytes", "currency", "classification"}
SO_EM = {"maintenance_deadline": {"nonequilibrium"},
         "exchange_mode": {"nonequilibrium"},
         "cost_bytes": {"equilibrium"}}
UNIDADES_TEMPO = {"s", "ms", "us", "ns"}
UNIDADES_FISICAS = {"W", "°C", "%"}
OPERADORES = {"<", ">", "<=", ">=", "==", "!="}
ACOES = {"dissolve", "subvert", "reclassify_as_equilibrium",
         "reclassify_as_nonequilibrium", "notify_shutdown", "act"}
LIMITE_STRING = 256  # bytes (FORMAL §2)

_TOKEN = re.compile(r"""
    (?P<string>"(?:\\.|[^"\\])*")
  | (?P<num>\d+\.\d+|\d+)
  | (?P<fisica>°C|%)
  | (?P<id>[a-zA-Z_][a-zA-Z0-9_]*)
  | (?P<op>->|<=|>=|==|!=|<|>)
  | (?P<punc>[:,;{}()])
  | (?P<lixo>\S)
""", re.VERBOSE)


class Erro(NamedTuple):
    codigo: str
    linha: int
    mensagem: str


# ----------------------------------------------------------------------
# Pré-processamento e tokenização
# ----------------------------------------------------------------------
def _sem_comentarios(texto: str) -> str:
    """Remove // e /* */ preservando quebras de linha (para numeração)."""
    saida = []
    i, n = 0, len(texto)
    while i < n:
        c = texto[i]
        if c == "/" and i + 1 < n and texto[i + 1] == "/":
            while i < n and texto[i] != "\n":
                saida.append(" ")
                i += 1
        elif c == "/" and i + 1 < n and texto[i + 1] == "*":
            fim = texto.find("*/", i + 2)
            fim = n if fim == -1 else fim + 2
            for ch in texto[i:fim]:
                saida.append(ch if ch == "\n" else " ")
            i = fim
        else:
            saida.append(c)
            i += 1
    return "".join(saida)


def _tokenizar(texto: str, erros: list[Erro]) -> list[tuple[str, str, int]]:
    tokens: list[tuple[str, str, int]] = []
    linha = 1
    pos = 0
    for m in _TOKEN.finditer(texto):
        linha += texto.count("\n", pos, m.start())
        pos = m.start()
        tipo = m.lastgroup
        if tipo == "lixo":
            # léxico estrito: caractere fora da linguagem vira erro (ex.: '=')
            erros.append(Erro("lexema_invalido", linha,
                              f"lexema {m.group()!r} não existe na linguagem"))
            continue
        tokens.append((tipo, m.group(), linha))
    return tokens


# ----------------------------------------------------------------------
# Parser de superfície
# ----------------------------------------------------------------------
class _Parser:
    def __init__(self, tokens):
        self.toks = tokens
        self.i = 0
        self.erros: list[Erro] = []
        self.formas: dict[str, dict] = {}
        self.reviews: list[str] = []

    # -- utilidades -----------------------------------------------------
    def peek(self):
        return self.toks[self.i] if self.i < len(self.toks) else None

    def proximo(self):
        tok = self.peek()
        if tok is not None:
            self.i += 1
        return tok

    def espera(self, valor: str, codigo: str, msg: str):
        tok = self.proximo()
        if tok is None:
            self.erros.append(Erro(codigo, self.toks[-1][2] if self.toks else 1,
                                   f"fim inesperado: {msg}"))
            return None
        if tok[1] != valor:
            self.erros.append(Erro(codigo, tok[2], f"{msg} (encontrado {tok[1]!r})"))
            return None
        return tok

    def e_ident(self, tok, valor: str | None = None) -> bool:
        return tok is not None and tok[0] == "id" and (valor is None or tok[1] == valor)

    def e_valor(self, tok, valor: str) -> bool:
        """Casa um token pelo texto, em qualquer categoria (ex.: '}')."""
        return tok is not None and tok[1] == valor

    # -- programa -------------------------------------------------------
    def programa(self):
        while self.peek() is not None:
            tok = self.proximo()
            if tok[0] != "id":
                self.erros.append(Erro("topo_invalido", tok[2],
                                       f"esperada declaração, encontrado {tok[1]!r}"))
                continue
            if tok[1] in CONJUGACOES:
                self.forma(tok[1], tok[2])
            elif tok[1] == "review":
                self.review(tok[2])
            elif tok[1] == "main":
                self.bloco_main(tok[2])
            else:
                self.erros.append(Erro("topo_invalido", tok[2],
                                       f"declaração desconhecida {tok[1]!r}"))
        self.pos_processamento()

    def forma(self, conjugacao: str, linha: int):
        nome = self.proximo()
        if not self.e_ident(nome):
            self.erros.append(Erro("estrutura_forma", linha,
                                   "esperado identificador da forma"))
            return
        self.espera("{", "estrutura_forma", "'{' após o nome da forma")
        atributos: dict[str, object] = {}
        ordem: list[str] = []
        while True:
            tok = self.peek()
            if tok is None:
                self.erros.append(Erro("bloco_nao_fechado", linha,
                                       f"forma '{nome[1]}' sem '}}'"))
                return
            if tok[1] == "}":
                self.proximo()
                break
            self._atributo(nome[1], conjugacao, atributos, ordem)
        self.formas[nome[1]] = {"conjugacao": conjugacao,
                                "atributos": atributos, "ordem": ordem}

    def _atributo(self, nome_forma: str, conjugacao: str, atributos, ordem):
        tok = self.proximo()
        if tok is None or tok[0] != "id":
            linha = tok[2] if tok else (self.toks[-1][2] if self.toks else 1)
            self.erros.append(Erro("estrutura_forma", linha,
                                   "esperado nome de atributo"))
            return
        nome, linha = tok[1], tok[2]
        if nome not in ATRIBUTOS:
            self.erros.append(Erro("atributo_desconhecido", linha,
                                   f"atributo {nome!r} não existe na linguagem"))
            # consome ':' valor ',' para não cascata de erros
            self._consumir_ate_virgula_ou_fecha()
            return
        self.espera(":", "estrutura_forma", "':' após atributo")
        valor = self._valor_atributo(nome, conjugacao, linha)
        if nome in atributos:
            self.erros.append(Erro("atributo_duplicado", linha,
                                   f"atributo {nome!r} repetido na forma "
                                   f"'{nome_forma}'"))
        atributos[nome] = valor
        ordem.append(nome)
        tok = self.peek()
        if tok is not None and tok[1] == ",":
            self.proximo()
            if self.e_valor(self.peek(), "}"):
                self.erros.append(Erro("virgula_final", self.peek()[2],
                                       "vírgula final antes de '}}'"))

    def _consumir_ate_virgula_ou_fecha(self):
        while True:
            tok = self.peek()
            if tok is None or tok[1] in (",", "}"):
                return
            self.proximo()

    def _valor_atributo(self, nome: str, conjugacao: str, linha: int):
        if nome in ("value", "source_path", "exchange_mode", "currency",
                    "classification"):
            tok = self.proximo()
            if tok is None or tok[0] not in ("string", "num", "id"):
                self.erros.append(Erro("estrutura_forma", linha,
                                       f"valor inválido para {nome!r}"))
                return None
            if tok[0] == "string":
                bytes_str = len(self._decodificar_string(tok[1]).encode("utf-8"))
                if bytes_str > LIMITE_STRING:
                    self.erros.append(Erro("string_muito_longa", tok[2],
                                           f"string excede {LIMITE_STRING} bytes"))
                if nome == "source_path" and ("/" in tok[1] or
                                              tok[1].startswith('".')):
                    self.erros.append(Erro("source_path_nao_simbolico", tok[2],
                                           "source_path deve ser nome simbólico "
                                           "de sensor FXP, nunca caminho de SO"))
            return tok[1]
        if nome in ("horizon", "maintenance_deadline"):
            return self._duracao(linha)
        if nome == "cost_bytes":
            tok = self.proximo()
            if tok is None or tok[0] != "num" or "." in tok[1]:
                self.erros.append(Erro("estrutura_forma", linha,
                                       "cost_bytes exige inteiro"))
                return None
            return int(tok[1])
        self.proximo()
        return None

    @staticmethod
    def _decodificar_string(literal: str) -> str:
        corpo = literal[1:-1]
        return re.sub(r"\\(.)", r"\1", corpo)

    def _duracao(self, linha: int):
        num = self.proximo()
        unidade = self.proximo()
        if num is None or num[0] != "num" or unidade is None or unidade[0] != "id" \
                or unidade[1] not in UNIDADES_TEMPO:
            self.erros.append(Erro("duracao_invalida", linha,
                                   "duração esperada: NUM[s|ms|us|ns]"))
            return None
        return f"{num[1]}{unidade[1]}"

    # -- review ---------------------------------------------------------
    def review(self, linha: int):
        nome = self.proximo()
        if not self.e_ident(nome):
            self.erros.append(Erro("estrutura_review", linha,
                                   "esperado identificador após 'review'"))
            return
        self.espera("{", "estrutura_review", "'{' após o nome da review")
        while True:
            tok = self.peek()
            if tok is None:
                self.erros.append(Erro("bloco_nao_fechado", linha,
                                       "review sem '}}'"))
                return
            if tok[1] == "}":
                self.proximo()
                break
            self._regra()
        self.reviews.append(nome[1])

    def _regra(self):
        when = self.proximo()
        if not self.e_ident(when, "when"):
            linha = when[2] if when else 1
            self.erros.append(Erro("regra_mal_formada", linha,
                                   "regra deve começar com 'when'"))
            self._consumir_ate_virgula_ou_fecha()
            return
        sensor = self.proximo()
        if sensor is None or sensor[0] not in ("id", "string"):
            self.erros.append(Erro("regra_mal_formada", when[2],
                                   "esperado sensor (identificador ou string)"))
            return
        op = self.proximo()
        if op is None or op[0] != "op" or op[1] not in OPERADORES:
            self.erros.append(Erro("operador_invalido", when[2],
                                   "operador de comparação inválido"))
            self._consumir_ate_virgula_ou_fecha()
            return
        self._threshold()
        self.espera("->", "regra_mal_formada", "'->' antes das ações")
        self._acoes()

    def _threshold(self):
        num = self.proximo()
        if num is None or num[0] != "num":
            linha = num[2] if num else 1
            self.erros.append(Erro("regra_mal_formada", linha,
                                   "threshold deve ser número"))
            return
        tok = self.peek()
        if tok is not None and (tok[1] in UNIDADES_FISICAS or
                                (tok[0] == "id" and tok[1] == "W")):
            self.proximo()  # unidade física/percentual (FORMAL §3)

    def _acoes(self):
        while True:
            acao = self.proximo()
            if acao is None or acao[0] != "id" or acao[1] not in ACOES:
                linha = acao[2] if acao else 1
                self.erros.append(Erro("acao_desconhecida", linha,
                                       "ação desconhecida na action_list"))
                self._consumir_ate_virgula_ou_fecha()
                return
            if acao[1] == "act":
                self.espera("(", "regra_mal_formada", "'(' no act")
                ator = self.proximo()
                if ator is None or ator[0] not in ("id", "string"):
                    self.erros.append(Erro("regra_mal_formada", acao[2],
                                           "ator esperado no act"))
                self.espera(",", "regra_mal_formada", "',' no act")
                self.proximo()  # expressão: string | número | identificador
                self.espera(")", "regra_mal_formada", "')' no act")
            tok = self.peek()
            if tok is not None and tok[1] == ",":
                self.proximo()
                continue
            return

    # -- main -------------------------------------------------------------
    def bloco_main(self, linha: int):
        self.espera("{", "estrutura_main", "'{' após 'main'")
        while True:
            tok = self.peek()
            if tok is None:
                self.erros.append(Erro("bloco_nao_fechado", linha,
                                       "main sem '}}'"))
                return
            if tok[1] == "}":
                self.proximo()
                break
            self._statement()

    def _statement(self):
        tok = self.proximo()
        if tok is None or tok[0] != "id":
            self.erros.append(Erro("statement_desconhecido", tok[2] if tok else 1,
                                   "statement inválido no main"))
            self._consumir_ate_virgula_ou_fecha()
            return
        if tok[1] in ("keep", "act"):
            self.espera("(", "estrutura_main", f"'(' no {tok[1]}")
            self.proximo()
            if tok[1] == "act":
                self.espera(",", "estrutura_main", "',' no act")
                self.proximo()
            self.espera(")", "estrutura_main", f"')' no {tok[1]}")
        elif tok[1] == "every":
            self._duracao(tok[2])
            self.espera("{", "estrutura_main", "'{' no every")
            profundeza = 1
            while profundeza > 0:
                interno = self.peek()
                if interno is None:
                    self.erros.append(Erro("bloco_nao_fechado", tok[2],
                                           "every sem '}}'"))
                    return
                if interno[1] == "{":
                    profundeza += 1
                elif interno[1] == "}":
                    profundeza -= 1
                elif interno[1] == ",":
                    self.proximo()
                    continue
                else:
                    self._statement()
                    continue
                self.proximo()
        else:
            self.erros.append(Erro("statement_desconhecido", tok[2],
                                   f"statement {tok[1]!r} não existe no main"))
            self._consumir_ate_virgula_ou_fecha()
        nxt = self.peek()
        if nxt is not None and nxt[1] == ",":
            self.proximo()
            if self.e_valor(self.peek(), "}"):
                self.erros.append(Erro("virgula_final", self.peek()[2],
                                       "vírgula final antes de '}}'"))

    # -- pós-processamento: cláusulas cross-block -------------------------
    def pos_processamento(self):
        vistas: dict[str, int] = {}
        for nome in self.reviews:
            if nome not in self.formas:
                self.erros.append(Erro(
                    "review_orfa", 1,
                    f"review para forma inexistente: {nome!r} — erro de "
                    f"compilação (FORMAL §3)"))
            vistas[nome] = vistas.get(nome, 0) + 1
            if vistas[nome] > 1:
                self.erros.append(Erro(
                    "review_duplicada", 1,
                    f"segunda review para {nome!r} — regras não são mescladas "
                    f"(FORMAL §3)"))


def _clausulas_por_forma(parser: _Parser):
    """Checagens por forma que exigem o corpo completo (value/horizon...)."""
    for nome, info in parser.formas.items():
        ordem = info["ordem"]
        if "value" not in ordem:
            parser.erros.append(Erro("value_obrigatorio", 1,
                                     f"forma '{nome}' sem 'value' (Lei 1)"))
        if "horizon" not in ordem:
            parser.erros.append(Erro("horizon_obrigatorio", 1,
                                     f"forma '{nome}' sem 'horizon' (Lei 1)"))
        if "value" in ordem and "horizon" in ordem and \
                ordem.index("value") > ordem.index("horizon"):
            parser.erros.append(Erro("ordem_value_horizon", 1,
                                     f"forma '{nome}': 'value' deve preceder "
                                     f"'horizon' (EBNF)"))
        if info["conjugacao"] == "nonequilibrium" and \
                "maintenance_deadline" not in ordem:
            parser.erros.append(Erro("maintenance_deadline_ausente", 1,
                                     f"forma '{nome}': nonequilibrium exige "
                                     f"maintenance_deadline"))
        for atributo, permitidas in SO_EM.items():
            if atributo in ordem and info["conjugacao"] not in permitidas:
                parser.erros.append(Erro("atributo_nao_aplicavel", 1,
                                         f"'{atributo}' não se aplica a "
                                         f"{info['conjugacao']} ('{nome}')"))


def _keeps_do_main(tokens):
    """Localiza keeps do main para a checagem de forma inexistente."""
    keeps = []
    for i in range(len(tokens) - 2):
        if tokens[i][0] == "id" and tokens[i][1] == "keep" and \
                tokens[i + 1][1] == "(" and tokens[i + 2][0] == "id":
            keeps.append((tokens[i + 2][1], tokens[i][2]))
    return keeps


def validar(texto: str) -> list[Erro]:
    """Valida o texto `.vl` e devolve todos os erros de superfície."""
    erros: list[Erro] = []
    tokens = _tokenizar(_sem_comentarios(texto), erros)
    parser = _Parser(tokens)
    parser.programa()
    _clausulas_por_forma(parser)
    for nome, linha in _keeps_do_main(parser.toks):
        if nome not in parser.formas:
            parser.erros.append(Erro("keep_forma_inexistente", linha,
                                     f"keep('{nome}') não aponta para forma "
                                     f"declarada — cláusula de erro"))
    parser.erros.extend(erros)
    parser.erros.sort(key=lambda e: e.linha)
    return parser.erros
