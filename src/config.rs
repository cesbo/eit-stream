use std::fs::File;
use std::io::Read;

use ini::{IniReader, IniItem};

use crate::error::Result;
use crate::{Instance, Service};


fn parse_multiplex<R: Read>(instance: &mut Instance, reader: &mut IniReader<R>) -> Result<()> {
    instance.multiplex.onid = instance.onid;
    instance.multiplex.codepage = instance.codepage;

    while let Some(e) = reader.next() {
        if let IniItem::Property(ref key, ref value) = e? {
            match key.as_str() {
                "onid" => instance.multiplex.onid = value.parse()?,
                "tsid" => instance.multiplex.tsid = value.parse()?,
                "codepage" => instance.multiplex.codepage = value.parse()?,
                // TODO: custom xmltv
                _ => {},
            };
        } else {
            break
        }
    }
    Ok(())
}


fn parse_service<R: Read>(instance: &mut Instance, reader: &mut IniReader<R>) -> Result<()> {
    let mut service = Service::default();
    service.epg_item_id = instance.multiplex.epg_item_id;
    service.onid = instance.multiplex.onid;
    service.tsid = instance.multiplex.tsid;
    service.codepage = instance.multiplex.codepage;

    while let Some(e) = reader.next() {
        if let IniItem::Property(ref key, ref value) = e? {
            match key.as_str() {
                "pnr" => service.pnr = value.parse()?,
                "codepage" => service.codepage = value.parse()?,
                "xmltv-id" => service.xmltv_id.push_str(&value),
                // TODO: custom xmltv
                _ => {},
            };
        } else {
            break
        }
    }

    instance.service_list.push(service);
    Ok(())
}


fn skip_section<R: Read>(reader: &mut IniReader<R>) -> Result<()> {
    while let Some(e) = reader.next() {
        if let IniItem::EndSection = e? {
            break
        }
    }

    Ok(())
}


pub fn parse_config(instance: &mut Instance, path: &str) -> Result<()> {
    let config = File::open(path)?;
    let mut reader = IniReader::new(config);

    instance.onid = 1;
    instance.eit_days = 3;
    instance.eit_rate = 3000;

    while let Some(e) = reader.next() {
        match e? {
            IniItem::Property(ref key, ref value) => {
                match key.as_str() {
                    "xmltv" => instance.open_xmltv(&value)?,
                    "output" => instance.open_output(&value)?,
                    "onid" => instance.onid = value.parse()?,
                    "codepage" => instance.codepage = value.parse()?,
                    "eit-days" => instance.eit_days = value.parse()?,
                    "eit-rate" => instance.eit_rate = value.parse()?,
                    _ => {},
                };
            },
            IniItem::StartSection(ref name) => {
                match name.as_str() {
                    "multiplex" => parse_multiplex(instance, &mut reader)?,
                    "service" => parse_service(instance, &mut reader)?,
                    _ => skip_section(&mut reader)?,
                };
            },
            _ => {},
        };
    }
    Ok(())
}
