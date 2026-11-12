# Dutchman Web

## Project inputs

Files whose names start with `~$` are lock files. They are ignored in project planning
and excluded from job, failure and skipped-file counts.

Use **Choose a project folder** to upload a folder with its relative paths preserved.
Folder uploads share the ZIP entry budget and upload byte limit. At most 10,000
individual files or archives can be uploaded at once.

Project planning uses the same rules as the desktop app and command line: top-level
Inventor and SolidWorks assemblies convert with parts resolved inside the project, then
unused parts convert separately. Assembly references determine which assemblies are top-level. A same-name
STEP export beside a native file is kept as a fallback if native conversion fails.
The JSON result includes a plain `plan` describing the selected files. The web upload
limits and retention settings below still apply.

`dutchman-web` is Dutchman as a web page: run it on one server, and convert from any browser
that can reach it, without installing Dutchman on each computer. Drop design files, or a `.zip`
of a project folder, on the page and press **Convert**. Each file gets a card saying whether it
converted, what was written (editable history or geometry), which features did not map, and its
losses as Info, Degraded, or Dropped, with a link to download its design. **Download all** gets
every design in one zip. The page waits for the final results; it does not stream conversion
stages, design counts, or elapsed-time heartbeats.

It converts exactly as `dutchman convert-all` does, each file in a process of its own, stopped
when it runs past its time or memory limit. Uploads and designs stay on the server, and are
deleted after a while (an hour by default).

## Run it

In Docker, from a checkout of the repository ([Docker](docker.md) has more):

```sh
DUTCHMAN_WEB_PASSWORD=choose-one docker compose up -d --build
```

Then open `http://<server>:8080` and sign in as `dutchman` with that password.

Linux release and nightly archives include `dutchman-web`. Or build it with Rust 1.88 or newer:

```sh
cargo install --git https://github.com/dutchman-cad/dutchman dutchman-web
dutchman-web                          # http://127.0.0.1:8080, this computer only
DUTCHMAN_WEB_PASSWORD=choose-one dutchman-web --listen 0.0.0.0:8080
```

It checks histories in SolidCraft when it finds `solidcraft-cli`, as the command does
([Feature history](history.md)): set `SOLIDCRAFT_CLI` to
its path, or put it on the server's `PATH`. The form's **Target SolidCraft version**
accepts x.y.z, at least 0.18.2. Leave it empty to target the found CLI's version, else
the newest known at build time (currently 0.40). The rolling 14-day window is currently
0.18.2–0.40; older targets get simplified features. FreeCAD helix history needs a target of 0.25 patch 1 or newer; older targets preserve
the original geometry and report the required version.

## Settings

Each setting is an option, or the environment variable beside it. `dutchman-web --help` lists
them.

