// tests/curvature_jump_phase3.rs
// Phase 3 — Jacobian + Newton Solver 在曲率跳跃 DAG 上的完整验证
//
// DAG 结构：
//   κ(Input) → one_minus_k(Sub) ─┐
//                                 ├→ d_eff(Mul) → t(Div) → Eq(t, 59.88)
//   AU(Const) → d(Mul) ──────────┘                    ↑
//   c(Const) ─────────────────────────────────────────┘
//
// 变量：κ | 常量：AU=149597870.7, c=299792.458 | 目标 t=59.88s

use std::collections::HashMap;
use symmath_common::ids::{NodeId, ValueId};
use symmath_common::symbol::SymbolTable;
use symmath_graph::graph::build_graph;
use symmath_graph::jacobian::{
    compute_all_derivatives, compute_jacobian, compute_residual,
};
use symmath_graph::solver::newton_solve;
use symmath_ir::builder::IRBuilder;
use symmath_ir::node::IRNode;

/// 构建曲率跳跃约束系统，返回 (ir_builder, node_idx, mapping)
fn build_system(
    symbols: &mut SymbolTable,
    au_val: f64,
    c_val: f64,
    t_target: f64,
) -> (IRBuilder, HashMap<&'static str, ValueId>) {
    let k_sym = symbols.intern("kappa");

    let mut ir = IRBuilder::new();
    let v_au = ir.push(IRNode::Const(au_val));
    let v_c = ir.push(IRNode::Const(c_val));
    let v_one = ir.push(IRNode::Const(1.0));
    let v_forty = ir.push(IRNode::Const(40.0));
    let v_target = ir.push(IRNode::Const(t_target));

    let v_k = ir.push(IRNode::LoadVar(k_sym));
    let v_one_minus_k = ir.push(IRNode::Sub(v_one, v_k));
    let v_d = ir.push(IRNode::Mul(v_au, v_forty));
    let v_d_eff = ir.push(IRNode::Mul(v_d, v_one_minus_k));
    let v_t = ir.push(IRNode::Div(v_d_eff, v_c));
    let _v_eq = ir.push(IRNode::Eq(v_t, v_target));

    let mut idx = HashMap::new();
    idx.insert("kappa", v_k);
    idx.insert("one_minus_k", v_one_minus_k);
    idx.insert("d", v_d);
    idx.insert("deff", v_d_eff);
    idx.insert("t", v_t);
    (ir, idx)
}

/// 从 ValueId 映射取 NodeId
fn vid(
    mapping: &HashMap<ValueId, NodeId>,
    idx: &HashMap<&'static str, ValueId>,
    key: &'static str,
) -> NodeId {
    mapping[&idx[key]]
}

// ============================================================
// 场景 1: 逐节点 AD 偏导数验证 — 在 κ=0.5 和 κ=0.997 两个工况下
// ============================================================

