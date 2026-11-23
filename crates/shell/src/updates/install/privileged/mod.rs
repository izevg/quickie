use std::path::Path;
use std::process::Command;

use super::{InstallError, run_tool};

const AUTHORIZATION: &str = include_str!("authorize.applescript");
const REPLACEMENT: &str = include_str!("replace.js");
const PROTECTED_BUNDLE: &str = "/Applications/Quickie.app";

pub(super) struct BundleProof {
    pub(super) original: String,
    pub(super) candidate: String,
    pub(super) owner: u32,
    pub(super) group: u32,
}

pub(super) fn replace(
    installed: &Path,
    staged: &Path,
    version: &str,
    proof: &BundleProof,
) -> Result<(), InstallError> {
    if installed != Path::new(PROTECTED_BUNDLE) {
        return Err(InstallError::Failed(
            "A protected update requires Quickie.app in the Applications directory.".into(),
        ));
    }
    authorize(AUTHORIZATION, REPLACEMENT, staged, version, proof)
}

fn authorize(
    authorization: &str,
    replacement: &str,
    staged: &Path,
    version: &str,
    proof: &BundleProof,
) -> Result<(), InstallError> {
    let output = run_tool(
        Command::new("/usr/bin/osascript")
            .args(["-e", authorization, replacement])
            .arg(staged)
            .args([&proof.original, &proof.candidate, version])
            .args([proof.owner.to_string(), proof.group.to_string()]),
        "macOS could not replace the protected Quickie application",
    )
    .map_err(InstallError::Failed)?;
    match String::from_utf8_lossy(&output.stdout).trim() {
        "QUICKIE_UPDATE_INSTALLED" => Ok(()),
        "QUICKIE_UPDATE_CANCELLED" => Err(InstallError::Cancelled),
        _ => Err(InstallError::Failed(
            "macOS returned an unexpected result for the Quickie replacement.".into(),
        )),
    }
}

#[cfg(test)]
pub(super) fn replace_disposable(
    installed: &Path,
    staged: &Path,
    test_directory: &Path,
    stage_owner: u32,
    version: &str,
    proof: &BundleProof,
) -> Result<(), String> {
    let test_script = format!(
        "{REPLACEMENT}\nfunction run(args) {{ return replace(args[6], args[7], args.slice(0, 6), args[8]); }}"
    );
    run_tool(
        Command::new("/usr/bin/osascript")
            .args(["-l", "JavaScript", "-e", &test_script])
            .arg(staged)
            .args([&proof.original, &proof.candidate, version])
            .args([proof.owner.to_string(), proof.group.to_string()])
            .arg(installed)
            .arg(test_directory.join("quickie-test.XXXXXX"))
            .arg(stage_owner.to_string()),
        "Could not run the protected updater against a disposable path",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{AUTHORIZATION, BundleProof, InstallError, authorize};
    use std::path::Path;

    #[test]
    fn authorization_bridge_quotes_arguments_without_shell_expansion() {
        let without_prompt = AUTHORIZATION.replace(" with administrator privileges", "");
        let script = r#"
function run(args) {
    if (args.join('|') !== "/tmp/Quickie's $(marker); path|old|new|0.3.0|501|20") {
        throw Error("Arguments were modified by the shell");
    }
    return "QUICKIE_UPDATE_INSTALLED";
}
"#;
        let result = authorize(
            &without_prompt,
            script,
            Path::new("/tmp/Quickie's $(marker); path"),
            "0.3.0",
            &BundleProof {
                original: "old".into(),
                candidate: "new".into(),
                owner: 501,
                group: 20,
            },
        );

        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn authorization_bridge_distinguishes_cancelled_requests() {
        let cancelled = r#"on run argv
return "QUICKIE_UPDATE_CANCELLED"
end run"#;
        let result = authorize(
            cancelled,
            "",
            Path::new("/tmp/test"),
            "0.3.0",
            &BundleProof {
                original: "old".into(),
                candidate: "new".into(),
                owner: 501,
                group: 20,
            },
        );

        assert!(matches!(result, Err(InstallError::Cancelled)));
    }
}
