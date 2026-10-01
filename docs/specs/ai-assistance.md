# AI Assistance

Helpers that improve how tools are described and used. Nothing here runs a model or sends data
anywhere unless a section says so.

## Tool lint

Models choose tools by reading their names, descriptions, and parameters, so a vague or bloated
definition costs accuracy and context. The lint checks the tools of a connected server offline, with
heuristics, and shows the result on the server page ("Tool quality"). Findings are hints, not proof.

| Rule                   | Severity | Reported when                                                                                                                            |
| ---------------------- | -------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `missingDescription`   | error    | the tool has no description (or only blanks)                                                                                             |
| `vagueDescription`     | warning  | fewer than 4 words, only the words of the name (`list_issues`: "Lists the issues"), or a vague word such as _various_, _stuff_, _helper_ |
| `unknownRequired`      | error    | `required` names a parameter that is not declared                                                                                        |
| `missingRequired`      | warning  | the schema declares parameters but marks none as required                                                                                |
| `undescribedParameter` | info     | parameters without a description (listed in one finding per tool)                                                                        |
| `overlappingTools`     | warning  | two descriptions share at least 60 % of their distinctive words (one finding per pair)                                                   |
| `oversizedDefinition`  | warning  | one definition takes more than about 1200 estimated tokens                                                                               |
| `oversizedServer`      | warning  | all definitions together take more than about 12000 estimated tokens                                                                     |

Words are compared in lower case, split at `_`, `-` and camelCase, without common stop words and with
a crude stem (`lists`, `listed` and `list` are one word). Token counts are the offline estimates of the
inspector. The thresholds are constants in `crates/mcp-studio-core/src/lint.rs`.

## Tool documentation

For a connected server, **Documentation** on its page writes Markdown that can be previewed, copied,
or saved as a `.md` file. It needs no model and sends nothing anywhere; it is built from:

- **the tool definitions**: title and description (purpose), the parameters as a table (type, whether
  required, description, allowed values, default, range; nested objects as `parent.child`), the
  output schema, and the hints the server gives (read-only, destructive, ...)
- **the recorded calls** of the server in the history: up to 3 recent successful calls per tool with
  distinct arguments, shown with the arguments and the start of the result (400 characters), and the
  different error messages with how often and when they happened. History results are already
  masked and size limited when they are stored. Cancelled calls are ignored.
- **the lint** of the tool: a "Documentation gaps" list when the description is missing or vague

Tools are sorted by name and the text is deterministic for the same input apart from the date line,
so it can be kept in Git and diffed.

## Test suites

Suites are saved per server (**Suites** in the toolbar) and are the input of the variant comparison.
A suite has a name, an optional **system prompt** that is tested together with the tool descriptions,
and cases. A case holds:

- an **input**, written as a user would ask it ("Which issues are open in a/b?")
- an **expectation**: the model calls a given **tool** first, calls **no tool**, or **answers** with
  text that contains a given string (ignoring case)
- optional notes

Saving replaces the whole suite; cases that are sent back with their id keep it. A suite stays with
its server and is removed with it. A suite may have no cases yet. Names, inputs, and the tool name or
answer text are required.

## Variant comparison

The **Compare** page runs a test suite against variants of the system prompt and the tool
descriptions and shows accuracy and tokens side by side.

- A variant is a label, an optional system prompt, and replacement descriptions for named tools. The
  baseline is the server as it is.
- Each case is one model call with the server's tool definitions; tools are never called. A case
  passes when the first tool call (or answer) matches the expectation.
- **Propose variants** asks the model for three variants aimed at the cases the baseline fails.
  Proposals that name unknown tools are dropped. The tool definitions and failing cases are sent to
  the provider you chose.
- The table shows accuracy, passed cases, input and output tokens, and the estimated size of the tool
  definitions; the most accurate variant (then the cheapest) is marked. A matrix lists what the model
  did per case and variant. Proposed variants can be copied as Markdown.

## Flow generation

**Generate a flow from a goal** on the Flows page turns a sentence into a flow.

- You describe the goal and choose the servers whose tools the flow may use (they are connected on
  demand) and the model.
- One model call gets the goal, the flow format, and a short description of every tool (name,
  description, parameter names and types, required parameters). The goal and these definitions are
  sent to the provider of that model. Tools are only described, never called.
- The answer is parsed (unknown keys are errors) and validated with the same checks that run before a
  flow runs: servers, tools, required arguments, argument types, references, cycles. If there are
  problems, the model gets one chance to repair them (a second call).
- A flow without problems is saved to the library and opened in the graph editor. A flow with
  problems is **not** opened: the problems and the model's YAML are shown instead.
- Generating never runs a flow. Running it is a separate step and still asks before every tool call.
