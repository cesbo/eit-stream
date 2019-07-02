#[macro_use]
extern crate error_rules;

use std::{
    io,
    env,
    time,
    thread,
    cmp,
};

use chrono;

use epg::{
    Epg,
    EpgError,
};

use mpegts::{
    ts,
    psi::{
        EIT_PID,
        Eit,
        EitItem,
        PsiDemux,
    },
};

use udp::UdpSocket;

use config::{
    Config,
    Schema,
    ConfigError,
};


#[derive(Debug, Error)]
#[error_prefix = "App"]
enum AppError {
    #[error_from]
    Io(io::Error),
    #[error_from]
    Epg(EpgError),
    #[error_from]
    Config(ConfigError),
    #[error_kind("unknown output format")]
    UnknownOutput,
    #[error_kind("output not defined")]
    MissingOutput,
    #[error_kind("xmltv not defined")]
    MissingXmltv,
}


type Result<T> = std::result::Result<T, AppError>;


include!(concat!(env!("OUT_DIR"), "/build.rs"));


fn version() {
    println!("eit-stream v.{} commit:{}", env!("CARGO_PKG_VERSION"), COMMIT);
}


fn usage(program: &str) {
    println!(r#"Usage: {} CONFIG

OPTIONS:
    -v, --version       Version information
    -h, --help          Print this text
    -H                  Configuration file format

CONFIG:
    Path to configuration file
"#, program);
}


#[derive(Debug)]
enum Output {
    None,
    Udp(UdpSocket),
}


impl Default for Output {
    fn default() -> Self {
        Output::None
    }
}


impl Output {
    fn open(addr: &str) -> Result<Self> {
        // TODO: remove collect()
        let dst = addr.splitn(2, "://").collect::<Vec<&str>>();
        match dst[0] {
            "udp" => {
                let s = UdpSocket::open(dst[1])?;
                Ok(Output::Udp(s))
            },
            _ => Err(AppError::UnknownOutput),
        }
    }

    fn send(&self, data: &[u8]) -> Result<()> {
        match self {
            Output::Udp(ref udp) => {
                udp.sendto(data)?;
            },
            Output::None => {},
        };
        Ok(())
    }
}


#[derive(Default, Debug)]
struct Instance {
    epg_list: Vec<Epg>,
    output: Output,

    multiplex: Multiplex,
    service_list: Vec<Service>,

    onid: u16,
    codepage: u8,
    eit_days: usize,
    eit_rate: usize,
}


impl Instance {
    fn open_xmltv(&mut self, path: &str) -> Result<()> {
        let mut epg = Epg::default();
        epg.load(path)?;
        self.epg_list.push(epg);
        Ok(())
    }

    fn open_output(&mut self, addr: &str) -> Result<()> {
        self.output = Output::open(addr)?;
        Ok(())
    }

    fn parse_config(&mut self, config: &Config) -> Result<()> {
        if ! config.get("enable", true)? {
            return Ok(())
        }

        self.multiplex.onid = config.get("onid", self.onid)?;
        self.multiplex.codepage = config.get("codepage", self.codepage)?;
        self.multiplex.tsid = config.get("tsid", 1)?;
        // TODO: custom xmltv

        for s in config.iter() {
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
            service.epg_item_id = self.multiplex.epg_item_id; // ?WTF
            service.onid = self.multiplex.onid;
            service.tsid = self.multiplex.tsid;
            service.codepage = s.get("codepage", self.multiplex.codepage)?;
            service.pnr = s.get("pnr", 0)?;
            // TODO: custom xmltv
            self.service_list.push(service);
        }

        Ok(())
    }
}


#[derive(Default, Debug)]
struct Multiplex {
    epg_item_id: usize,

    onid: u16,
    tsid: u16,
    codepage: u8,
}


#[derive(Default, Debug)]
struct Service {
    epg_item_id: usize,

    onid: u16,
    tsid: u16,
    codepage: u8,

    pnr: u16,
    xmltv_id: String,

    present: Eit,
    schedule: Eit,

    ts: Vec<u8>,
}


impl Service {
    fn clear(&mut self) {
        let current_time = chrono::Utc::now().timestamp() as u64;

        if ! self.present.items.is_empty() {
            let event = self.present.items.first().unwrap();
            if event.start + u64::from(event.duration) > current_time {
                return;
            }
            self.present.items.remove(0);
        }

        if self.present.items.is_empty() {
            if self.schedule.items.is_empty() {
                return;
            }

            self.present.items.push(self.schedule.items.remove(0));
        }

        let event = self.present.items.first().unwrap();
        if event.start > current_time {
            return;
        }

        if ! self.schedule.items.is_empty() {
            self.present.items.push(self.schedule.items.remove(0));
        }

        let mut event = self.present.items.first_mut().unwrap();
        event.status = 4;
    }
}


fn init_schema() -> Schema {
    let codepage_validator = |s: &str| -> bool {
        let v = s.parse::<usize>().unwrap_or(1000);
        ((v <= 11) || (v >= 13 && v <= 15) || (v == 21))
    };

    let mut schema_service = Schema::new("service",
        "Service configuration. Multiplex contains one or more services");
    schema_service.set("pnr",
        "Program Number. Required. Should be in range 1 .. 65535",
        true, Schema::range(1 .. 65535));
    schema_service.set("xmltv-id",
        "Program indentifier in the XMLTV. Required",
        true, None);
    schema_service.set("codepage",
        "Redefine codepage for service. Default: multiplex codepage",
        false, codepage_validator);

    let mut schema_multiplex = Schema::new("multiplex",
        "Multiplex configuration. App contains one or more multiplexes");
    schema_multiplex.set("tsid",
        "Transport Stream Identifier. Required. Range 1 .. 65535",
        true, Schema::range(1 .. 65535));
    schema_multiplex.set("codepage",
        "Redefine codepage for multiplex. Default: app codepage",
        false, codepage_validator);
    schema_multiplex.push(schema_service);

    let mut schema = Schema::new("",
        "eit-stream - MPEG-TS EPG (Electronic Program Guide) streamer");
    schema.set("xmltv",
        "Full path to XMLTV file or http/https address. Required",
        true, None);
    // TODO: udp address validator
    schema.set("output",
        "UDP Address. Requried. Example: udp://239.255.1.1:10000",
        true, None);
    schema.set("onid",
        "Original Network Identifier. Default: 1",
        false, None);
    schema.set("codepage",
        "EPG Codepage. Default: 0 - Latin (ISO 6937). Available values:\n\
        ; 1 - Western European (ISO 8859-1)\n\
        ; 2 - Central European (ISO 8859-2)\n\
        ; 3 - South European (ISO 8859-3)\n\
        ; 4 - North European (ISO 8859-4)\n\
        ; 5 - Cyrillic (ISO 8859-5)\n\
        ; 6 - Arabic (ISO 8859-6)\n\
        ; 7 - Greek (ISO 8859-7)\n\
        ; 8 - Hebrew (ISO 8859-8)\n\
        ; 9 - Turkish (ISO 8859-9)\n\
        ; 10 - Nordic (ISO 8859-10)\n\
        ; 11 - Thai (ISO 8859-11)\n\
        ; 13 - Baltic Rim (ISO 8859-13)\n\
        ; 14 - Celtic (ISO 8859-14)\n\
        ; 15 - Western European (ISO 8859-15)\n\
        ; 21 - UTF-8",
        false, codepage_validator);
    schema.set("eit-days",
        "How many days includes into EPG schedule. Range: 1 .. 7. Default: 3",
        false, Schema::range(1 .. 7));
    schema.set("eit-rate",
        "Limit EPG output bitrate in kbit/s. Range: 100 .. 20000. Default: 3000",
        false, Schema::range(100 .. 20000));

    schema.push(schema_multiplex);

    schema
}


fn load_config() -> Result<Config> {
    use std::process::exit;

    let mut schema = init_schema();

    let mut args = env::args();
    let program = args.next().unwrap();
    let arg = match args.next() {
        Some(v) => match v.as_ref() {
            "-v" | "--version" => {
                version();
                exit(0);
            },
            "-h" | "--help" => {
                usage(&program);
                exit(0);
            },
            "-H" => {
                println!("Configuration file format:\n\n{}", &schema.info());
                exit(0);
            },
            _ => v,
        },
        None => {
            usage(&program);
            exit(0);
        },
    };

    let config = Config::open(&arg)?;
    schema.check(&config)?;

    Ok(config)
}


fn wrap() -> Result<()> {
    let config = load_config()?;

    let mut instance = Instance::default();

    instance.onid = config.get("onid", 1)?;
    instance.codepage = config.get("codepage", 0)?;
    instance.eit_days = config.get("eit-days", 3)?;
    instance.eit_rate = config.get("eit-rate", 3000)?;

    instance.open_xmltv(config.get_str("xmltv").ok_or(AppError::MissingXmltv)?)?;
    instance.open_output(config.get_str("output").ok_or(AppError::MissingOutput)?)?;

    for m in config.iter() {
        match m.get_name() {
            "multiplex" => instance.parse_config(m)?,
            _ => {}
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

    let mut eit_cc = 0;
    let mut ts = Vec::<u8>::new();

    let rate_limit = instance.eit_rate * 1000 / 8;

    let mut present_skip = 0;
    let mut schedule_skip = 0;

    let idle_delay = time::Duration::from_secs(1);

    loop {
        while present_skip < instance.service_list.len() {
            let service = &mut instance.service_list[present_skip];
            service.clear();
            service.present.demux(EIT_PID, &mut eit_cc, &mut ts);
            present_skip += 1;
            if ts.len() >= rate_limit {
                break;
            }
        }

        if present_skip == instance.service_list.len() {
            present_skip = 0;

            while schedule_skip < instance.service_list.len() {
                let service = &instance.service_list[schedule_skip];
                service.schedule.demux(EIT_PID, &mut eit_cc, &mut ts);
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
        println!("{}", e.to_string());
    }
}
