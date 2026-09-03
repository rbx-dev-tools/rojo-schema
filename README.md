# rojo-schema

JSON Schemas for the file formats Rojo reads, compiled from Rojo's own source
rather than written by hand.

| Schema                                  | Applies to                                |
| --------------------------------------- | ----------------------------------------- |
| `schema/project.schema.json`             | `*.project.json`                          |
| `schema/meta.schema.json`                | `*.meta.json`, including `init.meta.json` |
| `schema/model.schema.json`               | `*.model.json`                            |
| `schema/input-action-system.schema.json` | an input action tree, opted into by name  |

The first three describe a format as Rojo reads it. The fourth is different in
kind: it narrows the model format to one purpose, and is described under
[Input action trees](#input-action-trees).

`schema/manifest.json` records which Rojo release the schemas were compiled
from, the reflection database behind the input action schema, the digest of
every source file that fed them, and the digest of each schema.

## Using them

Point a file at its schema:

```json
{
  "$schema": "https://raw.githubusercontent.com/rbx-forge/rojo-schema/main/schema/project.schema.json",
  "name": "my-game",
  "tree": { "$className": "DataModel" }
}
```

Rojo declares `$schema` as a real field on all three formats, so this does not
break parsing. Editors can also be configured by file pattern, which is the
better option for `.meta.json` and `.model.json` files.

That URL tracks `main`, so it follows Rojo. To freeze a project on the Rojo
release it actually runs, swap `main` for the matching tag:

```
https://raw.githubusercontent.com/rbx-forge/rojo-schema/rojo-7.7.0/schema/project.schema.json
```

### Letting your editor fetch it

VS Code downloads schemas only from an allowlist of URL prefixes, and it ships
with two entries, both under Microsoft's own repositories. Any other URL is
refused with `Location ... is untrusted`, whatever the host. Add this once, in
the workspace `.vscode/settings.json` so it travels with the repository:

```json
"json.schemaDownload.trustedDomains": {
  "https://raw.githubusercontent.com/rbx-forge/rojo-schema/": true
}
```

It is a prefix, not a domain, so this grants nothing beyond this repository.

Use a `raw.githubusercontent.com` URL rather than a release asset:
`github.com/.../releases/download/...` redirects to a signed host, which cannot
be allowlisted by prefix at all.

## Input action trees

`input-action-system.schema.json` validates a `.model.json` holding one
`InputContext`, the `InputAction` instances under it, and the `InputBinding`
instances under those. It is the narrow schema: point a file at it instead of
the model schema and an editor will complete key codes, refuse a class that has
no business in the tree, and catch a binding that contradicts its action.

```json
{
  "$schema": "https://raw.githubusercontent.com/rbx-forge/rojo-schema/main/schema/input-action-system.schema.json",
  "ClassName": "InputContext",
  "Children": [
    {
      "ClassName": "InputAction",
      "Name": "Move",
      "Properties": { "Type": "Direction2D" },
      "Children": [
        {
          "ClassName": "InputBinding",
          "Name": "Keyboard",
          "Properties": { "Forward": "W", "Backward": "S", "Left": "A", "Right": "D" }
        }
      ]
    }
  ]
}
```

What it enforces beyond the model schema:

- **One context per file, and one nesting order.** The root is an
  `InputContext`; only `InputAction` sits under it, only `InputBinding` under
  those, and nothing under a binding, which has no children key at all.
- **Only the properties a file can set.** The read-only members are dropped, so
  `BoolState` and the direction states of an `InputAction` are refused: they are
  what the engine reports, not what a file asks for. Nothing is listed by hand.
  The reflection database says which properties are read-only, non-serialising
  or invisible to scripts, and those are the ones that go.
- **Property values typed the way Rojo resolves them.** `Priority` is a number,
  `KeyCode` is a member of `Enum.KeyCode` by name, `Vector2Scale` is two
  numbers. Rojo's explicit form, `{ "Vector3": [0, 1, 0] }`, stays accepted
  everywhere. A `Ref` property has no shorthand at all, so only the explicit
  form is offered for it.
- **Unknown keys.** Rojo ignores a key it does not recognise. This schema
  refuses it, because a file opts into this schema to have its typos caught.

One thing here is written by hand, because Roblox publishes it nowhere as data:
the three classes and how they nest. Everything else, including every enum
member, comes out of the reflection database bundled with
`rbx_reflection_database`, so the schema follows Roblox rather than a snapshot
of it.

What the schema does **not** check is which binding properties suit the `Type`
of the action above them. A `Vector3Scale` under a `Direction2D` action passes,
and so does a `KeyCode` on a binding whose own `Type` is `Scriptable`. Roblox
documents none of this: the members of `InputActionType` and `InputBindingType`
carry no descriptions, and the input action guide does not tie a binding
property to an action type. Encoding it would mean writing a table of rules with
nothing to check it against, and a schema that wrongly rejects a valid file is
worse than one that stays quiet, because the first thing anyone does with it is
turn it off. It stays out until Roblox says.

This schema moves with the reflection database rather than with Rojo, so it can
change in a release where the other three do not.

## Releases

Every distinct set of schemas gets its own release, named after the Rojo release
it describes: `rojo-7.7.0`. Each one carries every schema and the manifest as
assets, and the notes state the Rojo tag, the generator version and the digest
of each file. The tag is what a project pins against, through the raw URL
above; the assets are there for anything that downloads rather than fetches.

Releases are immutable. If the compiler itself changes and produces different
schemas from the same Rojo release, the next snapshot is `rojo-7.7.0-r2`, and so
on. A change that leaves the schemas identical publishes nothing.

Two jobs keep this moving without anyone watching Rojo:

- **Track Rojo** runs daily. When Rojo publishes a release, it re-vendors the
  sources at that tag, recompiles, runs the full check suite and opens a pull
  request carrying the grammar diff. If the release broke the compiler, the job
  fails and no pull request appears, which is the intended outcome: a human
  looks at what moved upstream.
- **Release** runs when `schema/` or `vendor.toml` lands on `main`, and cuts the
  snapshot described above.

## How it is built

The grammar is not transcribed, it is read:

1. `vendor/` holds the Rojo source files that declare the formats, copied
   verbatim from a release tag. They are never compiled, only parsed, which is
   what lets them stay byte for byte identical to upstream.
2. `vendor.toml` pins the tag and a SHA-256 per file.
3. `syn` parses those files into an AST, and the serde attributes on them are
   read the way serde would: `rename`, `rename_all`, `alias`, `default`, `skip`,
   `skip_deserializing`, `flatten`, `untagged`, `deny_unknown_fields`.
4. The doc comments Rojo's authors wrote become the schema descriptions.

The result is that a field Rojo adds, renames or deletes moves the schema with
it, and every description is Rojo's own wording rather than a paraphrase.

Nothing is inferred silently. A type the compiler cannot describe, a container
that disappeared upstream, an enum shape it does not model: each is an error
that stops the build.

## Following a new Rojo release

The Track Rojo job does this on its own and opens a pull request. By hand, it is
the same four commands:

```sh
cargo run -- vendor --tag v7.8.0   # re-copies vendor/ and repins the digests
cargo run -- generate              # recompiles schema/
git diff schema/                   # read what changed in the grammar
cargo test
```

Three outcomes are possible, and they are meant to be distinguishable:

- **The diff is empty.** The release did not touch the formats.
- **The diff shows new or changed fields.** That is the release's grammar
  change, in Rojo's own words. Commit it.
- **`generate` fails.** A type moved out of a vendored file, or grew a shape
  the compiler does not model. The error names the container. Fix `vendor.toml`
  or the compiler, never the vendored file.

The file list itself is version-dependent: `src/syncback/mod.rs` did not exist
before 7.7.0, for instance. `vendor` says so plainly when a pinned path is
absent at the requested tag, which matters mostly when pinning an older release
on purpose.

`cargo run -- check` is the read-only form: it re-hashes `vendor/`, recompiles
twice to prove the output is deterministic, and compares against what is
committed. CI runs it, so a vendored file edited by hand and a schema left
stale both fail loudly.

## What these schemas do not do

- **Property values are not typed per class.** `$properties` accepts any value
  Rojo would resolve, but the project, meta and model schemas do not know that
  `Workspace.Gravity` is a number. Typing every class means pulling in Roblox's
  reflection database, a second source that moves on its own schedule, and it
  stays out of scope for those three: a schema compiled from Rojo alone is one
  that follows Rojo alone. The input action schema is the deliberate exception,
  narrow enough to be worth the second source.
- **Comments are a parser concern, not a schema one.** Rojo reads all three
  formats as JSONC, so comments and a `.jsonc` extension are fine. A validator
  has to strip them before validating, as editors already do.
- **`className` in a `.meta.json`.** Rojo only acts on it inside an
  `init.meta.json`, but it ignores unknown fields elsewhere rather than
  rejecting them, so the schema accepts it in both and says so in the field
  description.
- **`$path` cannot be resolved.** JSON Schema cannot know what class a
  filesystem path produces, so the constraints Rojo enforces between `$path` and
  `$className` are documented in the descriptions and not enforced here.

## Layout

```
vendor/        Rojo sources, verbatim, pinned by vendor.toml
src/ty.rs      the subset of Rust types the grammar is written in
src/ir.rs      syn AST to a serde-aware intermediate form
src/emit.rs    intermediate form to JSON Schema
src/vendor.rs  the pin file, its digests, and refreshing it from a tag
src/reflection.rs  the input action schema, compiled from the reflection database
schema/        the generated documents, committed
tests/         fixtures Rojo accepts and fixtures it rejects
```

Point `ROJO_SCHEMA_CORPUS` at a real Rojo project to validate every
`.project.json`, `.meta.json` and `.model.json` under it as part of the test
run. Nothing from that corpus is committed here.

## License

This project is [MPL-2.0](./LICENSE), the same license Rojo uses.

The files under `vendor/` are verbatim copies of Rojo's source, redistributed
unmodified and remaining the work of the Rojo authors. See
[THIRD-PARTY-NOTICES.md](./THIRD-PARTY-NOTICES.md) for the file list and the
tag they were taken from.
