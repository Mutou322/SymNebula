/// 星云图数据结构
///
/// 管理神经元-突触拓扑结构。
/// 纯标准库实现，不使用外部图库。

use std::collections::HashMap;

use crate::ast::Expr;
use crate::state::{NodeState, SolverState};

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
    /// Newton 迭代状态（仅 Eq 节点使用）
    pub solver_state: SolverState,
    /// 求解目标变量名（如 "x"），由 engine 自动推断或用户指定
    pub solve_target: Option<String>,
}

// ============================================================
// 突触（有向边）
// ============================================================

/// 有向边：源节点 -> 目标节点，并指定连接的符号名称
///
/// 携带双缓冲所需的 delay_buffer（锁存值供下一 Tick 读取）
/// 和 default_value（T0 初始值）。
#[derive(Debug, Clone)]
pub struct Synapse {
    pub from_node: usize,
    pub from_symbol: String,
    pub to_node: usize,
    pub to_symbol: String,
    /// 锁存值——Compute 阶段从此读取，Commit 阶段写入下一 Tick 值
    pub delay_buffer: Option<f64>,
    /// 初始静息电位（T0 时 delay_buffer 为空则用此值）
    pub default_value: Option<f64>,
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
        let formula_syms = formula.symbols();
        // 第一个符号作为默认求解目标（仅对 Eq 有用）
        let solve_target = if matches!(&formula, Expr::Eq(_, _)) {
            formula_syms.first().cloned()
        } else {
            None
        };
        self.nodes.push(Node {
            id,
            formula,
            state: NodeState::Gray,
            value: None,
            solver_state: SolverState::new(0.0),
            solve_target,
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
            delay_buffer: None,
            default_value: None,
        });
    }

    /// 添加有向边并设置默认值
    pub fn add_edge_with_default(
        &mut self,
        from_node: usize,
        from_symbol: &str,
        to_node: usize,
        to_symbol: &str,
        default_value: f64,
    ) {
        self.edges.push(Synapse {
            from_node,
            from_symbol: from_symbol.to_string(),
            to_node,
            to_symbol: to_symbol.to_string(),
            delay_buffer: None,
            default_value: Some(default_value),
        });
    }

    /// 获取目标节点在 Compute 阶段的已知输入
    ///
    /// 从所有入边的 delay_buffer 读取。
    /// 若 delay_buffer 为 None，则用 default_value 兜底。
    /// 若两者都为 None，该输入不参与计算。
    pub fn get_inputs_for_node(&self, node_id: usize) -> HashMap<String, f64> {
        let mut known = HashMap::new();
        for e in &self.edges {
            if e.to_node == node_id {
                let val = e.delay_buffer.or(e.default_value);
                if let Some(v) = val {
                    known.insert(e.to_symbol.clone(), v);
                }
            }
        }
        known
    }

    /// 将 src 中该节点相关条目的值写入对应出边的 delay_buffer（Commit 阶段）
    ///
    /// src 格式为 (node_id, symbol) -> value，遍历所有边，
    /// 若边的 from_node 匹配且 from_symbol 匹配，则写入 delay_buffer。
    pub fn commit_outputs(&mut self, src: &HashMap<(usize, String), f64>) {
        for edge in &mut self.edges {
            let key = (edge.from_node, edge.from_symbol.clone());
            if let Some(val) = src.get(&key) {
                edge.delay_buffer = Some(*val);
            }
        }
    }

    /// 设置某条边的默认值（用于测试或脚本初始化）
    pub fn set_edge_default_value(&mut self, from_node: usize, from_symbol: &str, to_node: usize, to_symbol: &str, default: f64) {
        for edge in &mut self.edges {
            if edge.from_node == from_node
                && edge.from_symbol == from_symbol
                && edge.to_node == to_node
                && edge.to_symbol == to_symbol
            {
                edge.default_value = Some(default);
                break;
            }
        }
    }
}
