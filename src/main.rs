mod error;
use crate::error::{Error, Result};

use std::{io, env, time, thread, cmp};
use ini::{IniReader, IniItem};

use std::fs::File;
use std::io::{BufRead, BufReader};
use chrono::Utc;
use epg::Epg;

use mpegts::psi::{EIT_PID, Eit, EitItem, PsiDemux};
use udp::UdpSocket;

include!(concat!(env!("OUT_DIR"), "/build.rs"));

fn version() {
    println!("eit-stream v.{} commit:{}", env!("CARGO_PKG_VERSION"), COMMIT);
}

fn usage(program: &str) {
    println!(r#"Usage: {} CONFIG

OPTIONS:
    -v, --version       Version information
    -h, --help          Print this text

CONFIG:
    Path to configuration file
"#, program);
}

#[derive(Default, Debug)]
struct Instance {
    epg_list: Vec<Epg>,
    output_list: Vec<UdpSocket>,

    multiplex_list: Vec<Multiplex>,
    service_list: Vec<Service>,

    onid: u16,
    codepage: u8,
}

#[derive(Default, Debug)]
struct Multiplex {
    epg_item_id: usize,
    output_item_id: usize,

    onid: u16,
    tsid: u16,
    codepage: u8,
}

#[derive(Default, Debug)]
struct Service {
    epg_item_id: usize,
    output_item_id: usize,

    onid: u16,
    tsid: u16,
    codepage: u8,

    pnr: u16,
    xmltv_id: String,

    present: Eit,
    schedule: Eit,

    ts: Vec<u8>,
}

fn parse_multiplex<R: io::Read>(instance: &mut Instance, config: &mut IniReader<R>) -> Result<()> {
    let mut multiplex = Multiplex::default();
    multiplex.onid = instance.onid;
    multiplex.codepage = instance.codepage;

    while let Some(e) = config.next() {
        match e? {
            IniItem::EndSection => break,
            IniItem::Property(key, value) => {
                match key.as_ref() {
                    "onid" => multiplex.onid = value.parse()?,
                    "tsid" => multiplex.tsid = value.parse()?,
                    "codepage" => multiplex.codepage = value.parse()?,
                    // TODO: custom output and xmltv
                    _ => {},
                }
            },
            _ => {},
        };
    }

    instance.multiplex_list.push(multiplex);
    Ok(())
}

//

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
    let mut version_up = false;

    while ! check_first_event(eit, current_time) {
        eit.items.remove(0);
        version_up = true;
    }

    if version_up {
        eit.version = (eit.version + 1) & 0x1F;
    }
}

fn clear_service(service: &mut Service) {
    let current_time = Utc::now().timestamp();

    clear_eit(&mut service.present, current_time);
    clear_eit(&mut service.schedule, current_time);

    if service.present.items.len() != 2 {
        while service.present.items.len() != 2 && service.schedule.items.len() > 0 {
            service.present.items.push(service.schedule.items.remove(0));
        }

        if let Some(item) = service.present.items.first_mut() {
            if current_time >= item.start {
                item.status = 4;
            }
        }
    }
}

//

fn parse_service<R: io::Read>(instance: &mut Instance, config: &mut IniReader<R>) -> Result<()> {
    let multiplex = match instance.multiplex_list.last() {
        Some(v) => v,
        None => return Err(Error::from("multiplex section not found")),
    };

    let mut service = Service::default();
    service.epg_item_id = multiplex.epg_item_id;
    service.output_item_id = multiplex.output_item_id;
    service.onid = multiplex.onid;
    service.tsid = multiplex.tsid;
    service.codepage = multiplex.codepage;

    while let Some(e) = config.next() {
        match e? {
            IniItem::EndSection => break,
            IniItem::Property(key, value) => {
                match key.as_ref() {
                    "pnr" => service.pnr = value.parse()?,
                    "codepage" => service.codepage = value.parse()?,
                    "xmltv-id" => service.xmltv_id.push_str(&value),
                    _ => {},
                }
            },
            _ => {},
        };
    }

    instance.service_list.push(service);
    Ok(())
}

