# .brainstorm v1 Spec — SymNebula 可重放快照容器

> 它不是文件格式，而是"确定性计算的可序列化宇宙状态"。

---

## 1. 文件本质

`.brainstorm` 是一个**确定性图计算系统的可重放快照容器**。

结构：

```
.brainstorm (zip)
 ├── graph.xml
 ├── state.json
 ├── clusters.json
 ├── runtime.meta.json
 ├── snapshots/
 │     ├── tick_0001.json
 │     ├── tick_0002.json
 │     └── ...
 └── version.txt
```

---

## 2. graph.xml（结构层 / 静态拓扑）

核心原则：graph = 拓扑 + 约束表达，不包含运行结果。

```xml
<graph version="1">
    <nodes>
        <node id="n1" type="eq">
            <expr>x + y = 10</expr>
        </node>
        <node id="n2" type="eq">
            <expr>x = z * z</expr>
        </node>
        <node id="n3" type="eq">
            <expr>z + y = 1</expr>
        </node>
    </nodes>

    <edges>
        <edge from="n1:x" to="n2:x"/>
        <edge from="n1:y" to="n3:y"/>
        <edge from="n2:z" to="n3:z"/>
    </edges>
</graph>
```

### 关键设计

- node = constraint container
- edge = symbolic alias / synapse（`from="n1:x"` = 节点 n1 的 x 端口）
- expr = AST string（Rust `parse_simple_eq` 可以解析）
- node type 可以是 `eq` / `expr` / `constant`
- node 可附加 `dynamic="true"` 属性

---

## 3. state.json（运行状态层）

核心原则：state = "当前解 + 状态机"。

```json
{
  "tick": 42,
  "variables": {
    "x": 1.23,
    "y": 4.56,
    "z": 0.78
  },
  "node_state": {
    "n1": "Yellow",
    "n2": "Green",
    "n3": "Purple"
  },
  "x_cluster_cache": {
    "cluster_0": [1.1, 2.2, 3.3]
  }
}
```

包含：X values / G/Y/P 状态 / cluster runtime buffer。

---

## 4. clusters.json（编译产物缓存）

这是**优化层**（可重建，但可缓存）。

```json
{
  "topology_version": 12,
  "clusters": [
    { "id": "c0", "nodes": ["n1", "n2", "n3"] },
    { "id": "c1", "nodes": ["n4"] }
  ]
}
```

关键设计：
- 由 Union-Find + graph closure 生成
- 可缓存（依赖 topology_version）
- 重建成本低，但加载时优先使用缓存

---

## 5. runtime.meta.json（执行配置）

```json
{
  "tick_step": 1,
  "solver": "newton_block",
  "tolerance": 1e-6,
  "max_iter": 20,
  "rollback_policy": "cluster_atomic",
  "color_rules": {
    "green": "residual < 1e-6",
    "yellow": "converging",
    "purple": "diverged or singular"
  }
}
```

---

## 6. snapshots/tick_xxxx.json（可选但关键）

用于 replay / debug / rollback visualization。

```json
{
  "tick": 42,
  "cluster_states": {
    "c0": {
      "x_cluster": [1.0, 2.0, 3.0],
      "status": "Green"
    }
  }
}
```

---

## 7. version.txt（强一致性关键）

```
brainstorm_version=1
symnebula_core=0.1.0
topology_version=12
```

---

## 8. 关键设计原则

### 8.1 graph / state 分离

| 层 | 内容 |
|----|------|
| graph.xml | 静态拓扑 + 约束表达 |
| state.json | 动态运行状态（数值 + 颜色） |

### 8.2 tick 可重放

只要 graph + initial state，就可以完全 deterministic replay。

### 8.3 cluster 是编译产物

- 不手写
- 可缓存
- 可重建（拓扑版本变化时）

### 8.4 commit 不进入 graph

- commit 只更新 state
- graph 永远纯净

### 8.5 rollback 依赖 state snapshot

不是 undo stack，而是 tick-level snapshot restore。

---

## 9. 设计归属

五哥 (Mutou322) — SymNebula 符号星云，GPL v3
