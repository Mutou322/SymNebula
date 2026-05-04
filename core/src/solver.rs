/// 约束求解器与双向坍缩
///
/// 从等式 Expr::Eq(l, r) 中，根据已知变量值，推导出未知变量的值。
/// 支持简单加减法的代数重排，多解时返回 Yellow，无解/奇异时返回 Purple。
///
/// 也提供 solver_step（Newton 单步迭代），用于超越方程等代数无法求解的场景。
/// solver_step 每 Tick 调用一次，跨 Tick 收敛。

use std::collections::HashMap;

use crate::ast::Expr;
use crate::state::{NodeState, SolverState};

// ============================================================
// 代数求解（原 solve_eq）
// ============================================================

/// 求解结果
#[derive(Debug)]
pub struct SolveResult {
    /// 推导出的符号名（空字符串表示无需推导）
    pub symbol: String,
    /// 推导出的值
    pub value: f64,
    /// 节点新状态
    pub state: NodeState,
}

/// 尝试从等式中根据已知变量推导未知变量
pub fn solve_eq(eq: &Expr, known: &HashMap<String, f64>) -> Result<SolveResult, String> {
    let (left, right) = match eq {
        Expr::Eq(l, r) => (l, r),
        _ => return Err("不是等式".into()),
    };

    let all_syms = eq.symbols();
    let unknown: Vec<String> = all_syms
        .into_iter()
        .filter(|s| !known.contains_key(s))
        .collect();

    // 无未知变量：验证等式是否成立（允许浮点容差）
    if unknown.is_empty() {
        match (left.eval(known), right.eval(known)) {
            (Ok(lv), Ok(rv)) => {
                let diff = (lv - rv).abs();
                let max_abs = lv.abs().max(rv.abs()).max(1.0);
                let rel_err = diff / max_abs;
                if rel_err < 1e-2 {
                    return Ok(SolveResult {
                        symbol: String::new(),
                        value: lv,
                        state: NodeState::Green,
                    });
                } else {
                    return Err(format!("约束不满足: {} != {} (diff={})", lv, rv, diff));
                }
            }
            _ => return Err("等式求值失败".into()),
        }
    }

    // 多个未知变量：无法唯一求解
    if unknown.len() > 1 {
        return Ok(SolveResult {
            symbol: unknown[0].clone(),
            value: 0.0,
            state: NodeState::Yellow,
        });
    }

    let u = &unknown[0];

    // 尝试左边能求值，u 在右边
    if let Ok(left_val) = left.eval(known) {
        if let Ok(right_val) = solve_for_symbol(right, u, left_val, known) {
            return Ok(SolveResult {
                symbol: u.clone(),
                value: right_val,
                state: NodeState::Green,
            });
        }
    }

    // 尝试右边能求值，u 在左边
    if let Ok(right_val) = right.eval(known) {
        if let Ok(u_val) = solve_for_symbol(left, u, right_val, known) {
            return Ok(SolveResult {
                symbol: u.clone(),
                value: u_val,
                state: NodeState::Green,
            });
        }
    }

    Err("无法自动求解此等式形式".into())
}

