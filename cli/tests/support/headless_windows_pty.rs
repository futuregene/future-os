//! Native Windows terminal fixture. ConPTY makes the real CLI see terminal
//! handles and interprets the input Ctrl+C byte using console signal semantics.
use super::*;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

pub(super) struct PtyRun {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    _master: Box<dyn MasterPty + Send>,
    transcript: Arc<Mutex<String>>,
    home: PathBuf,
}

impl PtyRun {
    pub(super) fn spawn(home: PathBuf, args: &[&str]) -> Self {
        wait_for_web_port_free();
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("create owned ConPTY");
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_future"));
        command.arg("headless");
        command.args(args);
        command.env_clear();
        for key in [
            "PATH",
            "SystemRoot",
            "TEMP",
            "TMP",
            "LANG",
            "LLVM_PROFILE_FILE",
            "RUST_BACKTRACE",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command.env("HOME", &home);
        command.env("USERPROFILE", &home);
        command.env("FUTURE_HOME", home.join(".future"));
        let child = pair
            .slave
            .spawn_command(command)
            .expect("spawn future headless on ConPTY");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("ConPTY reader");
        let writer = pair.master.take_writer().expect("ConPTY writer");
        let transcript = Arc::new(Mutex::new(String::new()));
        let sink = transcript.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => sink
                        .lock()
                        .unwrap()
                        .push_str(&String::from_utf8_lossy(&buf[..n])),
                }
            }
        });
        Self {
            child,
            writer,
            _master: pair.master,
            transcript,
            home,
        }
    }

    pub(super) fn text(&self) -> String {
        self.transcript.lock().unwrap().clone()
    }

    pub(super) fn wait_for(&self, needle: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.text().contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        self.text().contains(needle)
    }

    pub(super) fn interrupt(&mut self) {
        self.writer.write_all(&[3]).expect("send ConPTY Ctrl+C");
        self.writer.flush().expect("flush ConPTY Ctrl+C");
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for PtyRun {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}
