#!/usr/bin/env python3
"""Gerador da marca VerboLang — triângulo invertido de gradientes com pássaros.

Conceito: triângulo equilátero INVERTIDO (ponto para baixo — ressoa como o V
da marca original) cujos lados são traços de osciloscópio. Cada lado mergulha
num V (pássaro) com vazamento interno de cor. O dip do lado superior é mais
profundo, e os três ápices ficam ALINHADOS na linha do horizonte (tracejada),
que passa pelos três pontos — pássaros pousados no horizonte, dentro do V.
Cores puras nos vértices: azul (sup. esq.), vermelho (sup. dir.), verde
(inferior); gradientes com stop médio (teal, amarelo, violeta). O nó central
violeta é o nó principal (o "nó do verbo", como no V do logo original).
Simetria bilateral no eixo vertical.
"""
import math

SQ3_2 = math.sqrt(3) / 2

BLUE = "#4da3ff"; GREEN = "#3fb96f"; RED = "#ff5a52"
TEAL = "#35c9c1"; YELL = "#e0c94f"; VIOL = "#a55cff"
CORE = "#eaf4ff"
BG1, BG2, BGSTROKE = "#1d2733", "#11161d", "#2a3441"
HORIZON = "#8a93a3"


def fmt(p):
    return f"{p[0]:.2f} {p[1]:.2f}"


def row_y(p):
    """Altura da fileira de ápices (a linha do horizonte)."""
    return p["C"][1] + p["R"] / 4 - p["depth"] / 2


class Side:
    """Um lado no referencial local: x ao longo de A->B, v na normal interna."""

    def __init__(self, A, B, C, p):
        self.A, self.B = A, B
        ux, uy = B[0] - A[0], B[1] - A[1]
        self.len = math.hypot(ux, uy)
        ux, uy = ux / self.len, uy / self.len
        d = ux * (C[0] - A[0]) + uy * (C[1] - A[1])
        nx, ny = C[0] - A[0] - d * ux, C[1] - A[1] - d * uy
        nl = math.hypot(nx, ny)
        self.u = (ux, uy)
        self.n = (nx / nl, ny / nl)  # normal interna (aponta ao centroide)
        m2, m4 = self.len / 2, self.len / 4
        w, dep, rr = p["w"], p["depth"], p["rr"]
        self.P1, self.P2 = self.g(m2 - w, 0), self.g(m2 + w, 0)
        self.D = self.g(m2, dep)
        leg = math.hypot(w, dep)
        f = min(rr / leg, 0.45)  # arredondamento por DISTANCIA ao longo das pernas
        self.D1 = self.g(m2 - w * f, dep * (1 - f))
        self.D2 = self.g(m2 + w * f, dep * (1 - f))

    def g(self, x, v):
        return (self.A[0] + x * self.u[0] + v * self.n[0],
                self.A[1] + x * self.u[1] + v * self.n[1])

    def path_full(self, p):
        """Trace completo: pulsos de osciloscopio + dip (passaro)."""
        m2, m4 = self.len / 2, self.len / 4
        w, bw, bh = p["w"], p["bw"], p["bh"]
        s = [f"M {fmt(self.A)}"]
        for xc in (m4, 3 * m4):  # dois pulsos simetricos por lado
            s.append(f"L {fmt(self.g(xc - bw, 0))}")
            s.append(f"C {fmt(self.g(xc - bw * 0.45, 0))} {fmt(self.g(xc - bw * 0.22, -bh))} {fmt(self.g(xc, -bh))}")
            s.append(f"C {fmt(self.g(xc + bw * 0.22, -bh))} {fmt(self.g(xc + bw * 0.45, 0))} {fmt(self.g(xc + bw, 0))}")
            if xc == m4:  # o primeiro segmento termina no inicio do dip
                s.append(f"L {fmt(self.P1)}")
                s.append(f"L {fmt(self.D1)} Q {fmt(self.D)} {fmt(self.D2)} L {fmt(self.P2)}")
        s.append(f"L {fmt(self.B)}")
        return " ".join(s)

    def path_dip(self):
        return f"M {fmt(self.P1)} L {fmt(self.D1)} Q {fmt(self.D)} {fmt(self.D2)} L {fmt(self.P2)}"


