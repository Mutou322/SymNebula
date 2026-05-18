use crate::graph::ConstraintGraph;
use crate::jacobian::{
    collect_variables, compute_jacobian, compute_residual, Jacobian,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NewtonStatus {
    Converged,
    StepTaken,
    Singular,
}

/// 一次 Newton 迭代
///
/// 1. `tick()` 确保值最新
/// 2. 计算残差 F(x)，若 ||F|| < tol 返回 Converged
/// 3. 计算 Jacobian J
/// 4. 解 J·Δx = -F（高斯消元）
/// 5. 更新 Input 节点值：x ← x + Δx
/// 6. `tick()` 传播新值
pub fn newton_step(graph: &mut ConstraintGraph, tol: f64) -> NewtonStatus {
    graph.tick();

    let residual = compute_residual(graph);
    if residual.norm() < tol {
        return NewtonStatus::Converged;
    }

    let jacobian = compute_jacobian(graph);

    // 解 J·Δx = -F
    let b: Vec<f64> = residual.values.iter().map(|v| -v).collect();
    let delta = match solve_linear(&jacobian, &b) {
        Some(d) => d,
        None => return NewtonStatus::Singular,
    };

    // 应用 Δx 到变量节点
    let var_ids = collect_variables(graph);
    for (i, var_id) in var_ids.iter().enumerate() {
        let current = graph.nodes[*var_id].value.unwrap_or(0.0);
        graph.nodes[*var_id].value = Some(current + delta[i]);
        graph.mark_dirty(*var_id);
    }

    graph.tick();

    NewtonStatus::StepTaken
}

/// Newton 法收敛循环
///
/// 反复调用 `newton_step` 直到收敛或达到 `max_iter` 次迭代。
pub fn newton_solve(graph: &mut ConstraintGraph, tol: f64, max_iter: usize) -> bool {
    for _ in 0..max_iter {
        match newton_step(graph, tol) {
            NewtonStatus::Converged => return true,
            NewtonStatus::Singular => return false,
            NewtonStatus::StepTaken => continue,
        }
    }
    false
}

/// 高斯消元（列选主元）解 J·x = b
///
/// 要求方阵：n_cons == n_vars
fn solve_linear(jacobian: &Jacobian, b: &[f64]) -> Option<Vec<f64>> {
    let n = b.len();
    if n == 0 {
        return Some(Vec::new());
    }
    if n != jacobian.n_vars || n != jacobian.n_cons {
        return None;
    }

    let mut a = jacobian.rows.clone();
    let mut rhs = b.to_vec();

    // 前向消去 + 列选主元
    for col in 0..n {
        // 找主元行
        let mut max_row = col;
        let mut max_val = a[col][col].abs();
        for row in (col + 1)..n {
            if a[row][col].abs() > max_val {
                max_val = a[row][col].abs();
                max_row = row;
            }
        }

        if max_val < 1e-15 {
            return None; // 奇异
        }

        if max_row != col {
            a.swap(col, max_row);
            rhs.swap(col, max_row);
        }

        let pivot = a[col][col];

        for row in (col + 1)..n {
            let factor = a[row][col] / pivot;
            for k in col..n {
                a[row][k] -= factor * a[col][k];
            }
            rhs[row] -= factor * rhs[col];
        }
    }

    // 回代
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let mut sum = rhs[i];
        for j in (i + 1)..n {
            sum -= a[i][j] * x[j];
        }
        x[i] = sum / a[i][i];
    }

    Some(x)
}
