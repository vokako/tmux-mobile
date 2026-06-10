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

## Kotlin Gradle Plugin vs Gradle 8.14 Config Cache

### Context
Separate from the `BuildTask.kt` issue above. The generated
`src-tauri/gen/android/build.gradle.kts` pinned the Kotlin Gradle
plugin at **1.9.25** while the wrapper
(`gradle/wrapper/gradle-wrapper.properties`) ships **Gradle 8.14.3**.
Kotlin 1.9.25 only officially supports up to ~Gradle 8.6, and its
plugin internals are incompatible with the configuration cache in
Gradle 8.14. With config cache enabled (the user's global
`~/.gradle/gradle.properties` sets `org.gradle.configuration-cache=true`),
`build:android` fails during the cache fingerprint step with:

```
Could not load the value of field `parameters` of
  ...ConfigurationCacheFingerprint$ValueSource...
> Class 'org.jetbrains.kotlin.gradle.plugin.internal.
  CustomPropertiesFileValueSource$Parameters' not found ...
```

The Rust + frontend stages succeed; only the Gradle assemble step dies.

### Decision
Bumped the Kotlin Gradle plugin to **2.2.0** in
`src-tauri/gen/android/build.gradle.kts`:

```kotlin
classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:2.2.0")
```

Kotlin 2.2.0 officially supports Gradle 7.6.3–8.14 and is configuration
-cache-clean; it pairs with the existing AGP 8.11.0.

### Why not downgrade Gradle instead
AGP 8.11.0 requires Gradle ≥ 8.13, so the wrapper cannot drop below
8.13. Bumping Kotlin (rather than Gradle) is the only direction that
satisfies both AGP and the config cache.

### Required cleanup after the bump
The first build after editing the plugin version still failed with the
identical error — Gradle was **reusing the stale config-cache entry**
stored by the old 1.9.25 run. Delete the project Gradle state and
rebuild:

```bash
rm -rf src-tauri/gen/android/.gradle \
       src-tauri/gen/android/build \
       src-tauri/gen/android/app/build
```

After that the build succeeds (with only `jvmTarget` deprecation
warnings — Kotlin 2.x wants the `compilerOptions` DSL; non-blocking).

### Alternatives Considered
- **Disable config cache via project `gradle.properties`**: Rejected,
  same reason as the `BuildTask.kt` section — `~/.gradle` (user-level)
  overrides project-level, and Tauri doesn't forward
  `--no-configuration-cache` to `gradlew`.
- **Edit the user's global `~/.gradle/gradle.properties`**: Rejected —
  affects every project on the machine.

## Build Output Location (this machine)

Tauri prints the APK path as the in-project
`src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk`,
**but on this machine that path does not exist after a build.** The
Gradle build directory and cargo target are relocated to a global cache
(set in the user's environment, not in the repo):

- APK: `~/.cache/builds/gradle-builds/android-<hash>/app/outputs/apk/universal/release/app-universal-release.apk`
- Rust `.so`: `~/.cache/builds/cargo-target/aarch64-linux-android/release/libtmux_mobile.so`

Each project gets its own `android-<hash>` dir. To locate the freshest
APK: `find ~/.cache/builds -name '*.apk' -newermt '-10 minutes'`.

## Build Prerequisites (this machine)

- `NDK_HOME` is **not** set in the shell; `tauri android build` needs
  it. Export before building:
  `export NDK_HOME="$ANDROID_HOME/ndk/28.1.13356709"` (NDK 28.1 is the
  installed version under `$ANDROID_HOME/ndk/`).
- JDK 17 (`JAVA_HOME` → openjdk@17) and the `aarch64-linux-android`
  rustup target are present.
