# Comptime mutable-reference handoff

This note records the work on preventing a `comptime` block from folding away a write that can
reach a binding outside that block.

## 1. Earlier approach: names and syntax

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

## 2. Current approach: scoped value provenance

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

## 3. Known open hole before pushing

Do **not** push until this is fixed.

The direct evaluator for `Expr::Topology` builds a topology value without evaluating the index
expression. This program currently folds to `7` and drops the write:

```vx
fn touch(value : &mut i32) -> i32 {
  *value = 9i32;
  return 0i32;
}

fn main() -> i32 {
  let mut outside : i32 = 0i32;
  let value = comptime {
    let _ignored = Topology::NPU[touch(&mut outside)];
    7i32
  };
  return value;
}
```

The corresponding unknown-branch fixture passes because the static effect walker visits topology
indices. The direct path bypasses that walker. `spawn on(Topology::NPU[...])` currently refuses to
fold generically, which is safe, but it should be audited after fixing topology evaluation.

## TODO: close nested-expression coverage systematically

The goal is not to add one special case per bug. Every expression that owns child expressions must
either evaluate all of them before folding, or mark the enclosing `comptime` evaluation unsupported.
For a child that can write through a mutable reference, the resulting diagnostic should name the
outer binding when provenance is known.

### Immediate work

1. Fix `Expr::Topology` evaluation to evaluate its index expressions before constructing `Value::Topology`.
2. Add a direct failure fixture for `Topology::NPU[touch(&mut outside)]`, with no unknown branch.
3. Recheck direct `SpawnOn` evaluation after that change. It must either observe nested effects or
   refuse the fold; it must never fold away the effect.
4. Repeat the full `comptime*.vx` pass/fail suite and `cargo test --lib`.

### Expression owners to audit

For each item below, test both a direct `comptime` path and the same expression under an unknown
`if` or `match` path. The direct case catches evaluator skips; the unknown case catches effect
summary skips.

- `Topology`: NPU, GPU, AccCore, and nested `Slice` indices.
- `SpawnOn`: topology, body statements, and optional returned value.
- `StructInit`: fields containing calls, borrows, nested structs, and callable values.
- `EnumVariant`: payloads containing calls and mutable-reference values; then destructuring via
  `match`.
- `Array` and `VecMacro`: elements containing mutable references, callbacks, and nested indexes.
- `IndexAccess` and assignments: effect in the base, index, and right-hand side.
- `MemberAccess` and dereference: direct field/ref access and chains such as `&mut **p`.
- `FunctionCall` and `IndirectCall`: direct calls, function values, closures, unknown callables,
  and callback values stored inside aggregates.
- `UnsafeBlock`, nested `ComptimeBlock`, `Match`, `If`, loops, and returns inside those forms.
- `InlineMlir` inputs and clobbers, plus the argument-bearing autodiff and print expressions.

### Reference/container shapes to test

Use a distinct outer binding name in each failure test so one error cannot satisfy another test's
`FileCheck` assertion. Every shape needs a matching local-only pass test when the operation is
otherwise foldable.

- `&mut T`, `&mut &mut T`, `&mut &mut &mut T`, and immutable borrows of each.
- `struct A { p: &mut T }`, then nested `struct B { a: A }` and several levels of nesting.
- Mutable references in enum payloads, including a value passed through a `match` binding.
- Arrays or vectors of references, followed by indexing and a call through the selected element.
- A reference stored in an aggregate passed by value, `&Aggregate`, and `&mut Aggregate`.
- A function value or closure stored in an aggregate, including a closure that captures a mutable
  reference through a nested aggregate.
- Generic instances whose type arguments contain a mutable reference.
- Reassignment from an outer alias to a local alias, and the reverse, across nested lexical scopes.
- Recursive and mutually recursive callees that receive a mutable reference or an aggregate that
  carries one.

### Guardrails

- Keep the default evaluator rule conservative: an unmodelled expression must refuse to fold.
- Keep the effect summary value-based and scoped. Do not reintroduce a global set of alias names.
- Add a test whenever a new `Expr` variant gains child expressions. An exhaustive helper or test
  that forces a decision for each `Expr` variant would be better than relying on a catch-all arm.
