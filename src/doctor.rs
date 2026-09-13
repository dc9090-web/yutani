//! `yutani doctor`: report which Wayland protocols the compositor offers.

use cosmic::cctk::wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry,
};
use std::io;
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

/// The protocol table: one line per check.
fn write_checks(out: &mut impl io::Write, checks: &[Check]) -> io::Result<()> {
    writeln!(out, "{:<56} {:<9} found", "interface", "needed")?;
    for c in checks {
        let needed = if c.required { "required" } else { "optional" };
        let found = match c.found {
            Some(v) => format!("v{v}"),
            None => "MISSING".to_string(),
        };
        writeln!(out, "{:<56} {:<9} {}", c.interface, needed, found)?;
    }
    Ok(())
}

/// The GL line and the verdict.
fn write_verdict(out: &mut impl io::Write, gl_status: &str, ok: bool) -> io::Result<()> {
    writeln!(out, "\n{:<56} {}", "gl (rounded corners)", gl_status)?;
    if ok {
        writeln!(out, "\nOK: cosmic-comp advertises everything Yutani needs.")
    } else {
        writeln!(out, "\nMISSING required protocols — Yutani cannot run on this compositor.")
    }
}

/// `yutani doctor | head -1`: Rust ignores SIGPIPE, so once the reader has
/// gone every write fails with EPIPE. That is the reader's choice, not an
/// error — the report simply ends there. (`println!` would panic with
/// "failed printing to stdout" instead, turning a pipeline into a trace.)
fn tolerate_broken_pipe(r: io::Result<()>) -> io::Result<()> {
    match r {
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        r => r,
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
    let ok = all_required_present(&checks);
    let mut out = io::stdout().lock();
    tolerate_broken_pipe(write_checks(&mut out, &checks))?;

    // GPU thumbnail pass: EGL on the first render node + a tiny offscreen render.
    let gl_status = crate::backend::gl::open_render_node()
        .and_then(|gbm| {
            let gl = crate::backend::gl::Gl::new(&gbm)?;
            gl.self_test(&gbm)
        })
        .map(|()| "ok".to_string())
        .unwrap_or_else(|err| format!("unavailable: {err:#} (thumbnails will have square corners)"));
    tolerate_broken_pipe(write_verdict(&mut out, &gl_status, ok))?;

    Ok(if ok { ExitCode::SUCCESS } else { ExitCode::from(2) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `yutani doctor | head -1`: Rust ignores SIGPIPE, so once `head` has
    /// gone every write fails with EPIPE — which `println!` turned into a
    /// panic trace. The report goes through a fallible writer and a broken
    /// pipe ends it quietly; any other write error still surfaces.
    #[test]
    fn a_closed_pipe_ends_the_report_quietly_and_other_write_errors_still_surface() {
        struct Broken(std::io::ErrorKind);
        impl std::io::Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::from(self.0))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let checks = evaluate(&[]);
        let r = write_checks(&mut Broken(std::io::ErrorKind::BrokenPipe), &checks);
        assert!(r.is_err());
        assert!(tolerate_broken_pipe(r).is_ok(), "the reader went away: not an error");
        let r = write_verdict(&mut Broken(std::io::ErrorKind::Other), "ok", false);
        assert!(tolerate_broken_pipe(r).is_err(), "a real write failure must surface");

        let mut out = Vec::new();
        write_checks(&mut out, &checks).unwrap();
        write_verdict(&mut out, "ok", all_required_present(&checks)).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.starts_with("interface"), "{s}");
        assert!(s.contains("zwlr_layer_shell_v1") && s.contains("MISSING"), "{s}");
        assert!(s.contains("gl (rounded corners)") && s.contains("MISSING required protocols"), "{s}");
    }

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
