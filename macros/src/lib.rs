use proc_macro::TokenStream;
use quote::quote;
use syn::{parse::Parse, parse::ParseStream, parse_macro_input, ItemImpl, Lit, Token};

/// 属性参数：priority = u8, name = "str"
struct SolverArgs {
    priority: u8,
    name: String,
}

impl Default for SolverArgs {
    fn default() -> Self {
        SolverArgs {
            priority: 100,
            name: "solver".to_string(),
        }
    }
}

impl Parse for SolverArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = SolverArgs::default();

        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            if ident == "priority" {
                let lit: Lit = input.parse()?;
                if let Lit::Int(v) = lit {
                    args.priority = v.base10_parse::<u8>().unwrap_or(100);
                }
            } else if ident == "name" {
                let lit: Lit = input.parse()?;
                if let Lit::Str(v) = lit {
                    args.name = v.value();
                }
            }

            // 跳过逗号（可选）
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(args)
    }
}

/// 安全求解器属性宏。
///
/// 自动为 Solver 实现注入：
/// - catch_unwind 防止 panic 崩溃
/// - NaN / Inf 数值检查（validate_outputs）
/// - 自动映射 Result -> SolveResult
/// - priority() + name() 自动生成
#[proc_macro_attribute]
pub fn safe_solver(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as SolverArgs);
    let input = parse_macro_input!(item as ItemImpl);

    // ===== 提取 items 并分类 =====
    let mut supports_fn: Option<syn::ImplItemFn> = None;
    let mut solve_fn: Option<syn::ImplItemFn> = None;
    let mut other_items: Vec<syn::ImplItem> = Vec::new();

    for item in input.items.iter() {
        if let syn::ImplItem::Fn(method) = item {
            if method.sig.ident == "supports" {
                supports_fn = Some(method.clone());
            } else if method.sig.ident == "solve" {
                solve_fn = Some(method.clone());
            } else {
                other_items.push(item.clone());
            }
        } else {
            other_items.push(item.clone());
        }
    }

    let supports_body = supports_fn
        .map(|m| {
            let body = &m.block;
            quote! {
                fn supports(&self, node: &crate::graph::Node) -> bool {
                    #body
                }
            }
        })
        .unwrap_or_else(|| {
            quote! {
                fn supports(&self, _node: &crate::graph::Node) -> bool {
                    true
                }
            }
        });

    let solve_body = if let Some(solve) = solve_fn {
        let body = &solve.block;
        quote! { #body }
    } else {
        quote! { { Err("solve not implemented") } }
    };

    let self_ty = &input.self_ty;
    let generics = &input.generics;
    let trait_path = match &input.trait_ {
        Some((_, path, _)) => quote! { #path },
        None => {
            return syn::Error::new_spanned(self_ty, "safe_solver requires a trait impl")
                .to_compile_error()
                .into();
        }
    };

    let priority_val = args.priority;
    let name_lit = args.name;

    // ===== 生成最终代码 =====
    let expanded = quote! {
        impl #trait_path for #self_ty #generics {
            fn name(&self) -> &'static str {
                #name_lit
            }

            fn priority(&self) -> u8 {
                #priority_val
            }

            #supports_body

            fn solve(&self, node: &crate::graph::Node, ctx: &std::collections::HashMap<String, f64>)
                -> crate::solver_trait::SolveResult
            {
                use std::panic::{catch_unwind, AssertUnwindSafe};
                use crate::guard::num::validate_outputs;
                use crate::solver_trait::SolveResult;

                // catch_unwind：防止任何 panic 崩溃
                let result = catch_unwind(AssertUnwindSafe(|| -> Result<
                    std::collections::HashMap<String, f64>,
                    &'static str,
                > {
                    let mut outputs: std::collections::HashMap<String, f64> = {
                        // solve 函数体（返回 Result<HashMap, &str>）
                        #solve_body
                    }?;
                    // 数值合法性检查：NaN / Inf 拒绝
                    validate_outputs(&mut outputs)?;
                    Ok(outputs)
                }));

                match result {
                    Ok(Ok(map)) => SolveResult::Converged(map),
                    Ok(Err(e)) => SolveResult::Failed(e.into()),
                    Err(_) => SolveResult::Failed("solver panic".into()),
                }
            }

            #(#other_items)*
        }
    };

    TokenStream::from(expanded)
}

/// 集成器参数：name = "str"
struct IntegratorArgs {
    name: String,
}

impl Default for IntegratorArgs {
    fn default() -> Self {
        IntegratorArgs {
            name: "integrator".to_string(),
        }
    }
}

impl Parse for IntegratorArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = IntegratorArgs::default();

        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            if ident == "name" {
                let lit: Lit = input.parse()?;
                if let Lit::Str(v) = lit {
                    args.name = v.value();
                }
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(args)
    }
}

/// 安全积分器属性宏。
///
/// 自动为 Integrator 实现注入：
/// - catch_unwind 防止 panic 崩溃
/// - NaN / Inf 数值检查（validate_outputs）
/// - 自动映射 Result -> SolveResult
#[proc_macro_attribute]
pub fn safe_integrator(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as IntegratorArgs);
    let input = parse_macro_input!(item as ItemImpl);

    // ===== 提取 step 方法 =====
    let mut step_fn: Option<syn::ImplItemFn> = None;
    let mut other_items: Vec<syn::ImplItem> = Vec::new();

    for item in input.items.iter() {
        if let syn::ImplItem::Fn(method) = item {
            if method.sig.ident == "step" {
                step_fn = Some(method.clone());
            } else {
                other_items.push(item.clone());
            }
        } else {
            other_items.push(item.clone());
        }
    }

    let step_body = if let Some(step) = step_fn {
        let body = &step.block;
        quote! { #body }
    } else {
        quote! { { HashMap::new() } }
    };

    let self_ty = &input.self_ty;
    let generics = &input.generics;
    let trait_path = match &input.trait_ {
        Some((_, path, _)) => quote! { #path },
        None => {
            return syn::Error::new_spanned(self_ty, "safe_integrator requires a trait impl")
                .to_compile_error()
                .into();
        }
    };

    let name_lit = args.name;

    let expanded = quote! {
        impl #trait_path for #self_ty #generics {
            fn step(&self, node: &crate::graph::Node, ctx: &std::collections::HashMap<String, f64>, dt: f64)
                -> crate::solver_trait::SolveResult
            {
                use std::panic::{catch_unwind, AssertUnwindSafe};
                use crate::guard::num::validate_outputs;
                use crate::solver_trait::SolveResult;

                let result = catch_unwind(AssertUnwindSafe(|| -> Result<
                    std::collections::HashMap<String, f64>,
                    &'static str,
                > {
                    let mut outputs: std::collections::HashMap<String, f64> = {
                        #step_body
                    }?;
                    validate_outputs(&mut outputs)?;
                    Ok(outputs)
                }));

                match result {
                    Ok(Ok(map)) => SolveResult::Converged(map),
                    Ok(Err(e)) => SolveResult::Failed(e.into()),
                    Err(_) => SolveResult::Failed("integrator panic".into()),
                }
            }

            #(#other_items)*
        }

        impl #self_ty #generics {
            fn __integrator_name() -> &'static str {
                #name_lit
            }
        }
    };

    TokenStream::from(expanded)
}
