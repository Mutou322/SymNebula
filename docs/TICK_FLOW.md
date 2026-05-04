# Tick 流程详解

## 多节点自环 + Newton 迭代动态演化

显示 3~4 Tick 的多节点自环 + Newton 多变量迭代动态演化，展示反馈回路逐步收敛或变紫的动态效果。

Legend: G=Green (stable), Y=Yellow (pending/multi-solution), 
        P=Purple (singular/error), - = Grey (idle)

```
┌───────┬─────────────┬─────────────┬─────────────┐
│ Tick  │   Node N1   │   Node N2   │   Node N3   │
├───────┼─────────────┼─────────────┼─────────────┤
│ T0    │ - (x1=?)    │ - (x2=?)    │ - (x3=?)    │
│       │ Grey        │ Grey        │ Grey        │
├───────┼─────────────┼─────────────┼─────────────┤
│ T1    │ G (x1=1.2)  │ Y (x2=?)    │ - (x3=?)    │
│       │ Computed    │ Waiting     │ Idle        │
├───────┼─────────────┼─────────────┼─────────────┤
│ T2    │ G (x1=1.2)  │ G (x2=0.9)  │ Y (x3=?)    │
│       │ Stable      │ Stable      │ Pending     │
├───────┼─────────────┼─────────────┼─────────────┤
│ T3    │ G (x1=1.15) │ G (x2=0.92) │ P (x3=NaN)  │
│       │ Slight Adj  │ Slight Adj  │ Singular    │
├───────┼─────────────┼─────────────┼─────────────┤
│ T4    │ G (x1=1.14) │ G (x2=0.91) │ P (x3=NaN)  │
│       │ Converged   │ Converged   │ Singular    │
└───────┴─────────────┴─────────────┴─────────────┘
```

### 值流 + 异常流 + Tick 分阶段

```
Tick 1:
[C] Compute Stage
  N1(x1) G=1.0 ----> N2(x2) Y=? ----> N3(x3) - (idle)
                 ^
                 |
                self-loop (feedback not yet active)

[I] Integrator Stage
  N1(x1) stable, no change
  N2 pending (waiting upstream)
  N3 idle

[M] Commit Stage
  N1 -> next_buffer committed
  N2/Y stays yellow
  N3 still idle
```

```
Tick 2:
[C] Compute Stage
  N1(x1) G=1.05 ----> N2(x2) Y=0.8 ----> N3(x3) Y=?
                 ^
                 |
                feedback from N3 previous tick (still ?) 

[I] Integrator Stage
  N1 slight adjust
  N2 update attempt
  N3 starts evaluating

[M] Commit Stage
  N1 -> committed
  N2 -> committed
  N3 -> next_buffer holds intermediate (still yellow)
```

```
Tick 3:
[C] Compute Stage
  N1(x1) G=1.08 ----> N2(x2) G=0.82 ----> N3(x3) P=NaN !
                 ^
                 |
                feedback from N3 triggers singularity

[I] Integrator Stage
  N1 adjust based on feedback
  N2 converged
  N3 computation detects Jacobian=0, output cut

[M] Commit Stage
  N1 -> committed
  N2 -> committed
  N3 -> committed as purple (outputs cut)
```

### 状态变化总结

```
Tick | Node Values   | Node States
------------------------------------
T1   | x1=1.0       | N1=G, N2=Y, N3=-
T2   | x1=1.05      | N1=G, N2=Y, N3=Y
T3   | x1=1.08      | N1=G, N2=G, N3=P
```

### 滚动动画表格（每 Tick 三阶段完整展示）

```
Ticks:       N1        N2        N3
------------------------------------------
Tick 1 [C] | 1.00 G -->  -        -        
Tick 1 [I] | 1.00 G     -        -        
Tick 1 [M] | 1.00 G     -        -        
                                    
Tick 2 [C] | 1.05 G -->  0.80 Y --> -        
Tick 2 [I] | 1.05 G     0.80 Y    -        
Tick 2 [M] | 1.05 G     0.80 Y    -        
                                    
Tick 3 [C] | 1.08 G -->  0.82 G -->  ? Y ^
Tick 3 [I] | 1.08 G     0.82 G     0.50 Y
Tick 3 [M] | 1.08 G     0.82 G     0.50 Y
                                    
Tick 4 [C] | 1.10 G -->  0.85 G -->  NaN P !
Tick 4 [I] | 1.10 G     0.85 G     NaN P !
Tick 4 [M] | 1.10 G     0.85 G     NaN P !
```

### 要点说明

- **箭头 -->** 表示值传播方向
- **自环反馈** 用 `^` 表示
- **!** 表示输出切断（Purple 状态）
- **Yellow** = 多解待定 / 迭代中 / 等待上游
- **Green** = 计算成功，输出有效
- **Purple** = 奇异 / 数值异常，输出切断
- **Tick 分阶段**（Compute → Integrator → Commit），每阶段输出独立
