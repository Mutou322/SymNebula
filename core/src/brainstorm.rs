/// .brainstorm v1 — 确定性计算的可序列化宇宙状态
///
/// ZIP 容器格式：
///   graph.xml          — 拓扑 + 数学模型（XML Schema）
///   state.json         — 节点双缓冲 + 状态彩条
///   clusters.json      — ClusterCache + SCC blocks
///   runtime.meta.json  — Tick 运行信息 / 求解器设置
///   version.txt        — 版本一致性
///   assets/            — 可选 UI 资源

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::ast::{parse_expression, parse_simple_eq, Expr};
use crate::cluster::{ClusterCache, ClusterSolverV3, VarPort};
use crate::graph::NebulaGraph;
use crate::state::NodeState;

// ============================================================
// 1. 数据结构（serde 序列化层）
// ============================================================

/// state.json — 节点双缓冲 + 状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateJson {
    pub nodes: HashMap<String, StateEntry>,
}

/// 单个节点的状态条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateEntry {
    pub current_buffer: HashMap<String, f64>,
    pub next_buffer: HashMap<String, f64>,
    pub status: String,
}

/// clusters.json — ClusterCache + SCC blocks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClustersJson {
    pub topology_version: u64,
    pub clusters: Vec<ClusterEntry>,
}

/// 单个集群条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterEntry {
    pub id: String,
    pub nodes: Vec<String>,
    pub variables: HashMap<String, usize>,
    pub blocks: Vec<BlockEntry>,
}

/// SCC 分块
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockEntry {
    pub id: String,
    pub vars: Vec<usize>,
}

/// runtime.meta.json — 系统元信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeMetaJson {
    pub last_tick: u64,
    pub tick_time_ms: f64,
    pub global_vars_count: usize,
    pub solver_settings: SolverSettings,
    pub topology_version: u64,
}

/// 求解器设置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolverSettings {
    pub tolerance: f64,
    pub max_iterations: usize,
}

// ============================================================
// graph.xml 数据结构（XML 格式，不使用 serde）
// ============================================================

/// graph.xml — 拓扑 + 数学模型
pub struct GraphXml {
    pub name: String,
    pub version: String,
    pub nodes: Vec<GraphNode>,
    pub synapses: Vec<GraphSynapse>,
}

/// 节点的端口定义
#[derive(Debug, Clone)]
pub struct PortDef {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

/// graph.xml 中的节点
pub struct GraphNode {
    pub id: String,
    pub node_type: String,
    pub formula: String,
    pub ports: PortDef,
}

/// graph.xml 中的突触
pub struct GraphSynapse {
    pub from: String, // "node_id.port"
    pub to: String,
}

// ============================================================
// version.txt + assets（文本 / 二进制）
// ============================================================

/// version.txt
pub struct VersionTxt {
    pub brainstorm_version: u64,
    pub symnebula_core: String,
    pub topology_version: u64,
}

/// assets/ 条目
pub struct AssetEntry {
    pub name: String,
    pub data: Vec<u8>,
}

// ============================================================
// Brainstorm 完整容器
// ============================================================

/// .brainstorm 完整容器
pub struct Brainstorm {
    pub graph: GraphXml,
    pub state: StateJson,
    pub clusters: ClustersJson,
    pub meta: RuntimeMetaJson,
    pub assets: Vec<AssetEntry>,
    pub version: VersionTxt,
}

// ============================================================
// 2. 状态转换工具
// ============================================================

fn state_to_status(state: &NodeState) -> &'static str {
    match state {
        NodeState::Green => "Green",
        NodeState::Yellow => "Yellow",
        NodeState::Purple => "Purple",
        NodeState::Gray => "Gray",
    }
}

fn status_to_state(s: &str) -> Option<NodeState> {
    match s {
        "Green" | "green" => Some(NodeState::Green),
        "Yellow" | "yellow" => Some(NodeState::Yellow),
        "Purple" | "purple" => Some(NodeState::Purple),
        "Gray" | "gray" | "Grey" | "grey" => Some(NodeState::Gray),
        _ => None,
    }
}

fn detect_formula_type(s: &str) -> &'static str {
    let trimmed = s.trim();
    if trimmed.contains('=') {
        "eq"
    } else if trimmed.parse::<f64>().is_ok() {
        "constant"
    } else {
        "expr"
    }
}

// ============================================================
// 3. Brainstorm 构造器
// ============================================================

impl Brainstorm {
    /// 从引擎运行时状态构建快照。
    pub fn from_engine(
        graph: &NebulaGraph,
        solver: Option<&ClusterSolverV3>,
        env: Option<&HashMap<(usize, String), f64>>,
        tick: u64,
    ) -> Self {
        let graph_xml = GraphXml::from_graph(graph);
        let state = StateJson::from_graph(graph, env);
        let clusters = solver
            .map(|s| ClustersJson::from_solver(s, graph))
            .unwrap_or_else(|| ClustersJson {
                topology_version: graph.topology_version,
                clusters: Vec::new(),
            });
        let meta = RuntimeMetaJson {
            last_tick: tick,
            tick_time_ms: 0.0,
            global_vars_count: clusters.clusters.iter().map(|c| c.variables.len()).sum(),
            solver_settings: SolverSettings::default(),
            topology_version: graph.topology_version,
        };
        let version = VersionTxt {
            brainstorm_version: 1,
            symnebula_core: "0.1.0".into(),
            topology_version: graph.topology_version,
        };

        Brainstorm {
            graph: graph_xml,
            state,
            clusters,
            meta,
            assets: Vec::new(),
            version,
        }
    }

    /// 简化版：仅从 Graph + env 构建（无集群信息）
    pub fn from_graph(
        graph: &NebulaGraph,
        env: &HashMap<(usize, String), f64>,
        tick: u64,
    ) -> Self {
        Self::from_engine(graph, None, Some(env), tick)
    }
}

impl SolverSettings {
    fn default() -> Self {
        SolverSettings {
            tolerance: 1e-9,
            max_iterations: 50,
        }
    }
}

impl RuntimeMetaJson {
    pub fn default() -> Self {
        RuntimeMetaJson {
            last_tick: 0,
            tick_time_ms: 0.0,
            global_vars_count: 0,
            solver_settings: SolverSettings::default(),
            topology_version: 0,
        }
    }
}

// ============================================================
// 4. GraphXml — 序列化（Display） + 反序列化（parse）
// ============================================================

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

