/// 逻辑时钟调度器
///
/// 使用 Compute/Commit 双缓冲模式：
/// - Compute 阶段：从 delay_buffer 只读输入，通过 SolverManager 求解
/// - Commit 阶段：将计算结果写入 delay_buffer（锁存供下一 Tick 读取）
///
/// 内核只负责调度，不负责"怎么解方程"。
/// 所有求解能力通过 Solver trait 注入。

use std::collections::HashMap;

use crate::graph::NebulaGraph;
use crate::solver_trait::SolverManager;
use crate::state::NodeState;

pub struct Scheduler {
    pub graph: NebulaGraph,
    /// 全局环境：(node_id, symbol) -> value
    pub env: HashMap<(usize, String), f64>,
    pub tick: usize,
    /// 求解器管理器
    pub solver_mgr: SolverManager,
}

impl Scheduler {
    /// 使用默认求解器创建调度器
    pub fn new(graph: NebulaGraph) -> Self {
        let solver_mgr = SolverManager::new(vec![
            Box::new(crate::solver_trait::EvalSolver::new()),
            Box::new(crate::solver_trait::NewtonSolver::new()),
        ]);
        Scheduler {
            graph,
            env: HashMap::new(),
            tick: 0,
            solver_mgr,
        }
    }

    /// 使用自定义求解器管理器创建调度器
    pub fn with_solver(graph: NebulaGraph, solver_mgr: SolverManager) -> Self {
        Scheduler {
            graph,
            env: HashMap::new(),
            tick: 0,
            solver_mgr,
        }
    }

    /// 执行一个 Tick
    ///
    /// 分两阶段：
    /// 1. Compute — 从 delay_buffer 读取所有已知输入，通过 SolverManager 求解
    /// 2. Commit — 将结果锁存到 delay_buffer，更新节点状态
    pub fn step(&mut self) {
        // ============================================================
        // Phase 1: Compute
        // ============================================================
        let mut next_buffers: HashMap<(usize, String), f64> = HashMap::new();
        let mut state_changes: Vec<(usize, NodeState)> = Vec::new();

        let node_ids: Vec<usize> = self.graph.nodes.iter().map(|n| n.id).collect();

        for &node_id in &node_ids {
            let node_idx = match self.graph.nodes.iter().position(|n| n.id == node_id) {
                Some(idx) => idx,
                None => continue,
            };

            let known = self.graph.get_inputs_for_node(node_id);
            let node = &self.graph.nodes[node_idx];

            // 通过 SolverManager 求解
            let result = self.solver_mgr.solve_node(node, &known);

            // 将结果写入 next_buffers
            let values = result.values();
            for (sym, val) in &values {
                next_buffers.insert((node_id, sym.clone()), *val);
            }
            // 确保 output 存在
            if !values.contains_key("output") && !values.is_empty() {
                let first_val = values.values().next().unwrap();
                next_buffers.insert((node_id, "output".to_string()), *first_val);
            }

            let node_state = result.node_state();
            state_changes.push((node_id, node_state));
        }

        // ============================================================
        // Phase 2: Commit
        // ============================================================

        for (key, val) in &next_buffers {
            self.env.insert(key.clone(), *val);
        }

        self.graph.commit_outputs(&next_buffers);

        for (id, state) in &state_changes {
            if let Some(node) = self.graph.nodes.iter_mut().find(|n| n.id == *id) {
                node.state = state.clone();
                if *state == NodeState::Green {
                    if let Some(val) = next_buffers.get(&(*id, "output".to_string())) {
                        node.value = Some(*val);
                    }
                }
            }
        }

        self.tick += 1;
    }

    /// 执行多个 Tick
    pub fn step_n(&mut self, n: usize) {
        for _ in 0..n {
            self.step();
        }
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
    use crate::ast::{parse_simple_eq, Expr};
    use crate::graph::NebulaGraph;

    #[test]
    fn test_mvp() {
        let eq = parse_simple_eq("a + b = 10").unwrap();
        let mut graph = NebulaGraph::new();
        let node1 = graph.add_node(eq);

        let const_expr = Expr::Number(5.0);
        let node2 = graph.add_node(const_expr);

        graph.add_edge_with_default(node2, "output", node1, "a", 5.0);

        let mut scheduler = Scheduler::new(graph);
        scheduler.step();

        let b_val = scheduler.get_value(node1, "b").unwrap();
        assert!((b_val - 5.0).abs() < 1e-9, "期望 b = 5, 得到 {}", b_val);
    }

    #[test]
    fn test_feedback_loop() {
        let eq = parse_simple_eq("next_a = a + 1").unwrap();
        let mut graph = NebulaGraph::new();
        let node = graph.add_node(eq);

        graph.add_edge_with_default(node, "next_a", node, "a", 0.0);

        let mut scheduler = Scheduler::new(graph);
        scheduler.step();

        let out_t1 = scheduler.get_value(node, "output").unwrap();
        assert!((out_t1 - 1.0).abs() < 1e-9, "Tick1 期望 output=1, 得到 {}", out_t1);

        scheduler.step();

        let out_t2 = scheduler.get_value(node, "output").unwrap();
        assert!((out_t2 - 2.0).abs() < 1e-9, "Tick2 期望 output=2, 得到 {}", out_t2);

        scheduler.step();

        let out_t3 = scheduler.get_value(node, "output").unwrap();
        assert!((out_t3 - 3.0).abs() < 1e-9, "Tick3 期望 output=3, 得到 {}", out_t3);
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
