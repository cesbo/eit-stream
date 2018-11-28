extern crate getopts;
extern crate serde_json;
extern crate chrono;
extern crate socket;
extern crate epg;

use std::{env, time, thread};
use getopts::Options;

use std::fs::File;
use serde_json::Value;
use chrono::prelude::*;
use epg::{Epg, EpgChannel};

use socket::UdpSocket;

fn usage(app: &str, opts: &Options) {
    println!("Usage: {} [OPTIONS] ADDR", app);
    println!("\nOPTIONS:");
    println!("{}", opts.usage_with_format(|opts| opts.collect::<Vec<String>>().join("\n")));
    println!("\n");
    println!("    ADDR                 Destination address");
}

fn filter_channels(channels: &mut Vec<EpgChannel>) {
    let current_time = Utc::now().timestamp();

    for channel in channels {
        loop {
            {
                match channel.events.first_mut() {
                    Some(event) => if event.stop > current_time { break },
                    None => break,
                };
            }

            channel.events.remove(0);
        }
    }
}

fn main() {
    // Parse Options

    let args: Vec<String> = env::args().collect();
    let program = args[0].clone();

    let mut opts = Options::new();
    opts.optflag("h", "help", "Print this text");
    opts.optopt("c", "", "Astra config file", "FILE");
    opts.optopt("x", "", "XMLTV http address or file path", "ADDR");

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

    let opt_required = vec!["c", "x"];
    for o in opt_required {
        if ! matches.opt_defined(o) {
            println!("Error: option <-{}> required", o);
            return;
        }
    }

    let c_arg = matches.opt_str("c").unwrap();
    let x_arg = matches.opt_str("x").unwrap();
    let addr = matches.free[0].clone();

    // Open Socket

    let dst = addr.splitn(2, "://").collect::<Vec<&str>>();
    let sock = match dst[0] {
        "udp" => {
            match UdpSocket::open(dst[1]) {
                Ok(v) => v,
                Err(e) => {
                    println!("Error: failed to open UDP socket [{}]", e.to_string());
                    return;
                },
            }
        },
        _ => {
            println!("Error: unknown destination type [{}]", addr);
            return;
        },
    };

    // Open Astra Config

    let file = match File::open(c_arg) {
        Ok(v) => v,
        Err(e) => {
            println!("Error: failed to open config [{}]", e.to_string());
            return;
        },
    };

    let config: Value = match serde_json::from_reader(file) {
        Ok(v) => v,
        Err(e) => {
            println!("Error: failed to parse config [{}]", e.to_string());
            return;
        },
    };

    let config = match config.get("make_stream") {
        Some(v) => v,
        None => {
            println!("Error: channels not found in the config");
            return;
        },
    };

    let config = match config.as_array() {
        Some(v) => v,
        None => {
            println!("Error: channels has wrong format");
            return;
        },
    };

    // Open XMLTV

    let mut channels: Vec<EpgChannel> = Vec::new();
    {
        let mut epg = Epg::default();
        if let Err(e) = epg.load(&x_arg) {
            println!("Error: failed to parse XMLTV [{}]", e.to_string());
            return;
        }

        for item in config {
            if let Some(xmltv_id) = item.get("xmltv_id") {
                let xmltv_id = xmltv_id.as_str().unwrap_or("");
                match epg.channels.remove(xmltv_id) {
                    Some(v) => channels.push(v),
                    None => println!("Warning: channel {} not found", xmltv_id),
                };
            }
        }
    }

    filter_channels(&mut channels);

    // TODO: convert EpgChannel into Psi

    // Main Loop

    let delay_ms = time::Duration::from_millis(250);
    loop {
        // TODO: send ts packets
        thread::sleep(delay_ms);
    }
}
