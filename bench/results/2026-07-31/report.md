# Engine characterization — report card

4058 rows recorded (3698 sweep). Mechanical metrics only; qualitative
scores are joined from grades.jsonl after blind grading.

## Per model

`empty→stop` = truncated by the stop sequence before emitting anything.
`empty→budget` = spent the whole budget thinking. `mem-only` = replied
with a bare REMEMBER: line instead of answering. Load time is excluded
from latency (the first call per model pays it).

| model | provider | tier | p50 ttft | p50 total | answered | empty→stop | empty→budget | mem-only | fenced | 1-line | CMD: | reasoning tok |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `gemma3:4b` | ollama | watcher | 2208ms | 4867ms | 100% | 0% | 0% | 0% | 15% | 85% | 96% | - |
| `llama3.1:8b` | ollama | driver | 2554ms | 4946ms | 99% | 0% | 0% | 1% | 0% | 98% | 97% | - |
| `llama3.2:3b` | ollama | watcher | 1886ms | 3181ms | 64% | 1% | 0% | 35% | 0% | 94% | 53% | - |
| `mistral:latest` | ollama | driver | - | - | 0% | 0% | 0% | 0% | 0% | 100% | 0% | - |
| `qwen3.5:0.8b` | ollama | watcher | 1152ms | 2106ms | 89% | 1% | 0% | 9% | 0% | 93% | 57% | - |
| `qwen3.5:2b` | ollama | watcher | 2732ms | 4660ms | 100% | 0% | 0% | 0% | 0% | 100% | 96% | - |
| `qwen3.5:4b` | ollama | watcher | 7722ms | 10936ms | 100% | 0% | 0% | 0% | 1% | 93% | 93% | - |
| `qwen3.5:9b` | ollama | driver | 10072ms | 13147ms | 100% | 0% | 0% | 0% | 0% | 100% | 95% | - |

## Per prompt shape

S1 = shipped (memories before log) · S2 = memories in suffix · S3 = CMD: first

| shape | p50 ttft | p50 total | p50 prompt-eval | empty | 1-line | CMD: |
|---|---|---|---|---|---|---|
| S1 | 2937ms | 5173ms | 2741ms | 4% | 96% | 72% |
| S2 | 3425ms | 5577ms | 3263ms | 7% | 96% | 72% |
| S3 | 2936ms | 5031ms | 2719ms | 5% | 98% | 75% |
| S4 | 2974ms | 5214ms | 2773ms | 7% | 98% | 73% |
| S5 | 2978ms | 5053ms | 2757ms | 7% | 90% | 72% |
| S6 | 2841ms | 5178ms | 2672ms | 7% | 91% | 71% |
| S7 | 2791ms | 4698ms | 2599ms | 5% | 97% | 74% |

## Command headroom

`max_tokens` does no display work — the band clamps prose at render
and the paste path sends the command whole — so the ceiling only
ever caps the payload. Each question is run at the shipped 256 and
at 1024 to separate "terse model" from "budget cut it off".

