/// .brainstorm v1 — 确定性计算的可序列化宇宙状态
///
/// 纯标准库实现，零外部依赖。
/// 序列化：NebulaGraph + ClusterSolverV3 → .brainstorm (XML + JSON)
/// 反序列化：.brainstorm → NebulaGraph + 状态恢复
///
/// 文件结构：
///   .brainstorm (zip) — 暂未实现 zip，当前为纯文本输出
///     ├── graph.xml
///     ├── state.json
///     ├── clusters.json
///     ├── runtime.meta.json
///     ├── snapshots/tick_xxxx.json
///     └── version.txt

use std::collections::HashMap;
use std::fmt;

use crate::ast::{parse_expression, parse_simple_eq, Expr};
use crate::graph::{NebulaGraph, Node};
use crate::state::NodeState;
use crate::cluster::{ClusterCache, ClusterCompilation, ClusterSolverV3, TickCompilation};

// ============================================================
// 数据结构：.brainstorm 各层
// ============================================================

/// .brainstorm 完整容器
pub struct Brainstorm {
    pub graph: GraphXml,
    pub state: StateJson,
    pub clusters: ClustersJson,
    pub meta: RuntimeMetaJson,
    pub snapshots: Vec<SnapshotJson>,
    pub version: VersionTxt,
}

