use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};
use syn::{
    Attribute, Field, FieldMutability, Fields, FieldsNamed, FnArg, Ident, ImplItem, ImplItemFn, ItemEnum, ItemImpl,
    Pat, PatType, Variant, parse_quote, parse_quote_spanned, spanned::Spanned as _,
};

pub(crate) fn event_processor_impl(input: TokenStream) -> TokenStream {
    let mut item_impl: ItemImpl = syn::parse2(input).expect("Cannot parse input");
    let mut context = ProcessorContext::default();

    let mut i = 0;
    while i < item_impl.items.len() {
        if let ImplItem::Fn(ref mut method) = item_impl.items[i] {
            if let Some(attr) = find_and_remove_handler_attr(&mut method.attrs) {
                let handler = process_handler_method(method, attr);
                context.push_handler(handler);
                item_impl.items.remove(i);
                continue;
            }
        }
        i += 1;
    }

    generate_output(item_impl, context)
}

#[derive(Default)]
struct ProcessorContext {
    match_arms: Vec<TokenStream>,
    debug_arms: Vec<TokenStream>,
    handlers: Vec<ImplItem>,
    events: Vec<Variant>,
}

impl ProcessorContext {
    fn push_handler(&mut self, handler: HandlerResult) {
        let variant_ident = &handler.variant_ident;
        let method_name = &handler.method_name;
        let event_match = if handler.has_fields {
            quote! {{..}}
        } else {
            quote! {}
        };

        self.match_arms.push(quote! {
            Event::#variant_ident #event_match => self.#method_name(env, queue)
        });

        self.debug_arms.push(quote! {
            Event::#variant_ident #event_match => write!(f, stringify!(#variant_ident))
        });

        self.events.push(handler.event_variant);
        self.handlers.push(handler.generated_method);
    }
}

struct HandlerResult {
    variant_ident: Ident,
    method_name: Ident,
    event_variant: Variant,
    generated_method: ImplItem,
    has_fields: bool,
}

