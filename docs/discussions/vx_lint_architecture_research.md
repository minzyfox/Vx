# Research: configurable linting for Vx

Checked on 2026-10-22. This note is about Vx tooling, separate from the Vextor
indexing project. It builds on the local [style and linting discussion](vx_style_and_linting.md).
The examples marked **proposed** describe possible Vx behavior, not existing commands.

## Decision

Improve `vx-format` for source layout. Keep language errors and established
warnings in `vxc`. Add optional lint rules through a shared Vx analysis library
when there are enough useful rules to justify a dedicated `vx-lint` command.
Keep that command in this repository at first. A separate repository or plugin
interface would require a stable analysis API that Vx does not expose today.

The motivating examples need different tools:

| Example | Owner | Reason |
| --- | --- | --- |
| Poorly indented parameters in a multiline `fn` signature | `vx-format` | Layout can be changed without deciding whether the program is valid. |
| Final `return best;` where `best` would work | Optional style lint | Both forms are valid. Projects may prefer different forms. |
| `best;` in a function that must return a value | `vxc` | The type checker must reject a missing result. |
| Unused binding or unnecessary `mut` | Existing `vxc` warnings | `W1001` and `W1004` already cover these cases. |
| A host read of device memory | `vxc` | Placement is a language correctness rule. |

## How Rust separates the pieces

