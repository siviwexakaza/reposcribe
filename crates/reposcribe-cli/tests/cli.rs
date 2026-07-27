use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;

fn write(root: &std::path::Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

#[test]
fn version_alias_works() {
    Command::cargo_bin("reposcribe")
        .unwrap()
        .arg("--v")
        .assert()
        .success()
        .stdout(predicate::str::contains("reposcribe 0.1.0"));
}

#[test]
fn project_show_reports_a_monorepo() {
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir(repo.path().join(".git")).unwrap();
    write(
        repo.path(),
        "package.json",
        r#"{"name":"workspace","private":true,"workspaces":["apps/*"]}"#,
    );
    write(
        repo.path(),
        "apps/web/package.json",
        r#"{"name":"web","dependencies":{"next":"latest"}}"#,
    );
    write(repo.path(), "apps/api/Gemfile", "gem 'rails'\n");

    Command::cargo_bin("reposcribe")
        .unwrap()
        .args([
            "project",
            "show",
            "--repo",
            repo.path().to_str().unwrap(),
            "--no-color",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("monorepo (3 projects)"))
        .stdout(predicate::str::contains("web"))
        .stdout(predicate::str::contains("Rails"));
}

#[test]
fn commands_outside_a_repository_have_an_actionable_error() {
    let directory = tempfile::tempdir().unwrap();

    Command::cargo_bin("reposcribe")
        .unwrap()
        .args(["doctor", "--repo", directory.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not inside a Git repository"))
        .stderr(predicate::str::contains("pass --repo <path>"));
}

#[test]
fn configures_ai_without_saving_the_api_key() {
    let config_directory = tempfile::tempdir().unwrap();

    Command::cargo_bin("reposcribe")
        .unwrap()
        .args([
            "config",
            "ai",
            "--provider",
            "openai",
            "--model",
            "gpt-example",
            "--no-color",
        ])
        .env("REPOSCRIBE_CONFIG_DIR", config_directory.path())
        .env("OPENAI_API_KEY", "super-secret-test-key")
        .assert()
        .success()
        .stdout(predicate::str::contains("AI configuration saved"))
        .stdout(predicate::str::contains("gpt-example"))
        .stdout(predicate::str::contains("super-secret-test-key").not());

    let saved = fs::read_to_string(config_directory.path().join("config.toml")).unwrap();
    assert!(saved.contains("provider = \"openai\""));
    assert!(saved.contains("model = \"gpt-example\""));
    assert!(!saved.contains("super-secret-test-key"));
}

#[test]
fn missing_provider_key_has_an_actionable_error() {
    let config_directory = tempfile::tempdir().unwrap();

    Command::cargo_bin("reposcribe")
        .unwrap()
        .args([
            "config",
            "ai",
            "--provider",
            "anthropic",
            "--model",
            "claude-example",
            "--no-color",
        ])
        .env("REPOSCRIBE_CONFIG_DIR", config_directory.path())
        .env_remove("ANTHROPIC_API_KEY")
        .assert()
        .failure()
        .stderr(predicate::str::contains("ANTHROPIC_API_KEY"))
        .stderr(predicate::str::contains("export ANTHROPIC_API_KEY"));
}

#[test]
fn erd_requires_ai_configuration() {
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir(repo.path().join(".git")).unwrap();
    write(repo.path(), "schema.ts", "export const users = table();");
    let config = tempfile::tempdir().unwrap();

    Command::cargo_bin("reposcribe")
        .unwrap()
        .args([
            "erd",
            "--source",
            "schema.ts",
            "--repo",
            repo.path().to_str().unwrap(),
            "--no-color",
        ])
        .env("REPOSCRIBE_CONFIG_DIR", config.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("AI provider is required"))
        .stderr(predicate::str::contains("reposcribe config ai"));
}

#[test]
fn erd_requires_a_schema_file_or_model_directory() {
    Command::cargo_bin("reposcribe")
        .unwrap()
        .arg("erd")
        .assert()
        .failure()
        .stderr(predicate::str::contains("--source <FILE_OR_DIRECTORY>"));
}

#[tokio::test]
async fn erd_uses_ai_repository_analysis_and_renders_output() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

    let repo = tempfile::tempdir().unwrap();
    fs::create_dir(repo.path().join(".git")).unwrap();
    write(repo.path(), "schema.txt", "table users");
    let config = tempfile::tempdir().unwrap();
    write(
        config.path(),
        "config.toml",
        "[ai]\nprovider = \"openai\"\nmodel = \"gpt-example\"\n",
    );
    let destination = repo.path().join("reports/database");
    let server = MockServer::start().await;
    let analysis = serde_json::json!({
        "projects": [{
            "name": "shop",
            "root": ".",
            "framework": "AI detected framework",
            "database_technology": "AI detected database",
            "schema": {
                "name": "Shop database",
                "source_files": ["schema.txt"],
                "entities": [{
                    "name": "users",
                    "fields": [{
                        "name": "id",
                        "data_type": "integer",
                        "nullable": false,
                        "primary_key": true,
                        "unique": true
                    }]
                }],
                "relationships": []
            }
        }]
    });
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "output": [{
                "type": "function_call",
                "name": "submit_database_analysis",
                "call_id": "call_submit",
                "arguments": analysis.to_string()
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    Command::cargo_bin("reposcribe")
        .unwrap()
        .args([
            "erd",
            "--source",
            "schema.txt",
            "--repo",
            repo.path().to_str().unwrap(),
            "--output",
            "markdown",
            "--out",
            destination.to_str().unwrap(),
            "--no-color",
        ])
        .env("REPOSCRIBE_CONFIG_DIR", config.path())
        .env("OPENAI_API_KEY", "test-key")
        .env("OPENAI_BASE_URL", server.uri())
        .assert()
        .success()
        .stdout(predicate::str::contains("ERD created"))
        .stdout(predicate::str::contains("AI detected database"));

    let markdown = fs::read_to_string(destination.with_extension("md")).unwrap();
    assert!(markdown.contains("users"));
}

#[tokio::test]
async fn sequence_resolves_entry_through_ai_and_renders_output() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

    let repo = tempfile::tempdir().unwrap();
    fs::create_dir(repo.path().join(".git")).unwrap();
    write(repo.path(), "src/orders.rs", "fn create_order() {}");
    let config = tempfile::tempdir().unwrap();
    write(
        config.path(),
        "config.toml",
        "[ai]\nprovider = \"openai\"\nmodel = \"gpt-example\"\n",
    );
    let server = MockServer::start().await;
    let diagram = serde_json::json!({
        "name": "Create order sequence",
        "entry": "POST /orders",
        "source_files": ["src/orders.rs"],
        "mermaid": "sequenceDiagram\n  autonumber\n  participant C as Client\n  participant O as OrdersController.create_order()\n  C->>O: POST /orders\n  activate O\n  O-->>C: OrderResponse\n  deactivate O"
    });
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "output": [{
                "type": "function_call",
                "name": "submit_sequence_diagram",
                "call_id": "call_submit",
                "arguments": diagram.to_string()
            }]
        })))
        .mount(&server)
        .await;
    let destination = repo.path().join("reports/sequence");

    Command::cargo_bin("reposcribe")
        .unwrap()
        .args([
            "sequence",
            "--entry",
            "POST /orders",
            "--repo",
            repo.path().to_str().unwrap(),
            "--output",
            "markdown",
            "--out",
            destination.to_str().unwrap(),
            "--no-color",
        ])
        .env("REPOSCRIBE_CONFIG_DIR", config.path())
        .env("OPENAI_API_KEY", "test-key")
        .env("OPENAI_BASE_URL", server.uri())
        .assert()
        .success()
        .stdout(predicate::str::contains("Sequence diagram created"));

    let markdown = fs::read_to_string(destination.with_extension("md")).unwrap();
    assert!(markdown.contains("sequenceDiagram"));
    assert!(markdown.contains("Create order"));
}

