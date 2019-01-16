use std::fs::File;
use std::io::{Read, BufReader};

use ini::{IniReader, IniItem};

use crate::error::Result;
use crate::{Instance, Service};

fn parse_multiplex<R: Read>(instance: &mut Instance, config: &mut IniReader<R>) -> Result<()> {
    instance.multiplex.onid = instance.onid;
    instance.multiplex.codepage = instance.codepage;

    while let Some(e) = config.next() {
        match e? {
            IniItem::EndSection => break,
            IniItem::Property(key, value) => {
                match key.as_ref() {
                    "onid" => instance.multiplex.onid = value.parse()?,
                    "tsid" => instance.multiplex.tsid = value.parse()?,
                    "codepage" => instance.multiplex.codepage = value.parse()?,
                    // TODO: custom xmltv
                    _ => {},
                }
            },
            _ => {},
        };
    }

    Ok(())
}

fn parse_service<R: Read>(instance: &mut Instance, config: &mut IniReader<R>) -> Result<()> {
    let mut service = Service::default();
    service.epg_item_id = instance.multiplex.epg_item_id;
    service.onid = instance.multiplex.onid;
    service.tsid = instance.multiplex.tsid;
    service.codepage = instance.multiplex.codepage;

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

    instance.eit_schedule_time = 10;

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
                "eit_schedule_time" => instance.eit_schedule_time = value.parse()?,
                _ => {},
            },
            _ => {},
        };
    }

    Ok(())
}
