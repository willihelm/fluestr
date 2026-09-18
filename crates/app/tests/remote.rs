//! The LAN remote client, served and talked to (P16).
//!
//! What this pins down is the one claim the package rests on: a browser peer
//! reaches the *same* command layer the app's own windows do, over a socket,
//! and hears the same events. Without it the two views could drift apart
//! silently — the desktop would go on working, and only the remote would be
//! missing something.
//!
//! Against a mock Tauri app rather than a running window, because a window
//! needs a screen and an event loop, and none of what is tested here is about
//! either. The mock has no frontend embedded, so the page itself is not served
//! and is not what this checks; the socket is.

use std::time::Duration;

use app_lib::remote::{self, RemoteConfig};
use futures_util::{SinkExt, StreamExt};
use tauri::Manager;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

/// A port high enough to need no privileges and odd enough not to be somebody
/// else's. A clash is reported as a bind error rather than as a mysterious
/// failure to connect, because `apply` answers with it.
const PORT: u16 = 18787;

/// Builds an app with nothing in it but the state the commands work on.
fn mock() -> tauri::App<tauri::test::MockRuntime> {
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("a mock app");
    app.manage(app_lib::AppState::default());
    app
}

#[tokio::test]
async fn a_browser_reaches_the_same_commands_over_the_socket() {
    let app = mock();
    let handle = app.handle().clone();

    let status = remote::apply(
        &handle,
        &RemoteConfig {
            enabled: true,
            address: "127.0.0.1".into(),
            port: PORT,
        },
    );
    assert!(status.running, "not serving: {:?}", status.error);
    assert_eq!(status.url.as_deref(), Some("http://127.0.0.1:18787/"));

    let (mut socket, _) = connect().await;

    // A command with no arguments, answered with the value the app's own
    // windows get from the same call.
    socket
        .send(Message::Text(r#"{"id":1,"command":"app_version"}"#.into()))
        .await
        .expect("sending a call");
    let answer = next_result(&mut socket).await;
    assert_eq!(answer["id"], 1);
    assert_eq!(answer["ok"], true);
    assert_eq!(answer["value"], env!("CARGO_PKG_VERSION"));

    // A command that fails is an answer, not a dropped frame: a button on the
    // remote that does nothing for no stated reason is the failure mode the
    // whole transport has to avoid.
    socket
        .send(Message::Text(
            r#"{"id":2,"command":"download_model","args":{"id":"no-such-model"}}"#.into(),
        ))
        .await
        .expect("sending a call");
    let answer = next_result(&mut socket).await;
    assert_eq!(answer["id"], 2);
    assert_eq!(answer["ok"], false);
    assert!(
        answer["error"]
            .as_str()
            .unwrap_or_default()
            .contains("no-such-model"),
        "unhelpful error: {answer}"
    );

    remote::apply(&handle, &RemoteConfig::default());
}

/// Events reach the browser the same way they reach a window: the app emits
/// once, and every view is given the same payload — which is what lets a
/// remote peer share one chat with the operator instead of keeping its own.
///
/// Through a real command rather than by publishing a frame by hand, because
/// that is the round trip that matters: the browser asks for something, the
/// one command layer does it, and the consequence comes back as an event to
/// everybody including the browser that asked.
#[tokio::test]
async fn an_event_reaches_the_browser_that_caused_it() {
    let app = mock();
    let handle = app.handle().clone();

    let status = remote::apply(
        &handle,
        &RemoteConfig {
            enabled: true,
            address: "127.0.0.1".into(),
            // A port of its own: the tests in this file run at the same time,
            // and one listener per address and port is all there is.
            port: PORT + 1,
        },
    );
    assert!(status.running, "not serving: {:?}", status.error);

    let (mut socket, _) = connect_to(PORT + 1).await;

    socket
        .send(Message::Text(
            r#"{"id":1,"command":"clear_communication"}"#.into(),
        ))
        .await
        .expect("sending a call");

    // Both, in whichever order they come. The answer is written by the task
    // that ran the command and the event by the one draining the broadcast, so
    // nothing promises which reaches the wire first — and a client that
    // depended on it would be depending on a race. There is other traffic in
    // between as well: this connection's own arrival is announced to every
    // client, itself included.
    let mut answered = false;
    let mut cleared = false;
    while !(answered && cleared) {
        let frame = next_frame(&mut socket, Duration::from_secs(5)).await;
        match frame["type"].as_str() {
            Some("result") => {
                assert_eq!(frame["ok"], true, "the command failed: {frame}");
                answered = true;
            }
            Some("event") if frame["event"] == "communication-cleared" => cleared = true,
            _ => {}
        }
    }

    remote::apply(&handle, &RemoteConfig::default());
}

/// Switching the remote off drops what it was serving, immediately.
///
/// Part of the acceptance for P16, and not a detail: the listener is the only
/// thing standing between "a browser on the house network can drive this
/// session" and "anyone who can reach this address can". An operator who turns
/// it off has to be turning it off for the browsers that are already
/// connected, not only for the next one.
#[tokio::test]
async fn switching_the_remote_off_drops_the_browsers_it_was_serving() {
    let app = mock();
    let handle = app.handle().clone();

    let status = remote::apply(
        &handle,
        &RemoteConfig {
            enabled: true,
            address: "127.0.0.1".into(),
            port: PORT + 2,
        },
    );
    assert!(status.running, "not serving: {:?}", status.error);

    let (mut socket, _) = connect_to(PORT + 2).await;
    // Something over it first, so that what follows is a connection being
    // dropped rather than one that was never really up.
    socket
        .send(Message::Text(r#"{"id":1,"command":"app_version"}"#.into()))
        .await
        .expect("sending a call");
    assert_eq!(next_result(&mut socket).await["ok"], true);

    let status = remote::apply(&handle, &RemoteConfig::default());
    assert!(!status.running);

    // The socket ends. Not "stops carrying events" — ends, so the page knows
    // it is looking at a picture rather than at a chat.
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(message)) = socket.next().await {
            if matches!(message, Message::Close(_)) {
                return true;
            }
        }
        true
    })
    .await;
    assert!(closed.unwrap_or(false), "the connection stayed open");
}

/// A WebSocket upgrade is not subject to the same-origin policy, so without a
/// check on it any page anybody on this network opens could drive the session
/// out of the operator's own browser. That is a different thing from the
/// recorded "no authentication yet" decision, and this is what keeps the two
/// apart: a browser that states another site's origin is refused before the
/// socket exists.
#[tokio::test]
async fn a_page_from_another_site_cannot_open_the_socket() {
    let app = mock();
    let handle = app.handle().clone();

    let port = PORT + 4;
    let status = remote::apply(
        &handle,
        &RemoteConfig {
            enabled: true,
            address: "127.0.0.1".into(),
            port,
        },
    );
    assert!(status.running, "not serving: {:?}", status.error);

    // Up first, so that what follows is a refusal rather than a listener that
    // was not accepting yet.
    let (mut accepted, _) = connect_to(port).await;

    let mut request = format!("ws://127.0.0.1:{port}/ws")
        .into_client_request()
        .expect("a request");
    request
        .headers_mut()
        .insert("origin", "http://evil.example".parse().expect("an origin"));
    let refused = tokio_tungstenite::connect_async(request).await;
    assert!(
        refused.is_err(),
        "a page from another site was allowed to connect"
    );

    // And the page this app serves is not caught by it: its origin is the
    // address it was served on, which is what the browser sends.
    let mut request = format!("ws://127.0.0.1:{port}/ws")
        .into_client_request()
        .expect("a request");
    request.headers_mut().insert(
        "origin",
        format!("http://127.0.0.1:{port}")
            .parse()
            .expect("an origin"),
    );
    tokio_tungstenite::connect_async(request)
        .await
        .expect("the page this app serves was refused its own socket");

    let _ = accepted.close(None).await;
    remote::apply(&handle, &RemoteConfig::default());
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect() -> (
    Socket,
    tokio_tungstenite::tungstenite::http::Response<Option<Vec<u8>>>,
) {
    connect_to(PORT).await
}

async fn connect_to(
    port: u16,
) -> (
    Socket,
    tokio_tungstenite::tungstenite::http::Response<Option<Vec<u8>>>,
) {
    // The listener is brought up on Tauri's runtime, which this test does not
    // share, so the first attempt can legitimately land before it is
    // accepting.
    for _ in 0..50 {
        match tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws")).await {
            Ok(connected) => return connected,
            Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    }
    panic!("the socket never accepted a connection");
}

async fn next_result(socket: &mut Socket) -> serde_json::Value {
    loop {
        let frame = next_frame(socket, Duration::from_secs(5)).await;
        if frame["type"] == "result" {
            return frame;
        }
    }
}

/// The next JSON frame on the socket, whatever it is.
///
/// Whatever it is, because events and results share the socket and the app has
/// its own things to say on it — a client's arrival is announced to every
/// client, this one included. Filtering is the caller's, so that a test can be
/// explicit about what it is prepared to see past.
async fn next_frame(socket: &mut Socket, within: Duration) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let message = tokio::time::timeout_at(deadline, socket.next())
            .await
            .expect("nothing arrived")
            .expect("the socket closed")
            .expect("a frame");
        if let Message::Text(text) = message {
            return serde_json::from_str(&text).expect("a JSON frame");
        }
    }
}
