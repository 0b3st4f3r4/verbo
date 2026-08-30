# -*- coding: utf-8 -*-
"""
================================================================================
       POC: COMUNICAÇÃO ENTRE LLMs VIA VERBOLANG
================================================================================
Dois agentes LLM (Proponente ↔ Crítico) conversam sob o runtime VerboLang do
blueprint de referência (verbolang-complete-blueprint.py, carregado via
importlib — o mesmo motor, zero duplicação):

  - Cada agente é um ATOR no FXP: act(Agente, mensagem) → POST /v1/chat/completions
  - O estado da conversa são SENSORES numéricos no registro do FXP:
      dialogo_turns, dialogo_tokens, dialogo_latency_ms, dialogo_loop_risk
  - A conversa é uma forma `nonequilibrium`: exige keep() a cada tick (senão
    colapsa) e tem horizon de 8s (Alívio Termodinâmico ao esgotar)
  - Regras de revisão:
      when dialogo_loop_risk > 0.85  -> subvert          (§4.5: repetição sem propósito)
      when dialogo_tokens   > 2500   -> notify_shutdown  (orçamento estourado)
  - O Caderno registra turnos, tokens, latências, Joules ESTIMADOS (latência ×
    potência estimada da GPU — não é medição RAPL) e a cadeia SHA-256

Topologia "local primeiro": ambos os agentes usam o vLLM local por padrão.
Para mover um agente para outro nó (ex.: GLM cloud), defina no AGENTES[nome]
"base_url", "model" e "api_key_env" — cada agente pode apontar para um
endpoint diferente, sem tocar no runtime.

Requisitos: Python 3.10+ (apenas stdlib) e vLLM local ativo
(scripts/serve-local-llm.sh), com LOCAL_VLLM_KEY no ambiente ou em
~/.dsh/.credentials.yaml.

Uso:    python3 prototype/verbolang-llm-poc.py
        VBL_TICKS=20 python3 prototype/verbolang-llm-poc.py   # diálogo mais longo
================================================================================
"""

import hashlib
import importlib.util
import json
import os
import time
import urllib.error
import urllib.request

# --- Carrega o runtime VerboLang (blueprint de referência) -------------------
_HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location(
    "verbolang_blueprint", os.path.join(_HERE, "verbolang-complete-blueprint.py")
)
vbl = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(vbl)

# --- Configuração do barramento LLM ------------------------------------------
BASE_URL = os.environ.get("VBL_BASE_URL", "http://127.0.0.1:8000/v1")
MODEL = os.environ.get("VBL_MODEL", "qwen3-4b")
MAX_TICKS = int(os.environ.get("VBL_TICKS", "10"))
TEMPERATURA = float(os.environ.get("VBL_TEMPERATURE", "0.7"))  # baixa (ex.: 0.15) → respostas convergem → subvert
HORIZON = float(os.environ.get("VBL_HORIZON", "8.0"))          # alto (ex.: 40) → dá tempo de cruzar o limiar de loop
SEM_KEEP = os.environ.get("VBL_SEM_KEEP", "") != ""            # "1" → conversa abandonada → colapso (§4.1)
MAX_TOKENS = int(os.environ.get("VBL_MAX_TOKENS", "120"))  # modelos com reasoning podem exigir mais (ex.: 512)
GPU_WATTS_ESTIMADO = 80.0  # estimativa de drenagem da GPU em inferência (NÃO é RAPL)
LIMITE_LOOP = float(os.environ.get("VBL_LOOP_LIMITE", "0.85"))  # §4.5: repetição sem propósito
LIMITE_TOKENS = 2500       # orçamento total do diálogo
EMBED_URL = os.environ.get("VBL_EMBED_URL")   # nó de embeddings (ex.: http://127.0.0.1:8002/v1); ausente → hashing
EMBED_MODEL = os.environ.get("VBL_EMBED_MODEL", "qwen3-embedding-0.6b")
SEMENTE = (
    "Proponha um teste prático de honestidade termodinâmica para um runtime "
    "que orquestra múltiplos LLMs. Seja concreto e breve."
)

AGENTES = {
    "Proponente": {
        "system": (
            "Você é o Proponente em um diálogo estruturado. Responda em no "
            "máximo 2 frases, propondo UMA ideia prática relacionada ao que "
            "recebeu. Nunca repita frases anteriores."
        ),
    },
    "Critico": {
        "system": (
            "Você é o Crítico em um diálogo estruturado. Responda em no máximo "
            "2 frases: aponte UM risco material (energia, custo, prazo) da "
            "proposta recebida e uma condição objetiva para aceitá-la."
        ),
    },
}
ORDEM = ["Proponente", "Critico"]  # quem fala em cada turno (alternado)
DIALOGO = "DialogoVerboLang"


