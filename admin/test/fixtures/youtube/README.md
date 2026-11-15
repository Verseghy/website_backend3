# YouTube input fixtures

What YouTube's Share dialog, its share links and the player's menu write,
plus the forms YouTube's developer and Help pages print. They are the inputs
of the YoutubeEmbed node's reader (`admin/src/rich-text/youtube-input.ts`)
and of the editor's golden files `tests/fixtures/e1/embed-youtube-*.html`
(editor.md §4.3 row 9, §13.3, §13.6; ADR 0043). They come from the research
note on YouTube embeds of 2026-10-25 (research: youtube-embed §0, §10.4),
which recorded them in desktop Chrome, signed out.

Each file is the exact text that was copied or printed, with no trailing
newline. A `.html` file is embed code, which the tests feed both as HTML
(parse rule 60) and as plain text (the insert dialog's code route, D128); a
`.txt` file is one URL, which the tests feed to the lone-URL paste rule and
the dialog. The video and playlist ids are those of public videos and a
public playlist of YouTube's own channel and an official artist channel, and
the sample ids of YouTube's pages (research: youtube-embed §10.4); none is
the school's.

## Where each file comes from, and its licence

`THIRD_PARTY.md` at the repository root lists these files (ADR 0012 §4,
ADR 0043 §6, D138).

**Output of YouTube, captured on 2026-10-25.** The markup is YouTube's, not
this project's, so it is not under the repository licence, and no licence
is claimed for it (editor.md §13.1). Modified: every live `si` value (a
per-share tracking token) was replaced with an invented token
`invented_si-000N`; nothing else changed.

| File | What YouTube wrote |
|---|---|
| `youtube-embed-dialog-default.html` | Share > Embed, every option at its default |
| `youtube-embed-dialog-start.html` | Share > Embed with "Start at" 1:30 (`start=90`; `&` written as `&amp;`) |
| `youtube-embed-dialog-controls-off.html` | Share > Embed with "Show player controls" off (`controls=0`) |
| `youtube-embed-dialog-privacy.html` | Share > Embed in privacy-enhanced mode (the `youtube-nocookie.com` host) |
| `youtube-embed-dialog-all-options.html` | Share > Embed with start, controls off and privacy-enhanced mode (`si`, `controls`, `start` in that order) |
| `youtube-embed-dialog-short.html` | Share > Embed of a Short on its watch page: the same shape and size as a video |
| `youtube-embed-dialog-playlist.html` | Share > Embed of a playlist: `/embed/videoseries?list=` |
| `youtube-embed-dialog-playlist-all-options.html` | the playlist's Share > Embed with controls off and privacy-enhanced mode |
| `youtube-embed-context-menu-video.html` | the player menu's "Copy embed code": the player's size, the video's real title, no `si`, no start (unchanged) |
| `youtube-embed-context-menu-playlist.html` | "Copy embed code" in a playlist: `/embed/<id>?list=` (unchanged) |
| `youtube-share-link-video.txt` | the Share panel's link |
| `youtube-share-link-video-start.txt` | the Share panel's link with "Start at" 1:30: `t=90`, no `s` |
| `youtube-share-link-short.txt` | the Shorts player's Share sheet: the bare host `youtube.com` |
| `youtube-share-link-playlist.txt` | a playlist page's Share link: the bare host and `/playlist` |
| `youtube-context-menu-url-time.txt` | the player menu's "Copy video URL at current time" (`t=97`; unchanged) |
| `youtube-context-menu-url-playlist-time.txt` | the same in a playlist (`list` and `t=8`; unchanged) |
| `youtube-watch-url-playlist.txt` | a playlist page's "Play all" link, with the undocumented `pp` (unchanged) |
| `youtube-shorts-url.txt` | the Shorts player's own address (unchanged) |

**Code samples of YouTube's developer documentation**, copied from
developers.google.com on 2026-10-25, whose code samples are licensed under
the Apache License 2.0 (`Apache-2.0`; the licence text is
`LICENSE.apache-2.0` here). Unmodified.

| File | Sample |
|---|---|
| `youtube-dev-sample-autoplay-origin.html` | "YouTube Embedded Players and Player Parameters", "Embed a player using an `<iframe>` tag": `id`, `type`, `autoplay=1`, `origin` and a literal `&` |
| `youtube-documented-embed-listtype-playlist.txt` | the same page, "Loading a playlist": `/embed?listType=playlist&list=PL…`, the one playlist form the documentation gives |

**Constructed by this project**, under the repository licence:

| File | What it pins |
|---|---|
| `youtube-help-start-http.html` | an `http` embed address with `start=90`, in the 420 × 315 size of the start-time example of YouTube Help's "Embed videos & playlists" (D136) |
| `youtube-help-privacy-before.html` | the older shape of YouTube's code that legacy pages still carry: `allow="autoplay; encrypted-media"`, no `title`, no `referrerpolicy`, over two lines |
| `youtube-help-autoplay.html` | a hand-edited paste: an empty first query part (`?&autoplay=1`) and no space after the `src` value |
| `youtube-documented-embed-loop.txt` | `playlist=<id>&loop=1`, as the developer documentation describes the `loop` parameter |
| `youtube-documented-embed-start-end.txt` | `start` and `end` together |
| `youtube-documented-embed-deprecated-params.txt` | parameters that the documentation marks deprecated or ignored (`rel=0`, `showinfo`, `modestbranding`, `autohide`, `theme`, `controls=2`) |
| `youtube-watch-url-mix.txt` | the shape of a Mix link (`list=RD…`, `start_radio`, `rv`) |
| `youtube-watch-url-time.txt` | a watch address typed by hand with `t=90s`, which YouTube honours but no YouTube control was seen writing |

The three `youtube-help-*` files are written by this project after the
shapes the Help article shows, with the article's own video ids; they are
not copies of the article.
