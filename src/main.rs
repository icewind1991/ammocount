use fnv::FnvHashMap;
use main_error::MainError;
use std::env::args;
use std::fs;
use std::io::Write;
use tf_demo_parser::demo::data::UserInfo;
use tf_demo_parser::demo::gameevent_gen::GameEvent;
use tf_demo_parser::demo::message::packetentities::{EntityId, PacketEntity};
use tf_demo_parser::demo::message::Message;
use tf_demo_parser::demo::packet::datatable::{
    ClassId, ParseSendTable, SendTableName, ServerClass, ServerClassName,
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
    let (path, user, start, end) = if let (Some(path), Some(user), Some(start), Some(end)) =
        (args.next(), args.next(), args.next(), args.next())
    {
        (
            path,
            user,
            start.parse().expect("invalid start tick"),
            end.parse().expect("invalid end tick"),
        )
    } else {
        println!("usage: {} <demo> <steam id> <start tick> <end tick>", bin);
        return Ok(());
    };
    let file = fs::read(&path)?;
    let demo = Demo::new(&file);
    let parser = DemoParser::new_all_with_analyser(demo.get_stream(), AmmoCountAnalyser::new(user));
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
    for (tick, clip, max_clip, health) in state
        .into_iter()
        .filter(|(tick, _, _, _)| *tick >= start && *tick <= end)
    {
        let frame = ((tick - start) as f32 * time_per_tick * 60.0) as i32;
        for frame in last_frame..frame {
            println!("txt[{}] = \"{}/{}\";", frame, clip, max_clip);
            writeln!(&mut ammo_out, "txt[{}] = \"{}/{}\";", frame, clip, max_clip)?;
            writeln!(&mut health_out, "txt[{}] = \"{}\";", frame, health)?;
        }
        last_frame = frame;
    }
    Ok(())
}

pub struct AmmoCountAnalyser {
    output: Vec<(u32, u16, u16, u16)>,
    max_clip: FnvHashMap<EntityId, u16>,
    clip: FnvHashMap<EntityId, u16>,
    current_health: u16,
    class_names: Vec<ServerClassName>,
    local_player_id: EntityId,
    local_user_id: UserId,
    outer_map: FnvHashMap<i64, EntityId>,
    active_weapon: i64,
    start_tick: u32,
    last_tick: u32,
    prop_names: FnvHashMap<SendPropIdentifier, (SendTableName, SendPropName)>,
    target_user_name: String,
}

impl MessageHandler for AmmoCountAnalyser {
    type Output = Vec<(u32, u16, u16, u16)>;

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
#[allow(dead_code)]
const UBER_CHARGE_PROP: SendPropIdentifier =
    SendPropIdentifier::new("DT_TFWeaponMedigunDataNonLocal", "m_flChargeLevel");
#[allow(dead_code)]
const UBER_CHARGE_PROP_LOCAL: SendPropIdentifier =
    SendPropIdentifier::new("DT_LocalTFWeaponMedigunData", "m_flChargeLevel");

#[allow(dead_code)]
const WEAPON1_ID_PROP: SendPropIdentifier = SendPropIdentifier::new("m_hMyWeapons", "000");
#[allow(dead_code)]
const WEAPON2_ID_PROP: SendPropIdentifier = SendPropIdentifier::new("m_hMyWeapons", "001");
#[allow(dead_code)]
const WEAPON3_ID_PROP: SendPropIdentifier = SendPropIdentifier::new("m_hMyWeapons", "002");

const OUTER_NULL: i64 = 0x1FFFFF;

impl AmmoCountAnalyser {
    pub fn new(target_user_name: String) -> Self {
        AmmoCountAnalyser {
            output: Default::default(),
            class_names: Default::default(),
            local_player_id: Default::default(),
            outer_map: Default::default(),
            active_weapon: 0,
            start_tick: 0,
            last_tick: 0,
            prop_names: Default::default(),
            target_user_name,
            max_clip: Default::default(),
            clip: Default::default(),
            local_user_id: 0u32.into(),
            current_health: 0,
        }
    }

    #[allow(dead_code)]
    fn server_class(&self, id: ClassId) -> &str {
        self.class_names[u16::from(id) as usize].as_str()
    }

    #[allow(dead_code)]
    fn prop_name(&self, id: SendPropIdentifier) -> String {
        let (t, n) = self.prop_names.get(&id).unwrap();
        format!("{}.{}", t, n)
    }

    fn handle_event(&mut self, event: &GameEvent) {
        match event {
            GameEvent::PlayerSpawn(spawn) => {
                if UserId::from(spawn.user_id) == self.local_user_id {
                    self.clip = self.max_clip.clone();
                }
            }
            _ => {}
        }
    }

    fn handle_entity(&mut self, tick: u32, entity: &PacketEntity) {
        if self.start_tick == 0 {
            self.start_tick = tick;
        }

        for prop in entity.props() {
            match prop.value {
                SendPropValue::Integer(value) if value != OUTER_NULL => match prop.identifier {
                    ACTIVE_WEAPON_PROP if entity.entity_index == self.local_player_id => {
                        self.active_weapon = value;
                    }
                    HEALTH_PROP if entity.entity_index == self.local_player_id => {
                        self.current_health = value as u16;
                    }
                    OUTER_CONTAINER_PROP => {
                        if !self.outer_map.contains_key(&value) {
                            self.outer_map.insert(value, entity.entity_index);
                        }
                    }
                    CLIP_PROP => {
                        let clip_max = self.max_clip.entry(entity.entity_index).or_default();
                        *clip_max = (*clip_max).max(value as u16);
                        self.clip.insert(entity.entity_index, value as u16);
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        if tick != self.last_tick && tick > self.start_tick {
            if let Some(active_weapon) = self.outer_map.get(&self.active_weapon) {
                if self.clip.contains_key(active_weapon) {
                    self.output.push((
                        tick - self.start_tick,
                        self.clip[active_weapon].saturating_sub(1),
                        self.max_clip[active_weapon].saturating_sub(1),
                        self.current_health,
                    ));
                }
            }
            self.last_tick = tick;
        }
    }

    fn parse_user_info(&mut self, text: Option<&str>, data: Option<Stream>) -> ReadResult<()> {
        if let Some(user_info) = UserInfo::parse_from_string_table(text, data)? {
            if user_info
                .player_info
                .name
                .to_ascii_lowercase()
                .contains(&self.target_user_name)
            {
                self.local_player_id = user_info.entity_id;
                self.local_user_id = user_info.player_info.user_id.into();
            }
        }

        Ok(())
    }
}
