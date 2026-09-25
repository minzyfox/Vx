//! The transition harness for the unified `comptime` interpreter.
//!
//! This is deliberately separate from ordinary `eval_expr`: it is the seam where comptime-only
//! value facts, support, flow, and possible escaping writes meet. Expression-owner work adds
//! concrete semantics here one family at a time; the exhaustive dispatch below makes omission a
//! compile error instead of a silently pure catch-all.

use std::collections::HashMap;

use crate::arch::TransferCostGraph;
use crate::hir::check_state::{
    ComptimeEvalContext, ComptimeEvalFlow, ComptimeEvalOutcome, ComptimeEvalSupport,
    ComptimeEvalValue, ComptimeValueFacts,
};
use crate::hir::env::Value;
use crate::symbol::Symbol;
use crate::syntax::*;

/// The comparison-friendly result of one shadow interpretation.
#[derive(Clone, PartialEq)]
pub(crate) struct ComptimeObservation {
    pub outcome: ComptimeEvalOutcome,
    pub escaping_write: Option<Symbol>,
}

/// A comptime-only interpreter kept beside the legacy evaluator during migration.
#[derive(Clone)]
pub(crate) struct ComptimeInterpreter<'graph> {
    env: HashMap<Symbol, ComptimeEvalValue>,
    function_bodies: HashMap<Symbol, Function>,
    transfer_cost_graph: &'graph TransferCostGraph,
    context: ComptimeEvalContext,
}

impl<'graph> ComptimeInterpreter<'graph> {
    pub(crate) fn new(
        env: HashMap<Symbol, Value>,
        function_bodies: HashMap<Symbol, Function>,
        transfer_cost_graph: &'graph TransferCostGraph,
        context: ComptimeEvalContext,
    ) -> Self {
        Self {
            env: env
                .into_iter()
                .map(|(name, value)| (name, ComptimeEvalValue::known(value)))
                .collect(),
            function_bodies,
            transfer_cost_graph,
            context,
        }
    }

    pub(crate) fn observe_block(
        &mut self,
        stmts: &[Statement],
        tail: Option<&Expr>,
    ) -> ComptimeObservation {
        let outcome = self.block_with_tail(stmts, tail);
        ComptimeObservation {
            outcome,
            escaping_write: self.context.escaping_write().cloned(),
        }
    }

    fn unsupported_after(
        &mut self,
        children: impl IntoIterator<Item = ComptimeEvalOutcome>,
    ) -> ComptimeEvalOutcome {
        let mut outcome = ComptimeEvalOutcome::unsupported();
        for child in children {
            outcome.value.facts.merge_from(&child.value.facts);
            outcome.support.merge_from(child.support);
        }
        outcome.support = ComptimeEvalSupport::Unsupported;
        outcome
    }

    /// Sequence children in evaluation order. Their values need not agree, but their may-facts
    /// and unsupported status must survive even when the enclosing value becomes unknown.
    fn unknown_after(
        &mut self,
        children: impl IntoIterator<Item = ComptimeEvalOutcome>,
    ) -> ComptimeEvalOutcome {
        let mut outcome = ComptimeEvalOutcome::unknown();
        for child in children {
            outcome.value.facts.merge_from(&child.value.facts);
            outcome.support.merge_from(child.support);
            if child.flow != ComptimeEvalFlow::Normal {
                outcome.flow = child.flow;
            }
        }
        outcome
    }

    fn value_facts(&self, expr: &Expr) -> ComptimeValueFacts {
        match expr {
            Expr::Identifier(id) => self.identifier_facts(&id.name),
            Expr::Borrow(borrow) => self.value_facts(&borrow.expr),
            Expr::Dereference(deref) => self.value_facts(&deref.expr),
            Expr::MemberAccess(access) => self.value_facts(&access.base),
            Expr::IndexAccess(access) => {
                let mut facts = self.value_facts(&access.base);
                facts.merge_from(&self.value_facts(&access.index));
                facts
            }
            _ => ComptimeValueFacts::default(),
        }
    }

    fn identifier_facts(&self, name: &Symbol) -> ComptimeValueFacts {
        self.context
            .binding_facts(name)
            .cloned()
            .unwrap_or_else(|| {
                let mut facts = ComptimeValueFacts::default();
                if self.context.outer_reference_binding(name) {
                    facts.reference_origins.insert(name.clone());
                }
                if self.context.outer_callable_binding(name) {
                    // The closure's environment is outside the disappearing comptime block. Its
                    // body is unavailable here, so invoking it may write anything it captured.
                    facts.captured_writes.insert(name.clone());
                    facts.unknown_callable = true;
                }
                if self.function_bodies.contains_key(name) {
                    facts.callable_targets.insert(name.clone());
                }
                facts
            })
    }

