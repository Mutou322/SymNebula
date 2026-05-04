/// 约束求解器与双向坍缩
///
/// 从等式 Expr::Eq(l, r) 中，根据已知变量值，推导出未知变量的值。
/// 支持简单加减法的代数重排，多解时返回 Yellow，无解/奇异时返回 Purple。

use std::collections::HashMap;

use crate::ast::Expr;
use crate::state::NodeState;

/// 求解结果
#[derive(Debug)]
pub struct SolveResult {
    /// 推导出的符号名（空字符串表示无需推导）
    pub symbol: String,
    /// 推导出的值
    pub value: f64,
    /// 节点新状态
    pub state: NodeState,
}

/// 尝试从等式中根据已知变量推导未知变量
pub fn solve_eq(eq: &Expr, known: &HashMap<String, f64>) -> Result<SolveResult, String> {
    let (left, right) = match eq {
        Expr::Eq(l, r) => (l, r),
        _ => return Err("不是等式".into()),
    };

    let all_syms = eq.symbols();
    let unknown: Vec<String> = all_syms
        .into_iter()
        .filter(|s| !known.contains_key(s))
        .collect();

    // 无未知变量：验证等式是否成立
    if unknown.is_empty() {
        match (left.eval(known), right.eval(known)) {
            (Ok(lv), Ok(rv)) if (lv - rv).abs() < 1e-9 => {
                return Ok(SolveResult {
                    symbol: String::new(),
                    value: lv,
                    state: NodeState::Green,
                });
            }
            (Ok(lv), Ok(rv)) => {
                return Err(format!("约束不满足: {} != {}", lv, rv));
            }
            _ => return Err("等式求值失败".into()),
        }
    }

    // 多个未知变量：无法唯一求解
    if unknown.len() > 1 {
        return Ok(SolveResult {
            symbol: unknown[0].clone(),
            value: 0.0,
            state: NodeState::Yellow,
        });
    }

    let u = &unknown[0];

    // 尝试左边能求值，u 在右边
    if let Ok(left_val) = left.eval(known) {
        if let Ok(right_val) = solve_for_symbol(right, u, left_val, known) {
            return Ok(SolveResult {
                symbol: u.clone(),
                value: right_val,
                state: NodeState::Green,
            });
        }
    }

    // 尝试右边能求值，u 在左边
    if let Ok(right_val) = right.eval(known) {
        if let Ok(u_val) = solve_for_symbol(left, u, right_val, known) {
            return Ok(SolveResult {
                symbol: u.clone(),
                value: u_val,
                state: NodeState::Green,
            });
        }
    }

    Err("无法自动求解此等式形式".into())
}

