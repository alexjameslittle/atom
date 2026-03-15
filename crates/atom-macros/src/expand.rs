use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::Nothing;
use syn::{
    Error, FnArg, ForeignItem, ForeignItemFn, Ident, Item, ItemFn, ItemForeignMod, ItemStruct,
    PatType, Result, ReturnType, Signature, Type, TypePath, Visibility, parse2,
};

pub(crate) fn expand_atom_record(attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    parse2::<Nothing>(attr)?;

    match parse2::<Item>(item.clone())? {
        Item::Struct(ItemStruct { .. }) | Item::Enum(_) => Ok(item),
        other => Err(Error::new_spanned(
            other,
            "#[atom_record] may only be applied to structs or enums",
        )),
    }
}

pub(crate) fn expand_atom_export(attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    parse2::<Nothing>(attr)?;

    let function = parse2::<ItemFn>(item)?;
    validate_export_signature(&function.sig)?;

    let function_ident = &function.sig.ident;
    let wrapper_ident = format_ident!("__atom_export_{function_ident}");
    let wrapper_inputs = wrapper_inputs(&function.sig);
    let decode_input = decode_input(&function.sig)?;
    let call_arguments = call_arguments(&function.sig)?;
    let return_handling = return_handling(&function.sig, function_ident, &call_arguments)?;

    Ok(quote! {
        #function

        #[doc(hidden)]
        #[unsafe(export_name = concat!("atom_", env!("CARGO_CRATE_NAME"), "_", stringify!(#function_ident)))]
        pub unsafe extern "C" fn #wrapper_ident(#wrapper_inputs) -> i32 {
            if let Err(error) = ::atom_ffi::require_owned_buffer_slot(
                out_response_flatbuffer,
                "out_response_flatbuffer",
            ) {
                unsafe {
                    ::atom_ffi::write_error_buffer(out_error_flatbuffer, &error);
                }
                return error.exit_code();
            }

            if let Err(error) = ::atom_runtime::ensure_running() {
                unsafe {
                    ::atom_ffi::clear_buffer(out_response_flatbuffer);
                    ::atom_ffi::write_error_buffer(out_error_flatbuffer, &error);
                }
                return error.exit_code();
            }

            #decode_input
            #return_handling
        }
    })
}

pub(crate) fn expand_atom_import(attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    parse2::<Nothing>(attr)?;

    let import_block = parse2::<ItemForeignMod>(item)?;
    validate_import_block(&import_block)?;

    let functions = import_block
        .items
        .iter()
        .map(|item| match item {
            ForeignItem::Fn(function) => build_import_function(function),
            other => Err(Error::new_spanned(
                other,
                "#[atom_import] may only contain foreign functions",
            )),
        })
        .collect::<Result<Vec<_>>>()?;

    let slot_definitions = functions.iter().map(|function| {
        let slot_ident = &function.slot_ident;
        quote! {
            #[allow(non_upper_case_globals)]
            static #slot_ident: ::std::sync::atomic::AtomicPtr<()> =
                ::std::sync::atomic::AtomicPtr::new(::std::ptr::null_mut());
        }
    });
    let register_parameters = functions.iter().map(|function| {
        let register_ident = &function.register_ident;
        let register_ty = function_pointer_type(function.returns_value);
        quote!(#register_ident: #register_ty)
    });
    let register_body = functions.iter().map(|function| {
        let slot_ident = &function.slot_ident;
        let register_ident = &function.register_ident;
        quote! {
            #slot_ident.store(
                #register_ident.map_or(::std::ptr::null_mut(), |function| function as *mut ()),
                ::std::sync::atomic::Ordering::Release,
            );
        }
    });
    let wrappers = functions.iter().map(import_wrapper);

    Ok(quote! {
        #(#slot_definitions)*

        #[doc(hidden)]
        #[unsafe(export_name = concat!("atom_", env!("CARGO_CRATE_NAME"), "_register_imports"))]
        pub extern "C" fn __atom_import_register_imports(#(#register_parameters),*) {
            #(#register_body)*
        }

        #(#wrappers)*
    })
}

