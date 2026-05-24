use crate::Capability;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CoreAction {
    CliMessage {
        message: String,
    },
    SetupStep {
        step: SetupStep,
    },
    PreviewOnly {
        scope: ActionScope,
    },
    AlreadyBound {
        worker: String,
        binding: String,
    },
    ProviderCredentialsRequired {
        provider: String,
    },
    ReconcileFailed {
        scope: ActionScope,
        error: String,
    },
    Planned {
        target: String,
        action: String,
    },
    ProviderConfigured {
        capability: Capability,
        provider: String,
    },
    ProviderObjectDesiredRemoved {
        capability: Capability,
        name: String,
    },
    ProviderContainerCreated {
        provider: String,
        container: String,
    },
    ProviderContainerStarted {
        provider: String,
    },
    ProviderContainerRecreated {
        provider: String,
    },
    PlatformServiceCreated {
        provider: String,
        container: String,
    },
    PlatformServiceStarted {
        container: String,
    },
    PlatformSecretServiceCreated {
        provider: String,
        container: String,
    },
    DnsPublished {
        dns: String,
        provider: String,
    },
    DnsRemoved {
        dns: String,
        provider: String,
    },
    DatabaseRoleEnsured {
        role: String,
    },
    DatabaseAlreadyExists {
        database: String,
    },
    DatabaseCreated {
        database: String,
    },
    DatabaseGranted {
        database: String,
        role: String,
    },
    DatabaseDropped {
        database: String,
    },
    DatabaseAlreadyAbsent {
        database: String,
    },
    RedisPrefixReserved {
        prefix: String,
    },
    RedisPrefixReleased {
        prefix: String,
    },
    BucketEnsured {
        bucket: String,
        provider: String,
    },
    BucketDeleted {
        bucket: String,
        provider: String,
    },
    BucketObjectUploaded {
        bucket: String,
        path: String,
        provider: String,
    },
    BucketObjectRemoved {
        bucket: String,
        path: String,
        provider: String,
    },
    BucketObjectCopied {
        source: String,
        destination: String,
        provider: String,
    },
    BucketObjectsSynced {
        source: String,
        destination: String,
        provider: String,
    },
    QueueTopicEnsured {
        topic: String,
        provider: String,
    },
    QueueTopicDeleted {
        topic: String,
        provider: String,
    },
    PrometheusScrapeConfigured {
        worker: String,
        environment: String,
        container: String,
        port: u16,
        metrics_path: String,
    },
    GrafanaDatasourceCreated {
        name: String,
    },
    GrafanaDatasourceUpdated {
        name: String,
    },
    GrafanaDashboardApplied {
        name: String,
    },
    DeploymentContainerMatches {
        container: String,
    },
    ImagePulled {
        image: String,
    },
    NetworkCreated {
        network: String,
    },
    DeploymentEnvironmentProjected {
        vars: usize,
    },
    DeploymentContainerStarted {
        container: String,
    },
    ContainerConnectedToNetwork {
        container: String,
        network: String,
    },
    DeploymentContainerHealthy {
        container: String,
    },
    DeploymentContainerRemoved {
        container: String,
    },
    CloudflareDnsCnameDeleted {
        hostname: String,
    },
    CloudflareDnsCnameAbsent {
        hostname: String,
    },
    CloudflareDnsCnameUnmanaged {
        hostname: String,
    },
    CloudflareConnectorEnsured {
        container: String,
    },
    CloudflareConnectorStarted {
        container: String,
    },
    CloudflareTunnelEnsured {
        tunnel: String,
    },
    CloudflareTunnelRouteEnsured {
        hostname: String,
        service: String,
    },
    CloudflareDnsCnameEnsured {
        hostname: String,
        target: String,
    },
    ManualDnsRequired {
        hostname: String,
        domain: String,
    },
    ManualDnsCleanupRequired {
        hostname: String,
        domain: String,
    },
    CloudflareDirectDnsUnsupported {
        hostname: String,
    },
    NoManagedDomainForStaleRoute {
        hostname: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupStep {
    CreateLocalDirectories,
    InstallRunningBinary,
    WriteUserSystemdService,
    EnableRestartDaemon,
    CheckLocalHealth,
    SshIntoHost,
    RunRemoteInstaller,
    RunRemoteSetup,
    ExitSsh,
    SaveServerLocally,
    CheckRemoteHealth,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionScope {
    Objects,
    Bindings,
    Deployment,
    Provider,
    Reconcile,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConnectionExample {
    PostgresPsql { name: String, dns: String },
    PgAdmin { name: String, dns: String },
    RedisCli { dns: String },
    RedisInsight { dns: String },
    AwsS3MakeBucket { name: String, dns: String },
    S3Environment { name: String, dns: String },
    KafkaCat { name: String, dns: String },
    KafkaEnvironment { name: String, dns: String },
    BitwardenCli { name: String },
    BitwardenUri { name: String },
    OtelEndpoint { dns: String },
}

pub type CoreActions = Vec<CoreAction>;
