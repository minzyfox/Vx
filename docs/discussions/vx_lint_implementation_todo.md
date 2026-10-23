# TODO: implement Vx formatting and optional linting

Status: tasks 01 through 03 complete. Tasks 04 through 25 remain incomplete.

Prepared on 2026-10-23 from the local
[lint architecture research](vx_lint_architecture_research.md) and
[style discussion](vx_style_and_linting.md).
The file paths and existing behavior below came from this checkout.
Recheck them before implementation because the compiler changes frequently.

This document gives an implementing LLM small tasks, concrete contracts, and completion checks.
Names marked **proposed** describe additions, not existing APIs or commands.

## Intended result

Keep these responsibilities separate:

| Tool | Responsibility |
| --- | --- |
| `vx-format` | Format source layout, including multiline function parameters. |
| `vxc` | Enforce language rules and keep its existing coded warnings. |
| Shared analysis library | Return checked source and structured diagnostics without generating code. |
| Proposed `vx-lint` | Run optional project rules through the shared library. |
| `vx-analyzer` | Show the same findings with accurate codes, severity, and locations. |

Implement the formatter first.
Its release does not depend on the lint command.
Build the shared analysis API before adding rule implementations.
Add the command after several useful rules pass the release checks.

The existing Rust library, `vxc`, is the initial shared library.
Do not create a separate crate or repository for this work.
Check-only execution must avoid backend work.
The existing crate can still require LLVM and MLIR when it builds.

## Instructions for the implementing LLM

1. Read [repository guidance](../../agents/AGENTS.md) before changing code.
1. Use the Vx preflight skill for compiler changes and PR preparation.
1. Read [parallel compiler rules](../language_rules_for_a_parallel_compiler.md) before changing analysis boundaries.
1. Select the first incomplete task whose prerequisites are complete.
1. Read that task's files before editing them.
1. Implement one task at a time.
1. Run its focused checks before starting another task.
1. Record the commands, selected test counts, and results below the task.
1. Mark a task complete only when its completion condition holds.
1. Preserve unrelated edits and untracked files.

Use simple names and short explanations in source comments and tests.
Do not implement later phases while resolving an earlier task.
Do not claim that a proposed command already works.
Do not create an issue number or attach an unrelated issue.
Follow the repository's issue and commit rules when preparing commits.

For each task, record a handoff using this format:

```text
Task:
Status: incomplete / in progress / complete / blocked
Files changed:
Behavior implemented:
Commands run and selected test counts:
Mutation check and failing assertion:
Checks not run and reason:
Remaining problem:
Next task:
```

A mutation check temporarily breaks the behavior that an assertion checks.
The intended assertion must fail.
A compilation failure does not establish that the assertion works.
Restore the implementation before recording a passing result.
Break separate important assertions separately, as repository guidance requires.

If a required API differs from this document, record the difference first.
Update the affected task before continuing.
If resolving it changes language semantics, stop that task for review.
Continue independent tasks where possible.

Suggested prompt for an implementation session:

```text
Read agents/AGENTS.md and docs/discussions/vx_lint_implementation_todo.md.
Use the Vx preflight skill for compiler changes.
Implement the first incomplete task whose prerequisites are complete.
Keep the change limited to that task.
Run its focused checks and required mutation checks.
Record the handoff below the task.
Report the files changed, test results, and remaining problems.
Do not mark later tasks complete.
```

## Existing code map

| Existing file | Read this part | Why it matters |
| --- | --- | --- |
| [src/formatter.rs](../../src/formatter.rs) | `format_file`, `adjust_spacing`, `emit_formatted_string`, unit tests | Formatting uses tokens and tracks braces and square brackets. |
| [src/bin/vx-format.rs](../../src/bin/vx-format.rs) | `Cli`, `process_file` | Existing options are input files and `--indent`. |
| [src/lexer.rs](../../src/lexer.rs) | `TokenBase`, `Lexer::advance`, `new_with_comments` | Tokens preserve source text, positions, and macro expansion information. |
| [src/parser/decl.rs](../../src/parser/decl.rs) | `parse_comma_separated_params`, `parse_function` | Parsing converts a tail expression into a return statement. |
| [src/syntax/decl.rs](../../src/syntax/decl.rs) | `Function`, `clone_signature` | A function currently has no complete declaration span. |
| [src/syntax/types.rs](../../src/syntax/types.rs) | `Span` | Syntax spans include an expansion identifier. |
| [src/module_loader.rs](../../src/module_loader.rs) | `load_main`, `load_all`, `parse_file_renaming` | Loading follows imports and can select compiled `.vxlib` interfaces. |
| [src/driver.rs](../../src/driver.rs) | `Action`, `execute_vx_pipeline`, `execute_parallel`, `run_semantic_analysis` | The normal driver owns much of the analysis sequence and reporting. |
| [src/pipeline.rs](../../src/pipeline.rs) | `Frontend`, `run_frontend_on`, `Reported`, `report_errors`, `type_check_phase` | Parallel analysis retains warnings, but some error paths still print directly. |
| [src/diagnostic.rs](../../src/diagnostic.rs) | `Diagnostic`, `DiagnosticCode`, `SourceSpan`, `FixIt` | Existing diagnostics carry codes, notes, and edits. |
| [src/diagnostics_json.rs](../../src/diagnostics_json.rs) | Schema documentation and `render` | Existing compiler JSON has a versioned admission-report contract. |
| [src/ide/mod.rs](../../src/ide/mod.rs) | `IdeDiagnostic`, `Analysis::diagnostics`, `scratch_file` | Editor analysis duplicates compiler work and stages buffers in temporary files. |
| [vx-analyzer/src/main.rs](../../vx-analyzer/src/main.rs) | `send_diagnostics`, document notifications | The LSP bridge currently sends every diagnostic as an error without a code. |
| [src/lib.rs](../../src/lib.rs) | Public modules | Export the proposed shared analysis and lint modules here. |
| [Cargo.toml](../../Cargo.toml) | Dependencies, binaries, workspace | The main crate does not currently depend on a TOML parser or Serde. |
| [tests/integration_test.rs](../../tests/integration_test.rs) | Module declarations | Register new integration test modules here. |
| [tests/integration_test/compile_test.rs](../../tests/integration_test/compile_test.rs) | Frontend, warning, and `RUN` test runners | Different fixture directories use different runners. |
| [.github/workflows/ci.yml](../../.github/workflows/ci.yml) | Parallel pipeline gates | Named gates must select and run their intended tests. |

`tests/integration_test/lint_test.rs` checks Rust test source conventions.
It does not implement lint rules for Vx programs.
The `vx-lint: allow-atomic` comments also concern a separate Rust check.
Do not extend either mechanism into the proposed Vx linter.

## Version one decisions

These defaults make the plan executable without another design exercise.
They extend the research where it left details open.
They remain proposed product behavior until implementation and review.

