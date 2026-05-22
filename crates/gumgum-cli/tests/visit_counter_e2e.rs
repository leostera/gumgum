use gumgum_api::ServerRecord;
use gumgum_core::{DeploymentDescriptor, load_worker_path};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

const SHARED_HOSTS: &[&str] = &["starbase2", "192.168.0.3"];
const SHARED_DOMAINS: &[&str] = &["leostera.dev", "leostera.test"];

#[test]
#[ignore = "requires an explicit isolated VM; set GUMGUM_E2E_HOST, GUMGUM_E2E_ROOT_DOMAIN, and GUMGUM_E2E_ARTIFACT_DIR"]
fn visit_counter_deploys_from_fixture_manifest() {
    let host = required_env("GUMGUM_E2E_HOST");
    let root_domain = required_env("GUMGUM_E2E_ROOT_DOMAIN");
    let artifact_dir = PathBuf::from(required_env("GUMGUM_E2E_ARTIFACT_DIR"));
    refuse_shared_targets(&host, &root_domain);

    let test_domain =
        env::var("GUMGUM_E2E_TEST_DOMAIN").unwrap_or_else(|_| format!("test.{root_domain}"));
    let server_name = env::var("GUMGUM_E2E_SERVER_NAME").unwrap_or_else(|_| "e2e".to_owned());
    let apply = env::var("GUMGUM_E2E_APPLY").is_ok_and(|value| value == "1");

    fs::create_dir_all(&artifact_dir).expect("create artifact dir");
    let fixture = copy_fixture_with_domains(&root_domain, &test_domain, &artifact_dir);
    let gumgum = env!("CARGO_BIN_EXE_gumgum");

    let server = ServerRecord {
        name: server_name.clone(),
        host: host.clone(),
        root_domain: root_domain.clone(),
        test_domain: test_domain.clone(),
        health_url: format!("http://{host}:4747/health"),
    };
    let api_descriptor = fixture_descriptor(&fixture, "api/gumgum.toml", &server);
    let worker_descriptor = fixture_descriptor(&fixture, "worker/gumgum.toml", &server);

    let mut transcript = Transcript::new(artifact_dir.clone());
    transcript.note(format!(
        "host={host} server={server_name} root_domain={root_domain} test_domain={test_domain} apply={apply} fixture={}",
        fixture.display()
    ));

    if !apply {
        transcript
            .note("plan-only rust E2E smoke; set GUMGUM_E2E_APPLY=1 to mutate the isolated VM");
        transcript.run(
            Command::new(gumgum)
                .arg("server")
                .arg("add")
                .arg(&host)
                .arg("--name")
                .arg(&server_name)
                .arg("--root-domain")
                .arg(&root_domain),
        );
        transcript.run_owned(in_fixture(
            gumgum,
            &fixture,
            ["deploy", "--host", &server_name],
        ));
        return;
    }

    assert_success(
        &transcript.run(
            Command::new(gumgum)
                .arg("server")
                .arg("add")
                .arg(&host)
                .arg("--name")
                .arg(&server_name)
                .arg("--root-domain")
                .arg(&root_domain),
        ),
    );
    assert_success(&transcript.run(Command::new(gumgum).arg("server").arg("capabilities").arg("list").arg("--host").arg(&server_name).arg("--require").arg("gumgum:events,gumgum:objects:create_preview,gumgum:bindings:create_preview,gumgum:bindings:delete,gumgum:objects:delete,gumgum:deployments:delete,gumgum:buckets:objects")));
    assert_success(
        &transcript.run_to_artifact(
            Command::new("ssh")
                .arg(&host)
                .arg("docker ps --format '{{.Names}} {{.Image}} {{.Status}}'"),
            "containers-before.txt",
        ),
    );

    // The point of this E2E is that the fixture manifests carry desired app shape;
    // after VM setup, `gumgum deploy` should do the heavy convergence work.
    assert_success(&transcript.run_owned(in_fixture(
        gumgum,
        &fixture,
        ["deploy", "--host", &server_name],
    )));

    assert_success(&transcript.run_to_artifact(
        &mut in_fixture(
            gumgum,
            &fixture,
            ["--json", "graph", "--host", &server_name],
        ),
        "graph-after-deploy.json",
    ));
    let api_route = api_descriptor
        .routes
        .first()
        .expect("api fixture declares test ingress route")
        .clone();
    let api_response = transcript.run_to_artifact(
        Command::new("curl")
            .arg("-kfsS")
            .arg("--resolve")
            .arg(format!("{api_route}:443:{host}"))
            .arg(format!("https://{api_route}/")),
        "api-response.txt",
    );
    assert_success(&api_response);
    assert_output_contains(&api_response, "Hello visitor");
    assert_success(
        &transcript.run_to_artifact(
            Command::new("ssh")
                .arg(&host)
                .arg("docker ps --format '{{.Names}} {{.Image}} {{.Status}}'"),
            "containers-after.txt",
        ),
    );
    let worker_container = worker_descriptor.container.clone();
    let worker_health = transcript.run_to_artifact(
        Command::new("ssh").arg(&host).arg(format!(
            "docker inspect -f '{{{{.State.Status}}}} {{{{if .State.Health}}}}{{{{.State.Health.Status}}}}{{{{end}}}}' {worker_container}"
        )),
        "worker-container-health.txt",
    );
    assert_success(&worker_health);
    assert_output_contains(&worker_health, "running");

    let env_output = transcript.run_to_artifact(
        &mut in_fixture(
            gumgum,
            &fixture,
            ["env", "--host", &server_name, "--qualified"],
        ),
        "env-qualified.txt",
    );
    assert_success(&env_output);
    assert_output_contains(&env_output, "VISIT_COUNTER_API_USER_COUNTERS");
    assert_output_contains(&env_output, "VISIT_COUNTER_WORKER_DATABASE_URL");
    assert_output_contains(&env_output, "gumgum-provider-redis-main");
    assert_output_contains(&env_output, "gumgum-provider-minio-main");
    assert_output_contains(&env_output, "gumgum-provider-redpanda-main");
    assert_output_contains(&env_output, "gumgum-provider-postgres-main");
    assert_no_shared_provider_dns(&env_output);

    let events = transcript.run_to_artifact(
        &mut in_fixture(
            gumgum,
            &fixture,
            [
                "events",
                "--host",
                &server_name,
                "--grouped",
                "--limit",
                "20",
            ],
        ),
        "events-grouped.txt",
    );
    assert_success(&events);
    assert_output_contains(&events, "operation");

    let logs = transcript.run_to_artifact(
        &mut in_fixture(
            gumgum,
            &fixture,
            ["logs", "--host", &server_name, "--tail", "20"],
        ),
        "logs-workspace.txt",
    );
    assert_success(&logs);
    assert_output_contains(&logs, "visit-counter-api:");
    assert_output_contains(&logs, "visit-counter-worker:");
    let follow_logs = transcript.run_to_artifact(
        &mut timeout_in_fixture(
            gumgum,
            &fixture,
            ["logs", "-f", "--host", &server_name, "--tail", "5"],
        ),
        "logs-follow-workspace.txt",
    );
    assert_status_code(&follow_logs, 124);
    assert_output_contains(&follow_logs, "visit-counter-api:");
    assert_output_contains(&follow_logs, "visit-counter-worker:");

    assert_success(&transcript.run_owned(in_fixture(
        gumgum,
        &fixture,
        [
            "bucket",
            "cp",
            "README.md",
            "visit-requests/e2e/README.md",
            "--host",
            &server_name,
        ],
    )));
    assert_success(&transcript.run_owned(in_fixture(
        gumgum,
        &fixture,
        [
            "bucket",
            "cp",
            "visit-requests/e2e/README.md",
            "visit-requests/e2e/README.copy.md",
            "--host",
            &server_name,
        ],
    )));
    let bucket_ls = transcript.run_to_artifact(
        &mut in_fixture(
            gumgum,
            &fixture,
            [
                "bucket",
                "ls",
                "visit-requests",
                "e2e/",
                "--host",
                &server_name,
            ],
        ),
        "bucket-ls.txt",
    );
    assert_success(&bucket_ls);
    assert_output_contains(&bucket_ls, "README");

    let rollback = transcript.run_to_artifact(
        &mut in_fixture(
            gumgum,
            &fixture,
            [
                "rollback",
                "api/gumgum.toml",
                "--host",
                &server_name,
                "--worker",
                "visit-counter-api",
                "--preview",
            ],
        ),
        "rollback-api-preview.txt",
    );
    assert_success(&rollback);
    let publish = transcript.run_to_artifact(
        &mut in_fixture(
            gumgum,
            &fixture,
            [
                "--dry-run",
                "publish",
                "api/gumgum.toml",
                "--host",
                &server_name,
            ],
        ),
        "publish-api-dry-run.txt",
    );
    assert_success(&publish);
    let guarded_delete = transcript.run_to_artifact(
        &mut in_fixture(
            gumgum,
            &fixture,
            ["bucket", "delete", "visit-requests", "--host", &server_name],
        ),
        "delete-guard-bucket.txt",
    );
    assert_failure(&guarded_delete);
    assert_output_contains(&guarded_delete, "object has active bindings");
    assert_success(&transcript.run_to_artifact(
        &mut in_fixture(
            gumgum,
            &fixture,
            [
                "bucket",
                "unbind",
                "visit-requests",
                "--to",
                "visit-counter-api",
                "--as",
                "VISIT_REQUESTS_BUCKET",
                "--host",
                &server_name,
            ],
        ),
        "cleanup-bucket-unbind-api.txt",
    ));
    assert_success(&transcript.run_to_artifact(
        &mut in_fixture(
            gumgum,
            &fixture,
            [
                "bucket",
                "unbind",
                "visit-requests",
                "--to",
                "visit-counter-worker",
                "--as",
                "VISIT_REQUESTS_BUCKET",
                "--host",
                &server_name,
            ],
        ),
        "cleanup-bucket-unbind-worker.txt",
    ));
    assert_success(&transcript.run_to_artifact(
        &mut in_fixture(
            gumgum,
            &fixture,
            ["bucket", "delete", "visit-requests", "--host", &server_name],
        ),
        "cleanup-bucket-delete.txt",
    ));
    transcript.write_checksums();
}

