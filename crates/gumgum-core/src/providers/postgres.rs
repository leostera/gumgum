use tokio::process::Command as TokioCommand;

use super::docker::{
    created_provider_actions, ensure_network, inspect, run_provider_command, start_existing,
};
use super::types::{ProviderCredentials, ProviderSpec};

pub(crate) async fn ensure(
    provider: &ProviderSpec,
    credentials: ProviderCredentials,
) -> crate::Result<Vec<String>> {
    ensure_network().await?;
    if inspect(&provider.container).await {
        return start_existing(provider, "could not start postgres provider").await;
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
            .arg("-e")
            .arg(format!(
                "{}={}",
                credentials.username_env, credentials.username
            ))
            .arg("-e")
            .arg(format!(
                "{}={}",
                credentials.password_env, credentials.password
            ))
            .arg(&provider.image),
        "could not create postgres provider",
    )
    .await?;
    Ok(created_provider_actions(provider))
}