    /// Facts for the storage a write or mutable borrow reaches, distinct from the value held in
    /// that storage. Dereferencing an alias reaches its carried reference origins; selecting a
    /// field or index remains in the base storage.
    fn place_facts(&self, expr: &Expr) -> ComptimeValueFacts {
        match expr {
            Expr::Identifier(id) => {
                if let Some(binding) = self.context.binding_facts(&id.name) {
                    ComptimeValueFacts {
                        reference_origins: binding.captured_place_origins.clone(),
                        ..ComptimeValueFacts::default()
                    }
                } else if self.context.is_outer_binding(&id.name) {
                    let mut facts = ComptimeValueFacts::default();
                    facts.reference_origins.insert(id.name.clone());
                    facts
                } else {
                    ComptimeValueFacts::default()
                }
            }
            Expr::MemberAccess(access) => self.place_facts(&access.base),
            Expr::IndexAccess(access) => self.place_facts(&access.base),
            Expr::Dereference(deref) => self.value_facts(&deref.expr),
            _ => self.value_facts(expr),
        }
    }

    fn block(&mut self, stmts: &[Statement]) -> ComptimeEvalOutcome {
        self.block_with_tail(stmts, None)
    }

    /// Interpret a lexical block, keeping declarations available to its tail expression and
    /// restoring concrete bindings shadowed by the block when it exits. Provenance already had
    /// this discipline in `ComptimeEvalContext`; the value environment must mirror it or a local
    /// `let` in an `if`/`match` arm can leak a stale constant into the enclosing path.
    fn block_with_tail(&mut self, stmts: &[Statement], tail: Option<&Expr>) -> ComptimeEvalOutcome {
        let mut shadowed = HashMap::new();
        for stmt in stmts {
            if let Statement::LetDecl(decl) = stmt {
                shadowed
                    .entry(decl.name.clone())
                    .or_insert_with(|| self.env.get(&decl.name).cloned());
            }
        }

        self.context.push_scope();
        let mut result = ComptimeEvalOutcome::unknown();
        for stmt in stmts {
            result = self.statement(stmt);
            if result.flow != ComptimeEvalFlow::Normal {
                break;
            }
        }
        if result.flow == ComptimeEvalFlow::Normal {
            if let Some(tail) = tail {
                result = self.expr(tail);
            }
        }
        self.context.pop_scope();
        for (name, value) in shadowed {
            match value {
                Some(value) => {
                    self.env.insert(name, value);
                }
                None => {
                    self.env.remove(&name);
                }
            }
        }
        result
    }

    fn statement(&mut self, stmt: &Statement) -> ComptimeEvalOutcome {
        match stmt {
            Statement::LetDecl(let_decl) => {
                let outcome = self.expr(&let_decl.expr);
                self.context
                    .declare(let_decl.name.clone(), outcome.value.facts.clone());
                self.env
                    .insert(let_decl.name.clone(), outcome.value.clone());
                outcome
            }
            Statement::Return(ret) => {
                let mut outcome = ret
                    .expr
                    .as_ref()
                    .map(|expr| self.expr(expr))
                    .unwrap_or_default();
                outcome.flow = ComptimeEvalFlow::Return;
                outcome
            }
            Statement::ExprStmt(expr) => self.expr(&expr.expr),
            Statement::ForLoop(loop_stmt) => self.for_loop(loop_stmt),
            Statement::Assign(assign) => self.assign(&assign.lhs, &assign.rhs),
            Statement::CompoundAssign(assign) => self.compound_assign(&assign.lhs, &assign.rhs),
            Statement::Assert(assert) => self.expr(&assert.expr),
            Statement::Loop(loop_stmt) => self.loop_stmt(loop_stmt),
            Statement::Break(_) => ComptimeEvalOutcome {
                flow: ComptimeEvalFlow::Break,
                ..ComptimeEvalOutcome::default()
            },
            Statement::Continue(_) => ComptimeEvalOutcome {
                flow: ComptimeEvalFlow::Continue,
                ..ComptimeEvalOutcome::default()
            },
            Statement::MacroCall(_) | Statement::Error(_) => ComptimeEvalOutcome::unsupported(),
        }
    }

