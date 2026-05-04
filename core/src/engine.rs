/// 逻辑时钟调度器
///
/// 使用 Compute/Commit 双缓冲模式：
/// - Compute 阶段：所有节点从入边的 delay_buffer 只读输入，计算输出
/// - Commit 阶段：将计算结果写入 delay_buffer（锁存供下一 Tick 读取）
///
/// 自环（反馈环路）天然成立：Compute 读旧值，Commit 写新值。

use std::collections::HashMap;

use crate::ast::Expr;
use crate::graph::NebulaGraph;
use crate::solver::solve_eq;
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
        // 临时存储本轮计算结果： (node_id, symbol) -> value
        let mut next_buffers: HashMap<(usize, String), f64> = HashMap::new();
        let mut state_changes: Vec<(usize, NodeState)> = Vec::new();

        // 收集所有节点 ID
        let node_ids: Vec<usize> = self.graph.nodes.iter().map(|n| n.id).collect();

        for &node_id in &node_ids {
            // 查找该节点在图中的索引
            let node_idx = match self.graph.nodes.iter().position(|n| n.id == node_id) {
                Some(idx) => idx,
                None => continue,
            };

            // 从入边的 delay_buffer 读取已知输入（Compute 阶段只读）
            let known = self.graph.get_inputs_for_node(node_id);

            // 不再从 env 注入自身旧值——双缓冲模式下，
            // delay_buffer 已携带所有跨 Tick 的输入，
            // 注入 env 的旧输出会干扰求解器（让求解器误以为输出符号已知）。

            // 根据公式类型计算
            let formula = self.graph.nodes[node_idx].formula.clone();

            match &formula {
                Expr::Eq(_, _) => {
                    match solve_eq(&formula, &known) {
                        Ok(result) => {
                            if !result.symbol.is_empty() {
                                next_buffers.insert((node_id, result.symbol), result.value);
                                // output 作为该节点的标准输出
                                next_buffers.insert((node_id, "output".to_string()), result.value);
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
                    next_buffers.insert((node_id, "output".to_string()), *n);
                    state_changes.push((node_id, NodeState::Green));
                }
                _ => {
                    // 普通表达式：求值
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

        // 2a: 将计算结果写入 env（供外部查询）
        for (key, val) in &next_buffers {
            self.env.insert(key.clone(), *val);
        }

        // 2b: 将计算结果锁存到出边的 delay_buffer
        self.graph.commit_outputs(&next_buffers);

        // 2c: 更新节点状态
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
        // Node1: a + b = 10
        let eq = parse_simple_eq("a + b = 10").unwrap();
        let mut graph = NebulaGraph::new();
        let node1 = graph.add_node(eq);

        // Node2: 常量 5
        let const_expr = Expr::Number(5.0);
        let node2 = graph.add_node(const_expr);

        // 连线：Node2.output → Node1.a，并设置默认值
        graph.add_edge_with_default(node2, "output", node1, "a", 5.0);

        let mut scheduler = Scheduler::new(graph);

        // 执行一步
        scheduler.step();

        // 验证 Node1.b = 5
        let b_val = scheduler.get_value(node1, "b").unwrap();
        assert!((b_val - 5.0).abs() < 1e-9, "期望 b = 5, 得到 {}", b_val);
    }

    #[test]
    fn test_feedback_loop() {
        // next_a = a + 1，自环
        let eq = parse_simple_eq("next_a = a + 1").unwrap();
        let mut graph = NebulaGraph::new();
        let node = graph.add_node(eq);

        // 自环：next_a -> a，默认值 0
        graph.add_edge_with_default(node, "next_a", node, "a", 0.0);

        let mut scheduler = Scheduler::new(graph);
        scheduler.step();

        // 测试的是 next_a（output）每个 Tick 的值
        // 自环语义：Compute 读 a（从 delay_buffer），Commit 写 next_a
        // Tick 1: a=0 → next_a = 0 + 1 = 1
        let out_t1 = scheduler.get_value(node, "output").unwrap();
        assert!((out_t1 - 1.0).abs() < 1e-9, "Tick1 期望 output=1, 得到 {}", out_t1);

        scheduler.step();

        // Tick 2: a=1（从 delay_buffer） → next_a = 1 + 1 = 2
        let out_t2 = scheduler.get_value(node, "output").unwrap();
        assert!((out_t2 - 2.0).abs() < 1e-9, "Tick2 期望 output=2, 得到 {}", out_t2);

        scheduler.step();

        // Tick 3: a=2 → next_a = 2 + 1 = 3
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
