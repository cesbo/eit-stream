use std::{env, time, thread, cmp};

mod error;
use crate::error::{Error, Result};

mod config;
use crate::config::parse_config;

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
pub struct Instance {
    pub epg_list: Vec<Epg>,
    pub output_list: Vec<UdpSocket>,

    pub multiplex_list: Vec<Multiplex>,
    pub service_list: Vec<Service>,

    pub onid: u16,
    pub codepage: u8,
}

impl Instance {
    pub fn open_xmltv(&mut self, path: &str) -> Result<()> {
        let mut epg = Epg::default();
        epg.load(path)?;
        self.epg_list.push(epg);
        Ok(())
    }

    pub fn open_output(&mut self, addr: &str) -> Result<()> {
        let dst = addr.splitn(2, "://").collect::<Vec<&str>>();
        if dst[0] != "udp" {
            return Err(Error::from(format!("unknown output type [{}]", dst[0])));
        }
        let output = UdpSocket::open(dst[1])?;
        self.output_list.push(output);
        Ok(())
    }
}

#[derive(Default, Debug)]
pub struct Multiplex {
    pub epg_item_id: usize,
    pub output_item_id: usize,

    pub onid: u16,
    pub tsid: u16,
    pub codepage: u8,
}

#[derive(Default, Debug)]
pub struct Service {
    pub epg_item_id: usize,
    pub output_item_id: usize,

    pub onid: u16,
    pub tsid: u16,
    pub codepage: u8,

    pub pnr: u16,
    pub xmltv_id: String,

    present: Eit,
    schedule: Eit,

    ts: Vec<u8>,
}

impl Service {
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

        while ! Service::check_first_event(eit, current_time) {
            eit.items.remove(0);
            version_up = true;
        }

        if version_up {
            eit.version = (eit.version + 1) & 0x1F;
        }
    }

    pub fn clear(&mut self) {
        let current_time = Utc::now().timestamp();

        Service::clear_eit(&mut self.present, current_time);
        Service::clear_eit(&mut self.schedule, current_time);

        if self.present.items.len() != 2 {
            while self.present.items.len() != 2 && self.schedule.items.len() > 0 {
                self.present.items.push(self.schedule.items.remove(0));
            }

            if let Some(item) = self.present.items.first_mut() {
                if current_time >= item.start {
                    item.status = 4;
                }
            }
        }
    }
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

            service.clear();

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
