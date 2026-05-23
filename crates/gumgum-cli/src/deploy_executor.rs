use crate::{progress, server_client::ServerClient};
use gumgum_api::{BindingReport, BindingRequest, ObjectReport, ObjectRequest, ServerRecord};
use gumgum_core::{Capability, ObjectBinding, QueueBinding, WorkerManifest};

pub(crate) struct DeployExecutor<'a> {
    server: &'a ServerRecord,
    quiet: bool,
    client: ServerClient,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ManifestBindingIntent {
    capability: Capability,
    object_name: String,
    namespace: String,
    root_domain: String,
    worker: String,
    env: Option<String>,
    access: String,
}

impl ManifestBindingIntent {
    fn object_request(&self) -> ObjectRequest {
        ObjectRequest {
            capability: self.capability,
            name: self.object_name.clone(),
            namespace: self.namespace.clone(),
            root_domain: self.root_domain.clone(),
            password: None,
            preview: false,
        }
    }

    fn binding_request(&self) -> Option<BindingRequest> {
        self.env.as_ref().map(|env| BindingRequest {
            capability: self.capability,
            object_name: self.object_name.clone(),
            worker: self.worker.clone(),
            binding: env.clone(),
            access: self.access.clone(),
            preview: false,
        })
    }
}

impl<'a> DeployExecutor<'a> {
    pub(crate) fn new(server: &'a ServerRecord, quiet: bool) -> Self {
        Self {
            server,
            quiet,
            client: ServerClient::new(&server.host),
        }
    }

    pub(crate) async fn ensure_manifest_bindings(
        &self,
        manifest: &WorkerManifest,
        namespace: Option<&str>,
        env: crate::DeployEnv,
    ) -> gumgum_core::Result<()> {
        for intent in manifest_binding_intents(manifest, self.server, namespace, env) {
            let label = format!("{}/{}", env.label(), intent.object_name);
            let report = self.client.create_object(&intent.object_request()).await?;
            progress(
                self.quiet,
                format!(
                    "{} {}/{} — {}",
                    object_status(&report),
                    intent.capability,
                    label,
                    report.message
                ),
            );
            if let Some(request) = intent.binding_request() {
                let report = self.client.bind_object(&request).await?;
                progress(
                    self.quiet,
                    format!(
                        "{} binding {}/{}.{} — {}",
                        binding_status(&report),
                        env.label(),
                        request.worker,
                        request.binding,
                        report.message
                    ),
                );
            }
        }
        Ok(())
    }
}

fn object_status(report: &ObjectReport) -> &'static str {
    if !report.ok {
        "errored"
    } else if report
        .provider_actions
        .iter()
        .any(|action| action.contains("already"))
    {
        "ok"
    } else {
        "changed"
    }
}

fn binding_status(report: &BindingReport) -> &'static str {
    if !report.ok {
        "errored"
    } else if report
        .binding_actions
        .iter()
        .any(|action| action.contains("already"))
    {
        "ok"
    } else {
        "changed"
    }
}

fn manifest_binding_intents(
    manifest: &WorkerManifest,
    server: &ServerRecord,
    namespace: Option<&str>,
    env: crate::DeployEnv,
) -> Vec<ManifestBindingIntent> {
    let namespace = namespace
        .map(ToOwned::to_owned)
        .or_else(|| {
            manifest
                .project
                .as_ref()
                .map(|project| project.namespace.clone())
        })
        .unwrap_or_else(|| "root".to_owned());
    let mut intents = Vec::new();
    extend_binding_intents(
        &mut intents,
        Capability::Db,
        &manifest.database,
        manifest,
        &namespace,
        server,
        env,
    );
    extend_binding_intents(
        &mut intents,
        Capability::Kv,
        &manifest.kv,
        manifest,
        &namespace,
        server,
        env,
    );
    extend_binding_intents(
        &mut intents,
        Capability::Blob,
        &manifest.bucket,
        manifest,
        &namespace,
        server,
        env,
    );
    extend_queue_binding_intents(&mut intents, manifest, &namespace, server, env);
    intents
}