| Question | Default for this plan |
| --- | --- |
| Configure existing compiler `W` codes? | No. Preserve their current behavior. |
| New lint levels? | `allow`, `warn`, and `deny`. |
| Default for subjective style rules? | `allow`. |
| Rule names? | Explicit stable names such as `style.needless_final_return`. |
| Groups, aliases, wildcards, or `all`? | Defer them. Require exact registered names. |
| Source suppressions? | Defer them until Vx defines syntax and precedence. |
| Configuration file? | `vx-lint.toml`. |
| Configuration precedence? | Built-in defaults, one project file, then command-line overrides. |
| Rule parameters? | Typed values with strict validation. |
| Files eligible for optional findings? | Explicit input files only. Load imports for analysis. |
| Compiler errors in imports? | Preserve compiler policy, including relevant imported errors. |
| Errors during analysis? | Report compiler findings. Skip optional rules for that analysis unit. |
| Automatic source rewriting? | Defer `--fix`. Retain safe edit data for later clients. |
| Plugins or user-defined detector expressions? | Defer them. |
| Public semantic rule? | Select one later from a concrete request. Do not invent one. |

An analysis unit means one explicit input and the modules needed to check it.
An error in one unit must not prevent checking independent explicit inputs.
Deduplicate reports when explicit inputs share modules.
Never hide a language error because a project allows a style rule.

The initial optional rule candidates are:

| Proposed rule | Default | Parameter | Source |
| --- | --- | --- | --- |
| `style.needless_final_return` | `allow` | None | The research's motivating return example. |
| `style.long_function` | `allow` | `max_function_lines = 80` | The research's configuration example. |
| `style.too_many_parameters` | `allow` | `max_function_parameters = 6` | An additional candidate for this plan. |

The third candidate is a new proposal, not a decision from the research.
All three inspect source structure.
None requires a type-aware detector.
Require useful examples and a false-positive review before releasing a candidate.
Do not add a weak rule merely to reach three rules.

## Task order and dependencies

| Phase | Tasks | Prerequisites | Deliverable |
| --- | --- | --- | --- |
| Preparation | 01 | None | A recorded starting point and working environment. |
| Formatter | 02–04 | 01 | Tested multiline parameter indentation. |
| Shared analysis | 05–10 | 01 | Quiet analysis with source identity and structured diagnostics. |
| Rule controls | 11–13 | 05–10 | Registry, levels, checked configuration, and discovery. |
| Rules | 14–19 | 11–13 | Source-aware passes and documented rule candidates. |
| Command | 20–21 | 14–19 and rule release checks | `vx-lint` with stable text and JSON reporting. |
| Editor | 22–24 | 20–21 | Shared findings with accurate LSP diagnostics and safe edit transport. |
| Release | 25 | Earlier tasks in the release scope | User documentation, CI checks, and a truthful handoff. |

The formatter and shared analysis phases can ship separately.
Task 19 provides typed analysis access, not a fabricated semantic rule.
Native plugins remain outside this plan.

## Phase: preparation

### Task 01: record the starting point

Prerequisites: none.

- [x] Read the guidance and preflight links above.
- [x] Read the research and style discussion completely.
- [x] Record `git status --short` before making changes.
- [x] Check that this project contains a usable `venv` directory.
- [x] Stop environment-dependent work if the environment is absent or corrupt. The environment was usable.
- [x] Read `config.local` before selecting tool paths.
- [x] Source `config.local` before every Cargo or Rustup invocation.
- [x] Choose a worker limit that leaves two CPU cores idle.
- [x] Limit Cargo jobs, Rust test threads, and Rayon workers where applicable.
- [x] Run the existing formatter unit tests as the first focused baseline.
- [x] Record existing failures separately from new failures.

Done when: the handoff records the environment, unrelated changes, and focused baseline results.

Task: 01, record the starting point.
Status: complete.
Files changed: this task record and `build/vx-lint-task-01/formatter-baseline.log`.
Behavior implemented: none. This task records the starting point.
Commands run and selected test counts:

- Before changes, `git status --short --branch` showed no tracked edits on
  `enum-layout-records-step-one`. It showed existing untracked `.agents/`,
  `AGENTS.md`, `build/`, lint discussion files, other discussion and plan files,
  and `scripts/tools/check_test_macros.py`.
- Created `vx-lint` from local `main` at `472efb639e46a48b2f41489f927a9b38a7b98f5a`.
  The existing untracked files remained in place.
- `venv/bin/python` ran Python 3.14.7 and imported pip 26.2.1. The machine
  reported 14 logical CPUs, so the worker limit was 12.
- Read `config.local`. Its LLVM, Cargo, Rustup, and library paths were used
  when the formatter test command sourced it.
- `source config.local && CARGO_BUILD_JOBS=12 RAYON_NUM_THREADS=12 cargo test --lib formatter::tests -- --test-threads=12`
  selected 10 Vx formatter tests. All 10 passed. The workspace also ran
  `vx_std_core` with zero selected tests. Its 12 tests were filtered out.
  The Vx test binary filtered out 598 other tests. No existing or new test failed.
  Mutation check and failing assertion: not applicable. This task changed no behavior or assertions.
  Checks not run and reason: broader checks belong to later implementation tasks.
  Remaining problem: none for task 01. Cargo warned that the optional ANE model
  builder lacked a Python environment with working `coremltools`. The tests passed.
  Next task: 02, document multiline function parameter layout.

Full-suite follow-up on `vx-lint`:

- `source config.local && CARGO_BUILD_JOBS=12 RAYON_NUM_THREADS=12 cargo test --workspace -- --test-threads=12`
  first stopped at two socket tests because the sandbox denied local sockets.
- The same command with local socket permission passed 989 tests and failed three
  integration tests. One library test was ignored. The log is
  `build/vx-lint-task-01/full-suite-debug-with-sockets.log`.
- The CoreML dispatcher test failed because the virtual environment has no
  working `coremltools` or compiled model. Two backend corpus checks failed.
  The ANE LLVM checks expected a string without `abi=1`. The `box_oob.vx`
  check expected a segmentation fault, but the program printed a number.
- `source config.local && CARGO_BUILD_JOBS=12 RAYON_NUM_THREADS=12 cargo test -p vxc --test stress_test -- --test-threads=12`
  selected three ignored tests and ran none.
- `source config.local && CARGO_BUILD_JOBS=12 RAYON_NUM_THREADS=12 cargo test --workspace --doc -- --test-threads=12`
  ran zero documentation tests and passed.
- No compiler or test source changed during this follow-up. No commit was made during the test runs.

## Phase: formatter

### Task 02: document multiline function parameter layout

Prerequisites: Task 01.

Read: `src/formatter.rs`, `src/parser/decl.rs`, and `docs/lang/syntax.md`.

Use this minimal layout policy:

