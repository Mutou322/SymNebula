/// SymNebula 压力测试集
///
/// 测试 A：双向坍缩 — 链接到 b 而非 a，验证 a 自动解出 5
/// 测试 B：反馈环路 — next_a = a + 1，自环，验证 Tick 迭代
/// 测试 C：紫色奇异 — a / b = 10, b = 0，验证紫色安全切断
///
/// 运行: cargo run --bin stress

use sym_nebula_core::ast::{parse_simple_eq, Expr};
use sym_nebula_core::graph::NebulaGraph;
use sym_nebula_core::engine::Scheduler;
use sym_nebula_core::state::NodeState;

fn main() {
    let mut all_pass = true;

    // ============================================================
    // 测试 A：双向坍缩
    // ============================================================
    println!("{}", "=".repeat(60));
    println!("  测试 A：双向坍缩 (Symmetry Test)");
    println!("{}", "=".repeat(60));
    println!("  公式: a + b = 10");
    println!("  链接: 5 -> b  (不是 a)");
    println!("  预期: a 自动推导为 5");
    println!();

    let eq_a = parse_simple_eq("a + b = 10").expect("解析失败");
    let mut graph_a = NebulaGraph::new();
    let n1_a = graph_a.add_node(eq_a);
    let n2_a = graph_a.add_node(Expr::Number(5.0));

    // 链接 5 -> b（不是 a）
    graph_a.add_edge(n2_a, "output", n1_a, "b");

    let mut sched_a = Scheduler::new(graph_a);
    sched_a.env.insert((n2_a, "output".to_string()), 5.0);
    sched_a.step();

    let a_val = sched_a.get_value(n1_a, "a");
    let b_val = sched_a.get_value(n1_a, "b");

    println!("  结果: a = {:?}, b = {:?}", a_val, b_val);

    let a_ok = match a_val {
        Some(v) if (v - 5.0).abs() < 1e-9 => {
            println!("  ✅ 测试 A 通过: a = {}", v);
            true
        }
        Some(v) => {
            println!("  ❌ 测试 A 失败: a = {} (期望 5.0)", v);
            false
        }
        None => {
            println!("  ❌ 测试 A 失败: a 未被推导");
            false
        }
    };
    all_pass &= a_ok;
    println!();

    // ============================================================
    // 测试 B：反馈环路
    // ============================================================
    println!("{}", "=".repeat(60));
    println!("  测试 B：反馈环路 (Feedback Loop Test)");
    println!("{}", "=".repeat(60));
    println!("  公式: next_a = a + 1");
    println!("  链接: next_a -> a (自环)");
    println!("  初值: a = 0");
    println!("  预期: Tick1 → a = 1, Tick2 → a = 2");
    println!();

    let eq_b = parse_simple_eq("next_a = a + 1").expect("解析失败");
    let mut graph_b = NebulaGraph::new();
    let node_b = graph_b.add_node(eq_b);

    // 自环: next_a -> a
    graph_b.add_edge(node_b, "next_a", node_b, "a");

    let mut sched_b = Scheduler::new(graph_b);

    // 设置初值 a = 0
    sched_b.env.insert((node_b, "a".to_string()), 0.0);

    println!("  Tick 0: a = 0 (初值)");

    // Tick 1
    println!("  [调试] env 前: {:?}", sched_b.env);
    sched_b.step();
    println!("  [调试] env 后: {:?}", sched_b.env);
    let a_t1 = sched_b.get_value(node_b, "a");
    println!("  Tick 1: a = {:?}", a_t1);

    // Tick 2
    sched_b.step();
    let a_t2 = sched_b.get_value(node_b, "a");
    println!("  Tick 2: a = {:?}", a_t2);

    let b_ok = match (a_t1, a_t2) {
        (Some(t1), Some(t2)) if (t1 - 1.0).abs() < 1e-9 && (t2 - 2.0).abs() < 1e-9 => {
            println!("  ✅ 测试 B 通过: Tick1 = {}, Tick2 = {}", t1, t2);
            true
        }
        (Some(t1), Some(t2)) => {
            println!("  ❌ 测试 B 失败: Tick1 = {}, Tick2 = {} (期望 1, 2)", t1, t2);
            false
        }
        _ => {
            println!("  ❌ 测试 B 失败: 未推导出 a");
            false
        }
    };
    all_pass &= b_ok;
    println!();

    // ============================================================
    // 测试 C：紫色奇异
    // ============================================================
    println!("{}", "=".repeat(60));
    println!("  测试 C：紫色奇异态 (Purple Guard Test)");
    println!("{}", "=".repeat(60));
    println!("  公式: a / b = 10");
    println!("  输入: b = 0");
    println!("  预期: 节点变 Purple，不崩溃");
    println!();

    let eq_c = parse_simple_eq("a / b = 10").expect("解析失败");
    let mut graph_c = NebulaGraph::new();
    let node_c = graph_c.add_node(eq_c);

    let mut sched_c = Scheduler::new(graph_c);

    // 输入 b = 0（触发除零）
    sched_c.env.insert((node_c, "b".to_string()), 0.0);

    // 不应该崩溃
    sched_c.step();

    let state = &sched_c.graph.nodes[0].state;
    println!("  节点状态: {:?}", state);

    let c_ok = match state {
        NodeState::Purple => {
            println!("  ✅ 测试 C 通过: 节点正确进入 Purple 状态");
            true
        }
        NodeState::Green => {
            println!("  ❌ 测试 C 失败: 节点为 Green（除零未被检测）");
            false
        }
        _ => {
            println!("  ❌ 测试 C 失败: 状态={:?}（预期 Purple）", state);
            false
        }
    };
    all_pass &= c_ok;
    println!();

    // ============================================================
    // 汇总
    // ============================================================
    println!("{}", "=".repeat(60));
    if all_pass {
        println!("  🎉 全部压力测试通过！");
    } else {
        println!("  ❌ 部分测试失败，请查看上方详情");
    }
    println!("{}", "=".repeat(60));

    if !all_pass {
        std::process::exit(1);
    }
}
