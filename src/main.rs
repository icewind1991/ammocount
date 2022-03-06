use fnv::FnvHashMap;
use main_error::MainError;
use std::convert::TryFrom;
use std::env::args;
use std::fs;
use std::io::Write;
use steamid_ng::SteamID;
use tf_demo_parser::demo::data::UserInfo;
use tf_demo_parser::demo::gameevent_gen::GameEvent;
use tf_demo_parser::demo::message::packetentities::{EntityId, PacketEntity};
use tf_demo_parser::demo::message::Message;
use tf_demo_parser::demo::packet::datatable::{
    ParseSendTable, SendTableName, ServerClass, ServerClassName,
};
use tf_demo_parser::demo::packet::stringtable::StringTableEntry;
use tf_demo_parser::demo::parser::gamestateanalyser::UserId;
use tf_demo_parser::demo::parser::MessageHandler;
use tf_demo_parser::demo::sendprop::{SendPropIdentifier, SendPropName, SendPropValue};
use tf_demo_parser::{Demo, MessageType, ParserState};
use tf_demo_parser::{DemoParser, ReadResult, Stream};

fn main() -> Result<(), MainError> {
    let mut args = args();
    let bin = args.next().unwrap();
    let (path, steam_id, start, end, clipsize1, clipsize2) = if let (
        Some(path),
        Some(steam_id),
        Some(start),
        Some(end),
        Some(clipsize1),
        Some(clipsize2),
    ) = (
        args.next(),
        args.next(),
        args.next(),
        args.next(),
        args.next(),
        args.next(),
    ) {
        (
            path,
            SteamID::try_from(steam_id.as_str()).expect("invalid steam id"),
            start.parse().expect("invalid start tick"),
            end.parse().expect("invalid end tick"),
            clipsize1.parse().expect("invalid end tick"),
            clipsize2.parse().expect("invalid end tick"),
        )
    } else {
        println!(
                "usage: {} <demo> <steam id> <start tick> <end tick> <clipsize primary> <clipsize secondary>",
                bin
            );
        return Ok(());
    };
    let file = fs::read(&path)?;
    let demo = Demo::new(&file);
    let parser = DemoParser::new_all_with_analyser(
        demo.get_stream(),
        AmmoCountAnalyser::new(steam_id, [clipsize1, clipsize2]),
    );
    let (header, state) = parser.parse()?;
    let time_per_tick = header.duration / header.ticks as f32;
    let ammo_path = format!("{}_ammo.txt", path);
    let health_path = format!("{}_health.txt", path);
    let mut ammo_out = fs::File::create(ammo_path)?;
    let mut health_out = fs::File::create(health_path)?;
    println!("txt = []");
    writeln!(&mut ammo_out, "txt = []")?;
    writeln!(&mut health_out, "txt = []")?;
    let mut last_frame = 0;
    for (tick, clip, weapon_index, health) in state
        .into_iter()
        .filter(|(tick, _, _, _)| *tick >= start && *tick <= end)
    {
        let frame = ((tick - start) as f32 * time_per_tick * 60.0) as i32;
        let clip_size = if weapon_index == 0 {
            clipsize1
        } else if weapon_index == 1 {
            clipsize2
        } else {
            0
        };
        for frame in last_frame..frame {
            if clip_size == 0 {
                println!("txt[{}] = \"\";", frame);
                writeln!(&mut ammo_out, "txt[{}] = \"\";", frame)?;
            } else {
                println!("txt[{}] = \"{}/{}\";", frame, clip, clip_size);
                writeln!(
                    &mut ammo_out,
                    "txt[{}] = \"{}/{}\";",
                    frame, clip, clip_size
                )?;
            }
            writeln!(&mut health_out, "txt[{}] = \"{}\";", frame, health)?;
        }
        last_frame = frame;
    }
    Ok(())
}

pub struct AmmoCountAnalyser {
    output: Vec<(u32, u16, u8, u16)>,
    current_clip: [u16; 3],
    max_clip: [u16; 2],
    current_health: u16,
    class_names: Vec<ServerClassName>,
    // indexed by ClassId
    local_player_id: EntityId,
    local_user_id: UserId,
    local_weapons_ids: [i64; 3],
    outer_map: FnvHashMap<i64, EntityId>,
    active_weapon: i64,
    start_tick: u32,
    last_tick: u32,
    prop_names: FnvHashMap<SendPropIdentifier, (SendTableName, SendPropName)>,
    local_steam_id: SteamID,
}

impl MessageHandler for AmmoCountAnalyser {
    type Output = Vec<(u32, u16, u8, u16)>;

    fn does_handle(_message_type: MessageType) -> bool {
        true
    }

    fn handle_message(&mut self, message: &Message, tick: u32) {
        match message {
            Message::PacketEntities(entities) => {
                for entity in &entities.entities {
                    self.handle_entity(tick, entity)
                }
            }
            Message::GameEvent(event_msg) => {
                self.handle_event(&event_msg.event);
            }
            _ => {}
        }
    }

    fn handle_string_entry(&mut self, table: &str, _index: usize, entry: &StringTableEntry) {
        match table {
            "userinfo" => {
                let _ = self.parse_user_info(
                    entry.text.as_ref().map(|s| s.as_ref()),
                    entry.extra_data.as_ref().map(|data| data.data.clone()),
                );
            }
            _ => {}
        }
    }

