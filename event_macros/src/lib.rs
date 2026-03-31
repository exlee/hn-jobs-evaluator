use proc_macro::TokenStream;
mod event;
mod service;
#[proc_macro_attribute]
pub fn event_processor(_args: TokenStream, input: TokenStream) -> TokenStream {
    event::event_processor_impl(input.into()).into()
}

#[proc_macro_attribute]
pub fn service_handler(args: TokenStream, input: TokenStream) -> TokenStream {
    service::service_handler_impl(args.into(), input.into()).into()
}
