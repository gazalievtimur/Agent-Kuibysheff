# Example prompts (Scale-FS)

## Workspace search

```text
In the workspace corpus/, find the clearance code for Project Orion.
Put only that code in result.
```

## Large home file

```text
Read in/app.log and extract the TRACE_ID near the end of the file.
Put only that TRACE_ID in result.
```

## Oversize home file

```text
The file in/huge.log is too large for home.read. Use home.run with python to
extract the SECRET_TOKEN line. Put only that token in result.
```
