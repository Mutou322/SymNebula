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
