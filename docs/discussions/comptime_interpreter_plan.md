# Comptime interpreter plan

This note records the work on preventing a `comptime` block from folding away a write that can
reach a binding outside that block.

## 1. First approach

The first approach kept a set of names that looked like mutable aliases of an outer binding. When
an `if` or `match` condition was unknown, it walked both paths and rejected the `comptime` block
if it saw a write through one of those names.

That helped for the first direct cases, but it was the wrong model. A name is not the value that
may carry the reference.

It failed or became fragile when a mutable reference was:

- declared before or inside an unknown branch;
- copied into another local;
- reassigned to a local reference;
- shadowed by a new local with the same spelling;
- stored in a struct, enum payload, array, or closure environment;
- passed through a function value or callback; or
- reached through a member, dereference, index, assertion, topology expression, or nested call.

The biggest roadblock was that name tracking cannot distinguish `p` still aliasing `outside` from
the same `p` after `p = &mut local`. Adding more name sets only created stale facts and false
positives. Separately, the evaluator skipped several composite expressions, so a syntax walk could
miss a direct path that the evaluator silently folded.

## 2. 2nd approach

The current implementation tracks facts carried by values, not just identifier spellings.

`ComptimeValueFacts` records:

- `reference_origins`: outer bindings a value can reach through a mutable reference;
- `callable_targets`: known functions carried by a function value;
- `captured_writes`: outer bindings a closure may write; and
- `unknown_callable`: a callable whose body is unavailable.

`ComptimeEffects` stores those facts in lexical scopes. A `let` installs facts for its value;
assignment replaces the facts of the nearest local binding; leaving a block drops that scope. This
makes shadowing and reassignment behave correctly.

The checker propagates facts through references, dereferences, members, indexes, arrays, struct
literals, enum payloads, closures, casts, function values, and known callee bodies. It also walks
the type graph to recognise values that can carry `&mut`, including nested structs, enums, generic
instances, wrappers, pointers, and references.

The evaluator keeps the same scoped facts for paths it actually runs. If an argument or callee
cannot be evaluated, it asks the effect summary whether the call can write an outer reference
before refusing to fold. This prevents a skipped aggregate or callable argument from hiding a
write.

One later bug was important: `&Holder` was treated as removing all mutable-reference facts. That
is wrong for `Holder { value: &mut i32 }`: an immutable borrow of the holder can still expose and
use `holder.value`. The current rule keeps facts carried by the value for every borrow, and adds
the storage-place write fact for `&mut` as well.

Relevant commits:

- `5ea7c95d`: scoped mutable-reference provenance.
- `66dc5a1d`: restored focused comptime fixtures.
- `43ce6f90`: aggregate borrows preserve inner mutable references.

### Problems with that approach

The second approach still has two separate traversals: the concrete evaluator and the unknown-path
effect summary. They can disagree about an expression's children; the former topology-index bug
silently folded a call because only the summary visited the index. The needed audit was therefore
unbounded: every child-bearing expression had to be kept in sync in both walkers. Function aliases
and closure paths also made body discovery depend on spelling outside the comptime block.

The imported regression corpus remains valuable behavioral evidence, but extending this split
design would mean more whack-a-mole coverage work rather than a structural guarantee.

## 3. Current Design

The current design is one comptime interpreter that combines concrete evaluation, abstract
unknown-path traversal, provenance, and escaping-write detection. Every evaluated value carries
its provenance. Known paths execute concretely; unknown paths traverse the same owners
abstractly, so there is one source of truth for both a value and the effects needed to produce it.
An owner without a supported rule fails closed rather than being silently treated as pure.

### Unified comptime evaluation plan (new branch from `main`)

This work replaces the duplicated concrete evaluator and unknown-path effect summary with one
comptime interpreter. The current branch remains available as a reference implementation and
regression corpus. The provenance model and its fixtures are behavioral evidence to preserve, not
a reason to continue extending the two-walker design.

#### 1. Evaluation contract — complete

