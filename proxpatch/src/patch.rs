use std::process::{Command, Stdio};
use log::{info, debug, warn, error};

pub fn exec_upgrade(user: &str, node: &str, security_only: bool, dry_run: bool) -> Result<(), Box<dyn std::error::Error>> {
    debug!("→ Starting upgrade on node: {} (security_only={}, dry_run={})", node, security_only, dry_run);

    let inner_cmd = match (security_only, dry_run) {
        (true, true) => "unattended-upgrade --dry-run --debug",
        (true, false) => "unattended-upgrade",
        (false, true) => "apt-get update && apt-get -s dist-upgrade",
        (false, false) => "apt-get update && DEBIAN_FRONTEND=noninteractive apt-get -y dist-upgrade",
    };

    let remote_cmd = if user == "root" {
        inner_cmd.to_string()
    } else {
        format!("sudo sh -c \"{}\"", inner_cmd)
    };

    let output = Command::new("ssh")
        .args([
            "-o", "StrictHostKeyChecking=accept-new",
            "-o", "BatchMode=yes",
            &format!("{}@{}", user, node),
            &remote_cmd,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()?;

    if dry_run {
        if output.status.success() {
            info!("✓ [DRY RUN] Upgrade simulation completed on {}", node);
        } else {
            warn!("[DRY RUN] Upgrade simulation reported errors on {}", node);
        }
    } else if !output.status.success() {
        error!("✗ Upgrade failed on {}", node);
    } else {
        info!("✓ Upgrade completed on {}", node);
    }

    Ok(())
}

pub fn exec_reboot(user: &str, node: &str, dry_run: bool) -> Result<(), Box<dyn std::error::Error>> {
    debug!("→ Starting reboot for node: {}", node);

    if dry_run {
        info!("→ [DRY RUN] Would reboot node {}", node);
        return Ok(());
    }

    let remote_cmd = if user == "root" {
        String::from("reboot")
    } else {
        String::from("sudo reboot")
    };

    let output = Command::new("ssh")
        .args([
            "-o", "StrictHostKeyChecking=accept-new",
            "-o", "BatchMode=yes",
            &format!("{}@{}", user, node),
            &remote_cmd,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()?;

    if !output.status.success() {
        error!("✗ Reboot failed on {}", node);
    } else {
        info!("→ Rebooting node {}", node);
    }

    Ok(())
}

pub fn val_reboot(user: &str,node: &str) -> Result<bool, Box<dyn std::error::Error>> {
    debug!("→ Validating if reboot is required for node: {}", node);
    let output = Command::new("ssh")
        .args([
            "-o", "StrictHostKeyChecking=accept-new",
            "-o", "BatchMode=yes",
            &format!("{}@{}", user, node),
            "test -f /var/run/reboot-required",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()?;

    let reboot_required = output.status.success();

    if reboot_required {
        debug!("! Reboot required for node: {}", node);
    } else {
        debug!("✓ Reboot not required for node: {}", node);
    }

    Ok(reboot_required)
}

pub fn exec_enable_maintenance(user: &str, node: &str, node_name: &str, dry_run: bool) -> Result<(), Box<dyn std::error::Error>> {
    debug!("→ Setting node {} into maintenance mode.", node_name);

    if dry_run {
        info!("→ [DRY RUN] Would set node {} into maintenance mode.", node_name);
        return Ok(());
    }

    let remote_cmd = if user == "root" {
        format!("ha-manager crm-command node-maintenance enable {}", node_name)
    } else {
        format!("sudo ha-manager crm-command node-maintenance enable {}", node_name)
    };

    let output = Command::new("ssh")
        .args([
            "-o", "StrictHostKeyChecking=accept-new",
            "-o", "BatchMode=yes",
            &format!("{}@{}", user, node),
            &remote_cmd,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()?;

    if !output.status.success() {
        error!("✗ Unable to set node {} into maintenance mode.", node_name);
    } else {
        info!("✓ Node {} is now in maintenance mode.", node_name);
    }

    Ok(())
}

pub fn exec_disable_maintenance(user: &str, node: &str, node_name: &str, dry_run: bool) -> Result<(), Box<dyn std::error::Error>> {
    debug!("→ Disabling maintenance mode on node {}.", node_name);

    if dry_run {
        info!("→ [DRY RUN] Would disable maintenance mode on node {}.", node_name);
        return Ok(());
    }

    let remote_cmd = if user == "root" {
        format!("ha-manager crm-command node-maintenance disable {}", node_name)
    } else {
        format!("sudo ha-manager crm-command node-maintenance disable {}", node_name)
    };

    let output = Command::new("ssh")
        .args([
            "-o", "StrictHostKeyChecking=accept-new",
            "-o", "BatchMode=yes",
            &format!("{}@{}", user, node),
            &remote_cmd,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()?;

    if !output.status.success() {
        error!("✗ Unable to disable maintenance mode on node {}.", node_name);
    } else {
        info!("✓ Maintenance mode disabled on node {}.", node_name);
    }

    Ok(())
}