- [x] Preserve whether a parameter list occupies one line or several lines.
- [x] Indent parameter continuation lines one level beyond the function declaration.
- [x] Align a closing parenthesis on its own line with the function declaration.
- [x] Keep the function body at its normal block indentation.
- [x] Apply the same policy to supported method and foreign function declarations.
- [x] Respect `--indent`, including values other than two.
- [x] Preserve parameter separators and source tokens.
- [x] Preserve comments and formatter-disabled text according to existing guarantees.
- [x] Document the policy in `docs/lang/syntax.md`.

Proposed output for an existing multiline list:

```vx
fn choose(
  first: i32,
  second: i32
) -> i32 {
  first + second
}
```

Do not insert a trailing parameter comma.
The current parameter parser expects another parameter after a comma.
Do not change that grammar as part of formatting.
Do not add automatic line-width wrapping in this phase.
Do not invent attribute syntax to complete a formatter test.
Test attributes only if this checkout supports their syntax.

Done when: the layout policy and examples cover these cases without requiring a language change.

Task: 02, document multiline function parameter layout.
Status: complete.
Files changed: `docs/lang/syntax.md` and this task record.
Behavior implemented: none. The syntax guide states the target layout for future formatter work.
Commands run and selected test counts:

- Extracted two Vx examples from the new section into
  `build/vx-lint-task-01/layout-examples.vx`.
- `source config.local && target/debug/vxc --parse-only build/vx-lint-task-01/layout-examples.vx`
  parsed both examples successfully.
- `source config.local && target/debug/vxc --print-ast build/vx-lint-task-01/layout-examples.vx`
  checked both examples successfully.
- `mdformat --check docs/lang/syntax.md` and `git diff --check` passed.

Mutation check and failing assertion: not applicable. This task changed no behavior or assertions.
Checks not run and reason: formatter tests belong to Task 03. This task changed no formatter code.
Remaining problem: `vx-format` does not yet apply the documented multiline layout.
Next task: 03, add parameter indentation state.

### Task 03: add parameter indentation state

Prerequisites: Task 02.

Change: `src/formatter.rs` and its unit tests.

- [x] Add one failing test for a multiline function signature.
- [x] Identify function parameter parentheses from tokens and their declaration context.
- [x] Track matching parentheses without counting parentheses inside strings or comments.
- [x] Save the declaration's base indentation for each relevant parameter list.
- [x] Indent continuation lines according to Task 02.
- [x] Restore indentation when the matching parameter parenthesis closes.
- [x] Handle nested parentheses and square brackets inside parameter types.
- [x] Keep function calls and other parentheses under their existing formatting policy.
- [x] Preserve the current unchanged-output behavior for unknown lexer tokens.
- [x] Preserve formatter-off behavior.
- [x] Check whether earlier token normalization passes affect disabled text.
- [x] Fix only regressions caused by this change. None were found.

Do not treat every `(` as a parameter indentation level.
Do not replace the token formatter with a new parser-based formatter.
Do not format the entire repository to demonstrate this change.

Done when: the new test passes and existing formatter tests still pass.

Task: 03, add parameter indentation state.
Status: complete.
Files changed: `src/formatter.rs` and this task record.
Behavior implemented: multiline declaration parameters gain one indentation level.
The matching closing parenthesis restores the declaration's indentation.
Commands run and selected test counts:

- The new test first failed its exact indentation assertion. One test was selected.
- `source config.local && CARGO_BUILD_JOBS=12 RAYON_NUM_THREADS=12 cargo test -p vxc --lib formatter::tests -- --test-threads=12`
  passed all 11 selected formatter tests after the change.
- The same command passed all 11 formatter tests after the mutation was restored.
- `source config.local && CARGO_BUILD_JOBS=12 RAYON_NUM_THREADS=12 cargo test -p vxc --lib -- --test-threads=12`
  passed 608 tests. One test was ignored.
- `source config.local && cargo fmt --all -- --check` passed.
- `source config.local && CARGO_BUILD_JOBS=12 RAYON_NUM_THREADS=12 cargo clippy -p vxc --lib -- -D warnings`
  passed.
- Built `vx-format`. Temporary inputs checked a nested type, an `impl` method,
  an `extern` declaration, a multiline call, a disabled signature, delimiters in
  comments and strings, and `--indent 4`.
  The formatted temporary input also passed `vxc --parse-only`.

Mutation check and failing assertion: changing the new indentation step from one
level to zero made `multiline_function_parameters_use_one_extra_indent_level` fail.
The assertion showed the parameter lines at column zero. The code was restored.
Checks not run and reason: the full workspace suite has three recorded baseline
failures. Task 04 adds the complete formatter regression matrix.
Remaining problem: repeated formatting adds spaces before a formatter-off end
marker. The existing whitespace and comment handling causes this behavior.
Earlier normalization also runs before disabled-region handling. Task 04 must
check those existing behaviors while testing repeated formatting.
Next task: 04, complete formatter regression coverage.

### Task 04: complete formatter regression coverage

Prerequisites: Task 03.

Add focused cases to the unit tests in `src/formatter.rs`:

| Input | Required check |
| --- | --- |
| Top-level multiline function | Exact indentation of parameters, closing parenthesis, and body. |
| Supported method inside an `impl` | Parameter indentation starts from the method's block depth. |
| Supported foreign function declaration | Parameter indentation ends without requiring a body. |
| A first parameter beside the opening parenthesis | Later lines use the documented continuation indentation. |
| Nested tensor, generic, and bracket syntax | Inner delimiters do not close the outer parameter list. |
| Comments between parameters | Comments stay attached to valid lines. |
| Strings and comments containing delimiters | Their contents do not change indentation state. |
| Existing single-line signature | The parameter list stays on one line. |
| A multiline call expression | The change does not apply the declaration policy accidentally. |
| Formatter-disabled region containing a signature | The change preserves the existing disabled-region contract. |
| Two-space and four-space settings | The selected indentation width controls continuation lines. |
| Each new valid example formatted twice | The second result exactly equals the first result. |

- [ ] Check token preservation for representative valid examples.
- [ ] Exclude whitespace and end-of-file tokens when comparing token sequences.
- [ ] Compare token content rather than source positions.
- [ ] Parse representative formatted examples to check that formatting preserves valid syntax.
- [ ] Mutation-check indentation, delimiter matching, and repeated-formatting assertions separately.

Done when: every applicable matrix row has an assertion and the formatter diff stays focused.

## Phase: shared analysis

### Task 05: retain source identity and original text

Prerequisites: Task 01.

Read: `src/module_loader.rs`, `src/lexer.rs`, `src/diagnostic.rs`, and `src/ide/mod.rs`.

Proposed home: `src/analysis/source.rs`, exported through `src/analysis.rs`.

