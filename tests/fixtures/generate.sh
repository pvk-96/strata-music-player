#!/usr/bin/env bash
# Regenerate the test fixtures in this directory.
#
# The fixtures are committed so the test suite does not need ffmpeg at run time.
# Every file is about a tenth of a second of silence, apart from the artwork image.
#
# Requires ffmpeg (any recent version).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
out="$here"

tone() { printf 'sine=frequency=440:duration=0.15' ; }

# Stereo, so channel counts in the fixtures match real music.


ff() { ffmpeg -hide_banner -loglevel error -y "$@"; }

# --- MP3: full metadata, Vorbis comment style -----------------------------------------
ff -f lavfi -i "$(tone)" -ac 2 -c:a libmp3lame -b:a 64k \
  -metadata title="Blue Monday" \
  -metadata artist="New Order" \
  -metadata album="Power, Corruption & Lies" \
  -metadata album_artist="New Order" \
  -metadata genre="Electronic" \
  -metadata date="1983" \
  -metadata track="3" \
  -metadata disc="1" \
  "$out/complete.mp3"

# --- MP3 without any tags: the scanner must fall back to the filename -----------------
# ffmpeg always writes an ID3 header, so the tag is stripped afterwards.
ff -f lavfi -i "$(tone)" -ac 2 -c:a libmp3lame -b:a 64k "$out/tagged.mp3"
python3 - "$out" <<'STRIP'
import sys, pathlib
out = pathlib.Path(sys.argv[1])
data = (out / "tagged.mp3").read_bytes()
if data[:3] == b"ID3":
    size = (data[6] << 21) | (data[7] << 14) | (data[8] << 7) | data[9]
    data = data[10 + size:]
(out / "no_tags.mp3").write_bytes(data)
# The same bytes under a name with a space, so the filename fallback is exercised on a path
# that looks like a real one (tests/library.rs).
(out / "no tags.mp3").write_bytes(data)
(out / "tagged.mp3").unlink()
STRIP

# --- FLAC with embedded artwork -------------------------------------------------------
ff -f lavfi -i "$(tone)" -ac 2 -i "$out/cover.png" -c:a flac \
  -map 0:a -map 1:v -c:v copy -disposition:v attached_pic \
  -metadata title="Teardrop" -metadata artist="Massive Attack" \
  -metadata album="Mezzanine" -metadata album_artist="Massive Attack" \
  -metadata genre="Trip Hop" -metadata date="1998" \
  -metadata track="2" -metadata disc="1" \
  "$out/complete.flac"

# --- FLAC without embedded artwork, used for the filesystem fallback -------------------
ff -f lavfi -i "$(tone)" -ac 2 -c:a flac \
  -metadata title="Untitled" -metadata artist="Fixture" -metadata album="No Cover" \
  -metadata album_artist="Fixture" -metadata track="1" \
  "$out/no_cover.flac"

# --- Multi-disc album members ---------------------------------------------------------
for disc in 1 2; do
  ff -f lavfi -i "$(tone)" -ac 2 -c:a flac \
    -metadata title="Disc ${disc} Track 2" -metadata artist="Multi" \
    -metadata album="Multi Disc" -metadata album_artist="Multi" \
    -metadata track="2" -metadata disc="${disc}" \
    "$out/disc${disc}-track2.flac"
  ff -f lavfi -i "$(tone)" -ac 2 -c:a flac \
    -metadata title="Disc ${disc} Track 1" -metadata artist="Multi" \
    -metadata album="Multi Disc" -metadata album_artist="Multi" \
    -metadata track="1" -metadata disc="${disc}" \
    "$out/disc${disc}-track1.flac"
done

# --- Remaining target formats ----------------------------------------------------------
ff -f lavfi -i "$(tone)" -ac 2 -c:a flac -metadata title="Flac Only" \
   -metadata artist="Formats" -metadata album="Formats" -metadata album_artist="Formats" \
   "$out/format.flac"
ff -f lavfi -i "$(tone)" -ac 2 -c:a libvorbis -metadata title="Ogg Vorbis" \
   -metadata artist="Formats" -metadata album="Formats" -metadata album_artist="Formats" \
   "$out/format.ogg"
ff -f lavfi -i "$(tone)" -ac 2 -c:a libopus -metadata title="Opus" \
   -metadata artist="Formats" -metadata album="Formats" -metadata album_artist="Formats" \
   "$out/format.opus"
ff -f lavfi -i "$(tone)" -ac 2 -c:a aac -metadata title="AAC" \
   -metadata artist="Formats" -metadata album="Formats" -metadata album_artist="Formats" \
   "$out/format.m4a"
ff -f lavfi -i "$(tone)" -ac 2 -c:a alac -metadata title="ALAC" \
   -metadata artist="Formats" -metadata album="Formats" -metadata album_artist="Formats" \
   "$out/format-alac.m4a"
ff -f lavfi -i "$(tone)" -ac 2 -c:a pcm_s16le -metadata title="WAV" \
   -metadata artist="Formats" -metadata album="Formats" -metadata album_artist="Formats" \
   "$out/format.wav"
ff -f lavfi -i "$(tone)" -ac 2 -c:a pcm_s16be -metadata title="AIFF" \
   -metadata artist="Formats" -metadata album="Formats" -metadata album_artist="Formats" \
   "$out/format.aiff"

# --- Broken files ---------------------------------------------------------------------
head -c 4096 /dev/urandom > "$out/broken.mp3"
head -c 2048 /dev/urandom > "$out/broken.flac"
ff -f lavfi -i "$(tone)" -ac 2 -c:a flac -metadata title="Truncated" \
   -metadata artist="Broken" -metadata album="Broken" -metadata album_artist="Broken" \
   "$out/truncated.flac"
truncate -s 1024 "$out/truncated.flac"

# --- A text file that must be ignored by the scanner -----------------------------------
printf 'not audio\n' > "$out/notes.txt"

echo "fixtures written to $out"