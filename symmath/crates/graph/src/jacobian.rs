use std::collections::HashMap;
use symmath_ast::ops::BinaryOp;
use symmath_common::ids::NodeId;

use crate::graph::ConstraintGraph;
use crate::node::NodeKind;

/// 残差向量 F(x)，每个约束 f_i = lhs_i - rhs_i
#[derive(Debug, Clone)]
pub struct Residual {
    pub values: Vec<f64>,
}

impl Residual {
    pub fn norm(&self) -> f64 {
        self.values.iter().map(|x| x * x).sum::<f64>().sqrt()
    }
}

/// Jacobian 矩阵 J，J[i][j] = ∂f_i / ∂x_j
#[derive(Debug, Clone)]
pub struct Jacobian {
    pub n_vars: usize,
    pub n_cons: usize,
    pub rows: Vec<Vec<f64>>,
}

/// 收集所有 Input 节点（求解变量）
pub fn collect_variables(graph: &ConstraintGraph) -> Vec<NodeId> {
    graph
        .nodes
        .iter()
        .filter(|(_, n)| matches!(n.kind, NodeKind::Input(_)))
        .map(|(id, _)| id)
        .collect()
}

/// 收集所有约束节点
pub fn collect_constraints(graph: &ConstraintGraph) -> Vec<NodeId> {
    graph
        .nodes
        .iter()
        .filter(|(_, n)| matches!(n.kind, NodeKind::ConstraintEq(_, _)))
        .map(|(id, _)| id)
        .collect()
}

/// 计算残差向量 F(x)
///
/// 每个 `ConstraintEq(lhs, rhs)` 贡献 f_i = value(lhs) - value(rhs)
pub fn compute_residual(graph: &ConstraintGraph) -> Residual {
    let mut values = Vec::new();
    for (_, node) in graph.nodes.iter() {
        if let NodeKind::ConstraintEq(lhs, rhs) = &node.kind {
            let lhs_val = graph.nodes[*lhs]
                .value
                .expect("compute_residual: lhs has no value — run tick() first");
            let rhs_val = graph.nodes[*rhs]
                .value
                .expect("compute_residual: rhs has no value — run tick() first");
            values.push(lhs_val - rhs_val);
        }
    }
    Residual { values }
}

/// 对指定变量计算全图每个节点的偏导数（调试/验证用）
///
/// 返回 `HashMap<NodeId, f64>`，key 为图中所有节点 ID。
pub fn compute_all_derivatives(
    graph: &ConstraintGraph,
    var_id: NodeId,
) -> HashMap<NodeId, f64> {
    let mut derivs: HashMap<NodeId, f64> = HashMap::new();

    for (id, node) in graph.nodes.iter() {
        let d = match &node.kind {
            NodeKind::Const(_) => 0.0,
            NodeKind::Input(_) => {
                if id == var_id {
                    1.0
                } else {
                    0.0
                }
            }
            NodeKind::Op { op, inputs } => {
                let da = derivs.get(&inputs[0]).copied().unwrap_or(0.0);
                if inputs.len() == 1 {
                    match op {
                        BinaryOp::Sin => {
                            let v = graph.nodes[inputs[0]]
                                .value
                                .expect("AD sin: no value");
                            v.cos() * da
                        }
                        BinaryOp::Cos => {
                            let v = graph.nodes[inputs[0]]
                                .value
                                .expect("AD cos: no value");
                            -(v.sin()) * da
                        }
                        _ => da,
                    }
                } else {
                    let db = derivs.get(&inputs[1]).copied().unwrap_or(0.0);
                    match op {
                        BinaryOp::Add => da + db,
                        BinaryOp::Sub => da - db,
                        BinaryOp::Mul => {
                            let a = graph.nodes[inputs[0]]
                                .value
                                .expect("AD mul: no value");
                            let b = graph.nodes[inputs[1]]
                                .value
                                .expect("AD mul: no value");
                            a * db + b * da
                        }
                        BinaryOp::Div => {
                            let a = graph.nodes[inputs[0]]
                                .value
                                .expect("AD div: no value");
                            let b = graph.nodes[inputs[1]]
                                .value
                                .expect("AD div: no value");
                            (da * b - a * db) / (b * b)
                        }
                        BinaryOp::Pow => {
                            let a = graph.nodes[inputs[0]]
                                .value
                                .expect("AD pow: no value");
                            let b = graph.nodes[inputs[1]]
                                .value
                                .expect("AD pow: no value");
                            a * db + b * da
                        }
                        BinaryOp::Eq => da - db,
                        BinaryOp::Sin | BinaryOp::Cos => unreachable!(),
                    }
                }
            }
            NodeKind::ConstraintEq(_, _) => 0.0,
        };
        derivs.insert(id, d);
    }

    derivs
}