impl fmt::Display for GraphXml {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            r#"<nebula name="{}" version="{}">"#,
            escape_xml(&self.name),
            escape_xml(&self.version)
        )?;
        writeln!(f, "    <nodes>")?;
        for node in &self.nodes {
            writeln!(
                f,
                r#"        <node id="{}" type="{}">"#,
                escape_xml(&node.id),
                escape_xml(&node.node_type)
            )?;
            writeln!(
                f,
                r#"            <formula>{}</formula>"#,
                escape_xml(&node.formula)
            )?;
            if !node.ports.inputs.is_empty() || !node.ports.outputs.is_empty() {
                writeln!(f, "            <ports>")?;
                for inp in &node.ports.inputs {
                    writeln!(f, r#"                <input name="{}"/>"#, escape_xml(inp))?;
                }
                for out in &node.ports.outputs {
                    writeln!(f, r#"                <output name="{}"/>"#, escape_xml(out))?;
                }
                writeln!(f, "            </ports>")?;
            }
            writeln!(f, "        </node>")?;
        }
        writeln!(f, "    </nodes>")?;
        if !self.synapses.is_empty() {
            writeln!(f, "    <synapses>")?;
            for syn in &self.synapses {
                writeln!(
                    f,
                    r#"        <synapse from="{}" to="{}"/>"#,
                    escape_xml(&syn.from),
                    escape_xml(&syn.to)
                )?;
            }
            writeln!(f, "    </synapses>")?;
        }
        writeln!(f, "</nebula>")
    }
}

impl GraphXml {
    /// 从 NebulaGraph 构建 graph.xml
    pub fn from_graph(graph: &NebulaGraph) -> Self {
        let mut nodes = Vec::new();
        for node in &graph.nodes {
            let formula_str = format!("{}", node.formula);
            let node_id = format!("n{}", node.id);
            let node_type = if node.is_dynamic { "dynamic" } else { "formula" };

            let mut input_set: HashSet<String> = HashSet::new();
            let mut output_set: HashSet<String> = HashSet::new();

            for edge in &graph.edges {
                if edge.to_node == node.id {
                    input_set.insert(edge.to_symbol.clone());
                }
                if edge.from_node == node.id {
                    output_set.insert(edge.from_symbol.clone());
                }
            }

            let all_syms: HashSet<String> = node.formula.symbols().into_iter().collect();
            let unconnected: Vec<&String> = all_syms
                .iter()
                .filter(|s| !input_set.contains(*s) && !output_set.contains(*s))
                .collect();

            match &node.formula {
                Expr::Eq(_, _) => {
                    let target = node.solve_target.as_deref().unwrap_or("");
                    for s in &unconnected {
                        if s.as_str() == target {
                            output_set.insert(s.to_string());
                        } else {
                            input_set.insert(s.to_string());
                        }
                    }
                }
                Expr::Number(_) => {}
                _ => {
                    for s in &unconnected {
                        input_set.insert(s.to_string());
                    }
                }
            }

            let mut inputs: Vec<String> = input_set.into_iter().collect();
            inputs.sort();
            let mut outputs: Vec<String> = output_set.into_iter().collect();
            outputs.sort();
            if matches!(node.formula, Expr::Number(_)) && outputs.is_empty() {
                outputs.push("output".to_string());
            }

            nodes.push(GraphNode {
                id: node_id,
                node_type: node_type.to_string(),
                formula: formula_str,
                ports: PortDef { inputs, outputs },
            });
        }

        let mut synapses = Vec::new();
        for edge in &graph.edges {
            synapses.push(GraphSynapse {
                from: format!("n{}.{}", edge.from_node, edge.from_symbol),
                to: format!("n{}.{}", edge.to_node, edge.to_symbol),
            });
        }

        GraphXml {
            name: "SymNebula".into(),
            version: "1.0".into(),
            nodes,
            synapses,
        }
    }

    /// 从 XML 字符串解析
    pub fn parse(xml: &str) -> Result<Self, String> {
        let mut name = "SymNebula".to_string();
        let mut version = "1.0".to_string();
        let mut nodes = Vec::new();
        let mut synapses = Vec::new();
        let mut in_node = false;
        let mut cid = String::new();
        let mut ctype = String::new();
        let mut cformula = String::new();
        let mut in_port_section = false;
        let mut current_ports: Vec<(Vec<String>, Vec<String>)> = Vec::new();

        let mut push_node =
            |id: &str, ty: &str, formula: &str, ports: &(Vec<String>, Vec<String>)| {
                nodes.push(GraphNode {
                    id: id.to_string(),
                    node_type: ty.to_string(),
                    formula: formula.to_string(),
                    ports: PortDef {
                        inputs: ports.0.clone(),
                        outputs: ports.1.clone(),
                    },
                });
            };

        for line in xml.lines() {
            let t = line.trim();

            if t.starts_with("<nebula ") {
                if let Some(s) = t.find("name=") {
                    let after = &t[s + 6..];
                    if let Some(e) = after.find('"') {
                        name = after[..e].to_string();
                    }
                }
                if let Some(s) = t.find("version=") {
                    let after = &t[s + 9..];
                    if let Some(e) = after.find('"') {
                        version = after[..e].to_string();
                    }
                }
            }

            if t.starts_with("<node ") {
                in_node = true;
                cid.clear();
                ctype.clear();
                cformula.clear();
                in_port_section = false;
                if let Some(s) = t.find("id=") {
                    let after = &t[s + 4..];
                    if let Some(e) = after.find('"') {
                        cid = after[..e].to_string();
                    }
                }
                if let Some(s) = t.find("type=") {
                    let after = &t[s + 6..];
                    if let Some(e) = after.find('"') {
                        ctype = after[..e].to_string();
                    }
                }
                current_ports.push((Vec::new(), Vec::new()));
            }

            if in_node {
                if t.starts_with("<formula>") {
                    if let Some(rest) = t.strip_prefix("<formula>") {
                        if let Some(e) = rest.find("</formula>") {
                            cformula = rest[..e].to_string();
                        }
                    }
                }
                if t.starts_with("<ports>") {
                    in_port_section = true;
                }
                if t.starts_with("</ports>") {
                    in_port_section = false;
                }
                if in_port_section {
                    if let Some(rest) = t.strip_prefix(r#"<input name=""#) {
                        if let Some(e) = rest.find('"') {
                            if let Some(last) = current_ports.last_mut() {
                                last.0.push(rest[..e].to_string());
                            }
                        }
                    }
                    if let Some(rest) = t.strip_prefix(r#"<output name=""#) {
                        if let Some(e) = rest.find('"') {
                            if let Some(last) = current_ports.last_mut() {
                                last.1.push(rest[..e].to_string());
                            }
                        }
                    }
                }
                if t.starts_with("</node>") {
                    in_node = false;
                    if let Some(ports) = current_ports.pop() {
                        push_node(&cid, &ctype, &cformula, &ports);
                    }
                }
            }

            if t.starts_with("<synapse ") {
                let from = t
                    .split("from=")
                    .nth(1)
                    .and_then(|s| s.split('"').nth(1))
                    .unwrap_or("")
                    .to_string();
                let to = t
                    .split("to=")
                    .nth(1)
                    .and_then(|s| s.split('"').nth(1))
                    .unwrap_or("")
                    .to_string();
                if !from.is_empty() && !to.is_empty() {
                    synapses.push(GraphSynapse { from, to });
                }
            }
        }

        Ok(GraphXml {
            name,
            version,
            nodes,
            synapses,
        })
    }
}

