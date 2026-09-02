# -*- coding: utf-8 -*-
"""Contrato do montador do site (scripts/build_site.py) — regra de ouro
AGENTS §2: o teste existe junto com (antes de) a implementação.

O site de documentação (site/ — mdBook, publicado em verbolang.org/docs
via GitHub Pages) nasce da montagem:

    site/content/**  ──(reescrita de links)──►  site/src/**
    docs/*.md (referência)  ──────────────────►  site/src/reference/**
    README/CHANGELOG/PLAN/RELEASES ───────────►  site/src/project/**
    web/vendor, web/fonts, docs/brand ────────►  site/src/assets/**

Invariantes testados (o mesmo --check que roda no CI):
  1. toda entrada do SUMMARY.md existe em site/src;
  2. todo link .md/.vl nos .md montados resolve DENTRO do livro ou cai
     em URL canônica do GitHub (nada de link quebrado silencioso);
  3. nenhum link raiz-absoluto ("/x") — o livro precisa funcionar sob
     qualquer base (verbolang.org/docs, github.io/verbo/docs, file://);
  4. assets (fontes, mermaid vendored, marcas SVG) foram copiados;
  5. a trilha didática (guide/) está completa e listada no SUMMARY.
"""

from __future__ import annotations

import importlib.util
import re
from pathlib import Path

import pytest

RAIZ = Path(__file__).resolve().parents[2]


def _carregar():
    spec = importlib.util.spec_from_file_location(
        "build_site", RAIZ / "scripts" / "build_site.py"
    )
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


@pytest.fixture(scope="module")
def montado(tmp_path_factory):
    mod = _carregar()
    dest = tmp_path_factory.mktemp("site-src")
    mod.montar(dest)
    return mod, dest


# ── 1. SUMMARY × arquivos ──────────────────────────────────────────────────

def test_summary_entrada_existe_no_livro(montado):
    mod, src = montado
    entradas = mod.entradas_summary()
    assert len(entradas) >= 15, "SUMMARY deveria listar a trilha + referência + projeto"
    for rel in entradas:
        assert (src / rel).is_file(), f"SUMMARY lista {rel} e o livro não o tem"


def test_trilha_didatica_completa(montado):
    mod, src = montado
    capítulos = mod.entradas_summary()
    esperados = [
        "guide/overview.md",
        "guide/installation.md",
        "guide/forms.md",
        "guide/reviews.md",
        "guide/fxp.md",
        "guide/notebook.md",
        "guide/recipes.md",
    ]
    for cap in esperados:
        assert cap in capítulos, f"capítulo da trilha ausente no SUMMARY: {cap}"
        assert (src / cap).read_text(encoding="utf-8").strip(), f"{cap} vazio"


def test_fontes_da_trilha_existem_no_repo():
    for cap in Path(RAIZ / "site" / "content" / "guide").glob("*.md"):
        assert cap.stat().st_size > 500, f"{cap.name} curto demais para ser didático"


# ── 2. links resolvem dentro do livro ou vão para o GitHub ────────────────

LINK_MD = re.compile(r"\[[^\]]*\]\(([^)]+)\)")
LINK_HTML = re.compile(r'\b(?:src|href)="([^"]+)"')


def _links_quebrados(mod, src):
    problemas = []
    for md in sorted(src.rglob("*.md")):
        alvos = LINK_MD.findall(md.read_text(encoding="utf-8")) + \
            LINK_HTML.findall(md.read_text(encoding="utf-8"))
        for alvo in alvos:
            alvo = alvo.split(" ")[0].strip()
            if alvo.startswith(("http://", "https://", "mailto:")):
                assert alvo.startswith(("http://", "https://")), (
                    f"{md.relative_to(src)}: externa estranha {alvo}")
                continue
            caminho, _, _frag = alvo.partition("#")   # âncora não afeta o arquivo
            if not caminho:
                continue
            if caminho.startswith("/"):
                problemas.append(f"{md.relative_to(src)}: raiz-absoluto {alvo}")
                continue
            destino = (md.parent / caminho).resolve()
            if not destino.exists():
                problemas.append(f"{md.relative_to(src)}: não resolve {alvo}")
    return problemas