/// 用 Forward-mode AD 计算 Jacobian 矩阵
///
/// 对每个变量 x_j 独立种子（derivative = 1），沿 DAG 正向传播，
/// 在每个 `ConstraintEq(lhs, rhs)` 节点提取 J[i][j] = ∂lhs/∂x_j - ∂rhs/∂x_j。
///
/// **前置条件**：graph 必须在调用前已 `tick()`，所有节点有值。
/// **约束**：SlotMap 迭代顺序 = IR 构造顺序（拓扑序）。
pub fn compute_jacobian(graph: &ConstraintGraph) -> Jacobian {
    let var_ids = collect_variables(graph);
    let con_ids = collect_constraints(graph);
    let n_vars = var_ids.len();
    let n_cons = con_ids.len();

    let mut rows = vec![vec![0.0; n_vars]; n_cons];

    for (j, var_id) in var_ids.iter().enumerate() {
        let mut derivs: HashMap<NodeId, f64> = HashMap::new();

        for (id, node) in graph.nodes.iter() {
            let d = match &node.kind {
                NodeKind::Const(_) => 0.0,
                NodeKind::Input(_) => {
                    if id == *var_id {
                        1.0
                    } else {
                        0.0
                    }
                }
                NodeKind::Op { op, inputs } => {
                    let da = derivs.get(&inputs[0]).copied().unwrap_or(0.0);
                    if inputs.len() == 1 {
                        match op {
                            BinaryOp::Sin => {
                                let v = graph.nodes[inputs[0]]
                                    .value
                                    .expect("AD sin: no value");
                                v.cos() * da
                            }
                            BinaryOp::Cos => {
                                let v = graph.nodes[inputs[0]]
                                    .value
                                    .expect("AD cos: no value");
                                -(v.sin()) * da
                            }
                            _ => da,
                        }
                    } else {
                        let db = derivs.get(&inputs[1]).copied().unwrap_or(0.0);
                        match op {
                            BinaryOp::Add => da + db,
                            BinaryOp::Sub => da - db,
                            BinaryOp::Mul => {
                                let a = graph.nodes[inputs[0]]
                                    .value
                                    .expect("AD mul: no value");
                                let b = graph.nodes[inputs[1]]
                                    .value
                                    .expect("AD mul: no value");
                                a * db + b * da
                            }
                            BinaryOp::Div => {
                                let a = graph.nodes[inputs[0]]
                                    .value
                                    .expect("AD div: no value");
                                let b = graph.nodes[inputs[1]]
                                    .value
                                    .expect("AD div: no value");
                                (da * b - a * db) / (b * b)
                            }
                            BinaryOp::Pow => {
                                // Pow 当前实现同 Mul（见 update_node）
                                let a = graph.nodes[inputs[0]]
                                    .value
                                    .expect("AD pow: no value");
                                let b = graph.nodes[inputs[1]]
                                    .value
                                    .expect("AD pow: no value");
                                a * db + b * da
                            }
                            BinaryOp::Eq => da - db,
                            BinaryOp::Sin | BinaryOp::Cos => unreachable!(),
                        }
                    }
                }
                NodeKind::ConstraintEq(_, _) => 0.0,
            };
            derivs.insert(id, d);
        }

        // 从约束节点提取 J[i][j]
        for (i, con_id) in con_ids.iter().enumerate() {
            if let NodeKind::ConstraintEq(lhs, rhs) = &graph.nodes[*con_id].kind {
                let dl = derivs.get(lhs).copied().unwrap_or(0.0);
                let dr = derivs.get(rhs).copied().unwrap_or(0.0);
                rows[i][j] = dl - dr;
            }
        }
    }

    Jacobian {
        n_vars,
        n_cons,
        rows,
    }
}
