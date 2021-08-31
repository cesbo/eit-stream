use {
    std::{
        env,
        fs::File,
        io::Write,
        process::Command,
    },

    chrono::offset::Local,
};


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

    let dt = Local::now();

    let s = format!(
        "pub static BUILD_ID: &'static str = \"{}\";\n",
        commit
    );
    f.write_all(s.as_bytes()).unwrap();

    let s = format!(
        "pub static BUILD_DATE: &'static str = \"{}\";\n",
        dt.format("%Y-%m-%d")
    );
    f.write_all(s.as_bytes()).unwrap();
}