// ============================================================
// 5. StateJson — serde 序列化/反序列化
// ============================================================

impl StateJson {
    /// 从 Graph + 环境变量构建
    fn from_graph(
        graph: &NebulaGraph,
        env: Option<&HashMap<(usize, String), f64>>,
    ) -> Self {
        let per_node: HashMap<usize, HashMap<String, f64>> = if let Some(env_map) = env {
            let mut acc: HashMap<usize, HashMap<String, f64>> = HashMap::new();
            for ((nid, sym), val) in env_map {
                acc.entry(*nid).or_default().insert(sym.clone(), *val);
            }
            acc
        } else {
            let mut acc: HashMap<usize, HashMap<String, f64>> = HashMap::new();
            for node in &graph.nodes {
                let mut buf = HashMap::new();
                if let Some(v) = node.value {
                    buf.insert("output".to_string(), v);
                }
                for edge in &graph.edges {
                    if edge.from_node == node.id {
                        if let Some(v) = edge.delay_buffer.or(edge.default_value) {
                            buf.insert(edge.from_symbol.clone(), v);
                        }
                    }
                }
                for sym in node.formula.symbols() {
                    for edge in &graph.edges {
                        if edge.to_node == node.id && edge.to_symbol == sym {
                            if let Some(v) = edge.delay_buffer.or(edge.default_value) {
                                buf.entry(sym.clone()).or_insert(v);
                            }
                        }
                    }
                }
                if !buf.is_empty() {
                    acc.insert(node.id, buf);
                }
            }
            acc
        };

        let mut nodes = HashMap::new();
        for node in &graph.nodes {
            let node_id = format!("n{}", node.id);
            let buffer = per_node.get(&node.id).cloned().unwrap_or_default();
            let status = state_to_status(&node.state);
            nodes.insert(
                node_id,
                StateEntry {
                    current_buffer: buffer.clone(),
                    next_buffer: buffer,
                    status: status.to_string(),
                },
            );
        }

        StateJson { nodes }
    }

    /// serde 序列化为 JSON 字符串
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// serde 从 JSON 字符串反序列化
    pub fn parse(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| format!("state.json 解析失败: {}", e))
    }
}

// ============================================================
// 6. ClustersJson — serde 序列化/反序列化
// ============================================================

impl ClustersJson {
    /// 从 ClusterSolverV3 构建
    fn from_solver(solver: &ClusterSolverV3, graph: &NebulaGraph) -> Self {
        let mut clusters = Vec::new();
        if let Some(ref comp) = solver.compilation {
            for (ci, cluster_comp) in comp.clusters.iter().enumerate() {
                let nodes: Vec<String> =
                    cluster_comp.node_ids.iter().map(|nid| format!("n{}", nid)).collect();

                let mut variables = HashMap::new();
                for (port, &gidx) in &cluster_comp.global_idx_map {
                    let key = format!("n{}.{}", port.node_id, port.symbol);
                    variables.insert(key, gidx);
                }

                let mut blocks = Vec::new();
                for (bi, (_, var_indices)) in cluster_comp.blocks.iter().enumerate() {
                    blocks.push(BlockEntry {
                        id: format!("block_{}", bi),
                        vars: var_indices.clone(),
                    });
                }

                clusters.push(ClusterEntry {
                    id: format!("cluster_{}", ci),
                    nodes,
                    variables,
                    blocks,
                });
            }
        }
        ClustersJson {
            topology_version: graph.topology_version,
            clusters,
        }
    }

    /// serde 序列化为 JSON 字符串
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// serde 从 JSON 字符串反序列化
    pub fn parse(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| format!("clusters.json 解析失败: {}", e))
    }
}

// ============================================================
// 7. RuntimeMetaJson — serde 序列化/反序列化
// ============================================================

impl RuntimeMetaJson {
    /// serde 序列化为 JSON 字符串
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// serde 从 JSON 字符串反序列化
    pub fn parse(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| format!("runtime.meta.json 解析失败: {}", e))
    }
}

// ============================================================
// 8. VersionTxt — 文本序列化/反序列化
// ============================================================

impl VersionTxt {
    pub fn parse(txt: &str) -> Result<Self, String> {
        let mut bv = 0u64;
        let mut sc = String::new();
        let mut tv = 0u64;
        for line in txt.lines() {
            let t = line.trim();
            if let Some(v) = t.strip_prefix("brainstorm_version=") {
                bv = v.parse().unwrap_or(0);
            } else if let Some(v) = t.strip_prefix("symnebula_core=") {
                sc = v.to_string();
            } else if let Some(v) = t.strip_prefix("topology_version=") {
                tv = v.parse().unwrap_or(0);
            }
        }
        Ok(VersionTxt {
            brainstorm_version: bv,
            symnebula_core: sc,
            topology_version: tv,
        })
    }
}

// ============================================================
// 9. Brainstorm — 包内容生成
// ============================================================

impl Brainstorm {
    pub fn to_graph_xml(&self) -> String {
        format!("{}", self.graph)
    }

    pub fn to_state_json(&self) -> String {
        self.state.to_json()
    }

    pub fn to_clusters_json(&self) -> String {
        self.clusters.to_json()
    }

    pub fn to_meta_json(&self) -> String {
        self.meta.to_json()
    }

