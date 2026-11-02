// Smoke-only socket control: created before capture, one write inside its window.
pub(crate) async fn pre_capture_control(pre_control: std::path::PathBuf) {
    use tokio::io::AsyncWriteExt;
    while !pre_control.join("target").exists() {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let destination =
        std::fs::read_to_string(pre_control.join("target")).expect("A9 pre-capture target");
    let mut stream = tokio::net::TcpStream::connect(destination.trim())
        .await
        .expect("A9 pre-capture connection");
    std::fs::write(
        pre_control.join("pre.connected"),
        stream
            .local_addr()
            .expect("A9 pre-capture address")
            .to_string(),
    )
    .expect("A9 pre-capture marker");
    while !pre_control.join("capture.started").exists() {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    stream
        .write_all(b"M3 pre-capture control send")
        .await
        .expect("A9 pre-capture send");
    while !pre_control.join("capture.finished").exists() {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod a9_control_tests {
    #[tokio::test]
    async fn pre_capture_socket_waits_then_sends() {
        use tokio::io::AsyncReadExt;
        let control = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        std::fs::write(
            control.path().join("target"),
            listener.local_addr().unwrap().to_string(),
        )
        .unwrap();
        let hook = tokio::spawn(super::pre_capture_control(control.path().to_path_buf()));
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0; b"M3 pre-capture control send".len()];
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(100),
            socket.read(&mut bytes)
        )
        .await
        .is_err());
        assert!(control.path().join("pre.connected").exists());
        std::fs::write(control.path().join("capture.started"), b"start").unwrap();
        let received = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            socket.read_exact(&mut bytes),
        )
        .await;
        std::fs::write(control.path().join("capture.finished"), b"end").unwrap();
        hook.await.unwrap();
        received.expect("A9 pre-capture write missing").unwrap();
        assert_eq!(&bytes, b"M3 pre-capture control send");
    }
}