- For every new failure fixture, first demonstrate that it silently folds on the vulnerable code,
  then verify the final error names the intended outer binding.

## Existing coverage

The focused fixtures now cover assertions, nested struct fields, enum payload calls, local aliases,
reborrows, topology/spawn expressions under unknown control flow, value calls through dereference
and index expressions, aggregate callbacks, closure captures, and aggregate borrows. The local
struct and scalar-only borrowed-aggregate pass fixtures guard against stale-provenance false
positives.

That is strong coverage for the known paths, but it is not a proof that every direct evaluator
form is exhaustive. The topology repro above is the reminder to finish the systematic audit.

## Alternate design notes

Your comptime model seems to intentionally allow reaching into a runtime &mut from inside the block (your repro has outside as an ordinary stack var, not comptime-only storage) — so you can't take Zig's move of forbidding the boundary outright. That's fine, but it means you should take Rust/Miri's move instead: collapse the evaluator and the effect checker into one walk. Concretely:

Get rid of ComptimeEffects as a separate static analysis. Instead, make the evaluator itself always the source of truth: it executes (or, for unknown branches, symbolically executes both arms) using ComptimeValueFacts-style provenance as part of the value representation itself — every value the evaluator produces carries its provenance, always, not just when the checker bothers to compute it.
For anything the evaluator can't run concretely (unknown branch, opaque callee), it still walks the expression tree the same way it would to evaluate it, just abstractly — same recursive structure, same node handling, so there is no second copy of "what does Expr::Topology contain" living in a different file. One function, one truth, for "what does this subexpression touch."
Flip the proof obligation the way your guardrails already gesture at: don't try to prove impurity is absent (need-complete-coverage, fails open on omission); prove purity is present (fails closed on omission — an unhandled node type just refuses to fold, which you already do, but currently only because someone remembered to write that arm in both places).

If you do that, the "audit every expression owner" TODO list mostly collapses, because you stop needing FileCheck fixtures to catch the evaluator and checker disagreeing — that failure mode structurally can't occur if there's only one walker.

## Unified comptime evaluation plan (new branch from `main`)

This work replaces the duplicated concrete evaluator and unknown-path effect summary with one
comptime interpreter. The current branch remains available as a reference implementation and
regression corpus. The provenance model and its fixtures are behavioral evidence to preserve, not
a reason to continue extending the two-walker design.

### 1. Evaluation contract — complete

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

#### Evaluation and abstract-control rules

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

#### Diagnostics

If a possible escaping write has known outer origins, reject the block with E3033 and name one
deterministically selected origin. Prefer that diagnostic over a generic inability-to-fold error.
If no escaping write is possible but evaluation is unknown or unsupported, reject the fold with
the existing generic comptime-evaluation diagnostic. Diagnostics are emitted at the block's
existing chosen span and speculative evaluation remains silent.

This is a safety contract, not a promise to symbolically execute all runtime language features.
The proof obligation is positive: folding requires a supported, concrete, effect-safe result.
Anything not proved safe is left unfolded/refused as required by current `comptime` semantics.

> **Implementation status:** §1 is the settled target contract. The current interpreter implements
> only the owner slices recorded below; in particular, bounded-loop execution, full callable and
> closure semantics, aggregate-field precision, and full old/new value parity are still pending.

### 2. Shared state and result model — complete

- [x] Define the comptime-only result/abstract-value types that represent the contract above.
- [x] Define one evaluation context for lexical fact scopes, outer-block bindings, possible
  escaping writes, call/loop guards, and support status. It is not a separate analysis pass.
- [x] Preserve replacement-based scoped facts, place/value distinction, callable targets,
  captured writes, unknown callables, and recursive-call guards.
- [x] Define cloning and merge rules for abstract branch state, including a deterministic merge of
  escaping origins and a conservative rule for incompatible concrete environments.

### 3. Shared interpreter — staged implementation plan

The unit of work is one expression owner (or tightly coupled family), not a wholesale rewrite.
Each unit has an explicit supported or `unsupported` interpreter arm, its child-evaluation order,
and direct/unknown-control-flow fixtures. Do not add a catch-all arm to the core `Expr`,
`Statement`, or child-bearing `Topology` dispatch: a new AST variant must make the compiler fail
to build until the interpreter makes a decision for it.