#[tokio::test]
async fn flow_resolves_entry_through_ai_and_renders_output() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

    let repo = tempfile::tempdir().unwrap();
    fs::create_dir(repo.path().join(".git")).unwrap();
    write(repo.path(), "src/orders.rs", "fn create_order() {}");
    let config = tempfile::tempdir().unwrap();
    write(
        config.path(),
        "config.toml",
        "[ai]\nprovider = \"openai\"\nmodel = \"gpt-example\"\n",
    );
    let server = MockServer::start().await;
    let diagram = serde_json::json!({
        "name": "Create order flow",
        "entry": "create_order",
        "source_files": ["src/orders.rs"],
        "mermaid": "flowchart TD\n  A[create_order()] --> B{validate_order()}\n  B -->|valid| C[repository.save(order)]\n  B -->|invalid| D[return ValidationError]"
    });
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "output": [{
                "type": "function_call",
                "name": "submit_flow_diagram",
                "call_id": "call_submit",
                "arguments": diagram.to_string()
            }]
        })))
        .mount(&server)
        .await;
    let destination = repo.path().join("reports/flow");

    Command::cargo_bin("reposcribe")
        .unwrap()
        .args([
            "flow",
            "--entry",
            "create_order",
            "--repo",
            repo.path().to_str().unwrap(),
            "--output",
            "html",
            "--out",
            destination.to_str().unwrap(),
            "--no-color",
        ])
        .env("REPOSCRIBE_CONFIG_DIR", config.path())
        .env("OPENAI_API_KEY", "test-key")
        .env("OPENAI_BASE_URL", server.uri())
        .assert()
        .success()
        .stdout(predicate::str::contains("Flow diagram created"));

    let html = fs::read_to_string(destination.with_extension("html")).unwrap();
    assert!(html.contains("repository.save(order)"));
}
