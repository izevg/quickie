//! The package smoke test checks the real, sandboxed `WebKit` subprocesses.

use std::ffi::{CStr, OsStr};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

pub fn verify(title: &CStr, prefix: &Path) -> Result<(), String> {
    if title != c"Quickie smoke test" {
        return Err(format!("unexpected smoke page title: {title:?}"));
    }
    verify_library_paths(&prefix.join("lib"))?;
    let helpers = prefix.join("libexec/webkitgtk-6.0");
    let host = fs::read_to_string("/proc/self/status").map_err(|error| error.to_string())?;
    let host_filters =
        field(&host, "Seccomp_filters:").ok_or("the host has no Seccomp filter count")?;
    let processes = descendants(std::process::id());
    for helper in ["WebKitWebProcess", "WebKitNetworkProcess"] {
        let executable = helpers.join(helper);
        let found = processes.iter().any(|pid| {
            let process = format!("/proc/{pid}");
            let Ok(arguments) = fs::read(format!("{process}/cmdline")) else {
                return false;
            };
            if arguments.split(|&byte| byte == 0).next() != Some(executable.as_os_str().as_bytes())
            {
                return false;
            }
            fs::read_to_string(format!("{process}/status"))
                .is_ok_and(|status| sandboxed(&status, host_filters))
        });
        if !found {
            return Err(format!(
                "no live sandboxed {helper} from {}",
                executable.display()
            ));
        }
    }
    Ok(())
}

fn verify_library_paths(libraries: &Path) -> Result<(), String> {
    let maps = fs::read_to_string("/proc/self/maps").map_err(|error| error.to_string())?;
    for library in ["libwebkitgtk-6.0.so.", "libjavascriptcoregtk-6.0.so."] {
        let found = maps.lines().any(|line| {
            let Some(path_start) = line.find('/') else {
                return false;
            };
            let path = Path::new(&line[path_start..]);
            path.parent() == Some(libraries)
                && path
                    .file_name()
                    .and_then(OsStr::to_str)
                    .is_some_and(|name| name.starts_with(library))
        });
        if !found {
            return Err(format!(
                "the host did not load {library} from {}",
                libraries.display()
            ));
        }
    }
    Ok(())
}

fn descendants(root: u32) -> Vec<u32> {
    let mut processes = vec![root];
    let mut index = 0;
    while let Some(&pid) = processes.get(index) {
        index += 1;
        let Ok(tasks) = fs::read_dir(format!("/proc/{pid}/task")) else {
            continue;
        };
        for task in tasks.filter_map(Result::ok) {
            if let Ok(children) = fs::read_to_string(task.path().join(OsStr::new("children"))) {
                processes.extend(
                    children
                        .split_ascii_whitespace()
                        .filter_map(|child| child.parse::<u32>().ok()),
                );
            }
        }
    }
    processes
}

fn field(status: &str, key: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|line| line.strip_prefix(key))?
        .trim()
        .parse()
        .ok()
}

fn sandboxed(status: &str, host_filters: u32) -> bool {
    let nested_pid = status
        .lines()
        .find_map(|line| line.strip_prefix("NSpid:"))
        .is_some_and(|pids| pids.split_ascii_whitespace().nth(1).is_some());
    nested_pid
        && field(status, "Seccomp:") == Some(2)
        && field(status, "Seccomp_filters:").is_some_and(|filters| filters > host_filters)
}

#[cfg(test)]
mod tests {
    use super::sandboxed;

    #[test]
    fn requires_a_namespace_and_an_additional_seccomp_filter() {
        assert!(sandboxed(
            "NSpid: 714 2\nSeccomp: 2\nSeccomp_filters: 2\n",
            1
        ));
        assert!(!sandboxed(
            "NSpid: 714\nSeccomp: 2\nSeccomp_filters: 2\n",
            1
        ));
        assert!(!sandboxed(
            "NSpid: 714 2\nSeccomp: 2\nSeccomp_filters: 1\n",
            1
        ));
        assert!(!sandboxed(
            "NSpid: 714 2\nSeccomp: 0\nSeccomp_filters: 0\n",
            0
        ));
    }
}
