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

/// The route a frame declares, if it is parseable JSON carrying one.
///
/// Every message on this socket names itself by route, so the server's several meanings -- a
/// credential accepted or refused, a status committed, a report pushed -- are told apart by
/// comparing it, and the comparison lives in the helpers rather than being re-typed.
fn route(frame: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(frame)
        .ok()
        .and_then(|parsed| Some(parsed.get("route")?.as_str()?.to_owned()))
}

/// Whether this frame is the server refusing the credential.
///
/// It answers a bad token with an error frame and then leaves the connection open, so nothing
/// about the socket itself says the sign-in failed. A client that only watched for the success
/// event would sit on that connection forever, signed in to nothing.
pub fn is_signin_refusal(frame: &str) -> bool {
    route(frame).is_some_and(|route| route == "@wfm|cmd/auth/signIn:error")
}

/// Whether this frame is the server accepting the credential.
///
/// Acceptance and refusal are both command replies, `signIn:ok` and `signIn:error`. The status
/// event is not a reply to signing in, and a fresh connection is not guaranteed one -- the server
/// sends it when a status is *set*, not when a client *arrives* -- so it cannot be the signal a
/// client waits for before asking for the status it holds.
pub fn is_sign_in_success(frame: &str) -> bool {
    route(frame).is_some_and(|route| route == "@wfm|cmd/auth/signIn:ok")
}