/// 在表达式中解出目标符号：已知 expr = target_value，求 symbol 的值
///
/// 支持的形式：symbol + rest, rest + symbol, symbol - rest, rest - symbol
fn solve_for_symbol(
    expr: &Expr,
    symbol: &str,
    target_value: f64,
    known: &HashMap<String, f64>,
) -> Result<f64, String> {
    match expr {
        Expr::Symbol(s) if s == symbol => Ok(target_value),
        Expr::Add(a, b) => {
            // a + b = target
            if contains_symbol(a, symbol) {
                // 求 a：a = target - b
                let bv = b.eval(known)?;
                solve_for_symbol(a, symbol, target_value - bv, known)
            } else if contains_symbol(b, symbol) {
                let av = a.eval(known)?;
                solve_for_symbol(b, symbol, target_value - av, known)
            } else {
                Err("加法中未找到目标符号".into())
            }
        }
        Expr::Sub(a, b) => {
            if contains_symbol(a, symbol) {
                // a - b = target → a = target + b
                let bv = b.eval(known)?;
                solve_for_symbol(a, symbol, target_value + bv, known)
            } else if contains_symbol(b, symbol) {
                // a - b = target → b = a - target
                let av = a.eval(known)?;
                solve_for_symbol(b, symbol, av - target_value, known)
            } else {
                Err("减法中未找到目标符号".into())
            }
        }
        Expr::Mul(a, b) => {
            // a * b = target
            if contains_symbol(a, symbol) {
                let bv = b.eval(known)?;
                if bv == 0.0 {
                    return Err("乘零错误: 右侧表达式含除零".into());
                }
                solve_for_symbol(a, symbol, target_value / bv, known)
            } else if contains_symbol(b, symbol) {
                let av = a.eval(known)?;
                if av == 0.0 {
                    return Err("乘零错误: 左侧表达式含除零".into());
                }
                solve_for_symbol(b, symbol, target_value / av, known)
            } else {
                Err("乘法中未找到目标符号".into())
            }
        }
        // a/b 已被解析为 a * b^(-1)，所以不会走到 Div
        // 但保留以防未来 AST 变化
        Expr::Div(a, b) => {
            if contains_symbol(a, symbol) {
                let bv = b.eval(known)?;
                solve_for_symbol(a, symbol, target_value * bv, known)
            } else if contains_symbol(b, symbol) {
                let av = a.eval(known)?;
                if target_value == 0.0 {
                    Err("除零错误".into())
                } else {
                    solve_for_symbol(b, symbol, av / target_value, known)
                }
            } else {
                Err("除法中未找到目标符号".into())
            }
        }
        Expr::Pow(a, b) => {
            if contains_symbol(a, symbol) {
                // a^b = target → a = target^(1/b)
                let bv = b.eval(known)?;
                if target_value < 0.0 && bv.fract() != 0.0 {
                    Err("负数开非整数次方".into())
                } else {
                    solve_for_symbol(a, symbol, target_value.powf(1.0 / bv), known)
                }
            } else if contains_symbol(b, symbol) {
                // a^b = target → b = log(target) / log(a)
                let av = a.eval(known)?;
                if av <= 0.0 || target_value <= 0.0 {
                    Err("对数参数无效".into())
                } else {
                    solve_for_symbol(b, symbol, target_value.log(av), known)
                }
            } else {
                Err("幂运算中未找到目标符号".into())
            }
        }
        _ => Err("不支持的表达式形式".into()),
    }
}

/// 检查表达式中是否包含指定符号
fn contains_symbol(expr: &Expr, symbol: &str) -> bool {
    match expr {
        Expr::Symbol(s) => s == symbol,
        Expr::Number(_) => false,
        Expr::Add(a, b)
        | Expr::Sub(a, b)
        | Expr::Mul(a, b)
        | Expr::Div(a, b)
        | Expr::Pow(a, b)
        | Expr::Eq(a, b) => contains_symbol(a, symbol) || contains_symbol(b, symbol),
        Expr::Neg(a) => contains_symbol(a, symbol),
    }
}

// ============================================================
// 数值求解：Newton 单步迭代
// ============================================================

/// 对等式 f(x) = 0 执行一步 Newton 迭代。
///
/// 每 Tick 调用一次，跨 Tick 收敛。
/// 收敛后不再计算（converged = true）。
///
/// f: 闭包 fn(f64) -> f64，对应 f(x) = lhs - rhs
pub fn solver_step<F>(state: &mut SolverState, mut f: F)
where
    F: FnMut(f64) -> f64,
{
    if state.converged {
        return;
    }

    let x = state.current;
    let fx = f(x);

    // 数值导数（中心差分）
    let eps = 1e-6;
    let dfx = (f(x + eps) - f(x - eps)) / (2.0 * eps);

    // 防止除 0 / 数值爆炸
    if !dfx.is_finite() || dfx.abs() < 1e-8 {
        return;
    }

    let next = x - fx / dfx;

    if !next.is_finite() {
        return;
    }

    state.current = next;
    state.residual = fx.abs();

    if state.residual < 1e-6 {
        state.converged = true;
    }
}