#### 3.1 Establish the transition harness

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
- [ ] A narrow escaping-write comparator is live: it rejects that one safety disagreement with
  E3033. Extend it to normalized value/support/flow observations and record every unallowlisted
  disagreement as a diagnostic-quality internal failure or focused regression, never as a crash.
- [ ] Maintain an explicit, fixture-backed allow-list of intentional disagreements: a new-path
  rejection is allowed only when the old path demonstrably silently folded away the write.

#### 3.2 Port one expression owner at a time

For every checked item, use a small reviewable change with the acceptance rule below. Do not move
to a later family while its direct and unknown-path fixtures disagree with the required behavior.

- [ ] **Leaves and places — complete only after acceptance**

  - [x] Implement source-order traversal and provenance for identifiers, literals, borrows,
    dereferences, member/index access, assignments, and compound assignments.
  - [x] Preserve evaluated aggregate/member/index facts rather than recovering facts from syntax
    alone.
  - [x] Write the early-unknown index regression fixture.
  - [ ] Add the remaining direct/unknown/local-only place fixtures, including borrowed-place
    coverage.
  - [ ] Run those fixtures and confirm normalized old/new parity.

- [ ] **Pure operators and containers (non-topology) — complete only after acceptance**

  - [x] Implement structural traversal for unary/binary/relational/logical operators, casts,
    ranges, arrays, vectors, struct literals, and enum payloads.
  - [x] Implement concrete scalar arithmetic/relations and concrete arrays.
  - [x] Write the early-unknown binary-left/effectful-right regression fixture.
  - [ ] Implement or deliberately reject with focused fixtures aggregate values, casts, ranges,
    vectors, and enums.
  - [ ] Add direct/unknown/local-only fixtures and confirm normalized old/new parity.

- [ ] **Topology-bearing predicates — complete only after acceptance**

  - [x] Traverse every topology child and query concrete `TransferPredicate` reachability when
    both topologies are known.
  - [ ] Model dynamic topology-index values precisely enough for predicate results, or retain a
    tested fail-closed rule.
  - [ ] Add direct/unknown/local-only predicate fixtures, including the early-unknown child case.
  - [ ] Run fixtures and confirm normalized old/new parity.

- [ ] **Topology and placement owners — complete only after acceptance**

  - [x] Traverse `Topology` (NPU/GPU/AccCore and nested `Slice` indices), `Transfer`, `SpawnOn`,
    and `InlineMlir`; record MLIR clobber-place writes.
  - [x] Write the direct topology-index regression fixture and retain the imported unknown-path
    topology/spawn fixtures.
  - [ ] Define concrete semantics, or fixture-backed fail-closed behavior, for `Transfer`,
    `SpawnOn`, and MLIR values.
  - [ ] Add remaining local-only/early-unknown placement fixtures and confirm parity.

- [ ] **Blocks and control flow — complete only after acceptance**

  - [x] Implement lexical concrete-environment restoration for blocks and tail expressions.
  - [x] Implement known/unknown `if`, known-scalar/abstract `match`, and abstract `for`/`loop`
    state handling; preserve `return`, `break`, and `continue` flow.
  - [x] Retain imported unknown-branch, match, loop, return, and shadowing fixtures.
  - [ ] Implement or fixture-back fail-closed bounded-loop recurrence and enum-pattern precision.
  - [ ] Add any missing direct/local-only control-flow fixtures and confirm parity.

- [ ] **Calls and closures — complete only after acceptance**

  - [x] Implement known direct calls with mutable-parameter provenance, recursion rejection, and
    `MAX_CALL_DEPTH` enforcement.
  - [x] Carry named function values and known targets through locals, aggregates, members, indexes,
    and indirect-call syntax.
  - [x] Represent lowered `Closure_N` values as deferred generated targets with captured
    environments; detect writes through captured references and direct captured-scalar assignment.
  - [x] Write the direct captured-closure-assignment regression and retain imported function-value,
    aggregate-callback, and closure fixtures.
  - [ ] Model unlowered `Expr::Closure`, remaining closure/capture shapes, and opaque-call behavior
    with focused local-only cases.
  - [ ] Run the call/closure fixtures and confirm normalized old/new parity.

