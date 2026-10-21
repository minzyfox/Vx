//===- identity.rs - Vx Compiler ------------------------------------*- Rust -*-===//
//
// Part of the Vx Project, under the Apache License v2.0 with LLVM Exceptions.
// See LICENSE for license information.
// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
//
//===----------------------------------------------------------------------===//
//! Source-independent keys shared by aggregate layouts and function specializations.

use super::types::Mangle;
use super::{Expr, Type};

pub(crate) fn const_key(expr: &Expr) -> Option<String> {
    if let Some(value) = crate::syntax::types::const_value_text(expr) {
        return Some(format!("number({value:?})"));
    }
    Some(match expr {
        Expr::Identifier(id) => format!("name({:?})", id.name),
        Expr::StringLiteral(value) => format!("string({:?})", value.value),
        Expr::BinaryOp(expr) => format!(
            "binary({:?},{:?},{:?})",
            expr.op,
            const_key(&expr.lhs)?,
            const_key(&expr.rhs)?
        ),
        Expr::UnaryOp(expr) => format!("unary({:?},{:?})", expr.op, const_key(&expr.expr)?),
        Expr::RelationalOp(expr) => format!(
            "compare({:?},{:?},{:?})",
            expr.op,
            const_key(&expr.lhs)?,
            const_key(&expr.rhs)?
        ),
        Expr::LogicalOp(expr) => format!(
            "logical({:?},{:?},{:?})",
            expr.op,
            const_key(&expr.lhs)?,
            const_key(&expr.rhs)?
        ),
        Expr::MemberAccess(expr) => {
            format!("member({:?},{:?})", const_key(&expr.base)?, expr.member)
        }
        Expr::IndexAccess(expr) => format!(
            "index({:?},{:?})",
            const_key(&expr.base)?,
            const_key(&expr.index)?
        ),
        // Unsupported expressions must decline instead of depending on source annotations.
        _ => return None,
    })
}

pub(crate) fn topology_key(topology: &crate::syntax::Topology) -> Option<String> {
    use crate::syntax::Topology;
    Some(match topology {
        Topology::GPU(index) | Topology::NPU(index) | Topology::AccCore(index) => {
            format!("{:?}({:?})", topology.kind(), const_key(index)?)
        }
        Topology::Slice(base, start, end) => format!(
            "slice({:?},{:?},{:?})",
            topology_key(base)?,
            const_key(start)?,
            const_key(end)?
        ),
        _ => format!("{:?}", topology.kind()),
    })
}

pub(crate) fn placement_key(placement: &Option<crate::syntax::Placement>) -> Option<String> {
    Some(match placement {
        Some(p) => format!("({:?},{:?})", topology_key(&p.topology)?, p.space),
        None => "None".into(),
    })
}

/// Encode every byte, so punctuation and user names cannot collapse to the same symbol.
pub(crate) fn symbol_key(key: &str) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(key.len() * 2);
    for byte in key.bytes() {
        write!(result, "{byte:02x}").expect("writing to a String cannot fail");
    }
    result
}

/// Keep existing type names and append locations with their positions in the type tree.
/// Local borrow regions and source spelling do not affect a specialization's identity.
pub(crate) fn specialization_type_name(ty: &Type) -> Option<String> {
    fn visit(ty: &Type, path: &str, entries: &mut Vec<(String, String)>) -> Option<()> {
        let mut record = |key: String| entries.push((path.to_owned(), key));
        match ty {
            Type::Tensor(_, _, Some(p)) | Type::Pointer(_, Some(p), _) => {
                record(placement_key(&Some(p.clone()))?);
            }
            Type::Borrow {
                mem_space: Some(space),
                ..
            }
            | Type::Ref(_, space) => {
                record(format!("memory({space:?})"));
            }
            Type::Pinned(_, topology) => record(topology_key(topology)?),
            _ => {}
        }
        match ty {
            Type::Pointer(inner, _, _)
            | Type::Borrow { inner, .. }
            | Type::Ref(inner, _)
            | Type::Pinned(inner, _)
            | Type::Verified(inner) => {
                visit(inner, &format!("{path}/inner"), entries)?;
            }
            Type::GenericInstance(base, args) => {
                visit(base, &format!("{path}/base"), entries)?;
                for (i, arg) in args.iter().enumerate() {
                    visit(arg, &format!("{path}/arg{i}"), entries)?;
                }
            }
            Type::Function(params, result, _) | Type::Closure(params, result) => {
                for (i, param) in params.iter().enumerate() {
                    visit(param, &format!("{path}/param{i}"), entries)?;
                }
                visit(result, &format!("{path}/result"), entries)?;
            }
            _ => {}
        }
        Some(())
    }
    let mut entries = Vec::new();
    visit(ty, "", &mut entries)?;
    let mut name = ty.mangle();
    if !entries.is_empty() {
        name.push_str("$locations$");
        name.push_str(&symbol_key(&format!("{entries:?}")));
    }
    Some(name)
}
