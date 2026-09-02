#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""webui.py — servidor da UI (estático + ponte de métricas do Caderno).

Substitui o `python3 -m http.server` do `serve-local-llm.sh`: continua
servindo a RAIZ do repositório (a UI precisa alcançar /docs/cheatsheet/VBL-CHEATSHEET.md)
e adiciona as rotas de métricas em tempo real do runtime VerboLang:

    GET /                    → redirect para /web/ (entrada do dashboard)
    GET /api/sources         → ledgers candidatos (logs/, tmp-logs/, caderno*)
    GET /api/snapshot?src=X  → agregados do Caderno X (JSON)
    GET /api/events?src=X    → fluxo SSE (snapshot + evento a evento)

Fontes aceitas (detecção por sufixo/magic):
  - binário `.vcad` — frames [u32 LE len][linha canônica][hash 32 B],
    docs/NOTEBOOK-FORMAT-v1.md §2; tolera frame parcial no fim (arquivo em
    crescimento durante um `vbl run`) e detecta o rodapé (execução concluída);
  - `.jsonl` exportado — um objeto JSON por linha.

Honestidade termodinâmica: a ponte é só uma LEITORA. A verificação da cadeia
SHA-256 permanece no `vbl ledger-verify` (agente externo — AGENTS §1.4); os
vocabulários v1 (PT) e v1.1 (EN) do campo `kind` são normalizados para leitura,
espelhando o verificador. Zero dependências externas (só stdlib).

