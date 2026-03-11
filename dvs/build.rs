use std::process::Command;

fn main() {
    // Embed the git commit hash at compile time
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output();
    if let Ok(output) = output {
        if output.status.success() {
            let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!("cargo:rustc-env=DVS_GIT_COMMIT={hash}");
        }
    }
    println!("cargo:rerun-if-changed=../.git/HEAD");
}