A `comptime` block may be replaced by a constant only if all of the following are true:

1. Every operation on every concrete or potentially taken path has been interpreted by the shared
   comptime interpreter according to the rules below.
2. The block reaches a normal completion or `return` with a concrete result that can be converted
   to a source-level constant. A block with no result may disappear only after the same safety
   checks.
3. The interpretation is supported: it did not reach an unmodelled expression, statement,
   operation, opaque operation whose behavior is required, recursion/loop limit, or other
   indeterminate execution.
4. No executed or potentially executed operation writes a place whose provenance reaches a binding
   declared outside the block.

The interpreter must keep these states distinct. In particular, an absent concrete value must
never also mean “no effects” or “unsupported”:

| State | Meaning | Folding consequence |
| --- | --- | --- |
| **Known value** | A concrete `Value` is available. | May contribute to a fold. |
| **Unknown value** | Evaluation was modelled, but its runtime value is not known. | Prevents a value fold; child effects remain valid facts. |
| **Unsupported** | The interpreter has no sound semantics for this operation/path. | Refuse the fold; never retain a partial environment as certain. |
| **Flow** | Normal, return, break, continue, or indeterminate across viable paths. | Determines which later statements/expressions are reachable. |
| **Possible escaping write** | A write may reach one or more outer bindings. | Refuse the fold and report the selected outer binding. |

The implementation may use a single result type containing these dimensions, but must not collapse
them into `Option<Value>`. Concrete values stay in the existing `Value` type. A comptime-only
abstract value/result carries an optional concrete value together with
`ComptimeValueFacts`; provenance is not added to the compiler-wide `Value` representation.

##### Evaluation and abstract-control rules

- Every child expression required by the language's evaluation order is interpreted, even when an
  earlier child makes the enclosing result unknown. For example, an unknown array base does not
  excuse skipping an index expression, and an unknown earlier call argument does not excuse
  skipping later arguments.
- The only exception is semantically unreachable code: a known conditional chooses one arm; a
  `return`, `break`, or `continue` stops the appropriate subsequent evaluation; and a
  short-circuit operator may skip its right-hand child only when its known left-hand value proves
  the language would skip it.
- A known `if` or `match` executes only its selected path. An unknown condition/scrutinee
  interprets every viable path from cloned state, merges possible escaping writes, and produces an
  unknown value unless all viable paths produce the same concrete value under compatible state.
- A known finite loop executes with ordinary flow and scope rules. An unknown-trip-count loop must
  inspect its body abstractly for possible escaping writes, but otherwise refuses folding rather
  than claiming a result or post-loop environment that has not been proven.
- A known callee uses the same block interpreter with parameter facts and an isolated lexical
  scope. An indirect or opaque callee preserves possible writes from its receiver, arguments, and
  captures; it refuses folding whenever its result/effects cannot be proved.
- A declaration installs the facts of its evaluated value; assignment replaces facts for the
  nearest binding; leaving a lexical scope removes those facts. Storage-place provenance remains
  separate from value provenance: stores and `&mut` use the former, dereferences use the latter.
- All expression owners, including nested `Topology` indices, must be explicit cases in the
  shared interpreter. Unsupported syntax fails closed. There must be no catch-all path that
  silently treats an unhandled AST form as pure or concrete.

##### Diagnostics

If a possible escaping write has known outer origins, reject the block with E3033 and name one
deterministically selected origin. Prefer that diagnostic over a generic inability-to-fold error.
If no escaping write is possible but evaluation is unknown or unsupported, reject the fold with
the existing generic comptime-evaluation diagnostic. Diagnostics are emitted at the block's
existing chosen span and speculative evaluation remains silent.

This is a safety contract, not a promise to symbolically execute all runtime language features.
The proof obligation is positive: folding requires a supported, concrete, effect-safe result.
Anything not proved safe is left unfolded/refused as required by current `comptime` semantics.

