use crate::{progress, server_client::ServerClient};
use gumgum_api::{BindingRequest, ObjectRequest, ServerRecord};
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
    ) -> gumgum_core::Result<()> {
        for intent in manifest_binding_intents(manifest, self.server, namespace) {
            progress(
                self.quiet,
                format!("ensuring {}/{}", intent.capability, intent.object_name),
            );
            self.client.create_object(&intent.object_request()).await?;
            if let Some(request) = intent.binding_request() {
                progress(
                    self.quiet,
                    format!("ensuring binding {}.{}", request.worker, request.binding),
                );
                self.client.bind_object(&request).await?;
            }
        }
        Ok(())
    }
}

fn manifest_binding_intents(
    manifest: &WorkerManifest,
    server: &ServerRecord,
    namespace: Option<&str>,
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
    );
    extend_binding_intents(
        &mut intents,
        Capability::Kv,
        &manifest.kv,
        manifest,
        &namespace,
        server,
    );
    extend_binding_intents(
        &mut intents,
        Capability::Blob,
        &manifest.bucket,
        manifest,
        &namespace,
        server,
    );
    extend_queue_binding_intents(&mut intents, manifest, &namespace, server);
    intents
}

fn extend_queue_binding_intents(
    intents: &mut Vec<ManifestBindingIntent>,
    manifest: &WorkerManifest,
    namespace: &str,
    server: &ServerRecord,
) {
    for (binding, access) in manifest.queue.iter_with_access() {
        intents.push(queue_binding_intent(
            binding, access, manifest, namespace, server,
        ));
    }
}

fn queue_binding_intent(
    binding: &QueueBinding,
    access: &str,
    manifest: &WorkerManifest,
    namespace: &str,
    server: &ServerRecord,
) -> ManifestBindingIntent {
    ManifestBindingIntent {
        capability: Capability::Queue,
        object_name: binding.queue_id.clone(),
        namespace: namespace.to_owned(),
        root_domain: server.root_domain.clone(),
        worker: manifest.worker.name.clone(),
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
) {
    for binding in bindings {
        intents.push(ManifestBindingIntent {
            capability,
            object_name: binding.object_id(capability).unwrap_or_default().to_owned(),
            namespace: namespace.to_owned(),
            root_domain: server.root_domain.clone(),
            worker: manifest.worker.name.clone(),
            env: binding.binding.clone(),
            access: binding
                .access
                .clone()
                .unwrap_or_else(|| "read-write".to_owned()),
        });
    }
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

        let intents = manifest_binding_intents(&manifest, &server(), None);
        assert_eq!(intents.len(), 4);
        assert_eq!(
            intents
                .iter()
                .map(|intent| (intent.capability, intent.object_name.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (Capability::Db, "visits"),
                (Capability::Kv, "user-counters"),
                (Capability::Blob, "visit-requests"),
                (Capability::Queue, "visit-events"),
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

        let intents = manifest_binding_intents(&manifest, &server(), Some("workspace-ns"));
        assert_eq!(intents[0].namespace, "workspace-ns");
        assert_eq!(intents[0].access, "read-write");
        assert!(intents[0].binding_request().is_none());
        assert_eq!(intents[0].object_request().name, "cache");
    }
}
