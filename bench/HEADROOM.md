# Command headroom — what actually caps a long CMD

87 long-command probes (ffmpeg transcode, multi-stage jq, find+grep),
each run at the shipped `max_tokens = 256` and again at 1024, across 17
ollama cells. 2026-07-29.

## `max_tokens` is not the constraint. Raising it changes nothing.

| budget | probes | hit the ceiling | truncated | longest cmd | median cmd |
|---|---|---|---|---|---|
| 256 | 34 | **0** | **0** | 153 ch | 103 ch |
| 1024 | 51 | **0** | **0** | 171 ch | 91 ch |

Not one probe ended with `stop_reason = length`. The highest
`eval_tokens` observed anywhere was 108 — well under 256. Models stop on
their own long before the ceiling.

Pairing the same question at both budgets: **24 pairs returned a
byte-identical command**, 11 differed (temperature 0.2 sampling, not
length — the 1024 variants are not systematically longer).

So the ceiling can be raised safely, but on its own it buys nothing. The
diagnosis that `max_tokens` was capping the payload was wrong; it never
binds.

## What actually stops a long CMD from reaching the prompt

93 rows produced no usable command. Three distinct causes, none of them
budget:

### 1. No `CMD:` line at all — the dominant cause

The model answers in prose and never emits the tag. This is what S3
addresses directly: **command-first prompting moves the vend rate from
56% to 81%** (266 paired cells). That is the single biggest lever
measured in this project, and it costs +19 ms.

### 2. The command is in the prose, and the parser drops it — 18 rows

`split_answer`'s bare-command fallback only fires when a *line's first
word* is a PATH executable. A command wrapped in backticks after prose
never qualifies:

```
Use `du -h | sort -rh` to list files by size, largest first.
     ^^^^^^^^^^^^^^^^ correct, complete, and thrown away
```

18 of 93 no-command rows (**19%**) contain a backticked span whose first
word is a real PATH executable — validated against the same executable
set `split_answer` uses, so these are not prose-in-backticks false
positives. Recovered examples run to 189 characters:

| model | recovered command |
|---|---|
| `mistral` | `ffmpeg -i input.mov -c:v libx264 -crf 23 -preset slow -s 1920x1080 -c:a aac -b:a 192k -g …` (189 ch) |
| `llama3.2:3b` | `ffmpeg -i input.mov -vf scale=1920:1080 -c:v libx264 -crf 18 -c:a aac -b:a 128k -ar 44100 …` (120 ch) |
| `qwen3.5:9b` | `jq -r '.[] \| select(.status == "failed") \| "\(.name)\t\(.bytes)"' data.json \| sort -k2,2` (90 ch) |

These are *good* answers — several better than what the same model
produced when it did use the tag. `llama3.2:3b` vends a command on only
35% of rows but has a usable one in prose far more often.

### 3. Refusal and memory hijack — small models only

`qwen3.5:0.8b` on the ffmpeg question: *"I cannot convert video files to
web-friendly formats like H264 MP4 directly in this terminal
environment; I am an AI assistant and not capable of…"* — a refusal to a
plain shell-syntax question.

The same model on the jq question emitted
`REMEMBER: from data.json give me name and bytes for every failed item…`
— echoing the question back as a memory write, the same hijack
`mistral-nemo` shows. Both render as silence in the band.

## Implications

Ranked by measured effect:

1. **Adopt command-first (S3).** 56% → 81% vend rate, +19 ms. It also
   makes truncation safe in the direction you want: if a budget ever
   does bind, prose is lost instead of the command — and prose is
   display-bound anyway (the band clamps it at render).
2. **Teach `split_answer` to recover a backticked command from prose.**
   Recovers 19% of no-command rows, using validation the function
   already performs. Low risk: requires backticks *and* a PATH
   executable first word.
3. **Raise `max_tokens` if you like — but not for this.** It never
   binds. If it is raised, do it for `##` chat replies, not for CMD.

## What this does not settle

- Whether the recovered inline commands are *correct*. They look
  plausible and several are clearly good, but correctness is for blind
  grading, not for me eyeballing a table.
- Whether `stop: ["\n\n"]` forecloses genuinely multi-line commands
  (heredocs, `\`-continuations). Every probe here returned a single
  line, so the question is untouched by this data.
- LM Studio cells: these 87 probes are ollama-only so far. The LM Studio
  cells run at the end of Pass A.
