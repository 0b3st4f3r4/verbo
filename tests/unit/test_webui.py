# -*- coding: utf-8 -*-
"""Testes da ponte de métricas (scripts/webui.py).

Contrato definido ANTES da implementação (AGENTS §2 — regra de ouro:
testes primeiro). A ponte é uma leitora honesta do Caderno de produção:
parser do binário `.vcad` (frames [u32 len][linha canônica][hash 32B],
docs/NOTEBOOK-FORMAT-v1.md §2) e do JSONL exportado, agregados, SSE e
resolução segura de caminhos. A verificação da cadeia SHA-256 continua
sendo papel do `vbl ledger-verify` — a ponte NÃO valida a corrente.

O parser aqui é testado contra fixtures construídas de forma independente
(a partir da spec §2), não contra o código em teste.
"""

from __future__ import annotations

import hashlib
import http.client
import json
import struct
import threading
import time
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys_scripts = str(ROOT / "scripts")
if sys_scripts not in __import__("sys").path:
    __import__("sys").path.insert(0, sys_scripts)

import webui  # noqa: E402

SEP = "\x1f"  # separador da linha canônica (NOTEBOOK-FORMAT §3)
HASH_ZERO = "0" * 64


# ── fixtures independentes (spec NOTEBOOK-FORMAT-v1.md §2) ────────────────
def linha_canonica(seq, kind, msg, extra=None):
    linha = f"{seq}{SEP}{kind}{SEP}{msg}"
    if extra:
        linha += SEP + json.dumps(extra, sort_keys=True, separators=(",", ":"))
    return linha


def frame_bin(linha):
    b = linha.encode("utf-8")
    h = hashlib.sha256(b).digest()
    return struct.pack("<I", len(b)) + b + h


def vcad_bytes(frames, footer=True, count=None):
    data = b"VCAD\x01" + b"".join(frames)
    if footer:
        data += b"VFIM" + struct.pack("<I", count if count is not None else len(frames)) + b"0" * 64
    return data


EVENTOS_DEMO = [
    (0, "INFO", "Forma 'Demo' conjugada no sistema.", {"forma": "Demo", "tick": 0, "t": 0}),
    (1, "LEAK", "Forma 'Demo' dissipou 150.00 Joules (150.00 W por 1.00s)",
     {"forma": "Demo", "joules": 150, "watts": 150, "segundos": 1, "tick": 1, "t": 1}),
    (2, "SENSOR_READ", "Sensor 'cpu_temp' = 55", {"sensor": "cpu_temp", "valor": 55, "tick": 1, "t": 1}),
    (3, "ATUACAO", "Ator 'CpuPowerCap' <- 50 (aplicado: 50, sucesso)",
     {"ator": "CpuPowerCap", "valor": 50, "aplicado": 50, "sucesso": True, "tick": 2, "t": 2}),
    (4, "LEAK", "Forma 'Demo' dissipou 50.00 Joules (50.00 W por 1.00s)",
     {"forma": "Demo", "joules": 50, "watts": 50, "segundos": 1, "tick": 3, "t": 3}),
    (5, "SENSOR_READ", "Sensor 'cpu_temp' = 61.5", {"sensor": "cpu_temp", "valor": 61.5, "tick": 3, "t": 3}),
    (6, "ATUACAO", "Ator 'Ventoinha' <- True (falhou)",
     {"ator": "Ventoinha", "valor": True, "sucesso": False, "tick": 4, "t": 4}),
    (8, "SUBVERSION", "Subversão: limite térmico superado", {"forma": "Demo", "tick": 5, "t": 5}),
]


def eventos_em_bytes(eventos, footer=True):
    return vcad_bytes([frame_bin(linha_canonica(*e)) for e in eventos], footer=footer)


