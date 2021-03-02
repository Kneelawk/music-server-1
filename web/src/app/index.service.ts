import { Injectable } from '@angular/core';
import { HttpClient } from "@angular/common/http";

@Injectable({
  providedIn: 'root'
})
export class IndexService {
  private albumsUrl = '/cdn/index/albums';

  constructor(private client: HttpClient) { }
}
