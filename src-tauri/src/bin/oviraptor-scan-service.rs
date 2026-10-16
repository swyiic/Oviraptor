fn main() {
    if let Err(code) = oviraptor_lib::scanning_service::run_stdio() {
        eprintln!("{code}");
        std::process::exit(1);
    }
}
