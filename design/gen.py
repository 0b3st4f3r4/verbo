#!/usr/bin/env python3
"""Gerador da marca VerboLang — triângulo invertido de metal sobre vidro fosco.

Conceito: triângulo equilátero INVERTIDO (ponto para baixo — ressoa como o V
da marca original) como placa de metal escovado com ranhuras, apoiado num
painel de vidro fosco (glass-morphism) com aurora borrada azul-verde-vermelha
por trás. Cada lado é um traço de osciloscópio que mergulha num V (pássaro);
os três ápices ficam ALINHADOS na linha do horizonte (tracejada). Cores puras
nos vértices: azul (sup. esq.), vermelho (sup. dir.), verde (inferior). O nó
central violeta é o nó principal. Simetria bilateral no eixo vertical.
"""
import math

SQ3_2 = math.sqrt(3) / 2

BLUE = "#4da3ff"; GREEN = "#3fb96f"; RED = "#ff5a52"
TEAL = "#35c9c1"; YELL = "#e0c94f"; VIOL = "#a55cff"
CORE = "#eaf4ff"
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
        m2 = self.len / 2
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
    sides = [(gid, Side(A, B, C, dict(p, w=w, depth=d, **ov)), cols, main, ov)
             for gid, A, B, cols, w, d, main, ov in specs]

    freg = f'x="-256" y="-256" width="{size + 512}" height="{size + 512}"'
    grads = []
    for gid, side, (c1, cm, c2), _main, _ov in sides:
        grads.append(
            f'    <linearGradient id="{pre}{gid}" gradientUnits="userSpaceOnUse" '
            f'x1="{side.A[0]:.2f}" y1="{side.A[1]:.2f}" x2="{side.B[0]:.2f}" y2="{side.B[1]:.2f}">\n'
            f'      <stop offset="0" stop-color="{c1}"/>\n'
            f'      <stop offset="0.5" stop-color="{cm}"/>\n'
            f'      <stop offset="1" stop-color="{c2}"/>\n'
            f'    </linearGradient>')
    defs = f'''  <defs>
    <linearGradient id="{pre}glass" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#ffffff" stop-opacity="{p['glass_top']}"/>
      <stop offset="1" stop-color="#ffffff" stop-opacity="{p['glass_bot']}"/>
    </linearGradient>
    <linearGradient id="{pre}rim" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="0" y2="{size * p['rim_h']:.1f}">
      <stop offset="0" stop-color="#ffffff" stop-opacity="{p['rim_op']}"/>
      <stop offset="1" stop-color="#ffffff" stop-opacity="0"/>
    </linearGradient>
    <linearGradient id="{pre}metal" gradientUnits="userSpaceOnUse" x1="0" y1="{C[1] - R / 2:.2f}" x2="0" y2="{C[1] + R:.2f}">
      <stop offset="0" stop-color="{p['metal_top']}"/>
      <stop offset="1" stop-color="{p['metal_bot']}"/>
    </linearGradient>
    <filter id="{pre}bb" filterUnits="userSpaceOnUse" {freg}><feGaussianBlur stdDeviation="{p['blur_blob']}"/></filter>
    <filter id="{pre}bsh" filterUnits="userSpaceOnUse" {freg}><feGaussianBlur stdDeviation="{p['blur_shadow']}"/></filter>
    <filter id="{pre}bw" filterUnits="userSpaceOnUse" {freg}><feGaussianBlur stdDeviation="{p['blur_wash']}"/></filter>
    <filter id="{pre}bl1" filterUnits="userSpaceOnUse" {freg}><feGaussianBlur stdDeviation="{p['blur_l1']}"/></filter>
    <filter id="{pre}bl2" filterUnits="userSpaceOnUse" {freg}><feGaussianBlur stdDeviation="{p['blur_l2']}"/></filter>
    <clipPath id="{pre}pane"><rect x="0" y="0" width="{size}" height="{size}" rx="{p['rx']}"/></clipPath>
    <clipPath id="{pre}tri"><polygon points="{fmt(TL)} {fmt(TR)} {fmt(Bot)}"/></clipPath>
{chr(10).join(grads)}
  </defs>'''

    out = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {size} {size}" role="img" aria-label="VerboLang">', defs]

    # --- painel de vidro fosco (glass-morphism) ---
    # aurora borrada azul-verde-vermelha atras do vidro (simula o backdrop-blur)
    blobs = [
        (C[0] - R * 0.55, C[1] - R * 0.45, R * p["blob_r"], BLUE),
        (C[0] + R * 0.55, C[1] - R * 0.45, R * p["blob_r"], RED),
        (C[0], C[1] + R * 0.72, R * p["blob_r"] * 1.1, GREEN),
    ]
    out.append(f'  <g clip-path="url(#{pre}pane)">')
    for bx, by, br, bc in blobs:
        out.append(f'    <circle cx="{bx:.2f}" cy="{by:.2f}" r="{br:.2f}" fill="{bc}" '
                   f'opacity="{p["blob_op"]}" filter="url(#{pre}bb)"/>')
    out.append('  </g>')
    # vidro: fill translucido + luz de borda superior + contorno + reflexo diagonal
    out.append(f'  <rect x="0" y="0" width="{size}" height="{size}" rx="{p["rx"]}" fill="url(#{pre}glass)"/>')
    ins = p["rim_inset"]
    out.append(f'  <rect x="{ins}" y="{ins}" width="{size - 2 * ins:.2f}" height="{size - 2 * ins:.2f}" '
               f'rx="{max(p["rx"] - ins, 4):.2f}" fill="none" stroke="url(#{pre}rim)" stroke-width="{p["rim_w"]}"/>')
    out.append(f'  <rect x="0" y="0" width="{size}" height="{size}" rx="{p["rx"]}" fill="none" '
               f'stroke="#ffffff" stroke-opacity="{p["border_op"]}" stroke-width="{p["border_w"]}"/>')
    sheen = (size * 0.16, size * 0.44, -size * 0.12, size * 0.16)  # banda diagonal
    out.append(f'  <polygon points="{sheen[0]:.1f},0 {sheen[1]:.1f},0 {sheen[3]:.1f},{size} {sheen[2]:.1f},{size}" '
               f'fill="#ffffff" opacity="{p["sheen_op"]}" clip-path="url(#{pre}pane)"/>')

    # --- placa de metal escovado (no lugar do escuro) ---
    out.append(f'  <g clip-path="url(#{pre}tri)">')
    out.append(f'    <polygon points="{fmt(TL)} {fmt(TR)} {fmt(Bot)}" fill="url(#{pre}metal)"/>')
    gy = C[1] - R / 2 + p["groove_step"] * 0.5
    i = 0
    while gy < C[1] + R:
        if i % 2 == 0:
            out.append(f'    <line x1="0" y1="{gy:.2f}" x2="{size}" y2="{gy:.2f}" stroke="#000000" '
                       f'stroke-opacity="{p["groove_dark_op"]}" stroke-width="{p["groove_w"]}"/>')
        else:
            out.append(f'    <line x1="0" y1="{gy:.2f}" x2="{size}" y2="{gy:.2f}" stroke="#ffffff" '
                       f'stroke-opacity="{p["groove_light_op"]}" stroke-width="{p["groove_w"]}"/>')
        gy += p["groove_step"]; i += 1
    msh = (size * 0.18, size * 0.40, -size * 0.12, size * 0.10)  # sheen do metal
    out.append(f'    <polygon points="{msh[0]:.1f},0 {msh[1]:.1f},0 {msh[3]:.1f},{size} {msh[2]:.1f},{size}" '
               f'fill="#ffffff" opacity="{p["metal_sheen_op"]}"/>')
    out.append('  </g>')

    # linha do horizonte: passa pelos tres pontos (apices alinhados), sobre o metal
    out.append(f'  <line x1="{p["horizon_pad"]}" y1="{ry:.2f}" x2="{size - p["horizon_pad"]}" y2="{ry:.2f}" '
               f'stroke="{HORIZON}" stroke-opacity="{p["horizon_op"]}" stroke-width="{p["horizon_w"]}" '
               f'stroke-dasharray="{p["horizon_dash"]}"/>')

    # vazamento interno: banda de cor junto a cada lado, com o gradiente do proprio lado
    out.append(f'  <g clip-path="url(#{pre}tri)">')
    for gid, side, _cols, _main, _ov in sides:
        out.append(f'    <path d="M {fmt(side.A)} L {fmt(side.B)}" fill="none" stroke="url(#{pre}{gid})" '
                   f'stroke-width="{p["wash_w"]}" opacity="{p["wash_op"]}" filter="url(#{pre}bw)"/>')
    out.append('  </g>')

    # vazamento dos passaros: brilho do dip sangrando para dentro
    out.append(f'  <g clip-path="url(#{pre}tri)">')
    for gid, side, _cols, _main, ov in sides:
        pd = dict(p); pd.update(ov)
        for fid, wid, op in (("bl1", pd["leak1_w"], pd["leak1_op"]), ("bl2", pd["leak2_w"], pd["leak2_op"])):
            out.append(f'    <path d="{side.path_dip()}" fill="none" stroke="url(#{pre}{gid})" '
                       f'stroke-width="{wid}" opacity="{op}" stroke-linecap="round" '
                       f'stroke-linejoin="round" filter="url(#{pre}{fid})"/>')
    out.append('  </g>')

    # sombra sob os tracos: os lados em degradê ficam VISIVELMENTE sobre a placa de metal
    for _gid, side, _cols, _main, ov in sides:
        pd = dict(p); pd.update(ov)
        out.append(f'  <path d="{side.path_full(pd)}" fill="none" stroke="#000000" '
                   f'stroke-opacity="{p["trace_shadow_op"]}" stroke-width="{p["trace_shadow_w"]}" '
                   f'stroke-linecap="round" stroke-linejoin="round" filter="url(#{pre}bsh)"/>')

    # tracos principais (degradê) sobre a placa
    for gid, side, _cols, _main, ov in sides:
        pd = dict(p); pd.update(ov)
        out.append(f'  <path d="{side.path_full(pd)}" fill="none" stroke="url(#{pre}{gid})" '
                   f'stroke-width="{p["stroke"]}" stroke-linecap="round" stroke-linejoin="round"/>')

    # nos: apice do passaro (halo na cor do meio + miolo claro); o central e o principal
    for _gid, side, cols, main, _ov in sides:
        cm = cols[1]
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
           horizon_pad=16.0, horizon_op=0.35, horizon_w=3.2, horizon_dash="6.4 22.4",
           glass_top=0.14, glass_bot=0.05, border_op=0.30, border_w=2.0,
           rim_op=0.50, rim_h=0.45, rim_w=3.0, rim_inset=3.0, sheen_op=0.05,
           blob_r=0.62, blob_op=0.60, blur_blob=42.0,
           metal_top="#3f4a58", metal_bot="#252d37",
           groove_step=12.0, groove_w=1.1, groove_dark_op=0.20, groove_light_op=0.05,
           metal_sheen_op=0.06,
           trace_shadow_w=13.0, trace_shadow_op=0.35, blur_shadow=3.0)

