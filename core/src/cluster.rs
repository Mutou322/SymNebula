/// ClusterSolver — 多节点全局耦合方程组求解器
///
/// 数学本质：
///   ClusterSolver = Graph → Global Equation System F(X)=0 → Newton Solver
///
/// 把多个 Node 的约束拼成一个统一的非线性方程组，用 Newton 迭代全局求解。
/// 与 SolverManager 的区别：
///   SolverManager    — 单节点局部求解，每个 Node 独立解自己的公式
///   ClusterSolver    — 多节点全局耦合求解，跨节点变量同时迭代
///
/// 工程安全：
///   - 所有数值输出过 ensure_finite 检查
///   - 奇异 Jacobian → 返回错误，不 panic
///   - 收敛检查 → 残差阈值控制

use crate::guard::num::ensure_finite;
use crate::solver::Matrix;

/// 单变量约束函数 F_i(X)
type Constraint = Box<dyn Fn(&[f64]) -> f64>;

/// 方程组求解结果
#[derive(Debug, Clone)]
pub struct ClusterResult {
    pub x: Vec<f64>,
    pub residual: f64,
    pub converged: bool,
    pub iterations: usize,
}

/// 全局耦合方程组求解器
pub struct ClusterSolver {
    pub constraints: Vec<Constraint>,
    pub x: Vec<f64>,
    pub eps: f64,
    pub tol: f64,
    pub max_iter: usize,
    pub damping: f64,
}

impl ClusterSolver {
    pub fn new(x: Vec<f64>) -> Self {
        ClusterSolver {
            constraints: Vec::new(),
            x,
            eps: 1e-6,
            tol: 1e-9,
            max_iter: 100,
            damping: 0.1,
        }
    }

    pub fn add_constraint(&mut self, f: Constraint) {
        self.constraints.push(f);
    }

    pub fn eval_f(&self) -> Vec<f64> {
        self.constraints.iter().map(|f| f(&self.x)).collect()
    }

    /// 执行一步 Newton 迭代（带阻尼）
    pub fn step(&mut self) -> Result<bool, &'static str> {
        let n = self.x.len();
        let m = self.constraints.len();
        if m == 0 || n == 0 {
            return Ok(true);
        }

        let f0 = self.eval_f();

        // 检查数值合法性
        for &v in &f0 {
            ensure_finite(v)?;
        }

        let mut j = Matrix::new(m, n);

        // 数值 Jacobian：在原地扰动 x[i]，计算 F，恢复
        for i in 0..n {
            let orig = self.x[i];
            self.x[i] += self.eps;
            let f1 = self.eval_f();
            self.x[i] = orig;

            for k in 0..m {
                let diff = (f1[k] - f0[k]) / self.eps;
                j.set(k, i, ensure_finite(diff)?);
            }
        }

        // 解 J * dx = -F
        let rhs: Vec<f64> = f0.iter().map(|v| -v).collect();

        match j.solve(rhs) {
            Ok(dx) => {
                for i in 0..n {
                    self.x[i] += dx[i] * self.damping;
                    ensure_finite(self.x[i])?;
                }
                let new_f = self.eval_f();
                let residual = new_f.iter().map(|v| v.abs()).sum::<f64>() / (m as f64);
                Ok(residual < self.tol)
            }
            Err(_) => Err("singular Jacobian"),
        }
    }

    /// 执行多步迭代，返回收敛结果
    pub fn solve(&mut self) -> ClusterResult {
        let mut converged = false;
        let mut iterations = 0;

        for i in 0..self.max_iter {
            match self.step() {
                Ok(true) => {
                    converged = true;
                    iterations = i + 1;
                    break;
                }
                Ok(false) => {
                    iterations = i + 1;
                }
                Err(_) => {
                    iterations = i + 1;
                    break;
                }
            }
        }

        let residual = self.eval_f().iter().map(|v| v.abs()).sum::<f64>()
            / (self.constraints.len().max(1) as f64);

        ClusterResult {
            x: self.x.clone(),
            residual,
            converged,
            iterations,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_3node_coupled_system() {
        // x + y = 10,  x = z^2,  z + y = 1
        let mut solver = ClusterSolver::new(vec![1.0, 1.0, 1.0]);
        solver.add_constraint(Box::new(|v| v[0] + v[1] - 10.0));
        solver.add_constraint(Box::new(|v| v[0] - v[2] * v[2]));
        solver.add_constraint(Box::new(|v| v[2] + v[1] - 1.0));
        solver.damping = 0.05;
        solver.max_iter = 200;

        let result = solver.solve();

        println!(
            "3-node: x={:.4} y={:.4} z={:.4}  ok={}  res={:.2e}  iter={}",
            result.x[0], result.x[1], result.x[2],
            result.converged, result.residual, result.iterations
        );

        assert!((result.x[0] + result.x[1] - 10.0).abs() < 1.0);
        assert!((result.x[0] - result.x[2] * result.x[2]).abs() < 1.0);
        assert!((result.x[2] + result.x[1] - 1.0).abs() < 1.0);
    }

    #[test]
    fn test_linear_2node() {
        // 2x + 3y = 7,  4x - y = 1
        let mut solver = ClusterSolver::new(vec![0.0, 0.0]);
        solver.add_constraint(Box::new(|v| 2.0 * v[0] + 3.0 * v[1] - 7.0));
        solver.add_constraint(Box::new(|v| 4.0 * v[0] - v[1] - 1.0));
        solver.damping = 1.0;
        solver.max_iter = 50;

        let result = solver.solve();
        assert!(result.converged);
        assert!((result.x[0] - 5.0 / 7.0).abs() < 1e-4);
        assert!((result.x[1] - 13.0 / 7.0).abs() < 1e-4);
    }

    #[test]
    fn test_empty_system() {
        let mut solver = ClusterSolver::new(vec![1.0, 2.0]);
        let result = solver.solve();
        assert!(result.converged);
    }

    #[test]
    fn test_singular_system_does_not_panic() {
        // x + y = 5, 2x + 2y = 10（线性相关）
        let mut solver = ClusterSolver::new(vec![1.0, 1.0]);
        solver.add_constraint(Box::new(|v| v[0] + v[1] - 5.0));
        solver.add_constraint(Box::new(|v| 2.0 * v[0] + 2.0 * v[1] - 10.0));

        let result = solver.solve();
        println!("Singular: x={:.4} y={:.4}  ok={}", result.x[0], result.x[1], result.converged);
        // 不会 panic 即可
    }
}
