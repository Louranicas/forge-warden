# ADR 0010: Git treats candidate content as data

Status: D3 correction contract; C1 coordination handoff. Author: Bastion (O4).

The authenticated runtime supplies the Git executable, controller-owned config and immutable
subject/snapshot. Candidate repository config, worktree attributes and inherited process variables
must not select executables. Disabling one extension (fsmonitor) is not containment: status can
run clean filters; checkout can run smudge filters; diff can run textconv/external drivers; local
transport can invoke upload-pack with a different authority boundary.

Shared requirements for scripts (D3) and the Rust door (ADR0005 §2.5):

- Clear the child environment. Allow only explicit locale/runtime entries and the controls below;
  never inherit GIT_CONFIG_COUNT/PARAMETERS, GIT_DIR/WORK_TREE/INDEX_FILE, object alternates,
  namespace/replace settings, executable paths, loaders, SSH commands or pagers from the caller.
- Set LC_ALL=C, GIT_CONFIG_NOSYSTEM=1, GIT_CONFIG_GLOBAL=/dev/null,
  GIT_ATTR_NOSYSTEM=1, GIT_NO_REPLACE_OBJECTS=1; HOME is a non-user location. D3 uses /nonexistent;
  C1 uses its private empty directory. D3's PATH is /usr/bin:/bin for the trusted runtime; C1's
  no-PATH environment is stricter. Never discover a repository above C1's empty temporary cwd.
- Absolute trusted Git executable, argv only, no shell interpolation of patch bytes. Where a
  repository exists, it and its config belong to the controller, never the candidate. Disable
  core.attributesFile (/dev/null), core.hooksPath (/dev/null), core.fsmonitor (false). For any
  object diff use --no-ext-diff --no-textconv --binary --no-renames with explicit commit IDs.
- No worktree-touching Git command in a candidate checkout. C1 reads one bounded immutable patch
  snapshot through stdin in its empty directory (`apply --numstat -z` / `apply --summary`, not
  --index/--cached). D3 invokes only object lookups in its trusted repository. No candidate .git
  config, index or worktree is consulted by the D3 helper.
- Object acquisition is outside D3's helper. The owner-controlled controller must supply both
  commit objects from authenticated data transport into its own repository before launch; it must
  not treat a candidate-side Git command as a safe import step. No import implementation or hosted
  acceptance is asserted here.

C1 retains its own measured timeout/stream/process-group/runtime admission requirements; D3's
small object-bundle helper does not substitute for them. A trusted repository and these flags do
not sandbox a compromised Git/runtime or an arbitrary same-uid writer. C1's owner must cite or
reconcile this contract in the Rust implementation evidence rather than infer shared code reuse.
