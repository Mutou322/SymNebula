use sym_nebula_compute::{
    types::{Cluster, RuntimeSnapshot},
    AdaptiveScheduler,
};

fn main() {
    println!("{}", "=".repeat(55));
    println!("  SymNebula Adaptive Scheduler Demo");
    println!("{}", "=".repeat(55));

    let clusters = vec![
        Cluster { id: 1, num_nodes: 10_000 },
        Cluster { id: 2, num_nodes: 500_000 },
        Cluster { id: 3, num_nodes: 5_000 },
    ];

    // 假设 GPU 可用
    let mut scheduler = AdaptiveScheduler::new(true);

    let snapshot = RuntimeSnapshot { tick: 0 };

    println!();
    println!("  GPU_THRESHOLD = 50,000 nodes");
    println!();

    for cluster in &clusters {
        let result = scheduler.solve_cluster(cluster, &snapshot);
        println!(
            "  Cluster {} ({:>6} nodes) → {:>6?}  [{}]",
            cluster.id,
            cluster.num_nodes,
            scheduler.mode,
            if result == sym_nebula_compute::SolverResult::Success {
                "OK"
            } else {
                "FAIL"
            }
        );
    }

    println!();
    println!("{}", "=".repeat(55));
}
