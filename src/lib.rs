#![feature(plugin)]
#![feature(proc_macro)]

#![plugin(clippy)]

extern crate serde;
extern crate serde_json;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate error_chain;
extern crate hyper;
extern crate xmltree;
extern crate time;

#[cfg(test)]
mod tests;
pub mod errors;

use errors::*;
use hyper::{Client, Url};
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use xmltree::Element;

const STEAMAPI_URL: &'static str = "https://api.steampowered.com/";

#[derive(Debug)]
pub struct SteamApi {
  key: String
}

impl SteamApi {
  pub fn new<T>(key: T) -> Self
    where T: AsRef<str>
  {
    let key = key.as_ref().to_owned();
    SteamApi {
      key: key
    }
  }

  fn create_api_url(&self, service: &str, method: &str, params: Option<HashMap<&str, &str>>) -> Result<Url> {
    let mut url = Url::parse(STEAMAPI_URL).chain_err(|| "could not create base api url")?;
    {
      let mut path = match url.path_segments_mut() {
        Ok(p) => p,
        Err(_) => return Err("invalid base api url".into())
      };
      path.push(service);
      path.push(method);
      path.push("v0001");
    }
    {
      let mut pairs = url.query_pairs_mut();
      pairs.append_pair("format", "json");
      pairs.append_pair("key", &self.key);
      if let Some(p) = params {
        for (k, v) in p {
          pairs.append_pair(k, v);
        }
      }
    }
    Ok(url)
  }

  pub fn get_games(&self, profile: &mut Profile) -> Result<GamesResponse> {
    if let Some(ref games) = profile.games {
      return Ok(games.response.clone());
    }
    let mut params = HashMap::new();
    params.insert("steamid", profile.ids.steamid64.as_str());
    params.insert("include_appinfo", "1");
    params.insert("include_played_free_games", "1");
    let url = self.create_api_url("IPlayerService", "GetOwnedGames", Some(params))?;
    let client = Client::new();
    let res = client.get(url).send().chain_err(|| "failure to contact Steam API")?;
    if res.status != hyper::Ok {
      return Err("Steam API did not return a 200 OK".into());
    }
    let games: SteamResponse = serde_json::from_reader(res).chain_err(|| "could not parse Steam's JSON")?;
    profile.games = Some(games.clone());
    profile.save().chain_err(|| "error saving games")?;
    Ok(games.response)
  }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Ids {
  pub steamid64: String,
  pub custom_url: Option<String>,
}

impl Ids {
  pub fn new(steamid64: String, custom_url: Option<String>) -> Self {
    Ids {
      steamid64: steamid64,
      custom_url: custom_url
    }
  }
}

pub type TimestampedProfile = Timestamped<Profile>;

#[derive(Debug, Serialize, Deserialize)]
pub struct Profile {
  pub ids: Ids,
  pub games: Option<SteamResponse>
}

impl Profile {
  pub fn new(ids: Ids, games: Option<SteamResponse>) -> Self {
    Profile {
      ids: ids,
      games: games
    }
  }

  fn ensure_update(profile: TimestampedProfile, ids: Option<Ids>) -> Result<TimestampedProfile> {
    if profile.timestamp + 3600 >= time::now_utc().to_timespec().sec {
      Ok(profile)
    } else {
      let ids = match ids {
        Some(i) => i,
        None => {
          let id = profile.object.ids.steamid64;
          let url = format!("http://steamcommunity.com/profiles/{}/?xml=1", id);
          Profile::download_ids(&url, &id)?
        }
      };
      Ok(Timestamped::of(Profile::new(ids, None)))
    }
  }

  pub fn from_steam_id(steam_id: &str) -> Result<TimestampedProfile> {
    let path = Profile::get_path(steam_id);
    if !path.exists() {
      return Profile::ensure_update(
        Timestamped::of_time(time::now_utc().to_timespec().sec - 3601,
          Profile::new(
            Ids::new(steam_id.to_owned(), None),
            None
          )
        ),
        None
      )
    }
    let profile = Profile::from_file(&Profile::ensure_path(path)?)?;
    Profile::ensure_update(profile, None)
  }

  fn download_ids(url: &str, identifier: &str) -> Result<Ids> {
    let client = Client::new();
    let response = client.get(url).send().chain_err(|| "failure to contact Steam Community")?;
    let element = Element::parse(response).chain_err(|| "could not parse Steam Community XML")?;
    let id = element.get_child("steamID64").and_then(|e| e.text.clone());
    let id = match id {
      Some(id) => id,
      None => return Err(format!("could not find {}", identifier).into())
    };
    let custom_url = element.get_child("customURL").and_then(|e| e.text.clone());
    let custom_url = match custom_url {
      Some(x) => if x.is_empty() { None } else { Some(x) },
      None => None
    };
    Ok(Ids::new(id, custom_url))
  }

