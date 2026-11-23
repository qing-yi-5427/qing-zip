//! Experimental minimal entry for the existing isolated full-archive protocol.
fn main() -> std::process::ExitCode {
    #[cfg(nana_selected_worker)]
    {
        let mut args = std::env::args().skip(1);
        let mode = args.next();
        if args.next().is_none() {
            match mode.as_deref() {
                Some("--full-archive-worker") => return std::process::ExitCode::from(obz_core::engine::selected_worker::run_full_child()),
                Some("--internal-create-worker") => return std::process::ExitCode::from(obz_core::engine::selected_worker::create_zip::helper_worker()),
                Some("--internal-zip-append-worker") => return std::process::ExitCode::from(obz_core::engine::selected_worker::zip_append::helper_worker()),
                _ => (),
            }
        }
    }
    std::process::ExitCode::from(3)
}
