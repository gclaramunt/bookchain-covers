
# bookchain-covers

Download high-res covers for a specific asset

## Compile

Compile the code with `cargo build`.

## Configuration

This utility uses blockfrost api for cardano networks access.
You need to provide the project id  `.blockfrost.toml` file.
E.g.

```toml
project_id="<cardano project id>"
```

## Run

After building, the code can be run with `book_cli <parameters>` (e.g. `target/debug/book_cli`) or `cargo run -- <parameters>`

### Parameters

Usage: `book_cli <policy_id> <work_dir>? <total_files>?`

* policy_id (mandatory): policy id of the asset
* work_dir (optional): directory where to store the files (default: current directory)
* total_files (optional): maximum number of files to download (default: 10)

### Execution

First the policy id is validated against the book.io collection, then the policy assets metadata is fetched from cardano through cloudfrost api.
From the metadata, it extracts the ipfs CID of the cover image and if is not already present or the same image already exists, downloads it from the ipfs network, repeating the process until the specified amount of images have been downloaded.