def jsonl_bytes(eventos, final_newline=True):
    objs = []
    for seq, kind, msg, extra in eventos:
        obj = {"seq": seq, "kind": kind, "msg": msg, "hash": HASH_ZERO, **extra}
        objs.append(json.dumps(obj, sort_keys=True, separators=(",", ":")))
    texto = "\n".join(objs)
    return (texto + "\n").encode() if final_newline else texto.encode()


# ── 1. vocabulário: v1 PT → v1.1 EN ───────────────────────────────────────
def test_normaliza_vocabulario_pt_para_en():
    assert webui.normalize_kind("VAZAMENTO") == "LEAK"
    assert webui.normalize_kind("LEITURA") == "SENSOR_READ"
    assert webui.normalize_kind("ATUACAO") == "ACTUATION"
    assert webui.normalize_kind("SUBVERSAO") == "SUBVERSION"
    assert webui.normalize_kind("ALERTA") == "ALERT"
    assert webui.normalize_kind("AVALIACAO") == "ASSESSMENT"
    assert webui.normalize_kind("COLAPSO") == "COLLAPSE"


def test_mantem_kinds_v11_e_desconhecidos():
    assert webui.normalize_kind("LEAK") == "LEAK"
    assert webui.normalize_kind("INFO") == "INFO"
    assert webui.normalize_kind("subvert_applied") == "subvert_applied"
    assert webui.normalize_kind("KIND_NOVO_FUTURO") == "KIND_NOVO_FUTURO"


