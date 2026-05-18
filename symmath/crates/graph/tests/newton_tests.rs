// tests/newton_tests.rs
// Phase 3 — Jacobian + Newton Solver 集成测试

use std::collections::HashMap;
use symmath_common::ids::ValueId;
use symmath_common::symbol::SymbolTable;
use symmath_graph::graph::build_graph;
use symmath_graph::jacobian::{compute_jacobian, compute_residual};
use symmath_graph::solver::{newton_solve, newton_step, NewtonStatus};
use symmath_ir::builder::IRBuilder;
use symmath_ir::node::IRNode;

// ============================================================
// 辅助：2×2 线性系统 x + y = 10, x - y = 2
// ============================================================

fn build_linear_2x2(
    symbols: &mut SymbolTable,
) -> (IRBuilder, HashMap<&'static str, ValueId>) {
    let x_sym = symbols.intern("x");
    let y_sym = symbols.intern("y");

    let mut ir = IRBuilder::new();
    let v_x = ir.push(IRNode::LoadVar(x_sym));
    let v_y = ir.push(IRNode::LoadVar(y_sym));
    let v_ten = ir.push(IRNode::Const(10.0));
    let v_two = ir.push(IRNode::Const(2.0));
    let v_sum = ir.push(IRNode::Add(v_x, v_y));
    let v_diff = ir.push(IRNode::Sub(v_x, v_y));
    let _v_eq1 = ir.push(IRNode::Eq(v_sum, v_ten));
    let _v_eq2 = ir.push(IRNode::Eq(v_diff, v_two));

    let mut idx = HashMap::new();
    idx.insert("x", v_x);
    idx.insert("y", v_y);
    idx.insert("sum", v_sum);
    idx.insert("diff", v_diff);

    (ir, idx)
}

// ============================================================
// 辅助：曲率跳跃约束系统 — 已知 AU, c, t_target，求解 κ
// ============================================================

fn build_curvature_constraint(
    symbols: &mut SymbolTable,
) -> (IRBuilder, HashMap<&'static str, ValueId>) {
    let k_sym = symbols.intern("kappa");

    let mut ir = IRBuilder::new();
    let v_au = ir.push(IRNode::Const(149597870.7));
    let v_c = ir.push(IRNode::Const(299792.458));
    let v_one = ir.push(IRNode::Const(1.0));
    let v_forty = ir.push(IRNode::Const(40.0));
    let v_t_target = ir.push(IRNode::Const(59.88));
    let v_k = ir.push(IRNode::LoadVar(k_sym));
    let v_one_minus_k = ir.push(IRNode::Sub(v_one, v_k));
    let v_d = ir.push(IRNode::Mul(v_au, v_forty));
    let v_d_eff = ir.push(IRNode::Mul(v_d, v_one_minus_k));
    let v_t = ir.push(IRNode::Div(v_d_eff, v_c));
    let _v_eq = ir.push(IRNode::Eq(v_t, v_t_target));

    let mut idx = HashMap::new();
    idx.insert("kappa", v_k);
    idx.insert("t", v_t);
    idx.insert("d", v_d);
    idx.insert("deff", v_d_eff);

    (ir, idx)
}

// ============================================================
// 测试 1: 残差计算 — 解处残差应为 0
// ============================================================

#[test]
fn residual_at_solution() {
    let mut symbols = SymbolTable::new();
    let (ir, idx) = build_linear_2x2(&mut symbols);
    let (mut graph, mapping) = build_graph(&ir);

    // x = 6, y = 4 是精确解
    graph.nodes[mapping[&idx["x"]]].value = Some(6.0);
    graph.nodes[mapping[&idx["y"]]].value = Some(4.0);
    graph.mark_dirty(mapping[&idx["x"]]);
    graph.mark_dirty(mapping[&idx["y"]]);
    graph.tick();

    let residual = compute_residual(&graph);
    assert_eq!(residual.values.len(), 2, "should have 2 constraints");
    assert!(
        residual.norm() < 1e-12,
        "residual should be ~0 at solution, norm={}",
        residual.norm()
    );
}

// ============================================================
// 测试 2: Jacobian 矩阵 — 验证 ∂f_i/∂x_j
// ============================================================

#[test]
fn jacobian_2x2() {
    let mut symbols = SymbolTable::new();
    let (ir, idx) = build_linear_2x2(&mut symbols);
    let (mut graph, mapping) = build_graph(&ir);

    graph.nodes[mapping[&idx["x"]]].value = Some(0.0);
    graph.nodes[mapping[&idx["y"]]].value = Some(0.0);
    graph.mark_dirty(mapping[&idx["x"]]);
    graph.mark_dirty(mapping[&idx["y"]]);
    graph.tick();

    let j = compute_jacobian(&graph);
    assert_eq!(j.n_vars, 2);
    assert_eq!(j.n_cons, 2);

    // f₁ = x + y - 10 → ∂f₁/∂x = 1, ∂f₁/∂y = 1
    assert!((j.rows[0][0] - 1.0).abs() < 1e-12, "∂f₁/∂x should be 1");
    assert!((j.rows[0][1] - 1.0).abs() < 1e-12, "∂f₁/∂y should be 1");

    // f₂ = x - y - 2 → ∂f₂/∂x = 1, ∂f₂/∂y = -1
    assert!((j.rows[1][0] - 1.0).abs() < 1e-12, "∂f₂/∂x should be 1");
    assert!((j.rows[1][1] - (-1.0)).abs() < 1e-12, "∂f₂/∂y should be -1");
}

// ============================================================
// 测试 3: Newton 求解 2×2 线性系统
// ============================================================

