mod playersearch;
mod wrapping;

use crate::playersearch::get_player;
use crate::wrapping::Wrapping;
use cgmath::{Deg, Euler, Matrix3, Quaternion};
use fnv::FnvHashMap;
use main_error::MainError;
use serde::Serialize;
use splines::{Interpolation, Key, Spline};
use std::collections::HashMap;
use std::env::args;
use std::fs;
use std::io::Write;
use tf_demo_parser::demo::gameevent_gen::GameEvent;
use tf_demo_parser::demo::message::packetentities::{EntityId, PacketEntity};
use tf_demo_parser::demo::message::Message;
use tf_demo_parser::demo::packet::datatable::{
    ClassId, ParseSendTable, ServerClass, ServerClassName,
};
use tf_demo_parser::demo::packet::message::MessagePacketMeta;
use tf_demo_parser::demo::packet::stringtable::StringTableEntry;
use tf_demo_parser::demo::parser::gamestateanalyser::UserId;
use tf_demo_parser::demo::parser::MessageHandler;
use tf_demo_parser::demo::sendprop::{SendPropIdentifier, SendPropValue};
use tf_demo_parser::demo::vector::Vector;
use tf_demo_parser::DemoParser;
use tf_demo_parser::{Demo, MessageType, ParserState};
use tracing::warn;

