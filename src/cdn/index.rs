use crate::error::{ErrorKind, Result, ResultExt};
use actix_service::ServiceFactory;
use actix_web::{
    body::MessageBody,
    dev::{HttpResponseBuilder, ServiceRequest, ServiceResponse},
    http::StatusCode,
    web, App, HttpResponse,
};
use ffmpeg4::{format, DictionaryRef};
use futures::{stream, StreamExt};
use path_slash::PathExt;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use regex::{Regex, RegexSet};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path, sync::Arc, time::SystemTime};
use tokio::sync::RwLock;

#[derive(Debug)]
pub struct Index {
    artists: HashMap<String, Arc<RwLock<Artist>>>,
    artist_list: Vec<Arc<RwLock<Artist>>>,
    albums: HashMap<String, Arc<RwLock<Album>>>,
    album_list: Vec<Arc<RwLock<Album>>>,
}

#[derive(Debug)]
pub struct Artist {
    name: String,
    unique_name: String,
    albums: HashMap<String, Arc<RwLock<Album>>>,
    cover_url: Option<String>,
}

#[derive(Debug)]
pub struct Album {
    name: String,
    unique_name: String,
    artists: Vec<String>,
    artist_unique_names: Vec<String>,
    songs: Vec<Option<Arc<RwLock<Song>>>>,
    songs_by_name: HashMap<String, Arc<RwLock<Song>>>,
    cover_url: Option<String>,
    tracked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Song {
    title: String,
    unique_name: String,
    album: String,
    album_unique_name: String,
    artists: Vec<String>,
    artist_unique_names: Vec<String>,
    track: Option<u32>,
    url: String,
}

lazy_static::lazy_static! {
static ref TRACK_INFO_TRACK_PATTERN: Regex = Regex::new("(?P<track>\\d+)(/\\d+)?").unwrap();
static ref FILENAME_STRIP_SUFFIX: Regex = Regex::new("(?P<name>.+)\\.[^.]+$").unwrap();
static ref ARTIST_SPLIT_PATTERN: Regex = Regex::new("( +& +| *, +)").unwrap();
static ref PATH_SET: AsciiSet = NON_ALPHANUMERIC.remove(b'/').remove(b'-').remove(b'_').remove(b'.').remove(b'+');
}

impl Song {
    fn parse<P1: AsRef<Path>, P2: AsRef<Path>, S: AsRef<str>>(
        path: P1,
        base: P2,
        files_url: S,
    ) -> Result<Song> {
        let stripped = path.as_ref().strip_prefix(base).chain_err(|| {
            ErrorKind::IndexingError(Some(path.as_ref().to_string_lossy().to_string()))
        })?;
        let url = format!(
            "{}/{}",
            files_url.as_ref(),
            utf8_percent_encode(&stripped.to_slash_lossy(), &PATH_SET)
        );

        let context = format::input(&path).chain_err(|| {
            ErrorKind::IndexingError(Some(path.as_ref().to_string_lossy().to_string()))
        })?;

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
            unique_name: sanitize(&title),
            title,
            album: album.unwrap_or("Unknown".to_string()),
            album_unique_name: "".to_string(),
            artists: ARTIST_SPLIT_PATTERN
                .split(&artist.unwrap_or("Unknown".to_string()))
                .map(|s| s.to_string())
                .collect(),
            artist_unique_names: Default::default(),
            track,
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
    pub async fn index<P: AsRef<Path>, S: AsRef<str>>(
        base_dir: P,
        base_url: S,
        media_include: &RegexSet,
        media_exclude: &RegexSet,
        cover_include: &RegexSet,
        cover_exclude: &RegexSet,
    ) -> Result<Index> {
        info!("Indexing {}", base_dir.as_ref().to_string_lossy());
        let start_time = SystemTime::now();

        let mut index = Index {
            artists: Default::default(),
            artist_list: Default::default(),
            albums: Default::default(),
            album_list: Default::default(),
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
                        index.insert_song(song).await;
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

        debug!("Sorting artists...");
        index.artist_list.reserve(index.artists.len());
        let mut artist_names = index.artists.keys().collect::<Vec<_>>();
        artist_names.sort();
        for artist_name in artist_names {
            index.artist_list.push(index.artists[artist_name].clone());
        }

        debug!("Sorting albums...");
        index.album_list.reserve(index.albums.len());
        let mut album_names = index.albums.keys().collect::<Vec<_>>();
        album_names.sort();
        for album_name in album_names {
            index.album_list.push(index.albums[album_name].clone());
        }

        info!(
            "Indexed {} songs in {:?}",
            song_count,
            SystemTime::now().duration_since(start_time).unwrap()
        );

        info!("{} Albums loaded.", index.albums.len());

        debug!(
            "Albums: {:?}",
            &stream::iter(&index.album_list)
                .then(|rc| async move { rc.read().await.unique_name.clone() })
                .collect::<Vec<_>>()
                .await
        );

        info!("{} Artists loaded.", index.artists.len());

        debug!(
            "Artists: {:?}",
            &stream::iter(&index.artist_list)
                .then(|rc| async move { rc.read().await.unique_name.clone() })
                .collect::<Vec<_>>()
                .await
        );

        Ok(index)
    }

    async fn insert_song(&mut self, mut song: Song) {
        let album = self.get_or_insert_album(&song.album, &song.artists).await;

        if let Some(track) = song.track {
            album.write().await.tracked = true;

            if album.read().await.songs.len() <= (track - 1) as usize {
                album.write().await.songs.resize(track as usize, None);
            }

            song.album_unique_name = album.read().await.unique_name.clone();

            let mut artist_unique_names = vec![];
            for artist in song.artists.iter() {
                artist_unique_names.push(self.get_or_insert_artist(artist).await);
            }
            song.artist_unique_names = artist_unique_names;

            let song_name = song.unique_name.clone();
            let song = Arc::new(RwLock::new(song));

            album.write().await.songs[(track - 1) as usize] = Some(song.clone());
            album.write().await.songs_by_name.insert(song_name, song);
        } else {
            song.album_unique_name = album.read().await.unique_name.clone();

            let mut artist_unique_names = vec![];
            for artist in song.artists.iter() {
                artist_unique_names.push(self.get_or_insert_artist(artist).await);
            }
            song.artist_unique_names = artist_unique_names;

            let song_name = song.unique_name.clone();
            let song = Arc::new(RwLock::new(song));

            album.write().await.songs.push(Some(song.clone()));
            album.write().await.songs_by_name.insert(song_name, song);
        }
    }

    async fn get_or_insert_album(&mut self, name: &str, artists: &[String]) -> Arc<RwLock<Album>> {
        let mut unique_name = sanitize(name);

        if self.albums.contains_key(&unique_name) {
            let found = self
                .albums
                .get(&unique_name)
                .expect("BUG: Missing found artist")
                .clone();
            if found.read().await.name == name {
                for artist_name in artists {
                    if !found.read().await.artists.contains(artist_name) {
                        let artist_unique_name = self.get_or_insert_artist(artist_name).await;

                        found.write().await.artists.push(artist_name.clone());
                        found
                            .write()
                            .await
                            .artist_unique_names
                            .push(artist_unique_name.clone());
                        self.artists[&artist_unique_name]
                            .write()
                            .await
                            .albums
                            .insert(unique_name.clone(), found.clone());
                    }
                }

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
                if found.read().await.name == name {
                    for artist_name in artists {
                        if !found.read().await.artists.contains(artist_name) {
                            let artist_unique_name = self.get_or_insert_artist(artist_name).await;

                            found.write().await.artists.push(artist_name.clone());
                            found
                                .write()
                                .await
                                .artist_unique_names
                                .push(artist_unique_name.clone());
                            self.artists[&artist_unique_name]
                                .write()
                                .await
                                .albums
                                .insert(found_name.clone(), found.clone());
                        }
                    }

                    return found;
                }

                index += 1;
                found_name = format!("{}-{}", unique_name, index);
            }
            unique_name = found_name;
        }

        let mut artist_unique_names = vec![];
        for artist in artists.iter() {
            artist_unique_names.push(self.get_or_insert_artist(artist).await);
        }

        let album = Arc::new(RwLock::new(Album {
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
                .write()
                .await
                .albums
                .insert(unique_name.clone(), album.clone());
        }

        album
    }

    async fn get_or_insert_artist(&mut self, name: &str) -> String {
        let mut unique_name = sanitize(name);

        if self.artists.contains_key(&unique_name) {
            let found = self
                .artists
                .get(&unique_name)
                .expect("BUG: Missing found artist");
            let borrowed = found.read().await;
            if borrowed.name == name {
                return borrowed.unique_name.clone();
            }

            let mut index = 1u32;
            let mut found_name = format!("{}-{}", unique_name, index);
            while self.artists.contains_key(&found_name) {
                let found = self
                    .artists
                    .get(&found_name)
                    .expect("BUG: Missing found artist");
                let borrowed = found.read().await;
                if borrowed.name == name {
                    return borrowed.unique_name.clone();
                }

                index += 1;
                found_name = format!("{}-{}", unique_name, index);
            }
            unique_name = found_name;
        }

        let artist = Arc::new(RwLock::new(Artist {
            name: name.to_string(),
            unique_name: unique_name.clone(),
            albums: Default::default(),
            cover_url: None,
        }));

        // we couldn't find the artist, so we'll insert a new one
        self.artists.insert(unique_name.clone(), artist.clone());

        unique_name
    }
}

fn sanitize(s: &str) -> String {
    s.replace(
        |c: char| !((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9')),
        "-",
    )
    .to_ascii_lowercase()
}

pub fn apply_services<T, B>(app: App<T, B>) -> App<T, B>
where
    B: MessageBody,
    T: ServiceFactory<
        Config = (),
        Request = ServiceRequest,
        Response = ServiceResponse<B>,
        Error = actix_web::error::Error,
        InitError = (),
    >,
{
    app.service(get_albums)
        .service(get_artists)
        .service(get_album)
        .service(get_artist)
        .service(get_song)
}

#[get("/cdn/index/albums")]
async fn get_albums(index: web::Data<Index>) -> HttpResponse {
    let mut albums = vec![];
    for album in index.album_list.iter() {
        let album = album.read().await;
        albums.push(AlbumJson::from_album(&album).await);
    }

    HttpResponseBuilder::new(StatusCode::OK).json(albums)
}

#[get("/cdn/index/artists")]
async fn get_artists(index: web::Data<Index>) -> HttpResponse {
    let mut artists = vec![];
    for artist in index.artist_list.iter() {
        let artist = artist.read().await;
        artists.push(ArtistJson::from_artist(&artist).await);
    }

    HttpResponseBuilder::new(StatusCode::OK).json(artists)
}

#[get("/cdn/index/album/{album_name}")]
async fn get_album(
    index: web::Data<Index>,
    web::Path(album_name): web::Path<String>,
) -> Result<HttpResponse> {
    if let Some(album) = index.albums.get(&album_name) {
        let album = album.read().await;

        Ok(HttpResponseBuilder::new(StatusCode::OK).json(AlbumJson::from_album(&album).await))
    } else {
        bail!(ErrorKind::NoSuchResource);
    }
}

#[get("/cdn/index/artist/{artist_name}")]
async fn get_artist(
    index: web::Data<Index>,
    web::Path(artist_name): web::Path<String>,
) -> Result<HttpResponse> {
    if let Some(artist) = index.artists.get(&artist_name) {
        let artist = artist.read().await;

        Ok(HttpResponseBuilder::new(StatusCode::OK).json(ArtistJson::from_artist(&artist).await))
    } else {
        bail!(ErrorKind::NoSuchResource);
    }
}

#[get("/cdn/index/album/{album_name}/{song_name}")]
async fn get_song(
    index: web::Data<Index>,
    web::Path((album_name, song_name)): web::Path<(String, String)>,
) -> Result<HttpResponse> {
    if let Some(album) = index.albums.get(&album_name) {
        let album = album.read().await;
        if let Some(song) = album.songs_by_name.get(&song_name) {
            Ok(HttpResponseBuilder::new(StatusCode::OK).json(song.read().await.clone()))
        } else {
            bail!(ErrorKind::NoSuchResource)
        }
    } else {
        bail!(ErrorKind::NoSuchResource);
    }
}

#[derive(Serialize)]
struct AlbumJson {
    name: String,
    unique_name: String,
    artists: Vec<String>,
    artist_unique_names: Vec<String>,
    songs: Vec<Option<String>>,
    cover_url: Option<String>,
    tracked: bool,
}

impl AlbumJson {
    async fn from_album(album: &Album) -> AlbumJson {
        AlbumJson {
            name: album.name.clone(),
            unique_name: album.unique_name.clone(),
            artists: album.artists.clone(),
            artist_unique_names: album.artist_unique_names.clone(),
            songs: stream::iter(&album.songs)
                .then(|song| async move {
                    if let Some(song) = song {
                        Some(song.read().await.unique_name.clone())
                    } else {
                        None
                    }
                })
                .collect()
                .await,
            cover_url: album.cover_url.clone(),
            tracked: album.tracked,
        }
    }
}

#[derive(Serialize)]
struct ArtistJson {
    name: String,
    unique_name: String,
    albums: Vec<String>,
    cover_url: Option<String>,
}

impl ArtistJson {
    async fn from_artist(artist: &Artist) -> ArtistJson {
        ArtistJson {
            name: artist.name.clone(),
            unique_name: artist.unique_name.clone(),
            albums: artist.albums.keys().cloned().collect(),
            cover_url: artist.cover_url.clone(),
        }
    }
}
