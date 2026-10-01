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

## Running a flow

Steps run in order. A `condition` may jump **forward** to the step named by `then` or `else`; the
steps it jumps over are recorded as skipped, and a jump backwards is a validation error, so a flow
cannot loop forever. Without a target the next step runs. A step that fails stops the run.

Every step can use `inputs.<name>` and `steps.<id>.<field>` in `{{ ... }}`. Expressions support
paths (`steps.a.items[0].title`), literals, `== != < <= > >=`, `&& || !`, parentheses and the
functions `len(x)` and `contains(haystack, needle)`. A path that does not exist is an error, so a
typo is found at once. Text that is exactly one `{{ expression }}` keeps its type (a number stays a
number), so `limit: "{{ inputs.limit }}"` passes a number.

| Step        | Output (`steps.<id>.…`)                                            |
| ----------- | ------------------------------------------------------------------ |
| `input`     | the input values; each declared input needs a value of its type    |
| `tool`      | `result` (text), `content`, `structured`, `isError`                |
| `llm`       | `text`, `model`, `usage`, `calls` (the tool calls the model made)  |
| `condition` | `value`                                                            |
| `transform` | one entry per `values` key                                         |
| `output`    | one entry per `outputs` key; these also form the result of the run |

An `llm` step with `tools` is an agent: the model may call those tools, up to 10 rounds, and gets
the results back before it answers. Tool names are shown to the model as `server__tool`.

## Execution rules

- **Nothing runs without consent.** Before every tool call, from a `tool` step or asked for by a
  model, the user decides: allow once, always allow the tool, always allow the server, or deny.
  Tools and servers that were allowed are listed on the Flows page and can be removed there. A
  denied call never runs: a `tool` step fails, and a model is told that the user said no.
- Before a run starts, the flow is validated against the servers and tools that are connected (they
  are connected on demand). An invalid flow does not run at all.
- Every run is a trace: a `flow` span, a span per step, and a `tool` span for every tool call (see
  [recording-and-observability.md](recording-and-observability.md)). LLM spans carry the exact
  token usage the provider reported.
- A run is reproducible: the flow as it was, the inputs, the resolved prompts and arguments, every
  step's output, and every tool call with its result are stored. A run can be cancelled; the tool
  call in progress is cancelled on the server too.
- A run can be replayed with recorded tool results instead of live calls (**Replay** in the list
  of earlier runs). Templates, conditions and model calls run live; tool calls are answered from
  the record by server and tool, in the order they were recorded, even if the replayed flow passes
  other arguments. No tool is called, so nothing asks for confirmation and no server needs to be
  connected. A call without a recorded result left, or one that was denied or never returned a
  result in the original run, fails the step. A replay can use the flow as it was or the current
  version from the library, which shows what a changed prompt, model, or flow would have done with
  exactly the same tool data. A replay is a run of its own: it is stored, traced, and can be
  replayed again.
- Validation before a run: every referenced server and tool exists, and templated arguments match the
  tool's input schema.