def test_links_montados_resolvem(montado):
    mod, src = montado
    assert _links_quebrados(mod, src) == []


def test_reescrita_relativa_cheatsheet(montado):
    _, src = montado
    txt = (src / "reference" / "cheatsheet" / "VBL-CHEATSHEET.md").read_text("utf-8")
    # no repo o cheatsheet aponta ../PLAN.md; no livro, PLAN vive em project/
    # (reference/cheatsheet/ → project/: dois níveis acima)
    assert "(../../project/PLAN.md)" in txt, "link do PLAN não foi remapeado para project/"
    assert "(../PLAN.md)" not in txt, "link bruto do repo vazou no livro"


def test_reescrita_guia_para_referencia(montado):
    _, src = montado
    txt = (src / "guide" / "recipes.md").read_text("utf-8")
    assert "(../reference/FORMAL.md)" in txt, "guia não aponta a spec no livro"


def test_fallback_github_para_arquivo_de_fora(montado):
    _, src = montado
    txt = (src / "project" / "repository.md").read_text("utf-8")
    # README aponta scripts/webui.py, que não vai para o livro — vira URL canônica
    assert "https://github.com/0b3st4f3r4/verbo/blob/main/scripts/webui.py" in txt


# ── 3. base-path safety ────────────────────────────────────────────────────

def test_sem_link_raiz_absoluto_em_assets():
    for arq in ["site/theme/site.css", "site/theme/i18n.js",
                "site/theme/verbolang.js", "site/theme/mermaid-init.js"]:
        txt = (RAIZ / arq).read_text("utf-8")
        for padrao in ('href="/', 'src="/', 'url("/', 'url(/', 'fetch("/'):
            assert padrao not in txt, f"{arq}: caminho raiz-absoluto {padrao}"


# ── 4. assets ──────────────────────────────────────────────────────────────

def test_assets_copiados(montado):
    _, src = montado
    assets = src / "assets"
    assert (assets / "vendor" / "mermaid.min.js").is_file()
    assert (assets / "verbolog.svg").is_file()
    assert (assets / "brand" / "verbolog-triangle.svg").is_file()
    fontes = list((assets / "fonts").glob("*.woff2"))
    assert len(fontes) >= 4, "fontes Inter/Iosevka não foram copiadas"


def test_css_usa_fontes_que_existem():
    css = (RAIZ / "site" / "theme" / "site.css").read_text("utf-8")
    # o CSS é emitido em src/assets/ e as fontes em assets/fonts — ../..
    for fonte in re.findall(r'url\("\.\./\.\./assets/(fonts/[^"]+)"\)', css):
        assert (RAIZ / "web" / "fonts" / Path(fonte).name).is_file(), (
            f"site.css pede {fonte} e web/fonts não a tem")


# ── 5. book.toml coerente ──────────────────────────────────────────────────

def test_book_toml_assets_existem(montado):
    # o mdbook resolve additional-css/js a partir da raiz do livro (site/) na
    # hora do build — e esses assets sob src/ são emitidos pelo montador.
    # site/src é artefato git-ignorado (não existe no checkout limpo do CI),
    # então a verificação roda contra a árvore montada (o mesmo --check).
    _, src = montado
    toml = (RAIZ / "site" / "book.toml").read_text("utf-8")
    for m in re.finditer(r'additional-(css|js)\s*=\s*\[([^\]]*)\]', toml):
        for bruto in m.group(2).split(","):
            caminho = bruto.strip().strip('"').strip("'")
            if not caminho:
                continue
            if caminho.startswith("src/"):
                alvo = src / caminho[len("src/"):]
            else:
                alvo = RAIZ / "site" / caminho
            assert alvo.is_file(), (
                f"book.toml lista {caminho} e o livro montado não o tem")