ICO = dict(C=(32.0, 27.5), R=22.0, rx=14.0,
           w=5.0, depth=6.75, w_main=6.5, rr=1.4, bw=3.6, bh=2.0,
           stroke=2.2, wash_w=7.5, wash_op=0.20,
           leak1_w=4.6, leak1_op=0.30, leak2_w=2.2, leak2_op=0.50,
           blur_wash=2.2, blur_l1=1.2, blur_l2=0.55,
           apex_halo=3.1, apex_core=1.6, apex_halo_main=3.6, apex_core_main=1.9,
           vtx_halo=3.4, vtx_core=2.1,
           horizon_pad=2.0, horizon_op=0.35, horizon_w=0.4, horizon_dash="0.8 2.8",
           glass_top=0.14, glass_bot=0.05, border_op=0.30, border_w=1.0,
           rim_op=0.50, rim_h=0.45, rim_w=1.0, rim_inset=1.0, sheen_op=0.05,
           blob_r=0.62, blob_op=0.60, blur_blob=4.5,
           metal_top="#3f4a58", metal_bot="#252d37",
           groove_step=1.6, groove_w=0.28, groove_dark_op=0.20, groove_light_op=0.05,
           metal_sheen_op=0.06,
           trace_shadow_w=5.0, trace_shadow_op=0.35, blur_shadow=1.2)

if __name__ == "__main__":
    import os
    here = os.path.dirname(os.path.abspath(__file__))
    open(os.path.join(here, "emblem.svg"), "w").write(build("e", 512, EMB))
    open(os.path.join(here, "icon.svg"), "w").write(build("i", 64, ICO))
    print("generated v4 (vidro + metal)")
