// SymNebula Desktop — Pure wgpu + winit + egui
//
// 架构: App (Tick/集群) → sync_buffers → wgpu RenderPass (点+线+egui)
//
// 操作:
//   左键拖拽  → 旋转视角      S  → 搜索节点
//   右键拖拽  → 平移视角      R  → 重置相机
//   滚轮      → 缩放         空格 → 切换自动 Tick
//   左键单击  → 选中节点      Enter → 执行搜索

use glam::{Mat4, Vec3, Vec4};
use std::time::Instant;
use wgpu::util::DeviceExt;
use winit::{
    event::*,
    event_loop::EventLoop,
    keyboard::{KeyCode, PhysicalKey},
    window::Window,
};

use rand::Rng;
use sym_nebula_compute::types::NodeStatus;

mod cluster; // compute_cluster_tick, gather_inputs, node_formula_eval

// ============================================================
// 数据结构
// ============================================================

#[derive(Debug, Clone)]
struct Node {
    id: usize,
    name: String,
    position: [f32; 3],
    status: NodeStatus,
    x_value: f64,
    formula: String,
    inputs: Vec<usize>,
    highlighted: bool,
    flash_timer: f32,
}

#[derive(Debug, Clone)]
struct Synapse {
    from: usize,
    to: usize,
    weight: f64,
}

#[derive(Debug)]
struct ClusterState {
    id: usize,
    nodes: Vec<Node>,
    synapses: Vec<Synapse>,
    x_cluster: Vec<f64>,
    temp_x: Vec<f64>,
    status: NodeStatus,
}

#[derive(Debug)]
struct ClusterCache {
    topology_version: u64,
    clusters: Vec<ClusterState>,
}

struct App {
    tick: u64,
    cache: ClusterCache,
    converged: Vec<bool>,
    just_committed: Vec<bool>,
    auto_tick: bool,
    search_query: String,
    selected_id: Option<usize>,
    next_node_id: usize,
    command_buffer: String,
    ai_bridge: AiBridge,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("tick", &self.tick)
            .field("cache", &self.cache)
            .field("converged", &self.converged)
            .field("just_committed", &self.just_committed)
            .field("auto_tick", &self.auto_tick)
            .field("search_query", &self.search_query)
            .field("selected_id", &self.selected_id)
            .field("next_node_id", &self.next_node_id)
            .field("command_buffer", &self.command_buffer)
            .field("ai_bridge", &self.ai_bridge)
            .finish()
    }
}

impl App {
    fn new() -> Self {
        let mut rng = rand::thread_rng();
        let configs = vec![
            (1usize, 8usize,
             (0..8).map(|i| i as f64 * 0.3).collect::<Vec<f64>>(),
             [ -40.0, 0.0, 0.0 ],
             vec!["avg", "avg", "avg", "avg", "x+0.05", "x+0.05", "avg", "avg"]),
            (2, 8,
             vec![0.5f64; 8],
             [ 0.0, 0.0, 0.0 ],
             vec!["sum", "sum", "sum", "sum", "avg", "avg", "avg", "avg"]),
            (3, 8,
             vec![0.0f64; 8],
             [ 40.0, 0.0, 0.0 ],
             vec!["rand", "rand", "rand", "x+0.2", "x-0.1", "avg", "avg", "avg"]),
        ];

        let mut clusters = Vec::new();
        let mut next_id = 0usize;

        for (cid, count, x_cluster, center, formulas) in &configs {
            let spread = 14.0f32;
            let mut nodes = Vec::new();
            let ids: Vec<usize> = (0..*count).map(|_| { let id = next_id; next_id += 1; id }).collect();

            for (j, &nid) in ids.iter().enumerate() {
                nodes.push(Node {
                    id: nid,
                    name: format!("N{}", nid),
                    position: [
                        center[0] + rng.gen_range(-spread..spread),
                        center[1] + rng.gen_range(-spread..spread),
                        center[2] + rng.gen_range(-spread..spread),
                    ],
                    status: NodeStatus::Yellow,
                    x_value: x_cluster[j],
                    formula: formulas[j].to_string(),
                    inputs: Vec::new(),
                    highlighted: false,
                    flash_timer: 0.0,
                });
            }

            let mut synapses = Vec::new();
            for j in 0..*count {
                let from = ids[j];
                let to_ring = ids[(j + 1) % count];
                let to_cross = ids[(j + 2) % count];
                synapses.push(Synapse { from, to: to_ring, weight: rng.gen_range(0.4..0.9) });
                if (j + 2) % count != (j + 1) % count {
                    synapses.push(Synapse { from, to: to_cross, weight: rng.gen_range(0.2..0.6) });
                }
            }

            for node in &mut nodes {
                node.inputs = synapses.iter()
                    .filter(|s| s.to == node.id)
                    .map(|s| s.from)
                    .collect();
            }

            clusters.push(ClusterState {
                id: *cid,
                nodes,
                synapses,
                x_cluster: x_cluster.clone(),
                temp_x: x_cluster.clone(),
                status: NodeStatus::Yellow,
            });
        }

        let n = clusters.len();
        let next_node_id = next_id;
        Self {
            tick: 0,
            cache: ClusterCache { topology_version: 1, clusters },
            converged: vec![false; n],
            just_committed: vec![false; n],
            auto_tick: true,
            search_query: String::new(),
            selected_id: None,
            next_node_id,
            command_buffer: String::new(),
            ai_bridge: AiBridge::new(),
        }
    }