/// graph.xml
pub struct GraphXml {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

pub struct GraphNode {
    pub id: String,
    pub node_type: String, // "eq" | "expr" | "constant"
    pub expr: String,
    pub is_dynamic: bool,
}

pub struct GraphEdge {
    pub from: String, // "node_id:port"
    pub to: String,
}

/// state.json
pub struct StateJson {
    pub tick: u64,
    pub variables: HashMap<String, f64>,
    pub node_state: HashMap<String, String>, // "n1" → "Green"|"Yellow"|"Purple"|"Gray"
    pub x_cluster_cache: HashMap<String, Vec<f64>>,
}

/// clusters.json
pub struct ClustersJson {
    pub topology_version: u64,
    pub clusters: Vec<ClusterEntry>,
}

pub struct ClusterEntry {
    pub id: String,
    pub nodes: Vec<String>,
}

/// runtime.meta.json
pub struct RuntimeMetaJson {
    pub tick_step: u64,
    pub solver: String,
    pub tolerance: f64,
    pub max_iter: u64,
    pub rollback_policy: String,
    pub color_rules: HashMap<String, String>,
}

/// snapshots/tick_xxxx.json
pub struct SnapshotJson {
    pub tick: u64,
    pub cluster_states: HashMap<String, ClusterStateEntry>,
}

pub struct ClusterStateEntry {
    pub x_cluster: Vec<f64>,
    pub status: String,
}

/// version.txt
pub struct VersionTxt {
    pub brainstorm_version: u64,
    pub symnebula_core: String,
    pub topology_version: u64,
}

// ============================================================
// 构建器：从 SymNebula 对象 → Brainstorm
// ============================================================

impl Brainstorm {
    /// 从运行中的 ClusterSolverV3 构建 .brainstorm 快照
    pub fn from_solver(
        graph: &NebulaGraph,
        solver: &ClusterSolverV3,
        tick: u64,
    ) -> Self {
        let graph_xml = GraphXml::from_graph(graph);
        let state = StateJson::from_solver(graph, solver, tick);
        let clusters = ClustersJson::from_solver(solver);
        let meta = RuntimeMetaJson::default();
        let version = VersionTxt {
            brainstorm_version: 1,
            symnebula_core: "0.1.0".into(),
            topology_version: graph.topology_version,
        };
        let snapshots = Vec::new();

        Brainstorm {
            graph: graph_xml,
            state,
            clusters,
            meta,
            snapshots,
            version,
        }
    }
}

impl GraphXml {
    pub fn from_graph(graph: &NebulaGraph) -> Self {
        let mut nodes = Vec::new();
        for node in &graph.nodes {
            let (node_type, expr_str) = match &node.formula {
                Expr::Eq(_, _) => {
                    // 用 Display 输出
                    ("eq".into(), format!("{}", node.formula))
                }
                Expr::Number(_) => {
                    ("constant".into(), format!("{}", node.formula))
                }
                _ => {
                    ("expr".into(), format!("{}", node.formula))
                }
            };
            nodes.push(GraphNode {
                id: format!("n{}", node.id),
                node_type,
                expr: expr_str,
                is_dynamic: node.is_dynamic,
            });
        }

        let mut edges = Vec::new();
        for edge in &graph.edges {
            edges.push(GraphEdge {
                from: format!("n{}:{}", edge.from_node, edge.from_symbol),
                to: format!("n{}:{}", edge.to_node, edge.to_symbol),
            });
        }

        GraphXml { nodes, edges }
    }
}

impl StateJson {
    pub fn from_solver(
        graph: &NebulaGraph,
        solver: &ClusterSolverV3,
        tick: u64,
    ) -> Self {
        let mut variables = HashMap::new();
        // 从 solver 中获取所有全局变量的值（通过遍历 graph nodes + symbols 尝试 get_value）
        for node in &graph.nodes {
            for sym in node.formula.symbols() {
                if let Some(val) = solver.get_value(node.id, &sym) {
                    variables.insert(format!("n{}_{}", node.id, sym), val);
                }
            }
        }

        let mut node_state = HashMap::new();
        // 当前从 graph 读取节点状态（solver 不直接更新 graph 的 node state）
        for node in &graph.nodes {
            let state_label = match node.state {
                NodeState::Green => "Green",
                NodeState::Yellow => "Yellow",
                NodeState::Purple => "Purple",
                NodeState::Gray => "Gray",
            };
            node_state.insert(format!("n{}", node.id), state_label.into());
        }

        let x_cluster_cache = HashMap::new(); // 暂不序列化

        StateJson {
            tick,
            variables,
            node_state,
            x_cluster_cache,
        }
    }
}

impl ClustersJson {
    pub fn from_solver(solver: &ClusterSolverV3) -> Self {
        let mut clusters = Vec::new();
        if let Some(comp) = &solver.compilation {
            for (ci, cluster_comp) in comp.clusters.iter().enumerate() {
                let nodes: Vec<String> = cluster_comp.node_ids
                    .iter()
                    .map(|nid| format!("n{}", nid))
                    .collect();
                clusters.push(ClusterEntry {
                    id: format!("c{}", ci),
                    nodes,
                });
            }
        }
        ClustersJson {
            topology_version: 0, // 无直接访问
            clusters,
        }
    }
}

impl RuntimeMetaJson {
    pub fn default() -> Self {
        let mut color_rules = HashMap::new();
        color_rules.insert("green".into(), "residual < 1e-6".into());
        color_rules.insert("yellow".into(), "converging".into());
        color_rules.insert("purple".into(), "diverged or singular".into());

        RuntimeMetaJson {
            tick_step: 1,
            solver: "newton_block".into(),
            tolerance: 1e-6,
            max_iter: 20,
            rollback_policy: "cluster_atomic".into(),
            color_rules,
        }
    }
}

// ============================================================
// XML 序列化（纯 std 实现）
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
        writeln!(f, r#"<graph version="1">"#)?;
        writeln!(f, "    <nodes>")?;
        for node in &self.nodes {
            let dynamic_attr = if node.is_dynamic { r#" dynamic="true""# } else { "" };
            writeln!(
                f,
                r#"        <node id="{}" type="{}{}>"#,
                escape_xml(&node.id),
                escape_xml(&node.node_type),
                dynamic_attr,
            )?;
            writeln!(f, r#"            <expr>{}</expr>"#, escape_xml(&node.expr))?;
            writeln!(f, r#"        </node>"#)?;
        }
        writeln!(f, "    </nodes>")?;
        writeln!(f, "    <edges>")?;
        for edge in &self.edges {
            writeln!(
                f,
                r#"        <edge from="{}" to="{}"/>"#,
                escape_xml(&edge.from),
                escape_xml(&edge.to),
            )?;
        }
        writeln!(f, "    </edges>")?;
        writeln!(f, "</graph>")
    }
}

// ============================================================
// JSON 序列化（纯 std 实现，minimal json emitter）
// ============================================================

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// 简易 JSON 值枚举（纯 std，不依赖 serde）
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<JsonValue>),
    Obj(HashMap<String, JsonValue>),
}

impl fmt::Display for JsonValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsonValue::Null => write!(f, "null"),
            JsonValue::Bool(b) => write!(f, "{}", b),
            JsonValue::Number(n) => {
                if n.fract() == 0.0 && n.is_finite() {
                    write!(f, "{}", *n as i64)
                } else {
                    write!(f, "{:.15}", n)
                }
            }
            JsonValue::Str(s) => write!(f, "\"{}\"", json_escape(s)),
            JsonValue::Array(arr) => {
                write!(f, "[")?;
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            JsonValue::Obj(map) => {
                write!(f, "{{")?;
                let mut first = true;
                // 按 key 排序确保确定性输出
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                for key in keys {
                    if !first { write!(f, ", ")?; }
                    first = false;
                    write!(f, "\"{}\": {}", json_escape(key), map[key])?;
                }
                write!(f, "}}")
            }
        }
    }
}

// ============================================================
// 生成 .brainstorm 完整文本输出
// ============================================================

impl Brainstorm {
    /// 生成 graph.xml 内容
    pub fn to_graph_xml(&self) -> String {
        format!("{}", self.graph)
    }

