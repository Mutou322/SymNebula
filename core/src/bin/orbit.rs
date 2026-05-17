/// SymNebula 带反馈的万有引力轨道模拟
///
/// 一维简化模型：质点在引力作用下从静止落向原点。
/// 使用 Expr eval 模式 + 自环反馈实现欧拉积分。
///
/// 节点拓扑：
///   常量 DT=0.01, eps=1.0, GM=100.0
///   节点0: a = -GM / (x^2 + eps)     ← 引力加速度（等式，求解器解 a）
///   节点1: v + a * DT                 ← 速度积分（纯表达式，eval）
///   节点2: x + v * DT                 ← 位置积分（纯表达式，eval）
///
/// 自环边默认值：v=0, x=10（初始静止在 x=10 处）

use sym_nebula_core::ast::{parse_expression, parse_simple_eq, Expr};
use sym_nebula_core::engine::Scheduler;
use sym_nebula_core::graph::NebulaGraph;
use sym_nebula_core::state::NodeState;
use sym_nebula_core::viz::TickDisplay;

fn main() {
    println!("{}", "=".repeat(65));
    println!("  SymNebula --- 带反馈的万有引力轨道模拟");
    println!("{}", "=".repeat(65));
    println!("  模型: 一维质点受引力从静止落向原点");
    println!("  公式:");
    println!("    a = -GM / (x^2 + eps)      (等式, 求解器)");
    println!("    v + a * DT                  (纯表达式, eval)");
    println!("    x + v * DT                  (纯表达式, eval)");
    println!("  参数: GM=100, eps=1, DT=0.01");
    println!("  初值: x=10, v=0");
    println!();

    // ============================================================
    // 构建星云图
    // ============================================================
    let mut graph = NebulaGraph::new();

    // 常量节点
    let dt_node = graph.add_node(Expr::Number(0.01));
    let eps_node = graph.add_node(Expr::Number(1.0));
    let gm_node = graph.add_node(Expr::Number(100.0));

    // 节点0: a = -GM / (x^2 + eps)  (等式)
    let a_node = graph.add_node(
        parse_simple_eq("a = -GM / (x * x + eps)").expect("加速度公式解析失败"),
    );

    // 节点1: v + a * DT  (纯表达式)
    let v_node = graph.add_node(
        parse_expression("v + a * DT").expect("速度公式解析失败"),
    );

    // 节点2: x + v * DT  (纯表达式)
    let x_node = graph.add_node(
        parse_expression("x + v * DT").expect("位置公式解析失败"),
    );

    // ============================================================
    // 建立链接
    // ============================================================
    graph.add_edge_with_default(gm_node, "output", a_node, "GM", 100.0);
    graph.add_edge_with_default(eps_node, "output", a_node, "eps", 1.0);

    // 常量 DT 连到速度和位置节点
    graph.add_edge_with_default(dt_node, "output", v_node, "DT", 0.01);
    graph.add_edge_with_default(dt_node, "output", x_node, "DT", 0.01);

    // 加速度节点 output -> 速度节点的 a
    graph.add_edge_with_default(a_node, "output", v_node, "a", 0.0);

    // 速度节点自环: output -> v
    graph.add_edge_with_default(v_node, "output", v_node, "v", 0.0);

    // 速度节点 output -> 位置节点的 v
    graph.add_edge_with_default(v_node, "output", x_node, "v", 0.0);

    // 位置节点自环: output -> x
    graph.add_edge_with_default(x_node, "output", x_node, "x", 10.0);

    // 位置节点 output -> 加速度节点的 x
    graph.add_edge_with_default(x_node, "output", a_node, "x", 10.0);

    let mut scheduler = Scheduler::new(graph);
    let mut viz = TickDisplay::new();

    // ============================================================
    // 执行 Tick 循环
    // ============================================================
    println!("  执行 200 个 Tick (模拟时长为 {:.1}s):", 200.0 * 0.01);
    println!();
    println!("  {:>4} {:>10} {:>10} {:>10}  {}", "Tick", "x", "v", "a", "状态");
    println!("  {} {} {} {}  {}", "-".repeat(4), "-".repeat(10), "-".repeat(10), "-".repeat(10), "-".repeat(6));

    let num_ticks = 200;
    let print_interval = 20;
    let viz_interval = 10;

    for _ in 1..=num_ticks {
        scheduler.step();

        if scheduler.tick % viz_interval == 0 {
            viz.record(&scheduler);
        }

        let t = scheduler.tick;
        if t % print_interval == 0 {
            let x = scheduler.get_value(x_node, "output").unwrap_or(0.0);
            let v = scheduler.get_value(v_node, "output").unwrap_or(0.0);
            let a = scheduler.get_value(a_node, "output").unwrap_or(0.0);

            let purple = scheduler.graph.nodes.iter().any(|n| n.state == NodeState::Purple);
            let label = if purple { " !PURPLE!" } else { " 运行" };
            println!("  {:>4} {:>10.4} {:>10.4} {:>10.4}  {}", t, x, v, a, label);
        }
    }

    // ============================================================
    // 最终状态
    // ============================================================
    println!();
    println!("{}", "-".repeat(65));
    let fx = scheduler.get_value(x_node, "output").unwrap_or(0.0);
    let fv = scheduler.get_value(v_node, "output").unwrap_or(0.0);
    let fa = scheduler.get_value(a_node, "output").unwrap_or(0.0);
    println!("  模拟结束 (Tick {})", scheduler.tick);
    println!("  位置 x = {:.6}", fx);
    println!("  速度 v = {:.6}", fv);
    println!("  加速度 a = {:.6}", fa);

    let ek = 0.5 * fv * fv;
    let ep = -100.0 / fx.abs().max(0.001);
    println!("  动能 Ek = {:.4}, 势能 Ep = {:.4}, 总能量 = {:.4}", ek, ep, ek + ep);

    println!();
    viz.render();
    println!();
    println!("{}", "=".repeat(65));
}
