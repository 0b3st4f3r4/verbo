#!/usr/bin/env python3
"""Persiste a marca oficial VerboLang (icone, emblema, banner) a partir de gen.py."""
import os
import gen
from banner import build_banner

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)

LIC = ("Parte do projeto VerboLang — licenciado sob a GNU GPL-3.0 (ver LICENSE).")

def header(txt):
    return f"<!--\n{txt}\n{LIC}\n-->\n"

# 1) icone 64x64 (logo e favicon da UI)
icon = gen.build("i", 64, gen.ICO)
icon = icon.replace(
    '<svg xmlns',
    header("verbolog.svg — marca do projeto VerboLang.\n"
           "  Triângulo de metal escovado com ranhuras sobre painel de vidro\n"
           "  fosco (aurora borrada azul-verde-vermelha) — ressoa como o V da\n"
           "  marca. Cada lado é um traço de osciloscópio que mergulha num V\n"
           "  (um pássaro); os três ápices formam uma fileira alinhada sobre a\n"
           "  linha tracejada do horizonte.") + '<svg xmlns', 1)
open(os.path.join(ROOT, "web", "verbolog.svg"), "w").write(icon)

# 2) emblema 512 (marca mestre)
emb = gen.build("e", 512, gen.EMB)
emb = emb.replace(
    '<svg xmlns',
    header("verbolog-triangle.svg — emblema mestre da marca VerboLang.\n"
           "  Triângulo invertido (ponta do V para baixo, verde) em metal\n"
           "  escovado com ranhuras, sobre painel de vidro fosco com aurora\n"
           "  borrada azul-verde-vermelha. Vértices: azul #4da3ff (sup. esq.),\n"
           "  vermelho #ff5a52 (sup. dir.), verde #3fb96f. Arestas com stop\n"
           "  médio (teal, amarelo, violeta). Os três pássaros (dips) têm ápices\n"
           "  alinhados na linha tracejada do horizonte; o nó violeta central é\n"
           "  o nó principal. O vazamento interno mantém o centro sóbrio — o\n"
           "  triângulo vaza para dentro, nunca se fecha. Gerador: design/gen.py.") + '<svg xmlns', 1)
open(os.path.join(ROOT, "docs", "verbolog-triangle.svg"), "w").write(emb)

# 3) banner
ban = build_banner()
ban = ban.replace(
    '<svg xmlns',
    header("verbolog-banner.svg — banner do projeto.\n"
           "  Emblema triangular de metal sobre vidro à esquerda; a linha\n"
           "  tracejada do horizonte passa pela fileira dos três pássaros do\n"
           "  emblema e segue sob o traço espectral\n"
           "  (azul→teal→verde→amarelo→vermelho), cujo pássaro mergulha abaixo\n"
           "  da linha.") + '<svg xmlns', 1)
open(os.path.join(ROOT, "docs", "verbolog-banner.svg"), "w").write(ban)

print("persisted:",
      os.path.join(ROOT, "web", "verbolog.svg"),
      os.path.join(ROOT, "docs", "verbolog-triangle.svg"),
      os.path.join(ROOT, "docs", "verbolog-banner.svg"), sep="\n  ")