def test_montar_eh_idempotente(tmp_path):
    mod = _carregar()
    mod.montar(tmp_path)
    primeiro = {p.relative_to(tmp_path): p.stat().st_size for p in tmp_path.rglob("*") if p.is_file()}
    mod.montar(tmp_path)
    segundo = {p.relative_to(tmp_path): p.stat().st_size for p in tmp_path.rglob("*") if p.is_file()}
    assert primeiro == segundo


# ── 6. verificar(), montar_pages() e main() — gate de cobertura (CI) ──────

def test_verificar_aprovado_no_arvore_montada(montado):
    mod, src = montado
    assert mod.verificar(src) == []


def test_verificar_pega_asset_faltando(montado, tmp_path):
    mod, src = montado
    import shutil as _sh
    destino = tmp_path / "src"
    _sh.copytree(src, destino)
    (destino / "assets" / "verbolog.svg").unlink()
    problemas = mod.verificar(destino)
    assert any("verbolog.svg" in p for p in problemas)


def test_verificar_pega_link_quebrado(montado, tmp_path):
    mod, src = montado
    import shutil as _sh
    destino = tmp_path / "src"
    _sh.copytree(src, destino)
    cap = destino / "guide" / "overview.md"
    cap.write_text(cap.read_text("utf-8") + "\n[ruim](capitulo-fantasma.md)\n", "utf-8")
    assert any("capitulo-fantasma" in p for p in mod.verificar(destino))


def test_verificar_pega_raiz_absoluta(montado, tmp_path):
    mod, src = montado
    import shutil as _sh
    destino = tmp_path / "src"
    _sh.copytree(src, destino)
    cap = destino / "guide" / "forms.md"
    cap.write_text(cap.read_text("utf-8") + '\n<img src="/estatico.png">\n', "utf-8")
    assert any("raiz-absoluto" in p for p in mod.verificar(destino))


def test_verificar_pega_fonte_css_fantasma(montado, tmp_path):
    mod, src = montado
    import shutil as _sh
    destino = tmp_path / "src"
    _sh.copytree(src, destino)
    # o validador lê o CSS montado (cópia do tema): uma fonte fantasma nele
    # significa que web/fonts não tem o arquivo — quebra antes do deploy
    (destino / "assets" / "site.css").write_text(
        '@font-face { src: url("../../assets/fonts/fantasma.woff2"); }\n', "utf-8")
    assert any("fantasma.woff2" in p for p in mod.verificar(destino))


def test_reescrever_erro_link_escapa_do_repo():
    mod = _carregar()
    with pytest.raises(mod.ErroLink):
        mod.reescrever("[x](../../../../etc/passwd)",
                       "site/content/guide/overview.md", "guide/overview.md",
                       mod.construir_mapa())


def test_reescrever_erro_link_inexistente_no_repo():
    mod = _carregar()
    with pytest.raises(mod.ErroLink):
        mod.reescrever("[x](arquivo-fantasma.md)",
                       "site/content/guide/overview.md", "guide/overview.md",
                       mod.construir_mapa())


def test_copiar_arvore_ignora_ausente(tmp_path):
    mod = _carregar()
    mod._copiar_arvore(tmp_path / "nao-existe", tmp_path / "destino")
    assert not (tmp_path / "destino").exists()


def test_montar_pages_com_livro_falso(tmp_path):
    mod = _carregar()
    livro = tmp_path / "book"
    (livro / "guide").mkdir(parents=True)
    (livro / "index.html").write_text("<html></html>", "utf-8")
    (livro / "guide" / "x.html").write_text("<html></html>", "utf-8")
    artefato = mod.montar_pages(tmp_path / "pages", livro=livro)
    assert (artefato / "docs" / "index.html").is_file()
    assert (artefato / "index.html").is_file()
    assert (artefato / ".nojekyll").is_file()
    assert (artefato / "docs" / "guide" / "x.html").is_file()


