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

## YAML format (draft)

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

## Execution rules

- Every run is a trace; every step is a span (see
  [recording-and-observability.md](recording-and-observability.md)).
- Tool calls — from `tool` steps or from tools exposed to an `llm` step — require confirmation by
  default. Confirmation can be relaxed per server and per tool.
- A run is reproducible: inputs, resolved prompts, model, and tool results are stored, and a run can
  be replayed with recorded tool results instead of live calls.
- Validation before a run: every referenced server and tool exists, and templated arguments match the
  tool's input schema.