| model | probe | cmd chars | stop | truncated |
|---|---|---|---|---|
| `deepseek-coder:33b-instruct-q3_K_S` | long-ffmpeg-1024 | 0 | stop |  |
| `deepseek-coder:33b-instruct-q3_K_S` | long-ffmpeg-256 | 0 | stop |  |
| `deepseek-coder:33b-instruct-q3_K_S` | long-find-1024 | 0 | stop |  |
| `deepseek-coder:33b-instruct-q3_K_S` | long-jq-1024 | 0 | stop |  |
| `deepseek-coder:33b-instruct-q3_K_S` | long-jq-256 | 0 | stop |  |
| `deepseek-r1:14b` | long-ffmpeg-1024 | 0 | stop |  |
| `deepseek-r1:14b` | long-ffmpeg-256 | 0 | stop |  |
| `deepseek-r1:14b` | long-find-1024 | 0 | stop |  |
| `deepseek-r1:14b` | long-jq-1024 | 0 | stop |  |
| `deepseek-r1:14b` | long-jq-256 | 0 | stop |  |
| `gemma3:4b` | long-ffmpeg-1024 | 125 | stop |  |
| `gemma3:4b` | long-ffmpeg-256 | 125 | stop |  |
| `gemma3:4b` | long-find-1024 | 45 | stop |  |
| `gemma3:4b` | long-jq-1024 | 74 | stop |  |
| `gemma3:4b` | long-jq-256 | 52 | stop |  |
| `gemma4:12b` | long-ffmpeg-1024 | 92 | stop |  |
| `gemma4:12b` | long-ffmpeg-256 | 92 | stop |  |
| `gemma4:12b` | long-find-1024 | 47 | stop |  |
| `gemma4:12b` | long-jq-1024 | 89 | stop |  |
| `gemma4:12b` | long-jq-256 | 88 | stop |  |
| `gemma4:12b-mlx` | long-ffmpeg-1024 | 158 | stop |  |
| `gemma4:12b-mlx` | long-ffmpeg-256 | 158 | stop |  |
| `gemma4:12b-mlx` | long-find-1024 | 76 | stop |  |
| `gemma4:12b-mlx` | long-jq-1024 | 91 | stop |  |
| `gemma4:12b-mlx` | long-jq-256 | 91 | stop |  |
| `gemma4:e2b` | long-ffmpeg-1024 | 123 | stop |  |
| `gemma4:e2b` | long-ffmpeg-256 | 123 | stop |  |
| `gemma4:e2b` | long-find-1024 | 86 | stop |  |
| `gemma4:e2b` | long-jq-1024 | 68 | stop |  |
| `gemma4:e2b` | long-jq-256 | 76 | stop |  |
| `gemma4:e4b` | long-ffmpeg-1024 | 141 | stop |  |
| `gemma4:e4b` | long-ffmpeg-256 | 149 | stop |  |
| `gemma4:e4b` | long-find-1024 | 127 | stop |  |
| `gemma4:e4b` | long-jq-1024 | 20 | stop |  |
| `gemma4:e4b` | long-jq-256 | 96 | stop |  |
| `gemma4:e4b-mlx` | long-ffmpeg-1024 | 94 | stop |  |
| `gemma4:e4b-mlx` | long-ffmpeg-256 | 114 | stop |  |
| `gemma4:e4b-mlx` | long-find-1024 | 80 | stop |  |
| `gemma4:e4b-mlx` | long-jq-1024 | 90 | stop |  |
| `gemma4:e4b-mlx` | long-jq-256 | 90 | stop |  |
| `google/gemma-4-12b-qat` | long-ffmpeg-1024 | 167 | length | **yes** |
| `google/gemma-4-12b-qat` | long-ffmpeg-256 | 0 | length |  |
| `google/gemma-4-12b-qat` | long-find-1024 | 9 | length | **yes** |
| `google/gemma-4-12b-qat` | long-jq-1024 | 0 | length |  |
| `google/gemma-4-12b-qat` | long-jq-256 | 0 | length |  |
| `google/gemma-4-e4b` | long-ffmpeg-1024 | 0 | stop |  |
| `google/gemma-4-e4b` | long-ffmpeg-1024 | 0 | stop |  |
| `google/gemma-4-e4b` | long-ffmpeg-256 | 0 | stop |  |
| `google/gemma-4-e4b` | long-ffmpeg-256 | 0 | stop |  |
| `google/gemma-4-e4b` | long-find-1024 | 0 | stop |  |
| `google/gemma-4-e4b` | long-find-1024 | 0 | stop |  |
| `google/gemma-4-e4b` | long-jq-1024 | 0 | stop |  |
| `google/gemma-4-e4b` | long-jq-1024 | 0 | stop |  |
| `google/gemma-4-e4b` | long-jq-256 | 0 | stop |  |
| `google/gemma-4-e4b` | long-jq-256 | 0 | stop |  |
| `gpt-oss:20b` | long-ffmpeg-1024 | 0 | stop |  |
| `gpt-oss:20b` | long-ffmpeg-256 | 0 | stop |  |
| `gpt-oss:20b` | long-find-1024 | 0 | stop |  |
| `gpt-oss:20b` | long-jq-1024 | 0 | stop |  |
| `gpt-oss:20b` | long-jq-256 | 0 | stop |  |
| `llama3.1:8b` | long-ffmpeg-1024 | 53 | stop |  |
| `llama3.1:8b` | long-ffmpeg-256 | 81 | stop |  |
| `llama3.1:8b` | long-find-1024 | 13 | stop |  |
| `llama3.1:8b` | long-jq-1024 | 82 | stop |  |
| `llama3.1:8b` | long-jq-256 | 100 | stop |  |
| `llama3.2:3b` | long-ffmpeg-1024 | 90 | stop |  |
| `llama3.2:3b` | long-ffmpeg-256 | 221 | stop |  |
| `llama3.2:3b` | long-find-1024 | 70 | stop |  |
| `llama3.2:3b` | long-jq-1024 | 30 | stop |  |
| `llama3.2:3b` | long-jq-256 | 48 | stop |  |
| `qwen/qwen3-1.7b` | long-ffmpeg-1024 | 91 | length | **yes** |
| `qwen/qwen3-1.7b` | long-ffmpeg-256 | 99 | length | **yes** |
| `qwen/qwen3-1.7b` | long-find-1024 | 83 | length | **yes** |
| `qwen/qwen3-1.7b` | long-jq-1024 | 0 | stop |  |
| `qwen/qwen3-1.7b` | long-jq-256 | 60 | - |  |
| `qwen/qwen3-4b` | long-ffmpeg-1024 | 0 | stop |  |
| `qwen/qwen3-4b` | long-ffmpeg-256 | 0 | stop |  |
| `qwen/qwen3-4b` | long-find-1024 | 0 | stop |  |
| `qwen/qwen3-4b` | long-jq-1024 | 0 | stop |  |
| `qwen/qwen3-4b` | long-jq-256 | 0 | stop |  |
| `qwen/qwen3-8b` | long-ffmpeg-1024 | 109 | stop |  |
| `qwen/qwen3-8b` | long-ffmpeg-1024 | 0 | stop |  |
| `qwen/qwen3-8b` | long-ffmpeg-256 | 116 | stop |  |
| `qwen/qwen3-8b` | long-ffmpeg-256 | 0 | stop |  |
| `qwen/qwen3-8b` | long-find-1024 | 0 | stop |  |
| `qwen/qwen3-8b` | long-find-1024 | 0 | stop |  |
| `qwen/qwen3-8b` | long-jq-1024 | 0 | stop |  |
| `qwen/qwen3-8b` | long-jq-1024 | 0 | stop |  |
| `qwen/qwen3-8b` | long-jq-256 | 98 | length | **yes** |
| `qwen/qwen3-8b` | long-jq-256 | 0 | stop |  |
| `qwen3.5:0.8b` | long-ffmpeg-1024 | 27 | stop |  |
| `qwen3.5:0.8b` | long-ffmpeg-256 | 177 | stop |  |
| `qwen3.5:0.8b` | long-find-1024 | 51 | stop |  |
| `qwen3.5:0.8b` | long-jq-1024 | 145 | stop |  |
| `qwen3.5:0.8b` | long-jq-256 | 88 | stop |  |
| `qwen3.5:2b` | long-ffmpeg-1024 | 192 | stop |  |
| `qwen3.5:2b` | long-ffmpeg-256 | 116 | stop |  |
| `qwen3.5:2b` | long-find-1024 | 0 | stop |  |
| `qwen3.5:2b` | long-jq-1024 | 150 | stop | **yes** |
| `qwen3.5:2b` | long-jq-256 | 129 | stop | **yes** |
| `qwen3.5:4b` | long-ffmpeg-1024 | 89 | stop |  |
| `qwen3.5:4b` | long-ffmpeg-256 | 133 | stop |  |
| `qwen3.5:4b` | long-find-1024 | 136 | stop |  |
| `qwen3.5:4b` | long-jq-1024 | 0 | stop |  |
| `qwen3.5:4b` | long-jq-256 | 90 | stop |  |
| `qwen3.5:9b` | long-ffmpeg-1024 | 107 | stop |  |
| `qwen3.5:9b` | long-ffmpeg-256 | 107 | stop |  |
| `qwen3.5:9b` | long-find-1024 | 107 | stop |  |
| `qwen3.5:9b` | long-jq-1024 | 121 | stop |  |
| `qwen3.5:9b` | long-jq-256 | 161 | stop |  |
| `qwen3:14b` | long-ffmpeg-1024 | 187 | stop |  |
| `qwen3:14b` | long-ffmpeg-256 | 187 | stop |  |
| `qwen3:14b` | long-find-1024 | 72 | stop |  |
| `qwen3:14b` | long-jq-1024 | 90 | stop |  |
| `qwen3:14b` | long-jq-256 | 107 | stop |  |