def test_normaliza_eventos_lower_pt_para_en():
    # nota de versão v1.1 do NOTEBOOK-FORMAT: o verificador aceita os dois
    # vocabulários e produz estatísticas idênticas — a ponte espelha isso.
    pares = {
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
    for pt, en in pares.items():
        assert webui.normalize_kind(pt) == en, pt


# ── 2. parser do binário .vcad ────────────────────────────────────────────
def test_parse_vcad_completo_com_rodape():
    data = eventos_em_bytes(EVENTOS_DEMO, footer=True)
    eventos, rodape_ok = webui.parse_vcad(data)
    assert rodape_ok is True
    assert len(eventos) == len(EVENTOS_DEMO)
    e1 = eventos[1]
    assert e1["seq"] == 1
    assert e1["kind"] == "LEAK"          # vocabulário PT normalizado
    assert e1["joules"] == 150
    assert e1["tick"] == 1
    assert e1["forma"] == "Demo"
    # hash = hex dos 32 bytes crus do frame
    esperado = hashlib.sha256(linha_canonica(*EVENTOS_DEMO[1]).encode()).hexdigest()
    assert e1["hash"] == esperado


def test_parse_vcad_sem_rodape_marca_incompleto():
    data = eventos_em_bytes(EVENTOS_DEMO, footer=False)
    eventos, rodape_ok = webui.parse_vcad(data)
    assert rodape_ok is False
    assert len(eventos) == len(EVENTOS_DEMO)


def test_parse_vcad_descarta_frame_parcial():
    data = eventos_em_bytes(EVENTOS_DEMO, footer=False)
    completo = eventos_em_bytes(EVENTOS_DEMO[:4], footer=False)
    parcial = completo + data[len(completo):len(completo) + 17]  # 17 B de um frame maior
    eventos, _ = webui.parse_vcad(parcial)
    assert [e["seq"] for e in eventos] == [0, 1, 2, 3]


def test_parse_vcad_rejeita_magic_invalido():
    with pytest.raises(ValueError):
        webui.parse_vcad(b"NAOX\x01" + b"\x00" * 16)


def test_parse_vcad_vazio():
    assert webui.parse_vcad(b"") == ([], False)
    assert webui.parse_vcad(b"VCAD\x01") == ([], False)


def test_parse_vcad_extra_json_invalido_vira_campo_bruto():
    linha = f"7{SEP}INFO{SEP}msg{SEP}{{json quebrado"
    eventos, _ = webui.parse_vcad(vcad_bytes([frame_bin(linha)], footer=False))
    assert eventos[0]["_extra_bruto"] == "{json quebrado"


# ── 3. parser do JSONL exportado ──────────────────────────────────────────
def test_parse_jsonl_completo():
    eventos, rodape_ok = webui.parse_jsonl(jsonl_bytes(EVENTOS_DEMO))
    assert rodape_ok is True
    assert len(eventos) == len(EVENTOS_DEMO)
    assert eventos[2]["kind"] == "SENSOR_READ"
    assert eventos[2]["sensor"] == "cpu_temp"


def test_parse_jsonl_ignora_linha_final_incompleta():
    data = jsonl_bytes(EVENTOS_DEMO, final_newline=False) + b'\n{"seq": 99, "kind'
    eventos, _ = webui.parse_jsonl(data)
    assert [e["seq"] for e in eventos] == [e[0] for e in EVENTOS_DEMO]


# ── 4. agregados ──────────────────────────────────────────────────────────
def test_agregados_do_caderno_demo():
    eventos, _ = webui.parse_vcad(eventos_em_bytes(EVENTOS_DEMO, footer=True))
    agg = webui.aggregate(eventos, completo=True)
    assert agg["eventos"] == len(EVENTOS_DEMO)
    assert agg["joules"] == pytest.approx(200.0)          # 150 + 50 dos LEAK
    assert agg["atuacoes"] == {"total": 2, "ok": 1}       # ATUACAO normalizada
    assert agg["sensores"]["cpu_temp"] == {"valor": 61.5, "tick": 3}
    assert agg["atores"]["CpuPowerCap"] == {"valor": 50, "sucesso": True, "tick": 2}
    assert agg["atores"]["Ventoinha"]["sucesso"] is False
    assert agg["tick_max"] == 5
    assert agg["seq"] == 8
    assert agg["por_kind"]["LEAK"] == 2
    assert agg["por_kind"]["SENSOR_READ"] == 2
    assert agg["por_kind"]["ACTUATION"] == 2


def test_agregados_vazios():
    agg = webui.aggregate([], completo=False)
    assert agg["eventos"] == 0
    assert agg["joules"] == 0
    assert agg["seq"] == -1
    assert agg["tick_max"] == 0
    assert agg["sensores"] == {}


# ── 5. resolução segura de caminhos ───────────────────────────────────────
def test_resolve_src_aceita_caminho_interno():
    p = webui.resolve_src(ROOT, "logs/stage4/thermal-subversion.vcad")
    assert p == ROOT / "logs" / "stage4" / "thermal-subversion.vcad"


def test_resolve_src_rejeita_traversal_e_absoluto():
    with pytest.raises(ValueError):
        webui.resolve_src(ROOT, "../fora")
    with pytest.raises(ValueError):
        webui.resolve_src(ROOT, "logs/../../etc/passwd")
    with pytest.raises(ValueError):
        webui.resolve_src(ROOT, "/etc/passwd")
    with pytest.raises(ValueError):
        webui.resolve_src(ROOT, "")


# ── 6. seletor de fontes ──────────────────────────────────────────────────
def test_find_sources_lista_vcad_jsonl_e_caderno(tmp_path):
    (tmp_path / "logs" / "s4").mkdir(parents=True)
    (tmp_path / "tmp-logs").mkdir()
    (tmp_path / "logs" / "s4" / "demo.vcad").write_bytes(b"VCAD\x01")
    time.sleep(0.01)
    (tmp_path / "logs" / "s4" / "demo.vcad.jsonl").write_bytes(b"{}\n")
    time.sleep(0.01)
    (tmp_path / "tmp-logs" / "vivo.vcad").write_bytes(b"VCAD\x01")
    time.sleep(0.01)
    (tmp_path / "caderno_x.jsonl").write_bytes(b"{}\n")
    (tmp_path / "ignorado.txt").write_bytes(b"nao e ledger")

    fontes = webui.find_sources(tmp_path)
    caminhos = [f["caminho"] for f in fontes]
    # mtime desc: o mais novo primeiro
    assert caminhos[0] == "caderno_x.jsonl"
    assert "tmp-logs/vivo.vcad" in caminhos
    assert "logs/s4/demo.vcad.jsonl" in caminhos
    assert "logs/s4/demo.vcad" in caminhos
    assert all("ignorado.txt" != c for c in caminhos)
    assert set(fontes[0].keys()) == {"caminho", "mtime", "tamanho"}


# ── 7. frames SSE ─────────────────────────────────────────────────────────
def test_sse_frame_formato():
    s = webui.sse_frame("ev", {"seq": 3, "kind": "LEAK"}, id_seq=3)
    assert s == 'id: 3\nevent: ev\ndata: {"seq": 3, "kind": "LEAK"}\n\n'
    s2 = webui.sse_frame("snapshot", {"eventos": 0}, id_seq=None)
    assert s2.startswith("event: snapshot\ndata: ")


# ── 8. tailer ao vivo (crescimento, truncagem, formato jsonl) ────────────
def test_tailer_vcad_cresce_e_finaliza(tmp_path):
    alvo = tmp_path / "demo.vcad"
    alvo.write_bytes(eventos_em_bytes(EVENTOS_DEMO[:3], footer=False))
    t = webui.SourceTailer(alvo)
    novos = t.poll()
    assert [e["seq"] for e in novos] == [0, 1, 2]
    assert t.snapshot()["completo"] is False

    alvo.write_bytes(eventos_em_bytes(EVENTOS_DEMO, footer=True))  # append + rodapé
    novos = t.poll()
    assert [e["seq"] for e in novos] == [3, 4, 5, 6, 8]
    snap = t.snapshot()
    assert snap["completo"] is True
    assert snap["eventos"] == len(EVENTOS_DEMO)
    assert snap["ultimos"][-1]["seq"] == 8
    assert len(snap["ultimos"]) <= webui.FEED_CAP


def test_tailer_vcad_frame_parcial_no_flush(tmp_path):
    alvo = tmp_path / "demo.vcad"
    cheio = eventos_em_bytes(EVENTOS_DEMO, footer=False)
    alvo.write_bytes(cheio[: len(cheio) - 40])  # frame final incompleto
    t = webui.SourceTailer(alvo)
    assert [e["seq"] for e in t.poll()] == [e[0] for e in EVENTOS_DEMO[:-1]]
    alvo.write_bytes(cheio)  # runtime completa o frame
    assert [e["seq"] for e in t.poll()] == [EVENTOS_DEMO[-1][0]]


def test_tailer_reinicia_apos_truncagem(tmp_path):
    alvo = tmp_path / "demo.vcad"
    alvo.write_bytes(eventos_em_bytes(EVENTOS_DEMO, footer=True))
    t = webui.SourceTailer(alvo)
    t.poll()
    # nova execução no mesmo caminho: arquivo reescrito do zero
    novos = [(9, "INFO", "renasceu", {"tick": 0, "t": 0})]
    alvo.write_bytes(eventos_em_bytes(novos, footer=False))
    novos_ev = t.poll()
    assert [e["seq"] for e in novos_ev] == [9]
    assert t.snapshot()["eventos"] == 1


def test_tailer_jsonl(tmp_path):
    alvo = tmp_path / "demo.vcad.jsonl"
    alvo.write_bytes(jsonl_bytes(EVENTOS_DEMO[:2]))
    t = webui.SourceTailer(alvo)
    assert [e["seq"] for e in t.poll()] == [0, 1]
    assert t.snapshot()["formato"] == "jsonl"
    alvo.write_bytes(jsonl_bytes(EVENTOS_DEMO))
    assert [e["seq"] for e in t.poll()] == [e[0] for e in EVENTOS_DEMO[2:]]
    assert t.snapshot()["completo"] is True


def test_tailer_arquivo_ausente(tmp_path):
    t = webui.SourceTailer(tmp_path / "nem-existe.vcad")
    assert t.poll() == []
    snap = t.snapshot()
    assert snap["exists"] is False
    assert snap["eventos"] == 0


# ── 9. servidor HTTP (rotas estáticas + API) ──────────────────────────────
@pytest.fixture()
def servidor(tmp_path):
    """Servidor webui em porta efêmera com um repo mínimo em tmp_path."""
    (tmp_path / "web").mkdir()
    (tmp_path / "web" / "pagina.html").write_text("<html>ok</html>", encoding="utf-8")
    (tmp_path / "logs").mkdir()
    (tmp_path / "logs" / "demo.vcad").write_bytes(eventos_em_bytes(EVENTOS_DEMO, footer=True))

    srv = webui.make_server(tmp_path, port=0)
    th = threading.Thread(target=srv.serve_forever, daemon=True)
    th.start()
    yield tmp_path, srv.server_address[1]
    srv.shutdown()
    srv.server_close()


def requisicao(porta, caminho, metodo="GET"):
    conn = http.client.HTTPConnection("127.0.0.1", porta, timeout=5)
    conn.request(metodo, caminho)
    resp = conn.getresponse()
    corpo = resp.read()
    return resp, corpo, conn


def test_raiz_redireciona_para_web(servidor):
    _, porta, *_ = servidor
    resp, _, conn = requisicao(porta, "/")
    assert resp.status == 302
    assert resp.getheader("Location") == "/web/"
    conn.close()


def test_estatico_serve_arquivo(servidor):
    _, porta, *_ = servidor
    resp, corpo, conn = requisicao(porta, "/web/pagina.html")
    assert resp.status == 200
    assert corpo == b"<html>ok</html>"
    conn.close()


def test_api_sources(servidor):
    root, porta, *_ = servidor
    resp, corpo, conn = requisicao(porta, "/api/sources")
    assert resp.status == 200
    dados = json.loads(corpo)
    assert dados["fontes"][0]["caminho"] == "logs/demo.vcad"
    conn.close()


def test_api_snapshot_agregados(servidor):
    _, porta, *_ = servidor
    resp, corpo, conn = requisicao(porta, "/api/snapshot?src=logs/demo.vcad")
    assert resp.status == 200
    snap = json.loads(corpo)
    assert snap["exists"] is True
    assert snap["formato"] == "vcad"
    assert snap["completo"] is True
    assert snap["joules"] == pytest.approx(200.0)
    assert snap["atuacoes"] == {"total": 2, "ok": 1}
    assert snap["sensores"]["cpu_temp"]["valor"] == 61.5
    conn.close()


def test_api_snapshot_rejeita_src_inseguro(servidor):
    _, porta, *_ = servidor
    for src in ("../fora", "/etc/passwd", ""):
        resp, _, conn = requisicao(porta, f"/api/snapshot?src={src}")
        assert resp.status == 400, f"src={src!r} deveria ser 400"
        conn.close()


def test_api_snapshot_arquivo_ausente_existe_false(servidor):
    _, porta, *_ = servidor
    resp, corpo, conn = requisicao(porta, "/api/snapshot?src=logs/nem-existe.vcad")
    assert resp.status == 200
    assert json.loads(corpo)["exists"] is False
    conn.close()


# ── 10. SSE ao vivo: snapshot inicial, eventos novos, conclusão ──────────
def ler_frame(fp, limite=50):
    """Lê um frame SSE (linhas até a linha em branco)."""
    linhas = []
    for _ in range(limite):
        linha = fp.readline()
        if not linha:
            break
        if linha in (b"\n", b"\r\n"):
            if linhas:
                break
            continue
        linhas.append(linha.decode().strip())
    frame = {}
    for linha in linhas:
        campo, _, valor = linha.partition(": ")
        frame[campo] = valor
    return frame


def test_sse_fluxo_completo(tmp_path):
    (tmp_path / "logs").mkdir()
    alvo = tmp_path / "logs" / "vivo.vcad"
    alvo.write_bytes(eventos_em_bytes(EVENTOS_DEMO[:3], footer=False))

    srv = webui.make_server(tmp_path, port=0, poll=0.05)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    try:
        conn = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=5)
        conn.request("GET", "/api/events?src=logs/vivo.vcad")
        resp = conn.getresponse()
        assert resp.status == 200
        assert resp.getheader("Content-Type").startswith("text/event-stream")

        # 1º frame: snapshot com o histórico até agora
        f1 = ler_frame(resp.fp)
        assert f1["event"] == "snapshot"
        snap = json.loads(f1["data"])
        assert snap["eventos"] == 3
        assert snap["completo"] is False

        # runtime "grava" mais eventos + rodapé
        time.sleep(0.1)
        with open(alvo, "ab") as fh:
            fh.write(eventos_em_bytes(EVENTOS_DEMO, footer=True)[len(alvo.read_bytes()):])

        # frames subsequentes: ev's novos (com id) e snapshot atualizado
        vistos, snap_final = [], None
        prazo = time.time() + 5
        while time.time() < prazo and (8 not in vistos or snap_final is None):
            f = ler_frame(resp.fp)
            if not f:
                break
            if f.get("event") == "ev":
                ev = json.loads(f["data"])
                vistos.append(ev["seq"])
                assert "id" in f
            elif f.get("event") == "snapshot":
                snap_final = json.loads(f["data"])
        assert [3, 4, 5, 6, 8] == vistos
        assert snap_final is not None
        assert snap_final["completo"] is True
        assert snap_final["eventos"] == len(EVENTOS_DEMO)
        conn.close()
    finally:
        srv.shutdown()
        srv.server_close()


