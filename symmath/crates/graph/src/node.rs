use symmath_common::ids::NodeId;

/// 约束图中的节点类型
#[derive(Debug, Clone)]
pub enum NodeKind {
    /// 用户输入变量
    Input(symmath_common::ids::SymbolId),
    /// 常量
    Const(f64),
    /// 二元运算节点
    Op {
        op: symmath_ast::ops::BinaryOp,
        inputs: Vec<NodeId>,
    },
    /// 等式约束 lhs = rhs
    ConstraintEq(NodeId, NodeId),
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: NodeKind,
    pub value: Option<f64>,
    pub dirty: bool,
    pub dependents: Vec<NodeId>,
}