> **Implementation status:** §1 is the settled target contract. The current interpreter implements
> the owner slices recorded below, including bounded finite-range and definite-break loop
> execution and aggregate-field precision. Casts and standalone range values are fixture-backed
> live refusals pending typed comptime values. Full callable/closure semantics, aggregate values
> as fold results, and full old/new value parity remain pending.

#### 2. Shared state and result model — complete

- [x] Define the comptime-only result/abstract-value types that represent the contract above.
- [x] Define one evaluation context for lexical fact scopes, outer-block bindings, possible
  escaping writes, call/loop guards, and support status. It is not a separate analysis pass.
- [x] Preserve replacement-based scoped facts, place/value distinction, callable targets,
  captured writes, unknown callables, and recursive-call guards.
- [x] Define cloning and merge rules for abstract branch state, including a deterministic merge of
  escaping origins and a conservative rule for incompatible concrete environments.

#### 3. Shared interpreter — staged implementation plan

The unit of work is one expression owner (or tightly coupled family), not a wholesale rewrite.
Each unit has an explicit supported or `unsupported` interpreter arm, its child-evaluation order,
and direct/unknown-control-flow fixtures. Do not add a catch-all arm to the core `Expr`,
`Statement`, or child-bearing `Topology` dispatch: a new AST variant must make the compiler fail
to build until the interpreter makes a decision for it.

##### 3.1 Establish the transition harness

- [x] Add a private `ComptimeInterpreter` beside the existing evaluator. It owns the concrete
  environment, `ComptimeEvalContext`, and `ComptimeEvalOutcome`; do not edit ordinary `eval_expr`
  or `check_statement` in place.
- [x] Give it exhaustive dispatch over `Expr`, `Statement`, and `Topology`. Every initially
  unsupported form must still visit its semantically evaluated children before returning
  `unsupported`.
- [x] Define a normalized observation carrying concrete value where available, support status,
  control flow, and a deterministically selected outer write. An explicit fold/no-fold verdict is
  still derived by the legacy path, so it remains part of the parity work below.
- [x] During migration, run the old and new paths for every `comptime` block. The legacy path is
  still authoritative for values, except that the new path may refuse an unsafe fold when it finds
  an escaping write the legacy scan missed.
- [x] Keep the narrow escaping-write comparator live: it rejects that one safety disagreement with
  E3033.
- [x] Promote the fixture-backed fail-closed policy for enum values, `SpawnOn`, `Transfer`, and
  inline MLIR. These owners have no comptime semantics, so their local-only effects may not
  disappear while broader parity is still transitional.
- [x] Extend it to normalized value/support/flow observations and record every unallowlisted
  disagreement as a diagnostic-quality transition warning or focused regression, never as a
  crash.
- [x] Maintain an explicit, fixture-backed allow-list of intentional disagreements. It currently
  contains only legacy bounded recursion (`comptime_recursive_quicksort` and the countdown in
  `comptime_call_unsupported_body`) and legacy nested-return flow
  (`comptime_return_unreachable_outer_write`). An allowance must state its semantic reason and
  must not suppress a possible escaping-write diagnostic.

##### 3.2 Port one expression owner at a time

For every checked item, use a small reviewable change with the acceptance rule below. Do not move
to a later family while its direct and unknown-path fixtures disagree with the required behavior.

- [ ] **Leaves and places — complete only after acceptance**

  - [x] Implement source-order traversal and provenance for identifiers, literals, borrows,
    dereferences, member/index access, assignments, and compound assignments.
  - [x] Preserve evaluated aggregate/member/index facts rather than recovering facts from syntax
    alone. Arrays and ordinary struct literals retain per-component values, so a selected field
    does not inherit a sibling's mutable-reference or callable facts; lowered closure
    environments retain their separate capture model.
  - [x] Write the early-unknown index regression fixture.
  - [x] Cover borrowed places with the direct dereference failure
    `comptime_value_call_deref_outer_write`, the unknown-path reborrow failure
    `comptime_unknown_reborrow_outer_write`, and the local-only reassignment/shadowing pass case
    `comptime_scoped_reference_provenance`.
  - [ ] Run those fixtures and confirm normalized old/new parity.

