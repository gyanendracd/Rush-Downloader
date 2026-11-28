//! The real thing: yt-dlp and gallery-dl downloaded from their projects'
//! release pages, checked against the published sums and run once, into a
//! folder of the test's own. Needs the internet (about 50 MB), so it only
//! runs when asked for:
//! `cargo test -p rushdownload-engine --test tools -- --ignored`

use std::sync::Arc;

use rushdownload_engine::{Engine, MemoryStore, Settings, ToolSource};

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn installs_the_real_tools_from_their_release_pages() {
    let dir = std::env::temp_dir().join(format!("rushdownload-tools-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut settings = Settings::new(dir.join("out"));
    settings.auto_update_tools = false;
    let engine = Engine::with_secrets(dir.join("state"), settings, Arc::new(MemoryStore::default())).unwrap();
    for tool in engine.check_tools().await {
        assert!(tool.error.is_none(), "{}: {:?}", tool.name, tool.error);
        assert!(tool.latest.is_some(), "{}", tool.name);
        if !tool.can_install {
            continue;
        }
        let installed = engine.install_tool(&tool.name).await.unwrap_or_else(|e| panic!("{}: {e}", tool.name));
        println!("{}: {:?} at {:?}", installed.name, installed.version, installed.path);
        assert_eq!(installed.source, ToolSource::Managed);
        assert_eq!(installed.version, tool.latest);
        assert!(installed.path.as_ref().is_some_and(|p| p.is_file() && p.starts_with(dir.join("state"))));
        assert!(!installed.update_available);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
