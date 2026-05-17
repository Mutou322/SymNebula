// SymNebula Interactive Desktop Runtime — 全功能交互版（封闭闭环）
//
// - Node.inputs 上游连接列表（由突触自动填充）
// - gather_inputs 突触权重传播收集
// - node_formula_eval 公式 → 聚合函数（avg/sum/max/min/x+N/rand）
// - compute_cluster_tick 封闭 Tick：收集 → 求值 → 判定 → Rollback Commit
// - 突触可视化（颜色渐变 + 权重亮暗）
// - 全节点公式编辑面板
// - 波形动画 + 状态彩条 + 收敛追踪

use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::time::TimerMode;
use bevy_egui::{egui, EguiContexts, EguiPlugin};
use rand::Rng;
use sym_nebula_compute::types::{NodeStatus, GPU_THRESHOLD};

// ============================================================
// 数据结构
// ============================================================

/// 单节点
#[derive(Debug, Clone)]
struct Node {
    id: usize,
    position: Vec3,
    status: NodeStatus,
    x_value: f64,
    formula: String,
    /// 上游节点 ID 列表（通过突触连接）
    inputs: Vec<usize>,
}

/// 突触
#[derive(Debug, Clone)]
struct Synapse {
    from: usize, // Node ID
    to: usize,   // Node ID
    weight: f64,
}

/// 集群状态
#[derive(Debug)]
struct ClusterState {
    id: usize,
    nodes: Vec<Node>,
    synapses: Vec<Synapse>,
    x_cluster: Vec<f64>,
    temp_x: Vec<f64>,
    status: NodeStatus,
}

/// 集群缓存
#[derive(Debug)]
struct ClusterCache {
    topology_version: u64,
    clusters: Vec<ClusterState>,
}

/// 运行时 Resource
#[derive(Resource)]
struct Runtime {
    tick: usize,
    gpu_available: bool,
    cache: ClusterCache,
    converged: Vec<bool>,
    just_committed: Vec<bool>,
    tick_timer: Timer,
    auto_tick: bool,
}

impl Runtime {
    fn find_node(&self, node_id: usize) -> Option<&Node> {
        for c in &self.cache.clusters {
            if let Some(n) = c.nodes.iter().find(|n| n.id == node_id) {
                return Some(n);
            }
        }
        None
    }

    fn find_node_mut(&mut self, node_id: usize) -> Option<&mut Node> {
        for c in &mut self.cache.clusters {
            if let Some(n) = c.nodes.iter_mut().find(|n| n.id == node_id) {
                return Some(n);
            }
        }
        None
    }

    fn find_cluster(&self, cluster_id: usize) -> Option<&ClusterState> {
        self.cache.clusters.iter().find(|c| c.id == cluster_id)
    }

    fn find_cluster_mut(&mut self, cluster_id: usize) -> Option<&mut ClusterState> {
        self.cache.clusters.iter_mut().find(|c| c.id == cluster_id)
    }

    /// 执行一次封闭 Tick：收集输入 → 公式求值 → 判定 → Rollback-safe Commit
    fn advance(&mut self) {
        self.tick += 1;
        for (i, cluster) in self.cache.clusters.iter_mut().enumerate() {
            let was_green = self.converged[i];
            compute_cluster_tick(cluster);
            self.just_committed[i] = !was_green && cluster.status == NodeStatus::Green;
            if self.just_committed[i] {
                self.converged[i] = true;
                println!("✦ Cluster {} converged at tick {}", cluster.id, self.tick);
            }
            if cluster.status != NodeStatus::Green {
                self.converged[i] = false;
            }
        }
    }
}


// ============================================================
// 封闭 Tick 求解器 — 突触传播 + 公式求值 + 状态判定
// ============================================================