def test_montar_pages_sem_livro_exige_build(tmp_path):
    mod = _carregar()
    with pytest.raises(SystemExit):
        mod.montar_pages(tmp_path / "pages", livro=tmp_path / "vazio")


def test_main_check_e_montagem_e_pages(tmp_path, monkeypatch):
    mod = _carregar()
    assert mod.main(["--check"]) == 0
    destino = tmp_path / "src"
    assert mod.main(["--dest", str(destino)]) == 0
    assert (destino / "SUMMARY.md").is_file()
    # site/book é artefato do mdbook (git-ignorado): o checkout limpo do CI não
    # o tem. A fiação main → montar_pages é exercitada com um livro mínimo; o
    # empacotamento completo já é coberto por test_montar_pages_com_livro_falso.
    livro = tmp_path / "book"
    livro.mkdir()
    (livro / "index.html").write_text("<html></html>", "utf-8")
    monkeypatch.setattr(mod, "LIVRO", livro)
    pages = tmp_path / "pages"
    assert mod.main(["--pages", str(pages)]) == 0
    assert (pages / "docs" / "index.html").is_file()


def test_main_check_falha_com_problemas(tmp_path, monkeypatch):
    mod = _carregar()
    monkeypatch.setattr(mod, "verificar", lambda src: ["problema inventado"])
    assert mod.main(["--check"]) == 1


# ── 7. micro-cobertura: ramos de guarda do montador ───────────────────────

def test_reescrever_ancora_pura_passa_ilmene():
    mod = _carregar()
    assert mod.reescrever("[x](#secao)", "site/content/guide/overview.md",
                          "guide/overview.md", mod.construir_mapa()) == "[x](#secao)"


def test_verificar_pega_entrada_summary_sem_arquivo(montado, tmp_path):
    mod, src = montado
    import shutil as _sh
    destino = tmp_path / "src"
    _sh.copytree(src, destino)
    (destino / "reference" / "FORMAL.md").unlink()
    assert any("SUMMARY lista reference/FORMAL.md" in p for p in mod.verificar(destino))


def test_verificar_pega_fontes_incompletas(montado, tmp_path):
    mod, src = montado
    import shutil as _sh
    destino = tmp_path / "src"
    _sh.copytree(src, destino)
    _sh.rmtree(destino / "assets" / "fonts")
    assert any("incompletas" in p for p in mod.verificar(destino))


def test_verificar_pega_capitulo_fora_do_summary(montado, tmp_path, monkeypatch):
    mod, src = montado
    import shutil as _sh
    # o SUMMARY canônico é o da FONTE (repo): a checagem da trilha lê dele
    conteudo_falso = tmp_path / "content"
    _sh.copytree(mod.CONTEUDO, conteudo_falso)
    summary = conteudo_falso / "SUMMARY.md"
    texto = summary.read_text("utf-8")
    linha = next(l for l in texto.splitlines() if "guide/fxp.md" in l)
    summary.write_text(texto.replace(linha + "\n", ""), "utf-8")
    monkeypatch.setattr(mod, "CONTEUDO", conteudo_falso)
    assert any("guide/fxp.md" in p for p in mod.verificar(src))


def test_montar_pages_sobre_destino_existente(tmp_path):
    mod = _carregar()
    livro = tmp_path / "book"
    livro.mkdir()
    (livro / "index.html").write_text("<html></html>", "utf-8")
    artefato = tmp_path / "pages"
    artefato.mkdir()
    (artefato / "lixo.txt").write_text("x", "utf-8")
    mod.montar_pages(artefato, livro=livro)   # segunda passada: rmtree do destino
    assert not (artefato / "lixo.txt").exists()


def test_main_montagem_falha_com_problemas(tmp_path, monkeypatch):
    mod = _carregar()
    monkeypatch.setattr(mod, "verificar", lambda src: ["problema da montagem"])
    assert mod.main(["--dest", str(tmp_path / "src")]) == 1
