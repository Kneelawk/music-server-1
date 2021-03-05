/// Encapsulates all API responses coming from the server.
export interface ResponseResult<T> {
  Ok: T | null;
  Err: string | null;
}

/// Describes an album returned by the server.
export interface AlbumJson {
  name: string;
  unique_name: string;
  artists: string[],
  artist_unique_names: string[];
  songs: Array<string | null>;
  cover_url: string | null;
  tracked: boolean;
}