`rustc` has lint levels such as `allow`, `warn`, `deny`, and `forbid`.
It accepts command-line flags and source attributes, including scoped attributes.
`deny` turns a lint finding into an error, while `forbid` prevents a narrower
scope from lowering that lint level. Rust also has `expect`, which warns when
an expected finding no longer occurs. See the [rustc lint-level reference](https://doc.rust-lang.org/stable/rustc/lints/levels.html).

Cargo can set lint levels in the manifest's `[lints.rust]` and
`[lints.clippy]` tables. [Cargo documents the manifest format](https://doc.rust-lang.org/cargo/reference/manifest.html#the-lints-section).
Clippy groups its extra lints by purpose. Its default groups include
correctness, suspicious code, style, complexity, and performance. Pedantic,
restriction, and nursery groups are opt-in. See [Clippy's category list](https://doc.rust-lang.org/clippy/).

Clippy separates a rule's **level** from its **parameters**. Levels can be
chosen with attributes, command-line flags, or Cargo manifest entries.
Some thresholds and allowlists live in `clippy.toml` or `.clippy.toml`.
Clippy's documentation calls that configuration file unstable, so its
particular format is not a compatibility promise for Vx. See
[Clippy configuration](https://doc.rust-lang.org/stable/clippy/configuration.html).

Clippy has early passes over syntax and later passes that can use resolved
types. That distinction matters for Vx: a simple source-style check needs
tokens or syntax, while a rule about a call or placement may need type
information. See [Clippy's lint-pass guide](https://doc.rust-lang.org/stable/clippy/development/lint_passes.html).

External Rust rules are possible, but they have a cost. [Dylint](https://github.com/trailofbits/dylint)
loads user-selected lint libraries and can name them in workspace metadata.
Its documentation says the compiler and Clippy helper APIs used by those
libraries have no stability guarantee. This is a useful extension model,
but it is a poor first dependency for a young Vx compiler.

## What the FindBugs model adds

FindBugs' maintained successor, SpotBugs, separates **filters** from
**detectors**. An XML filter can include or exclude existing findings by
pattern, category, class, method, or source file. A filter does not define a
new detector. See the [SpotBugs filter format](https://spotbugs.readthedocs.io/en/latest/filter.html).
SpotBugs also supports detector plugins. Its [Eclipse integration guide](https://spotbugs.readthedocs.io/en/latest/eclipse.html#extending-the-eclipse-plugin-since-2-0-0)
describes loading custom detector packages.

The distinction applies to Vx. A project file can select rules, set levels,
and specify thresholds without being able to invent arbitrary analyses.
Custom rules need code, a documented analysis interface, versioning, and a
way to run trusted plugins. SpotBugs scans Java bytecode, so it also cannot
serve as a direct model for a Vx source-layout rule.

| External mechanism | What a Vx project could change | Cost and limit |
| --- | --- | --- |
| Project configuration | Select built-in rules, levels, and thresholds | Small first step; cannot express a new detector. |
| Declarative source patterns | Match selected syntax shapes | Needs a pattern language; type-sensitive rules remain difficult. |
| External lint executable | Read a versioned syntax or analysis report | Works across process boundaries; the report needs a stable schema. |
| Native lint plugin | Inspect compiler analysis directly | Fast and expressive; creates ABI, version, and trust problems. |

Start with project configuration. If users later need private rules, prefer a
versioned data interface to loading native code into the compiler process.
That preference is a design inference from Vx's current lack of a stable
analysis interface, not a feature Vx already supports.

## What Vx already provides

The compiler defines coded warnings in [`src/diagnostic.rs`](../../src/diagnostic.rs).
Diagnostics can carry source locations, notes, and suggested edits. The
[`vxc` driver](../../src/driver.rs) has `parse-only`, which does not type-check,
and `print-ast`, which does type-check but prints the AST. Neither is a quiet
lint-only action.

[`vx-format`](../../src/bin/vx-format.rs) accepts file paths and an `--indent`
value. Its implementation in [`src/formatter.rs`](../../src/formatter.rs)
works from tokens and tracks braces and square brackets for indentation.
It does not give multiline function parameters their own indentation level.
It has source comments to disable formatting for a file or a region, but no
project rule file.

The parser changes a final expression without a semicolon into a `Return`
statement in [`src/parser/decl.rs`](../../src/parser/decl.rs). After that
change, the checked function body alone cannot reliably tell whether a final
result was written as `best` or `return best;`. A style lint for this example
must inspect original tokens or preserve the source form through parsing.

[`vx-analyzer`](../../src/ide/mod.rs) already type-checks editor buffers, but
its `IdeDiagnostic` keeps only a message and location. The LSP bridge in
[`vx-analyzer/src/main.rs`](../../vx-analyzer/src/main.rs) currently sends
those findings as errors without a lint code. A shared lint engine should
preserve severity, code, source location, notes, and safe edits for the editor.

## Proposed Vx configuration

Start with a checked-in `vx-lint.toml` at the package or project root.
Use a fixed registry of named rules. The file would select a level and any
typed parameters for each rule. Unknown names and invalid values should be
reported, since silent typos make a project rule ineffective.

```toml
# Proposed format. Vx does not read this file today.
[lints]
"style.needless_final_return" = "warn"
"style.long_function" = "allow"

[parameters]
max_function_lines = 80
```

The first version should support `allow`, `warn`, and `deny`. `deny` changes
the command's exit status after analysis. The default for a subjective style
rule should be `allow` until users have tested its false-positive rate.
The command line can override project defaults for CI. A future scoped source
attribute could suppress one finding with a reason, after Vx defines the
attribute syntax and its precedence. Do not add plugin loading or arbitrary
rule expressions to the first configuration format.

This design separates three concerns:

1. The **rule** contains detection logic and a stable name.
1. The **level** chooses whether a finding is hidden, reported, or an error.
1. A **parameter** changes a rule's threshold or allowlist without changing
   its implementation.

The implementation should record the compiler version and lint rule set in
CI output. A rule's meaning may change as Vx evolves. Keep configuration
scoped to the current package or explicit input files, so a dependency's
warnings do not unexpectedly become an application's errors.

## Should Vx have a separate Clippy-like command?

Yes, **eventually**, if Vx gains a useful set of optional style and quality
rules. The first implementation should be a small `vx-lint` binary in the Vx
repository that reuses the compiler's parser, name resolution, type checker,
and diagnostic structures. A check-only analysis entry point would keep lint
work independent from code generation and make the same findings available to
the editor. A new repository adds version coordination before that interface
is stable.

One final `return` preference alone does not justify the command. A sensible
first rule set would include several documented checks with low false-positive
rates, each with source examples and tests. Existing `W` warnings should stay
in `vxc` rather than being copied into `vx-lint`.

The signature example is a formatter defect, not evidence that Vx needs
Clippy. The next concrete step for it is to define how multiline parameter
lists wrap and indent, then add formatter tests for nested types, comments,
attributes, and repeated formatting. A style rule can later address final
returns if projects actually request that convention.

## Implementation order

1. Document a canonical Vx layout for multiline parameters and fix
   `vx-format` to produce it.
1. Add a check-only compiler API that returns structured diagnostics without
   printing an AST or generating code.
1. Define rule names, defaults, levels, configuration validation, and tests.
1. Add syntax and type-aware lint passes through the shared analysis API.
1. Add the `vx-lint` command and pass its diagnostics through `vx-analyzer`.
1. Consider external detector packages only after the analysis API has a
   versioned contract and there is demand beyond repository-owned rules.

## Open design questions

- Should existing `W` codes accept configurable levels, or should only new
  optional rules do so at first?
- What source representation should retain the distinction between an
  explicit final `return` and a tail expression?
- How should lint levels interact with imports and packages compiled together?
- When does a project need a baseline or path filter like SpotBugs, rather
  than a narrow source suppression with an explanation?