    /// 生成 state.json 内容
    pub fn to_state_json(&self) -> String {
        let mut map: HashMap<String, JsonValue> = HashMap::new();

        map.insert("tick".into(), JsonValue::Number(self.state.tick as f64));

        let mut vars: HashMap<String, JsonValue> = HashMap::new();
        let mut var_keys: Vec<&String> = self.state.variables.keys().collect();
        var_keys.sort();
        for key in var_keys {
            vars.insert(key.clone(), JsonValue::Number(*self.state.variables.get(key).unwrap()));
        }
        map.insert("variables".into(), JsonValue::Obj(vars));

        let mut ns: HashMap<String, JsonValue> = HashMap::new();
        let mut ns_keys: Vec<&String> = self.state.node_state.keys().collect();
        ns_keys.sort();
        for key in ns_keys {
            ns.insert(key.clone(), JsonValue::Str(self.state.node_state.get(key).unwrap().clone()));
        }
        map.insert("node_state".into(), JsonValue::Obj(ns));

        let mut cc: HashMap<String, JsonValue> = HashMap::new();
        let mut cc_keys: Vec<&String> = self.state.x_cluster_cache.keys().collect();
        cc_keys.sort();
        for key in cc_keys {
            let arr: Vec<JsonValue> = self.state.x_cluster_cache.get(key).unwrap()
                .iter().map(|&v| JsonValue::Number(v)).collect();
            cc.insert(key.clone(), JsonValue::Array(arr));
        }
        map.insert("x_cluster_cache".into(), JsonValue::Obj(cc));

        format!("{}", JsonValue::Obj(map))
    }

    /// 生成 clusters.json 内容
    pub fn to_clusters_json(&self) -> String {
        let mut map: HashMap<String, JsonValue> = HashMap::new();
        map.insert("topology_version".into(), JsonValue::Number(self.clusters.topology_version as f64));

        let arr: Vec<JsonValue> = self.clusters.clusters.iter().map(|c| {
            let mut cmap: HashMap<String, JsonValue> = HashMap::new();
            cmap.insert("id".into(), JsonValue::Str(c.id.clone()));
            let nodes: Vec<JsonValue> = c.nodes.iter().map(|n| JsonValue::Str(n.clone())).collect();
            cmap.insert("nodes".into(), JsonValue::Array(nodes));
            JsonValue::Obj(cmap)
        }).collect();
        map.insert("clusters".into(), JsonValue::Array(arr));

        format!("{}", JsonValue::Obj(map))
    }

    /// 生成 runtime.meta.json 内容
    pub fn to_meta_json(&self) -> String {
        let mut map: HashMap<String, JsonValue> = HashMap::new();
        map.insert("tick_step".into(), JsonValue::Number(self.meta.tick_step as f64));
        map.insert("solver".into(), JsonValue::Str(self.meta.solver.clone()));
        map.insert("tolerance".into(), JsonValue::Number(self.meta.tolerance));
        map.insert("max_iter".into(), JsonValue::Number(self.meta.max_iter as f64));
        map.insert("rollback_policy".into(), JsonValue::Str(self.meta.rollback_policy.clone()));

        let mut cr: HashMap<String, JsonValue> = HashMap::new();
        let mut cr_keys: Vec<&String> = self.meta.color_rules.keys().collect();
        cr_keys.sort();
        for key in cr_keys {
            cr.insert(key.clone(), JsonValue::Str(self.meta.color_rules.get(key).unwrap().clone()));
        }
        map.insert("color_rules".into(), JsonValue::Obj(cr));

        format!("{}", JsonValue::Obj(map))
    }

    /// 生成 version.txt 内容
    pub fn to_version_txt(&self) -> String {
        format!(
            "brainstorm_version={}\nsymnebula_core={}\ntopology_version={}",
            self.version.brainstorm_version,
            self.version.symnebula_core,
            self.version.topology_version,
        )
    }

