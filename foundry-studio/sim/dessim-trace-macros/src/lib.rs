//! Function instrumentation macro for the dessim trace substrate.
//!
//! `#[trace]` wraps a function so that, when the downstream crate's `trace`
//! feature is enabled, its entry args and exit return value emit `tracing`
//! records targeted at `dessim::trace`.
//!
//! Conventions:
//! - span name = Rust function name;
//! - non-underscore named arguments are recorded with `Display` (`%arg`);
//! - `self` is skipped until the simulator grows explicit trace identities;
//! - the return value is recorded on function exit, formatted by convention:
//!   `Result<Option<T>, E>` → `Some(T)` / `None` / `Err(E)`,
//!   `Result<T, E>` → `T` / `Err(E)`,
//!   `Option<T>` → `Some(T)` / `None`,
//!   anything else → `Display`.
//!   Functions returning `()` emit no return record.
//! - return capture wraps the body in a `move ||` closure so early `return`
//!   and `?` exits are captured. Without the `trace` feature this still
//!   compiles to a one-shot inlined closure with no logging.
//!
//! Directives (use sparingly):
//! - `#[trace(skip(name, ...))]` — exclude args whose Display would dump
//!   non-trace-safe state (`TaskSet`, pathfinder handles, locks, etc.).
//! - `#[trace(fields(name = expr, ...))]` — add extra Display-formatted
//!   fields to the span. Useful for `self`-derived identity such as
//!   `fields(station = self.station_id())` until `TraceIdentity` exists.

use std::collections::HashSet;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, quote_spanned};
use syn::parse::Parser;
use syn::spanned::Spanned;
use syn::{
    Block, Expr, FnArg, GenericArgument, Ident, ImplItemFn, ItemFn, Pat, PathArguments, ReturnType, Signature, Type,
    parse_macro_input, parse_quote,
};

#[proc_macro_attribute]
pub fn trace(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attrs = match parse_attrs(attr.into()) {
        Ok(parsed) => parsed,
        Err(err) => return err.to_compile_error().into(),
    };
    let item_clone = item.clone();
    if let Ok(function) = syn::parse::<ImplItemFn>(item_clone) {
        return expand_impl_fn(function, &attrs).into();
    }
    let function = parse_macro_input!(item as ItemFn);
    expand_item_fn(function, &attrs).into()
}

#[derive(Default)]
struct TraceAttrs {
    skip: HashSet<String>,
    fields: Vec<(Ident, Expr)>,
}

fn parse_attrs(attr: TokenStream2) -> syn::Result<TraceAttrs> {
    let mut result = TraceAttrs::default();
    if attr.is_empty() {
        return Ok(result);
    }
    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("skip") {
            meta.parse_nested_meta(|nested| {
                let ident = nested.path.get_ident().ok_or_else(|| nested.error("expected identifier in skip(...)"))?;
                result.skip.insert(ident.to_string());
                Ok(())
            })
        } else if meta.path.is_ident("fields") {
            meta.parse_nested_meta(|nested| {
                let ident =
                    nested.path.get_ident().ok_or_else(|| nested.error("expected identifier in fields(...)"))?.clone();
                let value = nested.value()?;
                let expr: Expr = value.parse()?;
                result.fields.push((ident, expr));
                Ok(())
            })
        } else {
            Err(meta.error("expected `skip(...)` or `fields(...)`"))
        }
    });
    parser.parse2(attr)?;
    Ok(result)
}

