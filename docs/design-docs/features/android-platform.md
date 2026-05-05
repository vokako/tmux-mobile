# Android Platform Integration

## Context
Tauri 2 on Android has several plugin bugs and WebView limitations that require custom workarounds.

## Decision
Use custom `@JavascriptInterface` in `MainActivity.kt` for file opening instead of `tauri-plugin-opener`. Implement retry-based WebView detection for JS interface injection.

## How It Works
- `MainActivity.kt` → `FileOpener` inner class → `@JavascriptInterface` → `FileProvider.getUriForFile` → `Intent.ACTION_VIEW`
- `attachFileOpener` retries up to 50 times (5s) waiting for WebView in view hierarchy
- Frontend: `waitForFileOpener()` polls for `window.AndroidFileOpener` (up to 2s) before file open
- Downloads go to `/storage/emulated/0/Download/TmuxMobile/`
- Keyboard height via `OnGlobalLayoutListener`, safe area insets via `WindowInsetsCompat`
- Cleartext ws:// enabled via `network_security_config.xml`

## Alternatives Considered
- **tauri-plugin-opener**: Rejected — `openPath()` fails with `OpenArgs` deserialization error on Android
- **Single rootView.post for JS interface**: Rejected — WebView may not be in hierarchy yet, causing silent failure

## Trade-offs
- Custom native code to maintain in `MainActivity.kt`
- Retry loops add startup latency (up to 5s worst case)
- `gen/android/` files survive `tauri android init` only if backed up

## Lessons Learned
- NEVER use `tauriOpener.openPath()` on Android — always use `AndroidFileOpener`
- Always check `isAndroid` before falling back to generic Tauri APIs
- `addJavascriptInterface` via single `rootView.post` can fail silently — use retry loop with max attempts
- After changing `tauri.conf.json` identifier, must delete `gen/android/` and run `tauri android init`

## Gradle Configuration Cache vs Tauri's Generated BuildTask.kt

### Context
Gradle 8.11+ enables the configuration cache by default for new projects
(and many developers enable it globally in `~/.gradle/gradle.properties`
for speedup). The configuration cache is a correctness mode: it
serializes all task inputs at configuration time and re-uses them on
subsequent builds, skipping the configuration phase. Under this mode,
accessing `Task.project` from `@TaskAction` (execution time) is
forbidden — `Project` objects don't survive serialization.

Tauri's `tauri android init` generates
`src-tauri/gen/android/buildSrc/src/main/java/<package>/kotlin/BuildTask.kt`
which does exactly that:

```kotlin
@TaskAction
fun assemble() {
    // ...
    project.exec {                     // forbidden
        workingDir(File(project.projectDir, rootDirRel))   // forbidden
        if (project.logger.isEnabled(...)) { ... }         // forbidden
    }
}
```

Result: with config cache enabled, `build:android` fails with

```
Task `:app:rustBuildArm64Release` of type `BuildTask`:
  invocation of 'Task.project' at execution time is unsupported.
```

and Gradle refuses to proceed.

### Decision
Rewrote `BuildTask.kt` in-place to use Gradle's configuration-cache-safe
injection pattern:

```kotlin
open class BuildTask @Inject constructor(
    private val execOps: ExecOperations,
    private val layout: ProjectLayout,
) : DefaultTask() {
    // ...
    @TaskAction
    fun assemble() {
        val projectDir = layout.projectDirectory.asFile
        val taskLogger = logger  // DefaultTask.logger, task-scoped, cache-safe
        execOps.exec {
            workingDir(File(projectDir, rootDirRel))
            // ...
            if (taskLogger.isEnabled(LogLevel.DEBUG)) { args("-vv") }
        }
    }
}
```

`ExecOperations`, `ProjectLayout`, and `DefaultTask.logger` are all
officially supported, cache-safe alternatives. The `RustPlugin.kt`
registration (`tasks.maybeCreate("rustBuild…", BuildTask::class.java)`)
doesn't need to change — Gradle instantiates the task via the
`@Inject` constructor and supplies the services automatically.

### Trade-offs
- Tauri may regenerate this file on `tauri android init --force`. The
  patch then needs to be re-applied. Not expected to happen often.
- Upstream fix (tauri-apps/tauri) would make this workaround
  unnecessary but would also overwrite our version. When syncing
  upstream template changes, verify the generated BuildTask.kt still
  uses the injection pattern before discarding our version.
- Deleting `src-tauri/gen/android/buildSrc/build` and
  `.../buildSrc/.gradle` is required after editing BuildTask.kt to
  force Kotlin recompilation — Gradle sometimes caches stale class
  files from the prior `project.exec` version.

### Alternatives Considered
- **Disable config cache in project `gradle.properties`**: Rejected.
  User-level `~/.gradle/gradle.properties` overrides project-level,
  and Tauri CLI doesn't expose a way to pass
  `--no-configuration-cache` to `gradlew`.
- **Set `org.gradle.configuration-cache.problems=warn`**: Tested and
  works as a quick fix, but the cache then can't actually store
  anything useful — loses the speedup. Also silently accepts whatever
  other config-cache violations Tauri may introduce later.
- **Ask user to change global `~/.gradle/gradle.properties`**:
  Rejected — affects every Gradle project on the machine.

### Lessons Learned
- When a generated-template file is incompatible with a newer Gradle
  feature, prefer patching the generated file in-place (recorded in a
  design doc) over muting the error or changing global config.
- Configuration-cache-safe task pattern: inject `ExecOperations`,
  `ProjectLayout`, `FileSystemOperations`, etc. via `@Inject`; use
  `DefaultTask.logger` instead of `project.logger`.
- Clean `buildSrc/build` and `buildSrc/.gradle` when modifying
  plugin Kotlin — Gradle can run a stale compiled class against
  your updated source.
