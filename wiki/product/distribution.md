# Getting Serious: Builds, Providers, Distribution

The path from "runs on my mac" to `brew install goulash`. Domain in
hand: **goulash.dev** — which quietly settles the
[naming question](../naming/decision.md); you don't point a domain at
a working name. Ordered by leverage, not glamour.

## 0. Repo hygiene first (blocks everything downstream)

- **LICENSE** — none exists yet; nothing can be packaged, vendored, or
  accepted into homebrew-core without one. Rust convention is
  MIT/Apache-2.0 dual. *(User call.)*
- **Cargo.toml metadata** — `description`, `repository`, `license`,
  `keywords`; required by crates.io.
- **Reserve the crate name NOW.** crates.io is first-come-first-served
  and `goulash` is a plausibly-squattable word. Publish an early 0.1.x
  the moment a license lands — the name is the asset; polish can
  follow. `cargo install goulash` then works for free.

## 1. Builds & releases: cargo-dist does 1 and 3 together

[cargo-dist](https://opensource.axo.dev/cargo-dist/) generates the
whole release apparatus from one `dist init`: a tag-driven GitHub
Actions workflow producing per-target binaries
(`aarch64-apple-darwin` first, then `x86_64-apple-darwin`,
linux gnu/musl), GitHub Releases, **a Homebrew tap formula**, and a
**shell installer** — which goulash.dev can front:

```
curl -fsSL https://goulash.dev/install.sh | sh
```

Tags drive releases (`v0.2.0`); note this session's git access can't
push tags, so cutting a release is a from-the-mac action (or a
one-line workflow_dispatch).

### Cutting a release, as actually built

`.github/workflows/release.yml`, not cargo-dist (yet). Two entry
points and one convention:

```
git push origin v0.4.1        # tag push: the whole party
git push origin v0.4.1-rc.1   # candidate: binaries only
                              # or press Run, leave the tag box blank
```

**A hyphen makes it a candidate.** `v0.4.1-rc.1`, `v0.5.0-dev.2` —
anything semver calls a prerelease builds all four targets and
publishes them so you can hand them out, and does **not** touch the
Homebrew formula or take the "Latest release" banner. Ride as many as
you need while the bugs shake out, then push the bare tag once.

**Stable tags are write-once.** brew pins a sha256 per version, so
moving a released tag changes the tarball under a checksum users have
already resolved. The cheap fix for a bad release is the next patch
number.

**Dispatch with the tag box blank** takes `v` + the version in
`Cargo.toml` on main, so bumping the version *is* naming the release.
A tag on the wrong commit is caught before the toolchain is fetched:
`build` asserts the tag and `Cargo.toml` agree.

**The workflow is read from the tag**, so a fix to it only takes
effect for tags cut *after* it lands on main.

#### Why there is a `verify` job

Three v0.4.0 tag pushes built four platforms each, threw the binaries
away, published nothing, and reported **success**. `tag` is skipped by
design on a tag push; skipped-ness propagates down a `needs:` chain;
`build` carried a `!cancelled()` guard and `release`/`formula` did
not, so they inherited the skip — and a skipped job does not fail a
run.

That is this codebase's oldest failure shape wearing CI clothes: *it
looked like it worked*. So `verify` asserts the outcome from outside —
the release exists, carries eight assets, has the right prerelease
flag, and moved the formula only if it should have — and is itself
guarded so an upstream skip cannot skip it.

### The mac signing reality check

Signing is **channel-dependent**, not a blanket requirement:

- `brew install` and `curl | sh` never set the quarantine xattr —
  **Gatekeeper doesn't fire**. Homebrew-first distribution needs no
  Apple account at all.
- arm64 binaries get an **ad-hoc signature automatically** at link
  time; that satisfies the kernel's must-be-signed rule on Apple
  Silicon.
- Real signing (Developer ID cert, $99/yr) + **notarization**
  (`codesign --options runtime` → `notarytool submit` → staple) is
  needed only when users download binaries *with a browser* from
  GitHub Releases / goulash.dev. Do it when that channel matters, as
  CI steps with the cert + App Store Connect API key in Actions
  secrets — not before.

## 2. Providers: one adapter buys ten providers

Order of attack for the [engine](../architecture/llm-engine.md):

1. **OpenAI-compatible `/v1/chat/completions` adapter** — the highest
   leverage move in the codebase: llama.cpp server, LM Studio, vLLM,
   OpenRouter, Groq, Together, DeepSeek … are all this one shape.
   SSE streaming instead of ollama's JSONL; same worker, same
   coalescing, same stable prefix. And it makes **`auto-local` the
   default service**: probe the local candidates in order — ollama
   (`:11434`), LM Studio (`:1234`), llama.cpp server (`:8080`) — and
   bind whatever is up; `#/service auto-local` restores it after any
   explicit pin. LM Studio doubles as the test rig for the adapter.
2. **Anthropic Messages adapter** — and here the architecture pays
   off: the epoch-trimmed append-only session log was *designed* as a
   stable prefix, which maps 1:1 onto server-side **prompt caching**
   (`cache_control` breakpoints). The "cloud KV portability pipe
   dream" turns out to ship as a product feature.
3. **Apple Foundation Models** — mac-first, "it's fucking there";
   needs a small Swift shim speaking our engine protocol. Investigate
   after 1–2.

### Keys and the metering hazard

- Keys come from **env by default** (`api_key_env = "OPENAI_API_KEY"`),
  config-file value as fallback; keys never enter transcripts (the
  [privacy invariant](../architecture/block-history.md) already
  guarantees typed input is never recorded). Keychain integration
  later.
- **Proactive [commentary](../interaction/heckle-mode.md) is a cost
  bomb on metered providers**: an LLM call per command turn is free on
  ollama and real money on an API. Per-provider defaults: commentary
  on for local providers, off (or hard rate-limited) for metered ones
  — a provider knows whether it's metered.

## 3. Homebrew, staged

1. **Own tap, immediately** — cargo-dist maintains it on every
   release: `brew install chaboud/tap/goulash` (or a `goulash-dev`
   org tap to match the domain).
2. **homebrew-core, eventually** — wants a license, stable tagged
   releases, and demonstrated notability (stars/traction). The tap is
   the on-ramp; core is a milestone, not a starting point.

## 4. goulash.dev

- Phase 1: redirect to the repo.
- Phase 2: GitHub Pages from this repo — the wiki *is* the seed
  content — plus hosting `install.sh` for the one-liner.
- Phase 3: a real landing page ("Your shell, with a coach") when
  there's something to point at. `.dev` is HSTS-preloaded
  (HTTPS-only), which Pages handles.

## Sequence, compressed

license → crates.io name grab → cargo-dist + tap (unsigned, brew-only)
→ OpenAI-compat adapter → Anthropic + prompt caching → notarize when
browser downloads matter → Pages site → homebrew-core when earned.
