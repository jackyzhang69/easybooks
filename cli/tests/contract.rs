//! Request-shape contract tests for the EasyBooks CLI.
//!
//! These assert the EXACT request the CLI sends to the backend integration
//! endpoints (contract §3) — method, path, headers, and JSON body — against a
//! mockito server. They never hit a real backend.
//!
//! Auth model: EasyBooks is exchange-mode (`aud=eb`). The durable `jz_` is sent
//! only to accountd `POST /v1/token/exchange`. Product integration routes receive
//! the short-lived app JWT. Raw `jz_` must never appear as a product Bearer.
//!
//! Config is supplied via an isolated `HOME` + `JACKYZHANG_APP_HOME` with
//! `token/user.json` plus `EASYBOOKS_API_URL` / `EASYBOOKS_ACCOUNTD_URL` so the
//! tests are hermetic.

use assert_cmd::Command;
use mockito::Matcher;
use predicates::prelude::*;

fn easybooks() -> Command {
    Command::cargo_bin("easybooks").expect("easybooks binary")
}

const KEY: &str = "jz_test_key_for_contract";
const OWNER_BEARER: &str = "Bearer jz_test_key_for_contract";
const USER: &str = "11111111-1111-1111-1111-111111111111";

fn test_product_jwt(aud: &str, issuer: &str) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"ES256","typ":"JWT"}"#);
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    let payload = format!(r#"{{"aud":"{aud}","iss":"{issuer}","exp":{exp}}}"#);
    format!("{}.{}.sig", header, URL_SAFE_NO_PAD.encode(payload))
}

