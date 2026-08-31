// -*- coding: utf-8 -*-
// Testes do badge do modelo (web/badge.js) — regra de ouro AGENTS §2:
// o contrato existe ANTES da implementação. Rodar com: node badge.test.js
//
// Matriz de estados (401 diagnosticado em 31/08/2026: vLLM no ar exige
// chave; sem chave no navegador ⇒ 401 — NÃO é "modelo inativo"):
//   HTTP 200            → ok   "modelo ativo: <modelo>"
//   HTTP 401/403 s/ chv → warn "modelo no ar, mas sem chave no navegador (401)"
//   HTTP 401/403 c/ chv → err  "chave inválida (HTTP 401)"
//   outro HTTP          → err  "modelo inativo (HTTP <code>)"
//   rede falhou (null)  → err  "modelo fora do ar (carregando ou desligado)"

"use strict";
const assert = require("assert");
const path = require("path");
const { classificarModelo } = require(path.join(__dirname, "../../../web/badge.js"));

// 200: ativo, com ou sem chave (a chave só importa para conversar)
assert.deepStrictEqual(
  classificarModelo(200, false, "qwen3-4b"),
  { classe: "ok", texto: "modelo ativo: qwen3-4b" },
  "200 sem chave deveria ser ok/ativo",
);
assert.strictEqual(classificarModelo(200, true, "m1").classe, "ok", "200 com chave deveria ser ok");

// 401/403 sem chave no navegador: modelo NO AR — estado intermediário honesto
for (const codigo of [401, 403]) {
  const r = classificarModelo(codigo, false, "qwen3-4b");
  assert.strictEqual(r.classe, "warn", `HTTP ${codigo} sem chave deveria ser warn`);
  assert.ok(/sem chave/.test(r.texto), `texto deve mencionar "sem chave": ${r.texto}`);
  assert.ok(r.texto.includes(String(codigo)), `texto deve citar o código ${codigo}: ${r.texto}`);
}

// 401/403 com chave enviada: a chave está errada
for (const codigo of [401, 403]) {
  const r = classificarModelo(codigo, true, "qwen3-4b");
  assert.strictEqual(r.classe, "err", `HTTP ${codigo} com chave deveria ser err`);
  assert.ok(/chave inválida/.test(r.texto), `texto deve dizer "chave inválida": ${r.texto}`);
}

// outro HTTP: modelo inativo de fato
const r500 = classificarModelo(500, true, "qwen3-4b");
assert.strictEqual(r500.classe, "err");
assert.ok(/modelo inativo/.test(r500.texto) && /500/.test(r500.texto), r500.texto);

// rede falhou: null — cobre "carregando" e "desligado" sem inventar qual
const rFora = classificarModelo(null, false, "qwen3-4b");
assert.strictEqual(rFora.classe, "err");
assert.ok(/fora do ar/.test(rFora.texto), rFora.texto);

console.log("✓ badge.test.js — matriz completa do badge do modelo");
