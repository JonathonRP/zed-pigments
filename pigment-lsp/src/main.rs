#[tokio::main]
async fn main() {
    if std::env::args()
        .map(|s| s.to_lowercase())
        .any(|arg| arg == "-v" || arg == "--version")
    {
        println!("pigment-lsp v{}", env!("CARGO_PKG_VERSION"));
        return;
    } else if std::env::args()
        .map(|s| s.to_lowercase())
        .any(|arg| arg == "-h" || arg == "--help")
    {
        println!("Usage: pigment-lsp [options]");
        println!("Options:");
        println!("  -v, --version    Print version information");
        println!("  -h, --help       Print this help message");
        return;
    }

    pigment_lsp::lsp::start().await;
}