# ── 11. cobertura complementar: ramos de guarda e rotas de erro ───────────
def test_parse_canonical_line_malformada():
    with pytest.raises(ValueError):
        webui.parse_canonical_line("so-um-campo", HASH_ZERO)
    with pytest.raises(ValueError):
        webui.parse_canonical_line(f"1{SEP}LEAK", HASH_ZERO)  # 2 campos < 3


def test_parse_jsonl_ignora_linhas_vazias_e_nao_dict():
    data = b'\n{"seq": 1, "kind": "INFO", "msg": "ok"}\n\n[1,2]\n"texto"\n'
    eventos, rodape = webui.parse_jsonl(data)
    assert rodape is True
    assert [e["seq"] for e in eventos] == [1]


def test_aggregate_leak_com_joules_nao_numerico():
    eventos = [{"kind": "LEAK", "joules": "não-numérico", "seq": 1},
               {"kind": "LEAK", "seq": 2}]              # sem joules
    agg = webui.aggregate(eventos, completo=True)
    assert agg["joules"] == 0.0                         # TypeError/ValueError → ignora
    assert agg["por_kind"]["LEAK"] == 2


def test_resolve_src_rejeita_symlink_que_escapa(tmp_path, tmp_path_factory):
    fora = tmp_path_factory.mktemp("fora-da-raiz")      # fora da raiz servida
    (fora / "segredo.txt").write_text("x", encoding="utf-8")
    (tmp_path / "logs").mkdir()
    (tmp_path / "logs" / "atalho").symlink_to(fora)
    with pytest.raises(ValueError, match="escapa da raiz"):
        webui.resolve_src(tmp_path, "logs/atalho/segredo.txt")


