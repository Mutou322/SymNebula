use symmath_common::ids::{NodeId, SymbolId};

use crate::ops::*;

#[derive(Debug, Clone)]
pub enum Expr {
    Const(f64),
    Var(SymbolId),
    Unary { op: UnaryOp, input: NodeId },
    Binary { op: BinaryOp, lhs: NodeId, rhs: NodeId },
    Call { func: SymbolId, args: Vec<NodeId> },
}
