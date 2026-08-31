// -*- coding: utf-8 -*-
// Contrato do renderer markdown do web/ (web/md.js) — regra de ouro AGENTS §2.
// Rodar: node --test tests/unit/web/*.test.js
//
// O dialeto é o nosso (README/FORMAL/PLAN/chat): escape-primeiro (seguro por
// construção), títulos #..######, tabelas, cercas (código/mermaid/verbolang),
// listas 1 nível, citação, hr, inline (código protegido, negrito, itálico,
// tachado, links http + reescrita interna), LaTeX protegido p/ KaTeX.

"use strict";
const assert = require("assert");
const path = require("path");
const { esc, inline, md, hlVerbolang } = require(path.join(__dirname, "../../../web/md.js"));

// ── esc: HTML do source NUNCA vira marcação viva ──────────────────────────
assert.strictEqual(esc("<img src=x onerror=alert(1)>"), "&lt;img src=x onerror=alert(1)&gt;");
assert.strictEqual(esc("a & b < c > d"), "a &amp; b &lt; c &gt; d");

// ── md: títulos deslocam 1 (h1 é da página), id estável + callback p/ TOC ──
{
  const toc = [];
  const html = md("# Guia\n\n## Razão\n\n### Detalhe\n\n###### Fundo", {
    onHeading: (h) => toc.push(h),
  });
  assert.ok(html.includes('<h2 id="sec-1">Guia</h2>'), "h1 do doc vira h2 da página, ancorado");
  assert.ok(html.includes('<h3 id="sec-2">'), "# vira h3");
  assert.ok(html.includes('<h4 id="sec-3">'), "### vira h4");
  assert.ok(html.includes("<h6"), "###### satura em h6 (nunca h7)");
  assert.deepStrictEqual(toc.map((t) => t.text), ["Guia", "Razão", "Detalhe", "Fundo"]);
  assert.ok(toc.every((t) => t.id), "todo título tem id p/ âncora");
  assert.strictEqual(new Set(toc.map((t) => t.id)).size, toc.length, "ids únicos");
}

// ── md: tabela com células inline ──────────────────────────────────────────
{
  const html = md("| a | `b c` |\n|---|---|\n| 1 | **2** |");
  assert.ok(html.includes("<table>") && html.includes("<thead>") && html.includes("<tbody>"));
  assert.ok(html.includes('<th>a</th>') && html.includes('<th><code translate="no">b c</code></th>'));
  assert.ok(html.includes("<td><strong>2</strong></td>"));
}

// ── cercas: verbolang destacado; mermaid vira pre.mermaid-src; resto escapa ─
{
  const html = md('```verbolang\nevent SensorLuz {\n  value: "luz_do_solo",\n  horizon: 2.5s // pousa\n}\n```');
  assert.ok(html.includes("tok-k\">event</span>"), "keyword event destacada");
  assert.ok(html.includes('tok-s">"luz_do_solo"</span>'), "string destacada (aspas literais)");
  assert.ok(html.includes("tok-c\">// pousa</span>"), "comentário destacado");
  assert.ok(!html.includes("<h2"), "cerca não vira título");
}
{
  const html = md("```mermaid\nflowchart TB\n A-->B\n```");
  assert.ok(html.includes('class="mermaid-src"'), "mermaid marcado p/ enhance");
  assert.ok(html.includes("A--&gt;B"), "código do diagrama escapado");
}
{
  const html = md("```\n<b>&i</b>\n```");
  assert.ok(html.includes("&lt;b&gt;"), "código simples escapado");
}

// ── listas, citação, hr, parágrafos ────────────────────────────────────────
{
  const html = md("- um\n- dois\n  continuado\n\n> cita\n\n---\n\ntexto");
  assert.ok(html.includes("<ul><li>um</li><li>dois continuado</li></ul>"), "lista + continuação");
  assert.ok(html.includes("<blockquote>cita</blockquote>"));
  assert.ok(html.includes("<hr>"));
  assert.ok(html.includes("<p>texto</p>"));
}

// ── inline: código protege a marcação; links http vs internos .md ──────────
{
  const s = inline("`**não** é forte` e **isto é**");
  assert.ok(s.includes('<code translate="no">**não** é forte</code>'), "código protege ** **");
  assert.ok(s.includes("<strong>isto é</strong>"));
}
{
  const s = inline("[ir](https://exemplo.org/a)");
  assert.ok(s.includes('href="https://exemplo.org/a"') && s.includes('rel="noopener noreferrer"'));
}
{
  const reescrito = inline("[FORMAL](docs/FORMAL.md)", { internal: (h) => "?doc=" + h });
  assert.ok(reescrito.includes('href="?doc=docs/FORMAL.md"'), "link .md reescrito via opts.internal");
  const puro = inline("[FORMAL](docs/FORMAL.md)");
  assert.ok(!puro.includes("<a "), "sem opts.internal, link .md fica inerte (texto)");
}
{
  const s = inline("[x](javascript:alert(1))");
  assert.ok(!s.includes("<a "), "esquema não-http não vira link");
}

// ── plugins de tradução: código/diagrama/TeX carregam translate="no" ───────
{
  const html = md('```verbolang\nevent A { horizon: 1s }\n```\n\n```mermaid\nA-->B\n```\n\n```\ncru\n```\n\ncmd `pwd` $$E=m c^2$$');
  assert.ok(html.includes('<pre class="vl" translate="no">'), "bloco verbolang fora do alcance dos plugins");
  assert.ok(html.includes('<pre class="mermaid-src" translate="no">'), "mermaid fora do alcance");
  assert.ok(html.includes('<pre translate="no">'), "cerca simples fora do alcance");
  assert.ok(html.includes('<code translate="no">pwd</code>'), "código inline fora do alcance");
  assert.ok(html.includes('class="katex-src mdisp" translate="no"'), "TeX fora do alcance");
}

// ── LaTeX protegido (KaTeX depois; fallback TeX cru) ───────────────────────
{
  const s = md("energia $$E = m c^2$$ fim");
  assert.ok(s.includes('class="katex-src mdisp"'), "display marcado");
  assert.ok(s.includes("E = m c^2"), "TeX preservado p/ render");
}

// ── hlVerbolang: conjugações, ações, atributos; comentário vence ───────────
{
  const h = hlVerbolang("// event falso\nevent A { horizon: 1s }");
  assert.ok(h.includes("tok-c\">// event falso</span>"), "comentário inteiro é comentário");
  const ks = h.match(/tok-k/g).length;
  assert.ok(ks === 2, `keywords destacadas: event e horizon (${ks})`);
  assert.ok(!/tok-k">[^<]*<\/span> event/.test(h), "sem dupla marcação");
}
{
  const h = hlVerbolang('subvert quando "poesia_gerada_pelo_calor_do_silicio_e_resfriamento_da_mente"');
  assert.ok(h.includes("tok-s\">\"poesia"), "string com escapes intacta");
}

console.log("✓ md.test.js — dialeto markdown do web/ com escape-primeiro e verbolang");
