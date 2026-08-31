/*
 * md.js — renderer markdown do web/ (dialeto do projeto, escape-primeiro).
 *
 * Extraído do chat.html (que tem cópia inline; migra depois) e estendido
 * para a documentação (docs.html): títulos #..###### com id estável +
 * callback de índice, cercas ```verbolang com destaque próprio e reescrita
 * de links internos (.md) via opts.internal.
 *
 * Seguro por construção: TODO texto vira esc() antes de qualquer marcação —
 * HTML do source nunca vira marcação viva; links só http(s) (ou reescrita
 * explícita via opts.internal). O mesmo arquivo roda em node (module.exports)
 * para os testes (tests/unit/web/md.test.js).
 *
 * Plugins de tradução (Google/DeepL/Edge): código inline, cercas e TeX saem
 * com translate="no" — o plugin traduz a prosa e não corrompe EBNF, comandos
 * nem diagramas (o lang="pt-BR" do conteúdo fica no docs.html).
 * Parte do projeto VerboLang — GNU GPL-3.0 (ver LICENSE).
 */
(function (raiz) {
  "use strict";

  function esc(s) {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }

  // inline: código protegido → negrito → itálico → tachado → links; LaTeX
  // (\x03/\x05 já marcados por stripLatex) vira span.katex-src p/ KaTeX.
  function inline(s, opts) {
    s = esc(s);
    const codes = [];
    s = s.replace(/`([^`]+)`/g, (_, c) => {          // protege o conteúdo do código
      codes.push(c);
      return "\x00" + (codes.length - 1) + "\x00";
    });
    const maths = [];                                 // protege a matemática
    s = s.replace(/([\x03\x05])([^\x03\x05]+)\1/g, (_, d, tex) => {
      maths.push({ tex, disp: d === "\x05" });
      return "\x04" + (maths.length - 1) + "\x04";
    });
    s = s
      .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
      .replace(/(^|[\s(])\*([^*\n]+)\*(?=[\s).,!?:;]|$)/g, "$1<em>$2</em>")
      .replace(/~~([^~]+)~~/g, "<del>$1</del>");
    s = s.replace(/\[([^\]]+)\]\((https?:[^)\s]+)\)/g,
      '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>');
    if (opts && typeof opts.internal === "function") {
      s = s.replace(/\[([^\]]+)\]\(([^):]+\.md)\)/g, (m0, txt, href) => {
        const destino = opts.internal(href);
        return destino ? '<a href="' + destino + '">' + txt + "</a>" : m0;
      });
    }
    s = s.replace(/\x00(\d+)\x00/g, (_, i) => '<code translate="no">' + codes[+i] + "</code>");
    return s.replace(/\x04(\d+)\x04/g, (_, i) =>      // placeholder → KaTeX
      '<span class="katex-src' + (maths[+i].disp ? " mdisp" : "") + '" translate="no">' +
      maths[+i].tex + "</span>");
  }

  // LaTeX vazado (\( \), \[ \], $$ $$) → marcadores p/ inline() proteger.
  function stripLatex(src) {
    const mk = (ch) => (_, m) => ch + m.replace(/\s+/g, " ").trim() + ch;
    return src
      .replace(/\\\(([\s\S]*?)\\\)/g, mk("\x03"))   // inline
      .replace(/\\\[([\s\S]*?)\\\]/g, mk("\x05"))   // display
      .replace(/\$\$([\s\S]*?)\$\$/g, mk("\x05"));
  }

  // verbolang: conjugações/declarações/controle/ações/atributos (FORMAL §2).
  const VL_TOKEN = new RegExp(
    "(//[^\\n]*|/\\*[\\s\\S]*?\\*/)" +                 // 1: comentário
    "|(\"(?:[^\"\\\\\\n]|\\\\.)*\")" +                 // 2: string
    "|\\b(event|equilibrium|nonequilibrium|review|main|when|keep|every|" +
    "dissolve|subvert|reclassify_as_equilibrium|reclassify_as_nonequilibrium|" +
    "notify_shutdown|act|value|horizon|source_path|maintenance_deadline|" +
    "exchange_mode|cost_bytes|currency|classification|on)\\b", "g"); // 3: keyword

  function hlVerbolang(code) {
    const s = esc(code);
    let out = "", last = 0, m;
    VL_TOKEN.lastIndex = 0;
    while ((m = VL_TOKEN.exec(s)) !== null) {
      out += s.slice(last, m.index);
      if (m[1] !== undefined) out += '<span class="tok-c">' + m[1] + "</span>";
      else if (m[2] !== undefined) out += '<span class="tok-s">' + m[2] + "</span>";
      else out += '<span class="tok-k">' + m[3] + "</span>";
      last = VL_TOKEN.lastIndex;
    }
    return out + s.slice(last);
  }

  // md: blocos (cerca, título, tabela, lista, citação, hr) + inline.
  // opts: { headingBase = 1, onHeading({level, text, id}), internal(href) }.
  function md(src, opts) {
    const base = (opts && opts.headingBase) || 1;
    // protege cercas de código do tratamento LaTeX
    const fences = [];
    src = src.replace(/\r\n?/g, "\n").replace(/```[\s\S]*?(?:```|$)/g, (m0) => {
      fences.push(m0);
      return "\x02" + (fences.length - 1) + "\x02";
    });
    src = stripLatex(src);
    src = src.replace(/\x02(\d+)\x02/g, (_, i) => fences[+i]);
    const lines = src.split("\n");
    const out = [];
    let para = [];
    let i = 0;
    let sec = 0;
    const flushPara = () => {
      if (para.length) { out.push("<p>" + para.map((l) => inline(l, opts)).join("<br>") + "</p>"); para = []; }
    };
    while (i < lines.length) {
      const line = lines[i];
      const fence = line.match(/^\s*(```+|~~~+)\s*(\S*)\s*$/);
      if (fence) {                                    // bloco cercado
        flushPara();
        const lang = (fence[2] || "").toLowerCase();
        const buf = []; i++;
        while (i < lines.length && !/^\s*(```+|~~~+)\s*$/.test(lines[i])) { buf.push(lines[i]); i++; }
        i++;
        const code = buf.join("\n");
        if (lang === "verbolang" || lang === "vl") {
          out.push('<pre class="vl" translate="no"><code>' + hlVerbolang(code) + "</code></pre>");
        } else if (lang === "mermaid") {
          out.push('<pre class="mermaid-src" translate="no">' + esc(code) + "</pre>");
        } else {
          out.push('<pre translate="no"><code>' + esc(code) + "</code></pre>");
        }
        continue;
      }
      if (/^\s*$/.test(line)) { flushPara(); i++; continue; }
      const h = line.match(/^(#{1,6})\s+(.*)$/);
      if (h) {                                        // título → h(base+1)..h6
        flushPara();
        const nivel = Math.min(h[1].length + base, 6);
        const id = "sec-" + (++sec);
        const texto = h[2];
        if (opts && typeof opts.onHeading === "function") {
          opts.onHeading({ level: nivel, text: texto, id });
        }
        out.push('<h' + nivel + ' id="' + id + '">' + inline(texto, opts) + "</h" + nivel + ">");
        i++; continue;
      }
      if (/^\s*(-{3,}|\*{3,})\s*$/.test(line)) { flushPara(); out.push("<hr>"); i++; continue; }
      if (/^\s*>/.test(line)) {                       // citação
        flushPara();
        const buf = [];
        while (i < lines.length && /^\s*>/.test(lines[i])) { buf.push(lines[i].replace(/^\s*>\s?/, "")); i++; }
        out.push("<blockquote>" + buf.map((l) => inline(l, opts)).join("<br>") + "</blockquote>");
        continue;
      }
      if (line.includes("|") && i + 1 < lines.length &&
          /^\s*\|?[\s:|-]+\|?\s*$/.test(lines[i + 1]) && lines[i + 1].includes("-")) {
        flushPara();                                  // tabela
        const row = (l) => l.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map((c) => inline(c.trim(), opts));
        const head = row(line); i += 2;
        const rows = [];
        while (i < lines.length && lines[i].includes("|") && !/^\s*$/.test(lines[i])) { rows.push(row(lines[i])); i++; }
        let tbl = '<div class="tblwrap"><table><thead><tr>' +
                  head.map((c) => "<th>" + c + "</th>").join("") + "</tr></thead><tbody>";
        for (const r of rows) tbl += "<tr>" + r.map((c) => "<td>" + c + "</td>").join("") + "</tr>";
        out.push(tbl + "</tbody></table></div>");
        continue;
      }
      const li = line.match(/^(\s*)([-*]|\d+[.)])\s+(.*)$/);
      if (li) {                                       // lista (1 nível + continuação)
        flushPara();
        const ordered = /\d/.test(li[2]);
        const items = [];
        while (i < lines.length) {
          const m = lines[i].match(/^(\s*)([-*]|\d+[.)])\s+(.*)$/);
          if (!m) break;
          items.push(m[3]); i++;
          while (i < lines.length && /^\s{2,}\S/.test(lines[i]) && !/^\s*([-*]|\d+[.)])\s/.test(lines[i])) {
            items[items.length - 1] += " " + lines[i].trim(); i++;
          }
        }
        const tag = ordered ? "ol" : "ul";
        out.push("<" + tag + ">" + items.map((x) => "<li>" + inline(x, opts) + "</li>").join("") + "</" + tag + ">");
        continue;
      }
      para.push(line); i++;
    }
    flushPara();
    return out.join("");
  }

  raiz.vblMd = { esc, inline, stripLatex, hlVerbolang, md };
  if (typeof module === "object" && module.exports) {
    module.exports = raiz.vblMd;
  }
})(globalThis);