fn open_xmltv(instance: &mut Instance, path: &str) -> Result<()> {
    let mut epg = Epg::default();
    epg.load(path)?;
    instance.epg_list.push(epg);
    Ok(())
}

fn open_output(instance: &mut Instance, addr: &str) -> Result<()> {
    let dst = addr.splitn(2, "://").collect::<Vec<&str>>();
    if dst[0] != "udp" {
        return Err(Error::from(format!("unknown output type [{}]", dst[0])));
    }
    let output = UdpSocket::open(dst[1])?;
    instance.output_list.push(output);
    Ok(())
}

fn parse_config(instance: &mut Instance, path: &str) -> Result<()> {
    let config = File::open(path)?;
    let mut config = IniReader::new(BufReader::new(config));

    while let Some(e) = config.next() {
        match e? {
            IniItem::StartSection(name) => match name.as_ref() {
                "multiplex" => parse_multiplex(instance, &mut config)?,
                "service" => parse_service(instance, &mut config)?,
                _ => {},
            },
            IniItem::Property(key, value) => match key.as_ref() {
                "xmltv" => open_xmltv(instance, &value)?,
                "output" => open_output(instance, &value)?,
                "onid" => instance.onid = value.parse()?,
                "codepage" => instance.codepage = value.parse()?,
                _ => {},
            },
            _ => {},
        };
    }

    Ok(())
}

fn wrap() -> Result<()> {
    // Parse Options
    let mut args = env::args();
    let program = args.next().unwrap();
    let arg = match args.next() {
        Some(v) => match v.as_ref() {
            "-v" | "--version" => { version(); return Ok(()); },
            "-h" | "--help" => { usage(&program); return Ok(()); },
            _ => v,
        },
        None => {
            usage(&program);
            return Err(Error::from("Path to configuration file requried"));
        },
    };

    let mut instance = Instance::default();

    // Prase config
    parse_config(&mut instance, &arg)?;

    if instance.epg_list.is_empty() {
        return Err(Error::from("xmltv not defined"));
    }

    if instance.output_list.is_empty() {
        return Err(Error::from("output not defined"));
    }

    // Prepare EIT from EPG

    let current_time = Utc::now().timestamp();

    for service in &mut instance.service_list {
        let epg = instance.epg_list.get_mut(service.epg_item_id).unwrap();
        let epg_item = match epg.channels.get_mut(&service.xmltv_id) {
            Some(v) => v,
            None => {
                println!("Warning: service \"{}\" not found in XMLTV", &service.xmltv_id);
                continue;
            },
        };

        // Present+Following
        service.present.table_id = 0x4E;
        service.present.pnr = service.pnr;
        service.present.tsid = service.tsid;
        service.present.onid = service.onid;

        // Schedule
        service.schedule.table_id = 0x50;
        service.schedule.pnr = service.pnr;
        service.schedule.tsid = service.tsid;
        service.schedule.onid = service.onid;

        for event in &mut epg_item.events {
            if event.stop > current_time {
                event.codepage = service.codepage;
                service.schedule.items.push(EitItem::from(&*event));
            }
        }

        if service.schedule.items.is_empty() {
            println!("Warning: service \"{}\" has empty list", &service.xmltv_id);
        }
    }

    // Main loop

    let mut cc = 0;
    let mut ts = Vec::<u8>::new();

    let loop_delay_ms = time::Duration::from_millis(500 / (instance.service_list.len() as u64));
    let udp_delay_ms = time::Duration::from_millis(1);

    loop {
        for service in instance.service_list.iter_mut() {
            let now = time::Instant::now();

            clear_service(service);

            ts.clear();
            service.present.demux(EIT_PID, &mut cc, &mut ts);
            service.schedule.demux(EIT_PID, &mut cc, &mut ts);

            let output = instance.output_list.get(service.output_item_id).unwrap();

            let mut skip = 0;
            while skip < ts.len() {
                let pkt_len = cmp::min(ts.len() - skip, 1316);
                let next = skip + pkt_len;
                output.sendto(&ts[skip .. next]).unwrap();
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

fn main() {
    if let Err(e) = wrap() {
        println!("Error: {}", e.to_string());
    }
}