    fn assign(&mut self, lhs: &Expr, rhs: &Expr) -> ComptimeEvalOutcome {
        let left = self.expr(lhs);
        let right = self.expr(rhs);
        if let Expr::Identifier(id) = lhs {
            if self.context.is_outer_binding(&id.name) {
                self.context.note_escaping_write([id.name.clone()]);
            } else if let Some(captured_place) = self
                .context
                .binding_facts(&id.name)
                .map(|binding| binding.captured_place_origins.clone())
            {
                self.context
                    .note_escaping_write(captured_place.iter().cloned());
                if captured_place.is_empty() {
                    self.context.reassign(&id.name, right.value.facts.clone());
                    self.env.insert(id.name.clone(), right.value.clone());
                }
            } else {
                self.context.reassign(&id.name, right.value.facts.clone());
                self.env.insert(id.name.clone(), right.value.clone());
            }
        } else {
            self.context
                .note_escaping_write(self.place_facts(lhs).reference_origins);
        }
        self.unknown_after([left, right])
    }

    fn compound_assign(&mut self, lhs: &Expr, rhs: &Expr) -> ComptimeEvalOutcome {
        let left = self.expr(lhs);
        let right = self.expr(rhs);
        if let Expr::Identifier(id) = lhs {
            if self.context.is_outer_binding(&id.name) {
                self.context.note_escaping_write([id.name.clone()]);
            } else if let Some(captured_place) = self
                .context
                .binding_facts(&id.name)
                .map(|binding| binding.captured_place_origins.clone())
            {
                self.context
                    .note_escaping_write(captured_place.iter().cloned());
            }
        } else {
            self.context
                .note_escaping_write(self.place_facts(lhs).reference_origins);
        }
        self.unknown_after([left, right])
    }

    fn binary(
        &mut self,
        lhs: ComptimeEvalOutcome,
        rhs: ComptimeEvalOutcome,
        op: &BinaryOp,
    ) -> ComptimeEvalOutcome {
        let concrete = match (&lhs.value.concrete, &rhs.value.concrete) {
            (Some(Value::Int(a)), Some(Value::Int(b))) => match op {
                BinaryOp::Add => a.checked_add(*b).map(Value::Int),
                BinaryOp::Sub => a.checked_sub(*b).map(Value::Int),
                BinaryOp::Mul => a.checked_mul(*b).map(Value::Int),
                BinaryOp::Div => a.checked_div(*b).map(Value::Int),
                BinaryOp::Rem => a.checked_rem(*b).map(Value::Int),
                _ => None,
            },
            (Some(left), Some(right)) => match (left.as_f64(), right.as_f64(), op) {
                (Some(a), Some(b), BinaryOp::Add) => Some(Value::Number(a + b)),
                (Some(a), Some(b), BinaryOp::Sub) => Some(Value::Number(a - b)),
                (Some(a), Some(b), BinaryOp::Mul) => Some(Value::Number(a * b)),
                (Some(a), Some(b), BinaryOp::Div) => Some(Value::Number(a / b)),
                (Some(a), Some(b), BinaryOp::Rem) if b != 0.0 => Some(Value::Number(a % b)),
                _ => None,
            },
            _ => None,
        };
        let mut outcome = self.unknown_after([lhs, rhs]);
        outcome.value.concrete = concrete;
        outcome
    }

    fn array(&mut self, elements: Vec<ComptimeEvalOutcome>) -> ComptimeEvalOutcome {
        let concrete = elements
            .iter()
            .map(|outcome| outcome.value.concrete.clone())
            .collect::<Option<Vec<_>>>()
            .map(Value::Array);
        let mut outcome = self.unknown_after(elements);
        outcome.value.concrete = concrete;
        outcome
    }

    /// Run a direct call whose body is available to comptime evaluation. The call gets a fresh
    /// value environment, but its parameter facts are installed in the context's enclosing call
    /// scope, so `*param = ...` is recognised as a write through the caller's mutable reference.
    ///
    /// Calls without a body, arity mismatch, or recursion are deliberately unsupported during the
    /// transition. Their arguments have still been evaluated before this point, preserving any
    /// effect that occurs while producing an argument.
    fn known_function_call(
        &mut self,
        target: &Symbol,
        args: Vec<ComptimeEvalOutcome>,
    ) -> ComptimeEvalOutcome {
        let Some(function) = self.function_bodies.get(target).cloned() else {
            return self.unsupported_after(args);
        };
        if function.params.len() != args.len() || !self.context.push_call(target.clone()) {
            return self.unsupported_after(args);
        }

        let saved_env = std::mem::take(&mut self.env);
        self.context.push_scope();
        for ((parameter, _), argument) in function.params.iter().zip(&args) {
            self.context
                .declare(parameter.clone(), argument.value.facts.clone());
            self.env.insert(parameter.clone(), argument.value.clone());
        }
        let body = self.block(&function.body);
        self.context.pop_scope();
        self.env = saved_env;
        self.context.pop_call(target);

        let mut outcome = match body.flow {
            ComptimeEvalFlow::Return => ComptimeEvalOutcome {
                flow: ComptimeEvalFlow::Normal,
                ..body
            },
            ComptimeEvalFlow::Normal => ComptimeEvalOutcome::unknown(),
            _ => ComptimeEvalOutcome::unsupported(),
        };
        for argument in args {
            outcome.support.merge_from(argument.support);
        }
        outcome
    }

