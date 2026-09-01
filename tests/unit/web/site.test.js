// -*- coding: utf-8 -*-
// Contrato do tema/assets do site de documentação (site/, mdBook) — regra
// de ouro AGENTS §2. Rodar: node --test tests/unit/web/site.test.js
//
// Três superfícies:
//   site/assets/i18n.js      — cromo do site nas 7 línguas da família
//   site/assets/verbolang.js — gramática verbolang p/ highlight.js
//   site/content/SUMMARY.md  — a estante do livro (trilha, referência, projeto)
//
// O conteúdo dos capítulos é pt-BR (fonte canônica); o cromo segue o
// contrato da família: paridade total de chaves entre línguas, nenhum
// valor vazio, termo técnico intocável.

"use strict";
const assert = require("assert");
const fs = require("fs");
const path = require("path");

const RAIZ = path.join(__dirname, "../../../");
const SITE = path.join(RAIZ, "site");
const LINGUAS = ["pt", "en", "zh", "ru", "hi", "af", "ar"];

// ── i18n.js: paridade ×7, sem vazio, placeholder preservado ───────────────
const { I18N, idiomaPreferido, aplicar } = require(path.join(SITE, "theme/i18n.js"));
assert.deepStrictEqual(Object.keys(I18N).sort(), [...LINGUAS].sort(),
  "i18n do site deve falar as 7 línguas da família");
const chaves = Object.keys(I18N.pt).sort();
assert.deepStrictEqual(chaves, ["busca", "buscaPlaceholder", "imprimir", "repositorio", "tema"],
  "cromo do site: busca, tema, repo e impressão");
for (const l of LINGUAS) {
  assert.deepStrictEqual(Object.keys(I18N[l]).sort(), chaves, `i18n[${l}]: paridade de chaves`);
  for (const [k, v] of Object.entries(I18N[l])) assert.ok(String(v).trim(), `i18n[${l}].${k} vazio`);
}

// fallback pt fora das 7 e sem navigator/localStorage (node não os tem)
assert.strictEqual(idiomaPreferido({}), "pt");
assert.strictEqual(idiomaPreferido({ lang: "ru" }), "ru");
assert.strictEqual(idiomaPreferido({ navegador: "fr-FR" }), "pt",
  "língua fora da família cai no pt");

// aplicar() troca os textos do cromo num DOM fingido — os seletores são os
// IDs/cl reais que o mdBook 0.5 emite (verificados no livro compilado)
function noFalso() {
  const elementos = {};
  return {
    querySelector: (sel) => {
      elementos[sel] ||= { placeholder: "", title: "" };
      return elementos[sel];
    },
    _elementos: elementos,
  };
}
{
  const doc = noFalso();
  aplicar(doc, I18N.pt);
  const e = doc._elementos;
  assert.strictEqual(e["#mdbook-searchbar"].placeholder, I18N.pt.buscaPlaceholder);
  assert.strictEqual(e["#mdbook-search-toggle"].title, I18N.pt.busca);
  assert.strictEqual(e["#mdbook-theme-toggle"].title, I18N.pt.tema);
  assert.strictEqual(e['a[title="Git repository"]'].title, I18N.pt.repositorio);
  assert.strictEqual(e['a[title="Print this book"]'].title, I18N.pt.imprimir);
}

// ── verbolang.js: realce próprio (sem hljs — o mdBook 0.5 não o expõe) ────
const { PALAVRAS, realcar } = require(path.join(SITE, "theme/verbolang.js"));
// palavras-chave da especificação (FORMAL.md §2) — o teste é a própria tabela:
const FORMAIS = [
  "event", "equilibrium", "nonequilibrium",
  "review", "main",
  "when", "keep", "every",
  "dissolve", "subvert", "reclassify_as_equilibrium",
  "reclassify_as_nonequilibrium", "notify_shutdown", "act",
  "value", "horizon", "source_path", "maintenance_deadline",
  "exchange_mode", "cost_bytes", "currency", "classification",
  "s", "ms", "us", "ns", "W",
];
assert.deepStrictEqual(PALAVRAS.slice().sort(), FORMAIS.slice().sort(),
  "gramática verbolang diverge da FORMAL §2 (nem mais, nem menos)");