def _api_key(nome_env: str | None = None) -> str | None:
    """Chave do ambiente ou de ~/.dsh/.credentials.yaml (mesmo contrato do
    scripts/serve-local-llm.sh)."""
    k = os.environ.get(nome_env or "LOCAL_VLLM_KEY")
    if k:
        return k
    cred = os.path.join(
        os.environ.get("DSH_HOME", os.path.expanduser("~/.dsh")), ".credentials.yaml"
    )
    alvo = (nome_env or "LOCAL_VLLM_KEY") + ":"
    try:
        with open(cred, encoding="utf-8") as f:
            for line in f:
                if line.startswith(alvo):
                    return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return None


# ==============================================================================
# DETECTOR DE REPETIÇÃO: similaridade por embeddings, com fallback honesto
# ==============================================================================
def _cosine(a: list[float], b: list[float]) -> float:
    num = sum(x * y for x, y in zip(a, b))
    na = sum(x * x for x in a) ** 0.5
    nb = sum(x * x for x in b) ** 0.5
    return num / (na * nb) if na and nb else 0.0


def _hash_embed(texto: str, dim: int = 512) -> list[float]:
    """Embedding de hashing (100% stdlib): n-gramas de caracteres 3–5, com
    sinal, projetados em vetor L2-normalizado. Robusto a reordenação e
    reescrevas lexicais; não capta sinônimos profundos — é o fallback quando
    não há nó de embeddings, e o Caderno registra qual método está ativo."""
    vec = [0.0] * dim
    t = " ".join(texto.lower().split())
    for n in (3, 4, 5):
        for i in range(max(0, len(t) - n + 1)):
            h = int.from_bytes(hashlib.md5(t[i:i + n].encode()).digest()[:8], "big")
            vec[h % dim] += 1.0 if (h >> 63) & 1 else -1.0
    norm = sum(x * x for x in vec) ** 0.5
    return [x / norm for x in vec] if norm else vec


class DetectorRepeticao:
    """Similaridade por embeddings: usa /v1/embeddings (OpenAI-compatível) se
    o nó expuser; senão, hashing de n-gramas. O método ativo é registrado no
    Caderno — honestidade sobre como cada loop_risk foi medido."""

    def __init__(self):
        self.metodo = "hashing-ngramas (stdlib)"
        self._http_ok = bool(EMBED_URL) and self._probe()
        if self._http_ok:
            self.metodo = f"embeddings HTTP · {EMBED_MODEL}"
        vbl.Caderno.info(f"Detector de repetição: {self.metodo}")

    def _probe(self) -> bool:
        try:
            self._embed_http("probe")
            return True
        except Exception:
            return False

    def _embed_http(self, texto: str) -> list[float]:
        req = urllib.request.Request(
            EMBED_URL.rstrip("/") + "/embeddings",
            data=json.dumps({"model": EMBED_MODEL, "input": texto}).encode("utf-8"),
            headers={
                "Content-Type": "application/json",
                "Authorization": f"Bearer {_api_key()}",
            },
        )
        with urllib.request.urlopen(req, timeout=10) as r:
            return json.loads(r.read().decode("utf-8"))["data"][0]["embedding"]

    def embed(self, texto: str) -> list[float]:
        return self._embed_http(texto) if self._http_ok else _hash_embed(texto)


