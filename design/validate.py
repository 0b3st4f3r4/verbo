#!/usr/bin/env python3
"""Validacao objetiva das marcas persistidas (web/ e docs/).

Renderiza os SVGs oficiais propostos com o inkscape e roda sondas de pixel:
vertices nas cores puras, fileira de tres apices alinhados na linha do
horizonte, presenca da linha tracejada, legibilidade do icone a 32 px e
espectro do banner. Sai com exito 0 apenas se tudo passar.
"""
import os
import subprocess
import sys
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)

FILES = {
    "emblem": (os.path.join(ROOT, "docs", "verbolog-triangle.svg"), 512),
    "icon":   (os.path.join(ROOT, "web", "verbolog.svg"), 256),
    "banner": (os.path.join(ROOT, "docs", "verbolog-banner.svg"), 880),
}

# geometria esperada (espelha gen.EMB)
ROW_Y = 208.5 + 190.0 / 4 - 54.0 / 2          # 229.0
APICES = {(220.5, ROW_Y), (256.0, ROW_Y), (291.5, ROW_Y)}
VERTS = {(91.5, 113.5): "azul", (420.5, 113.5): "vermelho", (256.0, 398.5): "verde"}

def render(key):
    svg, w = FILES[key]
    out = os.path.join(HERE, f"preview-{key}.png")
    subprocess.run(["inkscape", svg, "-o", out, "-w", str(w)],
                   check=True, capture_output=True)
    return out

