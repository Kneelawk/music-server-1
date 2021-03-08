import { Component, OnInit, ViewEncapsulation } from '@angular/core';
import { BreakpointObserver, Breakpoints } from '@angular/cdk/layout';
import { Observable } from 'rxjs';
import { map, shareReplay } from 'rxjs/operators';
import { IndexService } from "../index.service";
import { AlbumJson } from "../index.types";

@Component({
  selector: 'app-navigation',
  templateUrl: './navigation.component.html',
  styleUrls: ['./navigation.component.styl'],
  encapsulation: ViewEncapsulation.None
})
export class NavigationComponent implements OnInit {

  isHandset$: Observable<boolean> = this.breakpointObserver.observe(Breakpoints.Handset)
    .pipe(
      map(result => result.matches),
      shareReplay()
    );

  albums$: Observable<AlbumJson[]> = this.index.getAlbums().pipe(
    map(result => result.Ok ?? [])
  );

  constructor(private breakpointObserver: BreakpointObserver, private index: IndexService) {}

  ngOnInit(): void {
    console.log("Albums: " + this.albums$);
  }

  url(url: string): string {
    return this.index.url(url)
  }

  getAlbumArtists(album: AlbumJson): string {
    let artists = album.artists.slice(0, 2).map(artist => artist.name).join(", ")
    if (album.artists.length > 2) {
      artists += ", ..."
    }
    return artists
  }
}
