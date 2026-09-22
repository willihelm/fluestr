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
            key: String::new(),
        },
    )
    .await;
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

    remote::apply(&handle, &RemoteConfig::default()).await;
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
            key: String::new(),
        },
    )
    .await;
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

    remote::apply(&handle, &RemoteConfig::default()).await;
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
            key: String::new(),
        },
    )
    .await;
    assert!(status.running, "not serving: {:?}", status.error);

    let (mut socket, _) = connect_to(PORT + 2).await;
    // Something over it first, so that what follows is a connection being
    // dropped rather than one that was never really up.
    socket
        .send(Message::Text(r#"{"id":1,"command":"app_version"}"#.into()))
        .await
        .expect("sending a call");
    assert_eq!(next_result(&mut socket).await["ok"], true);

    let status = remote::apply(&handle, &RemoteConfig::default()).await;
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
            key: String::new(),
        },
    )
    .await;
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
    remote::apply(&handle, &RemoteConfig::default()).await;
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

/// The optional key, end to end: refused without it, let in with it, and the
/// address the operator scans carries it.
///
/// The whole of the ceremony is meant to be scanning a QR code, so what is
/// pinned here is that the key really is in the URL, that the page can trade
/// it for a cookie at `/auth`, and that the cookie — and nothing else — is
/// what opens the socket.
#[tokio::test]
async fn a_key_stands_in_front_of_the_socket() {
    const KEY: &str = "k7p4m2rq";

    let app = mock();
    let handle = app.handle().clone();

    let port = PORT + 5;
    let status = remote::apply(
        &handle,
        &RemoteConfig {
            enabled: true,
            address: "127.0.0.1".into(),
            port,
            key: KEY.into(),
        },
    )
    .await;
    assert!(status.running, "not serving: {:?}", status.error);
    assert_eq!(
        status.url.as_deref(),
        Some(format!("http://127.0.0.1:{port}/#k={KEY}").as_str()),
        "the key has to be in the address the operator scans"
    );

    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();
    serving(&client, &base).await;

    // Nothing to go on: the socket is not opened.
    assert!(
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws"))
            .await
            .is_err(),
        "the socket was opened without the key"
    );

    // Which is what the page asks about before it tries, because a refused
    // upgrade tells a browser nothing it could act on.
    let state = get(&client, &format!("{base}/auth")).await;
    assert_eq!(state["required"], true);
    assert_eq!(state["unlocked"], false);

    let refused = client
        .post(format!("{base}/auth"))
        .body("not the key")
        .send()
        .await
        .expect("offering a key");
    assert_eq!(refused.status().as_u16(), 403);

    let accepted = client
        .post(format!("{base}/auth"))
        .body(KEY)
        .send()
        .await
        .expect("offering the key");
    assert_eq!(accepted.status().as_u16(), 204);

    let cookie = accepted
        .headers()
        .get("set-cookie")
        .expect("a cookie was set")
        .to_str()
        .expect("a readable cookie")
        .split(';')
        .next()
        .expect("a name and a value")
        .to_string();
    // Derived from the key rather than being it, so a jar that is read does
    // not give the key up.
    assert!(
        !cookie.contains(KEY),
        "the key itself was put in the cookie: {cookie}"
    );

    let mut request = format!("ws://127.0.0.1:{port}/ws")
        .into_client_request()
        .expect("a request");
    request
        .headers_mut()
        .insert("cookie", cookie.parse().expect("a cookie header"));
    let (mut socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("the key was offered and the socket still refused");

    let _ = socket.close(None).await;
    remote::apply(&handle, &RemoteConfig::default()).await;
}

/// Waits until the listener answers at all. It comes up on Tauri's runtime,
/// which this test does not share.
async fn serving(client: &reqwest::Client, base: &str) {
    for _ in 0..50 {
        if client.get(format!("{base}/auth")).send().await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the listener never answered");
}

/// One JSON request. Spelled out rather than through reqwest's `json`, which
/// is behind a feature this app does not build with.
async fn get(client: &reqwest::Client, url: &str) -> serde_json::Value {
    let body = client
        .get(url)
        .send()
        .await
        .expect("a request")
        .text()
        .await
        .expect("a body");
    serde_json::from_str(&body).expect("json")
}

/// Replacing the key does not take the listener down and put it up again.
///
/// It used to. Dropping a listener only *starts* its shutdown — the serving
/// task still owns the socket until it returns — so rebinding the same address
/// and port in the same breath lands on a port that is still held, and the
/// operator is told the port they have always used is already in use by an app
/// that is the one using it. The key is published into the running server
/// instead, and the browsers that were let in with the old one are dropped on
/// their own.
#[tokio::test]
async fn replacing_the_key_does_not_rebind_the_port() {
    let app = mock();
    let handle = app.handle().clone();

    let port = PORT + 6;
    let with_key = |key: &str| RemoteConfig {
        enabled: true,
        address: "127.0.0.1".into(),
        port,
        key: key.into(),
    };

    let status = remote::apply(&handle, &with_key("FIRSTKEY")).await;
    assert!(status.running, "not serving: {:?}", status.error);

    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();
    serving(&client, &base).await;

    // A browser that is in on the old key.
    let cookie = let_in(&client, &base, "FIRSTKEY").await;
    let mut socket = connect_with(port, &cookie).await;

    let status = remote::apply(&handle, &with_key("SECONDKY")).await;
    assert!(
        status.running,
        "the listener went down when the key changed: {:?}",
        status.error
    );
    assert_eq!(
        status.error, None,
        "changing the key reported a problem with the port"
    );
    // The new key is in the address, so the QR code on the Settings page is
    // the new one too.
    assert_eq!(
        status.url.as_deref(),
        Some(format!("http://127.0.0.1:{port}/#k=SECONDKY").as_str())
    );

    // The browser that was in on the old key is put out.
    let dropped = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(frame) = socket.next().await {
            match frame {
                Ok(Message::Close(_)) | Err(_) => return true,
                Ok(_) => continue,
            }
        }
        true
    })
    .await;
    assert!(
        dropped.unwrap_or(false),
        "a browser let in with the old key stayed connected"
    );

    // And the old key does not get back in, while the new one does.
    assert_eq!(
        client
            .post(format!("{base}/auth"))
            .body("FIRSTKEY")
            .send()
            .await
            .expect("offering the old key")
            .status()
            .as_u16(),
        403
    );
    let cookie = let_in(&client, &base, "SECONDKY").await;
    let mut socket = connect_with(port, &cookie).await;
    let _ = socket.close(None).await;

    remote::apply(&handle, &RemoteConfig::default()).await;
}

