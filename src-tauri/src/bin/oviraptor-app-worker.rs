//! Optional legacy read adapter. This is not the independent Agent host.
fn main() {
    if let Err(code) = oviraptor_lib::legacy_app_bridge::run_stdio() {
        eprintln!("{code}");
        std::process::exit(1);
    }
}
