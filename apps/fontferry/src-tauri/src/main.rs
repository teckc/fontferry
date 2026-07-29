fn main() {
    if let Err(error) = fontferry_app::run() {
        eprintln!("FontFerry: {error:#}");
        std::process::exit(1);
    }
}
