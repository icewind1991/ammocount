use std::convert::TryFrom;
use steamid_ng::SteamID;
use tf_demo_parser::demo::data::UserInfo;
use tf_demo_parser::demo::message::packetentities::EntityId;
use tf_demo_parser::demo::message::Message;
use tf_demo_parser::demo::packet::stringtable::StringTableEntry;
use tf_demo_parser::demo::parser::analyser::UserId;
use tf_demo_parser::demo::parser::MessageHandler;
use tf_demo_parser::{Demo, DemoParser, MessageType, ParserState};

pub fn get_player(demo: &Demo, user: Option<String>) -> (EntityId, UserId) {
    let parser = DemoParser::new_with_analyser(demo.get_stream(), PlayerSearchHandler::new(user));

    parser
        .parse()
        .expect("failed to parse demo")
        .1
        .expect("no server info or player not found")
}

enum PlayerFilter {
    Name(String),
    SteamId(SteamID),
}

impl PlayerFilter {
    fn new(filter: String) -> Self {
        match SteamID::try_from(filter.as_str()) {
            Ok(steam_id) => PlayerFilter::SteamId(steam_id),
            Err(_) => PlayerFilter::Name(filter),
        }
    }

    fn matches(&self, info: &UserInfo) -> bool {
        match self {
            PlayerFilter::Name(name) => info.player_info.name.to_ascii_lowercase().contains(name),
            PlayerFilter::SteamId(steam_id) => {
                SteamID::try_from(info.player_info.steam_id.as_str()).ok() == Some(*steam_id)
            }
        }
    }
}
struct PlayerSearchHandler {
    filter: Option<PlayerFilter>,
    entity: Option<EntityId>,
    user: Option<UserId>,
}

impl PlayerSearchHandler {
    pub fn new(user: Option<String>) -> Self {
        PlayerSearchHandler {
            filter: user.map(PlayerFilter::new),
            entity: None,
            user: None,
        }
    }
}

impl MessageHandler for PlayerSearchHandler {
    type Output = Option<(EntityId, UserId)>;

    fn does_handle(_message_type: MessageType) -> bool {
        true
    }

    fn handle_message(&mut self, message: &Message, _tick: u32, _parser_state: &ParserState) {
        if self.filter.is_none() {
            if let Message::ServerInfo(info) = message {
                self.entity = Some(EntityId::from(info.player_slot as u32 + 1));
            }
        }
    }

    fn handle_string_entry(
        &mut self,
        table: &str,
        _index: usize,
        entry: &StringTableEntry,
        _parser_state: &ParserState,
    ) {
        if table == "userinfo" {
            if let Ok(Some(info)) = UserInfo::parse_from_string_table(
                entry.text.as_deref(),
                entry.extra_data.as_ref().map(|data| data.data.clone()),
            ) {
                if let Some(filter) = self.filter.as_ref() {
                    if filter.matches(&info) && self.entity.is_none() {
                        println!(
                            "Found {} as entity {}, user {}",
                            info.player_info.name,
                            info.entity_id,
                            u8::from(info.player_info.user_id)
                        );
                        self.entity = Some(info.entity_id);
                        self.user = Some(info.player_info.user_id);
                    }
                } else {
                    if Some(info.entity_id) == self.entity && self.filter.is_none() {
                        self.user = Some(info.player_info.user_id);
                    }
                }
            }
        }
    }

    fn into_output(self, _state: &ParserState) -> Self::Output {
        Some((self.entity?, self.user?))
    }
}