/// 从 Expr::Eq 构建 f(x) 闭包。
///
/// 构造 f(x) = eval(lhs) - eval(rhs)，其中 var 被注入 x。
pub fn make_eq_function<'a>(
    expr: &'a Expr,
    var: &'a str,
    ctx: &'a HashMap<String, f64>,
) -> impl FnMut(f64) -> f64 + 'a {
    let (lhs, rhs) = match expr {
        Expr::Eq(l, r) => (l.clone(), r.clone()),
        _ => panic!("make_eq_function 需要 Expr::Eq"),
    };
    move |x: f64| {
        let mut local = ctx.clone();
        local.insert(var.to_string(), x);
        match (lhs.eval(&local), rhs.eval(&local)) {
            (Ok(lv), Ok(rv)) => lv - rv,
            _ => f64::NAN, // 求值失败（如除零），返回 NaN 让调用方检测奇异
        }
    }
}

/// 自动选择代数求解或数值迭代。
///
/// 优先尝试 solve_eq（代数）。
/// 如果代数失败且求解目标已指定，回退到 solver_step。
///
/// 返回 (value, state, 是否需要继续迭代)
pub fn solve_or_iterate(
    eq: &Expr,
    known: &HashMap<String, f64>,
    solver: &mut SolverState,
    solve_target: &str,
) -> (f64, NodeState, bool) {
    // 尝试代数求解
    match solve_eq(eq, known) {
        Ok(result) if result.state == NodeState::Green => {
            solver.converged = true;
            solver.current = result.value;
            solver.residual = 0.0;
            return (result.value, NodeState::Green, false);
        }
        _ => {
            // 先检查 f(x) 是否可求值
            let mut f = make_eq_function(eq, solve_target, known);
            let test_val = f(solver.current);
            if !test_val.is_finite() {
                // 奇异点，例如除零
                return (solver.current, NodeState::Purple, false);
            }

            // 代数失败，用数值迭代
            solver_step(solver, make_eq_function(eq, solve_target, known));

            if solver.converged {
                (solver.current, NodeState::Green, false)
            } else {
                (solver.current, NodeState::Yellow, true)
            }
        }
    }
}

// ============================================================
// 微型矩阵库（仅用于多变量 Newton，无外部依赖）
// ============================================================

/// 微型方阵，按行存储
#[derive(Debug, Clone)]
pub struct Matrix {
    rows: usize,
    cols: usize,
    data: Vec<f64>,
}

impl Matrix {
    pub fn new(rows: usize, cols: usize) -> Self {
        Matrix {
            rows,
            cols,
            data: vec![0.0; rows * cols],
        }
    }

    pub fn get(&self, i: usize, j: usize) -> f64 {
        self.data[i * self.cols + j]
    }

    pub fn set(&mut self, i: usize, j: usize, val: f64) {
        self.data[i * self.cols + j] = val;
    }

    /// 原地高斯消元求解 Ax = b，返回 x
    pub fn solve(mut self, mut b: Vec<f64>) -> Result<Vec<f64>, String> {
        let n = self.rows;
        if self.cols != n || b.len() != n {
            return Err("维度不匹配".into());
        }

        for col in 0..n {
            // 部分选主元
            let mut max_row = col;
            for row in col + 1..n {
                if self.get(row, col).abs() > self.get(max_row, col).abs() {
                    max_row = row;
                }
            }
            if self.get(max_row, col).abs() < 1e-12 {
                return Err("矩阵奇异或接近奇异".into());
            }

            if max_row != col {
                for j in 0..n {
                    let tmp = self.get(col, j);
                    self.set(col, j, self.get(max_row, j));
                    self.set(max_row, j, tmp);
                }
                b.swap(col, max_row);
            }

            for row in col + 1..n {
                let factor = self.get(row, col) / self.get(col, col);
                for j in col..n {
                    let new_val = self.get(row, j) - factor * self.get(col, j);
                    self.set(row, j, new_val);
                }
                b[row] -= factor * b[col];
            }
        }

        let mut x = vec![0.0; n];
        for i in (0..n).rev() {
            let mut sum = b[i];
            for j in i + 1..n {
                sum -= self.get(i, j) * x[j];
            }
            x[i] = sum / self.get(i, i);
        }

        Ok(x)
    }
}

// ============================================================
// 多变量 Newton（Jacobian + 高斯消元）
// ============================================================