fn main() -> Result<(), MainError> {
    let mut args = args();
    tracing_subscriber::fmt::init();
    let bin = args.next().unwrap();
    let (path, user, start, end) = match (args.next(), args.next(), args.next(), args.next()) {
        (Some(path), Some(user), Some(start), Some(end)) => (
            path,
            user,
            start.parse().expect("invalid start tick"),
            end.parse().expect("invalid end tick"),
        ),
        (Some(path), _, _, _) => {
            let (user, start, end) = args_from_name(&path);
            println!("Using {} with ticks {} to {}", user, start, end);
            (path, user, start, end)
        }
        _ => {
            println!("usage: {} <demo> [steam id] [start tick] [end tick]", bin);
            return Ok(());
        }
    };

    let file = fs::read(&path)?;
    let demo = Demo::new(&file);
    let (local_player_id, local_user_id) = get_player(&demo, Some(user));
    let parser = DemoParser::new_all_with_analyser(
        demo.get_stream(),
        AmmoCountAnalyser::new(local_player_id, local_user_id),
    );
    let (header, (state, errors)) = parser.parse()?;
    let time_per_tick = header.duration / header.ticks as f32;
    let ammo_path = format!("{}_ammo.txt", path);
    let health_path = format!("{}_health.txt", path);
    let uber_path = format!("{}_uber.txt", path);
    let pitch_path = format!("{}_pitch.txt", path);
    let yaw_path = format!("{}_yaw.txt", path);
    let hit_path = format!("{}_hit.txt", path);
    let weapon_path = format!("{}_weapon.txt", path);
    let camera_path = format!("{}_camera.txt", path);
    let mut ammo_out = fs::File::create(ammo_path)?;
    let mut health_out = fs::File::create(health_path)?;
    let mut pitch_out = fs::File::create(pitch_path)?;
    let mut yaw_out = fs::File::create(yaw_path)?;
    let mut hit_out = fs::File::create(hit_path)?;
    let mut weapon_out = fs::File::create(weapon_path)?;
    let mut camera_out = fs::File::create(camera_path)?;
    let mut uber_out = None;
    writeln!(&mut ammo_out, "txt = []")?;
    writeln!(&mut health_out, "txt = []")?;
    writeln!(&mut pitch_out, "txt = []")?;
    writeln!(&mut yaw_out, "txt = []")?;
    writeln!(&mut hit_out, "txt = []")?;
    writeln!(&mut weapon_out, "txt = []")?;
    writeln!(&mut camera_out, "txt = []")?;
    let mut last_frame = 0;
    let mut last_angles: Option<[f32; 2]> = None;

    let mut hit_last_damage: u32 = 0;
    let mut hit_last_tick: u32 = 0;
    let hit_time: u32 = 33;

    let pitches: Vec<_> = state
        .iter()
        .filter_map(|data| Some((data.tick, data.angles[0]?)))
        .map(|(tick, pitch)| {
            Key::new(
                tick as f32,
                Wrapping::<-180, 180>(pitch),
                Interpolation::Cosine,
            )
        })
        .collect();
    let yaws: Vec<_> = state
        .iter()
        .filter_map(|data| Some((data.tick, data.angles[1]?)))
        .map(|(tick, yaw)| {
            Key::new(
                tick as f32,
                Wrapping::<-180, 180>(yaw),
                Interpolation::Cosine,
            )
        })
        .collect();
    let x_s: Vec<_> = state
        .iter()
        .map(|data| (data.tick, data.position.x))
        .map(|(tick, x)| Key::new(tick as f32, x, Interpolation::Linear))
        .collect();
    let y_s: Vec<_> = state
        .iter()
        .map(|data| (data.tick, data.position.y))
        .map(|(tick, y)| Key::new(tick as f32, y, Interpolation::Linear))
        .collect();
    let z_s: Vec<_> = state
        .iter()
        .map(|data| (data.tick, data.position.z))
        .map(|(tick, z)| Key::new(tick as f32, z, Interpolation::Linear))
        .collect();

    let pitches = Spline::from_vec(pitches);
    let yaws = Spline::from_vec(yaws);

    let x_s = Spline::from_vec(x_s);
    let y_s = Spline::from_vec(y_s);
    let z_s = Spline::from_vec(z_s);

    let mut ticks_done = 0;

    let start_position = Vector {
        x: x_s.clamped_sample(start as f32).unwrap(),
        y: y_s.clamped_sample(start as f32).unwrap(),
        z: z_s.clamped_sample(start as f32).unwrap(),
    };
    let start_angles = [
        pitches.clamped_sample(start as f32).unwrap().0,
        yaws.clamped_sample(start as f32).unwrap().0,
    ];

    for data in state
        .into_iter()
        .filter(|data| data.tick >= start && data.tick <= end)
    {
        let frame = ((data.tick - start) as f32 * time_per_tick * 120.0) as i32;

        if let Some(hit) = data.hit {
            hit_last_damage = hit;
            hit_last_tick = data.tick;
        }
        let hit_ratio =
            (hit_time.saturating_sub(data.tick - hit_last_tick) as f64) / (hit_time as f64);
        let hit_number = hit_last_damage as f64 * hit_ratio;

        for frame in last_frame..frame {
            let tick = (frame as f32) / time_per_tick / 120.0;
            let tick = tick + start as f32;
            let angles = [
                pitches.clamped_sample(tick as f32).unwrap().0,
                yaws.clamped_sample(tick as f32).unwrap().0,
            ];
            let position = Vector {
                x: x_s.clamped_sample(tick as f32).unwrap(),
                y: y_s.clamped_sample(tick as f32).unwrap(),
                z: z_s.clamped_sample(tick as f32).unwrap(),
            };
            let mut delta_angles = match last_angles {
                Some(last_angles) => [angles[0] - last_angles[0], angles[1] - last_angles[1]],
                None => [0.0, 0.0],
            };

            if delta_angles[1] < -180.0 {
                delta_angles[1] += 360.0;
            }
            if delta_angles[1] > 180.0 {
                delta_angles[1] -= 360.0;
            }
            if let Some(uber) = data.uber {
                let uber_out = uber_out.get_or_insert_with(|| {
                    let mut uber_out = fs::File::create(&uber_path).unwrap();
                    writeln!(&mut uber_out, "txt = []").unwrap();
                    uber_out
                });
                writeln!(uber_out, "txt[{}] = \"{}\";", frame, uber)?;
            }
            writeln!(
                &mut ammo_out,
                "txt[{}] = \"{}/{}\";",
                frame, data.ammo, data.max_ammo
            )?;
            writeln!(&mut health_out, "txt[{}] = \"{}\";", frame, data.health)?;
            writeln!(&mut pitch_out, r#"txt[{}] = {};"#, frame, delta_angles[0])?;
            writeln!(&mut yaw_out, r#"txt[{}] = {};"#, frame, delta_angles[1])?;
            writeln!(&mut hit_out, r#"txt[{}] = {};"#, frame, hit_number as u32)?;
            writeln!(&mut weapon_out, r#"txt[{}] = "{}";"#, frame, data.weapon)?;

            #[derive(Serialize)]
            struct CameraOut {
                position: Vector,
                angle: [f32; 3],
            }

            let yaw_matrix = <Matrix3<f32>>::from_angle_y(degrees(-(angles[1] - start_angles[1])));
            let pitch_matrix =
                <Matrix3<f32>>::from_angle_x(degrees(-(angles[0] - start_angles[0])));
            let quat = <Quaternion<f32>>::from(yaw_matrix * pitch_matrix);
            let euler = Euler::from(quat);
            writeln!(
                &mut camera_out,
                r#"txt[{}] = {};"#,
                frame,
                serde_json::to_string(&CameraOut {
                    position: position - start_position,
                    angle: [
                        Deg::from(euler.x).0,
                        Deg::from(euler.y).0,
                        Deg::from(euler.z).0
                    ]
                })
                .unwrap()
            )?;
            ticks_done += 1;
            last_angles = Some(angles);
        }
        last_frame = frame;
    }
    println!("{} frames processed", ticks_done);

    errors.show();
    Ok(())
}

pub struct TickData {
    tick: u32,
    ammo: u16,
    max_ammo: u16,
    health: u16,
    uber: Option<u8>,
    angles: [Option<f32>; 2],
    hit: Option<u32>,
    weapon: String,
    position: Vector,
}

#[derive(Default)]
pub struct AmmoCountAnalyser {
    tick: u32,
    output: Vec<TickData>,
    max_clip: FnvHashMap<EntityId, u16>,
    clip: FnvHashMap<EntityId, u16>,
    current_health: u16,
    class_names: Vec<ServerClassName>,
    local_player_id: EntityId,
    local_user_id: UserId,
    entity_classes: FnvHashMap<EntityId, ClassId>,
    outer_map: FnvHashMap<i64, EntityId>,
    model_names: Vec<String>,
    active_weapon: i64,
    last_tick: u32,
    ammo: [u16; 2],
    max_ammo: [u16; 2],
    uber: u8,
    has_uber: bool,
    angles: [f32; 2],
    errors: Errors,
    hit: Option<u32>,
    pov: EntityId,
    model_indexes: HashMap<EntityId, u32>,
    tick_angles: [Option<f32>; 2],
    angle_delta_tick: [u32; 2],
    loadout: [i64; 2],
    position: Vector,
}

impl MessageHandler for AmmoCountAnalyser {
    type Output = (Vec<TickData>, Errors);

    fn does_handle(_message_type: MessageType) -> bool {
        true
    }

    fn handle_message(&mut self, message: &Message, tick: u32, state: &ParserState) {
        match message {
            Message::ServerInfo(info) => {
                self.pov = (info.player_slot as u32 + 1).into();
            }
            Message::PacketEntities(entities) => {
                for entity in &entities.entities {
                    self.handle_entity(tick, entity, state)
                }
            }
            Message::GameEvent(event_msg) => {
                self.handle_event(&event_msg.event);
            }
            _ => {}
        }
    }

    fn handle_string_entry(
        &mut self,
        table: &str,
        index: usize,
        entry: &StringTableEntry,
        _state: &ParserState,
    ) {
        match table {
            "modelprecache" => {
                let model = entry.text.as_deref().unwrap_or_default();
                let file_name = model.rsplit_once('/').unwrap_or_default().1;
                let name = file_name.split_once('.').unwrap_or_default().0;
                self.model_names.insert(index, name.to_string())
            }
            _ => {}
        }
    }

    fn handle_data_tables(
        &mut self,
        _parse_tables: &[ParseSendTable],
        server_classes: &[ServerClass],
        _state: &ParserState,
    ) {
        self.class_names = server_classes
            .iter()
            .map(|class| &class.name)
            .cloned()
            .collect();
    }

    fn handle_packet_meta(&mut self, tick: u32, meta: &MessagePacketMeta, _state: &ParserState) {
        if self.is_pov() {
            self.angles = [meta.view_angles[0].angles.x, meta.view_angles[0].angles.y];
            self.position = meta.view_angles[0].origin;
            self.tick_angles = [Some(self.angles[0]), Some(self.angles[1])];
        }
        self.tick = tick;
    }

    fn into_output(self, _state: &ParserState) -> Self::Output {
        (self.output, self.errors)
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
const DAMAGE_PROP_LOCAL: SendPropIdentifier =
    SendPropIdentifier::new("DT_TFPlayerScoringDataExclusive", "m_iDamageDone");
#[allow(dead_code)]
const EYE_ANGLES_X: SendPropIdentifier =
    SendPropIdentifier::new("DT_TFNonLocalPlayerExclusive", "m_angEyeAngles[0]");
#[allow(dead_code)]
const EYE_ANGLES_Y: SendPropIdentifier =
    SendPropIdentifier::new("DT_TFNonLocalPlayerExclusive", "m_angEyeAngles[1]");
#[allow(dead_code)]
const EYE_POS_X: SendPropIdentifier =
    SendPropIdentifier::new("DT_TFNonLocalPlayerExclusive", "m_vecOrigin[0]");
#[allow(dead_code)]
const EYE_POS_Y: SendPropIdentifier =
    SendPropIdentifier::new("DT_TFNonLocalPlayerExclusive", "m_vecOrigin[1]");
#[allow(dead_code)]
const EYE_POS_Z: SendPropIdentifier =
    SendPropIdentifier::new("DT_TFNonLocalPlayerExclusive", "m_vecOrigin[2]");

#[allow(dead_code)]
const WEAPON1_ID_PROP: SendPropIdentifier = SendPropIdentifier::new("m_hMyWeapons", "000");
#[allow(dead_code)]
const WEAPON2_ID_PROP: SendPropIdentifier = SendPropIdentifier::new("m_hMyWeapons", "001");
#[allow(dead_code)]
const WEAPON3_ID_PROP: SendPropIdentifier = SendPropIdentifier::new("m_hMyWeapons", "002");

#[allow(dead_code)]
const AMMO1_PROP: SendPropIdentifier = SendPropIdentifier::new("m_iAmmo", "001");
#[allow(dead_code)]
const AMMO2_PROP: SendPropIdentifier = SendPropIdentifier::new("m_iAmmo", "002");

#[allow(dead_code)]
const MODEL_INDEX: SendPropIdentifier =
    SendPropIdentifier::new("DT_BaseCombatWeapon", "m_iWorldModelIndex");

const OUTER_NULL: i64 = 0x1FFFFF;

impl AmmoCountAnalyser {
    pub fn new(local_player_id: EntityId, local_user_id: UserId) -> Self {
        AmmoCountAnalyser {
            local_player_id,
            local_user_id,
            ..Default::default()
        }
    }

    fn is_pov(&self) -> bool {
        self.pov == self.local_player_id
    }

    #[allow(dead_code)]
    fn server_class(&self, id: ClassId) -> &str {
        self.class_names[u16::from(id) as usize].as_str()
    }

    fn handle_event(&mut self, event: &GameEvent) {
        match event {
            GameEvent::PlayerSpawn(spawn) => {
                if UserId::from(spawn.user_id) == self.local_user_id {
                    self.clip = self.max_clip.clone();
                }
            }
            GameEvent::PlayerHurt(event) => {
                if UserId::from(event.attacker) == self.local_user_id {
                    self.hit = Some(event.damage_amount as u32);
                }
            }
            _ => {}
        }
    }

    fn handle_entity(&mut self, tick: u32, entity: &PacketEntity, state: &ParserState) {
        for prop in entity.props(state) {
            match prop.value {
                SendPropValue::Integer(value) if value != OUTER_NULL => {
                    if let Some((table_name, prop_name)) = prop.identifier.names() {
                        if table_name == "m_iChargeLevel" {
                            let entity_id: u32 = prop_name.parse().unwrap();
                            if EntityId::from(entity_id) == self.local_player_id {
                                if value > 0 {
                                    self.has_uber = true;
                                }
                                self.uber = value as u8;
                            }
                        }
                    }
                    match prop.identifier {
                        ACTIVE_WEAPON_PROP if entity.entity_index == self.local_player_id => {
                            self.active_weapon = value;
                        }
                        AMMO1_PROP if entity.entity_index == self.local_player_id => {
                            self.ammo[0] = value as u16;
                            self.max_ammo[0] = self.max_ammo[0].max(value as u16);
                        }
                        AMMO2_PROP if entity.entity_index == self.local_player_id => {
                            self.ammo[1] = value as u16;
                            self.max_ammo[1] = self.max_ammo[1].max(value as u16);
                        }
                        HEALTH_PROP if entity.entity_index == self.local_player_id => {
                            self.current_health = value as u16;
                        }
                        OUTER_CONTAINER_PROP => {
                            self.outer_map.insert(value, entity.entity_index);
                        }
                        CLIP_PROP => {
                            match self.entity_classes.get(&entity.entity_index) {
                                Some(class) if *class != entity.server_class => {
                                    self.max_clip.insert(entity.entity_index, value as u16);
                                }
                                _ => {
                                    let clip_max =
                                        self.max_clip.entry(entity.entity_index).or_default();
                                    *clip_max = (*clip_max).max(value as u16);
                                }
                            }
                            self.clip.insert(entity.entity_index, value as u16);
                        }
                        MODEL_INDEX => {
                            self.model_indexes.insert(entity.entity_index, value as u32);
                        }
                        WEAPON1_ID_PROP if entity.entity_index == self.local_player_id => {
                            if value != self.loadout[0] {
                                self.max_ammo[0] = 0;
                                self.ammo[0] = 0;
                                self.loadout[0] = value;
                            }
                        }
                        WEAPON2_ID_PROP if entity.entity_index == self.local_player_id => {
                            if value != self.loadout[1] {
                                self.max_ammo[1] = 0;
                                self.ammo[1] = 0;
                                self.loadout[1] = value;
                            }
                        }
                        _ => {}
                    }
                }
                SendPropValue::Float(value)
                    if !self.is_pov() && entity.entity_index == self.local_player_id =>
                {
                    match prop.identifier {
                        EYE_ANGLES_X => {
                            self.angles[0] = value;
                            self.tick_angles[0] = Some(value);
                            self.angle_delta_tick[0] = tick;
                        }
                        EYE_ANGLES_Y => {
                            self.angles[1] = value;
                            self.tick_angles[1] = Some(value);
                            self.angle_delta_tick[1] = tick;
                        }
                        EYE_POS_X => {
                            self.position.x = value;
                        }
                        EYE_POS_Y => {
                            self.position.y = value;
                        }
                        EYE_POS_Z => {
                            self.position.z = value;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        self.entity_classes
            .insert(entity.entity_index, entity.server_class);

        if self.tick > self.last_tick {
            let active_slot = if self.active_weapon == self.loadout[0] {
                0
            } else {
                1
            };
            if let Some(active_weapon) = self.outer_map.get(&self.active_weapon) {
                if self.clip.contains_key(active_weapon) {
                    let mut ammo = if self.max_clip[active_weapon] > 0 {
                        self.clip[active_weapon].saturating_sub(1)
                    } else {
                        self.ammo[active_slot]
                    };
                    let mut max_ammo = if self.max_clip[active_weapon] > 0 {
                        self.max_clip[active_weapon].saturating_sub(1)
                    } else {
                        self.max_ammo[active_slot]
                    };

                    let model_index = self
                        .model_indexes
                        .get(active_weapon)
                        .copied()
                        .unwrap_or_default();

                    let weapon = self
                        .model_names
                        .get(model_index as usize)
                        .cloned()
                        .unwrap_or_default();

                    if let Some(max_overwrite) = max_clip_overwrite(&weapon) {
                        max_ammo = max_overwrite;
                    }
                    ammo = ammo.min(max_ammo);

                    self.output.push(TickData {
                        tick: self.tick,
                        ammo,
                        max_ammo,
                        health: self.current_health,
                        uber: self.has_uber.then(|| self.uber),
                        angles: self.tick_angles,
                        hit: self.hit,
                        weapon,
                        position: self.position,
                    });

                    self.hit = None;
                    self.tick_angles = [None, None];
                } else {
                    self.errors.clip_not_found += 1;
                    warn!(
                        tick = self.tick,
                        weapon_handle = self.active_weapon,
                        weapon_id = display(active_weapon),
                        "can't find clip"
                    );
                }
            } else if self.active_weapon > 0 {
                self.errors.weapon_not_found += 1;
                warn!(
                    tick = self.tick,
                    weapon_handle = self.active_weapon,
                    "can't find weapon"
                );
            } else {
                self.errors.no_weapon_set += 1;
            }
            self.last_tick = self.tick;
        }
    }
}

#[derive(Default)]
pub struct Errors {
    no_weapon_set: u32,
    weapon_not_found: u32,
    clip_not_found: u32,
}

impl Errors {
    fn show(&self) {
        if self.no_weapon_set > 0 {
            eprint!("No weapon set {} times", self.no_weapon_set);
        }
        if self.weapon_not_found > 0 {
            eprint!("Weapon not found {} times", self.weapon_not_found);
        }
        if self.clip_not_found > 0 {
            eprint!("Clip not found {} times", self.clip_not_found);
        }
    }
}

fn args_from_name(name: &str) -> (String, u32, u32) {
    let name = name.rsplit_once('/').unwrap_or(("", name)).1;
    let name = name.split_once('.').unwrap_or((name, "")).0;
    let mut parts = name.split('_');
    let name = parts.next().expect("unexpected name format").to_lowercase();
    let tick = parts
        .next()
        .expect("unexpected name format")
        .to_lowercase()
        .parse()
        .expect("unexpected name format");
    (name, tick, tick + 5000)
}

fn max_clip_overwrite(weapon: &str) -> Option<u16> {
    match weapon {
        "c_pep_pistol" => Some(9),
        _ => None,
    }
}

pub const fn degrees(v: f32) -> Deg<f32> {
    Deg(v)
}