- [ ] Add a per-analysis source store.
- [ ] Give each source file an identifier independent of its module name.
- [ ] Keep its real path, original text, and line-start byte offsets.
- [ ] Record each loaded module's source identifier.
- [ ] Support an in-memory source override for a real file path.
- [ ] Consult overrides before reading that path from disk.
- [ ] Keep import resolution relative to the real project paths.
- [ ] Support overrides for imported files as well as the entry file.
- [ ] Keep source ownership local to one analysis request.
- [ ] Add explicit conversions between byte offsets and source line/column positions.
- [ ] Add a separate conversion for LSP UTF-16 positions.
- [ ] Check UTF-8 boundaries before constructing source slices.

Lexer columns currently advance by Unicode characters.
Some token lengths use bytes.
Do not assume those units match.
LSP positions use zero-based lines and UTF-16 code units.
Compiler source positions currently use one-based lines and columns.

Tests must cover ASCII, non-ASCII text, an emoji, CRLF, and the end of a file.
Check two files with identical text and different paths.
Check that two simultaneous analyses cannot exchange source text.

Done when: analysis can identify original sources and resolve unsaved files without changing their project paths.

### Task 06: collect diagnostics without printing

Prerequisites: Task 05.

Read: `src/diagnostic.rs`, `src/driver.rs`, `src/pipeline.rs`, and parser error types.

Proposed addition: an analysis diagnostic record containing source identity and the existing compiler diagnostic.

- [ ] Retain compiler severity, code, message, span, notes, fixes, and facts.
- [ ] Add file identity to primary locations, related notes, and edits where needed.
- [ ] Preserve diagnostics without source locations.
- [ ] Return parse and load errors through structured results.
- [ ] Preserve typed parser location data where available.
- [ ] Do not recover source positions by parsing formatted error messages.
- [ ] Separate diagnostic collection from terminal rendering.
- [ ] Remove direct printing only from paths used by the quiet API.
- [ ] Keep existing CLI rendering through explicit adapters.
- [ ] Preserve existing error limits and warning deduplication behavior.
- [ ] Keep the existing compiler JSON schema compatible.

Do not change `DiagnosticCode` into an unrestricted string throughout the compiler.
Optional rule names can travel beside existing compiler diagnostic codes.
Do not discard `DiagnosticFacts` because style rules do not need them.

Done when: failures return usable diagnostics and the quiet collection path writes nothing to stdout or stderr.

### Task 07: expose normal compiler analysis

Prerequisites: Task 06.

Read: `load_and_expand`, `prepare_semantic_analysis`, and `run_semantic_analysis` in `src/driver.rs`.

Proposed public entry: `analysis::analyze(request) -> AnalysisResult` in the existing `vxc` library.
Use equivalent names if repository conventions require them.
Keep the interface internal to this repository for now.

The request needs these inputs:

| Input | Purpose |
| --- | --- |
| Explicit entry paths | Define analysis units and optional-rule scope. |
| Source overrides | Analyze editor buffers without saving them. |
| Existing package mappings | Resolve imports with compiler semantics. |
| Relevant machine, host, and interface options | Check the same program that `vxc` would check. |
| Analysis schedule and worker limit | Support controlled sequential and parallel execution. |

The result needs original sources, structured diagnostics, and an optional checked-program handle.
Represent analysis success explicitly.
Do not return a usable checked-program handle after a compiler error.

- [ ] Extract reusable analysis from driver methods instead of copying their implementation.
- [ ] Preserve module loading, macro expansion, and name resolution order.
- [ ] Preserve declaration checks, return provenance, and compile-time body preparation.
- [ ] Preserve checks for functions, methods, and transfer implementations.
- [ ] Preserve generic instantiation and cross-call capacity checks.
- [ ] Preserve existing treatment of used and unused imported functions with errors.
- [ ] Return read-only access to checked modules and relevant type information.
- [ ] Keep ownership valid without returning references into temporary local environments.
- [ ] Make the existing normal compiler path consume the extracted implementation.
- [ ] End check-only work before HIR code generation, MLIR construction, linking, or execution.

Some frontend checks evaluate `comptime` expressions.
Preserve that language behavior.
Check-only means no backend compilation or program execution.

Done when: the normal compiler and quiet API share analysis code and agree on existing relevant fixtures.

### Task 08: expose parallel analysis without backend work

Prerequisites: Task 07.

Read: `Frontend`, `run_frontend_on`, `Reported`, and phase error handling in `src/pipeline.rs`.

- [ ] Expose the checked frontend result before MLIR generation.
- [ ] Return declaration and body errors instead of printing them inside analysis.
- [ ] Retain warnings when checking fails, according to existing reporting policy.
- [ ] Keep global declaration tables immutable during function checking.
- [ ] Keep diagnostic storage local to each analysis request.
- [ ] Preserve the normal compiler's imported-error policy.
- [ ] Preserve current fallback or rejection behavior for unsupported interface options.
- [ ] Report fallback explicitly through result metadata where callers need it.
- [ ] Never treat unsupported analysis as successful analysis.
- [ ] Sort findings deterministically after parallel work completes.
- [ ] Use file path, source range, diagnostic identifier, and message as stable ordering keys.
- [ ] Preserve existing code generation callers through adapters.

Do not generate MLIR and then discard it as a check-only implementation.
Do not introduce process-global lint configuration or source storage.
Do not use backend fallback counts as evidence that analysis stayed check-only.

Done when: sequential and parallel requests return equivalent findings without backend calls or duplicate reporting.

### Task 09: add a compiler check action

Prerequisites: Task 08.

This task adds a small proposed CLI adapter for testing and general compiler use.
The research requires the API, but does not require this particular action spelling.

Proposed command after implementation:

```sh
target/debug/vxc --action check program.vx
```

- [ ] Add `Check` to the driver's `Action` enum.
- [ ] Route it through the shared analysis API.
- [ ] Support a normal check, `-j 1`, and a permitted multiworker check.
- [ ] Return success when analysis contains warnings but no compiler errors.
- [ ] Return failure when analysis contains compiler errors.
- [ ] Render diagnostics once through the driver.
- [ ] Leave stdout empty for a successful text-mode check.
- [ ] Keep existing action aliases unchanged.
- [ ] Add action help and focused argument tests.
- [ ] Reuse existing JSON options only when their supported contract can remain accurate.
- [ ] Reject incompatible action options explicitly.

Do not add another shorthand flag in this task.
Do not interpret this action as a request to run optional style rules.

Done when: subprocess tests establish the action's output, exit status, and backend independence.

### Task 10: test the shared analysis boundary

Prerequisites: Task 09.

Proposed integration module: `tests/integration_test/analysis_test.rs`.
Register it in `tests/integration_test.rs`.
Keep smaller unit tests beside their implementation.

