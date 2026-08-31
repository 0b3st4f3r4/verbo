/*
 * i18n.js — cromo do site de documentação nas 7 línguas da família
 * (pt, en, zh, ru, hi, af, ar). O CONTEÚDO dos capítulos é pt-BR (fonte
 * canônica, como nos .md do repositório); o que se traduz aqui é só o
 * cromo do mdBook: placeholder e botão de busca, tema, repositório e
 * impressão.
 *
 * Contrato (testado em tests/unit/web/site.test.js): paridade total de
 * chaves entre línguas, nenhum valor vazio, fallback pt. O site obedece ao
 * `vllm.lang` da família (localStorage) quando existir e, senão, à língua
 * do navegador — só que esteja entre as 7. Parte do projeto VerboLang —
 * GNU GPL-3.0 (ver LICENSE).
 */
(function (raiz) {
  "use strict";

  const I18N = {
    pt: {
      buscaPlaceholder: "Pesquisar no livro…",
      busca: "Buscar (`/`)",
      tema: "Mudar tema",
      repositorio: "Repositório no GitHub",
      imprimir: "Imprimir este livro",
    },
    en: {
      buscaPlaceholder: "Search the book…",
      busca: "Search (`/`)",
      tema: "Change theme",
      repositorio: "Repository on GitHub",
      imprimir: "Print this book",
    },
    zh: {
      buscaPlaceholder: "搜索本书…",
      busca: "搜索（`/`）",
      tema: "更换主题",
      repositorio: "GitHub 仓库",
      imprimir: "打印本书",
    },
    ru: {
      buscaPlaceholder: "Поиск по книге…",
      busca: "Поиск (`/`)",
      tema: "Сменить тему",
      repositorio: "Репозиторий на GitHub",
      imprimir: "Печать книги",
    },
    hi: {
      buscaPlaceholder: "पुस्तक में खोजें…",
      busca: "खोज (`/`)",
      tema: "थीम बदलें",
      repositorio: "GitHub रिपॉज़िटरी",
      imprimir: "यह पुस्तक मुद्रित करें",
    },
    af: {
      buscaPlaceholder: "Deur die boek soek…",
      busca: "Soek (`/`)",
      tema: "Verander tema",
      repositorio: "Repository op GitHub",
      imprimir: "Druk hierdie boek",
    },
    ar: {
      buscaPlaceholder: "ابحث في الكتاب…",
      busca: "بحث (`/`)",
      tema: "تغيير المظهر",
      repositorio: "المستودع على GitHub",
      imprimir: "طباعة هذا الكتاب",
    },
  };

  // A língua preferida: env explícito → vllm.lang da família → navegador
  // → pt. Fora do navegador (testes em node), não há navigator/localStorage.
  function idiomaPreferido(env) {
    env = env || {};
    const candidatos = [env.lang];
    const noNavegador = typeof document !== "undefined";
    if (noNavegador) {
      try {
        candidatos.push(raiz.localStorage && raiz.localStorage.getItem("vllm.lang"));
      } catch (e) { /* storage bloqueado — segue o baile */ }
    }
    candidatos.push(env.navegador);
    if (noNavegador && raiz.navigator && raiz.navigator.language) {
      candidatos.push(raiz.navigator.language);
    }
    for (const candidato of candidatos) {
      if (!candidato) continue;
      const chave = String(candidato).slice(0, 2).toLowerCase();
      if (I18N[chave]) return chave;
    }
    return "pt";
  }

  // Troca os textos do cromo; seletores são os que o mdBook 0.5 emite.
  function aplicar(doc, dicionario) {
    const t = dicionario || I18N[idiomaPreferido()];
    const pega = (sel) => {
      try { return doc.querySelector(sel); } catch (e) { return null; }
    };
    const barra = pega("#mdbook-searchbar");
    if (barra) barra.placeholder = t.buscaPlaceholder;
    const toggleBusca = pega("#mdbook-search-toggle");
    if (toggleBusca) toggleBusca.title = t.busca;
    const botaoTema = pega("#mdbook-theme-toggle");
    if (botaoTema) botaoTema.title = t.tema;
    const repo = pega('a[title="Git repository"]');
    if (repo) repo.title = t.repositorio;
    const imprimir = pega('a[title="Print this book"]');
    if (imprimir) imprimir.title = t.imprimir;
    return t;
  }

  // No navegador, aplica sozinho na carga.
  if (typeof document !== "undefined") {
    aplicar(document, I18N[idiomaPreferido()]);
  }

  raiz.vblSiteI18n = { I18N, idiomaPreferido, aplicar };
  if (typeof module === "object" && module.exports) {
    module.exports = raiz.vblSiteI18n;
  }
})(globalThis);
