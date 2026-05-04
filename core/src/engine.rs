/// 逻辑时钟调度器
///
/// 每次 step() 遍历所有节点，汇集来自边的已知值，
/// 调用约束求解或求值，更新状态和环境。

use std::collections::HashMap;

use crate::ast::Expr;
use crate::graph::NebulaGraph;
use crate::solver::solve_eq;
use crate::state::NodeState;

pub struct Scheduler {
    pub graph: NebulaGraph,
    /// 全局环境：(node_id, symbol) -> value
    pub env: HashMap<(usize, String), f64>,
    pub tick: usize,
}

impl Scheduler {
    pub fn new(graph: NebulaGraph) -> Self {
        Scheduler {
            graph,
            env: HashMap::new(),
            tick: 0,
        }
    }

    /// 执行一个 tick：传播所有已知值，更新每个节点的状态
    pub fn step(&mut self) {
        let mut new_env = self.env.clone();
        let mut state_changes = Vec::new();

        // 收集所有节点的 ID
        let node_ids: Vec<usize> = self.graph.nodes.iter().map(|n| n.id).collect();

        for &node_id in &node_ids {
            // 查找该节点在图中的索引
            let node_idx = match self.graph.nodes.iter().position(|n| n.id == node_id) {
                Some(idx) => idx,
                None => continue,
            };

            // 收集该节点的已知输入
            let known = self.graph.collect_inputs(node_id, &self.env);

            // 根据公式类型处理
            let formula = self.graph.nodes[node_idx].formula.clone();

            match &formula {
                Expr::Eq(_, _) => {
                    // 约束求解
                    match solve_eq(&formula, &known) {
                        Ok(result) => {
                            if !result.symbol.is_empty() {
                                new_env.insert((node_id, result.symbol), result.value);
                                // 同时写入 output 作为该节点的输出值
                                new_env.insert((node_id, "output".to_string()), result.value);
                            }
                            state_changes.push((node_id, result.state));
                        }
                        Err(_) => {
                            state_changes.push((node_id, NodeState::Purple));
                        }
                    }
                }
                Expr::Number(n) => {
                    // 常量节点：输出自身值
                    new_env.insert((node_id, "output".to_string()), *n);
                    state_changes.push((node_id, NodeState::Green));
                }
                _ => {
                    // 普通表达式：求值
                    match formula.eval(&known) {
                        Ok(val) => {
                            new_env.insert((node_id, "output".to_string()), val);
                            state_changes.push((node_id, NodeState::Green));
                        }
                        Err(_) => {
                            state_changes.push((node_id, NodeState::Purple));
                        }
                    }
                }
            }
        }

        // 更新图状态
        for (id, state) in &state_changes {
            if let Some(node) = self.graph.nodes.iter_mut().find(|n| n.id == *id) {
                node.state = state.clone();
                if *state == NodeState::Green {
                    if let Some(val) = new_env.get(&(*id, "output".to_string())) {
                        node.value = Some(*val);
                    }
                }
            }
        }

        self.env = new_env;
        self.tick += 1;
    }

    /// 获取某个节点某个符号的值
    pub fn get_value(&self, node_id: usize, symbol: &str) -> Option<f64> {
        self.env.get(&(node_id, symbol.to_string())).copied()
    }

    /// 获取调度状态摘要
    pub fn get_status(&self) -> String {
        let total = self.graph.nodes.len();
        let green = self.graph.nodes.iter().filter(|n| n.state == NodeState::Green).count();
        let yellow = self.graph.nodes.iter().filter(|n| n.state == NodeState::Yellow).count();
        let purple = self.graph.nodes.iter().filter(|n| n.state == NodeState::Purple).count();
        format!("Tick {} | {}个节点: {}绿 {}黄 {}紫", self.tick, total, green, yellow, purple)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parse_simple_eq;
    use crate::graph::NebulaGraph;

    #[test]
    fn test_mvp() {
        // Node1: a + b = 10
        let eq = parse_simple_eq("a + b = 10").unwrap();
        let mut graph = NebulaGraph::new();
        let node1 = graph.add_node(eq);

        // Node2: 常量 5
        let const_expr = Expr::Number(5.0);
        let node2 = graph.add_node(const_expr);

        // 连线：Node2.output → Node1.a
        graph.add_edge(node2, "output", node1, "a");

        let mut scheduler = Scheduler::new(graph);

        // 初始化 Node2 的输出为 5
        scheduler.env.insert((node2, "output".to_string()), 5.0);

        // 执行一步
        scheduler.step();

        // 验证 Node1.b = 5
        let b_val = scheduler.get_value(node1, "b").unwrap();
        assert!((b_val - 5.0).abs() < 1e-9, "期望 b = 5, 得到 {}", b_val);
    }

    #[test]
    fn test_constant_node() {
        let mut graph = NebulaGraph::new();
        let n = graph.add_node(Expr::Number(42.0));

        let mut scheduler = Scheduler::new(graph);
        scheduler.step();

        let val = scheduler.get_value(n, "output").unwrap();
        assert!((val - 42.0).abs() < 1e-9);
        assert_eq!(scheduler.graph.nodes[0].state, NodeState::Green);
    }
}