fn extend_queue_binding_intents(
    intents: &mut Vec<ManifestBindingIntent>,
    manifest: &WorkerManifest,
    namespace: &str,
    server: &ServerRecord,
    env: crate::DeployEnv,
) {
    for (binding, access) in manifest.queue.iter_with_access() {
        intents.push(queue_binding_intent(
            binding, access, manifest, namespace, server, env,
        ));
    }
}

fn queue_binding_intent(
    binding: &QueueBinding,
    access: &str,
    manifest: &WorkerManifest,
    namespace: &str,
    server: &ServerRecord,
    env: crate::DeployEnv,
) -> ManifestBindingIntent {
    ManifestBindingIntent {
        capability: Capability::Queue,
        object_name: env_scoped_object_name(&binding.queue_id, env),
        namespace: namespace.to_owned(),
        root_domain: server.root_domain.clone(),
        worker: env_scoped_worker_name(&manifest.worker.name, env),
        env: Some(binding.binding.clone()),
        access: access.to_owned(),
    }
}

fn extend_binding_intents(
    intents: &mut Vec<ManifestBindingIntent>,
    capability: Capability,
    bindings: &[ObjectBinding],
    manifest: &WorkerManifest,
    namespace: &str,
    server: &ServerRecord,
    env: crate::DeployEnv,
) {
    for binding in bindings {
        intents.push(ManifestBindingIntent {
            capability,
            object_name: env_scoped_object_name(
                binding.object_id(capability).unwrap_or_default(),
                env,
            ),
            namespace: namespace.to_owned(),
            root_domain: server.root_domain.clone(),
            worker: env_scoped_worker_name(&manifest.worker.name, env),
            env: binding.binding.clone(),
            access: binding
                .access
                .clone()
                .unwrap_or_else(|| "read-write".to_owned()),
        });
    }
}

fn env_scoped_object_name(name: &str, env: crate::DeployEnv) -> String {
    format!("{}-{}", name, env.label())
}

