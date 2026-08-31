//! vbl-lang — front-end da VerboLang (Etapa 2, PLAN §2.1).
//!
//! Lexer, parser e AST conforme `docs/FORMAL.md` §2 (unidades léxicas) e §3
//! (gramática EBNF + notas semânticas). O parser devolve a AST **e** todos os
//! diagnósticos encontrados (com linha/coluna) — programas inválidos produzem
//! diagnósticos com códigos canônicos que reancoram a matriz de cláusulas de
//! erro da Etapa 1 (`tests/unit/test_clausulas_erro.py`).
//!
//! Camadas:
//! - [`token`]/[`lexer`]: unidades léxicas da FORMAL §2;
//! - [`ast`]: estrutura do programa (FORMAL §3);
//! - [`parser`]: descida recursiva sobre a EBNF + cláusulas de erro;
//! - [`canon`]: serialização `.vl` canônica reparseável (FORMAL §4.1);
//! - [`diag`]: diagnóstico com código canônico, linha e coluna.

pub mod ast;
pub mod canon;
pub mod diag;
pub mod lexer;
pub mod parser;
pub mod token;

pub use ast::{
    Action, Conjugation, CmpOp, Declaration, Duration, ExprKind, Expression, FormAttrs, FormDecl,
    MainBlock, PhysicalUnit, Program, ReviewDecl, Rule, SensorRef, Statement, Threshold, TimeUnit,
};
pub use diag::{Diagnostic, Diagnostics, Span};
pub use parser::parse;
