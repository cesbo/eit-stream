mod error;
use crate::error::{Error, Result};

use std::{io, env, time, thread, cmp};
use ini::{IniReader, IniItem};

use std::fs::File;
use std::io::{BufRead, BufReader};
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
    xmltv_list: Vec<Epg>,
    output_list: Vec<UdpSocket>,

    multiplex_list: Vec<Multiplex>,
    service_list: Vec<Service>,

    onid: u16,
    codepage: u8,
}

#[derive(Default, Debug)]
struct Multiplex {
    xmltv_item_id: usize,
    output_item_id: usize,

    onid: u16,
    tsid: u16,
    codepage: u8,
}

#[derive(Default, Debug)]
struct Service {
    xmltv_item_id: usize,
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

fn parse_service<R: io::Read>(instance: &mut Instance, config: &mut IniReader<R>) -> Result<()> {
    let multiplex = match instance.multiplex_list.last() {
        Some(v) => v,
        None => return Err(Error::from("multiplex section not found")),
    };

    let mut service = Service::default();
    service.xmltv_item_id = multiplex.xmltv_item_id;
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
    instance.xmltv_list.push(epg);
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

    if instance.xmltv_list.is_empty() {
        return Err(Error::from("xmltv not defined"));
    }

    if instance.output_list.is_empty() {
        return Err(Error::from("output not defined"));
    }

    println!("{:#?}", instance);

    Ok(())
}

fn main() {
    if let Err(e) = wrap() {
        println!("Error: {}", e.to_string());
    }
}
