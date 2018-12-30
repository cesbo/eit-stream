use std::fs::File;
use std::io::{Read, BufReader};

use ini::{IniReader, IniItem};

use crate::error::{Error, Result};
use crate::{Instance, Multiplex, Service};

fn parse_multiplex<R: Read>(instance: &mut Instance, config: &mut IniReader<R>) -> Result<()> {
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

fn parse_service<R: Read>(instance: &mut Instance, config: &mut IniReader<R>) -> Result<()> {
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

pub fn parse_config(instance: &mut Instance, path: &str) -> Result<()> {
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
                "xmltv" => instance.open_xmltv(&value)?,
                "output" => instance.open_output(&value)?,
                "onid" => instance.onid = value.parse()?,
                "codepage" => instance.codepage = value.parse()?,
                _ => {},
            },
            _ => {},
        };
    }

    Ok(())
}
