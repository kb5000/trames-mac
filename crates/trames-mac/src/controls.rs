// SPDX-License-Identifier: GPL-3.0-or-later
//! Owner-only controls. Cold CLI operations use the existing registry and exec
//! pipes; no guest data operation is routed through a coordinator.
use super::*;
use serde_json::{json, Value};
use trames_control::{Backend, Error, Options};
fn failure(e: impl std::fmt::Display) -> Error {
    Error::new("host_error", e.to_string(), 1)
}
pub(super) fn dispatch(selected: Option<&str>, args: &[String]) -> Result<u8> {
    if selected.is_some()
        && trames_control::Options::parse(args).is_ok_and(|(_, v)| {
            v.first().is_some_and(|s| s == "permissions")
                && v.get(1).is_some_and(|s| s == "defaults")
        })
    {
        return Err("permissions defaults is host-global and cannot be combined with -e".into());
    }
    let root = root()?;
    let name = selected
        .map(str::to_owned)
        .unwrap_or(default_environment(&root)?);
    let mut owner = Owner { root, name };
    Ok(trames_control::run(
        &mut owner,
        args,
        &mut io::stdout(),
        &mut io::stderr(),
    ))
}
struct Owner {
    root: PathBuf,
    name: String,
}
impl Owner {
    fn config(&self) -> trames_control::Result<(PathBuf, Config, model::Grant)> {
        let dir = directory(&self.root, &self.name).map_err(failure)?;
        let c = load(&dir).map_err(failure)?;
        let g = grants(&dir, &c).map_err(failure)?;
        Ok((dir, c, g))
    }
    fn request(c: &Config, argv: Vec<String>) -> super::super::exec::Request {
        super::super::exec::Request {
            argv,
            environment: vec![],
            user: (!c.session.user.is_empty()).then(|| c.session.user.clone()),
            cwd: (!c.session.cwd.is_empty()).then(|| c.session.cwd.clone()),
            tty: false,
            size: Some((80, 24)),
        }
    }
    fn program(c: &Config) -> &'static str {
        if c.environment.kind == trames_distro::boot::Target::Android {
            "/system/bin/trames"
        } else {
            "/usr/local/bin/trames"
        }
    }
}
impl Backend for Owner {
    fn info(&mut self) -> trames_control::Result<Value> {
        let (_, c, g) = self.config()?;
        let mut value = model::control_info(&c, &g, "macos");
        value["running"] = json!(running(&self.root, &self.name));
        value["connection"] = json!(if running(&self.root, &self.name) {
            "running"
        } else {
            "stopped"
        });
        Ok(value)
    }
    fn permission(&mut self, args: &[String]) -> trames_control::Result<Value> {
        let _guard = metadata_lock(&self.root).map_err(failure)?;
        let defaults = args.first().is_some_and(|s| s == "defaults");
        let args = if defaults { &args[1..] } else { args };
        let mut cli = load_cli(&self.root).map_err(failure)?;
        let mut current = if defaults {
            cli.permission_defaults.clone()
        } else {
            self.config()?.2
        };
        if !args.is_empty() && args != ["show"] {
            let allowed=match args[0].as_str(){"allow"=>true,"deny"=>false,_=>return Err(Error::usage("permissions [show|allow|deny CAPABILITY|--all]; permissions defaults [allow|deny CAPABILITY|--all]"))};
            if args.len() < 2 {
                return Err(Error::usage("Choose a capability or --all"));
            }
            let keys: Vec<_> = if args[1..] == ["--all"] {
                trames_control::CAPABILITIES
                    .iter()
                    .map(|(k, _)| *k)
                    .collect()
            } else {
                args[1..].iter().map(String::as_str).collect()
            };
            for key in &keys {
                current.set_permission(key, allowed).map_err(failure)?;
            }
            if defaults {
                cli.permission_defaults = current.clone();
                atomic_write(
                    &self.root.join("config.toml"),
                    model::encode(&cli).map_err(failure)?.as_bytes(),
                )
                .map_err(failure)?;
            } else {
                let (dir, mut c, _) = self.config()?;
                let mut value = serde_json::to_value(&c).map_err(failure)?;
                for key in keys {
                    value["interop"][key.replace('-', "_")]["enabled"] = json!(allowed);
                }
                c = serde_json::from_value(value).map_err(failure)?;
                let auth = model::Authorization {
                    environment_uuid: c.environment.uuid.clone(),
                    grant: current.clone(),
                };
                // Publish restrictions first. A partial two-file update cannot
                // broaden access before the owner manifest has been saved.
                if !allowed {
                    atomic_write(
                        &dir.join("grants.toml"),
                        model::encode(&auth).map_err(failure)?.as_bytes(),
                    )
                    .map_err(failure)?;
                }
                save(&dir, &c).map_err(failure)?;
                if allowed {
                    atomic_write(
                        &dir.join("grants.toml"),
                        model::encode(&auth).map_err(failure)?.as_bytes(),
                    )
                    .map_err(failure)?;
                }
            }
        }
        if defaults {
            Ok(json!(trames_control::CAPABILITIES
                .iter()
                .map(|(k, _)| (
                    k.to_string(),
                    json!({"allowed":current.permission(k),"supported":true})
                ))
                .collect::<BTreeMap<_, _>>()))
        } else {
            let (_, c, g) = self.config()?;
            Ok(model::control_info(&c, &g, "macos")["capabilities"].clone())
        }
    }
    fn call(&mut self, device: &str, method: &str, params: Value) -> trames_control::Result<Value> {
        let (_, c, g) = self.config()?;
        if method == "files.list" {
            let info = model::control_info(&c, &g, "macos");
            if info["capabilities"]["files"]["allowed"] != true {
                return Err(Error::denied("File exchange is disabled"));
            }
            if let Some(path) = params["path"].as_str() {
                let guest = path
                    .strip_prefix("guest:")
                    .ok_or_else(|| Error::usage("Use guest:/absolute/path to list guest files"))?;
                if !guest.starts_with('/') {
                    return Err(Error::usage("Guest path must be absolute"));
                }
                if running(&self.root, &self.name) {
                    let request = Self::request(
                        &c,
                        vec![
                            Self::program(&c).into(),
                            "--json".into(),
                            "files".into(),
                            "ls".into(),
                            guest.into(),
                        ],
                    );
                    let (code, out, err) = super::super::exec::capture(
                        &console_socket(&self.root, &self.name).map_err(failure)?,
                        request,
                    )
                    .map_err(Error::from)?;
                    if code != 0 {
                        return Err(failure(String::from_utf8_lossy(&err)));
                    }
                    let value: Value = serde_json::from_slice(&out).map_err(failure)?;
                    return Ok(value["data"].clone());
                }
                let _lock = lock(&self.root, &self.name).map_err(failure)?;
                let (dir, _, _) = self.config()?;
                let mut vfs =
                    Vfs::open_disk(dir.join("store"), DiskOptions::default()).map_err(failure)?;
                let entries = super::super::files::names(&mut vfs, guest.as_bytes())
                    .map_err(failure)?
                    .into_iter()
                    .map(|v| String::from_utf8_lossy(&v).into_owned())
                    .collect::<Vec<_>>();
                return Ok(json!({"entries":entries}));
            }
            return Ok(json!({"shares":info["shares"]}));
        }
        if method.starts_with("clipboard.") {
            if !g.effective(&c.interop).clipboard.enabled {
                return Err(Error::denied("Clipboard is disabled"));
            }
            return super::super::interop::clipboard(&trames_bridge::Request {
                version: 1,
                id: 1,
                method: method.into(),
                params,
            })
            .map_err(failure);
        }
        if !running(&self.root, &self.name) {
            return Err(Error::unavailable(
                "This operation needs a running environment; use env start first",
            ));
        }
        let request = Self::request(
            &c,
            vec![
                Self::program(&c).into(),
                "--json".into(),
                "debug".into(),
                "rpc".into(),
                device.into(),
                method.into(),
                params.to_string(),
            ],
        );
        let (code, out, err) = super::super::exec::capture(
            &console_socket(&self.root, &self.name).map_err(failure)?,
            request,
        )
        .map_err(failure)?;
        if code != 0 {
            return Err(failure(String::from_utf8_lossy(&err)));
        }
        let reply: Value = serde_json::from_slice(&out).map_err(failure)?;
        if let Some(error) = reply.get("error") {
            return Err(failure(error));
        }
        Ok(reply)
    }
    fn stream(
        &mut self,
        _device: &str,
        _method: &str,
        _argv: &[String],
        _o: &Options,
        _out: &mut dyn Write,
        _err: &mut dyn Write,
    ) -> trames_control::Result<u8> {
        Err(Error::usage(
            "On the host use exec -- COMMAND to execute inside the selected environment",
        ))
    }
    fn copy(
        &mut self,
        src: &str,
        dst: &str,
        o: &trames_control::copy::Options,
    ) -> trames_control::Result<Value> {
        let (dir, c, g) = self.config()?;
        if !g.effective(&c.interop).files.enabled {
            return Err(Error::denied("File exchange is disabled"));
        }
        let (guest, host, upload) = match (src.strip_prefix("guest:"), dst.strip_prefix("guest:")) {
            (Some(a), None) => (a, Path::new(dst), false),
            (None, Some(b)) => (b, Path::new(src), true),
            _ => {
                return Err(Error::usage(
                    "Use exactly one guest:/absolute/path and one host path",
                ))
            }
        };
        if !guest.starts_with('/') {
            return Err(Error::usage("Guest paths must be absolute"));
        }
        if o.interactive {
            return Err(Error::usage(
                "Host transfers support -n and -r; choose the destination explicitly",
            ));
        }
        if running(&self.root, &self.name) {
            use std::os::unix::net::UnixStream;
            let (a, b) = UnixStream::pair().map_err(failure)?;
            let socket = console_socket(&self.root, &self.name).map_err(failure)?;
            let mut argv = vec![
                Self::program(&c).into(),
                "__file".into(),
                if upload { "receive" } else { "send" }.into(),
                guest.into(),
            ];
            if o.recursive {
                argv.push("-r".into());
            }
            if o.no_clobber {
                argv.push("-n".into());
            }
            let request = Self::request(&c, argv);
            // A scoped encoder/decoder exists only for this explicit copy. The
            // running guest still owns its original VFS and existing exec pipe.
            let code = std::thread::scope(|scope| -> trames_control::Result<u8> {
                if upload {
                    if fs::symlink_metadata(host).map_err(failure)?.is_dir() && !o.recursive {
                        return Err(Error::usage("Directory copy requires -r"));
                    }
                    let encoder = scope.spawn(move || trames_control::archive::send(host, b));
                    let code = super::super::exec::run_streams_deadline(
                        &socket,
                        request,
                        a,
                        &mut io::sink(),
                        &mut io::stderr(),
                        o.deadline,
                    )
                    .map_err(Error::from);
                    let encoded = encoder.join().map_err(|_| failure("File encoder failed"))?;
                    let code = code?;
                    if code == 0 {
                        encoded?;
                    }
                    Ok(code)
                } else {
                    let decoder = scope.spawn(move || trames_control::archive::receive(b, host, o));
                    let mut output = a;
                    let code = super::super::exec::run_streams_deadline(
                        &socket,
                        request,
                        File::open("/dev/null").map_err(failure)?,
                        &mut output,
                        &mut io::stderr(),
                        o.deadline,
                    )
                    .map_err(Error::from);
                    drop(output);
                    let decoded = decoder.join().map_err(|_| failure("File decoder failed"))?;
                    let code = code?;
                    if code == 0 {
                        decoded?;
                    }
                    Ok(code)
                }
            })?;
            if code != 0 {
                return Err(failure(format!("Guest copy failed (exit {code})")));
            }
        } else {
            let _lock = lock(&self.root, &self.name).map_err(failure)?;
            let mut vfs =
                Vfs::open_disk(dir.join("store"), DiskOptions::default()).map_err(failure)?;
            super::super::files::copy(&mut vfs, guest.as_bytes(), host, upload, o)
                .map_err(failure)?;
        }
        Ok(json!({"copied":true}))
    }
}
