use proc_macro2::TokenStream;
use quote::quote;
use syn::{FnArg, ItemTrait, Meta, Pat, ReturnType, TraitItem, parse_quote, parse2};

struct ClosureImpl {
    name: syn::Ident,
    types: Vec<syn::Type>,
    return_type: ReturnType,
    is_option: bool,
    is_result: bool,
    arg_names: Vec<syn::Ident>,
}
/// Example usage of `service_handler`:
///
/// ```rust
/// #[service_handler]
/// pub trait AppService: Send + Sync {
///     #[service(fun=comments::get_comments_from_url,blank=Default::default())]
///     async fn get_comments_from_url(&self, url: &str, force: bool) -> Vec<Comment>;
/// }
/// ```
///
/// For default implementations:
/// ```rust
/// #[service_handler]
/// pub trait Service: Send + Sync {
///     #[service(default = in_module::function)]
///     fn some_fun(&self, a: u32, b: String);
/// }
/// ```
pub fn service_handler_impl(_args: TokenStream, input: TokenStream) -> TokenStream {
    let mut trait_item = match parse2::<ItemTrait>(input) {
        Ok(t) => t,
        Err(e) => return e.to_compile_error(),
    };

    let mut default_impls = Vec::new();
    let mut blank_impls = Vec::new();
    let mut closures: Vec<ClosureImpl> = Vec::new();
    for item in &mut trait_item.items {
        if let TraitItem::Fn(fn_item) = item {
            // Check for #[service(...)] attribute
            let mut service_attr_idx = None;
            for (i, attr) in fn_item.attrs.iter().enumerate() {
                if attr.path().is_ident("service") {
                    service_attr_idx = Some(i);
                    break;
                }
            }

            if let Some(idx) = service_attr_idx {
                let attr = fn_item.attrs.remove(idx);

                // Handle default = path and blank = expr
                if let Meta::List(list) = attr.meta {
                    let inner = list
                        .parse_args_with(
                            syn::punctuated::Punctuated::<syn::MetaNameValue, syn::Token![,]>::parse_terminated,
                        )
                        .unwrap();
                    let fn_name = &fn_item.sig.ident;
                    let args = fn_item.sig.inputs.iter().filter_map(|arg| {
                        if let FnArg::Typed(pat) = arg {
                            if let Pat::Ident(pi) = &*pat.pat {
                                return Some(quote!(#pi));
                            }
                        }
                        None
                    });
                    let args_collected: Vec<_> = args.collect();
                    let inputs = &fn_item.sig.inputs;
                    let output = &fn_item.sig.output;

                    let mut blank_expr = None;
                    for nv in &inner {
                        if nv.path.is_ident("default") {
                            let path = &nv.value;
                            default_impls.push(quote! {
                                fn #fn_name(#inputs) #output {
                                    #path(#(#args_collected),*)
                                }
                            });
                        }
                        if nv.path.is_ident("blank") {
                            blank_expr = Some(&nv.value);
                        }
                    }

                    let output = &fn_item.sig.output;
                    let body = if let Some(expr) = blank_expr {
                        quote! { #expr }
                    } else {
                        quote! { Default::default() }
                    };

                    blank_impls.push(quote! {
                        fn #fn_name(#inputs) #output {
                            #body
                        }
                    });
                }
            }

            // Transform async fn to sync returning Pin<Box<dyn Future>>
            if fn_item.sig.asyncness.is_some() {
                fn_item.sig.asyncness = None;
                let ret = &fn_item.sig.output;
                let output_type = match ret {
                    ReturnType::Default => quote!(()),
                    ReturnType::Type(_, ty) => quote!(#ty),
                };

                fn_item.sig.output = parse_quote! {
                    -> std::pin::Pin<Box<dyn std::future::Future<Output = #output_type> + Send + '_>>
                };
            }
            let args: Vec<syn::Type> = fn_item
                .sig
                .inputs
                .iter()
                .filter_map(|arg| {
                    if let FnArg::Typed(pat) = arg {
                        return Some(*pat.ty.clone());
                    }
                    None
                })
                .collect();
            let arg_names: Vec<syn::Ident> = fn_item
                .sig
                .inputs
                .iter()
                .filter_map(|arg| {
                    if let FnArg::Typed(pat) = arg {
                        if let Pat::Ident(pi) = &*pat.pat {
                            return Some(pi.ident.clone());
                        }
                    }
                    None
                })
                .collect();

            let return_type_path = match &fn_item.sig.output {
                ReturnType::Type(_, ty) => {
                    if let syn::Type::Path(p) = &**ty {
                        Some(p)
                    } else {
                        None
                    }
                }
                _ => None,
            };

            let is_option = return_type_path
                .as_ref()
                .map_or(false, |p| p.path.segments.last().map_or(false, |s| s.ident == "Option"));
            let is_result = return_type_path
                .as_ref()
                .map_or(false, |p| p.path.segments.last().map_or(false, |s| s.ident == "Result"));

            closures.push(ClosureImpl {
                name: fn_item.sig.ident.clone(),
                types: args,
                return_type: fn_item.sig.output.clone(),
                is_option,
                is_result,
                arg_names,
            });
        }
    }

    let trait_name = &trait_item.ident;
    let default_struct_name = syn::Ident::new(&format!("{}Default", trait_name), trait_name.span());
    let blank_struct_name = syn::Ident::new(&format!("{}Blank", trait_name), trait_name.span());
    let closures_struct_name = syn::Ident::new(&format!("{}Closures", trait_name), trait_name.span());
    let closures_impl: Vec<TokenStream> = closures
        .iter()
        .map(|c| {
            let n = &c.name;
            let ts = &c.types;
            let rt = &c.return_type;
            quote! {
                #n: Box<dyn Fn(#(#ts),*) #rt + Send + Sync>
            }
        })
        .collect();

    let closures_trait_impl: Vec<TokenStream> = closures
        .iter()
        .map(|c| {
            let n = &c.name;
            let args = &c.arg_names;
            let arg_types = &c.types;
            let rt = &c.return_type;
            quote! {
                fn #n(&self, #(#args: #arg_types),*) #rt {
                    (self.#n)(#(#args),*)
                }
            }
        })
        .collect();

    quote! {
        #trait_item

        pub struct #default_struct_name {}

        impl #trait_name for #default_struct_name {
            #(#default_impls)*
        }

        pub struct #blank_struct_name {}

        impl #trait_name for #blank_struct_name {
            #(#blank_impls)*
        }

        pub struct #closures_struct_name {
            #(#closures_impl),*
        }
        impl #trait_name for #closures_struct_name {
            #(#closures_trait_impl)*
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    fn pretty_print(tokens: proc_macro2::TokenStream) -> String {
        println!("HERE");
        // 1. Parse the tokens into a syntax tree (syn::File)
        // Note: This requires the tokens to be a valid Rust file (items only)
        let Ok(syntax_tree) = syn::parse2::<syn::File>(tokens.clone()) else {
            return tokens.to_string();
        };
        println!("HERE");

        // 2. Format it
        prettyplease::unparse(&syntax_tree)
    }
    #[test]
    fn service_debug_macro_output() {
        // Define what the input code looks like
        let input = quote! {

            #[service_handler]
            pub trait AppService: Send + Sync {
                #[service(default=comments::get_comments_from_url)]//,blank=Default::default333())]
                async fn get_comments_from_url(&self, url: &str, force: bool) -> Vec<Comment>;
            }
        }
        .into();

        // Run your transformation logic
        let output = service_handler_impl(TokenStream::new(), input);
        let formatted = pretty_print(output);

        // PRINT the result so you can see it in your terminal
        println!("--- MACRO OUTPUT ---");
        println!("{}", formatted);
        println!("--------------------");

        // Optional: you can even assert things here
        assert!(formatted.to_string().contains("generated_test_func"));
    }
}
