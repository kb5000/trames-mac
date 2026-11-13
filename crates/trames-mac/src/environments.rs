// SPDX-License-Identifier: GPL-3.0-or-later
//! Product environment registry. Guest I/O never enters this management path.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
use trames_vfs::{DiskOptions, Vfs};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mount {
    pub guest: String,
    pub host: String,
    pub readonly: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema: u32,
    #[serde(default)]
    pub target: trames_distro::boot::Target,
    pub distribution: String,
    #[serde(default)]
    pub init: super::systemd::Init,
    pub source_sha256: String,
    #[serde(default = "shell_command")]
    pub command: Vec<String>,
    pub cpus: usize,
    pub memory_mib: String,
    pub network: String,
    pub windows: bool,
    pub vulkan: bool,
    pub video: bool,
    #[serde(default)]
    pub audio: bool,
    #[serde(default)]
    pub audio_input: bool,
    pub mounts: Vec<Mount>,
}
fn shell_command() -> Vec<String> {
    vec!["/bin/sh".into(), "-l".into()]
}
impl Config {
    fn new(distribution: &str) -> Self {
        Self {
            schema: 1,
            target: if distribution == "android" {
                trames_distro::boot::Target::Android
            } else {
                trames_distro::boot::Target::Linux
            },
            distribution: distribution.into(),
            init: super::systemd::Init::Shell,
            source_sha256: catalog(distribution)
                .map_or("custom-rootfs", |c| c.1)
                .into(),
            command: if distribution == "android" {
                trames_distro::android::command()
            } else {
                shell_command()
            },
            cpus: std::thread::available_parallelism()
                .map_or(1, usize::from)
                .min(trames_hvf::max_parallel_cpus()),
            memory_mib: "auto".into(),
            network: if distribution == "android" {
                "off"
            } else {
                "full"
            }
            .into(),
            windows: true,
            vulkan: true,
            video: false,
            audio: false,
            audio_input: false,
            mounts: vec![],
        }
    }
    fn validate(&self) -> Result<()> {
        if self.schema != 1 {
            return Err(
                "unsupported environment configuration version; use a matching Trames release"
                    .into(),
            );
        }
        if !["alpine", "debian", "custom", "android"].contains(&self.distribution.as_str()) {
            return Err("unknown distribution".into());
        }
        if (self.target == trames_distro::boot::Target::Android) != (self.distribution == "android")
        {
            return Err(
                "target and image type differ; create a separate Linux or Android environment"
                    .into(),
            );
        }
        if self.target == trames_distro::boot::Target::Android
            && (self.init != super::systemd::Init::Shell
                || self.command != trames_distro::android::command()
                || !self.vulkan
                || !self.windows)
        {
            return Err(
                "Android boots its own init and requires Vulkan and windows; use exec for an Android shell"
                    .into(),
            );
        }
        if self.command.is_empty()
            || !self.command[0].starts_with('/')
            || self.command.iter().any(|a| a.contains('\0'))
        {
            return Err(
                "command must be a nonempty argument array beginning with an absolute Linux path"
                    .into(),
            );
        }
        if self.init == super::systemd::Init::Systemd && self.distribution == "alpine" {
            return Err(
                "systemd requires Debian or a custom rootfs with systemd installed; Alpine uses shell".into(),
            );
        }
        if self.cpus == 0 || self.cpus > trames_hvf::max_parallel_cpus() {
            return Err("invalid CPU count".into());
        }
        if self.memory_mib != "auto"
            && self
                .memory_mib
                .parse::<u64>()
                .ok()
                .filter(|n| *n > 0 && *n <= usize::MAX as u64 / (1 << 20))
                .is_none()
        {
            return Err("invalid memory size".into());
        }
        if !["off", "loopback", "outbound", "full"].contains(&self.network.as_str()) {
            return Err("invalid network mode".into());
        }
        for m in &self.mounts {
            if !Path::new(&m.host).is_absolute()
                || !m.guest.starts_with('/')
                || m.guest.split('/').any(|s| s == ".." || s == ".")
            {
                return Err("mounts require absolute paths without dot components".into());
            }
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CliConfig {
    schema: u32,
    default_environment: String,
    defaults: Config,
}
impl Default for CliConfig {
    fn default() -> Self {
        Self {
            schema: 1,
            default_environment: "default".into(),
            defaults: Config::new("alpine"),
        }
    }
}
fn load_cli(r: &Path) -> Result<CliConfig> {
    let file = match File::open(r.join("cli.json")) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(CliConfig::default()),
        Err(e) => return Err(e.into()),
    };
    let c: CliConfig = serde_json::from_reader(file.take(65537))?;
    validate_cli(&c)?;
    Ok(c)
}
fn validate_cli(c: &CliConfig) -> Result<()> {
    if c.schema != 1 {
        return Err("unsupported CLI configuration version".into());
    }
    name(&c.default_environment)?;
    catalog(&c.defaults.distribution)?;
    c.defaults.validate()
}
pub fn default_environment(r: &Path) -> Result<String> {
    Ok(load_cli(r)?.default_environment)
}
fn config_key(key: &str) -> &str {
    match key {
        "memory" | "memory-mib" => "memory_mib",
        "audio-input" => "audio_input",
        "distro" => "distribution",
        _ => key,
    }
}
fn memory(value: &str) -> Result<String> {
    if value == "auto" {
        return Ok(value.into());
    }
    let split = value
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(value.len());
    let number: u64 = value[..split].parse()?;
    let bytes = match &value[split..] {
        "" | "MiB" | "M" => number.checked_mul(1 << 20),
        "GiB" | "G" => number.checked_mul(1 << 30),
        "KiB" | "K" => number.checked_mul(1 << 10),
        "B" => Some(number),
        "MB" => number.checked_mul(1_000_000),
        "GB" => number.checked_mul(1_000_000_000),
        _ => return Err("memory must be auto or a size such as 4GiB / 4096MiB".into()),
    }
    .ok_or("memory size overflow")?;
    if bytes == 0 {
        return Err("memory must be positive".into());
    }
    Ok(bytes.div_ceil(1 << 20).to_string())
}
fn set_config(c: &mut Config, key: &str, value: &str) -> Result<()> {
    let key = config_key(key);
    let mut json = serde_json::to_value(&*c)?;
    let field = json.get(key).ok_or("unknown configuration key")?;
    if ![
        "command",
        "cpus",
        "memory_mib",
        "network",
        "init",
        "windows",
        "vulkan",
        "video",
        "audio",
        "audio_input",
        "mounts",
    ]
    .contains(&key)
    {
        return Err("this environment identity field is read-only".into());
    }
    json[key] = if key == "memory_mib" {
        serde_json::Value::String(memory(value)?)
    } else if key == "cpus" && matches!(value, "all" | "auto") {
        serde_json::json!(Config::new("alpine").cpus)
    } else if field.is_boolean() {
        serde_json::json!(match value {
            "true" | "on" => true,
            "false" | "off" => false,
            _ => return Err("expected true or false".into()),
        })
    } else if field.is_string() {
        serde_json::Value::String(value.into())
    } else {
        serde_json::from_str(value)?
    };
    *c = serde_json::from_value(json)?;
    Ok(())
}
fn read_value(value: &serde_json::Value) {
    match value {
        serde_json::Value::String(s) => println!("{s}"),
        _ => println!("{value}"),
    }
}
fn setting_value<'a>(
    document: &'a serde_json::Value,
    key: &str,
    global: bool,
) -> Result<&'a serde_json::Value> {
    let (parent, key) = if global {
        if key == "default-environment" || key == "default_environment" {
            (document, "default_environment")
        } else {
            (
                &document["defaults"],
                config_key(
                    key.strip_prefix("defaults.")
                        .ok_or("global settings use defaults.KEY or default-environment")?,
                ),
            )
        }
    } else {
        (document, config_key(key))
    };
    parent
        .get(key)
        .ok_or_else(|| "unknown configuration key".into())
}
/// Settings never open guest storage, install packages or start the environment.
/// This metadata lock is independent of the running store owner's lifetime lock.
pub fn settings(selected: Option<&str>, args: &[String]) -> Result<u8> {
    let mut args = args.to_vec();
    let global = args.iter().any(|a| a == "--global");
    args.retain(|a| a != "--global");
    let json = args.iter().any(|a| a == "--json");
    args.retain(|a| a != "--json");
    let op = args.first().map(String::as_str).unwrap_or("list");
    let r = root()?;
    let mut cli = load_cli(&r)?;
    let n = selected.unwrap_or(&cli.default_environment).to_owned();
    let dir = directory(&r, &n)?;
    let path = if global {
        r.join("cli.json")
    } else {
        dir.join("config.json")
    };
    if op == "path" {
        if args.len() != 1 {
            return Err("usage: config path".into());
        }
        println!("{}", path.display());
        return Ok(0);
    }
    let mut c = if global {
        cli.defaults.clone()
    } else {
        load(&dir)?
    };
    let document = if global {
        serde_json::to_value(&cli)?
    } else {
        serde_json::to_value(&c)?
    };
    match op {
        "get" if args.len() == 2 => read_value(setting_value(&document, &args[1], global)?),
        "list" | "show" if args.len() == 1 => {
            if json || op == "show" { println!("{}", serde_json::to_string_pretty(&document)?); }
            else {
                if global { println!("default-environment\t{}", cli.default_environment); }
                for (key,value) in serde_json::to_value(&c)?.as_object().unwrap() {
                    if ["schema","source_sha256"].contains(&key.as_str()) { continue; }
                    let key = match key.as_str() { "memory_mib" => "memory", "audio_input" => "audio-input", key => key };
                    print!("{}{key}\t", if global {"defaults."} else {""}); read_value(value);
                }
            }
        }
        "set" | "unset" | "edit" | "reset" => {
            let guard = OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(r.join("locks").join(if global { ".cli-config".into() } else { format!(".config-{n}") }))?;
            guard.try_lock().map_err(|_| "settings are being changed; retry")?;
            // Reload after obtaining the metadata lock, without opening the store.
            cli = load_cli(&r)?;
            c = if global { cli.defaults.clone() } else { load(&dir)? };
            match op {
                "set" if args.len() >= 3 && args.len()%2 == 1 => {
                    for pair in args[1..].chunks_exact(2) {
                        if global && pair[0] == "default-environment" { cli.default_environment=pair[1].clone(); }
                        else {
                            let key = if global { pair[0].strip_prefix("defaults.").ok_or("global settings use defaults.KEY or default-environment")? } else { &pair[0] };
                            if global && matches!(key, "distro" | "distribution") {
                                c.distribution=pair[1].clone(); c.source_sha256=catalog(&c.distribution)?.1.into();
                            } else { set_config(&mut c, key, &pair[1])?; }
                        }
                    }
                }
                "unset" if args.len() == 2 => {
                    if global && args[1] == "default-environment" { cli.default_environment=CliConfig::default().default_environment; }
                    else if global && matches!(args[1].as_str(),"defaults.distro"|"defaults.distribution") {
                        c.distribution="alpine".into(); c.source_sha256=catalog("alpine")?.1.into();
                    } else {
                        let key = if global { args[1].strip_prefix("defaults.").ok_or("global settings use defaults.KEY")? } else { &args[1] };
                        let defaults = if global { Config::new(&c.distribution) } else { cli.defaults.clone() };
                        let value = serde_json::to_value(&defaults)?;
                        let value = value.get(config_key(key)).ok_or("unknown configuration key")?;
                        let text = value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string());
                        set_config(&mut c, key, &text)?;
                    }
                }
                "reset" if global && args.len() == 1 => { cli=CliConfig::default(); c=cli.defaults.clone(); }
                "edit" if args.len() == 1 => {
                    let temp = r.join(format!(".config-edit-{}.json",std::process::id()));
                    let document = if global { cli.defaults=c.clone(); serde_json::to_vec_pretty(&cli)? } else { serde_json::to_vec_pretty(&c)? };
                    atomic_write(&temp,&document)?;
                    let result = (|| -> Result<()> {
                        let editor=std::env::var("VISUAL").or_else(|_|std::env::var("EDITOR")).unwrap_or_else(|_|"vi".into());
                        let status=Command::new("/bin/sh").arg("-c").arg(format!("exec {editor} \"$1\"" )).arg("trames-editor").arg(&temp).status()?;
                        if !status.success() { return Err("editor exited unsuccessfully; configuration was preserved".into()); }
                        let file=File::open(&temp)?.take(65537);
                        if global { cli=serde_json::from_reader(file)?; c=cli.defaults.clone(); }
                        else { c=serde_json::from_reader(file)?; }
                        Ok(())
                    })();
                    let _=fs::remove_file(temp); result?;
                }
                _ => return Err("usage: config [--global] set KEY VALUE | unset KEY | edit | get KEY | list [--json] | path".into()),
            }
            c.validate()?;
            if global { cli.defaults=c; validate_cli(&cli)?; atomic_write(&path,&serde_json::to_vec_pretty(&cli)?)?; eprintln!("Saved global defaults; applies to new environments."); }
            else { save(&dir,&c)?; eprintln!("Saved {n}; changes apply after an environment restart."); }
        }
        _ => return Err("usage: config [--global] get KEY | set KEY VALUE | unset KEY | list [--json] | edit | path".into()),
    }
    Ok(0)
}
pub fn listing(args: &[String]) -> Result<u8> {
    if args.len() > 1
        || args
            .first()
            .is_some_and(|a| !["--json", "-q", "--quiet"].contains(&a.as_str()))
    {
        return Err("usage: env ls [--json|-q]".into());
    }
    list(
        &root()?,
        args.first().map(String::as_str).unwrap_or("table"),
    )?;
    Ok(0)
}
pub fn setup(args: &[String]) -> Result<u8> {
    if args.len() != 3 || args[1] != "--init" || args[2] != "systemd" {
        return Err("usage: env setup NAME --init systemd".into());
    }
    let r = root()?;
    let n = &args[0];
    let _lock = lock(&r, n)?;
    let dir = directory(&r, n)?;
    let mut c = load(&dir)?;
    c.init = super::systemd::Init::Systemd;
    c.validate()?;
    provision_init(&dir.join("store"), &c)?;
    save(&dir, &c)?;
    eprintln!("Prepared systemd for {n}; start or restart the environment to use it.");
    Ok(0)
}

