#!/usr/bin/env python3
"""Validacao objetiva das marcas persistidas (web/ e docs/).

Renderiza os SVGs oficiais com o inkscape e roda sondas de pixel. Como o
fundo agora e vidro translucente, tudo e compostos sobre o escuro da UI
(#11161d) antes de medir — o caso de uso real da marca.
Sai com exito 0 apenas se todas as verificacoes passarem.
"""
import math
import os
import subprocess
import sys
from PIL import Image

import gen  # geometria canônica — as sondas derivam de gen.EMB (sem drift)

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
DARK = (17, 22, 29)  # #11161d — fundo escuro da UI

FILES = {
    "emblem": (os.path.join(ROOT, "docs", "verbolog-triangle.svg"), 512),
    "icon":   (os.path.join(ROOT, "web", "verbolog.svg"), 256),
    "banner": (os.path.join(ROOT, "docs", "verbolog-banner.svg"), 880),
}

# geometria esperada (derivada de gen.EMB — sem valores mágicos)
_EMB = gen.EMB
ROW_Y = gen.row_y(_EMB)                       # 229.0 — fileira dos ápices
CX, CY = _EMB["C"][0], ROW_Y                  # centro do disco = nó violeta
RG = 512 / 2 * _EMB["glass_r"]                # raio do disco de vidro
APICES = {(220.5, ROW_Y), (256.0, ROW_Y), (291.5, ROW_Y)}
TL_X, TL_Y = _EMB["C"][0] - _EMB["R"] * gen.SQ3_2, _EMB["C"][1] - _EMB["R"] / 2
TR_X = _EMB["C"][0] + _EMB["R"] * gen.SQ3_2

def render(key):
    svg, w = FILES[key]
    out = os.path.join(HERE, f"preview-{key}.png")
    subprocess.run(["inkscape", svg, "-o", out, "-w", str(w)],
                   check=True, capture_output=True)
    return out

def comp(path, bg=DARK):
    """Carrega RGBA e composita sobre bg (opaco)."""
    im = Image.open(path).convert("RGBA")
    base = Image.new("RGBA", im.size, bg + (255,))
    return Image.alpha_composite(base, im).convert("RGB")

def avg(px, x, y, r=2):
    n = 0; s = [0, 0, 0]
    for yy in range(int(y - r), int(y + r) + 1):
        for xx in range(int(x - r), int(x + r) + 1):
            p = px[xx, yy]; n += 1
            for i in range(3):
                s[i] += p[i]
    return tuple(v // n for v in s)

def lum(c):
    return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]

