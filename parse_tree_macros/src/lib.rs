#![allow(unused_imports)]
use ::proc_macro::TokenStream;
use ::proc_macro2::{Span, TokenStream as TokenStream2};
use ::quote::{quote, quote_spanned, ToTokens};
use ::syn::{
    parse::{self, Parse, ParseStream, Parser},
    punctuated::Punctuated,
    Result, *,
};
use syn::token::Impl;

// Thanks to discord user Yandros(MemeOverloard) for doing the bulk of the work with this
// macro
#[proc_macro_attribute]
pub fn trace_parser(attrs: TokenStream, input: TokenStream) -> TokenStream {
    parse_macro_input!(attrs as parse::Nothing);
    let mut input = parse_macro_input!(input as ImplItemMethod);
    let block = &mut input.block;

    let function_name = format!("{}", input.sig.ident);

    *block = parse_quote!({
        self.parse_stack.push(ParseStackEntry::Enter(#function_name.to_string()));
        let ret: Result<_> = (|| #block)();
        if let Err(e) = &ret {
            self.parse_stack.push(ParseStackEntry::ExitWithError(e.clone()));
        }
        else {
            self.parse_stack.push(ParseStackEntry::Exit);
        }
        ret
    });
    input.into_token_stream().into()
}

#[proc_macro_attribute]
pub fn trace_typechecker(attrs: TokenStream, input: TokenStream) -> TokenStream {
    parse_macro_input!(attrs as parse::Nothing);
    let mut input = parse_macro_input!(input as ImplItemMethod);
    let block = &mut input.block;

    let function_name = format!("{}", input.sig.ident);

    *block = parse_quote!({
        self.trace_stack.push(TraceStackEntry::Enter(#function_name.to_string()));
        let ret: Result<_> = (|| #block)();
        self.trace_stack.push(TraceStackEntry::Exit);
        ret
    });
    input.into_token_stream().into()
}

#[proc_macro_attribute]
pub fn local_impl(attrs: TokenStream, input: TokenStream) -> TokenStream {
    let mut orig_input = input.clone();

    local_impl_impl(attrs.into(), input.into()).map_or_else(
        |err| {
            orig_input.extend::<proc_macro::TokenStream>(err.to_compile_error().into());
            orig_input
        },
        Into::into,
    )
}

fn local_impl_impl(attrs: TokenStream2, input: TokenStream2) -> Result<TokenStream2> {
    let _ = parse2::<parse::Nothing>(attrs)?;
    let input = parse2::<ItemImpl>(input)?;

    let (impl_generics, generics, where_clause) = input.generics.split_for_impl();
    let trait_name = input.trait_.clone().expect("Expected a trait name").1;

    let method_heads = input
        .items
        .iter()
        .map(|item| match item {
            ImplItem::Method(ImplItemMethod { attrs, sig, .. }) => {
                quote! {
                    #(#attrs)*
                    #sig;
                }
            }
            _ => panic!("Only methods are allowed"),
        })
        .collect::<Vec<_>>();

    let trait_def = quote!(
        pub(crate) trait #trait_name #generics #where_clause {
            #(#method_heads)*
        }
    );

    let items = input.items;
    let self_ty = input.self_ty;

    let impl_block = quote!(
        impl #impl_generics #trait_name #generics for #self_ty {
            #(#items)*
        }
    );

    Ok(quote!(
        #trait_def
        #impl_block
    )
    .into_token_stream()
    .into())
}
