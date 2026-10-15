# The Character Animator project store

Reverse engineered from sample files for interoperability. Every statement below was checked against
`Boy.puppet`, `blood_cells.puppet` and a full 3 MB project (`news anchors`): 9 579 records, every object id verified.

## Where it lives

A `.puppet` zip and a project folder both contain `Ch Data/repo.noindex/`:

```
repo.noindex/
  HEAD                  "ref: refs/heads/main"          (stale; see below)
  config, description   plain git bare-repo boilerplate
  hooks/ info/          empty
  objects/xx/yyyy...    loose zlib git objects (only the very first commit survives here)
  objects/pack/         empty
  refs/heads/main       40 hex digits (stale)
  refs/heads/vNNN_X.Y   "ref: HEAD"
  interprocess.lock     empty
  projectdata.rope      THE LOG. Authoritative.
```

Character Animator keeps a git-shaped store but writes it through `projectdata.rope`, an append-only log of objects
and ref updates. The on-disk loose objects and ref files are a snapshot from when the repository was created, so the
reader loads them first and then replays the rope on top; the rope wins.

## Numbers

Used for every length and size. First byte `b`:

| `b`        | meaning                                         |
|------------|-------------------------------------------------|
| `0`        | the value 0                                     |
| `1..=8`    | `b` big-endian bytes follow and hold the value  |
| `9..=255`  | the value `b` itself                            |

The writer always emits the shortest spelling, so 4 is `01 04`, 15 is `0f`, 162 is `a2`, 256 is `02 01 00` and 0x142
is `02 01 42`. The reader also accepts longer spellings. (Lengths 1 to 8 need the long form because those bytes are
taken by the count.)

## Records

A rope is a plain concatenation of records, with no file header and no index:

```
offset  size   field
0       14     magic  "Rope10LMm" 04 c3 ff cf 09   (the tail is the number 0xC3FFCF09 in the form above)
14      1      kind   0 object, 1 set ref, 2 delete ref
15      1      type   see below
16      n      len    number: payload length in bytes
16+n    len    payload
...     m      trailer  the length of everything above (magic through payload), as below
```

### Trailer

The trailer lets a reader walk the file backwards. It is the record length `L` (magic through payload) written from
the back: if `9 <= L <= 255` it is the single byte `L`; otherwise it is the value in the fewest little-endian bytes
followed by that byte count. Examples: 39 is `27`; 0x16f is `6f 01 02`. The reader checks it against the length it
measured and rejects the record on mismatch.

### Objects: kind 0

`type` is the git object type: 1 commit, 2 tree, 3 blob, 4 annotated tag. Payload:

```
20     id        SHA-1 of "<type> <raw size>\0<content>", exactly as in git
n      raw size  number: size of the content
n      stored    number: size of the bytes that follow
...    bytes     the content, or a gzip stream of it
```

When `stored == raw` the bytes are the content itself. When they differ, the bytes are a gzip stream (RFC 1952;
header `1f 8b 08 00 00000000 02 13`, that is XFL 2 and OS 0x13) that inflates to exactly `raw` bytes. Character
Animator compresses objects larger than 512 bytes (largest raw object seen: 505, smallest compressed: 520), and
the encoder here does the same. Because deflate implementations differ, re-encoding a compressed record can change
its bytes (never its content); 8 560 of 9 579 sample records re-encode identically and the other 1 019 are all gzip.

The content is standard git: commits are `tree/parent/author/committer` headers, a blank line and a message; trees are
`<octal mode> <name>\0<20 byte id>` entries in git order (directories sort as `name/`); tags have
`object/type/tag/tagger` headers.

### Set ref: kind 1

Payload starts with the ref name as `number length, bytes`.

* `type 1`, direct: the name, then 20 bytes of object id. Example: `0f "refs/heads/main" <20 bytes>`.
* `type 2`, symbolic: the name, then the target name in the same form. Example: `01 04 "HEAD" 0f "refs/heads/main"`
  (the first record of every rope).

### Delete ref: kind 2, type 0

Payload is the ref name. Seen when a `refs/heads/vNNN_X.Y` branch is retired.

### Unknown records

The framing is self-describing, so a record of any other kind or type can be skipped and kept. This crate preserves
it as `Record::Unknown` and writes it back unchanged.

## What the log looks like

Every rope starts with `HEAD -> refs/heads/main`, the empty tree `4b825dc6...` and a first commit
(`.FIRST COMMIT FOR VERSION.`), then `refs/heads/main`, then `refs/heads/vNNN_X.Y -> HEAD` naming the Character
Animator build that created it (`190 25.0` is version 25.0, project version 190).

After that, every editing command appends its blobs, then trees from the leaves up, then the commit, then moves a
ref. In the large project 1 574 ref updates and 1 245 blobs follow. Observed conventions:

* `HEAD` is set DIRECTLY to the newest commit (type 1, name `HEAD`). It is the working tip. `refs/heads/main`
  moves only when the project is saved, so after an unsaved session `main` is behind `HEAD`.
* Commit messages are the name of the command wrapped in dots (`.Selectron.setFocusedSelection`) or, for
  user-visible edits, plain text (`Copy Media Files into Project Folder`). The author and committer are
  `<build> mac <macOS> user (<uid>) <red@monster> <seconds> <zone>`.
* Undo/redo and save points are annotated tags: `refs/tags/v190/recent/N` and `refs/tags/v190/saves/N`, 96 of them
  in the sample. A tag's message is an ltab record, for example
  `TF1:{ iProjVersion = 190, sComment = "preservedRedo", sMasterCommitOid = "...", sName = "Add Handle ..." }`.
* A tree entry is one file or folder of the project. Everything is a text blob; media is NOT stored here but in
  `Ch Media/` next to `Ch Data/`.

## What the blobs hold

Each blob is text shaped `ClassName{ key = value, ... }`: a Lua table constructor preceded by the class it
deserialises into, with the same prefix on nested objects (`_aBoundBehaviors = IDArray_001{ ... }`). Seen classes:
`Puppet`, `Skin`, `Behavior`, `KeyframeBucket`, `RecordingRef_001`, `ProjectUIData_001`, `ProjectSelection`, `Alias`
and more. Folder metadata files (`.folderData`, `.durationData`, `.treeFactoryKey`) are tiny blobs of the same shape.
`$Behaviors/Adobe.*.js` holds each stock behavior's definition and its JavaScript.

Two details a parser must know:

* Class names are not valid in a plain table constructor. `charcraft_rope::parse_class_table` removes them with a
  string-aware scan and parses the remainder with `charcraft-ltab`; the root class is returned alongside.
* Infinity is written as the Lua expression `1 / 0` (`_maxTrackItems = 1 / 0`) and NaN as `0 / 0`. The scan rewrites
  those to `inf` and `nan`.

All 348 files of the large project's `HEAD` parse this way, and so do the two puppets'.

## Safety

The reader treats everything as hostile: record, object and tree sizes are capped by `Limits`; gzip is inflated
through a size-bounded reader and must produce exactly the declared size; ids are verified; a damaged tail (an
interrupted append) keeps the good prefix and reports a warning; tree entry names that are empty, `.`, `..` or contain
`/` are rejected; tree depth, entry count and file count are bounded.

## Writing

`RopeStore::to_rope_bytes` replays the log, and appending is plain concatenation of encoded records
(`encode_record`) because every record is self-delimiting. Character Animator has never been seen to require an
index, a header or a footer.
