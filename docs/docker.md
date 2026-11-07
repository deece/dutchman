# Docker

Dutchman runs in a container with the `dutchman` command, the `dutchman-mcp`
[MCP server](mcp.md) and the `dutchman-web` [browser interface](web.md), for x86_64 and ARM64
Linux. It has no desktop app: `dutchman-gui` needs a desktop session. Use it to convert on a
server, in CI, or anywhere you would rather not install Dutchman.

## Get the image

### Pull the published image

Each release publishes `ghcr.io/dutchman-cad/dutchman`, made from the same Linux binaries as the
downloads; Docker picks the image for its own architecture:

```sh
docker pull ghcr.io/dutchman-cad/dutchman:latest
docker tag ghcr.io/dutchman-cad/dutchman:latest dutchman
docker run --rm dutchman --version
```

The `docker tag` line lets the commands below say `dutchman`; you can also give the full name
each time. The tags are:

- `X.Y.Z`: one release, such as `0.2.1`; it never changes.
- `latest`: the newest release.
- `nightly`: built each day from `main` against cadmpeg's `main`, after its tests pass.

To update, pull again (`docker pull ghcr.io/dutchman-cad/dutchman:latest`, then tag it again),
or with Compose, `docker compose pull && docker compose up -d`. A tool such as Watchtower can
pull new releases for you.

### Build it yourself

To build from a checkout of the repository instead, with the [`Dockerfile`](../Dockerfile) at its top:

```sh
docker build -t dutchman .
```

It builds `dutchman` from that checkout in a Rust container, then copies the three programs into
a small Debian image. The first build takes several minutes; later builds reuse the downloaded
crates. It compiles one crate per core at once, and the largest need several GB of memory each:
if the build stops with `cannot allocate memory` or `SIGKILL`, compile fewer at once, or give
Docker more memory. With about 8 GB for Docker, compile one at a time; on an Apple silicon Mac
that build took about 45 minutes:

```sh
docker build -t dutchman --build-arg CARGO_BUILD_JOBS=1 .
```

Check it:

```sh
docker run --rm dutchman --version
```

## Convert

The image runs `dutchman` in `/work`. Mount the folder that holds your designs there, and
run as yourself so that the converted designs belong to you:

```sh
docker run --rm -v "$PWD:/work" --user "$(id -u):$(id -g)" dutchman part.f3d
docker run --rm -v "$PWD:/work" --user "$(id -u):$(id -g)" dutchman convert --json part.f3d out.sc
docker run --rm -v "$PWD:/work" --user "$(id -u):$(id -g)" dutchman convert-all --json --out-dir converted/ project/
```

Paths are inside the container, so give them relative to the mounted folder. Every command,
option and exit code is the same as outside Docker ([Getting started](guide.md)), and `--json`
prints the same reports ([json.md](json.md)).

An Inventor assembly needs its part files: mount the whole project folder, not just the
assembly's own folder, or the parts it cannot find are reported as left out.

`convert-all` stops a file that runs past `--timeout` or holds more than `--max-memory` (no
limit by default). A file also stops when the container's own memory limit is nearly reached, so
the container isn't stopped first.

## In a browser

The repository's [`compose.yaml`](../compose.yaml) runs `dutchman-web` from the image on port
8080, asking for the password you give it:

```sh
DUTCHMAN_WEB_PASSWORD=choose-one docker compose up -d --build
```

Open `http://<server>:8080` and sign in as `dutchman`. Or run it with `docker run`:

```sh
docker run -d --name dutchman-web -p 8080:8080 -e DUTCHMAN_WEB_PASSWORD=choose-one \
  --entrypoint dutchman-web dutchman
```

When exposing the port beyond the local network, put it behind an HTTPS reverse proxy and
keep port 8080 private to the proxy. HTTP basic authentication over plain HTTP sends the
password readably. Preserve the request's `Host` header or set `DUTCHMAN_WEB_PUBLIC_ORIGIN`
to the proxy's public origin, such as `https://cad.example.org`.

[Dutchman Web](web.md) describes the page, its upload and disk limits, and its settings.

## As an MCP server

Run `dutchman-mcp` in the container, with stdin kept open (`-i`). Mount your designs at the
same path inside the container as outside it, because the tools take absolute paths on the
machine running the server. For Claude Code, with designs under `/home/me/cad`:

```sh
claude mcp add dutchman -- docker run --rm -i -v /home/me/cad:/home/me/cad --user 1000:1000 --entrypoint dutchman-mcp dutchman
```

Other clients take the same command in their `mcpServers` entry:

```json
{
  "mcpServers": {
    "dutchman": {
      "command": "docker",
      "args": ["run", "--rm", "-i", "-v", "/home/me/cad:/home/me/cad", "--user", "1000:1000",
               "--entrypoint", "dutchman-mcp", "dutchman"]
    }
  }
}
```

Replace `/home/me/cad` with your folder and `1000:1000` with your user and group ids
(`id -u`, `id -g`).

## With Compose

The [Compose example](../examples/compose/) serves SolidCraft and Dutchman on one address,
with a shared folder and a watched inbox.

Save as `compose.yaml` next to your designs, then convert with
`docker compose run --rm dutchman convert-all --out-dir converted/ .`:

```yaml
services:
  dutchman:
    image: ghcr.io/dutchman-cad/dutchman:latest
    user: "${LOCAL_UID:-1000}:${LOCAL_GID:-1000}"
    working_dir: /work
    volumes:
      - .:/work
```

Set `LOCAL_UID=$(id -u) LOCAL_GID=$(id -g)` first so the converted designs belong to you. On a
host with SELinux, add `:Z` to the volume (`.:/work:Z`).

## Checking histories in SolidCraft

Without `solidcraft-cli`, a written history is reported `unverified`
([Feature history](history.md)). On x86_64, an image you build
yourself can include a [SolidCraft release](https://github.com/tangency-labs/solidcraft/releases) to check
each history. Give its version and the SHA-256 of its `solidcraft-VERSION-linux-x86_64.tar.gz`:

```sh
docker build -t dutchman \
  --build-arg SOLIDCRAFT_VERSION=0.20.0 \
  --build-arg SOLIDCRAFT_SHA256=0c0965f8807945722780f83750c4bd3ce6544f9d827e01cad0664b28cf2a6910 .
```

The build stops if the download does not match the SHA-256.

## What the image leaves out

- **The desktop app.** Run `dutchman-gui` on a computer with a desktop.
- **FreeCAD.** `--to freecad` writes its macro without FreeCAD, but `--to fcstd` and
  `--verify` need FreeCAD's command-line program, which the image does not include. Build
  your own image on top of this one with FreeCAD installed, and set `FREECAD_CMD` to it.
