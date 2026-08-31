/*
 * verbolang.js — realce de sintaxe VerboLang para o livro de documentação
 * (site/, mdBook). O mdBook 0.5 não expõe highlight.js global nem registra
 * linguagens novas, então o destaque é feito aqui — escape-primeiro, com o
 * mesmo dialeto de tokens do renderer do dashboard (web/md.js): comentário
 * (tok-c), string (tok-s) e palavra-chave/atributo (tok-k), conforme a
 * FORMAL.md §2.
 *
 * As palavras-chave são a própria tabela da especificação — o teste
 * (tests/unit/web/site.test.js) compara as duas listas: mudou a FORMAL,
 * muda aqui e muda lá.
 *
 * No navegador, roda sozinho sobre <code class="language-verbolang">
 * (e language-vl). Em node, realcar/PALAVRAS são exportados para o teste.
 * Parte do projeto VerboLang — GNU GPL-3.0 (ver LICENSE).
 */
(function (raiz) {
  "use strict";

  // FORMAL §2: conjugações, declarações, controle, ações, atributos e as
  // unidades de tempo/potência que o lexer reconhece como palavras.
  const PALAVRAS = [
    "event", "equilibrium", "nonequilibrium",
    "review", "main",
    "when", "keep", "every",
    "dissolve", "subvert", "reclassify_as_equilibrium",
    "reclassify_as_nonequilibrium", "notify_shutdown", "act",
    "value", "horizon", "source_path", "maintenance_deadline",
    "exchange_mode", "cost_bytes", "currency", "classification",
    "s", "ms", "us", "ns", "W",
  ];

  function esc(s) {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }

  // 1: comentário  2: string  3: palavra-chave — sobre o texto JÁ escapado
  // (esc não toca em `"` nem `/`, então os grupos continuam corretos).
  const RE_TOKEN = new RegExp(
    "(//[^\\n]*|/\\*[\\s\\S]*?\\*/)" +
    "|(\"(?:[^\"\\\\\\n]|\\\\.)*\")" +
    "|\\b(" + PALAVRAS.join("|") + ")\\b",
    "g"
  );

  function realcar(codigo) {
    return esc(String(codigo)).replace(RE_TOKEN, (m, comentario, string, palavra) => {
      if (comentario !== undefined) return '<span class="tok-c">' + comentario + "</span>";
      if (string !== undefined) return '<span class="tok-s">' + string + "</span>";
      if (palavra !== undefined) return '<span class="tok-k">' + palavra + "</span>";
      return m;
    });
  }

  // No navegador, destaca os blocos ```verbolang do livro na carga.
  if (typeof document !== "undefined") {
    const aplicar = () => {
      for (const code of document.querySelectorAll("pre > code.language-verbolang, pre > code.language-vl")) {
        if (code.dataset.vblRealcado) continue;
        code.dataset.vblRealcado = "1";
        code.innerHTML = realcar(code.textContent);
      }
    };
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", aplicar);
    } else {
      aplicar();
    }
  }

  raiz.vblSiteVerbolang = { PALAVRAS, realcar };
  if (typeof module === "object" && module.exports) {
    module.exports = raiz.vblSiteVerbolang;
  }
})(globalThis);
