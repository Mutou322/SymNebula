use symmath_common::ids::{SymbolId, ValueId};

/// IR 是 Solver-Friendly 的执行图节点。
/// 不是人类数学结构——是执行原语。
#[derive(Debug, Clone)]
pub enum IRNode {
    Const(f64),
    LoadVar(SymbolId),
    Add(ValueId, ValueId),
    Sub(ValueId, ValueId),
    Mul(ValueId, ValueId),
    Div(ValueId, ValueId),
    Sin(ValueId),
    Cos(ValueId),
    Eq(ValueId, ValueId),
}
