use std::process::ExitCode;

use funsync_server::cli::{self, Command};
use funsync_server::{autostart, import, server};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match cli::parse(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("funsync-server: {e}\n\n{}", cli::USAGE);
            return ExitCode::from(2);
        }
    };
    let res = match cmd {
        Command::Help => {
            println!("{}", cli::USAGE);
            Ok(())
        }
        Command::Import {
            from,
            data_dir,
            force,
        } => import::import(&from, &data_dir, force).map(|s| {
            println!(
                "Imported from {} into {}: {} sources, {} playlists, {} categories, {} ratings",
                from.display(),
                data_dir.display(),
                s.sources,
                s.playlists,
                s.categories,
                s.ratings
            );
        }),
        Command::Serve {
            addr,
            data_dir,
            supervise,
        } => server::AppState::load(data_dir).and_then(|st| {
            tokio::runtime::Runtime::new()
                .map_err(|e| e.to_string())?
                .block_on(server::serve(st, addr, supervise))
        }),
        Command::Autostart { action, serve_args } => autostart::run(&action, &serve_args),
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("funsync-server: {e}");
            ExitCode::from(1)
        }
    }
}
