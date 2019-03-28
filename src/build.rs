use std::env;
use std::fs::File;
use std::io::Write;
use std::process::Command;

fn main() {
    let n = env::var("OUT_DIR").unwrap() + "/build.rs";
    let mut f = File::create(&n).unwrap();

    let output = Command::new("git")
        .arg("rev-parse")
        .arg("--short=8")
        .arg("HEAD")
        .output().unwrap();
    let commit = String::from_utf8(output.stdout).unwrap();
    let commit = commit.trim_end();

    let info = format!("pub static COMMIT: &'static str = \"{}\";\n", commit);
    f.write(info.as_bytes()).unwrap();
}
