// 封闭 Tick 求解器 — 突触传播 + 公式求值 + 状态判定

use sym_nebula_compute::types::NodeStatus;

use crate::{ClusterState, Node};

/// 收集节点的上游输入值（upstream.x_value × synapse.weight）
pub fn gather_inputs(cluster: &ClusterState, node: &Node) -> Vec<f64> {
    node.inputs.iter().map(|input_id| {
        let weight = cluster.synapses.iter()
            .find(|s| s.from == *input_id && s.to == node.id)
            .map(|s| s.weight)
            .unwrap_or(0.0);
        let from_val = cluster.nodes.iter()
            .find(|n| n.id == *input_id)
            .map(|n| n.x_value)
            .unwrap_or(0.0);
        from_val * weight
    }).collect()
}

/// 公式求值：将节点公式应用于输入值，返回新值
pub fn node_formula_eval(node: &Node, inputs: &[f64]) -> f64 {
    let f = node.formula.trim();
    if f.is_empty() || f == "rand" {
        return random_delta() * 5.0;
    }
    // 聚合函数
    if f == "avg" {
        if inputs.is_empty() { return 0.0; }
        return inputs.iter().sum::<f64>() / inputs.len() as f64;
    }
    if f == "sum" {
        return inputs.iter().sum();
    }
    if f == "max" {
        if inputs.is_empty() { return 0.0; }
        return inputs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    }
    if f == "min" {
        if inputs.is_empty() { return 0.0; }
        return inputs.iter().cloned().fold(f64::INFINITY, f64::min);
    }
    // 传统格式：对第一个输入操作
    let base = inputs.first().copied().unwrap_or(0.0);
    if let Some(rhs) = f.strip_prefix("x+") {
        return base + rhs.trim().parse::<f64>().unwrap_or(0.0);
    }
    if let Some(rhs) = f.strip_prefix("x-") {
        return base - rhs.trim().parse::<f64>().unwrap_or(0.0);
    }
    if let Some(rhs) = f.strip_prefix("x=") {
        return rhs.trim().parse::<f64>().unwrap_or(0.0);
    }
    if let Some(rhs) = f.strip_prefix("x*") {
        return base * rhs.trim().parse::<f64>().unwrap_or(0.0);
    }
    if let Some(rhs) = f.strip_prefix("x/") {
        let denom = rhs.trim().parse::<f64>().unwrap_or(1.0);
        if denom != 0.0 { return base / denom; }
        return 0.0;
    }
    // 纯数字
    if let Ok(v) = f.parse::<f64>() {
        return v;
    }
    random_delta() * 5.0
}

fn random_delta() -> f64 {
    rand::random::<f64>() * 0.2 - 0.1
}

/// 对一个集群执行完整封闭 Tick
pub fn compute_cluster_tick(cluster: &mut ClusterState) {
    cluster.temp_x.copy_from_slice(&cluster.x_cluster);

    for (i, node) in cluster.nodes.iter().enumerate() {
        let input_values = gather_inputs(cluster, node);
        let new_value = node_formula_eval(node, &input_values);
        cluster.temp_x[i] = new_value;
    }

    let sum: f64 = cluster.temp_x.iter().sum();
    cluster.status = if sum >= 8.0 {
        NodeStatus::Green
    } else if sum >= 3.0 {
        NodeStatus::Yellow
    } else {
        NodeStatus::Purple
    };

    if cluster.status == NodeStatus::Green {
        cluster.x_cluster.copy_from_slice(&cluster.temp_x);
        for (i, node) in cluster.nodes.iter_mut().enumerate() {
            node.x_value = cluster.x_cluster[i];
        }
    }

    for node in cluster.nodes.iter_mut() {
        node.status = cluster.status;
    }
}

// ============================================================
// 曲率跳跃测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Node, Synapse};

    fn make_node(id: usize, formula: &str, inputs: Vec<usize>) -> Node {
        Node { id, name: format!("n{}", id), position: [0.0; 3], status: NodeStatus::Yellow, x_value: 0.0, formula: formula.into(), inputs, highlighted: false, flash_timer: 0.0 }
    }

    #[test]
    fn curvature_jump_earth_to_kuiper() {
        // 计算图：地球 → 柯伊伯带 曲率跳跃时间
        // t = d_eff / c, d_eff = 40AU × (1-κ)
        let x_cluster_vals = vec![149597870.7, 299792.458, 0.997, 1.0, 0.0, 0.0, 0.0, 0.0];
        let nodes: Vec<Node> = vec![
            make_node(0, "x=149597870.7", vec![]),          // AU (km)
            make_node(1, "x=299792.458", vec![]),            // c (km/s)
            make_node(2, "x=0.997", vec![]),                 // κ
            make_node(3, "x=1", vec![]),                     // 1
            make_node(4, "x-0.997", vec![3]),                // 1-κ
            make_node(5, "x*40", vec![0]),                   // d = 40 AU
            make_node(6, "x*0.003", vec![5]),                // d_eff = d × (1-κ)
            make_node(7, "x/299792.458", vec![6]),           // t = d_eff / c
        ].into_iter().enumerate().map(|(i, mut n)| {
            n.x_value = x_cluster_vals[i]; // sync initial x_value from x_cluster
            n
        }).collect();
        let synapses = vec![
            Synapse { from: 0, to: 5, weight: 1.0 },
            Synapse { from: 3, to: 4, weight: 1.0 },
            Synapse { from: 5, to: 6, weight: 1.0 },
            Synapse { from: 6, to: 7, weight: 1.0 },
        ];

        let mut cluster = ClusterState {
            id: 0, nodes, synapses, x_cluster: x_cluster_vals.clone(), temp_x: x_cluster_vals, status: NodeStatus::Yellow,
        };

        // 当前引擎是 Jacobi 式同步更新：每个 tick 只传播一层。
        // 4 层链需要 3 个 tick
        for _ in 0..5 {
            compute_cluster_tick(&mut cluster);
        }

        let t = cluster.nodes.iter().find(|n| n.id == 7).unwrap().x_value;
        let d_eff = cluster.nodes.iter().find(|n| n.id == 6).unwrap().x_value;
        let d = cluster.nodes.iter().find(|n| n.id == 5).unwrap().x_value;
        println!("\n=== 地球 → 柯伊伯带 曲率跳跃时间 (SymNebula 计算) ===");
        println!("距离: 40 AU");
        println!("d = 40 × AU = {:.2e} km", d);
        println!("压缩系数 κ = 0.997, 1-κ = 0.003");
        println!("d_eff = d × (1-κ) = {:.2e} km", d_eff);
        println!("t = d_eff / c = {:.2} 秒", t);
        println!("t = {:.2} 分", t / 60.0);
        println!("t = {:.4} 小时", t / 3600.0);
        assert!((t - 60.0).abs() < 1.0, "curvature jump time should be ~60s, got {}", t);
    }
}
