use derive_more::From;
use ffmpeg4::{format, DictionaryRef};
use log::{debug, info, trace, warn};
use path_slash::PathExt;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use regex::{Regex, RegexSet};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, StripPrefixError},
    rc::Rc,
    time::SystemTime,
};

#[derive(Debug)]
pub struct Index {
    artists: HashMap<String, Artist>,
    albums: HashMap<String, Rc<RefCell<Album>>>,
}

#[derive(Debug)]
pub struct Artist {
    name: String,
    unique_name: String,
    albums: HashMap<String, Rc<RefCell<Album>>>,
    cover_url: Option<String>,
}

#[derive(Debug)]
pub struct Album {
    name: String,
    unique_name: String,
    artists: Vec<String>,
    artist_unique_names: Vec<String>,
    songs: Vec<Option<Rc<RefCell<Song>>>>,
    songs_by_name: HashMap<String, Rc<RefCell<Song>>>,
    cover_url: Option<String>,
    tracked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Song {
    title: String,
    unique_name: String,
    album: String,
    artists: Vec<String>,
    track: Option<u32>,
    url: String,
}

lazy_static::lazy_static! {
static ref TRACK_INFO_TRACK_PATTERN: Regex = Regex::new("(?P<track>\\d+)(/\\d+)?").unwrap();
static ref FILENAME_STRIP_SUFFIX: Regex = Regex::new("(?P<name>.+)\\.[^.]+$").unwrap();
static ref UNIQUE_NAME_ILLEGAL: Regex = Regex::new("[^a-zA-Z0-9]").unwrap();
static ref ARTIST_SPLIT_PATTERN: Regex = Regex::new("( +& +| *, +)").unwrap();
static ref PATH_SET: AsciiSet = NON_ALPHANUMERIC.remove(b'/').remove(b'-').remove(b'_').remove(b'.').remove(b'+');
}

impl Song {
    fn parse<P1: AsRef<Path>, P2: AsRef<Path>, S: AsRef<str>>(
        path: P1,
        base: P2,
        files_url: S,
    ) -> Result<Song, IndexingError> {
        let stripped = path.as_ref().strip_prefix(base)?;
        let url = format!(
            "{}/{}",
            files_url.as_ref(),
            utf8_percent_encode(&stripped.to_slash_lossy(), &PATH_SET)
        );

        let context = format::input(&path)?;

        trace!("Format Metadata:");
        let metadata = context.metadata();
        for (key, value) in metadata.iter() {
            trace!("  '{}': '{}'", key, value);
        }
        let mut title = Song::find_title(&metadata);
        let mut album = Song::find_album(&metadata);
        let mut artist = Song::find_artist(&metadata);
        let mut track = Song::find_track(&metadata);

        for (index, stream) in context.streams().enumerate() {
            if !(title.is_none() || album.is_none() || artist.is_none() || track.is_none()) {
                break;
            }

            trace!("Stream {} Metadata:", index);
            let metadata = stream.metadata();
            for (key, value) in metadata.iter() {
                trace!("  '{}': '{}'", key, value);
            }
            title = title.or_else(|| Song::find_title(&metadata));
            album = album.or_else(|| Song::find_album(&metadata));
            artist = artist.or_else(|| Song::find_artist(&metadata));
            track = track.or_else(|| Song::find_track(&metadata));
        }

        if title.is_none() {
            title = path
                .as_ref()
                .file_name()
                .map(|n| n.to_string_lossy())
                .and_then(|n| {
                    FILENAME_STRIP_SUFFIX
                        .captures(&n)
                        .and_then(|c| c.name("name"))
                        .map(|m| m.as_str().to_string())
                })
        }

        let title = title.unwrap_or("Unknown".to_string());

        Ok(Song {
            unique_name: UNIQUE_NAME_ILLEGAL
                .replace_all(&title, "-")
                .to_ascii_lowercase(),
            title,
            album: album.unwrap_or("Unknown".to_string()),
            // TODO: multi-artist stuff
            artists: ARTIST_SPLIT_PATTERN
                .split(&artist.unwrap_or("Unknown".to_string()))
                .map(|s| s.to_string())
                .collect(),
            track,
            // TODO: URL stuff
            url,
        })
    }

