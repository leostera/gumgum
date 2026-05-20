use crate::{progress, server_client::ServerClient};
use gumgum_api::{BindingRequest, ObjectRequest, ServerRecord};
use gumgum_core::{Capability, ObjectBinding, WorkerManifest};

pub(crate) struct DeployExecutor<'a> {
    server: &'a ServerRecord,
    quiet: bool,
    client: ServerClient,
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
    ) -> gumgum_core::Result<()> {
        for db in &manifest.database {
            self.ensure_object_and_binding(Capability::Db, db, &manifest.worker.name)
                .await?;
        }
        for kv in &manifest.kv {
            self.ensure_object_and_binding(Capability::Kv, kv, &manifest.worker.name)
                .await?;
        }
        Ok(())
    }

    async fn ensure_object_and_binding(
        &self,
        capability: Capability,
        binding: &ObjectBinding,
        worker: &str,
    ) -> gumgum_core::Result<()> {
        let object_name = binding
            .dns
            .as_deref()
            .and_then(|dns| dns.split('.').next())
            .unwrap_or(&binding.name)
            .to_owned();
        progress(self.quiet, format!("ensuring {capability}/{object_name}"));
        let _ = self
            .client
            .create_object(&ObjectRequest {
                capability,
                name: object_name.clone(),
                namespace: "root".to_owned(),
                root_domain: self.server.root_domain.clone(),
            })
            .await;
        if let Some(env) = &binding.binding {
            progress(self.quiet, format!("ensuring binding {worker}.{env}"));
            let _ = self
                .client
                .bind_object(&BindingRequest {
                    capability,
                    object_name,
                    worker: worker.to_owned(),
                    binding: env.clone(),
                    access: binding
                        .access
                        .clone()
                        .unwrap_or_else(|| "read-write".to_owned()),
                })
                .await;
        }
        Ok(())
    }
}
