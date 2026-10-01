# Prompt Flows

A flow is a directed graph of steps that combines LLM calls and MCP tool calls across several
servers. The visual editor (`@foblex/flow`) and the YAML file are two views of the same graph.

## Step types

| Type        | Purpose                                                                              |
| ----------- | ------------------------------------------------------------------------------------ |
| `input`     | Declares the flow's input variables                                                  |
| `llm`       | Calls a model with a prompt template; may expose MCP tools to the model (agent step) |
| `tool`      | Calls one MCP tool with templated arguments                                          |
| `condition` | Branches on an expression over variables                                             |
| `transform` | Maps or extracts values (JSONPath / template)                                        |
| `output`    | Declares the flow's result                                                           |

## YAML format

```yaml
version: 1
name: summarize-open-issues
inputs:
  repo: { type: string }
steps:
  - id: issues
    type: tool
    server: github
    tool: list_issues
    arguments:
      repo: "{{ inputs.repo }}"
      state: open
  - id: summary
    type: llm
    model: claude-sonnet-5-5
    prompt: |
      Summarize these issues in five bullets:
      {{ steps.issues.result }}
outputs:
  summary: "{{ steps.summary.text }}"
```

### Rules of the format

- The YAML file and the graph are two views of the same flow; converting in either direction loses
  nothing (`from_yaml(to_yaml(flow)) == flow`, covered by tests).
- `inputs:` and `outputs:` at the top level are shorthand for an `input` step with the id `inputs`
  at the start and an `output` step with the id `outputs` at the end. A flow whose input or output
  step has another id or another position is written with explicit `type: input` / `type: output`
  steps. A step with the id `inputs` or `outputs` cannot be combined with the matching shorthand.
- Unknown keys are errors, not ignored, so a typo such as `argumnets:` cannot silently drop part of
  a flow. `version` must be `1`.
- Empty optional fields (`system`, `tools`, `then`, `else`, empty maps) are left out of the file and
  read back as empty.
- Importing adds a flow to the library and never overwrites one: a name that is taken gets a
  number (`my-flow (2)`). Reading a file does not check servers and tools; that is validation.

## Models and providers

The `model` of an `llm` step names the provider with a prefix. Keys live in the OS keyring and are
only sent to their provider; the addresses are set on the **Providers** page.

| `model`                     | Provider                                                     |
| --------------------------- | ------------------------------------------------------------ |
| `claude-sonnet-5-5`         | Anthropic (a name without a known prefix means Anthropic)    |
| `anthropic:claude-opus-5-5` | Anthropic                                                    |
| `openai:gpt-4o`             | an OpenAI-compatible endpoint (OpenAI, LM Studio, vLLM, ...) |
| `ollama:llama3.1:8b`        | local Ollama; only the first colon is a prefix, tags stay    |

All providers offer the same to flows: a completion or a stream, tool use in both directions, and
the exact `usage` the provider reports.

## Execution rules

- Every run is a trace; every step is a span (see
  [recording-and-observability.md](recording-and-observability.md)).
- Tool calls — from `tool` steps or from tools exposed to an `llm` step — require confirmation by
  default. Confirmation can be relaxed per server and per tool.
- A run is reproducible: inputs, resolved prompts, model, and tool results are stored, and a run can
  be replayed with recorded tool results instead of live calls.
- Validation before a run: every referenced server and tool exists, and templated arguments match the
  tool's input schema.
