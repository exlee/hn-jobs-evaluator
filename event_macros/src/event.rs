use proc_macro2::Span;
use quote::{ToTokens, quote, quote_spanned};
use syn::{FnArg, Ident, ImplItem, ItemImpl, Pat, parse_quote, spanned::Spanned as _};

pub(crate) fn event_processor_impl(input: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    let mut item_impl: ItemImpl = syn::parse2(input).expect("Cannot parse input");
    let mut match_arms = Vec::new();
    let mut debug_arms = Vec::new();
    let mut handlers = Vec::new();
    let mut events = Vec::new();
    let mut i = 0;
    while i < item_impl.items.len() {
        let should_remove = if let ImplItem::Fn(method) = &mut item_impl.items[i] {
            if let Some(attr) = find_and_remove_handler_attr(&mut method.attrs) {
                let variant_ident: Ident = attr.parse_args().expect("Expected ident in #[handler()]");
                let method_name = &method.sig.ident;
                let method_body = &method.block;
                let span_ident = method.sig.inputs.iter().find_map(|arg| {
                    if let FnArg::Typed(pat_type) = arg {
                        let ty_str = pat_type.ty.to_token_stream().to_string();
                        if ty_str.ends_with("EventEnvelope") {
                            if let Pat::Ident(pat_ident) = &*pat_type.pat {
                                return Some(pat_ident.ident.clone());
                            }
                        }
                    }
                    None
                });
                let queue_ident = method.sig.inputs.iter().find_map(|arg| {
                    if let FnArg::Typed(pat_type) = arg {
                        let ty_str = pat_type.ty.to_token_stream().to_string();
                        if ty_str.contains("VecDeque") && ty_str.contains("EventEnvelope") {
                            if let Pat::Ident(pat_ident) = &*pat_type.pat {
                                return Some(pat_ident.ident.clone());
                            }
                        }
                    }
                    None
                });
                let method_inputs = method
                    .sig
                    .inputs
                    .clone()
                    .into_iter()
                    .filter(|arg| {
                        if let FnArg::Typed(pat_type) = arg {
                            let ty_str = pat_type.ty.to_token_stream().to_string();
                            let is_envelope = ty_str.ends_with("EventEnvelope");
                            let is_queue = ty_str.contains("VecDeque") && ty_str.contains("EventEnvelope");
                            !is_envelope && !is_queue
                        } else {
                            false
                        }
                    })
                    .collect::<Vec<FnArg>>();

                let span_assignment = if let Some(ident) = span_ident {
                    quote! {
                        let #ident = env;
                    }
                } else {
                    quote! {}
                };
                let queue_assignment = if let Some(ident) = queue_ident {
                    quote! {
                        let #ident = queue;
                    }
                } else {
                    quote! {}
                };

                let mut sig = method.sig.clone();
                let new_inputs = method_inputs
                    .clone()
                    .into_iter()
                    .filter(|arg| !matches!(arg, FnArg::Receiver(_)))
                    .collect();
                sig.inputs = new_inputs;
                let pat_fields: Vec<syn::PatType> = method_inputs
                    .clone()
                    .iter()
                    .filter_map(|arg| {
                        if let FnArg::Typed(pat) = arg {
                            Some(pat.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
                let event_match = if pat_fields.is_empty() {
                    quote! {}
                } else {
                    quote! {{..}}
                };

                let variant_fields_vec: Vec<syn::Field> = pat_fields
                    .clone()
                    .iter()
                    .filter_map(|p| {
                        if let syn::Pat::Ident(ident) = *p.pat.clone() {
                            let mut attrs = Vec::new();

                            fn check_type_for_skip(ty: &syn::Type) -> bool {
                                if let syn::Type::Path(type_path) = ty {
                                    if let Some(segment) = type_path.path.segments.last() {
                                        if segment.ident == "Arc" || segment.ident == "Rc" {
                                            return true;
                                        }
                                        if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                                            for arg in &args.args {
                                                if let syn::GenericArgument::Type(inner_ty) = arg {
                                                    if check_type_for_skip(inner_ty) {
                                                        return true;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                false
                            }

                            if check_type_for_skip(&p.ty) {
                                attrs.push(parse_quote!(#[serde(skip)]));
                            }

                            Some(syn::Field {
                                attrs,
                                vis: parse_quote!(),
                                mutability: syn::FieldMutability::None,
                                ident: Some(ident.ident),
                                colon_token: None,
                                ty: *p.ty.clone(),
                            })
                        } else {
                            None
                        }
                    })
                    .collect();
                let variant_fields = if variant_fields_vec.is_empty() {
                    syn::Fields::Unit
                } else {
                    syn::Fields::Named(syn::FieldsNamed {
                        brace_token: syn::token::Brace::default(),
                        named: variant_fields_vec.into_iter().collect(),
                    })
                };

                let enum_variant = syn::Variant {
                    attrs: vec![],
                    ident: variant_ident.clone(),
                    fields: variant_fields,
                    discriminant: None,
                };
                events.push(enum_variant);
                let field_names = pat_fields.iter().map(|pf| {
                    if let syn::Pat::Ident(ident) = *pf.pat.clone() {
                        quote! { #ident }
                    } else {
                        panic!("Expected identifier in function arguments")
                    }
                });
                let match_pattern = if field_names.clone().count() == 0 {
                    quote! {}
                } else {
                    quote! {{ #(#field_names),* }}
                };

                let guard_stmt = {
                    let variant_ident = variant_ident.clone();
                    quote! {
                        let Event::#variant_ident #match_pattern = env.event else { return; };
                    }
                };

                match_arms.push(quote! {
                    Event::#variant_ident #event_match => self.#method_name(env, queue)
                });

                debug_arms.push(quote! {
                    Event::#variant_ident #event_match => write!(f, stringify!(#variant_ident))
                });

                handlers.push(quote_spanned! { method.span() =>
                    fn #method_name(&self, env: EventEnvelope, queue: &mut VecDeque<EventEnvelope>)  {
                       #guard_stmt
                       #queue_assignment
                       #span_assignment
                       #method_body
                   }
                });
                true
            } else {
                false
            }
        } else {
            false
        };

        if should_remove {
            item_impl.items.remove(i);
        } else {
            i += 1;
        }
    }
    let handle_method = quote! {
       #[tracing::instrument(skip_all, parent=&env.span)]
       async fn handle(&self, env: EventEnvelope, queue: &mut VecDeque<EventEnvelope>) {
           use Event::*;
           tracing::debug!("handle: {:?}", env.event);
           match env.event {
               #(#match_arms),*
               ,_ => {},
           }
       }
    };

    let event_enum = syn::ItemEnum {
        attrs: parse_quote! {
            #[derive(Serialize, Deserialize, Clone )]
        },
        vis: parse_quote!(pub),
        enum_token: syn::token::Enum::default(),
        ident: syn::Ident::new("Event", Span::call_site()),
        generics: syn::Generics::default(),
        brace_token: syn::token::Brace::default(),
        variants: events.into_iter().collect(),
    };

    for handler in handlers {
        let hs = handler.clone().to_string();
        match syn::parse2(handler) {
            Ok(h) => item_impl.items.push(h),
            Err(e) => {
                eprintln!("Failed to parse handler:\n{}\nError: {}", hs, e);
                return e.to_compile_error().into();
            }
        }
    }
    //#(#handlers),*
    item_impl
        .items
        .push(syn::parse2(handle_method).expect("Failed to parse generated handle method"));
    item_impl.attrs.retain(|attr| !attr.path().is_ident("event_processor"));

    let debug_impl = quote! {
        impl std::fmt::Debug for Event {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self {
                    #(#debug_arms),*
                    ,_ => write!(f, "Unknown Event"),
                }
            }
        }
    };

    quote! {
        #event_enum
        #debug_impl
        #item_impl
    }
}
fn find_and_remove_handler_attr(attrs: &mut Vec<syn::Attribute>) -> Option<syn::Attribute> {
    let index = attrs.iter().position(|attr| attr.path().is_ident("handler"))?;
    Some(attrs.remove(index))
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    fn pretty_print(tokens: proc_macro2::TokenStream) -> String {
        // 1. Parse the tokens into a syntax tree (syn::File)
        // Note: This requires the tokens to be a valid Rust file (items only)
        let Ok(syntax_tree) = syn::parse2::<syn::File>(tokens.clone()) else {
            return tokens.to_string();
        };

        // 2. Format it
        prettyplease::unparse(&syntax_tree)
    }
    #[test]
    fn event_debug_macro_output() {
        // Define what the input code looks like
        let input = quote! {
            #[event_processor]
            impl EventHandler {
                pub fn normal_function() {
                    return 42
                }
                #[handler(MyStruct)]
                fn test_processor_1(&self, item: String, value: u32) {
                    println!("Hello world!");
                }
                #[handler(EventNoFields)]
                fn test_processor_2(&self) {
                    println!("Hello world!");
                }
                #[handler(EventNoFieldsEnv)]
                fn test_processor_3(&self, env: EventEnvelope) {
                    println!("Hello world!");
                }
                #[handler(EventNoFieldsEnvArc)]
                fn test_processor_4(&self, arc: Option<Arc<Something>>) {
                    println!("Hello world!");
                }

            }
        }
        .into();

        // Run your transformation logic
        let output = event_processor_impl(input);
        let formatted = pretty_print(output);

        // PRINT the result so you can see it in your terminal
        println!("--- MACRO OUTPUT ---");
        println!("{}", formatted);
        println!("--------------------");

        // Optional: you can even assert things here
        assert!(formatted.to_string().contains("generated_test_func"));
    }
}