| Option | Environment | Default | What it sets |
|---|---|---|---|
| `--listen` | `DUTCHMAN_WEB_LISTEN` | `127.0.0.1:8080` (`0.0.0.0:8080` in the Docker image) | The address and port to listen on |
| | `DUTCHMAN_WEB_PASSWORD` | none | Ask every browser for a user name and password. Environment only, so it never shows in a process listing |
| `--user` | `DUTCHMAN_WEB_USER` | `dutchman` | The user name asked for with the password |
| `--data-dir` | `DUTCHMAN_WEB_DATA` | `dutchman-web` in the system's temporary folder | Where uploads and designs are kept |
| `--keep` | `DUTCHMAN_WEB_KEEP` | 60 | Minutes a job's uploads and designs are kept |
| `--max-upload` | `DUTCHMAN_WEB_MAX_UPLOAD` | 200 | Megabytes one upload may hold, all its files together |
| `--max-unzipped` | `DUTCHMAN_WEB_MAX_UNZIPPED` | 2048 | Megabytes all zips of one upload may unpack to together |
| `--uploads` | `DUTCHMAN_WEB_UPLOADS` | 4 | Uploads received at once; excess requests get 503 immediately |
| `--max-data` | `DUTCHMAN_WEB_MAX_DATA` | 10240 | Refuse new jobs when saved data exceeds this many megabytes |
| `--min-free` | `DUTCHMAN_WEB_MIN_FREE` | 1024 | Refuse new jobs below this many megabytes of available disk space (Unix) |
| `--public-origin` | `DUTCHMAN_WEB_PUBLIC_ORIGIN` | none | Additional allowed browser origins, comma-separated, such as `https://cad.example.org` |
| `--runs` | `DUTCHMAN_WEB_RUNS` | 2 | Uploads converted at once; more wait their turn |
| `--jobs` | `DUTCHMAN_WEB_JOBS` | the machine's cores, at most 8 | Files of one upload converted at once |
| `--timeout` | `DUTCHMAN_WEB_TIMEOUT` | 900 | Seconds one file may take |
| `--max-memory` | `DUTCHMAN_WEB_MAX_MEMORY` | 8192 | Megabytes one file may hold (0: no limit) |
| `--output-dir` | `DUTCHMAN_WEB_OUTPUT` | none | A folder each job's designs are also saved in, such as a network share ([Save to a shared folder](#save-to-a-shared-folder)) |
| `--solidcraft` | `DUTCHMAN_WEB_SOLIDCRAFT` | none | The SolidCraft web app's same-origin path ([Open in SolidCraft](#open-in-solidcraft)) |
| `--base-path` | `DUTCHMAN_WEB_BASE_PATH` | none (the root) | The path it is reached under behind a reverse proxy, such as `/dutchman` ([Under a path](#under-a-path)) |

At most `--runs` × `--jobs` files convert at once, each holding up to `--max-memory`. Give the
server or container that much memory, or lower them.

## Uploads

- Design files are converted as they are. Files Dutchman does not know are skipped and
  counted.
- A `.zip` is unpacked first, keeping its folders, so an Inventor assembly finds its part
  files: zip the whole project folder. A zip of one folder unpacks as that folder. Entries
  that would land outside it are refused and symbolic links are skipped. All zips of one
  upload share the 5,000-entry cap and `--max-unzipped` byte cap.
- **SolidCraft design** writes `.sc` designs (`.solidcraft` for targets before 0.25.1); **FreeCAD macro (experimental)** writes
  `.FCMacro` rebuild macros (`--to freecad`). **Geometry only** is `--import-only`.

## When a file fails

A failed file's card offers to send it privately, as the desktop app does: join the
[Discord server](https://discord.gg/yTcseBEdeu), then a button copies a one-line message and opens
Dutchy's DM. The message names no file. The server sends nothing itself, and Discord opens in
your browser. See [Send the file privately](troubleshooting.md#send-the-file-privately).

## Save to a shared folder

With `--output-dir`, each converted design is also saved in that folder, such as a network
share the server has mounted, where it stays after the job is deleted. Designs keep the folders
and names they were uploaded with (`project/bracket.sc`). A name already taken in the
folder gets a number, as a browser's downloads do (`bracket (1).sc`), so a file already
there is never replaced. Designs are not saved through a symbolic link in the folder.

Each card says where its design was saved, and the API report gives it as `saved` in the
file's `output`, relative to the folder. A design that could not be saved, for instance because
the share is not mounted, still downloads; its card says why, as `save_error` does. The folder
counts towards neither `--max-data` nor the expiry.

In Docker, mount the share and run as a user who may write to it:

```sh
docker run -d -p 8080:8080 -e DUTCHMAN_WEB_PASSWORD=choose-one \
  -v /mnt/share:/designs --user "$(id -u):$(id -g)" \
  --entrypoint dutchman-web ghcr.io/dutchman-cad/dutchman:latest --output-dir /designs/Converted
```

## Under a path

The [Compose example](../examples/compose/) serves SolidCraft and Dutchman on one address,
with a shared folder and a watched inbox.

To share one address with other services, serve `dutchman-web` under a path such as
`/dutchman/`: start it with `--base-path /dutchman`, and have the reverse proxy send that path
on unchanged, without taking the prefix off. Every page and API route is then under the path
(`/dutchman/api/convert`), `/dutchman` moves to `/dutchman/`, and `/healthz` answers both at the
root and under the path. In Caddy:

```caddyfile
handle /dutchman* {
	reverse_proxy dutchman-web:8080
}
```

## Open in SolidCraft

With `--solidcraft`, each converted SolidCraft design gets an **Open in SolidCraft** link
beside Download, opening in a new tab. FreeCAD macros keep only their download link. The API
adds the full link as `open` in the file's `output`. Links are built from the current settings,
so changing the address on restart also changes existing jobs' links.

For SolidCraft and Dutchman on one origin, use a path: run `dutchman-web` with
`--base-path /dutchman --solidcraft /`, and serve SolidCraft at `/`. In Caddy, keeping the
Dutchman path unchanged as in [Under a path](#under-a-path):

```caddyfile
handle /dutchman* {
	reverse_proxy dutchman-web:8080
}
handle {
	reverse_proxy solidcraft:8080
}
```

The link opens `/?open=...`, with `/dutchman/jobs/<job>/files/<design>` encoded as the query
value. This also works over plain HTTP on a LAN. Downloads use Dutchman's HTTP basic
authentication; the browser reuses those credentials when SolidCraft fetches the design on
the same origin.

The address must start with exactly one `/`, without backslashes, control characters, a
query or a fragment; anything else stops `dutchman-web` at startup. This needs SolidCraft
[#6670](https://github.com/tangency-labs/solidcraft/pull/6670): use a build from its main
branch until its next release. Links work only while the job's design is kept.

## Keep it safe

- Set `DUTCHMAN_WEB_PASSWORD` unless everyone who can reach the server may convert files on it.
  Without it, `dutchman-web` says so when it listens beyond this computer.
- HTTP basic authentication over plain HTTP sends the password readably with every request.
  When reachable beyond the local network, put `dutchman-web` behind an HTTPS reverse proxy
  (Caddy, nginx, Traefik). Keep the server port private to that proxy.
- Browser uploads with `Sec-Fetch-Site: cross-site` are refused with 403. An `Origin`, when
  present, must match the request's `Host` (including its port), or an exact origin in
  `--public-origin`. Have the proxy preserve `Host`, or configure its public origin.
  Requests with neither header, such as curl and scripts, are allowed.
- On Unix, new data folders are created with mode 0700. Existing folders must belong to the
  current user, be directories rather than symbolic links, and have no group or other write
  permission. These checks also apply to the default temporary folder.
- New jobs get 503 when saved data exceeds `--max-data`, or available disk space is below
  `--min-free` on Unix. Rust's standard library has no portable free-space API; the existing
  `rustix` dependency supplies the Unix check. Other platforms enforce `--max-data` only.
  These are admission checks; uploads and conversions already running can still grow the
  saved data. Size the volume for the configured upload and conversion limits. Expired jobs
  continue to be deleted.
- Uploaded files are hostile until read. Each one is converted in a child process with the
  time and memory limits above, and uploads are limited in size and in what a zip may unpack
  to.

## Its API

Scripts can use the same server. Paths are relative to the server's address, under
`--base-path` when one is set; give the password with HTTP basic authentication.

| Request | Returns |
|---|---|
| `POST /api/convert`, a `multipart/form-data` body: `files` (one or more), and optionally `to` (`solidcraft` or `freecad`) and `import_only` (`true`) | The [`dutchman.convert-all/1`](json.md#dutchmanconvert-all1) report, with `job` and `designs_zip` added, a `private_report` (`id`, `size_bytes`) added to each failed file, and a `download` path added to each converted file's `output` (with `--output-dir`, `saved` or `save_error`; with `--solidcraft`, `open` for SolidCraft designs). Paths in it are relative to the upload. A refused upload is `{"error": "…"}` with status 400, 403, 413 or 503 |
| `GET /jobs/<job>/report.json` | The same report again, while the job is kept |
| `GET /jobs/<job>/files/<path>` | One design |
| `GET /jobs/<job>/designs.zip` | Every design of the job, in one zip |
| `GET /healthz` | `ok`, without a password, for health checks |

```sh
curl -u dutchman:choose-one -F files=@part.f3d -F files=@bracket.ipt http://server:8080/api/convert
```

The page itself posts to `/convert`, which sends the browser on to `/jobs/<job>`.

ZIP directory data and ZIP64 extensible records are limited to 16 MiB each before
the ZIP directory is allocated; entry budgets count every declared entry.
