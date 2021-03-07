/// Encapsulates all API responses coming from the server.
export interface ResponseResult<T> {
  Ok: T | null;
  Err: string | null;
}

/// Describes an album returned by the server.
export interface AlbumJson {
  name: string;
  unique_name: string;
  artists: ArtistRef[];
  songs: Array<SongRef | null>;
  cover_url: string | null;
  tracked: boolean;
}

/// Describes a reference to an artist.
export interface ArtistRef {
  name: string;
  unique_name: string;
}

/// Describes a reference to a song.
export interface SongRef {
  name: string;
  unique_name: string;
}
