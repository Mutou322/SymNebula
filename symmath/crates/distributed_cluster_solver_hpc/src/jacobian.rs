//! Forward AD + Sparse Block Jacobian + DAG evaluation.
//!
//! Provides automatic differentiation for the SoA DAG, Jacobian assembly,
//! dirty-aware forward evaluation, and residual computation.

use crate::{SparseBlock, NO_INPUT, KIND_INPUT, KIND_CONST, KIND_SUB, KIND_MUL, KIND_DIV, KIND_EQ, KIND_ADD};

/// Forward-mode AD: returns derivative of each node w.r.t. `var_node`.
///
/// Iterates all nodes in topological order. For input/const nodes, the seed
/// is 1.0 when `i == var_node` and 0.0 otherwise. For arithmetic nodes the
/// chain rule is applied using the already-computed derivatives of the
/// operands.
pub fn forward_ad(
    kinds: &[u32],
    input0: &[u32],
    input1: &[u32],
    values: &[f32],
    var_node: u32,
) -> Vec<f32> {
    let n = kinds.len();
    let mut deriv = vec![0.0f32; n];

    for i in 0..n {
        match kinds[i] {
            KIND_INPUT | KIND_CONST => {
                deriv[i] = if i as u32 == var_node { 1.0 } else { 0.0 };
            }
            KIND_SUB => {
                let a = input0[i] as usize;
                let b = input1[i] as usize;
                deriv[i] = deriv[a] - deriv[b];
            }
            KIND_ADD => {
                let a = input0[i] as usize;
                let b = input1[i] as usize;
                deriv[i] = deriv[a] + deriv[b];
            }
            KIND_MUL => {
                let a = input0[i] as usize;
                let b = input1[i] as usize;
                // d(u*v) = u*dv + v*du
                deriv[i] = values[a] * deriv[b] + values[b] * deriv[a];
            }
            KIND_DIV => {
                let a = input0[i] as usize;
                let b = input1[i] as usize;
                // d(u/v) = (du*v - u*dv) / v^2
                let denom = values[b] * values[b];
                deriv[i] = (deriv[a] * values[b] - values[a] * deriv[b]) / denom;
            }
            KIND_EQ => {
                let a = input0[i] as usize;
                deriv[i] = deriv[a];
            }
            _ => {}
        }
    }

    deriv
}

/// Build a SparseBlock Jacobian mapping variable nodes to constraint nodes.
///
/// For each variable, forward AD is computed once and reused across all
/// constraint rows.  Entry `J[i][j]` is the derivative of the LHS node of
/// constraint `i` with respect to variable `j`, where the LHS node is
/// `input0[constraint_nodes[i]]`.
pub fn build_block_jacobian(
    kinds: &[u32],
    input0: &[u32],
    input1: &[u32],
    values: &[f32],
    variable_nodes: &[u32],
    constraint_nodes: &[u32],
) -> SparseBlock {
    let n_rows = constraint_nodes.len();
    let n_cols = variable_nodes.len();
    let mut jac = SparseBlock::new(n_rows, n_cols);

    // Precompute forward AD for each variable so it is reused across constraints.
    let ad_cache: Vec<Vec<f32>> = variable_nodes
        .iter()
        .map(|&v| forward_ad(kinds, input0, input1, values, v))
        .collect();

    for (i, &cons_node) in constraint_nodes.iter().enumerate() {
        let lhs_node = input0[cons_node as usize] as usize;
        for (j, deriv) in ad_cache.iter().enumerate() {
            let val = deriv[lhs_node];
            if val != 0.0 {
                jac.col_indices.push(j as u32);
                jac.values.push(val);
            }
        }
        jac.row_offsets[i + 1] = jac.col_indices.len() as u32;
    }

    jac
}

/// Dirty-aware forward evaluation: only recompute nodes whose `dirty` flag
/// is set, then clear the flag.
///
/// The caller is responsible for keeping the node array in topological order
/// so that dependents always appear after their dependencies.
///
/// # Safety against NO_INPUT
///
/// - Kind 0 (INPUT) and 1 (CONST) never access `input0`/`input1` -- their
///   values are set externally or are constant.
/// - Kind 5 (EQ) only reads `input0`; `input1` is typically `NO_INPUT` and
///   is never dereferenced.
pub fn dag_tick(
    values: &mut [f32],
    kinds: &[u32],
    input0: &[u32],
    input1: &[u32],
    dirty: &mut [bool],
) {
    let n = kinds.len();
    for i in 0..n {
        if !dirty[i] {
            continue;
        }
        match kinds[i] {
            KIND_INPUT | KIND_CONST => {
                // Values are set externally; nothing to recompute.
            }
            KIND_SUB => {
                let a = input0[i] as usize;
                let b = input1[i] as usize;
                values[i] = values[a] - values[b];
            }
            KIND_MUL => {
                let a = input0[i] as usize;
                let b = input1[i] as usize;
                values[i] = values[a] * values[b];
            }
            KIND_DIV => {
                let a = input0[i] as usize;
                let b = input1[i] as usize;
                values[i] = values[a] / values[b];
            }
            KIND_EQ => {
                // Eq copies the lhs value; input1 is NO_INPUT by convention.
                let a = input0[i] as usize;
                values[i] = values[a];
            }
            KIND_ADD => {
                let a = input0[i] as usize;
                let b = input1[i] as usize;
                values[i] = values[a] + values[b];
            }
            _ => {}
        }
        dirty[i] = false;
    }
}

