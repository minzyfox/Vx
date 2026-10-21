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

    /// The same semantic type key serves generated IDs and layout agreement checks.
    pub fn type_key(&self, ty: &Type) -> Option<String> {
        aggregate_type_key(ty, self)
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

// Function symbols omit some placement details that generated layout metadata must retain.
fn aggregate_const_key(expr: &Expr) -> Option<String> {
    if let Some(value) = crate::syntax::types::const_value_text(expr) {
        return Some(format!("number({value:?})"));
    }
    Some(match expr {
        Expr::Identifier(id) => format!("name({:?})", id.name),
        Expr::StringLiteral(value) => format!("string({:?})", value.value),
        Expr::BinaryOp(expr) => format!(
            "binary({:?},{:?},{:?})",
            expr.op,
            aggregate_const_key(&expr.lhs)?,
            aggregate_const_key(&expr.rhs)?
        ),
        Expr::UnaryOp(expr) => format!(
            "unary({:?},{:?})",
            expr.op,
            aggregate_const_key(&expr.expr)?
        ),
        Expr::RelationalOp(expr) => format!(
            "compare({:?},{:?},{:?})",
            expr.op,
            aggregate_const_key(&expr.lhs)?,
            aggregate_const_key(&expr.rhs)?
        ),
        Expr::LogicalOp(expr) => format!(
            "logical({:?},{:?},{:?})",
            expr.op,
            aggregate_const_key(&expr.lhs)?,
            aggregate_const_key(&expr.rhs)?
        ),
        Expr::MemberAccess(expr) => format!(
            "member({:?},{:?})",
            aggregate_const_key(&expr.base)?,
            expr.member
        ),
        Expr::IndexAccess(expr) => format!(
            "index({:?},{:?})",
            aggregate_const_key(&expr.base)?,
            aggregate_const_key(&expr.index)?
        ),
        // Unsupported expressions must decline instead of depending on source annotations.
        _ => return None,
    })
}

fn aggregate_topology_key(topology: &crate::syntax::Topology) -> Option<String> {
    use crate::syntax::Topology;
    Some(match topology {
        Topology::GPU(index) | Topology::NPU(index) | Topology::AccCore(index) => {
            format!("{:?}({:?})", topology.kind(), aggregate_const_key(index)?)
        }
        Topology::Slice(base, start, end) => format!(
            "slice({:?},{:?},{:?})",
            aggregate_topology_key(base)?,
            aggregate_const_key(start)?,
            aggregate_const_key(end)?
        ),
        _ => format!("{:?}", topology.kind()),
    })
}

fn aggregate_placement_key(placement: &Option<crate::syntax::Placement>) -> Option<String> {
    Some(match placement {
        Some(p) => format!("({:?},{:?})", aggregate_topology_key(&p.topology)?, p.space),
        None => "None".into(),
    })
}

fn aggregate_type_key(ty: &Type, identities: &AggregateIdentities) -> Option<String> {
    use crate::syntax::types::Mangle;
    let keys = |args: &[Type]| {
        args.iter()
            .map(|ty| aggregate_type_key(ty, identities))
            .collect::<Option<Vec<_>>>()
    };
    Some(match ty {
        Type::Tensor(elem, dims, placement) => {
            let dimensions = dims
                .iter()
                .map(|dim| match dim {
                    Dim::Dyn => Some("?".into()),
                    Dim::Static(expr) => aggregate_const_key(expr),
                })
                .collect::<Option<Vec<_>>>()?;
            format!(
                "tensor({elem:?},{dimensions:?},{:?})",
                aggregate_placement_key(placement)?
            )
        }
        Type::Pointer(inner, placement, mutable) => format!(
            "pointer({:?},{:?},{mutable})",
            aggregate_type_key(inner, identities)?,
            aggregate_placement_key(placement)?
        ),
        Type::Borrow {
            inner,
            mem_space,
            is_mut,
            ..
        } => format!(
            "borrow({:?},{mem_space:?},{is_mut})",
            aggregate_type_key(inner, identities)?
        ),
        Type::Ref(inner, space) => format!(
            "ref({:?},{space:?})",
            aggregate_type_key(inner, identities)?
        ),
        Type::Verified(inner) => format!("verified({:?})", aggregate_type_key(inner, identities)?),
        Type::Pinned(inner, topology) => format!(
            "pinned({:?},{:?})",
            aggregate_type_key(inner, identities)?,
            aggregate_topology_key(topology)?
        ),
        Type::GenericInstance(base, args) => format!(
            "instance({:?},{:?})",
            aggregate_type_key(base, identities)?,
            keys(args)?
        ),
        Type::Function(params, result, unsafe_fn) => format!(
            "function({:?},{:?},{unsafe_fn})",
            keys(params)?,
            aggregate_type_key(result, identities)?
        ),
        Type::Closure(params, result) => format!(
            "closure({:?},{:?})",
            keys(params)?,
            aggregate_type_key(result, identities)?
        ),
        Type::Const(expr) => format!("constant({:?})", aggregate_const_key(expr)?),
        Type::Struct(name, id) | Type::Enum(name, id) => {
            if id.is_none() && crate::syntax::is_void_ty(ty) {
                ty.mangle()
            } else {
                let (_, id) = identities.resolve(name, *id)?;
                format!("nominal({id:?})")
            }
        }
        Type::Module(..) | Type::Unknown => return None,
        _ => ty.mangle(),
    })
}
