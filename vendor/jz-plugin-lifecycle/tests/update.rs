use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::time::{Duration, Instant};

use jz_plugin_lifecycle::update::{
    run_check, version_less, UpdateEnv, CHECK_INTERVAL_SECS, RETRY_AFTER_FAILURE_SECS,
    UPDATE_NOTICE_SCHEMA,
};
use mockito::{Matcher, Server};
use serde_json::{json, Value};

const NOW: u64 = 1_800_000_000;

fn catalog(version: &str) -> String {
    json!({"plugins": [
        {"name": "other", "version": "9.9.9"},
        {"name": "demo", "version": version},
    ]})
    .to_string()
}

fn check(home: &Path, url: &str, version: &str, now: u64) -> String {
    let mut out = Vec::new();
    run_check(
        "demo",
        version,
        &UpdateEnv {
            home,
            catalog_url: url,
            now_secs: now,
        },
        &mut out,
    );
    String::from_utf8(out).unwrap()
}

fn state(home: &Path) -> Value {
    serde_json::from_slice(&fs::read(home.join("demo/update-check.json")).unwrap()).unwrap()
}

fn write_state(home: &Path, value: Value) {
    fs::create_dir_all(home.join("demo")).unwrap();
    fs::write(home.join("demo/update-check.json"), value.to_string()).unwrap();
}

#[test]
fn version_less_is_numeric_and_total() {
    assert!(version_less("1.2.9", "1.2.10"));
    assert!(version_less("1.0", "1.0.1"));
    assert!(!version_less("1.0", "1.0.0"));
    assert!(!version_less("2.0.0", "1.9.9"));
    assert!(!version_less("1.0.0-beta", "1.0.1"));
    assert!(!version_less("", "1.0.1"));
    assert!(!version_less("1.0.0", "x"));
    assert!(!version_less("1..0", "1.1"));
}

#[test]
fn first_call_notifies_once_then_stays_quiet_within_a_day() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/marketplace.json")
        .with_body(catalog("1.2.0"))
        .expect(1)
        .create();
    let home = tempfile::tempdir().unwrap();
    let url = format!("{}/marketplace.json", server.url());

    let first = check(home.path(), &url, "1.0.0", NOW);
    assert_eq!(first.lines().count(), 1);
    let line: Value = serde_json::from_str(first.trim_end()).unwrap();
    assert_eq!(line["schema"], UPDATE_NOTICE_SCHEMA);
    assert_eq!(line["plugin_id"], "demo");
    assert_eq!(line["current"], "1.0.0");
    assert_eq!(line["latest"], "1.2.0");
    assert!(line["say_to_user"]
        .as_str()
        .unwrap()
        .contains("nothing is blocked"));

    assert_eq!(check(home.path(), &url, "1.0.0", NOW + 3_600), "");
    assert_eq!(
        check(home.path(), &url, "1.0.0", NOW + CHECK_INTERVAL_SECS - 1),
        ""
    );
    mock.assert();
}

#[test]
fn a_higher_latest_the_same_day_notifies_again() {
    let home = tempfile::tempdir().unwrap();
    write_state(
        home.path(),
        json!({"checked_at": NOW, "latest": "1.3.0", "notified_at": NOW, "notified_version": "1.2.0"}),
    );
    let out = check(home.path(), "http://127.0.0.1:1/unused", "1.0.0", NOW + 60);
    assert_eq!(out.lines().count(), 1);
    assert_eq!(state(home.path())["notified_version"], "1.3.0");
    assert_eq!(
        check(home.path(), "http://127.0.0.1:1/unused", "1.0.0", NOW + 120),
        ""
    );
}

#[test]
fn same_version_is_notified_again_after_a_day_if_still_behind() {
    let home = tempfile::tempdir().unwrap();
    write_state(
        home.path(),
        json!({"checked_at": NOW, "latest": "1.2.0", "notified_at": NOW, "notified_version": "1.2.0"}),
    );
    assert_eq!(
        check(
            home.path(),
            "http://127.0.0.1:1/unused",
            "1.0.0",
            NOW + CHECK_INTERVAL_SECS - 1
        ),
        ""
    );
    // Past 24 h the check is due too; a dead catalog keeps the old latest.
    let out = check(
        home.path(),
        "http://127.0.0.1:1/unused",
        "1.0.0",
        NOW + CHECK_INTERVAL_SECS,
    );
    assert_eq!(out.lines().count(), 1);
}

#[test]
fn equal_or_newer_installed_version_is_silent() {
    let mut server = Server::new();
    let _mock = server
        .mock("GET", "/m.json")
        .with_body(catalog("1.0.0"))
        .create();
    let home = tempfile::tempdir().unwrap();
    let url = format!("{}/m.json", server.url());
    assert_eq!(check(home.path(), &url, "1.0.0", NOW), "");
    assert_eq!(
        check(home.path(), &url, "1.1.0", NOW + CHECK_INTERVAL_SECS),
        ""
    );
    assert_eq!(state(home.path())["latest"], "1.0.0");
}

