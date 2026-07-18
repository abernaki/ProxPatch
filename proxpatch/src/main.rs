mod calculations;
mod cli;
mod cluster;
mod config;
mod helpers;
mod logging;
mod migrate;
mod models;
mod nodes;
mod patch;
mod utils_proxlb;
mod version;
mod vms;

use clap::Parser;
use cli::Cli;
use crate::calculations::calculate_migrations_for_node;
use crate::calculations::apply_plan_to_cluster;
use crate::config::load_config;
use crate::cluster::val_cluster_status;
use crate::helpers::test_pkg_jq;
use crate::migrate::exec_migrate;
use crate::patch::exec_reboot;
use crate::patch::exec_upgrade;
use crate::patch::val_reboot;
use crate::patch::exec_enable_maintenance;
use crate::patch::exec_disable_maintenance;
use crate::utils_proxlb::is_package_proxlb_installed;
use crate::utils_proxlb::set_systemd_proxlb;
use log::{info, debug, warn, error};
use models::{NodeWithVms};
use nodes::get_nodes;
use nodes::wait_for_node_online;
use std::collections::HashMap;
use std::path::Path;
use version::VERSION;
use vms::get_running_vms;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    logging::init(cli.debug)?;

    info!("→ Starting ProxPatch run");
    if let Err(e) = run_proxpatch(&cli) {
        error!("Run failed: {}", e);
        return Err(e);
    }

    info!("✓ ProxPatch run finished");
    Ok(())
}

fn run_proxpatch(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    info!("→ Starting ProxPatch v{}... (https://gyptazy.com/proxpatch/)", VERSION);
    test_pkg_jq();

    debug!("→ Validating for custom config file...");
    let config = if let Some(path) = cli.config.as_deref() {
        if Path::new(path).is_file() {
            debug!("→ Processing custom config file: {}", path);
            Some(load_config(path)?)
        } else {
            warn!("✗ Custom config file '{}' is not present. Processing with defaults.", path);
            None
        }
    } else {
        debug!("✓ No custom config file specified. Processing with defaults.");
        None
    };

    let dry_run = cli.dry_run;
    if dry_run {
        warn!("→ DRY RUN MODE: no upgrades, migrations, or reboots will actually be executed.");
    }

    debug!("→ Processing user validation...");
    let user = config.as_ref().map_or_else(
        || {
            debug!("✓ Using user: root");
            "root"
        },
        |c| {
            debug!("✓ Using user from config: {}", c.ssh_user);
            c.ssh_user.as_str()
        },
    );

    debug!("→ Processing ProxLB user config...");
    let deactivate_proxlb = match config.as_ref() {
        Some(c) => {
            debug!("✓ ProxLB is not used or will be ignored.");
            c.deactivate_proxlb
        }
        None => {
            debug!("✓ ProxLB will be auto-detected and stopped during patching if installed.");
            false
        }
    };

    let security_only = config.as_ref().map(|c| c.security_only).unwrap_or(false);
    if security_only {
        debug!("✓ security_only enabled — using unattended-upgrade instead of dist-upgrade.");
    }

    let excluded_nodes: &[String] = config.as_ref().map(|c| c.excluded_nodes.as_slice()).unwrap_or(&[]);
    let patch_only_nodes: &[String] = config.as_ref().map(|c| c.patch_only_nodes.as_slice()).unwrap_or(&[]);
    let nodes = get_nodes()?;
    let mut cluster: HashMap<String, NodeWithVms> = HashMap::new();

    for node in nodes {
        if excluded_nodes.contains(&node.node) {
            info!("→ Skipping excluded node: {}", node.node);
            continue;
        }

        let node_name = node.node.clone();
        let vms = get_running_vms(&node_name)?;

        cluster.insert(
            node_name.clone(),
            NodeWithVms {
                resources: node,
                vms,
            },
        );
    }

    for (node_name, data) in cluster.iter_mut() {
        let ssh_target = data.resources.ip.as_deref().unwrap_or(node_name);
        exec_upgrade(user, ssh_target, security_only, dry_run)?;
        data.resources.reboot_required = val_reboot(user, ssh_target)?;
    }

    let node_order: Vec<String> = cluster.keys().cloned().collect();

    for node_name in node_order {
        let reboot_required = cluster
            .get(&node_name)
            .map(|d| d.resources.reboot_required)
            .unwrap_or(false);

        if !reboot_required {
            continue;
        }

        if patch_only_nodes.contains(&node_name) {
            info!("→ {} needs a reboot but is in patch_only_nodes — leaving it for manual reboot.", node_name);
            continue;
        }

        let plans = calculate_migrations_for_node(&node_name, &cluster);

        if is_package_proxlb_installed() && !deactivate_proxlb {
            info!("→ ProxLB detected, stopping before patching…");
            if is_package_proxlb_installed() {
                set_systemd_proxlb("stop")?;
            }
        }

        for plan in plans {
            let from_ip = cluster
                .get(&plan.from)
                .and_then(|d| d.resources.ip.as_deref())
                .unwrap_or(&plan.from);

            exec_migrate(user, from_ip, &plan.from, &plan.to, plan.vmid, dry_run)?;
            apply_plan_to_cluster(&mut cluster, &plan);
        }

        if !val_cluster_status()? {
            error!("Cluster unhealthy after reboot of {}", node_name);
            return Err(format!("Cluster unhealthy. Not rebooting {}", node_name).into());
        }

        let ssh_target = cluster.get(&node_name).and_then(|d| d.resources.ip.as_deref()).unwrap_or(&node_name);

        exec_enable_maintenance(user, ssh_target, &node_name, dry_run)?;
        std::thread::sleep(std::time::Duration::from_secs(30));

        exec_reboot(user, ssh_target, dry_run)?;
        std::thread::sleep(std::time::Duration::from_secs(120));

        exec_disable_maintenance(user, ssh_target, &node_name, dry_run)?;
        std::thread::sleep(std::time::Duration::from_secs(30));

        if dry_run {
            info!("→ [DRY RUN] Skipping post-reboot online/health checks for {}", node_name);
        } else {
            if !wait_for_node_online(&node_name, 30)? {
                error!("Node {} did not come back online in time", node_name);
                return Err(format!("Node {} failed to rejoin cluster", node_name).into());
            }

            if !val_cluster_status()? {
                error!("Cluster unhealthy after reboot of {}", node_name);
                return Err(format!("Cluster unhealthy after reboot of {}", node_name).into());
            }
        }

    }

    if is_package_proxlb_installed() && !deactivate_proxlb {
        info!("→ ProxLB detected, starting after patching…");
        if is_package_proxlb_installed() {
            set_systemd_proxlb("start")?;
        }
    }

    info!("✓ All nodes up-to-date. Cluster healthy.");
    Ok(())

}
