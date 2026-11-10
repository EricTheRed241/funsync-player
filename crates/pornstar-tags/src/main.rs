fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let code = pornstar_tags::run(
        &args,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    );
    std::process::exit(code);
}
