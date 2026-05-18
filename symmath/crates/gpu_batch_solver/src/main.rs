// Phase 4 — GPU/SIMD Batch Solver 示例
// 地球→柯伊伯带曲率跳跃链路：t = 40·AU·(1-κ) / c
//
// DAG 拓扑：
//   idx  kind     inputs   meaning
//   0    Input    []       κ（曲率）
//   1    Const    []       1.0
//   2    Sub      [1, 0]   1 − κ
//   3    Const    []       40·AU（≈ 5.98e9 km）
//   4    Mul      [3, 2]   d_eff = d · (1−κ)
//   5    Const    []       c（光速, km/s）
//   6    Div      [4, 5]   t = d_eff / c（秒）
//
// CPU 串行/SIMD批处理 和 GPU Compute Shader 两种实现对照。

// ============================================================
// DAG 定义
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NodeKind {
    Input, // 用户变量
    Const, // 常量折叠
    Sub,   // 二元减法
    Mul,   // 二元乘法
    Div,   // 二元除法
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: NodeKind,
    pub inputs: Vec<usize>, // DAG 上游节点索引
    pub value: f64,
    pub dirty: bool,
}

// ============================================================
// DAG 构建
// ============================================================

/// 构建曲率跳跃 DAG
///
/// κ 作为参数传入，可做参数扫描 / 批量敏感性分析
pub fn build_dag(kappa: f64) -> Vec<Node> {
    let au = 149_597_870.7; // 天文单位 (km)
    let c = 299_792.458; // 光速 (km/s)

    vec![
        // 0: 输入 — κ
        Node {
            kind: NodeKind::Input,
            inputs: vec![],
            value: kappa,
            dirty: true,
        },
        // 1: 常量 1.0
        Node {
            kind: NodeKind::Const,
            inputs: vec![],
            value: 1.0,
            dirty: false,
        },
        // 2: 1 − κ
        Node {
            kind: NodeKind::Sub,
            inputs: vec![1, 0],
            value: 0.0,
            dirty: true,
        },
        // 3: d = 40 AU
        Node {
            kind: NodeKind::Const,
            inputs: vec![],
            value: 40.0 * au,
            dirty: false,
        },
        // 4: d_eff = d * (1−κ)
        Node {
            kind: NodeKind::Mul,
            inputs: vec![3, 2],
            value: 0.0,
            dirty: true,
        },
        // 5: 常量 c
        Node {
            kind: NodeKind::Const,
            inputs: vec![],
            value: c,
            dirty: false,
        },
        // 6: t = d_eff / c
        Node {
            kind: NodeKind::Div,
            inputs: vec![4, 5],
            value: 0.0,
            dirty: true,
        },
    ]
}

// ============================================================
// CPU Tick — 串行 / SIMD 批处理
// ============================================================

/// CPU 串行 tick：按拓扑序遍历，dirty 节点重新求值。
///
/// 扩展为 SIMD 批处理：
/// 对大规模同拓扑 DAG 可用 `packed_simd` / `std::simd` 做 lanes 向量化，
/// 每个 lane 承载一个实例的值，一次指令处理多个 κ 样本。
pub fn cpu_tick(nodes: &mut [Node]) {
    for i in 0..nodes.len() {
        if !nodes[i].dirty {
            continue;
        }
        nodes[i].value = match nodes[i].kind {
            NodeKind::Input | NodeKind::Const => nodes[i].value,
            NodeKind::Sub => {
                let a = nodes[nodes[i].inputs[0]].value;
                let b = nodes[nodes[i].inputs[1]].value;
                a - b
            }
            NodeKind::Mul => {
                let a = nodes[nodes[i].inputs[0]].value;
                let b = nodes[nodes[i].inputs[1]].value;
                a * b
            }
            NodeKind::Div => {
                let a = nodes[nodes[i].inputs[0]].value;
                let b = nodes[nodes[i].inputs[1]].value;
                a / b
            }
        };
        nodes[i].dirty = false;
    }
}

/// 批量 CPU tick：在多个 κ 样本上反复求值。
///
/// 用于敏感性分析：给定 κ 扫描范围，批量计算对应的 t。
/// 可向量化：每个样本是 SIMD lane, 一次 `cpu_tick` 处理 4/8/16 样本。
pub fn batch_cpu(samples: &[f64]) -> Vec<f64> {
    samples
        .iter()
        .map(|&kappa| {
            let mut dag = build_dag(kappa);
            cpu_tick(&mut dag);
            dag[6].value // t
        })
        .collect()
}

// ============================================================
// GPU Compute Shader (WGSL)
// ============================================================

#[cfg(feature = "gpu")]
const SHADER: &str = r#"
struct Node {
    value: f32,
};

@group(0) @binding(0)
var<storage, read_write> nodes: array<Node>;

@compute @workgroup_size(64)
fn main(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    let i = id.x;

    // ── Layer 1: 1 − κ ──
    if (i == 2u) {
        nodes[2].value = nodes[1].value - nodes[0].value;
    }

    workgroupBarrier();

    // ── Layer 2: d_eff = d * (1−κ) ──
    if (i == 4u) {
        nodes[4].value = nodes[3].value * nodes[2].value;
    }

    workgroupBarrier();

    // ── Layer 3: t = d_eff / c ──
    if (i == 6u) {
        nodes[6].value = nodes[4].value / nodes[5].value;
    }
}
"#;

