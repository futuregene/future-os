//! Ask every paired host what it advertises, and print it.
//!
//! This is the diagnostic behind "the terminal panel does not appear": a
//! capability the host never lists is a control the window hides without a
//! word, so the only way to tell "this machine serves no shells" from "the
//! window is broken" is to read the host's own list. `future remote peers
//! list` cannot answer it — the projection it prints carries connection state,
//! not capabilities — so the probe is its own client connection.
//!
//! ```text
//! cargo run -p future-host --example peer_features
//! desktop_AZYJSK5RT6PC5BQZ connected=true terminal=true features=[…, "terminal_v1"]
//! ```
//!
//! It opens a connection of its own, so a window already connected to the same
//! host reconnects afterwards (the host serves one live session per pairing).
//! Close the window first when the printed list has to be the only connection.

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime for the client connection");
    // The client runtime is installed by an entry point, and this probe is one.
    future_host::runtime::set(runtime.handle().clone());
    let peers = match runtime
        .block_on(async { future_host::remote_peer::runtime::connect_all(None).await })
    {
        Ok(peers) => peers,
        Err(error) => {
            eprintln!("could not reach a paired host: {error}");
            std::process::exit(1);
        }
    };
    if peers.is_empty() {
        eprintln!(
            "no remote desktop is paired with this machine; pair one with \
             `future remote peers add <invitation>`"
        );
        std::process::exit(1);
    }
    for peer in peers {
        println!(
            "{} connected={} terminal={} features={:?}",
            peer.desktop_id,
            peer.connected,
            peer.features
                .iter()
                .any(|feature| feature == future_remote_protocol::TERMINAL_FEATURE),
            peer.features
        );
    }
}
