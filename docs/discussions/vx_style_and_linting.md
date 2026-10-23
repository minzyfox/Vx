# Vx style and linting discussion

Vx has a grammar, syntax documentation, a formatter, and compiler warnings. It does not yet have a dedicated guide to idiomatic Vx or a general Clippy-like lint command. This note records how those pieces differ and what a lint tool would need from `vxc`.

## Return style

Vx supports Rust-style tail expressions. The final expression in a function body is returned when it has no trailing semicolon:

```vx
fn double(x: i32) -> i32 {
  x * 2
}
```

An explicit `return x * 2;` also works and is useful for early exits. The parser converts a final expression into a return statement internally. See [syntax](../lang/syntax.md) and [the parser](../../src/parser/decl.rs).

## How idioms become conventions

The grammar and type checker determine whether a program is valid. A formatter makes valid code look consistent. A style guide explains which valid patterns are preferred and why. A linter detects patterns that can be recognized reliably and reports advice or suggested changes. Standard-library code and examples also teach idioms by showing them in use.

These layers often develop over time: programmers establish patterns, code reviews reveal recurring advice, a guide records the reasons, and clear mechanical cases become formatter or lint rules. Projects may choose different policies even when they use the same language.

For Vx, an idioms guide could start with compiling examples for tail expressions, `comptime`, tensor ownership, placement, transfers, and `spawn on`. Each lint rule should follow a well-defined preference and have few false positives.

## What Vx has today

The [grammar](../lang/grammar.md) and [syntax guide](../lang/syntax.md) describe the language. [`vx-format`](../../src/bin/vx-format.rs) applies formatting rules to `.vx` files. `vxc` already emits [coded diagnostics](../../src/diagnostic.rs), including warnings with spans, notes, and suggested fixes. Some warning codes describe redundant or questionable code. There is no separate, general `vx-lint` binary in this repository at present.

The `vx-lint: allow-atomic` comments in the compiler source refer to a specific CI check on Rust code. The `cargo clippy` step also checks Rust code, not Vx programs.

## Division of responsibilities

| Tool | Responsibility |
| --- | --- |
| `vxc` | Decide whether Vx code is valid, compile it, and report its existing warnings, including unused code warnings. |
| `vx-format` | Rewrite spacing and layout into the standard format. |
| `vx-analyzer` | Connect compiler analysis to the editor, including diagnostics, hover information, and navigation. |
| Proposed `vx-lint` | Add optional advice about style, clarity, and best practices in valid Vx code. |

Existing compiler warnings would stay in `vxc`; `vx-lint` would not duplicate them. The proposed lint rules would have one implementation. `vx-lint` would run them from the command line or CI, while `vx-analyzer` could show their findings in the editor.

## Should a lint tool have its own repository?

It could eventually. Rust Clippy has a separate repository, but its rules still rely on the Rust compiler's syntax and type information. A separate Vx repository would similarly need a compiler interface and a way to keep compatible versions together.

The suggested starting point is a `vx-lint` command in this repository, sharing `vxc`'s parser and type checker. Keeping policy advice separate from language correctness does not require a separate repository. A split would make more sense once the lint collection and contributor community need an independent release cycle and the compiler exposes a stable interface.

## Compiler work for a Clippy-like tool

`vxc` already parses Vx, resolves names, checks types, and constructs diagnostics. A lint tool would need these additions:

1. **A check-only result.** Expose parsing, imports, macro expansion, name resolution, and type checking through an API that returns the checked program and diagnostics without generating code. The current [pipeline](../../src/pipeline.rs) keeps much of that result internal.
1. **Lint passes.** Let syntax rules inspect the AST and semantic rules inspect resolved types, ownership, and placement after the relevant analysis is complete.
1. **Rule controls.** Give rules stable names, default levels, and ways to enable or suppress them without changing whether a program is valid.
1. **Reporting.** Reuse compiler diagnostics for locations, explanations, and suggested edits, with output suitable for the command line and editors. The current `vx-analyzer` diagnostic type keeps only a message and location; it would need to carry severity, rule code, notes, and fixes through to the editor.
1. **Source fidelity for precise edits.** Preserve enough original syntax and locations to offer safe fixes. For example, the parser turns a final expression into a `Return` node, so a lint about explicit versus implicit returns cannot distinguish them from the checked AST alone.

A first version could expose a check-only API, add a small set of lint passes, and provide a `vx-lint` binary. `vx-analyzer` could then consume the same diagnostics for editor feedback.
