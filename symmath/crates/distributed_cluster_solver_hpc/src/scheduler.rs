//! Async scheduler that orchestrates the distributed cluster solve loop.
//!
//! Each iteration:
//! 1. GPU tick on every cluster (sequential, synchronous GPU work).
//! 2. Boundary sync via a ring of async channels (blend neighbour κ values).
//! 3. Rebuild GPU buffers so the next tick uses blended boundary values.
//! 4. Convergence check — stops early if the global residual norm falls below `tol`.

use std::sync::Arc;

use crate::ConvergenceMonitor;
use crate::cluster::Cluster;
use crate::communicator::{InMemoryCommunicator, BoundaryMessage};

/// Adjacency list: `cluster_dag[i]` lists the indices of clusters downstream of `i`.
pub type ClusterDag = Vec<Vec<usize>>;

// ─── Scheduler ─────────────────────────────────────────────────────────────

pub struct Scheduler {
    pub clusters: Vec<Cluster>,
    pub cluster_dag: ClusterDag,
    pub monitor: ConvergenceMonitor,
    pub communicator: InMemoryCommunicator,
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub pipeline: Arc<wgpu::ComputePipeline>,
    pub layout: Arc<wgpu::BindGroupLayout>,
}

impl Scheduler {
    /// Create a scheduler with the given clusters and GPU resources.
    pub fn new(
        clusters: Vec<Cluster>,
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        pipeline: Arc<wgpu::ComputePipeline>,
        layout: Arc<wgpu::BindGroupLayout>,
    ) -> Self {
        let n = clusters.len();
        let communicator = InMemoryCommunicator::new(n);
        Self {
            clusters,
            cluster_dag: build_cluster_dag(n),
            monitor: ConvergenceMonitor::new(1e-6),
            communicator,
            device,
            queue,
            pipeline,
            layout,
        }
    }

    /// Main async solve loop.
    ///
    /// Runs at most `max_iter` Newton iterations.  Exits early if the global
    /// residual L2 norm (over all clusters) falls below `tol`.
    pub async fn run(&mut self, max_iter: usize, tol: f32) {
        for iter in 0..max_iter {
            // ── 1. Tick all clusters (sequential GPU work) ──────────────
            let mut local_norms = Vec::with_capacity(self.clusters.len());
            for cluster in &mut self.clusters {
                let norm = cluster.tick(&self.device, &self.queue, &self.pipeline);
                local_norms.push(norm);
            }

            // ── 2. Boundary sync via async ring ────────────────────────
            //     Each cluster sends its κ (node 0) to the next cluster.
            let n = self.clusters.len();
            for i in 0..n {
                let k = self.clusters[i].nodes[0].value;
                let msg = BoundaryMessage {
                    from_cluster: i,
                    to_cluster: (i + 1) % n,
                    node_updates: vec![(0, k)],
                };
                self.communicator.send(i, msg).await;
            }

            //     Receive from previous neighbour and blend.
            for i in 0..n {
                if let Some(msg) = self.communicator.recv(i).await {
                    let neighbor_k = msg.node_updates[0].1;
                    let own_k = self.clusters[i].nodes[0].value;
                    let blended = (own_k + neighbor_k) * 0.5;
                    self.clusters[i].nodes[0].value = blended;
                }
            }

            // ── 3. Rebuild GPU buffers after boundary update ───────────
            for cluster in &mut self.clusters {
                cluster.rebuild_gpu_buffer(&self.device, &self.layout);
            }

            // ── 4. Convergence check ───────────────────────────────────
            let gnorm = (local_norms.iter().map(|n| n * n).sum::<f32>()).sqrt();
            self.monitor.global_residuals.push(gnorm);
            self.monitor.iteration = iter + 1;
            println!(
                "iter {:3}: global residual = {:.6e}  local = {:.3?}",
                iter, gnorm, local_norms
            );

            if gnorm < tol {
                println!("✓ GLOBAL CONVERGENCE at iteration {}", iter);
                self.monitor.converged = true;
                return;
            }
        }
        println!("✗ Not converged after {} iterations", max_iter);
    }

    /// Simple load balancing: report how many clusters have converged locally.
    ///
    /// In a full implementation this would redistribute nodes from
    /// high-residual clusters to low-residual ones.
    pub fn load_balance(&mut self) {
        let converged_count = self
            .clusters
            .iter()
            .filter(|c| c.local_residual < self.monitor.tol)
            .count();
        let total = self.clusters.len();
        println!(
            "  load balance: {}/{} clusters converged",
            converged_count, total
        );
    }
}

// ─── Free functions ────────────────────────────────────────────────────────

/// Build a ring-topology cluster DAG: each cluster is connected to its
/// immediate neighbour (i → i+1 mod n).
pub fn build_cluster_dag(n: usize) -> ClusterDag {
    (0..n).map(|i| vec![(i + 1) % n]).collect()
}