#[test]
fn newton_linear_2x2() {
    let mut symbols = SymbolTable::new();
    let (ir, idx) = build_linear_2x2(&mut symbols);
    let (mut graph, mapping) = build_graph(&ir);

    // 初始猜测 (0, 0)
    graph.nodes[mapping[&idx["x"]]].value = Some(0.0);
    graph.nodes[mapping[&idx["y"]]].value = Some(0.0);
    graph.mark_dirty(mapping[&idx["x"]]);
    graph.mark_dirty(mapping[&idx["y"]]);

    assert!(newton_solve(&mut graph, 1e-12, 10), "Newton should converge");

    let x = graph.nodes[mapping[&idx["x"]]].value.unwrap();
    let y = graph.nodes[mapping[&idx["y"]]].value.unwrap();
    assert!((x - 6.0).abs() < 1e-10, "x should be 6, got {}", x);
    assert!((y - 4.0).abs() < 1e-10, "y should be 4, got {}", y);
}

// ============================================================
// 测试 4: Newton 从非平凡初始值开始
// ============================================================

#[test]
fn newton_linear_from_non_trivial_guess() {
    let mut symbols = SymbolTable::new();
    let (ir, idx) = build_linear_2x2(&mut symbols);
    let (mut graph, mapping) = build_graph(&ir);

    // 初始猜测 (5, 3) — 已满足 x - y = 2，但不满足 x + y = 10
    graph.nodes[mapping[&idx["x"]]].value = Some(5.0);
    graph.nodes[mapping[&idx["y"]]].value = Some(3.0);
    graph.mark_dirty(mapping[&idx["x"]]);
    graph.mark_dirty(mapping[&idx["y"]]);

    assert!(newton_solve(&mut graph, 1e-12, 10), "Newton should converge");

    let x = graph.nodes[mapping[&idx["x"]]].value.unwrap();
    let y = graph.nodes[mapping[&idx["y"]]].value.unwrap();
    assert!((x - 6.0).abs() < 1e-10, "x should be 6, got {}", x);
    assert!((y - 4.0).abs() < 1e-10, "y should be 4, got {}", y);
}

// ============================================================
// 测试 5: 曲率跳跃 — 通过 Newton 求解 κ
// ============================================================

#[test]
fn newton_curvature_jump() {
    let mut symbols = SymbolTable::new();
    let (ir, idx) = build_curvature_constraint(&mut symbols);
    let (mut graph, mapping) = build_graph(&ir);

    // 初始猜测 κ = 0.5
    graph.nodes[mapping[&idx["kappa"]]].value = Some(0.5);
    graph.mark_dirty(mapping[&idx["kappa"]]);

    assert!(
        newton_solve(&mut graph, 1e-10, 10),
        "Newton should converge on curvature constraint"
    );

    let kappa = graph.nodes[mapping[&idx["kappa"]]].value.unwrap();
    println!("Newton solved κ = {:.6}", kappa);
    assert!(
        (kappa - 0.997).abs() < 0.001,
        "κ should converge to ~0.997, got {:.6}",
        kappa
    );

    // 验证约束满足：t == 59.88
    let t = graph.nodes[mapping[&idx["t"]]].value.unwrap();
    assert!(
        (t - 59.88).abs() < 0.1,
        "t should be ~59.88s at solution, got {:.2}",
        t
    );
}

// ============================================================
// 测试 6: 奇异 Jacobian 处理
// ============================================================

#[test]
fn singular_system_handling() {
    let mut symbols = SymbolTable::new();
    let x_sym = symbols.intern("x");
    let y_sym = symbols.intern("y");

    let mut ir = IRBuilder::new();
    let v_x = ir.push(IRNode::LoadVar(x_sym));
    let v_y = ir.push(IRNode::LoadVar(y_sym));
    let v_ten = ir.push(IRNode::Const(10.0));
    let v_five = ir.push(IRNode::Const(5.0));
    // 两个相同的约束：x + y = 10 出现两次 — Jacobian 奇异
    let v_sum1 = ir.push(IRNode::Add(v_x, v_y));
    let v_sum2 = ir.push(IRNode::Add(v_x, v_y));
    let _v_eq1 = ir.push(IRNode::Eq(v_sum1, v_ten));
    let _v_eq2 = ir.push(IRNode::Eq(v_sum2, v_five));

    let (mut graph, mapping) = build_graph(&ir);

    graph.nodes[mapping[&v_x]].value = Some(0.0);
    graph.nodes[mapping[&v_y]].value = Some(0.0);
    graph.mark_dirty(mapping[&v_x]);
    graph.mark_dirty(mapping[&v_y]);

    // 奇异系统应返回 false（不收敛）
    let result = newton_solve(&mut graph, 1e-10, 5);
    assert!(!result, "singular system should not converge");
}

// ============================================================
// 测试 7: 无约束系统
// ============================================================

#[test]
fn no_constraints() {
    let mut symbols = SymbolTable::new();
    let x_sym = symbols.intern("x");

    let mut ir = IRBuilder::new();
    let _v_x = ir.push(IRNode::LoadVar(x_sym));

    let (mut graph, _) = build_graph(&ir);

    let residual = compute_residual(&graph);
    assert_eq!(residual.values.len(), 0, "no constraints → empty residual");
    assert!(residual.norm() < 1e-15, "empty residual norm should be 0");

    let j = compute_jacobian(&graph);
    assert_eq!(j.n_cons, 0, "no constraints → empty Jacobian rows");
    assert_eq!(j.n_vars, 1, "one variable");

    // 无约束系统应立即"收敛"
    let status = newton_step(&mut graph, 1e-10);
    assert_eq!(status, NewtonStatus::Converged, "no constraints → converged");
}
