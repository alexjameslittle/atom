use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::Nothing;
use syn::{
    Error, FnArg, Ident, Item, ItemFn, ItemStruct, PatType, Result, ReturnType, Signature, Type,
    TypePath, parse2,
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
    let parameters = export_parameters(signature)?;
    if parameters.is_empty() {
        return Ok(TokenStream::new());
    }

    let decode_error = quote! {
        unsafe {
            ::atom_ffi::clear_buffer(out_response_flatbuffer);
            ::atom_ffi::write_error_buffer(out_error_flatbuffer, &error);
        }
        return error.exit_code();
    };

    if parameters.len() == 1 {
        let parameter = &parameters[0];
        let binding = &parameter.binding;
        let decode_ty = &parameter.decode_ty;
        return Ok(quote! {
            let #binding = match <#decode_ty as ::atom_ffi::AtomExportInput>::decode_atom_export(
                input_flatbuffer,
            ) {
                Ok(value) => value,
                Err(error) => {
                    #decode_error
                }
            };
        });
    }

    let bindings = parameters.iter().map(|parameter| &parameter.binding);
    let decode_tys = parameters.iter().map(|parameter| &parameter.decode_ty);
    Ok(quote! {
        let (#(#bindings),*) = match <(#(#decode_tys),*) as ::atom_ffi::AtomExportInput>::decode_atom_export(
            input_flatbuffer,
        ) {
            Ok(value) => value,
            Err(error) => {
                #decode_error
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
                decode_ty: decode_type(typed)?,
                mode: parameter_mode(typed)?,
            })
        })
        .collect()
}

fn decode_type(typed: &PatType) -> Result<Type> {
    match borrowed_str_type(typed.ty.as_ref()) {
        Some(()) => Ok(syn::parse_quote!(::std::string::String)),
        None => Ok(typed.ty.as_ref().clone()),
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

#[cfg(test)]
mod tests {
    use proc_macro2::TokenStream;
    use quote::quote;
    use syn::{ItemFn, parse_quote};

    use super::{expand_atom_export, expand_atom_record};

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
}
