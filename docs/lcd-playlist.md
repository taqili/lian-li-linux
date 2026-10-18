# LCD media playlist and readouts

This guide covers features added by the
[taqili fork](https://github.com/taqili/lian-li-linux) of
[Lian Li Linux](https://github.com/sgtaziz/lian-li-linux). They are not part of
the upstream project.

## Playlist

Choose **Playlist** as an LCD's media type to cycle through any mix of images
(PNG, JPEG, BMP), GIFs and videos (MP4, WebM, MKV, MOV, AVI, M4V). The
playlist runs in the daemon, so it keeps playing when the app is closed.

- **Add files** opens a picker that accepts several files at once.
- **Add folder** with **Keep folder in sync** turned on follows the folder: it
  is checked every minute, and new or removed files take effect from the next
  pass through the playlist. Turn **Keep folder in sync** off to add the
  folder's current files once instead. **Include subfolders** scans up to
  eight levels deep. Hidden files and symlinked folders are skipped.
- Items play in list order and loop. **Shuffle** randomises the order on every
  pass without playing the same item twice in a row.
- A playlist holds up to 200 files, counting the alternate playlist, and
  follows up to eight folders. This keeps several playlists and saved profiles
  within the app's limit on referenced media files. Duplicate paths are
  ignored.

Followed folders are read by the daemon. If the daemon runs in system mode, the
folder must be readable by the daemon's account, and folder contents are not
copied by managed media import.

### Timing

| Setting | Default | Effect |
| --- | --- | --- |
| Image duration | 10 s | How long each still image stays on screen |
| Minimum GIF/video time | 0 (off) | Loops a shorter clip until this much time has passed |
| Maximum GIF/video time | 0 (off) | Cuts a longer clip off at this point |
| Crossfade between items | Off | Blends each item into the next over 100–3000 ms (600 ms by default) |

GIFs and videos otherwise play through once before the next item. A playlist
with a single item keeps playing it without reloading. **FPS** caps GIF and
video playback (at most 60).

### Item settings

The sliders button on an item overrides the playlist's settings for that item:

- **Show for** sets an image's own duration.
- **Times to play** plays a GIF or video several times before moving on.
- **Start at** and **End at** trim a video to the part between them.
- **Fit** gives the item its own Fit, Fill or Stretch mode.

### Playback controls

Once the playlist is saved and the screen is connected, the playlist editor
shows **Previous**, **Pause**/**Resume** and **Next** buttons and the file on
screen. The same commands are available from a terminal, which lets you bind
them to keyboard shortcuts in your desktop environment:

```bash
lianli-control playlist next
lianli-control playlist previous
lianli-control playlist toggle          # pause or resume
lianli-control playlist status
lianli-control playlist next --device <device-id>
```

Without `--device`, the command applies to every screen showing a playlist.
**Previous** goes back through up to 16 recently shown items.

### Loading and failures

The next item starts loading in the background a few seconds before the
current image ends, or when a GIF or video finishes, and the current item stays
on screen until it is ready. An item that is missing or cannot be played is
skipped, with one warning in the daemon log, and retried after 30 seconds,
then progressively less often up to every 10 minutes. The playlist only fails
as a whole when none of its first eight items can be played.

**Fit**, **Fill** and **Stretch** work as for single files. Zoom and dragging
are not available for playlists.

### Resource use

Playlists of images only wake the renderer four times a second, or 15 times a
second when crossfades are on, and frames are re-sent only when
something changes. GIFs and videos are decoded and encoded live instead of
being prepared in advance, which keeps memory bounded for long clips but costs
more CPU than a single video. Short clips that fit in memory are encoded once
per loop and reused until a readout changes, unless **H.264** is selected,
which re-encodes every frame. Lower **FPS** if the daemon's CPU
use matters more than smoothness.

## Readouts

With **Fit** selected, the playlist can show two readouts in the black bars
beside or above and below the media: the first in the left or top bar, the
second in the right or bottom bar. Each readout can show:

| Readout | Shows |
| --- | --- |
| CPU temperature | The CPU control temperature |
| GPU temperature | The GPU edge temperature |
| GPU hotspot temperature | The AMD GPU junction temperature |
| CPU usage, GPU usage | Utilisation in percent |
| Clock | Local time |
| Other sensor | Any listed sensor, such as a fan speed |

Readings refresh every two seconds. Each readout can have its own label and a
small graph of the last minute. Temperatures can be shown in °F, and turn
orange and then red at the temperatures you set (75 °C and 85 °C by default).
The readouts can use any installed font. The playlist editor shows the current
values and which sensor each readout reads.

Which bars are used depends on the media and the screen orientation. On the
Lancool 207 Digital in its default portrait orientation (720×1472), widescreen
media leaves bars above and below. Rotated to landscape (1472×720), the same
media leaves 96-pixel bars on the left and right. A bar thinner than 64 pixels
is left empty, and nothing is drawn over the media. Turn on **Always keep room
for readouts** to shrink media that would otherwise fill the screen, so bars at
least as wide as the chosen size always exist. The preview shows where the
readouts will appear.

### Sensors

**Automatic** picks:

- **CPU:** the control temperature (`Tctl` on AMD through `k10temp`, `Package
  id 0` on Intel through `coretemp`).
- **GPU:** an NVIDIA GPU if present, otherwise the `edge` temperature of an
  AMD card. On systems with integrated and discrete AMD graphics, the card that
  also reports a `junction` temperature is chosen, which is the discrete one.
  The hotspot readout uses that card's `junction` temperature.

Sensors are matched by driver name, PCI address and label, never by `hwmonN`
number. Choose a specific sensor from the list to override the automatic pick.
If a sensor disappears, its readout shows `--` until it returns.

## Schedule

### Alternate playlist

An alternate playlist takes over while any listed program runs, or during a
daily time window, and the main playlist resumes afterwards. It can play its
own items or show only the readouts, enlarged to fill half of the screen each.

Programs are matched by process name or executable file name, ignoring case,
for example `steam` or `Cyberpunk2077.exe`. Running programs are checked every
five seconds, so a switch can take up to about ten seconds after a program
starts or quits. Games that only run on another operating system, such as on a
separate Windows boot, cannot be detected.

### Dimming

Dimming darkens the picture sent to the screen to the chosen percentage during
a daily time window. The screen's own brightness setting is not changed. Time
windows may cross midnight, such as 23:00 to 07:00.

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

If you run the daemon in system mode, restart `lianli-daemon-system.service`
with `sudo systemctl restart` instead. Restart the Lian Li Linux app as well so
it picks up the new interface.

Run `git pull` and then `makepkg -si` in the same folder to update.

To return to upstream, first switch every Playlist screen to another media
type and save. Upstream does not recognise the Playlist type and cannot read a
configuration that contains one. Then install `lianli-linux` from the AUR.
