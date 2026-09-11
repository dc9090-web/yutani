//! `yutani doctor`: report which Wayland protocols the compositor offers.

use cosmic::cctk::wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry,
};
use std::process::ExitCode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub interface: &'static str,
    pub required: bool,
    pub found: Option<u32>,
}

pub const REQUIRED: &[&str] = &[
    "ext_foreign_toplevel_list_v1",
    "zcosmic_toplevel_info_v1",
    "zcosmic_toplevel_manager_v1",
    "ext_image_copy_capture_manager_v1",
    "ext_foreign_toplevel_image_capture_source_manager_v1",
    "zwlr_layer_shell_v1",
    "zwp_linux_dmabuf_v1",
];

pub const OPTIONAL: &[&str] = &["zcosmic_overlap_notify_v1"];

/// Pure: match the advertised globals against what Yutani needs.
pub fn evaluate(globals: &[(String, u32)]) -> Vec<Check> {
    let lookup = |name: &str| {
        globals
            .iter()
            .find(|(iface, _)| iface == name)
            .map(|(_, version)| *version)
    };
    REQUIRED
        .iter()
        .map(|iface| Check { interface: iface, required: true, found: lookup(iface) })
        .chain(
            OPTIONAL
                .iter()
                .map(|iface| Check { interface: iface, required: false, found: lookup(iface) }),
        )
        .collect()
}

pub fn all_required_present(checks: &[Check]) -> bool {
    checks.iter().filter(|c| c.required).all(|c| c.found.is_some())
}

struct Doctor;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Doctor {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

pub fn run() -> anyhow::Result<ExitCode> {
    let conn = Connection::connect_to_env()?;
    let (globals, _queue) = registry_queue_init::<Doctor>(&conn)?;
    let advertised: Vec<(String, u32)> = globals
        .contents()
        .clone_list()
        .into_iter()
        .map(|g| (g.interface, g.version))
        .collect();

    let checks = evaluate(&advertised);
    println!("{:<56} {:<9} found", "interface", "needed");
    for c in &checks {
        let needed = if c.required { "required" } else { "optional" };
        let found = match c.found {
            Some(v) => format!("v{v}"),
            None => "MISSING".to_string(),
        };
        println!("{:<56} {:<9} {}", c.interface, needed, found);
    }

    if all_required_present(&checks) {
        println!("\nOK: cosmic-comp advertises everything Yutani needs.");
        Ok(ExitCode::SUCCESS)
    } else {
        println!("\nMISSING required protocols — Yutani cannot run on this compositor.");
        Ok(ExitCode::from(2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_interface_present_is_found_with_version() {
        let globals = vec![("zwlr_layer_shell_v1".to_string(), 5)];
        let checks = evaluate(&globals);
        let layer = checks
            .iter()
            .find(|c| c.interface == "zwlr_layer_shell_v1")
            .unwrap();
        assert!(layer.required);
        assert_eq!(layer.found, Some(5));
    }

    #[test]
    fn missing_interface_has_no_version() {
        let checks = evaluate(&[]);
        assert!(checks.iter().all(|c| c.found.is_none()));
        assert!(checks.iter().any(|c| c.interface == "zcosmic_overlap_notify_v1" && !c.required));
    }

    #[test]
    fn all_required_present_means_ok() {
        let globals: Vec<(String, u32)> = REQUIRED.iter().map(|i| (i.to_string(), 1)).collect();
        assert!(all_required_present(&evaluate(&globals)));
        assert!(!all_required_present(&evaluate(&globals[1..])));
    }
}
