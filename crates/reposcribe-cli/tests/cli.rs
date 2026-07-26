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
fn erd_generates_markdown_from_a_local_prisma_schema() {
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir(repo.path().join(".git")).unwrap();
    write(
        repo.path(),
        "package.json",
        r#"{"name":"shop","dependencies":{"@prisma/client":"latest"}}"#,
    );
    write(
        repo.path(),
        "prisma/schema.prisma",
        r#"
model Customer {
  id     Int     @id
  orders Order[]
}

model Order {
  id         Int      @id
  customerId Int
  customer   Customer @relation(fields: [customerId], references: [id])
}
"#,
    );
    let destination = repo.path().join("reports/shop-database");

    Command::cargo_bin("reposcribe")
        .unwrap()
        .args([
            "erd",
            "--repo",
            repo.path().to_str().unwrap(),
            "--output",
            "markdown",
            "--out",
            destination.to_str().unwrap(),
            "--no-color",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("ERD created"))
        .stdout(predicate::str::contains("Entities       2"));

    let rendered = fs::read_to_string(destination.with_extension("md")).unwrap();
    assert!(rendered.contains("erDiagram"));
    assert!(rendered.contains("Customer"));
    assert!(rendered.contains("Order"));
}

#[test]
fn erd_defaults_to_pdf() {
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir(repo.path().join(".git")).unwrap();
    write(repo.path(), "package.json", r#"{"name":"billing"}"#);
    write(
        repo.path(),
        "database/migrations/001_create_accounts.sql",
        "CREATE TABLE accounts (id INTEGER PRIMARY KEY, email TEXT UNIQUE NOT NULL);",
    );
    let destination = repo.path().join("reports/billing");

    Command::cargo_bin("reposcribe")
        .unwrap()
        .args([
            "erd",
            "--repo",
            repo.path().to_str().unwrap(),
            "--out",
            destination.to_str().unwrap(),
            "--no-color",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("billing.pdf"));

    let pdf = fs::read(destination.with_extension("pdf")).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
}

#[test]
fn erd_requires_a_project_choice_for_noninteractive_monorepos() {
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir(repo.path().join(".git")).unwrap();
    write(
        repo.path(),
        "package.json",
        r#"{"name":"workspace","private":true,"workspaces":["apps/*"]}"#,
    );
    for app in ["api", "admin"] {
        write(
            repo.path(),
            &format!("apps/{app}/package.json"),
            &format!(r#"{{"name":"{app}"}}"#),
        );
        write(
            repo.path(),
            &format!("apps/{app}/prisma/schema.prisma"),
            "model User { id Int @id }",
        );
    }

    Command::cargo_bin("reposcribe")
        .unwrap()
        .args(["erd", "--repo", repo.path().to_str().unwrap(), "--no-color"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Multiple databases were found"))
        .stderr(predicate::str::contains("--project"));
}
