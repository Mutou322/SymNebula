// tests/curvature_jump.rs
// 端到端验证：公式解析 → AST → IR → ConstraintGraph → tick → 结果

use std::collections::HashMap;
use symmath_common::ids::ValueId;
use symmath_common::symbol::SymbolTable;
use symmath_graph::graph::build_graph;
use symmath_ir::builder::IRBuilder;
use symmath_ir::node::IRNode;

/// 辅助：构建曲率跳跃 IR，κ 作为可变的 LoadVar
fn build_curvature_ir(symbols: &mut SymbolTable) -> (IRBuilder, HashMap<&'static str, ValueId>, ValueId, ValueId, ValueId) {
    let mut ir = IRBuilder::new();
    let au_sym = symbols.intern("AU");
    let c_sym = symbols.intern("c");
    let k_sym = symbols.intern("kappa");
    let _one_sym = symbols.intern("one");
    let _forty_sym = symbols.intern("forty");

    let v_au = ir.push(IRNode::LoadVar(au_sym));
    let v_c = ir.push(IRNode::LoadVar(c_sym));
    let v_k = ir.push(IRNode::LoadVar(k_sym));
    let v_one = ir.push(IRNode::Const(1.0));
    let v_one_minus_k = ir.push(IRNode::Sub(v_one, v_k));
    let v_forty = ir.push(IRNode::Const(40.0));
    let v_d = ir.push(IRNode::Mul(v_au, v_forty));
    let v_d_eff = ir.push(IRNode::Mul(v_d, v_one_minus_k));
    let v_t = ir.push(IRNode::Div(v_d_eff, v_c));

    let mut index = HashMap::new();
    index.insert("au", v_au);
    index.insert("c", v_c);
    index.insert("k", v_k);
    index.insert("d", v_d);
    index.insert("deff", v_d_eff);
    index.insert("t", v_t);

    (ir, index, v_d, v_d_eff, v_t)
}

// ============================================================
// 测试 1: 基础链路 — IR → ConstraintGraph → tick → 59.88s
// ============================================================

#[test]
fn curvature_jump_from_ir() {
    // (existing test, see below)
    let mut ir = IRBuilder::new();

    let v_au = ir.push(IRNode::Const(149597870.7));
    let v_c = ir.push(IRNode::Const(299792.458));
    let v_k = ir.push(IRNode::Const(0.997));
    let v_one = ir.push(IRNode::Const(1.0));
    let _v_one_minus_k = ir.push(IRNode::Sub(v_one, v_k));
    let v_forty = ir.push(IRNode::Const(40.0));
    let v_d = ir.push(IRNode::Mul(v_au, v_forty));
    let v_point_003 = ir.push(IRNode::Const(0.003));
    let v_d_eff = ir.push(IRNode::Mul(v_d, v_point_003));
    let v_t = ir.push(IRNode::Div(v_d_eff, v_c));

    let (graph, mapping) = build_graph(&ir);
    let mut graph = graph;
    graph.tick();

    let d_val = graph.nodes[mapping[&v_d]].value;
    let d_eff_val = graph.nodes[mapping[&v_d_eff]].value;
    let t_val = graph.nodes[mapping[&v_t]].value;

    assert!((d_val.unwrap() - 5_983_914_828.0).abs() < 1.0);
    assert!((d_eff_val.unwrap() - 17_951_744.48).abs() < 1.0);
    assert!((t_val.unwrap() - 59.88).abs() < 0.1);
}

// ============================================================
// 测试 2: 增量更新 — 改变 κ → mark_dirty → re-tick → t 变化
// ============================================================