/// 在表达式中解出目标符号：已知 expr = target_value，求 symbol 的值
///
/// 支持的形式：symbol + rest, rest + symbol, symbol - rest, rest - symbol
fn solve_for_symbol(
    expr: &Expr,
    symbol: &str,
    target_value: f64,
    known: &HashMap<String, f64>,
) -> Result<f64, String> {
    match expr {
        Expr::Symbol(s) if s == symbol => Ok(target_value),
        Expr::Add(a, b) => {
            // a + b = target
            if contains_symbol(a, symbol) {
                // 求 a：a = target - b
                let bv = b.eval(known)?;
                solve_for_symbol(a, symbol, target_value - bv, known)
            } else if contains_symbol(b, symbol) {
                let av = a.eval(known)?;
                solve_for_symbol(b, symbol, target_value - av, known)
            } else {
                Err("加法中未找到目标符号".into())
            }
        }
        Expr::Sub(a, b) => {
            if contains_symbol(a, symbol) {
                // a - b = target → a = target + b
                let bv = b.eval(known)?;
                solve_for_symbol(a, symbol, target_value + bv, known)
            } else if contains_symbol(b, symbol) {
                // a - b = target → b = a - target
                let av = a.eval(known)?;
                solve_for_symbol(b, symbol, av - target_value, known)
            } else {
                Err("减法中未找到目标符号".into())
            }
        }
        Expr::Mul(a, b) => {
            // a * b = target
            if contains_symbol(a, symbol) {
                let bv = b.eval(known)?;
                if bv == 0.0 {
                    return Err("乘零错误: 右侧表达式含除零".into());
                }
                solve_for_symbol(a, symbol, target_value / bv, known)
            } else if contains_symbol(b, symbol) {
                let av = a.eval(known)?;
                if av == 0.0 {
                    return Err("乘零错误: 左侧表达式含除零".into());
                }
                solve_for_symbol(b, symbol, target_value / av, known)
            } else {
                Err("乘法中未找到目标符号".into())
            }
        }
        // a/b 已被解析为 a * b^(-1)，所以不会走到 Div
        // 但保留以防未来 AST 变化
        Expr::Div(a, b) => {
            if contains_symbol(a, symbol) {
                let bv = b.eval(known)?;
                solve_for_symbol(a, symbol, target_value * bv, known)
            } else if contains_symbol(b, symbol) {
                let av = a.eval(known)?;
                if target_value == 0.0 {
                    Err("除零错误".into())
                } else {
                    solve_for_symbol(b, symbol, av / target_value, known)
                }
            } else {
                Err("除法中未找到目标符号".into())
            }
        }
        Expr::Pow(a, b) => {
            if contains_symbol(a, symbol) {
                // a^b = target → a = target^(1/b)
                let bv = b.eval(known)?;
                if target_value < 0.0 && bv.fract() != 0.0 {
                    Err("负数开非整数次方".into())
                } else {
                    solve_for_symbol(a, symbol, target_value.powf(1.0 / bv), known)
                }
            } else if contains_symbol(b, symbol) {
                // a^b = target → b = log(target) / log(a)
                let av = a.eval(known)?;
                if av <= 0.0 || target_value <= 0.0 {
                    Err("对数参数无效".into())
                } else {
                    solve_for_symbol(b, symbol, target_value.log(av), known)
                }
            } else {
                Err("幂运算中未找到目标符号".into())
            }
        }
        _ => Err("不支持的表达式形式".into()),
    }
}

/// 检查表达式中是否包含指定符号
fn contains_symbol(expr: &Expr, symbol: &str) -> bool {
    match expr {
        Expr::Symbol(s) => s == symbol,
        Expr::Number(_) => false,
        Expr::Add(a, b)
        | Expr::Sub(a, b)
        | Expr::Mul(a, b)
        | Expr::Div(a, b)
        | Expr::Pow(a, b)
        | Expr::Eq(a, b) => contains_symbol(a, symbol) || contains_symbol(b, symbol),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_solve_addition() {
        // a + 3 = 10, a 未知
        let expr = crate::ast::parse_simple_eq("a + 3 = 10").unwrap();
        let known = HashMap::new();
        let result = solve_eq(&expr, &known).unwrap();
        assert_eq!(result.state, NodeState::Green);
        assert!((result.value - 7.0).abs() < 1e-9);
    }

    #[test]
    fn test_solve_bidirectional() {
        let expr = crate::ast::parse_simple_eq("a + b = 10").unwrap();
        let mut known = HashMap::new();
        known.insert("a".to_string(), 3.0);
        let result = solve_eq(&expr, &known).unwrap();
        assert_eq!(result.state, NodeState::Green);
        assert!((result.value - 7.0).abs() < 1e-9);
    }

    #[test]
    fn test_multiple_unknowns() {
        let expr = crate::ast::parse_simple_eq("a + b = 10").unwrap();
        let known = HashMap::new();
        let result = solve_eq(&expr, &known).unwrap();
        assert_eq!(result.state, NodeState::Yellow);
    }

    #[test]
    fn test_multiplication() {
        let expr = crate::ast::parse_simple_eq("x * 5 = 20").unwrap();
        let known = HashMap::new();
        let result = solve_eq(&expr, &known).unwrap();
        assert_eq!(result.state, NodeState::Green);
        assert!((result.value - 4.0).abs() < 1e-9);
    }

    #[test]
    fn test_gravity_already_known() {
        // F = G * m1 * m2 / r^2 所有变量已知，验证等式成立
        let expr = crate::ast::parse_simple_eq("F = G * m1 * m2 / r^2").unwrap();
        let mut known = HashMap::new();
        known.insert("F".to_string(), 1.98e20);
        known.insert("G".to_string(), 6.674e-11);
        known.insert("m1".to_string(), 5.97e24);
        known.insert("m2".to_string(), 7.35e22);
        known.insert("r".to_string(), 3.84e8);
        let result = solve_eq(&expr, &known);
        assert!(result.is_ok());
    }
}
