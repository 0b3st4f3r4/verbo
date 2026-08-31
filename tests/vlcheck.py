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

CONJUGATIONS = {"event", "equilibrium", "nonequilibrium"}
ATTRIBUTES = {"value", "horizon", "source_path", "maintenance_deadline",
              "exchange_mode", "cost_bytes", "currency", "classification"}
ONLY_IN = {"maintenance_deadline": {"nonequilibrium"},
           "exchange_mode": {"nonequilibrium"},
           "cost_bytes": {"equilibrium"}}
TIME_UNITS = {"s", "ms", "us", "ns"}
PHYSICAL_UNITS = {"W", "°C", "%"}
OPERATORS = {"<", ">", "<=", ">=", "==", "!="}
ACTIONS = {"dissolve", "subvert", "reclassify_as_equilibrium",
           "reclassify_as_nonequilibrium", "notify_shutdown", "act"}
STRING_LIMIT = 256  # bytes (FORMAL §2)

_TOKEN = re.compile(r"""
    (?P<string>"(?:\\.|[^"\\])*")
  | (?P<num>\d+\.\d+|\d+)
  | (?P<physical>°C|%)
  | (?P<id>[a-zA-Z_][a-zA-Z0-9_]*)
  | (?P<op>->|<=|>=|==|!=|<|>)
  | (?P<punc>[:,;{}()])
  | (?P<junk>\S)
""", re.VERBOSE)


class Error(NamedTuple):
    code: str
    line: int
    message: str


# ----------------------------------------------------------------------
# Pré-processamento e tokenização
# ----------------------------------------------------------------------
def _strip_comments(text: str) -> str:
    """Remove // e /* */ preservando quebras de linha (para numeração)."""
    out = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if c == "/" and i + 1 < n and text[i + 1] == "/":
            while i < n and text[i] != "\n":
                out.append(" ")
                i += 1
        elif c == "/" and i + 1 < n and text[i + 1] == "*":
            end = text.find("*/", i + 2)
            end = n if end == -1 else end + 2
            for ch in text[i:end]:
                out.append(ch if ch == "\n" else " ")
            i = end
        else:
            out.append(c)
            i += 1
    return "".join(out)


def _tokenize(text: str, errors: list[Error]) -> list[tuple[str, str, int]]:
    tokens: list[tuple[str, str, int]] = []
    line = 1
    pos = 0
    for m in _TOKEN.finditer(text):
        line += text.count("\n", pos, m.start())
        pos = m.start()
        kind = m.lastgroup
        if kind == "junk":
            # léxico estrito: caractere fora da linguagem vira erro (ex.: '=')
            errors.append(Error("lexema_invalido", line,
                                f"lexema {m.group()!r} não existe na linguagem"))
            continue
        tokens.append((kind, m.group(), line))
    return tokens


