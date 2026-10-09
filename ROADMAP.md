# Roadmap

pisquared builds one goal at a time, in sequence. Each version ends on a single observable result, and
each result is stated as something a reader could check rather than something a contributor could assert.

## Why linear

Two earlier rounds of planning are preserved in `preliminary_plans/`. The second of them advanced
documentation, tool, and image in parallel within every version. That structure was chosen because three
axes with no integration point can each be individually correct and produce nothing that works — so every
version ended on a device that actually booted, which is the mechanism that survived from that round.

Sequential goals reach the same protection more cheaply. The tool produces something and the image consumes
it in *consecutive* versions, with a hardware check between them, so drift is caught at the boundary rather
than at the end of the project. The parallel structure bought insurance against a failure that sequencing
does not have.

What is kept from both rounds: the convergence check, the manual procedure written before the tool, and
shipping a prepared artifact rather than handing over something to edit.

## Target

**v1.0 is full functionality** — documentation site, setup tool, agent sandboxing, multiplexer, secure
connectivity, and MCP server installation.

## The ladder

```
  v0.1  DOWNLOAD     tool resolves device -> verified image           no hardware
  v0.2  CONFIGURE    tool injects operator config into the image       no hardware
  v0.3  FIRST        flashed device boots headless to a keyed shell   HARDWARE
  v0.4  AGENT        pi installed, one session runs                   HARDWARE
  v0.5  CONTAINED    nono policy applied before first agent run        HARDWARE
  v0.6  DURABLE      herdr sessions survive a disconnect               HARDWARE
  v0.7  OVERLAY      Tailscale; the LAN path closes                    HARDWARE
  v0.8  REPRODUCIBLE wiped card + fresh clone -> identical host        HARDWARE
  v1.0  TOOLING      MCP servers, each under a narrower policy         HARDWARE
```

Host components are added in the order **pi → nono → herdr → Tailscale**, inherited from the earlier rounds
and unchanged. Tailscale is last among them: by the time it is added there is something worth reaching, and
its credential and enrolment decisions have a settled host to apply to.

| Version | Result | Check | Hardware |
|---|---|---|---|
| **v0.1** | A verified, flashable base image | tool output matches the publisher's published hash, and its signature verifies | no |
| **v0.2** | An image carrying the operator's configuration | differs from upstream in exactly the documented fields; no mount, no elevation | no |
| **v0.3** | A device reachable without a console | flash, power on with nothing attached, key login, password refused | yes |
| **v0.4** | One agent session completes | a session runs end to end | yes |
| **v0.5** | The agent is contained | write inside the workspace succeeds, read outside returns `EPERM` | yes |
| **v0.6** | Sessions survive a disconnect | detach, reconnect from another device, same session and scrollback | yes |
| **v0.7** | Overlay is the only ingress | reachable over the overlay, LAN path closed | yes |
| **v0.8** | A host can be rebuilt from the repository | fresh clone and wiped card produce an identical host | yes |
| **v1.0** | Each MCP server is confined | every server under a policy narrower than the agent's | yes |

Seven of the nine versions need a device. The two that do not are the two that used to be blocked behind
having hardware in hand, and they are first because they are the ones where a wrong assumption is cheapest
to discover.

## What v0.1 establishes, and what it does not

v0.1's output is an **unmodified upstream image**. Nothing this project produces is in it. That is
deliberate: it is what allows the exit criterion to compare against the publisher's own published hash with
no hardware involved. That check compares one route — the tool's — rather than the two independent routes
originally planned, because the manual procedure that would have been the second is deferred. See the
requirements on manual paths above.

**v0.2 is the first version that emits something of ours.** Worth stating plainly so the ladder is not read
as though v0.1 already produces a pisquared image.

## v0.2 is the keystone

It is the version where the tool first writes something the image consumes. Everything after it inherits
that arrangement, and a disagreement between the two sides is the one failure in this project that is
silent by nature — a setting the operator believes is in effect and is not.

The mechanism is settled and is not revisited here: the tool opens the FAT boot partition of the image
file in user space and writes to it. No mounting, no elevation, one code path on all three operating
systems, and deterministic enough to test in CI with no hardware. This closes a seam the earlier plan left
open, having said the tool would never touch a card and then not saying how the file reached one.

## v0.5 is the version that justifies the repository

Everything before it is setup; everything after it is hardening. Its check runs a policy file this project
ships. Trusting the vendor that a sandbox works is not the same as having run our own configuration, and
this is the one version where the artifact under test is ours rather than theirs.

Two known limits carry into it from the earlier rounds' research and are stated here so they are designed
for rather than discovered: the chosen sandbox filters TCP only, so UDP and DNS pass unrestricted and are a
working exfiltration path; and its capability elevation is off by default, so a test asserting containment
cannot fail unless that is turned on deliberately.

## Requirements that hold at every version

Some properties are true from the first version rather than appearing at one:

- **The manual path stays first-class.** What is done by hand is written down as it is done, and the tool is
  checked against the written procedure rather than replacing it. It will be documented, though not
  necessarily at the same time as the tool that replaces it. v0.1 is the exception that shows this is a real
  deferral rather than a settled habit: the manual obtaining procedure was planned to be written there, before
  the tool, and is deferred to a concentrated documentation effort instead. The tool ships first and the page
  follows. The consequence is recorded rather than papered over — v0.1's operator whose device is not in the
  catalogue has no route, and v0.1's end-to-end check compares the tool's output against the publisher's
  published hash without a second route confirming it independently. What is deferred is listed in
  `openspec/changes/v0-1-image-download/proposal.md`, and the reasoning for writing the procedure first is
  kept in that change's `design.md` so the documentation effort inherits it.
- **Documentation lands with the work it describes.** Not as a parallel axis — as part of the group whose
  work calls for it.
- **Nothing claims a property before the version that makes it true.** The archived spec after v0.5
  describes a host that boots, runs an agent, and is contained. It claims neither credential handling nor
  overlay ingress, because at v0.5 it has neither.

## Deliberately not a version

- **Pre-release research.** An earlier plan opened with a research version holding five preconditions. All
  five were answerable at the moment they became relevant — on the booted device, or from a vendor's
  documentation — so it was removed rather than reordered. The kernel and Landlock question is the standing
  example: it is one command on a device that v0.3 produces.
- **MCP servers before v1.0.** Each is third-party code with its own credential and its own network needs,
  so they arrive after containment, the credential path, and the overlay are all in place.

## Earlier plans

`preliminary_plans/` holds the two rounds this supersedes, together with the decisions that shaped them,
including the externally verified facts about third-party tools. When this file and that directory disagree,
this file wins. `preliminary_plans/round1/HANDOFF.md` remains the best source for third-party facts and
`preliminary_plans/round1/bootstrap-pi-squared-host/design.md` for architectural reasoning; both carry
forward unchanged.
