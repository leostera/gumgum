use crate::{SetupArgs, progress};
use gumgum_api::{ServerRecord, SetupReport};
use gumgum_core::{
    ConfigStore, DaemonHealthClient, ErrorCode, GumgumError, GumgumInstaller, SetupOptions,
    SetupTarget, Subsystem, setup_actions,
};
pub(crate) async fn resolve_setup(args: SetupArgs) -> gumgum_core::Result<SetupTarget> {
    if args.host.is_none() {
        return Err(GumgumError::structured(
            Subsystem::Cli,
            ErrorCode::InvalidArgs,
            "gumgum setup now requires an explicit host",
        )
        .likely_cause("server setup is explicit; use `server add` to register hosts")
        .next_command("gumgum server add 0.0.0.0 --name local --root-domain <domain>")
        .next_command("gumgum server add <host> --name <name> --root-domain <domain>")
        .build());
    }
    GumgumInstaller::resolve_target(SetupOptions {
        host: args.host,
        name: args.name,
        user: args.user,
        root_domain: args.root_domain,
        test_domain: args.test_domain,
    })
    .await
}

pub(crate) async fn install_gumgumd(
    setup: SetupTarget,
    quiet: bool,
) -> gumgum_core::Result<SetupReport> {
    progress(quiet, "resolving setup target");
    if setup.local {
        progress(
            quiet,
            "installing local binary into ~/.gumgum/bin and daemon service into ~/.gumgum/daemon",
        );
        GumgumInstaller::install_local_user_service(quiet).await?;
        progress(
            quiet,
            format!("configuring host DNS for *.{}", setup.test_domain),
        );
        GumgumInstaller::configure_host_dns(&setup.test_domain, quiet).await?;
    } else {
        let target = setup.ssh_target();
        progress(quiet, format!("running remote bootstrap on {target}"));
        GumgumInstaller::run_remote_setup(&target, &setup, quiet).await?;
    }
    progress(quiet, "checking gumgumd health");
    DaemonHealthClient::wait_for_ping(&setup.host).await?;
    let health_url = format!("http://{}:7777/healthz", setup.host);
    ConfigStore::from_home_env()?.save_server(ServerRecord {
        name: setup.name.clone(),
        host: setup.host.clone(),
        root_domain: setup.root_domain.clone(),
        test_domain: setup.test_domain.clone(),
        health_url: health_url.clone(),
    })?;
    if !setup.local {
        progress(
            quiet,
            format!(
                "configuring local resolver for {} -> {}",
                setup.test_domain, setup.host
            ),
        );
        GumgumInstaller::configure_client_resolver(&setup.test_domain, &setup.host, quiet).await?;
    }
    Ok(SetupReport {
        ok: true,
        name: setup.name,
        host: setup.host,
        root_domain: setup.root_domain,
        test_domain: setup.test_domain,
        service: "gumgumd".to_owned(),
        health_url,
        actions: setup_actions(setup.local),
    })
}