    pub fn to_version_txt(&self) -> String {
        format!(
            "brainstorm_version={}\nsymnebula_core={}\ntopology_version={}",
            self.version.brainstorm_version,
            self.version.symnebula_core,
            self.version.topology_version,
        )
    }

    pub fn to_package(&self) -> HashMap<String, String> {
        let mut pkg = HashMap::new();
        pkg.insert("graph.xml".into(), self.to_graph_xml());
        pkg.insert("state.json".into(), self.to_state_json());
        pkg.insert("clusters.json".into(), self.to_clusters_json());
        pkg.insert("runtime.meta.json".into(), self.to_meta_json());
        pkg.insert("version.txt".into(), self.to_version_txt());
        pkg
    }
}

// ============================================================
// 10. Brainstorm → NebulaGraph 反序列化还原
// ============================================================

impl Brainstorm {
    /// 从 graph.xml 重建 NebulaGraph（仅拓扑，不含状态）
    pub fn to_graph(&self) -> Result<NebulaGraph, String> {
        let mut graph = NebulaGraph::new();
        let mut node_id_map: HashMap<String, usize> = HashMap::new();

        for gnode in &self.graph.nodes {
            let _id_num: usize = gnode
                .id
                .trim_start_matches('n')
                .parse()
                .map_err(|_| format!("invalid node id: {}", gnode.id))?;

            let expr = match detect_formula_type(&gnode.formula) {
                "eq" => parse_simple_eq(&gnode.formula)
                    .ok_or_else(|| format!("cannot parse eq: {}", gnode.formula))?,
                "constant" => {
                    let val: f64 = gnode.formula.trim().parse()
                        .map_err(|_| format!("invalid constant: {}", gnode.formula))?;
                    Expr::Number(val)
                }
                _ => parse_expression(&gnode.formula)
                    .map_err(|e| format!("parse error: {} (formula: {})", e, gnode.formula))?,
            };

            let nid = graph.add_node(expr);
            if let Some(node) = graph.nodes.iter_mut().find(|n| n.id == nid) {
                node.is_dynamic = gnode.node_type == "dynamic";
            }
            node_id_map.insert(gnode.id.clone(), nid);
        }

        for gsyn in &self.graph.synapses {
            let (from_node_str, from_sym) = gsyn
                .from
                .split_once('.')
                .ok_or_else(|| format!("invalid synapse from: {}", gsyn.from))?;
            let (to_node_str, to_sym) = gsyn
                .to
                .split_once('.')
                .ok_or_else(|| format!("invalid synapse to: {}", gsyn.to))?;

            let from_id = *node_id_map
                .get(from_node_str)
                .ok_or_else(|| format!("unknown node: {}", from_node_str))?;
            let to_id = *node_id_map
                .get(to_node_str)
                .ok_or_else(|| format!("unknown node: {}", to_node_str))?;

            graph.add_edge(from_id, from_sym, to_id, to_sym);
        }

        graph.topology_version = self.version.topology_version;
        Ok(graph)
    }

    /// 将 state.json 中的状态应用到 graph
    pub fn apply_state(&self, graph: &mut NebulaGraph) -> Result<(), String> {
        for (node_key, entry) in &self.state.nodes {
            let id_str = node_key.trim_start_matches('n');
            let node_id: usize = id_str.parse()
                .map_err(|_| format!("invalid node key: {}", node_key))?;

            if let Some(state) = status_to_state(&entry.status) {
                if let Some(node) = graph.nodes.iter_mut().find(|n| n.id == node_id) {
                    node.state = state;
                    if let Some(val) = entry.current_buffer.get("output") {
                        node.value = Some(*val);
                    }
                }
            }

            for edge in &mut graph.edges {
                if edge.from_node == node_id {
                    if let Some(val) = entry.current_buffer.get(&edge.from_symbol) {
                        edge.delay_buffer = Some(*val);
                    }
                }
            }
        }
        Ok(())
    }

    /// 从 state.json 构建 env HashMap
    pub fn to_env(&self) -> HashMap<(usize, String), f64> {
        let mut env = HashMap::new();
        for (node_key, entry) in &self.state.nodes {
            if let Ok(node_id) = node_key.trim_start_matches('n').parse::<usize>() {
                for (sym, val) in &entry.current_buffer {
                    env.insert((node_id, sym.clone()), *val);
                }
            }
        }
        env
    }
}

// ============================================================
// 11. BrainstormFile — pack/unpack/load_for_tick
// ============================================================

/// .brainstorm 文件的打包/解包 API（与朋友设计的接口一致）
pub struct BrainstormFile {
    pub path: String,
}