fn required_env(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("{name} is required for ignored E2E test"))
}

fn refuse_shared_targets(host: &str, root_domain: &str) {
    assert!(
        !SHARED_HOSTS.iter().any(|shared| host.contains(shared)),
        "refusing shared host {host}; use an isolated VM"
    );
    assert!(
        !SHARED_DOMAINS.contains(&root_domain),
        "refusing shared domain {root_domain}; use an isolated E2E domain"
    );
}

fn copy_fixture_with_domains(root_domain: &str, test_domain: &str, artifact_dir: &Path) -> PathBuf {
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/visit-counter");
    let destination = artifact_dir.join("fixture");
    if destination.exists() {
        fs::remove_dir_all(&destination).expect("clear previous fixture copy");
    }
    copy_dir(&source, &destination);
    for manifest in [
        destination.join("gumgum.toml"),
        destination.join("api/gumgum.toml"),
        destination.join("worker/gumgum.toml"),
    ] {
        let raw = fs::read_to_string(&manifest).expect("read fixture manifest");
        let patched = raw
            .replace("leostera.dev", root_domain)
            .replace("leostera.test", test_domain);
        fs::write(&manifest, patched).expect("write patched fixture manifest");
    }
    destination
}

fn copy_dir(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create fixture copy dir");
    for entry in fs::read_dir(source).expect("read fixture source") {
        let entry = entry.expect("read fixture entry");
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir(&source_path, &destination_path);
        } else {
            fs::copy(&source_path, &destination_path).expect("copy fixture file");
        }
    }
}

