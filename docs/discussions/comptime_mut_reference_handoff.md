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

## TODO: unified comptime evaluation design (new branch from `main`)

Do this work on a new branch based on `main`. Keep the current branch intact as the reference
implementation and regression corpus; do not merge its implementation as a prerequisite for this
design. The existing fixtures and the cases in this note are the behavioral evidence to preserve,
not a request to continue extending the two-walker implementation.

### 1. Establish the contract before implementation

- [ ] Write down the fold contract: a `comptime` block folds only when every executed or
  potentially executed operation is modelled, its result is concrete, and no write can reach a
  binding outside the block.
- [ ] Specify the result states separately: concrete value, unknown value, unsupported operation,
  normal/return/break/continue flow, and escaping-write diagnostic information. Do not use one
  `Option<Value>` to mean all of “unknown”, “unsupported”, and “no value”.
- [ ] Specify evaluation order for every compound expression, including the rule that children
  still have to be visited when an earlier child makes the enclosing value unknown.
- [ ] Specify abstract control flow: execute the chosen arm for a known condition; fork and merge
  all viable arms for an unknown condition; conservatively refuse folding for an unknown loop or
  opaque recursive path unless its effects can be represented safely.
- [ ] Decide and document the diagnostic policy for a possible outer write versus a generic
  inability to fold. Preserve the outer binding name whenever provenance identifies one.

### 2. Define the shared evaluator state

- [ ] Keep `Value` as the ordinary concrete-value representation. Introduce a comptime-only
  wrapper (for example, an evaluation result/value) that carries an optional concrete `Value` and
  `ComptimeValueFacts`.
- [ ] Replace the standalone `ComptimeEffects` *analysis pass* with one evaluation context that
  owns lexical fact scopes, the outer-block boundary, escaping-write state, and recursion/loop
  guards. It may retain much of the current data model; the requirement is one traversal, not a
  particular struct name.
- [ ] Keep value facts scoped and replacement-based: declaration installs facts, assignment
  replaces the nearest binding’s facts, and leaving a scope drops them.
- [ ] Keep places distinct from values: a mutable borrow/store needs storage-place provenance,
  while a dereference uses the provenance carried by the referenced value.
- [ ] Preserve callable target, captured-write, and unknown-callable facts, including the existing
  conservative handling of callable values and recursive/mutually recursive calls.

### 3. Build one recursive interpreter

- [ ] Create one internal expression/block/statement interpreter that returns the shared result
  and is used for both concrete and abstract evaluation.
- [ ] Make every `Expr` variant explicit in that interpreter; avoid a catch-all arm that can
  silently make a newly added AST form foldable. Apply the same discipline to nested expression
  owners such as `Topology`.
- [ ] Encode the language’s child-evaluation order once in that interpreter. In particular cover
  topology indices, array/vector elements, index base/index, call callee/arguments, borrows and
  places, aggregate fields/payloads, casts/operators, and topology-bearing predicates.
- [ ] Implement known and unknown `if`/`match` through the same interpreter, with branch state
  cloning and conservative merges rather than a separate effect-summary walk.
- [ ] Route direct calls, indirect calls, closures, aggregate-held callbacks, and known callee
  bodies through the same call mechanism. For unavailable callees, produce an abstract/unsupported
  result and retain any possible outer write from arguments or captures.
- [ ] Route statement-position expressions, expression-position blocks, `unsafe`, `spawn on`,
  `return`, `break`, `continue`, and supported loops through the same flow representation.
- [ ] Make unsupported syntax fail closed: it may prevent a fold, but it must never hide a child
  effect or leave a partially evaluated environment as if it were certain.

### 4. Migrate without losing behavior

- [ ] First prove the shared interpreter on small representative cases: the direct topology-index
  repro, an unknown base with an effectful index/operand, and a call with an earlier unknown but a
  later effectful argument.
- [ ] Migrate existing concrete arithmetic, comparisons, arrays, indexing, calls, blocks, and
  loops one family at a time, retaining their current precision where it is sound.
- [ ] Compare the new path against the current branch’s focused pass/fail fixtures. Preserve
  accepted local-only cases as well as rejected outer-write cases.
- [ ] Remove `comptime_expr_may_write`, `comptime_block_may_write`, and related duplicate summary
  machinery only after their coverage has moved to the shared interpreter.
- [ ] Remove obsolete state and comments only after the old path is no longer reachable.

### 5. Regression and acceptance suite

- [ ] Keep the existing provenance, aggregate, closure, reborrow, unknown-control-flow, and
  topology/spawn fixtures as regression tests.
- [ ] Add the direct `Topology::NPU[touch(&mut outside)]` failure case from this note.
- [ ] Add direct and unknown-control-flow pairs for each compound-expression family, but use them
  to validate interpreter semantics rather than to maintain two traversals in lockstep.
- [ ] Add cases where an early child is unknown and a later child writes through an outer mutable
  reference; cover indexing, calls, operators, borrows/places, and topology-bearing predicates.
- [ ] Cover all reference/container shapes listed above, each with a distinct outer binding in
  failure tests and a local-only pass counterpart where folding is supported.
- [ ] Cover known/unknown branches, match arms, loops, returns, recursion limits, opaque calls,
  closures/captures, reassignment, shadowing, and aggregate transport.
- [ ] Add a maintainability guardrail: adding an `Expr` variant or a child-bearing `Topology`
  variant must require an explicit shared-interpreter decision at compile time or in a dedicated
  exhaustive test.
- [ ] Run the focused `comptime*.vx` pass/fail suite and `cargo test --lib`, then the appropriate
  full frontend regression suite before proposing the new branch for review.

### Completion criteria

- [ ] There is no separate static mutable-effect traversal whose expression ownership rules can
  diverge from evaluation.
- [ ] A fold is accepted only with a concrete, supported result and no possible escaping write.
- [ ] The direct topology repro and the early-unknown-child cases cannot silently fold away a
  write.
- [ ] Existing intended folds and local-only provenance cases continue to pass.
- [ ] The new branch is independently reviewable against `main`; the current branch remains
  available for comparison or rollback.