fn validate_export_signature(signature: &Signature) -> Result<()> {
    if signature.constness.is_some() {
        return Err(Error::new_spanned(
            signature,
            "#[atom_export] does not support const functions",
        ));
    }

    if signature.asyncness.is_some() {
        return Err(Error::new_spanned(
            signature,
            "#[atom_export] does not support async functions",
        ));
    }

    if signature.unsafety.is_some() {
        return Err(Error::new_spanned(
            signature,
            "#[atom_export] expects a safe Rust function",
        ));
    }

    if signature.abi.is_some() {
        return Err(Error::new_spanned(
            signature,
            "#[atom_export] expects a normal Rust ABI function",
        ));
    }

    if !signature.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &signature.generics,
            "#[atom_export] does not support generic functions",
        ));
    }

    for input in &signature.inputs {
        if let FnArg::Receiver(receiver) = input {
            return Err(Error::new_spanned(
                receiver,
                "#[atom_export] does not support methods with self receivers",
            ));
        }
    }

    if signature.inputs.len() > 1 {
        return Err(Error::new_spanned(
            &signature.inputs,
            "#[atom_export] currently supports at most one parameter; wrap multiple fields in a #[atom_record] request struct",
        ));
    }

    Ok(())
}

fn validate_import_block(import_block: &ItemForeignMod) -> Result<()> {
    match import_block
        .abi
        .name
        .as_ref()
        .map(syn::LitStr::value)
        .as_deref()
    {
        Some("C") => {}
        _ => {
            return Err(Error::new_spanned(
                &import_block.abi,
                "#[atom_import] expects an extern \"C\" block",
            ));
        }
    }

    if import_block.items.is_empty() {
        return Err(Error::new_spanned(
            import_block,
            "#[atom_import] requires at least one foreign function",
        ));
    }

    for item in &import_block.items {
        match item {
            ForeignItem::Fn(function) => validate_import_signature(&function.sig)?,
            other => {
                return Err(Error::new_spanned(
                    other,
                    "#[atom_import] may only contain foreign functions",
                ));
            }
        }
    }

    Ok(())
}

fn validate_import_signature(signature: &Signature) -> Result<()> {
    if signature.constness.is_some() {
        return Err(Error::new_spanned(
            signature,
            "#[atom_import] does not support const functions",
        ));
    }

    if signature.asyncness.is_some() {
        return Err(Error::new_spanned(
            signature,
            "#[atom_import] does not support async functions",
        ));
    }

    if !signature.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &signature.generics,
            "#[atom_import] does not support generic functions",
        ));
    }

    if signature.variadic.is_some() {
        return Err(Error::new_spanned(
            signature,
            "#[atom_import] does not support variadic functions",
        ));
    }

    for input in &signature.inputs {
        if let FnArg::Receiver(receiver) = input {
            return Err(Error::new_spanned(
                receiver,
                "#[atom_import] does not support methods with self receivers",
            ));
        }
    }

    Ok(())
}

fn wrapper_inputs(signature: &Signature) -> TokenStream {
    if signature.inputs.is_empty() {
        quote! {
            out_response_flatbuffer: *mut ::atom_ffi::AtomOwnedBuffer,
            out_error_flatbuffer: *mut ::atom_ffi::AtomOwnedBuffer,
        }
    } else {
        quote! {
            input_flatbuffer: ::atom_ffi::AtomSlice,
            out_response_flatbuffer: *mut ::atom_ffi::AtomOwnedBuffer,
            out_error_flatbuffer: *mut ::atom_ffi::AtomOwnedBuffer,
        }
    }
}

fn decode_input(signature: &Signature) -> Result<TokenStream> {
    let Some(parameter) = export_parameters(signature)?.into_iter().next() else {
        return Ok(TokenStream::new());
    };

    let binding = parameter.binding;
    let decode_ty = parameter.decode_ty;

    Ok(quote! {
        let #binding = match <#decode_ty as ::atom_ffi::AtomExportInput>::decode_atom_export(
            input_flatbuffer,
        ) {
            Ok(value) => value,
            Err(error) => {
                unsafe {
                    ::atom_ffi::clear_buffer(out_response_flatbuffer);
                    ::atom_ffi::write_error_buffer(out_error_flatbuffer, &error);
                }
                return error.exit_code();
            }
        };
    })
}