/// The status the server says it committed, from a frame that carries one.
///
/// The server is the source of truth here: this reads the committed value rather than the screen
/// echoing back whatever was asked for. The answer to a set is the command echo `set:ok` with the
/// status it just committed in its payload -- which is what later sets in the same connection
/// receive. The event route arrives only at the start of a connection, announcing the status the
/// server held when the connection opened, so a client that read only the event would confirm the
/// first set and then hang on every later one.
pub fn committed_status(frame: &str) -> Option<Presence> {
    let parsed: serde_json::Value = serde_json::from_str(frame).ok()?;
    if !matches!(
        parsed.get("route")?.as_str()?,
        "@wfm|event/status/set" | "@wfm|cmd/status/set:ok"
    ) {
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
        std::thread::spawn(move || run(SOCKET_URL, &token, initial, &inbox, &held));
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
    url: &str,
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
        // A clean exit is the caller dropping the link, and a refused credential is just as final:
        // reconnecting with the same token that was refused would hammer the server to be told the
        // same thing, and the token only changes by the player linking again, which builds a new
        // link anyway.
        if session(url, token, &mut wanted, inbox, committed).is_ok() {
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
    url: &str,
    token: &str,
    wanted: &mut Presence,
    inbox: &Receiver<Command>,
    committed: &Mutex<Option<Presence>>,
) -> Result<(), ()> {
    let request = tungstenite::handshake::client::Request::builder()
        .uri(url)
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
    // rustls panics on the first connection if no process-wide provider has been chosen, and
    // nothing else in this binary installs one at a point this crate can rely on. Idempotent: a
    // second call returns Err and is ignored, so this is safe from any thread on any reconnect.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (mut socket, _) = tungstenite::connect(request).map_err(|_| ())?;
    // Reads would otherwise block until the server says something, which can be minutes -- and a
    // close asked for in the meantime would sit unsent that whole time. A one-second read timeout
    // turns the read into a poll the command channel gets a turn between.
    match socket.get_mut() {
        tungstenite::stream::MaybeTlsStream::Plain(stream) => {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
        }
        tungstenite::stream::MaybeTlsStream::Rustls(stream) => {
            let _ = stream.sock.set_read_timeout(Some(Duration::from_secs(1)));
        }
        #[allow(unreachable_patterns)]
        _ => {}
    }
    socket
        .send(tungstenite::Message::Text(sign_in_frame(token).into()))
        .map_err(|_| ())?;

    let mut signed_in = false;
    // How many one-second quiet passes have gone by while a status was asked for but never
    // committed. A socket can go half-open without reporting an error: the server's answer is
    // lost, and a read on a connection that is gone but not closed just times out forever.
    let mut unanswered = 0u32;
    loop {
        match inbox.try_recv() {
            Ok(Command::Close) => {
                let _ = socket.close(None);
                return Ok(());
            }
            Ok(Command::Set(next)) => {
                // A fresh ask invalidates any in-flight stall count.
                unanswered = 0;
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
            Ok(tungstenite::Message::Text(text)) => Some(text),
            // A ping, pong, or close is inbound traffic that answers no status request.
            Ok(_) => None,
            // The read timeout firing is the ordinary quiet case, not a dead connection.
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                None
            }
            Err(_) => return Err(()),
        };
        if let Some(text) = text {
            if is_signin_refusal(&text) {
                let _ = socket.close(None);
                return Ok(());
            }
            if is_sign_in_success(&text) {
                // The credential was accepted. Ask for the status this socket holds now, rather
                // than waiting for the server to volunteer one: the status event is the server's
                // answer to a set request, not a greeting a fresh connection is guaranteed, so
                // nothing would ever be sent -- committed a status that was never asked for.
                signed_in = true;
                unanswered = 0;
                if *committed.lock().expect("status mutex poisoned") != Some(*wanted) {
                    socket
                        .send(tungstenite::Message::Text(set_status_frame(*wanted).into()))
                        .map_err(|_| ())?;
                }
                continue;
            }
            if let Some(status) = committed_status(&text) {
                // The ask was answered.
                unanswered = 0;
                *committed.lock().expect("status mutex poisoned") = Some(status);
                // Normally a status event is read after the set request answered it. It can also
                // arrive unasked, when the server announces the account's current status -- in
                // which case asking for what is already held would be a wasted frame.
                if !signed_in {
                    signed_in = true;
                    if status != *wanted {
                        socket
                            .send(tungstenite::Message::Text(set_status_frame(*wanted).into()))
                            .map_err(|_| ())?;
                    }
                }
                continue;
            }
            // Any other frame -- the server's periodic reports, orders, chat -- answers no status
            // ask, so it must not quiet the stall count below: a chatty server would otherwise
            // mask a status ask whose answer was lost.
        }
        // Nothing was recognized this pass, so the server said nothing. A status asked for but
        // not committed within a few quiet passes is a lost answer -- or a dead connection that
        // has not noticed -- and asking again is the difference between settling and hanging on
        // "Asking warframe.market" forever. Re-ask on odd passes, and if it stays silent for
        // several more, trade the connection for a new one: `run` reconnects from the `Err`.
        if signed_in && *committed.lock().expect("status mutex poisoned") != Some(*wanted) {
            unanswered += 1;
            if unanswered >= 5 {
                return Err(());
            }
            if unanswered % 2 == 1 {
                socket
                    .send(tungstenite::Message::Text(set_status_frame(*wanted).into()))
                    .map_err(|_| ())?;
            }
        }
    }
}

#[cfg(test)]
mod handshake_tests {
    use std::net::TcpListener;
    use std::sync::Mutex;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::{Command, Presence, run, set_status_frame, sign_in_frame};

    /// Keep reading the socket until a text frame arrives, or the timeout closes it.
    fn read_text(socket: &mut tungstenite::WebSocket<std::net::TcpStream>) -> Option<String> {
        loop {
            match socket.read() {
                Ok(tungstenite::Message::Text(text)) => return Some(text.to_string()),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    }

    /// Stand in for warframe.market's own socket server, which echoes the `wfm` subprotocol the
    /// client talked during the handshake.
    // The closure's `Err` is a large HTTP error type, a shape tungstenite forces on this callback.
    #[allow(clippy::result_large_err)]
    fn accept_socket(stream: std::net::TcpStream) -> tungstenite::WebSocket<std::net::TcpStream> {
        tungstenite::accept_hdr(
            stream,
            |_request: &tungstenite::handshake::server::Request,
             mut response: tungstenite::handshake::server::Response| {
                let headers = response.headers_mut();
                headers.insert(
                    "Sec-WebSocket-Protocol",
                    "wfm".parse().expect("a valid header value"),
                );
                Ok(response)
            },
        )
        .expect("the server handshakes")
    }

    /// The server does not push an initial status to a connection that just signed in. The link
    /// must not wait for one that may never arrive: it asks for the status it was told to hold as
    /// soon as the credential is accepted.
    #[test]
    fn a_signed_in_link_asks_for_the_status_it_holds() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener binds");
        let address = listener.local_addr().expect("listener has an address");
        let url = format!("ws://{}/", address);

        let (commands, inbox) = std::sync::mpsc::channel();
        let committed = std::sync::Arc::new(Mutex::new(None));

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("the client connects");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("socket timeout is set");
            let mut socket = accept_socket(stream);
            assert_eq!(
                read_text(&mut socket),
                Some(sign_in_frame("test-token")),
                "the client announces its credential"
            );
            // Accept it, and send nothing else. The earlier bug waited for a status event from
            // the server as its go-ahead; a server that never volunteers one left the link
            // authenticated but silent, holding a status that was never sent.
            socket
                .send(tungstenite::Message::Text(
                    r#"{"route":"@wfm|cmd/auth/signIn:ok","payload":{}}"#.into(),
                ))
                .expect("the go-ahead is sent");
            assert_eq!(
                read_text(&mut socket),
                Some(set_status_frame(Presence::Ingame)),
                "accepted clients ask for the status they hold"
            );
            socket
                .send(tungstenite::Message::Text(
                    r#"{"route":"@wfm|event/status/set","payload":"ingame"}"#.into(),
                ))
                .expect("the committed status is echoed");
            // Hold the connection the way the real server does, until the client closes it. A
            // session that ended right after committing would be wiped by the reconnect loop.
            let _ = socket.get_ref().set_read_timeout(None);
            while socket.read().is_ok() {}
        });

        let client_committed = std::sync::Arc::clone(&committed);
        let client = thread::spawn(move || {
            run(
                &url,
                "test-token",
                Presence::Ingame,
                &inbox,
                &client_committed,
            )
        });

        let deadline = Instant::now() + Duration::from_secs(2);
        while *committed.lock().expect("committed status") != Some(Presence::Ingame) {
            assert!(
                Instant::now() < deadline,
                "the asked-for status was never committed; the link never asked for it"
            );
            thread::sleep(Duration::from_millis(10));
        }

        let _ = commands.send(Command::Close);
        client.join().expect("the client thread finishes");
        server.join().expect("the server thread finishes");
    }

    /// The switch screen: follow the game, then flip it to manual by holding the same status, then
    /// hand-pick another one. Each press is a fresh ask on the same connection, and each must land.
    #[test]
    fn a_link_settles_on_the_status_hand_picked_after_following() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener binds");
        let address = listener.local_addr().expect("listener has an address");
        let url = format!("ws://{}/", address);

        let (commands, inbox) = std::sync::mpsc::channel();
        let committed = std::sync::Arc::new(Mutex::new(None));

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("the client connects");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("socket timeout is set");
            let mut socket = accept_socket(stream);
            assert_eq!(
                read_text(&mut socket),
                Some(sign_in_frame("test-token")),
                "the client announces its credential"
            );
            socket
                .send(tungstenite::Message::Text(
                    r#"{"route":"@wfm|cmd/auth/signIn:ok","payload":{}}"#.into(),
                ))
                .expect("the go-ahead is sent");
            assert_eq!(
                read_text(&mut socket),
                Some(set_status_frame(Presence::Ingame)),
                "accepted clients ask for the status they hold"
            );
            socket
                .send(tungstenite::Message::Text(
                    r#"{"route":"@wfm|event/status/set","payload":"ingame"}"#.into(),
                ))
                .expect("the committed status is echoed");
            // The uncheck: the same status is asked for again, this time as a hand-picked one.
            assert_eq!(
                read_text(&mut socket),
                Some(set_status_frame(Presence::Ingame)),
                "flipping to manual re-asks for the status being held"
            );
            socket
                .send(tungstenite::Message::Text(
                    r#"{"route":"@wfm|event/status/set","payload":"ingame"}"#.into(),
                ))
                .expect("the committed status is echoed");
            // The hand-pick: a new status is asked for and must be committed.
            assert_eq!(
                read_text(&mut socket),
                Some(set_status_frame(Presence::Online)),
                "the hand-picked status is asked for"
            );
            socket
                .send(tungstenite::Message::Text(
                    r#"{"route":"@wfm|event/status/set","payload":"online"}"#.into(),
                ))
                .expect("the committed status is echoed");
            let _ = socket.get_ref().set_read_timeout(None);
            while socket.read().is_ok() {}
        });

        let client_committed = std::sync::Arc::clone(&committed);
        let client = thread::spawn(move || {
            run(
                &url,
                "test-token",
                Presence::Ingame,
                &inbox,
                &client_committed,
            )
        });

        let deadline = Instant::now() + Duration::from_secs(2);
        while *committed.lock().expect("committed status") != Some(Presence::Ingame) {
            assert!(
                Instant::now() < deadline,
                "the held status was never committed"
            );
            thread::sleep(Duration::from_millis(10));
        }

        let _ = commands.send(Command::Set(Presence::Ingame));
        thread::sleep(Duration::from_millis(100));
        let _ = commands.send(Command::Set(Presence::Online));

        let deadline = Instant::now() + Duration::from_secs(2);
        while *committed.lock().expect("committed status") != Some(Presence::Online) {
            assert!(
                Instant::now() < deadline,
                "the hand-picked status was never committed"
            );
            thread::sleep(Duration::from_millis(10));
        }

        let _ = commands.send(Command::Close);
        client.join().expect("the client thread finishes");
        server.join().expect("the server thread finishes");
    }

    /// Replays the live transcript captured against the real server: the first set is answered
    /// with the event (the status held when the connection opened, then the new one) and every
    /// later set with the command echo alone. Reading only the event commits the first status by
    /// luck and then never moves -- the switch must read the echo to settle at all.
    #[test]
    fn a_late_switch_reads_the_committed_status_from_the_command_echo() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener binds");
        let address = listener.local_addr().expect("listener has an address");
        let url = format!("ws://{}/", address);

        let (commands, inbox) = std::sync::mpsc::channel();
        let committed = std::sync::Arc::new(Mutex::new(None));

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("the client connects");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("socket timeout is set");
            let mut socket = accept_socket(stream);
            assert_eq!(
                read_text(&mut socket),
                Some(sign_in_frame("test-token")),
                "the client announces its credential"
            );
            socket
                .send(tungstenite::Message::Text(
                    r#"{"route":"@wfm|cmd/auth/signIn:ok","payload":{}}"#.into(),
                ))
                .expect("the go-ahead is sent");
            assert_eq!(
                read_text(&mut socket),
                Some(set_status_frame(Presence::Ingame)),
                "the first set goes out"
            );
            // The real server answers the first set with an event carrying the status held at
            // connect time, then the echo of what it committed.
            socket
                .send(tungstenite::Message::Text(
                    r#"{"route":"@wfm|event/status/set","payload":{"status":"online"}}"#.into(),
                ))
                .expect("the opening event is sent");
            socket
                .send(tungstenite::Message::Text(
                    r#"{"route":"@wfm|cmd/status/set:ok","payload":{"status":"ingame"}}"#.into(),
                ))
                .expect("the echo is sent");
            // The later switch is answered by the echo alone.
            assert_eq!(
                read_text(&mut socket),
                Some(set_status_frame(Presence::Invisible)),
                "the switch goes out"
            );
            socket
                .send(tungstenite::Message::Text(
                    r#"{"route":"@wfm|cmd/status/set:ok","payload":{"status":"invisible"}}"#.into(),
                ))
                .expect("the echo is sent");
            let _ = socket.get_ref().set_read_timeout(None);
            while socket.read().is_ok() {}
        });

        let client_committed = std::sync::Arc::clone(&committed);
        let client = thread::spawn(move || {
            run(
                &url,
                "test-token",
                Presence::Ingame,
                &inbox,
                &client_committed,
            )
        });

        let deadline = Instant::now() + Duration::from_secs(2);
        while *committed.lock().expect("committed status") != Some(Presence::Ingame) {
            assert!(
                Instant::now() < deadline,
                "the opening status was never committed"
            );
            thread::sleep(Duration::from_millis(10));
        }

        let _ = commands.send(Command::Set(Presence::Invisible));

        let deadline = Instant::now() + Duration::from_secs(2);
        while *committed.lock().expect("committed status") != Some(Presence::Invisible) {
            assert!(
                Instant::now() < deadline,
                "the switch was never committed: the echo was not read"
            );
            thread::sleep(Duration::from_millis(10));
        }

        let _ = commands.send(Command::Close);
        client.join().expect("the client thread finishes");
        server.join().expect("the server thread finishes");
    }

    /// The manual pick after following, with the server answering the first status but dropping
    /// the answer to the second. A lost echo leaves the connection half-open: reads on it time
    /// out forever rather than reporting an error, so the link must re-ask on its own and, when
    /// the answer stays lost, trade the connection for a fresh one that signs in again.
    #[test]
    fn a_dropped_status_answer_is_recovered_from_on_a_fresh_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener binds");
        let address = listener.local_addr().expect("listener has an address");
        let url = format!("ws://{}/", address);

        let (commands, inbox) = std::sync::mpsc::channel();
        let committed = std::sync::Arc::new(Mutex::new(None));

        let server = thread::spawn(move || {
            {
                let (stream, _) = listener.accept().expect("the client connects");
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("socket timeout is set");
                let mut socket = accept_socket(stream);
                assert_eq!(
                    read_text(&mut socket),
                    Some(sign_in_frame("test-token")),
                    "the first connection announces its credential"
                );
                socket
                    .send(tungstenite::Message::Text(
                        r#"{"route":"@wfm|cmd/auth/signIn:ok","payload":{}}"#.into(),
                    ))
                    .expect("the go-ahead is sent");
                assert_eq!(
                    read_text(&mut socket),
                    Some(set_status_frame(Presence::Ingame)),
                    "the first connection holds the wanted status"
                );
                socket
                    .send(tungstenite::Message::Text(
                        r#"{"route":"@wfm|event/status/set","payload":"ingame"}"#.into(),
                    ))
                    .expect("the committed status is echoed");
                // The manual pick, answer dropped on purpose. The real server keeps pushing its
                // periodic report frames while it stays connected, and those must not quiet the
                // stall count: the ask is still unanswered.
                assert_eq!(
                    read_text(&mut socket),
                    Some(set_status_frame(Presence::Online)),
                    "the switch asks for the new status"
                );
                socket
                    .get_ref()
                    .set_read_timeout(Some(Duration::from_millis(250)))
                    .expect("socket timeout is set");
                let reports_deadline = Instant::now() + Duration::from_secs(10);
                while Instant::now() < reports_deadline && socket.read().is_ok() {
                    socket
                        .send(tungstenite::Message::Text(
                            r#"{"route":"@wfm|event/reports/online","payload":{"connections":1}}"#
                                .into(),
                        ))
                        .expect("the report is pushed");
                }
            }
            let (stream, _) = listener.accept().expect("the client reconnects");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("socket timeout is set");
            let mut socket = accept_socket(stream);
            assert_eq!(
                read_text(&mut socket),
                Some(sign_in_frame("test-token")),
                "the fresh connection announces its credential"
            );
            socket
                .send(tungstenite::Message::Text(
                    r#"{"route":"@wfm|cmd/auth/signIn:ok","payload":{}}"#.into(),
                ))
                .expect("the go-ahead is sent");
            assert_eq!(
                read_text(&mut socket),
                Some(set_status_frame(Presence::Online)),
                "the fresh connection re-asks for the status it holds"
            );
            socket
                .send(tungstenite::Message::Text(
                    r#"{"route":"@wfm|event/status/set","payload":"online"}"#.into(),
                ))
                .expect("the committed status is echoed");
            let _ = socket.get_ref().set_read_timeout(None);
            while socket.read().is_ok() {}
        });

        let client_committed = std::sync::Arc::clone(&committed);
        let client = thread::spawn(move || {
            run(
                &url,
                "test-token",
                Presence::Ingame,
                &inbox,
                &client_committed,
            )
        });

        let deadline = Instant::now() + Duration::from_secs(2);
        while *committed.lock().expect("committed status") != Some(Presence::Ingame) {
            assert!(
                Instant::now() < deadline,
                "the held status was never committed"
            );
            thread::sleep(Duration::from_millis(10));
        }

        let _ = commands.send(Command::Set(Presence::Online));

        // The answer is dropped, so this has to survive re-asking and a reconnect.
        let deadline = Instant::now() + Duration::from_secs(15);
        while *committed.lock().expect("committed status") != Some(Presence::Online) {
            assert!(
                Instant::now() < deadline,
                "the hand-picked status was never committed"
            );
            thread::sleep(Duration::from_millis(10));
        }

        let _ = commands.send(Command::Close);
        client.join().expect("the client thread finishes");
        server.join().expect("the server thread finishes");
    }
}
