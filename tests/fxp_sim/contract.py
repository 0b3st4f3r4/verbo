# -*- coding: utf-8 -*-
"""Validador estrutural do IR — as cláusulas de erro da FORMAL §3 como teste.

Na Etapa 1 não há parser; este módulo fixa o CONTRATO que o parser/runtime
da Etapa 2 deverá satisfazer (matriz de rastreabilidade do AGENTS.md):
cada código de diagnóstico abaixo tem ≥ 1 teste em tests/unit/.

Camadas de verificação:
- Estruturais (independentes de registro): value/horizon obrigatórios e
  nesta ordem; maintenance_deadline obrigatório em nonequilibrium e proibido
  nas demais; exchange_mode só em nonequilibrium; cost_bytes só em
  equilibrium; review para forma existente e única; keep para forma
  declarada; durações e operadores bem formados; source_path simbólico.
- Contra registros (opcional): sensor/ator registrados e unidade do
  threshold compatível com a grandeza do sensor (FORMAL §3/§6).
"""

from __future__ import annotations

from dataclasses import dataclass, field

from . import ir

ATRIBUTO_POR_CONJUGACAO = {
    "maintenance_deadline": {"nonequilibrium"},
    "exchange_mode": {"nonequilibrium"},
    "cost_bytes": {"equilibrium"},
}
ORDEM_OBRIGATORIA = ("value", "horizon")


@dataclass
class Diagnostico:
    codigo: str
    mensagem: str
    contexto: str = ""
    extra: dict = field(default_factory=dict)

    def __str__(self):  # legibilidade nos asserts
        local = f" [{self.contexto}]" if self.contexto else ""
        return f"{self.codigo}{local}: {self.mensagem}"


def validar_programa(programa: dict, sensores: dict | None = None,
                     atores: dict | None = None) -> list[Diagnostico]:
    """Valida o programa inteiro; devolve TODOS os diagnósticos encontrados.

    `sensores`/`atores`: mapas nome -> descrição (ex.: grandeza/unidade do
    sensor) copiados do registro mínimo da FORMAL §6; quando None, as
    verificações de registro são puladas.
    """
    diagnosticos: list[Diagnostico] = []
    formas_vistas: set[str] = set()
    reviews_vistas: dict[str, int] = {}
    formas: dict[str, dict] = {}

    for declaracao in programa["declaracoes"]:
        if declaracao["tipo"] == "forma":
            diagnosticos.extend(
                _validar_forma(declaracao, formas_vistas, sensores)
            )
            formas[declaracao["nome"]] = declaracao
        elif declaracao["tipo"] == "review":
            diagnosticos.extend(
                _validar_review(declaracao, formas_vistas, reviews_vistas,
                                sensores, atores)
            )
        else:
            diagnosticos.append(Diagnostico("declaracao_desconhecida",
                                            f"tipo {declaracao['tipo']!r}"))

    if programa.get("main") is not None:
        diagnosticos.extend(
            _validar_main(programa["main"], formas_vistas, atores)
        )
    return diagnosticos


# ----------------------------------------------------------------------
def _validar_forma(decl: dict, vistas: set[str], sensores) -> list[Diagnostico]:
    diags: list[Diagnostico] = []
    nome = decl["nome"]
    ctx = f"forma {nome}"
    if decl["conjugacao"] not in ir.CONJUGACOES:
        diags.append(Diagnostico("conjugacao_desconhecida",
                                 f"conjugação {decl['conjugacao']!r} inválida", ctx))
    atributos = decl["atributos"]

    # value/horizon obrigatórios e nesta ordem (FORMAL §3, Lei 1 do MANIFESTO)
    for obrigatorio in ORDEM_OBRIGATORIA:
        if obrigatorio not in atributos:
            diags.append(Diagnostico(
                "atributo_obrigatorio_ausente",
                f"'{obrigatorio}' é obrigatório em toda forma (Lei 1)", ctx))
    chaves = list(atributos.keys())
    presentes = [a for a in ORDEM_OBRIGATORIA if a in chaves]
    if presentes != [a for a in chaves if a in ORDEM_OBRIGATORIA] or (
        len(presentes) == 2 and chaves.index("value") > chaves.index("horizon")
    ):
        diags.append(Diagnostico(
            "ordem_obrigatoria",
            "'value' deve preceder 'horizon' (EBNF: primeiros atributos)", ctx))

    # aplicabilidade por conjugação
    for atributo, permitidas in ATRIBUTO_POR_CONJUGACAO.items():
        if atributo in atributos and decl["conjugacao"] not in permitidas:
            diags.append(Diagnostico(
                "atributo_nao_aplicavel",
                f"'{atributo}' só se aplica a {sorted(permitidas)}", ctx))
    if decl["conjugacao"] == "nonequilibrium" and "maintenance_deadline" not in atributos:
        diags.append(Diagnostico(
            "maintenance_deadline_ausente",
            "nonequilibrium exige maintenance_deadline — sem ele a forma "
            "jamais colapsaria (FORMAL §3)", ctx))

    # durações bem formadas
    for atributo in ("horizon", "maintenance_deadline"):
        if atributo in atributos:
            try:
                ir.duracao(atributos[atributo])
            except ValueError:
                diags.append(Diagnostico("duracao_invalida",
                                         f"{atributo}={atributos[atributo]!r}", ctx))

    # source_path é nome EXCLUSIVAMENTE simbólico (FORMAL §3, nota)
    source_path = atributos.get("source_path")
    if source_path is not None:
        if not isinstance(source_path, str) or ("/" in source_path or
                                                source_path.startswith(".")):
            diags.append(Diagnostico(
                "source_path_nao_simbolico",
                f"source_path {source_path!r} não é nome simbólico de sensor FXP",
                ctx))
        elif sensores is not None and source_path not in sensores:
            diags.append(Diagnostico("sensor_nao_registrado",
                                     f"sensor {source_path!r} fora do registro",
                                     ctx))

    if nome in vistas:
        diags.append(Diagnostico("forma_duplicada",
                                 f"forma {nome!r} declarada duas vezes", ctx))
    vistas.add(nome)
    return diags


