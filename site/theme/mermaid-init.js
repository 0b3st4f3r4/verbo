/*
 * mermaid-init.js — diagramas Mermaid do livro, com o mermaid vendored do
 * repositório (web/vendor/mermaid.min.js, MIT — ver NOTICE). O montador
 * (scripts/build_site.py) copia o vendor para assets/vendor/, irmão deste
 * arquivo.
 *
 * O tema do diagrama segue o tema do mdBook (classe no <html>): claro →
 * "neutral", escuro → "dark". Um MutationObserver re-renderiza quando o
 * leitor troca de tema. Diagrama inválido: o código permanece visível,
 * contornado em tracejado vermelho (honestidade do erro).
 * Parte do projeto VerboLang — GNU GPL-3.0 (ver LICENSE).
 */
(function () {
  "use strict";

  let sequencia = 0;
  let carregando = null;
  const renderizados = [];

  function temaMermaid() {
    const classe = document.documentElement.className;
    return /\b(light|rust)\b/.test(classe) ? "neutral" : "dark";
  }

  function carregarMermaid() {
    if (window.mermaid) return Promise.resolve();
    if (!carregando) {
      const base = (document.currentScript && document.currentScript.src || "")
        .replace(/[^/]*$/, "");
      carregando = new Promise((resolver, recusar) => {
        const s = document.createElement("script");
        s.src = base + "vendor/mermaid.min.js";
        s.onload = resolver;
        s.onerror = () => recusar(new Error("mermaid.min.js não carregou"));
        document.head.appendChild(s);
      }).then(() => {
        window.mermaid.initialize({
          startOnLoad: false,
          securityLevel: "strict",
          theme: temaMermaid(),
          fontFamily: "Inter, system-ui, sans-serif",
        });
      });
    }
    return carregando;
  }

  function aplicarTema() {
    if (!window.mermaid) return;
    window.mermaid.initialize({
      startOnLoad: false,
      securityLevel: "strict",
      theme: temaMermaid(),
      fontFamily: "Inter, system-ui, sans-serif",
    });
    for (const r of renderizados) {
      window.mermaid.render("vbl-mmd-" + (++sequencia), r.src)
        .then(({ svg }) => { r.caixa.innerHTML = svg; })
        .catch(() => {});
    }
  }

  function renderizar() {
    const blocos = document.querySelectorAll("pre > code.language-mermaid");
    if (!blocos.length) return;
    carregarMermaid().then(() => {
      for (const code of blocos) {
        if (code.dataset.vblMermaid) continue;
        code.dataset.vblMermaid = "1";
        const src = code.textContent;
        window.mermaid.render("vbl-mmd-" + (++sequencia), src)
          .then(({ svg }) => {
            const caixa = document.createElement("div");
            caixa.className = "mermaid-box";
            caixa.innerHTML = svg;
            code.parentElement.replaceWith(caixa);
            renderizados.push({ caixa, src });
          })
          .catch(() => {
            const pre = code.parentElement;
            if (pre) pre.classList.add("mermaid-error");
          });
      }
    }).catch(() => { /* sem vendor: o código dos diagramas fica legível */ });
  }

  if (typeof document !== "undefined") {
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", renderizar);
    } else {
      renderizar();
    }
    new MutationObserver(aplicarTema).observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class"],
    });
  }
})();
