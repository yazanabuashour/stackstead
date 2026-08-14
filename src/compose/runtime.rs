use crate::manifest::StacksteadManifest;

use super::{
    claim::{ensure_runtime_claim, runtime_claim_exists, verify_runtime_claim},
    docker::{base_args, run_docker_compose},
    resources::{remove_labeled_runtime_resources, verify_runtime_resources},
};

pub fn up(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    let resources_present = verify_runtime_resources(manifest)?;
    if runtime_claim_exists(manifest)? {
        verify_runtime_claim(manifest)?;
    } else if resources_present {
        anyhow::bail!(
            "Compose namespace `{}` has runtime resources but no Stackstead ownership claim",
            manifest.compose_project
        );
    } else {
        ensure_runtime_claim(manifest)?;
    }
    verify_runtime_claim(manifest)?;
    verify_runtime_resources(manifest)?;
    let mut args = base_args(manifest);
    args.extend(["up".into(), "-d".into()]);
    run_docker_compose(manifest, &args)?;
    verify_runtime_resources(manifest)?;
    Ok(())
}

pub fn stop(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    let resources_present = verify_runtime_resources(manifest)?;
    if !runtime_claim_exists(manifest)? {
        if resources_present {
            anyhow::bail!(
                "Compose namespace `{}` has runtime resources but no Stackstead ownership claim",
                manifest.compose_project
            );
        }
        return Ok(());
    }
    verify_runtime_claim(manifest)?;
    if !resources_present {
        return Ok(());
    }
    let mut args = base_args(manifest);
    args.push("stop".into());
    run_docker_compose(manifest, &args)?;
    Ok(())
}

pub fn down_volumes(manifest: &StacksteadManifest) -> anyhow::Result<()> {
    let resources_present = verify_runtime_resources(manifest)?;
    if !runtime_claim_exists(manifest)? {
        if resources_present {
            anyhow::bail!(
                "Compose namespace `{}` has runtime resources but no Stackstead ownership claim",
                manifest.compose_project
            );
        }
        return Ok(());
    }
    verify_runtime_claim(manifest)?;
    if !resources_present {
        return Ok(());
    }
    let mut args = base_args(manifest);
    args.extend([
        "down".into(),
        "-v".into(),
        "--remove-orphans".into(),
        "--rmi".into(),
        "local".into(),
    ]);
    run_docker_compose(manifest, &args)?;
    if verify_runtime_resources(manifest)? {
        remove_labeled_runtime_resources(manifest)?;
    }
    if verify_runtime_resources(manifest)? {
        anyhow::bail!(
            "Compose namespace `{}` still has Stackstead runtime resources after teardown; retaining recovery state and ownership claim",
            manifest.compose_project
        );
    }
    Ok(())
}