const html = realcar('event X { value: "v", horizon: 2s } // blink');
assert.ok(html.includes('<span class="tok-k">event</span>'), "palavra-chave sem realce");
assert.ok(html.includes('<span class="tok-k">horizon</span>'), "atributo sem realce");
assert.ok(html.includes('<span class="tok-s">"v"</span>'), "string sem realce");
assert.ok(html.includes('<span class="tok-c">// blink</span>'), "comentário sem realce");
assert.ok(html.includes("2s"), "duração sumiu no realce");
assert.strictEqual(realcar('<script>'), "&lt;script&gt;", "realce deve escapar antes de marcar");

// ── SUMMARY.md: a estante — trilha completa, seções = pastas ──────────────
const summary = fs.readFileSync(path.join(SITE, "content/SUMMARY.md"), "utf-8");
const entradas = [...summary.matchAll(/\]\(([^)#]+\.md)\)/g)].map((m) => m[1]);
assert.ok(entradas.length >= 15, "SUMMARY com poucas entradas");
for (const secao of ["Trilha", "Referência", "Projeto"]) {
  assert.ok(new RegExp(`^# ${secao}\\s*$`, "m").test(summary), `SUMMARY sem a parte "${secao}"`);
}
// fontes da trilha existem; fontes da referência/projeto existem no repo
const FONTE_DA_ENTRADA = {
  "guide/": (p) => path.join(SITE, "content", p),
  "reference/": null, // montada — checada pelo pytest (test_site_build.py)
  "project/": null,
};
for (const entrada of entradas) {
  if (entrada.startsWith("guide/")) {
    assert.ok(fs.existsSync(path.join(SITE, "content", entrada)),
      `SUMMARY lista ${entrada} e site/content não a tem`);
  }
}
// capítulos da trilha, na ordem didática
const trilha = entradas.filter((e) => e.startsWith("guide/"));
assert.deepStrictEqual(trilha, [
  "guide/visao-geral.md",
  "guide/instalacao.md",
  "guide/formas.md",
  "guide/revisoes.md",
  "guide/fxp.md",
  "guide/caderno.md",
  "guide/receitas.md",
], "ordem didática da trilha mudou — revise SUMMARY e este contrato");

// landing (content/README.md) cita a versão da linha e a marca
const landing = fs.readFileSync(path.join(SITE, "content/README.md"), "utf-8");
assert.ok(/v2027\.0\.0-alpha\./.test(landing), "landing sem a versão da linha");
assert.ok(landing.includes("docs/brand/verbolog-triangle.svg"), "landing sem o emblema da marca");

// ── book.toml: assets declarados existem (em site/theme/ — o montador os
//    leva para src/assets/) ─────────────────────────────────────────────────
const toml = fs.readFileSync(path.join(SITE, "book.toml"), "utf-8");
assert.ok(/language\s*=\s*"pt-BR"/.test(toml), "book.toml sem language pt-BR");
assert.ok(/create-missing\s*=\s*false/.test(toml), "book.toml deve falhar com capítulo faltando");
for (const m of toml.matchAll(/"([^"]+\.(?:css|js))"/g)) {
  // vendor vendored tem fonte própria (web/vendor, rastreada — ver NOTICE);
  // todo o resto vem do tema (site/theme/).
  const fonte = m[1].startsWith("src/assets/vendor/")
    ? path.join(RAIZ, m[1].replace(/^src\/assets\//, "web/"))
    : path.join(SITE, m[1].replace(/^src\/assets\//, "theme/"));
  assert.ok(fs.existsSync(fonte),
    `book.toml lista ${m[1]} e a fonte não existe (${fonte})`);
}

console.log("✓ site.test.js: i18n ×7, gramática verbolang, SUMMARY e book.toml coerentes");
