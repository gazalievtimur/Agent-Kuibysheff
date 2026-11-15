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
The file in/huge.log is large. Use home.read with offset/max_chars windows
(follow next_offset when truncated) to find the SECRET_TOKEN= line and put
only that token value (SF_NEEDLE_...) in result.
```
