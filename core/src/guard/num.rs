/// 数值合法性守卫 — 所有求解器输出必须过这里
///
/// 确保：
/// - 没有 NaN 传播
/// - 没有 Inf 传播
/// - 没有除零后遗
///
/// 这是 SymNebula "永不崩溃" 防线的最后一道数值检查。

/// 检查 f64 是否有限数（非 NaN、非 Inf）
#[inline]
pub fn ensure_finite(v: f64) -> Result<f64, &'static str> {
    if v.is_finite() {
        Ok(v)
    } else if v.is_nan() {
        Err("NaN detected")
    } else {
        Err("Inf detected")
    }
}

/// 检查除数不为零
#[inline]
pub fn ensure_nonzero(v: f64) -> Result<f64, &'static str> {
    if v.abs() < 1e-12 {
        Err("division by zero")
    } else {
        Ok(v)
    }
}

/// 安全除法
#[inline]
pub fn safe_div(a: f64, b: f64) -> Result<f64, &'static str> {
    let b = ensure_nonzero(b)?;
    ensure_finite(a / b)
}

/// 安全开方
#[inline]
pub fn safe_sqrt(x: f64) -> Result<f64, &'static str> {
    if x < 0.0 {
        Err("sqrt of negative")
    } else {
        ensure_finite(x.sqrt())
    }
}

/// 验证 HashMap 中的所有值都是有限数
#[inline]
pub fn validate_outputs(
    map: &mut std::collections::HashMap<String, f64>,
) -> Result<(), &'static str> {
    for v in map.values_mut() {
        *v = ensure_finite(*v)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ensure_finite_normal() {
        assert_eq!(ensure_finite(42.0), Ok(42.0));
        assert_eq!(ensure_finite(-3.14), Ok(-3.14));
        assert_eq!(ensure_finite(0.0), Ok(0.0));
    }

    #[test]
    fn test_ensure_finite_nan() {
        assert!(ensure_finite(f64::NAN).is_err());
    }

    #[test]
    fn test_ensure_finite_inf() {
        assert!(ensure_finite(f64::INFINITY).is_err());
        assert!(ensure_finite(f64::NEG_INFINITY).is_err());
    }

    #[test]
    fn test_safe_div_normal() {
        assert_eq!(safe_div(10.0, 2.0).unwrap(), 5.0);
    }

    #[test]
    fn test_safe_div_by_zero() {
        assert!(safe_div(1.0, 0.0).is_err());
    }

    #[test]
    fn test_validate_outputs() {
        let mut map = std::collections::HashMap::new();
        map.insert("x".to_string(), 3.0);
        map.insert("y".to_string(), -1.5);
        assert!(validate_outputs(&mut map).is_ok());
    }

    #[test]
    fn test_validate_outputs_rejects_nan() {
        let mut map = std::collections::HashMap::new();
        map.insert("x".to_string(), f64::NAN);
        assert!(validate_outputs(&mut map).is_err());
    }
}
