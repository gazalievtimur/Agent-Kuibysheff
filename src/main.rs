use std::process::ExitCode;

fn main() -> ExitCode {
    // Must run before Tokio so the Linux sandbox helper stays single-threaded.
    sandbox_linux::try_run_helper();
    agent_Kuibyshev::app::run()
}
