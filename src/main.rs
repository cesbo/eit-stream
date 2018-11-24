extern crate getopts;
extern crate epg;

use std::env;
use getopts::Options;
use epg::Epg;

fn usage(app: &str, opts: &Options) {
    println!("Usage: {} [OPTIONS] udp", app);
    println!("\nOPTIONS:");
    println!("{}", opts.usage_with_format(|opts| opts.collect::<Vec<String>>().join("\n")));
    println!("\nARGS:");
    println!("    udp                 Destination address");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let program = args[0].clone();

    let mut opts = Options::new();
    opts.optflag("h", "help", "Print this text");
    opts.optopt("c", "", "Astra configuration file", "FILE");
    opts.optopt("s", "", "XMLTV http address or file path", "ADDR");

    let matches = match opts.parse(&args[1..]) {
        Ok(v) => v,
        Err(e) => {
            println!("Error: {}", e.to_string());
            return;
        }
    };

    if matches.opt_present("h") || matches.free.is_empty() {
        usage(&program, &opts);
        return;
    }

    let opt_required = vec!["c", "s"];
    for o in opt_required {
        if ! matches.opt_defined(o) {
            println!("Error: option <-{}> required", o);
            return;
        }
    }

    let c_arg = matches.opt_str("c").unwrap();
    let s_arg = matches.opt_str("s").unwrap();
    // let udp = matches.free[0].clone();

    let mut epg = Epg::default();
    if let Err(e) = epg.load(&s_arg) {
        println!("Failed to parse XMLTV. Error:{}", e.to_string());
        return;
    }

    // TODO: read Astra config
}
