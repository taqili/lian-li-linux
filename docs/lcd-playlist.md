# LCD media playlist and temperature readouts

This guide covers two features added by the
[taqili fork](https://github.com/taqili/lian-li-linux) of
[Lian Li Linux](https://github.com/sgtaziz/lian-li-linux). They are not part of
the upstream project.

## Playlist

Choose **Playlist** as an LCD's media type to cycle through any mix of images
(PNG, JPEG, BMP), GIFs and videos (MP4, WebM, MKV, MOV, AVI, M4V). The
playlist runs in the daemon, so it keeps playing when the app is closed.

- **Add files** opens a picker that accepts several files at once.
- **Add folder** adds the supported files in a folder, sorted by name. Turn on
  **Include subfolders** to scan up to eight levels deep. Hidden files and
  symlinked folders are skipped. The folder is read once when you add it, so
  add it again to pick up new files.
- Items play in list order and loop. **Shuffle** randomises the order on every
  pass without playing the same item twice in a row.
- A playlist holds up to 500 items. Duplicate paths are ignored.

### Timing

| Setting | Default | Effect |
| --- | --- | --- |
| Image duration | 10 s | How long each still image stays on screen |
| Minimum GIF/video time | 0 (off) | Loops a shorter clip until this much time has passed |
| Maximum GIF/video time | 0 (off) | Cuts a longer clip off at this point |

GIFs and videos otherwise play through once before the next item. A playlist
with a single item keeps playing it without reloading. **FPS** caps GIF and
video playback (at most 60).

The next item is decoded in the background while the current one plays, so the
switch is immediate. If an item cannot be played, it is skipped with one
warning in the daemon log and the screen keeps showing the current item.
Every file must exist when the configuration is saved, as with other media
types.

**Fit**, **Fill** and **Stretch** work as for single files. Zoom and dragging
are not available for playlists.

### Resource use

Images only wake the renderer four times a second and are re-sent only when
something changes. GIFs and videos are decoded and encoded live instead of
being prepared in advance, which keeps memory bounded for long clips but costs
more CPU than a single video. Lower **FPS** if the daemon's CPU use matters
more than smoothness.

## Temperature readouts

With **Fit** selected, the playlist can show CPU and GPU temperatures in °C in
the black bars beside or above and below the media. The CPU reading uses the
left or top bar and the GPU reading uses the right or bottom bar. Each can be
turned on separately. Readings refresh every two seconds.

Which bars are used depends on the media and the screen orientation. On the
Lancool 207 Digital in its default portrait orientation (720×1472), widescreen
media leaves bars above and below. Rotated to landscape (1472×720), the same
media leaves 96-pixel bars on the left and right. A bar thinner than 64 pixels
is left empty, and nothing is drawn over the media. The preview shows where
the readouts will appear.

### Sensors

**Automatic** picks:

- **CPU:** the control temperature (`Tctl` on AMD through `k10temp`, `Package
  id 0` on Intel through `coretemp`).
- **GPU:** an NVIDIA GPU if present, otherwise the `edge` temperature of an
  AMD card. On systems with integrated and discrete AMD graphics, the card that
  also reports a `junction` temperature is chosen, which is the discrete one.

Sensors are matched by driver name, PCI address and label, never by `hwmonN`
number. Choose a specific sensor from the list to override the automatic pick.
If a sensor disappears, its readout shows `--°C` until it returns.

## Installing the fork on Arch Linux and CachyOS

The fork's package replaces `lianli-linux` or `lianli-linux-git`. Pacman asks
to remove the installed package; answer yes. Configuration and media in
`~/.config/lianli` are kept.

```bash
git clone https://github.com/taqili/lian-li-linux.git
cd lian-li-linux/packaging/archlinux/fork
makepkg -si
systemctl --user restart lianli-daemon.service
```

Run `git pull` and then `makepkg -si` in the same folder to update.

To return to upstream, first switch every Playlist screen to another media
type and save. Upstream does not recognise the Playlist type and cannot read a
configuration that contains one. Then install `lianli-linux` from the AUR.
