fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    // Not locked for the whole run: the run takes minutes and other output
    // (a panic message) must still get through.
    let code = library_thumbs::run(&args, &mut std::io::stdout(), &mut std::io::stderr());
    std::process::exit(code);
}