# ----------------------------------------------------------------------
# Parser de superfície
# ----------------------------------------------------------------------
class _Parser:
    def __init__(self, tokens):
        self.toks = tokens
        self.i = 0
        self.errors: list[Error] = []
        self.forms: dict[str, dict] = {}
        self.reviews: list[str] = []

    # -- utilidades -----------------------------------------------------
    def peek(self):
        return self.toks[self.i] if self.i < len(self.toks) else None

    def advance(self):
        tok = self.peek()
        if tok is not None:
            self.i += 1
        return tok

    def expect(self, value: str, code: str, msg: str):
        tok = self.advance()
        if tok is None:
            self.errors.append(Error(code, self.toks[-1][2] if self.toks else 1,
                                     f"fim inesperado: {msg}"))
            return None
        if tok[1] != value:
            self.errors.append(Error(code, tok[2], f"{msg} (encontrado {tok[1]!r})"))
            return None
        return tok

    def is_ident(self, tok, value: str | None = None) -> bool:
        return tok is not None and tok[0] == "id" and (value is None or tok[1] == value)

    def is_value(self, tok, value: str) -> bool:
        """Casa um token pelo texto, em qualquer categoria (ex.: '}')."""
        return tok is not None and tok[1] == value

    # -- programa -------------------------------------------------------
    def program(self):
        while self.peek() is not None:
            tok = self.advance()
            if tok[0] != "id":
                self.errors.append(Error("topo_invalido", tok[2],
                                         f"esperada declaração, encontrado {tok[1]!r}"))
                continue
            if tok[1] in CONJUGATIONS:
                self.form(tok[1], tok[2])
            elif tok[1] == "review":
                self.review(tok[2])
            elif tok[1] == "main":
                self.main_block(tok[2])
            else:
                self.errors.append(Error("topo_invalido", tok[2],
                                         f"declaração desconhecida {tok[1]!r}"))
        self.post_process()

    def form(self, conjugation: str, line: int):
        name = self.advance()
        if not self.is_ident(name):
            self.errors.append(Error("estrutura_forma", line,
                                     "esperado identificador da forma"))
            return
        self.expect("{", "estrutura_forma", "'{' após o nome da forma")
        attributes: dict[str, object] = {}
        order: list[str] = []
        while True:
            tok = self.peek()
            if tok is None:
                self.errors.append(Error("bloco_nao_fechado", line,
                                         f"forma '{name[1]}' sem '}}'"))
                return
            if tok[1] == "}":
                self.advance()
                break
            self._attribute(name[1], conjugation, attributes, order)
        self.forms[name[1]] = {"conjugation": conjugation,
                               "attributes": attributes, "order": order}

    def _attribute(self, form_name: str, conjugation: str, attributes, order):
        tok = self.advance()
        if tok is None or tok[0] != "id":
            line = tok[2] if tok else (self.toks[-1][2] if self.toks else 1)
            self.errors.append(Error("estrutura_forma", line,
                                     "esperado nome de atributo"))
            return
        name, line = tok[1], tok[2]
        if name not in ATTRIBUTES:
            self.errors.append(Error("atributo_desconhecido", line,
                                     f"atributo {name!r} não existe na linguagem"))
            # consome ':' valor ',' para não cascata de erros
            self._consume_until_comma_or_close()
            return
        self.expect(":", "estrutura_forma", "':' após atributo")
        value = self._attribute_value(name, conjugation, line)
        if name in attributes:
            self.errors.append(Error("atributo_duplicado", line,
                                     f"atributo {name!r} repetido na forma "
                                     f"'{form_name}'"))
        attributes[name] = value
        order.append(name)
        tok = self.peek()
        if tok is not None and tok[1] == ",":
            self.advance()
            if self.is_value(self.peek(), "}"):
                self.errors.append(Error("virgula_final", self.peek()[2],
                                         "vírgula final antes de '}}'"))

    def _consume_until_comma_or_close(self):
        while True:
            tok = self.peek()
            if tok is None or tok[1] in (",", "}"):
                return
            self.advance()

    def _attribute_value(self, name: str, conjugation: str, line: int):
        if name in ("value", "source_path", "exchange_mode", "currency",
                    "classification"):
            tok = self.advance()
            if tok is None or tok[0] not in ("string", "num", "id"):
                self.errors.append(Error("estrutura_forma", line,
                                         f"valor inválido para {name!r}"))
                return None
            if tok[0] == "string":
                str_bytes = len(self._decode_string(tok[1]).encode("utf-8"))
                if str_bytes > STRING_LIMIT:
                    self.errors.append(Error("string_muito_longa", tok[2],
                                             f"string excede {STRING_LIMIT} bytes"))
                if name == "source_path" and ("/" in tok[1] or
                                              tok[1].startswith('".')):
                    self.errors.append(Error("source_path_nao_simbolico", tok[2],
                                             "source_path deve ser nome simbólico "
                                             "de sensor FXP, nunca caminho de SO"))
            return tok[1]
        if name in ("horizon", "maintenance_deadline"):
            return self._duration(line)
        if name == "cost_bytes":
            tok = self.advance()
            if tok is None or tok[0] != "num" or "." in tok[1]:
                self.errors.append(Error("estrutura_forma", line,
                                         "cost_bytes exige inteiro"))
                return None
            return int(tok[1])
        self.advance()
        return None

    @staticmethod
    def _decode_string(literal: str) -> str:
        body = literal[1:-1]
        return re.sub(r"\\(.)", r"\1", body)

    def _duration(self, line: int):
        num = self.advance()
        unit = self.advance()
        if num is None or num[0] != "num" or unit is None or unit[0] != "id" \
                or unit[1] not in TIME_UNITS:
            self.errors.append(Error("duracao_invalida", line,
                                     "duração esperada: NUM[s|ms|us|ns]"))
            return None
        return f"{num[1]}{unit[1]}"

    # -- review ---------------------------------------------------------
    def review(self, line: int):
        name = self.advance()
        if not self.is_ident(name):
            self.errors.append(Error("estrutura_review", line,
                                     "esperado identificador após 'review'"))
            return
        self.expect("{", "estrutura_review", "'{' após o nome da review")
        while True:
            tok = self.peek()
            if tok is None:
                self.errors.append(Error("bloco_nao_fechado", line,
                                         "review sem '}}'"))
                return
            if tok[1] == "}":
                self.advance()
                break
            self._rule()
        self.reviews.append(name[1])

    def _rule(self):
        when = self.advance()
        if not self.is_ident(when, "when"):
            line = when[2] if when else 1
            self.errors.append(Error("regra_mal_formada", line,
                                     "regra deve começar com 'when'"))
            self._consume_until_comma_or_close()
            return
        sensor = self.advance()
        if sensor is None or sensor[0] not in ("id", "string"):
            self.errors.append(Error("regra_mal_formada", when[2],
                                     "esperado sensor (identificador ou string)"))
            return
        op = self.advance()
        if op is None or op[0] != "op" or op[1] not in OPERATORS:
            self.errors.append(Error("operador_invalido", when[2],
                                     "operador de comparação inválido"))
            self._consume_until_comma_or_close()
            return
        self._threshold()
        self.expect("->", "regra_mal_formada", "'->' antes das ações")
        self._actions()

    def _threshold(self):
        num = self.advance()
        if num is None or num[0] != "num":
            line = num[2] if num else 1
            self.errors.append(Error("regra_mal_formada", line,
                                     "threshold deve ser número"))
            return
        tok = self.peek()
        if tok is not None and (tok[1] in PHYSICAL_UNITS or
                                (tok[0] == "id" and tok[1] == "W")):
            self.advance()  # unidade física/percentual (FORMAL §3)

    def _actions(self):
        while True:
            action = self.advance()
            if action is None or action[0] != "id" or action[1] not in ACTIONS:
                line = action[2] if action else 1
                self.errors.append(Error("acao_desconhecida", line,
                                         "ação desconhecida na action_list"))
                self._consume_until_comma_or_close()
                return
            if action[1] == "act":
                self.expect("(", "regra_mal_formada", "'(' no act")
                actor = self.advance()
                if actor is None or actor[0] not in ("id", "string"):
                    self.errors.append(Error("regra_mal_formada", action[2],
                                             "ator esperado no act"))
                self.expect(",", "regra_mal_formada", "',' no act")
                self.advance()  # expressão: string | número | identificador
                self.expect(")", "regra_mal_formada", "')' no act")
            tok = self.peek()
            if tok is not None and tok[1] == ",":
                self.advance()
                continue
            return

    # -- main -------------------------------------------------------------
    def main_block(self, line: int):
        self.expect("{", "estrutura_main", "'{' após 'main'")
        while True:
            tok = self.peek()
            if tok is None:
                self.errors.append(Error("bloco_nao_fechado", line,
                                         "main sem '}}'"))
                return
            if tok[1] == "}":
                self.advance()
                break
            self._statement()

    def _statement(self):
        tok = self.advance()
        if tok is None or tok[0] != "id":
            self.errors.append(Error("statement_desconhecido", tok[2] if tok else 1,
                                     "statement inválido no main"))
            self._consume_until_comma_or_close()
            return
        if tok[1] in ("keep", "act"):
            self.expect("(", "estrutura_main", f"'(' no {tok[1]}")
            self.advance()
            if tok[1] == "act":
                self.expect(",", "estrutura_main", "',' no act")
                self.advance()
            self.expect(")", "estrutura_main", f"')' no {tok[1]}")
        elif tok[1] == "every":
            self._duration(tok[2])
            self.expect("{", "estrutura_main", "'{' no every")
            depth = 1
            while depth > 0:
                inner = self.peek()
                if inner is None:
                    self.errors.append(Error("bloco_nao_fechado", tok[2],
                                             "every sem '}}'"))
                    return
                if inner[1] == "{":
                    depth += 1
                elif inner[1] == "}":
                    depth -= 1
                elif inner[1] == ",":
                    self.advance()
                    continue
                else:
                    self._statement()
                    continue
                self.advance()
        else:
            self.errors.append(Error("statement_desconhecido", tok[2],
                                     f"statement {tok[1]!r} não existe no main"))
            self._consume_until_comma_or_close()
        nxt = self.peek()
        if nxt is not None and nxt[1] == ",":
            self.advance()
            if self.is_value(self.peek(), "}"):
                self.errors.append(Error("virgula_final", self.peek()[2],
                                         "vírgula final antes de '}}'"))

    # -- pós-processamento: cláusulas cross-block -------------------------
    def post_process(self):
        seen: dict[str, int] = {}
        for name in self.reviews:
            if name not in self.forms:
                self.errors.append(Error(
                    "review_orfa", 1,
                    f"review para forma inexistente: {name!r} — erro de "
                    f"compilação (FORMAL §3)"))
            seen[name] = seen.get(name, 0) + 1
            if seen[name] > 1:
                self.errors.append(Error(
                    "review_duplicada", 1,
                    f"segunda review para {name!r} — regras não são mescladas "
                    f"(FORMAL §3)"))