def _validar_review(decl: dict, formas_vistas: set[str],
                    reviews_vistas: dict[str, int], sensores,
                    atores) -> list[Diagnostico]:
    diags: list[Diagnostico] = []
    nome = decl["forma"]
    ctx = f"review {nome}"

    # review órfã e review duplicada são ERROS DE COMPILAÇÃO (FORMAL §3)
    if nome not in formas_vistas:
        diags.append(Diagnostico(
            "review_orfa",
            f"review para forma inexistente: {nome!r} (regras não são "
            f"adicionadas a formas fantasma)", ctx))
    reviews_vistas[nome] = reviews_vistas.get(nome, 0) + 1
    if reviews_vistas[nome] > 1:
        diags.append(Diagnostico(
            "review_duplicada",
            f"segunda review para {nome!r} — regras não são mescladas "
            f"(FORMAL §3)", ctx))

    for i, regra in enumerate(decl["regras"]):
        rctx = f"{ctx} regra#{i}"
        if regra["op"] not in ir.OPERADORES:
            diags.append(Diagnostico("operador_invalido",
                                     f"op {regra['op']!r}", rctx))
        unidade = regra.get("unidade")
        if unidade is not None and unidade not in ("°C", "W", "%"):
            diags.append(Diagnostico("unidade_desconhecida",
                                     f"unidade {unidade!r}", rctx))
        if sensores is not None:
            sensor = sensores.get(regra["sensor"])
            if sensor is None:
                diags.append(Diagnostico("sensor_nao_registrado",
                                         f"sensor {regra['sensor']!r} fora do "
                                         f"registro", rctx))
            elif unidade is not None:
                esperada = ir.UNIDADE_POR_GRANDEZA.get(sensor.get("grandeza", ""))
                if esperada is not None and unidade != esperada:
                    diags.append(Diagnostico(
                        "unidade_incompativel",
                        f"unidade {unidade!r} incompatível com a grandeza "
                        f"{sensor.get('grandeza')!r} do sensor "
                        f"{regra['sensor']!r} (esperado {esperada!r})", rctx))
        for acao in regra["acoes"]:
            if acao["action"] not in ir.ACOES:
                diags.append(Diagnostico("acao_desconhecida",
                                         f"ação {acao['action']!r}", rctx))
            if acao["action"] == "act" and atores is not None:
                if acao.get("ator") not in atores:
                    diags.append(Diagnostico(
                        "ator_nao_registrado",
                        f"ator {acao.get('ator')!r} fora do registro FXP", rctx))
    return diags


def _validar_main(bloco: dict, formas_vistas: set[str], atores
                  ) -> list[Diagnostico]:
    diags: list[Diagnostico] = []

    def passe(statements, profundeza=0):
        if profundeza > 8:  # defesa contra aninhamento infinito
            return
        for stmt in statements:
            tipo = stmt["statement"]
            if tipo == "keep":
                # `keep` de forma inexistente — cláusula de erro (AGENTS.md Done)
                if stmt["forma"] not in formas_vistas:
                    diags.append(Diagnostico(
                        "keep_forma_inexistente",
                        f"keep('{stmt['forma']}') não aponta para forma "
                        f"declarada", "main"))
            elif tipo == "act":
                if atores is not None and stmt["ator"] not in atores:
                    diags.append(Diagnostico(
                        "ator_nao_registrado",
                        f"ator {stmt['ator']!r} fora do registro FXP", "main"))
            elif tipo == "every":
                try:
                    ir.duracao(stmt["periodo"])
                except ValueError:
                    diags.append(Diagnostico("duracao_invalida",
                                             f"every {stmt['periodo']!r}", "main"))
                passe(stmt["statements"], profundeza + 1)
            else:
                diags.append(Diagnostico("statement_desconhecido",
                                         f"{tipo!r}", "main"))

    passe(bloco["statements"])
    return diags
