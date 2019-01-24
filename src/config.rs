use std::fs::File;
use std::io::Read;

use ini::{IniReader, IniItem};

use crate::error::Result;
use crate::{Instance, Service};


pub trait Config {
    fn property(&mut self, _key: &str, _value: &str) -> Result<()>;

    fn parse<R: Read>(&mut self, reader: &mut IniReader<R>) -> Result <()>{
        while let Some(e) = reader.next() {
            match e? {
                IniItem::Property(ref key, ref value) => self.property(key, value)?,
                IniItem::StartSection(ref _name) => unimplemented!(),
                IniItem::EndSection => break,
            };
        }
        Ok(())
    }
}


pub fn parse_config(instance: &mut Instance, path: &str) -> Result<()> {
    let config = File::open(path)?;
    let mut config = IniReader::new(config);

    while let Some(e) = config.next() {
        match e? {
            IniItem::StartSection(name) => match name.as_ref() {
                "multiplex" => {
                    instance.multiplex.onid = instance.onid;
                    instance.multiplex.codepage = instance.codepage;
                    instance.multiplex.parse(&mut config)?;
                },
                "service" => {
                    let mut service = Service::default();
                    service.epg_item_id = instance.multiplex.epg_item_id;
                    service.onid = instance.multiplex.onid;
                    service.tsid = instance.multiplex.tsid;
                    service.codepage = instance.multiplex.codepage;
                    service.parse(&mut config)?;
                    instance.service_list.push(service);
                },
                _ => {},
            },
            IniItem::Property(key, value) => match key.as_ref() {
                "xmltv" => instance.open_xmltv(&value)?,
                "output" => instance.open_output(&value)?,
                "onid" => instance.onid = value.parse()?,
                "codepage" => instance.codepage = value.parse()?,
                "eit-days" => instance.eit_days = value.parse()?,
                "eit-rate" => instance.eit_rate = value.parse()?,
                _ => {},
            },
            _ => {},
        };
    }

    Ok(())
}