fn write_user_token(home: &std::path::Path, token: &str) {
    let platform = home.join(".jackyzhang.app");
    let token_dir = platform.join("token");
    std::fs::create_dir_all(&token_dir).expect("token dir");
    let path = token_dir.join("user.json");
    std::fs::write(
        &path,
        format!(r#"{{"token":"{token}","credential_kind":"user","slot":"user"}}"#),
    )
    .expect("user.json");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&platform, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&token_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn mock_exchange(accountd: &mut mockito::Server) -> (mockito::Mock, String) {
    let jwt = test_product_jwt("eb", &accountd.url());
    let mock = accountd
        .mock("POST", "/v1/token/exchange")
        .match_header("authorization", OWNER_BEARER)
        .match_body(Matcher::JsonString(
            r#"{"aud":"eb","scopes":["read","write"]}"#.to_string(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(format!(
            r#"{{"access_token":"{jwt}","token_type":"Bearer","expires_in":300}}"#
        ))
        .create();
    (mock, jwt)
}

fn isolate_platform(cmd: &mut Command, home: &std::path::Path) {
    cmd.env("HOME", home)
        .env("JACKYZHANG_APP_HOME", home.join(".jackyzhang.app"));
}

fn product_cmd(
    product: &mockito::Server,
    accountd: &mockito::Server,
) -> (tempfile::TempDir, Command) {
    let home = tempfile::tempdir().expect("tempdir");
    write_user_token(home.path(), KEY);
    let mut cmd = easybooks();
    isolate_platform(&mut cmd, home.path());
    cmd.env("EASYBOOKS_API_URL", product.url())
        .env("EASYBOOKS_ACCOUNTD_URL", accountd.url())
        .env("EASYBOOKS_PORTAL_OFFLINE", "1");
    (home, cmd)
}

fn authed_home_cmd() -> (tempfile::TempDir, Command) {
    let home = tempfile::tempdir().expect("tempdir");
    write_user_token(home.path(), KEY);
    let mut cmd = easybooks();
    isolate_platform(&mut cmd, home.path());
    (home, cmd)
}

#[test]
fn expense_add_posts_single_entry_with_bearer_and_cents() {
    let mut accountd = mockito::Server::new();
    let (exchange, app_jwt) = mock_exchange(&mut accountd);
    let app_bearer = format!("Bearer {app_jwt}");
    let mut server = mockito::Server::new();
    let mock = server
        .mock("POST", "/api/integrations/ingest/transactions")
        .match_header("authorization", app_bearer.as_str())
        .match_body(Matcher::JsonString(
            r#"{
                    "source_system": "manual",
                    "entries": [
                        {
                            "type": "expense",
                            "amount_cents": 12000,
                            "description": "Software subscription",
                            "date": "2026-05-01",
                            "category_name": "Software",
                            "classification": "business",
                            "source_id": "rcpt-123"
                        }
                    ]
                }"#
            .to_string(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"status":"ok","created":1,"existing":0,"skipped":0,"processed":1}"#)
        .create();

    let (_home, mut cmd) = product_cmd(&server, &accountd);
    cmd.args([
        "expense",
        "add",
        "--amount",
        "120.00",
        "--description",
        "Software subscription",
        "--date",
        "2026-05-01",
        "--category",
        "Software",
        "--classification",
        "business",
        "--source-id",
        "rcpt-123",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains(r#""created": 1"#));

    mock.assert();
    exchange.assert();
}

#[test]
fn income_add_dry_run_does_not_post_and_echoes_cents() {
    // No mock endpoint should be hit on a dry run. We still point at a server
    // URL; if the CLI erroneously POSTed, mockito would record an unmatched
    // request and the body assertions below would never appear.
    let server = mockito::Server::new();

    let (_home, mut cmd) = authed_home_cmd();
    cmd.env("EASYBOOKS_API_URL", server.url())
        .args([
            "income",
            "add",
            "--amount",
            "1,234.56",
            "--description",
            "Consulting fee",
            "--date",
            "2026-04-15",
            "--source-id",
            "inv-9",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""status": "dry_run""#))
        .stdout(predicate::str::contains(r#""amount_cents": 123456"#))
        .stdout(predicate::str::contains(r#""type": "income""#));
}

#[test]
fn tx_import_json_posts_envelope_verbatim() {
    let mut accountd = mockito::Server::new();
    let (exchange, app_jwt) = mock_exchange(&mut accountd);
    let app_bearer = format!("Bearer {app_jwt}");
    let mut server = mockito::Server::new();
    let mock = server
        .mock("POST", "/api/integrations/ingest/transactions")
        .match_header("authorization", app_bearer.as_str())
        .match_body(Matcher::JsonString(
            r#"{
                    "source_system": "stripe",
                    "entries": [
                        {
                            "type": "income",
                            "amount_cents": 5000,
                            "description": "Payout",
                            "date": "2026-03-01",
                            "source_id": "po_1"
                        },
                        {
                            "type": "expense",
                            "amount_cents": 250,
                            "description": "Fee",
                            "date": "2026-03-01",
                            "source_id": "fee_1"
                        }
                    ]
                }"#
            .to_string(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"status":"ok","created":2,"existing":0,"skipped":0,"processed":2}"#)
        .create();

    let payload = r#"{"source_system":"stripe","entries":[{"type":"income","amount_cents":5000,"description":"Payout","date":"2026-03-01","source_id":"po_1"},{"type":"expense","amount_cents":250,"description":"Fee","date":"2026-03-01","source_id":"fee_1"}]}"#;

    let (_home, mut cmd) = product_cmd(&server, &accountd);
    cmd.args(["tx", "import-json", "--json", payload])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""processed": 2"#));

    mock.assert();
    exchange.assert();
}

#[test]
fn tx_import_json_dry_run_validates_without_posting() {
    let server = mockito::Server::new();
    let payload = r#"{"source_system":"manual","entries":[{"type":"expense","amount_cents":999,"description":"X","date":"2026-01-01","source_id":"a"}]}"#;

    let (_home, mut cmd) = authed_home_cmd();
    cmd.env("EASYBOOKS_API_URL", server.url())
        .args(["tx", "import-json", "--json", payload, "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""status": "dry_run""#))
        .stdout(predicate::str::contains(r#""amount_cents": 999"#));
}

#[test]
fn tx_import_json_rejects_entry_missing_source_id() {
    let server = mockito::Server::new();
    // Missing source_id → local validation fails BEFORE any network call.
    let payload = r#"{"source_system":"manual","entries":[{"type":"expense","amount_cents":100,"description":"X","date":"2026-01-01"}]}"#;

    let (_home, mut cmd) = authed_home_cmd();
    cmd.env("EASYBOOKS_API_URL", server.url())
        .args(["tx", "import-json", "--json", payload, "--dry-run"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("source_id"));
}

#[test]
fn gmail_record_defaults_source_system_to_gmail() {
    let mut accountd = mockito::Server::new();
    let (exchange, app_jwt) = mock_exchange(&mut accountd);
    let app_bearer = format!("Bearer {app_jwt}");
    let mut server = mockito::Server::new();
    let mock = server
        .mock("POST", "/api/integrations/ingest/transactions")
        .match_header("authorization", app_bearer.as_str())
        .match_body(Matcher::JsonString(
            r#"{
                    "source_system": "gmail",
                    "entries": [
                        {
                            "type": "expense",
                            "amount_cents": 4200,
                            "description": "AWS",
                            "date": "2026-02-02",
                            "source_id": "msg-abc"
                        }
                    ]
                }"#
            .to_string(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"status":"ok","created":1,"existing":0,"skipped":0,"processed":1}"#)
        .create();

    // Note: no source_system in the input JSON — gmail record must default it.
    let payload = r#"{"entries":[{"type":"expense","amount_cents":4200,"description":"AWS","date":"2026-02-02","source_id":"msg-abc"}]}"#;

    let (_home, mut cmd) = product_cmd(&server, &accountd);
    cmd.args(["gmail", "record", "--json", payload])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""created": 1"#));

    mock.assert();
    exchange.assert();
}

#[test]
fn gmail_sync_is_v1_stub() {
    let (_home, mut cmd) = authed_home_cmd();
    cmd.env("EASYBOOKS_API_URL", "http://127.0.0.1:9")
        .args(["gmail", "sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            r#""status": "not_implemented_v1""#,
        ));
}

#[test]
fn top_level_json_flag_precedes_command_contract_matrix() {
    let home = tempfile::tempdir().expect("tempdir");
    easybooks()
        .env("HOME", home.path())
        .env("JACKYZHANG_APP_HOME", home.path().join(".jackyzhang.app"))
        .args(["--json", "doctor", "--no-fetch"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""binary_version""#));

    for command in ["login", "whoami"] {
        easybooks()
            .args(["--json", command, "--help"])
            .assert()
            .success();
    }

    easybooks()
        .args(["doctor", "--json", "--no-fetch"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument '--json'"));
}

#[test]
fn login_writes_config_and_masks_key() {
    let home = tempfile::tempdir().expect("tempdir");
    easybooks()
        .env("HOME", home.path())
        .env("JACKYZHANG_APP_HOME", home.path().join(".jackyzhang.app"))
        .args([
            "login",
            "--token-stdin",
            "--base-url",
            "http://192.168.1.69:8310",
        ])
        .write_stdin("jz_super_secret_value\n")
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""api_key_masked": "jz_***""#))
        // The raw secret must NEVER appear in output.
        .stdout(predicate::str::contains("jz_super_secret_value").not());

    // Shared user slot holds the token; product runtime config has base_url only.
    let slot = home
        .path()
        .join(".jackyzhang.app")
        .join("token")
        .join("user.json");
    assert!(slot.is_file(), "shared user.json should be written");
    let slot_raw = std::fs::read_to_string(&slot).expect("read user.json");
    assert!(slot_raw.contains("jz_super_secret_value"));
    let cfg = runtime_config_path(home.path());
    assert!(cfg.is_file(), "runtime config.json should be written");
    let cfg_raw = std::fs::read_to_string(&cfg).expect("read runtime config");
    assert!(
        !cfg_raw.contains("jz_super_secret_value"),
        "token must not live in product config"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&cfg).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "config.json must be mode 0600");
        let directory_mode = std::fs::metadata(cfg.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            directory_mode, 0o700,
            "easybooks runtime dir must be mode 0700"
        );
        let slot_mode = std::fs::metadata(&slot).unwrap().permissions().mode() & 0o777;
        assert_eq!(slot_mode, 0o600, "user.json must be mode 0600");
    }
}

#[test]
fn loads_legacy_easybooks_home_and_migrates_to_unified_runtime() {
    let home = tempfile::tempdir().expect("tempdir");
    let legacy_dir = home.path().join(".easybooks");
    std::fs::create_dir(&legacy_dir).expect("legacy dir");
    std::fs::write(
        legacy_dir.join("config.json"),
        r#"{"api_key":"jz_legacy_migrated_token","base_url":"http://192.168.1.69:8310"}"#,
    )
    .expect("write legacy config");

    easybooks()
        .env("HOME", home.path())
        .env("JACKYZHANG_APP_HOME", home.path().join(".jackyzhang.app"))
        .arg("whoami")
        .assert()
        .failure(); // no backend in this test; migration must still have run

    let migrated = runtime_config_path(home.path());
    assert!(
        migrated.is_file(),
        "unified runtime config should be written"
    );
    let migrated_raw = std::fs::read_to_string(&migrated).expect("read migrated config");
    assert!(migrated_raw.contains("192.168.1.69:8310"));
    assert!(
        !migrated_raw.contains("jz_legacy_migrated_token"),
        "migrated runtime config must not keep the portal token"
    );
    let slot = home
        .path()
        .join(".jackyzhang.app")
        .join("token")
        .join("user.json");
    let slot_raw = std::fs::read_to_string(&slot).expect("read migrated user.json");
    assert!(slot_raw.contains("jz_legacy_migrated_token"));
}

#[test]
fn login_rejects_token_argument() {
    for arguments in [
        vec!["login", "--token", "jz_must_not_enter_argv"],
        vec!["login", "--token=jz_must_not_enter_argv"],
    ] {
        let home = tempfile::tempdir().expect("tempdir");
        easybooks()
            .env("HOME", home.path())
            .env("JACKYZHANG_APP_HOME", home.path().join(".jackyzhang.app"))
            .args(arguments)
            .assert()
            .failure()
            .stdout(predicate::str::contains("jz_must_not_enter_argv").not())
            .stderr(predicate::str::contains("jz_must_not_enter_argv").not());
    }
}

#[test]
fn login_rejects_empty_or_multiline_stdin() {
    for input in [
        "\n",
        "jz_first\njz_second\n",
        "jz_extra_blank\n\n",
        " jz_leading_space\n",
    ] {
        let home = tempfile::tempdir().expect("tempdir");
        easybooks()
            .env("HOME", home.path())
            .env("JACKYZHANG_APP_HOME", home.path().join(".jackyzhang.app"))
            .args(["login", "--token-stdin"])
            .write_stdin(input)
            .assert()
            .failure()
            .stdout(predicate::str::contains("jz_").not())
            .stderr(predicate::str::contains("jz_").not());
        assert!(!runtime_config_path(home.path()).exists());
    }
}

#[cfg(unix)]
#[test]
fn login_rejects_symlinked_config_directory() {
    use std::os::unix::fs::symlink;

    let home = tempfile::tempdir().expect("home");
    let outside = tempfile::tempdir().expect("outside");
    let app = home.path().join(".jackyzhang.app");
    std::fs::create_dir(&app).expect("create platform home");
    symlink(outside.path(), app.join("easybooks")).expect("create symlink");

    easybooks()
        .env("HOME", home.path())
        .env("JACKYZHANG_APP_HOME", home.path().join(".jackyzhang.app"))
        .args(["login", "--token-stdin"])
        .write_stdin("jz_symlink_test\n")
        .assert()
        .failure()
        .stdout(predicate::str::contains("jz_symlink_test").not())
        .stderr(predicate::str::contains("jz_symlink_test").not());

    assert!(!outside.path().join("config.json").exists());
}

#[cfg(unix)]
#[test]
fn login_and_load_reject_symlinked_config_file() {
    use std::os::unix::fs::symlink;

    let home = tempfile::tempdir().expect("home");
    let config_directory = home.path().join(".jackyzhang.app").join("easybooks");
    std::fs::create_dir_all(&config_directory).expect("create config directory");
    let outside = home.path().join("outside-config.json");
    std::fs::write(&outside, "must remain unchanged").expect("write outside target");
    symlink(&outside, config_directory.join("config.json")).expect("create config symlink");

    easybooks()
        .env("HOME", home.path())
        .env("JACKYZHANG_APP_HOME", home.path().join(".jackyzhang.app"))
        .args(["login", "--token-stdin"])
        .write_stdin("jz_symlink_file\n")
        .assert()
        .failure()
        .stdout(predicate::str::contains("jz_symlink_file").not())
        .stderr(predicate::str::contains("jz_symlink_file").not());
    easybooks()
        .env("HOME", home.path())
        .env("JACKYZHANG_APP_HOME", home.path().join(".jackyzhang.app"))
        .arg("whoami")
        .assert()
        .failure();

    assert_eq!(
        std::fs::read_to_string(outside).expect("read outside target"),
        "must remain unchanged"
    );
}

#[test]
fn login_rejects_oversized_stdin() {
    let home = tempfile::tempdir().expect("home");
    easybooks()
        .env("HOME", home.path())
        .env("JACKYZHANG_APP_HOME", home.path().join(".jackyzhang.app"))
        .args(["login", "--token-stdin"])
        .write_stdin("x".repeat(4097))
        .assert()
        .failure();
    assert!(!runtime_config_path(home.path()).exists());
}

/// Read back the `base_url` persisted to the unified runtime config.
/// Tests run the binary with an isolated `HOME`, so this is the exact value
/// `login` wrote to disk (not the echoed stdout, though they must agree).
fn runtime_config_path(home: &std::path::Path) -> std::path::PathBuf {
    home.join(".jackyzhang.app")
        .join("easybooks")
        .join("config.json")
}

fn persisted_base_url(home: &std::path::Path) -> String {
    let cfg = runtime_config_path(home);
    let data = std::fs::read_to_string(&cfg).expect("config.json should exist after login");
    let json: serde_json::Value = serde_json::from_str(&data).expect("config.json is valid JSON");
    json["base_url"]
        .as_str()
        .expect("base_url is a string")
        .to_string()
}

const TEST_URL: &str = "https://easybooks-test.jackyzhang.app";
const PROD_URL: &str = "https://easybooks.jackyzhang.app";

/// login precedence (a): `$EASYBOOKS_API_URL` set + no `--base-url` →
/// persist the env value, NOT the PROD default. This is the governance footgun
/// the item-3 fix closes: previously the clap default silently won over env.
#[test]
fn login_uses_env_base_url_when_arg_absent() {
    let home = tempfile::tempdir().expect("tempdir");
    easybooks()
        .env("HOME", home.path())
        .env("JACKYZHANG_APP_HOME", home.path().join(".jackyzhang.app"))
        .env("EASYBOOKS_API_URL", TEST_URL)
        .args(["login", "--token-stdin"])
        .write_stdin("jz_envonly\n")
        .assert()
        .success()
        .stdout(predicate::str::contains(TEST_URL));

    assert_eq!(
        persisted_base_url(home.path()),
        TEST_URL,
        "with EASYBOOKS_API_URL set and no --base-url, login must persist the env value"
    );
}

/// login precedence (b): `--base-url` arg present overrides `$EASYBOOKS_API_URL`.
#[test]
fn login_arg_overrides_env_base_url() {
    let home = tempfile::tempdir().expect("tempdir");
    let arg_url = "http://192.168.1.69:8310";
    easybooks()
        .env("HOME", home.path())
        .env("JACKYZHANG_APP_HOME", home.path().join(".jackyzhang.app"))
        .env("EASYBOOKS_API_URL", TEST_URL)
        .args(["login", "--token-stdin", "--base-url", arg_url])
        .write_stdin("jz_argwins\n")
        .assert()
        .success()
        .stdout(predicate::str::contains(arg_url));

    assert_eq!(
        persisted_base_url(home.path()),
        arg_url,
        "--base-url must override EASYBOOKS_API_URL"
    );
}

/// login precedence (c): neither `--base-url` nor `$EASYBOOKS_API_URL` →
/// fall back to the PROD DEFAULT.
#[test]
fn login_defaults_to_prod_when_arg_and_env_absent() {
    let home = tempfile::tempdir().expect("tempdir");
    easybooks()
        .env("HOME", home.path())
        .env("JACKYZHANG_APP_HOME", home.path().join(".jackyzhang.app"))
        .env_remove("EASYBOOKS_API_URL")
        .args(["login", "--token-stdin"])
        .write_stdin("jz_default\n")
        .assert()
        .success()
        .stdout(predicate::str::contains(PROD_URL));

    assert_eq!(
        persisted_base_url(home.path()),
        PROD_URL,
        "with neither arg nor env, login must persist the PROD DEFAULT"
    );
}

#[test]
fn whoami_uses_integration_whoami_endpoint_and_reports_user_and_scope() {
    let mut accountd = mockito::Server::new();
    let (exchange, app_jwt) = mock_exchange(&mut accountd);
    let app_bearer = format!("Bearer {app_jwt}");
    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/api/integrations/whoami")
        .match_header("authorization", app_bearer.as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(format!(
            r#"{{"ok":true,"user_id":"{USER}","email":"user@example.com","scope":"read_write","source":"easybooks-integration"}}"#
        ))
        .create();

    let (_home, mut cmd) = product_cmd(&server, &accountd);
    cmd.args(["whoami"])
        .assert()
        .success()
        .stdout(predicate::str::contains(USER))
        .stdout(predicate::str::contains(r#""email": "user@example.com""#))
        .stdout(predicate::str::contains(r#""scope": "read_write""#))
        .stdout(predicate::str::contains(r#""api_key_masked": "jz_***""#));

    mock.assert();
    exchange.assert();
}

#[test]
fn categories_list_hits_categories_endpoint_with_type_and_bearer() {
    let mut accountd = mockito::Server::new();
    let (exchange, app_jwt) = mock_exchange(&mut accountd);
    let app_bearer = format!("Bearer {app_jwt}");
    let mut server = mockito::Server::new();
    let mock = server
        .mock("GET", "/api/integrations/categories")
        .match_query(Matcher::UrlEncoded("type".into(), "expense".into()))
        .match_header("authorization", app_bearer.as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"[]"#)
        .create();

    let (_home, mut cmd) = product_cmd(&server, &accountd);
    cmd.args(["categories", "list", "--type", "expense"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[]"));

    mock.assert();
    exchange.assert();
}

#[test]
fn invoice_create_dry_run_does_not_post() {
    let server = mockito::Server::new();
    let payload = r#"{"client":{"name":"Acme"},"issue_date":"2026-05-01","due_date":"2026-06-01","items":[{"description":"Work","quantity":1,"unit_price":100}]}"#;

    let (_home, mut cmd) = authed_home_cmd();
    cmd.env("EASYBOOKS_API_URL", server.url())
        .args(["invoice", "create", "--json", payload, "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""status": "dry_run""#))
        .stdout(predicate::str::contains("Acme"));
}

#[test]
fn invoice_send_posts_to_integration_route_with_bearer() {
    let mut accountd = mockito::Server::new();
    let (exchange, app_jwt) = mock_exchange(&mut accountd);
    let app_bearer = format!("Bearer {app_jwt}");
    let mut server = mockito::Server::new();
    let mock = server
        .mock("POST", "/api/integrations/invoice/inv_42/send")
        .match_header("authorization", app_bearer.as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"sent":true}"#)
        .create();

    let (_home, mut cmd) = product_cmd(&server, &accountd);
    cmd.args(["invoice", "send", "inv_42"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""sent": true"#));

    mock.assert();
    exchange.assert();
}

#[test]
fn missing_config_is_a_structured_error() {
    let home = tempfile::tempdir().expect("tempdir");
    // No user slot and no env override → readiness envelope (skills follow it).
    easybooks()
        .env("HOME", home.path())
        .env("JACKYZHANG_APP_HOME", home.path().join(".jackyzhang.app"))
        .env_remove("EASYBOOKS_API_URL")
        .args(["categories", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("jz.plugin.envelope.v1"))
        .stdout(predicate::str::contains("needs_agent"));
}

#[test]
fn tx_reclassify_posts_classification_and_learn_with_bearer() {
    let mut accountd = mockito::Server::new();
    let (exchange, app_jwt) = mock_exchange(&mut accountd);
    let app_bearer = format!("Bearer {app_jwt}");
    let mut server = mockito::Server::new();
    let mock = server
        .mock("POST", "/api/integrations/transactions/txn_77/reclassify")
        .match_header("authorization", app_bearer.as_str())
        .match_body(Matcher::JsonString(
            r#"{"classification":"mixed","learn":true}"#.to_string(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"ok":true,"classification":"mixed","learned":true}"#)
        .create();

    let (_home, mut cmd) = product_cmd(&server, &accountd);
    cmd.args(["tx", "reclassify", "txn_77", "--class", "mixed", "--learn"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""classification": "mixed""#));

    mock.assert();
    exchange.assert();
}

#[test]
fn tx_reclassify_defaults_learn_false() {
    let mut accountd = mockito::Server::new();
    let (exchange, app_jwt) = mock_exchange(&mut accountd);
    let app_bearer = format!("Bearer {app_jwt}");
    let mut server = mockito::Server::new();
    let mock = server
        .mock("POST", "/api/integrations/transactions/txn_77/reclassify")
        .match_header("authorization", app_bearer.as_str())
        .match_body(Matcher::JsonString(
            r#"{"classification":"personal","learn":false}"#.to_string(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"ok":true}"#)
        .create();

    let (_home, mut cmd) = product_cmd(&server, &accountd);
    cmd.args(["tx", "reclassify", "txn_77", "--class", "personal"])
        .assert()
        .success();

    mock.assert();
    exchange.assert();
}

#[test]
fn tx_attach_receipt_uploads_base64_and_prints_receipt_url() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("receipt.pdf");
    // "%PDF-1.4" → base64 "JVBERi0xLjQ=".
    std::fs::write(&file, b"%PDF-1.4").expect("write fixture");

    let mut accountd = mockito::Server::new();
    let (exchange, app_jwt) = mock_exchange(&mut accountd);
    let app_bearer = format!("Bearer {app_jwt}");
    let mut server = mockito::Server::new();
    let mock = server
        .mock("POST", "/api/integrations/transactions/txn_88/receipt")
        .match_header("authorization", app_bearer.as_str())
        .match_body(Matcher::JsonString(
            r#"{"filename":"receipt.pdf","content_type":"application/pdf","content_base64":"JVBERi0xLjQ="}"#
                .to_string(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"ok":true,"receipt_url":"https://cdn.example/receipts/txn_88.pdf"}"#)
        .create();

    let (_home, mut cmd) = product_cmd(&server, &accountd);
    cmd.args([
        "tx",
        "attach-receipt",
        "txn_88",
        "--file",
        file.to_str().unwrap(),
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains(
        "https://cdn.example/receipts/txn_88.pdf",
    ));

    mock.assert();
    exchange.assert();
}

#[test]
fn tx_attach_receipt_refuses_oversize_file_locally() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("big.png");
    // 10 MiB + 1 byte → over the local ceiling; must fail before any network.
    std::fs::write(&file, vec![0u8; 10 * 1024 * 1024 + 1]).expect("write fixture");

    // Point at an unroutable URL; if the CLI tried to POST it would hang/fail,
    // but the size guard must short-circuit first with a clear message.
    let (_home, mut cmd) = authed_home_cmd();
    cmd.env("EASYBOOKS_API_URL", "http://127.0.0.1:9")
        .args([
            "tx",
            "attach-receipt",
            "txn_88",
            "--file",
            file.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("10 MB limit"));
}

#[test]
fn tx_attach_receipt_rejects_unsupported_extension() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, b"hello").expect("write fixture");

    let (_home, mut cmd) = authed_home_cmd();
    cmd.env("EASYBOOKS_API_URL", "http://127.0.0.1:9")
        .args([
            "tx",
            "attach-receipt",
            "txn_88",
            "--file",
            file.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unsupported receipt type"));
}

#[test]
fn whoami_never_sends_raw_owner_token_to_product() {
    let mut accountd = mockito::Server::new();
    let (exchange, app_jwt) = mock_exchange(&mut accountd);
    let app_bearer = format!("Bearer {app_jwt}");
    let mut server = mockito::Server::new();
    let leaked = server
        .mock("GET", "/api/integrations/whoami")
        .match_header("authorization", OWNER_BEARER)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"ok":true,"user_id":"leaked","scope":"read"}"#)
        .expect(0)
        .create();
    let ok = server
        .mock("GET", "/api/integrations/whoami")
        .match_header("authorization", app_bearer.as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(format!(
            r#"{{"ok":true,"user_id":"{USER}","scope":"read_write","source":"easybooks-integration"}}"#
        ))
        .create();

    let (_home, mut cmd) = product_cmd(&server, &accountd);
    cmd.args(["whoami"])
        .assert()
        .success()
        .stdout(predicate::str::contains(USER));

    leaked.assert();
    ok.assert();
    exchange.assert();
}

#[test]
fn whoami_reexchanges_once_on_product_401() {
    let mut accountd = mockito::Server::new();
    let jwt = test_product_jwt("eb", &accountd.url());
    let app_bearer = format!("Bearer {jwt}");
    let exchange = accountd
        .mock("POST", "/v1/token/exchange")
        .match_header("authorization", OWNER_BEARER)
        .match_body(Matcher::JsonString(
            r#"{"aud":"eb","scopes":["read","write"]}"#.to_string(),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(format!(
            r#"{{"access_token":"{jwt}","token_type":"Bearer","expires_in":300}}"#
        ))
        .expect(2)
        .create();
    let mut server = mockito::Server::new();
    let unauthorized = server
        .mock("GET", "/api/integrations/whoami")
        .match_header("authorization", app_bearer.as_str())
        .with_status(401)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":"Unauthorized"}"#)
        .expect(2)
        .create();

    let (_home, mut cmd) = product_cmd(&server, &accountd);
    cmd.args(["whoami"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("401"));

    unauthorized.assert();
    exchange.assert();
}

#[test]
fn env_api_key_cannot_supply_token() {
    let home = tempfile::tempdir().expect("tempdir");
    easybooks()
        .env("HOME", home.path())
        .env("JACKYZHANG_APP_HOME", home.path().join(".jackyzhang.app"))
        .env("EASYBOOKS_API_KEY", KEY)
        .args(["whoami"])
        .assert()
        .success()
        .stdout(predicate::str::contains("jz.plugin.envelope.v1"))
        .stdout(predicate::str::contains("needs_agent"));
}

#[test]
fn feedback_offline_is_local_mirror_only() {
    let home = tempfile::tempdir().expect("tempdir");
    write_user_token(home.path(), KEY);
    easybooks()
        .env("HOME", home.path())
        .env("JACKYZHANG_APP_HOME", home.path().join(".jackyzhang.app"))
        .env("EASYBOOKS_ACCOUNTD_URL", "http://127.0.0.1:1")
        .args([
            "feedback",
            "create",
            "--title",
            "offline test",
            "--description",
            "accountd unreachable",
            "--kind",
            "bug-report",
            "--idempotency-key",
            "offline-test-1",
            "--user-confirmed",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains(r#""status": "received""#).not())
        .stderr(predicate::str::contains("not_delivered"))
        .stderr(predicate::str::contains("local_mirror"));
}

/// Mock accountd `events:batch` that records every ingest body it receives.
fn capture_signals(
    accountd: &mut mockito::Server,
) -> (
    mockito::Mock,
    std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
) {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = seen.clone();
    let mock = accountd
        .mock("POST", "/v1/products/easybooks/events:batch")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body_from_request(move |request| {
            let body: serde_json::Value =
                serde_json::from_slice(request.body().expect("signals body")).expect("json");
            sink.lock().unwrap().push(body);
            b"{}".to_vec()
        })
        .create();
    (mock, seen)
}

fn only_event(seen: &std::sync::Mutex<Vec<serde_json::Value>>) -> serde_json::Value {
    let batches = seen.lock().unwrap();
    assert_eq!(batches.len(), 1, "expected one signals batch: {batches:?}");
    let events = batches[0]["events"].as_array().expect("events array");
    assert_eq!(events.len(), 1);
    events[0].clone()
}

fn step_summary(event: &serde_json::Value) -> Vec<(String, String)> {
    event["properties"]["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .map(|s| {
            assert!(s["duration_ms"].as_f64().expect("duration_ms") >= 0.0);
            (
                s["name"].as_str().unwrap().to_string(),
                s["status"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn expense_add_post_failure_reports_redacted_diagnostics() {
    let mut accountd = mockito::Server::new();
    let _exchange = mock_exchange(&mut accountd);
    let (signals, seen) = capture_signals(&mut accountd);
    let mut server = mockito::Server::new();
    server
        .mock("POST", "/api/integrations/ingest/transactions")
        .with_status(500)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{"error":"vendor Acme rejected 120.00 for bob@example.com at /Users/jacky/books token=abc123secret"}"#,
        )
        .create();

    let (_home, mut cmd) = product_cmd(&server, &accountd);
    cmd.env("EASYBOOKS_PORTAL_OFFLINE", "0")
        .env_remove("EASYBOOKS_PRODUCT_SIGNALS_ENABLED")
        .args([
            "expense",
            "add",
            "--amount",
            "120.00",
            "--description",
            "Software subscription",
            "--date",
            "2026-05-01",
            "--source-id",
            "rcpt-123",
        ])
        .assert()
        .failure();

    signals.assert();
    let event = only_event(&seen);
    assert_eq!(event["event_name"], "posting_completed");
    assert_eq!(event["outcome"], "failed");
    assert_eq!(event["feature_id"], "expense");
    assert_eq!(event["error_code"], "posting_failed");
    let props = &event["properties"];
    let message = props["error_message"].as_str().expect("error_message");
    assert!(message.contains("HTTP 500"), "{message}");
    let stack = props["error_stack"].as_str().expect("error_stack");
    let wire = event.to_string();
    for leak in [
        "Acme",
        "120.00",
        "12000",
        "bob@example.com",
        "jacky",
        "abc123secret",
        "Software subscription",
        "rcpt-123",
    ] {
        assert!(!wire.contains(leak), "{leak} leaked: {wire}");
    }
    assert!(stack.len() <= 8192 && message.len() <= 2048);
    assert_eq!(
        props["metadata"]["plugin_version"],
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(props["metadata"]["input_summary"], "1 expense entry");
    assert_eq!(props["context"]["entry_count"], 1);
    assert_eq!(
        step_summary(&event),
        [
            ("validate".to_string(), "succeeded".to_string()),
            ("balance".to_string(), "succeeded".to_string()),
            ("post".to_string(), "failed".to_string()),
        ]
    );
    assert_eq!(props["steps"][2]["error_code"], "posting_failed");
}

#[test]
fn invoice_create_validation_failure_is_reported() {
    let mut accountd = mockito::Server::new();
    let _exchange = mock_exchange(&mut accountd);
    let (signals, seen) = capture_signals(&mut accountd);
    let server = mockito::Server::new();

    let (_home, mut cmd) = product_cmd(&server, &accountd);
    cmd.env("EASYBOOKS_PORTAL_OFFLINE", "0")
        .env_remove("EASYBOOKS_PRODUCT_SIGNALS_ENABLED")
        .args([
            "invoice",
            "create",
            "--json",
            r#"{"client":{"name":"Jane Doe"},"issue_date":"2026-05-01","due_date":"2026-05-31","items":[]}"#,
        ])
        .assert()
        .failure();

    signals.assert();
    let event = only_event(&seen);
    assert_eq!(event["event_name"], "invoice_created");
    assert_eq!(event["outcome"], "failed");
    assert_eq!(event["error_code"], "invoice_invalid");
    let message = event["properties"]["error_message"].as_str().unwrap();
    assert!(
        message.contains("`items` must be a non-empty array"),
        "{message}"
    );
    assert!(!event.to_string().contains("Jane"));
    assert_eq!(
        step_summary(&event),
        [("build".to_string(), "failed".to_string())]
    );
}

#[test]
fn invoice_create_success_reports_build_number_persist_steps() {
    let mut accountd = mockito::Server::new();
    let _exchange = mock_exchange(&mut accountd);
    let (signals, seen) = capture_signals(&mut accountd);
    let mut server = mockito::Server::new();
    server
        .mock("POST", "/api/integrations/ingest/invoice")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"invoice_id":"9f0c0d3e-0000-4000-8000-000000000001","invoice_number":"INV-0042","total":2260.0,"created":true}"#)
        .create();

    let (_home, mut cmd) = product_cmd(&server, &accountd);
    cmd.env("EASYBOOKS_PORTAL_OFFLINE", "0")
        .env_remove("EASYBOOKS_PRODUCT_SIGNALS_ENABLED")
        .args([
            "invoice",
            "create",
            "--json",
            r#"{"client":{"client_id":"11111111-1111-1111-1111-111111111111"},"issue_date":"2026-05-01","due_date":"2026-05-31","items":[{"description":"Consulting","quantity":2,"unit_price":1000}],"source_id":"inv-src-1"}"#,
        ])
        .assert()
        .success();

    signals.assert();
    let event = only_event(&seen);
    assert_eq!(event["outcome"], "succeeded");
    assert!(event.get("error_code").is_none());
    let props = &event["properties"];
    assert!(props.get("error_message").is_none() && props.get("error_stack").is_none());
    assert_eq!(
        props["metadata"]["input_summary"],
        "1 line items; client by client_id"
    );
    assert_eq!(props["context"]["idempotency_key_kind"], "source_id");
    assert_eq!(props["context"]["created"], true);
    let wire = event.to_string();
    for leak in ["Consulting", "2260", "INV-0042", "inv-src-1", "1000"] {
        assert!(!wire.contains(leak), "{leak} leaked: {wire}");
    }
    assert_eq!(
        step_summary(&event),
        [
            ("build".to_string(), "succeeded".to_string()),
            ("number".to_string(), "succeeded".to_string()),
            ("persist".to_string(), "succeeded".to_string()),
        ]
    );
}

#[test]
fn dry_run_validation_failure_emits_no_signal() {
    let mut accountd = mockito::Server::new();
    let _exchange = mock_exchange(&mut accountd);
    let (signals, seen) = capture_signals(&mut accountd);
    let server = mockito::Server::new();
    let (_home, mut cmd) = product_cmd(&server, &accountd);
    cmd.env("EASYBOOKS_PORTAL_OFFLINE", "0")
        .env_remove("EASYBOOKS_PRODUCT_SIGNALS_ENABLED")
        .args([
            "tx",
            "import-json",
            "--json",
            "{\"entries\":[]}",
            "--dry-run",
        ])
        .assert()
        .failure();
    assert!(seen.lock().unwrap().is_empty());
    drop(signals);
}

#[test]
fn rule_create_failure_reports_error_and_timed_step() {
    let mut accountd = mockito::Server::new();
    let _exchange = mock_exchange(&mut accountd);
    let (signals, seen) = capture_signals(&mut accountd);
    let mut server = mockito::Server::new();
    server
        .mock("POST", "/api/integrations/rules")
        .with_status(422)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":"vendor Acme Plumbing for bob@example.com token=abc123secret"}"#)
        .create();

    let (_home, mut cmd) = product_cmd(&server, &accountd);
    cmd.env("EASYBOOKS_PORTAL_OFFLINE", "0")
        .env_remove("EASYBOOKS_PRODUCT_SIGNALS_ENABLED")
        .args([
            "rules",
            "create",
            "--json",
            r#"{"match":"Acme Plumbing","category":"Repairs"}"#,
        ])
        .assert()
        .failure();

    signals.assert();
    let event = only_event(&seen);
    assert_eq!(event["event_name"], "rule_created");
    assert_eq!(event["outcome"], "failed");
    assert_eq!(event["error_code"], "rule_failed");
    let props = &event["properties"];
    let message = props["error_message"].as_str().expect("error_message");
    assert!(message.contains("HTTP 422"), "{message}");
    assert!(props["error_stack"].is_string());
    assert_eq!(
        props["metadata"]["plugin_version"],
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(props["context"]["operation"], "rule_create");
    let wire = event.to_string();
    for leak in ["Acme", "bob@example.com", "abc123secret", "Repairs"] {
        assert!(!wire.contains(leak), "{leak} leaked: {wire}");
    }
    assert_eq!(
        step_summary(&event),
        [("post".to_string(), "failed".to_string())]
    );
    assert_eq!(props["steps"][0]["error_code"], "rule_failed");
}

#[test]
fn invoice_send_success_reports_timed_step_without_error_fields() {
    let mut accountd = mockito::Server::new();
    let _exchange = mock_exchange(&mut accountd);
    let (signals, seen) = capture_signals(&mut accountd);
    let mut server = mockito::Server::new();
    server
        .mock("POST", "/api/integrations/invoice/inv_42/send")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"sent":true}"#)
        .create();

    let (_home, mut cmd) = product_cmd(&server, &accountd);
    cmd.env("EASYBOOKS_PORTAL_OFFLINE", "0")
        .env_remove("EASYBOOKS_PRODUCT_SIGNALS_ENABLED")
        .args(["invoice", "send", "inv_42"])
        .assert()
        .success();

    signals.assert();
    let event = only_event(&seen);
    assert_eq!(event["event_name"], "invoice_sent");
    assert_eq!(event["outcome"], "succeeded");
    assert!(event.get("error_code").is_none());
    let props = &event["properties"];
    assert!(props.get("error_message").is_none() && props.get("error_stack").is_none());
    assert!(props["metadata"]["os"].is_string());
    assert_eq!(
        step_summary(&event),
        [("send".to_string(), "succeeded".to_string())]
    );
}

#[test]
fn doctor_backend_failure_reports_error_and_probe_step() {
    let mut accountd = mockito::Server::new();
    let _exchange = mock_exchange(&mut accountd);
    let (signals, seen) = capture_signals(&mut accountd);
    let mut server = mockito::Server::new();
    server
        .mock("GET", "/api/integrations/whoami")
        .with_status(500)
        .with_body("internal error for bob@example.com")
        .create();

    let (_home, mut cmd) = product_cmd(&server, &accountd);
    cmd.env("EASYBOOKS_PORTAL_OFFLINE", "0")
        .env_remove("EASYBOOKS_PRODUCT_SIGNALS_ENABLED")
        .args(["doctor"])
        .assert()
        .success();

    signals.assert();
    let event = only_event(&seen);
    assert_eq!(event["event_name"], "doctor_completed");
    assert_eq!(event["outcome"], "failed");
    assert_eq!(event["error_code"], "backend_error");
    let props = &event["properties"];
    let message = props["error_message"].as_str().expect("error_message");
    assert!(
        message.starts_with("backend probe failed (error): "),
        "{message}"
    );
    assert!(!event.to_string().contains("bob@example.com"));
    assert_eq!(props["context"]["backend_status"], "error");
    assert_eq!(
        step_summary(&event),
        [("backend_probe".to_string(), "failed".to_string())]
    );
    assert_eq!(props["steps"][0]["error_code"], "backend_error");
}