def build(pre, size, p):
    C, R = p["C"], p["R"]
    TL = (C[0] - R * SQ3_2, C[1] - R / 2)   # vertice azul
    TR = (C[0] + R * SQ3_2, C[1] - R / 2)   # vertice vermelho
    Bot = (C[0], C[1] + R)                  # vertice verde (ponta do V)
    ry = row_y(p)                            # linha do horizonte (fileira de pontos)
    d_top = ry - (C[1] - R / 2)              # dip profundo do lado superior

    # (id, A, B, cores, largura do dip, profundidade, no principal?, overrides)
    specs = [
        ("gA", TL, Bot, (BLUE, TEAL, GREEN), p["w"], p["depth"], False, {}),
        ("gB", Bot, TR, (GREEN, YELL, RED), p["w"], p["depth"], False, {}),
        ("gC", TR, TL, (RED, VIOL, BLUE), p["w_main"], d_top, True,
         dict(leak1_w=p["leak1_w"] + 4, leak1_op=min(p["leak1_op"] + 0.08, 0.5),
              leak2_w=p["leak2_w"] + 1.5, leak2_op=min(p["leak2_op"] + 0.05, 0.6))),
    ]
    sides = [(gid, Side(A, B, C, dict(p, w=w, depth=d, **ov)), cols, main)
             for gid, A, B, cols, w, d, main, ov in specs]

    freg = f'x="-64" y="-64" width="{size + 128}" height="{size + 128}"'
    grads = []
    for gid, side, (c1, cm, c2), _main in sides:
        grads.append(
            f'    <linearGradient id="{pre}{gid}" gradientUnits="userSpaceOnUse" '
            f'x1="{side.A[0]:.2f}" y1="{side.A[1]:.2f}" x2="{side.B[0]:.2f}" y2="{side.B[1]:.2f}">\n'
            f'      <stop offset="0" stop-color="{c1}"/>\n'
            f'      <stop offset="0.5" stop-color="{cm}"/>\n'
            f'      <stop offset="1" stop-color="{c2}"/>\n'
            f'    </linearGradient>')
    defs = f'''  <defs>
    <radialGradient id="{pre}vbg" cx="0.5" cy="0.4" r="0.8">
      <stop offset="0" stop-color="{BG1}"/>
      <stop offset="1" stop-color="{BG2}"/>
    </radialGradient>
    <filter id="{pre}bw" filterUnits="userSpaceOnUse" {freg}><feGaussianBlur stdDeviation="{p['blur_wash']}"/></filter>
    <filter id="{pre}bl1" filterUnits="userSpaceOnUse" {freg}><feGaussianBlur stdDeviation="{p['blur_l1']}"/></filter>
    <filter id="{pre}bl2" filterUnits="userSpaceOnUse" {freg}><feGaussianBlur stdDeviation="{p['blur_l2']}"/></filter>
    <clipPath id="{pre}tri"><polygon points="{fmt(TL)} {fmt(TR)} {fmt(Bot)}"/></clipPath>
{chr(10).join(grads)}
  </defs>'''

    out = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {size} {size}" role="img" aria-label="VerboLang">', defs]
    out.append(f'  <rect x="0" y="0" width="{size}" height="{size}" rx="{p["rx"]}" fill="url(#{pre}vbg)"/>')

    # linha do horizonte: passa pelos tres pontos (apices alinhados)
    out.append(f'  <line x1="{p["horizon_pad"]}" y1="{ry:.2f}" x2="{size - p["horizon_pad"]}" y2="{ry:.2f}" '
               f'stroke="{HORIZON}" stroke-opacity="{p["horizon_op"]}" stroke-width="{p["horizon_w"]}" '
               f'stroke-dasharray="{p["horizon_dash"]}"/>')

    # vazamento interno: banda de cor junto a cada lado, com o gradiente do proprio lado
    out.append(f'  <g clip-path="url(#{pre}tri)">')
    for gid, side, _cols, _main in sides:
        out.append(f'    <path d="M {fmt(side.A)} L {fmt(side.B)}" fill="none" stroke="url(#{pre}{gid})" '
                   f'stroke-width="{p["wash_w"]}" opacity="{p["wash_op"]}" filter="url(#{pre}bw)"/>')
    out.append('  </g>')

    # vazamento dos passaros: brilho do dip sangrando para dentro
    out.append(f'  <g clip-path="url(#{pre}tri)">')
    for gid, side, _cols, _main in sides:
        pd = dict(p)
        pd.update(specs[[s[0] for s in specs].index(gid)][7])
        for fid, wid, op in (("bl1", pd["leak1_w"], pd["leak1_op"]), ("bl2", pd["leak2_w"], pd["leak2_op"])):
            out.append(f'    <path d="{side.path_dip()}" fill="none" stroke="url(#{pre}{gid})" '
                       f'stroke-width="{wid}" opacity="{op}" stroke-linecap="round" '
                       f'stroke-linejoin="round" filter="url(#{pre}{fid})"/>')
    out.append('  </g>')

    # tracos principais
    for gid, side, _cols, _main in sides:
        pd = dict(p)
        pd.update(specs[[s[0] for s in specs].index(gid)][7])
        out.append(f'  <path d="{side.path_full(pd)}" fill="none" stroke="url(#{pre}{gid})" '
                   f'stroke-width="{p["stroke"]}" stroke-linecap="round" stroke-linejoin="round"/>')

    # nos: apice do passaro (halo na cor do meio + miolo claro); o central e o principal
    for side, _cols, main in [(s[1], s[2], s[3]) for s in sides]:
        cm = dict(zip(("c1", "cm", "c2"), _cols))["cm"]
        halo = p["apex_halo_main"] if main else p["apex_halo"]
        core = p["apex_core_main"] if main else p["apex_core"]
        op = 0.28 if main else 0.22
        out.append(f'  <circle cx="{side.D[0]:.2f}" cy="{side.D[1]:.2f}" r="{halo}" fill="{cm}" opacity="{op}"/>')
        out.append(f'  <circle cx="{side.D[0]:.2f}" cy="{side.D[1]:.2f}" r="{core}" fill="{CORE}"/>')
    for v, c in ((TL, BLUE), (TR, RED), (Bot, GREEN)):
        out.append(f'  <circle cx="{v[0]:.2f}" cy="{v[1]:.2f}" r="{p["vtx_halo"]}" fill="{c}" opacity="0.25"/>')
        out.append(f'  <circle cx="{v[0]:.2f}" cy="{v[1]:.2f}" r="{p["vtx_core"]}" fill="{c}"/>')

    out.append('</svg>')
    return "\n".join(out)


