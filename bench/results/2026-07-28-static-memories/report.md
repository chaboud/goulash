# Engine characterization — report card

996 rows recorded (804 sweep). Mechanical metrics only; qualitative
scores are joined from grades.jsonl after blind grading.

## Per model

`empty→stop` = truncated by the stop sequence before emitting anything.
`empty→budget` = spent the whole budget thinking. `mem-only` = replied
with a bare REMEMBER: line instead of answering. Load time is excluded
from latency (the first call per model pays it).

| model | provider | tier | p50 ttft | p50 total | answered | empty→stop | empty→budget | mem-only | fenced | 1-line | CMD: | reasoning tok |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `deepseek-r1:14b` | ollama | driver | 3554ms | 9740ms | 100% | 0% | 0% | 0% | 0% | 0% | 0% | - |
| `gemma3:12b` | ollama | driver | 4232ms | 7929ms | 100% | 0% | 0% | 0% | 0% | 100% | 100% | - |
| `gemma3:4b` | ollama | watcher | 1314ms | 2748ms | 98% | 0% | 0% | 2% | 0% | 100% | 30% | - |
| `gemma4:12b` | ollama | driver | 4577ms | 8512ms | 98% | 0% | 0% | 2% | 0% | 100% | 82% | - |
| `gemma4:12b-mlx` | ollama | driver | 5373ms | 8246ms | 98% | 0% | 0% | 2% | 0% | 100% | 88% | - |
| `gemma4:e2b` | ollama | driver | 705ms | 1428ms | 98% | 0% | 0% | 2% | 0% | 100% | 96% | - |
| `llama3.1:8b` | ollama | driver | 1764ms | 2648ms | 98% | 0% | 0% | 2% | 0% | 100% | 35% | - |
| `llama3.2:3b` | ollama | watcher | 771ms | 1585ms | 100% | 0% | 0% | 0% | 0% | 98% | 35% | - |
| `mistral-nemo:latest` | ollama | driver | 2977ms | 4349ms | 93% | 0% | 0% | 7% | 0% | 98% | 72% | - |
| `mistral:latest` | ollama | driver | 2236ms | 4063ms | 98% | 0% | 0% | 2% | 2% | 98% | 82% | - |
| `qwen3.5:0.8b` | ollama | watcher | 1335ms | 1843ms | 100% | 0% | 0% | 0% | 0% | 100% | 53% | - |
| `qwen3.5:2b` | ollama | watcher | 3262ms | 4095ms | 96% | 0% | 0% | 4% | 0% | 100% | 37% | - |
| `qwen3.5:4b` | ollama | watcher | 8770ms | 10808ms | 100% | 0% | 0% | 0% | 0% | 100% | 95% | - |
| `qwen3.5:9b` | ollama | driver | 17018ms | 20337ms | 98% | 0% | 0% | 2% | 0% | 98% | 81% | - |
| `qwen3:14b` | ollama | driver | 5159ms | 12602ms | 100% | 0% | 0% | 0% | 0% | 100% | 100% | - |

## Per prompt shape

S1 = shipped (memories before log) · S2 = memories in suffix · S3 = CMD: first

| shape | p50 ttft | p50 total | p50 prompt-eval | empty | 1-line | CMD: |
|---|---|---|---|---|---|---|
| S1 | 2973ms | 5062ms | 2606ms | 1% | 93% | 56% |
| S2 | 3857ms | 5086ms | 3729ms | 2% | 92% | 53% |
| S3 | 2673ms | 4800ms | 2531ms | 2% | 92% | 81% |

## Cache behaviour

Ollama only — LM Studio exposes no prompt-eval timing, so its cache
evidence is TTFT flatness in the per-turn table instead.

