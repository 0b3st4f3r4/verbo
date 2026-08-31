# -*- coding: utf-8 -*-
"""Validação do cheat sheet canônico contra o modelo local (docs/PLAN.md §7).

Executa o banco fixo de 20 prompts (docs/CHEATSHEET-PROMPTS.yaml) N vezes
contra um endpoint OpenAI-compatível (o vLLM local do projeto), injetando
docs/VBL-CHEATSHEET.md como prompt de sistema, e avalia:

- prompts de sintaxe: o primeiro bloco de código da resposta deve passar no
  mini-validador `tests/vlcheck.py` (verificador sintático dedicado — o
  parser real é entregável da Etapa 2);
- rubrica semântica: âncoras obrigatórias da FORMAL presentes na resposta.

Aceito se ≥ 90% das respostas passam (rubrica E sintaxe, quando aplicável).
O relatório é versionado em docs/CHEATSHEET-VALIDACAO.md.

Uso:
  python3 scripts/validate_cheatsheet.py \
      --base-url http://127.0.0.1:8000/v1 --model qwen3-4b
(saída de execução real fica registrada no relatório versionado; o script
retorna código 1 se o limiar não for atingido)
"""

from __future__ import annotations

import argparse
import datetime
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

RAIZ = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(RAIZ / "tests"))
import vlcheck  # noqa: E402

BLOCO_CODIGO = re.compile(r"```(?:verbolang|vl)?\s*\n(.*?)```", re.DOTALL)


def abrir_caminho(relativo: str) -> str:
    return (RAIZ / relativo).read_text(encoding="utf-8")


def consultar(base_url: str, model: str, api_key: str | None,
              cheatsheet: str, enunciado: str, temperatura: float,
              max_tokens: int, tentativas: int = 3) -> str:
    """POST /chat/completions com o cheat sheet como prompt de sistema."""
    payload = {
        "model": model,
        "messages": [
            {"role": "system", "content": cheatsheet},
            {"role": "user", "content": enunciado},
        ],
        "temperature": temperatura,
        "max_tokens": max_tokens,
    }
    requisicao = urllib.request.Request(
        base_url.rstrip("/") + "/chat/completions",
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json"},
    )
    if api_key:
        requisicao.add_header("Authorization", f"Bearer {api_key}")
    ultimo_erro: Exception | None = None
    for _ in range(tentativas):
        try:
            with urllib.request.urlopen(requisicao, timeout=120) as resposta:
                dados = json.loads(resposta.read().decode("utf-8"))
                return dados["choices"][0]["message"]["content"]
        except (urllib.error.URLError, KeyError, json.JSONDecodeError) as erro:
            ultimo_erro = erro
            time.sleep(2.0)
    raise RuntimeError(f"falha ao consultar {base_url}: {ultimo_erro}")


def normalizar(texto: str) -> str:
    return texto.casefold()


def variantes(frase: str) -> set[str]:
    """Variações morfológicas ingênuas (plural/singular por palavra) para a
    rubrica não falhar por flexão do português ("disparo falso" vs
    "disparos falsos"). Aproximação deliberada: falsos positivos aqui são
    filtrados pela revisão do GQT sobre o relatório versionado."""
    base = normalizar(frase)
    formas = {base}
    plural = " ".join(palavra + "s" if len(palavra) >= 3 else palavra
                      for palavra in base.split())
    formas.add(plural)
    if base.endswith("s"):
        formas.add(" ".join(base.split()[:-1]))
    return formas


def rubrica_passa(resposta: str, rubrica: list, codigo: str | None = None) -> tuple[bool, list[dict]]:
    """Cada item (string) deve constar; item lista = qualquer alternativa.

    Item iniciado por `!` é NEGATIVO: o texto NÃO pode constar. Em prompts de
    sintaxe, itens negativos avaliam apenas o(s) bloco(s) de código (a prosa
    pode legitimamente mencionar o que a forma não usa).
    """
    alvo = normalizar(resposta)
    alvo_codigo = normalizar(codigo) if codigo is not None else None
    detalhes = []
    ok_geral = True
    for item in rubrica:
        alternativas = item if isinstance(item, list) else [item]
        negativo = all(str(a).startswith("!") for a in alternativas)
        if negativo:
            escopo = alvo_codigo if alvo_codigo is not None else alvo
            casou = not any(normalizar(str(a)[1:].strip()) in escopo
                            for a in alternativas)
        else:
            casou = any(any(v in alvo for v in variantes(str(a)))
                        for a in alternativas)
        detalhes.append({"item": item, "ok": casou})
        ok_geral = ok_geral and casou
    return ok_geral, detalhes


