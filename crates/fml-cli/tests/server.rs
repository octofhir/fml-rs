//! Настройка FHIR-сервера через публичный CLI; HTTP-запросы идут только на localhost.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const MAP: &str = r#"/// url = 'http://example.org/StructureMap/resolve'
/// name = 'resolve'
uses "http://hl7.org/fhir/6.0/StructureDefinition/Observation" alias ObservationR6 as source
uses "http://hl7.org/fhir/4.0/StructureDefinition/Observation" alias ObservationR4 as target
group Main(source src : ObservationR6, target tgt : ObservationR4) {
  src.bodyStructure as bs then {
    bs.reference as r -> tgt.code as c, c.text = evaluate(r, (resolve() as BodyStructure).id.toString()) "resolve";
  } "bodyStructure";
}
"#;

fn run(args: &[&str]) -> Output {
    let directory = tempfile::tempdir().unwrap();
    let map = directory.path().join("map.fml");
    let input = directory.path().join("input.json");
    std::fs::write(&map, MAP).unwrap();
    std::fs::write(
        &input,
        json!({
            "resourceType": "Observation",
            "bodyStructure": [{"reference": {"reference": "BodyStructure/abc"}}]
        })
        .to_string(),
    )
    .unwrap();
    Command::new(env!("CARGO_BIN_EXE_fml"))
        .arg("run")
        .arg(map)
        .arg(input)
        .args(["--fhir", "r6"])
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn server_is_disabled_without_configuration() {
    let output = run(&[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"resourceType": "Observation"})
    );
}

#[test]
fn server_url_resolves_typed_resources_over_http() {
    for (status, body, expected) in [
        (
            "200 OK",
            json!({"resourceType": "BodyStructure", "id": "abc"}),
            json!({"resourceType": "Observation", "code": {"text": "abc"}}),
        ),
        (
            "404 Not Found",
            json!({"resourceType": "OperationOutcome"}),
            json!({"resourceType": "Observation"}),
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/fhir/", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "CLI did not contact the configured server"
                        );
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).unwrap();
                assert_ne!(count, 0);
                request.extend_from_slice(&buffer[..count]);
                assert!(request.len() < 16 * 1024);
            }
            let body = body.to_string();
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/fhir+json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            String::from_utf8(request).unwrap()
        });
        let output = run(&["--server-url", &url]);
        let request = server.join().unwrap();
        assert!(
            request.starts_with("GET /fhir/BodyStructure/abc HTTP/1.1\r\n"),
            "{request}"
        );
        assert!(
            request
                .to_ascii_lowercase()
                .contains("accept: application/fhir+json\r\n"),
            "{request}"
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap(),
            expected
        );
    }
}

#[test]
fn invalid_server_urls_are_rejected_before_execution() {
    for url in [
        "not-a-url",
        "ftp://example.org/fhir",
        "https://user:password@example.org/fhir",
        "https://example.org/fhir?token=secret",
        "https://example.org/fhir#fragment",
    ] {
        let output = run(&["--server-url", url]);
        assert_eq!(output.status.code(), Some(2), "{url}");
        assert!(output.stdout.is_empty(), "{url}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("--server-url"),
            "{url}"
        );
    }
}
