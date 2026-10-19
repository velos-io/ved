# Nginx & Postgres Example (Shorthand Docstring)

A minimal two-tier application stack defined entirely inside a shorthand docstring (`"""..."""`) within `Vedfile`.

This specification demonstrates the deterministic shorthand notation implemented in T19:
- `Service <name> <image> [<port>]`
- `<name1> -> <name2>` (dependency order with healthy condition)
- `<group> (<s1>, <s2>)` (profile/group membership)
- `<name> -- <health>` (health check probe)
- `<name>= (<key>=<val>, ...)` (environment variable mapping)

## Usage

Start the stack directly using the shorthand Vedfile:

```bash
ved up -t compose
```

Inspect status and URLs:

```bash
ved ps -t compose
```

Stop the stack:

```bash
ved down -t compose
```

## Bootstrap into Standard Starlark Vedfile

To expand the shorthand specification into a full idiomatic Starlark `Vedfile`:

```bash
ved init
```