- [ ] **Pure operators and containers (non-topology) — complete only after acceptance**

  - [x] Implement structural traversal for unary/binary/relational/logical operators, casts,
    ranges, arrays, vectors, struct literals, and enum payloads.
  - [x] Implement concrete scalar arithmetic/relations, concrete arrays, and logical
    short-circuiting.
  - [x] Write the early-unknown binary-left/effectful-right failure fixture and the
    short-circuit-unreachable-write pass fixture.
  - [x] Preserve precise array-element and ordinary-struct-field provenance. The local-only
    `comptime_struct_field_precision` fixture proves that writing a local field does not become
    an outer write merely because a sibling holds `&mut outer`.
  - [x] Promote `AsCast` to a live refusal pending typed comptime values. The direct
    `comptime_cast_outer_write` and unknown-path `comptime_unknown_cast_outer_write` fixtures
    ensure an operand's outer write is diagnosed before the refusal;
    `comptime_cast_local_unsupported` proves legacy cannot silently drop a local cast statement
    and fold a later tail.
  - [x] Promote standalone `Range` values to a live refusal while retaining concrete integer-range
    execution for `for` loops. `comptime_range_outer_write` covers the later bound,
    `comptime_unknown_range_outer_write` covers the unknown-path first bound, and
    `comptime_range_local_unsupported` proves a local range statement cannot vanish before a
    foldable tail.
  - [ ] Implement or deliberately reject with focused fixtures aggregate *values* (as fold
    results), vectors, and enums. Structs still have no concrete legacy `Value` representation,
    so this completed provenance work is intentionally not a promise to fold a struct itself.
  - [ ] Add direct/unknown/local-only fixtures and confirm normalized old/new parity for the
    remaining container forms.

- [ ] **Topology-bearing predicates — complete only after acceptance**

  - [x] Traverse every topology child and query concrete `TransferPredicate` reachability when
    both topologies are known.
  - [ ] Model dynamic topology-index values precisely enough for predicate results, or retain a
    tested fail-closed rule.
  - [x] Add the early-unknown predicate fixture
    `comptime_unknown_predicate_later_outer_write`: an unknown first topology index cannot hide a
    write in the later topology operand.
  - [x] Add the direct predicate failure fixture
    `comptime_predicate_later_outer_write`.
  - [x] Add the local-only predicate pass fixture `comptime_predicate_local_write`.
  - [ ] Run fixtures and confirm normalized old/new parity.

- [ ] **Topology and placement owners — complete only after acceptance**

  - [x] Traverse `Topology` (NPU/GPU/AccCore and nested `Slice` indices), `Transfer`, `SpawnOn`,
    and `InlineMlir`; record MLIR clobber-place writes.
  - [x] Write the direct topology-index and direct `SpawnOn` regressions
    (`comptime_topology_index_outer_write` and `comptime_spawn_topology_outer_write`), and retain
    the imported unknown-path topology/spawn fixtures.
  - [x] Promote `SpawnOn` to a live fail-closed refusal pending concrete value semantics:
    `comptime_spawn_local_write_unsupported` rejects even a local-only spawn, while the direct
    outer-write fixture still names the escaping binding.
  - [x] Add the direct `Transfer` outer-write regression
    `comptime_transfer_outer_write`; its operand is observed before the transfer is refused.
  - [x] Promote `Transfer` to a live fail-closed refusal pending concrete value semantics:
    `comptime_transfer_local_write_unsupported` rejects even a local-only transfer.
  - [x] Add the direct inline-MLIR clobber regression
    `comptime_inline_mlir_outer_clobber`; its outer clobber is recorded before MLIR is refused.
  - [x] Promote inline MLIR to a live fail-closed refusal pending concrete value semantics:
    `comptime_inline_mlir_local_clobber_unsupported` rejects even a local-only clobber.
  - [x] Add the local-only topology-index pass fixture
    `comptime_topology_index_local_write`; retain the imported early-unknown topology/spawn
    failures.
  - [ ] Confirm normalized old/new parity for the placement fixtures.