fn expand_impl_fn(mut function: ImplItemFn, attrs: &TraceAttrs) -> TokenStream2 {
    let span_name = function.sig.ident.to_string();
    let body = build_traced_body(&span_name, &function.sig, &function.block, attrs);
    function.block = parse_quote!({ #body });
    quote!(#function)
}

fn expand_item_fn(mut function: ItemFn, attrs: &TraceAttrs) -> TokenStream2 {
    let span_name = function.sig.ident.to_string();
    let body = build_traced_body(&span_name, &function.sig, &function.block, attrs);
    function.block = Box::new(parse_quote!({ #body }));
    quote!(#function)
}

fn build_traced_body(span_name: &str, sig: &Signature, original_block: &Block, attrs: &TraceAttrs) -> TokenStream2 {
    let arg_idents = collect_arg_idents(sig, attrs);
    let extra_idents: Vec<&Ident> = attrs.fields.iter().map(|(ident, _)| ident).collect();
    let extra_exprs: Vec<&Expr> = attrs.fields.iter().map(|(_, expr)| expr).collect();

    let return_record = if is_unit_return(&sig.output) {
        quote!()
    } else {
        let format_expr = build_return_format_expr(&sig.output);
        quote! {
            #[cfg(feature = "trace")]
            {
                let __trace_ret_display: ::std::string::String = #format_expr;
                ::tracing::info!(
                    target: "dessim::trace",
                    trace_kind = "return",
                    trace_name = #span_name,
                    ret = %__trace_ret_display,
                );
            }
        }
    };

    let block_span = original_block.span();
    quote_spanned! { block_span =>
        #[cfg(feature = "trace")]
        let __trace_span = ::tracing::info_span!(
            #span_name
            #(, #arg_idents = %#arg_idents)*
            #(, #extra_idents = %#extra_exprs)*
        );
        #[cfg(feature = "trace")]
        let __trace_guard = __trace_span.enter();
        let __trace_ret = (move || #original_block)();
        #return_record
        __trace_ret
    }
}

fn collect_arg_idents(sig: &Signature, attrs: &TraceAttrs) -> Vec<Ident> {
    sig.inputs
        .iter()
        .filter_map(|arg| match arg {
            FnArg::Receiver(_) => None,
            FnArg::Typed(pat_type) => match pat_type.pat.as_ref() {
                Pat::Ident(pat_ident) => {
                    let name = pat_ident.ident.to_string();
                    if name.starts_with('_') || attrs.skip.contains(&name) {
                        None
                    } else {
                        Some(pat_ident.ident.clone())
                    }
                }
                _ => None,
            },
        })
        .collect()
}

fn is_unit_return(ret: &ReturnType) -> bool {
    match ret {
        ReturnType::Default => true,
        ReturnType::Type(_, ty) => matches!(&**ty, Type::Tuple(t) if t.elems.is_empty()),
    }
}

enum ReturnShape {
    Result,
    Option,
    ResultOption,
    Other,
}

fn classify_return(ret: &ReturnType) -> ReturnShape {
    let ty = match ret {
        ReturnType::Default => return ReturnShape::Other,
        ReturnType::Type(_, ty) => ty,
    };
    let Type::Path(path) = &**ty else {
        return ReturnShape::Other;
    };
    let Some(last) = path.path.segments.last() else {
        return ReturnShape::Other;
    };
    match last.ident.to_string().as_str() {
        "Result" => {
            if first_generic_is_option(&last.arguments) {
                ReturnShape::ResultOption
            } else {
                ReturnShape::Result
            }
        }
        "Option" => ReturnShape::Option,
        _ => ReturnShape::Other,
    }
}

fn first_generic_is_option(args: &PathArguments) -> bool {
    let PathArguments::AngleBracketed(args) = args else {
        return false;
    };
    let Some(GenericArgument::Type(Type::Path(path))) = args.args.first() else {
        return false;
    };
    path.path.segments.last().map(|seg| seg.ident == "Option").unwrap_or(false)
}

fn build_return_format_expr(ret: &ReturnType) -> TokenStream2 {
    match classify_return(ret) {
        ReturnShape::ResultOption => quote! {
            match &__trace_ret {
                ::std::result::Result::Ok(::std::option::Option::Some(__v)) => format!("Some({})", __v),
                ::std::result::Result::Ok(::std::option::Option::None) => "None".to_string(),
                ::std::result::Result::Err(__e) => format!("Err({})", __e),
            }
        },
        ReturnShape::Result => quote! {
            match &__trace_ret {
                ::std::result::Result::Ok(__v) => format!("{}", __v),
                ::std::result::Result::Err(__e) => format!("Err({})", __e),
            }
        },
        ReturnShape::Option => quote! {
            match &__trace_ret {
                ::std::option::Option::Some(__v) => format!("Some({})", __v),
                ::std::option::Option::None => "None".to_string(),
            }
        },
        ReturnShape::Other => quote! {
            format!("{}", &__trace_ret)
        },
    }
}