    fn callable_call(
        &mut self,
        facts: ComptimeValueFacts,
        args: Vec<ComptimeEvalOutcome>,
    ) -> ComptimeEvalOutcome {
        self.note_opaque_callable(&facts);
        let mut targets = facts.callable_targets.iter();
        let Some(first_target) = targets.next() else {
            return self.unsupported_after(args);
        };

        let mut merged = self.clone();
        let mut first_args = args.clone();
        if let Some(environment) = facts.callable_environments.get(first_target) {
            first_args.insert(
                0,
                ComptimeEvalOutcome {
                    value: ComptimeEvalValue {
                        concrete: None,
                        facts: environment.clone(),
                    },
                    ..ComptimeEvalOutcome::default()
                },
            );
        }
        let mut outcome = merged.known_function_call(first_target, first_args);
        for target in targets {
            let mut branch = self.clone();
            let mut branch_args = args.clone();
            if let Some(environment) = facts.callable_environments.get(target) {
                branch_args.insert(
                    0,
                    ComptimeEvalOutcome {
                        value: ComptimeEvalValue {
                            concrete: None,
                            facts: environment.clone(),
                        },
                        ..ComptimeEvalOutcome::default()
                    },
                );
            }
            let branch_outcome = branch.known_function_call(target, branch_args);
            merged.context.merge_branch(&branch.context);
            merged.env = Self::merge_environments(merged.env, &branch.env);
            outcome.merge_from(&branch_outcome);
        }
        self.context = merged.context;
        self.env = merged.env;
        outcome
    }

    fn function_call(
        &mut self,
        call: &FunctionCallExpr,
        args: Vec<ComptimeEvalOutcome>,
    ) -> ComptimeEvalOutcome {
        if self.function_bodies.contains_key(&call.name) {
            self.known_function_call(&call.name, args)
        } else {
            self.callable_call(self.identifier_facts(&call.name), args)
        }
    }

    /// An opaque callable cannot be assumed pure. Captured writes name outer storage directly;
    /// recording them means an indirect/callback call is conservatively refused rather than being
    /// folded away just because the body was not available to this transition interpreter.
    fn note_opaque_callable(&mut self, facts: &ComptimeValueFacts) {
        if facts.unknown_callable {
            self.context
                .note_escaping_write(facts.captured_writes.iter().cloned());
        }
    }

    /// Join the variable environment after an abstract branch. Branch-local declarations are
    /// absent from at least one side and therefore disappear; bindings that exist on both paths
    /// retain only a concrete value both paths agree on, while their provenance is a may-fact.
    fn merge_environments(
        mut left: HashMap<Symbol, ComptimeEvalValue>,
        right: &HashMap<Symbol, ComptimeEvalValue>,
    ) -> HashMap<Symbol, ComptimeEvalValue> {
        left.retain(|name, value| {
            let Some(other) = right.get(name) else {
                return false;
            };
            value.merge_from(other);
            true
        });
        left
    }

    fn if_expr(&mut self, if_expr: &IfExpr) -> ComptimeEvalOutcome {
        let condition = self.expr(&if_expr.cond);
        if condition.flow != ComptimeEvalFlow::Normal {
            return condition;
        }

        match condition.value.concrete {
            Some(Value::Bool(true)) => {
                let mut outcome = self.block(&if_expr.then_block);
                outcome.support.merge_from(condition.support);
                return outcome;
            }
            Some(Value::Bool(false)) => {
                let mut outcome = if_expr
                    .else_block
                    .as_ref()
                    .map(|branch| self.block(branch))
                    .unwrap_or_default();
                outcome.support.merge_from(condition.support);
                return outcome;
            }
            _ => {}
        }

        // An unknown condition has two viable paths. Each starts from the state after evaluating
        // the condition, then the states join: effects and reference origins union, while a
        // concrete value or binding survives only when both paths establish the same one.
        let mut then_interpreter = self.clone();
        let then_outcome = then_interpreter.block(&if_expr.then_block);
        let mut else_interpreter = self.clone();
        let else_outcome = if_expr
            .else_block
            .as_ref()
            .map(|branch| else_interpreter.block(branch))
            .unwrap_or_default();

        then_interpreter
            .context
            .merge_branch(&else_interpreter.context);
        self.context = then_interpreter.context;
        self.env = Self::merge_environments(then_interpreter.env, &else_interpreter.env);

        let mut outcome = then_outcome;
        outcome.merge_from(&else_outcome);
        outcome.support.merge_from(condition.support);
        outcome
    }

