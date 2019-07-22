#[macro_use]
extern crate error_rules;

use std::{
    io,
    time,
    thread,
    cmp,
    collections::HashMap,
};

use chrono;

use epg::{
    Epg,
    EpgError,
};

use mpegts::{
    ts,
    psi::{
        self,
        PsiDemux,
        Eit,
        EitItem,
        Tdt,
        Tot,
        Desc58,
        Desc58i,
    },
    textcode,
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


#[derive(Debug, Default)]
struct TdtTot {
    cc: u8,
    tdt: Tdt,
    tot: Tot,
}


impl TdtTot {
    fn parse_config(&mut self, config: &Config) -> Result<()> {
        let country = config.get_str("country").unwrap_or("   ");

        let (offset, offset_polarity) = {
            let offset = config.get_str("offset").unwrap_or("0");
            match offset.as_bytes()[0] {
                b'+' => (offset[1 ..].parse::<u16>().unwrap(), 0),
                b'-' => (offset[1 ..].parse::<u16>().unwrap(), 1),
                _ => (0, 0),
            }
        };

        if self.tot.descriptors.is_empty() {
            self.tot.descriptors.push(Desc58::default());
        }

        let desc = self.tot.descriptors
            .get_mut(0).unwrap()
            .downcast_mut::<Desc58>();

        desc.items.push(Desc58i {
            country_code: textcode::StringDVB::from_str(country, textcode::ISO6937),
            region_id: 0,
            offset_polarity,
            offset,
            time_of_change: 0,
            next_offset: 0,
        });

        Ok(())
    }

    fn update(&mut self) {
        let timestamp = time::SystemTime::now()
            .duration_since(time::UNIX_EPOCH).unwrap()
            .as_secs();
        self.tdt.time = timestamp;
        self.tot.time = timestamp;
    }

    fn demux(&mut self, dst: &mut Vec<u8>) {
        self.update();
        self.tdt.demux(psi::TDT_PID, &mut self.cc, dst);
        self.tot.demux(psi::TOT_PID, &mut self.cc, dst);
    }
}


#[derive(Default, Debug)]
struct Instance {
    epg_item_id: usize,
    epg_list: Vec<Epg>,
    epg_map: HashMap<String, usize>,

    output: Output,

    multiplex: Multiplex,
    service_list: Vec<Service>,

    onid: u16,
    codepage: u8,
    eit_days: usize,
    eit_rate: usize,

    tdt_tot: Option<TdtTot>,
}


impl Instance {
    fn open_xmltv(&mut self, config: &Config, def: usize) -> Result<usize> {
        let path = match config.get_str("xmltv") {
            Some(v) => v,
            None => return Ok(def),
        };

        if let Some(&v) = self.epg_map.get(path) {
            return Ok(v);
        }

        let mut epg = Epg::default();
        epg.load(path)?;
        let v = self.epg_list.len();
        self.epg_list.push(epg);
        self.epg_map.insert(path.to_owned(), v);

        Ok(v)
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
        self.multiplex.epg_item_id = self.open_xmltv(&config, self.epg_item_id)?;

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

            service.epg_item_id = self.open_xmltv(s, self.multiplex.epg_item_id)?;
            if service.epg_item_id == usize::max_value() {
                return Err(AppError::MissingXmltv);
            }

            service.onid = self.multiplex.onid;
            service.tsid = self.multiplex.tsid;
            service.codepage = s.get("codepage", self.multiplex.codepage)?;
            service.pnr = s.get("pnr", 0)?;
            self.service_list.push(service);
        }

        Ok(())
    }

    fn parse_tdt_tot(&mut self, config: &Config) -> Result<()> {
        if let Some(t) = &mut self.tdt_tot {
            t.parse_config(config)?;
        } else {
            let mut t = TdtTot::default();
            t.parse_config(config)?;
            self.tdt_tot = Some(t);
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

    let country_validator = |s: &str| -> bool {
        s.len() == 3
    };

    let offset_validator = |s: &str| -> bool {
        if s.is_empty() { return false }
        match s.as_bytes()[0] {
            b'+' => s[1 ..].parse::<u16>()
                .and_then(|v| Ok(v <= 720))
                .unwrap_or(false),
            b'-' => s[1 ..].parse::<u16>()
                .and_then(|v| Ok(v <= 780))
                .unwrap_or(false),
            b'0' if s.len() == 1 => true,
            _ => false,
        }
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
    schema_service.set("xmltv",
        "Redefine XMLTV source for service. Default: multiplex xmltv",
        false, None);

    let mut schema_multiplex = Schema::new("multiplex",
        "Multiplex configuration. App contains one or more multiplexes");
    schema_multiplex.set("tsid",
        "Transport Stream Identifier. Required. Range 1 .. 65535",
        true, Schema::range(1 .. 65535));
    schema_multiplex.set("codepage",
        "Redefine codepage for multiplex. Default: app codepage",
        false, codepage_validator);
    schema_multiplex.set("xmltv",
        "Redefine XMLTV source for multiplex. Default: app xmltv",
        false, None);
    schema_multiplex.push(schema_service);

    let mut schema_tdt_tot = Schema::new("tdt-tot",
        "Generate TDT/TOT tables");
    schema_tdt_tot.set("country",
        "Country code in ISO 3166-1 alpha-3 format",
        false, country_validator);
    schema_tdt_tot.set("offset",
        "Offset time from UTC in the range between -720 minutes and +780 minutes. Default: 0",
        false, offset_validator);

    let mut schema = Schema::new("",
        "eit-stream - MPEG-TS EPG (Electronic Program Guide) streamer\n");
    schema.set("xmltv",
        "Full path to XMLTV file or http/https address",
        false, None);
    // TODO: udp address validator
    schema.set("output",
        "UDP Address. Requried. Example: udp://239.255.1.1:10000",
        true, None);
    schema.set("onid",
        "Original Network Identifier. Default: 1",
        false, None);
    schema.set("codepage",
        "EPG Codepage. Default: 0 - Latin (ISO 6937). Available values:\n\
        ;  1 - Western European (ISO 8859-1)\n\
        ;  2 - Central European (ISO 8859-2)\n\
        ;  3 - South European (ISO 8859-3)\n\
        ;  4 - North European (ISO 8859-4)\n\
        ;  5 - Cyrillic (ISO 8859-5)\n\
        ;  6 - Arabic (ISO 8859-6)\n\
        ;  7 - Greek (ISO 8859-7)\n\
        ;  8 - Hebrew (ISO 8859-8)\n\
        ;  9 - Turkish (ISO 8859-9)\n\
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

    schema.push(schema_tdt_tot);
    schema.push(schema_multiplex);

    schema
}


fn load_config() -> Result<Config> {
    use std::process::exit;

    let mut schema = init_schema();

    let mut args = std::env::args();
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

    instance.epg_item_id = instance.open_xmltv(&config, usize::max_value())?;
    instance.open_output(config.get_str("output").ok_or(AppError::MissingOutput)?)?;

    for m in config.iter() {
        match m.get_name() {
            "multiplex" => instance.parse_config(m)?,
            "tdt-tot" => instance.parse_tdt_tot(m)?,
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

    let rate_limit = instance.eit_rate * 1000 / 8;
    let mut ts = Vec::<u8>::with_capacity(rate_limit);

    let mut present_skip = 0;
    let mut schedule_skip = 0;

    let idle_delay = time::Duration::from_secs(1);

    loop {
        if let Some(tdt_tot) = &mut instance.tdt_tot {
            tdt_tot.demux(&mut ts);
        }

        while present_skip < instance.service_list.len() {
            let service = &mut instance.service_list[present_skip];
            service.clear();
            service.present.demux(psi::EIT_PID, &mut eit_cc, &mut ts);
            present_skip += 1;
            if ts.len() >= rate_limit {
                break;
            }
        }

        if present_skip == instance.service_list.len() {
            present_skip = 0;

            while schedule_skip < instance.service_list.len() {
                let service = &instance.service_list[schedule_skip];
                service.schedule.demux(psi::EIT_PID, &mut eit_cc, &mut ts);
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