- [ ] **Blocks and control flow — complete only after acceptance**

  - [x] Implement lexical concrete-environment restoration for blocks and tail expressions.
  - [x] Implement known/unknown `if`, known-scalar/abstract `match`, and abstract `for`/`loop`
    state handling; preserve `return`, `break`, and `continue` flow.
  - [x] Retain imported unknown-branch, match, loop, return, and shadowing fixtures.
  - [x] Write the known-branch nested-shadow pass fixture
    `comptime_nested_scope_shadow_restores_value` for concrete-environment restoration.
  - [x] Write the known-control pass fixture `comptime_known_control_skips_outer_write`, covering
    a false `if` arm and a nonmatching scalar `match` arm with unreachable outer writes.
  - [x] Add the direct enum-pattern regression `comptime_enum_match_outer_write`: enum matching
    remains fail-closed for values, but every viable arm is observed for escaping writes.
  - [x] Promote enum values to a live fail-closed refusal pending concrete enum semantics; the
    local-only fixture `comptime_enum_match_local_write_unsupported` verifies that an enum-driven
    match cannot make a write vanish.
  - [x] Execute concrete finite integer ranges and definite-break `loop`s up to the shared loop
    budget; unsupported/indeterminate recurrence remains fail-closed.
  - [x] Add bounded-`for` and plain-`loop` outer-write failures, local-only passes, and recurrence
    passes (`comptime_bounded_for_outer_write`, `comptime_bounded_for_local_write`,
    `comptime_bounded_for_recurrence`, `comptime_loop_outer_write`,
    `comptime_loop_local_write`, and `comptime_loop_break_recurrence`).
  - [ ] Confirm normalized old/new parity for the control-flow fixtures.

- [ ] **Calls and closures — complete only after acceptance**

  - [x] Implement known direct calls with mutable-parameter provenance, bounded recursive frames,
    and `MAX_CALL_DEPTH` enforcement.
  - [x] Carry named function values and known targets through locals, aggregates, members, indexes,
    and indirect-call syntax.
  - [x] Represent lowered `Closure_N` values as deferred generated targets with captured
    environments; detect writes through captured references and direct captured-scalar assignment.
  - [x] Write the direct captured-closure-assignment regression and retain imported function-value,
    aggregate-callback, and closure fixtures.
  - [x] Write the local-only closure-capture pass fixture
    `comptime_closure_local_capture_write`.
  - [ ] Model unlowered `Expr::Closure`, remaining closure/capture shapes, and opaque-call behavior
    with focused local-only cases.
  - [ ] Run the call/closure fixtures and confirm normalized old/new parity.

- [ ] **Peripheral forms — complete only after acceptance**

  - [x] Give autodiff, print/println, macros, `sizeof`, memory-space expressions, and all other
    variants explicit child traversal or explicit leaf rejection.
  - [x] Keep every peripheral form fail-closed pending concrete semantics.
  - [ ] Decide and test concrete semantics versus permanent rejection for each form.
  - [ ] Add fixtures and confirm normalized old/new parity.

##### 3.3 Required acceptance shape for each owner

- [ ] First add a vulnerable direct-fold fixture and demonstrate that the pre-owner implementation
  silently folds it. Keep that demonstration in a separate preparatory commit or recorded test
  result; do not claim a regression without evidence.
- [ ] Add the final failure fixture that names a distinct intended outer binding in E3033.
- [ ] Add the equivalent unknown-`if` or unknown-`match` fixture where that owner can be reached
  through abstract control flow.
- [ ] Add a local-only pass fixture when the form is otherwise foldable, guarding against stale
  provenance and over-conservative rejection.
- [ ] Run the focused fixtures and compare old/new normalized observations. Resolve every
  unallowlisted disagreement before considering the owner complete.