def test_tailer_reinicia_se_arquivo_desaparece(tmp_path):
    alvo = tmp_path / "demo.vcad"
    alvo.write_bytes(eventos_em_bytes(EVENTOS_DEMO[:2], footer=False))
    t = webui.SourceTailer(alvo)
    t.poll()
    assert t.snapshot()["eventos"] == 2
    alvo.unlink()                                       # arquivo some
    assert t.poll() == []
    alvo.write_bytes(eventos_em_bytes(EVENTOS_DEMO[:1], footer=False))
    assert [e["seq"] for e in t.poll()] == [0]          # estado reiniciado


def test_tailer_vcad_header_ainda_nao_gravado(tmp_path):
    alvo = tmp_path / "demo.vcad"
    alvo.write_bytes(b"")                               # existe, mas vazio
    t = webui.SourceTailer(alvo)
    assert t.poll() == []
    alvo.write_bytes(b"VCAD")                           # header parcial (4 < 5 B)
    assert t.poll() == []


def test_tailer_jsonl_linha_vazia_e_corrompida(tmp_path):
    alvo = tmp_path / "demo.vcad.jsonl"
    alvo.write_bytes(b'{"seq": 1, "kind": "INFO", "msg": "ok"}\n\n{quebrada\n')
    t = webui.SourceTailer(alvo)
    novos = t.poll()
    assert [e["seq"] for e in novos] == [1]             # vazia e quebrada ignoradas