    fn pattern_matches(pattern: &Pattern, value: &Value) -> Option<bool> {
        match pattern {
            Pattern::Wildcard | Pattern::Identifier(_) => Some(true),
            Pattern::Literal(Expr::Identifier(id)) if id.name.as_ref() == "true" => {
                Some(matches!(value, Value::Bool(true)))
            }
            Pattern::Literal(Expr::Identifier(id)) if id.name.as_ref() == "false" => {
                Some(matches!(value, Value::Bool(false)))
            }
            Pattern::Literal(Expr::Number(number)) => match value {
                Value::Int(value) => number
                    .value
                    .parse::<i64>()
                    .ok()
                    .map(|other| value == &other),
                Value::Number(value) => number
                    .value
                    .parse::<f64>()
                    .ok()
                    .map(|other| value == &other),
                _ => Some(false),
            },
            Pattern::Literal(_) | Pattern::EnumVariant(..) => Some(false),
        }
    }

    fn bind_pattern_facts(&mut self, pattern: &Pattern, facts: &ComptimeValueFacts) {
        match pattern {
            Pattern::Identifier(name) => self.context.declare(name.clone(), facts.clone()),
            Pattern::EnumVariant(_, _, Some(payloads)) => {
                for payload in payloads {
                    // Aggregate field precision is an owner of its own. Until it is modelled,
                    // every payload binding receives the scrutinee's may-facts rather than
                    // dropping a possible reference origin on the floor.
                    self.bind_pattern_facts(payload, facts);
                }
            }
            Pattern::Wildcard | Pattern::Literal(_) | Pattern::EnumVariant(_, _, None) => {}
        }
    }

    fn match_arm(
        &mut self,
        arm: &MatchArm,
        scrutinee_facts: &ComptimeValueFacts,
    ) -> ComptimeEvalOutcome {
        self.context.push_scope();
        self.bind_pattern_facts(&arm.pattern, scrutinee_facts);
        let outcome = self.block(&arm.body);
        self.context.pop_scope();
        outcome
    }

    fn match_expr(&mut self, match_expr: &MatchExpr) -> ComptimeEvalOutcome {
        let scrutinee = self.expr(&match_expr.expr);
        if scrutinee.flow != ComptimeEvalFlow::Normal {
            return scrutinee;
        }

        let candidates: Vec<&MatchArm> = if let Some(value) = scrutinee.value.concrete.as_ref() {
            let mut matching = Vec::new();
            for arm in &match_expr.arms {
                match Self::pattern_matches(&arm.pattern, value) {
                    Some(true) => {
                        matching.push(arm);
                        break;
                    }
                    Some(false) => {}
                    None => return ComptimeEvalOutcome::unsupported(),
                }
            }
            matching
        } else {
            let mut viable = Vec::new();
            for arm in &match_expr.arms {
                viable.push(arm);
                if matches!(arm.pattern, Pattern::Wildcard | Pattern::Identifier(_)) {
                    break;
                }
            }
            viable
        };
        let Some((first, rest)) = candidates.split_first() else {
            return ComptimeEvalOutcome::unsupported();
        };

        let scrutinee_known = scrutinee.value.concrete.is_some();
        let scrutinee_support = scrutinee.support;
        let scrutinee_facts = scrutinee.value.facts;
        let mut merged = self.clone();
        let mut outcome = merged.match_arm(first, &scrutinee_facts);
        for arm in rest {
            let mut branch = self.clone();
            let branch_outcome = branch.match_arm(arm, &scrutinee_facts);
            merged.context.merge_branch(&branch.context);
            merged.env = Self::merge_environments(merged.env, &branch.env);
            outcome.merge_from(&branch_outcome);
        }
        self.context = merged.context;
        self.env = merged.env;

        // Known scalar patterns have a concrete selected arm. Unknown and enum-pattern cases
        // still need pattern/value modelling before their value can fold, but their effects have
        // been collected above, so fail closed rather than forgetting an arm's write.
        if !scrutinee_known {
            outcome.support = ComptimeEvalSupport::Unsupported;
        }
        outcome.support.merge_from(scrutinee_support);
        outcome
    }

