use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BODY: &str = "name: fixture\nversion: 1.0.0\ndescription: fixture-private-response\n";

#[test]
fn download_status_is_checked_before_installation() {
    for status in [404, 500, 200] {
        for existing in [true, false] {
            let temp = tempfile::tempdir().unwrap();
            let home = temp.path().join("home");
            let install = home.join("plugins/fixture");
            if existing {
                std::fs::create_dir_all(&install).unwrap();
                std::fs::write(install.join("manifest.yaml"), b"original manifest").unwrap();
                std::fs::write(install.join("marker"), b"preserve").unwrap();
            }
            let log = temp.path().join("log");
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "marketplace_http_worker",
                    "--ignored",
                    "--nocapture",
                ])
                .env_clear()
                .env("COVEN_CODE_HOME", &home)
                .env("NO_PROXY", "*")
                .env("no_proxy", "*")
                .env("MARKETPLACE_STATUS", status.to_string())
                .env("MARKETPLACE_COMPLETE", temp.path().join("complete"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(std::fs::File::create(&log).unwrap())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            let outcome = loop {
                match child.try_wait() {
                    Ok(Some(outcome)) => break outcome,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    result => {
                        let _ = child.kill();
                        let _ = child.wait();
                        panic!("{status}/{existing}: child wait failed: {result:?}");
                    }
                }
            };
            if status != 200 && existing {
                assert_eq!(
                    std::fs::read(install.join("manifest.yaml")).unwrap(),
                    b"original manifest",
                    "HTTP failure overwrote existing manifest"
                );
            }
            assert!(
                outcome.success(),
                "{status}/{existing}: {}",
                std::fs::read_to_string(log).unwrap()
            );
            assert_eq!(
                std::fs::read(temp.path().join("complete")).unwrap(),
                b"checked"
            );
            if status == 200 {
                assert_eq!(
                    std::fs::read(install.join("manifest.yaml")).unwrap(),
                    BODY.as_bytes()
                );
                assert!(!install.join("plugin.zip").exists());
            } else if existing {
                assert_eq!(std::fs::read_dir(&install).unwrap().count(), 2);
            } else {
                assert!(
                    !home.exists(),
                    "failed download created installation directories"
                );
            }
            if existing {
                assert_eq!(std::fs::read(install.join("marker")).unwrap(), b"preserve");
            }
        }
    }
}

#[test]
#[ignore = "bounded worker with owned home and loopback HTTP server"]
fn marketplace_http_worker() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    assert_eq!(claurst_core::config::config_home(), home);
    let status: u16 = std::env::var("MARKETPLACE_STATUS")
        .unwrap()
        .parse()
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/fixture.zip", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let read = socket.read(&mut buffer).await.unwrap();
                    assert!(read > 0);
                    request.extend_from_slice(&buffer[..read]);
                    assert!(request.len() < 16384);
                }
                assert!(request.starts_with(b"GET /fixture.zip HTTP/1.1\r\n"));
                let response = format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{BODY}", BODY.len());
                socket.write_all(response.as_bytes()).await.unwrap();
            });
            let result = claurst_plugins::marketplace::marketplace_install(&url).await;
            server.await.unwrap();
            if status == 200 {
                let installed = result.unwrap();
                assert_eq!(installed.name, "fixture");
                assert_eq!(installed.install_path, home.join("plugins/fixture"));
            } else {
                let error = result.expect_err("HTTP error response must not install a plugin");
                assert_eq!(error, format!("Download returned {status} {}", if status == 404 { "Not Found" } else { "Internal Server Error" }));
                assert!(!error.contains("fixture-private-response"), "{error}");
            }
        }).await.expect("loopback install exceeded deadline");
    });
    std::fs::write(
        std::env::var_os("MARKETPLACE_COMPLETE").unwrap(),
        b"checked",
    )
    .unwrap();
}
