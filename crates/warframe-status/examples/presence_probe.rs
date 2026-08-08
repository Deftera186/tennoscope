//! Talk to the live warframe.market presence socket and print every frame, so the exact way the
//! server answers the first, second, third... status set on one connection can be observed
//! first-hand. A bug that only shows against the real server is a bug this tool exists for.
//!
//! It signs in with the credential the app already holds, walks statuses online, invisible,
//! in-game, and online again on one connection, and closes -- so the app must be closed while
//! this runs, and the account reads as offline until the app reopens.
//!
//! ```sh
//! cargo run -p warframe-status --example presence_probe --release
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tungstenite::Message;

use warframe_status::{
    Presence, SOCKET_URL, SUBPROTOCOL, committed_status, set_status_frame, sign_in_frame,
};

/// The app's own data directory, so the probe finds the credential a linked install left behind.
/// This is the Linux path; a `cargo run ... -- <path>` argument overrides it on other systems.
fn default_database() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .expect("a HOME is set");
    home.join(".local/share/io.github.deftera186.tennoscope/tennoscope.sqlite3")
}

fn set_read_timeout(
    socket: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
    timeout: Duration,
) {
    match socket.get_mut() {
        tungstenite::stream::MaybeTlsStream::Plain(stream) => {
            let _ = stream.set_read_timeout(Some(timeout));
        }
        tungstenite::stream::MaybeTlsStream::Rustls(stream) => {
            let _ = stream.sock.set_read_timeout(Some(timeout));
        }
        #[allow(unreachable_patterns)]
        _ => {}
    }
}

/// Print every inbound frame until a committed event for `wanted` arrives, or `deadline` passes.
/// Returns whether the server committed the status before the deadline.
fn probe(
    socket: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
    wanted: Presence,
    deadline: Instant,
) -> bool {
    loop {
        match socket.read() {
            Ok(Message::Text(text)) => {
                let text = text.to_string();
                println!("[in] {text}");
                if committed_status(&text) == Some(wanted) {
                    println!("[verdict] the server committed {wanted:?}");
                    return true;
                }
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => {
                println!("[in] connection error: {error:?}");
                return false;
            }
        }
        if Instant::now() >= deadline {
            println!("[verdict] nothing committed within the window");
            return false;
        }
    }
}

/// The token stays out of cleartext on screen: it is the one secret in the frame, and the probe's
/// value is the server's behaviour, not the token -- so it is printed as its length.
fn mask_token(frame: &str) -> String {
    let mut parsed: serde_json::Value =
        serde_json::from_str(frame).expect("the sign-in frame is valid JSON");
    if let Some(token) = parsed
        .get_mut("payload")
        .and_then(|payload| payload.get_mut("token"))
    {
        let bytes = token.as_str().map_or(0, str::len);
        *token = serde_json::Value::String(format!("<{bytes} bytes>"));
    }
    parsed.to_string()
}

fn main() -> Result<(), String> {
    let database = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_database);
    // The store decides what it can read: the keyring is the primary credible holder and the
    // database only its fallback, so a machine with no database file still has a token.
    let store = warframe_market::open_credential_store(database.clone());
    let token = store
        .load()
        .map_err(|error| format!("the credential could not be read: {error:?}"))?
        .ok_or_else(|| format!("no linked account: no token was found at {database:?}"))?;
    println!("token loaded ({} bytes)", token.expose().len());

    let request = tungstenite::handshake::client::Request::builder()
        .uri(SOCKET_URL)
        .header("Sec-WebSocket-Protocol", SUBPROTOCOL)
        .header("Host", "ws.warframe.market")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tungstenite::handshake::client::generate_key(),
        )
        .body(())
        .map_err(|error| format!("handshake request: {error}"))?;
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (mut socket, _) =
        tungstenite::connect(request).map_err(|error| format!("connect: {error:?}"))?;
    set_read_timeout(&mut socket, Duration::from_millis(250));

    let sign_in = sign_in_frame(token.expose());
    socket
        .send(Message::Text(sign_in.clone().into()))
        .map_err(|error| format!("sign-in send failed: {error:?}"))?;
    println!("[out] {}", mask_token(&sign_in));

    let sign_in_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match socket.read() {
            Ok(Message::Text(text)) => {
                let text = text.to_string();
                println!("[in] {text}");
                if text.contains("@wfm|cmd/auth/signIn:ok") {
                    break;
                }
                if text.contains("@wfm|cmd/auth/signIn:error") {
                    return Err("the server refused the credential".to_owned());
                }
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => return Err(format!("sign-in read failed: {error:?}")),
        }
        if Instant::now() >= sign_in_deadline {
            return Err("the server never answered the sign-in".to_owned());
        }
    }
    println!("signed in; changing status on the same connection and watching for replies...");

    for (round, wanted) in [
        (1, Presence::Online),
        (2, Presence::Invisible),
        (3, Presence::Ingame),
        (4, Presence::Online),
    ] {
        println!("\n--- set #{round}: {wanted:?} ---");
        let frame = set_status_frame(wanted);
        socket
            .send(Message::Text(frame.clone().into()))
            .map_err(|error| format!("send failed: {error:?}"))?;
        println!("[out] {frame}");
        let committed = probe(&mut socket, wanted, Instant::now() + Duration::from_secs(5));
        if !committed {
            println!("(the connection survives for the next round)");
        }
    }

    println!(
        "\ndone; the socket now closes and the account reads as offline until the app reopens."
    );
    Ok(())
}
