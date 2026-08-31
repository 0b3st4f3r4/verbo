#!/usr/bin/env python3
"""Banner VerboLang: emblema-triangulo invertido + wordmark + traco espectral.

A linha tracejada do horizonte fica na altura exata da fileira de tres pontos
do emblema (os apices alinhados) e passa por eles; o traco espectral corre
sobre essa mesma linha, com o passaro mergulhando abaixo dela.
"""
import math
import gen


def build_banner():
    s = 160.0 / 512.0
    ox, oy = 40.0, 24.0
    badge = gen.build("b", 512, gen.EMB)
    inner = badge.split(">", 1)[1].rsplit("</svg>", 1)[0]

    # linha do horizonte = fileira de pontos do emblema, mapeada para o banner
    y = gen.row_y(gen.EMB) * s + oy

    # traco espectral: horizonte com dip (passaro) no centro da zona de texto
    x0, x1 = 230.0, 830.0
    cx, w, dep, rr = 520.0, 20.0, 26.0, 6.0
    leg = math.hypot(w, dep); f = rr / leg
    D1 = (cx - w * f, y + dep - dep * f)
    D2 = (cx + w * f, y + dep - dep * f)
    trace = (f"M {x0} {y:.2f} H {cx - w} L {D1[0]:.2f} {D1[1]:.2f} "
             f"Q {cx} {y + dep:.2f} {D2[0]:.2f} {D2[1]:.2f} L {cx + w} {y:.2f} H {x1}")

    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 880 200" width="880" height="200" role="img" aria-labelledby="vbl-title">
  <title id="vbl-title">VerboLang — triângulo invertido de gradientes com três pássaros alinhados no horizonte e traço espectral</title>

  <!-- linha de horizonte (passa sob o emblema; a linha interna dele continua na mesma altura) -->
  <line x1="40" y1="{y:.2f}" x2="840" y2="{y:.2f}" stroke="{gen.HORIZON}" stroke-opacity=".3" stroke-width="1" stroke-dasharray="2 7"/>

  <g transform="translate({ox},{oy}) scale({s})">{inner}</g>

  <!-- wordmark -->
  <text x="520" y="{y - 30:.2f}" text-anchor="middle" font-family="ui-monospace, 'Cascadia Code', Menlo, Consolas, monospace"
        font-size="27" letter-spacing="10" fill="#8a93a3">VERBOLANG</text>

  <!-- traco espectral: azul -> teal -> verde -> amarelo -> vermelho -->
  <defs>
    <linearGradient id="spec" gradientUnits="userSpaceOnUse" x1="{x0}" y1="0" x2="{x1}" y2="0">
      <stop offset="0" stop-color="{gen.BLUE}"/>
      <stop offset="0.25" stop-color="{gen.TEAL}"/>
      <stop offset="0.5" stop-color="{gen.GREEN}"/>
      <stop offset="0.75" stop-color="{gen.YELL}"/>
      <stop offset="1" stop-color="{gen.RED}"/>
    </linearGradient>
  </defs>
  <path d="{trace}" fill="none" stroke="url(#spec)" stroke-width="5" stroke-linecap="round" stroke-linejoin="round"/>

  <!-- nos (halo na cor local + miolo claro, mesmo tratamento do emblema) -->
  <circle cx="345" cy="{y:.2f}" r="7" fill="{gen.TEAL}" opacity="0.22"/>
  <circle cx="345" cy="{y:.2f}" r="3" fill="{gen.CORE}"/>
  <circle cx="{cx}" cy="{y + dep:.2f}" r="8" fill="{gen.GREEN}" opacity="0.22"/>
  <circle cx="{cx}" cy="{y + dep:.2f}" r="3.4" fill="{gen.CORE}"/>
  <circle cx="695" cy="{y:.2f}" r="7" fill="{gen.RED}" opacity="0.22"/>
  <circle cx="695" cy="{y:.2f}" r="3" fill="{gen.CORE}"/>

  <!-- terminais -->
  <line x1="{x0}" y1="{y - 8:.2f}" x2="{x0}" y2="{y + 8:.2f}" stroke="{gen.HORIZON}" stroke-opacity=".5" stroke-width="2" stroke-linecap="round"/>
  <line x1="{x1}" y1="{y - 8:.2f}" x2="{x1}" y2="{y + 8:.2f}" stroke="{gen.HORIZON}" stroke-opacity=".5" stroke-width="2" stroke-linecap="round"/>
</svg>'''

if __name__ == "__main__":
    import os
    here = os.path.dirname(os.path.abspath(__file__))
    open(os.path.join(here, "banner.svg"), "w").write(build_banner())
    print("banner generated (v3)")