    fn advance(&mut self, dt: f32) {
        self.tick += 1;
        for (i, cluster) in self.cache.clusters.iter_mut().enumerate() {
            let was_green = self.converged[i];
            cluster::compute_cluster_tick(cluster);
            self.just_committed[i] = !was_green && cluster.status == NodeStatus::Green;
            if self.just_committed[i] {
                self.converged[i] = true;
                println!("✦ Cluster {} converged at tick {}", cluster.id, self.tick);
            }
            if cluster.status != NodeStatus::Green {
                self.converged[i] = false;
            }
        }

        // Flash timer decay
        for cluster in &mut self.cache.clusters {
            for node in &mut cluster.nodes {
                if node.flash_timer > 0.0 {
                    node.flash_timer -= dt;
                    if node.flash_timer <= 0.0 {
                        node.highlighted = false;
                    }
                }
            }
        }
    }

    fn search_node(&mut self, query: &str) {
        let q = query.trim().to_lowercase();
        for cluster in &mut self.cache.clusters {
            for node in &mut cluster.nodes {
                node.highlighted = node.name.to_lowercase().contains(&q)
                    || node.id.to_string() == q;
                if node.highlighted {
                    node.flash_timer = 2.0;
                }
            }
        }
    }

    fn find_node(&self, id: usize) -> Option<&Node> {
        for c in &self.cache.clusters {
            if let Some(n) = c.nodes.iter().find(|n| n.id == id) {
                return Some(n);
            }
        }
        None
    }

    #[allow(dead_code)]
    fn find_node_mut(&mut self, id: usize) -> Option<&mut Node> {
        for c in &mut self.cache.clusters {
            if let Some(n) = c.nodes.iter_mut().find(|n| n.id == id) {
                return Some(n);
            }
        }
        None
    }

    fn find_cluster(&self, id: usize) -> Option<&ClusterState> {
        self.cache.clusters.iter().find(|c| c.id == id)
    }

    fn find_cluster_mut(&mut self, id: usize) -> Option<&mut ClusterState> {
        self.cache.clusters.iter_mut().find(|c| c.id == id)
    }

    fn cluster_id_of(&self, node_id: usize) -> Option<usize> {
        self.cache.clusters.iter().find(|c| c.nodes.iter().any(|n| n.id == node_id)).map(|c| c.id)
    }

    fn add_node(&mut self, name: &str, pos: [f32; 3]) {
        let id = self.next_node_id;
        self.next_node_id += 1;
        let target_cid = self.selected_id
            .and_then(|sid| self.cluster_id_of(sid))
            .unwrap_or(3);
        let cidx = self.cache.clusters.iter().position(|c| c.id == target_cid)
            .unwrap_or(0);
        let cluster = &mut self.cache.clusters[cidx];

        cluster.nodes.push(Node {
            id,
            name: name.to_string(),
            position: pos,
            status: NodeStatus::Yellow,
            x_value: 0.0,
            formula: "avg".to_string(),
            inputs: Vec::new(),
            highlighted: false,
            flash_timer: 0.0,
        });
        cluster.x_cluster.push(0.0);
        cluster.temp_x.push(0.0);
        self.cache.topology_version += 1;
        println!("✦ Added node #{} '{}' to cluster {}", id, name, cluster.id);
    }

    fn add_synapse(&mut self, from: usize, to: usize) {
        for cluster in &mut self.cache.clusters {
            let has_from = cluster.nodes.iter().any(|n| n.id == from);
            let has_to = cluster.nodes.iter().any(|n| n.id == to);
            if has_from && has_to {
                let weight = rand::thread_rng().gen_range(0.3..0.9);
                cluster.synapses.push(Synapse { from, to, weight });
                if let Some(node) = cluster.nodes.iter_mut().find(|n| n.id == to) {
                    if !node.inputs.contains(&from) {
                        node.inputs.push(from);
                    }
                }
                self.cache.topology_version += 1;
                println!("✦ Added synapse #{} → #{} weight={:.2}", from, to, weight);
                return;
            }
        }
        eprintln!("⚠ Cannot add synapse: nodes #{} and #{} not in same cluster", from, to);
    }