**Current acceptance status:** no §3.2 owner is checked off yet. The interpreter slices above are
implementation milestones only; each family still needs its direct/unknown/local-only fixtures
and normalized-parity result. The imported fixtures and the new structural regressions are input
to that gate, not evidence that it has passed.

**Known transition gaps to preserve for the next session:**

- Concrete environment scope restoration, bounded range recurrence, and definite-break loop
  recurrence are implemented and their six new focused loop fixtures pass when built with LLVM.
  They still require normalized-parity verification.
- Lowered closure values now defer their generated body and carry target/environment facts into
  `Closure_N_call`. An unlowered `Expr::Closure` is explicitly unsupported without running its
  body. Captured-place facts now preserve direct captured-scalar writes, including through the
  generated closure environment; the focused closure-write fixture passes.
- Named function values and lowered closure values populate `ComptimeValueFacts::callable_targets`.
  The latter also stores target-specific environment facts, so direct/indirect calls can interpret
  each known target with its generated `_env` argument. Opaque calls only reject conservatively
  when existing captured-write facts name an outer binding.
- Topology children are now included in comptime-body discovery, and a write through a mutable
  callee parameter preserves its caller's outer place origin. This makes direct calls in topology
  indices and direct mutable-parameter calls reject correctly.
- An outer function alias may not carry a resolved target into the shadow environment. Such a call
  is therefore treated as opaque and conservatively records every mutable-reference argument as a
  possible outer write. This makes the indirect alias and reborrow fixtures reject correctly
  without requiring a second, alias-sensitive body-discovery pass.
- Block interpretation now accumulates support status across every statement. An unsupported
  earlier statement can no longer be hidden by a later supported tail expression.
- The live comparator checks escaping writes first, then compares normalized concrete value,
  support, and flow while retaining the legacy fold verdict. An unallowlisted mismatch emits a
  transition warning rather than crashing or changing that verdict. Its only temporary allowance
  is nested-return flow, pinned by an existing frontend fixture. A separate narrow refusal policy
  also covers enum values, `SpawnOn`, `Transfer`, and inline MLIR.

#### Current hard stop: use the comparator to retire transition allowances

The LLVM build is now available with:

```bash
PATH="/opt/homebrew/opt/llvm@22/bin:$PATH" \
RUSTFLAGS="-Lnative=/opt/homebrew/opt/zstd/lib" \
cargo test
```

**Immediate next sequence:**

- [x] **Bounded recursive calls.** The shared interpreter now has the same bounded recursive-call
  behavior the legacy evaluator currently has, with isolated recursive frames and the existing
  depth limit still refusing evaluation at the limit. `comptime_recursive_quicksort` and the
  countdown in `comptime_call_unsupported_body` now agree with legacy evaluation; the recursive
  allowance is removed.
- [x] **Depth-limit diagnostic ownership.** The shadow result carries call-depth exhaustion to
  the fold boundary, which emits E8004 itself for `comptime_call_depth_limit`. During transition
  it deduplicates an E8004 already emitted by legacy assertion checking.
- [ ] **Nested-return flow.** Keep the shared interpreter's current flow as the intended
  semantics: a `return` nested in an expression-valued `if` makes following statements
  unreachable. Record the legacy evaluator's different behavior only as a transition allowance;
  remove that allowance when the new interpreter owns the fold verdict.
- [ ] **Owner acceptance.** Resume §3.2 with the remaining container semantics (aggregate fold
  values and vectors), then calls/closures, control flow, and peripheral forms. Check an
  owner only after its direct, unknown-path, and local-only fixtures have no unallowlisted
  comparator result.
- [ ] **Cut over.** Once every supported owner has parity and every unsupported owner has an
  explicit permanent refusal policy, use the shared interpreter's value/support/flow verdict for
  folding. Then delete the legacy fold evaluator and its name-based escaping-write scan in the
  mechanical cleanup described in §3.4.

The comparator is now implemented and exercised across the complete frontend pass corpus with no
unallowlisted transition warnings. It normalizes statement-only blocks to “no value,” so a final
statement value is not mistaken for a block result. The only current allowance is:

1. The legacy evaluator flattens a `return` nested in an expression-valued `if` into a normal
   expression value. The shared interpreter preserves the return flow and correctly leaves the
   following write unreachable. This is pinned by
   `comptime_return_unreachable_outer_write`.

The next work is owner acceptance for the remaining container semantics, followed by
calls/closures and control flow. Do not enable a blanket “shadow Unsupported refuses”: explicit
live refusal remains limited to the fixture-backed `SpawnOn`, `Transfer`, inline-MLIR, enum-match,
`AsCast`, and standalone `Range` owners.

##### 3.4 Cut over only after parity

- [ ] Keep dual-run enabled until every owner above has completed its fixture set and the full
  imported corpus has no unallowlisted disagreement.
- [ ] Switch `fold_comptime_block` to the new interpreter as the sole source of the fold verdict.
- [ ] Delete the old fold-specific evaluator path and the name-based `escaping_write` scan in one
  final mechanical cleanup change; do not leave a fallback that can silently choose the old path.
- [ ] Remove transitional comparison code, obsolete state, and comments only after the cutover
  suite is green.

#### 4. Migration sequencing

- [x] Add the three structural regressions: the direct topology-index write, an unknown base with
  an effectful index/operand, and a call with an earlier unknown plus a later effectful argument.
- [ ] Land owner changes in the order in §3.2 unless a failing fixture establishes a dependency.
- [ ] Keep each owner change independently reviewable and bisectable; do not combine unrelated
  owners merely to make a broad test suite pass.

#### 5. Regression and acceptance

- [x] Import and retain the existing provenance, aggregate, closure, reborrow, unknown-control-flow, and
  topology/spawn fixtures.
- [x] Add the direct `Topology::NPU[touch(&mut outside)]` failure fixture.
- [ ] Add direct and unknown-control-flow pairs for compound expressions to validate shared
  interpreter semantics.
- [x] Add early-unknown-child cases with a later outer-reference write for indexing, calls, and
  operators.
- [x] Add `comptime_struct_field_precision`, a local-only aggregate regression where an unrelated
  sibling retains `&mut outer`; it prevents a member selection from inheriting that sibling's
  provenance.
- [x] Add the early-unknown borrowed-place case
  `comptime_unknown_place_later_outer_write`.
- [x] Add the early-unknown-child case for topology-bearing predicates:
  `comptime_unknown_predicate_later_outer_write`.
- [ ] Cover the reference/container shapes already listed in this note, with distinct outer
  binding names in failures and local-only pass counterparts where folding is supported.
- [ ] Cover known/unknown branches, match arms, loops, returns, recursion limits, opaque calls,
  closures/captures, reassignment, shadowing, and aggregate transport.
- [x] Maintain exhaustive `Expr`, `Statement`, and child-bearing `Topology` dispatch in the
  shared interpreter: adding a variant requires an explicit compiler-checked arm. Do not add a
  wildcard arm.
- [ ] Run focused `comptime*.vx` pass/fail tests, `cargo test --lib`, and the applicable full
  frontend regression suite. **Latest run:** with the LLVM/zstd environment above, formatting and
  diff checks pass; `cargo test --lib` passes (562 passed, 1 ignored); and both frontend pass and
  fail corpora pass. A direct sweep of the complete frontend pass corpus has no unallowlisted
  comparator warnings. The full suite also
  has environment-only failures from absent `coremltools` and sandbox-disallowed TCP binds, so it
  is not the parity gate.

#### Completion criteria

- [ ] No standalone static mutable-effect traversal can disagree with the evaluator about an
  expression's children.
- [ ] A fold requires a concrete, supported result and no possible escaping write.
- [ ] Direct topology and early-unknown-child writes cannot silently fold away. The live safety
  comparator and focused fixtures are present; source execution verifies the covered direct and
  bounded-loop cases, while the unresolved indirect-call cases remain listed above.
- [ ] Intended folds and local-only provenance cases still pass.
- [ ] This branch remains independently reviewable against `main`.