def _clauses_per_form(parser: _Parser):
    """Checagens por forma que exigem o corpo completo (value/horizon...)."""
    for name, info in parser.forms.items():
        order = info["order"]
        if "value" not in order:
            parser.errors.append(Error("value_obrigatorio", 1,
                                       f"forma '{name}' sem 'value' (Lei 1)"))
        if "horizon" not in order:
            parser.errors.append(Error("horizon_obrigatorio", 1,
                                       f"forma '{name}' sem 'horizon' (Lei 1)"))
        if "value" in order and "horizon" in order and \
                order.index("value") > order.index("horizon"):
            parser.errors.append(Error("ordem_value_horizon", 1,
                                       f"forma '{name}': 'value' deve preceder "
                                       f"'horizon' (EBNF)"))
        if info["conjugation"] == "nonequilibrium" and \
                "maintenance_deadline" not in order:
            parser.errors.append(Error("maintenance_deadline_ausente", 1,
                                       f"forma '{name}': nonequilibrium exige "
                                       f"maintenance_deadline"))
        for attribute, allowed in ONLY_IN.items():
            if attribute in order and info["conjugation"] not in allowed:
                parser.errors.append(Error("atributo_nao_aplicavel", 1,
                                           f"'{attribute}' não se aplica a "
                                           f"{info['conjugation']} ('{name}')"))


def _main_keeps(tokens):
    """Localiza keeps do main para a checagem de forma inexistente."""
    keeps = []
    for i in range(len(tokens) - 2):
        if tokens[i][0] == "id" and tokens[i][1] == "keep" and \
                tokens[i + 1][1] == "(" and tokens[i + 2][0] == "id":
            keeps.append((tokens[i + 2][1], tokens[i][2]))
    return keeps


def validate(text: str) -> list[Error]:
    """Valida o texto `.vl` e devolve todos os erros de superfície."""
    errors: list[Error] = []
    tokens = _tokenize(_strip_comments(text), errors)
    parser = _Parser(tokens)
    parser.program()
    _clauses_per_form(parser)
    for name, line in _main_keeps(parser.toks):
        if name not in parser.forms:
            parser.errors.append(Error("keep_forma_inexistente", line,
                                       f"keep('{name}') não aponta para forma "
                                       f"declarada — cláusula de erro"))
    parser.errors.extend(errors)
    parser.errors.sort(key=lambda e: e.line)
    return parser.errors