    fn for_loop(&mut self, loop_stmt: &ForLoopStmt) -> ComptimeEvalOutcome {
        let iterable = self.expr(&loop_stmt.iterable);
        if iterable.flow != ComptimeEvalFlow::Normal {
            return iterable;
        }

        // The iterable may be empty, so join the state after zero iterations with the state after
        // one abstract iteration. That retains writes through outer references while preventing
        // the loop variable and body-local declarations from escaping the loop.
        let mut one_iteration = self.clone();
        one_iteration.context.push_scope();
        one_iteration
            .context
            .declare(loop_stmt.iter.clone(), ComptimeValueFacts::default());
        let _body = one_iteration.block(&loop_stmt.body);
        one_iteration.context.pop_scope();

        self.context.merge_branch(&one_iteration.context);
        self.env = Self::merge_environments(self.env.clone(), &one_iteration.env);
        let mut outcome = ComptimeEvalOutcome::unsupported();
        outcome.support.merge_from(iterable.support);
        outcome
    }

    fn loop_stmt(&mut self, loop_stmt: &LoopStmt) -> ComptimeEvalOutcome {
        // A plain `loop` enters its body at least once. Retain that iteration's effects, but
        // refuse to fold until recurrence and break-path reasoning are modelled. A definite
        // return remains a return; a definite break permits the following statement to run.
        let body = self.block(&loop_stmt.body);
        let mut outcome = ComptimeEvalOutcome::unsupported();
        outcome.flow = match body.flow {
            ComptimeEvalFlow::Return => ComptimeEvalFlow::Return,
            ComptimeEvalFlow::Break => ComptimeEvalFlow::Normal,
            ComptimeEvalFlow::Normal
            | ComptimeEvalFlow::Continue
            | ComptimeEvalFlow::Indeterminate => ComptimeEvalFlow::Indeterminate,
        };
        outcome
    }