    fn find_title(dict: &DictionaryRef) -> Option<String> {
        dict.get("title")
            .or_else(|| dict.get("TITLE"))
            .map(|s| s.to_string())
    }

    fn find_album(dict: &DictionaryRef) -> Option<String> {
        dict.get("album")
            .or_else(|| dict.get("ALBUM"))
            .map(|s| s.to_string())
    }

    fn find_artist(dict: &DictionaryRef) -> Option<String> {
        dict.get("artist")
            .or_else(|| dict.get("ARTIST"))
            .map(|s| s.to_string())
    }

    fn find_track(dict: &DictionaryRef) -> Option<u32> {
        dict.get("track")
            .or_else(|| dict.get("TRACK"))
            .and_then(|track_str| TRACK_INFO_TRACK_PATTERN.captures(track_str))
            .and_then(|captures| captures.name("track"))
            .and_then(|track_str| track_str.as_str().parse().ok().filter(|t| *t != 0))
    }
}

impl Index {
    pub fn index<P: AsRef<Path>, S: AsRef<str>>(
        base_dir: P,
        base_url: S,
        media_include: &RegexSet,
        media_exclude: &RegexSet,
        cover_include: &RegexSet,
        cover_exclude: &RegexSet,
    ) -> Result<Index, IndexingError> {
        info!("Indexing {}", base_dir.as_ref().to_str().unwrap());
        let start_time = SystemTime::now();

        let mut index = Index {
            artists: Default::default(),
            albums: Default::default(),
        };

        let mut song_count = 0u32;

        for dir in walkdir::WalkDir::new(&base_dir).follow_links(true) {
            match dir {
                Ok(dir) => {
                    let path = dir.path();
                    let path_str = path.to_string_lossy();
                    trace!("Visiting {}", path_str);

                    if media_include.is_match(&path_str) && !media_exclude.is_match(&path_str) {
                        trace!("Found media file.");

                        let song = Song::parse(&path, &base_dir, &base_url)?;
                        debug!("Loaded metadata: {:?}", &song);
                        index.insert_song(song);
                        song_count += 1;
                    }
                }
                Err(err) => {
                    warn!(
                        "Encountered invalid file while scanning. Path: {}",
                        if let Some(entry) = err.path() {
                            entry.to_string_lossy()
                        } else {
                            "Unknown".into()
                        }
                    );
                }
            }
        }

        info!(
            "Indexed {} songs in {:?}",
            song_count,
            SystemTime::now().duration_since(start_time).unwrap()
        );

        info!("{} Albums loaded.", index.albums.len());

        let mut albums = index.albums.keys().collect::<Vec<_>>();
        albums.sort();
        trace!("Albums: {:?}", albums);

        info!("{} Artists loaded.", index.artists.len());

        let mut artists = index.artists.keys().collect::<Vec<_>>();
        artists.sort();
        trace!("Artists: {:?}", artists);

        Ok(index)
    }

    fn insert_song(&mut self, song: Song) {
        let album = self.get_or_insert_album(&song.album, &song.artists);
        let mut album_borrowed = album.borrow_mut();

        if let Some(track) = song.track {
            album_borrowed.tracked = true;

            if album_borrowed.songs.len() <= (track - 1) as usize {
                album_borrowed.songs.resize(track as usize, None);
            }

            let song_name = song.unique_name.clone();
            let song = Rc::new(RefCell::new(song));

            album_borrowed.songs[(track - 1) as usize] = Some(song.clone());
            album_borrowed.songs_by_name.insert(song_name, song);
        } else {
            let song_name = song.unique_name.clone();
            let song = Rc::new(RefCell::new(song));

            album_borrowed.songs.push(Some(song.clone()));
            album_borrowed.songs_by_name.insert(song_name, song);
        }
    }