- [ ] **Peripheral forms — complete only after acceptance**

  - [x] Give autodiff, print/println, macros, `sizeof`, memory-space expressions, and all other
    variants explicit child traversal or explicit leaf rejection.
  - [x] Keep every peripheral form fail-closed pending concrete semantics.
  - [ ] Decide and test concrete semantics versus permanent rejection for each form.
  - [ ] Add fixtures and confirm normalized old/new parity.

#### 3.3 Required acceptance shape for each owner

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

- Concrete environment scope restoration is now implemented for block declarations, including
  tail expressions. It still needs shadowing/local-only fixture coverage before the control-flow
  owner can be accepted.
- Lowered closure values now defer their generated body and carry target/environment facts into
  `Closure_N_call`. An unlowered `Expr::Closure` is explicitly unsupported without running its
  body. Captured-place facts now preserve direct captured-scalar writes; the focused fixture still
  needs execution and parity verification.
- Named function values and lowered closure values populate `ComptimeValueFacts::callable_targets`.
  The latter also stores target-specific environment facts, so direct/indirect calls can interpret
  each known target with its generated `_env` argument. Opaque calls only reject conservatively
  when existing captured-write facts name an outer binding.
- The live comparator checks escaping writes only. It does not compare concrete value, support,
  or flow, and there is no allow-list yet.

#### 3.4 Cut over only after parity

- [ ] Keep dual-run enabled until every owner above has completed its fixture set and the full
  imported corpus has no unallowlisted disagreement.
- [ ] Switch `fold_comptime_block` to the new interpreter as the sole source of the fold verdict.
- [ ] Delete the old fold-specific evaluator path and the name-based `escaping_write` scan in one
  final mechanical cleanup change; do not leave a fallback that can silently choose the old path.
- [ ] Remove transitional comparison code, obsolete state, and comments only after the cutover
  suite is green.

### 4. Migration sequencing

- [x] Add the three structural regressions: the direct topology-index write, an unknown base with
  an effectful index/operand, and a call with an earlier unknown plus a later effectful argument.
- [ ] Land owner changes in the order in §3.2 unless a failing fixture establishes a dependency.
- [ ] Keep each owner change independently reviewable and bisectable; do not combine unrelated
  owners merely to make a broad test suite pass.

### 5. Regression and acceptance

- [x] Import and retain the existing provenance, aggregate, closure, reborrow, unknown-control-flow, and
  topology/spawn fixtures.
- [x] Add the direct `Topology::NPU[touch(&mut outside)]` failure fixture.
- [ ] Add direct and unknown-control-flow pairs for compound expressions to validate shared
  interpreter semantics.
- [x] Add early-unknown-child cases with a later outer-reference write for indexing, calls, and
  operators.
- [ ] Add the remaining early-unknown-child cases for borrows/places and topology-bearing
  predicates.
- [ ] Cover the reference/container shapes already listed in this note, with distinct outer
  binding names in failures and local-only pass counterparts where folding is supported.
- [ ] Cover known/unknown branches, match arms, loops, returns, recursion limits, opaque calls,
  closures/captures, reassignment, shadowing, and aggregate transport.
- [x] Maintain exhaustive `Expr`, `Statement`, and child-bearing `Topology` dispatch in the
  shared interpreter: adding a variant requires an explicit compiler-checked arm. Do not add a
  wildcard arm.
- [ ] Run focused `comptime*.vx` pass/fail tests, `cargo test --lib`, and the applicable full
  frontend regression suite. **Current blocker:** this workspace cannot build `vxc` because
  `llvm-config` is absent from `PATH`; formatting and diff checks pass, but no Rust or frontend
  test has run in this environment.

### Completion criteria

- [ ] No standalone static mutable-effect traversal can disagree with the evaluator about an
  expression's children.
- [ ] A fold requires a concrete, supported result and no possible escaping write.
- [ ] Direct topology and early-unknown-child writes cannot silently fold away. The live safety
  comparator and focused fixtures are present; execution verification is blocked on LLVM.
- [ ] Intended folds and local-only provenance cases still pass.
- [ ] This branch remains independently reviewable against `main`.
