# What a Character Animator project holds

Reverse engineered from `Boy.puppet`, `blood_cells.puppet` and the `news anchors` project, for interoperability. The
byte-level store (rope, objects, refs) is in `crates/rope/FORMAT.md`; this file is about what the files at `HEAD`
mean. All 361 class-tagged files of the three samples parse, decode and print back byte for byte.

## Two containers, one layout

```
Boy.puppet (zip)                         news anchors/ (folder)
  Boy.tmp.chproj          marker, empty    news anchors.chproj      marker, empty
  Ch Data/repo.noindex/   the store        Ch Data/repo.noindex/    the store
  Ch Media/Gathered Media/ artwork, audio  Ch Data/artwork.noindex/ cache (PNG per layer, named by guid)
  _archive_info           manifest         Ch Data/temp.noindex/    compiled behaviors, warp cache
                                           Ch Media/Gathered Media/ + Ch Media/Recordings/<date>/*.arf
```

A `.puppet` has one root puppet and the manifest; a project has scenes and any number of puppets. Both are opened by
`Project::open`.

## `_archive_info` (puppets only)

A Lua table: `iVersion`, `sType = "puppet"`, `sUniqueID` (differs from the root puppet's `_sGuid`),
`rootPuppetPPath = "/ProjectItems/<name>"`, `bContainsArtworkCache` and `tBehaviorDefaults`, the default parameter
values of every stock behavior that puppet uses (`["Adobe.Blinker.js"] = { length = 80, rate = 30, ... }`).

## Files at HEAD

| Path | Class | Meaning |
|---|---|---|
| `ProjectItems/<name>` | `Puppet` | a puppet; the ones directly under `ProjectItems/` are the project items |
| `ProjectItems/.Parts/**` | `Puppet`, `Skin` | the pieces of a rig (one puppet per group layer) and the artwork files |
| `ProjectItems/<name>.wav` | `AudioFile_000` | an audio item (alias, length, transcript) |
| `ProjectItems/<scene>/_sceneData` | `SceneData` | a scene's settings |
| `ProjectItems/<scene>/_idaTracks/idaTracksN` | `ProjectUIData_001{ i, v = Track{} }` | a track and its items |
| `ProjectItems/<scene>/_idaMarkers/*` | `ProjectUIData_001{ v = Marker_001{} }` | timeline markers |
| `ProjectItems/<scene>/_amKeyframeBuckets/<track item and take>/recordingRef` | `RecordingRef_001` | link from a take to its keyframes |
| `.../kfba/<n>` | `KeyframeBucket` | `_aKeyframes = { { t = seconds, v = value } }` |
| `$Behaviors/Adobe.*.js` | `Behavior` | a behavior's parameters, tags and JavaScript |
| `.Recordings/**.arf` | `EventGraphFile_000` | alias to a recording in `Ch Media/Recordings` |
| `.ui/**`, `.project/info`, `**/.folderData`, `.treeFactoryKey` | `ProjectUIData_001` | UI state, empty markers |

Paths inside files (`PPath{ p = "/ProjectItems/x" }`, `CPath{ p = "/ProjectItems/s:_children:_idaTracks:..." }`) are
project paths: the store path with a leading `/`; a `CPath` separates tree levels with `:`.

### Puppet

- `_idaBindings` (`IDArray_001`): the children. Each is a `SkinBinding` (a layer of the artwork: `_ppath` is a layer
  path inside the skin, such as `/ProjectItems/.Parts/X/X.ai/Ch/Head/mouth/Oh`) or a `PuppetBinding` (a child puppet
  file). A binding has `_name` (`false` means "use the layer's name"), `_active`, `_opacity`, `_blendMode`,
  `_attachToMatDecomposed` (position, scale, angle, shear), `_tTags`, `_keyTrigger` and `_tTriggers`.
- The first binding is the puppet's own artwork layer (its `_ppath` equals `_originatingSkinPPath0`).
- `_tTags`: `["Adobe.LipSynch.Oh"] = false`. The tag is applied when listed; every value in the samples is `false`.
- `_aBoundBehaviors`: `BB1..` with `_behaviorPPath = /$Behaviors/Adobe.X.js` and `_sparseParams`, the parameters that
  differ from the defaults (empty in the samples; the defaults live in the manifest and in `$Behaviors`).
- `_origin = Handle{}` with `_aChildren`, `_tTags`, `_locationTag`; `_tTaggedLocations` maps location names to `{x, y}`.
- `_oTriggerCollection` holds `TriggerGroup{ _idaTriggers = { Trigger{ _triggerKey, _triggerLatch } } }`.
- `_idaHotTakes`: `HotTake{}` with recorded `TakeClipboardItem`s (replays that a trigger starts).
- There is no stored swap set: mouth layers are found by their `Adobe.LipSynch.*` tags, triggered layers by key.

### Scene

`SceneData`: `_framerate`, `_pixWidth`, `_pixHeight`, `_sceneDurationSec`, `_centeredOrigin`, `_playbackSpeed`,
`_workAreaIn/Out`, `_takeCounter`, `_scenePuppetTrackItemCPath`. A `Track` has `_kind` (`trackKind_puppet`,
`trackKind_audio`), `_uiColor`, `_maxTrackItems` (`1 / 0` for unlimited) and `_idaTrackItems`; a `TrackItem` has
`_trackSource`, `_timeItem = TimeItem_001{ _durationTime, _parentOffsetTime, _trimInTime, _trimOutTime }` (seconds) and
`_rootTrackItemPuppet = PuppetInstance_001{ _behaviorBag = { BB1 = BehaviorInstance_001{ _paramBag, _egPile } } }`.
Each recorded take is an `EventGraphPileItem_` in `_egPile._idaPileItems`.

### Behavior

`_aParams` (`id`, `type`, `dephault`, `min`, `max`, `precision`, `uiName`), `_aBehaviorDefinedTags`
(`id`, `tagType`, `artMatches` = layer names that get the tag automatically), `_attributes`, `_jsCode`. Display text is
a localisation key, `$$$/animal/Behavior/Blinker/Parameter/rate=Blinks per Minute`; the text after `=` is shown.

## The table text

Class-tagged tables as in `charcraft-rope/FORMAT.md`, with two details found while printing them back:

- a line break inside a string is a backslash followed by the line break itself, and a tab is a plain tab; the
  `charcraft-ltab` printer writes `\n` and `\t` instead, so `ClassedDoc` converts them;
- infinity is written `1 / 0` and NaN `0 / 0`.

## Saving

`projectdata.rope` is only ever appended to: `Project` keeps the bytes it read, drops a torn tail, and appends the
records of the session. A save moves `refs/heads/main` to `HEAD` when the session committed. The loose objects and
ref files in `repo.noindex` are a snapshot from creation and are left as they are, as Character Animator does.