def avaliar(prompt: dict, resposta: str) -> dict:
    resultado = {"id": prompt["id"], "ok": True, "motivos": [],
                 "rubrica": []}
    codigo = None
    if prompt["tipo"] == "sintaxe":
        blocos = BLOCO_CODIGO.findall(resposta)
        if not blocos:
            resultado["ok"] = False
            resultado["motivos"].append("sem bloco de código .vl")
        else:
            codigo = "\n".join(blocos)
            erros = vlcheck.validar(blocos[0])
            if erros:
                resultado["ok"] = False
                resultado["motivos"].append(
                    "sintaxe: " + "; ".join(f"{e.codigo} L{e.linha} ({e.mensagem})"
                                            for e in erros[:4]))
    ok_rubrica, detalhes = rubrica_passa(resposta, prompt["rubrica"], codigo)
    resultado["rubrica"] = detalhes
    if not ok_rubrica:
        resultado["ok"] = False
        faltando = [d["item"] for d in detalhes if not d["ok"]]
        resultado["motivos"].append(f"rubrica: faltou {faltando}")
    return resultado


def gerar_relatorio(banco: dict, execucoes: list[dict], base_url: str,
                    model: str, saida: Path) -> float:
    total = sum(len(e["tentativas"]) for e in execucoes)
    passos = sum(1 for e in execucoes for t in e["tentativas"] if t["ok"])
    taxa = passos / total if total else 0.0
    aceito = taxa >= banco["limiar_aceitacao"]
    linhas = [
        "# CHEATSHEET-VALIDACAO.md — validação do cheat sheet (PLAN §7)",
        "",
        f"- Data: {datetime.datetime.now().isoformat(timespec='seconds')}",
        f"- Endpoint: `{base_url}` · modelo: `{model}`",
        f"- Banco: {len(execucoes)} prompts × {banco['execucoes_por_prompt']} execuções",
        f"- Verificador sintático: `tests/vlcheck.py` (mini-validador dedicado)",
        f"- Resultado: **{passos}/{total} = {taxa:.1%}** — "
        f"{'ACEITO' if aceito else 'REPROVADO'} (limiar ≥ "
        f"{banco['limiar_aceitacao']:.0%})",
        "",
        "| Prompt | Exec | Veredito | Motivos |",
        "|---|---|---|---|",
    ]
    for e in execucoes:
        for i, t in enumerate(e["tentativas"], 1):
            motivos = "; ".join(t["motivos"]) or "—"
            linhas.append(f"| {e['id']} | {i} | "
                          f"{'✅' if t['ok'] else '❌'} | {motivos} |")
    linhas += ["", "> Gerado por `scripts/validate_cheatsheet.py` — não editar à mão."]
    saida.write_text("\n".join(linhas) + "\n", encoding="utf-8")
    return taxa


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", default=os.environ.get("VBL_LLM_URL",
                          "http://127.0.0.1:8000/v1"))
    parser.add_argument("--model", default=os.environ.get("VBL_LLM_MODEL",
                          "qwen3-4b"))
    parser.add_argument("--api-key-env", default="LOCAL_VLLM_KEY")
    parser.add_argument("--banco", default="docs/CHEATSHEET-PROMPTS.yaml")
    parser.add_argument("--cheatsheet", default="docs/VBL-CHEATSHEET.md")
    parser.add_argument("--saida", default="docs/CHEATSHEET-VALIDACAO.md")
    parser.add_argument("--execucoes", type=int, default=None,
                        help="sobrepõe execucoes_por_prompt do banco")
    parser.add_argument("--temperatura", type=float, default=0.2)
    parser.add_argument("--max-tokens", type=int, default=512)
    parser.add_argument("--salvar-respostas", default=".cheatsheet-respostas.jsonl",
                        help="JSONL com as respostas brutas para triagem do GQT")
    args = parser.parse_args()

    import yaml  # PyYAML (requirements-dev.txt)
    banco = yaml.safe_load(abrir_caminho(args.banco))
    cheatsheet = abrir_caminho(args.cheatsheet)
    execucoes_por_prompt = args.execucoes or banco["execucoes_por_prompt"]

    api_key = os.environ.get(args.api_key_env) or None
    print(f"validando {len(banco['prompts'])} prompts × "
          f"{execucoes_por_prompt} execuções contra {args.base_url} ({args.model})")

    brutas = open(args.salvar_respostas, "a", encoding="utf-8") \
        if args.salvar_respostas else None
    resultados = []
    for prompt in banco["prompts"]:
        registro = {"id": prompt["id"], "tipo": prompt["tipo"],
                    "tentativas": []}
        for _ in range(execucoes_por_prompt):
            resposta = consultar(args.base_url, args.model, api_key,
                                 cheatsheet, prompt["enunciado"],
                                 args.temperatura, args.max_tokens)
            if brutas is not None:
                brutas.write(json.dumps({"id": prompt["id"],
                                         "resposta": resposta},
                                        ensure_ascii=False) + "\n")
                brutas.flush()
            avaliacao = avaliar(prompt, resposta)
            registro["tentativas"].append(avaliacao)
            marca = "✅" if avaliacao["ok"] else "❌"
            print(f"  {marca} {prompt['id']}: "
                  f"{'; '.join(avaliacao['motivos']) or 'ok'}")
        resultados.append(registro)
    if brutas is not None:
        brutas.close()

    taxa = gerar_relatorio(banco, resultados, args.base_url, args.model,
                           RAIZ / args.saida)
    print(f"taxa de aprovação: {taxa:.1%} — relatório em {args.saida}")
    return 0 if taxa >= banco["limiar_aceitacao"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