EMB = dict(C=(256.0, 208.5), R=190.0, rx=112.0,
           w=40.0, depth=54.0, w_main=52.0, rr=11.0, bw=17.0, bh=11.0,
           stroke=7.5, wash_w=58.0, wash_op=0.19,
           leak1_w=22.0, leak1_op=0.30, leak2_w=9.5, leak2_op=0.50,
           blur_wash=8.0, blur_l1=6.0, blur_l2=2.2,
           apex_halo=12.5, apex_core=4.4, apex_halo_main=14.5, apex_core_main=5.0,
           vtx_halo=17.0, vtx_core=6.5,
           horizon_pad=16.0, horizon_op=0.30, horizon_w=3.2, horizon_dash="6.4 22.4")

ICO = dict(C=(32.0, 27.5), R=22.0, rx=14.0,
           w=5.0, depth=6.75, w_main=6.5, rr=1.4, bw=3.6, bh=2.0,
           stroke=2.2, wash_w=7.5, wash_op=0.20,
           leak1_w=4.6, leak1_op=0.30, leak2_w=2.2, leak2_op=0.50,
           blur_wash=2.2, blur_l1=1.2, blur_l2=0.55,
           apex_halo=3.1, apex_core=1.2, apex_halo_main=3.6, apex_core_main=1.4,
           vtx_halo=3.4, vtx_core=1.7,
           horizon_pad=2.0, horizon_op=0.30, horizon_w=0.4, horizon_dash="0.8 2.8")

if __name__ == "__main__":
    import os
    here = os.path.dirname(os.path.abspath(__file__))
    open(os.path.join(here, "emblem.svg"), "w").write(build("e", 512, EMB))
    icon = build("i", 64, ICO)
    icon = icon.replace(
        '<rect x="0" y="0" width="64" height="64" rx="14" fill="url(#ivbg)"/>',
        f'<rect x="2" y="2" width="60" height="60" rx="14" fill="url(#ivbg)" stroke="{BGSTROKE}" stroke-width="1.5"/>')
    open(os.path.join(here, "icon.svg"), "w").write(icon)
    print("generated v3 (invertido)")