impl BrainstormFile {
    /// 创建一个指向目标路径的 .brainstorm 文件处理器
    ///
    /// ```ignore
    /// let bsf = BrainstormFile::new("snapshot.brainstorm");
    /// bsf.pack(&graph_xml_str, &state, &clusters, &runtime, None)?;
    /// ```
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        BrainstormFile {
            path: path.as_ref().to_string_lossy().to_string(),
        }
    }

    /// 打包写入 .brainstorm 文件
    ///
    /// `graph_xml`: graph.xml 的文本内容
    /// `state`:     状态数据（state.json）
    /// `clusters`:  集群数据（clusters.json）
    /// `runtime`:   运行时元信息（runtime.meta.json）
    /// `assets`:    可选资源文件列表 (name, data)
    pub fn pack(
        &self,
        graph_xml: &str,
        state: &StateJson,
        clusters: &ClustersJson,
        runtime: &RuntimeMetaJson,
        assets: Option<&Vec<(String, Vec<u8>)>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let file = File::create(&self.path)?;
        let mut zip = zip::ZipWriter::new(file);

        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        zip.start_file("graph.xml", options)?;
        zip.write_all(graph_xml.as_bytes())?;

        zip.start_file("state.json", options)?;
        zip.write_all(serde_json::to_string_pretty(state)?.as_bytes())?;

        zip.start_file("clusters.json", options)?;
        zip.write_all(serde_json::to_string_pretty(clusters)?.as_bytes())?;

        zip.start_file("runtime.meta.json", options)?;
        zip.write_all(serde_json::to_string_pretty(runtime)?.as_bytes())?;

        let version_txt = format!(
            "brainstorm_version=1\nsymnebula_core=0.1.0\ntopology_version={}\n",
            runtime.topology_version,
        );
        zip.start_file("version.txt", options)?;
        zip.write_all(version_txt.as_bytes())?;

        if let Some(files) = assets {
            for (name, data) in files {
                zip.start_file(format!("assets/{}", name), options)?;
                zip.write_all(data)?;
            }
        }

        zip.finish()?;
        Ok(())
    }

    /// 解包读取 .brainstorm 文件
    ///
    /// 返回 (graph_xml, state, clusters, runtime, assets)
    pub fn unpack(
        &self,
    ) -> Result<
        (
            String,                         // graph.xml
            StateJson,                      // state.json
            ClustersJson,                   // clusters.json
            RuntimeMetaJson,                // runtime.meta.json
            Vec<(String, Vec<u8>)>,         // assets/
        ),
        Box<dyn std::error::Error>,
    > {
        let file = File::open(&self.path)?;
        let mut zip = zip::ZipArchive::new(file)?;

        let mut graph_xml = String::new();
        let mut state_json = String::new();
        let mut clusters_json = String::new();
        let mut runtime_json = String::new();
        let mut assets: Vec<(String, Vec<u8>)> = Vec::new();

        for i in 0..zip.len() {
            let mut entry = zip.by_index(i)?;
            let name = entry.name().to_string();

            if name == "graph.xml" {
                entry.read_to_string(&mut graph_xml)?;
            } else if name == "state.json" {
                entry.read_to_string(&mut state_json)?;
            } else if name == "clusters.json" {
                entry.read_to_string(&mut clusters_json)?;
            } else if name == "runtime.meta.json" {
                entry.read_to_string(&mut runtime_json)?;
            } else if name.starts_with("assets/") {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                assets.push((name.replace("assets/", ""), buf));
            }
            // version.txt — silently ignored, metadata is in runtime
        }

        let state: StateJson = serde_json::from_str(&state_json)
            .map_err(|e| format!("state.json 解析失败: {}", e))?;
        let clusters: ClustersJson = serde_json::from_str(&clusters_json)
            .map_err(|e| format!("clusters.json 解析失败: {}", e))?;
        let runtime: RuntimeMetaJson = serde_json::from_str(&runtime_json)
            .map_err(|e| format!("runtime.meta.json 解析失败: {}", e))?;

        Ok((graph_xml, state, clusters, runtime, assets))
    }

    /// Tick 恢复 + ClusterCache 增量更新
    ///
    /// 如果 `previous_clusters` 的 topology_version 与当前一致，
    /// 直接复用缓存（零成本增量更新）。
    ///
    /// 返回 (state, clusters, runtime)
    pub fn load_for_tick(
        &self,
        previous_clusters: Option<&ClustersJson>,
    ) -> Result<(StateJson, ClustersJson, RuntimeMetaJson), Box<dyn std::error::Error>> {
        let (_graph, state, mut clusters, runtime, _assets) = self.unpack()?;

        if let Some(prev) = previous_clusters {
            if prev.topology_version == clusters.topology_version {
                clusters = prev.clone();
            }
        }

        Ok((state, clusters, runtime))
    }
}

// ============================================================
// 12. 兼容接口：Brainstorm ↔ ZIP 直接读写
// ============================================================

impl BrainstormFile {
    /// 将 Brainstorm 写入 ZIP 文件
    pub fn write_to(&self, bsi: &Brainstorm) -> Result<(), String> {
        let pkg = bsi.to_package();
        let file = File::create(&self.path).map_err(|e| format!("create file: {}", e))?;
        let mut zipw = zip::ZipWriter::new(file);

        let names = [
            "version.txt",
            "graph.xml",
            "state.json",
            "clusters.json",
            "runtime.meta.json",
        ];

        for name in &names {
            let content = pkg.get(*name).ok_or_else(|| format!("missing: {}", name))?;
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zipw.start_file(*name, opts)
                .map_err(|e| format!("zip start {}: {}", name, e))?;
            zipw.write_all(content.as_bytes())
                .map_err(|e| format!("zip write {}: {}", name, e))?;
        }

        zipw.finish().map_err(|e| format!("zip finish: {}", e))?;
        Ok(())
    }

    /// 从 ZIP 文件读取 Brainstorm
    pub fn read_from(&self) -> Result<Brainstorm, String> {
        let file = File::open(&self.path).map_err(|e| format!("open file: {}", e))?;
        let mut arch = zip::ZipArchive::new(file).map_err(|e| format!("zip open: {}", e))?;

        let mut gx = String::new();
        let mut sj = String::new();
        let mut cj = String::new();
        let mut mj = String::new();
        let mut vt = String::new();

        for i in 0..arch.len() {
            let mut file_entry = arch.by_index(i)
                .map_err(|e| format!("entry {}: {}", i, e))?;
            let name = file_entry.name().to_string();
            let mut c = String::new();
            file_entry.read_to_string(&mut c)
                .map_err(|e| format!("read {}: {}", name, e))?;
            match name.as_str() {
                "graph.xml" => gx = c,
                "state.json" => sj = c,
                "clusters.json" => cj = c,
                "runtime.meta.json" => mj = c,
                "version.txt" => vt = c,
                _ if name.starts_with("assets/") => {}
                _ => return Err(format!("unknown file: {}", name)),
            }
        }

        Ok(Brainstorm {
            graph: GraphXml::parse(&gx)?,
            state: StateJson::parse(&sj)?,
            clusters: ClustersJson::parse(&cj)?,
            meta: RuntimeMetaJson::parse(&mj)?,
            assets: Vec::new(),
            version: VersionTxt::parse(&vt)?,
        })
    }
}

// ============================================================
// 13. Expr Display — 公式 → 文本（用于序列化）
// ============================================================

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Number(n) => write!(f, "{}", n),
            Expr::Symbol(s) => write!(f, "{}", s),
            Expr::Neg(a) => write!(f, "-({})", a),
            Expr::Add(a, b) => write!(f, "({}) + ({})", a, b),
            Expr::Sub(a, b) => write!(f, "({}) - ({})", a, b),
            Expr::Mul(a, b) => write!(f, "({}) * ({})", a, b),
            Expr::Div(a, b) => write!(f, "({}) / ({})", a, b),
            Expr::Pow(a, b) => write!(f, "({}) ^ ({})", a, b),
            Expr::Eq(l, r) => write!(f, "({}) = ({})", l, r),
        }
    }
}

// ============================================================
// 14. Tick Restore — 从 Brainstorm 恢复 ClusterSolverV3
// ============================================================