    fn handle_data_tables(
        &mut self,
        parse_tables: &[ParseSendTable],
        server_classes: &[ServerClass],
    ) {
        for table in parse_tables {
            for prop_def in &table.props {
                // println!("{}.{}", prop_def.owner_table, prop_def.name);
                self.prop_names.insert(
                    prop_def.identifier(),
                    (table.name.clone(), prop_def.name.clone()),
                );
            }
        }

        self.class_names = server_classes
            .iter()
            .map(|class| &class.name)
            .cloned()
            .collect();
    }

    fn into_output(self, _state: &ParserState) -> Self::Output {
        self.output
    }
}

const CLIP_PROP: SendPropIdentifier = SendPropIdentifier::new("DT_LocalWeaponData", "m_iClip1");
const OUTER_CONTAINER_PROP: SendPropIdentifier =
    SendPropIdentifier::new("DT_AttributeContainer", "m_hOuter");
const ACTIVE_WEAPON_PROP: SendPropIdentifier =
    SendPropIdentifier::new("DT_BaseCombatCharacter", "m_hActiveWeapon");
const HEALTH_PROP: SendPropIdentifier = SendPropIdentifier::new("DT_BasePlayer", "m_iHealth");
const UBER_CHARGE_PROP: SendPropIdentifier =
    SendPropIdentifier::new("DT_TFWeaponMedigunDataNonLocal", "m_flChargeLevel");
const UBER_CHARGE_PROP_LOCAL: SendPropIdentifier =
    SendPropIdentifier::new("DT_LocalTFWeaponMedigunData", "m_flChargeLevel");

const WEAPON1_ID_PROP: SendPropIdentifier = SendPropIdentifier::new("m_hMyWeapons", "000");
const WEAPON2_ID_PROP: SendPropIdentifier = SendPropIdentifier::new("m_hMyWeapons", "001");
const WEAPON3_ID_PROP: SendPropIdentifier = SendPropIdentifier::new("m_hMyWeapons", "002");

impl AmmoCountAnalyser {
    pub fn new(steam_id: SteamID, max_clip: [u16; 2]) -> Self {
        AmmoCountAnalyser {
            output: Default::default(),
            current_clip: Default::default(),
            class_names: Default::default(),
            local_player_id: Default::default(),
            local_weapons_ids: Default::default(),
            outer_map: Default::default(),
            active_weapon: 0,
            start_tick: 0,
            last_tick: 0,
            prop_names: Default::default(),
            local_steam_id: steam_id,
            max_clip,
            local_user_id: 0u32.into(),
            current_health: 0,
        }
    }

    fn handle_event(&mut self, event: &GameEvent) {
        match event {
            GameEvent::PlayerSpawn(spawn) => {
                if UserId::from(spawn.user_id) == self.local_user_id {
                    self.current_clip[0] = self.max_clip[0];
                    self.current_clip[1] = self.max_clip[1];
                }
            }
            _ => {}
        }
    }

    fn handle_entity(&mut self, tick: u32, entity: &PacketEntity) {
        if self.start_tick == 0 {
            self.start_tick = tick;
        }
        self.handle_attribute_container(entity);

        for prop in entity.props() {
            match prop.value {
                SendPropValue::Integer(id) if id != 2097151 => {
                    if entity.entity_index == self.local_player_id {
                        if prop.identifier == WEAPON1_ID_PROP {
                            self.local_weapons_ids[0] = id;
                        } else if prop.identifier == WEAPON2_ID_PROP {
                            self.local_weapons_ids[1] = id;
                        } else if prop.identifier == WEAPON3_ID_PROP {
                            self.local_weapons_ids[2] = id;
                        } else if prop.identifier == ACTIVE_WEAPON_PROP {
                            self.active_weapon = id;
                        } else if prop.identifier == HEALTH_PROP {
                            self.current_health = id as u16;
                        }
                    }
                }
                _ => {}
            }
            for i in 0..3 {
                if let Some(weapon_entity_id) = self.outer_map.get(&self.local_weapons_ids[i]) {
                    if entity.entity_index == *weapon_entity_id {
                        if prop.identifier == CLIP_PROP {
                            if let SendPropValue::Integer(value) = prop.value {
                                let value = value - 1; //clip size starts from 1
                                self.current_clip[i] = value as _;
                            } else {
                                panic!("{}", prop.value)
                            }
                        }
                    }
                }
            }
        }

        if tick != self.last_tick && tick > self.start_tick {
            for i in 0..3 {
                if self.local_weapons_ids[i] == self.active_weapon {
                    self.output.push((
                        tick - self.start_tick,
                        self.current_clip[i],
                        i as u8,
                        self.current_health,
                    ));
                }
            }
            self.last_tick = tick;
        }
    }

    fn handle_attribute_container(&mut self, entity: &PacketEntity) {
        let mut out = 2097151;
        for prop in entity.props() {
            if prop.identifier == OUTER_CONTAINER_PROP {
                if let SendPropValue::Integer(outer_id) = prop.value {
                    if outer_id != 2097151 {
                        out = outer_id;
                    }
                }
            }
        }
        if out != 2097151 {
            if !self.outer_map.contains_key(&out) {
                self.outer_map.insert(out, entity.entity_index);
            }
        }
    }

    fn parse_user_info(&mut self, text: Option<&str>, data: Option<Stream>) -> ReadResult<()> {
        if let Some(user_info) = UserInfo::parse_from_string_table(text, data)? {
            if SteamID::try_from(user_info.player_info.steam_id.as_str()).ok()
                == Some(self.local_steam_id)
            {
                self.local_player_id = user_info.entity_id;
                self.local_user_id = user_info.player_info.user_id.into();
            }
        }

        Ok(())
    }
}