# ==============================================================================
# FXP-LLM: o barramento de I/O cujos sensores e atores são LLMs
# ==============================================================================
class FXPLLm(vbl.FXP):
    """Herda o FXP do blueprint; atores = agentes LLM, sensores = métricas do diálogo."""

    def __init__(self):
        super().__init__()
        self.turn_no = 0
        self.tokens_total = 0
        self.tokens_por_agente: dict[str, int] = {n: 0 for n in ORDEM}
        self.latency_ms = 0.0
        self.loop_risk = 0.0
        self.watts_estimados = 0.0
        self.last_msg: str | None = None
        self.ultima_resposta: dict[str, str] = {}   # último texto por agente (para o sumário)
        self.ultimo_embed: dict[str, list[float]] = {}
        self.detector = DetectorRepeticao()
        self.cpu_power = 0.0

        # Sensores numéricos da conversa (avaliáveis pelas regras `when`)
        self.sensors.update(
            {
                "dialogo_turns": lambda: float(self.turn_no),
                "dialogo_tokens": lambda: float(self.tokens_total),
                "dialogo_latency_ms": lambda: self.latency_ms,
                "dialogo_loop_risk": lambda: self.loop_risk,
            }
        )
        # Atores: um por agente LLM (valor = mensagem de entrada)
        for nome in ORDEM:
            self.actors[nome] = {
                "min": None,
                "max": None,
                "safety_limit": None,
                "current": "",
                "apply": (lambda msg, _n=nome: self._falar(_n, msg)),
                "description": f"Agente LLM '{nome}'",
            }

    def update_hardware_state(self):
        """Sem hardware simulado neste PoC: o 'mundo' são os LLMs. O custo
        material do tick é a energia estimada da última chamada (J = W × s)."""
        self.cpu_power = self.watts_estimados

    # ---------- Atuação: fala com um agente LLM -------------------------------
    def _falar(self, agente: str, mensagem: str) -> str:
        cfg = AGENTES[agente]
        base = cfg.get("base_url", BASE_URL).rstrip("/")
        model = cfg.get("model", MODEL)
        key = _api_key(cfg.get("api_key_env"))
        if not key:
            vbl.Caderno.alert(f"Chave de API ausente para '{agente}' (falha de I/O).")
            return ""

        payload = {
            "model": model,
            "messages": [
                {"role": "system", "content": cfg["system"]},
                {"role": "user", "content": mensagem[-900:]},
            ],
            "max_tokens": MAX_TOKENS,
            "temperature": TEMPERATURA,
        }
        req = urllib.request.Request(
            base + "/chat/completions",
            data=json.dumps(payload).encode("utf-8"),
            headers={
                "Content-Type": "application/json",
                "Authorization": f"Bearer {key}",
            },
        )
        t0 = time.perf_counter()
        try:
            with urllib.request.urlopen(req, timeout=90) as r:
                resp = json.loads(r.read().decode("utf-8"))
        except (urllib.error.URLError, TimeoutError, json.JSONDecodeError, KeyError) as e:
            vbl.Caderno.alert(f"Falha de I/O chamando o agente '{agente}': {e}")
            return ""
        dt = time.perf_counter() - t0

        msg = resp["choices"][0]["message"]
        texto = (msg.get("content") or "").strip()
        if not texto:
            vbl.Caderno.alert(
                f"'{agente}' respondeu vazio (o reasoning consumiu o orçamento de tokens?)."
            )
        tokens = int(resp.get("usage", {}).get("total_tokens", 0))

        self.turn_no += 1
        self.tokens_total += tokens
        self.tokens_por_agente[agente] += tokens
        self.latency_ms = dt * 1000.0
        self.watts_estimados = GPU_WATTS_ESTIMADO * dt  # J do tick ≈ energia da chamada

        emb = self.detector.embed(texto)
        prev_emb = self.ultimo_embed.get(agente)
        self.loop_risk = _cosine(prev_emb, emb) if prev_emb else 0.0
        self.ultimo_embed[agente] = emb
        self.ultima_resposta[agente] = texto
        self.last_msg = texto

        vbl.Caderno.info(
            f"turno {self.turn_no:02d} · {agente} · {tokens} tokens · "
            f"{self.latency_ms:.0f} ms · loop_risk={self.loop_risk:.2f}"
        )
        return texto

    # ---------- Um turno completo do diálogo ----------------------------------
    def llm_turn(self, engine) -> None:
        if DIALOGO not in engine.forms:
            return
        falante = ORDEM[self.turn_no % 2]
        entrada = self.last_msg if self.last_msg else SEMENTE
        ok = self.act(falante, entrada)
        vbl.Caderno.actuator_action(
            falante, f"entrada de {len(entrada)} chars", ok
        )
        texto = self.last_msg or ""
        cor = vbl.CYAN if falante == "Proponente" else vbl.YELLOW
        print(f"{cor}{vbl.BOLD}[{falante}]{vbl.RESET} {texto}")


# --- Extensão do bloco `main`: statement `llm_turn` ---------------------------
class MainLLM(vbl.MainInterpreter):
    def _run_statement(self, st: dict):
        if st.get("statement") == "llm_turn":
            self.engine.fxp.llm_turn(self.engine)
        else:
            super()._run_statement(st)


