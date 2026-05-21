use tokio::process::Command as TokioCommand;

use super::docker::{
    created_provider_actions, ensure_network, inspect, run_provider_command, start_existing,
};
use super::types::{ProviderCredentials, ProviderSpec};

pub(crate) async fn ensure(provider: &ProviderSpec) -> crate::Result<Vec<String>> {
    ensure_network().await?;
    if inspect(&provider.container).await {
        return start_existing(provider, "could not start redis provider").await;
    }
    run_provider_command(
        TokioCommand::new("docker")
            .arg("run")
            .arg("-d")
            .arg("--name")
            .arg(&provider.container)
            .arg("--restart")
            .arg("unless-stopped")
            .arg("--network")
            .arg("gumgum-network")
            .arg(&provider.image),
        "could not create redis provider",
    )
    .await?;
    Ok(created_provider_actions(provider))
}

pub(crate) async fn ensure_with_credentials(
    provider: &ProviderSpec,
    credentials: ProviderCredentials,
) -> crate::Result<Vec<String>> {
    ensure_network().await?;
    if inspect(&provider.container).await {
        return start_existing(provider, "could not start redis provider").await;
    }
    run_provider_command(
        TokioCommand::new("docker")
            .arg("run")
            .arg("-d")
            .arg("--name")
            .arg(&provider.container)
            .arg("--restart")
            .arg("unless-stopped")
            .arg("--network")
            .arg("gumgum-network")
            .arg(&provider.image)
            .arg("redis-server")
            .arg("--requirepass")
            .arg(credentials.password),
        "could not create redis provider",
    )
    .await?;
    Ok(created_provider_actions(provider))
}