fn env_scoped_worker_name(worker: &str, env: crate::DeployEnv) -> String {
    gumgum_core::sanitize_name(&format!("{}-{}", worker, env.label()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gumgum_core::{ObjectBinding, Project, Worker};

    fn server() -> ServerRecord {
        ServerRecord {
            name: "isolated".to_owned(),
            host: "192.0.2.10".to_owned(),
            root_domain: "example.invalid".to_owned(),
            test_domain: "test.example.invalid".to_owned(),
            health_url: "http://192.0.2.10:8080/health".to_owned(),
        }
    }

    fn binding(capability: Capability, id: &str, env: &str, access: &str) -> ObjectBinding {
        let mut binding = ObjectBinding {
            binding: Some(env.to_owned()),
            access: Some(access.to_owned()),
            ..Default::default()
        };
        match capability {
            Capability::Db => binding.db_id = Some(id.to_owned()),
            Capability::Kv => binding.kv_id = Some(id.to_owned()),
            Capability::Blob => binding.bucket_id = Some(id.to_owned()),
            Capability::Queue => binding.queue_id = Some(id.to_owned()),
            Capability::Secret => binding.secret_id = Some(id.to_owned()),
            Capability::Observability | Capability::Manual => {}
        }
        binding
    }

    #[test]
    fn manifest_binding_intents_cover_all_resource_capabilities() {
        let manifest = WorkerManifest {
            project: Some(Project {
                namespace: "visit-counter".to_owned(),
            }),
            worker: Worker {
                name: "visit-counter-api".to_owned(),
                image: None,
                build_context: Some(".".to_owned()),
                command: None,
                port: Some(3000),
                checks: Default::default(),
                health: Some("/healthz".to_owned()),
            },
            zone: Vec::new(),
            ingress: Vec::new(),
            database: vec![binding(
                Capability::Db,
                "visits",
                "DATABASE_URL",
                "read-write",
            )],
            kv: vec![binding(
                Capability::Kv,
                "user-counters",
                "USER_COUNTERS",
                "read-write",
            )],
            bucket: vec![binding(
                Capability::Blob,
                "visit-requests",
                "VISIT_REQUESTS_BUCKET",
                "read-write",
            )],
            queue: gumgum_core::QueueBindings {
                producer: vec![QueueBinding {
                    queue_id: "visit-events".to_owned(),
                    binding: "VISIT_EVENTS_QUEUE".to_owned(),
                }],
                consumer: Vec::new(),
            },
            observability: None,
            limits: None,
        };

        let intents =
            manifest_binding_intents(&manifest, &server(), None, crate::DeployEnv::Preview);
        assert_eq!(intents.len(), 4);
        assert_eq!(
            intents
                .iter()
                .map(|intent| (
                    intent.capability,
                    intent.object_name.as_str(),
                    intent.worker.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    Capability::Db,
                    "visits-preview",
                    "visit-counter-api-preview"
                ),
                (
                    Capability::Kv,
                    "user-counters-preview",
                    "visit-counter-api-preview",
                ),
                (
                    Capability::Blob,
                    "visit-requests-preview",
                    "visit-counter-api-preview",
                ),
                (
                    Capability::Queue,
                    "visit-events-preview",
                    "visit-counter-api-preview",
                ),
            ]
        );
        assert!(
            intents
                .iter()
                .all(|intent| intent.namespace == "visit-counter")
        );
        assert!(
            intents
                .iter()
                .all(|intent| intent.root_domain == "example.invalid")
        );
        assert!(
            intents
                .iter()
                .all(|intent| !intent.object_name.contains("leostera"))
        );
        assert_eq!(
            intents
                .iter()
                .find(|intent| intent.capability == Capability::Queue)
                .map(|intent| intent.access.as_str()),
            Some("write")
        );
    }

    #[test]
    fn manifest_binding_intents_default_access_and_skip_unbound_env() {
        let manifest = WorkerManifest {
            project: None,
            worker: Worker {
                name: "worker".to_owned(),
                image: Some("example/worker:latest".to_owned()),
                build_context: None,
                command: None,
                port: None,
                checks: Default::default(),
                health: None,
            },
            zone: Vec::new(),
            ingress: Vec::new(),
            database: Vec::new(),
            kv: vec![ObjectBinding {
                kv_id: Some("cache".to_owned()),
                binding: None,
                access: None,
                ..Default::default()
            }],
            bucket: Vec::new(),
            queue: Default::default(),
            observability: None,
            limits: None,
        };

        let intents = manifest_binding_intents(
            &manifest,
            &server(),
            Some("workspace-ns"),
            crate::DeployEnv::Prod,
        );
        assert_eq!(intents[0].namespace, "workspace-ns");
        assert_eq!(intents[0].access, "read-write");
        assert!(intents[0].binding_request().is_none());
        assert_eq!(intents[0].object_request().name, "cache-prod");
    }

    #[test]
    fn manifest_binding_intents_isolate_preview_and_prod_resources() {
        let manifest = WorkerManifest {
            project: None,
            worker: Worker {
                name: "api".to_owned(),
                image: Some("example/api:latest".to_owned()),
                build_context: None,
                command: None,
                port: Some(3000),
                checks: Default::default(),
                health: None,
            },
            zone: Vec::new(),
            ingress: Vec::new(),
            database: vec![binding(
                Capability::Db,
                "visits",
                "DATABASE_URL",
                "read-write",
            )],
            kv: Vec::new(),
            bucket: Vec::new(),
            queue: Default::default(),
            observability: None,
            limits: None,
        };

        let preview =
            manifest_binding_intents(&manifest, &server(), None, crate::DeployEnv::Preview);
        let prod = manifest_binding_intents(&manifest, &server(), None, crate::DeployEnv::Prod);

        assert_eq!(preview[0].object_name, "visits-preview");
        assert_eq!(preview[0].worker, "api-preview");
        assert_eq!(prod[0].object_name, "visits-prod");
        assert_eq!(prod[0].worker, "api-prod");
        let preview_request = preview[0].binding_request().unwrap();
        let prod_request = prod[0].binding_request().unwrap();
        assert_ne!(preview_request.object_name, prod_request.object_name);
        assert_ne!(preview_request.worker, prod_request.worker);
    }
}
