use std::fs::File;
use std::io::Read;

use ini::{IniReader, IniItem};

use crate::error::Result;
use crate::{Instance, Multiplex, Service};


pub trait Config {
    fn section<R: Read>(&mut self, _name: &str, _reader: &mut IniReader<R>) -> Result<()> { unimplemented!() }
    fn property(&mut self, _key: &str, _value: &str) -> Result<()> { unimplemented!() }

    fn parse<R: Read>(&mut self, reader: &mut IniReader<R>) -> Result<()> {
        while let Some(e) = reader.next() {
            match e? {
                IniItem::Property(ref key, ref value) => self.property(key, value)?,
                IniItem::StartSection(ref name) => self.section(name, reader)?,
                IniItem::EndSection => break,
            };
        }
        Ok(())
    }
}


impl Config for Instance {
    fn section<R: Read>(&mut self, name: &str, reader: &mut IniReader<R>) -> Result<()> {
        match name {
            "multiplex" => {
                self.multiplex.onid = self.onid;
                self.multiplex.codepage = self.codepage;
                self.multiplex.parse(reader)?;
            },
            "service" => {
                let mut service = Service::default();
                service.epg_item_id = self.multiplex.epg_item_id;
                service.onid = self.multiplex.onid;
                service.tsid = self.multiplex.tsid;
                service.codepage = self.multiplex.codepage;
                service.parse(reader)?;
                self.service_list.push(service);
            },
            _ => {},
        };
        Ok(())
    }

    fn property(&mut self, key: &str, value: &str) -> Result<()> {
        match key {
            "xmltv" => self.open_xmltv(&value)?,
            "output" => self.open_output(&value)?,
            "onid" => self.onid = value.parse()?,
            "codepage" => self.codepage = value.parse()?,
            "eit-days" => self.eit_days = value.parse()?,
            "eit-rate" => self.eit_rate = value.parse()?,
            _ => {},
        };
        Ok(())
    }
}


impl Config for Multiplex {
    fn property(&mut self, key: &str, value: &str) -> Result<()> {
        match key {
            "onid" => self.onid = value.parse()?,
            "tsid" => self.tsid = value.parse()?,
            "codepage" => self.codepage = value.parse()?,
            // TODO: custom xmltv
            _ => {},
        };
        Ok(())
    }
}


impl Config for Service {
    fn property(&mut self, key: &str, value: &str) -> Result<()> {
        match key {
            "pnr" => self.pnr = value.parse()?,
            "codepage" => self.codepage = value.parse()?,
            "xmltv-id" => self.xmltv_id.push_str(&value),
            // TODO: custom xmltv
            _ => {},
        };
        Ok(())
    }
}


pub fn parse_config(instance: &mut Instance, path: &str) -> Result<()> {
    let config = File::open(path)?;
    let mut reader = IniReader::new(config);

    instance.onid = 1;
    instance.eit_days = 3;
    instance.eit_rate = 3000;

    instance.parse(&mut reader)
}
