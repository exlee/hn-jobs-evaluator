#[tokio::main]
async fn main() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    let comments = jobs::api::firebase::get_comments(46857488, true).await;
    for c in comments {
        dbg!(c);
    }
}
