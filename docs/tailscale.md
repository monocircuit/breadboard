# The web UI from another device: `tailscale serve`

`breadboard serve` answers its web UI on `127.0.0.1` only (#435), so the
overview in a browser works on the machine it runs on and nowhere else. To
look at it from the phone or another laptop, put
[`tailscale serve`](https://tailscale.com/kb/1312/serve) in front of it: no
new code, no port opened to the network, and only the devices of your own
tailnet get in (#436). Until breadboard has its own way in, this is the way.

## What you need

- Tailscale on the machine `breadboard serve` runs on and on the device you
  look from, both logged in to the same tailnet.
- MagicDNS and HTTPS certificates switched on for the tailnet (admin console,
  DNS). `tailscale serve` asks for it the first time if they are off.
- `breadboard serve` running with its web UI, on port `7435` unless
  `[web] port` in [`breadboard.toml`](config.md) or the env variable
  `BREADBOARD_WEB_PORT` (which wins) says otherwise. `curl -s
  http://127.0.0.1:7435/ | head` on that machine shows the page.
- The machine's tailnet name in `[web] hosts`. The page answers only to the
  host names it knows (`127.0.0.1`, `localhost` and these) and every other
  one with a 403, and `tailscale serve` passes the name the browser asked
  for. `tailscale status --self --json` shows it as `DNSName`:

  ```toml
  [web]
  hosts = ["<machine>.<tailnet>.ts.net"]
  ```

  In the repo's `breadboard.toml` this names your machine for everyone who
  checks out the repo: keep that change local, never commit it.

## Start

```sh
tailscale serve --bg 7435
```

Tailscale answers with the address, `https://<machine>.<tailnet>.ts.net/`.
Open it on the other device. `--bg` keeps it running after the shell closes
and across reboots, until you stop it. Every URL of the page is relative, so
it also works under a path next to other services:
`tailscale serve --bg --set-path /breadboard 7435`.

## What is exposed

- **The read-only UI and nothing else**: what the overview shows, the
  orchestration, worktrees, MRs, pipelines and runs, with the names of
  branches, issues and MRs in them. No terminal, no files, no stack. The
  page makes no requests outside the machine and polls only its own
  address.
- **To every device and user of your tailnet** that its access rules let
  reach the machine on port 443. Everyone in a shared tailnet sees it too;
  limit it with the tailnet's access rules, or don't use a shared one.
- **Not to the internet**: that is `tailscale funnel`, which breadboard's
  UI is never put behind.
- The request reaches breadboard from `127.0.0.1`, as the local browser's
  does, so binding to localhost no longer tells the two apart. Never set
  `[web] writes = true` while `tailscale serve` points at the UI: every
  tailnet device could then do what the page's write actions do.

## Check and stop

```sh
tailscale serve status            # what is served, under which address
tailscale serve --https=443 off   # stops this one
```

`tailscale serve reset` stops everything the machine serves, other services
included. Then take the name out of `[web] hosts` again. Stopping
`breadboard serve` alone leaves the address in place, answering with an
error until the UI is back.