// ============================================================
// GPU 运行时调用
// ============================================================

#[cfg(feature = "gpu")]
fn run_gpu(values: &mut [f32]) -> Vec<f32> {
    use pollster::block_on;
    use wgpu::util::DeviceExt;

    block_on(async {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .expect("no GPU adapter found");
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default(), None)
            .await
            .unwrap();

        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("nodes"),
            contents: bytemuck::cast_slice(values),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("curvature_jump"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(SHADER)),
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("pipeline"),
            layout: None,
            module: &shader,
            entry_point: "main",
        });

        let bind_group_layout = pipeline.get_bind_group_layout(0);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bind_group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("encoder"),
            });
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("compute_pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&pipeline);
            cpass.set_bind_group(0, &bind_group, &[]);
            cpass.dispatch(7, 1, 1); // 7 nodes, 1 workgroup (<=64 threads)
        }

        queue.submit(Some(encoder.finish()));
        device.poll(wgpu::Maintain::Wait);

        let buffer_slice = buffer.slice(..);
        let mapping = buffer_slice
            .map_async(wgpu::MapMode::Read)
            .await
            .expect("GPU buffer mapping failed");
        let data = buffer_slice.get_mapped_range();
        let result: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
        drop(data); // release mapping before unmap
        buffer.unmap();
        result
    })
}

// ============================================================
// 主入口
// ============================================================

fn main() {
    println!("╔══════════════════════════════════════════╗");
    println!("║  曲率跳跃 Batch Solver — Phase 4 演示     ║");
    println!("╚══════════════════════════════════════════╝\n");

    // ── 单样本 CPU ──
    let kappa = 0.997;
    let mut dag = build_dag(kappa);
    cpu_tick(&mut dag);

    println!("DAG: 地球 → 柯伊伯带（κ = {kappa}）");
    println!("  idx  kind         value");
    for (i, n) in dag.iter().enumerate() {
        println!("  {i:>2}   {:<12} {:.4e}", format!("{:?}", n.kind), n.value);
    }
    println!(
        "  →  t = {:.2} s  (expected 59.88 s)\n",
        dag[6].value
    );

    // ── 批量 CPU（参数扫描）──
    let samples: Vec<f64> = (0..=10).map(|i| 0.99 + i as f64 * 0.001).collect();
    let results = batch_cpu(&samples);
    println!("批量 κ 扫描 (κ = 0.990 → 1.000):");
    for (i, &kappa) in samples.iter().enumerate() {
        let t = results[i];
        if i % 3 == 0 || i == samples.len() - 1 {
            println!("  κ = {kappa:.3} → t = {t:.2} s");
        }
    }

    // ── GPU（仅 feature=gpu 时可用）──
    #[cfg(feature = "gpu")]
    {
        println!("\n── GPU Compute ──");
        let mut values: Vec<f32> = dag.iter().map(|n| n.value as f32).collect();
        let gpu_result = run_gpu(&mut values);
        let t_gpu = gpu_result[6];
        println!("  GPU t = {t_gpu:.2} s  (expected 59.88 s)");
    }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_tick_default_kappa() {
        let mut dag = build_dag(0.997);
        cpu_tick(&mut dag);
        let t = dag[6].value;
        assert!((t - 59.88).abs() < 0.1, "t should be ~59.88s, got {t:.2}");
    }

    #[test]
    fn cpu_tick_kappa_0() {
        let mut dag = build_dag(0.0);
        cpu_tick(&mut dag);
        let t = dag[6].value;
        // κ=0 → 1-κ=1 → d_eff = d → t = d/c
        let expected = 40.0 * 149_597_870.7 / 299_792.458;
        assert!((t - expected).abs() < 0.1, "t should be ~19958s, got {t:.2}");
    }

    #[test]
    fn cpu_tick_kappa_0999() {
        let mut dag = build_dag(0.999);
        cpu_tick(&mut dag);
        let t = dag[6].value;
        // κ=0.999 → 1-κ=0.001 → t = 0.001 * d / c ≈ 19.96
        assert!((t - 19.96).abs() < 0.1, "t should be ~19.96s, got {t:.2}");
    }

    #[test]
    fn batch_cpu_matches_individual() {
        let samples = [0.1, 0.3, 0.5, 0.7, 0.9];
        let batch_results = batch_cpu(&samples);

        for (i, &kappa) in samples.iter().enumerate() {
            let mut dag = build_dag(kappa);
            cpu_tick(&mut dag);
            let individual = dag[6].value;
            assert!(
                (batch_results[i] - individual).abs() < 1e-12,
                "batch[{i}] should match individual, κ={kappa}"
            );
        }
    }

    #[test]
    fn all_nodes_computed_after_tick() {
        let mut dag = build_dag(0.997);
        cpu_tick(&mut dag);
        for (i, node) in dag.iter().enumerate() {
            assert!(!node.dirty, "node {i} should be clean after tick");
            assert!(
                node.value.is_finite(),
                "node {i} value should be finite, got {}",
                node.value
            );
        }
    }
}
