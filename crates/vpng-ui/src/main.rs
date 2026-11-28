//! `vpng-ui` — the graphical editor's program.
//!
//! The program opens a file, starts the local server and opens the window.
//! It runs until it is stopped.

#![forbid(unsafe_code)]

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use vpng_core::{Error, Limits, Result};
use vpng_ui::{random_token, Server, Session};

/// The usage text.
const USAGE: &str = "\
vpng-ui — the VPNG graphical editor

Usage:
  vpng-ui <file.vpng> [options]

Options:
  --port <n>        The port to listen on. 0 asks the operating system for
                    a free one. The default is 0.
  --max-region <n>  The largest number of pixels one display request may
                    carry. The default is 4194304.
  --no-browser      Do not open a browser; print the address instead.
  --help            Show this text.
  --version         Show the version.

The editor listens on the loopback interface only. It has no password, and
it can write the file it opens, so it is not reachable from the network.
";

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match run(&argv) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("vpng-ui: error: {error}");
            ExitCode::from(error.exit_code() as u8)
        }
    }
}

/// The program's options.
struct Options {
    path: PathBuf,
    port: u16,
    max_region: u64,
    open_browser: bool,
}

/// Run the program.
fn run(argv: &[String]) -> Result<()> {
    let options = parse(argv)?;
    let limits = Limits::default();

    let mut session = Session::open(&options.path, &limits)?;
    session.set_max_region_pixels(options.max_region)?;

    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, options.port));
    let server = Server::bind(address, session, random_token())?;
    let url = server.url()?;

    println!("vpng-ui: editing {}", options.path.display());
    println!("vpng-ui: the editor is at {url}");
    println!("vpng-ui: press Ctrl+C to stop it");

    if options.open_browser {
        if let Err(error) = open_browser(&url) {
            eprintln!("vpng-ui: the browser could not be opened: {error}");
            eprintln!("vpng-ui: open the address above by hand");
        }
    }

    server.serve()
}

/// Parse the command line.
fn parse(argv: &[String]) -> Result<Options> {
    let mut path: Option<PathBuf> = None;
    let mut port: u16 = 0;
    let mut max_region = vpng_ui::DEFAULT_MAX_REGION_PIXELS;
    let mut open_browser = true;

    let mut index = 0;
    while index < argv.len() {
        let argument = argv[index].as_str();
        match argument {
            "--help" | "-h" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--version" | "-V" => {
                println!("vpng-ui {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--no-browser" => open_browser = false,
            "--port" => {
                index += 1;
                let value = argv
                    .get(index)
                    .ok_or_else(|| Error::invalid("--port needs a number"))?;
                port = value
                    .parse::<u16>()
                    .map_err(|_| Error::invalid(format!("`{value}` is not a port number")))?;
            }
            "--max-region" => {
                index += 1;
                let value = argv
                    .get(index)
                    .ok_or_else(|| Error::invalid("--max-region needs a number"))?;
                max_region = value
                    .parse::<u64>()
                    .map_err(|_| Error::invalid(format!("`{value}` is not a number of pixels")))?;
                if max_region == 0 {
                    return Err(Error::invalid("--max-region must be at least one pixel"));
                }
            }
            other if other.starts_with("--") => {
                return Err(Error::invalid(format!("unknown option `{other}`")));
            }
            other => {
                if path.is_some() {
                    return Err(Error::invalid(
                        "the editor opens one file; give only one path",
                    ));
                }
                path = Some(PathBuf::from(other));
            }
        }
        index += 1;
    }

    let path = path.ok_or_else(|| {
        Error::invalid("the editor needs a file; run `vpng-ui --help` for the usage")
    })?;

    Ok(Options {
        path,
        port,
        max_region,
        open_browser,
    })
}

/// Open the system's default browser at an address.
///
/// The address is passed to the program directly, never through a shell, so
/// a file name with a space or an ampersand in it cannot change what the
/// command does.
fn open_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        Command::new("rundll32")
            .arg("url.dll,FileProtocolHandler")
            .arg(url)
            .spawn()
            .map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(url).spawn().map(|_| ())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Command::new("xdg-open").arg(url).spawn().map(|_| ())
    }
}
