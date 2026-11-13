// SPDX-License-Identifier: GPL-3.0-or-later
//! User-facing CLI. Parsing, help, config and completion never start a runtime.
use super::environments::{self as env, Result};
use std::io::IsTerminal;
const HELP: &str = "Usage: trames [-e ENVIRONMENT] COMMAND [OPTIONS]\n\n  shell                    Open an independent shell (default with a terminal)\n  exec [-t] [--setenv K=V] [--cwd DIR] -- COMMAND [ARG...]\n                           Execute argv, returning its exit code\n  -- COMMAND [ARG...]      Shorthand for exec\n  env ls [--json|-q]       List environments\n  env create NAME [--distro alpine|debian | --rootfs PATH]\n  env create NAME --target android --rootfs ANDROID_PACKAGE\n  apk install FILE.apk | launch PACKAGE[/ACTIVITY] | list\n  env start|stop|restart [NAME]\n  env enable|disable NAME [--now]   Autostart at Mac user login\n  env setup NAME --init systemd    Explicitly prepare systemd\n  session ls [--json|-q]   List terminals in the selected environment\n  session attach ID        Reattach a detached terminal\n  config get|set|unset|list|edit|path  Environment settings\n  config --global ...      CLI defaults and default environment\n  gui                      Open the environment manager\n  completion zsh|bash|fish  Print shell completion\n\nOptions: -e, --environment NAME; -h, --help; -V, --version\n\nExamples:\n  trames env create work --distro debian\n  trames -e work shell\n  printf hello | trames -e work exec -- cat\n  trames -e work exec -- sh -c 'echo error >&2; exit 7'\n  trames -e work config set memory 4GiB\n\nShell/exec start existing environments on demand. They never download or create\nan image. exec uses separate stdin/stdout/stderr pipes unless -t is requested.\nUse an explicit sh -c for shell syntax. Exiting a terminal leaves the environment\nrunning. env start always starts in the background and returns.\n";
pub fn dispatch(args: Vec<String>) -> Result<u8> {
    let (selected, args) = selection(args)?;
    let op = args.first().map(String::as_str).unwrap_or("");
    if op == "help"
        || args
            .iter()
            .take_while(|a| a.as_str() != "--")
            .any(|a| matches!(a.as_str(), "--help" | "-h"))
    {
        print!("{HELP}");
        return Ok(0);
    }
    if matches!(op, "--version" | "-V" | "version") {
        let runtime: serde_json::Value =
            serde_json::from_str(include_str!("../../../runtime-version.json"))?;
        println!(
            "{} {} ({}) runtime {}",
            super::product::name(),
            env!("CARGO_PKG_VERSION"),
            super::product::CHANNEL,
            runtime["revision"]
                .as_str()
                .unwrap_or_else(|| runtime["commit"].as_str().unwrap_or("unknown"))
        );
        return Ok(0);
    }
    if matches!(op, "gui" | "--manager") {
        unsafe extern "C" {
            fn trames_manager_run();
        }
        unsafe {
            trames_manager_run();
        }
        return Ok(0);
    }
    if op == "completion" {
        return completion(args.get(1).map(String::as_str).unwrap_or(""));
    }
    if op.is_empty() && !std::io::stdin().is_terminal() {
        print!("{HELP}");
        return Ok(0);
    }
    if op == "env" {
        return environments(&args[1..], selected.as_deref());
    }
    if op == "config" {
        return env::settings(selected.as_deref(), &args[1..]);
    }
    if op == "debug" && args.get(1).is_some_and(|s| s == "run") {
        return super::execute(args[2..].to_vec()).map_err(Into::into);
    }
    if op == "debug" && args.get(1).is_some_and(|s| s == "env-run") {
        let name = args
            .get(2)
            .ok_or("usage: debug env-run NAME [HOST_OPTIONS] [-- COMMAND ARGS...]")?;
        return env::start(&env::root()?, name, &args[3..]);
    }
    // Explicit low-level runner options remain available to existing backend
    // validation scripts. No environment management command uses this route.
    if [
        "--rootfs",
        "--storage",
        "--filesystem",
        "--cpus",
        "--timeout",
        "--pty",
        "--no-pty",
    ]
    .contains(&op)
    {
        return super::execute(args).map_err(Into::into);
    }
    if !matches!(op, "" | "shell" | "exec" | "--" | "session" | "apk") {
        return Err(format!("unknown command '{op}'; use trames --help").into());
    }
    let r = env::root()?;
    let n = selected.unwrap_or(env::default_environment(&r)?);
    let config = env::load(&env::directory(&r, &n)?).map_err(|e| format!("environment '{n}' is unavailable: {e}. Create it with `trames env create {n}`, or select an existing one with -e"))?;
    match op {
        "" | "shell" => {
            if args.len() > 1 {
                return Err("usage: trames [-e NAME] shell".into());
            }
            super::background::start(&r, &n)?;
            if config.target == trames_distro::boot::Target::Android {
                if op.is_empty() {
                    return Ok(0);
                }
                return super::exec::run(
                    &env::console_socket(&r, &n)?,
                    super::exec::Request {
                        argv: vec!["/system/bin/sh".into(), "-i".into()],
                        environment: vec!["TERM=xterm-256color".into()],
                        cwd: Some("/".into()),
                        tty: true,
                        size: super::session::window_size(),
                    },
                )
                .map_err(Into::into);
            }
            super::console::attach(&env::console_socket(&r, &n)?, 0)?;
            Ok(0)
        }
        "exec" | "--" => {
            let request = parse_exec(&args[1..], op == "--")?;
            super::background::start(&r, &n)?;
            Ok(super::exec::run(&env::console_socket(&r, &n)?, request)?)
        }
        "apk" => android_apps(&r, &n, &config, &args[1..]),
        "session" => {
            if config.target == trames_distro::boot::Target::Android {
                return Err("open an Android terminal with shell or exec -t".into());
            }
            let socket = env::console_socket(&r, &n)?;
            match args.get(1).map(String::as_str).unwrap_or("ls") {
                "ls" | "list" => {
                    super::console::list(&socket, &args[2..])?;
                    Ok(0)
                }
                "attach" if args.len() == 3 => {
                    super::console::attach(&socket, args[2].parse()?)?;
                    Ok(0)
                }
                _ => {
                    Err("usage: trames [-e NAME] session ls [--json|-q] | session attach ID".into())
                }
            }
        }
        _ => Err(format!("unknown command '{op}'; use trames --help").into()),
    }
}
fn selection(args: Vec<String>) -> Result<(Option<String>, Vec<String>)> {
    let mut it = args.into_iter();
    let mut selected = None;
    let mut rest = vec![];
    while let Some(a) = it.next() {
        if a == "--" {
            rest.push(a);
            rest.extend(it);
            break;
        }
        if a == "-e" || a == "--environment" {
            selected = Some(it.next().ok_or("--environment requires a name")?);
        } else if let Some(value) = a.strip_prefix("--environment=") {
            selected = Some(value.to_owned());
        } else {
            rest.push(a);
            rest.extend(it);
            break;
        }
    }
    Ok((selected, rest))
}
fn parse_exec(args: &[String], literal: bool) -> Result<super::exec::Request> {
    let mut r = super::exec::Request::default();
    let mut i = 0;
    if !literal {
        while let Some(a) = args.get(i) {
            match a.as_str() {
                "--" => {
                    i += 1;
                    break;
                }
                "-t" | "--tty" => r.tty = true,
                "--setenv" | "--cwd" | "-w" => {
                    i += 1;
                    let value = args.get(i).ok_or("missing exec option value")?.clone();
                    if a == "--setenv" {
                        r.environment.push(value);
                    } else {
                        r.cwd = Some(value);
                    }
                }
                a if a.starts_with("--setenv=") => r.environment.push(a[9..].into()),
                a if a.starts_with('-') => {
                    return Err(
                        format!("unknown exec option '{a}'; put -- before the command").into(),
                    )
                }
                _ => break,
            }
            i += 1;
        }
    }
    r.argv = args[i..].to_vec();
    if r.argv.is_empty() {
        return Err("usage: trames [-e NAME] exec [-t] [--setenv KEY=VALUE] [--cwd DIR] -- COMMAND [ARG...]".into());
    }
    for entry in &r.environment {
        if !entry.split_once('=').is_some_and(|(k, _)| !k.is_empty()) {
            return Err("--setenv requires KEY=VALUE".into());
        }
    }
    if r.argv
        .iter()
        .chain(&r.environment)
        .any(|a| a.contains('\0'))
    {
        return Err("arguments cannot contain NUL".into());
    }
    r.size = r.tty.then(super::session::window_size).flatten();
    Ok(r)
}
fn android_apps(
    root: &std::path::Path,
    name: &str,
    config: &env::Config,
    args: &[String],
) -> Result<u8> {
    if config.target != trames_distro::boot::Target::Android {
        return Err("apk commands require an Android environment".into());
    }
    let mut request = super::exec::Request {
        cwd: Some("/".into()),
        ..Default::default()
    };
    let input = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["install", path] => {
            let file = std::fs::File::open(path)?;
            let metadata = file.metadata()?;
            if !metadata.is_file() || metadata.len() == 0 {
                return Err("select a regular APK file".into());
            }
            request.argv = vec![
                "/system/bin/cmd".into(),
                "-w".into(),
                "package".into(),
                "install".into(),
                "-r".into(),
                "-S".into(),
                metadata.len().to_string(),
            ];
            Some(file)
        }
        ["launch", package] => {
            use std::io::Write;
            super::background::start(root, name)?;
            let console = env::console_socket(root, name)?;
            let component = if package.contains('/') {
                (*package).to_owned()
            } else {
                request.argv = vec![
                    "/system/bin/cmd".into(),
                    "-w".into(),
                    "package".into(),
                    "resolve-activity".into(),
                    "--brief".into(),
                    (*package).into(),
                ];
                let (code, output, error) = super::exec::capture(&console, request)?;
                std::io::stderr().write_all(&error)?;
                if code != 0 {
                    return Ok(code);
                }
                String::from_utf8(output)?.lines().rev().find(|line| line.starts_with(&format!("{package}/")))
                    .ok_or("Android has no available launch activity for this package; wait for user unlock or supply PACKAGE/ACTIVITY")?.to_owned()
            };
            let request = super::exec::Request {
                argv: vec![
                    "/system/bin/am".into(),
                    "start".into(),
                    "-W".into(),
                    "-n".into(),
                    component,
                ],
                cwd: Some("/".into()),
                ..Default::default()
            };
            let (code, output, error) = super::exec::capture(&console, request)?;
            std::io::stdout().write_all(&output)?;
            std::io::stderr().write_all(&error)?;
            // Android am can report an error while returning zero. Preserve its
            // real text and make launch failure visible to shell scripts.
            return Ok(
                if code == 0
                    && [output, error]
                        .iter()
                        .any(|b| String::from_utf8_lossy(b).contains("Error:"))
                {
                    1
                } else {
                    code
                },
            );
        }
        ["list"] => {
            request.argv = vec![
                "/system/bin/cmd".into(),
                "-w".into(),
                "package".into(),
                "list".into(),
                "packages".into(),
                "-3".into(),
            ];
            None
        }
        _ => {
            return Err(
                "usage: trames -e NAME apk install FILE.apk | launch PACKAGE[/ACTIVITY] | list"
                    .into(),
            )
        }
    };
    super::background::start(root, name)?;
    let console = env::console_socket(root, name)?;
    match input {
        Some(file) => super::exec::run_input(&console, request, file).map_err(Into::into),
        None => super::exec::run(&console, request).map_err(Into::into),
    }
}

