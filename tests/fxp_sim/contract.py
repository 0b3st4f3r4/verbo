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

ATTRIBUTE_BY_CONJUGATION = {
    "maintenance_deadline": {"nonequilibrium"},
    "exchange_mode": {"nonequilibrium"},
    "cost_bytes": {"equilibrium"},
}
REQUIRED_ORDER = ("value", "horizon")


@dataclass
class Diagnosis:
    code: str
    message: str
    context: str = ""
    extra: dict = field(default_factory=dict)

    def __str__(self):  # legibilidade nos asserts
        local = f" [{self.context}]" if self.context else ""
        return f"{self.code}{local}: {self.message}"


def validate_program(program: dict, sensors: dict | None = None,
                     actors: dict | None = None) -> list[Diagnosis]:
    """Valida o programa inteiro; devolve TODOS os diagnósticos encontrados.

    `sensors`/`actors`: mapas nome -> descrição (ex.: grandeza/unidade do
    sensor) copiados do registro mínimo da FORMAL §6; quando None, as
    verificações de registro são puladas.
    """
    diagnoses: list[Diagnosis] = []
    forms_seen: set[str] = set()
    reviews_seen: dict[str, int] = {}
    forms: dict[str, dict] = {}

    for declaration in program["declarations"]:
        if declaration["type"] == "form":
            diagnoses.extend(
                _validate_form(declaration, forms_seen, sensors)
            )
            forms[declaration["name"]] = declaration
        elif declaration["type"] == "review":
            diagnoses.extend(
                _validate_review(declaration, forms_seen, reviews_seen,
                                 sensors, actors)
            )
        else:
            diagnoses.append(Diagnosis("declaracao_desconhecida",
                                       f"tipo {declaration['type']!r}"))

    if program.get("main") is not None:
        diagnoses.extend(
            _validate_main(program["main"], forms_seen, actors)
        )
    return diagnoses


# ----------------------------------------------------------------------
def _validate_form(decl: dict, seen: set[str], sensors) -> list[Diagnosis]:
    diags: list[Diagnosis] = []
    name = decl["name"]
    ctx = f"forma {name}"
    if decl["conjugation"] not in ir.CONJUGATIONS:
        diags.append(Diagnosis("conjugacao_desconhecida",
                               f"conjugação {decl['conjugation']!r} inválida", ctx))
    attributes = decl["attributes"]

    # value/horizon obrigatórios e nesta ordem (FORMAL §3, Lei 1 do MANIFESTO)
    for required in REQUIRED_ORDER:
        if required not in attributes:
            diags.append(Diagnosis(
                "atributo_obrigatorio_ausente",
                f"'{required}' é obrigatório em toda forma (Lei 1)", ctx))
    keys = list(attributes.keys())
    present = [a for a in REQUIRED_ORDER if a in keys]
    if present != [a for a in keys if a in REQUIRED_ORDER] or (
        len(present) == 2 and keys.index("value") > keys.index("horizon")
    ):
        diags.append(Diagnosis(
            "ordem_obrigatoria",
            "'value' deve preceder 'horizon' (EBNF: primeiros atributos)", ctx))

    # aplicabilidade por conjugação
    for attribute, allowed in ATTRIBUTE_BY_CONJUGATION.items():
        if attribute in attributes and decl["conjugation"] not in allowed:
            diags.append(Diagnosis(
                "atributo_nao_aplicavel",
                f"'{attribute}' só se aplica a {sorted(allowed)}", ctx))
    if decl["conjugation"] == "nonequilibrium" and "maintenance_deadline" not in attributes:
        diags.append(Diagnosis(
            "maintenance_deadline_ausente",
            "nonequilibrium exige maintenance_deadline — sem ele a forma "
            "jamais colapsaria (FORMAL §3)", ctx))

    # durações bem formadas
    for attribute in ("horizon", "maintenance_deadline"):
        if attribute in attributes:
            try:
                ir.duration(attributes[attribute])
            except ValueError:
                diags.append(Diagnosis("duracao_invalida",
                                       f"{attribute}={attributes[attribute]!r}", ctx))

    # source_path é nome EXCLUSIVAMENTE simbólico (FORMAL §3, nota)
    source_path = attributes.get("source_path")
    if source_path is not None:
        if not isinstance(source_path, str) or ("/" in source_path or
                                                source_path.startswith(".")):
            diags.append(Diagnosis(
                "source_path_nao_simbolico",
                f"source_path {source_path!r} não é nome simbólico de sensor FXP",
                ctx))
        elif sensors is not None and source_path not in sensors:
            diags.append(Diagnosis("sensor_nao_registrado",
                                   f"sensor {source_path!r} fora do registro",
                                   ctx))

    if name in seen:
        diags.append(Diagnosis("forma_duplicada",
                               f"forma {name!r} declarada duas vezes", ctx))
    seen.add(name)
    return diags


