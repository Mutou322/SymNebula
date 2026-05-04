/// NebulaMind MVP 验证
///
/// 无 UI 原型，完成完整流程：
///   1. 定义公式 "a + b = 10"
///   2. 解析，识别 a、b 为接线柱
///   3. 创建 Node1（约束节点）和 Node2（常量 5）
///   4. 建立链接 Node2.output → Node1.a
///   5. 执行 scheduler.step()
///   6. 验证 Node1.b 自动变为 5

use nebula_core::ast::{parse_simple_eq, Expr};
use nebula_core::graph::NebulaGraph;
use nebula_core::engine::Scheduler;

fn main() {
    println!("{}", "=".repeat(60));
    println!("  NebulaMind MVP 验证");
    println!("  - 公式: a + b = 10");
    println!("  - 常量: 5");
    println!("  - 链接: 5 -> a");
    println!("  - 预期: b = 5");
    println!("{}", "=".repeat(60));
    println!();

    // Step 1: 解析公式
    println!("[1/5] 解析公式 \"a + b = 10\" ...");
    let eq = parse_simple_eq("a + b = 10")
        .expect("公式解析失败");
    let syms = eq.symbols();
    println!("       变量: {:?}", syms);
    assert!(syms.contains(&"a".to_string()));
    assert!(syms.contains(&"b".to_string()));
    println!("       OK");
    println!();

    // Step 2: 构建星云图
    println!("[2/5] 构建星云图 ...");
    let mut graph = NebulaGraph::new();

    let node1 = graph.add_node(eq);
    println!("       节点1 (约束): id={}, 公式=a+b=10", node1);

    let node2 = graph.add_node(Expr::Number(5.0));
    println!("       节点2 (常量5): id={}", node2);
    println!();

    // Step 3: 建立链接
    println!("[3/5] 建立链接: Node2.output -> Node1.a ...");
    graph.add_edge(node2, "output", node1, "a");
    println!("       OK");
    println!();

    // Step 4: 初始化调度器
    println!("[4/5] 初始化调度器 ...");
    let mut scheduler = Scheduler::new(graph);

    // 初始化 Node2 的输出值为 5
    scheduler.env.insert((node2, "output".to_string()), 5.0);

    // 检查传播
    let a_val = scheduler.get_value(node1, "a");
    println!("       Node1.a = {:?}", a_val);
    println!();

    // Step 5: 执行调度
    println!("[5/5] 执行 scheduler.step() ...");
    scheduler.step();
    println!("       Tick 完成: {}", scheduler.get_status());
    println!();

    // 验证结果
    println!("{}", "─".repeat(60));
    println!("  验证结果:");
    let b_val = scheduler.get_value(node1, "b");
    println!("    Node1.b = {:?}", b_val);

    match b_val {
        Some(v) if (v - 5.0).abs() < 1e-9 => {
            println!();
            println!("  ✅✅✅ MVP 验证通过！");
            println!("  公式 a + b = 10, 给定 a = 5, 系统自动推导出 b = {}", v);
            println!();
        }
        Some(v) => {
            println!();
            println!("  ❌ b = {}，期望 5.0", v);
            std::process::exit(1);
        }
        None => {
            println!();
            println!("  ❌ b 未被推导");
            std::process::exit(1);
        }
    }

    // 打印所有节点状态
    println!("  节点状态:");
    for node in &scheduler.graph.nodes {
        println!("    Node {}: {:?} (value={:?})", node.id, node.state, node.value);
    }
    println!();
}