    /// 生成完整 .brainstorm 包（所有文件内容）
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
// 反序列化：从 .brainstorm → NebulaGraph
// ============================================================

impl Brainstorm {
    /// 从 graph.xml 重建 NebulaGraph（仅拓扑，不含状态）
    pub fn to_graph(&self) -> Result<NebulaGraph, String> {
        let mut graph = NebulaGraph::new();
        let mut node_id_map: HashMap<String, usize> = HashMap::new();

        for gnode in &self.graph.nodes {
            let id_num: usize = gnode.id.trim_start_matches('n').parse().map_err(|_| format!("invalid node id: {}", gnode.id))?;
            let expr = match gnode.node_type.as_str() {
                "eq" => parse_simple_eq(&gnode.expr).ok_or_else(|| format!("cannot parse eq: {}", gnode.expr))?,
                "constant" => {
                    let val: f64 = gnode.expr.parse().map_err(|_| format!("invalid constant: {}", gnode.expr))?;
                    Expr::Number(val)
                }
                "expr" => parse_expression(&gnode.expr).map_err(|e| format!("parse error: {}", e))?,
                _ => return Err(format!("unknown node type: {}", gnode.node_type)),
            };
            let nid = graph.add_node(expr);
            if let Some(node) = graph.nodes.iter_mut().find(|n| n.id == nid) {
                node.is_dynamic = gnode.is_dynamic;
            }
            node_id_map.insert(gnode.id.clone(), nid);
        }

        for gedge in &self.graph.edges {
            let (from_node_str, from_sym) = gedge.from.split_once(':').ok_or_else(|| format!("invalid edge from: {}", gedge.from))?;
            let (to_node_str, to_sym) = gedge.to.split_once(':').ok_or_else(|| format!("invalid edge to: {}", gedge.to))?;

            let from_id = *node_id_map.get(from_node_str).ok_or_else(|| format!("unknown node: {}", from_node_str))?;
            let to_id = *node_id_map.get(to_node_str).ok_or_else(|| format!("unknown node: {}", to_node_str))?;

            graph.add_edge(from_id, from_sym, to_id, to_sym);
        }

        // 恢复 topology_version
        graph.topology_version = self.version.topology_version;

        Ok(graph)
    }
}

// ============================================================
// Expr Display — 便于序列化输出
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
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parse_simple_eq;

    #[test]
    fn test_expr_display() {
        let expr = parse_simple_eq("x + y = 10").unwrap();
        let s = format!("{}", expr);
        // 格式为 (x) + (y) = (10)，验证包含关键字符
        assert!(s.contains("x"));
        assert!(s.contains("y"));
        assert!(s.contains("10"));
    }

    #[test]
    fn test_graph_xml_roundtrip() {
        let mut g = NebulaGraph::new();
        let n0 = g.add_node(parse_simple_eq("x + y = 10").unwrap());
        let n1 = g.add_node(parse_simple_eq("x * z = 1").unwrap());
        g.add_edge_with_default(n0, "x", n1, "x", 0.0);

        let solver = ClusterSolverV3::new();
        let bsi = Brainstorm::from_solver(&g, &solver, 0);

        let xml = bsi.to_graph_xml();
        assert!(xml.contains("n0"));
        assert!(xml.contains("n1"));
        assert!(xml.contains("x"));
        assert!(xml.contains("y"));
        assert!(xml.contains("10"));
        assert!(xml.contains("<edge"));

        // 反序列化回 graph 并验证结构
        let g2 = bsi.to_graph().unwrap();
        assert_eq!(g2.nodes.len(), 2);
        assert_eq!(g2.edges.len(), 1);
    }

    #[test]
    fn test_state_json_output() {
        let mut g = NebulaGraph::new();
        g.add_node(parse_simple_eq("x + y = 10").unwrap());

        let solver = ClusterSolverV3::new();
        let bsi = Brainstorm::from_solver(&g, &solver, 42);

        let json = bsi.to_state_json();
        assert!(json.contains("\"tick\""));
        assert!(json.contains("42"));
    }

    #[test]
    fn test_clusters_json_output() {
        let mut g = NebulaGraph::new();
        g.add_node(parse_simple_eq("x + y = 10").unwrap());
        g.add_node(parse_simple_eq("x - y = 2").unwrap());
        g.add_edge_with_default(0, "x", 1, "x", 0.0);
        g.add_edge_with_default(0, "y", 1, "y", 0.0);

        let mut solver = ClusterSolverV3::new();
        let mut cache = ClusterCache::new();
        solver.compile(&g, &mut cache);

        let bsi = Brainstorm::from_solver(&g, &solver, 0);
        let json = bsi.to_clusters_json();
        assert!(json.contains("\"clusters\""));
        assert!(json.contains("\"c0\""));
    }

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

        let bsi = Brainstorm::from_solver(&g, &solver, 0);
        let pkg = bsi.to_package();

        assert!(pkg.contains_key("graph.xml"));
        assert!(pkg.contains_key("state.json"));
        assert!(pkg.contains_key("clusters.json"));
        assert!(pkg.contains_key("runtime.meta.json"));
        assert!(pkg.contains_key("version.txt"));

        // 验证 version.txt 内容
        let ver = &pkg["version.txt"];
        assert!(ver.contains("brainstorm_version=1"));

        // 反序列化并验证结构
        let g2 = bsi.to_graph().unwrap();
        assert_eq!(g2.nodes.len(), 3);
        assert_eq!(g2.edges.len(), 3);
    }
}
