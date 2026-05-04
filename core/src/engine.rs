/// 逻辑时钟调度器
///
/// 使用 Compute/Commit 双缓冲模式：
/// - Compute 阶段：所有节点从入边的 delay_buffer 只读输入，计算输出
/// - Commit 阶段：将计算结果写入 delay_buffer（锁存供下一 Tick 读取）
///
/// 自环（反馈环路）天然成立：Compute 读旧值，Commit 写新值。
///
/// Eq 节点处理策略：
///   优先代数求解（solve_eq），失败则降级为 Newton 数值迭代（solver_step）。
///   每 Tick 一步，跨 Tick 收敛。快机器收敛快，慢机器收敛慢，结果一致。

use std::collections::HashMap;

use crate::ast::Expr;
use crate::graph::NebulaGraph;
use crate::solver::{solve_eq, solve_or_iterate};
use crate::state::NodeState;

pub struct Scheduler {
    pub graph: NebulaGraph,
    /// 全局环境：(node_id, symbol) -> value
    /// 存储每个节点最后的输出值，供外部查询
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

    /// 执行一个 Tick
    ///
    /// 分两阶段：
    /// 1. Compute — 从 delay_buffer 读取所有已知输入，计算输出
    /// 2. Commit — 将计算结果锁存到 delay_buffer，更新节点状态
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
            let formula = self.graph.nodes[node_idx].formula.clone();

            match &formula {
                Expr::Eq(_, _) => {
                    // 优先代数求解
                    match solve_eq(&formula, &known) {
                        Ok(result) => {
                            if !result.symbol.is_empty() {
                                next_buffers.insert((node_id, result.symbol.clone()), result.value);
                                next_buffers.insert((node_id, "output".to_string()), result.value);
                            }
                            state_changes.push((node_id, result.state));
                        }
                        Err(_) => {
                            // 代数失败，尝试 Newton 数值迭代
                            let solve_target = self.graph.nodes[node_idx]
                                .solve_target
                                .clone()
                                .unwrap_or_default();

                            if solve_target.is_empty() {
                                state_changes.push((node_id, NodeState::Purple));
                            } else {
                                let solver = &mut self.graph.nodes[node_idx].solver_state;
                                let (val, state, _still_going) =
                                    solve_or_iterate(&formula, &known, solver, &solve_target);

                                // 检测奇异：如果 value 不是有限值或 residual 发散
                                let final_state = if !val.is_finite() || val.abs() > 1e15 {
                                    NodeState::Purple
                                } else {
                                    state
                                };

                                next_buffers.insert((node_id, solve_target.clone()), val);
                                next_buffers.insert((node_id, "output".to_string()), val);
                                state_changes.push((node_id, final_state));
                            }
                        }
                    }
                }
                Expr::Number(n) => {
                    next_buffers.insert((node_id, "output".to_string()), *n);
                    state_changes.push((node_id, NodeState::Green));
                }
                _ => {
                    match formula.eval(&known) {
                        Ok(val) => {
                            next_buffers.insert((node_id, "output".to_string()), val);
                            state_changes.push((node_id, NodeState::Green));
                        }
                        Err(_) => {
                            state_changes.push((node_id, NodeState::Purple));
                        }
                    }
                }
            }
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
    use crate::ast::parse_simple_eq;
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
