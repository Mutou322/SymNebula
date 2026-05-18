//! Global convergence monitoring.
//!
//! Functions to compute per-cluster and global residual norms, check
//! convergence, and record iteration history into a `ConvergenceMonitor`.

use crate::ConvergenceMonitor;

/// L2 norm of a single cluster's constraint residuals.
///
/// Computes `sqrt( sum( (values[cons[i]] - rhs[i])^2 ) )`.
pub fn local_residual_norm(
    values: &[f32],
    cons: &[u32],
    rhs: &[f32],
) -> f32 {
    let sum_sq: f32 = cons
        .iter()
        .zip(rhs.iter())
        .map(|(&cn, &r)| {
            let diff = values[cn as usize] - r;
            diff * diff
        })
        .sum();
    sum_sq.sqrt()
}

/// L2 norm of the vector of all cluster residual norms.
///
/// `sqrt( sum( local_norms[i]^2 ) )` -- gives a single scalar measuring
/// the global error across the entire distributed system.
pub fn global_residual_norm(local_norms: &[f32]) -> f32 {
    let sum_sq: f32 = local_norms.iter().map(|n| n * n).sum();
    sum_sq.sqrt()
}

/// Returns `true` when every cluster's local residual norm is below `tol`.
pub fn all_converged(local_norms: &[f32], tol: f32) -> bool {
    local_norms.iter().all(|&n| n < tol)
}

/// Record one iteration of convergence data into the monitor.
///
/// Pushes the per-cluster residual norms, the global norm, and the step-size
/// delta norm onto the history vectors.  Updates `iteration` counter and
/// `converged` flag automatically.
pub fn record_iteration(
    mon: &mut ConvergenceMonitor,
    local_norms: &[f32],
    global_norm: f32,
    delta_norm: f32,
) {
    mon.local_residuals.push(local_norms.to_vec());
    mon.global_residuals.push(global_norm);
    mon.deltas.push(delta_norm);
    mon.iteration = mon.global_residuals.len();
    mon.converged = all_converged(local_norms, mon.tol);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_values() -> Vec<f32> {
        vec![0.0, 1.0, 2.0, 3.0, 4.0]
    }

    #[test]
    fn local_residual_norm_basic() {
        let v = sample_values();
        // cons=[2,4], rhs=[0.0, 0.0] → residuals = [2.0, 4.0]
        let cons = vec![2u32, 4u32];
        let rhs = vec![0.0f32, 0.0f32];
        let norm = local_residual_norm(&v, &cons, &rhs);
        let expected = (2.0f32 * 2.0 + 4.0 * 4.0).sqrt();
        assert!((norm - expected).abs() < 1e-6);
    }

    #[test]
    fn local_residual_norm_zero() {
        let v = sample_values();
        let cons = vec![1u32, 2u32];
        let rhs = vec![1.0f32, 2.0f32];
        let norm = local_residual_norm(&v, &cons, &rhs);
        assert!(norm < 1e-6);
    }

    #[test]
    fn global_residual_norm_orthogonal() {
        // 3-4-5 triangle
        let norms = vec![3.0f32, 4.0f32];
        let g = global_residual_norm(&norms);
        assert!((g - 5.0).abs() < 1e-6);
    }

    #[test]
    fn global_residual_norm_empty() {
        assert!((global_residual_norm(&[]) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn all_converged_true() {
        assert!(all_converged(&[0.01, 0.02, 0.005], 0.1));
    }

    #[test]
    fn all_converged_false() {
        assert!(!all_converged(&[0.01, 0.2, 0.005], 0.1));
    }

    #[test]
    fn record_iteration_stores_history() {
        let mut mon = ConvergenceMonitor::new(0.1);
        record_iteration(&mut mon, &[0.05, 0.03], 0.058, 0.01);
        assert_eq!(mon.iteration, 1);
        assert_eq!(mon.local_residuals.len(), 1);
        assert_eq!(mon.global_residuals.len(), 1);
        assert_eq!(mon.deltas.len(), 1);
        assert!(mon.converged); // 0.05, 0.03 both < 0.1

        record_iteration(&mut mon, &[0.15, 0.03], 0.153, 0.02);
        assert_eq!(mon.iteration, 2);
        assert!(!mon.converged); // 0.15 >= 0.1
    }
}