| model | shape | turns | prompt chars (first→last) | prompt-eval (first→last) |
|---|---|---|---|---|
| `deepseek-r1:14b` | S1 | 19 | 1624 → 4836 | 8619ms → 2472ms |
| `deepseek-r1:14b` | S2 | 19 | 1624 → 4836 | 9022ms → 4145ms |
| `deepseek-r1:14b` | S3 | 19 | 1628 → 4840 | 1801ms → 2351ms |
| `gemma3:12b` | S1 | 19 | 1624 → 6440 | 6925ms → 2739ms |
| `gemma3:12b` | S2 | 19 | 1624 → 6470 | 8264ms → 4210ms |
| `gemma3:12b` | S3 | 19 | 1628 → 6353 | 8563ms → 2762ms |
| `gemma3:4b` | S1 | 19 | 1624 → 6132 | 1847ms → 845ms |
| `gemma3:4b` | S2 | 19 | 1624 → 6063 | 2602ms → 1164ms |
| `gemma3:4b` | S3 | 19 | 1628 → 5879 | 2413ms → 728ms |
| `gemma4:12b` | S1 | 19 | 1624 → 6815 | 7811ms → 2853ms |
| `gemma4:12b` | S2 | 19 | 1624 → 6733 | 8565ms → 4863ms |
| `gemma4:12b` | S3 | 19 | 1628 → 6605 | 8606ms → 2945ms |
| `gemma4:12b-mlx` | S1 | 19 | 1624 → 6649 | 8067ms → 3512ms |
| `gemma4:12b-mlx` | S2 | 19 | 1624 → 6681 | 8749ms → 4610ms |
| `gemma4:12b-mlx` | S3 | 19 | 1628 → 6418 | 7273ms → 3234ms |
| `gemma4:e2b` | S1 | 19 | 1624 → 6093 | 1036ms → 495ms |
| `gemma4:e2b` | S2 | 19 | 1624 → 6121 | 1666ms → 763ms |
| `gemma4:e2b` | S3 | 19 | 1628 → 6447 | 1642ms → 519ms |
| `llama3.1:8b` | S1 | 19 | 1624 → 5729 | 3424ms → 1068ms |
| `llama3.1:8b` | S2 | 19 | 1624 → 5532 | 3641ms → 1899ms |
| `llama3.1:8b` | S3 | 19 | 1628 → 6117 | 3605ms → 1206ms |
| `llama3.2:3b` | S1 | 19 | 1624 → 6560 | 1417ms → 464ms |
| `llama3.2:3b` | S2 | 19 | 1624 → 7035 | 1608ms → 816ms |
| `llama3.2:3b` | S3 | 19 | 1628 → 6506 | 1782ms → 425ms |
| `mistral-nemo:latest` | S1 | 19 | 1624 → 5381 | 6536ms → 3211ms |
| `mistral-nemo:latest` | S2 | 19 | 1624 → 5581 | 6591ms → 3722ms |
| `mistral-nemo:latest` | S3 | 19 | 1628 → 5278 | 6609ms → 1789ms |
| `mistral:latest` | S1 | 19 | 1624 → 6277 | 4137ms → 1401ms |
| `mistral:latest` | S2 | 19 | 1624 → 5943 | 4350ms → 2156ms |
| `mistral:latest` | S3 | 19 | 1628 → 6347 | 4408ms → 1314ms |
| `qwen3.5:0.8b` | S1 | 19 | 1624 → 6604 | 356ms → 1715ms |
| `qwen3.5:0.8b` | S2 | 19 | 1624 → 7485 | 394ms → 1901ms |
| `qwen3.5:0.8b` | S3 | 19 | 1628 → 6393 | 387ms → 1713ms |
| `qwen3.5:2b` | S1 | 19 | 1624 → 6530 | 773ms → 3744ms |
| `qwen3.5:2b` | S2 | 19 | 1624 → 6464 | 1189ms → 3624ms |
| `qwen3.5:2b` | S3 | 19 | 1628 → 7634 | 832ms → 4591ms |
| `qwen3.5:4b` | S1 | 19 | 1624 → 7562 | 2350ms → 12140ms |
| `qwen3.5:4b` | S2 | 19 | 1624 → 7660 | 2920ms → 12167ms |
| `qwen3.5:4b` | S3 | 19 | 1628 → 7324 | 3063ms → 11331ms |
| `qwen3.5:9b` | S1 | 19 | 1624 → 7287 | 4796ms → 22936ms |
| `qwen3.5:9b` | S2 | 19 | 1624 → 7360 | 5316ms → 22336ms |
| `qwen3.5:9b` | S3 | 19 | 1628 → 7770 | 5401ms → 23223ms |
| `qwen3:14b` | S1 | 6 | 1624 → 3402 | 8676ms → 6831ms |
