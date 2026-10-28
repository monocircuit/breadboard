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
- `breadboard serve` running, its web UI on its port: `7357` unless
  `[web] port` in [`breadboard.toml`](config.md) or `--web-port` says
  otherwise. `curl -s http://127.0.0.1:7357/ | head` on that machine shows
  the page.

## Start

```sh
tailscale serve --bg 7357
```

Tailscale answers with the address, in the form
`https://<machine>.<tailnet>.ts.net/`. Open it on the other device. `--bg`
keeps it running after the shell closes and across reboots, until you stop
it. The UI sits at the root of that address: absolute paths of its assets
would break under `--set-path`.

## What is exposed

- **The read-only UI and nothing else**: what the overview shows, the
  orchestration, worktrees, MRs, pipelines and runs, with the names of
  branches, issues and MRs in them. No terminal, no files, no stack. The
  page makes no requests outside the machine.
- **To every device and user of your tailnet** that its access rules let
  reach the machine on port 443. Everyone in a shared tailnet sees it too;
  limit it with the tailnet's access rules, or don't use a shared one.
- **Not to the internet**: that is `tailscale funnel`, which breadboard's
  UI is never put behind.
- The request reaches breadboard from `127.0.0.1`, as the local browser's
  does, so binding to localhost no longer tells the two apart. Never start
  `breadboard serve` with write actions switched on (`--web-write`) while
  `tailscale serve` points at it: every tailnet device could then merge,
  answer and stop runs.

## Check and stop

```sh
tailscale serve status    # what is served, under which address
tailscale serve --https=443 off
```

`off` stops exactly this one; `tailscale serve reset` stops everything the
machine serves, other services included. Stopping `breadboard serve` alone
leaves the address in place, answering with an error until the UI is back.