#[test]
fn verify_per_node_ad_derivatives() {
    let mut symbols = SymbolTable::new();
    let (ir, idx) = build_system(&mut symbols, 149597870.7, 299792.458, 59.88);
    let (mut graph, mapping) = build_graph(&ir);

    // ── 工况 A: κ = 0.5 ──
    let kappa_id = vid(&mapping, &idx, "kappa");
    graph.nodes[kappa_id].value = Some(0.5);
    graph.mark_dirty(kappa_id);
    graph.tick();

    let d = vid(&mapping, &idx, "d");
    let d_val = graph.nodes[d].value.unwrap();
    let c = 299792.458;

    let derivs = compute_all_derivatives(&graph, kappa_id);
    let one_minus_k_id = vid(&mapping, &idx, "one_minus_k");
    let deff_id = vid(&mapping, &idx, "deff");
    let t_id = vid(&mapping, &idx, "t");

    // 解析导数
    // ∂(1-κ)/∂κ = -1
    let d_one_minus_k = derivs[&one_minus_k_id];
    assert!(
        (d_one_minus_k - (-1.0)).abs() < 1e-12,
        "∂(1-κ)/∂κ should be -1 at κ=0.5, got {d_one_minus_k}"
    );

    // ∂d/∂κ = 0（d = 40·AU 与 κ 无关）
    let d_d = derivs[&d];
    assert!(
        d_d.abs() < 1e-12,
        "∂d/∂κ should be 0 at κ=0.5, got {d_d}"
    );

    // ∂d_eff/∂κ = d · ∂(1-κ)/∂κ = -d
    let d_deff = derivs[&deff_id];
    let expected_d_deff = -d_val;
    assert!(
        (d_deff - expected_d_deff).abs() < 1.0,
        "∂d_eff/∂κ should be -d ≈ -5.98e9 at κ=0.5, got {d_deff}"
    );

    // ∂t/∂κ = -d/c
    let d_t = derivs[&t_id];
    let expected_d_t = -d_val / c;
    assert!(
        (d_t - expected_d_t).abs() < 0.001,
        "∂t/∂κ should be -d/c ≈ -19958 at κ=0.5, got {d_t}, expected {expected_d_t}"
    );
    println!("κ=0.5  ∂t/∂κ = {:.6}  (analytical: {:.6})", d_t, expected_d_t);

    // ── 工况 B: κ = 0.997 重新计算 ──
    graph.nodes[kappa_id].value = Some(0.997);
    graph.mark_dirty(kappa_id);
    graph.tick();

    // 再次验证导数（因为是线性系统，导数值应该不变）
    let derivs2 = compute_all_derivatives(&graph, kappa_id);
    let d_t2 = derivs2[&t_id];
    let expected_d_t2 = -d_val / c;
    assert!(
        (d_t2 - expected_d_t2).abs() < 0.001,
        "∂t/∂κ should be constant -d/c even at κ=0.997, got {d_t2}"
    );
    println!("κ=0.997 ∂t/∂κ = {:.6}  (analytical: {:.6})", d_t2, expected_d_t2);
    println!("  (linear system → Jacobian is constant ✓)");
}

// ============================================================
// 场景 2: 脏传播跟踪 — Newton 步中只有相关节点被标记
// ============================================================

#[test]
fn dirty_trace_during_newton_step() {
    let mut symbols = SymbolTable::new();
    let (ir, idx) = build_system(&mut symbols, 149597870.7, 299792.458, 59.88);
    let (mut graph, mapping) = build_graph(&ir);

    let kappa_id = vid(&mapping, &idx, "kappa");

    // 初始猜测 κ = 0.5
    graph.nodes[kappa_id].value = Some(0.5);
    graph.mark_dirty(kappa_id);
    graph.tick();
    assert!(
        graph.dirty_nodes().is_empty(),
        "all clean before Newton step"
    );

    // compute_jacobian 不应污染任何节点（只读）
    let _jacobian = compute_jacobian(&graph);
    assert!(
        graph.dirty_nodes().is_empty(),
        "compute_jacobian is read-only, should not dirty any node"
    );

    // compute_residual 不应污染任何节点
    let _residual = compute_residual(&graph);
    assert!(
        graph.dirty_nodes().is_empty(),
        "compute_residual is read-only, should not dirty any node"
    );

    // ── 执行 Newton 迭代 ──
    // newton_step 内部：tick → residual → jacobian → apply_Δx → mark_dirty → tick
    // 线性系统需要 2 次调用才能收敛到 tol=1e-10
    // （第一次 tick 检查 residual→不收敛→apply Δκ→tick；
    //   第二次 tick 检查 residual→已收敛）
    assert!(
        newton_solve(&mut graph, 1e-10, 5),
        "Newton should converge within 5 iterations"
    );

    let dirty_after = graph.dirty_nodes();
    assert!(
        dirty_after.is_empty(),
        "all nodes clean after newton_step: {:?}",
        dirty_after
    );

    // 验证结果
    let kappa = graph.nodes[kappa_id].value.unwrap();
    assert!(
        (kappa - 0.997).abs() < 0.001,
        "κ should converge to 0.997, got {:.6}",
        kappa
    );

    println!("Newton step dirty trace: ✓");
    println!("  initial κ=0.5 → converged κ={:.6}", kappa);
    println!("  read-only operations do not pollute dirty set ✓");
    println!("  post-step all nodes clean ✓");
}

// ============================================================
// 场景 3: 多初始猜测收敛验证
// ============================================================

