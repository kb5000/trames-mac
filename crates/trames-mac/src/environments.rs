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
use trames_distro::environment as model;
pub use trames_distro::environment::Manifest as Config;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Instance {
    name: String,
    uuid: String,
    distribution: String,
    sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct CliConfig {
    version: u32,
    default_environment: String,
    defaults: Config,
}
impl Default for CliConfig {
    fn default() -> Self {
        Self {
            version: 1,
            default_environment: "default".into(),
            defaults: Config::default(),
        }
    }
}
fn load_cli(r: &Path) -> Result<CliConfig> {
    let p = r.join("config.toml");
    if !p.exists() {
        return Ok(CliConfig::default());
    }
    let c: CliConfig = model::decode(&fs::read_to_string(p)?)?;
    if c.version != 1 {
        return Err("unsupported CLI config version".into());
    }
    name(&c.default_environment)?;
    validate(&c.defaults)?;
    Ok(c)
}
pub fn default_environment(r: &Path) -> Result<String> {
    Ok(load_cli(r)?.default_environment)
}
fn cpus(c: &Config) -> Result<usize> {
    c.resources.cpus.resolve(
        std::thread::available_parallelism()
            .map_or(1, usize::from)
            .min(trames_hvf::max_parallel_cpus()),
    )
}
fn memory(c: &Config) -> Result<String> {
    Ok(model::memory_mib(
        c.resources
            .memory_limit
            .as_deref()
            .unwrap_or(&c.resources.memory),
    )?
    .map_or("auto".into(), |n| n.to_string()))
}
fn backend(c: &Config) -> Result<String> {
    Ok(match c.network.mode {
        model::NetworkMode::Host => "full",
        model::NetworkMode::None => "off",
        model::NetworkMode::Bridge => "outbound",
    }
    .into())
}
fn validate(c: &Config) -> Result<()> {
    c.validate()?;
    // Pure-function thunks accelerate the interpreter; HVF runs these instructions natively.
    c.components.thunking.resolve(false)?;
    cpus(c)?;
    if c.image.distribution == "alpine" && c.boot.mode == model::Init::Systemd {
        return Err("Alpine does not include systemd".into());
    }
    Ok(())
}
fn set_config(c: &mut Config, key: &str, value: &str) -> Result<()> {
    if key == "environment.name" || key == "environment.uuid" {
        return Err("use env rename/clone to change registered identity".into());
    }
    let mut json = serde_json::to_value(&*c)?;
    let parts: Vec<_> = key.split('.').collect();
    let mut slot = &mut json;
    for p in &parts[..parts.len() - 1] {
        slot = slot.get_mut(*p).ok_or("unknown configuration table")?;
    }
    let field = slot
        .get_mut(parts[parts.len() - 1])
        .ok_or("unknown configuration field")?;
    *field =
        serde_json::from_str(value).unwrap_or_else(|_| serde_json::Value::String(value.into()));
    let candidate: Config = serde_json::from_value(json)?;
    *c = candidate;
    Ok(())
}
fn metadata_lock(r: &Path) -> Result<File> {
    let guard = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(r.join("locks/.metadata"))?;
    guard.lock()?;
    Ok(guard)
}
pub fn settings(selected: Option<&str>, args: &[String]) -> Result<u8> {
    let r = root()?;
    let _metadata = metadata_lock(&r)?;
    let mut cli = load_cli(&r)?;
    let global = args.iter().any(|s| s == "--global");
    let args: Vec<_> = args
        .iter()
        .filter(|s| s.as_str() != "--global")
        .map(String::as_str)
        .collect();
    let op = args.first().copied().unwrap_or("show");
    let dir = directory(&r, selected.unwrap_or(&cli.default_environment))?;
    let path = if global {
        r.join("config.toml")
    } else {
        dir.join("env.toml")
    };
    if op == "path" {
        println!("{}", path.display());
        return Ok(0);
    }
    let mut c = if global {
        cli.defaults.clone()
    } else {
        load(&dir)?
    };
    match op {
        "show"|"list" if args.len()<=2=>{
            let value=if global{serde_json::to_value(&cli)?}else{serde_json::to_value(&c)?};
            if args.contains(&"--json"){println!("{}",serde_json::to_string_pretty(&value)?)}else if global {print!("{}",model::encode(&cli)?)}else{print!("{}",model::encode(&c)?)}
        }
        "validate" if args.len()==1=>{validate(&c)?;println!("Valid environment configuration");}
        "get" if args.len()==2=>{let v=if global{serde_json::to_value(&cli)?}else{serde_json::to_value(&c)?};let mut p=&v;for key in args[1].split('.') {p=p.get(key).ok_or("unknown configuration field")?;}if let Some(text)=p.as_str(){println!("{text}")}else{println!("{p}")}}
        "set" if args.len()>=3&&args.len()%2==1=>{
            for pair in args[1..].chunks_exact(2) {
                if global&&pair[0]=="default_environment"{name(pair[1])?;load(&directory(&r,pair[1])?)?;cli.default_environment=pair[1].into();}
                else{let key=if global{pair[0].strip_prefix("defaults.").ok_or("global environment keys use defaults.TABLE.FIELD")?}else{pair[0]};set_config(&mut c,key,pair[1])?;}
            }
            validate(&c)?;
            if global{cli.defaults=c;atomic_write(&path,model::encode(&cli)?.as_bytes())?;}else{save(&dir,&c)?;}
        }
        "edit" if args.len()==1=>{
            let temp=r.join(format!(".edit-{}.toml",std::process::id()));
            atomic_write(&temp,if global{model::encode(&cli)?}else{model::encode(&c)?}.as_bytes())?;
            let result=(||->Result<()>{
                let editor=std::env::var("EDITOR").unwrap_or("vi".into());
                if !Command::new(editor).arg(&temp).status()?.success(){return Err("editor failed".into());}
                let text=fs::read_to_string(&temp)?;
                if global{let value:CliConfig=model::decode(&text)?;if value.version!=1{return Err("unsupported CLI config version".into());}name(&value.default_environment)?;validate(&value.defaults)?;atomic_write(&path,model::encode(&value)?.as_bytes())?;}
                else{save(&dir,&Config::parse(&text)?)?;}Ok(())
            })();let _=fs::remove_file(temp);result?;
        }
        _=>return Err("usage: config show [--json] | path | validate | get KEY | set KEY VALUE [KEY VALUE...] | edit".into())
    }
    Ok(0)
}
pub fn shutdown() -> Result<u8> {
    let r = root()?;
    let mut names = vec![];
    for entry in fs::read_dir(r.join("environments"))? {
        let path = entry?.path().join("instance.toml");
        if path.is_file() {
            let instance: Instance = model::decode(&fs::read_to_string(path)?)?;
            names.push(instance.name);
        }
    }
    for n in names {
        stop(&r, &n)?;
    }
    Ok(0)
}
pub fn listing(args: &[String]) -> Result<u8> {
    list(
        &root()?,
        args.first().map(String::as_str).unwrap_or("table"),
    )?;
    Ok(0)
}
pub fn setup(args: &[String]) -> Result<u8> {
    let n = args.first().ok_or("environment required")?;
    let r = root()?;
    let _guard = lock(&r, n)?;
    let dir = directory(&r, n)?;
    let mut c = load(&dir)?;
    c.boot.mode = model::Init::Systemd;
    provision_init(&dir.join("store"), &c)?;
    save(&dir, &c)?;
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
    for entry in fs::read_dir(r.join("environments"))? {
        let path = entry?.path();
        let file = path.join("instance.toml");
        if file.is_file() {
            let i: Instance = model::decode(&fs::read_to_string(file)?)?;
            if i.name == n || i.uuid == n {
                return Ok(path);
            }
        }
    }
    Ok(r.join("environments").join(format!(".missing-{n}")))
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
        .open(
            r.join("locks").join(
                directory(r, n)?
                    .file_name()
                    .ok_or("invalid environment directory")?,
            ),
        )?;
    file.try_lock()
        .map_err(|_| "environment is running or another management operation is in progress")?;
    Ok(file)
}
pub(super) fn load(dir: &Path) -> Result<Config> {
    let i: Instance = model::decode(&fs::read_to_string(dir.join("instance.toml"))?)?;
    let path = if dir.join("env.toml").is_file() {
        dir.join("env.toml")
    } else {
        dir.join("effective.toml")
    };
    let mut c = if path.is_file() {
        Config::parse(&fs::read_to_string(path)?)?
    } else {
        Config::new(&i.distribution)
    };
    if !c.environment.uuid.is_empty() && c.environment.uuid != i.uuid {
        return Err("configuration UUID differs from registered environment".into());
    }
    c.environment.name = i.name;
    c.environment.uuid = i.uuid;
    c.image.sha256 = i.sha256;
    validate(&c)?;
    Ok(c)
}
pub(super) fn grants(dir: &Path, c: &Config) -> Result<model::Grant> {
    let path = dir.join("grants.toml");
    if !path.exists() {
        return Ok(Default::default());
    }
    let auth: model::Authorization = model::decode(&fs::read_to_string(path)?)?;
    Ok(auth.for_environment(&c.environment.uuid)?.clone())
}
fn grant(r: &Path, n: &str, args: &[String]) -> Result<()> {
    let _guard = metadata_lock(r)?;
    let dir = directory(r, n)?;
    let c = load(&dir)?;
    let mut g = grants(&dir, &c)?;
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] | ["show"] => {
            print!("{}", model::encode(&g)?);
            return Ok(());
        }
        ["share", alias, host, mode] => {
            name(alias)?;
            let host = fs::canonicalize(host)?;
            if !host.is_dir() {
                return Err("share must be a directory".into());
            }
            let readonly = match *mode {
                "ro" => true,
                "rw" => false,
                _ => return Err("share mode must be ro or rw".into()),
            };
            g.shares.insert(
                (*alias).into(),
                model::Share {
                    host: host.to_str().ok_or("share path is not UTF-8")?.into(),
                    readonly,
                },
            );
        }
        ["unshare", alias] => {
            g.shares.remove(*alias);
        }
        [cap, value] => {
            let enabled = match *value {
                "on" => true,
                "off" => false,
                _ => return Err("grant value must be on or off".into()),
            };
            *match *cap {
                "host-ui" => &mut g.host_ui,
                "clipboard" => &mut g.clipboard,
                "host-cli" => &mut g.host_cli,
                "host-exec" => &mut g.host_exec,
                "files" => &mut g.files,
                "ports" => &mut g.ports,
                "mounts" => &mut g.mounts,
                _ => return Err("unknown host capability".into()),
            } = enabled;
        }
        _ => return Err(
            "usage: env grant NAME [CAPABILITY on|off | share ALIAS HOST ro|rw | unshare ALIAS]"
                .into(),
        ),
    }
    atomic_write(
        &dir.join("grants.toml"),
        model::encode(&model::Authorization {
            environment_uuid: c.environment.uuid,
            grant: g,
        })?
        .as_bytes(),
    )?;
    println!("Host grants saved; restart the environment to apply");
    Ok(())
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
fn register(dir: &Path, c: &Config, explicit: bool) -> Result<()> {
    validate(c)?;
    let new_instance = !dir.join("instance.toml").exists();
    let i = Instance {
        name: c.environment.name.clone(),
        uuid: c.environment.uuid.clone(),
        distribution: c.image.distribution.clone(),
        sha256: c.image.sha256.clone(),
    };
    if new_instance {
        let grant = model::Authorization {
            environment_uuid: c.environment.uuid.clone(),
            grant: model::Grant {
                clipboard: true,
                ..Default::default()
            },
        };
        atomic_write(&dir.join("grants.toml"), model::encode(&grant)?.as_bytes())?;
    }
    atomic_write(&dir.join("instance.toml"), model::encode(&i)?.as_bytes())?;
    atomic_write(
        &dir.join(if explicit {
            "env.toml"
        } else {
            "effective.toml"
        }),
        model::encode(c)?.as_bytes(),
    )
}
fn save(dir: &Path, c: &Config) -> Result<()> {
    if dir.join("instance.toml").is_file() {
        let i: Instance = model::decode(&fs::read_to_string(dir.join("instance.toml"))?)?;
        if i.uuid != c.environment.uuid || i.name != c.environment.name {
            return Err("use env rename/clone to change registered identity".into());
        }
    }
    register(dir, c, true)
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
/// Cold optional-image download; execution workers never use this path.
fn android_images(r: &Path, archive: &Path) -> Result<Option<PathBuf>> {
    let Some(plan) = trames_distro::android_package::plan(archive)? else {
        return Ok(None);
    };
    let cache = std::env::var_os("TRAMES_ANDROID_IMAGE_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| r.join("cache/android"));
    private_dir(&cache)?;
    for image in plan.images {
        let path = cache.join(&image.filename);
        if path.is_file() && path.metadata()?.len() == image.size && digest(&path)? == image.sha256
        {
            continue;
        }
        eprintln!(
            "Downloading Android {} ({} bytes)…",
            image.image_name, image.size
        );
        let temp = cache.join(format!("{}.{}.part", image.filename, std::process::id()));
        let result = (|| -> Result<()> {
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
                    "1800",
                    "--max-filesize",
                ])
                .arg(image.size.to_string())
                .arg("--output")
                .arg(&temp)
                .arg(&image.url)
                .status()?;
            if !status.success()
                || temp.metadata()?.len() != image.size
                || digest(&temp)? != image.sha256
            {
                return Err("Android upstream download failed size/SHA-256 verification".into());
            }
            File::open(&temp)?.sync_all()?;
            fs::rename(&temp, &path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result?;
    }
    Ok(Some(cache))
}
struct Create<'a> {
    archive: Option<&'a str>,
    custom: Option<&'a str>,
    expected: Option<&'a str>,
    packages: Option<super::packages::Profile>,
    explicit: bool,
}
fn create(r: &Path, n: &str, mut c: Config, options: Create<'_>) -> Result<()> {
    let Create {
        archive,
        custom,
        expected,
        packages,
        explicit,
    } = options;
    let _lock = lock(r, n)?;
    let existing = directory(r, n)?;
    if existing.try_exists()? {
        return Err("environment already exists".into());
    }
    let dir = r.join("environments").join(&c.environment.uuid);
    if dir.try_exists()? {
        return Err("environment UUID already exists".into());
    }
    if custom.is_some() && archive.is_some() {
        return Err("choose --rootfs or --archive".into());
    }
    if expected.is_some() && custom.is_none() {
        return Err("--sha256 requires --rootfs".into());
    }
    if custom.is_some() {
        c.image.distribution = if c.environment.kind == trames_distro::boot::Target::Android {
            "android"
        } else {
            "custom"
        }
        .into();
    }
    if c.environment.kind == trames_distro::boot::Target::Android && custom.is_none() {
        return Err(
            "Android is an optional download; supply --target android --rootfs PACKAGE.tar.gz"
                .into(),
        );
    }
    validate(&c)?;
    use super::packages::Profile;
    let packages = packages.unwrap_or(if c.image.distribution == "debian" {
        Profile::Standard
    } else {
        Profile::Minimal
    });
    if c.image.distribution != "debian" && packages != Profile::Minimal {
        return Err("package presets require the built-in Debian image; custom rootfs and Alpine preserve their own package setup".into());
    }
    let source = if custom.is_none() {
        Some(source(r, &c.image.distribution, archive)?)
    } else {
        None
    };
    let android_images = if c.environment.kind == trames_distro::boot::Target::Android {
        android_images(
            r,
            Path::new(custom.ok_or("Android adapter/rootfs archive required")?),
        )?
    } else {
        None
    };
    eprintln!("Importing {} rootfs…", c.image.distribution);
    let stage = staging(r, n)?;
    let mut vfs = Vfs::open_disk(stage.0.join("store"), DiskOptions::default())?;
    c.image.sha256 = vfs.populate_staged(|vfs| -> Result<String> {
        if let Some(custom) = custom {
            if let Some(images) = &android_images {
                return trames_distro::android_package::install(
                    Path::new(custom),
                    images,
                    vfs,
                    expected,
                );
            }
            return super::rootfs::install(Path::new(custom), vfs, expected);
        }
        let mut input = File::open(source.as_ref().unwrap())?;
        match c.image.distribution.as_str() {
            "alpine" => {
                trames_distro::install_alpine(&mut input, vfs)?;
            }
            "debian" => {
                trames_distro::install_debian(&mut input, vfs)?;
            }
            _ => unreachable!(),
        }
        Ok(catalog(&c.image.distribution)?.1.into())
    })?;
    if !explicit {
        if let Some(mut embedded) = model::guest(&mut vfs, &c)? {
            embedded.environment = c.environment.clone();
            embedded.image = c.image.clone();
            c = embedded;
        }
    }
    if c.environment.kind == trames_distro::boot::Target::Linux {
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
    if c.environment.kind == trames_distro::boot::Target::Android {
        trames_distro::android::validate_image(&mut vfs)?;
    } else if custom.is_some() {
        super::rootfs::validate_command(&mut vfs, &c.session.shell)?;
    }
    vfs.sync()?;
    drop(vfs);
    super::packages::install(&stage.0.join("store"), cpus(&c)?, &backend(&c)?, packages)?;
    provision_init(&stage.0.join("store"), &c)?;
    register(&stage.0, &c, explicit)?;
    publish(&stage.0, &dir)?;
    println!("Created {n} ({})", c.image.distribution);
    Ok(())
}
fn provision_init(store: &Path, c: &Config) -> Result<()> {
    if c.boot.mode != model::Init::Systemd {
        return Ok(());
    }
    if c.image.distribution == "custom" {
        let vfs = Vfs::open_disk(store, DiskOptions::default())?;
        vfs.executable(b"/usr/lib/systemd/systemd")
            .map_err(|_| "custom systemd rootfs must already contain /usr/lib/systemd/systemd")?;
        Ok(())
    } else {
        super::systemd::install(store, cpus(c)?, &backend(c)?)
    }
}
pub(super) fn identity(r: &Path, n: &str) -> Result<String> {
    let i: Instance = model::decode(&fs::read_to_string(directory(r, n)?.join("instance.toml"))?)?;
    Ok(i.uuid)
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
    if !directory(r, n).is_ok_and(|p| p.join("instance.toml").is_file()) {
        return false;
    }
    lock(r, n).is_err()
}
pub(super) fn start(r: &Path, n: &str, extra: &[String]) -> Result<u8> {
    let _lock = lock(r, n)?;
    let dir = directory(r, n)?;
    let mut c = load(&dir)?;
    if !dir.join("env.toml").is_file() {
        let mut defaults = load_cli(r)?.defaults.using_image(&c.image.distribution);
        defaults.environment = c.environment.clone();
        defaults.image = c.image.clone();
        c = defaults;
        let mut vfs = Vfs::open_disk(dir.join("store"), DiskOptions::default())?;
        if let Some(mut embedded) = model::guest(&mut vfs, &c)? {
            embedded.environment = c.environment.clone();
            embedded.image = c.image.clone();
            c = embedded;
        }
        drop(vfs);
        atomic_write(&dir.join("effective.toml"), model::encode(&c)?.as_bytes())?;
    }
    let _control = super::lifecycle::SessionControl::bind(&socket(r, n)?)?;
    let mut args = vec![
        "--storage".into(),
        dir.join("store").to_string_lossy().into_owned(),
        "--cpus".into(),
        cpus(&c)?.to_string(),
        "--memory-mib".into(),
        memory(&c)?,
        "--network".into(),
        backend(&c)?,
    ];
    if c.network.mode == model::NetworkMode::Bridge {
        args.extend(super::networks::arguments(r, &c)?);
    }
    args.extend(["--host-profile".into(), dir.to_string_lossy().into_owned()]);
    if !c.display.windows {
        args.push("--no-windows".into());
    }
    if c.components.vulkan.resolve(true)? {
        args.push("--vulkan".into());
    }
    args.extend(["--cwd".into(), c.session.cwd.clone()]);
    if c.components.audio {
        args.push("--audio".into());
    }
    if c.components.audio_input {
        args.push("--audio-input".into());
    }
    if c.components.vaapi.resolve(true)? {
        args.push("--video".into());
    }
    let grant = grants(&dir, &c)?;
    for m in &grant.mounts(&c)? {
        let canonical = fs::canonicalize(m.host.as_ref().ok_or("missing mount source")?)?;
        if !grant
            .shares
            .values()
            .any(|share| canonical.starts_with(&share.host))
        {
            return Err("mount target resolves outside host grants".into());
        }
        args.extend([
            if m.readonly { "--mount-ro" } else { "--mount" }.into(),
            format!(
                "{}={}",
                m.target,
                m.host.as_ref().ok_or("named share requires a host grant")?
            ),
        ]);
    }
    if c.environment.kind == trames_distro::boot::Target::Android {
        args.extend(["--android-init".into(), "--no-pty".into()]);
    }
    // An explicit command is a maintenance session; require an explicit shell
    // override rather than silently bypassing this environment's init setting.
    args.extend([
        "--init".into(),
        match c.boot.mode {
            model::Init::Shell => "shell",
            model::Init::Systemd => "systemd",
            model::Init::Android => "shell",
        }
        .into(),
    ]);
    let extra = extra.strip_prefix(&["--".into()]).unwrap_or(extra);
    args.extend_from_slice(extra);
    Ok(super::execute_with_default(args, &c.command())?)
}
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}
fn terminal(r: &Path, n: &str) -> Result<()> {
    let c = load(&directory(r, n)?)?;
    if c.environment.kind == trames_distro::boot::Target::Android {
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
    identity(r, n)?;
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
                name: c.environment.name.clone(),
                distribution: c.image.distribution.clone(),
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
    let mut c = load_cli(r)?.defaults.using_image(d);
    c.environment.name = n.into();
    c.environment.uuid = model::uuid()?;
    c.image.sha256 = "imported-existing-store".into();
    save(&stage.0, &c)?;
    let mut copy = Vfs::open_disk(stage.0.join("store"), DiskOptions::default())?;
    if c.environment.kind == trames_distro::boot::Target::Android {
        trames_distro::android::validate_image(&mut copy)?;
    } else {
        super::systemd::renew_identity(&mut copy)?;
    }
    drop(copy);
    publish(&stage.0, &r.join("environments").join(&c.environment.uuid))?;
    drop(vfs);
    println!("Imported {n}; original store is unchanged");
    Ok(())
}
fn configure(r: &Path, n: &str, args: &[String]) -> Result<()> {
    let _lock = lock(r, n)?;
    let dir = directory(r, n)?;
    let mut c = load(&dir)?;
    apply_settings(&mut c, args)?;
    if c.session.shell != load(&dir)?.session.shell {
        let mut vfs = Vfs::open_disk(dir.join("store"), DiskOptions::default())?;
        super::rootfs::validate_command(&mut vfs, &c.session.shell)?;
    }
    save(&dir, &c)
}
fn apply_settings(c: &mut Config, settings: &[String]) -> Result<()> {
    let mut args = settings.iter();
    while let Some(key) = args.next() {
        if key == "--" {
            c.session.shell = args.cloned().collect();
            break;
        }
        let value = args.next().ok_or("missing setting value")?;
        let path = match key.as_str() {
            "--cpus" => "resources.cpus",
            "--memory" => "resources.memory",
            "--network" => "network.mode",
            "--bridge" => "network.bridge",
            "--init" => "boot.mode",
            "--vulkan" => "components.vulkan",
            "--vaapi" => "components.vaapi",
            "--thunking" => "components.thunking",
            _ => return Err(format!("unknown create option {key}").into()),
        };
        set_config(c, path, value)?;
    }
    validate(c)
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
            PathBuf::from("instance.toml"),
            PathBuf::from("store/manifest"),
        ];
        for e in fs::read_dir(dir.join("store/objects"))? {
            paths.push(PathBuf::from("store/objects").join(e?.file_name()));
        }
        for file in ["env.toml", "effective.toml"] {
            if dir.join(file).is_file() {
                paths.push(file.into());
            }
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
    matches!(s, "instance.toml" | "env.toml" | "effective.toml")
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
    c.environment.name = n.into();
    c.environment.uuid = model::uuid()?;
    c.mounts.clear();
    register(&stage.0, &c, true)?;
    let mut vfs = Vfs::open_disk(stage.0.join("store"), DiskOptions::default())?;
    super::systemd::renew_identity(&mut vfs)?;
    drop(vfs);
    publish(&stage.0, &r.join("environments").join(&c.environment.uuid))?;
    println!("Restored {n}; shared directories must be selected again");
    Ok(())
}
pub fn dispatch(args: &[String]) -> Result<u8> {
    let r = root()?;
    let op = args.first().map(String::as_str).unwrap_or("ls");
    if op == "list" || op == "ls" {
        return listing(&args[1..]);
    }
    if op == "create" {
        let _metadata = metadata_lock(&r)?;
        let positional = args.get(1).filter(|s| !s.starts_with('-'));
        let mut c = load_cli(&r)?.defaults;
        let (mut archive, mut custom, mut expected, mut profile, mut distro) =
            (None, None, None, None, None);
        let mut packages = None;
        let mut settings = vec![];
        let mut it = args[if positional.is_some() { 2 } else { 1 }..].iter();
        while let Some(k) = it.next() {
            if k == "--" {
                settings.push(k.clone());
                settings.extend(it.cloned());
                break;
            }
            let v = it.next().ok_or("missing create value")?;
            match k.as_str() {
                "--config" => profile = Some(v),
                "--distro" => distro = Some(v),
                "--rootfs" => custom = Some(v.as_str()),
                "--archive" => archive = Some(v.as_str()),
                "--sha256" => expected = Some(v.as_str()),
                "--packages" => packages = Some(super::packages::Profile::parse(v)?),
                _ => settings.extend([k.clone(), v.clone()]),
            }
        }
        if let Some(d) = distro {
            c = c.using_image(d);
        }
        if let Some(path) = profile {
            c = Config::parse(&fs::read_to_string(path)?)?;
        }
        if let Some(n) = positional {
            c.environment.name = n.clone();
        }
        name(&c.environment.name)?;
        if c.environment.uuid.is_empty() {
            c.environment.uuid = model::uuid()?;
        }
        apply_settings(&mut c, &settings)?;
        let source = c.image.source.clone();
        let custom = custom.or(source.as_deref());
        let sha = c.image.sha256.clone();
        let expected = expected.or((!sha.is_empty()).then_some(sha.as_str()));
        let n = c.environment.name.clone();
        let explicit = profile.is_some() || !settings.is_empty();
        create(
            &r,
            &n,
            c,
            Create {
                archive,
                custom,
                expected,
                packages,
                explicit,
            },
        )?;
        return Ok(0);
    }
    if op == "doctor" {
        println!(
            "{}",
            serde_json::json!({"registry":r,"runtime":include_str!("../../../runtime-version.json")})
        );
        return Ok(0);
    }
    let n = args.get(1).ok_or("environment name required")?;
    name(n)?;
    match op {
        "grant" => grant(&r, n, &args[2..])?,
        "share" => {
            let _metadata = metadata_lock(&r)?;
            let dir = directory(&r, n)?;
            let mut c = load(&dir)?;
            let alias = args.get(2).ok_or("share alias required")?;
            name(alias)?;
            let host = fs::canonicalize(args.get(3).ok_or("host directory required")?)?;
            if !host.is_dir() {
                return Err("share must be a directory".into());
            }
            let target = args.get(4).ok_or("guest mount path required")?;
            let readonly = match args.get(5).map(String::as_str).unwrap_or("rw") {
                "ro" => true,
                "rw" => false,
                _ => return Err("expected ro or rw".into()),
            };
            let mut g = grants(&dir, &c)?;
            g.shares.insert(
                alias.clone(),
                model::Share {
                    host: host.to_str().ok_or("host path must be UTF-8")?.into(),
                    readonly,
                },
            );
            c.mounts.retain(|m| m.target != *target);
            c.mounts.push(model::Mount {
                target: target.clone(),
                share: Some(alias.clone()),
                host: None,
                readonly,
            });
            validate(&c)?;
            atomic_write(
                &dir.join("grants.toml"),
                model::encode(&model::Authorization {
                    environment_uuid: c.environment.uuid.clone(),
                    grant: g,
                })?
                .as_bytes(),
            )?;
            save(&dir, &c)?;
        }
        "packages" => {
            let _guard = lock(&r, n)?;
            let c = load(&directory(&r, n)?)?;
            if c.image.distribution != "debian" {
                return Err("package presets require Debian".into());
            }
            super::packages::install(
                &directory(&r, n)?.join("store"),
                cpus(&c)?,
                &backend(&c)?,
                super::packages::Profile::parse(args.get(2).ok_or("package preset required")?)?,
            )?;
        }

        "config" | "show" => println!("{}", model::encode(&load(&directory(&r, n)?)?)?),
        "_serve" => return super::background::serve(&r, n),
        "attach" => return super::background::attach(&r, n, &args[2..]),
        "autostart" => super::background::autostart(&r, n, args.get(2).ok_or("expected on/off")?)?,
        "status" => super::background::status(&r, n)?,
        "terminal" => terminal(&r, n)?,
        "stop" => stop(&r, n)?,
        "configure" => configure(&r, n, &args[2..])?,
        "backup" | "export" => {
            backup(&r, n, Path::new(args.get(2).ok_or("backup path required")?))?
        }
        "restore" => restore(&r, n, Path::new(args.get(2).ok_or("archive required")?))?,
        "import-store" => import(&r, n, Path::new(args.get(2).ok_or("store path required")?))?,
        "rename" => {
            let _metadata = metadata_lock(&r)?;
            let _guard = lock(&r, n)?;
            let mut c = load(&directory(&r, n)?)?;
            let new = args.get(2).ok_or("new name required")?;
            name(new)?;
            if directory(&r, new)?.exists() {
                return Err("name already exists".into());
            }
            let path = directory(&r, n)?;
            c.environment.name = new.clone();
            let explicit = path.join("env.toml").is_file();
            register(&path, &c, explicit)?;
            let mut cli = load_cli(&r)?;
            if cli.default_environment == *n {
                cli.default_environment = new.clone();
                atomic_write(&r.join("config.toml"), model::encode(&cli)?.as_bytes())?;
            }
        }
        "remove" => {
            if args.get(2).map(String::as_str) != Some("--yes") {
                return Err("remove requires --yes".into());
            }
            let _guard = lock(&r, n)?;
            let path = directory(&r, n)?;
            super::background::remove(&r, n)?;
            let vfs = Vfs::open_disk(path.join("store"), DiskOptions::default())?;
            let trash = staging(&r, n)?;
            fs::remove_dir(&trash.0)?;
            fs::rename(path, &trash.0)?;
            drop(vfs);
        }
        _ => return Err(format!("unknown environment operation {op}").into()),
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("trames-config-{}", model::uuid().unwrap()));
            for d in [p.clone(), p.join("environments"), p.join("locks")] {
                private_dir(&d).unwrap();
            }
            Self(p)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn identity_survives_alias_changes_and_grants_are_not_portable() {
        let f = Fixture::new();
        let mut c = Config::new("debian");
        c.environment.name = "work".into();
        c.environment.uuid = model::uuid().unwrap();
        let dir = f.0.join("environments").join(&c.environment.uuid);
        private_dir(&dir).unwrap();
        register(&dir, &c, true).unwrap();
        assert_eq!(directory(&f.0, "work").unwrap(), dir);
        assert_eq!(directory(&f.0, &c.environment.uuid).unwrap(), dir);
        let original = socket(&f.0, "work").unwrap();
        c.environment.name = "renamed".into();
        register(&dir, &c, true).unwrap();
        assert_eq!(socket(&f.0, "renamed").unwrap(), original);
        let authorization = model::Authorization {
            environment_uuid: c.environment.uuid.clone(),
            grant: model::Grant {
                host_exec: true,
                ..Default::default()
            },
        };
        atomic_write(
            &dir.join("grants.toml"),
            model::encode(&authorization).unwrap().as_bytes(),
        )
        .unwrap();
        assert!(grants(&dir, &c).unwrap().host_exec);
        c.environment.uuid = model::uuid().unwrap();
        assert!(grants(&dir, &c).is_err());
        assert!(!backup_path(Path::new("grants.toml")));
    }
    #[test]
    fn dotted_updates_validate_as_one_transaction_and_do_not_modify_identity() {
        let mut c = Config::default();
        set_config(&mut c, "network.mode", "bridge").unwrap();
        assert!(validate(&c).is_err());
        set_config(&mut c, "network.bridge", "private").unwrap();
        validate(&c).unwrap();
        assert!(set_config(&mut c, "environment.uuid", "another").is_err());
        assert!(set_config(&mut c, "resources.typo", "4").is_err());
        let text = model::encode(&c).unwrap();
        assert!(text.contains("[resources]"));
        assert_eq!(
            Config::parse(&text).unwrap().network.bridge.as_deref(),
            Some("private")
        );
    }
    #[test]
    fn config_is_toml_only_and_preserves_explicit_profile_precedence() {
        let f = Fixture::new();
        atomic_write(
            &f.0.join("cli.json"),
            b"{\"default_environment\":\"legacy\"}",
        )
        .unwrap();
        assert_eq!(load_cli(&f.0).unwrap().default_environment, "default");
        let mut c = Config::default();
        c.environment.name = "work".into();
        c.environment.uuid = model::uuid().unwrap();
        let dir = f.0.join("environments").join(&c.environment.uuid);
        private_dir(&dir).unwrap();
        register(&dir, &c, false).unwrap();
        c.resources.cpus = model::CpuCount::Count(1);
        save(&dir, &c).unwrap();
        let mut cache = c.clone();
        cache.resources.cpus = model::CpuCount::Count(2);
        atomic_write(
            &dir.join("effective.toml"),
            model::encode(&cache).unwrap().as_bytes(),
        )
        .unwrap();
        assert_eq!(load(&dir).unwrap().resources.cpus.resolve(12).unwrap(), 1);
    }
}
