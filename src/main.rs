extern crate rusty_alfred;
extern crate playwith;
extern crate hyper;
extern crate crypto;
#[cfg(feature = "env")]
extern crate dotenv;

use crypto::digest::Digest;
use crypto::sha1::Sha1;
use hyper::Client;
use playwith::{PlayWith, Profile, SharedGame};
use playwith::errors::*;
use rusty_alfred::*;
use std::env::{args, var};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

fn main() {
  #[cfg(feature = "env")]
  { dotenv::dotenv().ok(); }
  let api_key = match var("STEAMAPI_KEY") {
    Ok(k) => k,
    Err(_) => {
      println!("{}", build_error("no api key").unwrap());
      return;
    }
  };
  let args: Vec<String> = args().skip(1).collect();
  if args.is_empty() {
    println!("{}", build_error("no arguments").unwrap());
    return;
  }
  let args: Vec<String> = args[0].split(' ').map(|x| x.to_owned()).collect();
  let profiles: Result<Vec<Profile>> = args.iter()
    .map(|x| {
      Profile::from_username(x)
        .or_else(|_| Profile::from_steam_id(x))
        .map(|x| x.object)
    })
    .collect();
  let mut profiles = match profiles {
    Ok(x) => x,
    Err(e) => {
      println!("{}", build_error(format!("failure to get profiles: {}", build_error_string(e))).unwrap());
      return;
    }
  };
  if let Ok(id) = var("YOUR_STEAM_ID") {
    let profile = match Profile::from_steam_id(&id) {
      Ok(p) => p,
      Err(e) => {
        println!("{}", build_error(format!("failure to create profile from your ID: {}", build_error_string(e))).unwrap());
        return;
      }
    };
    profiles.insert(0, profile.object);
  }
  let play_with = PlayWith::new(api_key);
  let mut shared = match play_with.find_shared_games(&mut profiles) {
    Ok(x) => x,
    Err(e) => {
      println!("{}", build_error(format!("failure to get games: {}", build_error_string(e))).unwrap());
      return;
    }
  };
  shared.sort_by_key(|x| !x.playtime_shared_average);
  match build_game_output(shared) {
    Ok(o) => println!("{}", o),
    Err(e) => println!("{}", build_error(&format!("error creating output: {}", build_error_string(e))).unwrap())
  }
}

trait Plural: CustomPlural {
  fn pluralize(&self, amount: usize) -> String {
    self.custom_pluralize(amount, "s")
  }
}

trait CustomPlural {
  fn custom_pluralize(&self, amount: usize, plural: &str) -> String;
}

impl<T> CustomPlural for T
  where T: AsRef<str>
{
  fn custom_pluralize(&self, amount: usize, plural: &str) -> String {
    let string = self.as_ref().to_owned();
    if amount == 1 {
      string
    } else {
      string + plural
    }
  }
}

impl<T> Plural for T where T: CustomPlural {}

fn minutes_to_pretty_string(mut mins: usize) -> String {
  let strings = vec![(60, "minute"), (24, "hour"), (7, "day"), (52, "week"), (0, "year")]
    .into_iter()
    .map(|(divisor, unit)| {
      let amt = if divisor == 0 {
        mins
      } else {
        let amt = mins % divisor;
        mins /= divisor;
        amt
      };
      if amt == 0 {
        return None;
      }
      Some(format!("{} {}", amt, unit.pluralize(amt)))
    })
    .filter(Option::is_some)
    .collect::<Option<Vec<String>>>()
    .unwrap();
  if strings.is_empty() {
    return "0 minutes".to_owned();
  }
  strings.into_iter().rev().collect::<Vec<_>>().join(", ")
}

fn build_game_output(shared: Vec<SharedGame>) -> Result<String> {
  let mut items = AlfredItems::new();
  if shared.is_empty() {
    items = items.item(AlfredItem::new("No games in common")
      .subtitle("The provided profiles have no games in common. :("));
  }
  for game in shared {
    let image = download_icon(&game)?;
    let pretty_time = minutes_to_pretty_string(game.playtime_shared_average);
    items = items.item(AlfredItem::new(&game.name)
      .subtitle(format!("{} played on average.", pretty_time))
      .arg(format!("steam://nav/games/details/{}", game.appid))
      .item_mods(AlfredItemMods::new()
        .alt(AlfredItemMod::new()
          .subtitle("Action to open store page in Steam")
          .arg(format!("steam://store/{}", game.appid)))
        .cmd(AlfredItemMod::new()
          .subtitle("Action to open store page in browser")
          .arg(format!("https://store.steampowered.com/app/{}/", game.appid))))
      .icon(AlfredItemIcon::new(image)));
  }
  items.to_json().chain_err(|| "could not create output json")
}

fn build_error_string(error: Error) -> String {
  error.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(": ")
}

fn build_error<T>(string: T) -> Result<String>
  where T: AsRef<str>
{
  AlfredItems::new()
    .item(AlfredItem::new("Error")
      .subtitle(string)
      .icon(AlfredItemIcon::new("icon.png")))
    .to_json()
    .chain_err(|| "could not create error json")
}

fn download_icon(game: &SharedGame) -> Result<String> {
  let icons_dir = Path::new("icons");
  if !icons_dir.exists() {
    if let Err(e) = std::fs::create_dir(icons_dir) {
      return Err(format!("could not create icons directory: {}", e).into());
    }
  }
  if icons_dir.exists() && !icons_dir.is_dir() {
    return Err("icons directory was a file".into());
  }
  let file_name = format!("icons/{}_{}.jpg", game.appid, game.img_icon_url);
  if Path::new(&file_name).exists() {
    let mut image_file = File::open(&file_name).chain_err(|| format!("could not open {}", file_name))?;
    let mut image = Vec::new();
    image_file.read_to_end(&mut image).chain_err(|| format!("could not read {}", file_name))?;
    let mut sha1 = Sha1::new();
    sha1.input(&image);
    if sha1.result_str().to_lowercase() == game.img_icon_url.to_lowercase() {
      return Ok(file_name);
    } else {
      std::fs::remove_file(&file_name).chain_err(|| format!("could not delete {}", file_name))?;
    }
  }
  let client = Client::new();
  let mut res = client
    .get(&format!("http://media.steampowered.com/steamcommunity/public/images/apps/{appid}/{hash}.jpg", appid=game.appid, hash=game.img_icon_url))
    .send()
    .chain_err(|| "could not send icon download request")?;
  let mut file = File::create(&file_name).chain_err(|| format!("could not create file {}", file_name))?;
  let mut image = Vec::new();
  res.read_to_end(&mut image).chain_err(|| "could not read icon download")?;
  file.write_all(&image).chain_err(|| format!("could not write {}", file_name))?;
  Ok(file_name)
}