    fn execute_command(&mut self, cmd: &str) {
        let parts: Vec<&str> = cmd.trim().split_whitespace().collect();
        if parts.is_empty() {
            return;
        }
        match parts[0] {
            "search" | "s" => {
                if parts.len() > 1 {
                    let query = parts[1..].join(" ");
                    self.search_query = query.clone();
                    self.search_node(&query);
                    println!("🔍 Searching: {}", query);
                }
            }
            "node" | "n" => {
                if parts.len() > 1 {
                    let name = parts[1];
                    let _formula = self.ai_bridge.suggest_formula(name);
                    let pos = [
                        rand::thread_rng().gen_range(-20.0..20.0),
                        rand::thread_rng().gen_range(-20.0..20.0),
                        rand::thread_rng().gen_range(-20.0..20.0),
                    ];
                    self.add_node(name, pos);
                    println!("✦ Node '{}' created via command", name);
                }
            }
            "help" | "h" | "?" => {
                println!("Commands: search <name>, node <name>, help");
            }
            _ => {
                println!("Unknown command: {}", cmd);
            }
        }
    }
}

// ============================================================
// AI Formula Bridge
// ============================================================

struct AiBridge {
    callback: Option<Box<dyn Fn(&str) -> String>>,
}

impl AiBridge {
    fn new() -> Self {
        Self { callback: None }
    }

    fn suggest_formula(&self, prompt: &str) -> String {
        if let Some(cb) = &self.callback {
            cb(prompt)
        } else {
            format!("f(x) = {}", prompt)
        }
    }
}

impl std::fmt::Debug for AiBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiBridge")
            .field("callback", if self.callback.is_some() { &"Some(..)" } else { &"None" })
            .finish()
    }
}

// ============================================================
// GPU 顶点类型
// ============================================================

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct NodeVertex {
    position: [f32; 3],
    energy: f32,
    status: u32,
    highlighted: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SynapseVertex {
    position: [f32; 3],
    color: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view: Mat4,
    proj: Mat4,
}

// ============================================================
// 相机
// ============================================================

struct Camera {
    pitch: f32,
    yaw: f32,
    distance: f32,
    target: Vec3,
}

impl Camera {
    fn new() -> Self {
        Self { pitch: -0.3, yaw: 0.0, distance: 120.0, target: Vec3::ZERO }
    }

    fn view_matrix(&self) -> Mat4 {
        let (sp, cp) = self.pitch.sin_cos();
        let (sy, cy) = self.yaw.sin_cos();
        let eye = Vec3::new(
            self.target.x + self.distance * cy * cp,
            self.target.y + self.distance * sp,
            self.target.z + self.distance * sy * cp,
        );
        Mat4::look_at_rh(eye, self.target, Vec3::Y)
    }
}

// ============================================================
// 颜色 & 波形
// ============================================================

fn status_color(status: NodeStatus) -> [f32; 3] {
    match status {
        NodeStatus::Green => [0.1, 1.0, 0.6],
        NodeStatus::Yellow => [1.0, 0.8, 0.1],
        NodeStatus::Purple => [0.8, 0.2, 1.0],
    }
}

fn status_egui_color(status: NodeStatus) -> egui::Color32 {
    match status {
        NodeStatus::Green => egui::Color32::from_rgb(25, 255, 140),
        NodeStatus::Yellow => egui::Color32::from_rgb(255, 204, 25),
        NodeStatus::Purple => egui::Color32::from_rgb(204, 50, 255),
    }
}

fn status_bar_char(status: NodeStatus) -> &'static str {
    match status {
        NodeStatus::Green => "G",
        NodeStatus::Yellow => "Y",
        NodeStatus::Purple => "P",
    }
}

fn wave_offset(x: f64) -> f32 {
    (x.sin() * 2.0) as f32
}

fn synapse_blend(s1: NodeStatus, s2: NodeStatus, weight: f64) -> [f32; 3] {
    let c1 = status_color(s1);
    let c2 = status_color(s2);
    let bright = (weight * 0.7 + 0.3) as f32;
    [
        (c1[0] + c2[0]) * 0.5 * bright,
        (c1[1] + c2[1]) * 0.5 * bright,
        (c1[2] + c2[2]) * 0.5 * bright,
    ]
}