fn call_arguments(signature: &Signature) -> Result<Vec<TokenStream>> {
    export_parameters(signature)?
        .into_iter()
        .map(|parameter| {
            let binding = parameter.binding;
            Ok(match parameter.mode {
                ParameterMode::Owned => quote!(#binding),
                ParameterMode::BorrowedStr => quote!(#binding.as_str()),
            })
        })
        .collect()
}

fn return_handling(
    signature: &Signature,
    function_ident: &Ident,
    call_arguments: &[TokenStream],
) -> Result<TokenStream> {
    let call = quote!(#function_ident(#(#call_arguments),*));

    match parse_result_output(&signature.output)? {
        ExportReturn::Value(output_ty) => Ok(quote! {
            let __atom_output = #call;
            match <#output_ty as ::atom_ffi::AtomExportOutput>::encode_atom_export(__atom_output) {
                Ok(bytes) => {
                    unsafe {
                        ::atom_ffi::write_response_buffer(out_response_flatbuffer, bytes);
                        ::atom_ffi::clear_buffer(out_error_flatbuffer);
                    }
                    0
                }
                Err(error) => {
                    unsafe {
                        ::atom_ffi::clear_buffer(out_response_flatbuffer);
                        ::atom_ffi::write_error_buffer(out_error_flatbuffer, &error);
                    }
                    error.exit_code()
                }
            }
        }),
        ExportReturn::Result { ok_ty, err_ty } => Ok(quote! {
            match #call {
                Ok(__atom_output) => {
                    match <#ok_ty as ::atom_ffi::AtomExportOutput>::encode_atom_export(__atom_output) {
                        Ok(bytes) => {
                            unsafe {
                                ::atom_ffi::write_response_buffer(out_response_flatbuffer, bytes);
                                ::atom_ffi::clear_buffer(out_error_flatbuffer);
                            }
                            0
                        }
                        Err(error) => {
                            unsafe {
                                ::atom_ffi::clear_buffer(out_response_flatbuffer);
                                ::atom_ffi::write_error_buffer(out_error_flatbuffer, &error);
                            }
                            error.exit_code()
                        }
                    }
                }
                Err(__atom_error) => {
                    let __atom_error: ::atom_ffi::AtomError =
                        <#err_ty as ::core::convert::Into<::atom_ffi::AtomError>>::into(__atom_error);
                    unsafe {
                        ::atom_ffi::clear_buffer(out_response_flatbuffer);
                        ::atom_ffi::write_error_buffer(out_error_flatbuffer, &__atom_error);
                    }
                    __atom_error.exit_code()
                }
            }
        }),
    }
}

fn parse_result_output(output: &ReturnType) -> Result<ExportReturn> {
    let output_ty = match output {
        ReturnType::Default => return Ok(ExportReturn::Value(syn::parse_quote!(()))),
        ReturnType::Type(_, ty) => ty.as_ref().clone(),
    };

    if let Type::Path(type_path) = &output_ty
        && let Some(segment) = type_path.path.segments.last()
        && segment.ident == "Result"
        && let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments
    {
        let mut generic_types = arguments.args.iter().filter_map(|argument| match argument {
            syn::GenericArgument::Type(ty) => Some(ty.clone()),
            _ => None,
        });
        let ok_ty = generic_types.next().ok_or_else(|| {
            Error::new_spanned(
                type_path,
                "Result return type must include an Ok value type",
            )
        })?;
        let err_ty = generic_types.next().ok_or_else(|| {
            Error::new_spanned(
                type_path,
                "Result return type must include an error value type",
            )
        })?;
        return Ok(ExportReturn::Result { ok_ty, err_ty });
    }

    Ok(ExportReturn::Value(output_ty))
}