#[test]
fn connection_failure_is_silent_and_retries_after_an_hour() {
    // A port nobody listens on.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("http://127.0.0.1:{port}/m.json");
    let home = tempfile::tempdir().unwrap();
    assert_eq!(check(home.path(), &url, "1.0.0", NOW), "");
    let saved = state(home.path());
    assert_eq!(
        saved["checked_at"].as_u64().unwrap(),
        NOW - CHECK_INTERVAL_SECS + RETRY_AFTER_FAILURE_SECS
    );
    assert!(saved.get("latest").is_none());
}

#[test]
fn failure_keeps_the_previous_latest() {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let home = tempfile::tempdir().unwrap();
    write_state(
        home.path(),
        json!({"checked_at": NOW - CHECK_INTERVAL_SECS, "latest": "1.1.0", "notified_at": NOW - 10, "notified_version": "1.1.0"}),
    );
    assert_eq!(
        check(
            home.path(),
            &format!("http://127.0.0.1:{port}/m.json"),
            "1.0.0",
            NOW
        ),
        ""
    );
    assert_eq!(state(home.path())["latest"], "1.1.0");
}

#[test]
fn a_server_that_never_answers_cannot_hold_the_caller() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let keep = std::thread::spawn(move || {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept() {
            held.push(socket);
        }
    });
    let home = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let out = check(
        home.path(),
        &format!("http://127.0.0.1:{port}/m.json"),
        "1.0.0",
        NOW,
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "took {:?}",
        started.elapsed()
    );
    assert_eq!(out, "");
    assert_eq!(
        state(home.path())["checked_at"].as_u64().unwrap(),
        NOW - CHECK_INTERVAL_SECS + RETRY_AFTER_FAILURE_SECS
    );
    drop(keep); // the accept loop ends with the process
}

#[test]
fn future_timestamps_and_junk_state_do_not_panic() {
    let mut server = Server::new();
    let _mock = server
        .mock("GET", "/m.json")
        .with_body(catalog("2.0.0"))
        .create();
    let url = format!("{}/m.json", server.url());

    let home = tempfile::tempdir().unwrap();
    write_state(
        home.path(),
        json!({"checked_at": u64::MAX, "notified_at": u64::MAX, "latest": "1.5.0", "notified_version": "1.5.0"}),
    );
    // checked_at in the future counts as due: refetches and notifies.
    assert_eq!(check(home.path(), &url, "1.0.0", NOW).lines().count(), 1);

    let junk = tempfile::tempdir().unwrap();
    fs::create_dir_all(junk.path().join("demo")).unwrap();
    fs::write(junk.path().join("demo/update-check.json"), b"{not json").unwrap();
    assert_eq!(check(junk.path(), &url, "1.0.0", 0).lines().count(), 1);

    let wrong_types = tempfile::tempdir().unwrap();
    write_state(
        wrong_types.path(),
        json!({"checked_at": "soon", "latest": 7, "notified_at": -1}),
    );
    assert_eq!(
        check(wrong_types.path(), &url, "1.0.0", u64::MAX)
            .lines()
            .count(),
        1
    );
}

#[test]
fn redirects_are_never_followed_and_no_credentials_are_sent() {
    let mut server = Server::new();
    let target = server
        .mock("GET", "/elsewhere.json")
        .with_body(catalog("5.0.0"))
        .expect(0)
        .create();
    let start = server
        .mock("GET", "/m.json")
        .match_header("authorization", Matcher::Missing)
        .match_header("cookie", Matcher::Missing)
        .match_header("proxy-authorization", Matcher::Missing)
        .with_status(302)
        .with_header("location", &format!("{}/elsewhere.json", server.url()))
        .expect(1)
        .create();
    let home = tempfile::tempdir().unwrap();
    assert_eq!(
        check(
            home.path(),
            &format!("{}/m.json", server.url()),
            "1.0.0",
            NOW
        ),
        ""
    );
    start.assert();
    target.assert();
}

#[test]
fn malformed_or_unrelated_catalogs_are_silent() {
    let mut server = Server::new();
    let _a = server.mock("GET", "/a.json").with_body("<html>").create();
    let _b = server
        .mock("GET", "/b.json")
        .with_body(json!({"plugins": [{"name": "elsewhere", "version": "9.0.0"}]}).to_string())
        .create();
    let _c = server
        .mock("GET", "/c.json")
        .with_body(catalog("not-a-version"))
        .create();
    let _d = server.mock("GET", "/d.json").with_status(500).create();
    for path in ["a", "b", "c", "d"] {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(
            check(
                home.path(),
                &format!("{}/{path}.json", server.url()),
                "1.0.0",
                NOW
            ),
            "",
            "{path}"
        );
    }
}