/// 从 Brainstorm 快照完整恢复运行时状态。
pub fn restore_tick(bsi: &Brainstorm) -> Result<(ClusterSolverV3, NebulaGraph), String> {
    let mut graph = bsi.to_graph()?;
    bsi.apply_state(&mut graph)?;

    let mut solver = ClusterSolverV3::new();
    let mut cache = ClusterCache::new();
    solver.compile(&graph, &mut cache);

    if let Some(ref comp) = solver.compilation {
        for (ci, cluster_comp) in comp.clusters.iter().enumerate() {
            if ci >= solver.cluster_xs.len() {
                break;
            }
            for node_id in &cluster_comp.node_ids {
                let node_key = format!("n{}", node_id);
                if let Some(entry) = bsi.state.nodes.get(&node_key) {
                    for (sym, val) in &entry.current_buffer {
                        let port = VarPort::new(*node_id, sym);
                        if let Some(&gidx) = cluster_comp.global_idx_map.get(&port) {
                            if gidx < solver.cluster_xs[ci].len() {
                                solver.cluster_xs[ci][gidx] = *val;
                            }
                        }
                    }
                }
            }
        }
    }

    Ok((solver, graph))
}

// ============================================================
// 15. 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parse_simple_eq;
    use crate::graph::NebulaGraph;

    /// graph.xml 输出基本验证
    #[test]
    fn test_graph_xml_basic() {
        let mut g = NebulaGraph::new();
        g.add_node(parse_simple_eq("x + y = 10").unwrap());
        g.add_node(parse_simple_eq("x * z = 1").unwrap());

        let bsi = Brainstorm::from_engine(&g, None, None, 0);
        let xml = bsi.to_graph_xml();

        assert!(xml.contains("<nebula"));
        assert!(xml.contains("<nodes>"));
        assert!(xml.contains("</nebula>"));
        assert!(xml.contains("n0"));
        assert!(xml.contains("n1"));
        assert!(xml.contains("x"));
        assert!(xml.contains("y"));
        assert!(xml.contains("10"));
    }

    /// graph.xml roundtrip
    #[test]
    fn test_graph_xml_roundtrip() {
        let mut g = NebulaGraph::new();
        let n0 = g.add_node(parse_simple_eq("x + y = 10").unwrap());
        let n1 = g.add_node(parse_simple_eq("x * z = 1").unwrap());
        g.add_edge_with_default(n0, "x", n1, "x", 0.0);

        let bsi = Brainstorm::from_engine(&g, None, None, 0);
        let g2 = bsi.to_graph().unwrap();
        assert_eq!(g2.nodes.len(), 2);
        assert_eq!(g2.edges.len(), 1);
    }

    /// state.json 输出验证双缓冲 + 状态彩条
    #[test]
    fn test_state_json_dual_buffer() {
        let mut g = NebulaGraph::new();
        g.add_node(parse_simple_eq("x + y = 10").unwrap());

        let bsi = Brainstorm::from_engine(&g, None, None, 42);
        let json = bsi.to_state_json();

        assert!(json.contains("\"current_buffer\""));
        assert!(json.contains("\"next_buffer\""));
        assert!(json.contains("\"status\""));
        assert!(json.contains("n0"));
    }

    /// state.json serde roundtrip
    #[test]
    fn test_state_json_roundtrip() {
        let mut g = NebulaGraph::new();
        g.add_node(parse_simple_eq("x + y = 10").unwrap());

        let bsi1 = Brainstorm::from_engine(&g, None, None, 0);
        let json = bsi1.to_state_json();
        let parsed = StateJson::parse(&json).unwrap();

        assert!(parsed.nodes.contains_key("n0"));
        assert_eq!(parsed.nodes["n0"].status, "Gray");
    }

    /// clusters.json 输出验证 blocks + variables
    #[test]
    fn test_clusters_json_with_blocks() {
        let mut g = NebulaGraph::new();
        g.add_node(parse_simple_eq("x + y = 10").unwrap());
        g.add_node(parse_simple_eq("x - y = 2").unwrap());
        g.add_edge_with_default(0, "x", 1, "x", 0.0);
        g.add_edge_with_default(0, "y", 1, "y", 0.0);

        let mut solver = ClusterSolverV3::new();
        let mut cache = ClusterCache::new();
        solver.compile(&g, &mut cache);

        let bsi = Brainstorm::from_engine(&g, Some(&solver), None, 0);
        let json = bsi.to_clusters_json();

        assert!(json.contains("\"clusters\""));
        assert!(json.contains("\"cluster_0\""));
        assert!(json.contains("\"blocks\""));
        assert!(json.contains("\"topology_version\""));
    }

    /// runtime.meta.json 输出验证
    #[test]
    fn test_meta_json_basic() {
        let mut g = NebulaGraph::new();
        g.add_node(parse_simple_eq("x + 1 = 5").unwrap());

        let bsi = Brainstorm::from_engine(&g, None, None, 42);
        let json = bsi.to_meta_json();

        assert!(json.contains("\"last_tick\""));
        assert!(json.contains("42"));
        assert!(json.contains("\"solver_settings\""));
        assert!(json.contains("\"tolerance\""));
    }

    /// 完整包输出验证 5 个文件
    #[test]
    fn test_full_package() {
        let mut g = NebulaGraph::new();
        g.add_node(parse_simple_eq("x + y = 7").unwrap());
        g.add_node(parse_simple_eq("x - y = 1").unwrap());
        let n2 = g.add_node(parse_simple_eq("2*x + z = 10").unwrap());
        g.add_edge_with_default(0, "x", 1, "x", 0.0);
        g.add_edge_with_default(0, "y", 1, "y", 0.0);
        g.add_edge_with_default(1, "x", n2, "x", 0.0);

        let mut solver = ClusterSolverV3::new();
        let mut cache = ClusterCache::new();
        solver.compile(&g, &mut cache);

        let bsi = Brainstorm::from_engine(&g, Some(&solver), None, 0);
        let pkg = bsi.to_package();

        assert!(pkg.contains_key("graph.xml"));
        assert!(pkg.contains_key("state.json"));
        assert!(pkg.contains_key("clusters.json"));
        assert!(pkg.contains_key("runtime.meta.json"));
        assert!(pkg.contains_key("version.txt"));

        let ver = &pkg["version.txt"];
        assert!(ver.contains("brainstorm_version=1"));

        let g2 = bsi.to_graph().unwrap();
        assert_eq!(g2.nodes.len(), 3);
        assert_eq!(g2.edges.len(), 3);
    }

    /// 带 synapses 的 XML roundtrip
    #[test]
    fn test_synapses_in_xml() {
        let mut g = NebulaGraph::new();
        let n0 = g.add_node(parse_simple_eq("a + b = 3").unwrap());
        let n1 = g.add_node(parse_simple_eq("a * b = 2").unwrap());
        g.add_edge_with_default(n0, "a", n1, "a", 0.5);
        g.add_edge_with_default(n0, "b", n1, "b", 2.0);

        let bsi = Brainstorm::from_engine(&g, None, None, 0);
        let xml = bsi.to_graph_xml();

        assert!(xml.contains("<synapses>"));
        assert!(xml.contains("from=\"n0.a\""));
        assert!(xml.contains("to=\"n1.b\""));
        assert!(xml.contains("</synapses>"));

        let g2 = bsi.to_graph().unwrap();
        assert_eq!(g2.nodes.len(), 2);
        assert_eq!(g2.edges.len(), 2);
    }

    /// 端口推断验证
    #[test]
    fn test_port_inference_eq() {
        let mut g = NebulaGraph::new();
        let n0 = g.add_node(parse_simple_eq("x + y = 10").unwrap());
        let n1 = g.add_node(parse_simple_eq("x = 5").unwrap());
        g.add_edge_with_default(n1, "x", n0, "x", 5.0);

        let xml = format!("{}", GraphXml::from_graph(&g));
        assert!(xml.contains("<ports>"));
        assert!(xml.contains("<input"));
        assert!(xml.contains("<output"));
    }

    /// Dynamic 节点 type 属性
    #[test]
    fn test_dynamic_node_type() {
        let mut g = NebulaGraph::new();
        let nid = g.add_node(parse_simple_eq("v_new = v + a * dt").unwrap());
        if let Some(node) = g.nodes.iter_mut().find(|node| node.id == nid) {
            node.is_dynamic = true;
        }

        let bsi = Brainstorm::from_engine(&g, None, None, 0);
        let xml = bsi.to_graph_xml();
        assert!(xml.contains("type=\"dynamic\""));
    }

    /// Full roundtrip: from_engine → to_package → parse → to_graph
    #[test]
    fn test_full_brainstorm_roundtrip() {
        let mut g = NebulaGraph::new();
        let n0 = g.add_node(parse_simple_eq("x + y = 7").unwrap());
        g.add_edge_with_default(n0, "x", n0, "y", 0.0);

        let bsi1 = Brainstorm::from_engine(&g, None, None, 0);
        let pkg = bsi1.to_package();

        let gx = GraphXml::parse(&pkg["graph.xml"]).unwrap();
        let sj = StateJson::parse(&pkg["state.json"]).unwrap();
        let cj = ClustersJson::parse(&pkg["clusters.json"]).unwrap();
        let mj = RuntimeMetaJson::parse(&pkg["runtime.meta.json"]).unwrap();
        let vt = VersionTxt::parse(&pkg["version.txt"]).unwrap();

        let bsi2 = Brainstorm {
            graph: gx,
            state: sj,
            clusters: cj,
            meta: mj,
            assets: Vec::new(),
            version: vt,
        };

        let g2 = bsi2.to_graph().unwrap();
        assert_eq!(g2.nodes.len(), 1);
        assert_eq!(g2.edges.len(), 1);
        assert_eq!(g2.topology_version, g.topology_version);
    }

    /// Expr Display 基本功能
    #[test]
    fn test_expr_display() {
        let expr = parse_simple_eq("x + y = 10").unwrap();
        let s = format!("{}", expr);
        assert!(s.contains("x"));
        assert!(s.contains("y"));
        assert!(s.contains("10"));
    }

    /// 状态转换函数验证
    #[test]
    fn test_status_conversion() {
        assert_eq!(state_to_status(&NodeState::Green), "Green");
        assert_eq!(state_to_status(&NodeState::Yellow), "Yellow");
        assert_eq!(state_to_status(&NodeState::Purple), "Purple");
        assert_eq!(state_to_status(&NodeState::Gray), "Gray");

        assert_eq!(status_to_state("Green"), Some(NodeState::Green));
        assert_eq!(status_to_state("Yellow"), Some(NodeState::Yellow));
        assert_eq!(status_to_state("Purple"), Some(NodeState::Purple));
        assert_eq!(status_to_state("Gray"), Some(NodeState::Gray));
        assert_eq!(status_to_state("gray"), Some(NodeState::Gray));
        assert_eq!(status_to_state("unknown"), None);
    }

    /// 公式类型检测
    #[test]
    fn test_detect_formula_type() {
        assert_eq!(detect_formula_type("x + y = 10"), "eq");
        assert_eq!(detect_formula_type("42"), "constant");
        assert_eq!(detect_formula_type(" 3.14 "), "constant");
        assert_eq!(detect_formula_type("x + y"), "expr");
    }

    /// pack/unpack 文件 I/O roundtrip
    #[test]
    fn test_pack_unpack_file_io() {
        let mut g = NebulaGraph::new();
        g.add_node(parse_simple_eq("x + y = 10").unwrap());

        let bsi = Brainstorm::from_engine(&g, None, None, 5);
        let graph_xml = bsi.to_graph_xml();
        let path = "/tmp/test_brainstorm_pack.brainstorm";
        let bsf = BrainstormFile::new(path);

        // pack
        bsf.pack(&graph_xml, &bsi.state, &bsi.clusters, &bsi.meta, None)
            .expect("pack 失败");

        // unpack
        let (_gx, state, _clusters, meta, _assets) = bsf.unpack().expect("unpack 失败");
        assert!(state.nodes.contains_key("n0"));
        assert_eq!(meta.last_tick, 5);

        // read_from
        let bsi2 = bsf.read_from().expect("read_from 失败");
        assert_eq!(bsi2.graph.nodes.len(), 1);

        // write_to
        let bsf2 = BrainstormFile::new("/tmp/test_brainstorm_write.brainstorm");
        bsf2.write_to(&bsi).expect("write_to 失败");
        let bsi3 = bsf2.read_from().expect("read_from 失败");
        assert_eq!(bsi3.graph.nodes.len(), 1);

        // cleanup
        let _ = std::fs::remove_file("/tmp/test_brainstorm_pack.brainstorm");
        let _ = std::fs::remove_file("/tmp/test_brainstorm_write.brainstorm");
    }

    /// load_for_tick 增量 ClusterCache
    #[test]
    fn test_load_for_tick_incremental() {
        let mut g = NebulaGraph::new();
        g.add_node(parse_simple_eq("x + y = 10").unwrap());

        let bsi = Brainstorm::from_engine(&g, None, None, 0);
        let path = "/tmp/test_brainstorm_tick.brainstorm";
        let bsf = BrainstormFile::new(path);
        bsf.write_to(&bsi).expect("write_to 失败");

        // 首次加载（无 previous）
        let (state, clusters, meta) = bsf.load_for_tick(None).expect("load_for_tick 失败");
        assert_eq!(meta.last_tick, 0);

        // 再次加载（复用 previous）
        let (state2, _clusters2, _meta2) = bsf
            .load_for_tick(Some(&clusters))
            .expect("load_for_tick 增量失败");
        assert_eq!(state2.nodes.len(), state.nodes.len());

        let _ = std::fs::remove_file(path);
    }

    /// 完整增量 Tick demo — 匹配你朋友的设计示例
    ///
    /// 流程: 手动构建状态 → pack → load_for_tick(复用缓存) → 更新状态 → pack
    #[test]
    fn test_incremental_tick_demo() {
        let path = "/tmp/test_incremental_tick_demo.brainstorm";
        let bsf = BrainstormFile::new(path);

        // ============================================================
        // Step 0: 手动构造节点状态（模拟 Tick 0 结果）
        // ============================================================
        let mut nodes: HashMap<String, StateEntry> = HashMap::new();
        nodes.insert(
            "A".to_string(),
            StateEntry {
                current_buffer: HashMap::from([("x".to_string(), 0.0)]),
                next_buffer: HashMap::from([("x".to_string(), 0.0)]),
                status: "Green".into(),
            },
        );
        nodes.insert(
            "B".to_string(),
            StateEntry {
                current_buffer: HashMap::from([("y".to_string(), 1.0)]),
                next_buffer: HashMap::from([("y".to_string(), 1.0)]),
                status: "Green".into(),
            },
        );
        nodes.insert(
            "C".to_string(),
            StateEntry {
                current_buffer: HashMap::from([("z".to_string(), 0.0)]),
                next_buffer: HashMap::from([("z".to_string(), 0.0)]),
                status: "Green".into(),
            },
        );
        let state = StateJson { nodes };

        // ============================================================
        // Step 1: 构建 ClusterCache
        // ============================================================
        let cluster0 = ClusterEntry {
            id: "cluster0".to_string(),
            nodes: vec!["A".to_string(), "B".to_string()],
            variables: HashMap::from([("x".to_string(), 0usize), ("y".to_string(), 1)]),
            blocks: vec![BlockEntry {
                id: "block0".to_string(),
                vars: vec![0, 1],
            }],
        };
        let cluster1 = ClusterEntry {
            id: "cluster1".to_string(),
            nodes: vec!["C".to_string()],
            variables: HashMap::from([("z".to_string(), 0usize)]),
            blocks: vec![BlockEntry {
                id: "block0".to_string(),
                vars: vec![0],
            }],
        };
        let clusters = ClustersJson {
            topology_version: 1,
            clusters: vec![cluster0, cluster1],
        };

        // ============================================================
        // Step 2: Runtime meta
        // ============================================================
        let runtime = RuntimeMetaJson {
            last_tick: 0,
            tick_time_ms: 16.0,
            global_vars_count: 3,
            solver_settings: SolverSettings {
                tolerance: 1e-9,
                max_iterations: 50,
            },
            topology_version: 1,
        };

        // ============================================================
        // Step 3: graph.xml
        // ============================================================
        let graph_xml = r#"<nebula name='MyNebula' version='1.0'>
        <nodes>
            <node id='A' type='formula'><formula>x + y = 10</formula></node>
            <node id='B' type='formula'><formula>x = z^2</formula></node>
            <node id='C' type='dynamic'><formula>z' + y = 1</formula></node>
        </nodes>
        <synapses>
            <synapse from='C.z' to='B.z'/>
            <synapse from='B.x' to='A.x'/>
        </synapses>
    </nebula>"#;

        // ============================================================
        // Step 4: 打包 Tick 0
        // ============================================================
        bsf.pack(graph_xml, &state, &clusters, &runtime, None)
            .expect("第一次 pack 失败");
        assert!(std::path::Path::new(path).exists());

        // ============================================================
        // Step 5: load_for_tick — 增量复用 ClusterCache
        // ============================================================
        let previous_clusters = Some(&clusters);
        let (mut state_tick1, clusters_tick1, runtime_tick1) = bsf
            .load_for_tick(previous_clusters)
            .expect("load_for_tick 失败");

        // 验证 topology_version 相同 → 缓存被复用（指针相等）
        assert_eq!(
            clusters_tick1.topology_version,
            clusters.topology_version,
            "topology_version 应一致，缓存被复用"
        );

        // 模拟 Tick 求解后的状态更新
        state_tick1
            .nodes
            .get_mut("A")
            .expect("节点 A 应存在")
            .status = "Green".into();
        state_tick1
            .nodes
            .get_mut("B")
            .expect("节点 B 应存在")
            .status = "Yellow".into();
        state_tick1
            .nodes
            .get_mut("C")
            .expect("节点 C 应存在")
            .status = "Purple".into();

        // ============================================================
        // Step 6: 再次打包 Tick 1
        // ============================================================
        bsf.pack(graph_xml, &state_tick1, &clusters_tick1, &runtime_tick1, None)
            .expect("第二次 pack 失败");

        // 验证 Tick 1 状态
        let (_, final_state, _, final_meta, _) = bsf.unpack().expect("unpack 失败");
        assert_eq!(final_meta.last_tick, 0); // runtime 未修改，但数据已持久化
        assert_eq!(
            final_state.nodes.get("A").unwrap().status,
            "Green",
            "A 应保持 Green"
        );
        assert_eq!(
            final_state.nodes.get("B").unwrap().status,
            "Yellow",
            "B 应变为 Yellow"
        );
        assert_eq!(
            final_state.nodes.get("C").unwrap().status,
            "Purple",
            "C 应变为 Purple"
        );

        let _ = std::fs::remove_file(path);
    }
}