  pub fn from_username(username: &str) -> Result<TimestampedProfile> {
    let url = format!("http://steamcommunity.com/id/{}/?xml=1", username);
    let ids = Profile::download_ids(&url, username)?;
    let mut profile = Profile::from_steam_id(&ids.steamid64)?;
    if profile.object.ids.custom_url.is_none() {
      profile.object.ids.custom_url = ids.custom_url.clone();
      profile.object.save()?;
    }
    Profile::ensure_update(profile, Some(ids))
  }

  pub fn from_file(file: &File) -> Result<TimestampedProfile> {
    serde_json::from_reader(file).chain_err(|| "could not read profile file")
  }

  pub fn get_path(steam_id: &str) -> PathBuf {
    let mut file_path = Path::new("profiles").join(steam_id);
    file_path.set_extension("json");
    file_path
  }

  pub fn get_own_path(&self) -> PathBuf {
    Profile::get_path(&self.ids.steamid64)
  }

  fn ensure_path<T>(path: T) -> Result<File>
    where T: AsRef<Path>
  {
    let file_path = path.as_ref();
    if !file_path.exists() {
      if let Some(parent) = file_path.parent() {
        if !parent.exists() {
          std::fs::create_dir(&parent).chain_err(|| "could not create parent dir")?;
        }
        if parent.exists() && !parent.is_dir() {
          return Err("parent directory existed but was not a file".into());
        }
      }
    }
    OpenOptions::new()
      .read(true)
      .write(true)
      .create(true)
      .open(&file_path)
      .chain_err(|| "could not open or create profile file")
  }

  pub fn save(&self) -> Result<()> {
    let mut file = Profile::ensure_path(self.get_own_path())?;
    let json = serde_json::to_string(&Timestamped::of(self)).chain_err(|| "could not encode profile to json")?;
    file.write_all(json.as_bytes()).chain_err(|| "could not write profile to file")?;
    Ok(())
  }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Timestamped<T> {
  pub timestamp: i64,
  pub object: T
}

impl<T> Timestamped<T> {
  pub fn of(object: T) -> Timestamped<T> {
    Timestamped {
      timestamp: time::now_utc().to_timespec().sec,
      object: object
    }
  }

  pub fn of_time(timestamp: i64, object: T) -> Timestamped<T> {
    Timestamped {
      timestamp: timestamp,
      object: object
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteamResponse {
  pub response: GamesResponse
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GamesResponse {
  pub game_count: usize,
  pub games: Vec<Game>
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Game {
  pub name: String,
  pub appid: usize,
  pub playtime_forever: usize,
  pub playtime_2weeks: Option<usize>,
  pub img_logo_url: String,
  pub img_icon_url: String
}

pub struct SharedGame {
  pub name: String,
  pub appid: usize,
  pub playtime_shared_average: usize,
  pub img_logo_url: String,
  pub img_icon_url: String,
}

pub struct PlayWith {
  pub api: SteamApi
}

impl PlayWith {
  pub fn new<T>(api_key: T) -> PlayWith
    where T: AsRef<str>
  {
    PlayWith {
      api: SteamApi::new(api_key)
    }
  }

  pub fn find_shared_games(&self, profiles: &mut [Profile]) -> Result<Vec<SharedGame>> {
    let games: Vec<Vec<Game>> = profiles.iter_mut()
      .map(|profile| self.api.get_games(profile).map(|g| g.games))
      .collect::<Result<Vec<_>>>()?;
    let app_ids: HashSet<usize> = games.iter()
      .map(|gs| gs.iter().map(|g| g.appid).collect::<HashSet<_>>())
      .fold(HashSet::new(), |acc, gs| {
        if acc.is_empty() {
          gs
        } else {
          acc.intersection(&gs).cloned().collect()
        }
      });
    let all_games: Vec<Game> = games.into_iter()
      .flat_map(|g| g)
      .collect();
    Ok(app_ids.into_iter()
      .map(|id| {
        let games: Vec<&Game> = all_games.iter().filter(|g| g.appid == id).collect();
        let game = games[0].clone();
        SharedGame {
          name: game.name,
          appid: game.appid,
          playtime_shared_average: games.iter().map(|g| g.playtime_forever).sum::<usize>() / games.len(),
          img_logo_url: game.img_logo_url,
          img_icon_url: game.img_icon_url
        }
      })
      .collect())
  }
}
