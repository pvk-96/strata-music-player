fn main() {
    // GTK insists on initialising itself on the main thread.
    let status = strata::app::run::main_with_args(std::env::args().collect());
    if status != 0 {
        std::process::exit(status);
    }
}