#[test]
fn convergence_from_multiple_initial_guesses() {
    let guesses = [0.1, 0.3, 0.5, 0.7, 0.9, 0.0, 1.0];

    for &guess in &guesses {
        let mut symbols = SymbolTable::new();
        let (ir, idx) = build_system(&mut symbols, 149597870.7, 299792.458, 59.88);
        let (mut graph, mapping) = build_graph(&ir);

        let kappa_id = vid(&mapping, &idx, "kappa");
        graph.nodes[kappa_id].value = Some(guess);
        graph.mark_dirty(kappa_id);

        let converged = newton_solve(&mut graph, 1e-10, 10);
        assert!(
            converged,
            "Newton should converge from κ={guess}"
        );

        let kappa = graph.nodes[kappa_id].value.unwrap();
        assert!(
            (kappa - 0.997).abs() < 0.001,
            "κ={guess} → converged to {kappa:.6}, expected 0.997"
        );

        let t_id = vid(&mapping, &idx, "t");
        let t = graph.nodes[t_id].value.unwrap();
        assert!(
            (t - 59.88).abs() < 0.1,
            "κ={guess} → t={t:.2}s, expected 59.88s"
        );

        println!("  κ={guess} → κ={kappa:.6}, t={t:.2}s ✓");
    }
    println!("All 7 initial guesses converged correctly ✓");
}

// ============================================================
// 场景 4: Jacobian 残差变化一致性 — 有限差分验证
// ============================================================

#[test]
fn jacobian_consistency_with_finite_difference() {
    let mut symbols = SymbolTable::new();
    let (ir, idx) = build_system(&mut symbols, 149597870.7, 299792.458, 59.88);
    let (mut graph, mapping) = build_graph(&ir);

    let kappa_id = vid(&mapping, &idx, "kappa");

    // 在工作点 κ=0.5 处测试
    graph.nodes[kappa_id].value = Some(0.5);
    graph.mark_dirty(kappa_id);
    graph.tick();

    // AD Jacobian
    let j_ad = compute_jacobian(&graph);
    let ad_deriv = j_ad.rows[0][0]; // ∂residual/∂κ

    // 有限差分：f(κ+ε) - f(κ-ε) / 2ε
    let eps = 1e-6;

    // f(κ+ε)
    graph.nodes[kappa_id].value = Some(0.5 + eps);
    graph.mark_dirty(kappa_id);
    graph.tick();
    let r_plus = compute_residual(&graph);

    // f(κ-ε)
    graph.nodes[kappa_id].value = Some(0.5 - eps);
    graph.mark_dirty(kappa_id);
    graph.tick();
    let r_minus = compute_residual(&graph);

    // 有限差分导数：∂r/∂κ ≈ (r(κ+ε) - r(κ-ε)) / 2ε
    let fd_deriv = (r_plus.values[0] - r_minus.values[0]) / (2.0 * eps);

    let rel_error = (ad_deriv - fd_deriv).abs() / fd_deriv.abs().max(1.0);
    assert!(
        rel_error < 1e-6,
        "AD Jacobian {ad_deriv:.6} differs from FD {fd_deriv:.6} by {rel_error:.2e}"
    );
    println!("Jacobian consistency at κ=0.5:");
    println!("  AD forward-mode: {ad_deriv:.6}");
    println!("  Finite-diff:     {fd_deriv:.6}");
    println!("  relative error:  {rel_error:.2e} ✓");
}

// ============================================================
// 场景 5: 不同 AU 值下验证 Newton 收敛
// ============================================================

#[test]
fn newton_with_different_parameters() {
    let scenarios = [
        ("Kuiper (40AU)", 149597870.7, 59.88, 0.997),
        ("AU=1e8", 100_000_000.0, 40.03, 0.997),
    ];

    for (name, au_val, t_target, expected_k) in &scenarios {
        let mut symbols = SymbolTable::new();
        let (ir, idx) = build_system(&mut symbols, *au_val, 299792.458, *t_target);
        let (mut graph, mapping) = build_graph(&ir);

        let kappa_id = vid(&mapping, &idx, "kappa");
        graph.nodes[kappa_id].value = Some(0.5);
        graph.mark_dirty(kappa_id);

        let ok = newton_solve(&mut graph, 1e-10, 10);
        assert!(ok, "{name}: Newton should converge");

        let kappa = graph.nodes[kappa_id].value.unwrap();
        assert!(
            (kappa - expected_k).abs() < 0.001,
            "{name}: κ should be {expected_k}, got {kappa:.6}"
        );

        let t_id = vid(&mapping, &idx, "t");
        let t = graph.nodes[t_id].value.unwrap();
        assert!(
            (t - t_target).abs() < 1.0,
            "{name}: t should be {t_target}s, got {t:.2}s"
        );

        println!("  {name}: κ={kappa:.6}, t={t:.2}s ✓");
    }
}