/// Recursive DFS: mark `node` dirty and propagate to every downstream
/// consumer that references `node` as `input0` or `input1`.
///
/// Only propagates when the input reference is not `NO_INPUT`.  The
/// early-exit check on `dirty[i]` prevents infinite loops in the presence
/// of cycles (which should not exist in a well-formed DAG, but the guard
/// is cheap).
pub fn mark_dirty(
    dirty: &mut [bool],
    input0: &[u32],
    input1: &[u32],
    node: u32,
) {
    dirty[node as usize] = true;
    let n = dirty.len();
    for i in 0..n {
        if dirty[i] {
            continue;
        }
        let i0 = input0[i];
        let i1 = input1[i];
        if i0 != NO_INPUT && i0 == node {
            mark_dirty(dirty, input0, input1, i as u32);
        }
        if i1 != NO_INPUT && i1 == node {
            mark_dirty(dirty, input0, input1, i as u32);
        }
    }
}

/// Compute residual vector for constraints: `values[node] - rhs` for each
/// constraint node.
///
/// Each `constraint_nodes[i]` identifies the node whose current value is
/// compared to `constraint_rhs[i]`.  Returns one residual per constraint.
pub fn compute_residuals(
    values: &[f32],
    constraint_nodes: &[u32],
    constraint_rhs: &[f32],
) -> Vec<f32> {
    constraint_nodes
        .iter()
        .zip(constraint_rhs.iter())
        .map(|(&cn, &rhs)| values[cn as usize] - rhs)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_curvature_dag;

    /// Verify forward_ad derivative w.r.t. κ (node 0) on the curvature DAG.
    #[test]
    fn forward_ad_curvature() {
        let dag = build_curvature_dag(0.3);
        let deriv = forward_ad(
            &dag.kinds,
            &dag.input0,
            &dag.input1,
            &dag.values,
            0, // w.r.t. κ
        );
        // Node 0 (κ) → seed = 1.0
        assert!((deriv[0] - 1.0).abs() < 1e-6);
        // Node 2 (1-κ) → derivative = -1.0
        assert!((deriv[2] + 1.0).abs() < 1e-6);
    }

    #[test]
    fn residuals_match() {
        let dag = build_curvature_dag(0.5);
        // Node 6 is the travel-time constraint; rhs = TARGET = 59.88
        let res = compute_residuals(&dag.values, &dag.constraint_nodes, &dag.constraint_rhs);
        assert_eq!(res.len(), 1);
        let expected = dag.values[6] - crate::TARGET;
        assert!((res[0] - expected).abs() < 1e-6);
    }

    #[test]
    fn mark_dirty_propagates() {
        // Simple graph: 0=input, 1=const, 2=sub(1,0), 3=mul(2,2)
        let _kinds = vec![KIND_INPUT, KIND_CONST, KIND_SUB, KIND_MUL];
        let input0 = vec![NO_INPUT, NO_INPUT, 1, 2];
        let input1 = vec![NO_INPUT, NO_INPUT, 0, 2];
        let mut dirty = vec![false; 4];

        mark_dirty(&mut dirty, &input0, &input1, 0);
        assert!(dirty[0]); // κ itself
        assert!(!dirty[1]); // constant not downstream
        assert!(dirty[2]); // sub(1,0) → references 0
        assert!(dirty[3]); // mul(2,2) → references 2
    }

    #[test]
    fn dag_tick_recomputes_dirty() {
        let kinds = vec![KIND_INPUT, KIND_CONST, KIND_SUB, KIND_MUL];
        let input0 = vec![NO_INPUT, NO_INPUT, 1, 2];
        let input1 = vec![NO_INPUT, NO_INPUT, 0, 2];
        let mut values = vec![0.3, 1.0, 0.0, 0.0];
        let mut dirty = vec![false; 4];

        // Mark node 0 dirty → propagates to 2 and 3
        mark_dirty(&mut dirty, &input0, &input1, 0);
        // Change input value
        values[0] = 0.7;
        dag_tick(&mut values, &kinds, &input0, &input1, &mut dirty);

        assert!((values[2] - (1.0 - 0.7)).abs() < 1e-6);
        assert!((values[3] - values[2] * values[2]).abs() < 1e-6);
        // No node should still be dirty
        assert!(dirty.iter().all(|&d| !d));
    }

    #[test]
    fn build_block_jacobian_curvature() {
        let dag = build_curvature_dag(0.3);
        let jac = build_block_jacobian(
            &dag.kinds,
            &dag.input0,
            &dag.input1,
            &dag.values,
            &dag.variable_nodes,
            &dag.constraint_nodes,
        );
        assert_eq!(jac.n_rows, 1);
        assert_eq!(jac.n_cols, 1);
        assert_eq!(jac.nnz(), 1);
        // d(travel_time)/dκ should be non-zero
        assert!(jac.values[0].abs() > 0.0);
    }

    #[test]
    fn forward_ad_var_node_mismatch() {
        // Seeding a non-existent variable → all derivs should be zero except
        // the exact match for input/const kind.
        let dag = build_curvature_dag(0.5);
        let deriv = forward_ad(
            &dag.kinds,
            &dag.input0,
            &dag.input1,
            &dag.values,
            999, // non-existent
        );
        // Everything should be zero (no node matches var_node)
        for (i, &d) in deriv.iter().enumerate() {
            if dag.kinds[i] == KIND_INPUT || dag.kinds[i] == KIND_CONST {
                // Not equal to 999, so seed = 0.0
                assert!(d == 0.0, "node {} should be 0.0, got {}", i, d);
            }
        }
    }
}