fn build_import_function(function: &ForeignItemFn) -> Result<ImportFunction> {
    let mut wrapper_inputs = Vec::with_capacity(function.sig.inputs.len());
    let mut input_bindings = Vec::with_capacity(function.sig.inputs.len());
    let mut input_types = Vec::with_capacity(function.sig.inputs.len());

    for (index, input) in function.sig.inputs.iter().enumerate() {
        let typed = match input {
            FnArg::Typed(typed) => typed,
            FnArg::Receiver(receiver) => {
                return Err(Error::new_spanned(
                    receiver,
                    "#[atom_import] does not support methods with self receivers",
                ));
            }
        };
        let binding = format_ident!("__atom_import_input_{index}");
        let ty = typed.ty.as_ref().clone();
        wrapper_inputs.push(quote!(#binding: #ty));
        input_bindings.push(binding);
        input_types.push(ty);
    }

    let tuple_type = match input_types.as_slice() {
        [] => quote!(()),
        [only] => quote!((#only,)),
        many => quote!((#(#many),*)),
    };
    let tuple_expr = match input_bindings.as_slice() {
        [] => quote!(()),
        [only] => quote!((#only,)),
        many => quote!((#(#many),*)),
    };
    let output_ty = match &function.sig.output {
        ReturnType::Default => None,
        ReturnType::Type(_, ty) => Some(ty.as_ref().clone()),
    };

    Ok(ImportFunction {
        attrs: function.attrs.clone(),
        vis: function.vis.clone(),
        ident: function.sig.ident.clone(),
        wrapper_inputs,
        input_tuple_type: tuple_type,
        input_tuple_expr: tuple_expr,
        output: function.sig.output.clone(),
        output_ty,
        returns_value: !matches!(function.sig.output, ReturnType::Default),
        slot_ident: format_ident!("__atom_import_slot_{}", function.sig.ident),
        register_ident: format_ident!("__atom_import_{}_fn", function.sig.ident),
    })
}

fn function_pointer_type(returns_value: bool) -> TokenStream {
    if returns_value {
        quote!(Option<extern "C" fn(::atom_ffi::AtomSlice, *mut ::atom_ffi::AtomOwnedBuffer)>)
    } else {
        quote!(Option<extern "C" fn(::atom_ffi::AtomSlice)>)
    }
}

fn import_wrapper(function: &ImportFunction) -> TokenStream {
    let attrs = &function.attrs;
    let vis = &function.vis;
    let ident = &function.ident;
    let wrapper_inputs = &function.wrapper_inputs;
    let slot_ident = &function.slot_ident;
    let input_tuple_type = &function.input_tuple_type;
    let input_tuple_expr = &function.input_tuple_expr;
    let output = &function.output;

    let call = if let Some(output_ty) = &function.output_ty {
        quote! {
            let __atom_import_fn: extern "C" fn(
                ::atom_ffi::AtomSlice,
                *mut ::atom_ffi::AtomOwnedBuffer,
            ) = unsafe { ::std::mem::transmute(__atom_import_ptr) };
            let mut __atom_import_response = ::atom_ffi::AtomOwnedBuffer::empty();
            __atom_import_fn(__atom_import_slice, &raw mut __atom_import_response);
            let __atom_import_response = unsafe { __atom_import_response.into_vec() };
            match <#output_ty as ::atom_ffi::AtomImportOutput>::decode_atom_import(
                &__atom_import_response,
            ) {
                Ok(value) => value,
                Err(error) => {
                    panic!(
                        "atom_import: failed to decode {}::{} response: {}",
                        env!("CARGO_CRATE_NAME"),
                        stringify!(#ident),
                        error,
                    );
                }
            }
        }
    } else {
        quote! {
            let __atom_import_fn: extern "C" fn(::atom_ffi::AtomSlice) =
                unsafe { ::std::mem::transmute(__atom_import_ptr) };
            __atom_import_fn(__atom_import_slice);
        }
    };

    quote! {
        #(#attrs)*
        #vis fn #ident(#(#wrapper_inputs),*) #output {
            let __atom_import_ptr =
                #slot_ident.load(::std::sync::atomic::Ordering::Acquire);
            assert!(
                !__atom_import_ptr.is_null(),
                concat!(
                    "atom_import: ",
                    env!("CARGO_CRATE_NAME"),
                    "::",
                    stringify!(#ident),
                    " not registered. Was the native provider registered at startup?"
                ),
            );

            let __atom_import_input =
                match <#input_tuple_type as ::atom_ffi::AtomImportInput>::encode_atom_import(
                    #input_tuple_expr,
                ) {
                    Ok(value) => value,
                    Err(error) => {
                        panic!(
                            "atom_import: failed to encode {}::{} arguments: {}",
                            env!("CARGO_CRATE_NAME"),
                            stringify!(#ident),
                            error,
                        );
                    }
                };
            let __atom_import_slice = ::atom_ffi::AtomSlice::from_bytes(&__atom_import_input);

            #call
        }
    }
}

fn export_parameters(signature: &Signature) -> Result<Vec<ExportParameter>> {
    signature
        .inputs
        .iter()
        .enumerate()
        .map(|(index, input)| {
            let typed = match input {
                FnArg::Typed(typed) => typed,
                FnArg::Receiver(receiver) => {
                    return Err(Error::new_spanned(
                        receiver,
                        "#[atom_export] does not support methods with self receivers",
                    ));
                }
            };

            Ok(ExportParameter {
                binding: format_ident!("__atom_input_{index}"),
                decode_ty: decode_type(typed),
                mode: parameter_mode(typed)?,
            })
        })
        .collect()
}

fn decode_type(typed: &PatType) -> Type {
    match borrowed_str_type(typed.ty.as_ref()) {
        Some(()) => syn::parse_quote!(::std::string::String),
        None => typed.ty.as_ref().clone(),
    }
}

fn parameter_mode(typed: &PatType) -> Result<ParameterMode> {
    if borrowed_str_type(typed.ty.as_ref()).is_some() {
        Ok(ParameterMode::BorrowedStr)
    } else if matches!(typed.ty.as_ref(), Type::Reference(_)) {
        Err(Error::new_spanned(
            &typed.ty,
            "#[atom_export] currently only supports borrowed `&str` parameters",
        ))
    } else {
        Ok(ParameterMode::Owned)
    }
}

fn borrowed_str_type(ty: &Type) -> Option<()> {
    let reference = match ty {
        Type::Reference(reference) => reference,
        _ => return None,
    };
    match reference.elem.as_ref() {
        Type::Path(TypePath { path, .. }) if path.is_ident("str") => Some(()),
        _ => None,
    }
}

enum ExportReturn {
    Value(Type),
    Result { ok_ty: Type, err_ty: Type },
}

enum ParameterMode {
    Owned,
    BorrowedStr,
}

struct ExportParameter {
    binding: Ident,
    decode_ty: Type,
    mode: ParameterMode,
}

struct ImportFunction {
    attrs: Vec<syn::Attribute>,
    vis: Visibility,
    ident: Ident,
    wrapper_inputs: Vec<TokenStream>,
    input_tuple_type: TokenStream,
    input_tuple_expr: TokenStream,
    output: ReturnType,
    output_ty: Option<Type>,
    returns_value: bool,
    slot_ident: Ident,
    register_ident: Ident,
}

#[cfg(test)]
mod tests {
    use proc_macro2::TokenStream;
    use quote::quote;
    use syn::{ItemFn, ItemForeignMod, parse_quote};

    use super::{expand_atom_export, expand_atom_import, expand_atom_record};

    fn normalize(tokens: proc_macro2::TokenStream) -> String {
        tokens.to_string()
    }

    #[test]
    fn atom_record_passthroughs_structs() {
        let item = quote! {
            pub struct DeviceInfo {
                pub model: String,
            }
        };
        let expanded = expand_atom_record(TokenStream::new(), item.clone()).expect("record macro");
        assert_eq!(normalize(expanded), normalize(item));
    }

    #[test]
    fn atom_record_rejects_non_records() {
        let item = quote!(
            pub fn get() {}
        );
        let error = expand_atom_record(TokenStream::new(), item).expect_err("function should fail");
        assert!(error.to_string().contains("structs or enums"));
    }

    #[test]
    fn atom_export_no_input_generates_runtime_gate_and_export_name() {
        let function: ItemFn = parse_quote! {
            pub fn get() -> DeviceInfo {
                unreachable!()
            }
        };

        let expanded =
            expand_atom_export(TokenStream::new(), quote!(#function)).expect("export macro");
        let normalized = normalize(expanded);
        assert!(normalized.contains("export_name"));
        assert!(normalized.contains("env ! (\"CARGO_CRATE_NAME\")"));
        assert!(normalized.contains(":: atom_runtime :: ensure_running ()"));
        assert!(normalized.contains(":: atom_ffi :: write_response_buffer"));
    }

    #[test]
    fn atom_export_result_input_generates_decode_and_error_routing() {
        let function: ItemFn = parse_quote! {
            pub fn echo(request: EchoRequest) -> Result<String, AppError> {
                unreachable!()
            }
        };

        let expanded =
            expand_atom_export(TokenStream::new(), quote!(#function)).expect("export macro");
        let normalized = normalize(expanded);
        assert!(
            normalized.contains(
                "< EchoRequest as :: atom_ffi :: AtomExportInput > :: decode_atom_export"
            )
        );
        assert!(normalized.contains("match echo (__atom_input_0)"));
        assert!(normalized.contains("Into < :: atom_ffi :: AtomError >"));
    }

    #[test]
    fn atom_export_enum_return_uses_codec_trait() {
        let function: ItemFn = parse_quote! {
            pub fn status() -> ConnectionStatus {
                ConnectionStatus::Connected
            }
        };

        let expanded =
            expand_atom_export(TokenStream::new(), quote!(#function)).expect("export macro");
        let normalized = normalize(expanded);
        assert!(normalized.contains(
            "< ConnectionStatus as :: atom_ffi :: AtomExportOutput > :: encode_atom_export"
        ));
    }

    #[test]
    fn atom_export_borrowed_str_decodes_string_then_borrows() {
        let function: ItemFn = parse_quote! {
            pub fn echo(message: &str) -> String {
                message.to_owned()
            }
        };

        let expanded =
            expand_atom_export(TokenStream::new(), quote!(#function)).expect("export macro");
        let normalized = normalize(expanded);
        assert!(normalized.contains(
            "< :: std :: string :: String as :: atom_ffi :: AtomExportInput > :: decode_atom_export"
        ));
        assert!(normalized.contains("echo (__atom_input_0 . as_str ())"));
    }

    #[test]
    fn atom_export_unit_return_uses_output_trait() {
        let function: ItemFn = parse_quote! {
            pub fn clear() {}
        };

        let expanded =
            expand_atom_export(TokenStream::new(), quote!(#function)).expect("export macro");
        let normalized = normalize(expanded);
        assert!(
            normalized.contains("< () as :: atom_ffi :: AtomExportOutput > :: encode_atom_export")
        );
    }

    #[test]
    fn atom_export_rejects_multiple_parameters() {
        let function: ItemFn = parse_quote! {
            pub fn echo(first: EchoRequest, second: EchoRequest) -> String {
                let _ = (first, second);
                unreachable!()
            }
        };

        let error =
            expand_atom_export(TokenStream::new(), quote!(#function)).expect_err("should fail");
        assert!(error.to_string().contains("supports at most one parameter"));
    }

    #[test]
    fn atom_import_generates_registration_slots_and_panic_guard() {
        let block: ItemForeignMod = parse_quote! {
            extern "C" {
                pub fn set(key: String, value: String);
            }
        };

        let expanded =
            expand_atom_import(TokenStream::new(), quote!(#block)).expect("import macro");
        let normalized = normalize(expanded);
        assert!(normalized.contains("AtomicPtr < () >"));
        assert!(normalized.contains("_register_imports"));
        assert!(normalized.contains("AtomImportInput"));
        assert!(normalized.contains("not registered"));
    }

    #[test]
    fn atom_import_returning_value_uses_output_trait_and_owned_buffer() {
        let block: ItemForeignMod = parse_quote! {
            extern "C" {
                pub fn get(key: String) -> GetResult;
            }
        };

        let expanded =
            expand_atom_import(TokenStream::new(), quote!(#block)).expect("import macro");
        let normalized = normalize(expanded);
        assert!(normalized.contains("AtomOwnedBuffer :: empty"));
        assert!(
            normalized
                .contains("< GetResult as :: atom_ffi :: AtomImportOutput > :: decode_atom_import")
        );
        assert!(normalized.contains(
            "extern \"C\" fn (:: atom_ffi :: AtomSlice , * mut :: atom_ffi :: AtomOwnedBuffer)"
        ));
    }

    #[test]
    fn atom_import_result_return_uses_import_output_trait() {
        let block: ItemForeignMod = parse_quote! {
            extern "C" {
                pub fn try_echo(message: String) -> Result<EchoResponse, AtomError>;
            }
        };

        let expanded =
            expand_atom_import(TokenStream::new(), quote!(#block)).expect("import macro");
        let normalized = normalize(expanded);
        assert!(normalized.contains(
            "< Result < EchoResponse , AtomError > as :: atom_ffi :: AtomImportOutput > :: decode_atom_import"
        ));
    }

    #[test]
    fn atom_import_unit_return_uses_input_only_function_pointer() {
        let block: ItemForeignMod = parse_quote! {
            extern "C" {
                pub fn remove(key: String);
            }
        };

        let expanded =
            expand_atom_import(TokenStream::new(), quote!(#block)).expect("import macro");
        let normalized = normalize(expanded);
        assert!(normalized.contains("extern \"C\" fn (:: atom_ffi :: AtomSlice)"));
        assert!(!normalized.contains("AtomImportOutput"));
    }

    #[test]
    fn atom_import_rejects_non_c_abi() {
        let block: ItemForeignMod = parse_quote! {
            extern "Rust" {
                pub fn set(key: String);
            }
        };

        let error =
            expand_atom_import(TokenStream::new(), quote!(#block)).expect_err("should fail");
        assert!(error.to_string().contains("extern \"C\""));
    }

    #[test]
    fn atom_import_rejects_non_function_items() {
        let block: ItemForeignMod = parse_quote! {
            extern "C" {
                static VALUE: i32;
            }
        };

        let error =
            expand_atom_import(TokenStream::new(), quote!(#block)).expect_err("should fail");
        assert!(error.to_string().contains("foreign functions"));
    }
}