fn process_handler_method(method: &ImplItemFn, attr: Attribute) -> HandlerResult {
    let variant_ident: Ident = attr.parse_args().expect("Expected ident in #[handler()]");
    let method_name = method.sig.ident.clone();
    let method_body = &method.block;

    let (envelope_ident, queue_ident, other_args) = analyze_method_inputs(&method.sig.inputs);

    let span_assignment = envelope_ident.map(|id| quote! { let #id = env; }).unwrap_or_default();
    let queue_assignment = queue_ident.map(|id| quote! { let #id = queue; }).unwrap_or_default();

    let pat_fields: Vec<PatType> = other_args
        .iter()
        .filter_map(|arg| {
            if let FnArg::Typed(pat) = arg {
                Some(pat.clone())
            } else {
                None
            }
        })
        .collect();

    let event_variant = create_event_variant(variant_ident.clone(), &pat_fields);
    let guard_stmt = create_guard_statement(&variant_ident, &pat_fields);
    let has_fields = !pat_fields.is_empty();

    let generated_method = parse_quote_spanned! { method.span() =>
        fn #method_name(&self, env: EventEnvelope, queue: &mut ::std::collections::VecDeque<EventEnvelope>) {
            #guard_stmt
            #queue_assignment
            #span_assignment
            #method_body
        }
    };

    HandlerResult {
        variant_ident,
        method_name,
        event_variant,
        generated_method,
        has_fields,
    }
}

fn analyze_method_inputs(
    inputs: &syn::punctuated::Punctuated<FnArg, syn::token::Comma>,
) -> (Option<Ident>, Option<Ident>, Vec<FnArg>) {
    let mut envelope_ident = None;
    let mut queue_ident = None;
    let mut other_args = Vec::new();

    for arg in inputs {
        if let FnArg::Typed(pat_type) = arg {
            let ty_str = pat_type.ty.to_token_stream().to_string();
            let is_envelope = ty_str.ends_with("EventEnvelope");
            let is_queue = ty_str.contains("VecDeque") && ty_str.contains("EventEnvelope");

            if is_envelope {
                if let Pat::Ident(pat_ident) = &*pat_type.pat {
                    envelope_ident = Some(pat_ident.ident.clone());
                }
            } else if is_queue {
                if let Pat::Ident(pat_ident) = &*pat_type.pat {
                    queue_ident = Some(pat_ident.ident.clone());
                }
            } else {
                other_args.push(arg.clone());
            }
        } else {
            other_args.push(arg.clone());
        }
    }

    (envelope_ident, queue_ident, other_args)
}

fn create_event_variant(ident: Ident, pat_fields: &[PatType]) -> Variant {
    let fields_vec: Vec<Field> = pat_fields
        .iter()
        .filter_map(|p| {
            if let Pat::Ident(pat_ident) = &*p.pat {
                Some(Field {
                    attrs: p.attrs.clone(),
                    vis: parse_quote!(),
                    mutability: FieldMutability::None,
                    ident: Some(pat_ident.ident.clone()),
                    colon_token: None,
                    ty: (*p.ty).clone(),
                })
            } else {
                None
            }
        })
        .collect();

    let fields = if fields_vec.is_empty() {
        Fields::Unit
    } else {
        Fields::Named(FieldsNamed {
            brace_token: Default::default(),
            named: fields_vec.into_iter().collect(),
        })
    };

    Variant {
        attrs: Vec::new(),
        ident,
        fields,
        discriminant: None,
    }
}

fn create_guard_statement(variant_ident: &Ident, pat_fields: &[PatType]) -> TokenStream {
    let field_names: Vec<_> = pat_fields
        .iter()
        .map(|pf| {
            if let Pat::Ident(ident) = &*pf.pat {
                quote! { #ident }
            } else {
                panic!("Expected identifier in function arguments")
            }
        })
        .collect();

    let match_pattern = if field_names.is_empty() {
        quote! {}
    } else {
        quote! {{ #(#field_names),* }}
    };

    quote! {
        let Event::#variant_ident #match_pattern = env.event else { unreachable!(); };
    }
}

fn generate_output(mut item_impl: ItemImpl, context: ProcessorContext) -> TokenStream {
    let ProcessorContext {
        match_arms,
        debug_arms,
        handlers,
        events,
    } = context;

    let event_enum = ItemEnum {
        attrs: parse_quote! { #[derive(Serialize, Deserialize, Clone)] },
        vis: parse_quote!(pub),
        enum_token: Default::default(),
        ident: Ident::new("Event", Span::call_site()),
        generics: Default::default(),
        brace_token: Default::default(),
        variants: events.into_iter().collect(),
    };

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

    item_impl.items.extend(handlers);

    item_impl.items.push(parse_quote! {
        #[tracing::instrument(skip_all, parent=&env.span)]
        async fn handle(&self, env: EventEnvelope, queue: &mut ::std::collections::VecDeque<EventEnvelope>) {
            use Event::*;
            tracing::debug!("handle: {:?}", env.event);
            match env.event {
                #(#match_arms),*
            }
        }
    });

    item_impl.items.push(parse_quote! {
        async fn run(&self, mut rx: ::tokio::sync::mpsc::Receiver<EventEnvelope>) {
            let mut queue: ::std::collections::VecDeque<EventEnvelope> = ::std::collections::VecDeque::new();
            loop {
                match rx.recv().await {
                    Some(envelope) => queue.push_back(envelope),
                    None => {
                        tracing::error!("event channel closed unexpectedly");
                        break;
                    }
                }
                while let Ok(envelope) = rx.try_recv() {
                    queue.push_back(envelope);
                }
                while let Some(envelope) = queue.pop_front() {
                    self.handle(envelope, &mut queue).await;
                }
            }
        }
    });

    item_impl.attrs.retain(|attr| !attr.path().is_ident("event_processor"));

    quote! {
        pub struct EventEnvelope {
            pub event: Event,
            pub span: tracing::Span,
        }
        #event_enum
        #debug_impl
        #item_impl
    }
}

fn find_and_remove_handler_attr(attrs: &mut Vec<Attribute>) -> Option<Attribute> {
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
    fn test_1() {
        let input = quote! {
            #[event_processor]
            impl EventHandler {
                #[handler(Event1)]
                fn handler(#[serde(skip)] test: Vec<Vec<u32>>) {
                    return;
                }
            }
        };
        let output = event_processor_impl(input);
        let formatted = pretty_print(output);
        insta::assert_snapshot!(formatted);
    }
    #[test]
    fn test_processor_1() {
        let input = quote! {
            #[event_processor]
            impl EventHandler {
                #[handler(MyStruct)]
                fn test_processor_1(&self, item: String, value: u32) {
                    println!("Hello world!");
                }
            }
        };
        let output = event_processor_impl(input);
        let formatted = pretty_print(output);
        insta::assert_snapshot!(formatted);
    }

    #[test]
    fn test_processor_2() {
        let input = quote! {
            #[event_processor]
            impl EventHandler {
                #[handler(EventNoFields)]
                fn test_processor_2(&self) {
                    println!("Hello world!");
                }
            }
        };
        let output = event_processor_impl(input);
        let formatted = pretty_print(output);
        insta::assert_snapshot!(formatted);
    }

    #[test]
    fn test_processor_3() {
        let input = quote! {
            #[event_processor]
            impl EventHandler {
                #[handler(EventNoFieldsEnv)]
                fn test_processor_3(&self, env: EventEnvelope) {
                    println!("Hello world!");
                }
            }
        };
        let output = event_processor_impl(input);
        let formatted = pretty_print(output);
        insta::assert_snapshot!(formatted);
    }

    #[test]
    fn test_processor_4() {
        let input = quote! {
            #[event_processor]
            impl EventHandler {
                #[handler(EventNoFieldsEnvArc)]
                fn test_processor_4(&self, #[serde(skip)] arc: Option<Arc<Something>>) {
                    println!("Hello world!");
                }
            }
        };
        let output = event_processor_impl(input);
        let formatted = pretty_print(output);
        insta::assert_snapshot!(formatted);
    }
}
