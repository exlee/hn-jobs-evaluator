use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::{
    FnArg, ItemTrait, Pat, ReturnType, TraitItem, parse_quote, parse_quote_spanned, parse2, spanned::Spanned as _,
};

struct ClosureImpl {
    name: syn::Ident,
    inputs: syn::punctuated::Punctuated<syn::FnArg, syn::Token![,]>,
    types: Vec<syn::Type>,
    orig_return_type: ReturnType,
    return_type: ReturnType,
    arg_names: Vec<syn::Ident>,
    is_async: bool,
    blank_expr: Option<syn::Expr>,
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
    let mut trait_item = match parse2::<ItemTrait>(input.clone()) {
        Ok(t) => t,
        Err(e) => {
            return e.to_compile_error();
        }
    };

    let mut default_impls = Vec::new();
    let mut blank_impls = Vec::new();
    let mut closures: Vec<ClosureImpl> = Vec::new();
    for item in &mut trait_item.items {
        if let TraitItem::Fn(fn_item) = item {
            let mut function_path = None;
            let mut blank_expr_captured = None;

            // Extract and remove #[function(...)] and #[blank(...)] attributes
            fn_item.attrs.retain(|attr| {
                if attr.path().is_ident("function") {
                    if let Ok(meta) = attr.parse_args::<syn::Expr>() {
                        function_path = Some(meta);
                    }
                    false
                } else if attr.path().is_ident("blank") {
                    if let Ok(meta) = attr.parse_args::<syn::Expr>() {
                        blank_expr_captured = Some(meta);
                    }
                    false
                } else {
                    true
                }
            });

            let is_async = fn_item.sig.asyncness.is_some();
            let return_type = match &fn_item.sig.output {
                syn::ReturnType::Default => {
                    // This is the '()' type
                    fn_item.sig.fn_token.span()
                }
                syn::ReturnType::Type(_, ty) => {
                    // This is the 'Vec<Story>' type
                    ty.span()
                }
            };

            // Transform async fn to sync returning Pin<Box<dyn Future>>
            let original_sig_output = fn_item.sig.output.clone();
            if is_async {
                fn_item.sig.asyncness = None;
                let ret = &fn_item.sig.output;
                let output_type = match ret {
                    ReturnType::Default => quote!(()),
                    ReturnType::Type(_, ty) => quote!(#ty),
                };

                fn_item.sig.output = parse_quote_spanned! { return_type =>
                    -> std::pin::Pin<Box<dyn std::future::Future<Output = #output_type> + Send + '_>>
                };
            }

            let fn_name = &fn_item.sig.ident;
            let args = fn_item.sig.inputs.iter().filter_map(|arg| {
                if let FnArg::Typed(pat) = arg {
                    if let Pat::Ident(pi) = &*pat.pat {
                        return Some(quote!(#pi));
                    }
                }
                None
            });
            let blank_inputs = fn_item.sig.inputs.iter().map(|arg| {
                if let FnArg::Typed(pat) = arg {
                    let mut pat = pat.clone();
                    pat.pat = Box::new(parse_quote!(_));
                    FnArg::Typed(pat)
                } else {
                    arg.clone()
                }
            });
            let args_collected: Vec<_> = args.collect();
            let inputs = &fn_item.sig.inputs;
            let output = &fn_item.sig.output;

            if let Some(path) = &function_path {
                let body = if is_async {
                    quote_spanned! { return_type =>
                        Box::pin(async move {
                            #path(#(#args_collected),*).await
                        })
                    }
                } else {
                    quote_spanned! { return_type => #path(#(#args_collected),*) }
                };
                default_impls.push(quote! {
                    fn #fn_name(#inputs) #output {
                        #body
                    }
                });
            }

            let body = if let Some(expr) = &blank_expr_captured {
                if is_async {
                    quote! {
                        Box::pin(async move {
                            #expr
                        })
                    }
                } else {
                    quote! { #expr }
                }
            } else {
                let return_type = match &fn_item.sig.output {
                    syn::ReturnType::Default => fn_item.sig.fn_token.span(),
                    syn::ReturnType::Type(_, ty) => ty.span(),
                };
                if is_async {
                    quote_spanned! { return_type =>  Box::pin(async { Default::default() }) }
                } else {
                    quote_spanned! { return_type => Default::default() }
                }
            };

            blank_impls.push(quote_spanned! { fn_item.sig.span() =>
                fn #fn_name(#(#blank_inputs),*) #output {
                    #body
                }
            });
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

            closures.push(ClosureImpl {
                name: fn_item.sig.ident.clone(),
                inputs: fn_item.sig.inputs.clone(),
                types: args,
                orig_return_type: original_sig_output,
                return_type: fn_item.sig.output.clone(),
                arg_names,
                is_async,
                blank_expr: blank_expr_captured,
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
            let rt = &c.orig_return_type;
            //     pub get_comments_from_url: Arc<dyn Fn(&str, bool) -> Vec<Comment> + Send + Sync>,
            quote! {
                #n: Arc<dyn Fn(#(#ts),*) #rt + Send + Sync>
            }
        })
        .collect();

    let closures_trait_impl: Vec<TokenStream> = closures
        .iter()
        .map(|c| {
            let n = &c.name;
            let args = &c.arg_names;
            let inputs = &c.inputs;
            let rt = &c.return_type;
            let is_async = c.is_async;

            let body = if is_async {
                quote! {
                    Box::pin(async move {
                        (self.#n)(#(#args),*)
                    })
                }
            } else {
                quote! { (self.#n)(#(#args),*) }
            };

            quote! {
                fn #n(#inputs) #rt {
                    #body
                }
            }
        })
        .collect();

    let closures_default_impl: Vec<TokenStream> = closures
        .iter()
        .map(|c| {
            let n = &c.name;
            let args = c.arg_names.iter().map(|_| quote!(_));
            let blank = if let Some(expr) = &c.blank_expr {
                quote!(#expr)
            } else {
                quote!(Default::default())
            };
            quote! {
                #n: Arc::new(|#(#args),*| #blank)
            }
        })
        .collect();

    quote! {
        #trait_item

        pub struct #default_struct_name {}

        impl #trait_name for #default_struct_name {
            #(#default_impls)*
        }
        #[cfg(test)]
        pub mod tests {
            use super::*;
            pub struct #blank_struct_name {}

            impl #trait_name for #blank_struct_name {
                #(#blank_impls)*
            }
            use std::sync::Arc;
            pub struct #closures_struct_name {
                #(#closures_impl),*
            }
            impl Default for #closures_struct_name {
                fn default() -> Self {
                    Self {
                        #(#closures_default_impl),*
                    }
                }
            }
            impl #trait_name for #closures_struct_name {
                #(#closures_trait_impl)*
            }
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
                #[function(crate::backend::comments::get_comments_from_url)]
                #[blank(Ok(Comment::default()))]
                async fn get_comments_from_url(&self, url: String, force: bool) -> Vec<Comment>;
                #[function(crate::backend::evaluation::evaluate_comment_cached)]
                async fn evaluate_comment_cached(
                    &self,
                    comment: Comment,
                    ev_cache: EvaluationCache,
                    api_key: String,
                ) -> anyhow::Result<Evaluation>;
                #[function(crate::backend::evaluation::create_evaluation_cache)]
                async fn create_evaluation_cache(
                    &self,
                    api_key: String,
                    pdf_path: PathBuf,
                    requirements: String,
                    ttl: Duration,
                ) -> Result<String, String>;
                #[function(crate::backend::job_description::parse_job_description)]
                fn parse_job_description(&self, llm_config: llmuxer::LlmConfig, input: String) -> Result<JobDescription, String>;
                #[function(notify_evaluation)]
                #[blank(Ok(()))]
                fn notify_evaluation(&self, id: u32, notify_data: NotifyData, evaluation: Evaluation) -> anyhow::Result<()>;

                #[function(crate::backend::front_page::get_front_page_stories)]
                async fn get_front_page_stories(&self) -> Vec<Story>;
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
