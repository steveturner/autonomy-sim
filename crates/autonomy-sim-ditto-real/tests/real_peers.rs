use std::{
    env,
    net::TcpListener,
    process::Command,
    sync::{Mutex, MutexGuard},
};

static REAL_DITTO_TEST_LOCK: Mutex<()> = Mutex::new(());

fn available_port_pair() -> u16 {
    for port in 20_000..60_000 {
        if TcpListener::bind(("127.0.0.1", port)).is_ok()
            && TcpListener::bind(("127.0.0.1", port + 1)).is_ok()
        {
            return port;
        }
    }
    panic!("no two consecutive loopback ports were available")
}

fn run_real_peer_mode(mode: &str, collection: Option<&str>) {
    // Ditto owns listening ports and process-global runtime state. Keep these
    // subprocess-backed tests serial so a parallel test runner cannot race the
    // port probe or oversubscribe the native runtime.
    let _guard: MutexGuard<'static, ()> = REAL_DITTO_TEST_LOCK
        .lock()
        .expect("locking real Ditto integration test");
    let license = match env::var("DITTO_LICENSE") {
        Ok(license) if !license.is_empty() => license,
        _ => {
            eprintln!("skipping real Ditto integration test: DITTO_LICENSE is not set");
            return;
        }
    };
    let storage = tempfile::tempdir().expect("creating real Ditto test storage");
    let mut command = Command::new(env!("CARGO_BIN_EXE_autonomy-sim-ditto-real-demo"));
    command
        .env_remove("NO_COLOR")
        .env("DITTO_LICENSE", license)
        .env("DITTO_REAL_MODE", mode)
        .env("DITTO_REAL_PORT_BASE", available_port_pair().to_string())
        .env("DITTO_REAL_STORAGE_ROOT", storage.path());
    if let Some(collection) = collection {
        command.env("DITTO_REAL_COLLECTION", collection);
    }
    let output = command.output().expect("launching real Ditto peer demo");
    assert!(
        output.status.success(),
        "real Ditto mode {mode} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn two_real_peers_converge_a_configured_collection() {
    run_real_peer_mode("converge", Some("cuas.tracks"));
}

#[test]
fn gated_link_blocks_then_restores_real_sync() {
    run_real_peer_mode("gated", None);
}
