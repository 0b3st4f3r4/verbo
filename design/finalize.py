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
    '<rect x="0" y="0" width="64" height="64" rx="14" fill="url(#ivbg)"/>',
    f'<rect x="2" y="2" width="60" height="60" rx="14" fill="url(#ivbg)" stroke="{gen.BGSTROKE}" stroke-width="1.5"/>')
icon = icon.replace(
    '<svg xmlns',
    header("verbolog.svg — marca do projeto VerboLang.\n"
           "  Triângulo invertido de gradientes azul-verde-vermelho — ressoa como\n"
           "  o V da marca. Cada lado é um traço de osciloscópio que mergulha num\n"
           "  V (um pássaro); os três ápices formam uma fileira alinhada sobre a\n"
           "  linha tracejada do horizonte, com a cor vazando para dentro.") + '<svg xmlns', 1)
open(os.path.join(ROOT, "web", "verbolog.svg"), "w").write(icon)

# 2) emblema 512 (marca mestre)
emb = gen.build("e", 512, gen.EMB)
emb = emb.replace(
    '<svg xmlns',
    header("verbolog-triangle.svg — emblema mestre da marca VerboLang.\n"
           "  Triângulo invertido (ponta do V para baixo, verde). Vértices: azul\n"
           "  #4da3ff (sup. esq.), vermelho #ff5a52 (sup. dir.), verde #3fb96f.\n"
           "  Arestas com stop médio (teal, amarelo, violeta). Os três pássaros\n"
           "  (dips) têm ápices alinhados na linha tracejada do horizonte; o nó\n"
           "  violeta central é o nó principal. O vazamento interno mantém o\n"
           "  centro escuro — o triângulo vaza para dentro, nunca se fecha.\n"
           "  Gerador: design/gen.py.") + '<svg xmlns', 1)
open(os.path.join(ROOT, "docs", "verbolog-triangle.svg"), "w").write(emb)

# 3) banner
ban = build_banner()
ban = ban.replace(
    '<svg xmlns',
    header("verbolog-banner.svg — banner do projeto.\n"
           "  Emblema triangular invertido à esquerda; a linha tracejada do\n"
           "  horizonte passa pela fileira dos três pássaros do emblema e segue\n"
           "  sob o traço espectral (azul→teal→verde→amarelo→vermelho), cujo\n"
           "  pássaro mergulha abaixo da linha.") + '<svg xmlns', 1)
open(os.path.join(ROOT, "docs", "verbolog-banner.svg"), "w").write(ban)

print("persisted:",
      os.path.join(ROOT, "web", "verbolog.svg"),
      os.path.join(ROOT, "docs", "verbolog-triangle.svg"),
      os.path.join(ROOT, "docs", "verbolog-banner.svg"), sep="\n  ")
