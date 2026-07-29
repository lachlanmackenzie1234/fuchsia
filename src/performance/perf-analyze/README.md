# Fuchsia Standalone Performance Analysis Tool (`perf-analyze`)

`perf-analyze` is a host-side tool designed to perform standalone, automated, and human-interactive performance analysis on Fuchsia traces. The tool acts as a CLI orchestrator that delegates trace processing to specialized analysis plugins.

---

## Subcommands

1.  **`query`**: Executes SQL queries to inspect and extract structured data from traces. Backed by Perfetto's Trace Processor.
2.  **`analyze`** *(Planned)*: Merges, summarizes, and processes structured query output to identify performance anomalies (e.g. jank, CPU starvation).
3.  **`visualize`** *(Planned)*: Generates HTML reports, flamegraphs, SVGs, or deep-link trampoline URLs for the Perfetto UI.

---

## Global Options

*   **`--format <json|markdown|text>`**: Specifies the output format (default: `text`).
    *   `text`: Unformatted tab-separated values (TSV), ideal for shell scripting.
    *   `markdown`: Formatted Markdown tables, ideal for doc insertion.
    *   `json`: Structured JSON, ideal for automated tool ingestion.

---

## The `query` Subcommand

The `query` subcommand executes SQL queries against a trace file using Perfetto's Trace Processor.

### Arguments

*   **`--trace <path_or_url>`** *(Required)*: File path to a local `.fxt` trace file or a URL to a remote trace.
*   **`--sql <query_string>`**: Executes a single raw SQL query. Mutually exclusive with `--batch`.
*   **`--batch <json_string_or_@filepath>`**: Executes a JSON array of queries in the format `[{"name": "query_name", "sql": "select ..."}, ...]`. Use `@filepath` to read the array from a local file. Mutually exclusive with `--sql`.

---

## Examples

#### 1. Print query help
```shell
fx perf-analyze query --help
```
*Output:*
```
usage: perf-analyze query [-h] --trace TRACE (--sql SQL | --batch BATCH)

options:
  -h, --help     show this help message and exit
  --trace TRACE  Trace file path or URL
  --sql SQL      SQL query to run
  --batch BATCH  Batch JSON or @file
```

#### 2. Run a single SQL query (default TSV format)
```shell
fx perf-analyze query \
  --trace src/performance/perf-analyze/test-data/sample_fxt.fxt \
  --sql "select count(*) as cnt from slice"
```
*Output:*
```
cnt
520
```

#### 3. Run a single SQL query (Markdown format)
```shell
fx perf-analyze --format markdown query \
  --trace src/performance/perf-analyze/test-data/sample_fxt.fxt \
  --sql "select count(*) as cnt from slice"
```
*Output:*
```
| cnt |
| --- |
| 520 |
```

#### 4. Run a batch of queries from a file (JSON format)
```shell
fx perf-analyze --format json query \
  --trace src/performance/perf-analyze/test-data/sample_fxt.fxt \
  --batch @src/performance/perf-analyze/test-data/sample_queries.json
```
*Output:*
```json
[
  {
    "name": "slice_count",
    "results": [
      {
        "cnt": 520
      }
    ]
  },
  {
    "name": "process_count",
    "results": [
      {
        "cnt": 2
      }
    ]
  }
]
```

---

## Running Tests

Unit tests are written using standard Python `unittest`. To build and run all unit tests, execute:

```shell
fx test perf_analyze_test
```