def main():
    failures = []

    # --- emblema ---
    em = comp(render("emblem"))
    if em.size != (512, 512):
        failures.append(f"emblema com dimensoes {em.size}, esperado 512x512")
    px = em.load()
    checks = [
        ("vertice sup-esq azul",       avg(px, 91, 114),  lambda c: c[2] > c[0] and c[2] > c[1]),
        ("vertice sup-dir vermelho",   avg(px, 421, 114), lambda c: c[0] > c[1] and c[0] > c[2]),
        ("vertice inferior verde",     avg(px, 256, 398), lambda c: c[1] > c[0] and c[1] > c[2]),
        ("no principal violeta claro", avg(px, 256, int(ROW_Y)), lambda c: min(c) > 200),
        ("fileira de apices",          None, None),
        ("metal sob a linha",          avg(px, 256, 320), lambda c: 35 < lum(c) < 110 and c[2] >= c[0]),
        ("vidro fosco (fundo)",        avg(px, 60, 320),  lambda c: 25 < lum(c) < 140),
        ("sem rastros (fora do vidro cor)", avg(px, 480, 480), lambda c: lum(c) < 120),
    ]
    lights = [avg(px, x, int(ROW_Y)) for x, _ in sorted(APICES)]
    ok_row = all(min(c) > 180 for c in lights)
    print(f"  emblema [{'OK' if ok_row else 'FALHOU'}] fileira de apices em y={ROW_Y:.0f}: {lights}")
    if not ok_row:
        failures.append("fileira de apices")

    # linha do horizonte: presenca acima do vidro/metal fora do triangulo
    def line_max(x0, x1):
        best = (0, None)
        for x in range(x0, x1, 2):
            c = avg(px, x, int(ROW_Y), r=1)
            if lum(c) > best[0]:
                best = (lum(c), c)
        return best
    for nome, x0, x1 in (("linha do horizonte (esq)", 24, 140), ("linha do horizonte (dir)", 372, 488)):
        L, c = line_max(x0, x1)
        ok = 55 < L < 170
        print(f"  emblema [{'OK' if ok else 'FALHOU'}] {nome}: {c} (lum {L:.0f})")
        if not ok:
            failures.append(nome)

    for name, c, f in checks:
        if c is None:
            continue
        ok = f(c)
        print(f"  emblema [{'OK' if ok else 'FALHOU'}] {name}: {c}")
        if not ok:
            failures.append(name)

    # vidro CIRCULAR: os cantos do quadro ficam FORA do disco (transparentes)
    raw = Image.open(os.path.join(HERE, "preview-emblem.png")).convert("RGBA")
    cantos = [raw.getpixel(pt)[3] for pt in ((8, 8), (503, 8), (8, 503), (503, 503))]
    ok = all(a < 10 for a in cantos)
    print(f"  emblema [{'OK' if ok else 'FALHOU'}] disco de vidro (cantos fora do circulo): alpha={cantos}")
    if not ok:
        failures.append("vidro nao e circular (cantos com tinta)")

    # centro do disco = NO VIOLETA: a borda do disco deve passar exatamente
    # onde a geometria manda (topo em cy-rg ~3; laterais em cx±rg ~30/482)
    bordas = {
        "topo y=1":  raw.getpixel((int(CX), 1))[3],
        "topo y=6":  raw.getpixel((int(CX), 6))[3],
        "esq x=27":  raw.getpixel((int(CX - RG) - 3, int(CY)))[3],
        "esq x=33":  raw.getpixel((int(CX - RG) + 3, int(CY)))[3],
        "dir x=485": raw.getpixel((int(CX + RG) + 3, int(CY)))[3],
        "dir x=479": raw.getpixel((int(CX + RG) - 3, int(CY)))[3],
    }
    fora = bordas["topo y=1"] < 10 and bordas["esq x=27"] < 10 and bordas["dir x=485"] < 10
    dentro = bordas["topo y=6"] > 15 and bordas["esq x=33"] > 15 and bordas["dir x=479"] > 15
    ok = fora and dentro
    print(f"  emblema [{'OK' if ok else 'FALHOU'}] disco centrado no no violeta ({CX:.0f},{CY:.0f}): {bordas}")
    if not ok:
        failures.append("disco fora de centro (bordas nao batem com cx±rg, cy-rg)")

    # luzes das pontas INTEIRAS: amostra junto a borda do disco, na direcao dos
    # vertices superiores (pontos mais apertados) — vidro com halo tenue, nada
    # brilhante cortado no raio
    for nome, (vx, vy) in (("borda do disco p/ vertice azul", (TL_X, TL_Y)),
                           ("borda do disco p/ vertice vermelho", (TR_X, TL_Y))):
        dx, dy = vx - CX, vy - CY
        d = math.hypot(dx, dy)
        sx, sy = CX + dx / d * (RG - 6), CY + dy / d * (RG - 6)
        c = avg(px, sx, sy, r=2)
        ok = lum(c) < 160
        print(f"  emblema [{'OK' if ok else 'FALHOU'}] {nome} ({sx:.0f},{sy:.0f}): {c} (lum {lum(c):.0f})")
        if not ok:
            failures.append(nome)

    # --- icone a 32 px (favicon, sobre escuro) ---
    ic = comp(render("icon")).resize((32, 32))
    cnt = {"verde": 0, "vermelho": 0, "brilho": 0}
    mx = 0
    for y in range(32):
        for x in range(32):
            r, g, b = ic.getpixel((x, y))
            L = lum((r, g, b)); mx = max(mx, L)
            if L > 175: cnt["brilho"] += 1
            elif g > r and g > b: cnt["verde"] += 1
            elif r > g and r > b: cnt["vermelho"] += 1
    ok = cnt["verde"] > 15 and cnt["vermelho"] > 15 and cnt["brilho"] >= 5 and mx >= 185
    print(f"  icone 32px (sobre escuro): {cnt} max_lum {mx:.0f} [{'OK' if ok else 'FALHOU'}]")
    if not ok:
        failures.append("icone ilegivel a 32px")

    # --- banner: espectro na linha do horizonte + alinhamento + linha tracejada ---
    bn = comp(render("banner"))
    if bn.size != (880, 200):
        failures.append(f"banner com dimensoes {bn.size}, esperado 880x200")
    pb = bn.load()
    yb = int(round(24.0 + ROW_Y * 160.0 / 512.0))   # ~96
    def bright(x):
        return max(pb[x, yb - 1], pb[x, yb], key=lum)
    seq = [bright(x) for x in (240, 420, 620, 820)]
    espectro = (seq[0][2] > seq[0][0] and seq[3][0] > seq[3][2]
                and seq[1][1] > 150 and seq[2][0] > 120)
    print(f"  banner traco y={yb}: {seq} [{'OK' if espectro else 'FALHOU'}]")
    if not espectro:
        failures.append("espectro do banner")

    no_central = max((pb[x, yy] for x in range(115, 126) for yy in range(yb - 4, yb + 5)),
                     key=lambda c: min(c))
    ok = min(no_central) > 190
    print(f"  banner no central do emblema na linha: {no_central} [{'OK' if ok else 'FALHOU'}]")
    if not ok:
        failures.append("alinhamento no banner")

    # linha tracejada fora da faixa do traco espectral (composta sobre escuro: cinza ~ (59,66,76))
    dash_ok, dash_px = False, None
    for x in range(202, 229):
        for yy in (yb - 1, yb, yb + 1):
            r, g, b = pb[x, yy]
            if 35 <= r <= 95 and 40 <= g <= 105 and 45 <= b <= 120 and b > r:
                dash_ok, dash_px = True, (r, g, b)
                break
        if dash_ok:
            break
    print(f"  banner linha tracejada: {dash_px} [{'OK' if dash_ok else 'FALHOU'}]")
    if not dash_ok:
        failures.append("linha tracejada do banner")

    # perna do dip profundo do lado superior
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