// ============================================================
// wgpu 渲染状态
// ============================================================

struct OrbitCtrl {
    left_down: bool,
    right_down: bool,
    prev_x: f64,
    prev_y: f64,
}

struct State {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    node_pipeline: wgpu::RenderPipeline,
    synapse_pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    node_buffer: wgpu::Buffer,
    synapse_buffer: wgpu::Buffer,
    camera: Camera,
    orbit: OrbitCtrl,
    window_size: (u32, u32),
    // egui
    egui_ctx: egui::Context,
    egui_renderer: egui_wgpu::Renderer,
    // Input state (accumulated between frames)
    cursor_pos: (f64, f64),
    left_click: bool,
    text_input: String,
    start: Instant,
}

impl State {
    async fn new(window: &'static Window) -> Self {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window).unwrap();
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
        }).await.unwrap();
        let (device, queue) = adapter.request_device(
            &wgpu::DeviceDescriptor { label: None, required_features: wgpu::Features::empty(), required_limits: wgpu::Limits::default() },
            None,
        ).await.unwrap();
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats[0];
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // Shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Nebula Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        // Camera uniform
        let camera = Camera::new();
        let camera_uniform = CameraUniform {
            view: camera.view_matrix(),
            proj: Mat4::perspective_rh_gl(45.0_f32.to_radians(), size.width as f32 / size.height as f32, 0.1, 500.0),
        };
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Camera"),
            contents: bytemuck::cast_slice(&[camera_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let camera_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Camera BGL"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Camera BG"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        // Node pipeline
        let node_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Node Pipeline Layout"),
            bind_group_layouts: &[&camera_bind_group_layout],
            push_constant_ranges: &[],
        });
        let node_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Node Pipeline"),
            layout: Some(&node_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_node",
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<NodeVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x3 },
                        wgpu::VertexAttribute { offset: 12, shader_location: 1, format: wgpu::VertexFormat::Float32 },
                        wgpu::VertexAttribute { offset: 16, shader_location: 2, format: wgpu::VertexFormat::Uint32 },
                        wgpu::VertexAttribute { offset: 20, shader_location: 3, format: wgpu::VertexFormat::Uint32 },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_node",
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::PointList, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        // Synapse pipeline
        let synapse_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Synapse Pipeline Layout"),
            bind_group_layouts: &[&camera_bind_group_layout],
            push_constant_ranges: &[],
        });
        let synapse_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Synapse Pipeline"),
            layout: Some(&synapse_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_synapse",
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<SynapseVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x3 },
                        wgpu::VertexAttribute { offset: 12, shader_location: 1, format: wgpu::VertexFormat::Float32x3 },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_synapse",
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        // Empty buffers (will be resized in sync)
        let node_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Node Buffer"),
            size: 256 * std::mem::size_of::<NodeVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let synapse_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Synapse Buffer"),
            size: 256 * std::mem::size_of::<SynapseVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let egui_ctx = egui::Context::default();
        let egui_renderer = egui_wgpu::Renderer::new(&device, format, None, 1);

        Self {
            surface, device, queue, config,
            node_pipeline, synapse_pipeline,
            camera_buffer, camera_bind_group,
            node_buffer, synapse_buffer,
            camera, orbit: OrbitCtrl { left_down: false, right_down: false, prev_x: 0.0, prev_y: 0.0 },
            window_size: (size.width, size.height),
            egui_ctx, egui_renderer,
            cursor_pos: (0.0, 0.0), left_click: false,
            text_input: String::new(),
            start: Instant::now(),
        }
    }

    fn resize(&mut self, w: u32, h: u32) {
        if w > 0 && h > 0 {
            self.window_size = (w, h);
            self.config.width = w;
            self.config.height = h;
            self.surface.configure(&self.device, &self.config);
        }
    }

    fn sync_buffers(&mut self, app: &App) {
        // Nodes
        let nodes: Vec<NodeVertex> = app.cache.clusters.iter().flat_map(|c| {
            c.nodes.iter().map(|n| NodeVertex {
                position: [n.position[0], n.position[1] + wave_offset(n.x_value), n.position[2]],
                energy: (n.x_value.abs() as f32 * 0.1).min(1.0),
                status: match n.status { NodeStatus::Green => 0, NodeStatus::Yellow => 1, NodeStatus::Purple => 2 },
                highlighted: if n.highlighted { 1 } else { 0 },
            })
        }).collect();
        let n_nodes = nodes.len() as u64;
        if n_nodes * std::mem::size_of::<NodeVertex>() as u64 > self.node_buffer.size() {
            self.node_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Node Buffer"),
                size: (n_nodes * std::mem::size_of::<NodeVertex>() as u64).next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        self.queue.write_buffer(&self.node_buffer, 0, bytemuck::cast_slice(&nodes));

        // Synapses
        let vertices: Vec<SynapseVertex> = app.cache.clusters.iter().flat_map(|c| {
            c.synapses.iter().filter_map(|s| {
                let from = c.nodes.iter().find(|n| n.id == s.from)?;
                let to = c.nodes.iter().find(|n| n.id == s.to)?;
                let color = synapse_blend(from.status, to.status, s.weight);
                Some([
                    SynapseVertex { position: [from.position[0], from.position[1] + wave_offset(from.x_value), from.position[2]], color },
                    SynapseVertex { position: [to.position[0], to.position[1] + wave_offset(to.x_value), to.position[2]], color },
                ])
            }).flatten()
        }).collect();
        let n_syn = vertices.len() as u64;
        if n_syn * std::mem::size_of::<SynapseVertex>() as u64 > self.synapse_buffer.size() {
            self.synapse_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Synapse Buffer"),
                size: (n_syn * std::mem::size_of::<SynapseVertex>() as u64).next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        self.queue.write_buffer(&self.synapse_buffer, 0, bytemuck::cast_slice(&vertices));

        // Camera
        let proj = Mat4::perspective_rh_gl(45.0_f32.to_radians(), self.window_size.0 as f32 / self.window_size.1 as f32, 0.1, 500.0);
        let cu = CameraUniform { view: self.camera.view_matrix(), proj };
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::cast_slice(&[cu]));
    }
}