| Case | Required assertions |
| --- | --- |
| Valid scalar function | Success, checked result, no backend entry. |
| Existing coded warning | Original code, warning severity, source, and exactly one report. |
| Syntax error | Failure with location, no checked result, no backend entry. |
| Type error | Failure with compiler code, no checked result, no backend entry. |
| Missing import | Failure with useful file context. |
| Relevant imported function with an error | Agreement with existing compiler policy. |
| Unused imported function with an error | Agreement with existing compiler policy. |
| Method and transfer implementation | Existing checks still run. |
| Macro and generic calls | Resolution and checking still use the shared frontend. |
| Supported compiled interface | Existing interface behavior remains available. |
| Unsupported parallel option | Explicit refusal or the existing documented fallback. |
| Unsaved entry and imported buffers | Findings reflect buffer text and real paths. |
| Two concurrent requests | No source, configuration, or diagnostic leakage. |
| Normal, `-j 1`, and multiworker checks | Equivalent findings and deterministic ordering. |

- [ ] Add a focused backend-entry sentinel or counter for boundary tests.
- [ ] Keep that test support local and free of production configuration switches.
- [ ] Mutation-check the no-backend assertion by inserting a sentinel entry at the boundary.
- [ ] Mutation-check warning severity, imported-error behavior, and source identity separately.
- [ ] Capture both subprocess output streams for quiet successful checks.
- [ ] Run the existing frontend, warning, and affected import tests.
- [ ] Run the preflight coverage gate when the changed compiler surface requires it.

Done when: tests establish quiet analysis and compiler agreement, including failure paths.

## Phase: rule controls

### Task 11: define the rule registry and levels

Prerequisites: Task 10.

Proposed files: `src/lint/mod.rs`, `src/lint/registry.rs`, and `src/lint/diagnostic.rs`.

- [ ] Define `LintLevel` with `Allow`, `Warn`, and `Deny`.
- [ ] Define each rule's stable name, description, default level, and analysis phase.
- [ ] Register rules in a fixed in-process table.
- [ ] Reject duplicate rule names during registry construction.
- [ ] Keep optional rule identity separate from compiler `W` and `E` codes.
- [ ] Reuse diagnostic messages, locations, notes, and edits through the shared representation.
- [ ] Apply levels after detection through one shared policy function.
- [ ] Hide an allowed finding completely.
- [ ] Report a warned finding with warning severity.
- [ ] Report a denied finding with error severity and a failing lint result.
- [ ] Preserve the rule name when its level changes.
- [ ] Keep compiler error counts independent of optional finding counts.

Do not make rule implementations inspect CLI flags or read configuration files.
Do not emit an existing compiler warning again under a new style name.

Done when: registry and level tests establish identity, defaults, suppression, severity, and result status.

### Task 12: parse and check project configuration

Prerequisites: Task 11.

Proposed file: `src/lint/config.rs`.
Review `Cargo.toml` before choosing a TOML parser dependency.
Add only dependencies needed for typed parsing and reporting.

Proposed version one format:

```toml
[lints]
"style.needless_final_return" = "warn"
"style.long_function" = "allow"
"style.too_many_parameters" = "allow"

[parameters]
max_function_lines = 80
max_function_parameters = 6
```

- [ ] Parse TOML through a maintained parser, not regular expressions.
- [ ] Reject malformed TOML and duplicate keys.
- [ ] Reject unknown tables, rule names, and parameter names.
- [ ] Reject unsupported levels and incorrect value types.
- [ ] Require positive integer thresholds within the implementation's numeric range.
- [ ] Reject negative, zero, fractional, and overflowing thresholds.
- [ ] Check parameters even when their rule currently uses `allow`.
- [ ] Report configuration paths and available parser locations.
- [ ] Treat an absent automatic configuration as built-in defaults.
- [ ] Treat an unreadable discovered configuration as an error.
- [ ] Treat an unreadable explicit configuration as an error.
- [ ] Prevent a malformed configuration from silently disabling linting.

Done when: valid examples parse and every invalid category has a focused rejection test.

### Task 13: resolve configuration and overrides consistently

Prerequisites: Task 12.

Use this discovery policy for version one:

1. An explicit `--config FILE` applies to all explicit inputs.
1. Otherwise, search from each explicit input's parent directory toward its project root.
1. Use the nearest `vx-lint.toml` found during that search.
1. Stop after checking a directory containing a `.git` file or directory.
1. Stop at the filesystem root if no Git boundary exists.
1. Read one configuration file for each input, without merging ancestor files.

The explicit configuration path is relative to the process working directory.
Resolve an input's automatic configuration from its real path.
Changing the working directory must not change that input's automatic configuration.

- [ ] Implement discovery independently from argument parsing.
- [ ] Resolve levels in this order: built-in, project, command line.
- [ ] Define proposed repeated `--allow RULE`, `--warn RULE`, and `--deny RULE` overrides.
- [ ] Apply the last occurrence for the same rule across those options.
- [ ] Preserve cross-option order when using Clap.
- [ ] Reject unknown override names before analyzing source files.
- [ ] Load dependency modules without applying optional rules to them automatically.
- [ ] Allow dependency source to receive optional findings when explicitly named as input.
- [ ] Deduplicate equivalent input paths before processing.
- [ ] Keep separate effective configurations for inputs from different projects.
- [ ] Return the effective configuration to both CLI and editor callers.

Test nested projects, `.git` files, missing files, explicit overrides, and two projects in one invocation.
Test the override sequence `--deny RULE --allow RULE --warn RULE`.
Its final level must be `warn`.

Done when: configuration lookup, scope, and precedence have deterministic tests.

## Phase: source-aware rules

### Task 14: retain source facts before parser rewriting

Prerequisites: Task 13.

Read: `parse_function`, return statement parsing, syntax spans, and macro expansion handling.
Proposed home: a source index beside `src/analysis/source.rs`.

Use original parser observations and original tokens.
Do not infer source spelling from the checked function body.

- [ ] Record original function name, declaration range, and matching body brace ranges.
- [ ] Record original parameter counts and parameter-list ranges.
- [ ] Record explicit return keyword, expression, and semicolon ranges.
- [ ] Distinguish an explicit final return from a normalized tail expression.
- [ ] Identify a final return only when it is the last direct statement of the function body.
- [ ] Keep source records in a side table associated with the source file.
- [ ] Use source identity and declaration position as keys rather than rewritten function names.
- [ ] Capture facts before name renaming, macro expansion, and compile-time folding lose source details.
- [ ] Preserve original records when checked modules contain specialized generic functions.
- [ ] Exclude macro-generated declarations from version one source-style findings.
- [ ] Exclude foreign declarations without bodies from body-length checks.
- [ ] Keep comments available when determining edit safety.

The checked `Function` type currently lacks a complete declaration range.
Do not locate functions by searching for their name in source strings.
Two declarations can have the same simple name in different contexts.
Do not add a serialized interface field solely to support a source-style preference.

Done when: source-index tests distinguish explicit returns, tail expressions, nested returns, and generated code.

### Task 15: add syntax pass execution