def avg(px, x, y, r=2):
    n = 0; s = [0, 0, 0]
    for yy in range(int(y - r), int(y + r) + 1):
        for xx in range(int(x - r), int(x + r) + 1):
            p = px[xx, yy]; n += 1
            for i in range(3):
                s[i] += p[i]
    return tuple(v // n for v in s)

def main():
    failures = []

    # --- emblema ---
    em = Image.open(render("emblem")).convert("RGB")
    if em.size != (512, 512):
        failures.append(f"emblema com dimensoes {em.size}, esperado 512x512")
    px = em.load()
    checks = [
        ("vertice sup-esq azul",       avg(px, 91, 114),  lambda c: c[2] > c[0] and c[2] > c[1]),
        ("vertice sup-dir vermelho",   avg(px, 421, 114), lambda c: c[0] > c[1] and c[0] > c[2]),
        ("vertice inferior verde",     avg(px, 256, 398), lambda c: c[1] > c[0] and c[1] > c[2]),
        ("apices alinhados (claros)",  None,              None),
        ("no principal violeta claro", avg(px, 256, int(ROW_Y)), lambda c: min(c) > 200),
        ("centro escuro sob a linha",  avg(px, 256, 320), lambda c: max(c) < 90),
        ("sem rastros (fundo)",        avg(px, 60, 320),  lambda c: max(c) < 60),
        ("linha do horizonte (esq)",   None,              None),
        ("linha do horizonte (dir)",   None,              None),
    ]
    # apices alinhados: tres miolos claros na mesma altura
    lights = [avg(px, x, int(ROW_Y)) for x, _ in sorted(APICES)]
    ok_row = all(min(c) > 180 for c in lights)
    print(f"  emblema [{'OK' if ok_row else 'FALHOU'}] fileira de apices em y={ROW_Y:.0f}: {lights}")
    if not ok_row:
        failures.append("fileira de apices")

    # linha do horizonte: maximo de luminancia ao longo de y=ROW_Y fora do triangulo
    def line_max(x0, x1):
        best = (0, (0, 0, 0))
        for x in range(x0, x1, 4):
            c = avg(px, x, int(ROW_Y), r=1)
            lum = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
            if lum > best[0]:
                best = (lum, c)
        return best
    for nome, x0, x1 in (("linha do horizonte (esq)", 30, 140), ("linha do horizonte (dir)", 370, 480)):
        lum, c = line_max(x0, x1)
        ok = 45 < lum < 130   # cinza da linha sobre fundo escuro
        print(f"  emblema [{'OK' if ok else 'FALHOU'}] {nome}: {c} (lum {lum:.0f})")
        if not ok:
            failures.append(nome)

    for name, c, f in checks:
        if c is None:
            continue
        ok = f(c)
        print(f"  emblema [{'OK' if ok else 'FALHOU'}] {name}: {c}")
        if not ok:
            failures.append(name)

    # --- icone a 32 px (favicon) ---
    ic = Image.open(render("icon")).convert("RGB").resize((32, 32))
    cnt = {"verde": 0, "vermelho": 0, "claro": 0}
    for y in range(32):
        for x in range(32):
            r, g, b = ic.getpixel((x, y))
            if r > 200 and g > 200 and b > 200: cnt["claro"] += 1
            elif g > r and g > b: cnt["verde"] += 1
            elif r > g and r > b: cnt["vermelho"] += 1
    ok = cnt["verde"] > 15 and cnt["vermelho"] > 15 and cnt["claro"] > 5
    print(f"  icone 32px: {cnt} [{'OK' if ok else 'FALHOU'}]")
    if not ok:
        failures.append("icone ilegivel a 32px")

    # --- banner: espectro na linha do horizonte + alinhamento + linha tracejada ---
    bn = Image.open(render("banner")).convert("RGB")
    if bn.size != (880, 200):
        failures.append(f"banner com dimensoes {bn.size}, esperado 880x200")
    pb = bn.load()
    yb = int(round(24.0 + ROW_Y * 160.0 / 512.0))   # ~96
    def bright(x):
        return max(pb[x, yb - 1], pb[x, yb], key=lambda c: sum(c))
    seq = [bright(x) for x in (240, 420, 620, 820)]
    espectro = (seq[0][2] > seq[0][0] and seq[3][0] > seq[3][2]
                and seq[1][1] > 150 and seq[2][0] > 120)
    print(f"  banner traco y={yb}: {seq} [{'OK' if espectro else 'FALHOU'}]")
    if not espectro:
        failures.append("espectro do banner")

    # no central do emblema visivel no banner (alinhado a linha)
    no_central = max((pb[x, yy] for x in range(115, 126) for yy in range(yb - 4, yb + 5)),
                     key=lambda c: min(c))
    ok = min(no_central) > 190
    print(f"  banner no central do emblema na linha: {no_central} [{'OK' if ok else 'FALHOU'}]")
    if not ok:
        failures.append("alinhamento no banner")

    # linha tracejada fora da faixa do traco espectral (o traco corre na mesma altura).
    # PNG do inkscape e RGBA com transparente = branco alfa-0: exigir alfa de dash (~0.3*255)
    # e a cor cinza do traco antes de aceitar.
    bna = Image.open(render("banner")).convert("RGBA")
    pba = bna.load()
    dash_ok, dash_px = False, None
    for x in range(202, 229, 1):
        for yy in (yb - 1, yb, yb + 1):
            r, g, b, a = pba[x, yy]
            if a >= 40 and abs(r - 138) < 55 and abs(g - 147) < 55 and abs(b - 163) < 55:
                dash_ok, dash_px = True, (r, g, b, a)
                break
        if dash_ok:
            break
    print(f"  banner linha tracejada: {dash_px} [{'OK' if dash_ok else 'FALHOU'}]")
    if not dash_ok:
        failures.append("linha tracejada do banner")

    # perna do dip profundo do lado superior (brilho do traco no caminho ate a linha)
    leg_mid = avg(px, 230, 171, r=1)
    ok = sum(leg_mid) > 240
    print(f"  emblema dip profundo (perna em 230,171): {leg_mid} [{'OK' if ok else 'FALHOU'}]")
    if not ok:
        failures.append("dip profundo")

    if failures:
        print("FALHAS:", failures)
        sys.exit(1)
    print("TUDO OK")

if __name__ == "__main__":
    main()