    fn get_or_insert_album(&mut self, name: &str, artists: &[String]) -> Rc<RefCell<Album>> {
        let mut unique_name = UNIQUE_NAME_ILLEGAL
            .replace_all(name, "-")
            .to_ascii_lowercase();

        if self.albums.contains_key(&unique_name) {
            let found = self
                .albums
                .get(&unique_name)
                .expect("BUG: Missing found artist")
                .clone();
            let mut borrowed = found.borrow_mut();
            if borrowed.name == name {
                for artist_name in artists {
                    if !borrowed.artists.contains(artist_name) {
                        borrowed.artists.push(artist_name.clone());
                        borrowed
                            .artist_unique_names
                            .push(self.get_or_insert_artist(artist_name));
                    }
                }

                drop(borrowed);
                return found;
            }

            let mut index = 1u32;
            let mut found_name = format!("{}-{}", unique_name, index);
            while self.albums.contains_key(&found_name) {
                let found = self
                    .albums
                    .get(&found_name)
                    .expect("BUG: Missing found artist")
                    .clone();
                let mut borrowed = found.borrow_mut();
                if borrowed.name == name {
                    for artist_name in artists {
                        if !borrowed.artists.contains(artist_name) {
                            borrowed.artists.push(artist_name.clone());
                            borrowed
                                .artist_unique_names
                                .push(self.get_or_insert_artist(artist_name));
                        }
                    }

                    drop(borrowed);
                    return found;
                }

                index += 1;
                found_name = format!("{}-{}", unique_name, index);
            }
            unique_name = found_name;
        }

        let artist_unique_names: Vec<String> = artists
            .iter()
            .map(|artist| self.get_or_insert_artist(artist))
            .collect();

        let album = Rc::new(RefCell::new(Album {
            name: name.to_string(),
            unique_name: unique_name.clone(),
            artists: artists.to_vec(),
            artist_unique_names: artist_unique_names.clone(),
            songs: Default::default(),
            songs_by_name: Default::default(),
            cover_url: None,
            tracked: false,
        }));

        self.albums.insert(unique_name.clone(), album.clone());
        for artist_unique_name in artist_unique_names {
            self.artists
                .get_mut(&artist_unique_name)
                .expect("BUG: Missing newly inserted artist")
                .albums
                .insert(unique_name.clone(), album.clone());
        }

        album
    }

    fn get_or_insert_artist(&mut self, name: &str) -> String {
        let mut unique_name = UNIQUE_NAME_ILLEGAL
            .replace_all(name, "-")
            .to_ascii_lowercase();

        if self.artists.contains_key(&unique_name) {
            let mut found = self
                .artists
                .get(&unique_name)
                .expect("BUG: Missing found artist");
            if found.name == name {
                return found.unique_name.clone();
            }

            let mut index = 1u32;
            let mut found_name = format!("{}-{}", unique_name, index);
            while self.artists.contains_key(&found_name) {
                found = self
                    .artists
                    .get(&found_name)
                    .expect("BUG: Missing found artist");
                if found.name == name {
                    return found.unique_name.clone();
                }

                index += 1;
                found_name = format!("{}-{}", unique_name, index);
            }
            unique_name = found_name;
        }

        // we couldn't find the artist, so we'll insert a new one
        self.artists.insert(
            unique_name.clone(),
            Artist {
                name: name.to_string(),
                unique_name: unique_name.clone(),
                albums: Default::default(),
                cover_url: None,
            },
        );

        unique_name
    }
}

#[derive(Debug, From)]
pub enum IndexingError {
    WalkdirError(walkdir::Error),
    FfmpegError(ffmpeg4::Error),
    StripPrefixError(StripPrefixError),
}
