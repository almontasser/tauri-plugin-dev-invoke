use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, FnArg, ItemFn, Pat};

#[proc_macro_attribute]
pub fn command(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let name = &input.sig.ident;
    let vis = &input.vis;
    let wrapper_name = format_ident!("__dev_invoke_wrapper_{}", name);
    let args_struct_name = format_ident!("__DevInvokeArgs_{}", name);

    // Extract arguments for the helper struct
    let mut struct_fields = Vec::new();
    let mut call_args = Vec::new();

    for arg in &input.sig.inputs {
        if let FnArg::Typed(pat_type) = arg {
            if let Pat::Ident(pat_ident) = &*pat_type.pat {
                let arg_name = &pat_ident.ident;
                let arg_type = &pat_type.ty;

                struct_fields.push(quote! {
                    pub #arg_name: #arg_type
                });
                call_args.push(quote! {
                    args.#arg_name
                });
            }
        }
    }

    let expanded = quote! {
        #[tauri::command]
        #input

        #[allow(non_camel_case_types)]
        #[derive(serde::Deserialize)]
        pub struct #args_struct_name {
            #(#struct_fields),*
        }

        #vis fn #wrapper_name(args_json: serde_json::Value) -> std::result::Result<serde_json::Value, String> {
            let args: #args_struct_name = serde_json::from_value(args_json)
                .map_err(|e| format!("Failed to deserialize arguments for {}: {}", stringify!(#name), e))?;

            // Call the original function
            let result = #name(#(#call_args),*);

            // Serialize result
            serde_json::to_value(result)
                .map_err(|e| format!("Failed to serialize result for {}: {}", stringify!(#name), e))
        }
    };

    TokenStream::from(expanded)
}
