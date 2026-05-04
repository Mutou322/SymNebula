/// 半隐式欧拉积分器 (Symplectic Euler)
///
/// 从 NewtonSolver / Tick 中完全抽离的独立积分器模块。
///
/// v_{n+1} = v_n + a_n * dt
/// x_{n+1} = x_n + v_{n+1} * dt
///
/// 相比显式欧拉，能量守恒性从 O(dt) 提升到 O(dt^2)。

use std::collections::HashMap;

use crate::graph::Node;
use crate::solver_trait::Integrator;
use symnebula_macros::safe_integrator;

// ============================================================
// 底层积分函数
// ============================================================

/// 半隐式欧拉积分一步。
///
/// v_{n+1} = v_n + a_n * dt
/// x_{n+1} = x_n + v_{n+1} * dt
///
/// 相比显式欧拉，能量守恒性从 O(dt) 提升到 O(dt^2)。
pub fn symplectic_euler_step(x: &mut f64, v: &mut f64, a: f64, dt: f64) {
    *v += a * dt;
    *x += *v * dt;
}

// ============================================================
// SymplecticEulerIntegrator — 半隐式欧拉
// ============================================================

/// 半隐式欧拉积分器。
///
/// v_{n+1} = v_n + a * dt
/// x_{n+1} = x_n + v_{n+1} * dt
pub struct SymplecticEulerIntegrator;

impl SymplecticEulerIntegrator {
    pub fn new() -> Self {
        SymplecticEulerIntegrator
    }
}

#[safe_integrator(name = "symplectic_euler")]
impl Integrator for SymplecticEulerIntegrator {
    fn step(&self, _node: &Node, ctx: &HashMap<String, f64>, dt: f64)
        -> core::result::Result<std::collections::HashMap<String, f64>, &'static str>
    {
        let v_old = ctx.get("v").copied().unwrap_or(0.0);
        let x_old = ctx.get("x").copied().unwrap_or(0.0);
        let a = ctx.get("a").copied().unwrap_or(0.0);

        let v_new = v_old + a * dt;
        let x_new = x_old + v_new * dt;

        let mut map = HashMap::new();
        map.insert("v".to_string(), v_new);
        map.insert("x".to_string(), x_new);
        map.insert("output".to_string(), (x_new + v_new) / 2.0);
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solver_trait::SolveResult;

    #[test]
    fn test_symplectic_euler() {
        let mut x = 0.0;
        let mut v = 1.0;
        let a = 0.0; // 匀速运动
        let dt = 0.01;

        symplectic_euler_step(&mut x, &mut v, a, dt);
        assert!((v - 1.0).abs() < 1e-9, "速度应不变");
        assert!((x - 0.01).abs() < 1e-9, "x = v * dt = 0.01");
    }

    #[test]
    fn test_symplectic_vs_explicit_energy() {
        // 简谐振动: a = -k*x (k=1)
        // 比较显式欧拉和半隐式欧拉的能量漂移
        let dt = 0.1;
        let steps = 200;

        // 显式欧拉: x_{n+1} = x_n + v_n * dt; v_{n+1} = v_n + a_n * dt
        let mut xe = 1.0;
        let mut ve = 0.0;
        let mut energy_e = Vec::new();

        for _ in 0..steps {
            let a = -xe;
            let v_new = ve + a * dt;
            let x_new = xe + ve * dt; // 显式：用旧速度
            xe = x_new;
            ve = v_new;
            energy_e.push(0.5 * (ve * ve + xe * xe));
        }

        // 半隐式欧拉: v_{n+1} = v_n + a_n * dt; x_{n+1} = x_n + v_{n+1} * dt
        let mut xs = 1.0;
        let mut vs = 0.0;
        let mut energy_s = Vec::new();

        for _ in 0..steps {
            let a = -xs;
            symplectic_euler_step(&mut xs, &mut vs, a, dt);
            energy_s.push(0.5 * (vs * vs + xs * xs));
        }

        // 半隐式欧拉的末态能量漂移应小于显式欧拉
        let drift_e = (energy_e[steps - 1] - energy_e[0]).abs();
        let drift_s = (energy_s[steps - 1] - energy_s[0]).abs();
        assert!(
            drift_s < drift_e,
            "半隐式欧拉能量漂移 ({}) 应小于显式欧拉 ({})",
            drift_s,
            drift_e
        );
    }

    #[test]
    fn test_symplectic_integrator_trait() {
        use crate::ast::Expr;
        use crate::graph::Node;
        use crate::state::NodeState;

        let integrator = SymplecticEulerIntegrator::new();
        let node = Node {
            id: 0,
            formula: Expr::Number(0.0),
            state: NodeState::Gray,
            value: None,
            solve_target: None,
            is_dynamic: true,
        };

        let mut ctx = HashMap::new();
        ctx.insert("x".to_string(), 1.0);
        ctx.insert("v".to_string(), 0.0);
        ctx.insert("a".to_string(), -1.0); // 简谐 a = -x

        let result = integrator.step(&node, &ctx, 0.1);
        if let SolveResult::Converged(map) = result {
            let x_new = map.get("x").unwrap();
            let v_new = map.get("v").unwrap();
            // v1 = 0 + (-1)*0.1 = -0.1
            // x1 = 1 + (-0.1)*0.1 = 0.99
            assert!((x_new - 0.99).abs() < 1e-9, "x 期望 0.99, 得到 {}", x_new);
            assert!((v_new - (-0.1)).abs() < 1e-9, "v 期望 -0.1, 得到 {}", v_new);
        } else {
            panic!("期望 Converged, 得到 {:?}", result);
        }
    }
}
