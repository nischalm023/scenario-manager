//! `#[derive(StreamRecord)]` — the record struct is the single source of
//! truth for its stream's column schema.
//!
//! The derive generates `StreamRecord::entry_columns()` from the struct's
//! fields: column names from field names, value types inferred from Rust
//! types (or declared explicitly), descriptions required on every field so
//! streams are self-describing by construction.
//!
//! ```ignore
//! #[derive(StreamRecord, Serialize)]
//! struct VtmOperationRecord {
//!     #[stream(desc = "Simulation time of the operation.")]
//!     time: SimTime,
//!     #[stream(desc = "Acting DA50.")]
//!     robot_id: u32,
//!     #[stream(text, desc = "Tote payload state at the boundary.")]
//!     payload: TotePayloadState,
//! }
//! ```
//!
//! Inference covers `SimTime`, `SimDuration`, unsigned/signed integers,
//! floats, `bool`, and `String`. Times and durations are NEVER bare
//! integers: use the domain newtypes and the derive infers the column type
//! — there is no marker for them. Everything else (enums, nested types)
//! needs an explicit kind marker — `uint`, `int`, `float`, `bool`, `text` —
//! matching how the value serializes positionally (unit enum variants
//! serialize as their name strings, hence `text`).

use proc_macro::TokenStream;
use proc_macro2::Ident;
use quote::quote;
use syn::{Data, DeriveInput, Error, Field, Fields, LitStr, Path, Type, parse_macro_input, spanned::Spanned};

#[proc_macro_derive(StreamRecord, attributes(stream))]
pub fn derive_stream_record(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> { // keep-qualified: proc_macro::TokenStream holds the bare name
    let Data::Struct(data) = &input.data else {
        return Err(Error::new(input.ident.span(), "StreamRecord can only be derived for structs"));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(Error::new(input.ident.span(), "StreamRecord requires named fields"));
    };

    let mut columns = Vec::new();
    for field in &fields.named {
        let name = field.ident.as_ref().unwrap().to_string();
        let (kind, description) = field_schema(field)?;
        columns.push(quote! {
            ::dessim::metrics::streams::EntryColumn::new(
                #name,
                #description,
                ::dessim::metrics::streams::EntryValueType::#kind,
            )
        });
    }

    let ident = &input.ident;
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::dessim::metrics::streams::StreamRecord
            for #ident #type_generics #where_clause
        {
            fn entry_columns() -> ::std::vec::Vec<::dessim::metrics::streams::EntryColumn> {
                ::std::vec![#(#columns),*]
            }
        }
    })
}

/// Extract (value-type variant, description) for one field from its
/// `#[stream(...)]` attribute and type.
fn field_schema(field: &Field) -> syn::Result<(Ident, String)> {
    let mut explicit_kind: Option<Ident> = None;
    let mut description: Option<String> = None;

    for attr in &field.attrs {
        if !attr.path().is_ident("stream") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("desc") {
                let value: LitStr = meta.value()?.parse()?;
                description = Some(value.value());
                return Ok(());
            }
            let kind = kind_variant(&meta.path)?;
            if explicit_kind.replace(kind).is_some() {
                return Err(meta.error("duplicate value-type marker"));
            }
            Ok(())
        })?;
    }

    let description = description.ok_or_else(|| {
        Error::new(
            field.span(),
            "every stream field needs #[stream(desc = \"...\")] — \
             streams are self-describing by construction",
        )
    })?;
    let kind = match explicit_kind {
        Some(kind) => kind,
        None => infer_kind(field)?,
    };
    Ok((kind, description))
}

fn kind_variant(path: &Path) -> syn::Result<Ident> {
    let span = path.span();
    let variant = if path.is_ident("uint") {
        "UInt"
    } else if path.is_ident("int") {
        "Int"
    } else if path.is_ident("float") {
        "Float"
    } else if path.is_ident("bool") {
        "Bool"
    } else if path.is_ident("text") {
        "Text"
    } else {
        return Err(Error::new(
            span,
            "unknown stream marker; expected one of uint, int, float, bool, \
             text, desc (times/durations have no marker — use the SimTime / \
             SimDuration field types and the column type is inferred)",
        ));
    };
    Ok(Ident::new(variant, span))
}

/// Infer the entry value type from the Rust field type where unambiguous.
fn infer_kind(field: &Field) -> syn::Result<Ident> {
    let Type::Path(type_path) = &field.ty else {
        return Err(non_inferable(field));
    };
    let Some(segment) = type_path.path.segments.last() else {
        return Err(non_inferable(field));
    };
    let variant = match segment.ident.to_string().as_str() {
        "SimTime" => "SimTime",
        "SimDuration" => "SimDuration",
        "u8" | "u16" | "u32" | "u64" | "usize" => "UInt",
        "i8" | "i16" | "i32" | "i64" | "isize" => "Int",
        "f32" | "f64" => "Float",
        "bool" => "Bool",
        "String" => "Text",
        _ => return Err(non_inferable(field)),
    };
    Ok(Ident::new(variant, field.ty.span()))
}

fn non_inferable(field: &Field) -> Error {
    Error::new(
        field.ty.span(),
        "cannot infer the entry value type for this field type; add an \
         explicit marker, e.g. #[stream(text, desc = \"...\")] for enums \
         (unit variants serialize as their name strings)",
    )
}