def test_api_events_rejeita_src_inseguro(servidor):
    _, porta, *_ = servidor
    resp, corpo, conn = requisicao(porta, "/api/events?src=../fora")
    assert resp.status == 400
    assert "erro" in json.loads(corpo)
    conn.close()


def test_api_events_heartbeat_visivel(tmp_path, monkeypatch):
    monkeypatch.setattr(webui, "HEARTBEAT_S", 0.05)     # ping quase todo loop
    (tmp_path / "logs").mkdir()
    alvo = tmp_path / "logs" / "vivo.vcad"
    alvo.write_bytes(eventos_em_bytes(EVENTOS_DEMO[:1], footer=False))
    srv = webui.make_server(tmp_path, port=0, poll=0.05)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    try:
        conn = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=5)
        conn.request("GET", "/api/events?src=logs/vivo.vcad")
        resp = conn.getresponse()
        assert resp.status == 200
        # lê bytes crus por até ~2 s procurando o comentário de heartbeat
        prazo, vistos = time.time() + 2.0, b""
        while time.time() < prazo and b": ping" not in vistos:
            pedaco = resp.fp.readline()
            if not pedaco:
                break
            vistos += pedaco
        assert b": ping" in vistos
        conn.close()
    finally:
        srv.shutdown()
        srv.server_close()


