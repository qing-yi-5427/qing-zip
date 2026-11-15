//! Experimental minimal entry for the existing isolated full-archive protocol.
fn main() -> std::process::ExitCode {
    #[cfg(nana_selected_worker)]
    {
        let mut args = std::env::args().skip(1);
        if args.next().as_deref() == Some("--full-archive-worker") && args.next().is_none() {
            return std::process::ExitCode::from(obz_core::engine::selected_worker::run_full_child());
        }
    }
    std::process::ExitCode::from(3)
}
