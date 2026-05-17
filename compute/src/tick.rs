use crate::types::{Cluster, RuntimeSnapshot};
use crate::AdaptiveScheduler;

/// 执行一个完整 Tick：对所有 cluster 求解、更新状态、输出
pub fn tick_step(clusters: &mut Vec<Cluster>, snapshot: &RuntimeSnapshot, scheduler: &mut AdaptiveScheduler) {
    println!("--- Tick {} ---", snapshot.tick);

    for cluster in clusters.iter_mut() {
        scheduler.solve_cluster(cluster, snapshot);

        let bar = match cluster.status {
            crate::types::NodeStatus::Green => "G",
            crate::types::NodeStatus::Yellow => "Y",
            crate::types::NodeStatus::Purple => "P",
        };
        println!(
            "  Cluster {} [{}] {:>6?} X[0]={:.2}",
            cluster.id, bar, scheduler.mode, cluster.x_cluster[0]
        );
    }
}
