// SymNebula Desktop Runtime
// Bevy + 自定义 GPU 渲染骨架
//
// 功能：
// - 3D 星云节点
// - Tick 同步
// - Green / Yellow / Purple 状态
// - GPU Instancing 入口
// - Runtime → RenderSnapshot 同步

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPlugin};
use rand::Rng;

// ============================================================
// Runtime Snapshot
// ============================================================

#[derive(Clone, Copy, PartialEq)]
enum NodeStatus {
    Green,
    Yellow,
    Purple,
}

#[derive(Clone)]
struct RenderNodeSnapshot {
    position: Vec3,
    energy: f32,
    status: NodeStatus,
}

#[derive(Resource)]
struct RuntimeSnapshot {
    tick: u64,
    nodes: Vec<RenderNodeSnapshot>,
}

// ============================================================
// ECS Components
// ============================================================

#[derive(Component)]
struct NebulaNode {
    index: usize,
}

// ============================================================
// App Entry
// ============================================================

fn main() {
    App::new()
        .insert_resource(create_initial_snapshot())
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "SymNebula".into(),
                resolution: (1600.0, 900.0).into(),
                present_mode: bevy::window::PresentMode::AutoVsync,
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin)
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                runtime_tick_system,
                sync_snapshot_to_gpu,
                orbit_camera_system,
                ui_system,
            ),
        )
        .run();
}

// ============================================================
// Initial Runtime Snapshot
// ============================================================

fn create_initial_snapshot() -> RuntimeSnapshot {
    let mut rng = rand::thread_rng();

    let mut nodes = vec![];

    // 示例：10k 节点
    for _ in 0..10_000 {
        nodes.push(RenderNodeSnapshot {
            position: Vec3::new(
                rng.gen_range(-100.0..100.0),
                rng.gen_range(-100.0..100.0),
                rng.gen_range(-100.0..100.0),
            ),
            energy: rng.gen_range(0.0..1.0),
            status: NodeStatus::Green,
        });
    }

    RuntimeSnapshot { tick: 0, nodes }
}

// ============================================================
// Scene Setup
// ============================================================

fn setup(
    mut commands: Commands,
    snapshot: Res<RuntimeSnapshot>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // --------------------------------------------------------
    // Camera
    // --------------------------------------------------------

    commands.spawn(Camera3dBundle {
        transform: Transform::from_xyz(0.0, 40.0, 150.0)
            .looking_at(Vec3::ZERO, Vec3::Y),
        ..default()
    });

    // --------------------------------------------------------
    // Light
    // --------------------------------------------------------

    commands.spawn(PointLightBundle {
        point_light: PointLight {
            intensity: 100_000.0,
            shadows_enabled: false,
            ..default()
        },
        transform: Transform::from_xyz(0.0, 200.0, 0.0),
        ..default()
    });

    // --------------------------------------------------------
    // Spawn Nebula Nodes
    // --------------------------------------------------------

    for (i, node) in snapshot.nodes.iter().enumerate() {
        commands.spawn((
            PbrBundle {
                mesh: meshes.add(Sphere::new(0.25)),
                material: materials.add(StandardMaterial {
                    emissive: Color::srgb(0.2, 0.8, 1.0).into(),
                    ..default()
                }),
                transform: Transform::from_translation(node.position),
                ..default()
            },
            NebulaNode { index: i },
        ));
    }
}

// ============================================================
// Runtime Tick
// ============================================================

fn runtime_tick_system(
    mut snapshot: ResMut<RuntimeSnapshot>,
    time: Res<Time>,
) {
    snapshot.tick += 1;

    let t = time.elapsed_seconds();

    for node in &mut snapshot.nodes {
        // ----------------------------------------------------
        // 模拟 X_cluster 动态波动
        // ----------------------------------------------------

        node.energy =
            ((t + node.position.x * 0.01).sin() * 0.5 + 0.5) as f32;

        // ----------------------------------------------------
        // 模拟状态传播
        // ----------------------------------------------------

        if node.energy > 0.75 {
            node.status = NodeStatus::Green;
        } else if node.energy > 0.35 {
            node.status = NodeStatus::Yellow;
        } else {
            node.status = NodeStatus::Purple;
        }
    }
}

// ============================================================
// Runtime → GPU Sync
// ============================================================

fn sync_snapshot_to_gpu(
    snapshot: Res<RuntimeSnapshot>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut query: Query<(
        &NebulaNode,
        &Handle<StandardMaterial>,
        &mut Transform,
    )>,
) {
    for (node_ref, material_handle, mut transform) in &mut query {
        let runtime_node = &snapshot.nodes[node_ref.index];

        // ----------------------------------------------------
        // Position animation
        // ----------------------------------------------------

        transform.translation.y =
            runtime_node.position.y + runtime_node.energy * 3.0;

        // ----------------------------------------------------
        // Status → Color
        // ----------------------------------------------------

        if let Some(material) = materials.get_mut(material_handle) {
            match runtime_node.status {
                NodeStatus::Green => {
                    material.emissive =
                        Color::srgb(0.1, 1.0, 0.8).into();
                }

                NodeStatus::Yellow => {
                    material.emissive =
                        Color::srgb(1.0, 0.8, 0.1).into();
                }

                NodeStatus::Purple => {
                    material.emissive =
                        Color::srgb(0.8, 0.1, 1.0).into();
                }
            }
        }
    }
}

// ============================================================
// Orbit Camera
// ============================================================

fn orbit_camera_system(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut query: Query<&mut Transform, With<Camera>>,
) {
    let mut transform = query.single_mut();

    let speed = 50.0 * time.delta_seconds();

    if keys.pressed(KeyCode::KeyW) {
        transform.translation.z -= speed;
    }

    if keys.pressed(KeyCode::KeyS) {
        transform.translation.z += speed;
    }

    if keys.pressed(KeyCode::KeyA) {
        transform.translation.x -= speed;
    }

    if keys.pressed(KeyCode::KeyD) {
        transform.translation.x += speed;
    }

    if keys.pressed(KeyCode::KeyQ) {
        transform.translation.y += speed;
    }

    if keys.pressed(KeyCode::KeyE) {
        transform.translation.y -= speed;
    }
}

// ============================================================
// HUD / Inspector
// ============================================================

fn ui_system(
    mut contexts: EguiContexts,
    snapshot: Res<RuntimeSnapshot>,
) {
    egui::Window::new("SymNebula Runtime")
        .default_width(300.0)
        .show(contexts.ctx_mut(), |ui| {
            ui.heading("ClusterSolver");

            ui.separator();

            ui.label(format!("Tick: {}", snapshot.tick));

            ui.label(format!(
                "Rendered Nodes: {}",
                snapshot.nodes.len()
            ));

            ui.separator();

            let green = snapshot
                .nodes
                .iter()
                .filter(|n| n.status == NodeStatus::Green)
                .count();

            let yellow = snapshot
                .nodes
                .iter()
                .filter(|n| n.status == NodeStatus::Yellow)
                .count();

            let purple = snapshot
                .nodes
                .iter()
                .filter(|n| n.status == NodeStatus::Purple)
                .count();

            ui.label(format!("Green: {}", green));
            ui.label(format!("Yellow: {}", yellow));
            ui.label(format!("Purple: {}", purple));

            ui.separator();

            ui.label("WASD → Move Camera");
            ui.label("Q/E → Up Down");

            ui.separator();

            ui.label("Runtime owns all math.");
            ui.label("UI is render only.");
        });
}