At `max_tokens = 256`: 3/46 truncated, longest command 221 chars.

At `max_tokens = 1024`: 5/69 truncated, longest command 192 chars.

## Cache behaviour

Ollama only — LM Studio exposes no prompt-eval timing, so its cache
evidence is TTFT flatness in the per-turn table instead.

| model | shape | turns | prompt chars (first→last) | prompt-eval (first→last) |
|---|---|---|---|---|
| `gemma3:4b` | S1 | 68 | 3154 → 7312 | 5802ms → 1054ms |
| `gemma3:4b` | S2 | 68 | 3154 → 7300 | 6195ms → 1830ms |
| `gemma3:4b` | S3 | 68 | 3168 → 7603 | 5776ms → 1065ms |
| `gemma3:4b` | S4 | 68 | 3370 → 7762 | 6802ms → 1087ms |
| `gemma3:4b` | S5 | 68 | 3593 → 7924 | 7138ms → 1074ms |
| `gemma3:4b` | S6 | 68 | 19040 → 23630 | 36696ms → 1206ms |
| `gemma3:4b` | S7 | 68 | 3603 → 8090 | 7020ms → 1065ms |
| `llama3.1:8b` | S1 | 68 | 3154 → 7007 | 9107ms → 1889ms |
| `llama3.1:8b` | S2 | 68 | 3154 → 6890 | 7877ms → 3457ms |
| `llama3.1:8b` | S3 | 68 | 3168 → 7196 | 6345ms → 1849ms |
| `llama3.1:8b` | S4 | 68 | 3370 → 7465 | 13286ms → 1861ms |
| `llama3.1:8b` | S5 | 68 | 3593 → 7388 | 12863ms → 1892ms |
| `llama3.1:8b` | S6 | 68 | 19040 → 23090 | 78226ms → 2352ms |
| `llama3.1:8b` | S7 | 68 | 3603 → 7633 | 9197ms → 1940ms |
| `llama3.2:3b` | S1 | 68 | 3154 → 7236 | 4372ms → 9176ms |
| `llama3.2:3b` | S2 | 68 | 3154 → 6710 | 5494ms → 1829ms |
| `llama3.2:3b` | S3 | 68 | 3168 → 7398 | 2468ms → 9509ms |
| `llama3.2:3b` | S4 | 68 | 3370 → 7571 | 5123ms → 9551ms |
| `llama3.2:3b` | S5 | 68 | 3593 → 7473 | 5023ms → 8740ms |
| `llama3.2:3b` | S6 | 68 | 19040 → 23306 | 35153ms → 11823ms |
| `llama3.2:3b` | S7 | 68 | 3603 → 7353 | 3522ms → 9325ms |
| `qwen3.5:0.8b` | S1 | 68 | 3154 → 7597 | 814ms → 1198ms |
| `qwen3.5:0.8b` | S2 | 68 | 3154 → 8131 | 726ms → 1301ms |
| `qwen3.5:0.8b` | S3 | 68 | 3168 → 8429 | 1033ms → 1446ms |
| `qwen3.5:0.8b` | S4 | 68 | 3370 → 8403 | 1197ms → 1237ms |
| `qwen3.5:0.8b` | S5 | 68 | 3593 → 7868 | 1269ms → 2687ms |
| `qwen3.5:0.8b` | S6 | 68 | 19040 → 23784 | 6915ms → 1654ms |
| `qwen3.5:0.8b` | S7 | 68 | 3603 → 8347 | 1078ms → 1243ms |
| `qwen3.5:2b` | S1 | 68 | 3154 → 8063 | 2410ms → 2855ms |
| `qwen3.5:2b` | S2 | 68 | 3154 → 8385 | 1766ms → 2792ms |
| `qwen3.5:2b` | S3 | 68 | 3168 → 8751 | 2093ms → 2916ms |
| `qwen3.5:2b` | S4 | 68 | 3370 → 8599 | 2853ms → 2897ms |
| `qwen3.5:2b` | S5 | 68 | 3593 → 8747 | 2984ms → 2906ms |
| `qwen3.5:2b` | S6 | 68 | 19040 → 24345 | 15203ms → 3159ms |
| `qwen3.5:2b` | S7 | 68 | 3603 → 8485 | 2852ms → 2833ms |
| `qwen3.5:4b` | S1 | 68 | 3154 → 8515 | 7287ms → 8174ms |
| `qwen3.5:4b` | S2 | 68 | 3154 → 8781 | 5810ms → 8286ms |
| `qwen3.5:4b` | S3 | 68 | 3168 → 8277 | 7269ms → 18454ms |
| `qwen3.5:4b` | S4 | 68 | 3370 → 8593 | 8391ms → 7857ms |
| `qwen3.5:4b` | S5 | 68 | 3593 → 8434 | 8763ms → 7655ms |
| `qwen3.5:4b` | S6 | 68 | 19040 → 24178 | 42946ms → 9143ms |
| `qwen3.5:4b` | S7 | 68 | 3603 → 8894 | 6576ms → 8303ms |
| `qwen3.5:9b` | S1 | 68 | 3154 → 8941 | 14108ms → 14886ms |
| `qwen3.5:9b` | S2 | 68 | 3154 → 8763 | 10723ms → 9992ms |
| `qwen3.5:9b` | S3 | 68 | 3168 → 8078 | 7496ms → 10031ms |
| `qwen3.5:9b` | S4 | 68 | 3370 → 8346 | 10151ms → 10335ms |
| `qwen3.5:9b` | S5 | 68 | 3593 → 8613 | 10951ms → 10891ms |
| `qwen3.5:9b` | S7 | 26 | 3603 → 12900 | 10963ms → 10570ms |

## Errors (476)

- 476x mistral:latest — http://127.0.0.1:11434/api/generate: status code 404

## Coverage gaps

Planned but not yet run:

- pass-b: 10582 cell(s)