// ============================================================
// Egui UI
// ============================================================

fn get_node_info(app: &App, nid: usize) -> Option<(usize, [f32; 3], NodeStatus, f64, String, Vec<(usize, f64, NodeStatus)>, Vec<(usize, f64, NodeStatus)>)> {
    let rn = app.find_node(nid)?;
    let cid = app.cluster_id_of(nid)?;
    let c = app.find_cluster(cid)?;
    let inputs = c.synapses.iter()
        .filter(|s| s.to == nid)
        .filter_map(|s| app.find_node(s.from).map(|pn| (s.from, s.weight, pn.status)))
        .collect();
    let outputs = c.synapses.iter()
        .filter(|s| s.from == nid)
        .filter_map(|s| app.find_node(s.to).map(|pn| (s.to, s.weight, pn.status)))
        .collect();
    Some((cid, rn.position, rn.status, rn.x_value, rn.formula.clone(), inputs, outputs))
}

fn build_ui(egui_ctx: &egui::Context, app: &mut App) {
    // ── Panel 1: Runtime ──
    egui::Window::new("SymNebula Runtime").default_width(360.0)
        .show(egui_ctx, |ui| {
            ui.heading("ClusterSolver");
            ui.separator();
            ui.label(format!("Tick: {}", app.tick));
            ui.checkbox(&mut app.auto_tick, "Auto Tick");
            if ui.button("Manual Tick").clicked() { app.advance(0.5); }
            ui.separator();

            for (i, c) in app.cache.clusters.iter().enumerate() {
                let ec = status_egui_color(c.status);
                let avg_x = c.nodes.iter().map(|n| n.x_value).sum::<f64>() / c.nodes.len() as f64;
                let mut line = format!("Cluster {} [{}]  {} nodes  x̄={:.2}",
                    c.id, status_bar_char(c.status), c.nodes.len(), avg_x);
                if app.just_committed[i] { line += " ✦ COMMIT"; }
                else if app.converged[i] { line += " ✓"; }
                ui.colored_label(ec, line);
            }
            ui.separator();

            let mut cnt = [0u32; 3];
            for c in &app.cache.clusters {
                match c.status {
                    NodeStatus::Green => cnt[0] += 1,
                    NodeStatus::Yellow => cnt[1] += 1,
                    NodeStatus::Purple => cnt[2] += 1,
                }
            }
            ui.label(egui::RichText::new(format!("Green:  {}", cnt[0])).color(status_egui_color(NodeStatus::Green)));
            ui.label(egui::RichText::new(format!("Yellow: {}", cnt[1])).color(status_egui_color(NodeStatus::Yellow)));
            ui.label(egui::RichText::new(format!("Purple: {}", cnt[2])).color(status_egui_color(NodeStatus::Purple)));
            ui.separator();
            ui.label("L-Drag→Rotate  R-Drag→Pan  Scroll→Zoom");
            ui.label("S→Search  : →Command  Space→Auto  R→Reset");
            ui.label("N→New Node  M→New Synapse  Click→Select");
        });

    // ── Panel 2: Formula Editor ──
    struct FmtEntry { cid: usize, nid: usize, formula: String, status: NodeStatus, xv: f64 }
    let entries: Vec<FmtEntry> = app.cache.clusters.iter().flat_map(|c| {
        c.nodes.iter().map(|n| FmtEntry {
            cid: c.id, nid: n.id, formula: n.formula.clone(),
            status: c.status, xv: n.x_value,
        })
    }).collect();

    let mut changed = Vec::new();
    egui::Window::new("Formula Editor").default_width(340.0)
        .show(egui_ctx, |ui| {
            egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                for e in &entries {
                    let ec = status_egui_color(e.status);
                    ui.horizontal(|ui| {
                        ui.colored_label(ec, format!("#{}", e.nid));
                        let mut fb = e.formula.clone();
                        ui.add(egui::TextEdit::singleline(&mut fb)
                            .desired_width(140.0).font(egui::TextStyle::Monospace));
                        if fb != e.formula { changed.push((e.cid, e.nid, fb)); }
                        ui.label(format!("x={:.3}", e.xv));
                    });
                }
            });
        });

    for (cid, nid, formula) in &changed {
        if let Some(c) = app.find_cluster_mut(*cid) {
            if let Some(n) = c.nodes.iter_mut().find(|n| n.id == *nid) {
                if n.formula != *formula {
                    n.formula = formula.clone();
                    app.cache.topology_version += 1;
                }
            }
        }
    }

    // ── Panel 3: Node Inspector ──
    if let Some(nid) = app.selected_id {
        let info = get_node_info(app, nid);
        if let Some((cid, pos, status, xv, formula, inputs, outputs)) = info {
            egui::Window::new(format!("Node #{} (Cluster {})", nid, cid)).default_width(320.0)
                .show(egui_ctx, |ui| {
                    ui.separator();
                    ui.label(format!("Pos: ({:.1}, {:.1}, {:.1})", pos[0], pos[1], pos[2]));
                    ui.colored_label(status_egui_color(status), format!("Status: {:?}", status));
                    ui.label(format!("x: {:.4}", xv));
                    ui.label(format!("Formula: {}", formula));
                    if !inputs.is_empty() {
                        ui.separator();
                        ui.label("← Inputs:");
                        for (id, w, s) in &inputs {
                            ui.colored_label(status_egui_color(*s), format!("  #{}  w={:.2}  {:?}", id, w, s));
                        }
                    }
                    if !outputs.is_empty() {
                        ui.separator();
                        ui.label("→ Outputs:");
                        for (id, w, s) in &outputs {
                            ui.colored_label(status_egui_color(*s), format!("  #{}  w={:.2}  {:?}", id, w, s));
                        }
                    }
                });
        }
    }
}

