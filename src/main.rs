extern crate getopts;
extern crate serde_json;
extern crate chrono;
extern crate socket;
extern crate mpegts;
extern crate epg;

use std::{env, time, thread};
use getopts::Options;

use std::fs::File;
use serde_json::Value;
use chrono::prelude::*;
use epg::Epg;

use mpegts::psi::{EitItem, Eit};

use socket::UdpSocket;

fn usage(app: &str, opts: &Options) {
    println!("Usage: {} [OPTIONS] ADDR", app);
    println!("\nOPTIONS:");
    println!("{}", opts.usage_with_format(|opts| opts.collect::<Vec<String>>().join("\n")));
    println!("\n");
    println!("    ADDR                 Destination address");
}

fn load_config(path: &str) -> Value {
    let file = match File::open(path) {
        Ok(v) => v,
        Err(e) => {
            println!("Error: failed to open config [{}]", e.to_string());
            return Value::Null;
        },
    };

    let mut config: Value = match serde_json::from_reader(file) {
        Ok(v) => v,
        Err(e) => {
            println!("Error: failed to parse config [{}]", e.to_string());
            return Value::Null;
        },
    };

    let config = match config.get_mut("make_stream") {
        Some(v) => v,
        None => {
            println!("Error: channels not found in the config");
            return Value::Null;
        },
    };

    config.take()
}

#[derive(Debug, Default)]
struct Channel {
    eit: Eit,
    items: Vec<EitItem>,
}

fn load_channel(config: &Value, epg: &mut Epg) -> Option<Channel> {
    let xmltv_id = match config.get("xmltv_id") {
        Some(v) => v.as_str().unwrap_or(""),
        None => return None,
    };

    let epg_item = match epg.channels.get_mut(xmltv_id) {
        Some(v) => v,
        None => return None,
    };

    let current_time = Utc::now().timestamp();

    let mut channel = Channel::default();
    channel.eit.table_id = 0x50;
    channel.eit.pnr = 0; // TODO
    channel.eit.tsid = 0; // TODO
    channel.eit.onid = 0; // TODO

    for event in epg_item.events.iter_mut() {
        if event.stop > current_time {
            event.codepage = 5; // TODO
            channel.items.push(EitItem::from(&*event));
        }
    }

    Some(channel)
}

fn load_channels(config_path: &str, xmltv_path: &str) -> Option<Vec<Channel>> {
    let config = match load_config(config_path) {
        Value::Array(v) => v,
        _ => {
            println!("Error: channels has wrong format");
            return None;
        },
    };

    let mut epg = Epg::default();
    if let Err(e) = epg.load(xmltv_path) {
        println!("Error: failed to parse XMLTV [{}]", e.to_string());
        return None;
    }

    let mut out: Vec<Channel> = Vec::new();
    for item in config {
        match load_channel(&item, &mut epg) {
            Some(v) => out.push(v),
            None => {},
        };
    }

    Some(out)
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

    let mut channels = match load_channels(&c_arg, &x_arg) {
        Some(v) => v,
        None => return,
    };

    // Main Loop

    // let delay_ms = time::Duration::from_millis(250);
    // loop {
    //     // TODO: send ts packets
    //     thread::sleep(delay_ms);
    // }
}
