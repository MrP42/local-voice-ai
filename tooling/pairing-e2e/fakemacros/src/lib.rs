use proc_macro::TokenStream;
#[proc_macro_attribute] pub fn command(_a: TokenStream, i: TokenStream) -> TokenStream { i }
#[proc_macro_attribute] pub fn specta(_a: TokenStream, i: TokenStream) -> TokenStream { i }
#[proc_macro_derive(Type)] pub fn ty(_i: TokenStream) -> TokenStream { TokenStream::new() }