def _validate_review(decl: dict, forms_seen: set[str],
                     reviews_seen: dict[str, int], sensors,
                     actors) -> list[Diagnosis]:
    diags: list[Diagnosis] = []
    name = decl["form"]
    ctx = f"review {name}"

    # review órfã e review duplicada são ERROS DE COMPILAÇÃO (FORMAL §3)
    if name not in forms_seen:
        diags.append(Diagnosis(
            "review_orfa",
            f"review para forma inexistente: {name!r} (regras não são "
            f"adicionadas a formas fantasma)", ctx))
    reviews_seen[name] = reviews_seen.get(name, 0) + 1
    if reviews_seen[name] > 1:
        diags.append(Diagnosis(
            "review_duplicada",
            f"segunda review para {name!r} — regras não são mescladas "
            f"(FORMAL §3)", ctx))

    for i, rule in enumerate(decl["rules"]):
        rctx = f"{ctx} regra#{i}"
        if rule["op"] not in ir.OPERATORS:
            diags.append(Diagnosis("operador_invalido",
                                   f"op {rule['op']!r}", rctx))
        unit = rule.get("unit")
        if unit is not None and unit not in ("°C", "W", "%"):
            diags.append(Diagnosis("unidade_desconhecida",
                                   f"unidade {unit!r}", rctx))
        if sensors is not None:
            sensor = sensors.get(rule["sensor"])
            if sensor is None:
                diags.append(Diagnosis("sensor_nao_registrado",
                                       f"sensor {rule['sensor']!r} fora do "
                                       f"registro", rctx))
            elif unit is not None:
                expected = ir.UNIT_BY_QUANTITY.get(sensor.get("quantity", ""))
                if expected is not None and unit != expected:
                    diags.append(Diagnosis(
                        "unidade_incompativel",
                        f"unidade {unit!r} incompatível com a grandeza "
                        f"{sensor.get('quantity')!r} do sensor "
                        f"{rule['sensor']!r} (esperado {expected!r})", rctx))
        for action in rule["actions"]:
            if action["action"] not in ir.ACTIONS:
                diags.append(Diagnosis("acao_desconhecida",
                                       f"ação {action['action']!r}", rctx))
            if action["action"] == "act" and actors is not None:
                if action.get("actor") not in actors:
                    diags.append(Diagnosis(
                        "ator_nao_registrado",
                        f"ator {action.get('actor')!r} fora do registro FXP", rctx))
    return diags


def _validate_main(block: dict, forms_seen: set[str], actors
                   ) -> list[Diagnosis]:
    diags: list[Diagnosis] = []

    def walk(statements, depth=0):
        if depth > 8:  # defesa contra aninhamento infinito
            return
        for stmt in statements:
            kind = stmt["statement"]
            if kind == "keep":
                # `keep` de forma inexistente — cláusula de erro (AGENTS.md Done)
                if stmt["form"] not in forms_seen:
                    diags.append(Diagnosis(
                        "keep_forma_inexistente",
                        f"keep('{stmt['form']}') não aponta para forma "
                        f"declarada", "main"))
            elif kind == "act":
                if actors is not None and stmt["actor"] not in actors:
                    diags.append(Diagnosis(
                        "ator_nao_registrado",
                        f"ator {stmt['actor']!r} fora do registro FXP", "main"))
            elif kind == "every":
                try:
                    ir.duration(stmt["period"])
                except ValueError:
                    diags.append(Diagnosis("duracao_invalida",
                                           f"every {stmt['period']!r}", "main"))
                walk(stmt["statements"], depth + 1)
            else:
                diags.append(Diagnosis("statement_desconhecido",
                                       f"{kind!r}", "main"))

    walk(block["statements"])
    return diags
