mod expand;

use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn atom_record(attr: TokenStream, item: TokenStream) -> TokenStream {
    match expand::expand_atom_record(attr.into(), item.into()) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn atom_export(attr: TokenStream, item: TokenStream) -> TokenStream {
    match expand::expand_atom_export(attr.into(), item.into()) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}