Prerequisites: Task 14.

Proposed file: `src/lint/passes.rs`.

- [ ] Give syntax passes read-only access to original source, source records, and tokens.
- [ ] Run each registered pass once for each eligible original source declaration.
- [ ] Skip optional passes after compiler errors in that analysis unit.
- [ ] Skip allowed rules when the registry says they need no collection work.
- [ ] Return findings instead of printing them.
- [ ] Apply the shared level policy once.
- [ ] Sort and deduplicate results after pass execution.
- [ ] Deduplicate generated generic instances by their original source identity.
- [ ] Keep deterministic output across worker counts.

Done when: a small test-only detector establishes scheduling, file scope, level handling, and one-finding behavior.
Do not register that detector as a public rule.

### Task 16: implement `style.needless_final_return`

Prerequisites: Task 15.

Proposed file: `src/lint/rules/needless_final_return.rs`.
Default: `allow`.

The first version checks a deliberately narrow case.
Report an explicit final `return value;` when `value` is a plain identifier.
Require the return to be the last direct statement in the function body.
Require the function to pass compiler analysis.
Do not broaden expression coverage in this task.

Example that should produce one finding when enabled:

```vx
fn choose(first: i32, second: i32) -> i32 {
  let best = first + second;
  return best;
}
```

Example that should produce no finding:

```vx
fn choose(first: i32, second: i32) -> i32 {
  let best = first + second;
  best
}
```

- [ ] Detect the explicit source keyword through the source index.
- [ ] Locate the finding at the explicit `return` keyword.
- [ ] Suggest using the identifier as the function's final expression.
- [ ] Preserve early returns and returns nested inside other blocks.
- [ ] Skip bare `return;`, generated code, and unsupported expression forms.
- [ ] Skip a return with following statements, even when they are unreachable.
- [ ] Keep a comment-bearing case advisory when an exact safe edit is uncertain.
- [ ] Offer edits only for a single-line form with unambiguous token ranges.
- [ ] Keep all comments and the expression text unchanged.
- [ ] Reparse and recheck the edited example in tests.
- [ ] Check that enabling this rule leaves existing compiler warnings unchanged.

An edit can require deleting both the keyword and semicolon.
Represent it as a checked edit group if the shared model supports multiple edits.
Otherwise, omit the edit until Task 24 provides that model.
Never remove a broad source range that also removes comments.

Required negative cases: tail expression, early return, nested return, bare return, macro-generated return, and following statement.
Also check a return inside a function with a compiler error.
The compiler error must remain visible, and optional passes must not run.

Done when: positive and negative cases check exact rule count, code, severity, and source range.

### Task 17: implement `style.long_function`

Prerequisites: Task 15.

Proposed file: `src/lint/rules/long_function.rs`.
Default: `allow`.
Parameter: `max_function_lines`, initially `80`.

Use this counting policy:

- [ ] Count physical source lines strictly between the body's brace lines.
- [ ] Include blank lines and comment lines within that range.
- [ ] Count zero lines when both braces occupy the same line.
- [ ] Exclude the signature and both brace lines.
- [ ] Treat LF and CRLF as the same line structure.
- [ ] Report only when the count exceeds the threshold.
- [ ] Locate the finding at the function name.
- [ ] Include the measured count and configured threshold in the message.
- [ ] Offer no automatic edit.

This definition favors a simple, reproducible count.
Document that putting statements on brace lines excludes those lines.
Do not describe it as a complexity measurement.

Test counts below, equal to, and above the threshold.
Test comments, blank lines, multiline signatures, same-line bodies, and threshold overrides.
Test that specialized generic instances do not repeat the same source finding.

Done when: tests establish the exact counting policy and boundary behavior.

### Task 18: evaluate and implement `style.too_many_parameters`

Prerequisites: Task 15.

Proposed file: `src/lint/rules/too_many_parameters.rs`.
Default: `allow`.
Parameter: `max_function_parameters`, initially `6`.

First evaluate this additional candidate against repository examples.
Record useful cases and cases where many parameters are appropriate.
If the candidate lacks useful cases, defer it and record the release-gate consequence.
Do not invent a replacement rule without a precise rule description.

If the candidate passes that review:

- [ ] Count parameters in the original function declaration.
- [ ] Count a receiver only when it is a declared parameter in the current grammar.
- [ ] Exclude generic parameters, return types, and contract clauses.
- [ ] Exclude foreign function declarations in version one.
- [ ] Report only when the count exceeds the threshold.
- [ ] Locate the finding at the function name.
- [ ] Include the measured count and configured threshold in the message.
- [ ] Offer no automatic edit.
- [ ] Check below, equal, and above-threshold declarations.
- [ ] Check supported methods, generic declarations, nested types, and configuration overrides.

Done when: the candidate includes documented review evidence and either a tested implementation or an explicit deferred status.
Deferral does not satisfy the three-rule command release gate.

### Task 19: provide typed pass access without inventing a rule

Prerequisites: Tasks 15–18 and the checked result from Task 10.

A typed pass can inspect resolved types after compiler checking succeeds.
The research does not define a specific new typed rule.
Keep the production rule registry limited to reviewed rules.

- [ ] Expose a read-only typed context through the shared lint runner.
- [ ] Reuse checked modules, resolved types, and source identity from analysis.
- [ ] Keep optional passes unable to change the compiler's frozen declaration tables.
- [ ] Skip typed passes when compiler analysis fails.
- [ ] Skip generated nodes when their original source location is unavailable.
- [ ] Test the interface with a test-only detector that reads an actual resolved type.
- [ ] Check that the test uses checked information rather than source annotation text alone.
- [ ] Check that running the detector does not cause a second compiler analysis.
- [ ] Leave production semantic rules in the deferred list below.

Done when: a test establishes typed access, failure handling, and reuse of one analysis result.
This task does not authorize a placement, ownership, or performance warning.

## Phase: command

### Rule release checks before Task 20

- [ ] Require at least three accepted, documented optional rules for the dedicated command.
- [ ] Give each rule positive and negative examples.
- [ ] Run enabled candidates against a recorded set of repository Vx files.
- [ ] Record true positives, false positives, and intentionally skipped source forms.
- [ ] Keep subjective rules disabled by default.
- [ ] Exclude rules that merely duplicate existing compiler warnings.

Three is this plan's concrete interpretation of the research's requirement for several useful rules.
If too few candidates pass, ship the formatter and analysis API first.
Keep the command tasks incomplete.
Do not lower the rule-quality requirement to complete the checklist.

### Task 20: add the `vx-lint` CLI adapter

Prerequisites: Tasks 11–19 and the rule release checks.

Proposed file: `src/bin/vx-lint.rs`.
Use Cargo's existing binary discovery or explicit registration consistently.

Proposed command examples after implementation:

```sh
target/debug/vx-lint program.vx
target/debug/vx-lint --config vx-lint.toml program.vx
target/debug/vx-lint --deny style.needless_final_return program.vx
target/debug/vx-lint --list-rules
```

- [ ] Require explicit `.vx` input paths for analysis mode.
- [ ] Support `--config`, `--allow`, `--warn`, and `--deny` as specified above.
- [ ] Support a worker limit without using every CPU core by default.
- [ ] Reuse relevant package, machine, host, and interface options from analysis.
- [ ] Keep option validation consistent with compiler support.
- [ ] Add `--list-rules` without requiring an input file.
- [ ] List stable names, default levels, descriptions, and supported parameters.
- [ ] Add a version option containing compiler package version and lint rule-set version.
- [ ] Analyze each input through the shared API.
- [ ] Render existing compiler diagnostics once alongside optional findings.
- [ ] Continue independent inputs after a source error.
- [ ] Never generate code or change input files.

Use these exit statuses:

| Status | Meaning |
| --- | --- |
| `0` | Analysis succeeded with no denied optional finding. Warnings can exist. |
| `1` | Compiler errors or denied optional findings exist. |
| `2` | Arguments, configuration, or input access prevented the requested run. |

For mixed inputs, status `2` takes precedence over `1`.
Status `1` takes precedence over `0`.
Reject no-input analysis instead of reporting success for no work.

Done when: subprocess tests check actual exit codes, output streams, overrides, multiple inputs, and unchanged input bytes.

### Task 21: add deterministic text and JSON reports

Prerequisites: Task 20.

Proposed option: `--format text|json`.
Default: `text`.

- [ ] Show file, location, severity, identifier, and message in text output.
- [ ] Include compiler and rule-set versions in a text report footer.
- [ ] Send text diagnostics and report metadata to stderr.
- [ ] Keep stdout empty in text mode, except for explicit listing or version requests.
- [ ] Write one complete report to stdout in JSON mode.
- [ ] Include schema version, compiler version, and rule-set version in JSON.
- [ ] Include input files, result status, counts, and effective configuration metadata.
- [ ] Include severity, compiler code or rule name, locations, notes, and safe edit groups.
- [ ] Describe coordinate units in the schema documentation.
- [ ] Keep findings in deterministic order.
- [ ] Escape quotes, backslashes, control characters, and non-ASCII content correctly.
- [ ] Keep ordinary progress messages out of JSON stdout.
- [ ] Define JSON failure reports for configuration, loading, and analysis errors.
- [ ] Keep the existing `vx-diagnostics-v1` compiler report unchanged.

Proposed separate schema name: `vx-lint-v1`.
Do not repurpose the compiler's admission report as a lint report.
Use a JSON serializer instead of constructing escaped strings in each rule.

Done when: parsed JSON assertions check every required field and repeated runs produce equivalent reports.

## Phase: editor

### Task 22: make IDE diagnostics use shared analysis

Prerequisites: Task 21.

Change: `src/ide/mod.rs` and shared analysis adapters.

- [ ] Replace duplicated analysis in `Analysis::diagnostics` with the shared API.
- [ ] Pass editor text through source overrides keyed by the real document path.
- [ ] Remove temporary-file staging from the diagnostics path after overrides work.
- [ ] Preserve other scratch-file consumers until their separate behavior can change safely.
- [ ] Resolve project configuration from the document path.
- [ ] Use the same registry and level policy as the CLI.
- [ ] Preserve severity, code, source identity, notes, and edit groups in `IdeDiagnostic`.
- [ ] Keep compiler errors visible when the project allows optional rules.
- [ ] Avoid publishing findings from an imported file at the entry file's coordinates.
- [ ] Check unsaved source in the entry file and imported files.
- [ ] Preserve existing hover and definition behavior.

Done when: CLI and IDE results agree for the same source snapshot and effective configuration.

### Task 23: preserve diagnostic meaning through LSP

Prerequisites: Task 22.

Change: `vx-analyzer/src/main.rs`.
LSP means Language Server Protocol, the editor's message format.

- [ ] Map compiler errors and denied lint findings to LSP error severity.
- [ ] Map compiler warnings and warned lint findings to LSP warning severity.
- [ ] Publish no diagnostic for an allowed rule.
- [ ] Preserve compiler codes and optional rule names in the LSP `code` field.
- [ ] Identify compiler findings as `vxc` and optional findings as `vx-lint`.
- [ ] Convert all ranges through the shared UTF-16 conversion.
- [ ] Attach located notes as related information.
- [ ] Retain unlocated notes in the message or structured diagnostic data.
- [ ] Retain safe edits in structured data for Task 24.
- [ ] Include document versions where supported by the current notification flow.
- [ ] Replace old diagnostics when a new successful analysis has none.
- [ ] Check that a source edit cannot leave stale lint diagnostics in the editor.

Done when: LSP mapping tests check warning severity, denied severity, codes, notes, Unicode ranges, and diagnostic clearing.

### Task 24: expose safe edit groups to the editor

Prerequisites: Task 23.

Keep this task narrow.
It supports edits already established as safe by a rule.
It does not add a batch fixer.

- [ ] Define an edit group that belongs to one finding and one source snapshot.
- [ ] Retain the expected original text or document version for stale-edit detection.
- [ ] Require edits in a group to have valid UTF-8 boundaries and nonoverlapping ranges.
- [ ] Keep every edit in its original source file.
- [ ] Exclude generated source and uncertain comment ranges.
- [ ] Advertise code-action support through the LSP server capabilities.
- [ ] Handle `textDocument/codeAction` requests using the established safe edit groups.
- [ ] Convert checked edits into a versioned LSP quick-fix code action.
- [ ] Reject stale edits when their source snapshot no longer matches.
- [ ] Apply all edits in a group together.
- [ ] Check the complete edited source by parsing and analyzing it in tests.
- [ ] Check that the intended finding disappears without introducing compiler errors.
- [ ] Retain advisory diagnostics when no safe edit exists.

For `return best;`, a group can remove the keyword and semicolon separately.
Keep the expression and surrounding comments unchanged.
Do not label a whole-function replacement as a safe edit merely because it parses.

Done when: an editor integration test applies the supported fix and a stale-source test rejects it.

## Phase: release and verification

### Task 25: finish user documentation and release checks

Prerequisites: all earlier tasks included in the release scope.

Proposed user guide: `docs/tools/vx-lint.md`.
Create its directory only if the existing documentation structure needs it.
Link it from the repository's current tooling documentation.