# ==============================================================================
# MONTAGEM E EXECUÇÃO
# ==============================================================================
def main():
    # Override global de topologia (ex.: GLM-5.3-Flash dos dois lados):
    #   VBL_AGENTE_URL=https://api.z.ai/api/coding/paas/v4 \
    #   VBL_AGENTE_MODEL=glm-5.3-flash VBL_AGENTE_KEY_ENV=ZAI_API_KEY \
    #   VBL_MAX_TOKENS=512 python3 prototype/verbolang-llm-poc.py
    if os.environ.get("VBL_AGENTE_URL"):
        for cfg in AGENTES.values():
            cfg["base_url"] = os.environ["VBL_AGENTE_URL"]
            cfg["model"] = os.environ.get("VBL_AGENTE_MODEL", MODEL)
            if os.environ.get("VBL_AGENTE_KEY_ENV"):
                cfg["api_key_env"] = os.environ["VBL_AGENTE_KEY_ENV"]

    # Override por agente (topologia mista): VBL_PROPONENTE_URL / VBL_CRITICO_URL…
    for nome, cfg in AGENTES.items():
        prefixo = "VBL_" + nome.upper() + "_"
        if os.environ.get(prefixo + "URL"):
            cfg["base_url"] = os.environ[prefixo + "URL"]
            cfg["model"] = os.environ.get(prefixo + "MODEL", MODEL)
            if os.environ.get(prefixo + "KEY_ENV"):
                cfg["api_key_env"] = os.environ[prefixo + "KEY_ENV"]

    alvo = AGENTES[ORDEM[0]].get("base_url", BASE_URL)
    chave_env = AGENTES[ORDEM[0]].get("api_key_env")
    if _api_key(chave_env) is None:
        print(f"{chave_env or 'LOCAL_VLLM_KEY'} não encontrada (env ou ~/.dsh/.credentials.yaml)")
        raise SystemExit(1)
    try:
        req = urllib.request.Request(
            alvo.rstrip("/") + "/models",
            headers={"Authorization": f"Bearer {_api_key(chave_env)}"},
        )
        with urllib.request.urlopen(req, timeout=8):
            pass
    except urllib.error.HTTPError as e:
        if e.code in (401, 403):
            print(f"Autenticação recusada por {alvo} (HTTP {e.code}) — confira a chave.")
            raise SystemExit(1)
        # 404/405: o endpoint não publica /models, mas está alcançável — segue.
    except OSError as e:
        print(f"Endpoint inacessível em {alvo}: {e}")
        print("Local: bash scripts/serve-local-llm.sh")
        raise SystemExit(1)

    engine = vbl.VerboLangEngine()
    engine.fxp = FXPLLm()  # substitui o FXP de hardware pelo barramento LLM

    # Bloco `main`: keep() da conversa (a menos que o teste abandone-a) + um turno por tick
    main_block = MainLLM(engine)
    statements = [] if SEM_KEEP else [{"statement": "keep", "form": DIALOGO}]
    statements.append({"statement": "llm_turn"})
    main_block.add_every(1.0, statements)

    # A conversa é uma forma nonequilibrium com regras de revisão
    dialogo = vbl.NonequilibriumForm(
        name=DIALOGO,
        value="dialogo_proponente_critico",
        horizon=HORIZON,  # Alívio Termodinâmico ao esgotar
        source_path="dialogo_turns",
        maintenance_deadline=3.0,
        exchange_mode="cooperation",
        current_time=engine.sim_time,
    )
    dialogo.add_review_condition(
        "dialogo_loop_risk", ">", LIMITE_LOOP, [{"action": "subvert"}]
    )
    dialogo.add_review_condition(
        "dialogo_tokens", ">", LIMITE_TOKENS, [{"action": "notify_shutdown"}]
    )
    engine.register_form(dialogo)

    modo = "SEM keep (teste de colapso)" if SEM_KEEP else "keep a cada tick"
    desc = " · ".join(f"{n}={AGENTES[n].get('model', MODEL)}" for n in ORDEM)
    vbl.Caderno.info(
        f"Barramento LLM: {desc} · max_tokens {MAX_TOKENS} · "
        f"horizon {HORIZON}s · temp {TEMPERATURA} · {modo} · "
        f"subvert se loop_risk > {LIMITE_LOOP}"
    )
    print(f"\n{vbl.BOLD}SEMENTE:{vbl.RESET} {SEMENTE}\n")

    for _ in range(MAX_TICKS):
        if DIALOGO not in engine.forms:
            break
        main_block.run_due()
        engine.tick()
        time.sleep(0.2)

    fxp = engine.fxp
    print("\n" + "=" * 60)
    print(f"Turnos realizados: {fxp.turn_no}")
    print(f"Tokens totais: {fxp.tokens_total} {fxp.tokens_por_agente}")
    lat = [f"{n}: {fxp.ultima_resposta.get(n, '')[:40]}…" for n in ORDEM]
    print(f"loop_risk final: {fxp.loop_risk:.2f} (limite {LIMITE_LOOP})")
    print(f"Últimas respostas: {' | '.join(lat)}")
    print(f"Formas restantes: {list(engine.forms) or 'nenhuma (diálogo dissolvido)'}")
    integro = vbl.Caderno.verify_chain()
    estado = "ÍNTEGRO" if integro else "CORROMPIDO"
    print(
        f"Caderno {estado} — cabeça: {vbl.Caderno.chain_head()[:16]}… · "
        f"{len(vbl.Caderno._events)} eventos"
    )
    caminho = vbl.Caderno.export_jsonl("caderno_llm_poc.jsonl")
    print(f"Log exportado: {caminho}")


if __name__ == "__main__":
    main()
