//! The `vpng ui` command: open the graphical editor.
//!
//! The editor is a local program. It opens a file, starts a server on the
//! loopback interface and opens the window in the browser. The command
//! prints the address, so the editor is usable even where no browser can be
//! launched.

use std::net::{Ipv4Addr, SocketAddr};
use std::process::Command;

use vpng_core::{Error, Limits, Result};
use vpng_ui::{random_token, Server, Session};

use crate::args::Args;

/// Open the graphical editor over the file the command names.
pub fn cmd_ui(args: &Args) -> Result<()> {
    let path = args
        .positional
        .first()
        .ok_or_else(|| Error::invalid("`vpng ui` needs a file to edit; run `vpng help`"))?;
    if args.positional.len() > 1 {
        return Err(Error::invalid(
            "the editor opens one file; give only one path",
        ));
    }

    let port = match args.get("port") {
        Some(text) => text
            .parse::<u16>()
            .map_err(|_| Error::invalid(format!("`{text}` is not a port number")))?,
        None => 0,
    };

    let max_region = match args.get("max-region") {
        Some(text) => {
            let pixels = text
                .parse::<u64>()
                .map_err(|_| Error::invalid(format!("`{text}` is not a number of pixels")))?;
            if pixels == 0 {
                return Err(Error::invalid("--max-region must be at least one pixel"));
            }
            pixels
        }
        None => vpng_ui::DEFAULT_MAX_REGION_PIXELS,
    };

    let mut session = Session::open(std::path::Path::new(path), &Limits::default())?;
    session.set_max_region_pixels(max_region)?;

    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let server = Server::bind(address, session, random_token())?;
    let url = server.url()?;

    println!("vpng: editing {path}");
    println!("vpng: the editor is at {url}");
    println!("vpng: press Ctrl+C to stop it");

    if !args.has("no-browser") {
        if let Err(error) = open_browser(&url) {
            eprintln!("vpng: the browser could not be opened: {error}");
            eprintln!("vpng: open the address above by hand");
        }
    }

    server.serve()
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
