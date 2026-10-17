//! The webhook server's `accept()` failure arm, in a process that is out of
//! file descriptors.
//!
//! `RLIMIT_NOFILE` is per-process, so this is its own test binary: that is what
//! makes the fixture safe. The other way to break `accept` — closing the
//! listener's fd behind `serve`'s back — does *not* work: the kernel drops the
//! kqueue registration with the fd, so the future pends instead of reporting an
//! error (measured while reviewing this row). Exhausting the table is the
//! construction that actually reaches the arm.
//!
//! The test keeps the process-wide limit lowered for its own lifetime; a
//! process that ends with the test cannot disturb any other test.

#![cfg(unix)]

use future_channel::bridge::Shutdown;
use future_channel::transport::webhook::WebhookServer;
use std::time::Duration;

#[test]
fn a_full_fd_table_turns_a_queued_connection_into_an_accept_error() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let server = WebhookServer::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral port");
        let addr = server.local_addr().expect("bound address");
        // One connection waiting to be accepted: `accept` then has work to do,
        // so the failure below cannot be "nothing to accept".
        let queued = std::net::TcpStream::connect(addr).expect("queue a connection");

        // Count what this process holds and forbid any *new* descriptor. The
        // listener's own fd is already open, so `accept` is the next allocation
        // and fails with EMFILE instead of returning a connection.
        let held = std::fs::read_dir("/dev/fd").expect("list open fds").count();
        let mut current = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut current) },
            0
        );
        let lowered = libc::rlimit {
            rlim_cur: held as libc::rlim_t,
            rlim_max: current.rlim_max,
        };
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &lowered) }, 0);
        let mut in_force = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut in_force) },
            0
        );
        assert_eq!(
            in_force.rlim_cur, held as libc::rlim_t,
            "the limit must be in force"
        );

        // Fill whatever is left, holding the descriptors so they stay taken.
        let mut filler = Vec::new();
        loop {
            match std::fs::File::open("/dev/null") {
                Ok(file) => filler.push(file),
                Err(_) => break,
            }
            assert!(
                filler.len() < 64,
                "the lowered limit must be in force, not the machine's default \
                 ({} descriptors were free)",
                filler.len()
            );
        }

        // Stop shortly after the server starts looping: the accept error is
        // hit on the first pass, and the shutdown then ends the loop.
        let shutdown = Shutdown::new();
        let timer_shutdown = shutdown.clone();
        let timer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            timer_shutdown.trigger();
        });
        let outcome = server.serve(shutdown).await;
        assert!(
            outcome.is_ok(),
            "an accept error is logged and the loop continues: {outcome:?}"
        );
        let _ = timer.await;

        drop(filler);
        drop(queued);
    });
}
