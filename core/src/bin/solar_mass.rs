/// 太阳质量计算器 — 让 SymNebula 自己算
///
/// 不给答案，只给方程和已知量，让引擎的代数求解器推导出 M。
///
/// 物理模型：
///   地球绕太阳近似圆周运动，万有引力 = 向心力
///   G * M * m / r^2 = m * v^2 / r
///   消去地球质量 m：
///   G * M / r^2 = v^2 / r   →   M = v^2 * r / G
///
/// 已知量（不给 M）：
///   G  = 6.674 × 10^{-11}     N·m²/kg²
///   r  = 1.496 × 10^{11}      m（1 AU）
///   T  = 365.25 × 24 × 3600   s（公转周期）
///   π  = 3.141592653589793
///
/// 由引擎自己算 v = 2πr/T，再由引擎解出 M。
use sym_nebula_core::ast::{parse_expression, parse_simple_eq, Expr};
use sym_nebula_core::engine::Scheduler;
use sym_nebula_core::graph::NebulaGraph;
use sym_nebula_core::state::NodeState;

fn main() {
    // ============================================================
    // 物理常量
    // ============================================================
    let G = 6.674e-11;
    let r_earth = 1.496e11; // 1 AU (m)
    let t_earth = 365.25 * 24.0 * 3600.0; // 轨道周期 (s)

    println!("{}", "=".repeat(65));
    println!("  SymNebula --- 太阳质量计算器");
    println!("{}", "=".repeat(65));
    println!();
    println!("  给引擎的已知量:");
    println!("    G  = {:.3e}  N·m²/kg²  (引力常数)", G);
    println!("    r  = {:.3e}  m        (地日距离 1 AU)", r_earth);
    println!("    T  = {:.3e}  s        (公转周期)", t_earth);
    println!();
    println!("  让引擎自己算:");
    println!("    v  = 2πr / T              (轨道速度，表达式求值)");
    println!("    M  = v² × r / G            (太阳质量，代数求解)");
    println!();

    // ============================================================
    // 构建星云图
    // ============================================================
    let mut graph = NebulaGraph::new();

    // 常量节点
    let g_node = graph.add_node(Expr::Number(G));
    let r_node = graph.add_node(Expr::Number(r_earth));
    let t_node = graph.add_node(Expr::Number(t_earth));
    let pi_node = graph.add_node(Expr::Number(std::f64::consts::PI));

    // 速度节点: 2 * PI * r / T（EvalSolver 直接求值）
    let v_node = graph.add_node(
        parse_expression("2 * PI * r / T")
            .expect("速度公式解析失败"),
    );

    // 约束方程: G * M / r^2 = v^2 / r（代数求解，得出 M）
    let eq_node = graph.add_node(
        parse_simple_eq("G * M / r ^ 2 = v ^ 2 / r")
            .expect("约束方程解析失败"),
    );

    // ============================================================
    // 建立突触连接
    // ============================================================
    // 输入到速度节点
    graph.add_edge_with_default(pi_node, "output", v_node, "PI", std::f64::consts::PI);
    graph.add_edge_with_default(r_node, "output", v_node, "r", r_earth);
    graph.add_edge_with_default(t_node, "output", v_node, "T", t_earth);

    // 输入到约束方程节点
    graph.add_edge_with_default(g_node, "output", eq_node, "G", G);
    graph.add_edge_with_default(r_node, "output", eq_node, "r", r_earth);
    graph.add_edge_with_default(v_node, "output", eq_node, "v", 0.0);

    // ============================================================
    // 执行调度
    // ============================================================
    let mut scheduler = Scheduler::new(graph);
    let expected_mass = 1.989e30; // 真实太阳质量 (kg)

    println!("  执行 Tick 计算...");
    println!();
    println!("  {:>4}  {:>20}  {:>12}  {:>6}  {}", "Tick", "M (kg)", "v (m/s)", "状态", "误差");
    println!("  {}  {}  {}  {}  {}", "-".repeat(4), "-".repeat(20), "-".repeat(12), "-".repeat(6), "-".repeat(8));

    for tick in 1..=3 {
        scheduler.step();

        // 读取引擎推导的值
        let m_val = scheduler.get_value(eq_node, "M").unwrap_or(0.0);
        let v_val = scheduler.get_value(v_node, "output").unwrap_or(0.0);
        let error = (m_val - expected_mass).abs() / expected_mass * 100.0;

        // 检查所有节点是否正常
        let all_green = scheduler.graph.nodes.iter().all(|n| n.state == NodeState::Green);
        let status = if all_green { "G" } else { "?" };

        println!(
            "  {:>4}  {:>20.6e}  {:>12.2}  {:>6}  {:.4}%",
            tick, m_val, v_val, status, error
        );
    }

    // ============================================================
    // 结果验证
    // ============================================================
    println!();
    println!("{}", "-".repeat(65));
    let final_m = scheduler.get_value(eq_node, "M").unwrap_or(0.0);
    let final_v = scheduler.get_value(v_node, "output").unwrap_or(0.0);
    let error_pct = (final_m - expected_mass).abs() / expected_mass * 100.0;

    println!();
    println!("  引擎推导结果:");
    println!("    太阳质量 M = {:.6e}  kg", final_m);
    println!("    理论值 M₀  = {:.6e}  kg", expected_mass);
    println!("    相对误差   = {:.4}%", error_pct);
    println!("    轨道速度 v = {:.2}  m/s", final_v);
    println!();

    if error_pct < 5.0 {
        println!("  ✅✅✅ 引擎正确推导出太阳质量！");
    } else {
        println!("  ❌ 结果偏差较大，请检查模型");
    }

    // 打印节点状态
    println!();
    println!("  节点状态:");
    for node in &scheduler.graph.nodes {
        println!("    Node {}: {:?} (formula: {})", node.id, node.state, node.formula);
    }
    println!();
    println!("{}", "=".repeat(65));
}
