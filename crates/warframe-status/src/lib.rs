//! The warframe.market presence socket.
//!
//! Status is not a REST resource: it is set over a WebSocket and holds for as long as the
//! connection does. That is the whole reason this is its own crate. `warframe-market` is
//! request/response over a `MarketTransport` trait, and a persistent connection with a background
//! thread and a reconnect loop is a different lifecycle that would infect its every test.
//!
//! Three statuses can be *set* -- `online`, `ingame`, `invisible`. `offline` is observed-only:
//! going offline means closing the socket, and this crate spells it that way rather than sending a
//! value the server does not accept.
#![forbid(unsafe_code)]

use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const SOCKET_URL: &str = "wss://ws.warframe.market/socket";
/// Connections without this subprotocol are refused by the server.
pub const SUBPROTOCOL: &str = "wfm";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Presence {
    Online,
    Ingame,
    Invisible,
}

impl Presence {
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Ingame => "ingame",
            Self::Invisible => "invisible",
        }
    }

    fn from_wire(value: &str) -> Option<Self> {
        match value {
            "online" => Some(Self::Online),
            "ingame" => Some(Self::Ingame),
            "invisible" => Some(Self::Invisible),
            _ => None,
        }
    }
}

/// The credential, going out over a socket. Built with `serde_json` rather than by formatting:
/// a JWT is not ours to assume anything about the shape of, and one stray quote in it would
/// otherwise become a malformed frame the server answers with a close.
pub fn sign_in_frame(token: &str) -> String {
    serde_json::json!({
        "route": "@wfm|cmd/auth/signIn",
        "payload": { "token": token },
    })
    .to_string()
}

/// `duration` is deliberately omitted: without one the status holds for the life of the
/// connection, which is exactly the claim "this application is running" makes. `activity` is
/// omitted too -- it is rich presence, and nobody asked for it.
pub fn set_status_frame(status: Presence) -> String {
    serde_json::json!({
        "route": "@wfm|cmd/status/set",
        "payload": { "status": status.wire() },
    })
    .to_string()
}

/// The status the server says it committed, from a frame that carries one.
///
/// The server is the source of truth here, and the docs are explicit that the first
/// `event/status/set` after signing in is what tells a client it may start sending. So this reads
/// the committed value rather than the screen echoing back whatever was asked for.
pub fn committed_status(frame: &str) -> Option<Presence> {
    let parsed: serde_json::Value = serde_json::from_str(frame).ok()?;
    if parsed.get("route")?.as_str()? != "@wfm|event/status/set" {
        return None;
    }
    let payload = parsed.get("payload")?;
    // The payload is observed as both the bare string and an object carrying it.
    let value = payload
        .as_str()
        .or_else(|| payload.get("status")?.as_str())?;
    Presence::from_wire(value)
}

/// A presence connection, held for as long as one is wanted.
///
/// The token is not stored: it is read from the caller's own credential store at connect time and
/// handed over per connection, matching the rule the rest of this application follows of reading a
/// credential at the moment it is used.
pub struct StatusLink {
    commands: Sender<Command>,
    committed: Arc<Mutex<Option<Presence>>>,
}

enum Command {
    Set(Presence),
    Close,
}

impl StatusLink {
    /// Open the socket and hold the given status on it.
    ///
    /// Returns immediately; the connection is made on its own thread, and `committed` stays `None`
    /// until the server has said what it committed.
    pub fn connect(token: String, initial: Presence) -> Self {
        let (commands, inbox) = channel();
        let committed = Arc::new(Mutex::new(None));
        let held = Arc::clone(&committed);
        std::thread::spawn(move || run(&token, initial, &inbox, &held));
        Self {
            commands,
            committed,
        }
    }

    /// What the server last said the status is, as opposed to what was asked for.
    pub fn committed(&self) -> Option<Presence> {
        *self.committed.lock().expect("status mutex poisoned")
    }

    pub fn set(&self, status: Presence) {
        let _ = self.commands.send(Command::Set(status));
    }
}

impl Drop for StatusLink {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Close);
    }
}

/// Connect, sign in, then pump commands until told to stop, reconnecting with backoff in between.
///
/// ponytail: fixed backoff doubling to a minute, no jitter. This is one connection per client, not
/// a fleet; add jitter if warframe.market ever asks for it.
fn run(
    token: &str,
    initial: Presence,
    inbox: &Receiver<Command>,
    committed: &Mutex<Option<Presence>>,
) {
    // Held across sessions rather than inside one: a status set just before the connection dropped
    // is still what the player asked for, and the reconnect should arrive already holding it.
    let mut wanted = initial;
    let mut backoff = Duration::from_secs(1);
    loop {
        // A clean exit is the caller dropping the link. Nothing to reconnect to.
        if session(token, &mut wanted, inbox, committed).is_ok() {
            return;
        }
        *committed.lock().expect("status mutex poisoned") = None;
        // A command arriving during the wait is not lost: it stays in the channel and the next
        // session reads it, so a status set while offline takes effect on reconnect.
        std::thread::sleep(backoff);
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

/// One connection, from handshake to close. `Ok` means the caller asked to stop; `Err` means the
/// connection failed or dropped and is worth retrying.
fn session(
    token: &str,
    wanted: &mut Presence,
    inbox: &Receiver<Command>,
    committed: &Mutex<Option<Presence>>,
) -> Result<(), ()> {
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
        .map_err(|_| ())?;
    let (mut socket, _) = tungstenite::connect(request).map_err(|_| ())?;
    // Reads would otherwise block until the server says something, which can be minutes -- and a
    // close asked for in the meantime would sit unsent that whole time. A one-second read timeout
    // turns the read into a poll the command channel gets a turn between.
    if let tungstenite::stream::MaybeTlsStream::Rustls(stream) = socket.get_ref() {
        let _ = stream.sock.set_read_timeout(Some(Duration::from_secs(1)));
    }
    socket
        .send(tungstenite::Message::Text(sign_in_frame(token).into()))
        .map_err(|_| ())?;

    let mut signed_in = false;
    loop {
        match inbox.try_recv() {
            Ok(Command::Close) => {
                let _ = socket.close(None);
                return Ok(());
            }
            Ok(Command::Set(next)) => {
                *wanted = next;
                if signed_in {
                    socket
                        .send(tungstenite::Message::Text(set_status_frame(next).into()))
                        .map_err(|_| ())?;
                }
            }
            // The sender lives in `StatusLink`, so a disconnected channel is the link dropped.
            Err(TryRecvError::Disconnected) => return Ok(()),
            Err(TryRecvError::Empty) => {}
        }
        let text = match socket.read() {
            Ok(tungstenite::Message::Text(text)) => text,
            Ok(_) => continue,
            // The read timeout firing is the ordinary quiet case, not a dead connection.
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(_) => return Err(()),
        };
        if let Some(status) = committed_status(&text) {
            *committed.lock().expect("status mutex poisoned") = Some(status);
            // The docs say to wait for this event before sending anything. The first one is the
            // go-ahead; asking for what the server already committed would be a wasted frame.
            if !signed_in {
                signed_in = true;
                if status != *wanted {
                    socket
                        .send(tungstenite::Message::Text(set_status_frame(*wanted).into()))
                        .map_err(|_| ())?;
                }
            }
        }
    }
}
