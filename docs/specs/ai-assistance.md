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
