use std::io::{Cursor, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MANIFEST: &[u8] = b"name: fixture\nversion: 1.0.0\n";

#[test]
fn download_staging_does_not_collide_with_plugin_files() {
    for case in ["yaml", "yaml-new", "zip-member", "zip"] {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let install = home.join("plugins/fixture");
        std::fs::create_dir_all(&install).unwrap();
        if case != "yaml-new" {
            std::fs::write(install.join("manifest.yaml"), b"original manifest").unwrap();
        }
        #[cfg(unix)]
        if case == "yaml" {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                install.join("manifest.yaml"),
                std::fs::Permissions::from_mode(0o640),
            )
            .unwrap();
        }
        std::fs::write(temp.path().join("normal-file"), b"mode control").unwrap();
        std::fs::write(install.join("marker"), b"preserve").unwrap();
        std::fs::write(install.join("plugin.zip"), b"preserved plugin file").unwrap();
        let body = if case.starts_with("yaml") {
            MANIFEST.to_vec()
        } else {
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            let entry = "manifest.yaml";
            writer
                .start_file(
                    entry,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Stored),
                )
                .unwrap();
            writer.write_all(MANIFEST).unwrap();
            if case == "zip-member" {
                writer
                    .start_file(
                        "plugin.zip",
                        zip::write::SimpleFileOptions::default()
                            .compression_method(zip::CompressionMethod::Stored),
                    )
                    .unwrap();
                writer.write_all(b"archive member contents").unwrap();
            }
            let body = writer.finish().unwrap().into_inner();
            assert_eq!(
                zip::ZipArchive::new(Cursor::new(&body)).unwrap().len(),
                if case == "zip-member" { 2 } else { 1 }
            );
            body
        };
        let payload = temp.path().join("payload");
        std::fs::write(&payload, &body).unwrap();
        let log = temp.path().join("log");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "marketplace_collision_worker",
                "--ignored",
                "--nocapture",
            ])
            .env_clear()
            .env("COVEN_CODE_HOME", &home)
            .env("NO_PROXY", "*")
            .env("no_proxy", "*")
            .env("COLLISION_PAYLOAD", payload)
            .env("COLLISION_COMPLETE", temp.path().join("complete"))
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
                    panic!("{case}: child wait failed: {result:?}");
                }
            }
        };
        assert!(
            install.join("plugin.zip").is_file(),
            "{case}: staging removed plugin.zip"
        );
        assert_eq!(
            std::fs::read(install.join("plugin.zip")).unwrap(),
            if case == "zip-member" {
                b"archive member contents".as_slice()
            } else {
                b"preserved plugin file".as_slice()
            },
            "{case}: staging overwrote plugin.zip"
        );
        assert_eq!(
            std::fs::read(install.join("manifest.yaml")).unwrap(),
            MANIFEST
        );
        #[cfg(unix)]
        if case.starts_with("yaml") {
            use std::os::unix::fs::PermissionsExt;
            let expected = if case == "yaml" {
                0o640
            } else {
                std::fs::metadata(temp.path().join("normal-file"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777
            };
            assert_eq!(
                std::fs::metadata(install.join("manifest.yaml"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                expected
            );
        }
        assert_eq!(std::fs::read(install.join("marker")).unwrap(), b"preserve");
        assert!(
            outcome.success(),
            "{case}: {}",
            std::fs::read_to_string(log).unwrap()
        );
        assert_eq!(
            std::fs::read(temp.path().join("complete")).unwrap(),
            b"checked"
        );
    }
}

#[test]
#[ignore = "bounded worker with owned home and loopback HTTP server"]
fn marketplace_collision_worker() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let home = std::path::PathBuf::from(std::env::var_os("COVEN_CODE_HOME").unwrap());
    assert_eq!(claurst_core::config::config_home(), home);
    let body = std::fs::read(std::env::var_os("COLLISION_PAYLOAD").unwrap()).unwrap();
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
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(headers.as_bytes()).await.unwrap();
                socket.write_all(&body).await.unwrap();
            });
            let result = claurst_plugins::marketplace::marketplace_install(&url).await;
            server.await.unwrap();
            let installed = result.unwrap();
            assert_eq!(installed.name, "fixture");
            assert_eq!(installed.install_path, home.join("plugins/fixture"));
        })
        .await
        .expect("loopback install exceeded deadline");
    });
    std::fs::write(std::env::var_os("COLLISION_COMPLETE").unwrap(), b"checked").unwrap();
}
