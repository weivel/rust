//! `weivel-host`, the host program's command line.

fn main() -> std::process::ExitCode {
    let exit = weivel_rust::program::main(std::env::args_os().skip(1).collect());
    std::process::ExitCode::from(weivel_rust::program::process_exit_code(exit))
}
