//===- identity.rs - Vx Compiler ------------------------------------*- Rust -*-===//
//
// Part of the Vx Project, under the Apache License v2.0 with LLVM Exceptions.
// See LICENSE for license information.
// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
//
//===----------------------------------------------------------------------===//
//! Generated layouts use declaration IDs for nominal types and erase function-local borrow regions.
//! Placement, access permissions, and reference kinds remain part of their identity.

use crate::gid::TypeId;
use crate::symbol::Symbol;
use crate::syntax::{Dim, Expr, NumberExpr, Type};
use rustc_hash::FxHashMap;

/// The declaration named by an aggregate use.
pub enum AggregateBase<'a> {
    Name(&'a str),
    Type(&'a Type),
    Declaration(TypeId),
}
impl<'a> From<&'a str> for AggregateBase<'a> {
    fn from(name: &'a str) -> Self {
        Self::Name(name)
    }
}
impl<'a> From<&'a Type> for AggregateBase<'a> {
    fn from(ty: &'a Type) -> Self {
        Self::Type(ty)
    }
}
impl<'a> From<TypeId> for AggregateBase<'a> {
    fn from(id: TypeId) -> Self {
        Self::Declaration(id)
    }
}

/// Declaration identity for generated layouts, shared across a compilation's workers.
#[derive(Debug, Clone, Default)]
pub struct AggregateIdentities {
    names: FxHashMap<Symbol, Option<TypeId>>,
    declarations: FxHashMap<TypeId, Symbol>,
}

impl AggregateIdentities {
    pub fn new(declarations: impl IntoIterator<Item = (TypeId, Symbol)>) -> Self {
        let mut result = Self::default();
        for (id, name) in declarations {
            result
                .names
                .entry(name.clone())
                .and_modify(|old| {
                    if *old != Some(id) {
                        *old = None;
                    }
                })
                .or_insert(Some(id));
            result
                .declarations
                .entry(id)
                .and_modify(|old| {
                    if name < *old {
                        *old = name.clone();
                    }
                })
                .or_insert(name);
        }
        result
    }

    pub fn from_layouts(layouts: &FxHashMap<TypeId, crate::registry::TypeDefinition>) -> Self {
        Self::new(
            layouts
                .iter()
                .map(|(id, def)| (*id, def.name.as_str().into())),
        )
    }

    /// An attached ID is authoritative. A missing ID requires an unambiguous name.
    pub fn resolve(&self, name: &Symbol, attached: Option<TypeId>) -> Option<(Symbol, TypeId)> {
        let id = match attached {
            Some(id) => id,
            None => self.names.get(name).copied().flatten()?,
        };
        Some((self.declarations.get(&id)?.clone(), id))
    }

    pub fn resolve_base(&self, base: AggregateBase<'_>) -> Option<(Symbol, TypeId)> {
        match base {
            AggregateBase::Name(name) => self.resolve(&name.into(), None),
            AggregateBase::Type(Type::Struct(name, id) | Type::Enum(name, id)) => {
                self.resolve(name, *id)
            }
            AggregateBase::Declaration(id) => Some((self.declarations.get(&id)?.clone(), id)),
            AggregateBase::Type(_) => None,
        }
    }

    pub fn len(&self) -> usize {
        self.declarations.len()
    }
    pub fn is_empty(&self) -> bool {
        self.declarations.is_empty()
    }

    /// Canonical metadata retains placement and access permissions but erases local borrow regions.
    pub fn normalize_type(&self, ty: &mut Type) -> Option<()> {
        fn normalize_constant(expr: &mut Expr) {
            use crate::syntax::Span;
            if let Some(value) = crate::syntax::types::const_value_text(expr) {
                *expr = Expr::Number(NumberExpr::new(value, None, Span::default()));
                return;
            }
            match expr {
                Expr::Identifier(expr) => expr.span = Span::default(),
                Expr::StringLiteral(expr) => expr.span = Span::default(),
                Expr::BinaryOp(expr) => {
                    normalize_constant(&mut expr.lhs);
                    normalize_constant(&mut expr.rhs);
                    expr.span = Span::default();
                    expr.operand_ty = None;
                }
                Expr::RelationalOp(expr) => {
                    normalize_constant(&mut expr.lhs);
                    normalize_constant(&mut expr.rhs);
                    expr.span = Span::default();
                    expr.operand_ty = None;
                }
                Expr::LogicalOp(expr) => {
                    normalize_constant(&mut expr.lhs);
                    normalize_constant(&mut expr.rhs);
                    expr.span = Span::default();
                }
                Expr::UnaryOp(expr) => {
                    normalize_constant(&mut expr.expr);
                    expr.span = Span::default();
                }
                Expr::MemberAccess(expr) => {
                    normalize_constant(&mut expr.base);
                    expr.span = Span::default();
                    expr.struct_name = None;
                }
                Expr::IndexAccess(expr) => {
                    normalize_constant(&mut expr.base);
                    normalize_constant(&mut expr.index);
                    expr.span = Span::default();
                }
                _ => {}
            }
        }

        fn normalize_topology(topology: &mut crate::syntax::Topology) {
            use crate::syntax::Topology;
            match topology {
                Topology::GPU(index) | Topology::NPU(index) | Topology::AccCore(index) => {
                    normalize_constant(index)
                }
                Topology::Slice(base, start, end) => {
                    normalize_topology(base);
                    normalize_constant(start);
                    normalize_constant(end);
                }
                _ => {}
            }
        }

        fn normalize_placement(placement: &mut Option<crate::syntax::Placement>) {
            if let Some(placement) = placement {
                normalize_topology(&mut placement.topology);
            }
        }
        if matches!(ty, Type::Struct(_, None)) && crate::syntax::is_void_ty(ty) {
            return Some(());
        }
        match ty {
            Type::Struct(name, id) | Type::Enum(name, id) => {
                let (name, id) = self.resolve(name, *id)?;
                // Struct is the common nominal form in retained layout metadata.
                *ty = Type::Struct(name, Some(id));
            }
            Type::Borrow {
                inner, region_id, ..
            } => {
                *region_id = 0;
                self.normalize_type(inner)?;
            }
            Type::Ref(inner, ..) | Type::Verified(inner) => {
                self.normalize_type(inner)?;
            }
            Type::Pointer(inner, placement, _) => {
                self.normalize_type(inner)?;
                normalize_placement(placement);
            }
            Type::Pinned(inner, topology) => {
                self.normalize_type(inner)?;
                normalize_topology(topology);
            }
            Type::GenericInstance(base, args) => {
                self.normalize_type(base)?;
                for arg in args {
                    self.normalize_type(arg)?;
                }
            }
            Type::Function(params, result, _) | Type::Closure(params, result) => {
                for param in params {
                    self.normalize_type(param)?;
                }
                self.normalize_type(result)?;
            }
            Type::Const(expr) => normalize_constant(expr),
            Type::Tensor(_, dims, placement) => {
                normalize_placement(placement);
                for dim in dims {
                    if let Dim::Static(expr) = dim {
                        normalize_constant(expr);
                    }
                }
            }
            Type::Module(..) | Type::Unknown => return None,
            _ => {}
        }
        Some(())
    }
}