/// Offers a key and returns the cookie it was given.
async fn let_in(client: &reqwest::Client, base: &str, key: &str) -> String {
    let answer = client
        .post(format!("{base}/auth"))
        .body(key.to_string())
        .send()
        .await
        .expect("offering the key");
    assert_eq!(answer.status().as_u16(), 204, "the key was refused");
    answer
        .headers()
        .get("set-cookie")
        .expect("a cookie was set")
        .to_str()
        .expect("a readable cookie")
        .split(';')
        .next()
        .expect("a name and a value")
        .to_string()
}

async fn connect_with(port: u16, cookie: &str) -> Socket {
    let mut request = format!("ws://127.0.0.1:{port}/ws")
        .into_client_request()
        .expect("a request");
    request
        .headers_mut()
        .insert("cookie", cookie.parse().expect("a cookie header"));
    let (socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("the socket was refused a browser that had been let in");
    socket
}

/// Switching the listener off and straight back on binds the same port again.
///
/// The other half of the same bug: two saves in quick succession — which is
/// what the Settings page's debounce produces — used to have the second one
/// bind a port the first one's task had not let go of yet. `apply` waits for
/// it now; without that wait this test reports "port is already in use" about
/// this app's own port.
#[tokio::test]
async fn it_can_be_switched_off_and_straight_back_on() {
    let app = mock();
    let handle = app.handle().clone();

    let port = PORT + 7;
    let on = RemoteConfig {
        enabled: true,
        address: "127.0.0.1".into(),
        port,
        key: String::new(),
    };

    let status = remote::apply(&handle, &on).await;
    assert!(status.running, "not serving: {:?}", status.error);
    // Up for real, so that what follows is a listener being taken down rather
    // than one that had not finished coming up.
    let (mut socket, _) = connect_to(port).await;
    let _ = socket.close(None).await;

    // No pause between them. That is the case: the debounce hands `apply` two
    // configurations back to back.
    remote::apply(&handle, &RemoteConfig::default()).await;
    let status = remote::apply(&handle, &on).await;

    assert_eq!(
        status.error, None,
        "binding the same port again failed right after it was released"
    );
    assert!(status.running, "the listener did not come back up");

    let (mut socket, _) = connect_to(port).await;
    let _ = socket.close(None).await;
    remote::apply(&handle, &RemoteConfig::default()).await;
}
