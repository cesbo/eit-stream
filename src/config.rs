use ini::{IniReader, Section};

use crate::error::Result;
use crate::{Instance, Service};


fn parse_multiplex(instance: &mut Instance, section: &Section) -> Result<()> {
    instance.multiplex.onid = instance.onid;
    instance.multiplex.codepage = instance.codepage;

    for (key, value) in section {
        match key.as_str() {
            "onid" => instance.multiplex.onid = value.parse()?,
            "tsid" => instance.multiplex.tsid = value.parse()?,
            "codepage" => instance.multiplex.codepage = value.parse()?,
            // TODO: custom xmltv
            _ => {},
        };
    }

    Ok(())
}


fn parse_service(instance: &mut Instance, section: &Section) -> Result<()> {
    let mut service = Service::default();
    service.epg_item_id = instance.multiplex.epg_item_id;
    service.onid = instance.multiplex.onid;
    service.tsid = instance.multiplex.tsid;
    service.codepage = instance.multiplex.codepage;

    for (key, value) in section {
        match key.as_str() {
            "pnr" => service.pnr = value.parse()?,
            "codepage" => service.codepage = value.parse()?,
            "xmltv-id" => service.xmltv_id.push_str(&value),
            // TODO: custom xmltv
            _ => {},
        };
    }

    instance.service_list.push(service);
    Ok(())
}


fn parse_base(instance: &mut Instance, section: &Section) -> Result<()> {
    for (key, value) in section {
        match key.as_str() {
            "xmltv" => instance.open_xmltv(&value)?,
            "output" => instance.open_output(&value)?,
            "onid" => instance.onid = value.parse()?,
            "codepage" => instance.codepage = value.parse()?,
            "eit-days" => instance.eit_days = value.parse()?,
            "eit-rate" => instance.eit_rate = value.parse()?,
            _ => {},
        };
    }
    Ok(())
}

pub fn parse_config(instance: &mut Instance, path: &str) -> Result<()> {
    let config = IniReader::open(path)?;

    instance.onid = 1;
    instance.eit_days = 3;
    instance.eit_rate = 3000;

    for (name, section) in &config {
        match name.as_str() {
            "" => parse_base(instance, section)?,
            "multiplex" => parse_multiplex(instance, section)?,
            "service" => parse_service(instance, section)?,
            _ => {},
        }
    }

    Ok(())
}