- [ ] Document the formatter policy and lint-command scope.
- [ ] Document every accepted rule with matching and nonmatching Vx examples.
- [ ] Document defaults, parameter counting, thresholds, and skipped source forms.
- [ ] Document configuration discovery, precedence, explicit-file scope, and exit statuses.
- [ ] Document compiler-error behavior and existing warning ownership.
- [ ] Document JSON schema and version metadata.
- [ ] Document editor diagnostics and supported quick fixes.
- [ ] Keep unimplemented commands clearly marked as proposed.
- [ ] Add focused lint and analyzer checks to CI.
- [ ] Check that each CI filter selects at least one intended test.
- [ ] Preserve existing compiler JSON consumers and warning tests.
- [ ] Run the Vx preflight checks required by the final changed surface.
- [ ] Record exact commands, results, mutation checks, and checks not run.
- [ ] Compare the final implementation with each task's completion condition.
- [ ] Leave deferred or incomplete tasks unchecked.

Done when: implemented behavior matches user documentation and the final handoff identifies every remaining limitation.

## Verification commands and test placement

These are command templates for implementation work.
They are not evidence that checks ran while preparing this TODO.

Activate the existing project virtual environment before Python tooling.
Deactivate it when work finishes.
Never replace a missing environment or hardcode an LLVM installation path.

Set a task-specific worker variable to a suitable integer before using these templates.
Use `VX_TASK_JOBS` for Cargo and Rayon limits.
Use `VX_TASK_TEST_THREADS` for Rust test concurrency.
Both values must respect the repository's two-idle-core rule.
Account for nested test pools when selecting them.
Set these variables in the same shell invocation as each command.

Every Cargo command needs `source config.local` in the same shell invocation.
Create logs under the relevant build directory.
Use a shell with `pipefail` when recording output through `tee`.
Check the Cargo exit status rather than accepting `tee` success.

Existing focused formatter command:

```sh
source config.local
CARGO_BUILD_JOBS="$VX_TASK_JOBS" RAYON_NUM_THREADS="$VX_TASK_JOBS" \
  cargo test -p vxc --lib formatter::tests \
  -- --test-threads "$VX_TASK_TEST_THREADS"
```

Existing frontend and warning runners:

```sh
source config.local
CARGO_BUILD_JOBS="$VX_TASK_JOBS" RAYON_NUM_THREADS="$VX_TASK_JOBS" \
  cargo test -p vxc --test integration_test \
  integration_test::compile_test::test_frontend \
  -- --test-threads "$VX_TASK_TEST_THREADS"
```

```sh
source config.local
CARGO_BUILD_JOBS="$VX_TASK_JOBS" RAYON_NUM_THREADS="$VX_TASK_JOBS" \
  cargo test -p vxc --test integration_test \
  integration_test::compile_test::test_warnings \
  -- --exact --test-threads "$VX_TASK_TEST_THREADS"
```

Existing flat-path coverage gate when preflight requires it:

```sh
source config.local
CARGO_BUILD_JOBS="$VX_TASK_JOBS" RAYON_NUM_THREADS="$VX_TASK_JOBS" \
  cargo test --release -p vxc --test integration_test \
  -- --exact integration_test::flat_corpus_sweep::flat_path_coverage_of_the_backend_corpus_holds \
  --nocapture --test-threads "$VX_TASK_TEST_THREADS"
```

Check that the exact gate reports one passed test.
For affected generated-code fixtures, follow preflight's `VX_FLAT_DBG=1` checks.
A successful program alone does not prove which backend path handled it.

Use these proposed test locations after creating their modules:

| Test surface | Proposed location | Suggested filter |
| --- | --- | --- |
| Source positions and analysis helpers | `src/analysis/` unit tests | `cargo test -p vxc --lib analysis::` |
| Registry, configuration, and rules | `src/lint/` unit tests | `cargo test -p vxc --lib lint::` |
| Check-only and imported-error behavior | `tests/integration_test/analysis_test.rs` | `cargo test -p vxc --test integration_test analysis_test` |
| Real lint command and exit status | `tests/integration_test/vx_lint_test.rs` | `cargo test -p vxc --test integration_test vx_lint_test` |
| Shared IDE diagnostic behavior | `src/ide/mod.rs` unit tests | `cargo test -p vxc --lib ide::` |
| LSP mapping and code actions | `vx-analyzer/src/main.rs` unit tests | `cargo test -p vx-analyzer` |

Apply the same environment and worker limits to these suggested filters.
Do not copy the short filter alone into a shell without `source config.local`.
Do not interpret zero selected tests as success.

Create dedicated fixtures under `tests/lint/` only when their Rust test module reads them.
The current compiler runners do not automatically discover that proposed directory.
Do not assume a `CHECK` comment asserts anything without the corresponding test runner.
Use `utils/update_mlir_test_checks.rs` when adding or changing MLIR checks, as repository guidance requires.
Most lint assertions should inspect structured diagnostics directly.
Avoid adding MLIR fixtures merely to test style advice.

## Required end-to-end cases

Each row needs an assertion at the named boundary.
A unit test of a helper does not replace CLI or editor coverage.

| Case | CLI result | Editor result |
| --- | --- | --- |
| All style rules at default levels | Existing compiler findings only. | Existing compiler findings only. |
| One matching rule at `warn` | One warning and exit `0`. | One warning with the rule name. |
| One matching rule at `deny` | One error and exit `1`. | One error with the rule name. |
| Matching rule overridden to `allow` | No optional finding. | No optional finding under equivalent configuration. |
| Compiler type error with all rules allowed | Compiler error and exit `1`. | Compiler error remains visible. |
| Unknown configuration rule | Configuration error and exit `2`. | Visible configuration problem associated with its source. |
| Imported source with optional style issues | No optional dependency findings unless explicitly named. | No findings assigned to the wrong file. |
| Two explicit inputs sharing imports | Stable findings without repeated shared compiler reports. | Accurate per-document findings. |
| Unsaved editor text differs from disk | CLI matches the supplied equivalent source snapshot. | Findings use the unsaved buffer. |
| Non-ASCII text before a finding | Accurate source range and JSON units. | Accurate UTF-16 range. |
| Safe return edit | Original file stays unchanged during linting. | A quick fix changes only the established safe ranges. |
| Repeated analysis after a correction | The old finding disappears. | The old published diagnostic clears. |

## Explicitly deferred work

Do not implement these features while completing earlier tasks:

- Scoped source attributes, suppression reasons, `forbid`, and `expect`.
- Configurable levels for existing compiler `W` codes.
- Rule groups, wildcard names, and project-wide preset packs.
- Baselines, path filters, and recursive directory scanning.
- Automatic line-width wrapping in `vx-format`.
- A general concrete syntax tree that preserves every source detail.
- Whole-project incremental lint caches and background scheduling.
- Production type-aware rules without a concrete detector specification.
- Advice about placement, ownership, or transfers that duplicates language errors.
- CLI `--fix`, batch edits, and automatic source rewrites in CI.
- A separate lint repository or stable public compiler analysis API.
- Declarative detector expressions and native plugin loading.
- Versioned external detector protocols before users need private rules.

Revisit a deferred item only with a separate task and concrete examples.
Preserve the formatter, correctness, and optional-policy boundaries throughout that work.
