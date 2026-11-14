fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    // Not locked for the whole run: a job's output is copied to stdout and
    // stderr from other threads (exec::tee).
    let code = job_runner::run(&args, &mut std::io::stdout(), &mut std::io::stderr());
    std::process::exit(code);
}
