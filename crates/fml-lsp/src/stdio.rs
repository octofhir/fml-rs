//! The base protocol over standard input and output: `Content-Length` headers and JSON bodies.

use std::io::{self, BufRead, Write};

use serde_json::Value;

use crate::Server;

/// Runs `server` over standard input and output until the client sends `exit`, and returns the
/// exit code.
pub fn run_stdio(server: Server) -> io::Result<i32> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    run(server, &mut stdin.lock(), &mut stdout.lock())
}

/// Runs `server` over any reader and writer.
pub fn run(
    mut server: Server,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<i32> {
    while let Some(body) = read_message(input)? {
        let Ok(message) = serde_json::from_slice::<Value>(&body) else {
            continue;
        };
        for reply in server.handle(&message) {
            write_message(output, &reply)?;
        }
        if server.exited() {
            return Ok(server.exit_code());
        }
    }
    // The client went away without `exit`.
    Ok(1)
}

fn read_message(input: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut length = None;
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let header = line.trim_end();
        if header.is_empty() {
            if length.is_some() {
                break;
            }
            continue;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0; length.unwrap_or(0)];
    input.read_exact(&mut body)?;
    Ok(Some(body))
}

fn write_message(output: &mut impl Write, message: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(message)?;
    write!(output, "Content-Length: {}\r\n\r\n", body.len())?;
    output.write_all(&body)?;
    output.flush()
}
