/*
 * badge.js — estado do modelo local, honesto por construção (dashboard web/).
 *
 * A matriz de estados é definida em tests/unit/web/badge.test.js (regra de
 * ouro, AGENTS §2) e cobre o caso medido em 31/08/2026: o vLLM exige chave,
 * então 401 com navegador sem chave NÃO significa "modelo inativo" — o
 * servidor está no ar; falta a credencial (abra a UI pela URL do
 * serve-local-llm.sh, que traz #k=…, ou defina a chave).
 *
 * Usado por index.html e metrics.html (script clássico, global
 * `classificarModelo`); o mesmo arquivo roda em node (module.exports) para
 * os testes. Parte do projeto VerboLang — GNU GPL-3.0 (ver LICENSE).
 */
(function (raiz) {
  "use strict";

  /**
   * Classifica o resultado da sonda GET {base}/v1/models.
   * @param {number|null} status código HTTP, ou null se a rede falhou
   * @param {boolean} temChave  se o navegador tem uma chave para enviar
   * @param {string}  modelo    nome do modelo exibido no badge
   * @returns {{classe: "ok"|"warn"|"err", texto: string}}
   */
  function classificarModelo(status, temChave, modelo) {
    if (status === 200) return { classe: "ok", texto: "modelo ativo: " + modelo };
    if (status === 401 || status === 403) {
      if (!temChave) {
        return {
          classe: "warn",
          texto: "modelo no ar, mas sem chave no navegador (" + status + ")",
        };
      }
      return { classe: "err", texto: "chave inválida (HTTP " + status + ")" };
    }
    if (status === null) {
      return { classe: "err", texto: "modelo fora do ar (carregando ou desligado)" };
    }
    return { classe: "err", texto: "modelo inativo (HTTP " + status + ")" };
  }

  raiz.classificarModelo = classificarModelo;
  if (typeof module === "object" && module.exports) {
    module.exports = { classificarModelo: classificarModelo };
  }
})(globalThis);
