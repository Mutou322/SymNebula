// tests/curvature_jump.rs
// 端到端验证：公式解析 → AST → IR → ConstraintGraph → tick → 结果

use symmath_graph::graph::build_graph;
use symmath_ir::builder::IRBuilder;
use symmath_ir::node::IRNode;

/// 手动构建 IR 图验证：AU → d → d_eff → t
#[test]
fn curvature_jump_from_ir() {
    // 1. 用更低层级的方法构建 IR（绕开 parser 的 ast→ir 管道，直接触达核心）
    let mut ir = IRBuilder::new();

    let v_au = ir.push(IRNode::Const(149597870.7)); // AU (km)
    let v_c = ir.push(IRNode::Const(299792.458)); // c (km/s)
    let v_k = ir.push(IRNode::Const(0.997)); // κ
    let v_one = ir.push(IRNode::Const(1.0));
    let _v_one_minus_k = ir.push(IRNode::Sub(v_one, v_k)); // 1 - κ
    let v_forty = ir.push(IRNode::Const(40.0));
    let v_d = ir.push(IRNode::Mul(v_au, v_forty)); // d = 40 × AU
    let v_point_003 = ir.push(IRNode::Const(0.003));
    let v_d_eff = ir.push(IRNode::Mul(v_d, v_point_003)); // d_eff = d × 0.003
    let v_t = ir.push(IRNode::Div(v_d_eff, v_c)); // t = d_eff / c

    // 2. Build graph
    let (graph, mapping) = build_graph(&ir);
    assert!(
        graph.dirty_nodes().len() >= 3,
        "op nodes should be dirty, got {}",
        graph.dirty_nodes().len()
    );

    // 3. Run tick propagation
    let mut graph = graph;
    graph.tick();

    // 4. Verify results
    let d_val = graph.nodes[mapping[&v_d]].value;
    let d_eff_val = graph.nodes[mapping[&v_d_eff]].value;
    let t_val = graph.nodes[mapping[&v_t]].value;

    println!("d  = {:?}", d_val);
    println!("d_eff = {:?}", d_eff_val);
    println!("t  = {:?} seconds", t_val);

    assert!(
        (d_val.unwrap() - 5_983_914_828.0).abs() < 1.0,
        "d should be ~5.98e9, got {:?}",
        d_val
    );
    assert!(
        (d_eff_val.unwrap() - 17_951_744.48).abs() < 1.0,
        "d_eff should be ~1.80e7, got {:?}",
        d_eff_val
    );
    assert!(
        (t_val.unwrap() - 59.88).abs() < 0.1,
        "t should be ~59.88s, got {:?}",
        t_val
    );
}
