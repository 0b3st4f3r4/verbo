#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""build_site — monta o livro mdBook do site de documentação (verbolang.org/docs).

O livro é publicado pelo GitHub Pages sob a base que o domínio determinar
(``verbolang.org/docs`` ou ``<conta>.github.io/verbo/docs``) — por isso todos
os links internos viram **relativos** na montagem. Fontes (commitadas) →
saída (gerada, git-ignorada):

    site/content/**                 → site/src/**                 (links reescritos)
    docs/{FORMAL,MANIFESTO,...}.md  → site/src/reference/**       (links reescritos)
    README.md, CHANGELOG.md,
    docs/PLAN.md, docs/RELEASES.md  → site/src/project/**         (links reescritos)
    examples/*.vl                   → site/src/reference/examples/*.vl
    docs/brand/*.svg                → site/src/assets/brand/*.svg
    web/verbolog.svg, web/fonts/*,
    web/vendor/mermaid.min.js       → site/src/assets/…
    site/theme/*                    → site/src/assets/*           (tema da marca)

Regra de links: nos .md fontes, alvos são caminhos **relativos ao arquivo
fonte no repositório** (ex.: do cheatsheet, ``../PLAN.md``). Na montagem,
cada alvo é resolvido contra o repositório e remapeado:

  * alvo dentro do livro        → link relativo novo (book↔book);
  * alvo fora do livro          → URL canônica no GitHub (blob/main);
  * alvo inexistente no repositório → ERRO (link quebrado não passa).

Comandos:
  (padrão)      monta ``site/src``
  --check       monta em temporário e valida (make site-check / CI)
  --pages DIR   artefato do GitHub Pages: livro em DIR/docs + landing raiz
                (DIR/index.html) + .nojekyll (requer ``mdbook build site`` antes)

Só stdlib. Parte do projeto VerboLang — GNU GPL-3.0 (ver LICENSE).
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import sys
import tempfile
from pathlib import Path
from urllib.parse import quote

RAIZ = Path(__file__).resolve().parents[1]
CONTEUDO = RAIZ / "site" / "content"
TEMA = RAIZ / "site" / "theme"
GITHUB = "https://github.com/0b3st4f3r4/verbo"

# ── o mapa: repositório → livro ────────────────────────────────────────────

# Documentos de referência e de projeto que entram no livro (links reescritos).
FONTES_FIXAS: list[tuple[str, str]] = [
    # Referência — a spec e seus anexos
    ("docs/FORMAL.md", "reference/FORMAL.md"),
    ("docs/MANIFESTO.md", "reference/MANIFESTO.md"),
    ("docs/FXP-SCHEMA-v1.md", "reference/FXP-SCHEMA-v1.md"),
    ("docs/NOTEBOOK-FORMAT-v1.md", "reference/NOTEBOOK-FORMAT-v1.md"),
    ("docs/cheatsheet/VBL-CHEATSHEET.md", "reference/cheatsheet/VBL-CHEATSHEET.md"),
    ("docs/cheatsheet/VBL-CHEATSHEET-AGENTS.md", "reference/cheatsheet/VBL-CHEATSHEET-AGENTS.md"),
    ("docs/adrs/ADR-001-core-language.md", "reference/ADR-001-core-language.md"),
    # Projeto — como o projeto se governa
    ("README.md", "project/repository.md"),
    ("CHANGELOG.md", "project/CHANGELOG.md"),
    ("docs/PLAN.md", "project/PLAN.md"),
    ("docs/RELEASES.md", "project/RELEASES.md"),
]


def construir_mapa() -> dict[str, str]:
    """Mapa completo repo-relativo → book-relativo (fixos + dinâmicos)."""
    mapa: dict[str, str] = {fonte: livro for fonte, livro in FONTES_FIXAS}

    # A trilha didática: identidade dentro do livro (site/content/guide → guide)
    for md in sorted(CONTEUDO.rglob("*.md")):
        rel = md.relative_to(CONTEUDO).as_posix()
        mapa[f"site/content/{rel}"] = rel

    # Exemplos executáveis viram assets de referência
    for exemplo in sorted((RAIZ / "examples").glob("*.vl")):
        mapa[f"examples/{exemplo.name}"] = f"reference/examples/{exemplo.name}"

    # Marcas SVG (banner do README, emblema da landing)
    for svg in sorted((RAIZ / "docs" / "brand").glob("*.svg")):
        mapa[f"docs/brand/{svg.name}"] = f"assets/brand/{svg.name}"

    return mapa


# ── reescrita de links ─────────────────────────────────────────────────────

RE_LINK_MD = re.compile(r"(\]\()([^)\s]+)((?:\s+\"[^\"]*\")?\))")
RE_LINK_HTML = re.compile(r'((?:src|href)=")([^"]+)(")')


class ErroLink(Exception):
    """Link que não resolve nem dentro do livro nem no repositório."""


def _remapear(alvo: str, fonte_repo: str, destino_book: str, mapa: dict[str, str]) -> str:
    """Um alvo de link → link do livro ou URL do GitHub (fragmento preservado)."""
    if alvo.startswith(("http://", "https://", "mailto:", "#", "data:")):
        return alvo

    caminho, _, fragmento = alvo.partition("#")
    if not caminho:
        return alvo

    fonte_dir = (RAIZ / fonte_repo).parent
    resolvido = (fonte_dir / caminho).resolve()
    try:
        rel = resolvido.relative_to(RAIZ).as_posix()
    except ValueError:  # escapou do repositório
        raise ErroLink(f"{fonte_repo}: link escapa do repo: {alvo}") from None

    if rel in mapa:
        livro_alvo = mapa[rel]
        livro_dir = (destino_book.rpartition("/")[0])
        novo = os.path.relpath(livro_alvo, livro_dir or ".").replace("\\", "/")
        return novo + (f"#{fragmento}" if fragmento else "")

    if resolvido.is_dir():
        return f"{GITHUB}/tree/main/{quote(rel)}"
    if resolvido.is_file():
        return f"{GITHUB}/blob/main/{quote(rel)}"

    raise ErroLink(f"{fonte_repo}: link quebrado no repositório: {alvo}")


def reescrever(texto: str, fonte_repo: str, destino_book: str, mapa: dict[str, str]) -> str:
    """Reescreve links markdown e atributos src/href de HTML embutido."""
    texto = RE_LINK_MD.sub(
        lambda m: m.group(1) + _remapear(m.group(2), fonte_repo, destino_book, mapa) + m.group(3),
        texto,
    )
    texto = RE_LINK_HTML.sub(
        lambda m: m.group(1) + _remapear(m.group(2), fonte_repo, destino_book, mapa) + m.group(3),
        texto,
    )
    return texto


# ── montagem ───────────────────────────────────────────────────────────────

def _copiar_arvore(origem: Path, destino: Path) -> None:
    if origem.is_dir():
        shutil.copytree(origem, destino, dirs_exist_ok=True)


def montar(dest: Path, mapa: dict[str, str] | None = None) -> Path:
    """Gera a árvore `src` do livro em `dest` (normalmente ``site/src``)."""
    mapa = mapa or construir_mapa()
    if dest.exists():
        shutil.rmtree(dest)
    dest.mkdir(parents=True)

    # 1. conteúdo do próprio site (landing + trilha), links reescritos
    #    SUMMARY.md é o índice do mdBook: seus links já são relativos ao
    #    src/ do livro (formato exigido pelo mdBook) — copia verbatim.
    for md in sorted(CONTEUDO.rglob("*.md")):
        rel = md.relative_to(CONTEUDO)
        bruto = md.read_text(encoding="utf-8")
        texto = (
            bruto
            if rel.name == "SUMMARY.md"
            else reescrever(
                bruto,
                f"site/content/{rel.as_posix()}",
                rel.as_posix(),
                mapa,
            )
        )
        alvo = dest / rel
        alvo.parent.mkdir(parents=True, exist_ok=True)
        alvo.write_text(texto, encoding="utf-8")

    # 2. referência e projeto (links reescritos)
    for fonte, livro in FONTES_FIXAS:
        texto = reescrever((RAIZ / fonte).read_text(encoding="utf-8"), fonte, livro, mapa)
        alvo = dest / livro
        alvo.parent.mkdir(parents=True, exist_ok=True)
        alvo.write_text(texto, encoding="utf-8")

    # 3. exemplos executáveis (assets estáticos)
    _copiar_arvore(RAIZ / "examples", dest / "reference" / "examples")

    # 4. assets: marcas, fontes, mermaid vendored e o tema
    _copiar_arvore(RAIZ / "docs" / "brand", dest / "assets" / "brand")
    _copiar_arvore(RAIZ / "web" / "fonts", dest / "assets" / "fonts")
    shutil.copy2(RAIZ / "web" / "verbolog.svg", dest / "assets" / "verbolog.svg")
    vendor = dest / "assets" / "vendor"
    vendor.mkdir(parents=True, exist_ok=True)
    shutil.copy2(RAIZ / "web" / "vendor" / "mermaid.min.js", vendor / "mermaid.min.js")
    _copiar_arvore(TEMA, dest / "assets")

    return dest


# ── validação ──────────────────────────────────────────────────────────────

RE_ENTRADA = re.compile(r"\]\(([^)#]+\.md)\)")


def entradas_summary(summary: Path | None = None) -> list[str]:
    """Capítulos listados no SUMMARY.md, na ordem (caminhos relativos ao src)."""
    summary = summary or CONTEUDO / "SUMMARY.md"
    return RE_ENTRADA.findall(summary.read_text(encoding="utf-8"))


def verificar(src: Path) -> list[str]:
    """Invariantes do livro montado — devolve a lista de problemas (vazia = ok)."""
    problemas: list[str] = []
    mapa = construir_mapa()

    for entrada in entradas_summary():
        if not (src / entrada).is_file():
            problemas.append(f"SUMMARY lista {entrada} e o livro não o tem")

    for md in sorted(src.rglob("*.md")):
        rel = md.relative_to(src).as_posix()
        texto = md.read_text(encoding="utf-8")
        alvos = [m.group(2) for m in RE_LINK_HTML.finditer(texto)]
        alvos += [m.group(2) for m in
                  re.finditer(r"(\]\()([^)\s]+)((?:\s+\"[^\"]*\")?\))", texto)]
        for alvo in alvos:
            caminho, _, _ = alvo.partition("#")
            if not caminho or alvo.startswith(("http://", "https://", "mailto:", "data:")):
                continue
            if caminho.startswith("/"):
                problemas.append(f"{rel}: link raiz-absoluto quebra a base /docs — {alvo}")
                continue
            if not (md.parent / caminho).resolve().exists():
                problemas.append(f"{rel}: não resolve no livro — {alvo}")

    # assets que o tema e os capítulos consomem
    obrigatorios = [
        "assets/site.css", "assets/verbolang.js", "assets/mermaid-init.js",
        "assets/i18n.js", "assets/verbolog.svg", "assets/vendor/mermaid.min.js",
        "assets/brand/verbolog-triangle.svg", "reference/examples/example1_free_thinking.vl",
    ]
    for arquivo in obrigatorios:
        if not (src / arquivo).is_file():
            problemas.append(f"falta asset montado: {arquivo}")

    fontes = list((src / "assets" / "fonts").glob("*.woff2")) if (src / "assets" / "fonts").is_dir() else []
    if len(fontes) < 4:
        problemas.append(f"fontes Inter/Iosevka incompletas ({len(fontes)} woff2)")

    # toda fonte citada pelo tema precisa ter vindo de web/fonts (o CSS é
    # emitido em src/assets/ e as fontes vivem em assets/fonts — ../..)
    css = (src / "assets" / "site.css").read_text(encoding="utf-8") if (src / "assets" / "site.css").is_file() else ""
    for fonte in re.findall(r'url\("\.\./\.\./assets/(fonts/[^"]+)"\)', css):
        if not (RAIZ / "web" / "fonts" / Path(fonte).name).is_file():
            problemas.append(f"site.css pede {fonte}, inexistente em web/fonts")

    # trilha completa (o didático é o coração do site)
    for capitulo in ["overview", "installation", "forms", "reviews", "fxp", "notebook", "recipes"]:
        entrada = f"guide/{capitulo}.md"
        if entrada not in entradas_summary():
            problemas.append(f"capítulo da trilha fora do SUMMARY: {entrada}")

    if not mapa:  # mapa sempre verdadeiro; guarda contra refatoração silenciosa
        problemas.append("mapa repo→livro vazio")
    return problemas


# ── artefato do GitHub Pages ───────────────────────────────────────────────

LIVRO = RAIZ / "site" / "book"   # saída do `mdbook build site` (artefato, não checkout)


def montar_pages(dest: Path, livro: Path | None = None) -> Path:
    """Artefato do Pages: livro em `dest/docs`, landing raiz e .nojekyll."""
    livro = livro or LIVRO
    if not (livro / "index.html").is_file():
        raise SystemExit("livro ausente — rode `mdbook build site` antes de --pages")
    if dest.exists():
        shutil.rmtree(dest)
    dest.mkdir(parents=True)
    shutil.copytree(livro, dest / "docs")
    shutil.copy2(RAIZ / "site" / "root" / "index.html", dest / "index.html")
    (dest / ".nojekyll").write_text("", encoding="utf-8")
    return dest


# ── CLI ────────────────────────────────────────────────────────────────────

def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--dest", type=Path, default=RAIZ / "site" / "src",
                        help="onde gerar a árvore src do livro (padrão: site/src)")
    parser.add_argument("--check", action="store_true",
                        help="monta em temporário, valida e não suja a árvore")
    parser.add_argument("--pages", type=Path, metavar="DIR",
                        help="monta o artefato do GitHub Pages em DIR")
    args = parser.parse_args(argv)

    if args.pages:
        caminho = montar_pages(args.pages)
        arquivos = sum(1 for p in caminho.rglob("*") if p.is_file())
        print(f"✓ artefato do Pages em {caminho} ({arquivos} arquivos; livro em docs/)")
        return 0

    if args.check:
        with tempfile.TemporaryDirectory(prefix="vbl-site-") as tmp:
            src = montar(Path(tmp) / "src")
            problemas = verificar(src)
        if problemas:
            for problema in problemas:
                print(f"✗ {problema}", file=sys.stderr)
            return 1
        paginas = len(list((RAIZ / "site" / "src").rglob("*.md"))) if (RAIZ / "site" / "src").is_dir() else 0
        print(f"✓ site ok — SUMMARY, links, assets e base-path validados "
              f"({len(entradas_summary())} entradas; {paginas} .md na árvore vigente)")
        return 0

    src = montar(args.dest)
    problemas = verificar(src)
    arquivos = sum(1 for p in src.rglob("*") if p.is_file())
    if problemas:
        for problema in problemas:
            print(f"✗ {problema}", file=sys.stderr)
        return 1
    print(f"✓ livro montado em {src} ({arquivos} arquivos) — `mdbook build site` em seguida")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