Uso: python3 scripts/webui.py [PORTA] [--root DIR] [--poll S]
Parte do projeto VerboLang — licenciado sob a GNU GPL-3.0 (ver LICENSE).
"""

from __future__ import annotations

import argparse
import json
import struct
import time
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

# ── constantes ────────────────────────────────────────────────────────────
MAGIC = b"VCAD"
FOOTER_MAGIC = b"VFIM"
FOOTER_BYTES = 4 + 4 + 64          # magic + u32 eventos + head hex (64 ASCII)
HEADER_BYTES = 5                   # magic (4) + versão (1)
SEP = "\x1f"                       # separador da linha canônica (NOTEBOOK §3)
FEED_CAP = 300                     # eventos do replay inicial (ultimos)
HEARTBEAT_S = 15.0                 # comentário SSE mantendo a conexão viva

# vocabulário v1 (PT) → v1.1 (EN) — NOTEBOOK-FORMAT-v1.md, nota de versão:
# níveis (maiúsculos) e eventos (minúsculos); o verificador aceita os dois e
# produz estatísticas idênticas — a ponte espelha o mesmo mapa.
_KIND_PT_EN = {
    "VAZAMENTO": "LEAK",
    "LEITURA": "SENSOR_READ",
    "ALERTA": "ALERT",
    "SUBVERSAO": "SUBVERSION",
    "ATUACAO": "ACTUATION",
    "AVALIACAO": "ASSESSMENT",
    "COLAPSO": "COLLAPSE",
    "transicao": "transition",
    "persistence": "persistence",
    "subvert_aplicado": "subvert_applied",
    "keep_forma_inexistente": "keep_unknown_form",
    "keep_ignorado": "keep_ignored",
    "reclassify_sem_deadline": "reclassify_no_deadline",
    "ator_inexistente": "actor_unknown",
    "ator_indisponivel": "actor_unavailable",
    "fallback_executado": "fallback_executed",
    "sensor_nao_registrado": "sensor_not_registered",
    "sensor_inacessivel": "sensor_inaccessible",
}


# ── parsing ───────────────────────────────────────────────────────────────
def normalize_kind(kind: str) -> str:
    """Vocabulário do Caderno: v1 PT → v1.1 EN; desconhecidos passam intactos."""
    return _KIND_PT_EN.get(kind, kind)


def parse_canonical_line(linha: str, hash_hex: str) -> dict:
    """`seq␟kind␟msg[␟extra_json]` → objeto fundido (NOTEBOOK §3/§4)."""
    parts = linha.split(SEP)
    if len(parts) < 3:
        raise ValueError(f"linha canônica malformada: {linha[:60]!r}")
    ev: dict = {
        "seq": int(parts[0]),
        "kind": normalize_kind(parts[1]),
        "msg": parts[2],
    }
    if len(parts) >= 4:
        try:
            extra = json.loads(parts[3])
        except ValueError:
            extra = None
        if isinstance(extra, dict):
            ev.update(extra)
        else:
            ev["_extra_bruto"] = parts[3]
    ev["hash"] = hash_hex
    return ev


def parse_vcad(data: bytes) -> tuple[list[dict], bool]:
    """Binário `.vcad` completo → (eventos, rodapé_presente).

    Frame parcial no fim é descartado (o tailer o recupera no próximo
    poll, quando o runtime completar a gravação).
    """
    if not data:
        return [], False
    if data[:4] != MAGIC:
        raise ValueError("magic inválido (não é um .vcad)")
    eventos: list[dict] = []
    off = HEADER_BYTES
    total = len(data)
    while total - off >= 4:
        (ln,) = struct.unpack_from("<I", data, off)
        if total - off < 4 + ln + 32:
            break                                   # frame incompleto
        linha = data[off + 4:off + 4 + ln].decode("utf-8", "replace")
        hash_hex = data[off + 4 + ln:off + 4 + ln + 32].hex()
        eventos.append(parse_canonical_line(linha, hash_hex))
        off += 4 + ln + 32
    rodape_ok = total - off == FOOTER_BYTES and data[off:off + 4] == FOOTER_MAGIC
    return eventos, rodape_ok


def parse_jsonl(data: bytes) -> tuple[list[dict], bool]:
    """JSONL exportado → (eventos, True). Linha final incompleta é ignorada."""
    eventos: list[dict] = []
    for ln in data.decode("utf-8", "replace").splitlines():
        ln = ln.strip()
        if not ln:
            continue
        try:
            obj = json.loads(ln)
        except ValueError:
            continue
        if isinstance(obj, dict):
            obj["kind"] = normalize_kind(str(obj.get("kind", "INFO")))
            eventos.append(obj)
    return eventos, True


# ── agregados ─────────────────────────────────────────────────────────────
def aggregate(eventos: list[dict], completo: bool) -> dict:
    """Agregados do painel (a integridade da cadeia é papel do ledger-verify)."""
    agg = {
        "eventos": len(eventos),
        "por_kind": {},
        "joules": 0.0,
        "atuacoes": {"total": 0, "ok": 0},
        "sensores": {},
        "atores": {},
        "tick_max": 0,
        "seq": -1,
        "completo": completo,
    }
    for ev in eventos:
        kind = ev.get("kind", "INFO")
        agg["por_kind"][kind] = agg["por_kind"].get(kind, 0) + 1
        if kind == "LEAK":
            try:
                agg["joules"] += float(ev.get("joules", 0))
            except (TypeError, ValueError):
                pass
        elif kind == "ACTUATION":
            agg["atuacoes"]["total"] += 1
            ok = ev.get("sucesso") is True
            if ok:
                agg["atuacoes"]["ok"] += 1
            nome = ev.get("ator")
            if nome:
                agg["atores"][nome] = {
                    "valor": ev.get("aplicado", ev.get("valor")),
                    "sucesso": ok,
                    "tick": ev.get("tick", 0),
                }
        elif kind == "SENSOR_READ":
            nome = ev.get("sensor")
            if nome:
                agg["sensores"][nome] = {
                    "valor": ev.get("valor"),
                    "tick": ev.get("tick", 0),
                }
        tick = ev.get("tick")
        if isinstance(tick, (int, float)) and tick > agg["tick_max"]:
            agg["tick_max"] = tick
        if "seq" in ev:
            agg["seq"] = ev["seq"]
    return agg


# ── caminhos e fontes ─────────────────────────────────────────────────────
def resolve_src(root: Path, src: str) -> Path:
    """`src` da query → caminho dentro da raiz; ValueError se escapar."""
    if not src or Path(src).is_absolute() or ".." in Path(src).parts:
        raise ValueError(f"src inválido: {src!r}")
    raiz = Path(root).resolve()
    alvo = (raiz / src).resolve()
    try:
        alvo.relative_to(raiz)
    except ValueError:
        raise ValueError(f"src escapa da raiz: {src!r}") from None
    return alvo


def find_sources(root: Path) -> list[dict]:
    """Ledgers candidatos por mtime desc (logs/, tmp-logs/, caderno*.jsonl)."""
    raiz = Path(root)
    fontes: list[dict] = []
    vistos: set[Path] = set()
    padroes = ["logs/**/*.vcad*", "tmp-logs/**/*.vcad*", "caderno*.jsonl"]
    for padrao in padroes:
        for p in raiz.glob(padrao):
            if p.is_file() and p not in vistos:
                vistos.add(p)
                st = p.stat()
                fontes.append({
                    "caminho": p.relative_to(raiz).as_posix(),
                    "mtime": st.st_mtime,
                    "tamanho": st.st_size,
                })
    fontes.sort(key=lambda f: (-f["mtime"], f["caminho"]))
    return fontes


# ── SSE ───────────────────────────────────────────────────────────────────
def sse_frame(event: str, data: dict, id_seq: int | None = None) -> str:
    """Um frame SSE: `id:` opcional, `event:` e `data:` + linha em branco."""
    linhas = []
    if id_seq is not None:
        linhas.append(f"id: {id_seq}")
    linhas.append(f"event: {event}")
    linhas.append(f"data: {json.dumps(data, ensure_ascii=False)}")
    return "\n".join(linhas) + "\n\n"


# ── tailer ao vivo ────────────────────────────────────────────────────────
class SourceTailer:
    """Leitor incremental de um ledger (.vcad em crescimento ou .jsonl).

    poll() devolve só os eventos novos; snapshot() reúne agregados + feed
    (últimos FEED_CAP). Truncagem/reescrita (nova execução no mesmo caminho)
    reinicia o estado — o painel reflete o arquivo corrente, sem invenção.
    """

    def __init__(self, path: Path, feed_cap: int = FEED_CAP):
        self.path = Path(path)
        self.feed_cap = feed_cap
        self._reset()

    def _reset(self) -> None:
        self.offset = 0
        self.formato: str | None = None
        self.completo = False
        self.eventos: list[dict] = []
        self._buf = b""

    def snapshot(self) -> dict:
        snap = {
            "src": str(self.path),
            "exists": self.path.exists(),
            "formato": self.formato,
        }
        snap.update(aggregate(self.eventos, self.completo))
        snap["ultimos"] = self.eventos[-self.feed_cap:]
        return snap

    def poll(self) -> list[dict]:
        if not self.path.exists():
            if self.formato is not None:
                self._reset()
            return []
        data = self.path.read_bytes()
        if self.formato is None:
            self.formato = "jsonl" if self.path.suffix == ".jsonl" else "vcad"
        if self.offset > len(data):                 # truncado/reescrito
            formato = self.formato
            self._reset()
            self.formato = formato
        if self.formato == "jsonl":
            return self._poll_jsonl(data)
        return self._poll_vcad(data)

    def _poll_vcad(self, data: bytes) -> list[dict]:
        if len(data) < HEADER_BYTES or data[:4] != MAGIC:
            return []                               # header ainda não gravado
        novos: list[dict] = []
        off = max(self.offset, HEADER_BYTES)
        total = len(data)
        while total - off >= 4:
            (ln,) = struct.unpack_from("<I", data, off)
            if total - off < 4 + ln + 32:
                break                               # frame parcial (flush pendente)
            linha = data[off + 4:off + 4 + ln].decode("utf-8", "replace")
            hash_hex = data[off + 4 + ln:off + 4 + ln + 32].hex()
            novos.append(parse_canonical_line(linha, hash_hex))
            off += 4 + ln + 32
        if total - off == FOOTER_BYTES and data[off:off + 4] == FOOTER_MAGIC:
            self.completo = True                    # execução concluída
            off = total
        self.offset = off
        self.eventos.extend(novos)
        return novos

    def _poll_jsonl(self, data: bytes) -> list[dict]:
        self._buf += data[self.offset:]
        self.offset = len(data)
        novos: list[dict] = []
        if b"\n" in self._buf:
            completo, _, self._buf = self._buf.rpartition(b"\n")
            for ln in completo.split(b"\n"):
                if not ln.strip():
                    continue
                try:
                    obj = json.loads(ln)
                except ValueError:
                    continue                        # linha parcial/corrompida
                if isinstance(obj, dict):
                    obj["kind"] = normalize_kind(str(obj.get("kind", "INFO")))
                    novos.append(obj)
        self.eventos.extend(novos)
        self.completo = True                        # exportado = execução finda
        return novos


# ── servidor HTTP ─────────────────────────────────────────────────────────
class WebUIHandler(SimpleHTTPRequestHandler):
    """Estático na raiz do repo + rotas /api/* (JSON e SSE)."""

    raiz: Path
    poll_s: float = 0.4

    def log_message(self, *args) -> None:           # silencioso (local, loopback)
        pass

    # -- helpers ------------------------------------------------------------
    def _send_json(self, obj, status: int = 200) -> None:
        corpo = json.dumps(obj, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(corpo)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(corpo)

    def _send_sse_headers(self) -> None:
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream; charset=utf-8")
        self.send_header("Cache-Control", "no-store")
        self.send_header("Connection", "close")
        self.close_connection = True
        self.end_headers()

    # -- rotas --------------------------------------------------------------
    def do_GET(self) -> None:
        parsed = urlsplit(self.path)
        if parsed.path == "/":
            self.send_response(302)
            self.send_header("Location", "/web/")
            self.send_header("Content-Length", "0")
            self.end_headers()
        elif parsed.path == "/api/sources":
            self._send_json({"fontes": find_sources(self.raiz)})
        elif parsed.path == "/api/snapshot":
            self._rota_snapshot(parsed)
        elif parsed.path == "/api/events":
            self._rota_events(parsed)
        else:
            super().do_GET()                        # estático (raiz do repo)

    def _src_da_query(self, parsed) -> Path:
        src = parse_qs(parsed.query).get("src", [""])[0]
        return resolve_src(self.raiz, src)          # ValueError → 400

    def _rota_snapshot(self, parsed) -> None:
        try:
            alvo = self._src_da_query(parsed)
        except ValueError as e:
            self._send_json({"erro": str(e)}, status=400)
            return
        tailer = SourceTailer(alvo)
        tailer.poll()
        self._send_json(tailer.snapshot())

    def _rota_events(self, parsed) -> None:
        try:
            alvo = self._src_da_query(parsed)
        except ValueError as e:
            self._send_json({"erro": str(e)}, status=400)
            return
        self._send_sse_headers()
        tailer = SourceTailer(alvo)
        try:
            tailer.poll()
            self.wfile.write(sse_frame("snapshot", tailer.snapshot()).encode("utf-8"))
            self.wfile.flush()
            completo_enviado = tailer.completo
            ultimo_beat = time.monotonic()
            while True:
                time.sleep(self.poll_s)
                novos = tailer.poll()
                for ev in novos:
                    frame = sse_frame("ev", ev, id_seq=ev.get("seq"))
                    self.wfile.write(frame.encode("utf-8"))
                if novos or tailer.completo != completo_enviado:
                    completo_enviado = tailer.completo
                    self.wfile.write(sse_frame("snapshot", tailer.snapshot()).encode("utf-8"))
                agora = time.monotonic()
                if agora - ultimo_beat >= HEARTBEAT_S:
                    self.wfile.write(b": ping\n\n")
                    ultimo_beat = agora
                self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError, OSError):
            return                                  # cliente foi embora


def make_server(root: Path, port: int = 8188, poll: float = 0.4) -> ThreadingHTTPServer:
    """Servidor no loopback (127.0.0.1) servindo `root` + rotas /api/*."""
    raiz = Path(root).resolve()

    class Handler(WebUIHandler):
        def __init__(self, *args, **kwargs):
            super().__init__(*args, directory=str(raiz), **kwargs)

    Handler.raiz = raiz
    Handler.poll_s = poll

    srv = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    srv.daemon_threads = True
    return srv


def main(argv=None) -> None:
    ap = argparse.ArgumentParser(description="UI VerboLang: estático + métricas do Caderno (SSE)")
    ap.add_argument("porta", nargs="?", type=int, default=8188, help="porta (default 8188)")
    ap.add_argument("--root", default=None, help="raiz servida (default: raiz do repositório)")
    ap.add_argument("--poll", type=float, default=0.4, help="intervalo de leitura do ledger (s)")
    args = ap.parse_args(argv)
    root = Path(args.root).resolve() if args.root else Path(__file__).resolve().parents[1]
    srv = make_server(root, args.porta, args.poll)
    print(f"webui: http://127.0.0.1:{args.porta}/  (raiz {root}; métricas em /web/)")
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        srv.server_close()


if __name__ == "__main__":
    main()
