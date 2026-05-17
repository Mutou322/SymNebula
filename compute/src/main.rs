use sym_nebula_compute::types::{Cluster, NodeStatus, RuntimeSnapshot};
use sym_nebula_compute::tick::tick_step;
use sym_nebula_compute::AdaptiveScheduler;

fn main() {
    println!("{}", "=".repeat(55));
    println!("  SymNebula — Adaptive Tick Simulation");
    println!("{}", "=".repeat(55));

    let mut clusters = vec![
        Cluster {
            id: 1,
            num_nodes: 10_000,
            x_cluster: vec![0.0; 5],
            status: NodeStatus::Yellow,
        },
        Cluster {
            id: 2,
            num_nodes: 500_000,
            x_cluster: vec![1.0; 5],
            status: NodeStatus::Yellow,
        },
        Cluster {
            id: 3,
            num_nodes: 5_000,
            x_cluster: vec![2.0; 5],
            status: NodeStatus::Yellow,
        },
    ];

    let mut scheduler = AdaptiveScheduler::new(true);

    println!();
    for t in 0..5 {
        let snapshot = RuntimeSnapshot { tick: t };
        tick_step(&mut clusters, &snapshot, &mut scheduler);
    }

    println!();
    println!("{}", "=".repeat(55));
}