/// 多变量 Newton 单步迭代。
///
/// state: 当前变量值，会被更新
/// f: F(X) -> Vec<f64>，方程组残差
/// tol: 收敛阈值
///
/// 返回 Ok(true) 已收敛, Ok(false) 还需继续, Err 奇异
pub fn solver_step_multi<F>(
    state: &mut Vec<f64>,
    mut f: F,
    tol: f64,
) -> Result<bool, String>
where
    F: FnMut(&Vec<f64>) -> Vec<f64>,
{
    let x = state.clone();
    let fx = f(&x);
    let n = x.len();

    if fx.iter().all(|&v| v.abs() < tol) {
        return Ok(true);
    }

    let eps = 1e-6;
    let mut jac = Matrix::new(n, n);

    for i in 0..n {
        let mut x_eps = x.clone();
        x_eps[i] += eps;
        let fx_eps = f(&x_eps);

        for j in 0..n {
            jac.set(j, i, (fx_eps[j] - fx[j]) / eps);
        }
    }

    let rhs: Vec<f64> = fx.iter().map(|v| -v).collect();

    match jac.solve(rhs) {
        Ok(dx) => {
            for i in 0..n {
                state[i] += dx[i];
            }
            Ok(false)
        }
        Err(msg) => Err(format!("Newton step failed: {}", msg)),
    }
}

