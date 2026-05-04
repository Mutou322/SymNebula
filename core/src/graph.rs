/// 星云图数据结构
///
/// 管理神经元-突触拓扑结构。
/// 纯标准库实现，不使用外部图库。

use std::collections::HashMap;

use crate::ast::Expr;
use crate::state::NodeState;

// ============================================================
// 神经元节点
// ============================================================

#[derive(Debug, Clone)]
pub struct Node {
    pub id: usize,
    /// 节点存储的公式（等式或表达式）
    pub formula: Expr,
    /// 当前状态
    pub state: NodeState,
    /// 当前数值（如果是推导出的值）
    pub value: Option<f64>,
}

// ============================================================
// 突触（有向边）
// ============================================================

/// 有向边：源节点 -> 目标节点，并指定连接的符号名称
#[derive(Debug, Clone)]
pub struct Synapse {
    pub from_node: usize,
    pub from_symbol: String,
    pub to_node: usize,
    pub to_symbol: String,
}

// ============================================================
// 星云图
// ============================================================

pub struct NebulaGraph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Synapse>,
    next_id: usize,
}

impl NebulaGraph {
    pub fn new() -> Self {
        NebulaGraph {
            nodes: vec![],
            edges: vec![],
            next_id: 0,
        }
    }

    /// 添加节点，返回节点 ID
    pub fn add_node(&mut self, formula: Expr) -> usize {
        let id = self.next_id;
        self.nodes.push(Node {
            id,
            formula,
            state: NodeState::Gray,
            value: None,
        });
        self.next_id += 1;
        id
    }

    /// 添加有向边
    pub fn add_edge(&mut self, from_node: usize, from_symbol: &str, to_node: usize, to_symbol: &str) {
        self.edges.push(Synapse {
            from_node,
            from_symbol: from_symbol.to_string(),
            to_node,
            to_symbol: to_symbol.to_string(),
        });
    }

    /// 收集目标节点的已知输入（来自上游节点的输出）
    pub fn collect_inputs(&self, node_id: usize, global_env: &HashMap<(usize, String), f64>) -> HashMap<String, f64> {
        let mut known = HashMap::new();
        for e in &self.edges {
            if e.to_node == node_id {
                if let Some(val) = global_env.get(&(e.from_node, e.from_symbol.clone())) {
                    known.insert(e.to_symbol.clone(), *val);
                }
            }
        }
        known
    }
}