fn in_fixture<const N: usize>(gumgum: &str, fixture: &Path, args: [&str; N]) -> Command {
    let mut command = Command::new(gumgum);
    command.current_dir(fixture).args(args);
    command
}

fn timeout_in_fixture<const N: usize>(gumgum: &str, fixture: &Path, args: [&str; N]) -> Command {
    let mut command = Command::new("timeout");
    command
        .current_dir(fixture)
        .arg("12s")
        .arg(gumgum)
        .args(args);
    command
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed with status {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_failure(output: &Output) {
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_status_code(output: &Output, expected: i32) {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "unexpected status\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_output_contains(output: &Output, needle: &str) {
    let combined = combined_output(output);
    assert!(
        combined.contains(needle),
        "missing {needle:?} in\n{combined}"
    );
}

fn combined_output(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_no_shared_provider_dns(output: &Output) {
    let combined = combined_output(output);
    assert!(
        !combined.contains(".leostera.dev"),
        "provider env leaked shared domain values:\n{combined}"
    );
}

fn fixture_descriptor(
    fixture: &Path,
    manifest: &str,
    server: &ServerRecord,
) -> DeploymentDescriptor {
    let path = fixture.join(manifest);
    let manifest = load_worker_path(&path).expect("load patched fixture worker manifest");
    DeploymentDescriptor::from_manifest(&path, &manifest, Some(server), false)
}

struct Transcript {
    dir: PathBuf,
    log: String,
}

impl Transcript {
    fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            log: String::new(),
        }
    }

    fn note(&mut self, message: impl AsRef<str>) {
        self.log.push_str(message.as_ref());
        self.log.push('\n');
        self.flush();
    }

    fn run_owned(&mut self, mut command: Command) -> Output {
        self.run(&mut command)
    }

    fn run(&mut self, command: &mut Command) -> Output {
        self.log.push_str(&format!("+ {:?}\n", command));
        let output = command.output().expect("run command");
        self.record_output(&output);
        output
    }

    fn run_to_artifact(&mut self, command: &mut Command, name: &str) -> Output {
        self.log.push_str(&format!("+ {:?} > {name}\n", command));
        let output = command.output().expect("run command");
        fs::write(self.dir.join(name), &output.stdout).expect("write artifact stdout");
        fs::write(self.dir.join(format!("{name}.stderr")), &output.stderr)
            .expect("write artifact stderr");
        self.record_output(&output);
        output
    }

    fn record_output(&mut self, output: &Output) {
        self.log.push_str(&format!("status: {}\n", output.status));
        self.log.push_str("stdout:\n");
        self.log.push_str(&String::from_utf8_lossy(&output.stdout));
        self.log.push_str("\nstderr:\n");
        self.log.push_str(&String::from_utf8_lossy(&output.stderr));
        self.log.push_str("\n---\n");
        self.flush();
    }

    fn flush(&self) {
        fs::write(self.dir.join("transcript.log"), &self.log).expect("write transcript");
    }

    fn write_checksums(&self) {
        let mut summary = String::new();
        for entry in fs::read_dir(&self.dir).expect("read artifact dir") {
            let entry = entry.expect("read artifact entry");
            if entry.path().is_file() {
                let size = entry.metadata().expect("artifact metadata").len();
                summary.push_str(&format!(
                    "{}\t{} bytes\n",
                    entry.file_name().to_string_lossy(),
                    size
                ));
            }
        }
        fs::write(self.dir.join("checksums.txt"), summary).expect("write checksum summary");
    }
}

fn unique_suffix() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos()
        .to_string()
}

#[test]
fn fixture_domain_patching_replaces_shared_domains() {
    let dir = env::temp_dir().join(format!("gumgum-e2e-patch-{}", unique_suffix()));
    fs::create_dir_all(&dir).unwrap();
    let fixture = copy_fixture_with_domains("example.invalid", "test.example.invalid", &dir);
    let workspace = fs::read_to_string(fixture.join("gumgum.toml")).unwrap();
    assert!(!workspace.contains("leostera.dev"));
    assert!(!workspace.contains("leostera.test"));
    assert!(workspace.contains("example.invalid"));
    for manifest in ["api/gumgum.toml", "worker/gumgum.toml"] {
        let raw = fs::read_to_string(fixture.join(manifest)).unwrap();
        assert!(!raw.contains("leostera.dev"));
        assert!(!raw.contains("leostera.test"));
    }
    let _ = fs::remove_dir_all(dir);
}