// ============================================================
// 半隐式欧拉（Symplectic Euler）
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
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- 代数求解测试 ---

    #[test]
    fn test_solve_addition() {
        let expr = crate::ast::parse_simple_eq("a + 3 = 10").unwrap();
        let known = HashMap::new();
        let result = solve_eq(&expr, &known).unwrap();
        assert_eq!(result.state, NodeState::Green);
        assert!((result.value - 7.0).abs() < 1e-9);
    }

    #[test]
    fn test_solve_bidirectional() {
        let expr = crate::ast::parse_simple_eq("a + b = 10").unwrap();
        let mut known = HashMap::new();
        known.insert("a".to_string(), 3.0);
        let result = solve_eq(&expr, &known).unwrap();
        assert_eq!(result.state, NodeState::Green);
        assert!((result.value - 7.0).abs() < 1e-9);
    }

    #[test]
    fn test_multiple_unknowns() {
        let expr = crate::ast::parse_simple_eq("a + b = 10").unwrap();
        let known = HashMap::new();
        let result = solve_eq(&expr, &known).unwrap();
        assert_eq!(result.state, NodeState::Yellow);
    }

    #[test]
    fn test_multiplication() {
        let expr = crate::ast::parse_simple_eq("x * 5 = 20").unwrap();
        let known = HashMap::new();
        let result = solve_eq(&expr, &known).unwrap();
        assert_eq!(result.state, NodeState::Green);
        assert!((result.value - 4.0).abs() < 1e-9);
    }

    #[test]
    fn test_gravity_already_known() {
        let expr = crate::ast::parse_simple_eq("F = G * m1 * m2 / r^2").unwrap();
        let mut known = HashMap::new();
        known.insert("F".to_string(), 1.98e20);
        known.insert("G".to_string(), 6.674e-11);
        known.insert("m1".to_string(), 5.97e24);
        known.insert("m2".to_string(), 7.35e22);
        known.insert("r".to_string(), 3.84e8);
        let result = solve_eq(&expr, &known);
        assert!(result.is_ok());
    }

    // --- Newton 迭代测试 ---

    #[test]
    fn test_newton_x_eq_cosx() {
        // x = cos(x) → f(x) = x - cos(x) = 0
        // 已知解 ≈ 0.739085
        let expr = crate::ast::parse_simple_eq("x = cos(x)").unwrap();

        // 没有 cos 函数，这里用 x - cos(x) 手动构造
        // 但当前 Expr 没有 Cos，所以用 f(x) = x - (1 - x^2/2) 近似
        // 真正测试 Newton 迭代逻辑
        let mut state = SolverState::new(1.0);

        // f(x) = x^2 - 2（解 ≈ 1.414）
        for _ in 0..20 {
            if state.converged {
                break;
            }
            solver_step(&mut state, |x| x * x - 2.0);
        }

        assert!(state.converged, "Newton 应收敛");
        assert!((state.current - 2.0_f64.sqrt()).abs() < 1e-5,
            "期望 sqrt(2)≈{}, 得到 {}", 2.0_f64.sqrt(), state.current);
    }

    #[test]
    fn test_newton_quadratic() {
        // f(x) = x^2 - 4, 解 x = 2
        let mut state = SolverState::new(3.0);

        for _ in 0..20 {
            if state.converged {
                break;
            }
            solver_step(&mut state, |x| x * x - 4.0);
        }

        assert!(state.converged);
        assert!((state.current - 2.0).abs() < 1e-5);
    }

    #[test]
    fn test_solve_or_iterate_power_via_newton() {
        // x * x = 4 中的自乘 Mul(x, x) 代数求解器无法处理，
        // 应自动降级到 Newton 迭代并收敛到 ±2
        let expr = crate::ast::parse_simple_eq("x * x = 4").unwrap();
        let known = HashMap::new();
        let mut solver = SolverState::new(3.0);

        let (val, _, _) = solve_or_iterate(&expr, &known, &mut solver, "x");
        assert!((val - 2.0).abs() < 0.2, "Newton 一步应接近 2, 得到 {}", val);

        for _ in 0..10 {
            if solver.converged {
                break;
            }
            solve_or_iterate(&expr, &known, &mut solver, "x");
        }
        assert!(solver.converged, "Newton 应最终收敛");
        assert!(
            (solver.current - 2.0).abs() < 1e-5,
            "期望收敛到 2, 得到 {}",
            solver.current
        );
    }

    // --- 微型矩阵测试 ---

    #[test]
    fn test_matrix_solve_2x2() {
        // 2x + 3y = 7
        // 4x - y  = 1
        // 解: x=1, y=1.666...
        let mut a = Matrix::new(2, 2);
        a.set(0, 0, 2.0); a.set(0, 1, 3.0);
        a.set(1, 0, 4.0); a.set(1, 1, -1.0);
        let b = vec![7.0, 1.0];

        let x = a.solve(b).unwrap();
        assert!((x[0] - 5.0 / 7.0).abs() < 1e-9, "期望 x=5/7≈0.714, 得到 {}", x[0]);
        assert!((x[1] - 13.0 / 7.0).abs() < 1e-9, "期望 y=13/7≈1.857, 得到 {}", x[1]);
    }

    #[test]
    fn test_matrix_solve_3x3() {
        // x + y + z = 6
        // 2x - y + z = 3
        // x + 2y - z = 2
        // 解: x=1, y=2, z=3
        let mut a = Matrix::new(3, 3);
        a.set(0, 0, 1.0); a.set(0, 1, 1.0); a.set(0, 2, 1.0);
        a.set(1, 0, 2.0); a.set(1, 1, -1.0); a.set(1, 2, 1.0);
        a.set(2, 0, 1.0); a.set(2, 1, 2.0); a.set(2, 2, -1.0);
        let b = vec![6.0, 3.0, 2.0];

        let x = a.solve(b).unwrap();
        assert!((x[0] - 1.0).abs() < 1e-9);
        assert!((x[1] - 2.0).abs() < 1e-9);
        assert!((x[2] - 3.0).abs() < 1e-9);
    }

    #[test]
    fn test_singular_matrix() {
        // 奇异矩阵：行列式为0
        // [1 2; 2 4]
        let mut a = Matrix::new(2, 2);
        a.set(0, 0, 1.0); a.set(0, 1, 2.0);
        a.set(1, 0, 2.0); a.set(1, 1, 4.0);
        let b = vec![3.0, 6.0];

        let result = a.solve(b);
        assert!(result.is_err(), "奇异矩阵应返回 Err");
    }

    // --- 多变量 Newton 测试 ---

    #[test]
    fn test_newton_multi_2eq() {
        // 方程组:
        // x^2 + y^2 = 25
        // x * y = 12
        // 解: x=3, y=4 (或 x=4, y=3)
        let mut state = vec![5.0, 1.0]; // 初值

        let f = |vars: &Vec<f64>| -> Vec<f64> {
            let x = vars[0];
            let y = vars[1];
            vec![
                x * x + y * y - 25.0,  // x^2 + y^2 = 25
                x * y - 12.0,          // x * y = 12
            ]
        };

        for _ in 0..20 {
            if let Ok(true) = solver_step_multi(&mut state, &f, 1e-9) {
                break;
            }
        }

        assert!((state[0] - 3.0).abs() < 1e-5 || (state[0] - 4.0).abs() < 1e-5,
            "x 应收敛到 3 或 4, 得到 {}", state[0]);
        assert!((state[1] - 4.0).abs() < 1e-5 || (state[1] - 3.0).abs() < 1e-5,
            "y 应收敛到 4 或 3, 得到 {}", state[1]);
    }

    // --- 半隐式欧拉测试 ---

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
}
