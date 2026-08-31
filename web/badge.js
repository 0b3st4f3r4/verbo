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
 * os testes. Os textos seguem as 7 línguas da família (vllm.lang — pt, en,
 * zh, ru, hi, af, ar); o padrão "pt" mantém o contrato original. Parte do
 * projeto VerboLang — GNU GPL-3.0 (ver LICENSE).
 */
(function (raiz) {
  "use strict";

  // Textos por língua; {m} = nome do modelo, {s} = código HTTP.
  const TEXTOS = {
    pt: {
      ativo: "modelo ativo: {m}",
      semChave: "modelo no ar, mas sem chave no navegador ({s})",
      chaveInvalida: "chave inválida (HTTP {s})",
      fora: "modelo fora do ar (carregando ou desligado)",
      inativo: "modelo inativo (HTTP {s})",
    },
    en: {
      ativo: "model active: {m}",
      semChave: "model up, but no key in the browser ({s})",
      chaveInvalida: "invalid key (HTTP {s})",
      fora: "model down (loading or off)",
      inativo: "model inactive (HTTP {s})",
    },
    zh: {
      ativo: "模型活跃：{m}",
      semChave: "模型在线，但浏览器中没有密钥（{s}）",
      chaveInvalida: "密钥无效（HTTP {s}）",
      fora: "模型离线（加载中或已关闭）",
      inativo: "模型未激活（HTTP {s}）",
    },
    ru: {
      ativo: "модель активна: {m}",
      semChave: "модель в сети, но в браузере нет ключа ({s})",
      chaveInvalida: "неверный ключ (HTTP {s})",
      fora: "модель недоступна (загружается или выключена)",
      inativo: "модель неактивна (HTTP {s})",
    },
    hi: {
      ativo: "मॉडल सक्रिय: {m}",
      semChave: "मॉडल चालू है, पर ब्राउज़र में कुंजी नहीं ({s})",
      chaveInvalida: "अमान्य कुंजी (HTTP {s})",
      fora: "मॉडल बंद है (लोड हो रहा है या बंद)",
      inativo: "मॉडल निष्क्रिय (HTTP {s})",
    },
    af: {
      ativo: "model aktief: {m}",
      semChave: "model aanlyn, maar geen sleutel in die blaaier ({s})",
      chaveInvalida: "ongeldige sleutel (HTTP {s})",
      fora: "model af (laai of afgeskakel)",
      inativo: "model onaktief (HTTP {s})",
    },
    ar: {
      ativo: "النموذج نشط: {m}",
      semChave: "النموذج متصل لكن لا مفتاح في المتصفح ({s})",
      chaveInvalida: "مفتاح غير صالح (HTTP {s})",
      fora: "النموذج غير متاح (قيد التحميل أو متوقف)",
      inativo: "النموذج غير نشط (HTTP {s})",
    },
  };

  /**
   * Classifica o resultado da sonda GET {base}/v1/models.
   * @param {number|null} status código HTTP, ou null se a rede falhou
   * @param {boolean} temChave  se o navegador tem uma chave para enviar
   * @param {string}  modelo    nome do modelo exibido no badge
   * @param {string}  [lang]    língua dos textos (pt|en|zh|ru|hi|af|ar)
   * @returns {{classe: "ok"|"warn"|"err", texto: string}}
   */
  function classificarModelo(status, temChave, modelo, lang) {
    const t = TEXTOS[lang] || TEXTOS.pt;
    if (status === 200) return { classe: "ok", texto: t.ativo.replace("{m}", modelo) };
    if (status === 401 || status === 403) {
      if (!temChave) {
        return { classe: "warn", texto: t.semChave.replace("{s}", String(status)) };
      }
      return { classe: "err", texto: t.chaveInvalida.replace("{s}", String(status)) };
    }
    if (status === null) return { classe: "err", texto: t.fora };
    return { classe: "err", texto: t.inativo.replace("{s}", String(status)) };
  }

  raiz.classificarModelo = classificarModelo;
  if (typeof module === "object" && module.exports) {
    module.exports = { classificarModelo: classificarModelo, TEXTOS: TEXTOS };
  }
})(globalThis);
