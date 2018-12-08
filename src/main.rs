extern crate getopts;
extern crate chrono;
extern crate udp;
extern crate mpegts;
extern crate epg;

use std::{env, time, thread, cmp};
use getopts::Options;

use std::fs::File;
use std::io::{BufRead, BufReader};
use chrono::prelude::*;
use epg::Epg;

use mpegts::psi::{Psi, EitItem, Eit, EIT_PID};

use udp::UdpSocket;

include!(concat!(env!("OUT_DIR"), "/build.rs"));

#[derive(Default, Debug)]
struct Channel {
    onid: u16,
    tsid: u16,
    pnr: u16,
    lang: String,
    codepage: usize,
    id: String,

    present: Eit,
    schedule: Eit,
}

fn version() {
    println!("eit-stream v.{} commit:{}", env!("CARGO_PKG_VERSION"), COMMIT);
}

fn usage(app: &str, opts: &Options) {
    println!("Usage: {} [OPTIONS] ADDR", app);
    println!("\nOPTIONS:");
    println!("{}", opts.usage_with_format(|opts| opts.collect::<Vec<String>>().join("\n")));
    println!("\n");
    println!("    ADDR                 Destination address");
    println!("\n");
    println!("Config format:");
    println!("onid tsid pnr lang codepage id");
}

fn load_config(path: &str) -> Option<Vec<Channel>> {
    let file = match File::open(path) {
        Ok(v) => v,
        Err(e) => {
            println!("Error: failed to open config [{}]", e.to_string());
            return None;
        },
    };
    let file = BufReader::new(&file);

    let mut channels: Vec<Channel> = Vec::new();

    for line in file.lines() {
        let line = match line {
            Ok(v) => v,
            _ => continue,
        };
        if line.is_empty() {
            continue;
        }
        let line = line.trim_start();
        if line.starts_with(";") {
            continue;
        }
        let items: Vec<&str> = line.split_whitespace().collect();
        if items.len() < 6 {
            continue;
        }

        let mut channel = Channel::default();
        channel.onid = match items[0].parse() {
            Ok(v) => v,
            _ => continue,
        };
        channel.tsid = match items[1].parse() {
            Ok(v) => v,
            _ => continue,
        };
        channel.pnr = match items[2].parse() {
            Ok(v) => v,
            _ => continue,
        };
        channel.codepage = match items[4].parse() {
            Ok(v) => v,
            _ => continue,
        };
        if items[3].len() != 3 {
            continue;
        }
        channel.lang.push_str(items[3]);
        channel.id.push_str(items[5]);
        channels.push(channel);
    }

    Some(channels)
}

fn load_channel(channel: &mut Channel, epg: &mut Epg) {
    let epg_item = match epg.channels.get_mut(&channel.id) {
        Some(v) => v,
        None => {
            println!("Warning: channel \"{}\" not found in XMLTV", &channel.id);
            return;
        },
    };

    let current_time = Utc::now().timestamp();

    // Present+Following
    channel.present.table_id = 0x4E;
    channel.present.pnr = channel.pnr;
    channel.present.tsid = channel.tsid;
    channel.present.onid = channel.onid;

    // Schedule
    channel.schedule.table_id = 0x50;
    channel.schedule.pnr = channel.pnr;
    channel.schedule.tsid = channel.tsid;
    channel.schedule.onid = channel.onid;

    for event in epg_item.events.iter_mut() {
        if event.stop > current_time {
            event.codepage = channel.codepage;
            channel.schedule.items.push(EitItem::from(&*event));
            if channel.schedule.items.len() == 12 {
                break;
            }
        }
    }

    if channel.schedule.items.is_empty() {
        println!("Warning: channel \"{}\" has empty list", &channel.id);
    }
}

#[inline]
fn check_first_event(eit: &Eit, current_time: i64) -> bool {
    if let Some(event) = eit.items.first() {
        if current_time >= event.start + i64::from(event.duration) {
            return false;
        }
    }
    return true;
}

fn clear_eit(eit: &mut Eit, current_time: i64) {
    let mut count = 0;
    while ! check_first_event(eit, current_time) {
        eit.items.remove(0);
        count += 1;
    }

    if count > 0 {
        eit.version = (eit.version + 1) & 0x1F;
    }
}

fn clear_channel(channel: &mut Channel) {
    let current_time = Utc::now().timestamp();

    clear_eit(&mut channel.present, current_time);
    clear_eit(&mut channel.schedule, current_time);

    while channel.present.items.len() != 2 && channel.schedule.items.len() > 0 {
        channel.present.items.push(channel.schedule.items.remove(0));
    }

    if let Some(item) = channel.present.items.first_mut() {
        if current_time >= item.start {
            item.status = 4;
        }
    }
}

fn load_channels(config_path: &str, xmltv_path: &str) -> Option<Vec<Channel>> {
    let mut channels = match load_config(config_path) {
        Some(v) => v,
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

    for channel in channels.iter_mut() {
        load_channel(channel, &mut epg);
    }

    Some(channels)
}

fn main() {
    // Parse Options

    let args: Vec<String> = env::args().collect();
    let program = args[0].clone();

    let mut opts = Options::new();
    opts.optflag("v", "version", "Version information");
    opts.optflag("h", "help", "Print this text");
    opts.optopt("c", "", "Config file", "FILE");
    opts.optopt("x", "", "XMLTV http address or file path", "ADDR");

    let matches = match opts.parse(&args[1..]) {
        Ok(v) => v,
        Err(e) => {
            println!("Error: {}", e.to_string());
            return;
        }
    };

    if matches.opt_present("v") {
        version();
        return;
    }

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
    let udp_socket = match dst[0] {
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

    if channels.len() == 0 {
        println!("Error: empty channel list");
        return;
    }

    // Main Loop

    let mut cc = 0;
    let mut psi = Psi::default();

    let loop_delay_ms = time::Duration::from_millis(250 / (channels.len() as u64));
    let udp_delay_ms = time::Duration::from_millis(1);

    loop {
        for channel in channels.iter_mut() {
            let now = time::Instant::now();

            clear_channel(channel);

            // TODO: UdpOutput

            channel.present.assemble(&mut psi);
            let mut ts = Vec::<u8>::new();
            psi.pid = EIT_PID;
            psi.cc = cc;
            psi.demux(&mut ts);
            cc = psi.cc;
            let mut skip = 0;
            while skip < ts.len() {
                let pkt_len = cmp::min(ts.len() - skip, 1316);
                let next = skip + pkt_len;
                udp_socket.sendto(&ts[skip .. next]).unwrap();
                thread::sleep(udp_delay_ms);
                skip = next;
            }

            let now = now.elapsed();
            if loop_delay_ms > now {
                thread::sleep(loop_delay_ms - now);
            }
        }
    }
}
