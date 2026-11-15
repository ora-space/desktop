//! The real InstallPlugins path admits processless MCP packages without an Agent kind filter.
use super::*;
use pretty_assertions::assert_eq;

/// A universal MCP archive needs no main.js and keeps its executable under assets.
#[test]
fn installs_mcp_packages_through_the_existing_plan_path() {
    ora_logging::with_trace_logging(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let root = tempfile::tempdir().unwrap();
                let source = root.path().join("mcp.orax");
                let mut zip = zip::ZipWriter::new(std::fs::File::create(&source).unwrap());
                for (name, bytes) in [
                    ("orax.toml", b"resolver = 1\nidentifier = \"ora-space.echo\"\nkind = \"mcp\"\nversion = \"1.0.0\"\ndescription = \"test\"\n".as_slice()),
                    ("assets/config.json", br#"{"schemaVersion":1,"transport":{"type":"stdio","command":"assets/server","args":[{"context":"workspace"}]}}"#.as_slice()),
                    ("assets/server", b"#!/bin/sh\nexit 0\n".as_slice()),
                ] {
                    zip.start_file(name, zip::write::SimpleFileOptions::default().unix_permissions(0o755)).unwrap();
                    zip.write_all(bytes).unwrap();
                }
                zip.finish().unwrap();
                let installer = PluginInstaller::new(
                    root.path().join("home"),
                    ArchiveDownload {
                        archive: source.clone(),
                        count: Arc::default(),
                    },
                    /*host_target*/ None,
                );
                assert_eq!(
                    installer.execute(&command(&source, "1.0.0"), &node()).await,
                    result(PluginItemOutcome::Installed {
                        version: PluginVersion::new("1.0.0")
                    })
                );
                let installed = installer
                    .catalog()
                    .installed(
                        &PluginId::new("official/ora-space.echo"),
                        &PluginVersion::new("1.0.0"),
                    )
                    .unwrap();
                assert!(!installed.join("main.js").exists());
                assert_eq!(
                    std::fs::read(installed.join("assets").join("server")).unwrap(),
                    b"#!/bin/sh\nexit 0\n"
                );
            });
    });
}
