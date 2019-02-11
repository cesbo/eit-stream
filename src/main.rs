use std::{env, time, thread, cmp};

use chrono;
use epg::Epg;
use mpegts::ts;
use mpegts::psi::{EIT_PID, Eit, EitItem, PsiDemux};
use udp::UdpSocket;

mod error;
use error::{Error, Result};

use config::Config;


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


#[derive(Debug)]
pub enum Output {
    None,
    Udp(UdpSocket),
}


impl Default for Output {
    fn default() -> Self {
        Output::None
    }
}


impl Output {
    pub fn open(addr: &str) -> Result<Self> {
        let dst = addr.splitn(2, "://").collect::<Vec<&str>>();
        match dst[0] {
            "udp" => {
                let s = UdpSocket::open(dst[1])?;
                Ok(Output::Udp(s))
            },
            _ => {
                Err(Error::from(format!("unknown output type [{}]", dst[0])))
            }
        }
    }

    pub fn send(&self, data: &[u8]) -> Result<()> {
        match self {
            Output::Udp(ref udp) => {
                udp.sendto(data)?;
            },
            Output::None => {},
        };
        Ok(())
    }

    pub fn is_open(&self) -> bool {
        match self {
            Output::None => false,
            _ => true,
        }
    }
}


#[derive(Default, Debug)]
pub struct Instance {
    pub epg_list: Vec<Epg>,
    pub output: Output,

    pub multiplex: Multiplex,
    pub service_list: Vec<Service>,

    pub onid: u16,
    pub codepage: u8,
    pub eit_days: usize,
    pub eit_rate: usize,
}


impl Instance {
    pub fn open_xmltv(&mut self, path: &str) -> Result<()> {
        let mut epg = Epg::default();
        epg.load(path)?;
        self.epg_list.push(epg);
        Ok(())
    }

    pub fn open_output(&mut self, addr: &str) -> Result<()> {
        self.output = Output::open(addr)?;
        Ok(())
    }
}


#[derive(Default, Debug)]
pub struct Multiplex {
    pub epg_item_id: usize,

    pub onid: u16,
    pub tsid: u16,
    pub codepage: u8,
}


#[derive(Default, Debug)]
pub struct Service {
    pub epg_item_id: usize,

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
    fn check_first_event(eit: &Eit, current_time: u64) -> bool {
        if let Some(event) = eit.items.first() {
            if current_time >= event.start + u64::from(event.duration) {
                return false;
            }
        }
        return true;
    }

    fn clear_eit(eit: &mut Eit, current_time: u64) {
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
        let current_time = chrono::Utc::now().timestamp() as u64;

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

    // Parse config
    let config = Config::open(&arg)?;
    instance.onid = config.get("onid", 1)?;
    instance.codepage = config.get("codepage", 0)?;
    instance.eit_days = config.get("eit-days", 3)?;
    instance.eit_rate = config.get("eit-rate", 3000)?;

    match config.get_str("xmltv") {
        Some(v) => instance.open_xmltv(v)?,
        None => return Err(Error::from("xmltv not defined")),
    };

    match config.get_str("output") {
        Some(v) => instance.open_output(v)?,
        None => return Err(Error::from("output not defined")),
    };

    for m in config.iter() {
        if m.get_name() != "multiplex" || false == m.get("enable", true)? {
            continue;
        }

        instance.multiplex.onid = m.get("onid", instance.onid)?;
        instance.multiplex.codepage = m.get("codepage", instance.codepage)?;
        instance.multiplex.tsid = m.get("tsid", 1)?;
        // TODO: custom xmltv

        for s in m.iter() {
            if s.get_name() != "service" {
                continue;
            }

            let mut service = Service::default();
            match s.get_str("xmltv-id") {
                Some(v) => service.xmltv_id.push_str(v),
                None => {
                    eprintln!("Warning: 'xmltv-id' option not defined for service at line {}", s.get_line());
                    continue;
                },
            };
            service.epg_item_id = instance.multiplex.epg_item_id; // ?WTF
            service.onid = instance.multiplex.onid;
            service.tsid = instance.multiplex.tsid;
            service.codepage = s.get("codepage", instance.multiplex.codepage)?;
            service.pnr = s.get("pnr", 0)?;
            // TODO: custom xmltv
            instance.service_list.push(service);
        }
    }

    // Prepare EIT from EPG
    let now = chrono::Utc::now();
    let current_time = now.timestamp() as u64;
    let last_time = (now + chrono::Duration::days(instance.eit_days as i64)).timestamp() as u64;

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
            if event.start > last_time {
                break;
            }
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

    let rate_limit = instance.eit_rate * 1000 / 8;

    let mut present_skip = 0;
    let mut schedule_skip = 0;

    let idle_delay = time::Duration::from_secs(1);

    loop {
        while present_skip < instance.service_list.len() {
            let service = &mut instance.service_list[present_skip];
            service.clear();
            service.present.demux(EIT_PID, &mut cc, &mut ts);
            present_skip += 1;
            if ts.len() >= rate_limit {
                break;
            }
        }

        if present_skip == instance.service_list.len() {
            present_skip = 0;

            while schedule_skip < instance.service_list.len() {
                let service = &instance.service_list[schedule_skip];
                service.schedule.demux(EIT_PID, &mut cc, &mut ts);
                schedule_skip += 1;
                if ts.len() >= rate_limit {
                    break;
                }
            }

            if schedule_skip == instance.service_list.len() {
                schedule_skip = 0;
            }
        }

        let packets = ts.len() / ts::PACKET_SIZE;
        if packets == 0 {
            thread::sleep(idle_delay);
            continue;
        }

        let pps = time::Duration::from_nanos(1_000_000_000_u64 / (((6 + packets) / 7) as u64));

        // TODO: UDP output
        let mut skip = 0;
        while skip < ts.len() {
            let pkt_len = cmp::min(ts.len() - skip, 1316);
            let next = skip + pkt_len;
            if next > rate_limit { break };
            instance.output.send(&ts[skip..next]).unwrap();
            thread::sleep(pps);
            skip = next;
        }

        ts.drain(.. skip);
    }
}


fn main() {
    if let Err(e) = wrap() {
        println!("Error: {}", e.to_string());
    }
}