// ============================================================
// 节点拾取
// ============================================================

fn pick_node(app: &App, camera: &Camera, mouse: (f64, f64), size: (u32, u32)) -> Option<usize> {
    let aspect = size.0 as f32 / size.1 as f32;
    let proj = Mat4::perspective_rh_gl(45.0_f32.to_radians(), aspect, 0.1, 500.0);
    let view = camera.view_matrix();

    // Transform point to clip space, then screen space
    fn project(pos: [f32; 3], proj: Mat4, view: Mat4, w: f32, h: f32) -> (f32, f32) {
        let p = Vec4::new(pos[0], pos[1], pos[2], 1.0);
        let clip = proj * view * p;
        if clip.w == 0.0 { return (0.0, 0.0); }
        let ndc_x = clip.x / clip.w;
        let ndc_y = clip.y / clip.w;
        let sx = (ndc_x + 1.0) * 0.5 * w;
        let sy = (1.0 - ndc_y) * 0.5 * h;
        (sx, sy)
    }

    let mut best: Option<(usize, f32)> = None;
    let (mx, my) = mouse;
    for c in &app.cache.clusters {
        for n in &c.nodes {
            let wp = [n.position[0], n.position[1] + wave_offset(n.x_value), n.position[2]];
            let (sx, sy) = project(wp, proj, view, size.0 as f32, size.1 as f32);
            let dx = sx - mx as f32;
            let dy = sy - my as f32;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist < 15.0 && best.as_ref().map_or(true, |&(_, bd)| dist < bd) {
                best = Some((n.id, dist));
            }
        }
    }
    best.map(|(id, _)| id)
}

// ============================================================
// Main
// ============================================================

