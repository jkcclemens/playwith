extern crate dotenv;

use {SteamApi, PlayWith};
use std::sync::{Once, ONCE_INIT};
use std::env;

static START: Once = ONCE_INIT;

fn init() {
  START.call_once(|| {
    dotenv::dotenv().ok();
  });
}

fn get_key() -> String {
  env::var("STEAMAPI_KEY").unwrap()
}

#[test]
fn test_username_to_id() {
  init();
  let expected = "76561198054973203";
  let steam_api = SteamApi::new(get_key());
  let steam_id = steam_api.get_steamid_from_username("jkcclemens").unwrap();
  assert_eq!(expected, steam_id);
}

#[test]
fn kek() {
  init();
  let ids = vec!["76561198054973203", "76561198004382761", "76561198011447878"];
  let play_with = PlayWith::new(get_key());
  let mut games = play_with.find_shared_games(&ids).unwrap();
  games.sort_by_key(|g| !g.playtime_shared_average);
  let string = games.iter()
    .map(|g| format!("{}: {} minutes on average", g.name, g.playtime_shared_average))
    .collect::<Vec<_>>()
    .join("\n");
  println!("{}", string);
}