fn catalog(d: &str) -> Result<(&'static str, &'static str)> {
    match d {
        "alpine" => Ok((trames_distro::ALPINE_URL, trames_distro::ALPINE_SHA256)),
        "debian" => Ok((trames_distro::DEBIAN_URL, trames_distro::DEBIAN_SHA256)),
        _ => Err("distribution must be alpine or debian".into()),
    }
}
pub fn root() -> Result<PathBuf> {
    let p = if let Some(p) = std::env::var_os("TRAMES_HOME") {
        PathBuf::from(p)
    } else {
        super::product::registry(Path::new(
            &std::env::var_os("HOME").ok_or("HOME is not set")?,
        ))
    };
    if !p.is_absolute() {
        return Err("TRAMES_HOME must be absolute".into());
    }
    private_dir(&p)?;
    private_dir(&p.join("environments"))?;
    private_dir(&p.join("locks"))?;
    Ok(fs::canonicalize(p)?)
}
pub(super) fn private_dir(p: &Path) -> Result<()> {
    match fs::symlink_metadata(p) {
        Ok(m) if !m.file_type().is_dir() => {
            return Err("management path must be a real directory".into())
        }
        Ok(_) => (),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(p)?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}
fn name(n: &str) -> Result<()> {
    if n.is_empty()
        || n.len() > 40
        || !n
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err("name must contain 1–40 letters, digits, '-' or '_'".into());
    }
    Ok(())
}
pub(super) fn directory(r: &Path, n: &str) -> Result<PathBuf> {
    name(n)?;
    Ok(r.join("environments").join(n))
}
fn lock(r: &Path, n: &str) -> Result<File> {
    name(n)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(r.join("locks").join(n))?;
    file.try_lock()
        .map_err(|_| "environment is running or another management operation is in progress")?;
    Ok(file)
}
pub(super) fn load(dir: &Path) -> Result<Config> {
    if !fs::symlink_metadata(dir)?.is_dir() {
        return Err("environment must be a real directory".into());
    }
    let c: Config = serde_json::from_reader(File::open(dir.join("config.json"))?.take(65537))?;
    c.validate()?;
    Ok(c)
}
pub(super) fn atomic_write(p: &Path, data: &[u8]) -> Result<()> {
    let temp = p.with_extension(format!("tmp-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    let result = (|| {
        file.write_all(data)?;
        file.sync_all()?;
        fs::rename(&temp, p)?;
        File::open(p.parent().unwrap())?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
fn save(dir: &Path, c: &Config) -> Result<()> {
    c.validate()?;
    atomic_write(&dir.join("config.json"), &serde_json::to_vec_pretty(c)?)
}
fn digest(p: &Path) -> Result<String> {
    let mut hash = Sha256::new();
    let mut file = File::open(p)?;
    let mut buf = [0; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn source(r: &Path, d: &str, local: Option<&str>) -> Result<PathBuf> {
    let (url, hash) = catalog(d)?;
    if let Some(local) = local {
        let p = PathBuf::from(local);
        if digest(&p)? != hash {
            return Err("rootfs SHA-256 mismatch".into());
        }
        return Ok(p);
    }
    let cache = r.join("cache");
    private_dir(&cache)?;
    let path = cache.join(format!("{hash}.tar.gz"));
    if path.exists() && digest(&path)? == hash {
        return Ok(path);
    }
    let temporary = cache.join(format!("{hash}.{}.part", std::process::id()));
    eprintln!("Downloading verified {d} ARM64 rootfs…");
    let status = Command::new("/usr/bin/curl")
        .args([
            "--fail",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--retry",
            "2",
            "--connect-timeout",
            "20",
            "--max-time",
            "600",
            "--max-filesize",
            "67108864",
            "--output",
        ])
        .arg(&temporary)
        .arg(url)
        .status();
    let result = (|| {
        if !status?.success() {
            return Err(
                "download failed; your environment has not been created. Retry create".into(),
            );
        }
        if digest(&temporary)? != hash {
            return Err("rootfs SHA-256 mismatch; cached download rejected".into());
        }
        File::open(&temporary)?.sync_all()?;
        fs::rename(&temporary, &path)?;
        Ok(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn staging(r: &Path, n: &str) -> Result<Staging> {
    let path = r
        .join("environments")
        .join(format!(".{}-{}", n, std::process::id()));
    // The per-name lock is held: stale unpublished stores from killed creators
    // cannot belong to a live operation and may be safely reclaimed.
    for entry in fs::read_dir(r.join("environments"))? {
        let entry = entry?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(&format!(".{n}-"))
        {
            fs::remove_dir_all(entry.path())?;
        }
    }
    fs::DirBuilder::new().mode(0o700).create(&path)?;
    Ok(Staging(path))
}
fn publish(stage: &Path, dir: &Path) -> Result<()> {
    if dir.try_exists()? {
        return Err("environment already exists".into());
    }
    fs::rename(stage, dir)?;
    File::open(dir.parent().unwrap())?.sync_all()?;
    Ok(())
}
fn create(
    r: &Path,
    n: &str,
    mut c: Config,
    archive: Option<&str>,
    custom: Option<&str>,
    expected: Option<&str>,
    packages: Option<super::packages::Profile>,
) -> Result<()> {
    let _lock = lock(r, n)?;
    let dir = directory(r, n)?;
    if dir.try_exists()? {
        return Err("environment already exists".into());
    }
    if custom.is_some() && archive.is_some() {
        return Err("choose --rootfs or --archive".into());
    }
    if expected.is_some() && custom.is_none() {
        return Err("--sha256 requires --rootfs".into());
    }
    if custom.is_some() {
        c.distribution = if c.target == trames_distro::boot::Target::Android {
            "android"
        } else {
            "custom"
        }
        .into();
    }
    if c.target == trames_distro::boot::Target::Android && custom.is_none() {
        return Err(
            "Android is an optional download; supply --target android --rootfs PACKAGE.tar.gz"
                .into(),
        );
    }
    c.validate()?;
    use super::packages::Profile;
    let packages = packages.unwrap_or(if c.distribution == "debian" {
        Profile::Standard
    } else {
        Profile::Minimal
    });
    if c.distribution != "debian" && packages != Profile::Minimal {
        return Err("package presets require the built-in Debian image; custom rootfs and Alpine preserve their own package setup".into());
    }
    let source = if custom.is_none() {
        Some(source(r, &c.distribution, archive)?)
    } else {
        None
    };
    eprintln!("Importing {} rootfs…", c.distribution);
    let stage = staging(r, n)?;
    let mut vfs = Vfs::open_disk(stage.0.join("store"), DiskOptions::default())?;
    c.source_sha256 = vfs.populate_staged(|vfs| -> Result<String> {
        if let Some(custom) = custom {
            return super::rootfs::install(Path::new(custom), vfs, expected);
        }
        let mut input = File::open(source.as_ref().unwrap())?;
        match c.distribution.as_str() {
            "alpine" => {
                trames_distro::install_alpine(&mut input, vfs)?;
            }
            "debian" => {
                trames_distro::install_debian(&mut input, vfs)?;
            }
            _ => unreachable!(),
        }
        Ok(catalog(&c.distribution)?.1.into())
    })?;
    if c.target == trames_distro::boot::Target::Linux {
        if custom.is_none()
            && vfs
                .stat_at(trames_vfs::AT_FDCWD, b"/etc/resolv.conf")
                .is_ok()
        {
            let fd = vfs.open_at(trames_vfs::AT_FDCWD, b"/etc/resolv.conf", 0x201, 0)?;
            let result = vfs.write(fd, b"nameserver 169.254.0.53\n");
            let closed = vfs.close(fd);
            result?;
            closed?;
        } else {
            trames_distro::install_resolver(&mut vfs, b"nameserver 169.254.0.53\n")?;
        }
    }
    if c.target == trames_distro::boot::Target::Android {
        trames_distro::android::validate_image(&mut vfs)?;
    } else if custom.is_some() {
        super::rootfs::validate_command(&mut vfs, &c.command)?;
    }
    vfs.sync()?;
    drop(vfs);
    super::packages::install(&stage.0.join("store"), c.cpus, &c.network, packages)?;
    provision_init(&stage.0.join("store"), &c)?;
    save(&stage.0, &c)?;
    publish(&stage.0, &dir)?;
    println!("Created {n} ({})", c.distribution);
    Ok(())
}
fn provision_init(store: &Path, c: &Config) -> Result<()> {
    if c.init != super::systemd::Init::Systemd {
        return Ok(());
    }
    if c.distribution == "custom" {
        let vfs = Vfs::open_disk(store, DiskOptions::default())?;
        vfs.executable(b"/usr/lib/systemd/systemd")
            .map_err(|_| "custom systemd rootfs must already contain /usr/lib/systemd/systemd")?;
        Ok(())
    } else {
        super::systemd::install(store, c.cpus, &c.network)
    }
}
pub(super) fn socket(r: &Path, n: &str) -> Result<PathBuf> {
    let tmp = PathBuf::from(format!("/private/tmp/trames-{}", unsafe {
        libc::geteuid()
    }));
    private_dir(&tmp)?;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let m = fs::symlink_metadata(&tmp)?;
    if m.uid() != unsafe { libc::geteuid() } || m.permissions().mode() & 0o077 != 0 {
        return Err("unsafe control socket directory permissions".into());
    }
    let hash = format!(
        "{:x}",
        Sha256::digest(directory(r, n)?.as_os_str().as_encoded_bytes())
    );
    Ok(tmp.join(format!("{}.sock", &hash[..32])))
}
pub(super) fn console_socket(r: &Path, n: &str) -> Result<PathBuf> {
    Ok(socket(r, n)?.with_extension("console"))
}
pub(super) fn running(r: &Path, n: &str) -> bool {
    lock(r, n).is_err()
}
pub(super) fn start(r: &Path, n: &str, extra: &[String]) -> Result<u8> {
    let _lock = lock(r, n)?;
    let dir = directory(r, n)?;
    let c = load(&dir)?;
    let _control = super::lifecycle::SessionControl::bind(&socket(r, n)?)?;
    let mut args = vec![
        "--storage".into(),
        dir.join("store").to_string_lossy().into_owned(),
        "--cpus".into(),
        c.cpus.to_string(),
        "--memory-mib".into(),
        c.memory_mib,
        "--network".into(),
        c.network,
    ];
    if !c.windows {
        args.push("--no-windows".into());
    }
    if c.vulkan {
        args.push("--vulkan".into());
    }
    if c.audio {
        args.push("--audio".into());
    }
    if c.audio_input {
        args.push("--audio-input".into());
    }
    if c.video {
        args.push("--video".into());
    }
    for m in c.mounts {
        args.extend([
            if m.readonly { "--mount-ro" } else { "--mount" }.into(),
            format!("{}={}", m.guest, m.host),
        ]);
    }
    if c.target == trames_distro::boot::Target::Android {
        args.extend(["--android-init".into(), "--no-pty".into()]);
    }
    // An explicit command is a maintenance session; require an explicit shell
    // override rather than silently bypassing this environment's init setting.
    args.extend([
        "--init".into(),
        match c.init {
            super::systemd::Init::Shell => "shell",
            super::systemd::Init::Systemd => "systemd",
        }
        .into(),
    ]);
    let extra = extra.strip_prefix(&["--".into()]).unwrap_or(extra);
    args.extend_from_slice(extra);
    Ok(super::execute_with_default(args, &c.command)?)
}
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}
fn terminal(r: &Path, n: &str) -> Result<()> {
    let c = load(&directory(r, n)?)?;
    if c.target == trames_distro::boot::Target::Android {
        return super::background::start(r, n);
    }
    let launchers = r.join("launchers");
    private_dir(&launchers)?;
    let path = launchers.join(format!("{n}.command"));
    let text = format!(
        "#!/bin/sh\nexport TRAMES_HOME={}\nexec {} -e {} shell\n",
        shell_quote(&r.to_string_lossy()),
        shell_quote(&std::env::current_exe()?.to_string_lossy()),
        shell_quote(n)
    );
    atomic_write(&path, text.as_bytes())?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    if !Command::new("/usr/bin/open")
        .args(["-a", "Terminal"])
        .arg(path)
        .status()?
        .success()
    {
        return Err("could not open Terminal".into());
    }
    Ok(())
}
pub(super) fn stop(r: &Path, n: &str) -> Result<()> {
    load(&directory(r, n)?)?;
    if super::background::registered(r, n) {
        super::background::unload(r, n)?;
    }
    if !running(r, n) {
        println!("{n} is stopped");
        return Ok(());
    }
    UnixStream::connect(socket(r, n)?)
        .map_err(|_| "environment is busy or still starting; retry stop after session ready")?;
    let began = Instant::now();
    while running(r, n) {
        if began.elapsed() > Duration::from_secs(60) {
            return Err("stop timed out; no forced kill was sent".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("Stopped {n}");
    Ok(())
}
#[derive(Serialize)]
struct Row {
    name: String,
    distribution: String,
    state: &'static str,
    path: PathBuf,
    config: Option<Config>,
    error: Option<String>,
}
fn list(r: &Path, format: &str) -> Result<()> {
    let mut rows = vec![];
    for entry in fs::read_dir(r.join("environments"))? {
        let e = entry?;
        let n = e.file_name().to_string_lossy().into_owned();
        if name(&n).is_err() {
            continue;
        }
        match load(&e.path()) {
            Ok(c) => rows.push(Row {
                name: n.clone(),
                distribution: c.distribution.clone(),
                state: if running(r, &n) {
                    "running / busy"
                } else {
                    "stopped"
                },
                path: e.path(),
                config: Some(c),
                error: None,
            }),
            Err(error) => rows.push(Row {
                name: n,
                distribution: "Unknown".into(),
                state: "needs attention",
                path: e.path(),
                config: None,
                error: Some(error.to_string()),
            }),
        }
    }
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    if format == "--json" {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else if matches!(format, "-q" | "--quiet") {
        for row in rows {
            println!("{}", row.name);
        }
    } else {
        println!("{:<24} {:<12} STATUS", "NAME", "DISTRO");
        for row in rows {
            println!("{:<24} {:<12} {}", row.name, row.distribution, row.state);
        }
    }
    Ok(())
}
fn import(r: &Path, n: &str, source: &Path) -> Result<()> {
    let _lock = lock(r, n)?;
    let dir = directory(r, n)?;
    if dir.try_exists()? {
        return Err("environment already exists".into());
    }
    let source = fs::canonicalize(source)?;
    if !source.join("manifest").is_file() {
        return Err("select an existing Trames store containing manifest and objects".into());
    }
    let vfs = Vfs::open_disk(&source, DiskOptions::default())?;
    let d = if vfs.executable(b"/system/bin/init").is_ok() {
        "android"
    } else if vfs
        .stat_at(trames_vfs::AT_FDCWD, b"/etc/debian_version")
        .is_ok()
    {
        "debian"
    } else if vfs
        .stat_at(trames_vfs::AT_FDCWD, b"/etc/alpine-release")
        .is_ok()
    {
        "alpine"
    } else {
        "custom"
    };
    let stage = staging(r, n)?;
    private_dir(&stage.0.join("store/objects"))?;
    let mut files = vec![PathBuf::from("store/manifest")];
    for e in fs::read_dir(source.join("objects"))? {
        files.push(PathBuf::from("store/objects").join(e?.file_name()));
    }
    for rel in files {
        if !backup_path(&rel) {
            return Err("unexpected store object".into());
        }
        let from = source.join(rel.strip_prefix("store")?);
        if !fs::symlink_metadata(&from)?.is_file() {
            return Err("store contains a nonregular object".into());
        }
        let to = stage.0.join(rel);
        fs::copy(from, &to)?;
        File::open(to)?.sync_all()?;
    }
    let mut c = Config::new(d);
    c.source_sha256 = "imported-existing-store".into();
    save(&stage.0, &c)?;
    let mut copy = Vfs::open_disk(stage.0.join("store"), DiskOptions::default())?;
    if c.target == trames_distro::boot::Target::Android {
        trames_distro::android::validate_image(&mut copy)?;
    } else {
        super::systemd::renew_identity(&mut copy)?;
    }
    drop(copy);
    publish(&stage.0, &dir)?;
    drop(vfs);
    println!("Imported {n}; original store is unchanged");
    Ok(())
}
fn configure(r: &Path, n: &str, args: &[String]) -> Result<()> {
    let _lock = lock(r, n)?;
    let dir = directory(r, n)?;
    let mut c = load(&dir)?;
    apply_settings(&mut c, args)?;
    if c.command != load(&dir)?.command {
        let mut vfs = Vfs::open_disk(dir.join("store"), DiskOptions::default())?;
        super::rootfs::validate_command(&mut vfs, &c.command)?;
    }
    save(&dir, &c)
}
fn apply_settings(c: &mut Config, settings: &[String]) -> Result<()> {
    let mut args = settings.iter();
    while let Some(k) = args.next() {
        if k == "--" {
            c.command = args.cloned().collect();
            break;
        }
        if k == "--clear-mounts" {
            c.mounts.clear();
            continue;
        }
        let v = args.next().ok_or("configuration value missing")?;
        let on = || match v.as_str() {
            "on" => Ok(true),
            "off" => Ok(false),
            _ => Err("expected on or off"),
        };
        match k.as_str() {
            "--command" => c.command = serde_json::from_str(v)?,
            "--init" => c.init = super::systemd::Init::parse(v)?,
            "--cpus" => c.cpus = v.parse()?,
            "--memory-mib" => c.memory_mib = v.clone(),
            "--network" => c.network = v.clone(),
            "--windows" => c.windows = on()?,
            "--vulkan" => c.vulkan = on()?,
            "--video" => c.video = on()?,
            "--audio" => c.audio = on()?,
            "--audio-input" => c.audio_input = on()?,
            "--mount" | "--mount-ro" => {
                let (g, h) = v.split_once('=').ok_or("mount must be GUEST=HOST")?;
                let host = fs::canonicalize(h)?;
                if !host.is_dir() {
                    return Err("mount source must be a directory".into());
                }
                c.mounts.push(Mount {
                    guest: g.into(),
                    host: host.to_string_lossy().into_owned(),
                    readonly: k == "--mount-ro",
                });
            }
            _ => return Err("unknown configuration setting".into()),
        }
    }
    c.validate()?;
    Ok(())
}
fn backup(r: &Path, n: &str, destination: &Path) -> Result<()> {
    let _lock = lock(r, n)?;
    let dir = directory(r, n)?;
    load(&dir)?;
    let vfs = Vfs::open_disk(dir.join("store"), DiskOptions::default())?;
    if destination.try_exists()? {
        return Err("backup destination already exists".into());
    }
    let temp = destination.with_extension(format!("partial-{}", std::process::id()));
    let result = (|| {
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temp)?;
        let gzip = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        let mut tar = tar::Builder::new(gzip);
        let mut inventory = BTreeMap::new();
        let mut paths = vec![
            PathBuf::from("config.json"),
            PathBuf::from("store/manifest"),
        ];
        for e in fs::read_dir(dir.join("store/objects"))? {
            paths.push(PathBuf::from("store/objects").join(e?.file_name()));
        }
        paths.sort();
        for rel in paths {
            if !backup_path(&rel) {
                return Err("unexpected store object".into());
            }
            let path = dir.join(&rel);
            if !fs::symlink_metadata(&path)?.is_file() {
                return Err("backup refuses nonregular objects".into());
            }
            inventory.insert(rel.to_string_lossy().into_owned(), digest(&path)?);
            tar.append_path_with_name(path, rel)?;
        }
        let bytes = serde_json::to_vec(&inventory)?;
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        tar.append_data(&mut header, "inventory.json", bytes.as_slice())?;
        let file = tar.into_inner()?.finish()?;
        file.sync_all()?;
        // A hard link publishes without replacing an existing destination.
        fs::hard_link(&temp, destination)?;
        File::open(
            destination
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?
        .sync_all()?;
        Ok(())
    })();
    let _ = fs::remove_file(&temp);
    drop(vfs);
    result
}
fn backup_path(p: &Path) -> bool {
    let Some(s) = p.to_str() else {
        return false;
    };
    s == "config.json"
        || s == "store/manifest"
        || s.strip_prefix("store/objects/").is_some_and(|x| {
            x.len() == 20 && x.ends_with(".dat") && x[..16].bytes().all(|c| c.is_ascii_hexdigit())
        })
}
fn restore(r: &Path, n: &str, archive: &Path) -> Result<()> {
    let _lock = lock(r, n)?;
    let dir = directory(r, n)?;
    if dir.try_exists()? {
        return Err("restore requires a new environment name".into());
    }
    let stage = staging(r, n)?;
    private_dir(&stage.0.join("store/objects"))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(File::open(archive)?));
    let mut hashes = BTreeMap::new();
    let mut inventory = None;
    let mut seen = BTreeSet::new();
    let mut total = 0u64;
    for item in tar.entries()? {
        let mut item = item?;
        let p = item.path()?.into_owned();
        if !seen.insert(p.clone())
            || !item.header().entry_type().is_file()
            || (!backup_path(&p) && p != Path::new("inventory.json"))
        {
            return Err("invalid backup path, type or duplicate entry".into());
        }
        total = total
            .checked_add(item.size())
            .ok_or("backup size overflow")?;
        if total > 1 << 40 || seen.len() > 262146 {
            return Err("backup exceeds supported store size".into());
        }
        if p == Path::new("inventory.json") {
            let mut bytes = vec![];
            item.take(32 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > 32 * 1024 * 1024 {
                return Err("oversized backup inventory".into());
            }
            inventory = Some(serde_json::from_slice::<BTreeMap<String, String>>(&bytes)?);
            continue;
        }
        let path = stage.0.join(&p);
        let mut out = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)?;
        io::copy(&mut item, &mut out)?;
        out.sync_all()?;
        hashes.insert(p.to_string_lossy().into_owned(), digest(&path)?);
    }
    // Force gzip trailer validation even though tar stops at its end markers.
    io::copy(&mut tar.into_inner(), &mut io::sink())?;
    if inventory.as_ref() != Some(&hashes) {
        return Err("backup file checksum/inventory mismatch".into());
    }
    let mut c = load(&stage.0)?;
    // A portable backup never grants new native paths to its recipient.
    c.mounts.clear();
    save(&stage.0, &c)?;
    let mut vfs = Vfs::open_disk(stage.0.join("store"), DiskOptions::default())?;
    super::systemd::renew_identity(&mut vfs)?;
    drop(vfs);
    publish(&stage.0, &dir)?;
    println!("Restored {n}; shared directories must be selected again");
    Ok(())
}
pub fn dispatch(args: &[String]) -> Result<u8> {
    let r = root()?;
    let op = args.first().map(String::as_str).unwrap_or("list");
    if op == "default" {
        return super::cli::dispatch(vec!["shell".into()]);
    }
    if op == "help" || op == "--help" {
        return super::cli::dispatch(vec!["--help".into()]);
    }
    if op == "list" {
        list(&r, "table")?;
        return Ok(0);
    }
    if op == "doctor" {
        println!(
            "{}",
            serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"runtime":include_str!("../../../runtime-version.json"),"home":r,"channel":super::product::CHANNEL,"architecture":std::env::consts::ARCH,"cpus":trames_hvf::max_parallel_cpus(),"rootfs":[{"name":"alpine","version":trames_distro::ALPINE_VERSION,"sha256":trames_distro::ALPINE_SHA256},{"name":"debian","version":trames_distro::DEBIAN_VERSION,"sha256":trames_distro::DEBIAN_SHA256}]})
        );
        return Ok(0);
    }
    let n = args.get(1).ok_or("environment name required")?;
    name(n)?;
    match op {
        "create" => {
            let mut c = load_cli(&r)?.defaults;
            let mut archive = None;
            let mut custom = None;
            let mut expected = None;
            let mut packages = None;
            let mut settings = vec![];
            let mut it = args[2..].iter();
            while let Some(k) = it.next() {
                if k == "--" {
                    settings.push(k.clone());
                    settings.extend(it.cloned());
                    break;
                }
                let v = it.next().ok_or("missing create option value")?;
                match k.as_str() {
                    "--distro" => {
                        catalog(v)?;
                        c.distribution = v.clone();
                        c.source_sha256 = catalog(v)?.1.into();
                    }
                    "--target" => c.target = trames_distro::boot::Target::parse(v)?,
                    "--archive" => archive = Some(v.as_str()),
                    "--rootfs" => custom = Some(v.as_str()),
                    "--sha256" => expected = Some(v.as_str()),
                    "--packages" => packages = Some(super::packages::Profile::parse(v)?),
                    _ => {
                        settings.extend([k.clone(), v.clone()]);
                    }
                }
            }
            if c.target == trames_distro::boot::Target::Android {
                c.distribution = "android".into();
                c.command = trames_distro::android::command();
                c.network = "off".into();
            } else if custom.is_some() {
                c.distribution = "custom".into();
            }
            apply_settings(&mut c, &settings)?;
            create(&r, n, c, archive, custom, expected, packages)?;
        }
        "packages" => {
            if args.len() > 3 { return Err("usage: env packages NAME [standard|development]".into()); }
            let profile = super::packages::Profile::parse(args.get(2).map(String::as_str).unwrap_or("standard"))?;
            let _lock = lock(&r, n)?;
            let dir = directory(&r, n)?;
            let c = load(&dir)?;
            if c.distribution != "debian" { return Err("package presets require the built-in Debian image".into()); }
            super::packages::install(&dir.join("store"), c.cpus, &c.network, profile)?;
        }
        "config" | "show" => println!(
            "{}",
            serde_json::to_string_pretty(&load(&directory(&r, n)?)?)?
        ),
        "start" | "run" => return Err("use env start NAME to start in the background, shell to open a terminal, or exec -- COMMAND to run a command".into()),
        "_serve" => {if args.len()!=2{return Err("invalid service invocation".into());}return super::background::serve(&r,n);},
        "attach" => return super::background::attach(&r,n,&args[2..]),
        "autostart" => super::background::autostart(&r,n,args.get(2).ok_or("expected on or off")?)?,
        "status" => super::background::status(&r,n)?,
        "import-store" | "import" => import(
            &r,
            n,
            Path::new(args.get(2).ok_or("existing store path required")?),
        )?,
        "terminal" => terminal(&r, n)?,
        "stop" => stop(&r, n)?,
        "configure" => configure(&r, n, &args[2..])?,
        "backup" => backup(&r, n, Path::new(args.get(2).ok_or("backup path required")?))?,
        "restore" => restore(&r, n, Path::new(args.get(2).ok_or("backup path required")?))?,
        "delete" => {
            if args.get(2).map(String::as_str) != Some("--yes") {
                return Err("delete requires --yes; create a backup first if needed".into());
            }
            let _lock = lock(&r, n)?;
            let dir = directory(&r, n)?;
            load(&dir)?;
            super::background::remove(&r,n)?;
            let vfs = Vfs::open_disk(dir.join("store"), DiskOptions::default())?;
            let trash = staging(&r, n)?;
            fs::remove_dir(&trash.0)?;
            fs::rename(&dir, &trash.0)?;
            drop(vfs);
            File::open(dir.parent().unwrap())?.sync_all()?;
        }
        "check" => {
            let _lock = lock(&r, n)?;
            let dir = directory(&r, n)?;
            load(&dir)?;
            let vfs = Vfs::open_disk(dir.join("store"), DiskOptions::default())?;
            println!(
                "Store opens successfully; {} inodes, {} logical bytes",
                vfs.capacity().nodes_used,
                vfs.capacity().used_bytes
            );
        }
        _ => {
            return Err(
                "usage: env list|create|packages|start|stop|configure|backup|restore|delete|check|doctor"
                    .into(),
            )
        }
    }
    Ok(0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn management_names_and_backup_paths_are_confined() {
        for n in ["../x", ".", "a/b", "", "-x;rm"] {
            assert!(name(n).is_err());
        }
        assert!(name("debian-work").is_ok());
        for p in [
            "/store/manifest",
            "store/../config.json",
            "store/objects/x.dat",
            "store/objects/0123456789abcdef.dat/other",
        ] {
            assert!(!backup_path(Path::new(p)));
        }
        assert!(backup_path(Path::new("store/objects/0123456789abcdef.dat")));
    }
    #[test]
    fn init_requires_compatible_image_and_old_configs_keep_shell() {
        let mut old = serde_json::to_value(Config::new("debian")).unwrap();
        old.as_object_mut().unwrap().remove("init");
        old.as_object_mut().unwrap().remove("target");
        let mut c: Config = serde_json::from_value(old).unwrap();
        assert!(c.init == super::super::systemd::Init::Shell);
        assert_eq!(c.target, trames_distro::boot::Target::Linux);
        c.init = super::super::systemd::Init::Systemd;
        assert!(c.validate().is_ok());
        c.distribution = "alpine".into();
        assert!(c.validate().is_err());
    }
    #[test]
    fn android_owns_its_init_graphics_and_separate_identity() {
        let mut c = Config::new("android");
        assert!(c.validate().is_ok());
        assert_eq!(c.command, trames_distro::android::command());
        c.init = super::super::systemd::Init::Systemd;
        assert!(c.validate().is_err());
        c.init = super::super::systemd::Init::Shell;
        c.target = trames_distro::boot::Target::Linux;
        assert!(c.validate().is_err());
    }
    #[test]
    fn future_config_and_invalid_limits_are_rejected() {
        let mut c = Config::new("alpine");
        c.schema = 2;
        assert!(c.validate().is_err());
        c.schema = 1;
        c.memory_mib = "18446744073709551615".into();
        assert!(c.validate().is_err());
    }
}