def test_api_events_sobrevive_a_cliente_que_foge(tmp_path):
    (tmp_path / "logs").mkdir()
    alvo = tmp_path / "logs" / "vivo.vcad"
    alvo.write_bytes(eventos_em_bytes(EVENTOS_DEMO, footer=False))
    srv = webui.make_server(tmp_path, port=0, poll=0.05)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    try:
        conn = http.client.HTTPConnection("127.0.0.1", srv.server_address[1], timeout=5)
        conn.request("GET", "/api/events?src=logs/vivo.vcad")
        resp = conn.getresponse()
        resp.read(1)                                    # lê um byte e some
        conn.close()                                    # sem enfrentar o stream
        time.sleep(0.3)                                 # handler tenta escrever → BrokenPipe
        # o servidor segue de pé para os próximos clientes
        resp2, corpo, conn2 = requisicao(srv.server_address[1], "/api/sources")
        assert resp2.status == 200
        conn2.close()
    finally:
        srv.shutdown()
        srv.server_close()


def test_main_sobe_e_encerra_com_ctrl_c(monkeypatch):
    fechado = {}

    class ServidorFalso:
        def serve_forever(self):
            raise KeyboardInterrupt                     # Ctrl+C do operador

        def server_close(self):
            fechado["ok"] = True

    monkeypatch.setattr(webui, "make_server", lambda *a, **k: ServidorFalso())
    webui.main(["9999", "--root", "/tmp", "--poll", "0.1"])
    assert fechado["ok"] is True