fn main() {
    let event_loop = EventLoop::new().unwrap();
    let window = Box::leak(Box::new(
        event_loop.create_window(
            Window::default_attributes().with_title("SymNebula Desktop — wgpu + egui")
        ).unwrap()
    ));
    let window: &Window = &*window; // shadow to shared ref

    let mut app = App::new();
    let mut state = pollster::block_on(State::new(window));

    let tick_interval = std::time::Duration::from_millis(500);
    let mut last_tick = Instant::now();

    event_loop.run(move |event, target| {
        match event {
            Event::WindowEvent { event, .. } => {
                match event {
                    WindowEvent::CloseRequested => target.exit(),
                    WindowEvent::RedrawRequested => {
                        // ── Tick ──
                        if app.auto_tick {
                            let now = Instant::now();
                            if now.duration_since(last_tick) >= tick_interval {
                                app.advance(0.5);
                                last_tick = now;
                            }
                        }

                        // ── Handle accumulated text input for search ──
                        if !state.text_input.is_empty() {
                            let q = state.text_input.clone();
                            app.search_query.clone_from(&q);
                            app.search_node(&q);
                            state.text_input.clear();
                        }

                        // ── Node picking on click ──
                        if state.left_click {
                            if !state.egui_ctx.wants_pointer_input() {
                                app.selected_id = pick_node(&app, &state.camera, state.cursor_pos, state.window_size);
                                if app.selected_id.is_none() {
                                    app.search_query.clear();
                                    for c in &mut app.cache.clusters {
                                        for n in &mut c.nodes { n.highlighted = false; }
                                    }
                                }
                            }
                            state.left_click = false;
                        }

                        // ── Egui frame ──
                        let elapsed = state.start.elapsed().as_secs_f64();
                        let raw_input = egui::RawInput {
                            time: Some(elapsed),
                            screen_rect: Some(egui::Rect::from_min_max(
                                egui::pos2(0.0, 0.0),
                                egui::pos2(state.window_size.0 as f32, state.window_size.1 as f32),
                            )),
                            ..Default::default()
                        };
                        state.egui_ctx.begin_frame(raw_input);
                        build_ui(&state.egui_ctx, &mut app);
                        let output = state.egui_ctx.end_frame();
                        let clipped_primitives = state.egui_ctx.tessellate(output.shapes, state.egui_ctx.pixels_per_point());

                        // ── Sync GPU buffers ──
                        state.sync_buffers(&app);

                        // ── Render ──
                        let output_texture = match state.surface.get_current_texture() {
                            Ok(t) => t,
                            Err(wgpu::SurfaceError::Lost) => { state.surface.configure(&state.device, &state.config); return; }
                            Err(e) => { eprintln!("Surface error: {e:?}"); return; }
                        };
                        let view = output_texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
                        let mut encoder = state.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Encoder") });

                        // Egui texture updates
                        for (id, delta) in &output.textures_delta.set {
                            state.egui_renderer.update_texture(&state.device, &state.queue, *id, &delta);
                        }

                        // Egui screen descriptor
                        let screen_descriptor = egui_wgpu::ScreenDescriptor {
                            size_in_pixels: [state.window_size.0, state.window_size.1],
                            pixels_per_point: state.egui_ctx.pixels_per_point(),
                        };

                        // Upload egui buffers
                        state.egui_renderer.update_buffers(
                            &state.device, &state.queue, &mut encoder, &clipped_primitives, &screen_descriptor
                        );

                        {
                            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("Main Render Pass"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &view,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                            });

                            // Draw nodes
                            pass.set_pipeline(&state.node_pipeline);
                            pass.set_bind_group(0, &state.camera_bind_group, &[]);
                            pass.set_vertex_buffer(0, state.node_buffer.slice(..));
                            let n_nodes: u32 = app.cache.clusters.iter().map(|c| c.nodes.len() as u32).sum();
                            if n_nodes > 0 {
                                pass.draw(0..n_nodes, 0..1);
                            }

                            // Draw synapses
                            pass.set_pipeline(&state.synapse_pipeline);
                            pass.set_vertex_buffer(0, state.synapse_buffer.slice(..));
                            let n_syn_verts: u32 = app.cache.clusters.iter().map(|c| c.synapses.len() as u32 * 2).sum();
                            if n_syn_verts > 0 {
                                pass.draw(0..n_syn_verts, 0..1);
                            }

                            // Egui overlay
                            state.egui_renderer.render(&mut pass, &clipped_primitives, &screen_descriptor);
                        }

                        state.queue.submit(std::iter::once(encoder.finish()));
                        output_texture.present();
                    }
                    WindowEvent::Resized(size) => state.resize(size.width, size.height),
                    WindowEvent::CursorMoved { position, .. } => {
                        state.cursor_pos = (position.x, position.y);
                        if state.orbit.left_down || state.orbit.right_down {
                            let dx = position.x - state.orbit.prev_x;
                            let dy = position.y - state.orbit.prev_y;
                            if state.orbit.left_down {
                                state.camera.yaw += dx as f32 * 0.008;
                                state.camera.pitch = (state.camera.pitch + dy as f32 * 0.008).clamp(-1.5, 1.5);
                            }
                            if state.orbit.right_down {
                                let f = state.camera.distance * 0.002;
                                let (sy, cy) = state.camera.yaw.sin_cos();
                                let (sp, cp) = state.camera.pitch.sin_cos();
                                state.camera.target.x -= (dx as f32 * cy * cp + dy as f32 * sy) * f;
                                state.camera.target.y += dy as f32 * sp * f;
                                state.camera.target.z -= (dx as f32 * sy * cp - dy as f32 * cy) * f;
                            }
                            state.orbit.prev_x = position.x;
                            state.orbit.prev_y = position.y;
                        }
                        state.left_click = false; // consume click on drag
                    }
                    WindowEvent::MouseInput { state: btn_state, button, .. } => {
                        match button {
                            MouseButton::Left => {
                                state.orbit.left_down = btn_state == ElementState::Pressed;
                                if btn_state == ElementState::Pressed {
                                    state.orbit.prev_x = state.cursor_pos.0;
                                    state.orbit.prev_y = state.cursor_pos.1;
                                    state.left_click = true;
                                }
                            }
                            MouseButton::Right => {
                                state.orbit.right_down = btn_state == ElementState::Pressed;
                                if btn_state == ElementState::Pressed {
                                    state.orbit.prev_x = state.cursor_pos.0;
                                    state.orbit.prev_y = state.cursor_pos.1;
                                }
                            }
                            _ => {}
                        }
                    }
                    WindowEvent::MouseWheel { delta, .. } => {
                        let dy = match delta {
                            MouseScrollDelta::LineDelta(_, y) => y * 2.0,
                            MouseScrollDelta::PixelDelta(p) => p.y as f32 * 0.1,
                        };
                        state.camera.distance = (state.camera.distance * (1.0 - dy * 0.02)).clamp(15.0, 400.0);
                    }
                    WindowEvent::KeyboardInput { event: ke, .. } => {
                        let pressed = ke.state == ElementState::Pressed;
                        if pressed {
                            if let PhysicalKey::Code(code) = ke.physical_key {
                                match code {
                                    KeyCode::Space => { app.auto_tick = !app.auto_tick; }
                                    KeyCode::Backspace => { app.command_buffer.pop(); }
                                    KeyCode::KeyR => state.camera = Camera::new(),
                                    KeyCode::KeyS => { state.text_input.push('s'); }
                                    KeyCode::Enter => {
                                        if !app.command_buffer.is_empty() {
                                            let cmd = app.command_buffer.clone();
                                            app.execute_command(&cmd);
                                            app.command_buffer.clear();
                                        } else {
                                            let q = app.search_query.clone();
                                            app.search_node(&q);
                                        }
                                    }
                                    KeyCode::KeyN => {
                                        let mut rng = rand::thread_rng();
                                        let pos = [
                                            state.camera.target.x + rng.gen_range(-10.0..10.0),
                                            state.camera.target.y + rng.gen_range(-10.0..10.0),
                                            state.camera.target.z + rng.gen_range(-10.0..10.0),
                                        ];
                                        app.add_node("NeuronNew", pos);
                                    }
                                    KeyCode::KeyM => {
                                        if let Some(sid) = app.selected_id {
                                            let others: Vec<usize> = app.cache.clusters.iter()
                                                .flat_map(|c| c.nodes.iter().map(|n| n.id))
                                                .filter(|&id| id != sid)
                                                .collect();
                                            if !others.is_empty() {
                                                let idx = rand::thread_rng().gen_range(0..others.len());
                                                app.add_synapse(sid, others[idx]);
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            if let Some(ref text) = ke.text {
                                if !text.is_empty() {
                                    // ':' prefix routes to command buffer, otherwise search
                                    if text == ":" {
                                        // activate command mode - clear search, route to command
                                        app.command_buffer.clear();
                                        app.search_query.clear();
                                        app.search_node("");
                                    } else if !app.command_buffer.is_empty() || text.starts_with(':') {
                                        let clean = text.trim_start_matches(':');
                                        app.command_buffer.push_str(clean);
                                    } else {
                                        state.text_input.push_str(text);
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::AboutToWait => {
                window.request_redraw();
            }
            _ => {}
        }
    }).unwrap();
}