#[test]
fn incremental_update() {
    let mut symbols = SymbolTable::new();
    let (ir, idx, _v_d, _v_deff, v_t) = build_curvature_ir(&mut symbols);

    let (mut graph, mapping) = build_graph(&ir);

    // 设置初始值
    graph.nodes[mapping[&idx["au"]]].value = Some(149597870.7);
    graph.nodes[mapping[&idx["c"]]].value = Some(299792.458);
    graph.nodes[mapping[&idx["k"]]].value = Some(0.997);
    graph.mark_dirty(mapping[&idx["au"]]);
    graph.mark_dirty(mapping[&idx["c"]]);
    graph.mark_dirty(mapping[&idx["k"]]);

    graph.tick();

    let t0 = graph.nodes[mapping[&v_t]].value.unwrap();
    println!("κ=0.997: t = {:.2}s", t0);
    assert!((t0 - 59.88).abs() < 0.1, "initial t should be ~60s");

    // ── 增量更新：κ 从 0.997 → 0.999 ──
    // κ=0.999 → 1-κ=0.001 → d_eff ÷ 3 → t ÷ 3 ≈ 19.96s
    graph.nodes[mapping[&idx["k"]]].value = Some(0.999);
    graph.mark_dirty(mapping[&idx["k"]]);

    // 验证 dirty 传播：k → one_minus_k → d_eff → t
    let dirty = graph.dirty_nodes();
    let dirty_ids: Vec<_> = dirty.iter().map(|id| id).collect();
    println!("  dirty nodes after mark κ: {:?}", dirty_ids);
    assert!(dirty.contains(&mapping[&idx["k"]]), "κ itself should be dirty");
    assert!(dirty.contains(&mapping[&idx["deff"]]), "d_eff depends on κ, should be dirty");
    assert!(dirty.contains(&mapping[&v_t]), "t depends on κ, should be dirty");

    // AU 和 c 是输入节点，不应被 κ 的传播污染
    assert!(!dirty.contains(&mapping[&idx["c"]]), "c unrelated to κ, should NOT be dirty");

    graph.tick();
    let t1 = graph.nodes[mapping[&v_t]].value.unwrap();
    println!("κ=0.999: t = {:.2}s", t1);

    // κ=0.999 → 1-κ=0.001 → 有效航程缩为 1/3 → t 也缩为 ~1/3
    assert!((t1 - 19.96).abs() < 0.1, "t should drop to ~20s with κ=0.999, got {:.2}", t1);
    assert!(graph.dirty_nodes().is_empty(), "all nodes should be clean after tick");

    // ── 再次增量更新：AU 翻倍 → t 翻倍 ──
    graph.nodes[mapping[&idx["au"]]].value = Some(299195741.4); // 2× AU
    graph.mark_dirty(mapping[&idx["au"]]);

    let dirty = graph.dirty_nodes();
    assert!(dirty.contains(&mapping[&idx["d"]]), "d depends on AU");
    assert!(dirty.contains(&mapping[&idx["deff"]]), "d_eff depends on AU via d");
    assert!(dirty.contains(&mapping[&v_t]), "t depends on AU transitively");

    // c 和 k 不应被 AU 污染
    assert!(!dirty.contains(&mapping[&idx["c"]]), "c not affected by AU");
    assert!(!dirty.contains(&mapping[&idx["k"]]), "κ not affected by AU");

    graph.tick();
    let t2 = graph.nodes[mapping[&v_t]].value.unwrap();
    println!("AU×2 + κ=0.999: t = {:.2}s", t2);
    assert!((t2 - 39.92).abs() < 0.2, "double AU → double t, got {:.2}", t2);
}

// ============================================================
// 测试 3: dirty propagation — 标记一个节点，验证 DAG 传播
// ============================================================

#[test]
fn dirty_propagation_dag() {
    let mut symbols = SymbolTable::new();
    let (ir, idx, _v_d, _v_deff, v_t) = build_curvature_ir(&mut symbols);

    let (mut graph, mapping) = build_graph(&ir);

    // 设置所有输入值
    for (key, val) in [("au", 149597870.7), ("c", 299792.458), ("k", 0.997)] {
        let id = mapping[&idx[key]];
        graph.nodes[id].value = Some(val);
        graph.mark_dirty(id);
    }
    graph.tick();
    assert!(graph.dirty_nodes().is_empty(), "all clean after first tick");

    // ── 标记 AU dirty，验证只有下游节点变脏 ──
    let id_au = mapping[&idx["au"]];
    let id_d = mapping[&idx["d"]];
    let id_deff = mapping[&idx["deff"]];
    let id_t = mapping[&v_t];
    let id_c = mapping[&idx["c"]];
    let id_k = mapping[&idx["k"]];

    graph.nodes[id_au].value = Some(100_000_000.0); // change AU
    graph.mark_dirty(id_au);

    let dirty: Vec<_> = graph.dirty_nodes();
    let expected = [id_au, id_d, id_deff, id_t];
    for &e in &expected {
        assert!(dirty.contains(&e), "node {:?} should be dirty after AU change", e);
    }
    // 不应污染
    let clean = [id_c, id_k];
    for &c in &clean {
        assert!(!dirty.contains(&c), "node {:?} should NOT be dirty", c);
    }

    println!("DAG propagation: {} nodes dirty (expected 4)", dirty.len());

    // ── tick 后全部变干净 ──
    graph.tick();
    assert!(graph.dirty_nodes().is_empty(), "all clean after propagation tick");

    // ── 验证结果正确传播 ──
    let t_val = graph.nodes[id_t].value.unwrap();
    println!("AU=1e8, κ=0.997: t = {:.2}s", t_val);
    // d = 1e8 × 40 = 4e9, d_eff = 4e9 × 0.003 = 1.2e7, t = 1.2e7 / 299792.458 ≈ 40.03
    assert!((t_val - 40.03).abs() < 0.1, "t should be ~40.03s with AU=1e8");
}
