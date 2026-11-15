// SPDX-License-Identifier: GPL-3.0-or-later
//! Cold management API for the native manager. No guest task or I/O enters here.
//! The GUI consumes descriptors and the original TOML codec, not another parser.
use super::*;
use serde_json::{json, Value};

fn field(
    path: &str,
    title: &str,
    section: &str,
    kind: &str,
    options: &[&str],
    help: &str,
) -> Value {
    json!({"path":path,"title":title,"section":section,"kind":kind,"options":options,"help":help})
}
fn fields() -> Vec<Value> {
    vec![
        field(
            "resources.cpus",
            "CPU cores",
            "Compute",
            "text",
            &[],
            "all, or a positive core count",
        ),
        field(
            "resources.memory",
            "Memory",
            "Compute",
            "text",
            &[],
            "auto shares host memory; a size is a demand-backed limit",
        ),
        field(
            "resources.memory_limit",
            "Optional hard limit",
            "Compute",
            "optional",
            &[],
            "Leave empty for no additional limit",
        ),
        field(
            "boot.mode",
            "Boot mode",
            "Startup and sessions",
            "choice",
            &["shell", "systemd", "android"],
            "Prepare systemd before selecting it",
        ),
        field(
            "session.shell",
            "Shell and arguments",
            "Startup and sessions",
            "argv",
            &[],
            "Each token is a literal argument, not shell syntax",
        ),
        field(
            "session.cwd",
            "Working directory",
            "Startup and sessions",
            "text",
            &[],
            "Directory used by new sessions",
        ),
        field(
            "display.windows",
            "Application windows",
            "Native components",
            "bool",
            &[],
            "Open native windows only when a guest application needs them",
        ),
        field(
            "components.vulkan",
            "Vulkan",
            "Native components",
            "switch",
            &["true", "false", "auto"],
            "Native GPU graphics and compute; guest loaders are separate",
        ),
        field(
            "components.vaapi",
            "Video acceleration",
            "Native components",
            "switch",
            &["true", "false", "auto"],
            "VideoToolbox through the VA-API adapter",
        ),
        field(
            "components.audio",
            "Audio output",
            "Native components",
            "bool",
            &[],
            "Use the default macOS output device",
        ),
        field(
            "components.audio_input",
            "Microphone input",
            "Native components",
            "bool",
            &[],
            "Also requires macOS microphone permission",
        ),
        field(
            "components.thunking",
            "Native function acceleration",
            "Native components",
            "choice",
            &["auto", "false"],
            "Mac runs ordinary instructions natively; interpreter thunks are not enabled",
        ),
        field(
            "network.mode",
            "Network mode",
            "Network connection",
            "choice",
            &["host", "bridge", "none"],
            "Host is the default; bridge joins a Trames-owned private network",
        ),
        field(
            "network.bridge",
            "Trames network",
            "Network connection",
            "network",
            &[],
            "Required only in bridge mode; no macOS bridge is created",
        ),
        field(
            "network.publish",
            "Published ports",
            "Network connection",
            "ports",
            &[],
            "Only bridge mode needs port publication",
        ),
    ]
}
fn capabilities() -> Vec<Value> {
    [
        ("clipboard", "Clipboard", "Exchange text with the Mac", "clipboard"),
        ("files", "Shared file control", "Only access authorized shared directories", "file-transfer"),
        ("mounts", "Dynamic mounts", "Only mount authorized share aliases", "mount-control"),
        ("ports", "Port management", "Manage publication for this environment", "port-forward"),
        ("host_ui", "Host UI actions", "Open the Trames interface from the guest", "host-control"),
        ("host_cli", "Trames CLI", "Read-only settings and this environment's lifecycle", "host-control"),
        ("host_exec", "Run Mac commands", "Execute commands as your macOS user", "host-exec"),
    ].into_iter().map(|(key,title,help,device)|json!({"key":key,"title":title,"help":help,"device":device,"command":key.replace('_',"-")})).collect()
}
fn revision(dir: &Path) -> Result<String> {
    // Configuration-only optimistic concurrency, outside all execution paths.
    let mut hash = Sha256::new();
    for file in ["instance.toml", "env.toml", "effective.toml"] {
        hash.update(file.as_bytes());
        match fs::read(dir.join(file)) {
            Ok(bytes) => {
                hash.update([1]);
                hash.update(bytes);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => hash.update([0]),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn inspect(dir: &Path) -> Result<Value> {
    let c = load(dir)?;
    let explicit = dir.join("env.toml").is_file();
    let (auth, grants_error) = match grants(dir, &c) {
        Ok(g) => (g, None),
        Err(e) => (model::Grant::default(), Some(e.to_string())),
    };
    Ok(
        json!({"config":c,"toml":model::encode(&c)?,"grants":auth,"grants_error":grants_error,
        "revision":revision(dir)?,"profile":dir.join("env.toml"),
        "source":if explicit {"host"} else {"guest"},
        "source_note":if explicit {"Explicit host profile"} else {"Guest profile, cached at last start; omitted values use defaults"}}),
    )
}
fn snapshot(r: &Path) -> Result<Value> {
    let mut environments = Vec::new();
    for row in rows(r)? {
        let mut value = serde_json::to_value(&row)?;
        if let Some(c) = &row.config {
            value["autostart"] =
                super::super::background::login_enabled(r, &c.environment.uuid).into();
        }
        match inspect(&row.path) {
            Ok(detail) => {
                value
                    .as_object_mut()
                    .unwrap()
                    .extend(detail.as_object().unwrap().clone());
            }
            Err(e) => {
                value["error"] = e.to_string().into();
            }
        }
        environments.push(value);
    }
    let (config, global_error) = match load_cli(r) {
        Ok(c) => (c, None),
        Err(e) => (CliConfig::default(), Some(e.to_string())),
    };
    let (networks, networks_error) = match super::super::networks::listing(r) {
        Ok(n) => (n, None),
        Err(e) => (Vec::new(), Some(e.to_string())),
    };
    Ok(
        json!({"version":1,"environments":environments,"networks":networks,"networks_error":networks_error,
        "global":config,"global_error":global_error,"global_path":r.join("config.toml"),"registry":r,
        "fields":fields(),"capabilities":capabilities(),"product":super::super::product::name()}),
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    version: u32,
    revision: String,
    changes: BTreeMap<String, Value>,
}
fn prepare(dir: &Path, request: &Patch) -> Result<(Config, Value)> {
    if request.version != 1 {
        return Err("unsupported manager protocol version".into());
    }
    if revision(dir)? != request.revision {
        return Err("Configuration changed outside the manager. Reload before saving; your draft was preserved.".into());
    }
    let before = load(dir)?;
    let mut after = before.clone();
    let mut changes = Vec::new();
    for (path, value) in &request.changes {
        // Installed image provenance and registered identity are not editable.
        if !(fields().iter().any(|f| f["path"] == *path)
            || path == "mounts"
            || capabilities()
                .iter()
                .any(|cap| path == &format!("interop.{}.enabled", cap["key"].as_str().unwrap())))
        {
            return Err(format!("unsupported manager configuration field: {path}").into());
        }
        let old = serde_json::to_value(&before)?;
        let old = path
            .split('.')
            .try_fold(&old, |v, k| v.get(k))
            .cloned()
            .unwrap_or(Value::Null);
        set_config(&mut after, path, &serde_json::to_string(value)?)?;
        if old != *value {
            changes.push(json!({"path":path,"before":old,"after":value}));
        }
    }
    validate(&after)?;
    // A bridge profile must refer to a real named network before we save it.
    if let Some(bridge) = &after.network.bridge {
        let r = dir
            .parent()
            .and_then(Path::parent)
            .ok_or("invalid environment registry")?;
        if !super::super::networks::listing(r)?
            .iter()
            .any(|b| b.name == *bridge)
        {
            return Err("selected Trames network does not exist".into());
        }
    }
    let result = json!({"version":1,"toml":model::encode(&after)?,"changes":changes});
    Ok((after, result))
}
pub(super) fn dispatch(args: &[String]) -> Result<u8> {
    let r = root()?;
    let response = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["snapshot"] => snapshot(&r)?,
        [op @ ("validate" | "save"), id] => {
            let mut bytes = Vec::new();
            io::stdin().take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > 1024 * 1024 {
                return Err("manager request exceeds 1 MiB".into());
            }
            let request: Patch = serde_json::from_slice(&bytes)?;
            let _guard = metadata_lock(&r)?;
            let dir = directory(&r, id)?;
            let (candidate, response) = prepare(&dir, &request)?;
            if *op == "save" {
                // Saving is cold metadata work. Never stop, admit or pause a
                // running guest merely to change its next-start configuration.
                save(&dir, &candidate)?;
            }
            response
        }
        _ => {
            return Err(
                "usage: _manager snapshot | validate UUID | save UUID (JSON on stdin)".into(),
            )
        }
    };
    println!("{}", serde_json::to_string(&response)?);
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("trames-manager-{}", model::uuid().unwrap()));
            for d in ["environments", "locks", "networks"] {
                private_dir(&p.join(d)).unwrap();
            }
            Self(p)
        }
        fn env(&self) -> PathBuf {
            let mut c = Config::default();
            c.environment.name = "work".into();
            c.environment.uuid = model::uuid().unwrap();
            let p = self.0.join("environments").join(&c.environment.uuid);
            private_dir(&p).unwrap();
            save(&p, &c).unwrap();
            p
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn staging_is_not_registered_and_publication_rechecks_concurrent_names() {
        let f = Fixture::new();
        let first = f.env();
        let stage = f.0.join("environments/.work-preparing");
        fs::rename(&first, &stage).unwrap();
        assert!(!directory(&f.0, "work").unwrap().exists());
        publish_environment(&f.0, "work", &stage, &first).unwrap();
        assert_eq!(directory(&f.0, "work").unwrap(), first);
        let other = f.env();
        let second_stage = f.0.join("environments/.work-second");
        fs::rename(&other, &second_stage).unwrap();
        assert!(publish_environment(&f.0, "work", &second_stage, &other)
            .unwrap_err()
            .to_string()
            .contains("registered while"));
        assert!(
            first.join("instance.toml").is_file() && second_stage.join("instance.toml").is_file()
        );
        assert!(!other.exists());
    }
    #[test]
    fn next_start_patch_does_not_need_the_running_environment_lock_or_mutate_grants() {
        let f = Fixture::new();
        let p = f.env();
        let c = load(&p).unwrap();
        let _running = lock(&f.0, &c.environment.uuid).unwrap();
        let grants_before = fs::read(p.join("grants.toml")).unwrap();
        let request = Patch {
            version: 1,
            revision: revision(&p).unwrap(),
            changes: BTreeMap::from([
                ("resources.memory".into(), json!("4GiB")),
                ("interop.host_exec.enabled".into(), json!(true)),
            ]),
        };
        let (after, _) = prepare(&p, &request).unwrap();
        save(&p, &after).unwrap();
        assert_eq!(load(&p).unwrap().resources.memory, "4GiB");
        assert_eq!(fs::read(p.join("grants.toml")).unwrap(), grants_before);
        assert!(!grants(&p, &after).unwrap().host_exec);
        assert!(prepare(&p, &request)
            .unwrap_err()
            .to_string()
            .contains("changed outside"));
    }
    #[test]
    fn invalid_batch_and_identity_changes_never_publish_a_partial_profile() {
        let f = Fixture::new();
        let p = f.env();
        let original = fs::read(p.join("env.toml")).unwrap();
        for changes in [
            BTreeMap::from([
                ("resources.memory".into(), json!("8GiB")),
                ("network.mode".into(), json!("bridge")),
            ]),
            BTreeMap::from([("environment.uuid".into(), json!("other"))]),
        ] {
            assert!(prepare(
                &p,
                &Patch {
                    version: 1,
                    revision: revision(&p).unwrap(),
                    changes
                }
            )
            .is_err());
            assert_eq!(fs::read(p.join("env.toml")).unwrap(), original);
        }
    }
    #[test]
    fn malformed_grants_and_networks_do_not_hide_environment_configuration() {
        let f = Fixture::new();
        let p = f.env();
        let corrupt = "not valid TOML =[";
        fs::write(p.join("grants.toml"), corrupt).unwrap();
        fs::write(f.0.join("networks/broken.toml"), corrupt).unwrap();
        let value = snapshot(&f.0).unwrap();
        assert!(value["networks_error"].is_string());
        let row = &value["environments"][0];
        assert!(row["config"].is_object() && row["revision"].is_string());
        assert!(row["grants_error"].is_string());
        assert_eq!(row["grants"]["host_exec"], false);
        assert_eq!(fs::read_to_string(p.join("grants.toml")).unwrap(), corrupt);
    }
    #[test]
    fn invalid_profiles_remain_discoverable_and_guest_cache_is_not_a_host_profile() {
        let f = Fixture::new();
        let p = f.env();
        fs::remove_file(p.join("env.toml")).unwrap();
        assert_eq!(inspect(&p).unwrap()["source"], "guest");
        fs::write(p.join("env.toml"), "not valid TOML =[").unwrap();
        let value = snapshot(&f.0).unwrap();
        assert!(value["environments"][0]["error"].is_string());
        assert!(value["environments"][0]["path"].is_string());
    }
}
