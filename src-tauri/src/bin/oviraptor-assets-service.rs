//! Standalone Assets application. All launch/guardian parsing is inside the
//! narrow library facade so no business environment precedes guardian entry.
fn main() {
    if let Err(error) = oviraptor_lib::assets_service::run_app_stdio() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
