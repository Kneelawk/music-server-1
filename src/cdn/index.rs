use crate::{
    error::{ErrorKind, Result, ResultExt},
    util::w_ok,
};
use actix_web::{dev::HttpResponseBuilder, http::StatusCode, web, HttpResponse, Scope};
use ffmpeg4::{format, DictionaryRef};
use futures::{stream, StreamExt};
use path_slash::PathExt;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use regex::{Regex, RegexSet};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};
use tokio::sync::RwLock;

macro_rules! indexing_error {
    ($path:expr, $desc:expr) => {
        || ErrorKind::IndexingError(Some($path.as_ref().to_string_lossy().to_string()), $desc)
    };
}

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
    artists: Vec<ArtistRef>,
    songs: Vec<Option<Arc<RwLock<Song>>>>,
    songs_by_name: HashMap<String, Arc<RwLock<Song>>>,
    cover_url: Option<String>,
    cover_rating: u32,
    tracked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Song {
    name: String,
    unique_name: String,
    album: AlbumRef,
    artists: Vec<ArtistRef>,
    track: Option<u32>,
    cover_url: Option<String>,
    url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtistRef {
    name: String,
    unique_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumRef {
    name: String,
    unique_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SongRef {
    name: String,
    unique_name: String,
}

lazy_static::lazy_static! {
static ref TRACK_INFO_TRACK_PATTERN: Regex = Regex::new("(?P<track>\\d+)(/\\d+)?").unwrap();
static ref FILENAME_STRIP_SUFFIX: Regex = Regex::new("(?P<name>.+)\\.[^.]+$").unwrap();
static ref ARTIST_SPLIT_PATTERN: Regex = Regex::new("( +& +| *, +)").unwrap();
static ref PATH_SET: AsciiSet = NON_ALPHANUMERIC.remove(b'/').remove(b'-').remove(b'_').remove(b'.').remove(b'+');
}

impl Song {
    async fn parse<P1: AsRef<Path>, P2: AsRef<Path>, S: AsRef<str>>(
        path: &P1,
        base: &P2,
        files_url: &S,
    ) -> Result<Song> {
        let url = find_url(&path, base, files_url)?;

        // TODO: make this async... probably with blocking()
        let context =
            format::input(&path).chain_err(indexing_error!(path, "probing media file"))?;

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
            name: title,
            album: AlbumRef {
                name: album.unwrap_or("Unknown".to_string()),
                unique_name: "".to_string(),
            },
            artists: ARTIST_SPLIT_PATTERN
                .split(&artist.unwrap_or("Unknown".to_string()))
                .map(|s| ArtistRef {
                    name: s.to_string(),
                    unique_name: "".to_string(),
                })
                .collect(),
            track,
            cover_url: None,
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
        let mut previous_parent = None;
        let mut previous_song_parent = None;
        let mut previous_album = None;
        let mut found_covers: Vec<PathBuf> = vec![];

        for dir in walkdir::WalkDir::new(&base_dir).follow_links(true) {
            match dir {
                Ok(dir) => {
                    let path = dir.path();
                    let path_str = path.to_string_lossy();
                    trace!("Visiting {}", path_str);

                    // don't keep covers from other directories
                    if !paths_eq(&previous_parent, &path.parent()) {
                        found_covers.clear();
                    }
                    previous_parent = path.parent().map(|p| p.to_path_buf());

                    if media_include.is_match(&path_str) && !media_exclude.is_match(&path_str) {
                        trace!("Found media file.");

                        let song = Song::parse(&path, &base_dir, &base_url).await?;
                        debug!("Loaded metadata: {:?}", &song);
                        let song = index.insert_song(song).await;
                        song_count += 1;

                        previous_song_parent = path.parent().map(|p| p.to_path_buf());
                        previous_album = Some(song.read().await.album.unique_name.clone());

                        // if we found the cover first, then this vec should be populated
                        for cover in found_covers.drain(..) {
                            let album = song.read().await.album.unique_name.clone();
                            trace!(
                                "Inserting cover into new album: {}: {}",
                                &album,
                                cover.to_string_lossy()
                            );
                            index
                                .insert_cover(&cover, &base_dir, &base_url, &album)
                                .await?;
                        }
                    } else if cover_include.is_match(&path_str)
                        && !cover_exclude.is_match(&path_str)
                    {
                        trace!("Found cover file.");

                        // if we found songs first, then the album should have been created already
                        if paths_eq(&previous_song_parent, &path.parent()) {
                            let previous_album = previous_album.as_ref().expect(
                                "BUG: previous_song_parent was Some but previous_album was None",
                            );
                            trace!("Editing existing album: {}: {}", previous_album, &path_str);
                            index
                                .insert_cover(&path, &base_dir, &base_url, previous_album)
                                .await?;
                        } else {
                            // we haven't found any songs for this album yet
                            trace!("Found cover: {} for new album.", &path_str);
                            found_covers.push(path.to_path_buf());
                        }
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

    async fn insert_song(&mut self, mut song: Song) -> Arc<RwLock<Song>> {
        for artist in song.artists.iter_mut() {
            artist.unique_name = self.get_or_insert_artist(&artist.name).await;
        }

        let album = self
            .get_or_insert_album(&song.album.name, &song.artists)
            .await;

        song.album.unique_name = album.read().await.unique_name.clone();

        if let Some(cover_url) = &album.read().await.cover_url {
            song.cover_url = Some(cover_url.clone());
        }

        if let Some(track) = song.track {
            album.write().await.tracked = true;

            if album.read().await.songs.len() <= (track - 1) as usize {
                album.write().await.songs.resize(track as usize, None);
            }

            let song_name = song.unique_name.clone();
            let song = Arc::new(RwLock::new(song));

            album.write().await.songs[(track - 1) as usize] = Some(song.clone());
            album
                .write()
                .await
                .songs_by_name
                .insert(song_name, song.clone());

            song
        } else {
            let song_name = song.unique_name.clone();
            let song = Arc::new(RwLock::new(song));

            album.write().await.songs.push(Some(song.clone()));
            album
                .write()
                .await
                .songs_by_name
                .insert(song_name, song.clone());

            song
        }
    }

    async fn get_or_insert_album(
        &mut self,
        name: &str,
        artists: &[ArtistRef],
    ) -> Arc<RwLock<Album>> {
        let mut unique_name = sanitize(name);

        if self.albums.contains_key(&unique_name) {
            let found = self
                .albums
                .get(&unique_name)
                .expect("BUG: Missing found artist")
                .clone();
            if found.read().await.name == name {
                for artist_ref in artists {
                    if found
                        .read()
                        .await
                        .artists
                        .iter()
                        .find(|a| a.unique_name == artist_ref.unique_name)
                        .is_none()
                    {
                        found.write().await.artists.push(artist_ref.clone());
                        self.artists[&artist_ref.unique_name]
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
                    for artist_ref in artists {
                        if found
                            .read()
                            .await
                            .artists
                            .iter()
                            .find(|a| a.unique_name == artist_ref.unique_name)
                            .is_none()
                        {
                            found.write().await.artists.push(artist_ref.clone());
                            self.artists[&artist_ref.unique_name]
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

        let album = Arc::new(RwLock::new(Album {
            name: name.to_string(),
            unique_name: unique_name.clone(),
            artists: artists.to_vec(),
            songs: Default::default(),
            songs_by_name: Default::default(),
            cover_url: None,
            cover_rating: 0,
            tracked: false,
        }));

        self.albums.insert(unique_name.clone(), album.clone());
        for artist_ref in artists {
            self.artists
                .get_mut(&artist_ref.unique_name)
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

    async fn insert_cover<P1: AsRef<Path>, P2: AsRef<Path>, S1: AsRef<str>, S2: AsRef<str>>(
        &mut self,
        path: &P1,
        base: &P2,
        files_url: &S1,
        album_unique_name: &S2,
    ) -> Result<()> {
        if self.albums.contains_key(album_unique_name.as_ref()) {
            let mut album = self.albums[album_unique_name.as_ref()].write().await;
            let rating = Index::rate_cover(&path)?;
            if rating > album.cover_rating {
                let cover_url = Some(find_url(&path, &base, &files_url)?);
                album.cover_url = cover_url.clone();
                album.cover_rating = rating;

                // update all songs for the current album
                for song in album.songs.iter() {
                    if let Some(song) = song {
                        song.write().await.cover_url = cover_url.clone();
                    }
                }
            }
        } else {
            warn!(
                "Attempted to insert a cover for a nonexistent album: {}",
                album_unique_name.as_ref()
            );
        }

        Ok(())
    }

    fn rate_cover<P: AsRef<Path>>(path: &P) -> Result<u32> {
        let file_name = path
            .as_ref()
            .file_name()
            .chain_err(indexing_error!(path, "rating cover"))?;
        let name = file_name.to_string_lossy();
        let mut value = 0u32;

        if name.contains("cover") {
            value += 100;
        }

        if name.contains("small") {
            value += 20;
        }

        Ok(value)
    }
}

fn sanitize(s: &str) -> String {
    s.replace(
        |c: char| !((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9')),
        "-",
    )
    .to_ascii_lowercase()
}

fn find_url<P1: AsRef<Path>, P2: AsRef<Path>, S: AsRef<str>>(
    path: &P1,
    base: P2,
    files_url: &S,
) -> Result<String> {
    let stripped = path
        .as_ref()
        .strip_prefix(base)
        .chain_err(indexing_error!(path, "formatting url"))?;
    Ok(format!(
        "{}/{}",
        files_url.as_ref(),
        utf8_percent_encode(&stripped.to_slash_lossy(), &PATH_SET)
    ))
}

fn paths_eq(p1: &Option<PathBuf>, p2: &Option<&Path>) -> bool {
    if p1.is_none() {
        return p2.is_none();
    }
    if p2.is_none() {
        return false;
    }

    p1.as_ref().unwrap() == p2.unwrap()
}

pub fn apply_services() -> Scope {
    web::scope("/index")
        .service(get_albums)
        .service(get_artists)
        .service(get_album)
        .service(get_artist)
        .service(get_song)
}

#[get("/albums")]
async fn get_albums(index: web::Data<Index>) -> HttpResponse {
    let mut albums = vec![];
    for album in index.album_list.iter() {
        let album = album.read().await;
        albums.push(AlbumJson::from_album(&album).await);
    }

    HttpResponseBuilder::new(StatusCode::OK).json(w_ok(albums))
}

#[get("/artists")]
async fn get_artists(index: web::Data<Index>) -> HttpResponse {
    let mut artists = vec![];
    for artist in index.artist_list.iter() {
        let artist = artist.read().await;
        artists.push(ArtistJson::from_artist(&artist).await);
    }

    HttpResponseBuilder::new(StatusCode::OK).json(w_ok(artists))
}

#[get("/album/{album_name}")]
async fn get_album(
    index: web::Data<Index>,
    web::Path(album_name): web::Path<String>,
) -> Result<HttpResponse> {
    if let Some(album) = index.albums.get(&album_name) {
        let album = album.read().await;

        Ok(
            HttpResponseBuilder::new(StatusCode::OK)
                .json(w_ok(AlbumJson::from_album(&album).await)),
        )
    } else {
        bail!(ErrorKind::NoSuchResource);
    }
}

#[get("/artist/{artist_name}")]
async fn get_artist(
    index: web::Data<Index>,
    web::Path(artist_name): web::Path<String>,
) -> Result<HttpResponse> {
    if let Some(artist) = index.artists.get(&artist_name) {
        let artist = artist.read().await;

        Ok(HttpResponseBuilder::new(StatusCode::OK)
            .json(w_ok(ArtistJson::from_artist(&artist).await)))
    } else {
        bail!(ErrorKind::NoSuchResource);
    }
}

#[get("/album/{album_name}/{song_name}")]
async fn get_song(
    index: web::Data<Index>,
    web::Path((album_name, song_name)): web::Path<(String, String)>,
) -> Result<HttpResponse> {
    if let Some(album) = index.albums.get(&album_name) {
        let album = album.read().await;
        if let Some(song) = album.songs_by_name.get(&song_name) {
            Ok(HttpResponseBuilder::new(StatusCode::OK).json(w_ok(song.read().await.clone())))
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
    artists: Vec<ArtistRef>,
    songs: Vec<Option<SongRef>>,
    cover_url: Option<String>,
    tracked: bool,
}

impl AlbumJson {
    async fn from_album(album: &Album) -> AlbumJson {
        AlbumJson {
            name: album.name.clone(),
            unique_name: album.unique_name.clone(),
            artists: album.artists.clone(),
            songs: stream::iter(&album.songs)
                .then(|song| async move {
                    if let Some(song) = song {
                        let song = song.read().await;
                        Some(SongRef {
                            name: song.name.clone(),
                            unique_name: song.unique_name.clone(),
                        })
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
    albums: Vec<AlbumRef>,
    cover_url: Option<String>,
}

impl ArtistJson {
    async fn from_artist(artist: &Artist) -> ArtistJson {
        let mut albums = vec![];

        for album in artist.albums.values() {
            let album = album.read().await;
            albums.push(AlbumRef {
                name: album.name.clone(),
                unique_name: album.unique_name.clone(),
            })
        }

        ArtistJson {
            name: artist.name.clone(),
            unique_name: artist.unique_name.clone(),
            albums,
            cover_url: artist.cover_url.clone(),
        }
    }
}
