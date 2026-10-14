<!--
Part of the Vx Project, under the Apache License v2.0 with LLVM Exceptions.
See LICENSE for license information.
SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
-->

# Nested enum layout TODO plan

**Status:** in progress, October 13, 2026. Implementation tasks for
[issue 570](https://github.com/vx-lang/Vx/issues/570), based on
[the nested enum and struct layout proposal](nested_enum_and_struct_layouts.md).
Section 1 is implemented as of October 9, 2026. Section 2's first three tasks
are implemented as of October 13, 2026. Its remaining tasks and the later
sections describe work to do.

The target is correct construction, calls, returns, borrows, matching, and
drops for `Result<u64, Error>`, where `Error` contains `Option<i32>`.
A normal compile and a parallel compile (`-j`) must both use the default
code generator and preserve every active payload. Use local fixture types;
the public file API and `std::io::Error` remain separate work under issue 1341.

## 1. Reject unsupported payload layouts before enabling nested support

**Baseline before the guard, verified on macOS, October 8, 2026:** an outer enum with
`Number(i32)` and `Nested(Inner)`, where `Inner` carries `i64`, was accepted by
the flat path. Constructing `Nested` generated a 16-byte inner-enum store at
byte 4 of an 8-byte outer allocation. The MLIR parsed and lowered successfully;
the defect was an out-of-bounds store, not a parser rejection. Normal and
`-j 2` compilation both accepted the reproducer.

The regression
`flat_declines_an_enum_with_an_unsupported_nested_payload` in
`tests/integration_test/flat_codegen_differential.rs` failed at the intended
decline assertion before the guard. It now runs without an ignore marker and
passes. Three additional decline tests cover reversed variant order and a
second payload position in both orders; a fifth checks `Outer<T>` with `T`
substituted by `Inner`. These tests assert the exact lowering result,
`Decline::TypeNotModelled { what: "an enum with no modelled instance layout" }`.
Registry and type-checking failures fail setup rather than counting as a decline.
A control test checks that variants
with genuinely absent payload positions still compile and run on the flat path.
Run the five decline regressions with:

```sh
source venv/bin/activate
source config.local
cargo test -j 2 --test integration_test \
  integration_test::flat_codegen_differential::flat_declines_an_enum_with_an_unsupported \
  -- --nocapture
deactivate
```

When nested layouts are supported later, replace the decline expectations with
checks for correct storage and preserved values.

`driver_falls_back_for_an_unsupported_enum_layout` and
`parallel_driver_falls_back_for_an_unsupported_enum_layout` run the real
compiler with normal compilation and `-j 2`. They require a successful exit,
the exact decline reason, the AST fallback marker, and MLIR that parses and
verifies. The parallel test also requires its frontend's fallback marker; it
skips hosts that cannot leave two CPU cores idle. Both stop before execution.
The `enum` test filter below includes these CLI regressions.

- [x] In `Lowerer::enum_instance_layout_of` in `src/hir/flatten.rs`, distinguish
  an absent payload position from a present payload whose layout is unsupported.
  A present unsupported payload must make the entire layout decline; replace
  the current `continue` after a failed `payload_field` call.
- [x] Add a regression where one variant has a supported payload and another
  has an unsupported payload at the same position. Check both variant orders
  and a later payload position. Assert that no partial layout is accepted.
- [x] Temporarily restore the skip and confirm the intended regression
  assertion fails; restore the guard and rerun.

**Done when:** an unsupported variant cannot silently disappear from the size
calculation, and existing supported enum fixtures still pass. Land this guard
independently before the remaining implementation.

**Validation on macOS, October 9, 2026:** `cargo test -j 2 --test integration_test enum -- --test-threads=2` passed all 17 selected tests. The
exact backend coverage gate in section 6 passed one selected test without
changing `KNOWN_DECLINES`. `cargo fmt --check` passed. Restoring the old skip
made all five decline assertions and both CLI assertions fail. Changing only
the decline reason also made those seven assertions fail, while compilation
still declined and the CLI fallback succeeded. Rejecting absent positions made
the control assertion fail in the initial validation. All mutations were
removed and the 17 tests rerun.
The command-line nested reproducer now declines before flat code generation
in normal and `-j 2` compilation and falls back to the AST generator. The
existing `enum_mixing_a_number_and_a_struct_payload.vx` fixture still uses the
flat path and prints `4 7 0` in both modes. Linux validation remains for CI.

## 2. Carry complete layouts from lowering to code generation

**First implementation step, October 13, 2026:** the lowerer, worker, and
emitter argument now use `GeneratedAggregateLayout` from `src/layout.rs`.
The record carries size, alignment, logical fields, physical storage fields,
and each variant's actual payload types. Each logical field groups its
`FieldLayout`, physical storage index, nested aggregate ID, and pointer target ID.
Physical fields identify padding separately from values.

The lowerer derives its cached `TypeDefinition` from the shared record through
`TypeLayout`. Enum offsets and nested aggregate sizes also come from the record.
An explicit conversion builds the emitter's storage view. Field stores, loads,
and addresses use the physical field index from that view.
Tests insert explicit padding into a generated record and check both views
and all three field operations. Production layout calculations retain their
existing storage shapes and supported cases.

`lower_data_match` reads actual payload types from the record and retains them
for type inference inside each arm. Pointer target IDs do not depend on the
order in which a worker resolves layouts. Producers, both compiler drivers,
the emitter argument, and direct test callers use the named record.

The second implementation step remains outstanding. The emitter still fills
generated layouts' `field_agg` and `field_pointee` entries with `None`.
It accepts the first record for each type ID without checking whether later
records agree. Emitter metadata-transfer and duplicate-layout tests remain
to do.

Some supporting behavior already exists:

- `lower_data_match` uses each variant's actual payload type for bindings,
  rather than the shared storage slot's type. Emission still needs each
  variant's recorded aggregate and pointer target IDs.

- `emit_module_mlir` installs generated layouts before computing function
  signatures and rebuilds the callee map afterward. It does not rebuild the
  nominal lookup after installing those layouts.

- Commit a7bf5f09 (October 10, 2026, PR #1533) added generic structs containing
  `Option<T>`. The emitter obtains a nested aggregate ID from the field load's
  result type. This supports that shape without transferring complete layout
  metadata. The fixture is `tests/backend/pass/generic_struct_with_an_option_field.vx`.

- [x] Replace the tuple in `Lowerer::agg_layouts` and
  `Worker::local_agg_layouts` with a named record in a shared layout module.
  Include type ID, total size, alignment, logical offsets, physical storage
  types, logical-to-physical field mapping, nested aggregate IDs, and pointer
  target IDs where known.

- [x] Separate logical field descriptions from physical storage fields,
  including padding, within the record. Group related metadata into field
  records. Make the mapping explicit when logical and physical field indexes
  differ.

- [x] Make the shared record the authoritative source for generated layouts.
  Derive the lowerer and emitter views from it. Reuse suitable `TypeLayout`
  and `FieldLayout` concepts from `src/layout.rs`. Keep MLIR storage details
  separate from logical field metadata behind an explicit conversion to the
  emitter's representation.

- [ ] Keep each variant's actual payload types separate from the types used to
  store the shared payload slots. Preserve the existing variant-specific match
  binding types. Keep nested aggregate and pointer target IDs specific to each
  variant's concrete payload types. Preserve those IDs through emission.
  A shared storage slot cannot supply one aggregate or pointer target ID for
  every variant. Use the active variant's payload metadata for enum field loads.
  A carrier type must not determine a match binding's type, signedness, or
  pointer metadata.

- [x] Update the producers and consumers in `src/hir/flatten.rs`,
  `src/session.rs`, `src/driver.rs`, `src/pipeline.rs`, and `src/codegen/flat.rs`.
  Update direct callers in `tests/integration_test/flat_codegen_differential.rs`
  and code generation unit tests as well.

- [ ] In `emit_module_mlir`, install complete generated layouts and rebuild
  the required nominal lookup before computing callee and function signatures.
  Preserve the existing layout installation and callee-map rebuild order.
  Preserve nested aggregate and pointer target metadata instead of filling it
  with `None`.

- [ ] When functions or workers produce the same type ID, assert their layout
  records agree. Keep mutable resolution local to each worker.

- [ ] Add focused tests for metadata surviving the worker-to-emitter path,
  equal duplicate records being accepted, and conflicting records being
  rejected. Check variant-specific metadata when variants share a storage slot.
  Check existing normal and parallel compilation behavior.

Keep this step focused on complete metadata transfer and layout agreement.
Preserve existing storage shapes. Enable new nested shapes with resolver and
storage tests in section 3.

**Validation of the first implementation step on macOS, October 13, 2026:**
95 lowering tests, 26 emitter tests, and 165 differential tests passed.
The exact backend coverage gate passed one selected test without changing
`KNOWN_DECLINES`. `cargo fmt --all --check` and `git diff --check` passed.
The seven new tests are included in the lowering and emitter counts.

```sh
source venv/bin/activate
export RAYON_NUM_THREADS=2
source config.local
cargo test -j 2 -p vxc --lib hir::flatten::tests -- --test-threads=2
source config.local
cargo test -j 2 -p vxc --lib codegen::flat::tests -- --test-threads=2
source config.local
cargo test -j 2 --test integration_test \
  integration_test::flat_codegen_differential -- --test-threads=2
source config.local
cargo test -j 2 --test integration_test \
  integration_test::flat_corpus_sweep::flat_path_coverage_of_the_backend_corpus_holds \
  -- --exact
deactivate
```

Twelve separate mutations failed at their intended assertions. They changed
nested IDs, size, alignment, logical-to-physical mapping, variant types, pointer
targets, the lowerer view, the emitter's padding and mapping, and each field
operation's physical index. All mutations were removed before the final tests.

Five existing fixtures passed linked native execution with a normal compile
and `-j 2`: `generic_struct_with_an_option_field.vx`,
`narrow_enum_payload_in_an_expression.vx`,
`enum_variants_with_different_number_types.vx`,
`enum_mixing_a_number_and_a_struct_payload.vx`, and `enum_drops_its_payload.vx`.
Each used `VX_FLAT_DBG=1 target/debug/vxc tests/backend/pass/<fixture> --run`,
with `-j 2` added for the parallel compile. Checks required the flat-path
marker and expected values through FileCheck or the fixture's `EXPECT` line.
Logs are in `build/enum-layout-step1-*.log`. Linux validation remains for CI.

The repository's pre-commit hook also ran Clippy and the full test suite.
Clippy passed with `--workspace --all-targets --all-features -- -D warnings`.
The full `cargo test` run reported five integration test failures:

- `ane_device_test::the_dispatcher_reports_the_device_coreml_chose`: the
  dispatcher could not find the model on its search path.
- `compile_test::test_backend`,
  `emit_obj_produces_objects::the_backend_corpus_emits_object_files`, and
  `flat_corpus_sweep::flat_path_answers_match_the_backend_expectations`:
  boolean and string fixtures could not link required runtime symbols.
- `compile_test::test_backend_fail`: `box_oob.vx` did not produce its expected
  `Caught SIGSEGV` diagnostic.

These failures remain unresolved. They were not checked against the baseline.

**Done when:** the named record reaches every emitter entry point without
losing metadata, duplicate layouts cannot depend on insertion order, and
existing programs retain their behavior. Review this as the first PR after
the safety guard.

## 3. Resolve concrete nested types and build correct storage

- [ ] Add one concrete layout resolver beside the lowerer's instance tables;
  put reusable size and alignment calculations in `src/layout.rs`. Reuse
  complete registry layouts without changing the serialized registry format.
- [ ] Make `lower_ty_synth`, `payload_field`, `struct_instance_layout_of`, and
  `enum_instance_layout_of` use this resolver. Support a non-generic struct
  with an incomplete registry layout, an enum inside a struct, and an enum
  inside another enum's payload.
- [ ] Resolve declaration IDs before using unambiguous name lookup. Preserve
  nominal IDs for non-generic structs and concrete instance IDs for generics.
  Use structural `Type` arguments instead of parsing their display strings.
- [ ] Cache completed layouts and track types currently being resolved using
  their concrete generic arguments. Decline cycles through values, retain
  existing infinitely sized type diagnostics, and avoid expanding pointer or
  borrow targets just to compute their storage size.
- [ ] Register each completed enum's size, alignment, and fields in
  `instance_layouts` as well as its emitter storage shape. Construction,
  allocation, and access must use the same layout.
- [ ] For each payload position, independently find the maximum payload size
  and maximum alignment. Round the slot size up to that alignment, align its
  offset after the preceding field, and round the whole enum size up to the
  maximum tag/slot alignment. Use checked arithmetic and propagate failures.
- [ ] Build LLVM storage that realizes those offsets, size, and alignment.
  Use validated alignment carriers and explicit padding where needed; preserve
  integer bits, floating-point bits, and pointer round trips during whole-value
  copies. Preserve existing supported payload types.
- [ ] Use the explicit logical-to-physical field mapping when padding adds
  storage fields. Preserve logical field identity through field loads, stores,
  and address operations so the emitter can use the mapping. Do not identify
  fields with `offsets.position`, since zero-sized fields can share offsets.
  A mapping alone cannot recover field identity from a shared byte offset.
- [ ] Add unit tests for cache reuse, distinct generic instances, failure
  propagation, value cycles, pointer recursion, arithmetic overflow, tail
  padding, and zero-sized fields. Use the existing `proptest` dependency for
  size/alignment combinations with an independent layout oracle.
- [ ] Compare generated storage with LLVM's layout calculation, including a
  12-byte struct aligned to 4 sharing a slot with `i64`: the slot must have
  size 16 and alignment 8, with an aligned offset after the tag.
- [ ] Audit `src/codegen/generator.rs` and `src/codegen/lower/expr.rs` before
  changing storage shapes. Verify existing supported calls across imported
  modules and differential fixtures remain compatible.

**Done when:** nested concrete layouts are complete and agree with LLVM,
including padding and alignment. Review this as the second implementation PR;
enable each new shape together with tests that exercise it.

## 4. Preserve nested values through access, calls, returns, and drops

- [ ] Use the resolver for `lower_data_match` bindings and field access. Track
  the loaded payload's actual aggregate ID, pointer metadata, and signedness.
- [ ] Audit `op_field_load`, `op_field_store`, and `op_field_addr` in
  `src/codegen/flat/emit/aggregate.rs`, plus aggregate loads, stores, and
  parameters in `src/codegen/flat/emit/memory.rs`. Store constructed values
  as values rather than addresses of temporary stack slots.
- [ ] Check chained field access and borrowed method receivers against the
  nested value's own layout. Pass nested values as parameters and return both
  whole results and extracted `Error` and `Option<i32>` values.
- [ ] Handle a match whose arms all return an aggregate using the existing
  unreachable-block path when its merge block is provably unreachable. Avoid
  inventing a zero aggregate for the current non-scalar default return decline.
  Preserve reachable match expressions unless an acceptance case needs a fix.
- [ ] Audit generated drop calls: inspect only the active variant, preserve
  the checker's move rules, and drop a moved owning payload exactly once.

**Done when:** nested fields and payloads survive every required value
operation. Review this with the behavior fixtures below as the third PR;
split out an independent return or drop defect if the audit finds one.

## 5. Add acceptance fixtures with checks that prove their claims

Add these fixtures under `tests/backend/pass/`. Include an imported helper
module to exercise calls whose layouts come from different workers.

| TODO fixture | Required checks |
| --- | --- |
| [ ] `result_holds_an_error_with_an_optional_code.vx` | `Result<u64, Error>` preserves an `Ok` count above the signed 32-bit range and `Err` kind, operation, `Some(negative_code)`, and `None`; check construction, calls, returns, and extracted accessors. |
| [ ] `result_holds_a_small_number_or_a_struct.vx` | `Result<i8, Bad>` preserves both variant types through the real `core::result` methods `unwrap_or` and `unwrap_err` on valid branches. |
| [ ] `generic_struct_holds_an_option.vx` | `Holder<Option<i32>>` preserves both states through direct field access and a borrowed accessor. |
| [ ] `enum_holds_another_enum.vx` | `Option<Option<i32>>`, `Result<Option<i32>, Error>`, and a multi-value variant preserve outer/inner tags and every payload position. |
| [ ] `enum_payloads_have_different_size_and_alignment.vx` | A 12-byte, 4-aligned struct and `i64` share a correctly sized, 8-aligned slot; assert LLVM offsets/alignment and values surviving a native call. |
| [ ] `nested_result_drops_only_its_active_payload.vx` | Only the active owner drops; moving it prevents a second drop and inactive variants never drop it. |

- [ ] Use `REQUIRES: flat-codegen` for mixed layouts the legacy generator
  cannot represent. Continue testing both generators for supported fixtures
  and compare observable values.
- [ ] Generate MLIR checks with `utils/update_mlir_test_checks.rs`; inspect
  that each check asserts the property its test claims.
- [ ] For each important assertion, temporarily introduce its specific fault
  from the proposal's mutation table, confirm that assertion fails, restore
  the implementation, and rerun. Compilation failures and unrelated assertion
  failures do not prove payload checks.

## 6. Validate and document completion

- [ ] Before implementation validation, confirm `venv` and `config.local`
  exist, activate the environment, source `config.local` before every Cargo
  command, leave two CPU cores idle, and save logs under the build directory.

- [ ] Run focused layout and emitter unit tests and all new fixtures using a
  normal compile, a parallel compile with two workers when the host allows it,
  and a linked native executable. Require valid MLIR, expected output, and
  `VX_FLAT_DBG=1` confirmation of the flat path.

- [ ] Run existing scalar, struct payload, nested generic struct, and drop
  fixtures, applicable differential tests, and
  `tests/integration_test/codegen_determinism.rs` checks.

- [ ] Run the exact backend coverage gate and confirm a nonzero test count:

  ```sh
  source config.local
  cargo test -j 2 --test integration_test \
    integration_test::flat_corpus_sweep::flat_path_coverage_of_the_backend_corpus_holds \
    -- --exact
  ```

- [ ] Remove entries from `KNOWN_DECLINES` in
  `tests/integration_test/flat_corpus_sweep.rs` only after the corresponding
  fixture demonstrably uses the flat path.

- [ ] Verify the acceptance cases on macOS and Linux; record exact commands,
  results, mutation failures, and any remaining limitations. Deactivate the
  virtual environment after validation.

- [ ] Update the compiler limitations in `core_library.md`,
  `file_io_usability.md`, and `io_error_contract.md` in this directory. Add a
  nested enum example to `docs/tutorial/generics.md` and explain recursive
  substitution and unchanged move rules in `docs/semantics/generics.md`.

- [ ] Complete the `vx-pr-preflight` review before opening or updating a PR.
  Branch from `main`; use a GitHub stack for dependent PRs reviewed together.
  Report `Issue: #570` as partial work until the safety guard and all remaining
  issue acceptance cases have been verified.

**Final completion check:** every acceptance case uses the default compiler,
normal and parallel compilation agree, native output is correct, layouts agree
with LLVM, and mutation checks prove the important assertions. Unsupported
layouts still decline in a controlled way. Public file wrappers, buffers,
the native file backend, and full legacy-generator support remain separate work.
