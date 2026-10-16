fn main() {
    if let Err(code) = oviraptor_lib::scanning_service::run_app_stdio() {
        eprintln!("{code}");
        std::process::exit(1);
    }
}
