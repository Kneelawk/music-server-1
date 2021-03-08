import { Injectable } from '@angular/core';
import { HttpClient } from "@angular/common/http";
import { environment } from "../environments/environment";
import { AlbumJson, ResponseResult } from "./index.types";

@Injectable({
  providedIn: 'root'
})
export class IndexService {
  private albumsUrl = '/cdn/index/albums';
  private baseUrl = environment.serve ? 'http://localhost:8980' : '';

  constructor(private client: HttpClient) { }

  getAlbums() {
    return this.client.get<ResponseResult<AlbumJson[]>>(this.url(this.albumsUrl));
  }

  url(path: string) {
    return this.baseUrl + path
  }
}
