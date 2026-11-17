//! Register a root task type beside its definition. The linked descriptor calls
//! `Task::create`; children need neither registration nor a prescribed constructor.

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{Error, ItemStruct, parse_macro_input};

#[proc_macro_attribute]
pub fn task(args: TokenStream, item: TokenStream) -> TokenStream {
    if !args.is_empty() {
        return Error::new(Span::call_site(), "#[task] takes no arguments; its name is the struct name")
            .to_compile_error()
            .into();
    }
    let item = parse_macro_input!(item as ItemStruct);
    if !item.generics.params.is_empty() {
        return Error::new_spanned(&item.generics, "register a concrete task type").to_compile_error().into();
    }
    let name = &item.ident;
    quote! {
        #item

        const _: () = {
            #[::dessim::task::registration_slice(::dessim::task::REGISTRATIONS)]
            #[linkme(crate = ::dessim::task::__linkme)]
            static REGISTRATION: ::dessim::task::Registration =
                ::dessim::task::Registration::of::<#name>(stringify!(#name));
        };
    }
    .into()
}
