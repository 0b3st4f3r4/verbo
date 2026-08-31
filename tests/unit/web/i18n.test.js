// -*- coding: utf-8 -*-
// Testes de i18n do dashboard (web/) — regra de ouro AGENTS §2: o contrato
// de paridade existe junto com a implementação. Rodar: node --test tests/unit/web/
//
// Contrato: as três superfícies com texto (badge.js, index.html, metrics.html)
// falam EXATAMENTE as 7 línguas da família (pt, en, zh, ru, hi, af, ar), com
// paridade total de chaves entre línguas e nenhum valor vazio. O chat
// (chat.html) é a referência histórica das 7.

"use strict";
const assert = require("assert");
const fs = require("fs");
const path = require("path");

const WEB = path.join(__dirname, "../../../web");
const LINGUAS = ["pt", "en", "zh", "ru", "hi", "af", "ar"];

function extrairObjeto(arquivo, declaracao) {
  const s = fs.readFileSync(path.join(WEB, arquivo), "utf8");
  const i = s.indexOf(declaracao);
  assert.ok(i >= 0, `${arquivo}: "${declaracao}" não encontrado`);
  // fatia do literal do objeto e fecha as chaves contando o balanceamento
  let j = s.indexOf("{", i);
  let prof = 0, dentro = false;
  for (let k = j; k < s.length; k++) {
    const c = s[k];
    if (c === "\"" || c === "'" || c === "`") {         // pula strings (sem escapes bugados nos dicionários)
      const aspa = c;
      k++;
      while (k < s.length && s[k] !== aspa) k += s[k] === "\\" ? 2 : 1;
      continue;
    }
    if (c === "{") { prof++; dentro = true; }
    if (c === "}" && dentro) { prof--; if (prof === 0) { j = k + 1; break; } }
  }
  return new Function(`return (${s.slice(s.indexOf("{", i), j)})`)();
}

// ── badge.js: TEXTOS com paridade e placeholders preservados ──────────────
const { TEXTOS, classificarModelo } = require(path.join(WEB, "badge.js"));
assert.deepStrictEqual(Object.keys(TEXTOS).sort(), [...LINGUAS].sort(), "badge.js deve falar as 7 línguas");
const chavesBadge = Object.keys(TEXTOS.pt).sort();
for (const l of LINGUAS) {
  assert.deepStrictEqual(Object.keys(TEXTOS[l]).sort(), chavesBadge, `badge.js[${l}]: paridade de chaves`);
  for (const [k, v] of Object.entries(TEXTOS[l])) assert.ok(v.trim(), `badge.js[${l}].${k} vazio`);
}
// amostra por língua: 401 sem chave cita o código; 200 cita o modelo
for (const l of LINGUAS) {
  const r = classificarModelo(401, false, "qwen3-4b", l);
  assert.ok(r.texto.includes("401"), `badge[${l}] 401 deve citar o código`);
  assert.ok(classificarModelo(200, false, "m1", l).texto.includes("m1"), `badge[${l}] 200 deve citar o modelo`);
}
// padrão pt: o contrato original permanece
assert.strictEqual(classificarModelo(200, false, "qwen3-4b").texto, "modelo ativo: qwen3-4b");

// ── index.html e metrics.html: 7 línguas, paridade de chaves, sem vazio ───
function verificarPagina(arquivo, declaracao) {
  const d = extrairObjeto(arquivo, declaracao);
  assert.deepStrictEqual(Object.keys(d).sort(), [...LINGUAS].sort(),
    `${arquivo}: ${declaracao} deve falar as 7 línguas`);
  const chaves = Object.keys(d.pt).filter((k) => k !== "dir").sort();
  for (const l of LINGUAS) {
    assert.deepStrictEqual(
      Object.keys(d[l]).filter((k) => k !== "dir").sort(), chaves,
      `${arquivo}[${l}]: paridade de chaves com pt`);
    for (const k of chaves) assert.ok(String(d[l][k]).trim(), `${arquivo}[${l}].${k} vazio`);
    assert.ok(d[l].dir === "ltr" || d[l].dir === "rtl", `${arquivo}[${l}].dir inválido`);
  }
  return d;
}

const home = verificarPagina("index.html", "const HOME_I18N");
const metrics = verificarPagina("metrics.html", "const I18N");

// rtl somente no árabe, nas duas páginas (o chat já define dir por língua)
for (const l of LINGUAS) {
  const esperado = l === "ar" ? "rtl" : "ltr";
  assert.strictEqual(home[l].dir, esperado, `index[${l}].dir`);
  assert.strictEqual(metrics[l].dir, esperado, `metrics[${l}].dir`);
}

// termos técnicos que nunca se traduzem (âncoras de honestidade do produto)
for (const l of LINGUAS) {
  assert.ok(metrics[l].foot.includes("vbl ledger-verify"), `metrics[${l}].foot perdeu o comando de verificação`);
  assert.ok(metrics[l].ledgerNenhum.includes("vbl run"), `metrics[${l}].ledgerNenhum perdeu o vbl run`);
  assert.ok(metrics[l].bSseTitle.includes("webui.py"), `metrics[${l}].bSseTitle perdeu a ponte`);
}

console.log("✓ i18n.test.js — 7 línguas com paridade em badge.js, index.html e metrics.html");