    fn expr(&mut self, expr: &Expr) -> ComptimeEvalOutcome {
        match expr {
            Expr::Identifier(id) if id.name.as_ref() == "true" => {
                ComptimeEvalOutcome::known(Value::Bool(true))
            }
            Expr::Identifier(id) if id.name.as_ref() == "false" => {
                ComptimeEvalOutcome::known(Value::Bool(false))
            }
            Expr::Identifier(id) => {
                let mut value = self.env.get(&id.name).cloned().unwrap_or_default();
                value.facts.merge_from(&self.value_facts(expr));
                ComptimeEvalOutcome {
                    value,
                    ..Default::default()
                }
            }
            Expr::Number(number) => number
                .value
                .parse::<i64>()
                .map(Value::Int)
                .or_else(|_| number.value.parse::<f64>().map(Value::Number))
                .map(ComptimeEvalOutcome::known)
                .unwrap_or_else(|_| ComptimeEvalOutcome::unsupported()),
            Expr::EnumVariant(variant) => self.unknown_after(
                variant
                    .payload
                    .iter()
                    .flatten()
                    .map(|value| self.expr(value))
                    .collect::<Vec<_>>(),
            ),
            Expr::StringLiteral(_)
            | Expr::MemorySpace(_)
            | Expr::SizeOf(_)
            | Expr::MacroCall(_) => ComptimeEvalOutcome::unsupported(),
            Expr::Transfer(transfer) => self.unsupported_after([self.expr(&transfer.expr)]),
            Expr::TransferPredicate(predicate) => {
                let from = self.topology(&predicate.from);
                let to = self.topology(&predicate.to);
                let concrete = match (&from.value.concrete, &to.value.concrete) {
                    (Some(Value::Topology(from)), Some(Value::Topology(to))) => {
                        let from_memory = self.transfer_cost_graph.default_memory_for(from);
                        let to_memory = self.transfer_cost_graph.default_memory_for(to);
                        Some(Value::Bool(
                            self.transfer_cost_graph
                                .transfer_path(&from_memory, &to_memory)
                                .is_some(),
                        ))
                    }
                    _ => None,
                };
                let mut outcome = self.unknown_after([from, to]);
                outcome.value.concrete = concrete;
                outcome
            }
            Expr::FunctionCall(call) => {
                self.function_call(call, call.args.iter().map(|arg| self.expr(arg)).collect())
            }
            Expr::IndirectCall(call) => {
                let callee = self.expr(&call.callee);
                let facts = callee.value.facts.clone();
                let args = call.args.iter().map(|arg| self.expr(arg)).collect();
                let mut outcome = self.callable_call(facts, args);
                outcome.support.merge_from(callee.support);
                outcome
            }
            Expr::Array(array) => self.array(
                array
                    .elements
                    .iter()
                    .map(|element| self.expr(element))
                    .collect(),
            ),
            Expr::MemberAccess(access) => {
                let base = self.expr(&access.base);
                let mut outcome = self.unknown_after([base.clone()]);
                outcome.value.facts = base.value.facts;
                outcome
            }
            Expr::IndexAccess(access) => {
                let base = self.expr(&access.base);
                let index = self.expr(&access.index);
                let concrete = match (&base.value.concrete, &index.value.concrete) {
                    (Some(Value::Array(items)), Some(Value::Int(index))) if *index >= 0 => {
                        items.get(*index as usize).cloned()
                    }
                    _ => None,
                };
                let mut outcome = self.unknown_after([base, index]);
                // An aggregate's precise field/index facts are not modelled yet. Keeping the
                // union of its evaluated children is conservative and, unlike syntax-only
                // lookup, preserves a function or mutable reference stored through a local.
                outcome.value.concrete = concrete;
                outcome
            }
            Expr::MethodCall(call) => {
                let mut children = vec![self.expr(&call.base)];
                children.extend(call.args.iter().map(|arg| self.expr(arg)));
                self.unsupported_after(children)
            }
            Expr::BinaryOp(op) => {
                let lhs = self.expr(&op.lhs);
                let rhs = self.expr(&op.rhs);
                self.binary(lhs, rhs, &op.op)
            }
            Expr::RelationalOp(op) => {
                let lhs = self.expr(&op.lhs);
                let rhs = self.expr(&op.rhs);
                let mut outcome = self.unknown_after([lhs.clone(), rhs.clone()]);
                outcome.value.concrete = match (&lhs.value.concrete, &rhs.value.concrete) {
                    (Some(Value::Int(a)), Some(Value::Int(b))) => Some(Value::Bool(match &op.op {
                        RelationalOp::Eq => a == b,
                        RelationalOp::NotEq => a != b,
                        RelationalOp::Lt => a < b,
                        RelationalOp::Gt => a > b,
                        RelationalOp::Le => a <= b,
                        RelationalOp::Ge => a >= b,
                    })),
                    _ => None,
                };
                outcome
            }
            Expr::LogicalOp(op) => {
                let lhs = self.expr(&op.lhs);
                let rhs = self.expr(&op.rhs);
                let mut outcome = self.unknown_after([lhs.clone(), rhs.clone()]);
                outcome.value.concrete = match (&lhs.value.concrete, &rhs.value.concrete, &op.op) {
                    (Some(Value::Bool(a)), Some(Value::Bool(b)), LogicalOp::And) => {
                        Some(Value::Bool(*a && *b))
                    }
                    (Some(Value::Bool(a)), Some(Value::Bool(b)), LogicalOp::Or) => {
                        Some(Value::Bool(*a || *b))
                    }
                    _ => None,
                };
                outcome
            }
            Expr::UnaryOp(op) => {
                let inner = self.expr(&op.expr);
                let mut outcome = self.unknown_after([inner.clone()]);
                outcome.value.concrete = match (&inner.value.concrete, &op.op) {
                    (Some(Value::Bool(value)), UnaryOp::Not) => Some(Value::Bool(!value)),
                    (Some(Value::Int(value)), UnaryOp::Neg) => value.checked_neg().map(Value::Int),
                    (Some(Value::Number(value)), UnaryOp::Neg) => Some(Value::Number(-value)),
                    _ => None,
                };
                outcome
            }
            Expr::Borrow(borrow) => {
                let inner = self.expr(&borrow.expr);
                let mut outcome = self.unknown_after([inner.clone()]);
                outcome.value.facts = inner.value.facts;
                if borrow.is_mut {
                    outcome
                        .value
                        .facts
                        .merge_from(&self.place_facts(&borrow.expr));
                }
                outcome
            }
            Expr::Dereference(deref) => {
                let inner = self.expr(&deref.expr);
                let mut outcome = self.unknown_after([inner.clone()]);
                outcome.value.facts = inner.value.facts;
                outcome
            }
            Expr::UnsafeBlock(block) => self.block_with_tail(&block.stmts, block.ret.as_deref()),
            Expr::ComptimeBlock(block) => self.block_with_tail(&block.stmts, block.ret.as_deref()),
            Expr::StructInit(init) => {
                let fields = init
                    .fields
                    .iter()
                    .map(|(_, value)| (self.expr(value), self.place_facts(value)))
                    .collect::<Vec<_>>();
                let mut outcome = self.unknown_after(
                    fields
                        .iter()
                        .map(|(outcome, _)| outcome.clone())
                        .collect::<Vec<_>>(),
                );
                let target: Symbol = format!("{}_call", init.name).into();
                if init.name.starts_with("Closure_") && self.function_bodies.contains_key(&target) {
                    let mut environment = ComptimeValueFacts::default();
                    for (field, place) in fields {
                        environment.merge_from(&field.value.facts);
                        // A generated closure environment retains the captured place. This is
                        // deliberately distinct from an ordinary struct literal, whose fields are
                        // values and do not make every outer scalar into a mutable reference.
                        environment
                            .reference_origins
                            .extend(place.reference_origins.iter().cloned());
                        environment
                            .captured_place_origins
                            .extend(place.reference_origins);
                    }
                    outcome.value.facts.callable_targets.insert(target.clone());
                    outcome
                        .value
                        .facts
                        .callable_environments
                        .insert(target, environment);
                }
                outcome
            }
            Expr::Topology(topology) => self.topology(&topology.top),
            Expr::If(if_expr) => self.if_expr(if_expr),
            Expr::Range(range) => {
                self.unknown_after([self.expr(&range.start), self.expr(&range.end)])
            }
            Expr::Match(match_expr) => self.match_expr(match_expr),
            Expr::Grad(grad) => self.unsupported_after(
                grad.args
                    .iter()
                    .map(|arg| self.expr(arg))
                    .collect::<Vec<_>>(),
            ),
            Expr::Vjp(vjp) => {
                let mut children = vjp
                    .args
                    .iter()
                    .map(|arg| self.expr(arg))
                    .collect::<Vec<_>>();
                children.push(self.expr(&vjp.cotangent));
                self.unsupported_after(children)
            }
            Expr::Jvp(jvp) => {
                let mut children = jvp
                    .args
                    .iter()
                    .map(|arg| self.expr(arg))
                    .collect::<Vec<_>>();
                children.push(self.expr(&jvp.tangent));
                self.unsupported_after(children)
            }
            Expr::SpawnOn(spawn) => self.unsupported_after([
                self.topology(&spawn.top),
                self.block_with_tail(&spawn.stmts, spawn.ret.as_deref()),
            ]),
            Expr::VecMacro(vector) => self.unknown_after(
                vector
                    .elements
                    .iter()
                    .map(|element| self.expr(element))
                    .collect::<Vec<_>>(),
            ),
            // A closure body is deferred until invocation. Checked closures are normally lowered
            // to a generated `Closure_N` struct before this point; an unlowered literal remains
            // unsupported, but must not execute its body merely because it is being created.
            Expr::Closure(_) => ComptimeEvalOutcome::unsupported(),
            Expr::AsCast(cast) => self.unknown_after([self.expr(&cast.expr)]),
            Expr::Print(print) => self.unsupported_after(
                print
                    .args
                    .iter()
                    .map(|arg| self.expr(arg))
                    .collect::<Vec<_>>(),
            ),
            Expr::Println(print) => self.unsupported_after(
                print
                    .args
                    .iter()
                    .map(|arg| self.expr(arg))
                    .collect::<Vec<_>>(),
            ),
            Expr::InlineMlir(mlir) => {
                let mut children = mlir
                    .inputs
                    .iter()
                    .map(|(_, value, _)| self.expr(value))
                    .collect::<Vec<_>>();
                for clobber in &mlir.clobbers {
                    children.push(self.expr(clobber));
                    self.context
                        .note_escaping_write(self.place_facts(clobber).reference_origins);
                }
                self.unsupported_after(children)
            }
        }
    }

    fn topology(&mut self, topology: &Topology) -> ComptimeEvalOutcome {
        match topology {
            Topology::NPU(index) | Topology::AccCore(index) | Topology::GPU(index) => {
                let mut outcome = self.unknown_after([self.expr(index)]);
                if outcome.support.is_supported() {
                    outcome.value.concrete = Some(Value::Topology(topology.clone()));
                }
                outcome
            }
            Topology::Slice(base, start, end) => {
                let mut outcome =
                    self.unknown_after([self.topology(base), self.expr(start), self.expr(end)]);
                if outcome.support.is_supported() {
                    outcome.value.concrete = Some(Value::Topology(topology.clone()));
                }
                outcome
            }
            Topology::CPU
            | Topology::AMX
            | Topology::ANE
            | Topology::CpuAvx512
            | Topology::CpuNeon
            | Topology::Custom(_) => ComptimeEvalOutcome::known(Value::Topology(topology.clone())),
            // `Current` depends on the checker’s active topology; it has no constant spelling in
            // this self-contained transition harness yet, but it is still a modelled unknown.
            Topology::Current => ComptimeEvalOutcome::unknown(),
        }
    }
}
