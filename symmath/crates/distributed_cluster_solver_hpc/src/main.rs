//! Phase 5 HPC — 分布式 Cluster 求解器入口
//!
//! 初始化 4 个 Cluster 的曲率跳跃 DAG，启动 GPU Batch Newton + 异步边界同步。

use std::sync::Arc;
use distributed_cluster_solver_hpc::*;
use distributed_cluster_solver_hpc::cluster::Cluster;
use distributed_cluster_solver_hpc::scheduler::Scheduler;

/// 构建 4 个 cluster，各给不同初始 κ
fn build_clusters(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Vec<Cluster> {
    let guesses = [0.1_f32, 0.3, 0.5, 0.7];
    guesses
        .iter()
        .enumerate()
        .map(|(id, &k)| {
            let dag = build_curvature_dag(k);
            let nodes = dag_to_nodes(&dag);
            Cluster::new(
                id,
                nodes,
                dag.variable_nodes,
                dag.constraint_nodes,
                dag.constraint_rhs,
                device,
                layout,
            )
        })
        .collect()
}

fn main() {
    println!("═══ Phase 5 HPC — 分布式 Cluster GPU Newton Solver ═══");
    println!("4 clusters × 7-node curvature DAG, ring sync, global convergence\n");

    // ── GPU 初始化 ──
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(
        &wgpu::RequestAdapterOptions::default(),
    ))
    .expect("no GPU adapter");

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: None,
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
        },
        None,
    ))
    .expect("failed to create device");

    let device = Arc::new(device);
    let queue = Arc::new(queue);

    // ── Shader & Pipeline ──
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("hpc_newton"),
        source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!("shader.wgsl"))),
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("hpc_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("hpc_pipeline_layout"),
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });

    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("hpc_newton_pipeline"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: "main",
        compilation_options: wgpu::PipelineCompilationOptions::default(),
    });

    // ── Clusters ──
    let clusters = build_clusters(&device, &bind_group_layout);

    // ── Scheduler ──
    let mut scheduler = Scheduler::new(
        clusters,
        device,
        queue,
        Arc::new(pipeline),
        Arc::new(bind_group_layout),
    );

    // ── 求解 ──
    pollster::block_on(scheduler.run(20, 1e-3));

    // ── 验证 ──
    println!("\n═══ 验证 ═══");
    for (i, cluster) in scheduler.clusters.iter().enumerate() {
        let kappa = cluster.nodes[0].value;
        let t = cluster.nodes[6].value;
        println!(
            "  Cluster {i}: κ = {kappa:.6}  t = {t:.4}s  |t − TARGET| = {:.2e}",
            (t - TARGET).abs()
        );
    }
    let all_ok = scheduler.clusters.iter().all(|c| {
        let t = c.nodes[6].value;
        (t - TARGET).abs() < 0.5
    });
    if all_ok {
        println!("✓ 所有 cluster 收敛至曲率跳跃解");
    } else {
        println!("✗ 部分 cluster 未收敛");
    }
}
