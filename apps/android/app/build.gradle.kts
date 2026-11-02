import groovy.json.JsonSlurper

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.mozilla.rust-android-gradle.rust-android")
}

android {
    namespace = "com.mydomain.android"
    compileSdk = 35
    ndkVersion = "30.0.14904198"

    defaultConfig {
        applicationId = "com.mydomain.android"
        minSdk = 24
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"

        // Firebase ids for FCM, read from the gitignored
        // google-services-android.json (see Fcm.kt). Empty = FCM off.
        val fcm = firebaseIds(applicationId ?: "")
        buildConfigField("String", "FCM_APP_ID", "\"${fcm["app_id"]}\"")
        buildConfigField("String", "FCM_API_KEY", "\"${fcm["api_key"]}\"")
        buildConfigField("String", "FCM_PROJECT_ID", "\"${fcm["project_id"]}\"")
        buildConfigField("String", "FCM_SENDER_ID", "\"${fcm["sender_id"]}\"")
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }
}

/**
 * Firebase ids from `google-services-android.json` (repo `apps/android/`, or
 * `app/google-services.json`). Prefers the client registered for this
 * package, else the first one: we initialise Firebase in code, so the
 * package name needn't match the way the google-services plugin demands.
 */
fun firebaseIds(pkg: String): Map<String, String> {
    val empty = mapOf("app_id" to "", "api_key" to "", "project_id" to "", "sender_id" to "")
    val file = listOf(rootProject.file("google-services-android.json"), file("google-services.json"))
        .firstOrNull { it.exists() } ?: return empty
    @Suppress("UNCHECKED_CAST")
    val root = JsonSlurper().parse(file) as Map<String, Any?>
    val info = root["project_info"] as Map<String, Any?>
    @Suppress("UNCHECKED_CAST")
    val clients = root["client"] as List<Map<String, Any?>>
    fun pkgOf(c: Map<String, Any?>) =
        ((c["client_info"] as Map<*, *>)["android_client_info"] as Map<*, *>)["package_name"]
    val client = clients.firstOrNull { pkgOf(it) == pkg } ?: clients.firstOrNull() ?: return empty
    @Suppress("UNCHECKED_CAST")
    val keys = client["api_key"] as List<Map<String, Any?>>
    return mapOf(
        "app_id" to ((client["client_info"] as Map<*, *>)["mobilesdk_app_id"] as String),
        "api_key" to (keys.firstOrNull()?.get("current_key") as String? ?: ""),
        "project_id" to (info["project_id"] as String),
        "sender_id" to (info["project_number"] as String),
    )
}

// Locate the Rust toolchain. Android Studio's Gradle daemon often doesn't
// inherit your shell PATH (esp. when launched from the macOS Dock), so point
// at the rustup shims directly. Honors CARGO_HOME, else defaults to ~/.cargo.
// Override per-machine with -Prust.cargoBin=/path/to/cargo/bin if needed.
val cargoBin: String =
    (project.findProperty("rust.cargoBin") as String?)
        ?: System.getenv("CARGO_HOME")?.let { "$it/bin" }
        ?: "${System.getProperty("user.home")}/.cargo/bin"

// Build the Rust crate (../rust) into per-ABI .so files and bundle them.
cargo {
    module = "../rust"
    libname = "mydomain_net"
    // rust-android-gradle target names -> Android ABIs:
    //   arm64 -> arm64-v8a, x86_64 -> x86_64, arm -> armeabi-v7a, x86 -> x86
    targets = listOf("arm64", "x86_64", "arm", "x86")
    profile = "release"
    cargoCommand = "$cargoBin/cargo"
    rustcCommand = "$cargoBin/rustc"
    // The plugin's generated linker-wrapper.sh invokes `python`; modern macOS
    // only ships `python3`, so point it there (override with -Prust.python=...).
    pythonCommand = (project.findProperty("rust.python") as String?) ?: "python3"
    // The crate is a member of the repo-root Cargo workspace, so Cargo writes to
    // <repo>/target, not ../rust/target. rootDir = apps/android.
    targetDirectory = "${rootDir}/../../target"
}

// Make sure the Rust libs are built before Gradle packages the JNI libs, and
// never trust the merge's up-to-date check: on 2026-11-02 we found every APK
// for a day had shipped a stale (pre-QUIC) libmydomain_net.so because
// mergeDebugJniLibFolders kept reporting UP-TO-DATE. Re-merging is a cheap copy.
tasks.whenTaskAdded {
    if (name == "mergeDebugJniLibFolders" || name == "mergeReleaseJniLibFolders") {
        dependsOn("cargoBuild")
        outputs.upToDateWhen { false }
    }
}


dependencies {
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.7")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")

    val composeBom = platform("androidx.compose:compose-bom:2024.10.01")
    implementation(composeBom)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-graphics")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-core")

    // Material XML theme used by the activity (Theme.Material3.*).
    implementation("com.google.android.material:material:1.12.0")

    // FCM: signals wake this device without a server-held connection.
    implementation(platform("com.google.firebase:firebase-bom:33.7.0"))
    implementation("com.google.firebase:firebase-messaging")
}
