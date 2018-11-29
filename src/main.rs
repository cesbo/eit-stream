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

use mpegts::ts;
use mpegts::psi::{Psi, EitItem, Eit, EIT_PID};

use socket::UdpSocket;

#[derive(Default)]
struct Channel {
    present: Eit,
    schedule: Eit,
}

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

    // Present+Following
    channel.present.table_id = 0x4E;
    channel.present.pnr = 1;
    channel.present.tsid = 1;
    channel.present.onid = 1;

    // Schedule
    channel.schedule.table_id = 0x50;
    channel.schedule.pnr = 1;
    channel.schedule.tsid = 1;
    channel.schedule.onid = 1;

    for event in epg_item.events.iter_mut() {
        if event.stop > current_time {
            event.codepage = 5; // TODO
            channel.schedule.items.push(EitItem::from(&*event));
            if channel.schedule.items.len() == 12 {
                break;
            }
        }
    }

    Some(channel)
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
    while ! check_first_event(eit, current_time) {
        eit.items.remove(0);
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

    let mut channels: Vec<Channel> = Vec::new();
    for item in config {
        match load_channel(&item, &mut epg) {
            Some(v) => channels.push(v),
            None => {},
        };
    }
    Some(channels)
}

fn main() {
    // Parse Options

    let args: Vec<String> = env::args().collect();
    let program = args[0].clone();

    let mut opts = Options::new();
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

    // Main Loop

    let mut udp_packet: Vec<u8> = Vec::new();
    udp_packet.resize(1460 / 188 * 188, 0x00);
    let mut udp_packet_skip = 0;

    let mut packet = ts::new_ts();
    ts::set_pid(&mut packet, EIT_PID);
    ts::set_cc(&mut packet, 0);

    let mut psi = Psi::default();

    let delay_ms = time::Duration::from_millis(100);
    loop {
        for channel in channels.iter_mut() {
            clear_channel(channel);

            // TODO: UdpOutput
            channel.present.assemble(&mut psi);
            while psi.demux(&mut packet) {
                let e = udp_packet_skip + 188;
                udp_packet[udp_packet_skip .. e].copy_from_slice(&packet[0 .. 188]);
                udp_packet_skip += 188;
                if udp_packet_skip == udp_packet.len() {
                    udp_socket.sendto(&udp_packet).unwrap();
                    udp_packet_skip = 0;
                    thread::sleep(delay_ms);
                }
            }

            channel.schedule.assemble(&mut psi);
            while psi.demux(&mut packet) {
                let e = udp_packet_skip + 188;
                udp_packet[udp_packet_skip .. e].copy_from_slice(&packet[0 .. 188]);
                udp_packet_skip += 188;
                if udp_packet_skip == udp_packet.len() {
                    udp_socket.sendto(&udp_packet).unwrap();
                    udp_packet_skip = 0;
                    thread::sleep(delay_ms);
                }
            }
        }

        thread::sleep(delay_ms);
    }
}
