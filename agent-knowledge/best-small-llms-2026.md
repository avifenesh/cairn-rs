# Best Small/Local LLMs for cairn-rs (2025-2026)

Researched 2026-04-05. Focus: models in the 1B-14B range runnable on consumer
hardware via Ollama, suitable for cairn-rs integration.

---

## Tier 1 -- Best All-Round (Recommended)

### Qwen 3 (Alibaba, 2025)
- **Sizes**: 0.6B, 1.7B, 4B, 8B, 14B, 30B, 32B
- **Context**: 32K native (131K with YaRN); 4B/30B variants have 256K
- **Strengths**: Reasoning, code generation, tool/function calling, 100+ languages
- **License**: Apache 2.0
- **Ollama**: `ollama pull qwen3` (all sizes available)
- **Why cairn-rs**: Best reasoning-per-parameter in the small range. The 8B and 14B
  hit a sweet spot for local coding/agentic tasks. Native tool-calling support
  makes it ideal for structured agent workflows.

### Gemma 4 (Google DeepMind, April 2026)
- **Sizes**: E2B (2.3B), E4B (4.5B), 26B-MoE (3.8B active), 31B dense
- **Context**: 128K (edge), 256K (workstation models)
- **Strengths**: Reasoning, multimodal (text+image+audio), configurable thinking
  modes, function calling, agentic workflows
- **License**: Apache 2.0
- **Ollama**: `ollama pull gemma4` (all variants)
- **Why cairn-rs**: The 26B MoE only activates 3.8B params at inference -- runs
  like a 4B model but reasons like a 26B. Edge models (E2B/E4B) are viable on
  laptops. Thinking mode toggle is useful for agent reasoning steps.

### Gemma 3 (Google, 2025)
- **Sizes**: 270M, 1B, 4B, 12B, 27B
- **Context**: 32K (270M/1B), 128K (4B+)
- **Strengths**: Multimodal (4B+), 140+ languages, QAT variants for 3x memory savings
- **License**: Gemma license (permissive, allows commercial use)
- **Ollama**: `ollama pull gemma3`
- **Why cairn-rs**: The 1B and 4B are extremely efficient for lightweight tasks.
  QAT variants let the 12B run in ~4GB RAM.

---

## Tier 2 -- Strong Specialists

### Qwen 3.5 (Alibaba, March 2026)
- **Sizes**: 0.8B, 2B, 4B, 9B, 27B, 35B, 122B (MoE variants)
- **Context**: 256K across all sizes
- **Strengths**: Multimodal (vision), 201 languages, Gated Delta Networks + sparse
  MoE for efficient inference
- **License**: Apache 2.0 (expected, same as Qwen3)
- **Ollama**: `ollama pull qwen3.5`
- **Why cairn-rs**: If you need vision capabilities (reading screenshots, diagrams)
  alongside text reasoning. The 9B is the default and a strong pick.

### Qwen 2.5 Coder (Alibaba, 2025)
- **Sizes**: 0.5B, 1.5B, 3B, 7B, 14B, 32B
- **Context**: 32K
- **Strengths**: Code generation, code repair, code reasoning; 40+ programming
  languages; 32B matches GPT-4o on code benchmarks
- **License**: Apache 2.0
- **Ollama**: `ollama pull qwen2.5-coder`
- **Why cairn-rs**: Best pure-coding model in the small range. The 7B and 14B are
  excellent for code-centric agent tasks.

### Phi-4 (Microsoft, Dec 2024)
- **Sizes**: 14B only
- **Context**: 16K
- **Strengths**: Math (80.4 MATH), code (82.6 HumanEval), reasoning, MMLU 84.8
- **License**: MIT
- **Ollama**: `ollama pull phi4`
- **Why cairn-rs**: MIT license is maximally permissive. Strong reasoning for 14B.
  Limitation: only 16K context is tight for long agent conversations.

### Phi-4-mini (Microsoft, 2025)
- **Sizes**: 3.8B
- **Context**: 128K
- **Strengths**: Math, reasoning, multilingual, function calling
- **License**: MIT
- **Ollama**: `ollama pull phi4-mini`
- **Why cairn-rs**: The 128K context in a 2.5GB model is remarkable. Good for
  lightweight agent tasks where long context matters more than peak quality.

---

## Tier 3 -- Viable Alternatives

### Mistral Small 3 (Mistral AI)
- **Sizes**: 24B
- **Context**: 32K (22B variant: 128K)
- **Strengths**: Agentic capabilities, native function calling, JSON output,
  multilingual, strong system prompt adherence
- **License**: Apache 2.0
- **Ollama**: `ollama pull mistral-small`
- **Why cairn-rs**: Good agentic/function-calling model but 24B is on the heavier
  side for consumer hardware. Best if you have 16GB+ VRAM.

### Mistral Nemo (Mistral + NVIDIA, 2025)
- **Sizes**: 12B
- **Context**: 128K
- **Strengths**: Reasoning, world knowledge, coding; drop-in replacement for
  Mistral 7B
- **License**: Apache 2.0
- **Ollama**: `ollama pull mistral-nemo`
- **Why cairn-rs**: Solid 12B all-rounder with huge context window.

### Llama 4 Scout (Meta, 2025)
- **Sizes**: 109B total, 17B active (MoE)
- **Context**: 10M tokens
- **Strengths**: Multimodal, multilingual (12 languages), code generation
- **License**: Llama license (permissive with usage limits)
- **Ollama**: `ollama pull llama4` (67GB download)
- **Why cairn-rs**: The 10M context is extraordinary but the 67GB download and
  memory requirements push it beyond typical consumer hardware.

### Llama 3.2 (Meta, 2024)
- **Sizes**: 1B, 3B
- **Context**: 128K
- **Strengths**: Compact, good for simple tasks, summarization
- **License**: Llama license
- **Ollama**: `ollama pull llama3.2`
- **Why cairn-rs**: Ultra-lightweight option for simple extraction/classification.

---

## Practical Recommendations for cairn-rs

| Use Case | Recommended Model | Ollama Command |
|---|---|---|
| Default agent reasoning | Qwen 3 8B | `ollama pull qwen3:8b` |
| Code generation/repair | Qwen 2.5 Coder 14B | `ollama pull qwen2.5-coder:14b` |
| Budget/laptop inference | Gemma 4 E4B or Qwen 3 4B | `ollama pull gemma4:e4b` |
| Long-context tasks | Phi-4-mini (128K) | `ollama pull phi4-mini` |
| Multimodal (vision) | Qwen 3.5 9B | `ollama pull qwen3.5:9b` |
| Max quality, single GPU | Gemma 4 26B-MoE | `ollama pull gemma4:26b` |
| Permissive license only | Phi-4 (MIT) | `ollama pull phi4` |

### Hardware Guidelines
- **8GB RAM/VRAM**: Models up to ~4B (Gemma 4 E4B, Qwen 3 4B, Phi-4-mini)
- **16GB RAM/VRAM**: Models up to ~14B (Qwen 3 14B, Phi-4, Gemma 3 12B)
- **24GB+ VRAM**: Gemma 4 26B-MoE, Mistral Small, Qwen 3 32B
- Quantized (Q4) variants reduce requirements by ~50-60%

### Key Takeaway
Qwen 3 and Gemma 4 are the clear leaders for 2026 local LLM usage. Both are
Apache 2.0, both have excellent Ollama support, and both offer strong
reasoning and tool-calling capabilities across multiple size points. For
cairn-rs, defaulting to **Qwen 3 8B** for general agent work and **Gemma 4
26B-MoE** when more reasoning power is needed covers most scenarios well.