/// 收集节点的上游输入值（upstream.x_value × synapse.weight）
fn gather_inputs(cluster: &ClusterState, node: &Node) -> Vec<f64> {
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
fn node_formula_eval(node: &Node, inputs: &[f64]) -> f64 {
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

/// 对一个集群执行完整封闭 Tick
fn compute_cluster_tick(cluster: &mut ClusterState) {
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

fn random_delta() -> f64 {
    rand::random::<f64>() * 0.2 - 0.1
}

// ============================================================
// 选中节点
// ============================================================

#[derive(Resource, Default)]
struct SelectedNode {
    node_id: Option<usize>,
}

// ============================================================
// ECS 组件
// ============================================================

#[derive(Component)]
struct NebulaNode {
    node_id: usize,
}

// ============================================================
// App Entry
// ============================================================

fn main() {
    App::new()
        .insert_resource(Runtime::new())
        .insert_resource(SelectedNode::default())
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "SymNebula — Interactive Desktop Runtime".into(),
                        resolution: (1600.0, 900.0).into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(AssetPlugin {
                    watch_for_changes_override: None,
                    ..default()
                }),
        )
        .add_plugins(EguiPlugin)
        .add_systems(Startup, setup_scene)
        .add_systems(
            Update,
            (
                tick_system,
                node_pick_system,
                drag_system,
                update_nodes,
                draw_synapses,
                orbit_camera_system,
                ui_system,
            )
                .chain(),
        )
        .run();
}

// ============================================================
// Runtime 初始化
// ============================================================

impl Runtime {
    fn new() -> Self {
        let mut rng = rand::thread_rng();

        // 3 个簇配置
        let configs = vec![
            (1usize, 8usize,
             (0..8).map(|i| i as f64 * 0.3).collect::<Vec<f64>>(),
             Vec3::new(-40.0, 0.0, 0.0),
             vec!["avg", "avg", "avg", "avg", "x+0.05", "x+0.05", "avg", "avg"]),
            (2, 8,
             vec![0.5f64; 8],
             Vec3::new(0.0, 0.0, 0.0),
             vec!["sum", "sum", "sum", "sum", "avg", "avg", "avg", "avg"]),
            (3, 8,
             vec![0.0f64; 8],
             Vec3::new(40.0, 0.0, 0.0),
             vec!["rand", "rand", "rand", "x+0.2", "x-0.1", "avg", "avg", "avg"]),
        ];

        let mut clusters = Vec::new();
        let mut next_id = 0usize;

        for (cid, count, x_cluster, center, formulas) in &configs {
            let spread = 14.0f32;
            let mut nodes = Vec::new();
            let ids: Vec<usize> = (0..*count).map(|_| { let id = next_id; next_id += 1; id }).collect();

            for (j, &nid) in ids.iter().enumerate() {
                let pos = Vec3::new(
                    center.x + rng.gen_range(-spread..spread),
                    center.y + rng.gen_range(-spread..spread),
                    center.z + rng.gen_range(-spread..spread),
                );
                nodes.push(Node {
                    id: nid,
                    position: pos,
                    status: NodeStatus::Yellow,
                    x_value: x_cluster[j],
                    formula: formulas[j].to_string(),
                    inputs: Vec::new(),
                });
            }

            // 簇内突触：环状 + 交叉
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

            // 根据突触填充节点 inputs
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
        Self {
            tick: 0,
            gpu_available: true,
            cache: ClusterCache { topology_version: 1, clusters },
            converged: vec![false; n],
            just_committed: vec![false; n],
            tick_timer: Timer::from_seconds(0.5, TimerMode::Repeating),
            auto_tick: true,
        }
    }
}

// ============================================================
// Scene Setup
// ============================================================

fn setup_scene(
    mut commands: Commands,
    runtime: Res<Runtime>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn(Camera3dBundle {
        transform: Transform::from_xyz(0.0, 30.0, 130.0)
            .looking_at(Vec3::ZERO, Vec3::Y),
        ..default()
    });

    commands.spawn(DirectionalLightBundle {
        directional_light: DirectionalLight {
            illuminance: 3000.0,
            shadows_enabled: false,
            ..default()
        },
        transform: Transform::from_xyz(50.0, 100.0, 50.0)
            .looking_at(Vec3::ZERO, Vec3::Y),
        ..default()
    });

    let sphere = meshes.add(Sphere::new(0.4));

    for cluster in &runtime.cache.clusters {
        for node in &cluster.nodes {
            let (base, emissive) = status_colors(node.status);
            commands.spawn((
                PbrBundle {
                    mesh: sphere.clone(),
                    material: materials.add(StandardMaterial {
                        base_color: base,
                        emissive: emissive.into(),
                        ..default()
                    }),
                    transform: Transform::from_translation(node.position),
                    ..default()
                },
                NebulaNode {
                    node_id: node.id,
                },
            ));
        }
    }
}

// ============================================================
// 颜色工具
// ============================================================

fn status_colors(status: NodeStatus) -> (Color, Color) {
    match status {
        NodeStatus::Green => (Color::srgb(0.1, 1.0, 0.6), Color::srgb(0.0, 0.5, 0.3)),
        NodeStatus::Yellow => (Color::srgb(1.0, 0.8, 0.1), Color::srgb(0.5, 0.4, 0.0)),
        NodeStatus::Purple => (Color::srgb(0.8, 0.2, 1.0), Color::srgb(0.4, 0.0, 0.5)),
    }
}

fn status_egui_color(status: NodeStatus) -> egui::Color32 {
    match status {
        NodeStatus::Green => egui::Color32::from_rgb(25, 255, 140),
        NodeStatus::Yellow => egui::Color32::from_rgb(255, 204, 25),
        NodeStatus::Purple => egui::Color32::from_rgb(204, 50, 255),
    }
}

fn wave_offset(x: f64) -> f32 {
    (x.sin() * 2.0) as f32
}

fn synapse_blend(s1: NodeStatus, s2: NodeStatus, weight: f64) -> Color {
    fn comp(s: NodeStatus) -> (f32, f32, f32) {
        match s {
            NodeStatus::Green => (0.1, 1.0, 0.6),
            NodeStatus::Yellow => (1.0, 0.8, 0.1),
            NodeStatus::Purple => (0.8, 0.2, 1.0),
        }
    }
    let (r1, g1, b1) = comp(s1);
    let (r2, g2, b2) = comp(s2);
    let bright = (weight * 0.7 + 0.3) as f32;
    Color::srgb(
        (r1 + r2) * 0.5 * bright,
        (g1 + g2) * 0.5 * bright,
        (b1 + b2) * 0.5 * bright,
    )
}

fn status_bar_char(status: NodeStatus) -> &'static str {
    match status {
        NodeStatus::Green => "G",
        NodeStatus::Yellow => "Y",
        NodeStatus::Purple => "P",
    }
}

// ============================================================
// Tick 系统
// ============================================================

fn tick_system(mut runtime: ResMut<Runtime>, time: Res<Time>) {
    if !runtime.auto_tick {
        return;
    }
    runtime.tick_timer.tick(time.delta());
    if runtime.tick_timer.just_finished() {
        runtime.advance();
    }
}

// ============================================================
// 鼠标拾取
// ============================================================

fn node_pick_system(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    mut selected: ResMut<SelectedNode>,
    runtime: Res<Runtime>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let window = windows.single();
    let Some(cursor) = window.cursor_position() else { return };
    let Ok((camera, cam_transform)) = cameras.get_single() else { return };
    let Some(ray) = camera.viewport_to_world(cam_transform, cursor) else { return };

    let mut best: Option<(usize, f32)> = None;
    for c in &runtime.cache.clusters {
        for node in &c.nodes {
            let visual = node.position
                + Vec3::new(0.0, wave_offset(c.x_cluster[node.id % c.x_cluster.len()]), 0.0);
            let to = visual - ray.origin;
            let t = to.dot(*ray.direction);
            if t < 0.0 { continue; }
            let closest = ray.origin + *ray.direction * t;
            let dist = closest.distance(visual);
            if dist < 1.5 && (best.is_none() || t < best.unwrap().1) {
                best = Some((node.id, t));
            }
        }
    }
    selected.node_id = best.map(|(id, _)| id);
}

// ============================================================
// 节点拖拽
// ============================================================

fn drag_system(
    mouse: Res<ButtonInput<MouseButton>>,
    mut motion_events: EventReader<MouseMotion>,
    selected: Res<SelectedNode>,
    mut runtime: ResMut<Runtime>,
) {
    if !mouse.pressed(MouseButton::Left) { return; }
    let Some(nid) = selected.node_id else { return };
    for ev in motion_events.read() {
        let d = Vec3::new(ev.delta.x, -ev.delta.y, 0.0) * 0.05;
        if let Some(node) = runtime.find_node_mut(nid) {
            node.position += d;
        }
    }
}

// ============================================================
// 节点更新 — 波形动画 + 状态颜色
// ============================================================

fn update_nodes(
    runtime: Res<Runtime>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut query: Query<(&NebulaNode, &mut Transform, &Handle<StandardMaterial>)>,
) {
    for (node, mut transform, mat_handle) in &mut query {
        let Some(rn) = runtime.find_node(node.node_id) else { continue };
        let wave = wave_offset(rn.x_value);
        transform.translation = rn.position + Vec3::new(0.0, wave, 0.0);
        let (base, emissive) = status_colors(rn.status);
        if let Some(mat) = materials.get_mut(mat_handle) {
            mat.base_color = base;
            mat.emissive = emissive.into();
        }
    }
}

// ============================================================
// 突触可视化 — 颜色渐变 + 权重亮暗
// ============================================================

fn draw_synapses(
    mut gizmos: Gizmos,
    runtime: Res<Runtime>,
    selected: Res<SelectedNode>,
) {
    for cluster in &runtime.cache.clusters {
        for synapse in &cluster.synapses {
            let Some(from) = cluster.nodes.iter().find(|n| n.id == synapse.from) else { continue };
            let Some(to) = cluster.nodes.iter().find(|n| n.id == synapse.to) else { continue };

            let fw = wave_offset(from.x_value);
            let tw = wave_offset(to.x_value);
            let a = from.position + Vec3::new(0.0, fw, 0.0);
            let b = to.position + Vec3::new(0.0, tw, 0.0);

            let color = synapse_blend(from.status, to.status, synapse.weight);

            let highlight = selected.node_id == Some(synapse.from)
                || selected.node_id == Some(synapse.to);
            gizmos.line(a, b, if highlight { Color::srgb(0.9, 0.9, 1.0) } else { color });
        }

        // 选中节点高亮圈
        for node in &cluster.nodes {
            if selected.node_id == Some(node.id) {
                let w = wave_offset(node.x_value);
                let pos = node.position + Vec3::new(0.0, w, 0.0);
                gizmos.circle(pos, Dir3::Y, 0.8, Color::srgb(1.0, 1.0, 1.0));
            }
        }
    }
}

// ============================================================
// 轨道相机
// ============================================================

fn orbit_camera_system(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut query: Query<&mut Transform, With<Camera>>,
) {
    let mut transform = query.single_mut();
    let speed = 80.0 * time.delta_seconds();
    if keys.pressed(KeyCode::KeyW) { transform.translation.z -= speed; }
    if keys.pressed(KeyCode::KeyS) { transform.translation.z += speed; }
    if keys.pressed(KeyCode::KeyA) { transform.translation.x -= speed; }
    if keys.pressed(KeyCode::KeyD) { transform.translation.x += speed; }
    if keys.pressed(KeyCode::KeyQ) { transform.translation.y += speed; }
    if keys.pressed(KeyCode::KeyE) { transform.translation.y -= speed; }
}

// ============================================================
// HUD — egui 面板（Runtime + Formula + Inspector）
// ============================================================

fn ui_system(
    mut contexts: EguiContexts,
    mut runtime: ResMut<Runtime>,
    mut selected: ResMut<SelectedNode>,
) {
    // ===================== Panel 1: Runtime Status =====================
    egui::Window::new("SymNebula Runtime").default_width(360.0)
        .show(contexts.ctx_mut(), |ui| {
            ui.heading("ClusterSolver");
            ui.separator();
            ui.label(format!("Tick: {}", runtime.tick));
            ui.checkbox(&mut runtime.auto_tick, "Auto Tick");
            if ui.button("Manual Tick").clicked() { runtime.advance(); }
            ui.label(format!("Cache v{}", runtime.cache.topology_version));
            ui.separator();

            for (i, c) in runtime.cache.clusters.iter().enumerate() {
                let ec = status_egui_color(c.status);
                let mode = if c.nodes.len() > GPU_THRESHOLD && runtime.gpu_available { "GPU" } else { "CPU" };
                let avg_x = c.nodes.iter().map(|n| n.x_value).sum::<f64>() / c.nodes.len() as f64;
                let mut line = format!("Cluster {} [{}] {}  {} nodes  x̄={:.2}",
                    c.id, status_bar_char(c.status), mode, c.nodes.len(), avg_x);
                if runtime.just_committed[i] { line += " ✦ COMMIT"; }
                else if runtime.converged[i] { line += " ✓"; }
                ui.colored_label(ec, line);
            }
            ui.separator();

            let mut cnt = [0u32; 3];
            for c in &runtime.cache.clusters {
                match c.status {
                    NodeStatus::Green => { cnt[0] += 1; }
                    NodeStatus::Yellow => { cnt[1] += 1; }
                    NodeStatus::Purple => { cnt[2] += 1; }
                }
            }
            ui.label(egui::RichText::new(format!("Green:  {}", cnt[0])).color(status_egui_color(NodeStatus::Green)));
            ui.label(egui::RichText::new(format!("Yellow: {}", cnt[1])).color(status_egui_color(NodeStatus::Yellow)));
            ui.label(egui::RichText::new(format!("Purple: {}", cnt[2])).color(status_egui_color(NodeStatus::Purple)));
            ui.separator();
            ui.label("W/S/A/D → Move   Q/E → Up/Down");
            ui.label("Left-click → Select   Drag → Move");
        });

    // ===================== Panel 2: Formula Editor =====================
    // 缓冲所有公式数据，避免借用冲突
    struct FmtEntry {
        cid: usize,
        nid: usize,
        formula: String,
        status: NodeStatus,
        xv: f64,
    }
    let entries: Vec<FmtEntry> = runtime.cache.clusters.iter().flat_map(|c| {
        c.nodes.iter().map(|n| FmtEntry {
            cid: c.id, nid: n.id,
            formula: n.formula.clone(),
            status: c.status,
            xv: n.x_value,
        })
    }).collect();

    let mut changed = Vec::new();
    egui::Window::new("Formula Editor").default_width(340.0)
        .show(contexts.ctx_mut(), |ui| {
            egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                for e in entries.iter() {
                    let ec = status_egui_color(e.status);
                    ui.colored_label(ec, format!("Cluster {} #{}  {:?}  x={:.2}",
                        e.cid, e.nid, e.status, e.xv));
                    ui.horizontal(|ui| {
                        ui.colored_label(ec, format!("#{}", e.nid));
                        let mut fb = e.formula.clone();
                        ui.add(egui::TextEdit::singleline(&mut fb)
                            .desired_width(140.0)
                            .font(egui::TextStyle::Monospace));
                        if fb != e.formula { changed.push((e.cid, e.nid, fb)); }
                        ui.label(format!("xv={:.3}", e.xv));
                    });
                }
            });
        });

    // 写回修改的公式
    for (cid, nid, formula) in &changed {
        if let Some(c) = runtime.find_cluster_mut(*cid) {
            if let Some(n) = c.nodes.iter_mut().find(|n| n.id == *nid) {
                if n.formula != *formula {
                    n.formula = formula.clone();
                    runtime.cache.topology_version += 1;
                }
            }
        }
    }

    // ===================== Panel 3: Node Inspector =====================
    if let Some(nid) = selected.node_id {
        let Some(rn) = runtime.find_node(nid) else { selected.node_id = None; return; };
        let cid = cid_from_id(&runtime, nid);

        egui::Window::new("Node Inspector").default_width(320.0)
            .show(contexts.ctx_mut(), |ui| {
                ui.heading(format!("Node #{} (Cluster {})", nid, cid));
                ui.separator();
                ui.label(format!("Position: ({:.1}, {:.1}, {:.1})", rn.position.x, rn.position.y, rn.position.z));
                ui.colored_label(status_egui_color(rn.status), format!("Status: {:?}", rn.status));
                ui.label(format!("x_value: {:.4}", rn.x_value));
                ui.label(format!("Formula: {}", rn.formula));

                // 突触列表
                if let Some(c) = runtime.find_cluster(cid) {
                    ui.separator();
                    ui.label("Synapses:");
                    for s in &c.synapses {
                        if s.from == nid || s.to == nid {
                            let peer = if s.from == nid { s.to } else { s.from };
                            let arrow = if s.from == nid { "→" } else { "←" };
                            let name = format!("  {}  #{}  w={:.2}", arrow, peer, s.weight);
                            if let Some(pn) = runtime.find_node(peer) {
                                ui.colored_label(status_egui_color(pn.status), format!("{}  {:?}", name, pn.status));
                            } else {
                                ui.label(name);
                            }
                        }
                    }
                }
            });
    }
}

fn cid_from_id(runtime: &Runtime, node_id: usize) -> usize {
    for c in &runtime.cache.clusters {
        if c.nodes.iter().any(|n| n.id == node_id) { return c.id; }
    }
    0
}