fn environments(args: &[String], selected: Option<&str>) -> Result<u8> {
    let op = args.first().map(String::as_str).unwrap_or("ls");
    if matches!(op, "--help" | "-h" | "help") {
        print!("{HELP}");
        return Ok(0);
    }
    if matches!(op, "ls" | "list") {
        return env::listing(&args[1..]);
    }
    if matches!(op, "start" | "stop" | "restart" | "enable" | "disable") {
        let r = env::root()?;
        let mut it = args[1..].iter();
        let mut name = selected.map(str::to_owned);
        let mut now = false;
        for a in &mut it {
            match a.as_str() {
                "--now" if matches!(op, "enable" | "disable") => now = true,
                "--detach" | "-d" if op == "start" => (),
                a if !a.starts_with('-') && name.is_none() => name = Some(a.into()),
                _ => return Err(format!("unexpected argument '{a}' for env {op}").into()),
            }
        }
        let n = name.unwrap_or(env::default_environment(&r)?);
        env::load(&env::directory(&r, &n)?)?;
        match op {
            "start" => super::background::start(&r, &n)?,
            "stop" => env::stop(&r, &n)?,
            "restart" => {
                env::stop(&r, &n)?;
                super::background::start(&r, &n)?;
            }
            "enable" | "disable" => {
                super::background::autostart(&r, &n, if op == "enable" { "on" } else { "off" })?;
                if now {
                    if op == "enable" {
                        super::background::start(&r, &n)?;
                    } else {
                        env::stop(&r, &n)?;
                    }
                }
            }
            _ => unreachable!(),
        }
        return Ok(0);
    }
    if op == "setup" {
        return env::setup(&args[1..]);
    }
    env::dispatch(args)
}
fn completion(shell: &str) -> Result<u8> {
    match shell {
        "bash" => print!("{}", include_str!("../../../completions/trames.bash")),
        "zsh" => print!("{}", include_str!("../../../completions/_trames")),
        "fish" => print!("{}", include_str!("../../../completions/trames.fish")),
        _ => return Err("usage: trames completion bash|zsh|fish".into()),
    }
    Ok(0)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn strings(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| (*s).into()).collect()
    }
    #[test]
    fn argv_is_literal_and_guest_environment_is_separate() {
        let (e, args) = selection(strings(&[
            "-e",
            "work",
            "exec",
            "--setenv",
            "X=a b",
            "--",
            "printf",
            "%s",
            "-e",
            "$(touch x)",
        ]))
        .unwrap();
        assert_eq!(e.as_deref(), Some("work"));
        let r = parse_exec(&args[1..], false).unwrap();
        assert_eq!(r.environment, ["X=a b"]);
        assert_eq!(r.argv, ["printf", "%s", "-e", "$(touch x)"]);
        assert!(!r.tty);
        assert!(parse_exec(&strings(&["--setenv", "oops", "--", "true"]), false).is_err());
    }
}
